//! Render Graph 統合単体テスト

use super::*;
use std::fs::File;
use tempfile::tempdir;
use ve_audio::WaveformBucket;
use ve_core::{
    Camera3D, CommandBus, CommandSource, EditCommand, FrameRate, PhonemeSegment, ProjectSettings,
    TrackMatte, TrackMatteMode, Transform3D,
};

fn write_solid_png(path: &std::path::Path, rgba: [u8; 4]) {
    let file = File::create(path).unwrap();
    let mut enc = png::Encoder::new(file, 8, 8);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    let data: Vec<u8> = rgba.into_iter().cycle().take(8 * 8 * 4).collect();
    w.write_image_data(&data).unwrap();
}

#[test]
fn filter_object_applies_to_immediate_layer_distinct_from_effect_chain() {
    let mut scene = Scene::new("Test", 16, 16, FrameRate::FPS_60);
    scene.layers[0].items.push(TimelineItem::new(
        "Rect",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 16.0,
            height: 16.0,
            color: RgbaColor::WHITE,
        },
    ));
    let mut params = BTreeMap::new();
    params.insert("gain".to_string(), 0.5);
    scene.layers[1].items.push(TimelineItem::new(
        "DimFilter",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::FilterObject {
            effect_type: "brightness".into(),
            scope: FilterTargetScope::ImmediateLayer,
            params,
        },
    ));

    let mut graph = RenderGraph::new();
    let frame = graph.render_scene(&scene, RationalTime::ZERO);
    assert!((frame.pixels[0].r - 0.5).abs() < 1e-4);
}

#[test]
fn track_matte_alpha_masks_target_layer() {
    let mut scene = Scene::new("MatteTest", 16, 16, FrameRate::FPS_60);
    scene.layers[0].items.push(TimelineItem::new(
        "FullWhite",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 16.0,
            height: 16.0,
            color: RgbaColor::WHITE,
        },
    ));
    scene.layers[1].items.push(TimelineItem::new(
        "LeftMatte",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 8.0,
            height: 16.0,
            color: RgbaColor::WHITE,
        },
    ));
    scene.layers[0].track_matte = Some(TrackMatte {
        mode: TrackMatteMode::Alpha,
        source_layer_index: 1,
    });

    let mut graph = RenderGraph::new();
    let frame = graph.render_scene(&scene, RationalTime::ZERO);
    assert!(frame.pixels[2].r > 0.9);
    assert!(frame.pixels[12].r < 0.05);
}

#[test]
fn group_control_offsets_targeted_layer() {
    let mut scene = Scene::new("GroupTest", 16, 16, FrameRate::FPS_60);
    scene.layers[0].items.push(TimelineItem::new(
        "Box",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 4.0,
            height: 4.0,
            color: RgbaColor::WHITE,
        },
    ));
    let mut grp = TimelineItem::new(
        "Group1",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::GroupControl {
            target_layer_count: 1,
        },
    );
    grp.transform.x = 8.0;
    scene.layers[1].items.push(grp);

    let mut graph = RenderGraph::new();
    let frame = graph.render_scene(&scene, RationalTime::ZERO);
    assert!(frame.pixels[0].r < 0.05);
    assert!(frame.pixels[8].r > 0.9);
}

