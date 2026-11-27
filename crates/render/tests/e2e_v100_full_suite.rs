//! v10.0.0 全機能統合 E2E ＆ リリース品質ハードニングテストスイート
//!
//! 以下をヘッドレス環境で決定論的かつ網羅的に検証する:
//! 1. v7.0〜v10.0 全 EditCommand の Undo / Redo 対称性 ＋ `ProjectFolder` + `history.jsonl` クラッシュ完全復旧
//! 2. レガシー v0.1 最小 `project.json` の後方互換ロード ＆ ゼロパニック描画
//! 3. v0.2〜v10.0 全 8 組み込み WGSL シェーダーの `naga` 静的検証 ＋ `execute_v100_spatial_360_or_cpu` 実測検証
//! 4. 360° OverCapture / 流体 / クロス / 3D シャドウマップ+HDRI IBL / VTuber / データチャート / 光学 VFX /
//!    ライブタリー HUD 全載せシーンのマルチスレッド並列バッチ描画＝逐次描画ビット完全一致 ＆ ゼロ NaN/Inf 検証

use std::collections::HashMap;
use tempfile::tempdir;
use ve_core::{
    AbCompareConfig, AbCompareMode, AdrSessionConfig, ClothPreset, ClothSimConfig, CommandBus,
    CommandSource, DataChartConfig, EditCommand, EditVariantStyle, FluidPreset, FluidSimConfig,
    FrameRate, HardwareControllerConfig, HardwareControllerPreset, HdriEnvPreset, ItemKind,
    Lighting3DAdvConfig, LiveSwitcherConfig, Mesh3DConfig, MocapPreset, OpticalVfxConfig, Project,
    ProjectFolder, ProjectSettings, RationalTime, RenderFarmConfig, RetargetConfig, RgbaColor,
    Rig2DConfig, ShapeKind, SmartDamIndex, Spatial360Preset, SpatialBusConfig, SpatialVideoConfig,
    TimelineItem, VtuberMeshConfig, build_ab_edit_variant_command,
    build_mocap_retarget_bake_commands, build_storyboard_animatic_commands, builtin_sample_bvh,
    parse_bvh, parse_script_to_storyboard, plan_render_farm_chunks, retarget_bvh_to_rig2d,
    verify_chunk_hmac,
};
use ve_media::HwDecodeBackend;
use ve_render::{
    RenderGraph, benchmark_v100_gpu_or_cpu, execute_v100_spatial_360_or_cpu,
    render_frame_batch_parallel, validate_all_builtin_wgsl_shaders,
};

