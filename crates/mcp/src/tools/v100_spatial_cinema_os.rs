//! v10.0 #57〜#60 Spatial Computing, Physics Fluids & Autonomous Cinema OS MCP ツール群
//! （10 ツール / 10 ドメイン・全 100 MCP ツール完全体系化達成）:
//! - #57 `configure_360_overcapture_and_horizon_lock`
//! - #57 `configure_stereoscopic_spatial_3d`
//! - #58 `add_or_update_fluid_simulation`
//! - #58 `add_or_update_cloth_simulation`
//! - #58 `configure_3d_shadows_and_hdri_ibl`
//! - #59 `import_bvh_mocap_and_retarget`
//! - #59 `extract_silhouette_mocap_pose`
//! - #60 `manage_lan_render_farm`
//! - #60 `produce_storyboard_animatic_from_script`
//! - #60 `predict_retention_and_generate_ab_variants`

use super::*;
use crate::session::EditorSession;
use std::io::{Read, Write};
use std::net::TcpStream;
use ve_core::{
    ClothPreset, ClothSimConfig, EditVariantStyle, FluidPreset, FluidSimConfig,
    GyroOrientationSample, HdriEnvPreset, Lighting3DAdvConfig, MocapPreset, Projection360Mode,
    RenderFarmConfig, RenderFarmCoordinatorServer, RenderFarmWorkerNode, RetargetConfig,
    Spatial360Preset, SpatialVideoConfig, StereoLayout, analyze_viewer_retention,
    build_ab_edit_variant_command, build_farm_concat_manifest, build_mocap_retarget_bake_commands,
    build_storyboard_animatic_commands, builtin_sample_bvh, compass_heading_label,
    evaluate_horizon_locked_euler, extract_silhouette_pose_from_rgba8, parse_bvh,
    parse_script_to_storyboard, plan_render_farm_chunks, retarget_bvh_to_rig2d,
    retarget_silhouette_sequence_to_rig2d, simulate_cloth_pbd_at_time, simulate_fluid_2d_at_time,
    verify_chunk_hmac,
};

