//! v1.0 #20: ゴールデン視覚回帰テストスイート
//! 描画エンジン（Linear Premultiplied 合成・ベクターシェイプ＆トリムパス・パーティクル・
//! エクスプレッション・.cube 3D LUT・並列バッチ描画）の決定論的出力と品質不変条件を継続的に検証する。

use std::collections::HashMap;
use ve_core::{
    BlendMode, ItemKind, ParticleConfig, Project, ProjectSettings, RationalTime, RgbaColor,
    SceneColorGrade, ShapeKind, TimelineItem, VectorShapeConfig,
};
use ve_media::HwDecodeBackend;
use ve_render::{RenderGraph, render_frame_batch_parallel};

fn build_golden_test_project() -> Project {
    let mut proj = Project::new("GoldenVisualRegression", ProjectSettings::default());
    let scene = proj.active_scene_mut().unwrap();
    scene.width = 160;
    scene.height = 90;

    // Layer 0: 背景青矩形
    let mut bg = TimelineItem::new(
        "BgRect",
        RationalTime::ZERO,
        RationalTime::from_seconds(4.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 160.0,
            height: 90.0,
            color: RgbaColor {
                r: 0.10,
                g: 0.20,
                b: 0.45,
                a: 1.0,
            },
        },
    );
    bg.transform.x = 0.0;
    bg.transform.y = 0.0;
    scene.layers[0].items.push(bg);

    // Layer 1: エクスプレッション駆動の発光円（Add ブレンド）
    let mut orb = TimelineItem::new(
        "ExprOrb",
        RationalTime::ZERO,
        RationalTime::from_seconds(4.0),
        ItemKind::Shape {
            shape: ShapeKind::Ellipse,
            width: 40.0,
            height: 40.0,
            color: RgbaColor {
                r: 0.95,
                g: 0.30,
                b: 0.15,
                a: 0.9,
            },
        },
    );
    orb.transform.x = 30.0;
    orb.transform.y = 25.0;
    orb.blend_mode = BlendMode::Add;
    orb.expressions
        .insert("x".into(), "30 + sin(time * 3.14159) * 20".into());
    scene.layers[1].items.push(orb);

    // Layer 2: フリーベジェベクターシェイプ＋トリムパス
    let mut vec_cfg = VectorShapeConfig::star();
    vec_cfg.width = 70.0;
    vec_cfg.height = 70.0;
    vec_cfg.stroke_width = 3.5;
    vec_cfg.trim_end = 0.85;
    let mut vec_item = TimelineItem::new(
        "TrimBezierStar",
        RationalTime::ZERO,
        RationalTime::from_seconds(4.0),
        ItemKind::VectorShape { config: vec_cfg },
    );
    vec_item.transform.x = 45.0;
    vec_item.transform.y = 10.0;
    scene.layers[2].items.push(vec_item);

    // Layer 3: 決定論的シードパーティクル
    let emitter = ParticleConfig {
        seed: 2026,
        count: 45,
        size_px: 5.0,
        ..ParticleConfig::default()
    };
    let mut part = TimelineItem::new(
        "GoldenParticles",
        RationalTime::ZERO,
        RationalTime::from_seconds(4.0),
        ItemKind::ParticleEmitter { config: emitter },
    );
    part.transform.x = 80.0;
    part.transform.y = 45.0;
    scene.layers[3].items.push(part);

    // シーンカラーグレーディング（組み込み 3D LUT + コントラスト）
    scene.color_grade = SceneColorGrade {
        enabled: true,
        lut_path: Some("builtin:teal_orange".into()),
        lut_intensity: 0.6,
        contrast: 1.08,
        saturation: 1.1,
        ..SceneColorGrade::default()
    };
    proj
}

#[test]
fn golden_visual_regression_determinism_and_channel_invariants() {
    let proj = build_golden_test_project();
    let t = RationalTime::from_seconds(1.0);

    // 1. 決定論性検証: 異なる RenderGraph インスタンスが 1 バイトの誤差もなく同一 RGBA8 を出力する
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let img1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap();
    let img2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap();
    let rgba1 = img1.to_rgba8_srgb();
    let rgba2 = img2.to_rgba8_srgb();
    assert_eq!(rgba1, rgba2, "renderer must be bit-for-bit deterministic");
    assert_eq!(rgba1.len(), 160 * 90 * 4);

    // 2. 色・輝度・コントラスト不変条件: 背景青・中央の発光/ストローク/パーティクルが描画されていること
    let corner_px = &rgba1[0..4];
    assert!(
        corner_px[2] > corner_px[0] && corner_px[3] == 255,
        "corner background should stay cool blue opaque: {corner_px:?}"
    );
    let bright_pixels = rgba1
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| u16::from(px[0]) + u16::from(px[1]) > 240)
        .count();
    assert!(
        bright_pixels >= 100,
        "expected orb + bezier star + particles to produce >=100 bright pixels, got {bright_pixels}"
    );

    // 3. 時系列アニメーション検証: t=0.5s と t=1.0s で確実に画面が変化していること
    let rgba_early = g1
        .render_project_scene(
            &proj,
            proj.active_scene_id,
            RationalTime::from_seconds(0.5),
            false,
        )
        .unwrap()
        .to_rgba8_srgb();
    let diff_bytes = rgba1
        .iter()
        .zip(rgba_early.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        diff_bytes > 400,
        "animated expression & particles must visibly change across frames: diff={diff_bytes}"
    );
}