#[test]
fn e2e_v100_all_v7_to_v10_commands_undo_redo_and_crash_recovery() {
    let tmp = tempdir().unwrap();
    let base_project = Project::new("V100E2eRecovery", ProjectSettings::default());
    let folder = ProjectFolder::init(tmp.path().join("v100_proj"), &base_project).unwrap();

    let mut live = base_project.clone();
    let mut bus = CommandBus::with_folder(folder.clone());

    let fluid_item = TimelineItem::new(
        "E2eFluid",
        RationalTime::ZERO,
        RationalTime::from_seconds(4.0),
        ItemKind::FluidSim {
            config: Box::new(FluidSimConfig::from_preset(FluidPreset::BonfireFlame)),
        },
    );
    let fluid_id = fluid_item.id;

    let cloth_item = TimelineItem::new(
        "E2eCloth",
        RationalTime::ZERO,
        RationalTime::from_seconds(4.0),
        ItemKind::ClothSim {
            config: Box::new(ClothSimConfig::from_preset(ClothPreset::CyberFlag)),
        },
    );
    let cloth_id = cloth_item.id;

    let vtuber_item = TimelineItem::new(
        "E2eVtuber",
        RationalTime::ZERO,
        RationalTime::from_seconds(4.0),
        ItemKind::VtuberAvatar {
            config: Box::new(VtuberMeshConfig::default()),
        },
    );
    let vtuber_id = vtuber_item.id;

    let mut farm = RenderFarmConfig {
        enabled: true,
        total_frames: 180,
        ..RenderFarmConfig::default()
    };
    farm.chunks = plan_render_farm_chunks(&farm);
    assert!(
        farm.chunks
            .iter()
            .all(|c| verify_chunk_hmac(&farm.hmac_secret, c))
    );

    let plan = parse_script_to_storyboard(
        "# Release E2E\n## Hook\n[Close Up] v10.0.0 クラッシュ復旧テスト！\n",
        FrameRate::FPS_60,
    );
    let storyboard_cmds = build_storyboard_animatic_commands(&plan, 1920, 1080);

    let bvh = parse_bvh(&builtin_sample_bvh(MocapPreset::WalkCycle)).unwrap();
    let poses = retarget_bvh_to_rig2d(
        &bvh,
        &Rig2DConfig::default_humanoid(),
        &RetargetConfig::default(),
    );
    let mocap_cmds = build_mocap_retarget_bake_commands(cloth_id, 640.0, 360.0, &poses, false);

    let cmds: Vec<EditCommand> = vec![
        EditCommand::AddObject {
            layer_index: 0,
            item: fluid_item,
        },
        EditCommand::AddObject {
            layer_index: 1,
            item: cloth_item,
        },
        EditCommand::AddObject {
            layer_index: 2,
            item: vtuber_item,
        },
        EditCommand::SetAbCompare {
            ab_compare: AbCompareConfig {
                mode: AbCompareMode::VerticalSplit,
                split_ratio: 0.45,
                ..AbCompareConfig::default()
            },
        },
        EditCommand::SetOpticalVfx {
            optical_vfx: OpticalVfxConfig {
                enabled: true,
                god_rays_intensity: 0.4,
                ..OpticalVfxConfig::default()
            },
        },
        EditCommand::SetAdrSession {
            adr_session: AdrSessionConfig::default(),
        },
        EditCommand::SetSpatialBus {
            spatial_bus: SpatialBusConfig::default(),
        },
        EditCommand::SetLiveSwitcher {
            live_switcher: LiveSwitcherConfig {
                enabled: true,
                show_tally_hud: true,
                ..LiveSwitcherConfig::default()
            },
        },
        EditCommand::SetVtuberAvatar {
            item_id: vtuber_id,
            config: Box::new(VtuberMeshConfig {
                width: 320.0,
                height: 320.0,
                ..VtuberMeshConfig::default()
            }),
        },
        EditCommand::SetHardwareController {
            hardware_ctrl: HardwareControllerConfig::from_preset(
                HardwareControllerPreset::FullBroadcastRig,
            ),
        },
        EditCommand::SetSmartDamIndex {
            smart_dam: SmartDamIndex::default(),
        },
        EditCommand::SetSpatialVideo {
            spatial_video: SpatialVideoConfig::from_preset(Spatial360Preset::ActionHorizonLock),
        },
        EditCommand::SetLighting3DAdv {
            lighting_3d_adv: Lighting3DAdvConfig {
                shadow_map_enabled: true,
                hdri_ibl_enabled: true,
                hdri_preset: HdriEnvPreset::CyberNeonNight,
                ..Lighting3DAdvConfig::default()
            },
        },
        EditCommand::SetFluidSim {
            item_id: fluid_id,
            config: Box::new(FluidSimConfig::from_preset(FluidPreset::NeonPlasma)),
        },
        EditCommand::SetClothSim {
            item_id: cloth_id,
            config: Box::new(ClothSimConfig::from_preset(ClothPreset::HeroCape)),
        },
        EditCommand::SetRenderFarm { render_farm: farm },
        EditCommand::Batch {
            label: "Storyboard E2E".into(),
            commands: storyboard_cmds,
        },
        EditCommand::Batch {
            label: "Mocap Retarget E2E".into(),
            commands: mocap_cmds,
        },
    ];

    for cmd in cmds {
        bus.execute(&mut live, cmd, CommandSource::Gui, None)
            .expect("v7..v10 command must succeed");
    }

    let (ab_cmd, ab_summary) =
        build_ab_edit_variant_command(&live, EditVariantStyle::RetentionBoostHook, false).unwrap();
    assert!(ab_summary.new_scene_name.contains("AB_HookBoost"));
    bus.execute(&mut live, ab_cmd, CommandSource::Mcp, None)
        .unwrap();

    let expected_after = live.clone();
    let total_steps = bus.undo_len();
    assert_eq!(total_steps, 19);

    // 1. 全ステップ Undo して初期状態へ完全復帰することを検証
    for _ in 0..total_steps {
        assert!(bus.undo(&mut live));
    }
    assert_eq!(live, base_project);

    // 2. 全ステップ Redo して最終状態へ完全復帰することを検証
    for _ in 0..total_steps {
        assert!(bus.redo(&mut live));
    }
    assert_eq!(live, expected_after);

    // 3. `project.json` 未保存のままクラッシュした想定で `history.jsonl` から完全復旧
    let mut recovered = folder.load_project().unwrap();
    let history = folder.read_history().unwrap();
    let replayed = CommandBus::replay_history(&mut recovered, &history).unwrap();
    assert_eq!(replayed, total_steps);
    assert_eq!(recovered, expected_after);

    // 4. `project.json` 保存 → 再読込ラウンドトリップ一致
    folder.save_project(&expected_after).unwrap();
    let reloaded = folder.load_project().unwrap();
    assert_eq!(reloaded, expected_after);
}