#[test]
fn precomp_uses_hash_cache_and_invalidates_on_edit() {
    let tmp = tempdir().unwrap();
    let cache = PrecompCacheManager::new(tmp.path().join("precomp"), 1024 * 1024).unwrap();
    let settings = ProjectSettings {
        width: 16,
        height: 16,
        fps: FrameRate::FPS_60,
        ..Default::default()
    };
    let mut project = Project::new("CacheTest", settings);
    let mut bus = CommandBus::new();

    let rect = TimelineItem::new(
        "Rect",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 16.0,
            height: 16.0,
            color: RgbaColor::WHITE,
        },
    );
    let rect_id = rect.id;
    let precomp_scene_id = Uuid::new_v4();
    let precomp_item_id = Uuid::new_v4();

    bus.execute(
        &mut project,
        EditCommand::AddObject {
            layer_index: 0,
            item: rect,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();
    bus.execute(
        &mut project,
        EditCommand::PrecomposeSelection {
            item_ids: vec![rect_id],
            new_scene_id: precomp_scene_id,
            precomp_name: "Comp1".into(),
            precomp_item_id,
            target_layer_index: 0,
        },
        CommandSource::Gui,
        None,
    )
    .unwrap();

    let main_id = project.active_scene_id;
    let mut graph = RenderGraph::new().with_disk_cache(cache);

    let f1 = graph
        .render_project_scene(&project, main_id, RationalTime::ZERO, false)
        .unwrap();
    assert!(f1.pixels[0].r > 0.9);
    assert_eq!(graph.cache_misses, 1);
    assert_eq!(graph.cache_hits, 0);

    let _f2 = graph
        .render_project_scene(&project, main_id, RationalTime::ZERO, false)
        .unwrap();
    assert_eq!(graph.cache_hits, 1);

    project.scenes[1].layers[0].items[0].transform.opacity = 0.5;
    let f3 = graph
        .render_project_scene(&project, main_id, RationalTime::ZERO, false)
        .unwrap();
    assert_eq!(graph.cache_misses, 2);
    assert!((f3.pixels[0].r - 0.5).abs() < 0.05);
}

#[test]
fn character_sprite_switches_png_by_volume_and_vowel_lipsync() {
    let tmp = tempdir().unwrap();
    let closed_png = tmp.path().join("closed.png");
    let open_png = tmp.path().join("open.png");
    let a_png = tmp.path().join("a.png");
    // Closed = 青, Open = 赤, A = 緑
    write_solid_png(&closed_png, [0, 0, 255, 255]);
    write_solid_png(&open_png, [255, 0, 0, 255]);
    write_solid_png(&a_png, [0, 255, 0, 255]);

    let mut png_paths = BTreeMap::new();
    png_paths.insert(MouthShape::Closed, closed_png);
    png_paths.insert(MouthShape::Open, open_png);
    png_paths.insert(MouthShape::A, a_png);

    let audio_asset = Uuid::new_v4();
    let mut scene = Scene::new("LipSync", 32, 32, FrameRate::FPS_60);
    let mut audio_item = TimelineItem::new(
        "Voice",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Audio {
            asset_id: audio_asset,
            source_offset: RationalTime::ZERO,
            volume: 1.0,
        },
    );
    audio_item.phoneme_segments = vec![PhonemeSegment {
        start: RationalTime::from_seconds(0.5),
        end: RationalTime::from_seconds(1.5),
        phoneme: "a".into(),
        mouth: MouthShape::A,
    }];
    scene.layers[3].items.push(audio_item);

    // 1) VolumeRms モード検証: 0.0s..1.0s は無音 (rms=0.0)、1.0s..2.0s は発話 (rms=0.5)
    let mut graph = RenderGraph::new();
    graph.register_waveform(
        audio_asset,
        WaveformPeaks {
            sample_rate: 48_000,
            samples_per_bucket: 48_000,
            buckets: vec![
                WaveformBucket {
                    min: 0.0,
                    max: 0.0,
                    rms: 0.0,
                },
                WaveformBucket {
                    min: -0.5,
                    max: 0.5,
                    rms: 0.5,
                },
            ],
        },
    );

    let sprite_rms = TimelineItem::new(
        "Tachie",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::CharacterSprite {
            config: CharacterSpriteConfig {
                linked_audio_layer: Some(3),
                mode: LipSyncMode::VolumeRms {
                    closed_threshold: 0.05,
                    open_threshold: 0.2,
                },
                png_paths: png_paths.clone(),
                psd_config: None,
                phoneme_override: vec![],
            },
            width: 32.0,
            height: 32.0,
        },
    );
    scene.layers[0].items.push(sprite_rms);

    let silent_frame = graph.render_scene(&scene, RationalTime::from_seconds(0.2));
    assert!(silent_frame.pixels[0].b > 0.9); // Closed (青)
    let loud_frame = graph.render_scene(&scene, RationalTime::from_seconds(1.2));
    assert!(loud_frame.pixels[0].r > 0.9); // Open (赤)

    // 2) VowelPhoneme モード検証: 1.0s は母音 "a" (緑)
    if let ItemKind::CharacterSprite { config, .. } = &mut scene.layers[0].items[0].kind {
        config.mode = LipSyncMode::VowelPhoneme;
    }
    let vowel_frame = graph.render_scene(&scene, RationalTime::from_seconds(1.0));
    assert!(vowel_frame.pixels[0].g > 0.9); // A (緑)
}

#[test]
fn camera_3d_depth_sorts_closer_layer_in_front() {
    let mut scene = Scene::new("Cam3D", 32, 32, FrameRate::FPS_60);
    scene.camera = Camera3D {
        enabled: true,
        position: [0.0, 0.0, -1000.0],
        target: [0.0, 0.0, 0.0],
        fov_deg: 50.0,
        dof_enabled: false,
        focus_distance: 1000.0,
        aperture: 0.0,
        ..Camera3D::default()
    };

    // Layer 0 (下レイヤー) だが 3D 空間では手前 (z = -200) にある赤い四角
    let mut near_red = TimelineItem::new(
        "NearRed",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 32.0,
            height: 32.0,
            color: RgbaColor {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        },
    );
    near_red.transform_3d = Transform3D {
        enable_3d: true,
        z: -200.0,
        rotation_x_deg: 0.0,
        rotation_y_deg: 0.0,
    };
    scene.layers[0].items.push(near_red);

    // Layer 1 (上レイヤー) だが 3D 空間では奥 (z = +200) にある青い四角
    let mut far_blue = TimelineItem::new(
        "FarBlue",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 32.0,
            height: 32.0,
            color: RgbaColor {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            },
        },
    );
    far_blue.transform_3d = Transform3D {
        enable_3d: true,
        z: 200.0,
        rotation_x_deg: 0.0,
        rotation_y_deg: 0.0,
    };
    scene.layers[1].items.push(far_blue);

    let mut graph = RenderGraph::new();
    let frame = graph.render_scene(&scene, RationalTime::ZERO);
    let center = frame.pixels[16 * 32 + 16];
    // Zソートにより手前の赤が奥の青より前に描画される
    assert!(center.r > 0.9 && center.b < 0.1, "got {center:?}");
}

