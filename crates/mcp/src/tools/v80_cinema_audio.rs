//! v8.0 #49〜#52 Pro Cinema VFX, Motion Foundry & Audio Restoration MCP ツール群（4 ツール / 全 86 MCP ツール）:
//! 1. `planar_track_and_roto_brush` (#49)
//! 2. `svg_morph_text_path_and_warp` (#50)
//! 3. `cinema_optical_vfx` (#51)
//! 4. `audio_restoration_adr_spatial_bus` (#52)

use super::*;
use ve_audio::{
    apply_adaptive_dereverb, apply_declick, apply_hum_removal, build_adr_cue_timeline_commands,
    decode_foa_ambix_to_binaural_stereo, downmix_51_to_stereo_itu, encode_stereo_to_51_surround,
    encode_stereo_to_foa_ambix, rotate_foa_ambix, synthesize_adr_preroll_beeps,
    write_multichannel_wav_file,
};
use ve_core::{
    AdrCue, OpticalVfxConfig, OpticalVfxPreset, PlanarTrackConfig, PseudoDepthMode,
    RotoBrushConfig, RotoKeyframe, SpatialAudioBusMode, SvgMorphConfig, SvgShapePreset,
    VectorFillMode, WarpPreset, build_planar_track_corner_pin_command, parse_svg_path_to_vertices,
};
use ve_render::{
    propagate_roto_brush_in_scene, snap_roto_vertices_to_edges, track_planar_quad_in_scene,
};

pub(crate) const DOMAIN_ID: &str = "v80_cinema_audio";
pub(crate) const DOMAIN_LABEL: &str =
    "v8.0 Pro Cinema VFX, Motion Foundry & Audio Restoration (#49-#52)";

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "planar_track_and_roto_brush",
            "description": "v8.0 #49: Run 4-point planar homography tracking (ZNCC + RANSAC) and bake animated CornerPin keyframes, or configure/propagate Sobel edge-snapping keyframe Roto Brush masks (1-Undo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["track_planar", "track_and_bake_corner_pin", "roto_snap", "roto_propagate", "set_roto_brush", "get_roto_brush"]
                    },
                    "item_id": { "type": "string" },
                    "start_sec": { "type": "number" },
                    "end_sec": { "type": "number" },
                    "step_frames": { "type": "integer" },
                    "initial_quad": {
                        "type": "array",
                        "description": "4 normalized [x, y] corners [TL, TR, BR, BL]"
                    },
                    "feather_px": { "type": "number" },
                    "snap_radius_px": { "type": "integer" },
                    "edge_weight": { "type": "number" },
                    "center_x": { "type": "number" },
                    "center_y": { "type": "number" },
                    "radius_x": { "type": "number" },
                    "radius_y": { "type": "number" },
                    "vertex_count": { "type": "integer" },
                    "invert": { "type": "boolean" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "svg_morph_text_path_and_warp",
            "description": "v8.0 #50: Import SVG <path> shapes & create/update equal-arclength path morphing items, enable tangent-following Text-on-Path on text items, or configure bicubic Grid Warp & luma displacement (1-Undo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list_presets", "add_svg_morph", "update_svg_morph", "set_text_on_path", "set_warp_displace"]
                    },
                    "item_id": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "preset_a": { "type": "string" },
                    "preset_b": { "type": "string" },
                    "path_a_d": { "type": "string" },
                    "path_b_d": { "type": "string" },
                    "morph_progress": { "type": "number" },
                    "auto_morph_hz": { "type": "number" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                    "stroke_width": { "type": "number" },
                    "svg_path_d": { "type": "string" },
                    "path_offset": { "type": "number" },
                    "animate_speed": { "type": "number" },
                    "follow_tangent": { "type": "boolean" },
                    "show_guide": { "type": "boolean" },
                    "warp_preset": { "type": "string" },
                    "amount": { "type": "number" },
                    "frequency": { "type": "number" },
                    "speed": { "type": "number" },
                    "luma_displace_px": { "type": "number" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "cinema_optical_vfx",
            "description": "v8.0 #51: Configure scene Cinema Optical VFX — Pseudo-Depth Rack Focus Bokeh, Volumetric God Rays, and Brown-Conrady Lens Distortion (k1, k2, p1, p2) + Physical Spectral Chromatic Aberration.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "set", "apply_preset", "disable"]
                    },
                    "preset": {
                        "type": "string",
                        "enum": ["anamorphic_rack_focus", "cathedral_god_rays", "action_cam_de_fish", "vintage_prism_lens"]
                    },
                    "enabled": { "type": "boolean" },
                    "bokeh_enabled": { "type": "boolean" },
                    "depth_mode": { "type": "string", "enum": ["vertical_tilt_shift", "radial_center", "luminance_saliency"] },
                    "focus_depth": { "type": "number" },
                    "focus_band": { "type": "number" },
                    "bokeh_radius_px": { "type": "number" },
                    "bokeh_highlight_boost": { "type": "number" },
                    "bokeh_blades": { "type": "integer" },
                    "god_rays_enabled": { "type": "boolean" },
                    "light_x": { "type": "number" },
                    "light_y": { "type": "number" },
                    "god_rays_intensity": { "type": "number" },
                    "god_rays_decay": { "type": "number" },
                    "god_rays_threshold": { "type": "number" },
                    "lens_enabled": { "type": "boolean" },
                    "k1": { "type": "number" },
                    "k2": { "type": "number" },
                    "p1": { "type": "number" },
                    "p2": { "type": "number" },
                    "chromatic_aberration": { "type": "number" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "audio_restoration_adr_spatial_bus",
            "description": "v8.0 #52: Apply Audio Restoration DSP (50/60Hz harmonic notch Hum Removal, MAD De-Click, Adaptive De-Reverb), manage ADR 3-2-1 Countdown Cues & Sync Beeps, and encode/decode ITU-R BS.775 5.1 Surround or FOA Ambisonics (AmbiX W,Y,Z,X).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_restoration_fx", "test_restoration_dsp", "add_adr_cue", "place_adr_timeline", "export_adr_beeps_wav", "set_spatial_bus", "export_spatial_bus_wav", "get_state"]
                    },
                    "item_id": { "type": "string" },
                    "fx_kind": { "type": "string", "enum": ["hum_remove", "declick", "dereverb"] },
                    "base_hz": { "type": "number" },
                    "harmonics": { "type": "integer" },
                    "q": { "type": "number" },
                    "sensitivity": { "type": "number" },
                    "strength": { "type": "number" },
                    "decay_ms": { "type": "number" },
                    "character": { "type": "string" },
                    "line_text": { "type": "string" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "preroll_sec": { "type": "number" },
                    "subtitle_layer": { "type": "integer" },
                    "mode": { "type": "string", "enum": ["stereo", "surround51", "ambisonics_foa_ambi_x"] },
                    "azimuth_deg": { "type": "number" },
                    "elevation_deg": { "type": "number" },
                    "lfe_crossover_hz": { "type": "number" },
                    "head_yaw_deg": { "type": "number" },
                    "head_pitch_deg": { "type": "number" },
                    "output_path": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
    ]
}

