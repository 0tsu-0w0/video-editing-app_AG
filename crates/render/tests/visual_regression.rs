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

#[test]
fn golden_v30_mesh3d_pbr_and_gpu_compute_determinism() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        let mut cube_item = TimelineItem::new(
            "PBR Torus",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Mesh3D {
                config: ve_core::Mesh3DConfig {
                    preset: ve_core::Mesh3DPreset::Torus,
                    width: 120.0,
                    height: 120.0,
                    metallic: 0.75,
                    roughness: 0.22,
                    auto_spin_deg_per_sec: 90.0,
                    ..ve_core::Mesh3DConfig::default()
                },
            },
        );
        cube_item.transform.x = 100.0;
        cube_item.transform.y = 30.0;
        scene.layers[2].items.push(cube_item);
    }

    let t0 = RationalTime::from_seconds(0.2);
    let t1 = RationalTime::from_seconds(1.1);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let r1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    let r2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    assert_eq!(
        r1, r2,
        "v3.0 PBR Mesh3D rendering must be bit-for-bit deterministic"
    );

    let later = g1
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap()
        .to_rgba8_srgb();
    let diff = r1.iter().zip(later.iter()).filter(|(a, b)| a != b).count();
    assert!(
        diff > 400,
        "v3.0 spinning 3D PBR mesh must visibly animate across frames: diff={diff}"
    );

    let caps = ve_render::detect_gpu_v30_capabilities();
    assert!(caps.color_shader_valid && caps.scopes_shader_valid && caps.composite_shader_valid);
}

#[test]
fn golden_v40_acescg_generative_bg_and_physics_determinism() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        let bg = TimelineItem::new(
            "Aurora BG",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::GenerativeBg {
                config: ve_core::GenerativeBgConfig {
                    preset: ve_core::GenerativeBgPreset::AuroraBorealis,
                    width: scene.width as f32,
                    height: scene.height as f32,
                    ..ve_core::GenerativeBgPreset::for_preset(
                        ve_core::GenerativeBgPreset::AuroraBorealis,
                    )
                },
            },
        );
        scene.layers[0].items.insert(0, bg);
        scene.color_management = ve_core::ColorManagementConfig {
            enabled: true,
            tone_mapper: ve_core::ToneMapper::AgX,
            exposure_stops: 0.35,
            ..ve_core::ColorManagementConfig::default()
        };
    }

    let t0 = RationalTime::from_seconds(0.3);
    let t1 = RationalTime::from_seconds(1.4);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let r1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    let r2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    assert_eq!(
        r1, r2,
        "v4.0 ACEScg + GenerativeBg rendering must be bit-for-bit deterministic"
    );

    let later = g1
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap()
        .to_rgba8_srgb();
    let diff = r1.iter().zip(later.iter()).filter(|(a, b)| a != b).count();
    assert!(
        diff > 400,
        "v4.0 GenerativeBg aurora must animate across frames: diff={diff}"
    );

    let (best_t, score, hero) =
        ve_render::select_best_thumbnail_frame(&mut g1, &proj, proj.active_scene_id, 5)
            .expect("hero thumbnail should succeed");
    assert!(best_t >= 0.0 && score > 1.0);
    assert_eq!((hero.width, hero.height), (1280, 720));
}

#[test]
fn golden_v50_rigged_puppet_speed_ramp_and_cinema_finishing_determinism() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        let mut puppet = TimelineItem::new(
            "HeroPuppet",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::RiggedPuppet {
                config: Box::new(ve_core::Rig2DConfig::default_humanoid()),
            },
        );
        puppet.transform.x = 60.0;
        puppet.transform.y = 10.0;
        puppet.speed_ramp = ve_core::SpeedRampPreset::BulletTimeDip.default_config();
        scene.layers[2].items.push(puppet);
        scene.cinema_finishing = ve_core::CinemaFinishingConfig {
            burn_in_timecode: true,
            watermark_text: "V5 GOLDEN".into(),
            ..ve_core::CinemaFinishingConfig::preset_kodak_35mm()
        };
    }

    let t0 = RationalTime::from_seconds(0.2);
    let t1 = RationalTime::from_seconds(1.2);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let r1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    let r2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    assert_eq!(
        r1, r2,
        "v5.0 RiggedPuppet + CinemaFinishing rendering must be bit-for-bit deterministic"
    );

    let later = g1
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap()
        .to_rgba8_srgb();
    let diff = r1.iter().zip(later.iter()).filter(|(a, b)| a != b).count();
    assert!(
        diff > 400,
        "v5.0 rigged puppet + grain + timecode burn-in must progress over time: diff={diff}"
    );
}