#[test]
fn plugin_effect_executes_via_render_graph() {
    let mut scene = Scene::new("PluginTest", 16, 16, FrameRate::FPS_60);
    let mut item = TimelineItem::new(
        "White",
        RationalTime::ZERO,
        RationalTime::from_seconds(1.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 16.0,
            height: 16.0,
            color: RgbaColor::WHITE,
        },
    );
    let mut params = BTreeMap::new();
    params.insert("strength".to_string(), 1.0);
    item.effect_chain.push(EffectNode {
        id: Uuid::new_v4(),
        effect_type: "plugin.vignette".to_string(),
        enabled: true,
        params,
    });
    scene.layers[0].items.push(item);

    let mut graph = RenderGraph::new();
    let frame = graph.render_scene(&scene, RationalTime::ZERO);
    assert!(frame.pixels[8 * 16 + 8].r > frame.pixels[0].r);
}

#[test]
fn sliding_window_box_blur_matches_naive_reference() {
    let (w, h) = (13u32, 11u32);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    for y in 0..h {
        for x in 0..w {
            img.pixels[(y * w + x) as usize] = LinearPremul {
                r: ((x * 7 + y * 3) % 11) as f32 / 10.0,
                g: ((x * 5 + y * 13) % 17) as f32 / 16.0,
                b: ((x + y * 2) % 9) as f32 / 8.0,
                a: 1.0,
            };
        }
    }
    let orig = img.clone();

    for radius in [1, 3, 6] {
        let mut fast = orig.clone();
        box_blur_separable(&mut fast, radius);

        // ナイーブ参照実装との厳密一致検証
        let denom = (radius * 2 + 1) as f64;
        let mut tmp = vec![LinearPremul::TRANSPARENT; orig.pixels.len()];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
                for dx in -radius..=radius {
                    let p = orig.pixels[(y * w as i32 + (x + dx).clamp(0, w as i32 - 1)) as usize];
                    r += f64::from(p.r);
                    g += f64::from(p.g);
                    b += f64::from(p.b);
                    a += f64::from(p.a);
                }
                tmp[(y * w as i32 + x) as usize] = LinearPremul {
                    r: (r / denom) as f32,
                    g: (g / denom) as f32,
                    b: (b / denom) as f32,
                    a: (a / denom) as f32,
                };
            }
        }
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
                for dy in -radius..=radius {
                    let p = tmp[((y + dy).clamp(0, h as i32 - 1) * w as i32 + x) as usize];
                    r += f64::from(p.r);
                    g += f64::from(p.g);
                    b += f64::from(p.b);
                    a += f64::from(p.a);
                }
                let got = fast.pixels[(y * w as i32 + x) as usize];
                assert!((got.r - (r / denom) as f32).abs() < 1e-5);
                assert!((got.g - (g / denom) as f32).abs() < 1e-5);
                assert!((got.b - (b / denom) as f32).abs() < 1e-5);
                assert!((got.a - (a / denom) as f32).abs() < 1e-5);
            }
        }
    }
}

