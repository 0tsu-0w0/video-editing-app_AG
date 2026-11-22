use crate::session::SharedEditorSession;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;
use ve_ai::{
    ApiProvider, ClaudeRefiner, CloudUploadGuard, OsCredentialStore, SubtitleSegment,
    import_tts_file_pair,
};
use ve_audio::{
    AudioGraph, BeatDetectionConfig, detect_audio_beats, estimate_audio_offset_xcorr,
    separate_vocal_bgm_stems,
};
use ve_core::{
    AlignMode, AssetRef, AudioDuckingConfig, AudioEffectNode, BezierMask, Camera3D,
    CharacterSpriteConfig, CommandSource, CubicBezier, EditCommand, EditorKeymap, EffectNode,
    EffectPreset, FrameRate, HighlightConfig, Interpolation, ItemKind, JumpCutConfig, Keyframe,
    KeymapPreset, Light3DType, LipSyncMode, MaskMode, MaskVertex, MediaKind, MotionPreset,
    MulticamCutPoint, ParticleConfig, ParticlePreset, ProjectTemplate, PsdCharacterConfig,
    RationalTime, RgbaColor, SceneColorGrade, ScriptBuildConfig, ShapeKind, TemplateParamValue,
    TextAnimator, TextStylePreset, TimelineItem, TimelineMarker, TrackMatte, TrackMatteMode,
    UiLanguage, UiThemePreset, VectorShapeConfig, archive_project_to_zip, base64_encode,
    build_align_commands, build_bake_expression_command, build_beat_cut_commands,
    build_beat_markers_command, build_close_gaps_commands, build_fit_duration_to_layer_commands,
    build_instantiate_template_command, build_jump_cut_commands, build_motion_preset_commands,
    build_multicam_switch_cut_commands, build_multicam_sync_align_commands,
    build_replace_text_commands, build_script_timeline_commands, build_shorts_clip_scene_command,
    build_strip_filler_words_commands, build_text_based_delete_commands,
    build_text_based_reorder_commands, builtin_motion_templates, builtin_project_templates,
    compile_expression, detect_speech_intervals_from_rms, evaluate_item_property,
    export_ass_subtitles, export_otio_json, export_srt_subtitles, extract_highlight_candidates,
    extract_motion_template_from_scene, extract_transcript_entries, format_youtube_chapters,
    official_registry_catalog, parse_ass_subtitles, parse_aviutl_exo, parse_script_auto,
    parse_srt_subtitles, search_text_items, subtitle_entries_to_commands,
    validate_project_delivery, verify_and_install_extension, wrap_subtitle_japanese,
};
use ve_media::{
    ExportConfig, ExportFormat, FfmpegCli, HwDecodeBackend, VideoEncoder, decode_audio_file,
    write_wav_pcm16,
};
use ve_plugin_api::{PluginRegistry, WasmPluginManager, emit_sample_wasm_plugins};
use ve_render::{RenderGraph, emit_sample_cube_luts, validate_wgsl};

const MAX_BATCH_COMMANDS: usize = 500;
const ALLOWED_TTS_AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a", "aac"];

fn finite_clamped(val: Option<&Value>, default: f64, min: f64, max: f64) -> f64 {
    match val.and_then(Value::as_f64) {
        Some(v) if v.is_finite() => v.clamp(min, max),
        _ => default,
    }
}

