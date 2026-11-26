//! v9.0 #53〜#56 Live Broadcast, VTuber & Hardware Ecosystem MCP ツール群（4 ツール / 全 90 MCP ツール）:
//! 1. `live_switcher_and_broadcast` (#53)
//! 2. `vtuber_mesh_physics_and_vmc` (#54)
//! 3. `hardware_controller_osc_midi_deck` (#55)
//! 4. `smart_dam_search_and_relink` (#56)

use super::*;
use ve_core::{
    DamAssetRecord, HardwareControllerConfig, HardwareControllerPreset, HardwareDispatchOutcome,
    HardwareEvent, HardwareProtocol, OscArg, SmartBinRule, StingerKind, StreamProtocol,
    VmcTrackFrame, VtuberAvatarPreset, VtuberBlendShapes, VtuberMeshConfig,
    apply_vmc_osc_message_to_blendshapes, build_live_switcher_veproj_commands,
    build_relink_assets_command, build_vtuber_vmc_bake_command, compute_file_head_fingerprint,
    decode_osc_packet, encode_osc_message, evaluate_smart_bin, export_live_session_veproj_json,
    extract_visual_features_from_rgba8, find_similar_shots, map_hardware_event_to_outcome,
    relink_missing_assets_from_dir, simulate_verlet_hair_chain,
};
use ve_media::build_live_stream_ffmpeg_args;

pub(crate) const DOMAIN_ID: &str = "v90_broadcast_ecosystem";
pub(crate) const DOMAIN_LABEL: &str =
    "v9.0 Live Broadcast, VTuber, Hardware Bridge & Smart DAM (#53-#56)";

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "live_switcher_and_broadcast",
            "description": "v9.0 #53: Control 2-bus (PGM/PVW) live broadcast switcher with RTMP/RTMPS/SRT FFmpeg sidecar config, alpha stinger transitions, and simultaneous .veproj NLE timeline generation (1-Undo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_state", "configure", "trigger_take", "cut_to_channel", "bake_nle_timeline", "export_veproj", "preview_ffmpeg_args"]
                    },
                    "enabled": { "type": "boolean" },
                    "recording_active": { "type": "boolean" },
                    "show_tally_hud": { "type": "boolean" },
                    "channel": { "type": "integer" },
                    "preview_channel": { "type": "integer" },
                    "use_stinger": { "type": "boolean" },
                    "stinger_kind": { "type": "string", "enum": ["cut", "cross_dissolve", "diagonal_wipe", "slash_flash", "luma_swipe", "zoom_burst"] },
                    "stinger_duration_sec": { "type": "number" },
                    "timestamp_sec": { "type": "number" },
                    "note": { "type": "string" },
                    "protocol": { "type": "string", "enum": ["local_record_only", "rtmp", "rtmps", "srt"] },
                    "endpoint_url": { "type": "string" },
                    "video_bitrate_kbps": { "type": "integer" },
                    "srt_latency_ms": { "type": "integer" },
                    "stinger_layer_index": { "type": "integer" },
                    "output_path": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "vtuber_mesh_physics_and_vmc",
            "description": "v9.0 #54: Add/configure VTuber 2D triangular mesh avatar with blendshapes (blink/mouth/head yaw-pitch-roll), distance-constrained Verlet hair physics, and UDP OSC / VMC face tracking keyframe bake (1-Undo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_avatar", "update_avatar", "apply_vmc_osc", "bake_vmc_frames", "test_verlet_physics"]
                    },
                    "item_id": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "preset": { "type": "string", "enum": ["anime_cyan", "cyber_fox_violet", "mascot_cat_gold", "mecha_idol_white"] },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                    "blink_l": { "type": "number" },
                    "blink_r": { "type": "number" },
                    "mouth_open": { "type": "number" },
                    "mouth_smile": { "type": "number" },
                    "head_yaw_deg": { "type": "number" },
                    "head_pitch_deg": { "type": "number" },
                    "head_roll_deg": { "type": "number" },
                    "show_wireframe": { "type": "boolean" },
                    "wind_amp": { "type": "number" },
                    "vmc_blend_name": { "type": "string" },
                    "vmc_blend_value": { "type": "number" },
                    "sample_count": { "type": "integer" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "hardware_controller_osc_midi_deck",
            "description": "v9.0 #55: Configure and dispatch OSC (UDP), MIDI-over-UDP, and RFC 6455 Stream Deck WebSocket hardware controller bindings via CommandBus (1-Undo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_config", "apply_preset", "dispatch_event", "test_loopback_bridge"]
                    },
                    "preset": { "type": "string", "enum": ["colorist_console", "audio_fader_bank", "live_stream_deck", "full_broadcast_rig"] },
                    "enabled": { "type": "boolean" },
                    "protocol": { "type": "string", "enum": ["osc_udp", "midi_udp", "stream_deck_ws"] },
                    "pattern": { "type": "string" },
                    "value_01": { "type": "number" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "smart_dam_search_and_relink",
            "description": "v9.0 #56: Local Smart DAM — index assets (5 dominant colors, warmth, 16-D feature vector, BPM/LUFS, SHA-256 head fingerprint), evaluate Smart Bins, find cosine-similar shots, and batch auto-relink missing assets (1-Undo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_summary", "index_project_assets", "upsert_synthetic_record", "query_smart_bin", "find_similar", "relink_missing_from_dir"]
                    },
                    "bin_index": { "type": "integer" },
                    "min_warmth": { "type": "number" },
                    "max_warmth": { "type": "number" },
                    "min_bpm": { "type": "number" },
                    "max_bpm": { "type": "number" },
                    "tag_contains": { "type": "string" },
                    "target_hex_color": { "type": "string" },
                    "query_path": { "type": "string" },
                    "top_k": { "type": "integer" },
                    "search_dir": { "type": "string" },
                    "apply": { "type": "boolean" },
                    "file_name": { "type": "string" },
                    "file_path": { "type": "string" },
                    "rgb": { "type": "array", "items": { "type": "integer" } },
                    "bpm": { "type": "number" },
                    "lufs": { "type": "number" },
                    "tags": { "type": "array", "items": { "type": "string" } }
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
        "live_switcher_and_broadcast"
            | "vtuber_mesh_physics_and_vmc"
            | "hardware_controller_osc_midi_deck"
            | "smart_dam_search_and_relink"
    ) {
        return None;
    }
    Some(call_inner(guard, fps, name, args))
}