#[test]
fn mem_precomp_cache_is_bounded_by_lru_cap() {
    let mut graph = RenderGraph::new();
    for i in 0..(MAX_MEM_PRECOMP_FRAMES + 10) {
        graph.insert_mem_precomp(
            format!("key_{i:04}"),
            LinearImage::new(2, 2, LinearPremul::TRANSPARENT),
        );
    }
    assert_eq!(graph.mem_precomp_cache_len(), MAX_MEM_PRECOMP_FRAMES);
}

#[test]
fn renders_image_and_video_assets_with_proxy_and_frame_cache() {
    use ve_core::{AssetRef, MediaKind};
    use ve_media::{ExportConfig, ExportFormat, VideoEncoder};

    let tmp = tempdir().unwrap();
    let png_path = tmp.path().join("overlay_red.png");
    write_solid_png(&png_path, [255, 0, 0, 255]);

    let cli = FfmpegCli::default();
    let write_solid_mp4 = |path: &std::path::Path, rgba: [u8; 4]| {
        let cfg = ExportConfig {
            width: 32,
            height: 32,
            fps: FrameRate::FPS_30,
            format: ExportFormat::Mp4H264,
            crf: 18,
            encoder: VideoEncoder::Software,
        };
        let mut session = cli.start_export(path, &cfg, None).unwrap();
        let frame: Vec<u8> = rgba.into_iter().cycle().take(32 * 32 * 4).collect();
        for _ in 0..10 {
            session.write_rgba8_frame(&frame).unwrap();
        }
        session.finish().unwrap();
    };

    let orig_mp4 = tmp.path().join("orig_blue.mp4");
    let proxy_mp4 = tmp.path().join("proxy_green.mp4");
    write_solid_mp4(&orig_mp4, [0, 0, 255, 255]);
    write_solid_mp4(&proxy_mp4, [0, 255, 0, 255]);

    let settings = ProjectSettings {
        width: 32,
        height: 32,
        fps: FrameRate::FPS_30,
        ..Default::default()
    };
    let mut project = Project::new("MediaRender", settings);
    let vid_id = Uuid::new_v4();
    let img_id = Uuid::new_v4();
    project.assets.push(AssetRef {
        id: vid_id,
        name: "orig_blue.mp4".into(),
        kind: MediaKind::Video,
        relative_path: None,
        fallback_absolute_path: Some(orig_mp4),
        collected: false,
    });
    project.assets.push(AssetRef {
        id: img_id,
        name: "overlay_red.png".into(),
        kind: MediaKind::Image,
        relative_path: None,
        fallback_absolute_path: Some(png_path),
        collected: false,
    });

    // Layer 0: フルサイズ動画、Layer 1: 左上 8x8 の赤い画像
    project.active_scene_mut().unwrap().layers[0]
        .items
        .push(TimelineItem::new(
            "VideoClip",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Video {
                asset_id: vid_id,
                source_offset: RationalTime::ZERO,
                volume: 1.0,
            },
        ));
    project.active_scene_mut().unwrap().layers[1]
        .items
        .push(TimelineItem::new(
            "ImageClip",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Image { asset_id: img_id },
        ));

    let main_id = project.active_scene_id;
    let mut graph = RenderGraph::new();

    // 1) 原本レンダリング: 左上(2,2)は赤いPNG、右下(24,24)は青い動画
    let f1 = graph
        .render_project_scene(&project, main_id, RationalTime::from_seconds(0.1), false)
        .unwrap();
    assert!(f1.pixels[2 * 32 + 2].r > 0.8);
    assert!(f1.pixels[24 * 32 + 24].b > 0.6);
    assert_eq!(graph.video_frame_cache_len(), 1);

    // 2) プロキシ登録＋プレビューモード (use_proxies = true) -> 緑のプロキシから描画
    graph.register_proxy(vid_id, proxy_mp4);
    let f_proxy = graph
        .render_project_scene(&project, main_id, RationalTime::from_seconds(0.1), false)
        .unwrap();
    assert!(f_proxy.pixels[24 * 32 + 24].g > 0.6);

    // 3) 書き出しモード (use_proxies = false) -> 青い原本へ自動切替
    graph.use_proxies = false;
    let f_export = graph
        .render_project_scene(&project, main_id, RationalTime::from_seconds(0.1), false)
        .unwrap();
    assert!(f_export.pixels[24 * 32 + 24].b > 0.6);
}