#[test]
fn golden_v60_data_chart_and_vfx_paint_determinism() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        let mut chart = TimelineItem::new(
            "RevenueChart",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::DataChart {
                config: Box::new(ve_core::DataChartConfig::for_kind(
                    ve_core::ChartKind::BarChart,
                )),
            },
        );
        chart.transform.x = 20.0;
        chart.transform.y = 10.0;
        chart.vfx_paint = ve_core::VfxPaintConfig {
            enabled: true,
            animate_write_on: true,
            strokes: vec![ve_core::PaintStroke::brush_glow(
                vec![[0.12, 0.78], [0.88, 0.24]],
                6.0,
                ve_core::RgbaColor {
                    r: 1.0,
                    g: 0.35,
                    b: 0.55,
                    a: 1.0,
                },
            )],
        };
        scene.layers[2].items.push(chart);
    }

    let t0 = RationalTime::from_seconds(0.25);
    let t1 = RationalTime::from_seconds(1.5);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let r1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    let r2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    assert_eq!(
        r1, r2,
        "v6.0 DataChart + VfxPaint rendering must be bit-for-bit deterministic"
    );

    let later = g1
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap()
        .to_rgba8_srgb();
    let diff = r1.iter().zip(later.iter()).filter(|(a, b)| a != b).count();
    assert!(
        diff > 300,
        "v6.0 DataChart bar growth + Write-On VFX paint must progress over time: diff={diff}"
    );
}

#[test]
fn golden_v70_frame_pool_reuse_interval_tree_and_ab_split_determinism() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        // 120 クリップ追加して RenderGraph の $O(\log N)$ 区間木パス (>= 64 items) を発火させる
        for i in 0..120 {
            let mut bar = TimelineItem::new(
                format!("IdxClip_{i}"),
                RationalTime::from_seconds(i as f64 * 0.2),
                RationalTime::from_seconds(0.35),
                ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: 24.0,
                    height: 16.0,
                    color: RgbaColor {
                        r: 0.25,
                        g: 0.85,
                        b: 0.65,
                        a: 0.9,
                    },
                },
            );
            bar.transform.x = ((i * 11) % 120) as f32;
            bar.transform.y = ((i * 7) % 60) as f32;
            scene.layers[1].items.push(bar);
        }
        scene.ab_compare = ve_core::AbCompareConfig {
            mode: ve_core::AbCompareMode::VerticalSplit,
            reference: ve_core::AbReferenceSource::UngradedBypass,
            split_ratio: 0.5,
            show_divider: true,
            ..ve_core::AbCompareConfig::default()
        };
    }

    let t = RationalTime::from_seconds(1.1);
    let mut g_linear = RenderGraph::new(); // active_revision == 0 -> 線形走査
    let mut g_indexed = RenderGraph::new();
    g_indexed.active_revision = 42; // active_revision > 0 -> 区間木 $O(\log N)$ パス

    let out_lin = g_linear
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap()
        .to_rgba8_srgb();
    let out_idx1 = g_indexed
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap()
        .to_rgba8_srgb();
    let out_idx2 = g_indexed
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap()
        .to_rgba8_srgb();

    // 1. 区間木パスと線形走査パス、および 2 回目のフレームプール再利用描画が 1 バイトの誤差もなく完全一致すること
    assert_eq!(out_lin, out_idx1);
    assert_eq!(out_idx1, out_idx2);
    assert!(
        g_indexed.frame_pool.metrics(0).pool_reuses >= 4,
        "2nd render pass must reuse pooled layer canvases"
    );

    // 2. A/B スプリット比較ビュー (Before Ungraded vs After Graded) が決定論的かつ左右でグレーディング差を可視化すること
    let ab1 = ve_render::render_ab_compare_scene(
        &mut g_indexed,
        &proj,
        proj.active_scene_id,
        t,
        160,
        90,
        None,
    )
    .unwrap()
    .to_rgba8_srgb();
    let ab2 = ve_render::render_ab_compare_scene(
        &mut g_linear,
        &proj,
        proj.active_scene_id,
        t,
        160,
        90,
        None,
    )
    .unwrap()
    .to_rgba8_srgb();
    assert_eq!(ab1, ab2);
    assert_ne!(
        ab1, out_idx1,
        "A/B split must differ from full graded-only frame"
    );
}

