//! v10.1 最適化・軽量化 回帰検証テストスイート
//!
//! 1. `TextRenderer` 省メモリ BBox LRU キャッシュ（ヒット率・ビット完全一致・メモリ圧縮率・定常アニメーション判定）
//! 2. `RenderGraph` の `Arc<CubeLut3D>` キャッシュ・プロキシ/フォルダ同一値ガード・リビジョン別シーン署名キャッシュ
//! 3. `LinearImage` ホイスト済み合成/トラックマット/`to_rgba8_srgb`/`write_rgba16f_bytes` ＆ カラーグレーディング/スコープ LUT 決定論性

use tempfile::tempdir;
use uuid::Uuid;
use ve_core::{
    BlendMode, ColorPalette, ItemKind, Project, ProjectFolder, ProjectSettings, RationalTime,
    RgbaColor, SceneColorGrade, ShapeKind, TextAnimator, TextStyle, TimelineItem, TrackMatte,
    TrackMatteMode,
};
use ve_render::{
    LinearImage, LinearPremul, RenderGraph, TextRenderer, apply_scene_color_grade,
    compute_video_scopes,
};

#[test]
fn text_renderer_bbox_lru_cache_hits_matches_uncached_bit_for_bit_and_bounds_memory() {
    let mut cached = TextRenderer::new();
    let mut uncached = TextRenderer::new();
    let style = TextStyle {
        outline_width: 3.0,
        outline_color: RgbaColor::BLACK,
        shadow_offset: [3.0, 3.0],
        shadow_blur: 2.0,
        shadow_color: RgbaColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.6,
        },
        box_enabled: true,
        ..TextStyle::default()
    };
    let palette = ColorPalette::default();
    let no_anim = TextAnimator::default();

    // 1 回目はミス → キャッシュ格納、2 回目はヒットでビット完全一致
    let first = cached.render_text_styled(
        "最適化テスト Subtitle",
        "sans-serif",
        36.0,
        RgbaColor::WHITE,
        &no_anim,
        &style,
        &palette,
        0.0,
        640,
        360,
    );
    let second = cached.render_text_styled(
        "最適化テスト Subtitle",
        "sans-serif",
        36.0,
        RgbaColor::WHITE,
        &no_anim,
        &style,
        &palette,
        2.5, // 静的テキストは時刻が変わってもキャッシュヒットする
        640,
        360,
    );
    let fresh = uncached.render_text_styled(
        "最適化テスト Subtitle",
        "sans-serif",
        36.0,
        RgbaColor::WHITE,
        &no_anim,
        &style,
        &palette,
        2.5,
        640,
        360,
    );

    assert_eq!(cached.text_cache_misses(), 1);
    assert_eq!(cached.text_cache_hits(), 1);
    assert_eq!(first.pixels, second.pixels);
    assert_eq!(second.pixels, fresh.pixels);

    // 透過領域をトリミングして保持しているため、フルキャンバス (640*360*16B ≈ 3.69MB) より大幅に小さい
    let full_canvas_bytes = 640 * 360 * std::mem::size_of::<LinearPremul>();
    assert!(
        cached.text_cache_bytes() < full_canvas_bytes / 2,
        "expected tight bbox cache ({}) to be < 50% of full canvas ({})",
        cached.text_cache_bytes(),
        full_canvas_bytes
    );

    // 文字ごとアニメーション: 途中の時刻はキャッシュせず、完了後の定常区間はキャッシュヒットする
    let anim = TextAnimator {
        enabled: true,
        delay_per_char_sec: 0.05,
        duration_per_char_sec: 0.15,
        ..TextAnimator::default()
    };
    let mid_a = cached.render_text_styled(
        "Hello",
        "sans-serif",
        32.0,
        RgbaColor::WHITE,
        &anim,
        &TextStyle::default(),
        &palette,
        0.05,
        320,
        120,
    );
    let mid_b = cached.render_text_styled(
        "Hello",
        "sans-serif",
        32.0,
        RgbaColor::WHITE,
        &anim,
        &TextStyle::default(),
        &palette,
        0.15,
        320,
        120,
    );
    assert_ne!(mid_a.pixels, mid_b.pixels);

    let hits_before_settled = cached.text_cache_hits();
    let settled_1 = cached.render_text_styled(
        "Hello",
        "sans-serif",
        32.0,
        RgbaColor::WHITE,
        &anim,
        &TextStyle::default(),
        &palette,
        1.5,
        320,
        120,
    );
    let settled_2 = cached.render_text_styled(
        "Hello",
        "sans-serif",
        32.0,
        RgbaColor::WHITE,
        &anim,
        &TextStyle::default(),
        &palette,
        3.0,
        320,
        120,
    );
    assert_eq!(cached.text_cache_hits(), hits_before_settled + 1);
    assert_eq!(settled_1.pixels, settled_2.pixels);
}