#[test]
fn scene_change_wipe_and_layer_offset_work() {
    let mut scene = Scene::new("TransTest", 32, 16, FrameRate::FPS_60);
    scene.layers[0].offset_x = 4.0;
    scene.layers[0].items.push(TimelineItem::new(
        "White",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 28.0,
            height: 16.0,
            color: RgbaColor::WHITE,
        },
    ));
    scene.layers[1].items.push(TimelineItem::new(
        "WipeLeft",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Transition {
            transition: TransitionKind::WipeLeft,
            scope: FilterTargetScope::ImmediateLayer,
            softness: 0.02,
        },
    ));

    let mut graph = RenderGraph::new();
    // t=1.0s => progress=0.5: x=0..4 is transparent due to layer.offset_x=4, x=6 is visible, x=28 is wiped out
    let frame = graph.render_scene_inner(None, &scene, RationalTime::from_seconds(1.0), true, 0);
    assert!(frame.pixels[2].a < 0.05);
    assert!(frame.pixels[6].r > 0.8);
    assert!(frame.pixels[28].a < 0.05);
}

#[test]
fn chroma_key_removes_green_and_preserves_red() {
    let mut img = LinearImage::new(8, 4, LinearPremul::TRANSPARENT);
    let green = LinearPremul::from_srgb_straight(RgbaColor {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    });
    let red = LinearPremul::from_srgb_straight(RgbaColor {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    });
    for y in 0..4 {
        for x in 0..8 {
            img.pixels[(y * 8 + x) as usize] = if x < 4 { green } else { red };
        }
    }
    let plugins = PluginRegistry::new();
    let params = BTreeMap::new();
    apply_named_effect(&mut img, "chroma_key", &params, 0.0, &plugins);
    assert!(img.pixels[1].a < 0.05, "green should be keyed out");
    assert!(img.pixels[6].a > 0.95 && img.pixels[6].r > 0.9);
}