fn call_inner(
    guard: &mut crate::session::EditorSession,
    _fps: FrameRate,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "live_switcher_and_broadcast" => handle_live_switcher(guard, args),
        "vtuber_mesh_physics_and_vmc" => handle_vtuber(guard, args),
        "hardware_controller_osc_midi_deck" => handle_hardware_ctrl(guard, args),
        "smart_dam_search_and_relink" => handle_smart_dam(guard, args),
        _ => Err(format!("unknown v9.0 tool: {name}")),
    }
}

fn parse_stinger_kind(s: &str) -> Option<StingerKind> {
    match s.to_ascii_lowercase().as_str() {
        "cut" => Some(StingerKind::Cut),
        "cross_dissolve" => Some(StingerKind::CrossDissolve),
        "diagonal_wipe" => Some(StingerKind::DiagonalWipe),
        "slash_flash" => Some(StingerKind::SlashFlash),
        "luma_swipe" => Some(StingerKind::LumaSwipe),
        "zoom_burst" => Some(StingerKind::ZoomBurst),
        _ => None,
    }
}

fn parse_protocol(s: &str) -> Option<StreamProtocol> {
    match s.to_ascii_lowercase().as_str() {
        "local_record_only" => Some(StreamProtocol::LocalRecordOnly),
        "rtmp" => Some(StreamProtocol::Rtmp),
        "rtmps" => Some(StreamProtocol::Rtmps),
        "srt" => Some(StreamProtocol::Srt),
        _ => None,
    }
}

