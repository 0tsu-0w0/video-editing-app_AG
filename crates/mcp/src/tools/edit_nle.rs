//! v7.0 #46: 内蔵 MCP ドメインモジュール — Edit & Timeline NLE

#[allow(unused_imports)]
use super::*;

pub(super) const DOMAIN_ID: &str = "edit_nle";
pub(super) const DOMAIN_LABEL: &str = "Edit & Timeline NLE";

pub(super) fn definitions() -> Vec<Value> {
    let mut out = Vec::new();
    if let Value::Array(items) = json!([
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
        }
    ]) {
        out.extend(items);
    }
    if let Value::Array(items) = json!([
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
            "name": "switch_scene",
            "description": "アクティブ編集シーンを切り替える（プリコンポーズ再編集・メイン復帰）",
            "inputSchema": {
                "type": "object",
                "required": ["scene_id"],
                "properties": { "scene_id": { "type": "string" } }
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
            "name": "auto_jump_cut",
            "description": "音声レイヤーの無音区間を自動検出し、連動レイヤーごと一括ジェットカット（1 Undo単位）を実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audio_layer_index": { "type": "integer", "default": 3 },
                    "rms_threshold": { "type": "number", "default": 0.025 },
                    "min_silence_sec": { "type": "number", "default": 0.45 },
                    "pad_before_sec": { "type": "number", "default": 0.08 },
                    "pad_after_sec": { "type": "number", "default": 0.12 },
                    "ripple_linked_layers": { "type": "array", "items": { "type": "integer" } }
                }
            }
        },
        {
            "name": "import_media",
            "description": "ローカルの動画・音声・画像・PSD素材を安全にタイムラインへ取り込む（拡張子検証・Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "manage_markers",
            "description": "タイムラインマーカーの追加・更新・削除・一覧・YouTubeチャプターテキスト出力を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": { "type": "string", "enum": ["list", "add", "update", "remove", "youtube_chapters"] },
                    "time_sec": { "type": "number" },
                    "label": { "type": "string" },
                    "color_hex": { "type": "string" },
                    "marker_id": { "type": "string" }
                }
            }
        },
        {
            "name": "set_scene_settings",
            "description": "アクティブシーンの解像度・FPS・縦動画プリセット(9:16等)を変更し、任意でアイテム座標を自動リフレームする（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "enum": ["hd720", "full_hd1080", "uhd4k", "shorts_9x16", "square_1x1"] },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "fps": { "type": "number" },
                    "rescale_items": { "type": "boolean", "default": true }
                }
            }
        }
    ]) {
        out.extend(items);
    }
    if let Value::Array(items) = json!([
        {
            "name": "import_script",
            "description": "CSV（話者,セリフ,表情,背景画像）または Markdown 対話台本から話者カラー付き字幕タイムラインを一括生成する（1ステップUndo）",
            "inputSchema": {
                "type": "object",
                "required": ["content"],
                "properties": {
                    "content": { "type": "string" },
                    "start_sec": { "type": "number" },
                    "chars_per_sec": { "type": "number", "default": 6.5 },
                    "pause_gap_sec": { "type": "number", "default": 0.25 },
                    "subtitle_layer": { "type": "integer", "default": 1 },
                    "show_speaker_prefix": { "type": "boolean", "default": false }
                }
            }
        },
        {
            "name": "import_subtitles_or_exo",
            "description": "SRT / ASS 字幕または AviUtl .exo データをタイムラインへ一括インポートする（1ステップUndo）",
            "inputSchema": {
                "type": "object",
                "required": ["format", "content"],
                "properties": {
                    "format": { "type": "string", "enum": ["srt", "ass", "exo"] },
                    "content": { "type": "string" },
                    "layer_index": { "type": "integer", "default": 1 }
                }
            }
        },
        {
            "name": "export_subtitles_or_otio",
            "description": "アクティブシーンを SRT / ASS 字幕または OpenTimelineIO (.otio JSON) へエクスポートする",
            "inputSchema": {
                "type": "object",
                "required": ["format"],
                "properties": {
                    "format": { "type": "string", "enum": ["srt", "ass", "otio"] },
                    "file_name": { "type": "string" }
                }
            }
        },
        {
            "name": "extract_highlights",
            "description": "音声エネルギー・字幕密度・強調キーワード・マーカーから見どころハイライト区間を自動抽出し、任意で 9:16 ショート動画シーンを自動生成する（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target_clip_count": { "type": "integer", "default": 3 },
                    "clip_duration_sec": { "type": "number", "default": 15.0 },
                    "audio_layer_index": { "type": "integer", "default": 3 },
                    "create_shorts_scene": { "type": "boolean", "default": false },
                    "candidate_index": { "type": "integer", "default": 0 },
                    "shorts_name": { "type": "string" }
                }
            }
        },
        {
            "name": "text_based_edit",
            "description": "テキストベース編集（文字起こし一覧・選択/検索句の連動リップルカット・フィラー自動除去・前後並べ替えを 1 ステップ Undo で実行）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "delete_segments", "remove_fillers", "reorder"]
                    },
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "id": { "type": "string" },
                    "query": { "type": "string" },
                    "direction": { "type": "string", "enum": ["up", "down"], "default": "up" },
                    "strip_words": { "type": "boolean", "default": false },
                    "linked_layers": { "type": "array", "items": { "type": "integer" } }
                }
            }
        },
        {
            "name": "sync_multicam",
            "description": "v1.0 #17: マルチカメラ音声相互相関（Cross-Correlation）による自動同期整列、およびライブスイッチングカット組み立てを実行する（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["auto_sync_audio", "apply_switch_cuts"]
                    },
                    "angle_layers": { "type": "array", "items": { "type": "integer" } },
                    "max_lag_sec": { "type": "number" },
                    "apply": { "type": "boolean" },
                    "manual_offsets": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "offset_sec": { "type": "number" }
                            }
                        }
                    },
                    "output_layer_index": { "type": "integer" },
                    "range_start_sec": { "type": "number" },
                    "range_end_sec": { "type": "number" },
                    "cuts": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "time_sec": { "type": "number" },
                                "active_angle": { "type": "integer" }
                            }
                        }
                    }
                }
            }
        },
        {
            "name": "nle_pro_edit",
            "description": "v1.0 #19: スリップ/ロール/スライドトリム、クリップグルーピング＆A/Vリンク解除、トラックミキサー（パン/ソロ/バス/センド）、オフラインボーカル＆BGMステム分離を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["slip_trim", "roll_trim", "slide_trim", "group", "unlink", "set_layer_mixer", "separate_stems"]
                    },
                    "id": { "type": "string" },
                    "ids": { "type": "array", "items": { "type": "string" } },
                    "left_item_id": { "type": "string" },
                    "right_item_id": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "delta_sec": { "type": "number" },
                    "volume": { "type": "number" },
                    "pan": { "type": "number" },
                    "solo": { "type": "boolean" },
                    "muted": { "type": "boolean" },
                    "bus": { "type": "string" },
                    "send_reverb": { "type": "number" },
                    "asset_id": { "type": "string" }
                }
            }
        },
        {
            "name": "kinetic_subtitles_and_localize",
            "description": "v2.1 #28: ワード単位キネティック字幕（カラオケ/ワードポップ/強調語パルス）の一括設定 ＆ 多言語字幕ローカライズCSVテーブル抽出・1-Undo言語一括切替を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["set_kinetic_style", "extract_localization_table", "apply_locale_translations"]
                    },
                    "id": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "mode": {
                        "type": "string",
                        "enum": ["word_pop_color", "karaoke_sweep", "progressive_reveal", "keyword_pulse"]
                    },
                    "active_color": { "type": "string" },
                    "inactive_color": { "type": "string" },
                    "emphasis_color": { "type": "string" },
                    "pop_scale": { "type": "number" },
                    "keywords": { "type": "array" },
                    "source_locale": { "type": "string" },
                    "target_locales": { "type": "array" },
                    "format": { "type": "string", "enum": ["json", "csv"] },
                    "locale": { "type": "string" },
                    "csv": { "type": "string" },
                    "translations": { "type": "object" }
                }
            }
        },
        {
            "name": "speed_ramp_and_scene_detect",
            "description": "v5.0 #39: クリップのスピードランプ（解析積分 Time-Remapping 曲線：バレットタイム・ヒーロー加速・滑らかスロー・カスタムノット）設定およびカラーヒストグラム χ² 距離による自動シーンカット検出＆1-Undo 自動分割を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["set_speed_ramp", "detect_scene_cuts"]
                    },
                    "id": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "preset": {
                        "type": "string",
                        "enum": ["uniform", "bullet_time_dip", "ramp_up_hero", "ease_in_slow_mo", "flash_burst"]
                    },
                    "smooth_blend": { "type": "boolean" },
                    "knots": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "norm_time": { "type": "number" },
                                "speed": { "type": "number" }
                            }
                        }
                    },
                    "sample_interval_sec": { "type": "number" },
                    "sensitivity": { "type": "number" },
                    "split_clips": { "type": "boolean" },
                    "target_layer_index": { "type": "integer" },
                    "apply": { "type": "boolean" }
                }
            }
        }
    ]) {
        out.extend(items);
    }
    out
}