#[test]
fn mosaic_and_rounded_clip_modify_pixels_cleanly() {
    let mut img = LinearImage::new(16, 16, LinearPremul::from_srgb_straight(RgbaColor::WHITE));
    let plugins = PluginRegistry::new();
    let mut rparams = BTreeMap::new();
    rparams.insert("radius".into(), 6.0);
    apply_named_effect(&mut img, "rounded_clip", &rparams, 0.0, &plugins);
    // Corner pixel (0,0) clipped transparent, center (8,8) remains opaque white
    assert!(img.pixels[0].a < 0.05);
    assert!(img.pixels[8 * 16 + 8].a > 0.95);

    let mut mparams = BTreeMap::new();
    mparams.insert("block_size".into(), 4.0);
    apply_named_effect(&mut img, "mosaic", &mparams, 0.0, &plugins);
    // All pixels inside top-left 4x4 block become identical
    assert_eq!(img.pixels[0], img.pixels[3 * 16 + 3]);
}

#[test]
fn rotate_linear_image_rotates_around_center_cleanly() {
    let mut img = LinearImage::new(20, 10, LinearPremul::TRANSPARENT);
    img.pixels.fill(LinearPremul::from_srgb_straight(RgbaColor {
        r: 1.0,
        g: 0.2,
        b: 0.1,
        a: 1.0,
    }));
    let rotated = rotate_linear_image(&img, 90.0);
    assert!(rotated.width >= 10 && rotated.height >= 20);
    let center =
        rotated.pixels[((rotated.height / 2) * rotated.width + rotated.width / 2) as usize];
    assert!(center.a > 0.9 && center.r > 0.9);
}

#[test]
fn scaled_project_scene_render_matches_full_composition_coords() {
    let mut proj = Project::new(
        "ScaledPreview",
        ve_core::ProjectSettings {
            width: 200,
            height: 100,
            ..Default::default()
        },
    );
    // 右側領域 (x=140..180, y=40..80) に赤い矩形を配置
    let mut box_item = TimelineItem::new(
        "RightBox",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 40.0,
            height: 40.0,
            color: RgbaColor {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        },
    );
    box_item.transform.x = 140.0;
    box_item.transform.y = 40.0;
    proj.active_scene_mut().unwrap().layers[0]
        .items
        .push(box_item);

    let mut rg = RenderGraph::new();
    // 1/2 スケール (100x50) で描画しても右側ボックスが (70..90, 20..40) に正しく収まる
    let scaled = rg
        .render_project_scene_scaled(
            &proj,
            proj.active_scene_id,
            RationalTime::ZERO,
            100,
            50,
            false,
        )
        .unwrap();
    assert_eq!((scaled.width, scaled.height), (100, 50));
    let px = scaled.pixels[(30 * 100 + 80) as usize];
    assert!(
        px.r > 0.9,
        "expected red pixel at scaled (80,30), got {px:?}"
    );
}