pub(crate) const DOMAIN_ID: &str = "v100_spatial_cinema_os";
pub(crate) const DOMAIN_LABEL: &str =
    "v10.0 空間360°・流体/クロス物理・BVHモキャップ・分散レンダー＆自律AIプロデューサー (#57〜#60)";

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "configure_360_overcapture_and_horizon_lock",
            "description": "v10.0 #57: 360° 全天球動画の OverCapture（透視投影切り出し）・Tiny Planet（逆ステレオ投影）・ジャイロ水平ロックを設定またはプリセット適用する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["status", "configure", "apply_preset"] },
                    "preset": { "type": "string", "enum": ["action_horizon_lock", "tiny_planet_orbit", "spatial_vr_sbs", "anaglyph_3d_review"] },
                    "enabled": { "type": "boolean" },
                    "projection_mode": { "type": "string", "enum": ["off", "over_capture", "tiny_planet", "equirectangular_review"] },
                    "yaw_deg": { "type": "number" },
                    "pitch_deg": { "type": "number" },
                    "roll_deg": { "type": "number" },
                    "fov_deg": { "type": "number" },
                    "tiny_planet_zoom": { "type": "number" },
                    "horizon_lock_enabled": { "type": "boolean" },
                    "horizon_lock_smoothing": { "type": "number" },
                    "show_horizon_guides": { "type": "boolean" },
                    "sample_time_sec": { "type": "number" }
                }
            }
        }),
        json!({
            "name": "configure_stereoscopic_spatial_3d",
            "description": "v10.0 #57: ステレオスコピック空間 3D ビデオ出力（Side-by-Side / Top-and-Bottom / Red-Cyan アナグリフ / 単眼 L・R）および IPD 視差・コンバージェンスを制御する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "stereo_layout": { "type": "string", "enum": ["off", "side_by_side", "top_and_bottom", "anaglyph_red_cyan", "mono_left", "mono_right"] },
                    "ipd_parallax_px": { "type": "number" },
                    "convergence_offset_px": { "type": "number" }
                }
            }
        }),
        json!({
            "name": "add_or_update_fluid_simulation",
            "description": "v10.0 #58: Jos Stam 安定法 決定論 2D Navier-Stokes 流体（煙・炎・墨インク・ネオンプラズマ）アイテムを追加／更新し、非圧縮発散残差や密度統計を返す",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["add", "update", "simulate_stats"] },
                    "item_id": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "preset": { "type": "string", "enum": ["cinema_smoke", "bonfire_flame", "sumi_ink", "neon_plasma"] },
                    "grid_w": { "type": "integer" },
                    "grid_h": { "type": "integer" },
                    "vorticity_confinement": { "type": "number" },
                    "buoyancy": { "type": "number" },
                    "time_sec": { "type": "number" }
                }
            }
        }),
        json!({
            "name": "add_or_update_cloth_simulation",
            "description": "v10.0 #58: Position-Based Dynamics（PBD）決定論クロス物理（シルクバナー・マント・カーテン・サイバー旗）アイテムを追加／更新する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["add", "update", "simulate_stats"] },
                    "item_id": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "preset": { "type": "string", "enum": ["silk_banner", "hero_cape", "curtain_drape", "cyber_flag"] },
                    "wind_strength": { "type": "number" },
                    "wind_turbulence": { "type": "number" },
                    "gravity": { "type": "number" },
                    "show_wireframe": { "type": "boolean" },
                    "time_sec": { "type": "number" }
                }
            }
        }),
        json!({
            "name": "configure_3d_shadows_and_hdri_ibl",
            "description": "v10.0 #58: 3D シーンの直交デプスシャドウマップ（3×3 / 5×5 PCF ソフトシャドウ＋地面シャドウキャッチャー）および Equirectangular HDRI 拡散/鏡面 IBL を設定する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "shadow_map_enabled": { "type": "boolean" },
                    "shadow_map_size": { "type": "integer" },
                    "shadow_softness_pcf": { "type": "integer" },
                    "shadow_strength": { "type": "number" },
                    "ground_catcher_enabled": { "type": "boolean" },
                    "hdri_ibl_enabled": { "type": "boolean" },
                    "hdri_preset": { "type": "string", "enum": ["studio_softbox", "golden_sunset", "cyber_neon_night", "overcast_daylight"] },
                    "hdri_rotation_deg": { "type": "number" },
                    "hdri_exposure": { "type": "number" }
                }
            }
        }),
        json!({
            "name": "import_bvh_mocap_and_retarget",
            "description": "v10.0 #59: 標準 .bvh モーションキャプチャ（または内蔵プリセット）を解析し、体格差吸収スケーリング＋足接地ロック（Foot Skate Locking）付きで 2D パペットまたは 3D メッシュへ 1-Undo リターゲット＆ベイクする",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bvh_text": { "type": "string" },
                    "preset": { "type": "string", "enum": ["walk_cycle", "wave_and_jump", "hero_landing"] },
                    "target_item_id": { "type": "string" },
                    "target_is_3d": { "type": "boolean" },
                    "foot_lock_enabled": { "type": "boolean" },
                    "scale_by_bone_ratio": { "type": "boolean" },
                    "bake_keyframes": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "extract_silhouette_mocap_pose",
            "description": "v10.0 #59: アクティブシーンの複数フレームをオフラインレンダリングし、前景シルエットから 7 関節 2D 骨格姿勢を抽出して足接地ロック付きリターゲット結果を返す（任意でアイテムへベイク）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "start_sec": { "type": "number" },
                    "frame_count": { "type": "integer" },
                    "step_sec": { "type": "number" },
                    "luma_threshold": { "type": "number" },
                    "target_item_id": { "type": "string" },
                    "bake_keyframes": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "manage_lan_render_farm",
            "description": "v10.0 #60: LAN 分散レンダーファームのワーカーノード登録・重み比例フレームチャンク分割・RFC 2104 HMAC-SHA256 署名検証・FFmpeg concat 結合マニフェスト生成・ローカルループバックテストを実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["status", "plan_chunks", "add_worker", "concat_manifest", "test_loopback_farm"] },
                    "total_frames": { "type": "integer" },
                    "worker_id": { "type": "string" },
                    "host": { "type": "string" },
                    "port": { "type": "integer" },
                    "cpu_threads": { "type": "integer" },
                    "weight": { "type": "number" }
                }
            }
        }),
        json!({
            "name": "produce_storyboard_animatic_from_script",
            "description": "v10.0 #60: 自律 AI シネマプロデューサー — 台本（Markdown/テキスト）をショットフレーミング・カメラノート・尺付き絵コンテへ分解し、任意でプレイアブル・アニマティックを 1-Undo でタイムライン構築する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "script_text": { "type": "string" },
                    "build_animatic": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "predict_retention_and_generate_ab_variants",
            "description": "v10.0 #60: 自律 AI シネマプロデューサー — タイムラインの視聴維持率カーブ予測・冒頭フック評価・離脱リスク診断を実行し、任意で A/B 自動編集バリアントシーンを生成＆全 100 MCP ツール到達統計を返す",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["predict_retention", "generate_ab_variant", "cinema_os_summary"] },
                    "style": { "type": "string", "enum": ["fast_paced_shorts", "cinematic_narrative", "retention_boost_hook"] },
                    "switch_to": { "type": "boolean" }
                }
            }
        }),
    ]
}