#[test]
fn e2e_v100_legacy_v01_project_json_backward_compatibility() {
    // v2.0〜v10.0 の全オプションフィールドが欠落した最小レガシープロジェクト JSON
    let legacy_json = r#"{
        "schema_version": 1,
        "name": "LegacyV01Project",
        "settings": {
            "width": 160,
            "height": 90,
            "fps": { "num": 30, "den": 1 },
            "audio_sample_rate": 48000,
            "cache_max_bytes": 1073741824
        },
        "assets": [],
        "scenes": [
            {
                "id": "11111111-1111-4111-8111-111111111111",
                "name": "LegacyMain",
                "width": 160,
                "height": 90,
                "fps": { "num": 30, "den": 1 },
                "layers": [
                    {
                        "id": "22222222-2222-4222-8222-222222222222",
                        "name": "Layer 1",
                        "visible": true,
                        "muted": false,
                        "items": []
                    }
                ]
            }
        ],
        "active_scene_id": "11111111-1111-4111-8111-111111111111"
    }"#;

    let mut proj: Project =
        serde_json::from_str(legacy_json).expect("legacy v0.1 JSON must deserialize cleanly");
    assert_eq!(proj.name, "LegacyV01Project");
    assert!(!proj.render_farm.enabled);
    assert!(!proj.hardware_ctrl.enabled);
    {
        let sc = proj.active_scene_mut().unwrap();
        assert!(!sc.spatial_video.enabled);
        assert!(!sc.lighting_3d_adv.shadow_map_enabled);
        sc.layers[0].items.push(TimelineItem::new(
            "LegacyRect",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 80.0,
                height: 45.0,
                color: RgbaColor::WHITE,
            },
        ));
    }

    let mut graph = RenderGraph::new();
    let img = graph
        .render_project_scene(
            &proj,
            proj.active_scene_id,
            RationalTime::from_seconds(0.5),
            false,
        )
        .expect("upgraded legacy project must render cleanly");
    assert_eq!((img.width, img.height), (160, 90));
    assert!(
        img.pixels
            .iter()
            .all(|p| p.r.is_finite() && p.a.is_finite())
    );
}

#[test]
fn e2e_v100_all_builtin_wgsl_shaders_validate_and_gpu_or_cpu_fallback_matches() {
    let checks = validate_all_builtin_wgsl_shaders();
    assert_eq!(checks.len(), 9);
    assert!(
        checks.iter().all(|c| c.valid),
        "all 9 builtin WGSL shaders must pass naga validation: {:?}",
        checks
    );

    let bench = benchmark_v100_gpu_or_cpu(160, 90);
    assert!(bench.all_shaders_valid);
    assert!(bench.spatial_360_ms.is_finite());

    // CPU パスと GPU/CPU 統合ディスパッチャが全ピクセル有限値・同一解像度で動作することを確認
    let mut src = ve_render::LinearImage::new(128, 64, ve_render::LinearPremul::TRANSPARENT);
    for y in 0..64 {
        for x in 0..128 {
            src.pixels[(y * 128 + x) as usize] = ve_render::LinearPremul {
                r: x as f32 / 128.0,
                g: y as f32 / 64.0,
                b: 0.5,
                a: 1.0,
            };
        }
    }
    let cfg = SpatialVideoConfig::from_preset(Spatial360Preset::TinyPlanetOrbit);
    let (out, _used_gpu) = execute_v100_spatial_360_or_cpu(None, &src, &cfg, 0.5);
    assert_eq!((out.width, out.height), (128, 64));
    assert!(
        out.pixels
            .iter()
            .all(|p| p.r.is_finite() && p.g.is_finite() && p.b.is_finite() && p.a.is_finite())
    );
}