pub(crate) fn try_call(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    if !matches!(
        name,
        "planar_track_and_roto_brush"
            | "svg_morph_text_path_and_warp"
            | "cinema_optical_vfx"
            | "audio_restoration_adr_spatial_bus"
    ) {
        return None;
    }
    Some(call_inner(guard, fps, name, args))
}

fn parse_svg_preset(s: &str) -> Option<SvgShapePreset> {
    match s.to_ascii_lowercase().as_str() {
        "lightning" | "lightning_bolt" => Some(SvgShapePreset::LightningBolt),
        "heart" => Some(SvgShapePreset::Heart),
        "play" | "play_badge" => Some(SvgShapePreset::PlayBadge),
        "crown" => Some(SvgShapePreset::Crown),
        "wave" | "wave_ribbon" => Some(SvgShapePreset::WaveRibbon),
        "arch" | "arch_curve" => Some(SvgShapePreset::ArchCurve),
        _ => None,
    }
}

fn parse_quad_arg(args: &Value) -> [[f32; 2]; 4] {
    let default = PlanarTrackConfig::default().initial_quad;
    let Some(arr) = args.get("initial_quad").and_then(Value::as_array) else {
        return default;
    };
    if arr.len() != 4 {
        return default;
    }
    let mut out = default;
    for (i, pt) in arr.iter().enumerate() {
        if let Some(pair) = pt.as_array()
            && pair.len() >= 2
            && let (Some(x), Some(y)) = (pair[0].as_f64(), pair[1].as_f64())
        {
            out[i] = [x.clamp(-0.5, 1.5) as f32, y.clamp(-0.5, 1.5) as f32];
        }
    }
    out
}

fn call_inner(
    guard: &mut crate::session::EditorSession,
    _fps: FrameRate,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "planar_track_and_roto_brush" => handle_planar_roto(guard, args),
        "svg_morph_text_path_and_warp" => handle_svg_morph_warp(guard, args),
        "cinema_optical_vfx" => handle_optical_vfx(guard, args),
        "audio_restoration_adr_spatial_bus" => handle_audio_restoration_adr(guard, args),
        _ => Err(format!("unknown v8.0 tool: {name}")),
    }
}