#[test]
fn golden_v80_planar_roto_svg_morph_optical_vfx_and_adr_determinism() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        let mut svg_item = TimelineItem::new(
            "SvgMorphLogo",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::SvgMorph {
                config: Box::new(ve_core::SvgMorphConfig {
                    width: 110.0,
                    height: 110.0,
                    path_a_d: ve_core::SvgShapePreset::LightningBolt
                        .svg_path_d()
                        .to_string(),
                    path_b_d: ve_core::SvgShapePreset::Heart.svg_path_d().to_string(),
                    morph_progress: 0.4,
                    auto_morph_hz: 0.5,
                    ..ve_core::SvgMorphConfig::default()
                }),
            },
        );
        svg_item.transform.x = 24.0;
        svg_item.transform.y = 8.0;
        svg_item.warp_displace = ve_core::WarpDisplaceConfig {
            enabled: true,
            preset: ve_core::WarpPreset::FlagWave,
            amount: 0.35,
            ..ve_core::WarpDisplaceConfig::default()
        };
        svg_item.roto_brush = ve_core::RotoBrushConfig {
            enabled: true,
            keyframes: vec![ve_core::RotoKeyframe {
                time: RationalTime::ZERO,
                vertices: ve_core::RotoBrushConfig::default_ellipse_vertices(
                    0.5, 0.5, 0.42, 0.42, 8,
                ),
                feather_px: 2.0,
                expansion_px: 0.0,
            }],
            ..ve_core::RotoBrushConfig::default()
        };
        scene.layers[1].items.push(svg_item);

        scene.optical_vfx =
            ve_core::OpticalVfxConfig::from_preset(ve_core::OpticalVfxPreset::VintagePrismLens);
        scene.adr_session.enabled = true;
        scene.adr_session.cues.push(ve_core::AdrCue::new(
            1,
            "Hero",
            "ここだ！",
            RationalTime::from_seconds(3.0),
            RationalTime::from_seconds(1.5),
        ));
    }

    let t0 = RationalTime::from_seconds(0.5);
    let t1 = RationalTime::from_seconds(1.2);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let f1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    let f2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap()
        .to_rgba8_srgb();
    assert_eq!(
        f1, f2,
        "v8.0 SvgMorph + Warp + RotoBrush + OpticalVFX + ADR must be bit-for-bit deterministic"
    );

    let f_later = g1
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap()
        .to_rgba8_srgb();
    let diff = f1
        .iter()
        .zip(f_later.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        diff > 200,
        "v8.0 SVG morph + flag warp + ADR streamer must animate over time: diff={diff}"
    );
}

#[test]
fn golden_v90_vtuber_mesh_verlet_and_live_stinger_determinism() {
    let mut proj = build_golden_test_project();
    {
        let scene = proj.active_scene_mut().unwrap();
        let mut vt = TimelineItem::new(
            "VtuberHero",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::VtuberAvatar {
                config: Box::new(ve_core::VtuberMeshConfig {
                    width: 110.0,
                    height: 110.0,
                    preset: ve_core::VtuberAvatarPreset::CyberFoxViolet,
                    blend_shapes: ve_core::VtuberBlendShapes {
                        head_yaw_deg: 22.0,
                        mouth_open: 0.65,
                        mouth_smile: 0.7,
                        ..ve_core::VtuberBlendShapes::default()
                    },
                    show_wireframe: true,
                    ..ve_core::VtuberMeshConfig::default()
                }),
            },
        );
        vt.transform.x = 80.0;
        vt.transform.y = 48.0;
        scene.layers[1].items.push(vt);
        scene.live_switcher.enabled = true;
        scene.live_switcher.recording_active = true;
        scene.live_switcher.show_tally_hud = true;
    }

    let t0 = RationalTime::from_seconds(0.4);
    let t1 = RationalTime::from_seconds(1.1);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let f1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap();
    let f2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t0, false)
        .unwrap();
    assert_eq!(
        f1.to_rgba8_srgb(),
        f2.to_rgba8_srgb(),
        "v9.0 VTuber 2D mesh + Verlet physics + Live tally HUD must be bit-for-bit deterministic"
    );

    let f_later = g1
        .render_project_scene(&proj, proj.active_scene_id, t1, false)
        .unwrap();
    let stinger = ve_render::render_live_stinger_transition(
        &f1,
        &f_later,
        &ve_core::StingerConfig::default(),
        0.5,
    );
    assert_ne!(
        stinger.to_rgba8_srgb(),
        f1.to_rgba8_srgb(),
        "Live stinger diagonal wipe must visibly transform frame at progress=0.5"
    );
}

