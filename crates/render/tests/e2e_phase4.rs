use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;
use uuid::Uuid;
use ve_core::{
    CommandBus, CommandSource, CubicBezier, EditCommand, EffectNode, FrameRate, Interpolation,
    ItemKind, Keyframe, PrecompCacheManager, Project, ProjectFolder, ProjectSettings, RationalTime,
    RgbaColor, ShapeKind, TimelineItem,
};
use ve_render::{RenderGraph, ShaderHotReloader, ShaderReloadStatus};

/// Phase 1a 〜 Phase 4 を横断するエンドツーエンド統合テスト:
/// 1フォルダプロジェクト初期化 -> CommandBus編集（キーフレーム＋プラグインエフェクト＋Smart Precompose）
/// -> ハッシュLRUキャッシュ付きレンダリング -> カスタムWGSLホットリロード -> history.jsonlクラッシュ復旧
#[test]
fn end_to_end_project_precomp_plugin_hot_shader_and_recovery() {
    let tmp = tempdir().unwrap();
    let proj_dir = tmp.path().join("E2EProject");

    let settings = ProjectSettings {
        width: 32,
        height: 32,
        fps: FrameRate::FPS_60,
        ..Default::default()
    };
    let initial_project = Project::new("E2E Hybrid Project", settings);
    let folder = ProjectFolder::init(&proj_dir, &initial_project).unwrap();

    let mut live = initial_project.clone();
    let mut bus = CommandBus::with_folder(folder.clone());

    // 1. シェイプ追加 + キーフレーム + プラグインエフェクト (plugin.sepia) + Smart Precompose
    let rect = TimelineItem::new(
        "HeroPanel",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 32.0,
            height: 32.0,
            color: RgbaColor::WHITE,
        },
    );
    let rect_id = rect.id;
    let precomp_scene_id = Uuid::new_v4();
    let precomp_item_id = Uuid::new_v4();

    let mut sepia_params = BTreeMap::new();
    sepia_params.insert("mix".to_string(), 1.0);

    bus.execute(
        &mut live,
        EditCommand::Batch {
            label: "build_motion_clip".into(),
            commands: vec![
                EditCommand::AddObject {
                    layer_index: 0,
                    item: rect,
                },
                EditCommand::SetKeyframe {
                    item_id: rect_id,
                    property: "opacity".into(),
                    keyframe: Keyframe {
                        time: RationalTime::ZERO,
                        value: 0.5,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                    },
                },
                EditCommand::SetKeyframe {
                    item_id: rect_id,
                    property: "opacity".into(),
                    keyframe: Keyframe {
                        time: RationalTime::from_seconds(1.0),
                        value: 1.0,
                        interpolation: Interpolation::Linear,
                    },
                },
                EditCommand::AddEffect {
                    item_id: rect_id,
                    effect: EffectNode {
                        id: Uuid::new_v4(),
                        effect_type: "plugin.sepia".into(),
                        enabled: true,
                        params: sepia_params,
                    },
                },
                EditCommand::PrecomposeSelection {
                    item_ids: vec![rect_id],
                    new_scene_id: precomp_scene_id,
                    precomp_name: "HeroComp".into(),
                    precomp_item_id,
                    target_layer_index: 0,
                },
            ],
        },
        CommandSource::Mcp,
        None,
    )
    .unwrap();

    // 2. ハッシュLRUキャッシュ付き RenderGraph で描画（セピア調プラグイン反映＆2回目キャッシュヒット）
    let cache_mgr = PrecompCacheManager::new(folder.cache_precomp_dir(), 10 * 1024 * 1024).unwrap();
    let mut rg = RenderGraph::new().with_disk_cache(cache_mgr);
    let main_id = live.active_scene_id;

    let frame_t1 = rg
        .render_project_scene(&live, main_id, RationalTime::from_seconds(1.0), false)
        .unwrap();
    // セピア変換後は R > G > B の暖色バランスになる
    let px = frame_t1.pixels[16 * 32 + 16];
    assert!(px.r > px.b, "expected sepia tint R > B, got {px:?}");
    assert_eq!(rg.cache_misses, 1);

    let _cached = rg
        .render_project_scene(&live, main_id, RationalTime::from_seconds(1.0), false)
        .unwrap();
    assert_eq!(rg.cache_hits, 1);

    // 3. カスタム WGSL シェーダーホットリロード検証（リポジトリ同梱のサンプル WGSL を実検証）
    let mut reloader = ShaderHotReloader::new();
    let sample_shader = tmp.path().join("custom.wgsl");
    fs::write(
        &sample_shader,
        include_str!("../../../shaders/custom_chromatic_aberration.wgsl"),
    )
    .unwrap();
    let entry = reloader.register_file("chromatic", &sample_shader).unwrap();
    assert!(matches!(entry.status, ShaderReloadStatus::Loaded { .. }));

    // 4. クラッシュ復旧: 保存前 project.json に対して history.jsonl をリプレイし完全一致を確認
    let mut recovered = folder.load_project().unwrap();
    let history = folder.read_history().unwrap();
    CommandBus::replay_history(&mut recovered, &history).unwrap();
    assert_eq!(recovered, live);
}