#[test]
fn renders_audio_spectrum_bars_and_audio_reactive_bounce() {
    use ve_audio::WaveformBucket;
    use ve_core::AudioReactiveConfig;

    let mut proj = Project::new(
        "AudioSpectrumTest",
        ve_core::ProjectSettings {
            width: 240,
            height: 160,
            ..Default::default()
        },
    );
    let audio_id = Uuid::new_v4();
    let sr = 48_000;
    // 0.0..1.0s 無音、1.0..2.0s 強い音声
    let mut buckets = Vec::new();
    for i in 0..200 {
        if i < 100 {
            buckets.push(WaveformBucket {
                min: 0.0,
                max: 0.0,
                rms: 0.0,
            });
        } else {
            buckets.push(WaveformBucket {
                min: -0.9,
                max: 0.9,
                rms: 0.7,
            });
        }
    }
    let peaks = WaveformPeaks {
        sample_rate: sr,
        samples_per_bucket: 480,
        buckets,
    };

    // Layer 3: Audio
    proj.active_scene_mut().unwrap().layers[3]
        .items
        .push(TimelineItem::new(
            "Beat",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Audio {
                asset_id: audio_id,
                source_offset: RationalTime::ZERO,
                volume: 1.0,
            },
        ));

    // Layer 0: AudioSpectrum
    let mut spec = TimelineItem::new(
        "Spectrum",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::AudioSpectrum {
            linked_audio_layer: 3,
            width: 160.0,
            height: 80.0,
            bar_count: 16,
            color: RgbaColor {
                r: 0.1,
                g: 0.9,
                b: 1.0,
                a: 1.0,
            },
            peak_color: RgbaColor {
                r: 1.0,
                g: 0.3,
                b: 0.6,
                a: 1.0,
            },
            sensitivity: 1.5,
        },
    );
    spec.transform.x = 10.0;
    spec.transform.y = 70.0;
    proj.active_scene_mut().unwrap().layers[0].items.push(spec);

    // Layer 1: Audio-reactive bouncing square at (185, 100), bounces up -45px on loud audio
    let mut box_item = TimelineItem::new(
        "Bounce",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 30.0,
            height: 30.0,
            color: RgbaColor {
                r: 1.0,
                g: 0.9,
                b: 0.1,
                a: 1.0,
            },
        },
    );
    box_item.transform.x = 185.0;
    box_item.transform.y = 100.0;
    box_item.audio_reactive = AudioReactiveConfig {
        enabled: true,
        linked_audio_layer: 3,
        band: AudioReactiveBand::Rms,
        sensitivity: 1.4,
        scale_amount: 0.2,
        y_bounce_px: -45.0,
        opacity_amount: 0.0,
    };
    proj.active_scene_mut().unwrap().layers[1]
        .items
        .push(box_item);

    let mut rg = RenderGraph::new();
    rg.register_waveform(audio_id, peaks);

    let quiet = rg
        .render_project_scene(
            &proj,
            proj.active_scene_id,
            RationalTime::from_seconds(0.3),
            false,
        )
        .unwrap();
    let loud = rg
        .render_project_scene(
            &proj,
            proj.active_scene_id,
            RationalTime::from_seconds(1.5),
            false,
        )
        .unwrap();

    // Count lit cyan/pink pixels in upper half of spectrum area (x=10..170, y=70..115)
    let count_spec_upper = |img: &LinearImage| -> usize {
        let mut n = 0;
        for y in 70..115 {
            for x in 10..170 {
                let p = img.pixels[(y * img.width + x) as usize];
                if p.g > 0.1 || p.b > 0.1 {
                    n += 1;
                }
            }
        }
        n
    };
    assert_eq!(count_spec_upper(&quiet), 0);
    assert!(
        count_spec_upper(&loud) > 200,
        "loud spectrum should draw tall bars: {}",
        count_spec_upper(&loud)
    );

    // Bouncing yellow box shifts upward on loud frame (y ~ 56..92 instead of 100..130)
    let quiet_upper_px = quiet.pixels[(72 * quiet.width + 200) as usize];
    let loud_upper_px = loud.pixels[(72 * loud.width + 200) as usize];
    assert!(quiet_upper_px.r < 0.05);
    assert!(
        loud_upper_px.r > 0.8 && loud_upper_px.g > 0.6,
        "expected bounced yellow box at (200,72), got {loud_upper_px:?}"
    );
}