pub fn tool_definitions() -> Value {
    let mut tools = match json!([
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
    ]) {
        Value::Array(arr) => arr,
        _ => Vec::new(),
    };
    if let Value::Array(extra) = json!([
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
            "name": "refine_subtitles",
            "description": "タイムライン上の字幕を整形する（mode='local_wrap' で18文字自動改行、mode='claude' で Claude API 文脈補正・要 user_confirmed=true）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["local_wrap", "claude"], "default": "local_wrap" },
                    "max_chars_per_line": { "type": "integer", "default": 18 },
                    "user_confirmed": { "type": "boolean", "default": false }
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
            "name": "start_export",
            "description": "確認ガード付き（user_confirmed=true 必須）でアクティブシーンをプロジェクト exports/ フォルダへ書き出す",
            "inputSchema": {
                "type": "object",
                "required": ["user_confirmed"],
                "properties": {
                    "user_confirmed": { "type": "boolean" },
                    "format": { "type": "string", "enum": ["mp4_h264", "mp4_h265", "webm_vp9_alpha", "wav_audio_only"], "default": "wav_audio_only" },
                    "file_name": { "type": "string", "default": "mcp_export" }
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
        tools.extend(extra);
    }
    if let Value::Array(v06_extra) = json!([
        {
            "name": "set_audio_ducking",
            "description": "発話レイヤー（サイドチェイン）に応じて BGM レイヤーの音量を自動減衰させるダッキングを設定する（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "voice_layer_index": { "type": "integer" },
                    "bgm_layer_index": { "type": "integer" },
                    "threshold_rms": { "type": "number" },
                    "reduction_db": { "type": "number" },
                    "attack_sec": { "type": "number" },
                    "release_sec": { "type": "number" }
                }
            }
        },
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
        }
    ]) {
        tools.extend(v06_extra);
    }
    if let Value::Array(v07_extra) = json!([
        {
            "name": "set_bezier_mask",
            "description": "アイテムへ自由ベジェマスク（楕円/矩形/ひし形/カスタム頂点、加算・減算・反転・フェザー）を設定またはクリアする（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "action": { "type": "string", "enum": ["add_preset", "clear"], "default": "add_preset" },
                    "preset": { "type": "string", "enum": ["ellipse", "rectangle", "diamond"], "default": "ellipse" },
                    "mode": { "type": "string", "enum": ["add", "subtract"], "default": "add" },
                    "inverted": { "type": "boolean", "default": false },
                    "feather_px": { "type": "number", "default": 4.0 },
                    "opacity": { "type": "number", "default": 1.0 },
                    "vertices": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "x": { "type": "number" },
                                "y": { "type": "number" }
                            }
                        }
                    }
                }
            }
        },
        {
            "name": "list_or_reload_wasm_plugins",
            "description": "plugins/*.wasm のサンドボックス WebAssembly プラグインを検証・一覧取得し、任意でサンプル .wasm を生成する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plugins_dir": { "type": "string" },
                    "emit_samples": { "type": "boolean", "default": false }
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
            "name": "manage_presets",
            "description": "字幕スタイルプリセット・エフェクトチェーンプリセット（presets/*.json＋組み込み）の一覧取得・保存・1クリック適用を行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "apply_text_style", "apply_effect_preset", "save_text_style", "save_effect_preset"]
                    },
                    "id": { "type": "string" },
                    "name": { "type": "string" }
                }
            }
        }
    ]) {
        tools.extend(v07_extra);
    }
    if let Value::Array(v08_extra) = json!([
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
            "name": "detect_beats",
            "description": "BGM/音声から BPM と拍・小節位置を自動検出し、任意でビートマーカー一括配置や選択クリップの音ハメ自動カットを行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "min_bpm": { "type": "number", "default": 70.0 },
                    "max_bpm": { "type": "number", "default": 180.0 },
                    "sensitivity": { "type": "number", "default": 1.35 },
                    "beats_per_bar": { "type": "integer", "default": 4 },
                    "add_markers": { "type": "boolean", "default": false },
                    "downbeats_only": { "type": "boolean", "default": true },
                    "cut_item_ids": { "type": "array", "items": { "type": "string" } }
                }
            }
        },
        {
            "name": "set_color_grade",
            "description": "アクティブシーンの 3-Way カラーグレーディング（露出/コントラスト/彩度/色温度/ティント/Lift/Gamma/Gain）および .cube 3D LUT を設定・リセットする（Undo対応）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "reset": { "type": "boolean", "default": false },
                    "exposure": { "type": "number" },
                    "contrast": { "type": "number" },
                    "saturation": { "type": "number" },
                    "temperature": { "type": "number" },
                    "tint": { "type": "number" },
                    "lift": { "type": "array", "items": { "type": "number" } },
                    "gamma": { "type": "array", "items": { "type": "number" } },
                    "gain": { "type": "array", "items": { "type": "number" } },
                    "lut_path": { "type": "string" },
                    "lut_intensity": { "type": "number" },
                    "emit_sample_luts": { "type": "boolean", "default": false }
                }
            }
        }
    ]) {
        tools.extend(v08_extra);
    }
    if let Value::Array(v09_extra) = json!([
        {
            "name": "set_expression",
            "description": "Pure-Rust エクスプレッション（式アニメーション: wiggle/loopOut/audio_band/layer参照等）の設定・解除・一覧・即時評価・キーフレーム焼き込み(Bake)を行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action", "id"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["set", "clear", "list", "evaluate", "bake"]
                    },
                    "id": { "type": "string" },
                    "property": { "type": "string" },
                    "expression": { "type": "string" },
                    "time_sec": { "type": "number" },
                    "step_frames": { "type": "integer", "default": 3 }
                }
            }
        },
        {
            "name": "manage_templates",
            "description": "モーショングラフィックス テンプレート（.mogrt 相当・レスポンシブ時間伸縮）およびプロジェクトテンプレートの一覧・配置・公開パラメータ更新・現シーン保存を行う（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "instantiate", "update_instance", "save_from_scene", "list_project_templates", "save_project_template"]
                    },
                    "template_id": { "type": "string" },
                    "name": { "type": "string" },
                    "category": { "type": "string" },
                    "layer_index": { "type": "integer", "default": 1 },
                    "start_sec": { "type": "number", "default": 0.0 },
                    "duration_sec": { "type": "number" },
                    "intro_sec": { "type": "number", "default": 0.45 },
                    "outro_sec": { "type": "number", "default": 0.45 },
                    "item_id": { "type": "string" },
                    "values": { "type": "object" }
                }
            }
        },
        {
            "name": "set_scene_motion_blur",
            "description": "シャッター角度ベースモーションブラー、3D ディレクショナル／ポイントライト＆ドロップシャドウキャッチャー、およびパーティクル／ベクターシェイプ設定を操作する（Undo対応）",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "set_motion_blur", "set_light_3d", "set_particle", "set_vector_shape"]
                    },
                    "enabled": { "type": "boolean" },
                    "shutter_angle_deg": { "type": "number" },
                    "shutter_phase_deg": { "type": "number" },
                    "preview_samples": { "type": "integer" },
                    "export_samples": { "type": "integer" },
                    "light_type": { "type": "string", "enum": ["directional", "point"] },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "intensity": { "type": "number" },
                    "ambient": { "type": "number" },
                    "cast_shadow": { "type": "boolean" },
                    "shadow_softness": { "type": "number" },
                    "shadow_opacity": { "type": "number" },
                    "item_id": { "type": "string" },
                    "preset": { "type": "string" },
                    "count": { "type": "integer" },
                    "seed": { "type": "integer" },
                    "speed": { "type": "number" },
                    "size_px": { "type": "number" },
                    "trim_start": { "type": "number" },
                    "trim_end": { "type": "number" },
                    "trim_offset": { "type": "number" },
                    "stroke_width": { "type": "number" },
                    "dash_length": { "type": "number" },
                    "dash_gap": { "type": "number" }
                }
            }
        }
    ]) {
        tools.extend(v09_extra);
    }

    if let Value::Array(v10_extra) = json!([
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
            "name": "validate_and_package_project",
            "description": "v1.0 #19: プロジェクト書き出し前検証チェックリスト（オフライン素材・無音ギャップ・高解像度）の実行、および Pure-Rust ZIP アーカイブパッケージ化を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["validate", "archive_zip"]
                    },
                    "file_name": { "type": "string" }
                }
            }
        },
        {
            "name": "manage_ecosystem",
            "description": "v1.0 #20: カスタムキーバインドプリセット（AviUtl2/Premiere/DaVinci/AE）・多言語/テーマ設定・SHA-256検証付き公式拡張レジストリ・HWデコード/エンコード検出を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_settings", "set_settings", "export_keymap_json", "import_keymap_json", "list_registry", "install_extension", "hw_status"]
                    },
                    "language": { "type": "string", "enum": ["ja", "en"] },
                    "theme": { "type": "string", "enum": ["dark_studio", "midnight_oled", "high_contrast", "light_daylight"] },
                    "font_scale": { "type": "number" },
                    "keymap_preset": { "type": "string", "enum": ["aviutl2", "premiere", "davinci", "after_effects"] },
                    "keymap_json": { "type": "string" },
                    "extension_id": { "type": "string" },
                    "hw_decode": { "type": "string" },
                    "parallel_export_threads": { "type": "integer" }
                }
            }
        }
    ]) {
        tools.extend(v10_extra);
    }
    Value::Array(tools)
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
            drop(guard);

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
        "refine_subtitles" => {
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("local_wrap");
            let max_chars = args
                .get("max_chars_per_line")
                .and_then(Value::as_u64)
                .unwrap_or(18)
                .clamp(8, 40) as usize;
            let user_confirmed = args
                .get("user_confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let scene = guard
                .project
                .active_scene()
                .ok_or("no active scene")?
                .clone();
            let mut text_items: Vec<(Uuid, RationalTime, RationalTime, String)> = Vec::new();
            for layer in &scene.layers {
                for item in &layer.items {
                    if let ItemKind::Text { content, .. } = &item.kind {
                        text_items.push((item.id, item.start, item.end(), content.clone()));
                    }
                }
            }
            text_items.sort_by_key(|(_, st, _, _)| *st);
            if text_items.is_empty() {
                return Ok(
                    json!({ "content": [{ "type": "text", "text": json!({"updated": 0}).to_string() }] }),
                );
            }

            let cmds: Vec<EditCommand> = match mode {
                "local_wrap" => text_items
                    .iter()
                    .filter_map(|(id, _, _, content)| {
                        let wrapped = wrap_subtitle_japanese(content, max_chars);
                        if wrapped != *content {
                            Some(EditCommand::SetTextContent {
                                item_id: *id,
                                content: wrapped,
                            })
                        } else {
                            None
                        }
                    })
                    .collect(),
                "claude" => {
                    let segments: Vec<SubtitleSegment> = text_items
                        .iter()
                        .map(|(_, st, en, txt)| SubtitleSegment {
                            start: *st,
                            end: *en,
                            text: txt.clone(),
                            phonemes: Vec::new(),
                        })
                        .collect();
                    let dur_sec = scene.total_duration().as_seconds().max(1.0);
                    let store = OsCredentialStore::new();
                    let refined = ClaudeRefiner::default()
                        .refine_subtitles(&store, &segments, dur_sec, user_confirmed)
                        .map_err(|e| e.to_string())?;
                    text_items
                        .iter()
                        .zip(refined.iter())
                        .filter_map(|((id, _, _, before), after)| {
                            if &after.text != before {
                                Some(EditCommand::SetTextContent {
                                    item_id: *id,
                                    content: after.text.clone(),
                                })
                            } else {
                                None
                            }
                        })
                        .collect()
                }
                other => return Err(format!("unknown refine mode: {other}")),
            };
            let updated = cmds.len();
            if updated > 0 {
                let sess = &mut *guard;
                sess.bus
                    .execute(
                        &mut sess.project,
                        EditCommand::Batch {
                            label: format!("mcp_refine_{mode}"),
                            commands: cmds,
                        },
                        CommandSource::Mcp,
                        None,
                    )
                    .map_err(|e| e.to_string())?;
            }
            Ok(json!({
                "content": [{ "type": "text", "text": json!({"mode": mode, "updated": updated}).to_string() }]
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
        "start_export" => {
            let confirmed = args
                .get("user_confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !confirmed {
                return Err(
                    "書き出し実行には user_confirmed: true の明示的確認が必要です".to_string(),
                );
            }
            let fmt_str = args
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("wav_audio_only");
            let (format, ext, transparent_bg) = match fmt_str {
                "mp4_h264" => (ExportFormat::Mp4H264, "mp4", false),
                "mp4_h265" => (ExportFormat::Mp4H265, "mp4", false),
                "mp4_h265_hdr10" => (ExportFormat::Mp4H265HdrPq10Bit, "mp4", false),
                "webm_vp9_alpha" => (ExportFormat::WebmVp9Alpha, "webm", true),
                "wav_audio_only" => (ExportFormat::WavAudioOnly, "wav", false),
                other => return Err(format!("unsupported export format: {other}")),
            };
            let raw_name = args
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or("mcp_export");
            let safe_stem: String = raw_name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .take(48)
                .collect();
            let stem = if safe_stem.is_empty() {
                "mcp_export".to_string()
            } else {
                safe_stem
            };
            let out_dir = guard
                .folder
                .as_ref()
                .map(|f| f.exports_dir())
                .unwrap_or_else(|| std::env::temp_dir().join("ve_mcp_exports"));
            std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let output_path = out_dir.join(format!("{stem}.{ext}"));

            let proj = guard.project.clone();
            let scene_id = proj.active_scene_id;
            let folder = guard.folder.clone();
            let scene = proj.scene(scene_id).ok_or("no active scene")?;
            let dur = scene
                .total_duration()
                .max(RationalTime::from_seconds(1.0))
                .min(RationalTime::from_seconds(600.0));
            let sr = proj.settings.audio_sample_rate.max(8_000);
            let audio_graph = AudioGraph::from_project_scene(&proj, scene_id, folder.as_ref(), sr);
            let pcm = audio_graph.render_range(RationalTime::ZERO, dur);

            if format == ExportFormat::WavAudioOnly {
                write_wav_pcm16(&output_path, &pcm, sr, 2).map_err(|e| e.to_string())?;
            } else {
                let mut rg = RenderGraph::new();
                rg.use_proxies = false;
                rg.set_project_folder(folder);
                let total_frames = ((dur.as_seconds() * scene.fps.as_f64()).ceil() as i64).max(1);
                let cfg = ExportConfig {
                    width: scene.width.max(16),
                    height: scene.height.max(16),
                    fps: scene.fps,
                    format,
                    crf: 18,
                    encoder: VideoEncoder::Auto,
                };
                let wav_tmp = out_dir.join(format!("{stem}_tmp_audio.wav"));
                let has_audio = !audio_graph.clips.is_empty();
                if has_audio {
                    write_wav_pcm16(&wav_tmp, &pcm, sr, 2).map_err(|e| e.to_string())?;
                }
                let mut pipe = FfmpegCli::default()
                    .start_export(&output_path, &cfg, has_audio.then_some(wav_tmp.as_path()))
                    .map_err(|e| e.to_string())?;
                for f in 0..total_frames {
                    let t = RationalTime::from_frame(f, cfg.fps);
                    let img = rg
                        .render_project_scene(&proj, scene_id, t, transparent_bg)
                        .ok_or("render frame failed")?;
                    pipe.write_rgba8_frame(&img.to_rgba8_srgb())
                        .map_err(|e| e.to_string())?;
                }
                let res = pipe.finish().map_err(|e| e.to_string());
                if has_audio {
                    let _ = std::fs::remove_file(&wav_tmp);
                }
                res?;
            }

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({"exported": output_path.display().to_string(), "format": fmt_str}).to_string()
                }]
            }))
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
        "set_audio_ducking" => {
            let mut ducking = guard
                .project
                .active_scene()
                .map(|s| s.audio_ducking)
                .unwrap_or_else(AudioDuckingConfig::default);
            let max_layer = guard
                .project
                .active_scene()
                .map(|s| s.layers.len().saturating_sub(1))
                .unwrap_or(0);
            if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                ducking.enabled = en;
            }
            if let Some(vl) = args.get("voice_layer_index").and_then(Value::as_u64) {
                ducking.voice_layer_index = (vl as usize).min(max_layer);
            }
            if let Some(bl) = args.get("bgm_layer_index").and_then(Value::as_u64) {
                ducking.bgm_layer_index = (bl as usize).min(max_layer);
            }
            if args.get("threshold_rms").is_some() {
                ducking.threshold_rms = finite_clamped(
                    args.get("threshold_rms"),
                    f64::from(ducking.threshold_rms),
                    0.001,
                    0.9,
                ) as f32;
            }
            if args.get("reduction_db").is_some() {
                ducking.reduction_db = finite_clamped(
                    args.get("reduction_db"),
                    f64::from(ducking.reduction_db),
                    -48.0,
                    0.0,
                ) as f32;
            }
            if args.get("attack_sec").is_some() {
                ducking.attack_sec = finite_clamped(
                    args.get("attack_sec"),
                    f64::from(ducking.attack_sec),
                    0.002,
                    2.0,
                ) as f32;
            }
            if args.get("release_sec").is_some() {
                ducking.release_sec = finite_clamped(
                    args.get("release_sec"),
                    f64::from(ducking.release_sec),
                    0.01,
                    5.0,
                ) as f32;
            }
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAudioDucking { ducking },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&ducking).unwrap_or_default()
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
        "set_bezier_mask" => {
            let item_id = parse_uuid(args, "id")?;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("add_preset");
            let mut masks = guard
                .project
                .active_scene()
                .and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.masks.clone())
                })
                .ok_or_else(|| format!("object not found: {item_id}"))?;

            if action == "clear" {
                masks.clear();
            } else {
                let mut mask = match args.get("preset").and_then(Value::as_str) {
                    Some("rectangle") => BezierMask::rectangle(),
                    Some("diamond") => BezierMask::diamond(),
                    _ => BezierMask::ellipse(),
                };
                if let Some(m) = args.get("mode").and_then(Value::as_str) {
                    mask.mode = if m == "subtract" {
                        MaskMode::Subtract
                    } else {
                        MaskMode::Add
                    };
                }
                if let Some(inv) = args.get("inverted").and_then(Value::as_bool) {
                    mask.inverted = inv;
                }
                mask.feather_px = finite_clamped(
                    args.get("feather_px"),
                    f64::from(mask.feather_px),
                    0.0,
                    96.0,
                ) as f32;
                mask.opacity =
                    finite_clamped(args.get("opacity"), f64::from(mask.opacity), 0.0, 1.0) as f32;
                if let Some(verts) = args.get("vertices").and_then(Value::as_array)
                    && verts.len() >= 3
                {
                    let parsed: Vec<MaskVertex> = verts
                        .iter()
                        .filter_map(|v| {
                            let x = v.get("x")?.as_f64()? as f32;
                            let y = v.get("y")?.as_f64()? as f32;
                            if x.is_finite() && y.is_finite() {
                                Some(MaskVertex::corner(x.clamp(-0.5, 1.5), y.clamp(-0.5, 1.5)))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if parsed.len() >= 3 {
                        mask.vertices = parsed;
                    }
                }
                masks.push(mask);
            }

            let count = masks.len();
            let s = &mut *guard;
            s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetMasks { item_id, masks },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({ "item_id": item_id, "mask_count": count }).to_string()
                }]
            }))
        }
        "list_or_reload_wasm_plugins" => {
            let dir = if let Some(p) = args.get("plugins_dir").and_then(Value::as_str) {
                PathBuf::from(p)
            } else if let Some(f) = guard.folder.as_ref() {
                f.root().join("plugins")
            } else {
                PathBuf::from("plugins")
            };
            if args
                .get("emit_samples")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                emit_sample_wasm_plugins(&dir).map_err(|e| e.to_string())?;
            }
            let mut reg = PluginRegistry::with_builtins();
            let mut mgr = WasmPluginManager::new(&dir);
            let events = mgr.poll_and_sync(&mut reg);
            let statuses: Vec<Value> = mgr
                .list_status()
                .into_iter()
                .map(|st| {
                    json!({
                        "id": st.id,
                        "name": st.name,
                        "path": st.path.display().to_string(),
                        "last_error": st.last_error,
                    })
                })
                .collect();
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "plugins_dir": dir.display().to_string(),
                        "events": events,
                        "wasm_plugins": statuses
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
        "manage_presets" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "list" => {
                    let mut text_presets = TextStylePreset::default_presets();
                    let mut effect_presets = EffectPreset::default_presets();
                    if let Some(folder) = guard.folder.as_ref() {
                        text_presets.extend(folder.load_text_style_presets());
                        effect_presets.extend(folder.load_effect_presets());
                    }
                    let text_names: Vec<&str> =
                        text_presets.iter().map(|p| p.name.as_str()).collect();
                    let fx_names: Vec<&str> =
                        effect_presets.iter().map(|p| p.name.as_str()).collect();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "text_style_presets": text_names,
                                "effect_presets": fx_names,
                            }).to_string()
                        }]
                    }))
                }
                "apply_text_style" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("missing 'name'")?;
                    let preset = guard
                        .folder
                        .as_ref()
                        .and_then(|f| {
                            f.load_text_style_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .or_else(|| {
                            TextStylePreset::default_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .ok_or_else(|| format!("text style preset not found: {preset_name}"))?;
                    let batch = EditCommand::Batch {
                        label: format!("apply_text_style:{preset_name}"),
                        commands: vec![
                            EditCommand::SetTextFont {
                                item_id,
                                font_family: preset.font_family,
                                font_size: preset.font_size,
                            },
                            EditCommand::SetItemColor {
                                item_id,
                                color: preset.color,
                            },
                            EditCommand::SetTextStyle {
                                item_id,
                                style: preset.text_style,
                            },
                        ],
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, batch, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"applied_text_style": preset_name, "id": item_id}).to_string()
                        }]
                    }))
                }
                "apply_effect_preset" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("missing 'name'")?;
                    let preset = guard
                        .folder
                        .as_ref()
                        .and_then(|f| {
                            f.load_effect_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .or_else(|| {
                            EffectPreset::default_presets()
                                .into_iter()
                                .find(|p| p.name == preset_name)
                        })
                        .ok_or_else(|| format!("effect preset not found: {preset_name}"))?;
                    let effects = preset.instantiate();
                    let count = effects.len();
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::PasteEffects { item_id, effects },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"applied_effect_preset": preset_name, "added_effects": count}).to_string()
                        }]
                    }))
                }
                "save_text_style" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("missing 'name'")?;
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("プロジェクトフォルダが未保存のためプリセットを保存できません")?;
                    let item = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .find(|i| i.id == item_id)
                        })
                        .ok_or_else(|| format!("object not found: {item_id}"))?;
                    let ItemKind::Text {
                        font_family,
                        font_size,
                        color,
                        ..
                    } = &item.kind
                    else {
                        return Err("指定オブジェクトはテキストではありません".into());
                    };
                    let preset = TextStylePreset {
                        name: preset_name.trim().to_string(),
                        font_family: font_family.clone(),
                        font_size: *font_size,
                        color: *color,
                        text_style: item.text_style,
                    };
                    let path = folder
                        .save_text_style_preset(&preset)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"saved_text_style": preset.name, "path": path.display().to_string()}).to_string()
                        }]
                    }))
                }
                "save_effect_preset" => {
                    let item_id = parse_uuid(args, "id")?;
                    let preset_name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("missing 'name'")?;
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("プロジェクトフォルダが未保存のためプリセットを保存できません")?;
                    let item = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .find(|i| i.id == item_id)
                        })
                        .ok_or_else(|| format!("object not found: {item_id}"))?;
                    let preset = EffectPreset {
                        name: preset_name.trim().to_string(),
                        effects: item.effect_chain.clone(),
                    };
                    let path = folder
                        .save_effect_preset(&preset)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"saved_effect_preset": preset.name, "path": path.display().to_string()}).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown preset action: {other}")),
            }
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
        "detect_beats" => {
            let cfg = BeatDetectionConfig {
                min_bpm: finite_clamped(args.get("min_bpm"), 70.0, 40.0, 220.0) as f32,
                max_bpm: finite_clamped(args.get("max_bpm"), 180.0, 50.0, 260.0) as f32,
                sensitivity: finite_clamped(args.get("sensitivity"), 1.35, 0.8, 3.5) as f32,
                beats_per_bar: (args
                    .get("beats_per_bar")
                    .and_then(Value::as_u64)
                    .unwrap_or(4) as usize)
                    .clamp(1, 12),
            };
            let scene_id = guard.project.active_scene_id;
            let total_dur = guard
                .project
                .active_scene()
                .map(|s| s.total_duration())
                .ok_or("no active scene")?;
            let sr = guard.project.settings.audio_sample_rate.max(8_000);
            let graph =
                AudioGraph::from_project_scene(&guard.project, scene_id, guard.folder.as_ref(), sr);
            let capped = total_dur.min(RationalTime::from_seconds(300.0));
            let pcm = if graph.clips.is_empty() {
                Vec::new()
            } else {
                graph.render_range(RationalTime::ZERO, capped)
            };
            let result = detect_audio_beats(&pcm, sr, &cfg);

            let mut markers_added = 0usize;
            if args
                .get("add_markers")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && !result.beats.is_empty()
            {
                let downbeats_only = args
                    .get("downbeats_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let pairs: Vec<(f64, bool)> = result
                    .beats
                    .iter()
                    .map(|b| (b.time_sec, b.is_downbeat))
                    .collect();
                if let Some(cmd) = build_beat_markers_command(&pairs, downbeats_only) {
                    if let EditCommand::Batch { ref commands, .. } = cmd {
                        markers_added = commands.len();
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                }
            }

            let mut cuts_performed = 0usize;
            if let Some(arr) = args.get("cut_item_ids").and_then(Value::as_array)
                && !result.beats.is_empty()
            {
                let ids: Vec<Uuid> = arr
                    .iter()
                    .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .collect();
                let beat_secs: Vec<f64> = result.beats.iter().map(|b| b.time_sec).collect();
                let scene_clone = guard
                    .project
                    .active_scene()
                    .cloned()
                    .ok_or("no active scene")?;
                if let Some((cuts, cmd)) = build_beat_cut_commands(&scene_clone, &ids, &beat_secs) {
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    cuts_performed = cuts;
                }
            }

            let beats_json: Vec<Value> = result
                .beats
                .iter()
                .map(|b| {
                    json!({
                        "time_sec": b.time_sec,
                        "strength": b.strength,
                        "is_downbeat": b.is_downbeat,
                    })
                })
                .collect();
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "bpm": result.bpm,
                        "confidence": result.confidence,
                        "beat_count": result.beats.len(),
                        "markers_added": markers_added,
                        "cuts_performed": cuts_performed,
                        "beats": beats_json,
                    }).to_string()
                }]
            }))
        }
        "set_color_grade" => {
            let mut emitted_luts: Vec<String> = Vec::new();
            if args
                .get("emit_sample_luts")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let dir = guard
                    .folder
                    .as_ref()
                    .map(|f| f.luts_dir())
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_mcp_luts"));
                let paths = emit_sample_cube_luts(&dir)?;
                emitted_luts = paths.into_iter().map(|p| p.display().to_string()).collect();
            }

            let mut grade = if args.get("reset").and_then(Value::as_bool).unwrap_or(false) {
                SceneColorGrade::default()
            } else {
                guard
                    .project
                    .active_scene()
                    .map(|s| s.color_grade.clone())
                    .ok_or("no active scene")?
            };
            if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                grade.enabled = en;
            } else if !args.get("reset").and_then(Value::as_bool).unwrap_or(false) {
                grade.enabled = true;
            }
            if args.get("exposure").is_some() {
                grade.exposure = finite_clamped(args.get("exposure"), 0.0, -3.0, 3.0) as f32;
            }
            if args.get("contrast").is_some() {
                grade.contrast = finite_clamped(args.get("contrast"), 1.0, 0.2, 2.5) as f32;
            }
            if args.get("saturation").is_some() {
                grade.saturation = finite_clamped(args.get("saturation"), 1.0, 0.0, 2.5) as f32;
            }
            if args.get("temperature").is_some() {
                grade.temperature = finite_clamped(args.get("temperature"), 0.0, -1.0, 1.0) as f32;
            }
            if args.get("tint").is_some() {
                grade.tint = finite_clamped(args.get("tint"), 0.0, -1.0, 1.0) as f32;
            }
            let parse_vec3 = |key: &str, lo: f64, hi: f64| -> Option<[f32; 3]> {
                let arr = args.get(key)?.as_array()?;
                if arr.len() != 3 {
                    return None;
                }
                Some([
                    finite_clamped(arr.first(), 0.0, lo, hi) as f32,
                    finite_clamped(arr.get(1), 0.0, lo, hi) as f32,
                    finite_clamped(arr.get(2), 0.0, lo, hi) as f32,
                ])
            };
            if let Some(v) = parse_vec3("lift", -0.5, 0.5) {
                grade.lift = v;
            }
            if let Some(v) = parse_vec3("gamma", -0.5, 0.5) {
                grade.gamma = v;
            }
            if let Some(v) = parse_vec3("gain", 0.2, 2.5) {
                grade.gain = v;
            }
            if let Some(lp) = args.get("lut_path").and_then(Value::as_str) {
                grade.lut_path = if lp.trim().is_empty() {
                    None
                } else {
                    Some(lp.trim().to_string())
                };
            }
            if args.get("lut_intensity").is_some() {
                grade.lut_intensity =
                    finite_clamped(args.get("lut_intensity"), 1.0, 0.0, 1.0) as f32;
            }

            let sess = &mut *guard;
            sess.bus
                .execute(
                    &mut sess.project,
                    EditCommand::SetSceneColorGrade {
                        grade: grade.clone(),
                    },
                    CommandSource::Mcp,
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": json!({
                        "color_grade": grade,
                        "emitted_luts": emitted_luts
                    }).to_string()
                }]
            }))
        }
        "set_expression" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing action")?;
            let id = parse_uuid(args, "id")?;
            match action {
                "list" => {
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    let item = scene
                        .layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == id)
                        .ok_or_else(|| format!("object not found: {id}"))?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": id,
                                "expressions": item.expressions
                            }).to_string()
                        }]
                    }))
                }
                "set" | "clear" => {
                    let property = args
                        .get("property")
                        .and_then(Value::as_str)
                        .ok_or("missing property")?
                        .to_string();
                    let expr = if action == "clear" {
                        None
                    } else {
                        let s = args
                            .get("expression")
                            .and_then(Value::as_str)
                            .ok_or("missing expression")?;
                        compile_expression(s).map_err(|e| e.to_string())?;
                        Some(s.to_string())
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetExpression {
                                item_id: id,
                                property: property.clone(),
                                expression: expr.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": id,
                                "property": property,
                                "expression": expr
                            }).to_string()
                        }]
                    }))
                }
                "evaluate" => {
                    let property = args
                        .get("property")
                        .and_then(Value::as_str)
                        .ok_or("missing property")?;
                    let t = args
                        .get("time_sec")
                        .and_then(Value::as_f64)
                        .map(RationalTime::from_seconds)
                        .unwrap_or(guard.playhead);
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    let (l_idx, item) = scene
                        .layers
                        .iter()
                        .enumerate()
                        .find_map(|(li, l)| l.items.iter().find(|i| i.id == id).map(|it| (li, it)))
                        .ok_or_else(|| format!("object not found: {id}"))?;
                    let val = evaluate_item_property(scene, l_idx, item, property, t, [0.0; 4]);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": id,
                                "property": property,
                                "time_sec": t.as_seconds(),
                                "value": val
                            }).to_string()
                        }]
                    }))
                }
                "bake" => {
                    let property = args
                        .get("property")
                        .and_then(Value::as_str)
                        .ok_or("missing property")?;
                    let step_frames = args
                        .get("step_frames")
                        .and_then(Value::as_u64)
                        .unwrap_or(3)
                        .clamp(1, 60) as u32;
                    let cmd = {
                        let scene = guard.project.active_scene().ok_or("no active scene")?;
                        let (l_idx, item) = scene
                            .layers
                            .iter()
                            .enumerate()
                            .find_map(|(li, l)| {
                                l.items.iter().find(|i| i.id == id).map(|it| (li, it))
                            })
                            .ok_or_else(|| format!("object not found: {id}"))?;
                        build_bake_expression_command(
                            scene,
                            l_idx,
                            item,
                            property,
                            step_frames,
                            |_| [0.0; 4],
                        )?
                    };
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "baked": true,
                                "item_id": id,
                                "property": property,
                                "step_frames": step_frames
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown set_expression action: {other}")),
            }
        }
        "manage_templates" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing action")?;
            let all_motion_templates = || {
                let mut list = builtin_motion_templates();
                if let Some(folder) = guard.folder.as_ref() {
                    list.extend(folder.load_motion_templates());
                }
                list
            };
            match action {
                "list" => {
                    let list = all_motion_templates();
                    let summary: Vec<Value> = list
                        .iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "name": t.name,
                                "category": t.category,
                                "description": t.description,
                                "default_duration_sec": t.responsive_time.default_duration_sec,
                                "exposed_params": t.exposed_params
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "templates": summary }).to_string() }]
                    }))
                }
                "instantiate" => {
                    let tpl_id = args
                        .get("template_id")
                        .and_then(Value::as_str)
                        .ok_or("missing template_id")?;
                    let list = all_motion_templates();
                    let tpl = list
                        .iter()
                        .find(|t| t.id == tpl_id || t.name == tpl_id)
                        .ok_or_else(|| format!("template not found: {tpl_id}"))?
                        .clone();
                    let layer_index =
                        args.get("layer_index").and_then(Value::as_u64).unwrap_or(1) as usize;
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        guard.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    ));
                    let dur_opt = args.get("duration_sec").and_then(Value::as_f64);
                    let overrides = parse_template_param_values(args.get("values"));
                    let cmd = build_instantiate_template_command(
                        &tpl,
                        layer_index,
                        start,
                        dur_opt,
                        &overrides,
                    );
                    let sess = &mut *guard;
                    sess.bus
                        .execute(&mut sess.project, cmd, CommandSource::Mcp, None)
                        .map_err(|e| e.to_string())?;
                    let created_id = sess
                        .project
                        .active_scene()
                        .and_then(|s| s.layers.get(layer_index))
                        .and_then(|l| l.items.last())
                        .map(|i| i.id);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "instantiated": tpl.id,
                                "created_item_id": created_id
                            }).to_string()
                        }]
                    }))
                }
                "update_instance" => {
                    let item_id = parse_uuid(args, "item_id")?;
                    let values = parse_template_param_values(args.get("values"));
                    let duration_sec = args.get("duration_sec").and_then(Value::as_f64);
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::UpdateTemplateInstance {
                                item_id,
                                values,
                                duration_sec,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "updated": item_id }).to_string() }]
                    }))
                }
                "save_from_scene" => {
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("project folder not initialized")?
                        .clone();
                    let scene = guard
                        .project
                        .active_scene()
                        .ok_or("no active scene")?
                        .clone();
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("カスタムテンプレート");
                    let category = args
                        .get("category")
                        .and_then(Value::as_str)
                        .unwrap_or("カスタム");
                    let intro = finite_clamped(args.get("intro_sec"), 0.45, 0.0, 10.0);
                    let outro = finite_clamped(args.get("outro_sec"), 0.45, 0.0, 10.0);
                    let tpl =
                        extract_motion_template_from_scene(&scene, name, category, intro, outro);
                    let saved = folder
                        .save_motion_template(&tpl)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "saved_path": saved.display().to_string(),
                                "template_id": tpl.id,
                                "exposed_count": tpl.exposed_params.len()
                            }).to_string()
                        }]
                    }))
                }
                "list_project_templates" => {
                    let mut list = builtin_project_templates();
                    if let Some(folder) = guard.folder.as_ref() {
                        list.extend(folder.load_project_templates());
                    }
                    let summary: Vec<Value> = list
                        .iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "name": t.name,
                                "description": t.description
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "project_templates": summary }).to_string() }]
                    }))
                }
                "save_project_template" => {
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("project folder not initialized")?
                        .clone();
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("プロジェクト雛形");
                    let tpl = ProjectTemplate {
                        id: format!("proj_custom_{}", &Uuid::new_v4().simple().to_string()[..6]),
                        name: name.to_string(),
                        description: "プロジェクトから保存された雛形".to_string(),
                        project: guard.project.clone(),
                    };
                    let saved = folder
                        .save_project_template(&tpl)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "saved_path": saved.display().to_string(),
                                "id": tpl.id
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown manage_templates action: {other}")),
            }
        }
        "set_scene_motion_blur" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing action")?;
            match action {
                "get" => {
                    let scene = guard.project.active_scene().ok_or("no active scene")?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "motion_blur": scene.motion_blur,
                                "light": scene.camera.light
                            }).to_string()
                        }]
                    }))
                }
                "set_motion_blur" => {
                    let mut mb = guard
                        .project
                        .active_scene()
                        .ok_or("no active scene")?
                        .motion_blur;
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        mb.enabled = en;
                    }
                    if args.get("shutter_angle_deg").is_some() {
                        mb.shutter_angle_deg =
                            finite_clamped(args.get("shutter_angle_deg"), 180.0, 0.0, 360.0) as f32;
                    }
                    if args.get("shutter_phase_deg").is_some() {
                        mb.shutter_phase_deg =
                            finite_clamped(args.get("shutter_phase_deg"), -90.0, -180.0, 180.0)
                                as f32;
                    }
                    if let Some(ps) = args.get("preview_samples").and_then(Value::as_u64) {
                        mb.preview_samples = ps.clamp(1, 8) as u32;
                    }
                    if let Some(es) = args.get("export_samples").and_then(Value::as_u64) {
                        mb.export_samples = es.clamp(1, 16) as u32;
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetMotionBlur { motion_blur: mb },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "motion_blur": mb }).to_string() }]
                    }))
                }
                "set_light_3d" => {
                    let mut cam = guard
                        .project
                        .active_scene()
                        .ok_or("no active scene")?
                        .camera;
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        cam.light.enabled = en;
                        if en {
                            cam.enabled = true;
                        }
                    }
                    if let Some(lt) = args.get("light_type").and_then(Value::as_str) {
                        cam.light.light_type = match lt {
                            "point" => Light3DType::Point,
                            _ => Light3DType::Directional,
                        };
                    }
                    if let Some(arr) = args.get("position").and_then(Value::as_array)
                        && arr.len() >= 3
                    {
                        cam.light.position = [
                            finite_clamped(
                                Some(&arr[0]),
                                cam.light.position[0] as f64,
                                -5000.0,
                                5000.0,
                            ) as f32,
                            finite_clamped(
                                Some(&arr[1]),
                                cam.light.position[1] as f64,
                                -5000.0,
                                5000.0,
                            ) as f32,
                            finite_clamped(
                                Some(&arr[2]),
                                cam.light.position[2] as f64,
                                -5000.0,
                                5000.0,
                            ) as f32,
                        ];
                    }
                    if args.get("intensity").is_some() {
                        cam.light.intensity =
                            finite_clamped(args.get("intensity"), 1.15, 0.0, 3.0) as f32;
                    }
                    if args.get("ambient").is_some() {
                        cam.light.ambient =
                            finite_clamped(args.get("ambient"), 0.35, 0.0, 1.5) as f32;
                    }
                    if let Some(cs) = args.get("cast_shadow").and_then(Value::as_bool) {
                        cam.light.cast_shadow = cs;
                    }
                    if args.get("shadow_softness").is_some() {
                        cam.light.shadow_softness =
                            finite_clamped(args.get("shadow_softness"), 8.0, 0.0, 24.0) as f32;
                    }
                    if args.get("shadow_opacity").is_some() {
                        cam.light.shadow_opacity =
                            finite_clamped(args.get("shadow_opacity"), 0.45, 0.0, 1.0) as f32;
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
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "light": cam.light }).to_string() }]
                    }))
                }
                "set_particle" => {
                    let item_id = parse_uuid(args, "item_id")?;
                    let mut cfg = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::ParticleEmitter { config } = i.kind
                                {
                                    return Some(config);
                                }
                                None
                            })
                        })
                        .ok_or_else(|| format!("particle item not found: {item_id}"))?;
                    if let Some(p) = args.get("preset").and_then(Value::as_str) {
                        let preset = match p {
                            "snow" => ParticlePreset::Snow,
                            "sparks" => ParticlePreset::Sparks,
                            "light_orbs" => ParticlePreset::LightOrbs,
                            "speed_lines" => ParticlePreset::SpeedLines,
                            "action_lines" => ParticlePreset::ActionLines,
                            _ => ParticlePreset::Confetti,
                        };
                        cfg = ParticleConfig::for_preset(preset);
                    }
                    if let Some(c) = args.get("count").and_then(Value::as_u64) {
                        cfg.count = c.clamp(1, 600) as u32;
                    }
                    if let Some(sd) = args.get("seed").and_then(Value::as_u64) {
                        cfg.seed = sd;
                    }
                    if args.get("speed").is_some() {
                        cfg.speed =
                            finite_clamped(args.get("speed"), cfg.speed as f64, 0.0, 4000.0) as f32;
                    }
                    if args.get("size_px").is_some() {
                        cfg.size_px =
                            finite_clamped(args.get("size_px"), cfg.size_px as f64, 1.0, 200.0)
                                as f32;
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetParticleConfig {
                                item_id,
                                config: cfg,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "particle": cfg }).to_string() }]
                    }))
                }
                "set_vector_shape" => {
                    let item_id = parse_uuid(args, "item_id")?;
                    let mut cfg = guard
                        .project
                        .active_scene()
                        .and_then(|s| {
                            s.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::VectorShape { config } = &i.kind
                                {
                                    return Some(config.clone());
                                }
                                None
                            })
                        })
                        .ok_or_else(|| format!("vector_shape item not found: {item_id}"))?;
                    if args.get("trim_start").is_some() {
                        cfg.trim_start =
                            finite_clamped(args.get("trim_start"), cfg.trim_start as f64, 0.0, 1.0)
                                as f32;
                    }
                    if args.get("trim_end").is_some() {
                        cfg.trim_end =
                            finite_clamped(args.get("trim_end"), cfg.trim_end as f64, 0.0, 1.0)
                                as f32;
                    }
                    if args.get("trim_offset").is_some() {
                        cfg.trim_offset = finite_clamped(
                            args.get("trim_offset"),
                            cfg.trim_offset as f64,
                            -10.0,
                            10.0,
                        ) as f32;
                    }
                    if args.get("stroke_width").is_some() {
                        cfg.stroke_width = finite_clamped(
                            args.get("stroke_width"),
                            cfg.stroke_width as f64,
                            0.0,
                            128.0,
                        ) as f32;
                    }
                    if args.get("dash_length").is_some() {
                        cfg.dash_length = finite_clamped(
                            args.get("dash_length"),
                            cfg.dash_length as f64,
                            0.0,
                            500.0,
                        ) as f32;
                    }
                    if args.get("dash_gap").is_some() {
                        cfg.dash_gap =
                            finite_clamped(args.get("dash_gap"), cfg.dash_gap as f64, 0.0, 500.0)
                                as f32;
                    }
                    let sess = &mut *guard;
                    sess.bus
                        .execute(
                            &mut sess.project,
                            EditCommand::SetVectorShape {
                                item_id,
                                config: cfg.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "vector_shape": cfg }).to_string() }]
                    }))
                }
                other => Err(format!("unknown set_scene_motion_blur action: {other}")),
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
        "validate_and_package_project" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "validate" => {
                    let report = validate_project_delivery(&guard.project, guard.folder.as_ref());
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&report).unwrap_or_default()
                        }]
                    }))
                }
                "archive_zip" => {
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("ZIP パッケージ化には保存済みプロジェクトフォルダが必要です")?;
                    let raw_name = args
                        .get("file_name")
                        .and_then(Value::as_str)
                        .unwrap_or("project_archive");
                    let safe_stem: String = raw_name
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                        .take(48)
                        .collect();
                    let stem = if safe_stem.is_empty() {
                        "project_archive".to_string()
                    } else {
                        safe_stem
                    };
                    let out_dir = folder.exports_dir();
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let zip_path = out_dir.join(format!("{stem}.zip"));
                    let summary =
                        archive_project_to_zip(folder, &zip_path).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&summary).unwrap_or_default()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown validate_and_package_project action: {other}"
                )),
            }
        }
        "manage_ecosystem" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing 'action'")?;
            match action {
                "get_settings" => {
                    let settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&settings).unwrap_or_default()
                        }]
                    }))
                }
                "set_settings" => {
                    let mut settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    if let Some(lang) = args.get("language").and_then(Value::as_str) {
                        settings.language = match lang {
                            "en" => UiLanguage::English,
                            _ => UiLanguage::Japanese,
                        };
                    }
                    if let Some(th) = args.get("theme").and_then(Value::as_str) {
                        settings.theme = match th {
                            "midnight_oled" => UiThemePreset::MidnightOled,
                            "high_contrast" => UiThemePreset::HighContrast,
                            "light_daylight" => UiThemePreset::LightDaylight,
                            _ => UiThemePreset::DarkStudio,
                        };
                    }
                    if args.get("font_scale").is_some() {
                        settings.font_scale = finite_clamped(
                            args.get("font_scale"),
                            f64::from(settings.font_scale),
                            0.85,
                            1.5,
                        ) as f32;
                    }
                    if let Some(kp) = args.get("keymap_preset").and_then(Value::as_str) {
                        let preset = match kp {
                            "premiere" => KeymapPreset::Premiere,
                            "davinci" => KeymapPreset::DaVinci,
                            "after_effects" => KeymapPreset::AfterEffects,
                            _ => KeymapPreset::AviUtl2,
                        };
                        settings.keymap = EditorKeymap::for_preset(preset);
                    }
                    if let Some(t) = args.get("parallel_export_threads").and_then(Value::as_u64) {
                        settings.export_worker_threads = (t as usize).clamp(1, 16);
                    }
                    if let Some(f) = guard.folder.as_ref() {
                        f.save_user_settings(&settings).map_err(|e| e.to_string())?;
                    }
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&settings).unwrap_or_default()
                        }]
                    }))
                }
                "export_keymap_json" => {
                    let settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    let km_json = settings
                        .keymap
                        .to_json_pretty()
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": km_json
                        }]
                    }))
                }
                "import_keymap_json" => {
                    let raw = args
                        .get("keymap_json")
                        .and_then(Value::as_str)
                        .ok_or("missing 'keymap_json'")?;
                    let km = EditorKeymap::from_json(raw).map_err(|e| e.to_string())?;
                    let mut settings = guard
                        .folder
                        .as_ref()
                        .map(|f| f.load_user_settings())
                        .unwrap_or_default();
                    settings.keymap = km;
                    if let Some(f) = guard.folder.as_ref() {
                        f.save_user_settings(&settings).map_err(|e| e.to_string())?;
                    }
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "imported_bindings": settings.keymap.bindings.len(), "preset": settings.keymap.preset }).to_string()
                        }]
                    }))
                }
                "list_registry" => {
                    let catalog: Vec<_> = official_registry_catalog()
                        .into_iter()
                        .map(|(entry, _)| entry)
                        .collect();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&catalog).unwrap_or_default()
                        }]
                    }))
                }
                "install_extension" => {
                    let ext_id = args
                        .get("extension_id")
                        .and_then(Value::as_str)
                        .ok_or("missing 'extension_id'")?;
                    let folder = guard
                        .folder
                        .as_ref()
                        .ok_or("拡張機能インストールには保存済みプロジェクトフォルダが必要です")?;
                    let (entry, payload) = official_registry_catalog()
                        .into_iter()
                        .find(|(e, _)| e.id == ext_id)
                        .ok_or_else(|| {
                            format!("extension not found in official catalog: {ext_id}")
                        })?;
                    let installed_path =
                        verify_and_install_extension(&entry, &payload, folder.root())
                            .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "installed_id": entry.id,
                                "sha256_verified": entry.sha256,
                                "permissions": entry.permissions,
                                "path": installed_path.display().to_string()
                            }).to_string()
                        }]
                    }))
                }
                "hw_status" => {
                    let cli = FfmpegCli::default();
                    let hw_decoders: Vec<&str> = cli
                        .detect_available_hw_decoders()
                        .into_iter()
                        .map(HwDecodeBackend::display_name)
                        .collect();
                    let hw_encoders: Vec<String> = cli
                        .detect_available_hw_encoders()
                        .into_iter()
                        .map(|e| format!("{e:?}"))
                        .collect();
                    let resolved_decode = cli.resolve_hw_decoder(HwDecodeBackend::Auto);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "available_hw_decoders": hw_decoders,
                                "resolved_hw_decode": resolved_decode.display_name(),
                                "available_hw_encoders": hw_encoders,
                                "cpu_threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown manage_ecosystem action: {other}")),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn parse_template_param_values(val: Option<&Value>) -> BTreeMap<String, TemplateParamValue> {
    let mut out = BTreeMap::new();
    let Some(obj) = val.and_then(Value::as_object) else {
        return out;
    };
    for (k, v) in obj {
        if let Some(s) = v.as_str() {
            if s.starts_with('#')
                && (s.len() == 7 || s.len() == 9)
                && let Some(col) = RgbaColor::from_hex(s)
            {
                out.insert(k.clone(), TemplateParamValue::Color(col));
                continue;
            }
            out.insert(k.clone(), TemplateParamValue::Text(s.to_string()));
        } else if let Some(n) = v.as_f64()
            && n.is_finite()
        {
            out.insert(k.clone(), TemplateParamValue::Number(n as f32));
        }
    }
    out
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

    #[test]
    fn v04_mcp_tools_markers_refine_wrap_and_guarded_export() {
        let mut proj = Project::new("McpV04", ProjectSettings::default());
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .push(TimelineItem::new(
                "LongSub",
                RationalTime::ZERO,
                RationalTime::from_seconds(2.0),
                ItemKind::Text {
                    content: "これは非常に長い字幕テキストなので自動で読みやすく改行されます。"
                        .into(),
                    font_family: "sans-serif".into(),
                    font_size: 40.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
        let session = Arc::new(Mutex::new(EditorSession::new(proj)));

        call_tool(
            &session,
            "manage_markers",
            &json!({"action": "add", "time_sec": 12.0, "label": "本編開始"}),
        )
        .unwrap();
        let ch = call_tool(
            &session,
            "manage_markers",
            &json!({"action": "youtube_chapters"}),
        )
        .unwrap();
        let ch_text = ch["content"][0]["text"].as_str().unwrap();
        assert!(ch_text.contains("00:12 本編開始"));

        call_tool(
            &session,
            "refine_subtitles",
            &json!({"mode": "local_wrap", "max_chars_per_line": 16}),
        )
        .unwrap();
        {
            let g = session.lock().unwrap();
            if let ItemKind::Text { content, .. } =
                &g.project.active_scene().unwrap().layers[1].items[0].kind
            {
                assert!(content.contains('\n'));
            }
        }

        assert!(call_tool(&session, "start_export", &json!({"user_confirmed": false})).is_err());
        let exp = call_tool(
            &session,
            "start_export",
            &json!({"user_confirmed": true, "format": "wav_audio_only", "file_name": "../escape_test"}),
        )
        .unwrap();
        let out_str = exp["content"][0]["text"].as_str().unwrap();
        assert!(out_str.contains("escape_test.wav"));
        assert!(!out_str.contains(".."));
    }

    #[test]
    fn v06_mcp_tools_ducking_script_and_interop() {
        let proj = Project::new("McpV06", ProjectSettings::default());
        let session = Arc::new(Mutex::new(EditorSession::new(proj)));

        // 1. set_audio_ducking
        call_tool(
            &session,
            "set_audio_ducking",
            &json!({
                "enabled": true,
                "voice_layer_index": 3,
                "bgm_layer_index": 2,
                "reduction_db": -14.0
            }),
        )
        .unwrap();
        {
            let g = session.lock().unwrap();
            let d = g.project.active_scene().unwrap().audio_ducking;
            assert!(d.enabled);
            assert_eq!(d.voice_layer_index, 3);
            assert_eq!(d.bgm_layer_index, 2);
            assert!((d.reduction_db - (-14.0)).abs() < 1e-4);
        }

        // 2. import_script (Markdown dialogue)
        let md = "**霊夢** [smile]: こんにちは！\n**魔理沙**: v0.6 の新機能を紹介するぜ！\n";
        let res = call_tool(
            &session,
            "import_script",
            &json!({"content": md, "subtitle_layer": 1, "start_sec": 0.0}),
        )
        .unwrap();
        assert!(
            res["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"imported_lines\":2")
        );

        // 3. export_subtitles_or_otio (SRT + OTIO)
        let srt_res = call_tool(
            &session,
            "export_subtitles_or_otio",
            &json!({"format": "srt"}),
        )
        .unwrap();
        let srt_json: Value =
            serde_json::from_str(srt_res["content"][0]["text"].as_str().unwrap()).unwrap();
        let srt_body = srt_json["content"].as_str().unwrap();
        assert!(srt_body.contains("こんにちは！"));
        assert!(srt_body.contains("-->"));

        let otio_res = call_tool(
            &session,
            "export_subtitles_or_otio",
            &json!({"format": "otio"}),
        )
        .unwrap();
        assert!(
            otio_res["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Timeline.1")
        );

        // 4. import_subtitles_or_exo (SRT back into layer 0)
        let imp = call_tool(
            &session,
            "import_subtitles_or_exo",
            &json!({"format": "srt", "content": srt_body, "layer_index": 0}),
        )
        .unwrap();
        assert!(
            imp["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"imported_count\":2")
        );
    }

    #[test]
    fn v07_mcp_tools_mask_wasm_and_extract_highlights() {
        let mut proj = Project::new("McpV07", ProjectSettings::default());
        let item = TimelineItem::new(
            "HeroSub",
            RationalTime::from_seconds(2.0),
            RationalTime::from_seconds(8.0),
            ItemKind::Text {
                content: "これはヤバい！すごい結論まとめポイント！".into(),
                font_family: "sans-serif".into(),
                font_size: 46.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        let item_id = item.id;
        proj.active_scene_mut().unwrap().layers[1].items.push(item);
        let session = Arc::new(Mutex::new(EditorSession::new(proj)));

        // 1. set_bezier_mask
        call_tool(
            &session,
            "set_bezier_mask",
            &json!({"id": item_id.to_string(), "preset": "ellipse", "feather_px": 6.0}),
        )
        .unwrap();
        {
            let g = session.lock().unwrap();
            assert_eq!(
                g.project.active_scene().unwrap().layers[1].items[0]
                    .masks
                    .len(),
                1
            );
        }

        // 2. list_or_reload_wasm_plugins with emit_samples in temp dir
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v07_wasm_{}", Uuid::new_v4()));
        let wasm_res = call_tool(
            &session,
            "list_or_reload_wasm_plugins",
            &json!({"plugins_dir": tmp_dir.display().to_string(), "emit_samples": true}),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let wasm_txt = wasm_res["content"][0]["text"].as_str().unwrap();
        assert!(wasm_txt.contains("wasm.wasm_invert"), "wasm_txt={wasm_txt}");

        // 3. extract_highlights + auto create 9:16 Shorts scene
        let hl_res = call_tool(
            &session,
            "extract_highlights",
            &json!({"clip_duration_sec": 5.0, "create_shorts_scene": true, "shorts_name": "AutoShorts"}),
        )
        .unwrap();
        let hl_txt = hl_res["content"][0]["text"].as_str().unwrap();
        assert!(hl_txt.contains("created_shorts_scene_id"));
        {
            let g = session.lock().unwrap();
            let s = g.project.active_scene().unwrap();
            assert_eq!(s.name, "AutoShorts");
            assert_eq!((s.width, s.height), (1080, 1920));
        }
    }

    #[test]
    fn v071_mcp_tools_presets_and_marker_update() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v071_presets_{}", Uuid::new_v4()));
        let mut proj = Project::new("McpV071", ProjectSettings::default());
        let item = TimelineItem::new(
            "Sub",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Text {
                content: "プリセットテスト".into(),
                font_family: "sans-serif".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        let item_id = item.id;
        proj.active_scene_mut().unwrap().layers[1].items.push(item);
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let mut ed = EditorSession::new(proj);
        ed.folder = Some(folder);
        let session = Arc::new(Mutex::new(ed));

        // 1. manage_markers add + update
        let added = call_tool(
            &session,
            "manage_markers",
            &json!({"action": "add", "time_sec": 4.0, "label": "仮ラベル"}),
        )
        .unwrap();
        let added_json: Value =
            serde_json::from_str(added["content"][0]["text"].as_str().unwrap()).unwrap();
        let mid = added_json["marker_id"].as_str().unwrap();

        call_tool(
            &session,
            "manage_markers",
            &json!({"action": "update", "marker_id": mid, "label": "確定チャプター", "color_hex": "#ff4466"}),
        )
        .unwrap();
        {
            let g = session.lock().unwrap();
            let m = &g.project.active_scene().unwrap().markers[0];
            assert_eq!(m.label, "確定チャプター");
            assert_eq!(m.color.to_hex_rgb(), "#ff4466");
        }

        // 2. manage_presets list + apply_text_style + save_text_style + apply_effect_preset + save_effect_preset
        let list = call_tool(&session, "manage_presets", &json!({"action": "list"})).unwrap();
        let list_txt = list["content"][0]["text"].as_str().unwrap();
        assert!(list_txt.contains("ゆっくり座布団字幕"));
        assert!(list_txt.contains("シネマティック暖色"));

        call_tool(
            &session,
            "manage_presets",
            &json!({"action": "apply_text_style", "id": item_id.to_string(), "name": "ゆっくり座布団字幕"}),
        )
        .unwrap();
        call_tool(
            &session,
            "manage_presets",
            &json!({"action": "apply_effect_preset", "id": item_id.to_string(), "name": "シネマティック暖色"}),
        )
        .unwrap();
        call_tool(
            &session,
            "manage_presets",
            &json!({"action": "save_text_style", "id": item_id.to_string(), "name": "カスタム座布団"}),
        )
        .unwrap();
        call_tool(
            &session,
            "manage_presets",
            &json!({"action": "save_effect_preset", "id": item_id.to_string(), "name": "カスタムシネマ"}),
        )
        .unwrap();

        let list2 = call_tool(&session, "manage_presets", &json!({"action": "list"})).unwrap();
        let list2_txt = list2["content"][0]["text"].as_str().unwrap();
        assert!(list2_txt.contains("カスタム座布団"));
        assert!(list2_txt.contains("カスタムシネマ"));
        {
            let g = session.lock().unwrap();
            let it = &g.project.active_scene().unwrap().layers[1].items[0];
            assert!(it.text_style.box_enabled);
            assert_eq!(it.effect_chain.len(), 2);
        }
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v08_mcp_tools_text_edit_beats_and_color_grade() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v08_{}", Uuid::new_v4()));
        let mut proj = Project::new("McpV08", ProjectSettings::default());
        let mk_sub = |name: &str, s: f64, d: f64, txt: &str| {
            TimelineItem::new(
                name,
                RationalTime::from_seconds(s),
                RationalTime::from_seconds(d),
                ItemKind::Text {
                    content: txt.into(),
                    font_family: "sans-serif".into(),
                    font_size: 44.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            )
        };
        proj.active_scene_mut().unwrap().layers[1].items.extend([
            mk_sub("T1", 0.0, 2.0, "えーと…"),
            mk_sub("T2", 2.0, 2.0, "本編スタート！"),
            mk_sub("T3", 4.0, 2.0, "まとめです"),
        ]);
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let mut ed = EditorSession::new(proj);
        ed.folder = Some(folder);
        let session = Arc::new(Mutex::new(ed));

        // 1. text_based_edit list + remove_fillers
        let listed = call_tool(&session, "text_based_edit", &json!({"action": "list"})).unwrap();
        assert!(
            listed["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"is_filler_only\":true")
        );
        call_tool(
            &session,
            "text_based_edit",
            &json!({"action": "remove_fillers"}),
        )
        .unwrap();
        {
            let g = session.lock().unwrap();
            let items = &g.project.active_scene().unwrap().layers[1].items;
            assert_eq!(items.len(), 2);
            assert!((items[0].start.as_seconds() - 0.0).abs() < 1e-6);
        }

        // 2. detect_beats (no audio -> safe zero result)
        let beats = call_tool(&session, "detect_beats", &json!({})).unwrap();
        assert!(
            beats["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"beat_count\":0")
        );

        // 3. set_color_grade + emit_sample_luts + builtin LUT
        let cg = call_tool(
            &session,
            "set_color_grade",
            &json!({
                "enabled": true,
                "exposure": 0.35,
                "temperature": 0.25,
                "gain": [1.1, 1.0, 0.9],
                "lut_path": "builtin:teal_orange",
                "lut_intensity": 0.75,
                "emit_sample_luts": true
            }),
        )
        .unwrap();
        let cg_txt = cg["content"][0]["text"].as_str().unwrap();
        assert!(cg_txt.contains("teal_orange_cinema.cube"));
        {
            let g = session.lock().unwrap();
            let grade = &g.project.active_scene().unwrap().color_grade;
            assert!(grade.enabled);
            assert_eq!(grade.lut_path.as_deref(), Some("builtin:teal_orange"));
            assert!((grade.exposure - 0.35).abs() < 1e-4);
        }
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v09_mcp_tools_expressions_templates_and_fx() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v09_{}", Uuid::new_v4()));
        let proj = Project::new("McpV09", ProjectSettings::default());
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let mut ed = EditorSession::new(proj);
        ed.folder = Some(folder);
        let session = Arc::new(Mutex::new(ed));

        // 1. add_object particle_emitter & vector_shape
        let p_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "SnowFX", "kind": "particle_emitter", "preset": "snow", "duration_sec": 4.0}),
        )
        .unwrap();
        let p_id: String = serde_json::from_str::<Value>(
            p_res["content"][0]["text"].as_str().unwrap(),
        )
        .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();

        let v_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Wave", "kind": "vector_shape", "preset": "underline_wave", "duration_sec": 3.0}),
        )
        .unwrap();
        let v_id: String = serde_json::from_str::<Value>(
            v_res["content"][0]["text"].as_str().unwrap(),
        )
        .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();

        // 2. set_expression set + evaluate + bake
        call_tool(
            &session,
            "set_expression",
            &json!({"action": "set", "id": v_id, "property": "trim_end", "expression": "ease(local_time, 0, 1, 0, 1)"}),
        )
        .unwrap();
        let eval = call_tool(
            &session,
            "set_expression",
            &json!({"action": "evaluate", "id": v_id, "property": "trim_end", "time_sec": 0.5}),
        )
        .unwrap();
        let eval_v = serde_json::from_str::<Value>(eval["content"][0]["text"].as_str().unwrap())
            .unwrap()["value"]
            .as_f64()
            .unwrap();
        assert!((eval_v - 0.5).abs() < 1e-2);
        call_tool(
            &session,
            "set_expression",
            &json!({"action": "bake", "id": v_id, "property": "trim_end", "step_frames": 15}),
        )
        .unwrap();

        // 3. manage_templates list + instantiate + update_instance
        let t_list = call_tool(&session, "manage_templates", &json!({"action": "list"})).unwrap();
        assert!(
            t_list["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("builtin_lower_third_news")
        );
        let inst = call_tool(
            &session,
            "manage_templates",
            &json!({
                "action": "instantiate",
                "template_id": "builtin_lower_third_news",
                "layer_index": 2,
                "start_sec": 0.0,
                "duration_sec": 6.0,
                "values": { "title": "AI テンプレート速報" }
            }),
        )
        .unwrap();
        let inst_id = serde_json::from_str::<Value>(inst["content"][0]["text"].as_str().unwrap())
            .unwrap()["created_item_id"]
            .as_str()
            .unwrap()
            .to_string();
        call_tool(
            &session,
            "manage_templates",
            &json!({
                "action": "update_instance",
                "item_id": inst_id,
                "duration_sec": 8.0,
                "values": { "title": "更新後見出し", "accent_color": "#ff4488" }
            }),
        )
        .unwrap();

        // 4. set_scene_motion_blur + set_light_3d + set_particle
        call_tool(
            &session,
            "set_scene_motion_blur",
            &json!({"action": "set_motion_blur", "enabled": true, "shutter_angle_deg": 240.0, "preview_samples": 4}),
        )
        .unwrap();
        call_tool(
            &session,
            "set_scene_motion_blur",
            &json!({"action": "set_light_3d", "enabled": true, "light_type": "point", "intensity": 1.6, "cast_shadow": true}),
        )
        .unwrap();
        call_tool(
            &session,
            "set_scene_motion_blur",
            &json!({"action": "set_particle", "item_id": p_id, "count": 180}),
        )
        .unwrap();

        {
            let g = session.lock().unwrap();
            let s = g.project.active_scene().unwrap();
            assert!(s.motion_blur.enabled);
            assert!((s.motion_blur.shutter_angle_deg - 240.0).abs() < 1e-3);
            assert!(s.camera.light.enabled);
            assert_eq!(s.camera.light.light_type, Light3DType::Point);
        }

        // Verify exact MCP tool count = 54 (v1.0)
        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 54);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v10_mcp_tools_multicam_nle_delivery_and_ecosystem() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v10_{}", Uuid::new_v4()));
        let proj = Project::new("McpV10", ProjectSettings::default());
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let mut ed = EditorSession::new(proj);
        ed.folder = Some(folder);
        let session = Arc::new(Mutex::new(ed));

        // 1. Add two camera clips and test sync_multicam (manual_offsets + apply_switch_cuts)
        let c1_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "Cam1", "kind": "shape", "start_sec": 0.0, "duration_sec": 6.0}),
        )
        .unwrap();
        let c1_id = serde_json::from_str::<Value>(c1_res["content"][0]["text"].as_str().unwrap())
            .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();
        let c2_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Cam2", "kind": "shape", "start_sec": 0.5, "duration_sec": 6.0}),
        )
        .unwrap();
        let c2_id = serde_json::from_str::<Value>(c2_res["content"][0]["text"].as_str().unwrap())
            .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();

        call_tool(
            &session,
            "sync_multicam",
            &json!({
                "action": "auto_sync_audio",
                "manual_offsets": [
                    {"id": c1_id, "offset_sec": 0.0},
                    {"id": c2_id, "offset_sec": -0.5}
                ],
                "apply": true
            }),
        )
        .unwrap();
        let sw_res = call_tool(
            &session,
            "sync_multicam",
            &json!({
                "action": "apply_switch_cuts",
                "angle_layers": [0, 1],
                "output_layer_index": 2,
                "range_start_sec": 0.0,
                "range_end_sec": 5.0,
                "cuts": [
                    {"time_sec": 0.0, "active_angle": 0},
                    {"time_sec": 2.5, "active_angle": 1}
                ]
            }),
        )
        .unwrap();
        assert!(
            sw_res["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"generated_clips\":2")
        );

        // 2. nle_pro_edit: group + unlink + set_layer_mixer
        call_tool(
            &session,
            "nle_pro_edit",
            &json!({"action": "group", "ids": [c1_id, c2_id]}),
        )
        .unwrap();
        call_tool(
            &session,
            "nle_pro_edit",
            &json!({
                "action": "set_layer_mixer",
                "layer_index": 3,
                "volume": 0.8,
                "pan": -0.5,
                "solo": true,
                "bus": "Voice",
                "send_reverb": 0.3
            }),
        )
        .unwrap();
        {
            let g = session.lock().unwrap();
            let s = g.project.active_scene().unwrap();
            assert!(s.layers[0].items[0].group_id.is_some());
            assert_eq!(s.layers[0].items[0].group_id, s.layers[1].items[0].group_id);
            assert!(s.layers[3].solo);
            assert!((s.layers[3].pan - (-0.5)).abs() < 1e-4);
            assert_eq!(s.layers[3].bus, "Voice");
        }

        // 3. validate_and_package_project: validate + archive_zip
        let val = call_tool(
            &session,
            "validate_and_package_project",
            &json!({"action": "validate"}),
        )
        .unwrap();
        assert!(
            val["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"passed\":true")
        );
        let zip_res = call_tool(
            &session,
            "validate_and_package_project",
            &json!({"action": "archive_zip", "file_name": "v10_release_pack"}),
        )
        .unwrap();
        let zip_json: Value =
            serde_json::from_str(zip_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(zip_json["file_count"].as_u64().unwrap() >= 1);
        assert!(std::path::Path::new(zip_json["zip_path"].as_str().unwrap()).exists());

        // 4. manage_ecosystem: settings + keymap json + SHA-256 registry install + hw_status
        call_tool(
            &session,
            "manage_ecosystem",
            &json!({
                "action": "set_settings",
                "language": "en",
                "theme": "high_contrast",
                "font_scale": 1.15,
                "keymap_preset": "premiere",
                "parallel_export_threads": 4
            }),
        )
        .unwrap();
        let km = call_tool(
            &session,
            "manage_ecosystem",
            &json!({"action": "export_keymap_json"}),
        )
        .unwrap();
        let km_txt = km["content"][0]["text"].as_str().unwrap();
        assert!(km_txt.contains("premiere"));
        call_tool(
            &session,
            "manage_ecosystem",
            &json!({"action": "import_keymap_json", "keymap_json": km_txt}),
        )
        .unwrap();
        let reg = call_tool(
            &session,
            "manage_ecosystem",
            &json!({"action": "list_registry"}),
        )
        .unwrap();
        assert!(
            reg["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("official_wgsl_film_vignette")
        );
        let inst = call_tool(
            &session,
            "manage_ecosystem",
            &json!({"action": "install_extension", "extension_id": "official_wgsl_film_vignette"}),
        )
        .unwrap();
        assert!(
            inst["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("sha256_verified")
        );
        let hw = call_tool(
            &session,
            "manage_ecosystem",
            &json!({"action": "hw_status"}),
        )
        .unwrap();
        assert!(
            hw["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("resolved_hw_decode")
        );

        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 54);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