pub(super) fn try_call(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    if !matches!(
        name,
        "get_project_summary"
            | "get_timeline_range"
            | "get_object_detail"
            | "get_selection"
            | "get_playhead"
            | "render_frame_preview"
            | "add_object"
            | "delete_object"
            | "move_object"
            | "split_object"
            | "set_property"
            | "switch_scene"
            | "batch_execute"
            | "align_items"
            | "search_replace_text"
            | "set_layer_settings"
            | "auto_jump_cut"
            | "import_media"
            | "manage_markers"
            | "set_scene_settings"
            | "import_script"
            | "import_subtitles_or_exo"
            | "export_subtitles_or_otio"
            | "extract_highlights"
            | "text_based_edit"
            | "sync_multicam"
            | "nle_pro_edit"
            | "kinetic_subtitles_and_localize"
            | "speed_ramp_and_scene_detect"
    ) {
        return None;
    }
    Some(call_inner(guard, fps, name, args))
}

fn call_inner(
    guard: &mut crate::session::EditorSession,
    fps: FrameRate,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
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
            let total_items: usize = scene.layers.iter().map(|l| l.items.len()).sum();
            if total_items >= 64 {
                let idx = ve_core::SceneIntervalIndex::build(scene);
                for hit in idx.query_range(start, end) {
                    let item = &scene.layers[hit.layer_index].items[hit.item_index];
                    items.push(json!({
                        "layer_index": hit.layer_index,
                        "id": item.id,
                        "name": item.name,
                        "start_sec": item.start.as_seconds(),
                        "end_sec": item.end().as_seconds(),
                        "start_frame": item.start.to_frame_round(fps),
                        "end_frame": item.end().to_frame_round(fps),
                    }));
                }
            } else {
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
            let proj_clone = guard.project.clone();
            let active_id = proj_clone.active_scene_id;
            let (out_w, out_h) = {
                let s = proj_clone
                    .active_scene()
                    .ok_or_else(|| "no active scene".to_string())?;
                (
                    ((s.width as f64) * scale).round().clamp(16.0, 4096.0) as u32,
                    ((s.height as f64) * scale).round().clamp(16.0, 4096.0) as u32,
                )
            };
            let _ = guard;

            let mut rg = RenderGraph::new();
            let linear = rg
                .render_project_scene_scaled(&proj_clone, active_id, t, out_w, out_h, false)
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
                "particle_emitter" => {
                    let preset = match args
                        .get("preset")
                        .and_then(Value::as_str)
                        .unwrap_or("confetti")
                    {
                        "snow" => ParticlePreset::Snow,
                        "sparks" => ParticlePreset::Sparks,
                        "light_orbs" => ParticlePreset::LightOrbs,
                        "speed_lines" => ParticlePreset::SpeedLines,
                        "action_lines" => ParticlePreset::ActionLines,
                        _ => ParticlePreset::Confetti,
                    };
                    ItemKind::ParticleEmitter {
                        config: ParticleConfig::for_preset(preset),
                    }
                }
                "vector_shape" => {
                    let config = match args.get("preset").and_then(Value::as_str).unwrap_or("star")
                    {
                        "arrow" => VectorShapeConfig::arrow(),
                        "underline_wave" => VectorShapeConfig::underline_wave(RgbaColor::WHITE),
                        "speech_bubble" => VectorShapeConfig::speech_bubble(),
                        "circle_ring" => VectorShapeConfig::circle_ring(),
                        _ => VectorShapeConfig::star(),
                    };
                    ItemKind::VectorShape { config }
                }
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
        "auto_jump_cut" => {
            let audio_layer_index = args
                .get("audio_layer_index")
                .and_then(Value::as_u64)
                .unwrap_or(3) as usize;
            let cfg = JumpCutConfig {
                rms_threshold: finite_clamped(args.get("rms_threshold"), 0.025, 0.001, 0.9) as f32,
                min_silence_sec: finite_clamped(args.get("min_silence_sec"), 0.45, 0.05, 10.0),
                pad_before_sec: finite_clamped(args.get("pad_before_sec"), 0.08, 0.0, 1.0),
                pad_after_sec: finite_clamped(args.get("pad_after_sec"), 0.12, 0.0, 1.0),
                ripple_linked_layers: args
                    .get("ripple_linked_layers")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_u64().map(|n| n as usize))
                            .collect()
                    })
                    .unwrap_or_else(|| vec![0, 1, 2]),
            };
            let scene = guard
                .project
                .active_scene()
                .ok_or("no active scene")?
                .clone();
            let total_dur = scene.total_duration();
            if total_dur <= RationalTime::ZERO {
                return Err("active scene has no media duration".into());
            }
            let sr = guard.project.settings.audio_sample_rate.max(8_000);
            let graph = AudioGraph::from_project_scene(
                &guard.project,
                guard.project.active_scene_id,
                guard.folder.as_ref(),
                sr,
            );
            let pcm = graph.render_range(RationalTime::ZERO, total_dur);
            let bucket_sec = 0.05;
            let bucket_frames = ((sr as f64) * bucket_sec).round().max(1.0) as usize;
            let rms_series: Vec<f32> = pcm
                .chunks(bucket_frames * 2)
                .map(|chunk| {
                    let mut sum = 0.0f64;
                    let mut n = 0usize;
                    for &[l, r] in chunk.as_chunks::<2>().0 {
                        let m = f64::from((l + r) * 0.5);
                        sum += m * m;
                        n += 1;
                    }
                    if n == 0 {
                        0.0
                    } else {
                        (sum / n as f64).sqrt() as f32
                    }
                })
                .collect();
            let intervals = detect_speech_intervals_from_rms(
                &rms_series,
                bucket_sec,
                total_dur.as_seconds(),
                &cfg,
            );
            let Some((kept_segments, removed_sec, cmd)) = build_jump_cut_commands(
                &scene,
                audio_layer_index,
                &intervals,
                &cfg.ripple_linked_layers,
            ) else {
                return Ok(json!({
                    "content": [{ "type": "text", "text": json!({"kept_segments": 0, "removed_sec": 0.0}).to_string() }]
                }));
            };
            let sess = &mut *guard;
            sess.bus
                .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({"kept_segments": kept_segments, "removed_sec": removed_sec}).to_string()
                }]
            }))
        }
        "import_media" => {
            let raw_path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or("missing 'path'")?;
            let path = PathBuf::from(raw_path);
            if !path.is_file() {
                return Err(format!("media file not found: {raw_path}"));
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Media".into());
            let start = parse_time_arg(args, "start_frame", "start_sec", fps, guard.playhead);
            let default_layer = match ext.as_str() {
                "mp4" | "mov" | "mkv" | "webm" | "avi" => 0,
                "wav" | "mp3" | "flac" | "ogg" | "m4a" | "aac" => 3,
                "psd" => 2,
                _ => 1,
            };
            let layer_index = args
                .get("layer_index")
                .and_then(Value::as_u64)
                .map(|v| v as usize)
                .unwrap_or(default_layer);
            let asset_id = Uuid::new_v4();
            let cli = FfmpegCli::default();

            let (media_kind, item) = match ext.as_str() {
                "mp4" | "mov" | "mkv" | "webm" | "avi" => {
                    let dur_sec = cli
                        .probe(&path)
                        .ok()
                        .map(|p| p.duration_sec)
                        .filter(|d| d.is_finite() && *d > 0.05)
                        .unwrap_or(5.0);
                    let dur = RationalTime::from_seconds(dur_sec);
                    let mut it = TimelineItem::new(
                        file_name.clone(),
                        start,
                        dur,
                        ItemKind::Video {
                            asset_id,
                            source_offset: RationalTime::ZERO,
                            volume: 1.0,
                        },
                    );
                    it.playback.source_duration = Some(dur);
                    (MediaKind::Video, it)
                }
                "wav" | "mp3" | "flac" | "ogg" | "m4a" | "aac" => {
                    let dur_sec = decode_audio_file(&path)
                        .ok()
                        .map(|b| b.duration_sec())
                        .or_else(|| cli.probe(&path).ok().map(|p| p.duration_sec))
                        .filter(|d| d.is_finite() && *d > 0.05)
                        .unwrap_or(3.0);
                    let dur = RationalTime::from_seconds(dur_sec);
                    let mut it = TimelineItem::new(
                        file_name.clone(),
                        start,
                        dur,
                        ItemKind::Audio {
                            asset_id,
                            source_offset: RationalTime::ZERO,
                            volume: 1.0,
                        },
                    );
                    it.playback.source_duration = Some(dur);
                    (MediaKind::Audio, it)
                }
                "png" | "jpg" | "jpeg" | "webp" | "bmp" => (
                    MediaKind::Image,
                    TimelineItem::new(
                        file_name.clone(),
                        start,
                        RationalTime::from_seconds(5.0),
                        ItemKind::Image { asset_id },
                    ),
                ),
                "psd" => (
                    MediaKind::Image,
                    TimelineItem::new(
                        file_name.clone(),
                        start,
                        RationalTime::from_seconds(5.0),
                        ItemKind::CharacterSprite {
                            config: CharacterSpriteConfig {
                                linked_audio_layer: Some(3),
                                mode: LipSyncMode::default(),
                                png_paths: BTreeMap::new(),
                                psd_config: Some(PsdCharacterConfig {
                                    psd_path: path.clone(),
                                    ..Default::default()
                                }),
                                phoneme_override: Vec::new(),
                            },
                            width: 300.0,
                            height: 400.0,
                        },
                    ),
                ),
                _ => return Err(format!("unsupported media extension: .{ext}")),
            };

            let item_id = item.id;
            let sess = &mut *guard;
            sess.project.assets.push(AssetRef {
                id: asset_id,
                name: file_name.clone(),
                kind: media_kind,
                relative_path: None,
                fallback_absolute_path: Some(path),
                collected: false,
            });
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::AddObject { layer_index, item },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{ "type": "text", "text": json!({"id": item_id, "asset_id": asset_id, "name": file_name}).to_string() }]
            }))
        }
        "manage_markers" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "list" => {
                    let markers = guard
                        .project
                        .active_scene()
                        .map(|s| s.markers.clone())
                        .unwrap_or_default();
                    let text = serde_json::to_string_pretty(&markers).unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "youtube_chapters" => {
                    let chapters = guard
                        .project
                        .active_scene()
                        .map(|s| format_youtube_chapters(&s.markers))
                        .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": chapters }] }))
                }
                "add" => {
                    let time = parse_time_arg(args, "time_frame", "time_sec", fps, guard.playhead);
                    let label = args
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("マーカー");
                    let mut marker = TimelineMarker::new(time, label);
                    if let Some(hex) = args.get("color_hex").and_then(Value::as_str)
                        && let Some(c) = RgbaColor::from_hex(hex)
                    {
                        marker.color = c;
                    }
                    let mid = marker.id;
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::AddMarker { marker },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({"marker_id": mid, "time_sec": time.as_seconds()}).to_string() }]
                    }))
                }
                "update" => {
                    let marker_id = parse_uuid(args, "marker_id")?;
                    let existing = guard
                        .project
                        .active_scene()
                        .and_then(|s| s.markers.iter().find(|m| m.id == marker_id).cloned())
                        .ok_or_else(|| format!("marker not found: {marker_id}"))?;
                    let label = args
                        .get("label")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or(existing.label);
                    let color = args
                        .get("color_hex")
                        .and_then(Value::as_str)
                        .and_then(RgbaColor::from_hex)
                        .unwrap_or(existing.color);
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::UpdateMarker {
                                marker_id,
                                label: label.clone(),
                                color,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({"updated": marker_id, "label": label, "color_hex": color.to_hex_rgb()}).to_string() }]
                    }))
                }
                "remove" => {
                    let marker_id = parse_uuid(args, "marker_id")?;
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::RemoveMarker { marker_id },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({"removed": marker_id}).to_string() }]
                    }))
                }
                other => Err(format!("unknown marker action: {other}")),
            }
        }
        "set_scene_settings" => {
            let cur = guard
                .project
                .active_scene()
                .ok_or_else(|| "no active scene".to_string())?;
            let (mut width, mut height, mut new_fps) = (cur.width, cur.height, cur.fps);
            if let Some(preset_str) = args.get("preset").and_then(Value::as_str) {
                let preset = match preset_str {
                    "hd720" => ve_core::ScenePreset::Hd720,
                    "full_hd1080" => ve_core::ScenePreset::FullHd1080,
                    "uhd4k" => ve_core::ScenePreset::Uhd4k,
                    "shorts_9x16" => ve_core::ScenePreset::Shorts9x16,
                    "square_1x1" => ve_core::ScenePreset::Square1x1,
                    other => return Err(format!("unknown preset: {other}")),
                };
                let (pw, ph) = preset.dimensions();
                width = pw;
                height = ph;
                new_fps = preset.default_fps();
            }
            if let Some(w) = args.get("width").and_then(Value::as_u64) {
                width = (w as u32).clamp(16, 8192);
            }
            if let Some(h) = args.get("height").and_then(Value::as_u64) {
                height = (h as u32).clamp(16, 8192);
            }
            if let Some(f) = args.get("fps").and_then(Value::as_f64) {
                new_fps = if (f - 23.976).abs() < 0.05 {
                    FrameRate::FPS_23_976
                } else if (f - 24.0).abs() < 0.1 {
                    FrameRate::FPS_24
                } else if (f - 29.97).abs() < 0.05 {
                    FrameRate::FPS_29_97
                } else if (f - 30.0).abs() < 0.1 {
                    FrameRate::FPS_30
                } else {
                    FrameRate::FPS_60
                };
            }
            let rescale_items = args
                .get("rescale_items")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetSceneSettings {
                        width,
                        height,
                        fps: new_fps,
                        rescale_items,
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "width": width,
                        "height": height,
                        "fps": new_fps.as_f64(),
                        "rescale_items": rescale_items
                    }).to_string()
                }]
            }))
        }
        "import_script" => {
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or("missing 'content'")?;
            let lines = parse_script_auto(content);
            if lines.is_empty() {
                return Err("台本から有効なセリフ行を検出できませんでした".to_string());
            }
            let (scene_w, scene_h, max_layer) = guard
                .project
                .active_scene()
                .map(|s| (s.width, s.height, s.layers.len().saturating_sub(1)))
                .ok_or_else(|| "no active scene".to_string())?;
            let cfg = ScriptBuildConfig {
                start_time: RationalTime::from_seconds(finite_clamped(
                    args.get("start_sec"),
                    guard.playhead.as_seconds(),
                    0.0,
                    86_400.0,
                )),
                chars_per_sec: finite_clamped(args.get("chars_per_sec"), 6.5, 1.0, 30.0),
                pause_gap_sec: finite_clamped(args.get("pause_gap_sec"), 0.25, 0.0, 10.0),
                subtitle_layer: (args
                    .get("subtitle_layer")
                    .and_then(Value::as_u64)
                    .unwrap_or(1) as usize)
                    .min(max_layer),
                show_speaker_prefix: args
                    .get("show_speaker_prefix")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                ..Default::default()
            };
            let (count, end_time, cmd) =
                build_script_timeline_commands(&lines, None, &cfg, scene_w, scene_h)
                    .ok_or("failed to build script timeline")?;
            let s = &mut *guard;
            s.bus
                .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "imported_lines": count,
                        "end_sec": end_time.as_seconds(),
                        "subtitle_layer": cfg.subtitle_layer
                    }).to_string()
                }]
            }))
        }
        "import_subtitles_or_exo" => {
            let format = args
                .get("format")
                .and_then(Value::as_str)
                .ok_or("missing 'format'")?;
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or("missing 'content'")?;
            let (scene_w, scene_h, scene_fps, max_layer) = guard
                .project
                .active_scene()
                .map(|s| (s.width, s.height, s.fps, s.layers.len().saturating_sub(1)))
                .ok_or_else(|| "no active scene".to_string())?;
            let layer_index = (args.get("layer_index").and_then(Value::as_u64).unwrap_or(1)
                as usize)
                .min(max_layer);

            let (count, batch_cmd) = match format {
                "srt" => {
                    let entries = parse_srt_subtitles(content);
                    let n = entries.len();
                    let cmd = subtitle_entries_to_commands(&entries, layer_index, scene_w, scene_h)
                        .ok_or("SRT から有効な字幕を検出できませんでした")?;
                    (n, cmd)
                }
                "ass" => {
                    let entries = parse_ass_subtitles(content);
                    let n = entries.len();
                    let cmd = subtitle_entries_to_commands(&entries, layer_index, scene_w, scene_h)
                        .ok_or("ASS から有効な字幕を検出できませんでした")?;
                    (n, cmd)
                }
                "exo" => {
                    let parsed = parse_aviutl_exo(content, scene_fps);
                    if parsed.is_empty() {
                        return Err("AviUtl .exo からオブジェクトを検出できませんでした".into());
                    }
                    let n = parsed.len();
                    let commands = parsed
                        .into_iter()
                        .map(|(l_idx, item)| EditCommand::AddObject {
                            layer_index: l_idx.min(max_layer),
                            item,
                        })
                        .collect();
                    (
                        n,
                        EditCommand::Batch {
                            label: "import_aviutl_exo".into(),
                            commands,
                        },
                    )
                }
                other => return Err(format!("unsupported format: {other}")),
            };
            let s = &mut *guard;
            s.bus
                .execute(&mut s.project, batch_cmd, CommandSource::Mcp, None)
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({ "format": format, "imported_count": count }).to_string()
                }]
            }))
        }
        "export_subtitles_or_otio" => {
            let format = args
                .get("format")
                .and_then(Value::as_str)
                .ok_or("missing 'format'")?;
            let (ext, payload) = match format {
                "srt" => {
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    ("srt", export_srt_subtitles(scene))
                }
                "ass" => {
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    ("ass", export_ass_subtitles(scene))
                }
                "otio" => (
                    "otio",
                    export_otio_json(&guard.project, guard.project.active_scene_id)
                        .ok_or("failed to export OTIO")?,
                ),
                other => return Err(format!("unsupported export format: {other}")),
            };

            let mut saved_path: Option<String> = None;
            if let Some(raw_name) = args.get("file_name").and_then(Value::as_str) {
                let safe_stem: String = raw_name
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                    .take(48)
                    .collect();
                let stem = if safe_stem.is_empty() {
                    format!("export_{ext}")
                } else {
                    safe_stem
                };
                let out_dir = guard
                    .folder
                    .as_ref()
                    .map(|f| f.exports_dir())
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_mcp_exports"));
                std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                let out_path = out_dir.join(format!("{stem}.{ext}"));
                std::fs::write(&out_path, &payload).map_err(|e| e.to_string())?;
                saved_path = Some(out_path.display().to_string());
            }

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "format": format,
                        "saved_path": saved_path,
                        "content": payload
                    }).to_string()
                }]
            }))
        }
        "extract_highlights" => {
            let cfg = HighlightConfig {
                audio_layer_index: args
                    .get("audio_layer_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(3) as usize,
                target_clip_count: (args
                    .get("target_clip_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(3) as usize)
                    .clamp(1, 10),
                clip_duration_sec: finite_clamped(args.get("clip_duration_sec"), 15.0, 2.0, 120.0),
                ..Default::default()
            };

            let scene_id = guard.project.active_scene_id;
            let total_dur = guard
                .project
                .active_scene()
                .map(|s| s.total_duration())
                .ok_or_else(|| "no active scene".to_string())?;
            let sr = guard.project.settings.audio_sample_rate.max(8_000);
            let graph =
                AudioGraph::from_project_scene(&guard.project, scene_id, guard.folder.as_ref(), sr);
            let mut rms_samples: Vec<(f64, f32)> = Vec::new();
            if !graph.clips.is_empty() {
                let capped = total_dur.min(RationalTime::from_seconds(600.0));
                let pcm = graph.render_range(RationalTime::ZERO, capped);
                let frames_per_bucket = (sr as usize / 2).max(1); // 0.5s buckets
                let total_frames = pcm.len() / graph.channels.max(1);
                let mut f = 0usize;
                while f < total_frames {
                    let end_f = (f + frames_per_bucket).min(total_frames);
                    let mut sum_sq = 0.0f64;
                    let mut cnt = 0usize;
                    for fi in f..end_f {
                        let l = pcm[fi * graph.channels];
                        sum_sq += f64::from(l) * f64::from(l);
                        cnt += 1;
                    }
                    let rms = if cnt > 0 {
                        (sum_sq / cnt as f64).sqrt() as f32
                    } else {
                        0.0
                    };
                    rms_samples.push(((f as f64 + (end_f - f) as f64 * 0.5) / sr as f64, rms));
                    f = end_f;
                }
            }

            let candidates = {
                let scene = guard.project.active_scene().ok_or("no active scene")?;
                extract_highlight_candidates(scene, &rms_samples, &cfg)
            };

            let mut created_shorts_scene_id: Option<Uuid> = None;
            if args
                .get("create_shorts_scene")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let c_idx = args
                    .get("candidate_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let cand = candidates
                    .get(c_idx)
                    .ok_or("該当するハイライト候補が見つかりません")?;
                let shorts_name = args
                    .get("shorts_name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Shorts: {}", cand.suggested_title));
                let (new_sid, cmd) =
                    build_shorts_clip_scene_command(&guard.project, scene_id, cand, shorts_name)
                        .ok_or("ショートシーン生成に失敗しました")?;
                let s = &mut *guard;
                s.bus
                    .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                    .map_err(|e| e.to_string())?;
                created_shorts_scene_id = Some(new_sid);
            }

            let cand_json: Vec<Value> = candidates
                .iter()
                .map(|c| {
                    json!({
                        "start_sec": c.start.as_seconds(),
                        "end_sec": c.end.as_seconds(),
                        "score": c.score,
                        "reason": c.reason,
                        "suggested_title": c.suggested_title,
                    })
                })
                .collect();
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "candidates": cand_json,
                        "created_shorts_scene_id": created_shorts_scene_id
                    }).to_string()
                }]
            }))
        }
        "text_based_edit" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            let linked_layers: Vec<usize> = args
                .get("linked_layers")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as usize))
                        .collect()
                })
                .unwrap_or_else(|| {
                    let cnt = guard
                        .project
                        .active_scene()
                        .map(|s| s.layers.len())
                        .unwrap_or(4);
                    (0..cnt).collect()
                });
            match action {
                "list" => {
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    let entries = extract_transcript_entries(scene, None, None);
                    let rows: Vec<Value> = entries
                        .iter()
                        .map(|e| {
                            json!({
                                "item_id": e.item_id,
                                "layer_index": e.layer_index,
                                "start_sec": e.start.as_seconds(),
                                "end_sec": e.end.as_seconds(),
                                "text": e.clean_text,
                                "is_filler_only": e.is_filler_only,
                                "matched_fillers": e.matched_fillers,
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({"entries": rows}).to_string() }]
                    }))
                }
                "delete_segments" => {
                    let mut target_ids: Vec<Uuid> = Vec::new();
                    if let Some(arr) = args.get("ids").and_then(Value::as_array) {
                        for v in arr {
                            if let Some(s) = v.as_str()
                                && let Ok(id) = Uuid::parse_str(s)
                            {
                                target_ids.push(id);
                            }
                        }
                    }
                    if let Some(q) = args
                        .get("query")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        let scene = guard.project.active_scene().ok_or("no active scene")?;
                        for e in extract_transcript_entries(scene, None, None) {
                            if e.clean_text.contains(q) && !target_ids.contains(&e.item_id) {
                                target_ids.push(e.item_id);
                            }
                        }
                    }
                    let scene_clone = guard
                        .project
                        .active_scene()
                        .cloned()
                        .ok_or("no active scene")?;
                    let res =
                        build_text_based_delete_commands(&scene_clone, &target_ids, &linked_layers)
                            .ok_or("削除対象のテキスト区間が見つかりませんでした")?;
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, res.command, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "removed_ranges": res.removed_ranges,
                                "removed_sec": res.removed_sec
                            }).to_string()
                        }]
                    }))
                }
                "remove_fillers" => {
                    let strip_words = args
                        .get("strip_words")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let scene_clone = guard
                        .project
                        .active_scene()
                        .cloned()
                        .ok_or("no active scene")?;
                    if strip_words {
                        let (count, cmd) =
                            build_strip_filler_words_commands(&scene_clone, None, None)
                                .ok_or("除去対象のフィラー語句は見つかりませんでした")?;
                        let sess = &mut *guard;
                        sess.bus
                            .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                            .map_err(|e| e.to_string())?;
                        Ok(json!({
                            "content": [{ "type": "text", "text": json!({"stripped_items": count}).to_string() }]
                        }))
                    } else {
                        let filler_ids: Vec<Uuid> =
                            extract_transcript_entries(&scene_clone, None, None)
                                .into_iter()
                                .filter(|e| e.is_filler_only)
                                .map(|e| e.item_id)
                                .collect();
                        let res = build_text_based_delete_commands(
                            &scene_clone,
                            &filler_ids,
                            &linked_layers,
                        )
                        .ok_or("フィラーのみの区間は見つかりませんでした")?;
                        let sess = &mut *guard;
                        sess.bus
                            .execute(&mut sess.project, res.command, CommandSource::Mcp, None)
                            .map_err(|e| e.to_string())?;
                        Ok(json!({
                            "content": [{
                                "type": "text",
                                "text": json!({
                                    "removed_filler_ranges": res.removed_ranges,
                                    "removed_sec": res.removed_sec
                                }).to_string()
                            }]
                        }))
                    }
                }
                "reorder" => {
                    let id = parse_uuid(args, "id")?;
                    let move_up = args
                        .get("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("up")
                        != "down";
                    let scene_clone = guard
                        .project
                        .active_scene()
                        .cloned()
                        .ok_or("no active scene")?;
                    let cmd = build_text_based_reorder_commands(
                        &scene_clone,
                        id,
                        move_up,
                        &linked_layers,
                    )
                    .ok_or("並べ替え対象または隣接区間が見つかりません")?;
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({"reordered": id, "direction": if move_up { "up" } else { "down" }}).to_string() }]
                    }))
                }
                other => Err(format!("unknown text_based_edit action: {other}")),
            }
        }
        "sync_multicam" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "auto_sync_audio" => {
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let max_lag_sec = finite_clamped(args.get("max_lag_sec"), 5.0, 0.1, 30.0);
                    let mut offsets: Vec<(Uuid, f64)> = Vec::new();
                    let mut reports: Vec<Value> = Vec::new();

                    if let Some(manual) = args.get("manual_offsets").and_then(Value::as_array) {
                        for entry in manual {
                            let id = parse_uuid(entry, "id")?;
                            let off = finite_clamped(entry.get("offset_sec"), 0.0, -300.0, 300.0);
                            offsets.push((id, off));
                            reports.push(json!({
                                "item_id": id,
                                "offset_sec": off,
                                "correlation": 1.0,
                                "confidence": 1.0
                            }));
                        }
                    } else {
                        let angle_layers: Vec<usize> = args
                            .get("angle_layers")
                            .and_then(Value::as_array)
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_u64().map(|n| n as usize))
                                    .collect()
                            })
                            .filter(|v: &Vec<usize>| !v.is_empty())
                            .unwrap_or_else(|| vec![0, 1]);
                        let scene = guard.project.active_scene().ok_or("no active scene")?;
                        let mut angle_clips: Vec<(usize, Uuid, Option<PathBuf>)> = Vec::new();
                        for &li in &angle_layers {
                            if let Some(layer) = scene.layers.get(li)
                                && let Some(item) = layer.items.first()
                            {
                                let path = match &item.kind {
                                    ItemKind::Video { asset_id, .. }
                                    | ItemKind::Audio { asset_id, .. } => {
                                        guard.project.asset(*asset_id).and_then(|a| {
                                            if let Some(f) = guard.folder.as_ref() {
                                                match f.resolve_asset(a) {
                                                    ve_core::AssetResolution::Found(p) => Some(p),
                                                    ve_core::AssetResolution::Offline => None,
                                                }
                                            } else {
                                                a.fallback_absolute_path.clone()
                                            }
                                        })
                                    }
                                    _ => None,
                                };
                                angle_clips.push((li, item.id, path));
                            }
                        }
                        if angle_clips.len() < 2 {
                            return Err(
                                "マルチカム同期には2つ以上のアングルクリップが必要です".into()
                            );
                        }
                        let ref_buf = angle_clips[0]
                            .2
                            .as_deref()
                            .and_then(|p| decode_audio_file(p).ok());
                        offsets.push((angle_clips[0].1, 0.0));
                        reports.push(json!({
                            "layer_index": angle_clips[0].0,
                            "item_id": angle_clips[0].1,
                            "offset_sec": 0.0,
                            "correlation": 1.0,
                            "confidence": 1.0
                        }));
                        for (li, item_id, path_opt) in angle_clips.into_iter().skip(1) {
                            let est = match (
                                ref_buf.as_ref(),
                                path_opt.as_deref().and_then(|p| decode_audio_file(p).ok()),
                            ) {
                                (Some(rb), Some(tb)) => {
                                    estimate_audio_offset_xcorr(rb, &tb, max_lag_sec)
                                }
                                _ => Default::default(),
                            };
                            offsets.push((item_id, est.offset_sec));
                            reports.push(json!({
                                "layer_index": li,
                                "item_id": item_id,
                                "offset_sec": est.offset_sec,
                                "correlation": est.correlation,
                                "confidence": est.confidence
                            }));
                        }
                    }

                    let mut applied = false;
                    if apply
                        && let Some(cmd) = guard
                            .project
                            .active_scene()
                            .and_then(|s| build_multicam_sync_align_commands(s, &offsets))
                    {
                        let sess = &mut *guard;
                        sess.bus
                            .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                            .map_err(|e| e.to_string())?;
                        applied = true;
                    }

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "applied": applied, "angles": reports }).to_string()
                        }]
                    }))
                }
                "apply_switch_cuts" => {
                    let angle_layers: Vec<usize> = args
                        .get("angle_layers")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_u64().map(|n| n as usize))
                                .collect()
                        })
                        .ok_or("missing 'angle_layers'")?;
                    let max_layer = guard
                        .project
                        .active_scene()
                        .map(|s| s.layers.len().saturating_sub(1))
                        .ok_or("no active scene")?;
                    let output_layer_index = (args
                        .get("output_layer_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(max_layer as u64)
                        as usize)
                        .min(max_layer);
                    let range_start = RationalTime::from_seconds(finite_clamped(
                        args.get("range_start_sec"),
                        0.0,
                        0.0,
                        86_400.0,
                    ));
                    let default_end = guard
                        .project
                        .active_scene()
                        .map(|s| s.total_duration().as_seconds().max(1.0))
                        .unwrap_or(5.0);
                    let range_end = RationalTime::from_seconds(finite_clamped(
                        args.get("range_end_sec"),
                        default_end,
                        0.01,
                        86_400.0,
                    ));
                    let cuts: Vec<MulticamCutPoint> = args
                        .get("cuts")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|c| {
                                    let t = c.get("time_sec")?.as_f64()?;
                                    let a = c.get("active_angle")?.as_u64()? as usize;
                                    Some(MulticamCutPoint {
                                        time: RationalTime::from_seconds(t.max(0.0)),
                                        active_angle: a,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    let cmd = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            build_multicam_switch_cut_commands(
                                s,
                                &angle_layers,
                                &cuts,
                                output_layer_index,
                                range_start,
                                range_end,
                            )
                        })
                        .ok_or("切替カットの生成対象クリップがありません")?;
                    let cut_clips = match &cmd {
                        EditCommand::Batch { commands, .. } => commands.len(),
                        _ => 1,
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "output_layer_index": output_layer_index,
                                "generated_clips": cut_clips
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown sync_multicam action: {other}")),
            }
        }
        "nle_pro_edit" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "slip_trim" => {
                    let item_id = parse_uuid(args, "id")?;
                    let delta = RationalTime::from_seconds(finite_clamped(
                        args.get("delta_sec"),
                        0.0,
                        -3600.0,
                        3600.0,
                    ));
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SlipTrim { item_id, delta },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({"slip_trimmed": item_id, "delta_sec": delta.as_seconds()}).to_string() }] }),
                    )
                }
                "roll_trim" => {
                    let left_item_id = parse_uuid(args, "left_item_id")?;
                    let right_item_id = parse_uuid(args, "right_item_id")?;
                    let delta = RationalTime::from_seconds(finite_clamped(
                        args.get("delta_sec"),
                        0.0,
                        -3600.0,
                        3600.0,
                    ));
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::RollTrim {
                                left_item_id,
                                right_item_id,
                                delta,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({"roll_trimmed": [left_item_id, right_item_id], "delta_sec": delta.as_seconds()}).to_string() }] }),
                    )
                }
                "slide_trim" => {
                    let item_id = parse_uuid(args, "id")?;
                    let left_neighbor_id = args
                        .get("left_item_id")
                        .and_then(Value::as_str)
                        .and_then(|s| Uuid::parse_str(s).ok());
                    let right_neighbor_id = args
                        .get("right_item_id")
                        .and_then(Value::as_str)
                        .and_then(|s| Uuid::parse_str(s).ok());
                    let delta = RationalTime::from_seconds(finite_clamped(
                        args.get("delta_sec"),
                        0.0,
                        -3600.0,
                        3600.0,
                    ));
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SlideTrim {
                                item_id,
                                left_neighbor_id,
                                right_neighbor_id,
                                delta,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({"slide_trimmed": item_id, "delta_sec": delta.as_seconds()}).to_string() }] }),
                    )
                }
                "group" | "unlink" => {
                    let ids_arr = args
                        .get("ids")
                        .and_then(Value::as_array)
                        .ok_or("missing 'ids' array")?;
                    let mut item_ids = Vec::with_capacity(ids_arr.len());
                    for v in ids_arr {
                        let s = v.as_str().ok_or("id must be string")?;
                        item_ids.push(Uuid::parse_str(s).map_err(|e| e.to_string())?);
                    }
                    let count = item_ids.len();
                    let group_id = if action == "group" {
                        Some(Uuid::new_v4())
                    } else {
                        None
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetGroupId { item_ids, group_id },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({"action": action, "group_id": group_id, "count": count}).to_string() }] }),
                    )
                }
                "set_layer_mixer" => {
                    let layer_index =
                        args.get("layer_index")
                            .and_then(Value::as_u64)
                            .ok_or("missing 'layer_index'")? as usize;
                    let cur = guard
                        .project
                        .active_scene()
                        .and_then(|s| s.layers.get(layer_index))
                        .ok_or("layer out of bounds")?;
                    let volume = if args.get("volume").is_some() {
                        finite_clamped(args.get("volume"), f64::from(cur.volume), 0.0, 2.0) as f32
                    } else {
                        cur.volume
                    };
                    let pan = if args.get("pan").is_some() {
                        finite_clamped(args.get("pan"), f64::from(cur.pan), -1.0, 1.0) as f32
                    } else {
                        cur.pan
                    };
                    let solo = args
                        .get("solo")
                        .and_then(Value::as_bool)
                        .unwrap_or(cur.solo);
                    let muted = args
                        .get("muted")
                        .and_then(Value::as_bool)
                        .unwrap_or(cur.muted);
                    let bus = args
                        .get("bus")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| cur.bus.clone());
                    let send_reverb = if args.get("send_reverb").is_some() {
                        finite_clamped(
                            args.get("send_reverb"),
                            f64::from(cur.send_reverb),
                            0.0,
                            1.0,
                        ) as f32
                    } else {
                        cur.send_reverb
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetLayerMixer {
                                layer_index,
                                volume,
                                pan,
                                solo,
                                muted,
                                bus: bus.clone(),
                                send_reverb,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "layer_index": layer_index,
                                "volume": volume,
                                "pan": pan,
                                "solo": solo,
                                "muted": muted,
                                "bus": bus,
                                "send_reverb": send_reverb
                            }).to_string()
                        }]
                    }))
                }
                "separate_stems" => {
                    let asset_id = parse_uuid(args, "asset_id")?;
                    let asset = guard
                        .project
                        .asset(asset_id)
                        .cloned()
                        .ok_or("asset not found")?;
                    let src_path = if let Some(f) = guard.folder.as_ref() {
                        match f.resolve_asset(&asset) {
                            ve_core::AssetResolution::Found(p) => p,
                            ve_core::AssetResolution::Offline => {
                                return Err("audio asset is offline".into());
                            }
                        }
                    } else {
                        asset
                            .fallback_absolute_path
                            .clone()
                            .ok_or("asset has no path")?
                    };
                    let buf = decode_audio_file(&src_path).map_err(|e| e.to_string())?;
                    let stems = separate_vocal_bgm_stems(&buf);
                    let out_dir = guard
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir())
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_stems"));
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let stem_base: String = asset
                        .name
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                        .take(32)
                        .collect();
                    let vocal_path = out_dir.join(format!("{stem_base}_vocals.wav"));
                    let bgm_path = out_dir.join(format!("{stem_base}_bgm.wav"));
                    write_wav_pcm16(
                        &vocal_path,
                        &stems.vocals.samples,
                        stems.vocals.sample_rate,
                        stems.vocals.channels as u16,
                    )
                    .map_err(|e| e.to_string())?;
                    write_wav_pcm16(
                        &bgm_path,
                        &stems.accompaniment.samples,
                        stems.accompaniment.sample_rate,
                        stems.accompaniment.channels as u16,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "method": stems.method,
                                "vocals_path": vocal_path.display().to_string(),
                                "bgm_path": bgm_path.display().to_string(),
                                "frames": stems.vocals.frame_count()
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown nle_pro_edit action: {other}")),
            }
        }
        "kinetic_subtitles_and_localize" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;

            match action {
                "set_kinetic_style" => {
                    let mut cfg = KineticSubtitleConfig {
                        enabled: args.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                        ..KineticSubtitleConfig::default()
                    };
                    if let Some(m) = args.get("mode").and_then(Value::as_str) {
                        cfg.mode = match m {
                            "karaoke_sweep" => KineticHighlightMode::KaraokeSweep,
                            "progressive_reveal" => KineticHighlightMode::ProgressiveReveal,
                            "keyword_pulse" => KineticHighlightMode::KeywordPulse,
                            _ => KineticHighlightMode::WordPopColor,
                        };
                    }
                    if let Some(hex) = args.get("active_color").and_then(Value::as_str)
                        && let Some(c) = RgbaColor::from_hex(hex)
                    {
                        cfg.active_color = c;
                    }
                    if let Some(hex) = args.get("inactive_color").and_then(Value::as_str)
                        && let Some(c) = RgbaColor::from_hex(hex)
                    {
                        cfg.inactive_color = c;
                    }
                    if let Some(hex) = args.get("emphasis_color").and_then(Value::as_str)
                        && let Some(c) = RgbaColor::from_hex(hex)
                    {
                        cfg.emphasis_color = c;
                    }
                    if args.get("pop_scale").is_some() {
                        cfg.pop_scale = finite_clamped(
                            args.get("pop_scale"),
                            f64::from(cfg.pop_scale),
                            1.0,
                            1.8,
                        ) as f32;
                    }
                    if let Some(arr) = args.get("keywords").and_then(Value::as_array) {
                        cfg.auto_emphasize_keywords = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .filter(|k| !k.trim().is_empty())
                            .take(64)
                            .collect();
                    }

                    let target_ids: Vec<Uuid> = if args.get("id").is_some() {
                        vec![parse_uuid(args, "id")?]
                    } else {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        scene
                            .layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .filter(|i| matches!(i.kind, ItemKind::Text { .. }))
                            .map(|i| i.id)
                            .collect()
                    };
                    if target_ids.is_empty() {
                        return Err("対象となるテキスト／字幕アイテムがありません".into());
                    }
                    let count = target_ids.len();
                    let cmd = if count == 1 {
                        EditCommand::SetKineticSubtitle {
                            item_id: target_ids[0],
                            kinetic: cfg.clone(),
                        }
                    } else {
                        EditCommand::Batch {
                            label: format!("キネティック字幕一括設定 ({count}件)"),
                            commands: target_ids
                                .into_iter()
                                .map(|id| EditCommand::SetKineticSubtitle {
                                    item_id: id,
                                    kinetic: cfg.clone(),
                                })
                                .collect(),
                        }
                    };
                    s.bus
                        .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "updated_count": count, "config": cfg }).to_string() }]
                    }))
                }
                "extract_localization_table" => {
                    let src_loc = args
                        .get("source_locale")
                        .and_then(Value::as_str)
                        .unwrap_or("ja");
                    let scene = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let mut table = extract_localization_table(scene, src_loc);
                    if let Some(arr) = args.get("target_locales").and_then(Value::as_array) {
                        for v in arr {
                            if let Some(loc) = v.as_str().map(str::trim)
                                && !loc.is_empty()
                                && !table.locales.iter().any(|l| l == loc)
                            {
                                table.locales.push(loc.to_string());
                            }
                        }
                    }
                    let fmt = args.get("format").and_then(Value::as_str).unwrap_or("json");
                    let text = if fmt == "csv" {
                        export_localization_csv(&table)
                    } else {
                        serde_json::to_string_pretty(&table).unwrap_or_default()
                    };
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "apply_locale_translations" => {
                    let locale = args
                        .get("locale")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'locale'".to_string())?;
                    let table = if let Some(csv) = args.get("csv").and_then(Value::as_str) {
                        import_localization_csv(csv)
                    } else if let Some(map) = args.get("translations").and_then(Value::as_object) {
                        let mut entries = Vec::new();
                        for (k, v) in map {
                            if let (Ok(id), Some(txt)) = (Uuid::parse_str(k), v.as_str()) {
                                let mut tr = BTreeMap::new();
                                tr.insert(locale.to_string(), txt.to_string());
                                entries.push(LocalizationEntry {
                                    item_id: id,
                                    item_name: String::new(),
                                    start_sec: 0.0,
                                    translations: tr,
                                });
                            }
                        }
                        LocalizationTable {
                            locales: vec![locale.to_string()],
                            entries,
                        }
                    } else {
                        return Err("missing 'csv' or 'translations'".into());
                    };
                    let cmds = {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_apply_locale_commands(scene, &table, locale)
                    };
                    let applied_count = cmds.len();
                    if applied_count > 0 {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("字幕言語切替 [{locale}] ({applied_count}件)"),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "locale": locale, "applied_count": applied_count }).to_string() }]
                    }))
                }
                other => Err(format!(
                    "unknown kinetic_subtitles_and_localize action: {other}"
                )),
            }
        }
        "speed_ramp_and_scene_detect" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "set_speed_ramp" => {
                    let id_str = args
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'id'".to_string())?;
                    let item_id = Uuid::parse_str(id_str).map_err(|e| e.to_string())?;
                    let preset = match args.get("preset").and_then(Value::as_str) {
                        Some("bullet_time_dip") => SpeedRampPreset::BulletTimeDip,
                        Some("ramp_up_hero") => SpeedRampPreset::RampUpHero,
                        Some("ease_in_slow_mo") => SpeedRampPreset::EaseInSlowMo,
                        Some("flash_burst") => SpeedRampPreset::FlashBurst,
                        _ => SpeedRampPreset::Uniform,
                    };
                    let mut cfg = preset.default_config();
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        cfg.enabled = en;
                    }
                    if let Some(sm) = args.get("smooth_blend").and_then(Value::as_bool) {
                        cfg.smooth_blend = sm;
                    }
                    if let Some(arr) = args.get("knots").and_then(Value::as_array)
                        && !arr.is_empty()
                    {
                        let parsed: Vec<SpeedRampKnot> = arr
                            .iter()
                            .filter_map(|v| {
                                Some(SpeedRampKnot {
                                    norm_time: v.get("norm_time")?.as_f64()? as f32,
                                    speed: v.get("speed")?.as_f64()? as f32,
                                })
                            })
                            .collect();
                        if !parsed.is_empty() {
                            cfg.knots = parsed;
                        }
                    }
                    cfg.knots = cfg.normalized();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetSpeedRamp {
                                item_id,
                                speed_ramp: cfg.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": item_id.to_string(),
                                "speed_ramp": cfg,
                                "applied": true
                            }).to_string()
                        }]
                    }))
                }
                "detect_scene_cuts" => {
                    let interval = finite_clamped(args.get("sample_interval_sec"), 0.25, 0.08, 1.0);
                    let sensitivity =
                        finite_clamped(args.get("sensitivity"), 0.55, 0.1, 1.0) as f32;
                    let split_clips = args
                        .get("split_clips")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(false);
                    let target_layer = args
                        .get("target_layer_index")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize);

                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let cuts = detect_scene_cuts_in_scene(
                        &mut rg,
                        &s.project,
                        s.project.active_scene_id,
                        interval,
                        sensitivity,
                    );
                    let applied_commands = if apply && !cuts.is_empty() {
                        let cmds = {
                            let sc = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            build_scene_cut_commands(sc, &cuts, split_clips, target_layer)
                        };
                        let n = cmds.len();
                        if !cmds.is_empty() {
                            s.bus
                                .execute(
                                    &mut s.project,
                                    EditCommand::Batch {
                                        label: format!("自動シーンカット適用 ({}件)", cuts.len()),
                                        commands: cmds,
                                    },
                                    CommandSource::Mcp,
                                    None,
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        n
                    } else {
                        0
                    };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "cut_count": cuts.len(),
                                "cuts": cuts,
                                "applied_commands": applied_commands
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown speed_ramp_and_scene_detect action: {other}"
                )),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}