#[test]
fn golden_v100_spatial_360_fluid_cloth_ibl_and_mocap_determinism() {
    let mut proj = Project::new(
        "GoldenV100",
        ProjectSettings {
            width: 160,
            height: 96,
            fps: ve_core::FrameRate::FPS_30,
            ..ProjectSettings::default()
        },
    );
    {
        let scene = proj.active_scene_mut().unwrap();
        scene.lighting_3d_adv = ve_core::Lighting3DAdvConfig {
            shadow_map_enabled: true,
            shadow_map_size: 96,
            hdri_ibl_enabled: true,
            hdri_preset: ve_core::HdriEnvPreset::GoldenSunset,
            ground_catcher_enabled: true,
            ..ve_core::Lighting3DAdvConfig::default()
        };

        let mut fluid = TimelineItem::new(
            "FluidFlame",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::FluidSim {
                config: Box::new(ve_core::FluidSimConfig {
                    width: 84.0,
                    height: 84.0,
                    grid_w: 28,
                    grid_h: 28,
                    ..ve_core::FluidSimConfig::from_preset(ve_core::FluidPreset::BonfireFlame)
                }),
            },
        );
        fluid.transform.x = 42.0;
        fluid.transform.y = 48.0;
        scene.layers[0].items.push(fluid);

        let mut cloth = TimelineItem::new(
            "ClothBanner",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::ClothSim {
                config: Box::new(ve_core::ClothSimConfig {
                    width: 80.0,
                    height: 60.0,
                    cols: 8,
                    rows: 6,
                    ..ve_core::ClothSimConfig::from_preset(ve_core::ClothPreset::CyberFlag)
                }),
            },
        );
        cloth.transform.x = 118.0;
        cloth.transform.y = 48.0;
        scene.layers[1].items.push(cloth);

        let mut mesh = TimelineItem::new(
            "CubeShadowIBL",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Mesh3D {
                config: ve_core::Mesh3DConfig {
                    width: 64.0,
                    height: 64.0,
                    ..ve_core::Mesh3DConfig::default()
                },
            },
        );
        mesh.transform.x = 80.0;
        mesh.transform.y = 48.0;
        scene.layers[2].items.push(mesh);

        scene.spatial_video =
            ve_core::SpatialVideoConfig::from_preset(ve_core::Spatial360Preset::Anaglyph3DReview);
    }

    let t = RationalTime::from_seconds(0.8);
    let mut g1 = RenderGraph::new();
    let mut g2 = RenderGraph::new();
    let f1 = g1
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap();
    let f2 = g2
        .render_project_scene(&proj, proj.active_scene_id, t, false)
        .unwrap();
    let rgba1 = f1.to_rgba8_srgb();
    assert_eq!(
        rgba1,
        f2.to_rgba8_srgb(),
        "v10.0 fluid + cloth + 3D shadow/IBL + spatial 3D must be bit-for-bit deterministic"
    );

    // レンダリング結果からシルエット 2D 姿勢抽出も決定論的に動作する
    let pose = ve_core::extract_silhouette_pose_from_rgba8(&rgba1, f1.width, f1.height, 0.08, 0.8);
    assert!(pose.confidence > 0.1);
}
