use crate::session::SharedEditorSession;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;
use ve_ai::{ApiProvider, CloudUploadGuard, import_tts_file_pair};
use ve_core::{
    AlignMode, AssetRef, AudioEffectNode, Camera3D, CharacterSpriteConfig, CommandSource,
    CubicBezier, EditCommand, EffectNode, FrameRate, Interpolation, ItemKind, Keyframe,
    LipSyncMode, MediaKind, MotionPreset, RationalTime, RgbaColor, ShapeKind, TextAnimator,
    TimelineItem, TrackMatte, TrackMatteMode, base64_encode, build_align_commands,
    build_close_gaps_commands, build_fit_duration_to_layer_commands, build_motion_preset_commands,
    build_replace_text_commands, search_text_items,
};
use ve_plugin_api::PluginRegistry;
use ve_render::{RenderGraph, validate_wgsl};

const MAX_BATCH_COMMANDS: usize = 500;
const ALLOWED_TTS_AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a", "aac"];

fn finite_clamped(val: Option<&Value>, default: f64, min: f64, max: f64) -> f64 {
    match val.and_then(Value::as_f64) {
        Some(v) if v.is_finite() => v.clamp(min, max),
        _ => default,
    }
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "get_project_summary",
            "description": "プロジェクトの解像度・FPS・尺・シーン・レイヤー概要を取得する",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_timeline_range",
            "description": "指定範囲（フレームまたは秒）に存在するタイムラインオブジェクト一覧を取得する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "start_frame": { "type": "integer" },
                    "end_frame": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "end_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "get_object_detail",
            "description": "指定IDのタイムラインオブジェクト詳細を取得する",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": { "id": { "type": "string" } }
            }
        },
        {
            "name": "get_selection",
            "description": "現在GUIで選択中のオブジェクトID一覧を取得する",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_playhead",
            "description": "現在の再生ヘッド位置（ticks / seconds / frame）を取得する",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "render_frame_preview",
            "description": "指定時刻のレンダリング結果をPNG画像(base64)として返却し、視覚確認を可能にする",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "frame": { "type": "integer" },
                    "seconds": { "type": "number" },
                    "scale": { "type": "number", "default": 0.5 }
                }
            }
        },
        {
            "name": "add_object",
            "description": "タイムラインへテキストまたは図形オブジェクトを追加する",
            "inputSchema": {
                "type": "object",
                "required": ["layer_index", "name", "kind"],
                "properties": {
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "kind": { "type": "string", "enum": ["text", "shape"] },
                    "content": { "type": "string" },
                    "font_size": { "type": "number" },
                    "width": { "type": "number" },
                    "height": { "type": "number" }
                }
            }
        },
        {
            "name": "delete_object",
            "description": "指定IDのオブジェクトを削除する",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": { "id": { "type": "string" } }
            }
        },
        {
            "name": "move_object",
            "description": "オブジェクトのレイヤーまたは開始位置を変更する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "target_layer_index"],
                "properties": {
                    "id": { "type": "string" },
                    "target_layer_index": { "type": "integer" },
                    "new_start_sec": { "type": "number" },
                    "new_start_frame": { "type": "integer" }
                }
            }
        },
        {
            "name": "split_object",
            "description": "指定時刻でオブジェクトを2つに分割する",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "split_at_sec": { "type": "number" },
                    "split_at_frame": { "type": "integer" }
                }
            }
        },
        {
            "name": "set_property",
            "description": "オブジェクトの Transform またはテキスト内容を更新する",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "opacity": { "type": "number" },
                    "content": { "type": "string" }
                }
            }
        },
        {
            "name": "add_effect",
            "description": "オブジェクトの個別エフェクトチェーンへエフェクトを追加する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "effect_type"],
                "properties": {
                    "id": { "type": "string" },
                    "effect_type": { "type": "string" },
                    "params": { "type": "object" }
                }
            }
        },
        {
            "name": "update_effect_params",
            "description": "オブジェクト上の既存エフェクトのパラメータを更新する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "effect_id", "params"],
                "properties": {
                    "id": { "type": "string" },
                    "effect_id": { "type": "string" },
                    "params": { "type": "object" }
                }
            }
        },
        {
            "name": "set_keyframe",
            "description": "オブジェクトのプロパティへキーフレーム（ベジェ曲線イージング対応）を設定する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "property", "value"],
                "properties": {
                    "id": { "type": "string" },
                    "property": { "type": "string" },
                    "time_sec": { "type": "number" },
                    "time_frame": { "type": "integer" },
                    "value": { "type": "number" },
                    "easing": { "type": "string", "enum": ["linear", "hold", "ease_out", "ease_in_out"] }
                }
            }
        },
        {
            "name": "precompose_selection",
            "description": "指定オブジェクト群をワンクリック結合（Smart Precompose）して新シーンへ移送する",
            "inputSchema": {
                "type": "object",
                "required": ["ids", "name"],
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "name": { "type": "string" },
                    "target_layer_index": { "type": "integer" }
                }
            }
        },
        {
            "name": "set_track_matte",
            "description": "指定レイヤーにトラックマット（alpha / alpha_inverted / luma / luma_inverted）を設定または解除する",
            "inputSchema": {
                "type": "object",
                "required": ["layer_index"],
                "properties": {
                    "layer_index": { "type": "integer" },
                    "mode": { "type": "string", "enum": ["alpha", "alpha_inverted", "luma", "luma_inverted"] },
                    "source_layer_index": { "type": "integer" }
                }
            }
        },
        {
            "name": "switch_scene",
            "description": "アクティブ編集シーンを切り替える（プリコンポーズ再編集・メイン復帰）",
            "inputSchema": {
                "type": "object",
                "required": ["scene_id"],
                "properties": { "scene_id": { "type": "string" } }
            }
        },
        {
            "name": "set_camera_3d",
            "description": "アクティブシーンの3Dカメラ位置・注視点・FOV・被写界深度を設定する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "target": { "type": "array", "items": { "type": "number" } },
                    "fov_deg": { "type": "number" },
                    "dof_enabled": { "type": "boolean" },
                    "focus_distance": { "type": "number" },
                    "aperture": { "type": "number" }
                }
            }
        },
        {
            "name": "add_character_sprite",
            "description": "指定音声レイヤーと自動連動するPNG差分立ち絵（自動口パク）オブジェクトを配置する",
            "inputSchema": {
                "type": "object",
                "required": ["layer_index", "name"],
                "properties": {
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "linked_audio_layer": { "type": "integer" },
                    "mode": { "type": "string", "enum": ["volume_rms", "vowel_phoneme"] },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "import_tts_file_pair",
            "description": "voice.wav と同名の .txt / .lab を自動取込し、音声・字幕・音素タイミングを一括配置する",
            "inputSchema": {
                "type": "object",
                "required": ["wav_path"],
                "properties": {
                    "wav_path": { "type": "string" },
                    "audio_layer_index": { "type": "integer" },
                    "subtitle_layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "estimate_transcription_cost",
            "description": "外部AI文字起こし（Gemini / Claude）送信前の概算トークンとコストを安全見積りする",
            "inputSchema": {
                "type": "object",
                "required": ["provider", "duration_sec"],
                "properties": {
                    "provider": { "type": "string", "enum": ["gemini", "claude"] },
                    "duration_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "list_plugins",
            "description": "登録済みエフェクト／ジェネレータプラグインのマニフェスト一覧を取得する",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "validate_custom_wgsl",
            "description": "カスタム WGSL シェーダー文字列を naga で静的検証し、エントリーポイントまたは構文エラーを返す",
            "inputSchema": {
                "type": "object",
                "required": ["source"],
                "properties": { "source": { "type": "string" } }
            }
        },
        {
            "name": "batch_execute",
            "description": "複数の EditCommand を1つの Undo 単位として原子的に実行する",
            "inputSchema": {
                "type": "object",
                "required": ["commands"],
                "properties": {
                    "label": { "type": "string" },
                    "commands": { "type": "array" }
                }
            }
        },
        {
            "name": "align_items",
            "description": "選択/指定アイテムの隙間詰め・等間隔整列・左揃え・右揃え・再生ヘッド吸着・参照レイヤーへの長さフィットを1 Undo単位で実行する",
            "inputSchema": {
                "type": "object",
                "required": ["mode", "ids"],
                "properties": {
                    "mode": { "type": "string", "enum": ["close_gaps", "align_left", "align_right", "snap_playhead", "fit_to_layer"] },
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "gap_sec": { "type": "number", "default": 0.0 },
                    "reference_layer_index": { "type": "integer", "default": 3 }
                }
            }
        },
        {
            "name": "search_replace_text",
            "description": "シーン内の全テキストオブジェクトから文字列を検索、または replacement 指定時は一括置換（1 Undo単位）を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" },
                    "replacement": { "type": "string" },
                    "case_sensitive": { "type": "boolean", "default": false }
                }
            }
        },
        {
            "name": "set_layer_settings",
            "description": "AviUtl2準拠のレイヤー一括設定（座標オフセット・音量・可視・ミュート・色タグ）を更新する",
            "inputSchema": {
                "type": "object",
                "required": ["layer_index"],
                "properties": {
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "visible": { "type": "boolean" },
                    "muted": { "type": "boolean" },
                    "offset_x": { "type": "number" },
                    "offset_y": { "type": "number" },
                    "volume": { "type": "number" },
                    "color_hex": { "type": "string" }
                }
            }
        },
        {
            "name": "manage_effect",
            "description": "オブジェクトのエフェクトチェーンの並び替え・有効切替・削除を行う",
            "inputSchema": {
                "type": "object",
                "required": ["id", "action"],
                "properties": {
                    "id": { "type": "string" },
                    "action": { "type": "string", "enum": ["reorder", "remove", "toggle"] },
                    "effect_id": { "type": "string" },
                    "from_index": { "type": "integer" },
                    "to_index": { "type": "integer" },
                    "enabled": { "type": "boolean" }
                }
            }
        },
        {
            "name": "add_audio_effect",
            "description": "クリップまたはシーンマスターへ音声DSPエフェクトを追加、または目標ラウドネス(LUFS)を設定する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "effect_type": { "type": "string" },
                    "params": { "type": "object" },
                    "target_lufs": { "type": "number" }
                }
            }
        },
        {
            "name": "apply_motion_preset",
            "description": "指定オブジェクトへ登場・退場イージングや衝撃シェイクのワンクリックプリセットを適用する",
            "inputSchema": {
                "type": "object",
                "required": ["id", "preset"],
                "properties": {
                    "id": { "type": "string" },
                    "preset": { "type": "string", "enum": ["slide_in_left", "slide_in_bottom", "fade_in_out", "pop_bounce", "zoom_in", "impact_shake"] }
                }
            }
        }
    ])
}