#[test]
fn golden_parallel_batch_render_matches_sequential_bit_for_bit() {
    let proj = build_golden_test_project();
    let times: Vec<RationalTime> = (0..8)
        .map(|f| RationalTime::from_seconds(f as f64 / 30.0))
        .collect();

    let mut seq_graph = RenderGraph::new();
    let seq_frames: Vec<Vec<u8>> = times
        .iter()
        .map(|&t| {
            seq_graph
                .render_project_scene(&proj, proj.active_scene_id, t, false)
                .unwrap()
                .to_rgba8_srgb()
        })
        .collect();

    let par_frames = render_frame_batch_parallel(
        &proj,
        proj.active_scene_id,
        None,
        &times,
        false,
        false,
        false,
        HwDecodeBackend::Software,
        &HashMap::new(),
        &HashMap::new(),
        4,
    );

    assert_eq!(par_frames.len(), seq_frames.len());
    for (idx, (seq, par)) in seq_frames.iter().zip(par_frames.iter()).enumerate() {
        assert_eq!(
            seq, par,
            "parallel frame {idx} diverged from sequential golden frame"
        );
    }
}

#[test]
fn golden_v20_node_graph_and_ai_vision_determinism() {
    let mut proj = build_golden_test_project();
    let t = RationalTime::from_seconds(1.0);

    // 1. レイヤーレンダリングと NodeCompositeGraph::from_scene_layers の視覚一致を検証
    let mut g_layers = RenderGraph::new();
    let layer_rgba = g_layers
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap()
        .to_rgba8_srgb();

    {
        let scene = proj.active_scene_mut().unwrap();
        scene.node_graph = ve_core::NodeCompositeGraph::from_scene_layers(scene);
    }
    let mut g_nodes = RenderGraph::new();
    let node_rgba_1 = g_nodes
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap()
        .to_rgba8_srgb();
    let node_rgba_2 = RenderGraph::new()
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap()
        .to_rgba8_srgb();

    assert_eq!(
        node_rgba_1, node_rgba_2,
        "node graph rendering must be bit-for-bit deterministic"
    );
    assert_eq!(
        layer_rgba, node_rgba_1,
        "NodeCompositeGraph::from_scene_layers must reproduce layer composition bit-for-bit"
    );
}

#[test]
fn golden_v21_curves_hsl_corner_pin_and_kinetic_subtitles() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        scene.color_grade.enabled = true;
        scene.color_grade.curves = ve_core::ColorCurves::preset_warm_cinema();
        scene.color_grade.hsl_qualifier = ve_core::HslQualifier {
            enabled: true,
            hue_center_deg: 200.0,
            hue_width_deg: 40.0,
            sat_scale: 1.25,
            ..ve_core::HslQualifier::default()
        };
        if let Some(item) = scene.layers[0].items.first_mut() {
            item.corner_pin = ve_core::CornerPinConfig {
                enabled: true,
                tl: [0.15, 0.12],
                tr: [0.85, 0.18],
                br: [0.92, 0.88],
                bl: [0.08, 0.86],
                feather_px: 1.5,
            };
        }
        let mut sub = TimelineItem::new(
            "Kinetic Sub",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Text {
                content: "今日は 最高の 動画編集 です".into(),
                font_family: "Sans".into(),
                font_size: 28.0,
                color: RgbaColor::WHITE,
                animator: ve_core::TextAnimator::default(),
            },
        );
        sub.kinetic_subtitle.enabled = true;
        sub.transform.x = 20.0;
        sub.transform.y = 110.0;
        scene.layers[2].items.push(sub);
    }

    let t1 = RationalTime::from_seconds(0.4);
    let t2 = RationalTime::from_seconds(1.5);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let a1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap()
        .to_rgba8_srgb();
    let a2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap()
        .to_rgba8_srgb();
    assert_eq!(a1, a2, "v2.1 rendering must be bit-for-bit deterministic");

    let later = g1
        .render_project_scene(&proj, proj.active_scene_id, t2, false)
        .unwrap()
        .to_rgba8_srgb();
    let changed = a1.iter().zip(later.iter()).filter(|(x, y)| x != y).count();
    assert!(
        changed > 200,
        "v2.1 kinetic subtitles + motion must visibly progress over time: changed={changed}"
    );
}