fn handle_planar_roto(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;
    let active_id = guard.project.active_scene_id;

    match action {
        "track_planar" | "track_and_bake_corner_pin" => {
            let cfg = PlanarTrackConfig {
                start_time: RationalTime::from_seconds(finite_clamped(
                    args.get("start_sec"),
                    0.0,
                    0.0,
                    3600.0,
                )),
                end_time: RationalTime::from_seconds(finite_clamped(
                    args.get("end_sec"),
                    1.0,
                    0.05,
                    3600.0,
                )),
                step_frames: args
                    .get("step_frames")
                    .and_then(Value::as_u64)
                    .unwrap_or(2)
                    .clamp(1, 30) as u32,
                initial_quad: parse_quad_arg(args),
                ..PlanarTrackConfig::default()
            };
            let proj_snap = guard.project.clone();
            let mut rg = RenderGraph::new();
            let res = track_planar_quad_in_scene(&mut rg, &proj_snap, active_id, &cfg);

            let mut baked = false;
            if action == "track_and_bake_corner_pin" {
                let item_id = parse_uuid(args, "item_id")?;
                let item_start = guard
                    .project
                    .active_scene()
                    .and_then(|s| {
                        s.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find(|i| i.id == item_id)
                            .map(|i| i.start)
                    })
                    .ok_or_else(|| format!("item not found: {item_id}"))?;
                let feather = finite_clamped(args.get("feather_px"), 1.0, 0.0, 16.0) as f32;
                if let Some(cmd) =
                    build_planar_track_corner_pin_command(item_id, item_start, &res, feather)
                {
                    let s = &mut *guard;
                    s.bus
                        .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    baked = true;
                }
            }

            let text = serde_json::to_string_pretty(&json!({
                "action": action,
                "frame_count": res.frames.len(),
                "mean_confidence": res.mean_confidence,
                "mean_inliers": res.mean_inliers,
                "baked_corner_pin": baked,
                "first_quad": res.frames.first().map(|f| f.quad),
                "last_quad": res.frames.last().map(|f| f.quad),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "roto_snap" | "roto_propagate" => {
            let item_id = parse_uuid(args, "item_id")?;
            let item_start = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.start)
                })
                .ok_or_else(|| format!("item not found: {item_id}"))?;
            let cx = finite_clamped(args.get("center_x"), 0.5, 0.05, 0.95) as f32;
            let cy = finite_clamped(args.get("center_y"), 0.5, 0.05, 0.95) as f32;
            let rx = finite_clamped(args.get("radius_x"), 0.25, 0.04, 0.48) as f32;
            let ry = finite_clamped(args.get("radius_y"), 0.25, 0.04, 0.48) as f32;
            let vcnt = args
                .get("vertex_count")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(4, 24) as usize;
            let snap_r = args
                .get("snap_radius_px")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(2, 32) as u32;
            let edge_w = finite_clamped(args.get("edge_weight"), 0.8, 0.1, 0.98) as f32;
            let feather = finite_clamped(args.get("feather_px"), 2.0, 0.0, 32.0) as f32;
            let base_verts = RotoBrushConfig::default_ellipse_vertices(cx, cy, rx, ry, vcnt);

            let proj_snap = guard.project.clone();
            let mut rg = RenderGraph::new();
            let kfs = if action == "roto_snap" {
                let t = RationalTime::from_seconds(finite_clamped(
                    args.get("start_sec"),
                    guard.playhead.as_seconds(),
                    0.0,
                    3600.0,
                ));
                let img = rg
                    .render_project_scene_scaled(&proj_snap, active_id, t, 256, 144, false)
                    .ok_or_else(|| "scene render failed".to_string())?;
                let snapped = snap_roto_vertices_to_edges(&img, &base_verts, snap_r, edge_w);
                vec![RotoKeyframe {
                    time: (t - item_start).max(RationalTime::ZERO),
                    vertices: snapped,
                    feather_px: feather,
                    expansion_px: 0.0,
                }]
            } else {
                let t0 = RationalTime::from_seconds(finite_clamped(
                    args.get("start_sec"),
                    0.0,
                    0.0,
                    3600.0,
                ));
                let t1 = RationalTime::from_seconds(finite_clamped(
                    args.get("end_sec"),
                    1.0,
                    0.05,
                    3600.0,
                ));
                let step = args
                    .get("step_frames")
                    .and_then(Value::as_u64)
                    .unwrap_or(3)
                    .clamp(1, 30) as u32;
                propagate_roto_brush_in_scene(
                    &mut rg,
                    &proj_snap,
                    active_id,
                    item_start,
                    &base_verts,
                    t0,
                    t1,
                    step,
                    snap_r,
                    edge_w,
                    feather,
                )
            };

            let roto = RotoBrushConfig {
                enabled: args.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                invert: args.get("invert").and_then(Value::as_bool).unwrap_or(false),
                mode: MaskMode::Add,
                edge_snap_radius_px: snap_r,
                edge_weight: edge_w,
                keyframes: kfs.clone(),
            };
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetRotoBrush {
                        item_id,
                        roto_brush: roto,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;

            let text = serde_json::to_string_pretty(&json!({
                "action": action,
                "item_id": item_id.to_string(),
                "keyframe_count": kfs.len(),
                "vertex_count": vcnt,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "set_roto_brush" => {
            let item_id = parse_uuid(args, "item_id")?;
            let mut roto = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.roto_brush.clone())
                })
                .ok_or_else(|| format!("item not found: {item_id}"))?;
            if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                roto.enabled = en;
            }
            if let Some(inv) = args.get("invert").and_then(Value::as_bool) {
                roto.invert = inv;
            }
            if roto.keyframes.is_empty() && roto.enabled {
                roto.keyframes.push(RotoKeyframe {
                    time: RationalTime::ZERO,
                    vertices: RotoBrushConfig::default_ellipse_vertices(0.5, 0.5, 0.32, 0.32, 8),
                    feather_px: finite_clamped(args.get("feather_px"), 2.0, 0.0, 32.0) as f32,
                    expansion_px: 0.0,
                });
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetRotoBrush {
                        item_id,
                        roto_brush: roto.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "item_id": item_id.to_string(),
                "roto_brush": roto,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "get_roto_brush" => {
            let item_id = parse_uuid(args, "item_id")?;
            let roto = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.roto_brush.clone())
                })
                .ok_or_else(|| format!("item not found: {item_id}"))?;
            let text = serde_json::to_string_pretty(&roto).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}

fn handle_svg_morph_warp(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;

    match action {
        "list_presets" => {
            let svg_presets: Vec<Value> = SvgShapePreset::ALL
                .iter()
                .map(|p| {
                    json!({
                        "preset": p,
                        "label": p.display_name(),
                        "d": p.svg_path_d(),
                    })
                })
                .collect();
            let warp_presets: Vec<Value> = WarpPreset::ALL
                .iter()
                .map(|p| json!({ "preset": p, "label": p.display_name() }))
                .collect();
            let text = serde_json::to_string_pretty(&json!({
                "svg_presets": svg_presets,
                "warp_presets": warp_presets,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "add_svg_morph" => {
            let path_a = args
                .get("path_a_d")
                .and_then(Value::as_str)
                .map(String::from)
                .or_else(|| {
                    args.get("preset_a")
                        .and_then(Value::as_str)
                        .and_then(parse_svg_preset)
                        .map(|p| p.svg_path_d().to_string())
                })
                .unwrap_or_else(|| SvgShapePreset::LightningBolt.svg_path_d().to_string());
            let path_b = args
                .get("path_b_d")
                .and_then(Value::as_str)
                .map(String::from)
                .or_else(|| {
                    args.get("preset_b")
                        .and_then(Value::as_str)
                        .and_then(parse_svg_preset)
                        .map(|p| p.svg_path_d().to_string())
                })
                .unwrap_or_else(|| SvgShapePreset::Heart.svg_path_d().to_string());
            // パス構文を事前検証
            let (va, closed_a) = parse_svg_path_to_vertices(&path_a)?;
            if !path_b.trim().is_empty() {
                let _ = parse_svg_path_to_vertices(&path_b)?;
            }

            let cfg = SvgMorphConfig {
                width: finite_clamped(args.get("width"), 420.0, 32.0, 4096.0) as f32,
                height: finite_clamped(args.get("height"), 420.0, 32.0, 4096.0) as f32,
                path_a_d: path_a,
                path_b_d: path_b,
                morph_progress: finite_clamped(args.get("morph_progress"), 0.0, 0.0, 1.0) as f32,
                auto_morph_hz: finite_clamped(args.get("auto_morph_hz"), 0.5, 0.0, 8.0) as f32,
                fill: VectorFillMode::default(),
                stroke_color: RgbaColor::WHITE,
                stroke_width: finite_clamped(args.get("stroke_width"), 5.0, 0.0, 64.0) as f32,
                trim_start: 0.0,
                trim_end: 1.0,
                sample_count: 72,
            };
            let layer_index = args.get("layer_index").and_then(Value::as_u64).unwrap_or(1) as usize;
            let start =
                RationalTime::from_seconds(finite_clamped(args.get("start_sec"), 0.0, 0.0, 3600.0));
            let dur = RationalTime::from_seconds(finite_clamped(
                args.get("duration_sec"),
                5.0,
                0.1,
                3600.0,
            ));
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("SVG Morph")
                .to_string();
            let item = TimelineItem::new(
                name,
                start,
                dur,
                ItemKind::SvgMorph {
                    config: Box::new(cfg),
                },
            );
            let id = item.id;
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::AddObject { layer_index, item },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "item_id": id.to_string(),
                "vertices_a": va.len(),
                "closed_a": closed_a,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "update_svg_morph" => {
            let item_id = parse_uuid(args, "item_id")?;
            let mut cfg = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .and_then(|i| match &i.kind {
                            ItemKind::SvgMorph { config } => Some((**config).clone()),
                            _ => None,
                        })
                })
                .ok_or_else(|| format!("SvgMorph item not found: {item_id}"))?;
            if let Some(d) = args.get("path_a_d").and_then(Value::as_str) {
                let _ = parse_svg_path_to_vertices(d)?;
                cfg.path_a_d = d.to_string();
            }
            if let Some(d) = args.get("path_b_d").and_then(Value::as_str) {
                if !d.trim().is_empty() {
                    let _ = parse_svg_path_to_vertices(d)?;
                }
                cfg.path_b_d = d.to_string();
            }
            if let Some(v) = args.get("morph_progress").and_then(Value::as_f64) {
                cfg.morph_progress = v.clamp(0.0, 1.0) as f32;
            }
            if let Some(v) = args.get("auto_morph_hz").and_then(Value::as_f64) {
                cfg.auto_morph_hz = v.clamp(0.0, 8.0) as f32;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetSvgMorph {
                        item_id,
                        config: Box::new(cfg.clone()),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "set_text_on_path" => {
            let item_id = parse_uuid(args, "item_id")?;
            let mut top = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.text_on_path.clone())
                })
                .ok_or_else(|| format!("item not found: {item_id}"))?;
            top.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if let Some(d) = args.get("svg_path_d").and_then(Value::as_str) {
                let _ = parse_svg_path_to_vertices(d)?;
                top.svg_path_d = d.to_string();
            } else if let Some(p) = args
                .get("preset_a")
                .and_then(Value::as_str)
                .and_then(parse_svg_preset)
            {
                top.svg_path_d = p.svg_path_d().to_string();
            }
            if let Some(v) = args.get("path_offset").and_then(Value::as_f64) {
                top.path_offset = v as f32;
            }
            if let Some(v) = args.get("animate_speed").and_then(Value::as_f64) {
                top.animate_speed = v as f32;
            }
            if let Some(b) = args.get("follow_tangent").and_then(Value::as_bool) {
                top.follow_tangent = b;
            }
            if let Some(b) = args.get("show_guide").and_then(Value::as_bool) {
                top.show_guide = b;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetTextOnPath {
                        item_id,
                        text_on_path: top.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&top).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "set_warp_displace" => {
            let item_id = parse_uuid(args, "item_id")?;
            let mut warp = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.warp_displace.clone())
                })
                .ok_or_else(|| format!("item not found: {item_id}"))?;
            warp.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if let Some(ps) = args.get("warp_preset").and_then(Value::as_str) {
                warp.preset = match ps.to_ascii_lowercase().as_str() {
                    "custom_grid" | "grid" => WarpPreset::CustomGrid,
                    "flag_wave" | "flag" => WarpPreset::FlagWave,
                    "bulge_lens" | "bulge" => WarpPreset::BulgeLens,
                    "twirl_vortex" | "twirl" => WarpPreset::TwirlVortex,
                    "perspective_arch" | "arch" => WarpPreset::PerspectiveArch,
                    "ripple_water" | "ripple" => WarpPreset::RippleWater,
                    _ => warp.preset,
                };
            }
            if let Some(v) = args.get("amount").and_then(Value::as_f64) {
                warp.amount = v as f32;
            }
            if let Some(v) = args.get("frequency").and_then(Value::as_f64) {
                warp.frequency = v as f32;
            }
            if let Some(v) = args.get("speed").and_then(Value::as_f64) {
                warp.speed = v as f32;
            }
            if let Some(v) = args.get("luma_displace_px").and_then(Value::as_f64) {
                warp.luma_displace_px = v as f32;
            }
            warp.ensure_grid_size();
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetWarpDisplace {
                        item_id,
                        warp_displace: warp.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&warp).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}

fn handle_optical_vfx(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;

    match action {
        "get" => {
            let cfg = guard
                .project
                .active_scene()
                .map(|s| s.optical_vfx.clone())
                .unwrap_or_default();
            let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "disable" => {
            let mut cfg = guard
                .project
                .active_scene()
                .map(|s| s.optical_vfx.clone())
                .unwrap_or_default();
            cfg.enabled = false;
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetOpticalVfx {
                        optical_vfx: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "apply_preset" => {
            let preset_str = args
                .get("preset")
                .and_then(Value::as_str)
                .unwrap_or("anamorphic_rack_focus");
            let preset = match preset_str {
                "cathedral_god_rays" => OpticalVfxPreset::CathedralGodRays,
                "action_cam_de_fish" => OpticalVfxPreset::ActionCamDeFish,
                "vintage_prism_lens" => OpticalVfxPreset::VintagePrismLens,
                _ => OpticalVfxPreset::AnamorphicRackFocus,
            };
            let cfg = OpticalVfxConfig::from_preset(preset);
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetOpticalVfx {
                        optical_vfx: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "set" => {
            let mut cfg = guard
                .project
                .active_scene()
                .map(|s| s.optical_vfx.clone())
                .unwrap_or_default();
            cfg.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if let Some(b) = args.get("bokeh_enabled").and_then(Value::as_bool) {
                cfg.bokeh_enabled = b;
            }
            if let Some(dm) = args.get("depth_mode").and_then(Value::as_str) {
                cfg.depth_mode = match dm {
                    "radial_center" => PseudoDepthMode::RadialCenter,
                    "luminance_saliency" => PseudoDepthMode::LuminanceSaliency,
                    _ => PseudoDepthMode::VerticalTiltShift,
                };
            }
            if let Some(v) = args.get("focus_depth").and_then(Value::as_f64) {
                cfg.focus_depth = v as f32;
            }
            if let Some(v) = args.get("focus_band").and_then(Value::as_f64) {
                cfg.focus_band = v as f32;
            }
            if let Some(v) = args.get("bokeh_radius_px").and_then(Value::as_f64) {
                cfg.bokeh_radius_px = v as f32;
            }
            if let Some(v) = args.get("bokeh_highlight_boost").and_then(Value::as_f64) {
                cfg.bokeh_highlight_boost = v as f32;
            }
            if let Some(v) = args.get("bokeh_blades").and_then(Value::as_u64) {
                cfg.bokeh_blades = v as u32;
            }
            if let Some(b) = args.get("god_rays_enabled").and_then(Value::as_bool) {
                cfg.god_rays_enabled = b;
            }
            if let Some(v) = args.get("light_x").and_then(Value::as_f64) {
                cfg.light_pos[0] = v as f32;
            }
            if let Some(v) = args.get("light_y").and_then(Value::as_f64) {
                cfg.light_pos[1] = v as f32;
            }
            if let Some(v) = args.get("god_rays_intensity").and_then(Value::as_f64) {
                cfg.god_rays_intensity = v as f32;
            }
            if let Some(v) = args.get("god_rays_decay").and_then(Value::as_f64) {
                cfg.god_rays_decay = v as f32;
            }
            if let Some(v) = args.get("god_rays_threshold").and_then(Value::as_f64) {
                cfg.god_rays_threshold = v as f32;
            }
            if let Some(b) = args.get("lens_enabled").and_then(Value::as_bool) {
                cfg.lens_enabled = b;
            }
            if let Some(v) = args.get("k1").and_then(Value::as_f64) {
                cfg.k1 = v as f32;
            }
            if let Some(v) = args.get("k2").and_then(Value::as_f64) {
                cfg.k2 = v as f32;
            }
            if let Some(v) = args.get("p1").and_then(Value::as_f64) {
                cfg.p1 = v as f32;
            }
            if let Some(v) = args.get("p2").and_then(Value::as_f64) {
                cfg.p2 = v as f32;
            }
            if let Some(v) = args.get("chromatic_aberration").and_then(Value::as_f64) {
                cfg.chromatic_aberration = v as f32;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetOpticalVfx {
                        optical_vfx: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}

fn handle_audio_restoration_adr(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;
    let sr = guard.project.settings.audio_sample_rate.max(8_000);

    match action {
        "add_restoration_fx" => {
            let kind = args
                .get("fx_kind")
                .and_then(Value::as_str)
                .unwrap_or("hum_remove");
            if !matches!(kind, "hum_remove" | "declick" | "dereverb") {
                return Err(format!("invalid fx_kind '{kind}'"));
            }
            let mut node = AudioEffectNode::with_defaults(kind);
            if let Some(v) = args.get("base_hz").and_then(Value::as_f64) {
                node.params.insert("base_hz".into(), v as f32);
            }
            if let Some(v) = args.get("harmonics").and_then(Value::as_u64) {
                node.params.insert("harmonics".into(), v as f32);
            }
            if let Some(v) = args.get("q").and_then(Value::as_f64) {
                node.params.insert("q".into(), v as f32);
            }
            if let Some(v) = args.get("sensitivity").and_then(Value::as_f64) {
                node.params.insert("sensitivity".into(), v as f32);
            }
            if let Some(v) = args.get("strength").and_then(Value::as_f64) {
                node.params.insert("strength".into(), v as f32);
            }
            if let Some(v) = args.get("decay_ms").and_then(Value::as_f64) {
                node.params.insert("decay_ms".into(), v as f32);
            }

            let target_item = if args.get("item_id").and_then(Value::as_str).is_some() {
                Some(parse_uuid(args, "item_id")?)
            } else {
                None
            };
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::AddAudioEffect {
                        item_id: target_item,
                        effect: node.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "added_effect": node,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "test_restoration_dsp" => {
            // 合成テスト信号（50Hz ハム＋クリック＋残響テイル）で 3 種修復 DSP を検証
            let frames = (sr as usize) / 2;
            let mut pcm = vec![0.0_f32; frames * 2];
            for i in 0..frames {
                let t = i as f32 / sr as f32;
                let hum = 0.25 * (std::f32::consts::TAU * 50.0 * t).sin()
                    + 0.12 * (std::f32::consts::TAU * 100.0 * t).sin();
                let voice = if t < 0.15 {
                    0.3 * (std::f32::consts::TAU * 440.0 * t).sin()
                } else {
                    0.18 * (-(t - 0.15) / 0.2).exp() * (std::f32::consts::TAU * 440.0 * t).sin()
                };
                pcm[i * 2] = hum + voice;
                pcm[i * 2 + 1] = hum + voice;
            }
            pcm[400 * 2] = 0.95;
            pcm[1200 * 2 + 1] = -0.95;

            let (notches, hum_before_db, hum_after_db) = apply_hum_removal(
                &mut pcm,
                2,
                sr,
                finite_clamped(args.get("base_hz"), 50.0, 40.0, 120.0) as f32,
                args.get("harmonics").and_then(Value::as_u64).unwrap_or(5) as usize,
                finite_clamped(args.get("q"), 24.0, 4.0, 80.0) as f32,
                1.0,
            );
            let declick_rep = apply_declick(
                &mut pcm,
                2,
                finite_clamped(args.get("sensitivity"), 4.0, 2.0, 10.0) as f32,
                8,
            );
            apply_adaptive_dereverb(
                &mut pcm,
                2,
                sr,
                finite_clamped(args.get("strength"), 0.65, 0.0, 1.0) as f32,
                finite_clamped(args.get("decay_ms"), 160.0, 30.0, 500.0) as f32,
                -24.0,
            );

            let text = serde_json::to_string_pretty(&json!({
                "hum_notches": notches,
                "hum_before_db": hum_before_db,
                "hum_after_db": hum_after_db,
                "clicks_repaired": declick_rep.clicks_repaired,
                "samples_interpolated": declick_rep.samples_interpolated,
                "peak_after_declick": declick_rep.peak_after,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "add_adr_cue" => {
            let mut adr = guard
                .project
                .active_scene()
                .map(|s| s.adr_session.clone())
                .unwrap_or_default();
            adr.enabled = true;
            let next_num = adr.cues.iter().map(|c| c.cue_number).max().unwrap_or(0) + 1;
            let mut cue = AdrCue::new(
                next_num,
                args.get("character")
                    .and_then(Value::as_str)
                    .unwrap_or("Actor A"),
                args.get("line_text")
                    .and_then(Value::as_str)
                    .unwrap_or("ADR 台詞キュー"),
                RationalTime::from_seconds(finite_clamped(args.get("start_sec"), 3.0, 0.0, 3600.0)),
                RationalTime::from_seconds(finite_clamped(
                    args.get("duration_sec"),
                    2.5,
                    0.2,
                    300.0,
                )),
            );
            cue.preroll_sec = finite_clamped(args.get("preroll_sec"), 3.0, 1.0, 5.0);
            adr.cues.push(cue.clone());
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAdrSession {
                        adr_session: adr.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "cue": cue,
                "total_cues": adr.cues.len(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "place_adr_timeline" => {
            let cues = guard
                .project
                .active_scene()
                .map(|s| s.adr_session.cues.clone())
                .unwrap_or_default();
            let sub_layer = args
                .get("subtitle_layer")
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize;
            let Some(cmd) = build_adr_cue_timeline_commands(&cues, sub_layer) else {
                return Err("no ADR cues to place".to_string());
            };
            let s = &mut *guard;
            s.bus
                .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "placed_cues": cues.len(),
                "subtitle_layer": sub_layer,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "export_adr_beeps_wav" => {
            let preroll = finite_clamped(args.get("preroll_sec"), 3.0, 1.0, 5.0);
            let pcm = synthesize_adr_preroll_beeps(sr, preroll);
            let out_path = args
                .get("output_path")
                .and_then(Value::as_str)
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    guard
                        .folder
                        .as_ref()
                        .map(|f| f.exports_dir().join("adr_preroll_beeps.wav"))
                })
                .unwrap_or_else(|| std::env::temp_dir().join("ve_adr_preroll_beeps.wav"));
            let bytes = write_multichannel_wav_file(&out_path, &pcm, 2, sr)?;
            let text = serde_json::to_string_pretty(&json!({
                "output_path": out_path.display().to_string(),
                "bytes": bytes,
                "sample_rate": sr,
                "preroll_sec": preroll,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "set_spatial_bus" => {
            let mut bus = guard
                .project
                .active_scene()
                .map(|s| s.spatial_bus.clone())
                .unwrap_or_default();
            if let Some(m) = args.get("mode").and_then(Value::as_str) {
                bus.mode = match m {
                    "surround51" => SpatialAudioBusMode::Surround51,
                    "ambisonics_foa_ambi_x" => SpatialAudioBusMode::AmbisonicsFoaAmbiX,
                    _ => SpatialAudioBusMode::Stereo,
                };
            }
            if let Some(v) = args.get("azimuth_deg").and_then(Value::as_f64) {
                bus.azimuth_deg = v as f32;
            }
            if let Some(v) = args.get("elevation_deg").and_then(Value::as_f64) {
                bus.elevation_deg = v as f32;
            }
            if let Some(v) = args.get("lfe_crossover_hz").and_then(Value::as_f64) {
                bus.lfe_crossover_hz = v as f32;
            }
            if let Some(v) = args.get("head_yaw_deg").and_then(Value::as_f64) {
                bus.head_yaw_deg = v as f32;
            }
            if let Some(v) = args.get("head_pitch_deg").and_then(Value::as_f64) {
                bus.head_pitch_deg = v as f32;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetSpatialBus {
                        spatial_bus: bus.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&bus).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "export_spatial_bus_wav" => {
            let bus = guard
                .project
                .active_scene()
                .map(|s| s.spatial_bus.clone())
                .unwrap_or_default();
            let graph = AudioGraph::from_project_scene(
                &guard.project,
                guard.project.active_scene_id,
                guard.folder.as_ref(),
                sr,
            );
            let dur = guard
                .project
                .active_scene()
                .map(|s| s.total_duration())
                .unwrap_or_else(|| RationalTime::from_seconds(1.0))
                .max(RationalTime::from_seconds(0.5))
                .min(RationalTime::from_seconds(30.0));
            let mut stereo = graph.render_range(RationalTime::ZERO, dur);
            if stereo.iter().all(|&v| v.abs() < 1e-6) {
                // 無音プロジェクトでも空間エンコード検証可能な 0.5s テストトーン生成
                let frames = (sr as usize) / 2;
                stereo = vec![0.0; frames * 2];
                for i in 0..frames {
                    let t = i as f32 / sr as f32;
                    let s = 0.25 * (std::f32::consts::TAU * 440.0 * t).sin();
                    stereo[i * 2] = s;
                    stereo[i * 2 + 1] = s;
                }
            }

            let out_path = args
                .get("output_path")
                .and_then(Value::as_str)
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    guard
                        .folder
                        .as_ref()
                        .map(|f| f.exports_dir().join("spatial_bus_master.wav"))
                })
                .unwrap_or_else(|| std::env::temp_dir().join("ve_spatial_bus_master.wav"));

            let (channels, bytes, rms_vec, downmix_frames) = match bus.mode {
                SpatialAudioBusMode::Surround51 => {
                    let s51 = encode_stereo_to_51_surround(&stereo, sr, &bus);
                    let rms = s51.channel_rms_db().to_vec();
                    let dm = downmix_51_to_stereo_itu(&s51);
                    let bytes = write_multichannel_wav_file(&out_path, &s51.samples, 6, sr)?;
                    (6u16, bytes, rms, dm.len() / 2)
                }
                SpatialAudioBusMode::AmbisonicsFoaAmbiX => {
                    let mut foa = encode_stereo_to_foa_ambix(&stereo, sr, &bus);
                    rotate_foa_ambix(&mut foa, bus.head_yaw_deg, bus.head_pitch_deg);
                    let rms = foa.channel_rms_db().to_vec();
                    let bin = decode_foa_ambix_to_binaural_stereo(&foa);
                    let bytes = write_multichannel_wav_file(&out_path, &foa.samples, 4, sr)?;
                    (4u16, bytes, rms, bin.len() / 2)
                }
                SpatialAudioBusMode::Stereo => {
                    let bytes = write_multichannel_wav_file(&out_path, &stereo, 2, sr)?;
                    (2u16, bytes, vec![], stereo.len() / 2)
                }
            };
            let text = serde_json::to_string_pretty(&json!({
                "mode": bus.mode,
                "channels": channels,
                "bytes": bytes,
                "channel_rms_db": rms_vec,
                "binaural_or_stereo_frames": downmix_frames,
                "output_path": out_path.display().to_string(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "get_state" => {
            let (adr, bus, master_fx) = guard
                .project
                .active_scene()
                .map(|s| {
                    (
                        s.adr_session.clone(),
                        s.spatial_bus.clone(),
                        s.master_audio_effects.clone(),
                    )
                })
                .unwrap_or_default();
            let text = serde_json::to_string_pretty(&json!({
                "adr_session": adr,
                "spatial_bus": bus,
                "master_audio_effects_count": master_fx.len(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}