pub(crate) fn try_call(
    guard: &mut EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    match name {
        "configure_360_overcapture_and_horizon_lock" => Some(call_spatial_360(guard, args)),
        "configure_stereoscopic_spatial_3d" => Some(call_stereo_3d(guard, args)),
        "add_or_update_fluid_simulation" => Some(call_fluid_sim(guard, args)),
        "add_or_update_cloth_simulation" => Some(call_cloth_sim(guard, args)),
        "configure_3d_shadows_and_hdri_ibl" => Some(call_3d_shadows_ibl(guard, args)),
        "import_bvh_mocap_and_retarget" => Some(call_bvh_retarget(guard, args)),
        "extract_silhouette_mocap_pose" => Some(call_silhouette_mocap(guard, args)),
        "manage_lan_render_farm" => Some(call_lan_render_farm(guard, args)),
        "produce_storyboard_animatic_from_script" => {
            Some(call_storyboard_animatic(guard, fps, args))
        }
        "predict_retention_and_generate_ab_variants" => Some(call_retention_and_ab(guard, args)),
        _ => None,
    }
}

fn call_spatial_360(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("configure");
    let mut cfg = guard
        .project
        .active_scene()
        .ok_or("no active scene")?
        .spatial_video
        .clone();

    match action {
        "status" => {}
        "apply_preset" => {
            let preset = match args
                .get("preset")
                .and_then(Value::as_str)
                .unwrap_or("action_horizon_lock")
            {
                "tiny_planet_orbit" => Spatial360Preset::TinyPlanetOrbit,
                "spatial_vr_sbs" => Spatial360Preset::SpatialVrSbs,
                "anaglyph_3d_review" => Spatial360Preset::Anaglyph3DReview,
                _ => Spatial360Preset::ActionHorizonLock,
            };
            cfg = SpatialVideoConfig::from_preset(preset);
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::SetSpatialVideo {
                        spatial_video: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
        }
        _ => {
            cfg.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if let Some(m) = args.get("projection_mode").and_then(Value::as_str) {
                cfg.projection_mode = match m {
                    "off" => Projection360Mode::Off,
                    "tiny_planet" => Projection360Mode::TinyPlanet,
                    "equirectangular_review" => Projection360Mode::EquirectangularReview,
                    _ => Projection360Mode::OverCapture,
                };
            }
            cfg.yaw_deg =
                finite_clamped(args.get("yaw_deg"), cfg.yaw_deg as f64, -180.0, 180.0) as f32;
            cfg.pitch_deg =
                finite_clamped(args.get("pitch_deg"), cfg.pitch_deg as f64, -90.0, 90.0) as f32;
            cfg.roll_deg =
                finite_clamped(args.get("roll_deg"), cfg.roll_deg as f64, -180.0, 180.0) as f32;
            cfg.fov_deg =
                finite_clamped(args.get("fov_deg"), cfg.fov_deg as f64, 25.0, 155.0) as f32;
            cfg.tiny_planet_zoom = finite_clamped(
                args.get("tiny_planet_zoom"),
                cfg.tiny_planet_zoom as f64,
                0.25,
                4.0,
            ) as f32;
            if let Some(hl) = args.get("horizon_lock_enabled").and_then(Value::as_bool) {
                cfg.horizon_lock_enabled = hl;
                if hl && cfg.gyro_samples.is_empty() {
                    cfg.gyro_samples = vec![
                        GyroOrientationSample {
                            time_sec: 0.0,
                            yaw_deg: 0.0,
                            pitch_deg: 6.0,
                            roll_deg: -15.0,
                        },
                        GyroOrientationSample {
                            time_sec: 2.0,
                            yaw_deg: 20.0,
                            pitch_deg: -5.0,
                            roll_deg: 18.0,
                        },
                    ];
                }
            }
            cfg.horizon_lock_smoothing = finite_clamped(
                args.get("horizon_lock_smoothing"),
                cfg.horizon_lock_smoothing as f64,
                0.0,
                0.98,
            ) as f32;
            if let Some(g) = args.get("show_horizon_guides").and_then(Value::as_bool) {
                cfg.show_horizon_guides = g;
            }
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::SetSpatialVideo {
                        spatial_video: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
        }
    }

    let sample_t = finite_clamped(args.get("sample_time_sec"), 0.5, 0.0, 3600.0) as f32;
    let (ey, ep, er) = evaluate_horizon_locked_euler(&cfg, sample_t);
    let text = serde_json::to_string_pretty(&json!({
        "spatial_video": cfg,
        "effective_euler_at_sample": {
            "time_sec": sample_t,
            "yaw_deg": ey,
            "pitch_deg": ep,
            "roll_deg": er,
            "compass_heading": compass_heading_label(ey)
        }
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_stereo_3d(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let mut cfg = guard
        .project
        .active_scene()
        .ok_or("no active scene")?
        .spatial_video
        .clone();
    cfg.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    cfg.stereo_layout = match args
        .get("stereo_layout")
        .and_then(Value::as_str)
        .unwrap_or("side_by_side")
    {
        "off" => StereoLayout::Off,
        "top_and_bottom" => StereoLayout::TopAndBottom,
        "anaglyph_red_cyan" => StereoLayout::AnaglyphRedCyan,
        "mono_left" => StereoLayout::MonoLeft,
        "mono_right" => StereoLayout::MonoRight,
        _ => StereoLayout::SideBySide,
    };
    cfg.ipd_parallax_px = finite_clamped(
        args.get("ipd_parallax_px"),
        cfg.ipd_parallax_px as f64,
        0.0,
        64.0,
    ) as f32;
    cfg.convergence_offset_px = finite_clamped(
        args.get("convergence_offset_px"),
        cfg.convergence_offset_px as f64,
        -48.0,
        48.0,
    ) as f32;

    guard
        .bus
        .execute(
            &mut guard.project,
            EditCommand::SetSpatialVideo {
                spatial_video: cfg.clone(),
            },
            CommandSource::Mcp,
            None,
        )
        .map_err(|e| e.to_string())?;

    let text = serde_json::to_string_pretty(&json!({
        "enabled": cfg.enabled,
        "stereo_layout": cfg.stereo_layout,
        "ipd_parallax_px": cfg.ipd_parallax_px,
        "convergence_offset_px": cfg.convergence_offset_px
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_fluid_sim(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args.get("action").and_then(Value::as_str).unwrap_or("add");
    let preset = match args
        .get("preset")
        .and_then(Value::as_str)
        .unwrap_or("bonfire_flame")
    {
        "cinema_smoke" => FluidPreset::CinemaSmoke,
        "sumi_ink" => FluidPreset::SumiInk,
        "neon_plasma" => FluidPreset::NeonPlasma,
        _ => FluidPreset::BonfireFlame,
    };
    let mut cfg = FluidSimConfig::from_preset(preset);
    if let Some(gw) = args.get("grid_w").and_then(Value::as_u64) {
        cfg.grid_w = (gw as u32).clamp(16, 96);
    }
    if let Some(gh) = args.get("grid_h").and_then(Value::as_u64) {
        cfg.grid_h = (gh as u32).clamp(16, 96);
    }
    cfg.vorticity_confinement = finite_clamped(
        args.get("vorticity_confinement"),
        cfg.vorticity_confinement as f64,
        0.0,
        12.0,
    ) as f32;
    cfg.buoyancy = finite_clamped(args.get("buoyancy"), cfg.buoyancy as f64, -5.0, 8.0) as f32;

    let t = finite_clamped(args.get("time_sec"), 0.8, 0.0, 10.0) as f32;
    let stats = simulate_fluid_2d_at_time(&cfg, t);
    let total_density: f32 = stats.density.iter().sum();

    let item_id = match action {
        "simulate_stats" => None,
        "update" => {
            let id = parse_uuid(args, "item_id")?;
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::SetFluidSim {
                        item_id: id,
                        config: Box::new(cfg.clone()),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Some(id)
        }
        _ => {
            let scene = guard.project.active_scene().ok_or("no active scene")?;
            let layer_idx = args.get("layer_index").and_then(Value::as_u64).unwrap_or(1) as usize;
            let layer_idx = layer_idx.min(scene.layers.len().saturating_sub(1));
            let start =
                RationalTime::from_seconds(finite_clamped(args.get("start_sec"), 0.0, 0.0, 3600.0));
            let dur = RationalTime::from_seconds(finite_clamped(
                args.get("duration_sec"),
                4.0,
                0.2,
                600.0,
            ));
            let mut item = TimelineItem::new(
                format!("Fluid_{preset:?}"),
                start,
                dur,
                ItemKind::FluidSim {
                    config: Box::new(cfg.clone()),
                },
            );
            item.transform.x = scene.width as f32 * 0.5;
            item.transform.y = scene.height as f32 * 0.5;
            let id = item.id;
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::AddObject {
                        layer_index: layer_idx,
                        item,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Some(id)
        }
    };

    let text = serde_json::to_string_pretty(&json!({
        "item_id": item_id,
        "preset": cfg.preset,
        "grid": [stats.width, stats.height],
        "steps_simulated": stats.steps_simulated,
        "total_density": total_density,
        "max_divergence_after_project": stats.max_divergence_after_project
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_cloth_sim(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args.get("action").and_then(Value::as_str).unwrap_or("add");
    let preset = match args
        .get("preset")
        .and_then(Value::as_str)
        .unwrap_or("silk_banner")
    {
        "hero_cape" => ClothPreset::HeroCape,
        "curtain_drape" => ClothPreset::CurtainDrape,
        "cyber_flag" => ClothPreset::CyberFlag,
        _ => ClothPreset::SilkBanner,
    };
    let mut cfg = ClothSimConfig::from_preset(preset);
    cfg.wind_strength = finite_clamped(
        args.get("wind_strength"),
        cfg.wind_strength as f64,
        0.0,
        8.0,
    ) as f32;
    cfg.wind_turbulence = finite_clamped(
        args.get("wind_turbulence"),
        cfg.wind_turbulence as f64,
        0.0,
        4.0,
    ) as f32;
    cfg.gravity = finite_clamped(args.get("gravity"), cfg.gravity as f64, 0.0, 5.0) as f32;
    if let Some(wf) = args.get("show_wireframe").and_then(Value::as_bool) {
        cfg.show_wireframe = wf;
    }

    let t = finite_clamped(args.get("time_sec"), 0.9, 0.0, 10.0) as f32;
    let state = simulate_cloth_pbd_at_time(&cfg, t);
    let max_z = state
        .positions
        .iter()
        .map(|p| p[2].abs())
        .fold(0.0f32, f32::max);

    let item_id = match action {
        "simulate_stats" => None,
        "update" => {
            let id = parse_uuid(args, "item_id")?;
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::SetClothSim {
                        item_id: id,
                        config: Box::new(cfg.clone()),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Some(id)
        }
        _ => {
            let scene = guard.project.active_scene().ok_or("no active scene")?;
            let layer_idx = args.get("layer_index").and_then(Value::as_u64).unwrap_or(1) as usize;
            let layer_idx = layer_idx.min(scene.layers.len().saturating_sub(1));
            let start =
                RationalTime::from_seconds(finite_clamped(args.get("start_sec"), 0.0, 0.0, 3600.0));
            let dur = RationalTime::from_seconds(finite_clamped(
                args.get("duration_sec"),
                4.0,
                0.2,
                600.0,
            ));
            let mut item = TimelineItem::new(
                format!("Cloth_{preset:?}"),
                start,
                dur,
                ItemKind::ClothSim {
                    config: Box::new(cfg.clone()),
                },
            );
            item.transform.x = scene.width as f32 * 0.5;
            item.transform.y = scene.height as f32 * 0.5;
            let id = item.id;
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::AddObject {
                        layer_index: layer_idx,
                        item,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Some(id)
        }
    };

    let text = serde_json::to_string_pretty(&json!({
        "item_id": item_id,
        "preset": cfg.preset,
        "vertices": state.positions.len(),
        "pinned_anchors": state.pinned.iter().filter(|&&p| p).count(),
        "max_billow_z": max_z
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_3d_shadows_ibl(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let mut cfg: Lighting3DAdvConfig = guard
        .project
        .active_scene()
        .ok_or("no active scene")?
        .lighting_3d_adv
        .clone();
    if let Some(v) = args.get("shadow_map_enabled").and_then(Value::as_bool) {
        cfg.shadow_map_enabled = v;
    }
    if let Some(sz) = args.get("shadow_map_size").and_then(Value::as_u64) {
        cfg.shadow_map_size = (sz as u32).clamp(64, 512);
    }
    if let Some(pcf) = args.get("shadow_softness_pcf").and_then(Value::as_u64) {
        cfg.shadow_softness_pcf = (pcf as u32).clamp(1, 3);
    }
    cfg.shadow_strength = finite_clamped(
        args.get("shadow_strength"),
        cfg.shadow_strength as f64,
        0.0,
        1.0,
    ) as f32;
    if let Some(gc) = args.get("ground_catcher_enabled").and_then(Value::as_bool) {
        cfg.ground_catcher_enabled = gc;
    }
    if let Some(ibl) = args.get("hdri_ibl_enabled").and_then(Value::as_bool) {
        cfg.hdri_ibl_enabled = ibl;
    }
    if let Some(p) = args.get("hdri_preset").and_then(Value::as_str) {
        cfg.hdri_preset = match p {
            "golden_sunset" => HdriEnvPreset::GoldenSunset,
            "cyber_neon_night" => HdriEnvPreset::CyberNeonNight,
            "overcast_daylight" => HdriEnvPreset::OvercastDaylight,
            _ => HdriEnvPreset::StudioSoftbox,
        };
    }
    cfg.hdri_rotation_deg = finite_clamped(
        args.get("hdri_rotation_deg"),
        cfg.hdri_rotation_deg as f64,
        0.0,
        360.0,
    ) as f32;
    cfg.hdri_exposure = finite_clamped(
        args.get("hdri_exposure"),
        cfg.hdri_exposure as f64,
        0.1,
        4.0,
    ) as f32;

    guard
        .bus
        .execute(
            &mut guard.project,
            EditCommand::SetLighting3DAdv {
                lighting_3d_adv: cfg.clone(),
            },
            CommandSource::Mcp,
            None,
        )
        .map_err(|e| e.to_string())?;

    let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_bvh_retarget(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let bvh_src = if let Some(txt) = args.get("bvh_text").and_then(Value::as_str)
        && !txt.trim().is_empty()
    {
        txt.to_string()
    } else {
        let preset = match args
            .get("preset")
            .and_then(Value::as_str)
            .unwrap_or("walk_cycle")
        {
            "wave_and_jump" => MocapPreset::WaveAndJump,
            "hero_landing" => MocapPreset::HeroLanding,
            _ => MocapPreset::WalkCycle,
        };
        builtin_sample_bvh(preset)
    };

    let clip = parse_bvh(&bvh_src)?;
    let retarget_cfg = RetargetConfig {
        scale_by_bone_ratio: args
            .get("scale_by_bone_ratio")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        foot_lock_enabled: args
            .get("foot_lock_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        ..RetargetConfig::default()
    };

    let rig = Rig2DConfig::default_humanoid();
    let poses = retarget_bvh_to_rig2d(&clip, &rig, &retarget_cfg);
    let foot_locked_frames = poses
        .iter()
        .filter(|p| p.left_foot_locked || p.right_foot_locked)
        .count();

    let mut baked_commands = 0usize;
    let bake = args
        .get("bake_keyframes")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if bake && let Ok(target_id) = parse_uuid(args, "target_item_id") {
        let target_is_3d = args
            .get("target_is_3d")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (bx, by) = guard
            .project
            .active_scene()
            .and_then(|sc| {
                sc.layers
                    .iter()
                    .flat_map(|l| l.items.iter())
                    .find(|it| it.id == target_id)
                    .map(|it| (it.transform.x, it.transform.y))
            })
            .unwrap_or((640.0, 360.0));
        let cmds = build_mocap_retarget_bake_commands(target_id, bx, by, &poses, target_is_3d);
        baked_commands = cmds.len();
        if !cmds.is_empty() {
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::Batch {
                        label: "Bake BVH Mocap Retarget".into(),
                        commands: cmds,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
        }
    }

    let text = serde_json::to_string_pretty(&json!({
        "joints": clip.joints.len(),
        "frames": clip.frames.len(),
        "frame_time_sec": clip.frame_time_sec,
        "retargeted_poses": poses.len(),
        "foot_locked_frames": foot_locked_frames,
        "baked_keyframe_commands": baked_commands
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_silhouette_mocap(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let start_sec = finite_clamped(args.get("start_sec"), 0.0, 0.0, 3600.0);
    let count = args
        .get("frame_count")
        .and_then(Value::as_u64)
        .unwrap_or(4)
        .clamp(1, 24) as usize;
    let step_sec = finite_clamped(args.get("step_sec"), 0.15, 0.01, 5.0);
    let luma_thr = finite_clamped(args.get("luma_threshold"), 0.15, 0.02, 0.95) as f32;

    let mut graph = RenderGraph::new();
    let scene_id = guard.project.active_scene_id;
    let mut sil_frames = Vec::with_capacity(count);
    for i in 0..count {
        let t_sec = start_sec + i as f64 * step_sec;
        let t = RationalTime::from_seconds(t_sec);
        let img = graph
            .render_project_scene_scaled(&guard.project, scene_id, t, 128, 72, false)
            .ok_or("failed to render scene frame for silhouette mocap")?;
        let rgba = img.to_rgba8_srgb();
        sil_frames.push(extract_silhouette_pose_from_rgba8(
            &rgba,
            img.width,
            img.height,
            luma_thr,
            t_sec as f32,
        ));
    }

    let rig = Rig2DConfig::default_humanoid();
    let poses =
        retarget_silhouette_sequence_to_rig2d(&sil_frames, &rig, &RetargetConfig::default());
    let mut baked = 0usize;
    if args
        .get("bake_keyframes")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && let Ok(target_id) = parse_uuid(args, "target_item_id")
    {
        let cmds = build_mocap_retarget_bake_commands(target_id, 640.0, 360.0, &poses, false);
        baked = cmds.len();
        if !cmds.is_empty() {
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::Batch {
                        label: "Bake Silhouette Mocap".into(),
                        commands: cmds,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
        }
    }

    let avg_conf =
        sil_frames.iter().map(|f| f.confidence).sum::<f32>() / sil_frames.len().max(1) as f32;
    let text = serde_json::to_string_pretty(&json!({
        "extracted_frames": sil_frames.len(),
        "avg_confidence": avg_conf,
        "retargeted_poses": poses.len(),
        "baked_keyframe_commands": baked
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_lan_render_farm(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("plan_chunks");
    let mut cfg: RenderFarmConfig = guard.project.render_farm.clone();

    match action {
        "status" => {
            let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "add_worker" => {
            let wid = args
                .get("worker_id")
                .and_then(Value::as_str)
                .unwrap_or("node-lan-extra")
                .to_string();
            let host = args
                .get("host")
                .and_then(Value::as_str)
                .unwrap_or("192.168.1.50")
                .to_string();
            let port = args.get("port").and_then(Value::as_u64).unwrap_or(19481) as u16;
            let threads = args.get("cpu_threads").and_then(Value::as_u64).unwrap_or(8) as u32;
            let weight = finite_clamped(args.get("weight"), 1.0, 0.1, 16.0) as f32;
            cfg.workers.push(RenderFarmWorkerNode {
                worker_id: wid,
                host,
                port,
                cpu_threads: threads,
                weight,
                healthy: true,
            });
            cfg.enabled = true;
            cfg.chunks = plan_render_farm_chunks(&cfg);
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::SetRenderFarm {
                        render_farm: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "worker_count": cfg.workers.len(),
                "chunk_count": cfg.chunks.len()
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "concat_manifest" => {
            if cfg.chunks.is_empty() {
                cfg.chunks = plan_render_farm_chunks(&cfg);
            }
            let manifest = build_farm_concat_manifest(&cfg.chunks);
            Ok(json!({ "content": [{ "type": "text", "text": manifest }] }))
        }
        "test_loopback_farm" => {
            cfg.enabled = true;
            cfg.coordinator_port = 0;
            cfg.chunks = plan_render_farm_chunks(&cfg);
            let mut srv = RenderFarmCoordinatorServer::bind_localhost(&cfg)?;
            let port = srv.port;
            let token = cfg.bearer_token.clone();
            let handle = std::thread::spawn(move || {
                let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
                let req = format!(
                    "POST /api/v1/claim HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"
                );
                s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
                let mut resp = String::new();
                let _ = s.read_to_string(&mut resp);
                Ok::<String, String>(resp)
            });
            for _ in 0..20 {
                if srv.poll_once() > 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let resp = handle
                .join()
                .map_err(|_| "loopback worker thread panicked".to_string())??;
            let text = serde_json::to_string_pretty(&json!({
                "coordinator_port": port,
                "requests_served": srv.requests_served,
                "hmac_all_valid": srv.chunks.iter().all(|c| verify_chunk_hmac(&cfg.hmac_secret, c)),
                "worker_response_ok": resp.contains("200 OK")
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        _ => {
            if let Some(tf) = args.get("total_frames").and_then(Value::as_u64) {
                cfg.total_frames = tf.clamp(1, 1_000_000);
            }
            cfg.enabled = true;
            cfg.chunks = plan_render_farm_chunks(&cfg);
            let all_valid = cfg
                .chunks
                .iter()
                .all(|c| verify_chunk_hmac(&cfg.hmac_secret, c));
            guard
                .bus
                .execute(
                    &mut guard.project,
                    EditCommand::SetRenderFarm {
                        render_farm: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "total_frames": cfg.total_frames,
                "worker_count": cfg.workers.len(),
                "chunk_count": cfg.chunks.len(),
                "hmac_all_valid": all_valid,
                "chunks": cfg.chunks
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
    }
}

fn call_storyboard_animatic(
    guard: &mut EditorSession,
    fps: FrameRate,
    args: &Value,
) -> Result<Value, String> {
    let script = args
        .get("script_text")
        .and_then(Value::as_str)
        .unwrap_or(
            "# Autonomous Cinema OS\n## Opening Hook\n[Close Up] 100 MCP ツールで映像制作を自律自動化！\n## Showcase\n[Wide] 360° 全天球・流体・クロス・BVH モーションをローカル完結で合成します。\n",
        );
    let plan = parse_script_to_storyboard(script, fps);
    let build = args
        .get("build_animatic")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut created_commands = 0usize;
    if build {
        let (w, h) = guard
            .project
            .active_scene()
            .map(|s| (s.width, s.height))
            .unwrap_or((1920, 1080));
        let cmds = build_storyboard_animatic_commands(&plan, w, h);
        created_commands = cmds.len();
        guard
            .bus
            .execute(
                &mut guard.project,
                EditCommand::Batch {
                    label: "Produce Storyboard Animatic".into(),
                    commands: cmds,
                },
                CommandSource::Mcp,
                None,
            )
            .map_err(|e| e.to_string())?;
    }

    let text = serde_json::to_string_pretty(&json!({
        "title": plan.title,
        "beat_count": plan.beats.len(),
        "total_duration_sec": plan.total_duration_sec,
        "created_commands": created_commands,
        "beats": plan.beats
    }))
    .unwrap_or_default();
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn call_retention_and_ab(guard: &mut EditorSession, args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("predict_retention");
    match action {
        "generate_ab_variant" => {
            let style = match args
                .get("style")
                .and_then(Value::as_str)
                .unwrap_or("retention_boost_hook")
            {
                "fast_paced_shorts" => EditVariantStyle::FastPacedShorts,
                "cinematic_narrative" => EditVariantStyle::CinematicNarrative,
                _ => EditVariantStyle::RetentionBoostHook,
            };
            let switch_to = args
                .get("switch_to")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let (cmd, summary) = build_ab_edit_variant_command(&guard.project, style, switch_to)?;
            guard
                .bus
                .execute(&mut guard.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&summary).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "cinema_os_summary" => {
            let catalog = tool_domain_catalog();
            let total_tools: usize = catalog.iter().map(|d| d.tool_count).sum();
            let diag = analyze_viewer_retention(&guard.project, None);
            let text = serde_json::to_string_pretty(&json!({
                "version": "10.0.0",
                "roadmap_completed": "60/60",
                "mcp_domains": catalog.len(),
                "mcp_total_tools": total_tools,
                "predicted_avg_retention_pct": diag.predicted_avg_retention_pct,
                "hook_score": diag.hook_score
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        _ => {
            let diag = analyze_viewer_retention(&guard.project, None);
            let text = serde_json::to_string_pretty(&diag).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
    }
}