#[test]
fn render_graph_noop_guards_disk_sig_cache_and_color_paths_stay_deterministic() {
    let tmp = tempdir().unwrap();
    let mut proj = Project::new(
        "PerfCacheGuard",
        ProjectSettings {
            width: 160,
            height: 90,
            ..Default::default()
        },
    );
    {
        let sc = proj.active_scene_mut().unwrap();
        let mut rect = TimelineItem::new(
            "Bg",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 120.0,
                height: 60.0,
                color: RgbaColor {
                    r: 0.75,
                    g: 0.45,
                    b: 0.20,
                    a: 1.0,
                },
            },
        );
        rect.transform.rotation_deg = 17.5;
        rect.blend_mode = BlendMode::Screen;
        sc.layers[0].items.push(rect);

        let matte = TimelineItem::new(
            "Matte",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 100.0,
                height: 70.0,
                color: RgbaColor::WHITE,
            },
        );
        sc.layers[1].items.push(matte);
        sc.layers[0].track_matte = Some(TrackMatte {
            source_layer_index: 1,
            mode: TrackMatteMode::Alpha,
        });

        sc.color_grade = SceneColorGrade {
            enabled: true,
            lut_path: Some("builtin:teal_orange".into()),
            lut_intensity: 0.7,
            contrast: 1.1,
            saturation: 1.05,
            ..SceneColorGrade::default()
        };
    }

    let folder = ProjectFolder::init(tmp.path().join("perf_proj"), &proj).unwrap();
    let mut rg = RenderGraph::new();
    rg.active_revision = 1;
    rg.set_project_folder(Some(folder.clone()));
    assert!(rg.enable_disk_frame_cache(folder.cache_frames_dir(), 32 * 1024 * 1024));

    let fake_asset = Uuid::new_v4();
    let fake_proxy = tmp.path().join("proxy.mp4");
    rg.register_proxy(fake_asset, fake_proxy.clone());

    let t = RationalTime::from_seconds(0.5);
    let first = rg
        .render_project_scene_cached_rgba8(&proj, proj.active_scene_id, t, 160, 90, false)
        .unwrap();
    // 同一フォルダ・同一プロキシ再登録はキャッシュを破棄しない
    rg.set_project_folder(Some(folder.clone()));
    rg.register_proxy(fake_asset, fake_proxy);
    let second = rg
        .render_project_scene_cached_rgba8(&proj, proj.active_scene_id, t, 160, 90, false)
        .unwrap();
    assert_eq!(first, second);
    assert!(rg.cache_hits >= 1);

    // LinearImage <-> Rgba16Float ＆ VideoScope LUT の整合性検証
    let mut img = LinearImage::new(
        64,
        36,
        LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.6,
            g: 0.3,
            b: 0.8,
            a: 0.9,
        }),
    );
    let mut overlay = LinearImage::new(
        32,
        18,
        LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.2,
            g: 0.8,
            b: 0.4,
            a: 0.75,
        }),
    );
    for mode in [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Multiply,
        BlendMode::Screen,
    ] {
        img.blend_with_mode(&overlay, 4, 4, 1.0, mode);
        img.blend_with_mode(&overlay, 8, 6, 0.5, mode);
    }
    let grade = SceneColorGrade {
        enabled: true,
        contrast: 1.05,
        saturation: 1.1,
        ..SceneColorGrade::default()
    };
    apply_scene_color_grade(&mut img, &grade, None);
    let scopes = compute_video_scopes(&img);
    assert!(scopes.avg_luma_ire > 5.0 && scopes.peak_luma_ire <= 100.1);

    // Rgba16Float 往復＆再利用バッファ書き込みが厳密一致すること
    let mut reused = Vec::new();
    img.write_rgba16f_bytes(&mut reused);
    let direct = img.to_rgba16f_bytes();
    assert_eq!(reused, direct);
    let restored = LinearImage::from_rgba16f_bytes(64, 36, &reused).unwrap();
    overlay.apply_track_matte(&restored, TrackMatteMode::LumaInverted);
    assert_eq!(restored.to_rgba8_srgb().len(), 64 * 36 * 4);
}