#[test]
fn v09_expressions_shutter_motion_blur_and_3d_lighting_render_cleanly() {
    let mut proj = Project::new("v09_render_test", ProjectSettings::default());
    let scene = proj.active_scene_mut().unwrap();
    scene.width = 200;
    scene.height = 120;
    scene.fps = FrameRate::FPS_30;
    scene.motion_blur.enabled = true;
    scene.motion_blur.shutter_angle_deg = 360.0;
    scene.motion_blur.preview_samples = 5;

    let mut box_item = TimelineItem::new(
        "MovingBox",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 20.0,
            height: 20.0,
            color: RgbaColor::WHITE,
        },
    );
    box_item.transform.y = 50.0;
    // 1秒あたり +600px 移動するエクスプレッション（1フレーム30fpsで20px移動）
    box_item.expressions.insert("x".into(), "time * 600".into());
    scene.layers[0].items.push(box_item);

    let mut graph = RenderGraph::new();
    let blurred = graph
        .render_project_scene(
            &proj,
            proj.active_scene_id,
            RationalTime::from_seconds(0.1),
            false,
        )
        .unwrap();

    // モーションブラーありでは水平方向に中間輝度（0.1 < r < 0.9）の残像ピクセルが生じる
    let mid_trail = (0..blurred.width)
        .filter(|&x| {
            let p = blurred.pixels[(60 * blurred.width + x) as usize];
            p.r > 0.08 && p.r < 0.92
        })
        .count();
    assert!(
        mid_trail >= 4,
        "expected motion blur trail pixels, got {mid_trail}"
    );

    // 3D カメラ＋ライト＆シャドウキャッチャーでも正常描画される
    let scene2 = proj.active_scene_mut().unwrap();
    scene2.motion_blur.enabled = false;
    scene2.camera.enabled = true;
    scene2.camera.light.enabled = true;
    scene2.camera.light.cast_shadow = true;
    scene2.layers[0].items[0].transform_3d.enable_3d = true;
    scene2.layers[0].items[0].transform_3d.rotation_y_deg = 45.0;
    let lit = graph
        .render_project_scene(
            &proj,
            proj.active_scene_id,
            RationalTime::from_seconds(0.1),
            false,
        )
        .unwrap();
    assert!(lit.pixels.iter().any(|p| p.r > 0.1));
}

#[test]
fn v10_persistent_frame_cache_persists_across_render_graph_instances() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().join("frames");

    let mut proj = Project::new("DiskCacheTest", ProjectSettings::default());
    let scene = proj.active_scene_mut().unwrap();
    scene.width = 64;
    scene.height = 36;
    scene.layers[0].items.push(TimelineItem::new(
        "Box",
        RationalTime::ZERO,
        RationalTime::from_seconds(2.0),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 32.0,
            height: 18.0,
            color: RgbaColor {
                r: 0.2,
                g: 0.8,
                b: 0.4,
                a: 1.0,
            },
        },
    ));

    let t = RationalTime::from_seconds(0.5);
    let first = {
        let mut g = RenderGraph::new();
        assert!(g.enable_disk_frame_cache(cache_dir.clone(), 4 * 1024 * 1024));
        let rgba = g
            .render_project_scene_cached_rgba8(&proj, proj.active_scene_id, t, 64, 36, false)
            .unwrap();
        assert_eq!(g.cache_misses, 1);
        assert_eq!(g.cache_hits, 0);
        rgba
    };

    // 再起動相当の新規 RenderGraph でも即キャッシュヒットし、同一ピクセルが返る
    let mut g2 = RenderGraph::new();
    assert!(g2.enable_disk_frame_cache(cache_dir, 4 * 1024 * 1024));
    let second = g2
        .render_project_scene_cached_rgba8(&proj, proj.active_scene_id, t, 64, 36, false)
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(g2.cache_hits, 1);
    let stats = g2.disk_frame_cache_stats().unwrap();
    assert_eq!(stats.0, 1);
    assert_eq!(stats.2, 1);
}