fn parse_time_arg(
    args: &Value,
    frame_key: &str,
    sec_key: &str,
    fps: FrameRate,
    default: RationalTime,
) -> RationalTime {
    if let Some(f) = args.get(frame_key).and_then(Value::as_i64) {
        RationalTime::from_frame(f, fps)
    } else if let Some(s) = args.get(sec_key).and_then(Value::as_f64) {
        RationalTime::from_seconds(s)
    } else {
        default
    }
}

fn parse_uuid(args: &Value, key: &str) -> Result<Uuid, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument '{key}'"))?;
    Uuid::parse_str(raw).map_err(|e| format!("invalid uuid for '{key}': {e}"))
}

fn parse_params_map(val: Option<&Value>) -> BTreeMap<String, f32> {
    let mut map = BTreeMap::new();
    if let Some(obj) = val.and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(n) = v.as_f64() {
                map.insert(k.clone(), n as f32);
            }
        }
    }
    map
}

pub fn call_tool(session: &SharedEditorSession, name: &str, args: &Value) -> Result<Value, String> {
    let mut guard = session
        .lock()
        .map_err(|_| "session lock poisoned".to_string())?;
    let fps = guard.project.settings.fps;

    match name {
        "get_project_summary" => {
            let scene_summaries: Vec<Value> = guard
                .project
                .scenes
                .iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "name": s.name,
                        "width": s.width,
                        "height": s.height,
                        "fps": s.fps.as_f64(),
                        "layer_count": s.layers.len(),
                        "duration_sec": s.total_duration().as_seconds(),
                    })
                })
                .collect();
            let text = serde_json::to_string_pretty(&json!({
                "name": guard.project.name,
                "active_scene_id": guard.project.active_scene_id,
                "undo_depth": guard.bus.undo_len(),
                "scenes": scene_summaries,
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "get_timeline_range" => {
            let start = parse_time_arg(args, "start_frame", "start_sec", fps, RationalTime::ZERO);
            let end = parse_time_arg(
                args,
                "end_frame",
                "end_sec",
                fps,
                RationalTime::from_seconds(3600.0),
            );
            let scene = guard
                .project
                .active_scene()
                .ok_or_else(|| "no active scene".to_string())?;
            let mut items = Vec::new();
            for (layer_idx, layer) in scene.layers.iter().enumerate() {
                for item in &layer.items {
                    if item.end() > start && item.start < end {
                        items.push(json!({
                            "layer_index": layer_idx,
                            "id": item.id,
                            "name": item.name,
                            "start_sec": item.start.as_seconds(),
                            "end_sec": item.end().as_seconds(),
                            "start_frame": item.start.to_frame_round(fps),
                            "end_frame": item.end().to_frame_round(fps),
                        }));
                    }
                }
            }
            let text = serde_json::to_string_pretty(&items).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "get_object_detail" => {
            let id = parse_uuid(args, "id")?;
            let scene = guard
                .project
                .active_scene()
                .ok_or_else(|| "no active scene".to_string())?;
            for (layer_idx, layer) in scene.layers.iter().enumerate() {
                if let Some(item) = layer.items.iter().find(|i| i.id == id) {
                    let text = serde_json::to_string_pretty(&json!({
                        "layer_index": layer_idx,
                        "item": item,
                    }))
                    .unwrap_or_default();
                    return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
                }
            }
            Err(format!("object not found: {id}"))
        }
        "get_selection" => {
            let text = serde_json::to_string_pretty(&guard.selected_items).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "get_playhead" => {
            let t = guard.playhead;
            let text = serde_json::to_string_pretty(&json!({
                "ticks": t.ticks,
                "seconds": t.as_seconds(),
                "frame": t.to_frame_floor(fps),
            }))
            .unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "render_frame_preview" => {
            let t = parse_time_arg(args, "frame", "seconds", fps, guard.playhead);
            let scale = finite_clamped(args.get("scale"), 0.5, 0.1, 1.0);
            let mut proj_clone = guard.project.clone();
            let active_id = proj_clone.active_scene_id;
            let (out_w, out_h) = {
                let s = proj_clone
                    .active_scene_mut()
                    .ok_or_else(|| "no active scene".to_string())?;
                s.width = ((s.width as f64) * scale).round().clamp(16.0, 4096.0) as u32;
                s.height = ((s.height as f64) * scale).round().clamp(16.0, 4096.0) as u32;
                (s.width, s.height)
            };
            drop(guard);

            let mut rg = RenderGraph::new();
            let linear = rg
                .render_project_scene(&proj_clone, active_id, t, false)
                .ok_or_else(|| "failed to render scene".to_string())?;
            let rgba8 = linear.to_rgba8_srgb();
            let png_bytes = encode_rgba8_to_png(out_w, out_h, &rgba8)?;
            let b64 = base64_encode(&png_bytes);
            Ok(json!({
                "content": [
                    {
                        "type": "image",
                        "data": b64,
                        "mimeType": "image/png"
                    }
                ]
            }))
        }
        "add_object" => {
            let layer_index = args
                .get("layer_index")
                .and_then(Value::as_u64)
                .ok_or("missing layer_index")? as usize;
            let item_name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("AI Object")
                .to_string();
            let start = RationalTime::from_seconds(finite_clamped(
                args.get("start_sec"),
                0.0,
                0.0,
                86_400.0,
            ));
            let duration = RationalTime::from_seconds(finite_clamped(
                args.get("duration_sec"),
                2.0,
                0.01,
                86_400.0,
            ));
            let kind_str = args.get("kind").and_then(Value::as_str).unwrap_or("text");
            let kind = match kind_str {
                "shape" => ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: finite_clamped(args.get("width"), 200.0, 1.0, 8192.0) as f32,
                    height: finite_clamped(args.get("height"), 100.0, 1.0, 8192.0) as f32,
                    color: RgbaColor::WHITE,
                },
                _ => ItemKind::Text {
                    content: args
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("テキスト")
                        .to_string(),
                    font_family: "sans-serif".to_string(),
                    font_size: finite_clamped(args.get("font_size"), 48.0, 4.0, 512.0) as f32,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            };
            let item = TimelineItem::new(item_name, start, duration, kind);
            let id = item.id;
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::AddObject { layer_index, item },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"created_id": id}).to_string() }] }),
            )
        }
        "delete_object" => {
            let id = parse_uuid(args, "id")?;
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::DeleteObject { item_id: id },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "deleted" }] }))
        }
        "move_object" => {
            let id = parse_uuid(args, "id")?;
            let target_layer_index = args
                .get("target_layer_index")
                .and_then(Value::as_u64)
                .ok_or("missing target_layer_index")? as usize;
            let new_start = parse_time_arg(
                args,
                "new_start_frame",
                "new_start_sec",
                fps,
                RationalTime::ZERO,
            );
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::MoveObject {
                        item_id: id,
                        target_layer_index,
                        new_start,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "moved" }] }))
        }
        "split_object" => {
            let id = parse_uuid(args, "id")?;
            let split_at =
                parse_time_arg(args, "split_at_frame", "split_at_sec", fps, guard.playhead);
            let right_id = Uuid::new_v4();
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SplitObject {
                        item_id: id,
                        split_at,
                        new_right_id: right_id,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"right_id": right_id}).to_string() }] }),
            )
        }
        "set_property" => {
            let id = parse_uuid(args, "id")?;
            let mut cmds = Vec::new();
            if let Some(content) = args.get("content").and_then(Value::as_str) {
                cmds.push(EditCommand::SetTextContent {
                    item_id: id,
                    content: content.to_string(),
                });
            }
            if args.get("x").is_some() || args.get("y").is_some() || args.get("opacity").is_some() {
                let mut tf = guard
                    .project
                    .active_scene()
                    .and_then(|s| {
                        s.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find(|i| i.id == id)
                            .map(|i| i.transform)
                    })
                    .ok_or_else(|| format!("object not found: {id}"))?;
                if let Some(x) = args.get("x").and_then(Value::as_f64) {
                    tf.x = x as f32;
                }
                if let Some(y) = args.get("y").and_then(Value::as_f64) {
                    tf.y = y as f32;
                }
                if let Some(o) = args.get("opacity").and_then(Value::as_f64) {
                    tf.opacity = o as f32;
                }
                cmds.push(EditCommand::SetTransform {
                    item_id: id,
                    transform: tf,
                });
            }
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::Batch {
                        label: "set_property".into(),
                        commands: cmds,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "updated" }] }))
        }
        "add_effect" => {
            let id = parse_uuid(args, "id")?;
            let effect_type = args
                .get("effect_type")
                .and_then(Value::as_str)
                .ok_or("missing effect_type")?
                .to_string();
            let effect_id = Uuid::new_v4();
            let effect = EffectNode {
                id: effect_id,
                effect_type,
                enabled: true,
                params: parse_params_map(args.get("params")),
            };
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::AddEffect {
                        item_id: id,
                        effect,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"effect_id": effect_id}).to_string() }] }),
            )
        }
        "update_effect_params" => {
            let id = parse_uuid(args, "id")?;
            let effect_id = parse_uuid(args, "effect_id")?;
            let params = parse_params_map(args.get("params"));
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::UpdateEffectParams {
                        item_id: id,
                        effect_id,
                        params,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "updated" }] }))
        }
        "set_keyframe" => {
            let id = parse_uuid(args, "id")?;
            let property = args
                .get("property")
                .and_then(Value::as_str)
                .ok_or("missing property")?
                .to_string();
            let value = args
                .get("value")
                .and_then(Value::as_f64)
                .ok_or("missing value")? as f32;
            let time = parse_time_arg(args, "time_frame", "time_sec", fps, RationalTime::ZERO);
            let interpolation = match args.get("easing").and_then(Value::as_str) {
                Some("hold") => Interpolation::Hold,
                Some("ease_out") => Interpolation::Bezier(CubicBezier::EASE_OUT),
                Some("ease_in_out") => Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                _ => Interpolation::Linear,
            };
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetKeyframe {
                        item_id: id,
                        property,
                        keyframe: Keyframe {
                            time,
                            value,
                            interpolation,
                        },
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "keyframe_set" }] }))
        }
        "precompose_selection" => {
            let ids_arr = args
                .get("ids")
                .and_then(Value::as_array)
                .ok_or("missing ids array")?;
            let mut item_ids = Vec::with_capacity(ids_arr.len());
            for v in ids_arr {
                let s = v.as_str().ok_or("id must be string")?;
                item_ids.push(Uuid::parse_str(s).map_err(|e| e.to_string())?);
            }
            let precomp_name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Precomp")
                .to_string();
            let target_layer_index = args
                .get("target_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let new_scene_id = Uuid::new_v4();
            let precomp_item_id = Uuid::new_v4();
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::PrecomposeSelection {
                        item_ids,
                        new_scene_id,
                        precomp_name,
                        precomp_item_id,
                        target_layer_index,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({"scene_id": new_scene_id, "precomp_item_id": precomp_item_id}).to_string()
                }]
            }))
        }
        "set_track_matte" => {
            let layer_index = args
                .get("layer_index")
                .and_then(Value::as_u64)
                .ok_or("missing layer_index")? as usize;
            let matte = if let (Some(mode_s), Some(src_idx)) = (
                args.get("mode").and_then(Value::as_str),
                args.get("source_layer_index").and_then(Value::as_u64),
            ) {
                let mode = match mode_s {
                    "alpha_inverted" => TrackMatteMode::AlphaInverted,
                    "luma" => TrackMatteMode::Luma,
                    "luma_inverted" => TrackMatteMode::LumaInverted,
                    _ => TrackMatteMode::Alpha,
                };
                Some(TrackMatte {
                    mode,
                    source_layer_index: src_idx as usize,
                })
            } else {
                None
            };
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetTrackMatte { layer_index, matte },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "track_matte_updated" }] }))
        }
        "switch_scene" => {
            let scene_id = parse_uuid(args, "scene_id")?;
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SwitchScene { scene_id },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "switched" }] }))
        }
        "set_camera_3d" => {
            let mut cam = guard
                .project
                .active_scene()
                .map(|s| s.camera)
                .unwrap_or_else(Camera3D::default);
            if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                cam.enabled = en;
            }
            if let Some(arr) = args.get("position").and_then(Value::as_array)
                && arr.len() == 3
            {
                cam.position = [
                    arr[0].as_f64().unwrap_or(0.0) as f32,
                    arr[1].as_f64().unwrap_or(0.0) as f32,
                    arr[2].as_f64().unwrap_or(-1000.0) as f32,
                ];
            }
            if let Some(arr) = args.get("target").and_then(Value::as_array)
                && arr.len() == 3
            {
                cam.target = [
                    arr[0].as_f64().unwrap_or(0.0) as f32,
                    arr[1].as_f64().unwrap_or(0.0) as f32,
                    arr[2].as_f64().unwrap_or(0.0) as f32,
                ];
            }
            if let Some(fov) = args.get("fov_deg").and_then(Value::as_f64)
                && fov.is_finite()
            {
                cam.fov_deg = fov.clamp(5.0, 150.0) as f32;
            }
            if let Some(dof) = args.get("dof_enabled").and_then(Value::as_bool) {
                cam.dof_enabled = dof;
            }
            if let Some(fd) = args.get("focus_distance").and_then(Value::as_f64)
                && fd.is_finite()
            {
                cam.focus_distance = fd.clamp(1.0, 100_000.0) as f32;
            }
            if let Some(ap) = args.get("aperture").and_then(Value::as_f64)
                && ap.is_finite()
            {
                cam.aperture = ap.clamp(0.0, 1.0) as f32;
            }
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetCamera3D { camera: cam },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": "camera_3d_updated" }] }))
        }
        "add_character_sprite" => {
            let layer_index = args
                .get("layer_index")
                .and_then(Value::as_u64)
                .ok_or("missing layer_index")? as usize;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("立ち絵")
                .to_string();
            let linked_audio_layer = args
                .get("linked_audio_layer")
                .and_then(Value::as_u64)
                .map(|v| v as usize);
            let mode = match args.get("mode").and_then(Value::as_str) {
                Some("vowel_phoneme") => LipSyncMode::VowelPhoneme,
                _ => LipSyncMode::default(),
            };
            let start = RationalTime::from_seconds(finite_clamped(
                args.get("start_sec"),
                0.0,
                0.0,
                86_400.0,
            ));
            let duration = RationalTime::from_seconds(finite_clamped(
                args.get("duration_sec"),
                5.0,
                0.01,
                86_400.0,
            ));
            let item = TimelineItem::new(
                name,
                start,
                duration,
                ItemKind::CharacterSprite {
                    config: CharacterSpriteConfig {
                        linked_audio_layer,
                        mode,
                        png_paths: BTreeMap::new(),
                        psd_config: None,
                        phoneme_override: Vec::new(),
                    },
                    width: 260.0,
                    height: 320.0,
                },
            );
            let id = item.id;
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::AddObject { layer_index, item },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"created_id": id}).to_string() }] }),
            )
        }
        "import_tts_file_pair" => {
            let wav_str = args
                .get("wav_path")
                .and_then(Value::as_str)
                .ok_or("missing wav_path")?;
            let wav_path = PathBuf::from(wav_str);
            let ext_ok = wav_path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ALLOWED_TTS_AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()));
            if !ext_ok || !wav_path.is_file() {
                return Err(format!(
                    "wav_path must point to an existing audio file ({ALLOWED_TTS_AUDIO_EXTS:?})"
                ));
            }
            let audio_layer = args
                .get("audio_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(3) as usize;
            let sub_layer = args
                .get("subtitle_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize;
            let start = RationalTime::from_seconds(finite_clamped(
                args.get("start_sec"),
                guard.playhead.as_seconds(),
                0.0,
                86_400.0,
            ));

            let bundle = import_tts_file_pair(&wav_path).map_err(|e| e.to_string())?;
            let asset_id = Uuid::new_v4();
            guard.project.assets.push(AssetRef {
                id: asset_id,
                name: wav_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "voice.wav".into()),
                kind: MediaKind::Audio,
                relative_path: None,
                fallback_absolute_path: Some(wav_path),
                collected: false,
            });

            let mut audio_item = TimelineItem::new(
                "TTS音声",
                start,
                bundle.duration,
                ItemKind::Audio {
                    asset_id,
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            );
            audio_item.phoneme_segments = bundle.phonemes.clone();
            let sub_item = TimelineItem::new(
                "TTS字幕",
                start,
                bundle.duration,
                ItemKind::Text {
                    content: bundle.subtitle_text.clone(),
                    font_family: "sans-serif".to_string(),
                    font_size: 46.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            );

            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::Batch {
                        label: "import_tts_file_pair".into(),
                        commands: vec![
                            EditCommand::AddObject {
                                layer_index: audio_layer,
                                item: audio_item,
                            },
                            EditCommand::AddObject {
                                layer_index: sub_layer,
                                item: sub_item,
                            },
                        ],
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "subtitle": bundle.subtitle_text,
                        "phoneme_count": bundle.phonemes.len(),
                        "duration_sec": bundle.duration.as_seconds()
                    }).to_string()
                }]
            }))
        }
        "estimate_transcription_cost" => {
            let provider = match args.get("provider").and_then(Value::as_str) {
                Some("claude") => ApiProvider::Claude,
                _ => ApiProvider::Gemini,
            };
            let duration_sec = finite_clamped(args.get("duration_sec"), 60.0, 0.0, 86_400.0);
            let est =
                CloudUploadGuard::default().estimate_audio_transcription(provider, duration_sec);
            let text = serde_json::to_string_pretty(&est).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "list_plugins" => {
            let manifests = PluginRegistry::with_builtins().list_manifests();
            let text = serde_json::to_string_pretty(&manifests).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "validate_custom_wgsl" => {
            let source = args
                .get("source")
                .and_then(Value::as_str)
                .ok_or("missing WGSL 'source'")?;
            match validate_wgsl(source) {
                Ok(entry_points) => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({"valid": true, "entry_points": entry_points}).to_string()
                    }]
                })),
                Err(err) => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({"valid": false, "error": err}).to_string()
                    }]
                })),
            }
        }
        "batch_execute" => {
            let label = args
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("mcp_batch")
                .to_string();
            let raw_cmds = args
                .get("commands")
                .ok_or("missing commands array")?
                .clone();
            let commands: Vec<EditCommand> =
                serde_json::from_value(raw_cmds).map_err(|e| format!("invalid commands: {e}"))?;
            if commands.len() > MAX_BATCH_COMMANDS {
                return Err(format!(
                    "batch_execute exceeds max limit of {MAX_BATCH_COMMANDS} commands"
                ));
            }
            let count = commands.len();
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::Batch { label, commands },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"executed": count}).to_string() }] }),
            )
        }
        "align_items" => {
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .ok_or("missing 'mode'")?;
            let raw_ids = args
                .get("ids")
                .and_then(Value::as_array)
                .ok_or("missing 'ids' array")?;
            let mut ids = Vec::with_capacity(raw_ids.len());
            for v in raw_ids {
                let s = v.as_str().ok_or("non-string id in 'ids'")?;
                ids.push(Uuid::parse_str(s).map_err(|e| format!("invalid uuid: {e}"))?);
            }
            let scene = guard
                .project
                .active_scene()
                .ok_or("no active scene")?
                .clone();
            let playhead = guard.playhead;
            let cmd = match mode {
                "close_gaps" => {
                    let gap_sec = finite_clamped(args.get("gap_sec"), 0.0, 0.0, 60.0);
                    build_close_gaps_commands(&scene, &ids, gap_sec)
                }
                "align_left" => build_align_commands(&scene, &ids, AlignMode::Left, playhead),
                "align_right" => build_align_commands(&scene, &ids, AlignMode::Right, playhead),
                "snap_playhead" => {
                    build_align_commands(&scene, &ids, AlignMode::Playhead, playhead)
                }
                "fit_to_layer" => {
                    let ref_layer = args
                        .get("reference_layer_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(3) as usize;
                    build_fit_duration_to_layer_commands(&scene, &ids, ref_layer)
                }
                other => return Err(format!("unknown align mode: {other}")),
            };
            let changed = if let Some(cmd) = cmd {
                let sess = &mut *guard;
                sess.bus
                    .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                    .map_err(|e| e.to_string())?;
                true
            } else {
                false
            };
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"mode": mode, "changed": changed}).to_string() }] }),
            )
        }
        "search_replace_text" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or("missing 'query'")?;
            let case_sensitive = args
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let scene = guard
                .project
                .active_scene()
                .ok_or("no active scene")?
                .clone();
            if let Some(replacement) = args.get("replacement").and_then(Value::as_str) {
                if let Some((count, cmd)) =
                    build_replace_text_commands(&scene, query, replacement, case_sensitive)
                {
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({"replaced": count}).to_string() }] }),
                    )
                } else {
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({"replaced": 0}).to_string() }] }),
                    )
                }
            } else {
                let matches: Vec<Value> = search_text_items(&scene, query, case_sensitive)
                    .into_iter()
                    .map(|m| {
                        json!({
                            "id": m.item_id,
                            "layer_index": m.layer_index,
                            "name": m.item_name,
                            "start_sec": m.start.as_seconds(),
                            "content": m.content,
                        })
                    })
                    .collect();
                let text = serde_json::to_string_pretty(&matches).unwrap_or_default();
                Ok(json!({ "content": [{ "type": "text", "text": text }] }))
            }
        }
        "set_layer_settings" => {
            let layer_index = args
                .get("layer_index")
                .and_then(Value::as_u64)
                .ok_or("missing 'layer_index'")? as usize;
            let cur = guard
                .project
                .active_scene()
                .and_then(|s| s.layers.get(layer_index))
                .ok_or("invalid layer_index")?
                .clone();
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or(cur.name);
            let visible = args
                .get("visible")
                .and_then(Value::as_bool)
                .unwrap_or(cur.visible);
            let muted = args
                .get("muted")
                .and_then(Value::as_bool)
                .unwrap_or(cur.muted);
            let offset_x = finite_clamped(
                args.get("offset_x"),
                f64::from(cur.offset_x),
                -4096.0,
                4096.0,
            ) as f32;
            let offset_y = finite_clamped(
                args.get("offset_y"),
                f64::from(cur.offset_y),
                -4096.0,
                4096.0,
            ) as f32;
            let volume = finite_clamped(args.get("volume"), f64::from(cur.volume), 0.0, 4.0) as f32;
            let color_tag = match args.get("color_hex").and_then(Value::as_str) {
                Some("") => None,
                Some(hex) => RgbaColor::from_hex(hex).or(cur.color_tag),
                None => cur.color_tag,
            };
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetLayerSettings {
                        layer_index,
                        name,
                        visible,
                        muted,
                        offset_x,
                        offset_y,
                        volume,
                        color_tag,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"layer_index": layer_index, "updated": true}).to_string() }] }),
            )
        }
        "manage_effect" => {
            let item_id = parse_uuid(args, "id")?;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            let cmd = match action {
                "reorder" => EditCommand::ReorderEffect {
                    item_id,
                    from_index: args
                        .get("from_index")
                        .and_then(Value::as_u64)
                        .ok_or("missing 'from_index'")? as usize,
                    to_index: args
                        .get("to_index")
                        .and_then(Value::as_u64)
                        .ok_or("missing 'to_index'")? as usize,
                },
                "remove" => EditCommand::RemoveEffect {
                    item_id,
                    effect_id: parse_uuid(args, "effect_id")?,
                },
                "toggle" => EditCommand::ToggleEffect {
                    item_id,
                    effect_id: parse_uuid(args, "effect_id")?,
                    enabled: args.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                },
                other => return Err(format!("unknown action: {other}")),
            };
            let sess = &mut *guard;
            sess.bus
                .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"ok": true, "action": action}).to_string() }] }),
            )
        }
        "add_audio_effect" => {
            let item_id = match args.get("id").and_then(Value::as_str) {
                Some(s) if !s.is_empty() => {
                    Some(Uuid::parse_str(s).map_err(|e| format!("invalid uuid: {e}"))?)
                }
                _ => None,
            };
            let mut cmds = Vec::new();
            if let Some(eff_type) = args.get("effect_type").and_then(Value::as_str) {
                let mut effect = AudioEffectNode::with_defaults(eff_type);
                for (k, v) in parse_params_map(args.get("params")) {
                    effect.params.insert(k, v);
                }
                cmds.push(EditCommand::AddAudioEffect { item_id, effect });
            }
            if let Some(lufs) = args.get("target_lufs").and_then(Value::as_f64)
                && lufs.is_finite()
            {
                cmds.push(EditCommand::SetTargetLufs {
                    target_lufs: Some(lufs.clamp(-36.0, -3.0) as f32),
                });
            }
            if cmds.is_empty() {
                return Err("provide 'effect_type' or 'target_lufs'".into());
            }
            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::Batch {
                        label: "mcp_audio_effect".into(),
                        commands: cmds,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "content": [{ "type": "text", "text": json!({"ok": true}).to_string() }] }))
        }
        "apply_motion_preset" => {
            let item_id = parse_uuid(args, "id")?;
            let preset_name = args
                .get("preset")
                .and_then(Value::as_str)
                .ok_or("missing 'preset'")?;
            let preset = match preset_name {
                "slide_in_left" => MotionPreset::SlideInLeft,
                "slide_in_bottom" => MotionPreset::SlideInBottom,
                "fade_in_out" => MotionPreset::FadeInOut,
                "pop_bounce" => MotionPreset::PopBounce,
                "zoom_in" => MotionPreset::ZoomIn,
                "impact_shake" => MotionPreset::ImpactShake,
                other => return Err(format!("unknown preset: {other}")),
            };
            let item = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .cloned()
                })
                .ok_or("item not found")?;
            let cmd = build_motion_preset_commands(&item, preset);
            let sess = &mut *guard;
            sess.bus
                .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            Ok(
                json!({ "content": [{ "type": "text", "text": json!({"applied": preset_name}).to_string() }] }),
            )
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn encode_rgba8_to_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::EditorSession;
    use std::sync::{Arc, Mutex};
    use ve_core::{Project, ProjectSettings};

    #[test]
    fn v03_mcp_tools_align_search_replace_layer_audio_and_preset() {
        let mut proj = Project::new("McpV03", ProjectSettings::default());
        let t1 = TimelineItem::new(
            "Sub1",
            RationalTime::from_seconds(0.0),
            RationalTime::from_seconds(1.5),
            ItemKind::Text {
                content: "誤字テスト1".into(),
                font_family: "sans-serif".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        let t2 = TimelineItem::new(
            "Sub2",
            RationalTime::from_seconds(5.0),
            RationalTime::from_seconds(1.5),
            ItemKind::Text {
                content: "誤字テスト2".into(),
                font_family: "sans-serif".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        let (id1, id2) = (t1.id, t2.id);
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .extend([t1, t2]);
        let session = Arc::new(Mutex::new(EditorSession::new(proj)));

        call_tool(
            &session,
            "align_items",
            &json!({"mode": "close_gaps", "ids": [id1.to_string(), id2.to_string()], "gap_sec": 0.5}),
        )
        .unwrap();
        {
            let g = session.lock().unwrap();
            assert!(
                (g.project.active_scene().unwrap().layers[1].items[1]
                    .start
                    .as_seconds()
                    - 2.0)
                    .abs()
                    < 1e-6
            );
        }

        call_tool(
            &session,
            "search_replace_text",
            &json!({"query": "誤字", "replacement": "修正済み"}),
        )
        .unwrap();
        call_tool(
            &session,
            "set_layer_settings",
            &json!({"layer_index": 1, "offset_x": 20.0, "volume": 0.8, "color_hex": "#33aa55"}),
        )
        .unwrap();
        call_tool(
            &session,
            "add_audio_effect",
            &json!({"effect_type": "limiter", "target_lufs": -14.0}),
        )
        .unwrap();
        call_tool(
            &session,
            "apply_motion_preset",
            &json!({"id": id1.to_string(), "preset": "slide_in_left"}),
        )
        .unwrap();

        let g = session.lock().unwrap();
        let scene = g.project.active_scene().unwrap();
        assert_eq!(scene.target_lufs, Some(-14.0));
        assert_eq!(scene.master_audio_effects.len(), 1);
        assert!((scene.layers[1].offset_x - 20.0).abs() < 1e-4);
        assert!(scene.layers[1].items[0].keyframes.contains_key("x"));
        if let ItemKind::Text { content, .. } = &scene.layers[1].items[0].kind {
            assert_eq!(content, "修正済みテスト1");
        } else {
            panic!("expected text");
        }
    }
}