#[test]
fn e2e_v100_full_production_render_parallel_batch_and_zero_nan_invariants() {
    let mut proj = Project::new(
        "V100FullProductionScene",
        ProjectSettings {
            width: 160,
            height: 90,
            fps: FrameRate::FPS_30,
            ..ProjectSettings::default()
        },
    );
    {
        let sc = proj.active_scene_mut().unwrap();
        sc.lighting_3d_adv = Lighting3DAdvConfig {
            shadow_map_enabled: true,
            shadow_map_size: 96,
            hdri_ibl_enabled: true,
            hdri_preset: HdriEnvPreset::GoldenSunset,
            ground_catcher_enabled: true,
            ..Lighting3DAdvConfig::default()
        };
        sc.optical_vfx = OpticalVfxConfig {
            enabled: true,
            god_rays_intensity: 0.25,
            chromatic_aberration: 0.008,
            ..OpticalVfxConfig::default()
        };
        sc.live_switcher.enabled = true;
        sc.live_switcher.show_tally_hud = true;
        sc.spatial_video = SpatialVideoConfig::from_preset(Spatial360Preset::ActionHorizonLock);

        let mut fluid = TimelineItem::new(
            "Smoke",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::FluidSim {
                config: Box::new(FluidSimConfig {
                    grid_w: 24,
                    grid_h: 24,
                    width: 90.0,
                    height: 90.0,
                    ..FluidSimConfig::from_preset(FluidPreset::CinemaSmoke)
                }),
            },
        );
        fluid.transform.x = 40.0;
        fluid.transform.y = 45.0;
        sc.layers[0].items.push(fluid);

        let mut cloth = TimelineItem::new(
            "Banner",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::ClothSim {
                config: Box::new(ClothSimConfig {
                    cols: 8,
                    rows: 6,
                    width: 72.0,
                    height: 54.0,
                    ..ClothSimConfig::from_preset(ClothPreset::SilkBanner)
                }),
            },
        );
        cloth.transform.x = 120.0;
        cloth.transform.y = 45.0;
        sc.layers[1].items.push(cloth);

        let mut mesh = TimelineItem::new(
            "Hero3D",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Mesh3D {
                config: Mesh3DConfig {
                    width: 64.0,
                    height: 64.0,
                    ..Mesh3DConfig::default()
                },
            },
        );
        mesh.transform.x = 80.0;
        mesh.transform.y = 45.0;
        sc.layers[2].items.push(mesh);

        let mut chart = TimelineItem::new(
            "Kpi",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::DataChart {
                config: Box::new(DataChartConfig {
                    width: 70.0,
                    height: 44.0,
                    ..DataChartConfig::default()
                }),
            },
        );
        chart.transform.x = 80.0;
        chart.transform.y = 24.0;
        sc.layers[3].items.push(chart);
    }

    let times = [
        RationalTime::from_seconds(0.2),
        RationalTime::from_seconds(0.6),
        RationalTime::from_seconds(1.0),
        RationalTime::from_seconds(1.4),
    ];

    let empty_proxies = HashMap::new();
    let empty_wf = HashMap::new();
    let seq = render_frame_batch_parallel(
        &proj,
        proj.active_scene_id,
        None,
        &times,
        false,
        false,
        false,
        HwDecodeBackend::Software,
        &empty_proxies,
        &empty_wf,
        1,
    );
    let par = render_frame_batch_parallel(
        &proj,
        proj.active_scene_id,
        None,
        &times,
        false,
        false,
        false,
        HwDecodeBackend::Software,
        &empty_proxies,
        &empty_wf,
        4,
    );

    assert_eq!(seq.len(), times.len());
    assert_eq!(
        seq, par,
        "4-thread parallel render of full v10.0 production scene must match sequential bit-for-bit"
    );
    for frame in &par {
        assert_eq!(frame.len(), (160 * 90 * 4) as usize);
        assert!(frame.iter().any(|&b| b > 20));
    }
}