fn handle_live_switcher(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;
    let mut sw = guard
        .project
        .active_scene()
        .map(|s| s.live_switcher.clone())
        .ok_or_else(|| "no active scene".to_string())?;

    match action {
        "get_state" => {
            let text = serde_json::to_string_pretty(&sw).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "configure" => {
            if let Some(v) = args.get("enabled").and_then(Value::as_bool) {
                sw.enabled = v;
            }
            if let Some(v) = args.get("recording_active").and_then(Value::as_bool) {
                sw.recording_active = v;
            }
            if let Some(v) = args.get("show_tally_hud").and_then(Value::as_bool) {
                sw.show_tally_hud = v;
            }
            if let Some(ch) = args.get("preview_channel").and_then(Value::as_u64) {
                sw.preview_channel = (ch as usize).min(sw.sources.len().saturating_sub(1));
            }
            if let Some(sk) = args.get("stinger_kind").and_then(Value::as_str)
                && let Some(kind) = parse_stinger_kind(sk)
            {
                sw.stinger.kind = kind;
            }
            if let Some(d) = args.get("stinger_duration_sec").and_then(Value::as_f64) {
                sw.stinger.duration_sec = (d as f32).clamp(0.1, 3.0);
            }
            if let Some(p) = args.get("protocol").and_then(Value::as_str)
                && let Some(proto) = parse_protocol(p)
            {
                sw.stream_target.protocol = proto;
            }
            if let Some(ep) = args.get("endpoint_url").and_then(Value::as_str) {
                sw.stream_target.endpoint_url = ep.to_string();
            }
            if let Some(vb) = args.get("video_bitrate_kbps").and_then(Value::as_u64) {
                sw.stream_target.video_bitrate_kbps = (vb as u32).clamp(500, 50_000);
            }
            if let Some(lat) = args.get("srt_latency_ms").and_then(Value::as_u64) {
                sw.stream_target.srt_latency_ms = (lat as u32).clamp(20, 4000);
            }

            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetLiveSwitcher {
                        live_switcher: sw.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&sw).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "trigger_take" | "cut_to_channel" => {
            sw.enabled = true;
            let ts = finite_clamped(
                args.get("timestamp_sec"),
                guard.playhead.as_seconds(),
                0.0,
                86_400.0,
            );
            if action == "cut_to_channel" {
                let ch = args.get("channel").and_then(Value::as_u64).unwrap_or(0) as usize;
                sw.cut_to_channel(ch, ts);
            } else {
                if let Some(ch) = args.get("preview_channel").and_then(Value::as_u64) {
                    sw.preview_channel = (ch as usize).min(sw.sources.len().saturating_sub(1));
                }
                let use_stinger = args
                    .get("use_stinger")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let note = args
                    .get("note")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP Take");
                sw.trigger_take(ts, use_stinger, note);
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetLiveSwitcher {
                        live_switcher: sw.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "program_channel": sw.program_channel,
                "preview_channel": sw.preview_channel,
                "event_count": sw.events.len(),
                "transition_progress": sw.transition_progress,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "bake_nle_timeline" => {
            let layer_idx = args
                .get("stinger_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(3) as usize;
            let cmd = build_live_switcher_veproj_commands(&guard.project, &sw, layer_idx)
                .ok_or_else(|| "no live switch events recorded to bake".to_string())?;
            let s = &mut *guard;
            s.bus
                .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "baked_events": sw.events.len(),
                "stinger_layer_index": layer_idx,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "export_veproj" => {
            let layer_idx = args
                .get("stinger_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(3) as usize;
            let json_body = export_live_session_veproj_json(&guard.project, &sw, layer_idx)?;
            let out_path = args
                .get("output_path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .or_else(|| {
                    guard
                        .folder
                        .as_ref()
                        .map(|f| f.exports_dir().join("live_session_master.veproj.json"))
                })
                .unwrap_or_else(|| std::env::temp_dir().join("ve_live_session_master.veproj.json"));
            if let Some(parent) = out_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&out_path, &json_body).map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "output_path": out_path.display().to_string(),
                "bytes": json_body.len(),
                "event_count": sw.events.len(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "preview_ffmpeg_args" => {
            let iso = args
                .get("output_path")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let argv = build_live_stream_ffmpeg_args(&sw.stream_target, "", iso.as_deref())
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "protocol": sw.stream_target.protocol,
                "arg_count": argv.len(),
                "ffmpeg_args": argv,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}

fn parse_vtuber_preset(s: &str) -> Option<VtuberAvatarPreset> {
    match s.to_ascii_lowercase().as_str() {
        "anime_cyan" => Some(VtuberAvatarPreset::AnimeCyan),
        "cyber_fox_violet" => Some(VtuberAvatarPreset::CyberFoxViolet),
        "mascot_cat_gold" => Some(VtuberAvatarPreset::MascotCatGold),
        "mecha_idol_white" => Some(VtuberAvatarPreset::MechaIdolWhite),
        _ => None,
    }
}

fn apply_vtuber_args(cfg: &mut VtuberMeshConfig, args: &Value) {
    if let Some(p) = args.get("preset").and_then(Value::as_str)
        && let Some(pr) = parse_vtuber_preset(p)
    {
        cfg.preset = pr;
    }
    if let Some(v) = args.get("width").and_then(Value::as_f64) {
        cfg.width = (v as f32).clamp(64.0, 2048.0);
    }
    if let Some(v) = args.get("height").and_then(Value::as_f64) {
        cfg.height = (v as f32).clamp(64.0, 2048.0);
    }
    if let Some(v) = args.get("blink_l").and_then(Value::as_f64) {
        cfg.blend_shapes.blink_l = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(v) = args.get("blink_r").and_then(Value::as_f64) {
        cfg.blend_shapes.blink_r = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(v) = args.get("mouth_open").and_then(Value::as_f64) {
        cfg.blend_shapes.mouth_open = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(v) = args.get("mouth_smile").and_then(Value::as_f64) {
        cfg.blend_shapes.mouth_smile = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(v) = args.get("head_yaw_deg").and_then(Value::as_f64) {
        cfg.blend_shapes.head_yaw_deg = (v as f32).clamp(-35.0, 35.0);
    }
    if let Some(v) = args.get("head_pitch_deg").and_then(Value::as_f64) {
        cfg.blend_shapes.head_pitch_deg = (v as f32).clamp(-25.0, 25.0);
    }
    if let Some(v) = args.get("head_roll_deg").and_then(Value::as_f64) {
        cfg.blend_shapes.head_roll_deg = (v as f32).clamp(-25.0, 25.0);
    }
    if let Some(v) = args.get("show_wireframe").and_then(Value::as_bool) {
        cfg.show_wireframe = v;
    }
    if let Some(v) = args.get("wind_amp").and_then(Value::as_f64) {
        cfg.physics.wind_amp = (v as f32).clamp(0.0, 1.0);
    }
}

fn handle_vtuber(guard: &mut crate::session::EditorSession, args: &Value) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;

    match action {
        "add_avatar" => {
            let mut cfg = VtuberMeshConfig::default();
            apply_vtuber_args(&mut cfg, args);
            let layer_index = args.get("layer_index").and_then(Value::as_u64).unwrap_or(1) as usize;
            let start = RationalTime::from_seconds(finite_clamped(
                args.get("start_sec"),
                0.0,
                0.0,
                86_400.0,
            ));
            let dur = RationalTime::from_seconds(finite_clamped(
                args.get("duration_sec"),
                5.0,
                0.2,
                3600.0,
            ));
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("VTuber Avatar");
            let (cx, cy) = guard
                .project
                .active_scene()
                .map(|s| (s.width as f32 * 0.5, s.height as f32 * 0.5))
                .unwrap_or((960.0, 540.0));
            let mut item = TimelineItem::new(
                name,
                start,
                dur,
                ItemKind::VtuberAvatar {
                    config: Box::new(cfg.clone()),
                },
            );
            item.transform.x = cx;
            item.transform.y = cy;
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
                "preset": cfg.preset,
                "grid": [cfg.grid_cols, cfg.grid_rows],
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "update_avatar" | "apply_vmc_osc" => {
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
                            ItemKind::VtuberAvatar { config } => Some((**config).clone()),
                            _ => None,
                        })
                })
                .ok_or_else(|| format!("VtuberAvatar item not found: {item_id}"))?;

            if action == "apply_vmc_osc" {
                let bname = args
                    .get("vmc_blend_name")
                    .and_then(Value::as_str)
                    .unwrap_or("A");
                let bval = finite_clamped(args.get("vmc_blend_value"), 0.8, 0.0, 1.0) as f32;
                let pkt = encode_osc_message(
                    "/VMC/Ext/Blend/Val",
                    &[OscArg::String(bname.to_string()), OscArg::Float(bval)],
                );
                for msg in decode_osc_packet(&pkt) {
                    apply_vmc_osc_message_to_blendshapes(&msg, &mut cfg.blend_shapes);
                }
            } else {
                apply_vtuber_args(&mut cfg, args);
            }

            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetVtuberAvatar {
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
        "bake_vmc_frames" => {
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
            let n = args
                .get("sample_count")
                .and_then(Value::as_u64)
                .unwrap_or(6)
                .clamp(2, 60) as usize;
            let frames: Vec<VmcTrackFrame> = (0..n)
                .map(|i| {
                    let frac = i as f32 / (n - 1) as f32;
                    VmcTrackFrame {
                        time: item_start + RationalTime::from_seconds(f64::from(frac) * 2.0),
                        shapes: VtuberBlendShapes {
                            head_yaw_deg: (frac * std::f32::consts::TAU).sin() * 20.0,
                            head_pitch_deg: (frac * std::f32::consts::TAU).cos() * 8.0,
                            mouth_open: (frac * std::f32::consts::TAU * 2.0).sin().abs(),
                            blink_l: if i == n / 2 { 1.0 } else { 0.0 },
                            blink_r: if i == n / 2 { 1.0 } else { 0.0 },
                            ..VtuberBlendShapes::default()
                        },
                    }
                })
                .collect();
            let cmd = build_vtuber_vmc_bake_command(item_id, item_start, &frames)
                .ok_or_else(|| "failed to build VMC bake command".to_string())?;
            let s = &mut *guard;
            s.bus
                .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "baked_frames": frames.len(),
                "properties": ["vt_yaw", "vt_pitch", "vt_roll", "vt_mouth", "vt_blink_l", "vt_blink_r"],
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "test_verlet_physics" => {
            let cfg = ve_core::VerletChainConfig {
                wind_amp: finite_clamped(args.get("wind_amp"), 0.25, 0.0, 1.0) as f32,
                ..ve_core::VerletChainConfig::default()
            };
            let yaw = finite_clamped(args.get("head_yaw_deg"), 25.0, -35.0, 35.0) as f32;
            let chain = simulate_verlet_hair_chain(&cfg, yaw, 6.0, 1.0, 1.0);
            let text = serde_json::to_string_pretty(&json!({
                "node_count": chain.len(),
                "tip_offset": chain.last().copied().unwrap_or([0.0, 0.0]),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}

fn handle_hardware_ctrl(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;

    match action {
        "get_config" => {
            let text =
                serde_json::to_string_pretty(&guard.project.hardware_ctrl).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "apply_preset" => {
            let preset = match args
                .get("preset")
                .and_then(Value::as_str)
                .unwrap_or("full_broadcast_rig")
                .to_ascii_lowercase()
                .as_str()
            {
                "colorist_console" => HardwareControllerPreset::ColoristConsole,
                "audio_fader_bank" => HardwareControllerPreset::AudioFaderBank,
                "live_stream_deck" => HardwareControllerPreset::LiveStreamDeck,
                _ => HardwareControllerPreset::FullBroadcastRig,
            };
            let mut cfg = HardwareControllerConfig::from_preset(preset);
            cfg.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetHardwareController {
                        hardware_ctrl: cfg.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "preset": preset,
                "binding_count": cfg.bindings.len(),
                "ports": {
                    "osc_udp": cfg.osc_udp_port,
                    "midi_udp": cfg.midi_udp_port,
                    "stream_deck_ws": cfg.stream_deck_ws_port
                }
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "dispatch_event" => {
            let proto = match args
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("osc_udp")
                .to_ascii_lowercase()
                .as_str()
            {
                "midi_udp" => HardwareProtocol::MidiUdp,
                "stream_deck_ws" => HardwareProtocol::StreamDeckWs,
                _ => HardwareProtocol::OscUdp,
            };
            let pattern = args
                .get("pattern")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'pattern'".to_string())?
                .to_string();
            let val = finite_clamped(args.get("value_01"), 1.0, 0.0, 1.0) as f32;
            let ev = HardwareEvent {
                protocol: proto,
                pattern: pattern.clone(),
                value_01: val,
            };
            let outcome = map_hardware_event_to_outcome(
                &guard.project.hardware_ctrl,
                &ev,
                &guard.project,
                guard.playhead,
            );
            let mut applied_cmd = None;
            if let Some(HardwareDispatchOutcome::EditCommand(cmd)) = outcome {
                applied_cmd = Some(cmd.command_name().to_string());
                let s = &mut *guard;
                s.bus
                    .execute(&mut s.project, *cmd, CommandSource::Mcp, None)
                    .map_err(|e| e.to_string())?;
            }
            let text = serde_json::to_string_pretty(&json!({
                "protocol": proto,
                "pattern": pattern,
                "value_01": val,
                "applied_command": applied_cmd,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "test_loopback_bridge" => {
            let mut srv = ve_core::HardwareBridgeServer::bind_localhost(0, 0, 0)?;
            let udp = std::net::UdpSocket::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
            let pkt = encode_osc_message("/ve/color/saturation", &[OscArg::Float(0.75)]);
            udp.send_to(&pkt, ("127.0.0.1", srv.osc_port))
                .map_err(|e| e.to_string())?;
            udp.send_to(&[0xB0, 7, 96], ("127.0.0.1", srv.midi_port))
                .map_err(|e| e.to_string())?;
            std::thread::sleep(std::time::Duration::from_millis(10));
            let evs = srv.poll_events();
            let text = serde_json::to_string_pretty(&json!({
                "osc_port": srv.osc_port,
                "midi_port": srv.midi_port,
                "ws_port": srv.ws_port,
                "received_count": evs.len(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}

fn handle_smart_dam(
    guard: &mut crate::session::EditorSession,
    args: &Value,
) -> Result<Value, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'action'".to_string())?;

    match action {
        "get_summary" => {
            let dam = &guard.project.smart_dam;
            let text = serde_json::to_string_pretty(&json!({
                "record_count": dam.records.len(),
                "smart_bin_count": dam.smart_bins.len(),
                "smart_bins": dam.smart_bins.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "index_project_assets" => {
            let mut dam = guard.project.smart_dam.clone();
            let mut indexed = 0usize;
            for asset in &guard.project.assets {
                let p_opt = asset.fallback_absolute_path.as_deref();
                let (sz, hash) = p_opt
                    .and_then(|p| compute_file_head_fingerprint(p).ok())
                    .unwrap_or((0, String::new()));
                dam.upsert_record(DamAssetRecord {
                    asset_id: Some(asset.id),
                    file_path: p_opt
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| asset.name.clone()),
                    file_name: asset.name.clone(),
                    kind: asset.kind,
                    file_size_bytes: sz,
                    head_sha256: hash,
                    dominant_colors: Vec::new(),
                    avg_luma: 0.5,
                    avg_saturation: 0.3,
                    warmth_score: 0.0,
                    edge_energy: 0.2,
                    feature_vec: [0.25; 16],
                    bpm: None,
                    integrated_lufs: None,
                    duration_sec: None,
                    tags: Vec::new(),
                });
                indexed += 1;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetSmartDamIndex {
                        smart_dam: dam.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "indexed_count": indexed,
                "total_records": dam.records.len(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "upsert_synthetic_record" => {
            let mut dam = guard.project.smart_dam.clone();
            let fname = args
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or("shot_sample.mp4");
            let fpath = args
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or(fname);
            let rgb = args
                .get("rgb")
                .and_then(Value::as_array)
                .and_then(|a| {
                    if a.len() >= 3 {
                        Some([
                            a[0].as_u64().unwrap_or(128) as u8,
                            a[1].as_u64().unwrap_or(128) as u8,
                            a[2].as_u64().unwrap_or(128) as u8,
                        ])
                    } else {
                        None
                    }
                })
                .unwrap_or([240, 125, 45]);
            let mut rgba = vec![0u8; 16 * 16 * 4];
            for px in rgba.as_chunks_mut::<4>().0 {
                *px = [rgb[0], rgb[1], rgb[2], 255];
            }
            let (dom, luma, sat, warmth, edge, feat) =
                extract_visual_features_from_rgba8(16, 16, &rgba);
            let bpm = args.get("bpm").and_then(Value::as_f64).map(|v| v as f32);
            let lufs = args.get("lufs").and_then(Value::as_f64).map(|v| v as f32);
            let tags = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            let kind = if bpm.is_some() || lufs.is_some() {
                MediaKind::Audio
            } else {
                MediaKind::Video
            };
            let (sz, hash) = compute_file_head_fingerprint(std::path::Path::new(fpath))
                .unwrap_or((0, String::new()));

            dam.upsert_record(DamAssetRecord {
                asset_id: None,
                file_path: fpath.to_string(),
                file_name: fname.to_string(),
                kind,
                file_size_bytes: sz,
                head_sha256: hash,
                dominant_colors: dom,
                avg_luma: luma,
                avg_saturation: sat,
                warmth_score: warmth,
                edge_energy: edge,
                feature_vec: feat,
                bpm,
                integrated_lufs: lufs,
                duration_sec: Some(5.0),
                tags,
            });
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetSmartDamIndex {
                        smart_dam: dam.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&json!({
                "file_name": fname,
                "warmth_score": warmth,
                "total_records": dam.records.len(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "query_smart_bin" => {
            let dam = &guard.project.smart_dam;
            let rule = if let Some(idx) = args.get("bin_index").and_then(Value::as_u64) {
                dam.smart_bins
                    .get(idx as usize)
                    .cloned()
                    .ok_or_else(|| format!("smart bin index out of range: {idx}"))?
            } else {
                SmartBinRule {
                    name: "AdHoc Query".into(),
                    media_kind: None,
                    min_warmth: args
                        .get("min_warmth")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32),
                    max_warmth: args
                        .get("max_warmth")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32),
                    min_bpm: args
                        .get("min_bpm")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32),
                    max_bpm: args
                        .get("max_bpm")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32),
                    min_lufs: None,
                    max_lufs: None,
                    tag_contains: args
                        .get("tag_contains")
                        .and_then(Value::as_str)
                        .map(String::from),
                    target_hex_color: args
                        .get("target_hex_color")
                        .and_then(Value::as_str)
                        .map(String::from),
                    max_color_dist: 95.0,
                }
            };
            let hits = evaluate_smart_bin(dam, &rule);
            let text = serde_json::to_string_pretty(&json!({
                "bin_name": rule.name,
                "hit_count": hits.len(),
                "hits": hits.iter().map(|r| json!({
                    "file_name": r.file_name,
                    "file_path": r.file_path,
                    "warmth_score": r.warmth_score,
                    "bpm": r.bpm,
                    "integrated_lufs": r.integrated_lufs,
                })).collect::<Vec<_>>(),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "find_similar" => {
            let dam = &guard.project.smart_dam;
            let qpath = args
                .get("query_path")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'query_path'".to_string())?;
            let qrec = dam
                .records
                .iter()
                .find(|r| r.file_path == qpath || r.file_name == qpath)
                .ok_or_else(|| format!("record not found in Smart DAM: {qpath}"))?;
            let top_k = args.get("top_k").and_then(Value::as_u64).unwrap_or(5) as usize;
            let hits = find_similar_shots(dam, &qrec.feature_vec, Some(&qrec.file_path), top_k);
            let text = serde_json::to_string_pretty(&json!({
                "query": qrec.file_name,
                "hits": hits,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "relink_missing_from_dir" => {
            let dir = args
                .get("search_dir")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'search_dir'".to_string())?;
            let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
            let plan = relink_missing_assets_from_dir(
                &guard.project,
                &guard.project.smart_dam,
                std::path::Path::new(dir),
            );
            let mut applied = false;
            if apply && let Some(cmd) = build_relink_assets_command(&plan) {
                let s = &mut *guard;
                s.bus
                    .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                    .map_err(|e| e.to_string())?;
                applied = true;
            }
            let text = serde_json::to_string_pretty(&json!({
                "scanned_files": plan.scanned_files,
                "missing_assets": plan.missing_assets,
                "matched_count": plan.matches.len(),
                "applied": applied,
                "matches": plan.matches,
                "unresolved_names": plan.unresolved_names,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        other => Err(format!("unsupported action '{other}'")),
    }
}
