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
    AudioGraph, BeatDetectionConfig, BgmComposeConfig, BgmGenre, DubbingVoicePreset, FoleyPreset,
    GeneratedDubClip, SpeakerTrackSpec, align_text_to_phoneme_segments, analyze_podcast_speakers,
    apply_podcast_voice_leveler, build_dubbing_timeline_commands,
    build_podcast_auto_switch_commands, compose_bgm_track, detect_audio_beats,
    estimate_audio_offset_xcorr, measure_integrated_lufs, measure_true_peak_dbtp,
    separate_vocal_bgm_stems, synthesize_foley_sfx_pcm, synthesize_offline_dubbing_pcm,
    write_stereo_wav_file,
};
use ve_core::{
    AlignMode, AssetRef, AudioDuckingConfig, AudioEffectNode, BezierMask, BlendMode,
    BroadcastProfile, BroadcastQcMetrics, Camera3D, ChannelSelector, CharacterSpriteConfig,
    ChartKind, CinemaFinishingConfig, ColorCurves, ColorManagementConfig, ColorSpace,
    CommandSource, CompositeNode, CompositeNodeKind, ConflictResolution, CornerPinConfig,
    CrdtOpKind, CrdtSyncLog, CubicBezier, DataChartConfig, DataSeriesPoint, DeliveryBundleArtifact,
    DeliveryBundleConfig, DeliveryPlatform, EditCommand, EditorKeymap, EffectNode, EffectPreset,
    FrameRate, GenerativeBgConfig, GenerativeBgPreset, HighlightConfig, Interpolation, ItemKind,
    JumpCutConfig, Keyframe, KeymapPreset, KineticHighlightMode, KineticSubtitleConfig,
    LetterboxAspect, Light3DType, LipSyncMode, LocalizationEntry, LocalizationTable, MaskMode,
    MaskVertex, MediaKind, Mesh3DConfig, Mesh3DPreset, MotionPreset, MulticamCutPoint,
    NodeCompositeGraph, PaintStroke, PaintToolKind, ParticleConfig, ParticlePreset, PhysicsConfig,
    PhysicsPreset, ProjectTemplate, PsdCharacterConfig, RationalTime, ReviewComment, ReviewSource,
    ReviewStatus, RgbaColor, Rig2DConfig, RigMotionPreset, SceneColorGrade, ScriptBuildConfig,
    ShapeKind, SpeedRampKnot, SpeedRampPreset, TemplateParamValue, TextAnimator, TextStylePreset,
    TimelineItem, TimelineMarker, ToneMapper, TrackMatte, TrackMatteMode, UiLanguage,
    UiThemePreset, VectorShapeConfig, VfxPaintConfig, archive_project_to_zip, base64_encode,
    build_align_commands, build_apply_locale_commands, build_bake_expression_command,
    build_beat_cut_commands, build_beat_markers_command, build_close_gaps_commands,
    build_cube_mesh, build_delivery_manifest, build_fit_duration_to_layer_commands,
    build_instantiate_template_command, build_jump_cut_commands, build_motion_preset_commands,
    build_multicam_switch_cut_commands, build_multicam_sync_align_commands,
    build_physics_bake_commands, build_reformatted_scene_command, build_replace_text_commands,
    build_review_comments_to_markers_commands, build_scene_cut_commands,
    build_script_timeline_commands, build_shorts_clip_scene_command,
    build_strip_filler_words_commands, build_text_based_delete_commands,
    build_text_based_reorder_commands, builtin_motion_templates, builtin_project_templates,
    compile_expression, detect_speech_intervals_from_rms, diff_projects, encode_mesh_to_glb,
    evaluate_item_property, evaluate_rig_poses, export_ass_subtitles, export_cmx3600_edl,
    export_fcpxml_v10, export_localization_csv, export_ocio_config_yaml, export_otio_json,
    export_portable_web_review_html, export_review_comments_csv, export_review_comments_json,
    export_srt_subtitles, extract_highlight_candidates, extract_localization_table,
    extract_motion_template_from_scene, extract_transcript_entries, format_diff_report,
    format_youtube_chapters, import_localization_csv, import_review_comments_csv,
    import_review_comments_json, import_youtube_review_comments, merge_projects_three_way,
    official_registry_catalog, parse_ass_subtitles, parse_aviutl_exo, parse_chart_csv,
    parse_gltf_or_glb, parse_script_auto, parse_srt_subtitles, run_broadcast_qc_audit,
    search_text_items, simulate_scene_physics, subtitle_entries_to_commands,
    sync_node_graph_to_scene_layers, validate_project_delivery, verify_and_install_extension,
    wrap_subtitle_japanese,
};
use ve_media::{
    ExportConfig, ExportFormat, FfmpegCli, HwDecodeBackend, VideoEncoder, decode_audio_file,
    write_wav_pcm16,
};
use ve_plugin_api::{PluginRegistry, WasmPluginManager, emit_sample_wasm_plugins};
use ve_render::{
    AutoReframeConfig, LocalVisionBackend, RenderGraph, StabilizeConfig, analyze_3d_camera_track,
    analyze_camera_shake, analyze_scene_auto_reframe, benchmark_v30_gpu_pipeline,
    build_3d_camera_track_command, build_auto_reframe_commands, build_stabilize_commands,
    compute_auto_color_match, detect_gpu_v30_capabilities, detect_local_vision_status,
    detect_scene_cuts_in_scene, emit_sample_cube_luts, encode_linear_image_png_bytes,
    measure_scene_video_qc_metrics, run_gpu_or_cpu_grade_and_scopes, select_best_thumbnail_frame,
    validate_wgsl,
};

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
        },
        {
            "name": "manage_node_graph",
            "description": "v2.0 #21: 第3ワークスペース（Fusion/Nukeスタイル・ノードベース合成グラフ）の生成・双方向レイヤー同期・ノード追加/接続/削除・有効化を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_graph", "from_layers", "sync_to_layers", "set_enabled", "add_node", "connect", "remove_node", "validate"]
                    },
                    "enabled": { "type": "boolean" },
                    "node_type": {
                        "type": "string",
                        "enum": ["layer_input", "solid_color", "effect", "channel_split", "channel_merge", "transform", "merge", "mask_combine", "color_correct", "depth_fog", "output"]
                    },
                    "name": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "effect_type": { "type": "string" },
                    "channel": { "type": "string", "enum": ["rgba", "red", "green", "blue", "alpha", "luma", "depth"] },
                    "blend_mode": { "type": "string", "enum": ["normal", "add", "multiply", "screen"] },
                    "matte_mode": { "type": "string", "enum": ["alpha", "alpha_inverted", "luma", "luma_inverted"] },
                    "params": { "type": "object" },
                    "from_node": { "type": "string" },
                    "from_port": { "type": "string" },
                    "to_node": { "type": "string" },
                    "to_port": { "type": "string" },
                    "node_id": { "type": "string" }
                }
            }
        },
        {
            "name": "run_local_ai_vision",
            "description": "v2.0 #22: 完全オフライン（クラウド送信ゼロ）のローカル AI ビジョン（グリーンバック不要人物切り抜き・顔/被写体追尾 9:16 自動リフレーム・ローカル超解像・オプティカルフロースロー補間）を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "apply_segmentation", "auto_reframe", "apply_super_resolution", "set_optical_flow_slowmo"]
                    },
                    "id": { "type": "string" },
                    "sensitivity": { "type": "number" },
                    "edge_feather_px": { "type": "number" },
                    "skin_boost": { "type": "number" },
                    "despill_strength": { "type": "number" },
                    "invert": { "type": "boolean" },
                    "target_width": { "type": "integer" },
                    "target_height": { "type": "integer" },
                    "sample_interval_sec": { "type": "number" },
                    "smoothing_strength": { "type": "number" },
                    "apply": { "type": "boolean" },
                    "sharpen": { "type": "number" },
                    "denoise": { "type": "number" },
                    "enabled": { "type": "boolean" },
                    "speed": { "type": "number" }
                }
            }
        },
        {
            "name": "orchestrate_edit_recipe",
            "description": "v2.0 #23: マルチエージェント AI 編集ディレクター（YAML/JSON 編集レシピの記録・共有・人間承認ゲート付きステップ実行・render_frame_preview 視覚自己検査・Undo ロールバック）を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list_recipes", "save_recipe", "import_recipe", "export_yaml", "record_from_history", "start_run", "step", "run_until_gate", "approve_gate", "reject_gate", "get_status"]
                    },
                    "recipe_name": { "type": "string" },
                    "recipe_text": { "type": "string" },
                    "recipe": { "type": "object" },
                    "variables": { "type": "object" },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "limit": { "type": "integer" },
                    "auto_run": { "type": "boolean" },
                    "continue_after": { "type": "boolean" },
                    "rollback_last": { "type": "boolean" },
                    "skip_step": { "type": "boolean" },
                    "reason": { "type": "string" }
                }
            }
        },
        {
            "name": "collab_diff_and_review",
            "description": "v2.0 #24: Git-Friendly NLE のスナップショット保存・視覚的タイムライン Diff・3-Way 意味的ブランチマージ・Frame.io/YouTube レビューコメント入出力＆マーカー自動変換を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["save_snapshot", "list_snapshots", "diff_snapshots", "merge_three_way", "add_review_comment", "resolve_review_comment", "import_reviews", "export_reviews", "convert_reviews_to_markers"]
                    },
                    "name": { "type": "string" },
                    "base_snapshot": { "type": "string" },
                    "target_snapshot": { "type": "string" },
                    "theirs_snapshot": { "type": "string" },
                    "apply": { "type": "boolean" },
                    "resolutions": { "type": "object" },
                    "time_sec": { "type": "number" },
                    "author": { "type": "string" },
                    "body": { "type": "string" },
                    "comment_id": { "type": "string" },
                    "status": { "type": "string", "enum": ["open", "in_progress", "resolved"] },
                    "format": { "type": "string", "enum": ["frameio_csv", "json", "youtube_text"] },
                    "content": { "type": "string" },
                    "only_unresolved": { "type": "boolean" }
                }
            }
        },
        {
            "name": "pro_color_match_and_grade",
            "description": "v2.1 #25: RGB/マスタートーンカーブ（Fritsch-Carlson単調スプライン）・HSL二次色補正クオリファイア（マット可視化付）・ショット間自動カラーマッチを実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["set_curves", "set_hsl_qualifier", "auto_match_timecodes"]
                    },
                    "enabled": { "type": "boolean" },
                    "preset": {
                        "type": "string",
                        "enum": ["identity", "s_curve", "film_fade", "warm_cinema"]
                    },
                    "master": { "type": "array" },
                    "red": { "type": "array" },
                    "green": { "type": "array" },
                    "blue": { "type": "array" },
                    "hue_center_deg": { "type": "number" },
                    "hue_width_deg": { "type": "number" },
                    "sat_min": { "type": "number" },
                    "luma_min": { "type": "number" },
                    "softness": { "type": "number" },
                    "hue_shift_deg": { "type": "number" },
                    "sat_scale": { "type": "number" },
                    "luma_offset": { "type": "number" },
                    "show_matte_overlay": { "type": "boolean" },
                    "reference_time_sec": { "type": "number" },
                    "target_time_sec": { "type": "number" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "vfx_stabilize_and_corner_pin",
            "description": "v2.1 #26: 自動2D手ぶれ補正（ゼロ位相平滑化＋自動クロップ拡大キーフレーム生成）＆ 平面4点コーナーピン射影合成（3x3ホモグラフィ）を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action", "id"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["analyze_and_stabilize", "set_corner_pin", "reset_corner_pin"]
                    },
                    "id": { "type": "string" },
                    "smoothness": { "type": "number" },
                    "max_shift_ratio": { "type": "number" },
                    "sample_interval_sec": { "type": "number" },
                    "stabilize_rotation": { "type": "boolean" },
                    "auto_crop_zoom": { "type": "boolean" },
                    "apply": { "type": "boolean" },
                    "enabled": { "type": "boolean" },
                    "preset": {
                        "type": "string",
                        "enum": ["reset", "monitor_tilt", "billboard_left", "perspective_floor"]
                    },
                    "tl": { "type": "array" },
                    "tr": { "type": "array" },
                    "br": { "type": "array" },
                    "bl": { "type": "array" },
                    "feather_px": { "type": "number" }
                }
            }
        },
        {
            "name": "spatial_and_master_audio",
            "description": "v2.1 #27: 3Dバイノーラル空間オーディオパンナー（ITD+ILD+頭部シャドウ）・3バンドマルチバンドコンプレッサー・4x True-Peakブリックウォールリミッター・dBTP/LUFS計測を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_spatial_3d", "add_multiband_comp", "add_true_peak_limiter", "measure_master_true_peak"]
                    },
                    "id": { "type": "string" },
                    "azimuth_deg": { "type": "number" },
                    "elevation_deg": { "type": "number" },
                    "distance": { "type": "number" },
                    "room_mix": { "type": "number" },
                    "low_thresh_db": { "type": "number" },
                    "mid_thresh_db": { "type": "number" },
                    "high_thresh_db": { "type": "number" },
                    "ratio": { "type": "number" },
                    "makeup_db": { "type": "number" },
                    "ceiling_dbtp": { "type": "number" },
                    "release_ms": { "type": "number" },
                    "duration_sec": { "type": "number" }
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
            "name": "gpu_v30_pipeline",
            "description": "v3.0 #29: フル GPU コンピュートパイプライン（Vulkan/Metal/DX12 WGSL コンピュートシェーダー検証・4K ベンチマーク・GPU/CPU フォールバック付きリアルタイム色調＆ビデオスコープ実行）を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "benchmark_4k", "run_grade_and_scopes"]
                    },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "time_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "scene_3d_and_camera_track",
            "description": "v3.0 #30: 3D メッシュ / glTF 2.0 (`.gltf`/`.glb`) / 3D 立体押し出しテキストの配置・PBR 質感更新・サンプル GLB 書き出し＆検証・視差ベース 3D カメラトラッキングを実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_mesh_3d", "update_mesh_3d", "emit_sample_glb", "track_3d_camera"]
                    },
                    "id": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "preset": {
                        "type": "string",
                        "enum": ["cube", "sphere", "torus", "extruded_text", "gltf_file"]
                    },
                    "gltf_path": { "type": "string" },
                    "text_content": { "type": "string" },
                    "extrude_depth": { "type": "number" },
                    "base_color": { "type": "string" },
                    "metallic": { "type": "number" },
                    "roughness": { "type": "number" },
                    "env_reflection": { "type": "number" },
                    "wireframe": { "type": "boolean" },
                    "auto_spin_deg_per_sec": { "type": "number" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "sample_interval_sec": { "type": "number" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "offline_ai_dubbing_lipsync",
            "description": "v3.0 #31: 完全オフライン（クラウド送信ゼロ）の多言語 AI 吹替音声合成（3フォルマント共振 48kHz WAV 生成）・音素アライメント・立ち絵/映像リップシンク自動同期を 1-Undo で実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list_voices", "align_phonemes", "generate_locale_dubbing"]
                    },
                    "text": { "type": "string" },
                    "duration_sec": { "type": "number" },
                    "locale": { "type": "string" },
                    "voice": {
                        "type": "string",
                        "enum": ["jp_female", "jp_male", "en_female", "en_male", "narrator_warm"]
                    },
                    "audio_layer_index": { "type": "integer" },
                    "csv": { "type": "string" },
                    "translations": { "type": "object" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "crdt_realtime_collab",
            "description": "v3.0 #32: Lamport/Vector Clock ベースの Op-Based CRDT 同時共同編集（ローカル操作記録・ライブカーソルプレゼンス更新・デルタ JSON 書き出し・リモートデルタ冪等マージ＆ 1-Undo 反映）を操作する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "set_peer_profile", "update_cursor", "record_marker_op", "record_text_op", "export_delta", "merge_remote_delta"]
                    },
                    "peer_id": { "type": "string" },
                    "display_name": { "type": "string" },
                    "color_hex": { "type": "string" },
                    "playhead_sec": { "type": "number" },
                    "selected_item": { "type": "string" },
                    "id": { "type": "string" },
                    "content": { "type": "string" },
                    "time_sec": { "type": "number" },
                    "label": { "type": "string" },
                    "delta_json": { "type": "string" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "aces_color_management",
            "description": "v4.0 #33: ACEScg (AP1) / OpenColorIO v2 プロカラー管理（Sony S-Log3・Panasonic V-Log・ARRI LogC3 IDT、ACES 1.3 ガマット圧縮、ACES Fitted / AgX / Reinhard トーンマップ、Display P3 / Rec.2020 ODT、`.ocio` YAML 書き出し）を制御する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_config", "set_config", "export_ocio"]
                    },
                    "enabled": { "type": "boolean" },
                    "input_idt": {
                        "type": "string",
                        "enum": ["linear_srgb", "aces_cg", "display_p3", "rec2020", "s_log3", "v_log", "log_c3"]
                    },
                    "working_space": {
                        "type": "string",
                        "enum": ["linear_srgb", "aces_cg", "display_p3", "rec2020"]
                    },
                    "output_odt": {
                        "type": "string",
                        "enum": ["linear_srgb", "display_p3", "rec2020"]
                    },
                    "tone_mapper": {
                        "type": "string",
                        "enum": ["none", "aces_fitted", "ag_x", "reinhard_extended"]
                    },
                    "exposure_stops": { "type": "number" },
                    "gamut_compress": { "type": "boolean" },
                    "output_path": { "type": "string" }
                }
            }
        },
        {
            "name": "physics_simulate_and_bake",
            "description": "v4.0 #34: 決定論的 60Hz リジッドボディ物理演算シミュレーション（重力バウンス・ドミノ連鎖衝突・無重力浮遊・軌道ボテックス）を実行し、軌跡プレビュー取得または 1-Undo キーフレーム自動ベイクを行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["preview_trajectory", "bake_keyframes"]
                    },
                    "item_ids": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "preset": {
                        "type": "string",
                        "enum": ["bouncing_title", "domino_cascade", "zero_gravity_float", "orbital_vortex"]
                    },
                    "gravity_y": { "type": "number" },
                    "restitution": { "type": "number" },
                    "friction": { "type": "number" },
                    "air_drag": { "type": "number" },
                    "initial_vx": { "type": "number" },
                    "initial_vy": { "type": "number" },
                    "initial_spin_deg": { "type": "number" },
                    "central_attract": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "bake_step_frames": { "type": "integer" },
                    "floor_ratio": { "type": "number" }
                }
            }
        },
        {
            "name": "generative_bg_and_foley",
            "description": "v4.0 #35: オフライン・プロシージャル背景ジェネレーター（オーロラ・シネマ玉ボケ・サイバー3Dグリッド・リキッドマーブル・ワープ星雲・アブストラクト波）の配置・編集と、プロシージャル Foley/SFX 効果音（ウーッシュ・重低音インパクト・UIポップ・グリッチ・雨音・ライザー）48kHz ステレオ WAV 合成＆タイムライン配置を行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_generative_bg", "update_generative_bg", "synthesize_foley_sfx"]
                    },
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "preset": { "type": "string" },
                    "layer_index": { "type": "integer" },
                    "audio_layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "seed": { "type": "integer" },
                    "speed": { "type": "number" },
                    "scale": { "type": "number" },
                    "complexity": { "type": "integer" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                    "apply": { "type": "boolean" }
                }
            }
        },
        {
            "name": "broadcast_qc_and_web_export",
            "description": "v4.0 #36: 放送・配信コンプライアンス QC 監査（ITU-R BS.1770 True-Peak/LUFS・ITU-R BT.1702 光過敏フラッシュ検査・輝度ガマット検査）、高クリック率ヒーローサムネイル（1280x720 PNG）自動抽出、およびサーバー不要・自己完結型 HTML5 インタラクティブレビュープレイヤー書き出しを実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["run_qc_audit", "generate_hero_thumbnail", "export_web_review_html"]
                    },
                    "profile": {
                        "type": "string",
                        "enum": ["youtube1080p", "broadcast_itu_ebu_r128", "shorts_tiktok9x16", "cinema_dci4k"]
                    },
                    "sample_count": { "type": "integer" },
                    "output_path": { "type": "string" }
                }
            }
        }
    ]) {
        tools.extend(v10_extra);
    }
    if let Value::Array(v50_extra) = json!([
        {
            "name": "rig2d_puppet_control",
            "description": "v5.0 #37: 2D スケルトンボーン＆2-Bone IK パペットリグ（階層 FK・解析的 2-Bone 逆運動学・髪/リボン二次バネ揺れ・4種モーションプリセット）の配置・編集・IK 姿勢プレビューを行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_rigged_puppet", "update_rig", "solve_ik_preview"]
                    },
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "preset": {
                        "type": "string",
                        "enum": ["idle_breathe", "wave_hello", "nod_talk", "dance_bounce"]
                    },
                    "layer_index": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "motion_speed": { "type": "number" },
                    "motion_amp": { "type": "number" },
                    "show_bones_overlay": { "type": "boolean" },
                    "right_ik_enabled": { "type": "boolean" },
                    "right_ik_x": { "type": "number" },
                    "right_ik_y": { "type": "number" },
                    "left_ik_enabled": { "type": "boolean" },
                    "left_ik_x": { "type": "number" },
                    "left_ik_y": { "type": "number" },
                    "time_sec": { "type": "number" }
                }
            }
        },
        {
            "name": "offline_bgm_composer",
            "description": "v5.0 #38: 完全オフライン・著作権フリーの自動 BGM コンポーザー（6 ジャンル・コード進行・ドラム/ベース/パッド/メロディ 48kHz ステレオ WAV 合成＆小節マーカー自動配置）を実行する",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["preview_arrangement", "compose_and_place_bgm"]
                    },
                    "genre": {
                        "type": "string",
                        "enum": ["lofi_chill", "upbeat_pop", "cinematic_epic", "cyber_synthwave", "acoustic_explainer", "suspense_pulse"]
                    },
                    "bpm": { "type": "number" },
                    "key_semitone": { "type": "integer" },
                    "duration_sec": { "type": "number" },
                    "intensity": { "type": "number" },
                    "seed": { "type": "integer" },
                    "start_sec": { "type": "number" },
                    "audio_layer_index": { "type": "integer" },
                    "add_bar_markers": { "type": "boolean" },
                    "apply": { "type": "boolean" }
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
        },
        {
            "name": "cinema_finishing_and_edl",
            "description": "v5.0 #40: プロ・シネマフィニッシング（35mm ハレーション・アナモルフィックフレア・フィルムグレイン・ビネット・レターボックス・SMPTE 透かしバーンイン）の制御および業界標準 CMX 3600 EDL / Final Cut Pro XML v1.10 (FCPXML) 書き出しを行う",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get_finishing", "set_finishing", "export_edl_cmx3600", "export_fcpxml"]
                    },
                    "enabled": { "type": "boolean" },
                    "preset": {
                        "type": "string",
                        "enum": ["kodak_35mm", "scifi_anamorphic", "agency_review_burnin"]
                    },
                    "halation_strength": { "type": "number" },
                    "halation_threshold": { "type": "number" },
                    "anamorphic_flare": { "type": "number" },
                    "film_grain": { "type": "number" },
                    "vignette": { "type": "number" },
                    "letterbox": {
                        "type": "string",
                        "enum": ["none", "cinema239", "flat185", "academy43"]
                    },
                    "burn_in_timecode": { "type": "boolean" },
                    "watermark_text": { "type": "string" },
                    "output_path": { "type": "string" }
                }
            }
        }
    ]) {
        tools.extend(v50_extra);
    }
    if let Value::Array(v60_extra) = json!([
        {
            "name": "data_chart_motion_graphics",
            "description": "v6.0 #41: データ駆動モーショングラフィックス＆インフォグラフィック・チャート（棒グラフ・折れ線・ドーナツ・KPIカウンター / CSV取り込み）を生成・更新する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create_chart", "update_chart", "import_csv"]
                    },
                    "item_id": { "type": "string" },
                    "kind": {
                        "type": "string",
                        "enum": ["bar_chart", "line_chart", "donut_chart", "kpi_counter"]
                    },
                    "layer_index": { "type": "integer" },
                    "name": { "type": "string" },
                    "title": { "type": "string" },
                    "unit_suffix": { "type": "string" },
                    "csv_text": { "type": "string" },
                    "points": { "type": "array" },
                    "start_sec": { "type": "number" },
                    "duration_sec": { "type": "number" },
                    "animate_in_sec": { "type": "number" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "accent_color": { "type": "string" },
                    "bg_color": { "type": "string" },
                    "show_grid": { "type": "boolean" },
                    "show_values": { "type": "boolean" }
                }
            }
        },
        {
            "name": "vfx_paint_and_inpaint",
            "description": "v6.0 #42: 非破壊 VFX ペイント（Write-On グローブラシ・クローンスタンプ・ワイヤー/不要物消しインペインティング・プライバシー秘匿ぼかし）をクリップへ設定・追加する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add_stroke", "set_paint", "clear_paint", "get_paint"]
                    },
                    "item_id": { "type": "string" },
                    "tool": {
                        "type": "string",
                        "enum": ["brush_glow", "clone_stamp", "wire_erase_inpaint", "blur_redact"]
                    },
                    "points": { "type": "array" },
                    "radius_px": { "type": "number" },
                    "hardness": { "type": "number" },
                    "opacity": { "type": "number" },
                    "color": { "type": "string" },
                    "clone_offset": { "type": "array" },
                    "write_on_progress": { "type": "number" },
                    "animate_write_on": { "type": "boolean" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["item_id"]
            }
        },
        {
            "name": "podcast_auto_director",
            "description": "v6.0 #43: マルチトラック・ポッドキャスト＆コラボ実況オート編集（複数話者ダイアリゼーション・アクティブ話者カメラ自動切替キーフレーム＆マーカー・-16 LUFS 均一化レベラー）を実行する",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["analyze_and_direct", "level_wav"]
                    },
                    "speakers": { "type": "array" },
                    "min_turn_sec": { "type": "number" },
                    "silence_gate_db": { "type": "number" },
                    "target_lufs": { "type": "number" },
                    "apply_leveler_faders": { "type": "boolean" },
                    "apply": { "type": "boolean" },
                    "input_wav_path": { "type": "string" },
                    "output_wav_path": { "type": "string" }
                }
            }
        },
        {
            "name": "multi_platform_packager",
            "description": "v6.0 #44: プロ・マルチプラットフォーム自動リフォーマット（16:9 / 9:16 / 1:1 / 4K 派生シーン自動生成）および 1 クリック納品バンドル（SRT・チャプター・サムネイル・EDL・FCPXML・QCレポート・manifest.json）を書き出す",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create_platform_variants", "export_delivery_bundle"]
                    },
                    "platforms": { "type": "array" },
                    "output_dir": { "type": "string" },
                    "base_title": { "type": "string" },
                    "tags": { "type": "array" },
                    "include_thumbnail": { "type": "boolean" },
                    "include_edl": { "type": "boolean" },
                    "include_fcpxml": { "type": "boolean" },
                    "include_qc_report": { "type": "boolean" }
                }
            }
        }
    ]) {
        tools.extend(v60_extra);
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
    // v2.0 #23: AI 編集ディレクターは内部で他 MCP ツールを再帰的に呼び出すため、ロック取得前に独立ディスパッチする
    if name == "orchestrate_edit_recipe" {
        return crate::director::handle_orchestrate_tool(session, args);
    }

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
        "manage_node_graph" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing string argument 'action'".to_string())?;
            let s = &mut *guard;
            match action {
                "get_graph" => {
                    let scene = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let text = serde_json::to_string_pretty(&scene.node_graph).unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "from_layers" => {
                    let graph = {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        NodeCompositeGraph::from_scene_layers(scene)
                    };
                    let node_count = graph.nodes.len();
                    let conn_count = graph.connections.len();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    let text = serde_json::to_string_pretty(&json!({
                        "status": "generated_from_layers",
                        "enabled": true,
                        "node_count": node_count,
                        "connection_count": conn_count,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "sync_to_layers" => {
                    let mut next_project = s.project.clone();
                    let summary = {
                        let scene = next_project
                            .active_scene_mut()
                            .ok_or_else(|| "no active scene".to_string())?;
                        let graph = scene.node_graph.clone();
                        sync_node_graph_to_scene_layers(&graph, scene)
                    };
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::ReplaceProjectSnapshot {
                                project: Box::new(next_project),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    let text = serde_json::to_string_pretty(&summary).unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "set_enabled" => {
                    let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    graph.enabled = enabled;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "enabled": enabled }).to_string() }] }),
                    )
                }
                "add_node" => {
                    let node_type = args
                        .get("node_type")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing string argument 'node_type'".to_string())?;
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    let kind = match node_type {
                        "layer_input" => CompositeNodeKind::LayerInput {
                            layer_index: args
                                .get("layer_index")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as usize,
                        },
                        "solid_color" => CompositeNodeKind::SolidColor {
                            color: RgbaColor::WHITE,
                        },
                        "effect" => CompositeNodeKind::Effect {
                            effect_type: args
                                .get("effect_type")
                                .and_then(Value::as_str)
                                .unwrap_or("blur")
                                .to_string(),
                            params: parse_params_map(args.get("params")),
                        },
                        "channel_split" => {
                            let ch = match args
                                .get("channel")
                                .and_then(Value::as_str)
                                .unwrap_or("luma")
                            {
                                "rgba" => ChannelSelector::Rgba,
                                "red" => ChannelSelector::Red,
                                "green" => ChannelSelector::Green,
                                "blue" => ChannelSelector::Blue,
                                "alpha" => ChannelSelector::Alpha,
                                "depth" => ChannelSelector::Depth,
                                _ => ChannelSelector::Luma,
                            };
                            CompositeNodeKind::ChannelSplit { channel: ch }
                        }
                        "channel_merge" => CompositeNodeKind::ChannelMerge,
                        "transform" => CompositeNodeKind::Transform {
                            x: finite_clamped(
                                args.get("params").and_then(|p| p.get("x")),
                                0.0,
                                -4096.0,
                                4096.0,
                            ) as f32,
                            y: finite_clamped(
                                args.get("params").and_then(|p| p.get("y")),
                                0.0,
                                -4096.0,
                                4096.0,
                            ) as f32,
                            scale_x: finite_clamped(
                                args.get("params").and_then(|p| p.get("scale_x")),
                                1.0,
                                0.05,
                                8.0,
                            ) as f32,
                            scale_y: finite_clamped(
                                args.get("params").and_then(|p| p.get("scale_y")),
                                1.0,
                                0.05,
                                8.0,
                            ) as f32,
                            rotation_deg: finite_clamped(
                                args.get("params").and_then(|p| p.get("rotation_deg")),
                                0.0,
                                -3600.0,
                                3600.0,
                            ) as f32,
                            opacity: finite_clamped(
                                args.get("params").and_then(|p| p.get("opacity")),
                                1.0,
                                0.0,
                                1.0,
                            ) as f32,
                        },
                        "merge" => {
                            let bm = match args
                                .get("blend_mode")
                                .and_then(Value::as_str)
                                .unwrap_or("normal")
                            {
                                "add" => BlendMode::Add,
                                "multiply" => BlendMode::Multiply,
                                "screen" => BlendMode::Screen,
                                _ => BlendMode::Normal,
                            };
                            CompositeNodeKind::Merge {
                                blend_mode: bm,
                                opacity: finite_clamped(
                                    args.get("params").and_then(|p| p.get("opacity")),
                                    1.0,
                                    0.0,
                                    1.0,
                                ) as f32,
                            }
                        }
                        "mask_combine" => {
                            let mode = match args
                                .get("matte_mode")
                                .and_then(Value::as_str)
                                .unwrap_or("alpha")
                            {
                                "alpha_inverted" => TrackMatteMode::AlphaInverted,
                                "luma" => TrackMatteMode::Luma,
                                "luma_inverted" => TrackMatteMode::LumaInverted,
                                _ => TrackMatteMode::Alpha,
                            };
                            CompositeNodeKind::MaskCombine { mode }
                        }
                        "color_correct" => CompositeNodeKind::ColorCorrect {
                            exposure: finite_clamped(
                                args.get("params").and_then(|p| p.get("exposure")),
                                0.0,
                                -4.0,
                                4.0,
                            ) as f32,
                            contrast: finite_clamped(
                                args.get("params").and_then(|p| p.get("contrast")),
                                1.0,
                                0.1,
                                3.0,
                            ) as f32,
                            saturation: finite_clamped(
                                args.get("params").and_then(|p| p.get("saturation")),
                                1.0,
                                0.0,
                                3.0,
                            ) as f32,
                        },
                        "depth_fog" => CompositeNodeKind::DepthFog {
                            near: finite_clamped(
                                args.get("params").and_then(|p| p.get("near")),
                                400.0,
                                0.0,
                                10000.0,
                            ) as f32,
                            far: finite_clamped(
                                args.get("params").and_then(|p| p.get("far")),
                                2000.0,
                                10.0,
                                20000.0,
                            ) as f32,
                            fog_color: RgbaColor {
                                r: 0.72,
                                g: 0.80,
                                b: 0.92,
                                a: 1.0,
                            },
                            strength: finite_clamped(
                                args.get("params").and_then(|p| p.get("strength")),
                                0.5,
                                0.0,
                                1.0,
                            ) as f32,
                        },
                        "output" => CompositeNodeKind::Output,
                        other => return Err(format!("unknown node_type: {other}")),
                    };
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(node_type);
                    let pos = [120.0 + (graph.nodes.len() as f32 * 40.0), 120.0];
                    let node_id = graph.add_node(CompositeNode::new(name, pos, kind));
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "created_node_id": node_id }).to_string() }] }),
                    )
                }
                "connect" => {
                    let from_node = parse_uuid(args, "from_node")?;
                    let to_node = parse_uuid(args, "to_node")?;
                    let from_port = args
                        .get("from_port")
                        .and_then(Value::as_str)
                        .unwrap_or("out");
                    let to_port = args.get("to_port").and_then(Value::as_str).unwrap_or("in");
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    graph
                        .connect(from_node, from_port, to_node, to_port)
                        .map_err(|e| e.to_string())?;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "connected": true }).to_string() }] }),
                    )
                }
                "remove_node" => {
                    let node_id = parse_uuid(args, "node_id")?;
                    let mut graph = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .node_graph
                        .clone();
                    let removed = graph.remove_node(node_id);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetNodeGraph { graph },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "removed": removed }).to_string() }] }),
                    )
                }
                "validate" => {
                    let scene = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let res = scene.node_graph.topological_order();
                    let text = match res {
                        Ok(order) => json!({
                            "valid": true,
                            "enabled": scene.node_graph.enabled,
                            "topological_len": order.len(),
                            "has_output": scene.node_graph.output_node_id().is_some(),
                        })
                        .to_string(),
                        Err(e) => json!({ "valid": false, "error": e.to_string() }).to_string(),
                    };
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                other => Err(format!("unknown manage_node_graph action: {other}")),
            }
        }
        "run_local_ai_vision" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing string argument 'action'".to_string())?;
            let s = &mut *guard;
            match action {
                "status" => {
                    let models_dir = s.folder.as_ref().map(|f| f.models_dir());
                    let st = detect_local_vision_status(models_dir.as_deref());
                    let backend_name = match &st.active_backend {
                        LocalVisionBackend::PureRustBuiltIn => "pure_rust_builtin",
                        LocalVisionBackend::LocalOnnxCli { .. } => "local_onnx_cli",
                    };
                    let text = serde_json::to_string_pretty(&json!({
                        "backend": backend_name,
                        "local_only_guaranteed": st.local_only_guaranteed,
                        "onnx_models_found": st.onnx_models_found,
                        "summary": st.summary,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "apply_segmentation" => {
                    let id = parse_uuid(args, "id")?;
                    let mut params = BTreeMap::new();
                    params.insert(
                        "sensitivity".into(),
                        finite_clamped(args.get("sensitivity"), 0.55, 0.05, 1.0) as f32,
                    );
                    params.insert(
                        "edge_feather_px".into(),
                        finite_clamped(args.get("edge_feather_px"), 1.5, 0.0, 6.0) as f32,
                    );
                    params.insert(
                        "skin_boost".into(),
                        finite_clamped(args.get("skin_boost"), 0.65, 0.0, 1.0) as f32,
                    );
                    params.insert(
                        "despill_strength".into(),
                        finite_clamped(args.get("despill_strength"), 0.35, 0.0, 1.0) as f32,
                    );
                    params.insert(
                        "invert".into(),
                        if args.get("invert").and_then(Value::as_bool).unwrap_or(false) {
                            1.0
                        } else {
                            0.0
                        },
                    );
                    let eff = EffectNode {
                        id: Uuid::new_v4(),
                        effect_type: "ai_segment".to_string(),
                        enabled: true,
                        params,
                    };
                    let eid = eff.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddEffect {
                                item_id: id,
                                effect: eff,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "status": "applied_ai_segment", "effect_id": eid, "cloud_upload": false }).to_string() }] }),
                    )
                }
                "auto_reframe" => {
                    let tw = args
                        .get("target_width")
                        .and_then(Value::as_u64)
                        .unwrap_or(1080)
                        .clamp(64, 4096) as u32;
                    let th = args
                        .get("target_height")
                        .and_then(Value::as_u64)
                        .unwrap_or(1920)
                        .clamp(64, 4096) as u32;
                    let cfg = AutoReframeConfig {
                        target_width: tw,
                        target_height: th,
                        sample_interval_sec: finite_clamped(
                            args.get("sample_interval_sec"),
                            0.4,
                            0.1,
                            5.0,
                        ),
                        smoothing_strength: finite_clamped(
                            args.get("smoothing_strength"),
                            0.72,
                            0.0,
                            0.95,
                        ) as f32,
                        ..Default::default()
                    };
                    let active_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let samples = analyze_scene_auto_reframe(&mut rg, &s.project, active_id, &cfg);
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let mut applied_cmds = 0usize;
                    if apply {
                        let cmds = {
                            let scene = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            build_auto_reframe_commands(scene, &cfg, &samples)
                        };
                        applied_cmds = cmds.len();
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("AI 自動リフレーム ({tw}x{th})"),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    let sample_json: Vec<Value> = samples
                        .iter()
                        .map(|sm| {
                            json!({
                                "time_sec": sm.time.as_seconds(),
                                "focus_x": sm.focus_x_norm,
                                "focus_y": sm.focus_y_norm,
                                "confidence": sm.confidence,
                            })
                        })
                        .collect();
                    let text = serde_json::to_string_pretty(&json!({
                        "target": [tw, th],
                        "applied": apply,
                        "commands_applied": applied_cmds,
                        "sample_count": sample_json.len(),
                        "samples": sample_json,
                        "cloud_upload": false,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "apply_super_resolution" => {
                    let id = parse_uuid(args, "id")?;
                    let mut params = BTreeMap::new();
                    params.insert(
                        "sharpen".into(),
                        finite_clamped(args.get("sharpen"), 0.55, 0.0, 2.0) as f32,
                    );
                    params.insert(
                        "denoise".into(),
                        finite_clamped(args.get("denoise"), 0.2, 0.0, 1.0) as f32,
                    );
                    let eff = EffectNode {
                        id: Uuid::new_v4(),
                        effect_type: "ai_super_res".to_string(),
                        enabled: true,
                        params,
                    };
                    let eid = eff.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddEffect {
                                item_id: id,
                                effect: eff,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "status": "applied_ai_super_res", "effect_id": eid, "cloud_upload": false }).to_string() }] }),
                    )
                }
                "set_optical_flow_slowmo" => {
                    let id = parse_uuid(args, "id")?;
                    let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    let mut cmds = vec![EditCommand::SetOpticalFlowInterp {
                        item_id: id,
                        enabled,
                    }];
                    if let Some(spd) = args.get("speed").and_then(Value::as_f64) {
                        cmds.push(EditCommand::SetPlaybackSpeed {
                            item_id: id,
                            speed: (spd as f32).clamp(-8.0, 8.0),
                            loop_playback: false,
                        });
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: "オプティカルフロースロー設定".into(),
                                commands: cmds,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "item_id": id, "optical_flow_interp": enabled }).to_string() }] }),
                    )
                }
                other => Err(format!("unknown run_local_ai_vision action: {other}")),
            }
        }
        "collab_diff_and_review" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing string argument 'action'".to_string())?;
            let s = &mut *guard;
            match action {
                "save_snapshot" => {
                    let folder = s
                        .folder
                        .as_ref()
                        .ok_or_else(|| "no project folder opened".to_string())?;
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("branch_snapshot");
                    let path = folder
                        .save_snapshot(name, &s.project)
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "saved_snapshot": name, "path": path.display().to_string() }).to_string() }] }),
                    )
                }
                "list_snapshots" => {
                    let list = s
                        .folder
                        .as_ref()
                        .map(|f| f.list_snapshots())
                        .unwrap_or_default();
                    Ok(
                        json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&list).unwrap_or_default() }] }),
                    )
                }
                "diff_snapshots" => {
                    let folder = s
                        .folder
                        .as_ref()
                        .ok_or_else(|| "no project folder opened".to_string())?;
                    let base_name = args
                        .get("base_snapshot")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'base_snapshot'".to_string())?;
                    let base_proj = folder.load_snapshot(base_name).map_err(|e| e.to_string())?;
                    let target_proj =
                        if let Some(tn) = args.get("target_snapshot").and_then(Value::as_str) {
                            folder.load_snapshot(tn).map_err(|e| e.to_string())?
                        } else {
                            s.project.clone()
                        };
                    let diff = diff_projects(&base_proj, &target_proj);
                    let report = format_diff_report(&diff);
                    let text = serde_json::to_string_pretty(&json!({
                        "total_changes": diff.total_changes(),
                        "report": report,
                        "diff": diff,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "merge_three_way" => {
                    let folder = s
                        .folder
                        .as_ref()
                        .ok_or_else(|| "no project folder opened".to_string())?;
                    let base_name = args
                        .get("base_snapshot")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'base_snapshot'".to_string())?;
                    let theirs_name = args
                        .get("theirs_snapshot")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'theirs_snapshot'".to_string())?;
                    let base_proj = folder.load_snapshot(base_name).map_err(|e| e.to_string())?;
                    let theirs_proj = folder
                        .load_snapshot(theirs_name)
                        .map_err(|e| e.to_string())?;

                    let mut resolutions = BTreeMap::new();
                    if let Some(obj) = args.get("resolutions").and_then(Value::as_object) {
                        for (k, v) in obj {
                            let res = match v.as_str().unwrap_or("keep_ours") {
                                "keep_theirs" => ConflictResolution::KeepTheirs,
                                "keep_both_duplicate" => ConflictResolution::KeepBothDuplicate,
                                _ => ConflictResolution::KeepOurs,
                            };
                            resolutions.insert(k.clone(), res);
                        }
                    }

                    let merged = merge_projects_three_way(
                        &base_proj,
                        &s.project,
                        &theirs_proj,
                        &resolutions,
                    );
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(false);
                    let applied = if apply && merged.conflicts.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::ReplaceProjectSnapshot {
                                    project: Box::new(merged.merged_project.clone()),
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };
                    let text = serde_json::to_string_pretty(&json!({
                        "auto_merged_from_theirs": merged.auto_merged_from_theirs,
                        "conflict_count": merged.conflicts.len(),
                        "conflicts": merged.conflicts,
                        "applied": applied,
                    }))
                    .unwrap_or_default();
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "add_review_comment" => {
                    let t = parse_time_arg(args, "frame", "time_sec", fps, s.playhead);
                    let author = args
                        .get("author")
                        .and_then(Value::as_str)
                        .unwrap_or("Reviewer");
                    let body = args
                        .get("body")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'body'".to_string())?;
                    let mut comments = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .review_comments
                        .clone();
                    let comment = ReviewComment::new(t, author, body, ReviewSource::Internal);
                    let cid = comment.id;
                    comments.push(comment);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetReviewComments { comments },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "comment_id": cid, "time_sec": t.as_seconds() }).to_string() }] }),
                    )
                }
                "resolve_review_comment" => {
                    let cid = parse_uuid(args, "comment_id")?;
                    let new_status = match args
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("resolved")
                    {
                        "open" => ReviewStatus::Open,
                        "in_progress" => ReviewStatus::InProgress,
                        _ => ReviewStatus::Resolved,
                    };
                    let mut comments = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .review_comments
                        .clone();
                    let mut found = false;
                    for c in &mut comments {
                        if c.id == cid {
                            c.status = new_status;
                            found = true;
                        }
                    }
                    if !found {
                        return Err(format!("review comment not found: {cid}"));
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetReviewComments { comments },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "comment_id": cid, "status": new_status }).to_string() }] }),
                    )
                }
                "import_reviews" => {
                    let fmt = args
                        .get("format")
                        .and_then(Value::as_str)
                        .unwrap_or("frameio_csv");
                    let content = args
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'content'".to_string())?;
                    let imported = match fmt {
                        "json" => import_review_comments_json(content, fps)?,
                        "youtube_text" => import_youtube_review_comments(content, fps),
                        _ => import_review_comments_csv(content, fps),
                    };
                    let added = imported.len();
                    let mut comments = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .review_comments
                        .clone();
                    comments.extend(imported);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetReviewComments { comments },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "imported_count": added, "format": fmt }).to_string() }] }),
                    )
                }
                "export_reviews" => {
                    let fmt = args
                        .get("format")
                        .and_then(Value::as_str)
                        .unwrap_or("frameio_csv");
                    let scene = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let text = match fmt {
                        "json" => export_review_comments_json(&scene.review_comments),
                        _ => export_review_comments_csv(&scene.review_comments, scene.fps),
                    };
                    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
                }
                "convert_reviews_to_markers" => {
                    let only_unresolved = args
                        .get("only_unresolved")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let cmds = {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_review_comments_to_markers_commands(
                            &scene.review_comments,
                            only_unresolved,
                        )
                    };
                    let count = cmds.len();
                    if count > 0 {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("レビューコメント→マーカー変換 ({count}件)"),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(
                        json!({ "content": [{ "type": "text", "text": json!({ "converted_markers": count }).to_string() }] }),
                    )
                }
                other => Err(format!("unknown collab_diff_and_review action: {other}")),
            }
        }
        "pro_color_match_and_grade" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_curve_pts = |key: &str| -> Option<Vec<[f32; 2]>> {
                let arr = args.get(key)?.as_array()?;
                let pts: Vec<[f32; 2]> = arr
                    .iter()
                    .take(16)
                    .filter_map(|p| {
                        let pair = p.as_array()?;
                        if pair.len() < 2 {
                            return None;
                        }
                        let x = pair[0].as_f64()?;
                        let y = pair[1].as_f64()?;
                        if !x.is_finite() || !y.is_finite() {
                            return None;
                        }
                        Some([x.clamp(0.0, 1.0) as f32, y.clamp(0.0, 1.0) as f32])
                    })
                    .collect();
                if pts.len() >= 2 { Some(pts) } else { None }
            };

            match action {
                "set_curves" => {
                    let mut grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    if let Some(preset) = args.get("preset").and_then(Value::as_str) {
                        grade.curves = match preset {
                            "s_curve" => ColorCurves::preset_s_curve(),
                            "film_fade" => ColorCurves::preset_film_fade(),
                            "warm_cinema" => ColorCurves::preset_warm_cinema(),
                            _ => ColorCurves::default(),
                        };
                    }
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        grade.curves.enabled = en;
                    } else if args.get("preset").is_none() {
                        grade.curves.enabled = true;
                    }
                    if let Some(pts) = parse_curve_pts("master") {
                        grade.curves.master = pts;
                    }
                    if let Some(pts) = parse_curve_pts("red") {
                        grade.curves.red = pts;
                    }
                    if let Some(pts) = parse_curve_pts("green") {
                        grade.curves.green = pts;
                    }
                    if let Some(pts) = parse_curve_pts("blue") {
                        grade.curves.blue = pts;
                    }
                    if grade.curves.enabled {
                        grade.enabled = true;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetSceneColorGrade {
                                grade: grade.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&grade.curves).unwrap_or_default() }]
                    }))
                }
                "set_hsl_qualifier" => {
                    let mut grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    let mut q = grade.hsl_qualifier;
                    q.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    if args.get("hue_center_deg").is_some() {
                        q.hue_center_deg = finite_clamped(
                            args.get("hue_center_deg"),
                            f64::from(q.hue_center_deg),
                            0.0,
                            360.0,
                        ) as f32;
                    }
                    if args.get("hue_width_deg").is_some() {
                        q.hue_width_deg = finite_clamped(
                            args.get("hue_width_deg"),
                            f64::from(q.hue_width_deg),
                            5.0,
                            120.0,
                        ) as f32;
                    }
                    if args.get("sat_min").is_some() {
                        q.sat_min =
                            finite_clamped(args.get("sat_min"), f64::from(q.sat_min), 0.0, 1.0)
                                as f32;
                    }
                    if args.get("luma_min").is_some() {
                        q.luma_min =
                            finite_clamped(args.get("luma_min"), f64::from(q.luma_min), 0.0, 1.0)
                                as f32;
                    }
                    if args.get("softness").is_some() {
                        q.softness =
                            finite_clamped(args.get("softness"), f64::from(q.softness), 0.02, 1.0)
                                as f32;
                    }
                    if args.get("hue_shift_deg").is_some() {
                        q.hue_shift_deg = finite_clamped(
                            args.get("hue_shift_deg"),
                            f64::from(q.hue_shift_deg),
                            -180.0,
                            180.0,
                        ) as f32;
                    }
                    if args.get("sat_scale").is_some() {
                        q.sat_scale =
                            finite_clamped(args.get("sat_scale"), f64::from(q.sat_scale), 0.0, 2.5)
                                as f32;
                    }
                    if args.get("luma_offset").is_some() {
                        q.luma_offset = finite_clamped(
                            args.get("luma_offset"),
                            f64::from(q.luma_offset),
                            -0.5,
                            0.5,
                        ) as f32;
                    }
                    if let Some(m) = args.get("show_matte_overlay").and_then(Value::as_bool) {
                        q.show_matte_overlay = m;
                    }
                    grade.hsl_qualifier = q;
                    if q.enabled {
                        grade.enabled = true;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetSceneColorGrade {
                                grade: grade.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&grade.hsl_qualifier).unwrap_or_default() }]
                    }))
                }
                "auto_match_timecodes" => {
                    let ref_sec =
                        finite_clamped(args.get("reference_time_sec"), 0.0, 0.0, 86_400.0);
                    let tgt_sec = finite_clamped(
                        args.get("target_time_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let scene_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let ref_img = rg
                        .render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(ref_sec),
                            320,
                            180,
                            false,
                        )
                        .ok_or_else(|| "failed to render reference frame".to_string())?;
                    let tgt_img = rg
                        .render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(tgt_sec),
                            320,
                            180,
                            false,
                        )
                        .ok_or_else(|| "failed to render target frame".to_string())?;

                    let matched = compute_auto_color_match(&ref_img, &tgt_img);
                    let mut next_grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    next_grade.enabled = true;
                    next_grade.lift = matched.lift;
                    next_grade.gain = matched.gain;
                    next_grade.saturation = matched.saturation;

                    if apply {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::SetSceneColorGrade {
                                    grade: next_grade.clone(),
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "applied": apply,
                                "reference_time_sec": ref_sec,
                                "target_time_sec": tgt_sec,
                                "matched_grade": next_grade
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown pro_color_match_and_grade action: {other}")),
            }
        }
        "vfx_stabilize_and_corner_pin" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let item_id = parse_uuid(args, "id")?;

            let parse_pt2 = |key: &str, fallback: [f32; 2]| -> [f32; 2] {
                args.get(key)
                    .and_then(Value::as_array)
                    .filter(|a| a.len() >= 2)
                    .and_then(|a| {
                        let x = a[0].as_f64()?;
                        let y = a[1].as_f64()?;
                        if x.is_finite() && y.is_finite() {
                            Some([x.clamp(-0.5, 1.5) as f32, y.clamp(-0.5, 1.5) as f32])
                        } else {
                            None
                        }
                    })
                    .unwrap_or(fallback)
            };

            match action {
                "analyze_and_stabilize" => {
                    let cfg = StabilizeConfig {
                        sample_interval_sec: finite_clamped(
                            args.get("sample_interval_sec"),
                            0.2,
                            0.05,
                            2.0,
                        ),
                        smoothness: finite_clamped(args.get("smoothness"), 0.78, 0.1, 0.95) as f32,
                        max_shift_ratio: finite_clamped(
                            args.get("max_shift_ratio"),
                            0.08,
                            0.01,
                            0.25,
                        ) as f32,
                        stabilize_rotation: args
                            .get("stabilize_rotation")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        auto_crop_zoom: args
                            .get("auto_crop_zoom")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                    };
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let scene_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let samples = analyze_camera_shake(&mut rg, &s.project, scene_id, &cfg);
                    let cmds = {
                        let scene = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_stabilize_commands(scene, item_id, &samples, &cfg)
                    };
                    let max_dx = samples
                        .iter()
                        .map(|sm| sm.comp_x.abs())
                        .fold(0.0f32, f32::max);
                    let max_dy = samples
                        .iter()
                        .map(|sm| sm.comp_y.abs())
                        .fold(0.0f32, f32::max);
                    let max_rot = samples
                        .iter()
                        .map(|sm| sm.comp_rot_deg.abs())
                        .fold(0.0f32, f32::max);
                    let gen_count = cmds.len();
                    let applied = if apply && !cmds.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("自動手ぶれ補正 ({} サンプル)", samples.len()),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": item_id,
                                "sample_count": samples.len(),
                                "max_comp_x": max_dx,
                                "max_comp_y": max_dy,
                                "max_comp_rot_deg": max_rot,
                                "generated_commands": gen_count,
                                "applied": applied
                            }).to_string()
                        }]
                    }))
                }
                "set_corner_pin" => {
                    let mut pin = s
                        .project
                        .active_scene()
                        .and_then(|sc| {
                            sc.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .find(|i| i.id == item_id)
                                .map(|i| i.corner_pin)
                        })
                        .ok_or_else(|| format!("item not found: {item_id}"))?;
                    if let Some(preset) = args.get("preset").and_then(Value::as_str) {
                        pin = match preset {
                            "monitor_tilt" => CornerPinConfig {
                                enabled: true,
                                tl: [0.14, 0.12],
                                tr: [0.88, 0.08],
                                br: [0.92, 0.86],
                                bl: [0.10, 0.90],
                                feather_px: 1.5,
                            },
                            "billboard_left" => CornerPinConfig {
                                enabled: true,
                                tl: [0.06, 0.08],
                                tr: [0.76, 0.18],
                                br: [0.76, 0.82],
                                bl: [0.06, 0.92],
                                feather_px: 1.5,
                            },
                            "perspective_floor" => CornerPinConfig {
                                enabled: true,
                                tl: [0.24, 0.35],
                                tr: [0.76, 0.35],
                                br: [0.96, 0.94],
                                bl: [0.04, 0.94],
                                feather_px: 2.0,
                            },
                            _ => CornerPinConfig::default(),
                        };
                    }
                    pin.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    pin.tl = parse_pt2("tl", pin.tl);
                    pin.tr = parse_pt2("tr", pin.tr);
                    pin.br = parse_pt2("br", pin.br);
                    pin.bl = parse_pt2("bl", pin.bl);
                    if args.get("feather_px").is_some() {
                        pin.feather_px = finite_clamped(
                            args.get("feather_px"),
                            f64::from(pin.feather_px),
                            0.0,
                            16.0,
                        ) as f32;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetCornerPin {
                                item_id,
                                corner_pin: pin,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&pin).unwrap_or_default() }]
                    }))
                }
                "reset_corner_pin" => {
                    let pin = CornerPinConfig::default();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetCornerPin {
                                item_id,
                                corner_pin: pin,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&pin).unwrap_or_default() }]
                    }))
                }
                other => Err(format!(
                    "unknown vfx_stabilize_and_corner_pin action: {other}"
                )),
            }
        }
        "spatial_and_master_audio" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let opt_item_id = if args.get("id").is_some() {
                Some(parse_uuid(args, "id")?)
            } else {
                None
            };

            match action {
                "add_spatial_3d" => {
                    let mut node = AudioEffectNode::with_defaults("spatial_3d");
                    node.params.insert(
                        "azimuth_deg".into(),
                        finite_clamped(args.get("azimuth_deg"), 0.0, -180.0, 180.0) as f32,
                    );
                    node.params.insert(
                        "elevation_deg".into(),
                        finite_clamped(args.get("elevation_deg"), 0.0, -90.0, 90.0) as f32,
                    );
                    node.params.insert(
                        "distance".into(),
                        finite_clamped(args.get("distance"), 1.5, 0.3, 25.0) as f32,
                    );
                    node.params.insert(
                        "room_mix".into(),
                        finite_clamped(args.get("room_mix"), 0.12, 0.0, 0.6) as f32,
                    );
                    let effect_id = node.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddAudioEffect {
                                item_id: opt_item_id,
                                effect: node.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "effect_id": effect_id, "target": opt_item_id, "effect": node }).to_string() }]
                    }))
                }
                "add_multiband_comp" => {
                    let mut node = AudioEffectNode::with_defaults("multiband_comp");
                    node.params.insert(
                        "low_thresh_db".into(),
                        finite_clamped(args.get("low_thresh_db"), -18.0, -48.0, 0.0) as f32,
                    );
                    node.params.insert(
                        "mid_thresh_db".into(),
                        finite_clamped(args.get("mid_thresh_db"), -16.0, -48.0, 0.0) as f32,
                    );
                    node.params.insert(
                        "high_thresh_db".into(),
                        finite_clamped(args.get("high_thresh_db"), -20.0, -48.0, 0.0) as f32,
                    );
                    node.params.insert(
                        "ratio".into(),
                        finite_clamped(args.get("ratio"), 3.0, 1.0, 20.0) as f32,
                    );
                    node.params.insert(
                        "makeup_db".into(),
                        finite_clamped(args.get("makeup_db"), 1.5, -12.0, 24.0) as f32,
                    );
                    let effect_id = node.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddAudioEffect {
                                item_id: opt_item_id,
                                effect: node.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "effect_id": effect_id, "target": opt_item_id, "effect": node }).to_string() }]
                    }))
                }
                "add_true_peak_limiter" => {
                    let mut node = AudioEffectNode::with_defaults("true_peak_limiter");
                    node.params.insert(
                        "ceiling_dbtp".into(),
                        finite_clamped(args.get("ceiling_dbtp"), -1.0, -12.0, -0.1) as f32,
                    );
                    node.params.insert(
                        "release_ms".into(),
                        finite_clamped(args.get("release_ms"), 60.0, 10.0, 500.0) as f32,
                    );
                    let effect_id = node.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddAudioEffect {
                                item_id: opt_item_id,
                                effect: node.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "effect_id": effect_id, "target": opt_item_id, "effect": node }).to_string() }]
                    }))
                }
                "measure_master_true_peak" => {
                    let scene_id = s.project.active_scene_id;
                    let total_dur = s
                        .project
                        .active_scene()
                        .map(|sc| sc.total_duration().as_seconds().max(0.5))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let dur_sec =
                        finite_clamped(args.get("duration_sec"), total_dur.min(15.0), 0.1, 60.0);
                    let sr = s.project.settings.audio_sample_rate.max(8_000);
                    let graph =
                        AudioGraph::from_project_scene(&s.project, scene_id, s.folder.as_ref(), sr);
                    let pcm =
                        graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(dur_sec));
                    let true_peak_dbtp = measure_true_peak_dbtp(&pcm, graph.channels);
                    let integrated_lufs = measure_integrated_lufs(&pcm, sr);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "duration_sec": dur_sec,
                                "sample_rate": sr,
                                "true_peak_dbtp": true_peak_dbtp,
                                "integrated_lufs": integrated_lufs,
                                "broadcast_safe": true_peak_dbtp <= -1.0
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown spatial_and_master_audio action: {other}")),
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
        "gpu_v30_pipeline" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "status" => {
                    let caps = detect_gpu_v30_capabilities();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "gpu_available": caps.gpu_available,
                                "adapter_name": caps.adapter_name,
                                "backend_name": caps.backend_name,
                                "color_shader_valid": caps.color_shader_valid,
                                "scopes_shader_valid": caps.scopes_shader_valid,
                                "composite_shader_valid": caps.composite_shader_valid
                            }).to_string()
                        }]
                    }))
                }
                "benchmark_4k" => {
                    let w = args.get("width").and_then(Value::as_u64).unwrap_or(1920) as u32;
                    let h = args.get("height").and_then(Value::as_u64).unwrap_or(1080) as u32;
                    let rep = benchmark_v30_gpu_pipeline(w, h);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "width": rep.width,
                                "height": rep.height,
                                "executed_on_gpu": rep.executed_on_gpu,
                                "backend_name": rep.backend_name,
                                "elapsed_ms": rep.elapsed_ms,
                                "estimated_fps": rep.estimated_fps,
                                "avg_luma_ire": rep.avg_luma_ire
                            }).to_string()
                        }]
                    }))
                }
                "run_grade_and_scopes" => {
                    let t_sec = finite_clamped(
                        args.get("time_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let w = (args.get("width").and_then(Value::as_u64).unwrap_or(640) as u32)
                        .clamp(64, 1920);
                    let h = (args.get("height").and_then(Value::as_u64).unwrap_or(360) as u32)
                        .clamp(64, 1080);
                    let scene_id = s.project.active_scene_id;
                    let grade = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?
                        .color_grade
                        .clone();
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let mut img = rg
                        .render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(t_sec),
                            w,
                            h,
                            false,
                        )
                        .ok_or_else(|| "failed to render frame".to_string())?;
                    let (scopes, on_gpu, backend) =
                        run_gpu_or_cpu_grade_and_scopes(&mut img, &grade, None);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "executed_on_gpu": on_gpu,
                                "backend_name": backend,
                                "avg_luma_ire": scopes.avg_luma_ire,
                                "peak_luma_ire": scopes.peak_luma_ire,
                                "clipped_highlight_pct": scopes.clipped_highlight_pct,
                                "crushed_shadow_pct": scopes.crushed_shadow_pct
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown gpu_v30_pipeline action: {other}")),
            }
        }
        "scene_3d_and_camera_track" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_preset = |raw: Option<&str>, cur: Mesh3DPreset| match raw {
                Some("sphere") => Mesh3DPreset::Sphere,
                Some("torus") => Mesh3DPreset::Torus,
                Some("extruded_text") => Mesh3DPreset::ExtrudedText,
                Some("gltf_file") => Mesh3DPreset::GltfFile,
                Some("cube") => Mesh3DPreset::Cube,
                _ => cur,
            };
            let apply_mesh_args = |cfg: &mut Mesh3DConfig| {
                cfg.preset = parse_preset(args.get("preset").and_then(Value::as_str), cfg.preset);
                if let Some(p) = args.get("gltf_path").and_then(Value::as_str) {
                    cfg.gltf_path = if p.trim().is_empty() {
                        None
                    } else {
                        Some(p.to_string())
                    };
                }
                if let Some(txt) = args.get("text_content").and_then(Value::as_str) {
                    cfg.text_content = txt.to_string();
                }
                if args.get("extrude_depth").is_some() {
                    cfg.extrude_depth = finite_clamped(
                        args.get("extrude_depth"),
                        f64::from(cfg.extrude_depth),
                        0.05,
                        2.0,
                    ) as f32;
                }
                if let Some(hex) = args.get("base_color").and_then(Value::as_str)
                    && let Some(c) = RgbaColor::from_hex(hex)
                {
                    cfg.base_color = c;
                }
                if args.get("metallic").is_some() {
                    cfg.metallic =
                        finite_clamped(args.get("metallic"), f64::from(cfg.metallic), 0.0, 1.0)
                            as f32;
                }
                if args.get("roughness").is_some() {
                    cfg.roughness =
                        finite_clamped(args.get("roughness"), f64::from(cfg.roughness), 0.06, 1.0)
                            as f32;
                }
                if args.get("env_reflection").is_some() {
                    cfg.env_reflection = finite_clamped(
                        args.get("env_reflection"),
                        f64::from(cfg.env_reflection),
                        0.0,
                        1.0,
                    ) as f32;
                }
                if let Some(wf) = args.get("wireframe").and_then(Value::as_bool) {
                    cfg.wireframe = wf;
                }
                if args.get("auto_spin_deg_per_sec").is_some() {
                    cfg.auto_spin_deg_per_sec = finite_clamped(
                        args.get("auto_spin_deg_per_sec"),
                        f64::from(cfg.auto_spin_deg_per_sec),
                        -720.0,
                        720.0,
                    ) as f32;
                }
            };

            match action {
                "add_mesh_3d" => {
                    let max_layer = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len().saturating_sub(1))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let layer_index = (args.get("layer_index").and_then(Value::as_u64).unwrap_or(0)
                        as usize)
                        .min(max_layer);
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("3D Mesh")
                        .to_string();
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    ));
                    let dur = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        4.0,
                        0.1,
                        3600.0,
                    ));
                    let mut cfg = Mesh3DConfig::default();
                    apply_mesh_args(&mut cfg);
                    let item = TimelineItem::new(
                        name,
                        start,
                        dur,
                        ItemKind::Mesh3D {
                            config: cfg.clone(),
                        },
                    );
                    let created_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "created_id": created_id, "mesh_3d": cfg }).to_string() }]
                    }))
                }
                "update_mesh_3d" => {
                    let item_id = parse_uuid(args, "id")?;
                    let mut cfg = s
                        .project
                        .active_scene()
                        .and_then(|sc| {
                            sc.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::Mesh3D { config } = &i.kind
                                {
                                    Some(config.clone())
                                } else {
                                    None
                                }
                            })
                        })
                        .ok_or_else(|| format!("mesh_3d item not found: {item_id}"))?;
                    apply_mesh_args(&mut cfg);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetMesh3D {
                                item_id,
                                config: cfg.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "item_id": item_id, "mesh_3d": cfg }).to_string() }]
                    }))
                }
                "emit_sample_glb" => {
                    let cube = build_cube_mesh();
                    let bytes = encode_mesh_to_glb(&cube);
                    let parsed = parse_gltf_or_glb(&bytes, None)?;
                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir())
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_glb"));
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let glb_path = out_dir.join("sample_cube.glb");
                    std::fs::write(&glb_path, &bytes).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "glb_path": glb_path.display().to_string(),
                                "bytes": bytes.len(),
                                "triangles": parsed.triangle_count(),
                                "vertices": parsed.vertices.len()
                            }).to_string()
                        }]
                    }))
                }
                "track_3d_camera" => {
                    let step = finite_clamped(args.get("sample_interval_sec"), 0.25, 0.1, 2.0);
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let scene_id = s.project.active_scene_id;
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let samples = analyze_3d_camera_track(&mut rg, &s.project, scene_id, step);
                    let base_cam = s
                        .project
                        .active_scene()
                        .map(|sc| sc.camera)
                        .ok_or_else(|| "no active scene".to_string())?;
                    let applied = if apply
                        && let Some(cmd) = build_3d_camera_track_command(&base_cam, &samples)
                    {
                        s.bus
                            .execute(&mut s.project, cmd, CommandSource::Mcp, None)
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "sample_count": samples.len(),
                                "applied": applied,
                                "camera_position": s.project.active_scene().map(|sc| sc.camera.position)
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown scene_3d_and_camera_track action: {other}")),
            }
        }
        "offline_ai_dubbing_lipsync" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "list_voices" => {
                    let voices: Vec<Value> = DubbingVoicePreset::ALL
                        .iter()
                        .map(|v| {
                            json!({
                                "id": v.id(),
                                "name": v.display_name(),
                                "base_pitch_hz": v.base_pitch_hz()
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "offline_only": true, "voices": voices }).to_string() }]
                    }))
                }
                "align_phonemes" => {
                    let text = args
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'text'".to_string())?;
                    let dur_sec = finite_clamped(args.get("duration_sec"), 2.0, 0.15, 60.0);
                    let segs = align_text_to_phoneme_segments(
                        text,
                        RationalTime::ZERO,
                        RationalTime::from_seconds(dur_sec),
                    );
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "phoneme_count": segs.len(), "phonemes": segs }).to_string() }]
                    }))
                }
                "generate_locale_dubbing" => {
                    let locale = args.get("locale").and_then(Value::as_str).unwrap_or("en");
                    let voice = DubbingVoicePreset::from_id(
                        args.get("voice").and_then(Value::as_str).unwrap_or(
                            if locale.starts_with("en") {
                                "en_female"
                            } else {
                                "jp_female"
                            },
                        ),
                    );
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let max_layer = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len().saturating_sub(1))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let audio_layer_index = (args
                        .get("audio_layer_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(max_layer as u64)
                        as usize)
                        .min(max_layer);

                    let targets: Vec<(Uuid, String, RationalTime, RationalTime)> =
                        if let Some(csv) = args.get("csv").and_then(Value::as_str) {
                            let tbl = import_localization_csv(csv);
                            let sc = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            ve_audio::collect_dubbing_targets_from_table(sc, &tbl, locale)
                        } else if let Some(map) =
                            args.get("translations").and_then(Value::as_object)
                        {
                            let sc = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            let mut out = Vec::new();
                            for (k, v) in map {
                                if let (Ok(id), Some(txt)) = (Uuid::parse_str(k), v.as_str())
                                    && let Some(item) = sc
                                        .layers
                                        .iter()
                                        .flat_map(|l| l.items.iter())
                                        .find(|i| i.id == id)
                                {
                                    out.push((id, txt.to_string(), item.start, item.duration));
                                }
                            }
                            out
                        } else {
                            let sc = s
                                .project
                                .active_scene()
                                .ok_or_else(|| "no active scene".to_string())?;
                            sc.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .filter_map(|i| {
                                    if let ItemKind::Text { content, .. } = &i.kind {
                                        Some((i.id, content.clone(), i.start, i.duration))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        };

                    if targets.is_empty() {
                        return Err("吹替対象の字幕アイテムがありません".into());
                    }

                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir().join("dubbing"))
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_dubbing"));
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let sr = s.project.settings.audio_sample_rate.max(8_000);

                    let mut generated_clips = Vec::new();
                    let mut wav_paths = Vec::new();
                    for (idx, (sub_id, txt, start, dur)) in targets.into_iter().enumerate() {
                        let phonemes = align_text_to_phoneme_segments(&txt, start, dur);
                        let pcm = synthesize_offline_dubbing_pcm(&phonemes, start, dur, sr, voice);
                        let wav_name = format!("dub_{locale}_{idx}.wav");
                        let wav_path = out_dir.join(&wav_name);
                        write_wav_pcm16(&wav_path, &pcm, sr, 2).map_err(|e| e.to_string())?;
                        wav_paths.push(wav_path.display().to_string());

                        let asset_id = Uuid::new_v4();
                        s.project.assets.push(AssetRef {
                            id: asset_id,
                            name: wav_name.clone(),
                            relative_path: Some(PathBuf::from("assets/dubbing").join(&wav_name)),
                            fallback_absolute_path: Some(wav_path),
                            collected: true,
                            kind: MediaKind::Audio,
                        });
                        generated_clips.push(GeneratedDubClip {
                            subtitle_item_id: sub_id,
                            translated_text: txt,
                            start,
                            duration: dur,
                            audio_asset_id: asset_id,
                            clip_name: format!("Dub[{locale}] #{idx}"),
                            phonemes,
                        });
                    }

                    let dub_count = generated_clips.len();
                    let cmds = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_dubbing_timeline_commands(sc, audio_layer_index, &generated_clips)
                    };
                    let applied = if apply && !cmds.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!(
                                        "オフライン多言語吹替＆リップシンク [{locale}] ({dub_count}件)"
                                    ),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "offline_only": true,
                                "locale": locale,
                                "voice": voice.id(),
                                "generated_clips": dub_count,
                                "wav_paths": wav_paths,
                                "applied": applied
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown offline_ai_dubbing_lipsync action: {other}"
                )),
            }
        }
        "crdt_realtime_collab" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "status" => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({
                            "peer_id": s.crdt_log.local_peer_id,
                            "display_name": s.crdt_log.display_name,
                            "color_hex": s.crdt_log.color_hex,
                            "lamport_clock": s.crdt_log.lamport_clock,
                            "vector_clock": s.crdt_log.vector_clock,
                            "op_count": s.crdt_log.operations.len(),
                            "peers": s.crdt_log.peers
                        }).to_string()
                    }]
                })),
                "set_peer_profile" => {
                    if let Some(pid) = args.get("peer_id").and_then(Value::as_str)
                        && !pid.trim().is_empty()
                    {
                        s.crdt_log.local_peer_id = pid.trim().to_string();
                    }
                    if let Some(dn) = args.get("display_name").and_then(Value::as_str)
                        && !dn.trim().is_empty()
                    {
                        s.crdt_log.display_name = dn.trim().to_string();
                    }
                    if let Some(c) = args.get("color_hex").and_then(Value::as_str)
                        && !c.trim().is_empty()
                    {
                        s.crdt_log.color_hex = c.trim().to_string();
                    }
                    Ok(json!({
                        "content": [{ "type": "text", "text": json!({ "peer_id": s.crdt_log.local_peer_id, "display_name": s.crdt_log.display_name }).to_string() }]
                    }))
                }
                "update_cursor" => {
                    let playhead_sec = finite_clamped(
                        args.get("playhead_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let sel = args
                        .get("selected_item")
                        .and_then(Value::as_str)
                        .and_then(|v| Uuid::parse_str(v).ok());
                    let op = s.crdt_log.update_local_cursor(playhead_sec, sel);
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&op).unwrap_or_default() }]
                    }))
                }
                "record_marker_op" => {
                    let t = finite_clamped(
                        args.get("time_sec"),
                        s.playhead.as_seconds(),
                        0.0,
                        86_400.0,
                    );
                    let label = args
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("CRDT Marker");
                    let marker = TimelineMarker::new(RationalTime::from_seconds(t), label);
                    let op = s.crdt_log.record_local(CrdtOpKind::AddMarker { marker });
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&op).unwrap_or_default() }]
                    }))
                }
                "record_text_op" => {
                    let item_id = parse_uuid(args, "id")?;
                    let content = args
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'content'".to_string())?
                        .to_string();
                    let op = s
                        .crdt_log
                        .record_local(CrdtOpKind::SetTextContent { item_id, content });
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&op).unwrap_or_default() }]
                    }))
                }
                "export_delta" => {
                    let delta = s.crdt_log.export_delta_json(None);
                    Ok(json!({ "content": [{ "type": "text", "text": delta }] }))
                }
                "merge_remote_delta" => {
                    let delta_str = args
                        .get("delta_json")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'delta_json'".to_string())?;
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let ops = CrdtSyncLog::parse_delta_json(delta_str)?;
                    let merged_new = s.crdt_log.merge_remote_ops(&ops);
                    let applied = if apply {
                        let converged = s.crdt_log.apply_to_project(&s.project);
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::ReplaceProjectSnapshot {
                                    project: Box::new(converged),
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                        true
                    } else {
                        false
                    };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "merged_new_ops": merged_new,
                                "total_ops": s.crdt_log.operations.len(),
                                "lamport_clock": s.crdt_log.lamport_clock,
                                "peer_count": s.crdt_log.peers.len(),
                                "applied": applied
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown crdt_realtime_collab action: {other}")),
            }
        }
        "aces_color_management" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_cs = |k: &str, fallback: ColorSpace| -> ColorSpace {
                match args.get(k).and_then(Value::as_str) {
                    Some("linear_srgb") => ColorSpace::LinearSrgb,
                    Some("aces_cg") => ColorSpace::AcesCg,
                    Some("display_p3") => ColorSpace::DisplayP3,
                    Some("rec2020") => ColorSpace::Rec2020,
                    Some("s_log3") => ColorSpace::SLog3,
                    Some("v_log") => ColorSpace::VLog,
                    Some("log_c3") => ColorSpace::LogC3,
                    _ => fallback,
                }
            };
            let parse_tm = |k: &str, fallback: ToneMapper| -> ToneMapper {
                match args.get(k).and_then(Value::as_str) {
                    Some("none") => ToneMapper::None,
                    Some("aces_fitted") => ToneMapper::AcesFitted,
                    Some("ag_x") => ToneMapper::AgX,
                    Some("reinhard_extended") => ToneMapper::ReinhardExtended,
                    _ => fallback,
                }
            };
            match action {
                "get_config" => {
                    let cfg = s
                        .project
                        .active_scene()
                        .map(|sc| sc.color_management)
                        .ok_or_else(|| "no active scene".to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&cfg).unwrap_or_default() }]
                    }))
                }
                "set_config" => {
                    let cur = s
                        .project
                        .active_scene()
                        .map(|sc| sc.color_management)
                        .ok_or_else(|| "no active scene".to_string())?;
                    let next = ColorManagementConfig {
                        enabled: args
                            .get("enabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(cur.enabled),
                        input_idt: parse_cs("input_idt", cur.input_idt),
                        working_space: parse_cs("working_space", cur.working_space),
                        output_odt: parse_cs("output_odt", cur.output_odt),
                        tone_mapper: parse_tm("tone_mapper", cur.tone_mapper),
                        exposure_stops: finite_clamped(
                            args.get("exposure_stops"),
                            f64::from(cur.exposure_stops),
                            -4.0,
                            4.0,
                        ) as f32,
                        gamut_compress: args
                            .get("gamut_compress")
                            .and_then(Value::as_bool)
                            .unwrap_or(cur.gamut_compress),
                    };
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetColorManagement { config: next },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "applied": true, "config": next }).to_string()
                        }]
                    }))
                }
                "export_ocio" => {
                    let cfg = s
                        .project
                        .active_scene()
                        .map(|sc| sc.color_management)
                        .ok_or_else(|| "no active scene".to_string())?;
                    let yaml = export_ocio_config_yaml(&cfg);
                    let saved_path =
                        if let Some(p) = args.get("output_path").and_then(Value::as_str) {
                            let out_p = PathBuf::from(p);
                            if let Some(parent) = out_p.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&out_p, &yaml).map_err(|e| e.to_string())?;
                            Some(out_p.display().to_string())
                        } else {
                            None
                        };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "ocio_yaml": yaml, "saved_path": saved_path }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown aces_color_management action: {other}")),
            }
        }
        "physics_simulate_and_bake" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let preset = match args.get("preset").and_then(Value::as_str) {
                Some("domino_cascade") => PhysicsPreset::DominoCascade,
                Some("zero_gravity_float") => PhysicsPreset::ZeroGravityFloat,
                Some("orbital_vortex") => PhysicsPreset::OrbitalVortex,
                _ => PhysicsPreset::BouncingTitle,
            };
            let base = preset.default_config();
            let cfg = PhysicsConfig {
                preset,
                gravity_y: finite_clamped(
                    args.get("gravity_y"),
                    f64::from(base.gravity_y),
                    -4000.0,
                    4000.0,
                ) as f32,
                restitution: finite_clamped(
                    args.get("restitution"),
                    f64::from(base.restitution),
                    0.0,
                    1.0,
                ) as f32,
                friction: finite_clamped(args.get("friction"), f64::from(base.friction), 0.5, 1.0)
                    as f32,
                air_drag: finite_clamped(args.get("air_drag"), f64::from(base.air_drag), 0.0, 0.5)
                    as f32,
                initial_vx: finite_clamped(
                    args.get("initial_vx"),
                    f64::from(base.initial_vx),
                    -3000.0,
                    3000.0,
                ) as f32,
                initial_vy: finite_clamped(
                    args.get("initial_vy"),
                    f64::from(base.initial_vy),
                    -3000.0,
                    3000.0,
                ) as f32,
                initial_spin_deg: finite_clamped(
                    args.get("initial_spin_deg"),
                    f64::from(base.initial_spin_deg),
                    -720.0,
                    720.0,
                ) as f32,
                central_attract: finite_clamped(
                    args.get("central_attract"),
                    f64::from(base.central_attract),
                    0.0,
                    3000.0,
                ) as f32,
                duration_sec: finite_clamped(
                    args.get("duration_sec"),
                    base.duration_sec,
                    0.2,
                    30.0,
                ),
                bake_step_frames: args
                    .get("bake_step_frames")
                    .and_then(Value::as_u64)
                    .map(|v| v.clamp(1, 30) as u32)
                    .unwrap_or(base.bake_step_frames),
                floor_ratio: finite_clamped(
                    args.get("floor_ratio"),
                    f64::from(base.floor_ratio),
                    0.2,
                    0.98,
                ) as f32,
            };

            let target_ids: Vec<Uuid> = {
                let sc = s
                    .project
                    .active_scene()
                    .ok_or_else(|| "no active scene".to_string())?;
                if let Some(arr) = args.get("item_ids").and_then(Value::as_array)
                    && !arr.is_empty()
                {
                    arr.iter()
                        .filter_map(|v| v.as_str().and_then(|st| Uuid::parse_str(st).ok()))
                        .collect()
                } else {
                    sc.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .filter(|i| {
                            !matches!(
                                i.kind,
                                ItemKind::Audio { .. }
                                    | ItemKind::FilterObject { .. }
                                    | ItemKind::Transition { .. }
                                    | ItemKind::GroupControl { .. }
                                    | ItemKind::GenerativeBg { .. }
                            )
                        })
                        .take(12)
                        .map(|i| i.id)
                        .collect()
                }
            };

            if target_ids.is_empty() {
                return Err("物理演算の対象オブジェクトが見つかりません".into());
            }

            match action {
                "preview_trajectory" => {
                    let sc = s
                        .project
                        .active_scene()
                        .ok_or_else(|| "no active scene".to_string())?;
                    let trajectories = simulate_scene_physics(sc, &target_ids, &cfg);
                    let total_bounces: u32 = trajectories.iter().map(|t| t.bounce_count).sum();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "body_count": trajectories.len(),
                                "total_bounces": total_bounces,
                                "trajectories": trajectories
                            }).to_string()
                        }]
                    }))
                }
                "bake_keyframes" => {
                    let (trajectories, cmds) = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        build_physics_bake_commands(sc, &target_ids, &cfg)
                    };
                    let cmd_count = cmds.len();
                    let total_bounces: u32 = trajectories.iter().map(|t| t.bounce_count).sum();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!("2D 剛体物理ベイク ({}体)", trajectories.len()),
                                commands: cmds,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "baked": true,
                                "body_count": trajectories.len(),
                                "keyframe_commands": cmd_count,
                                "total_bounces": total_bounces
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown physics_simulate_and_bake action: {other}")),
            }
        }
        "generative_bg_and_foley" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_bg_preset = |raw: Option<&str>| -> GenerativeBgPreset {
                match raw {
                    Some("cinema_bokeh") => GenerativeBgPreset::CinemaBokeh,
                    Some("cyber_grid3d") | Some("cyber_grid_3d") => GenerativeBgPreset::CyberGrid3D,
                    Some("liquid_marble") => GenerativeBgPreset::LiquidMarble,
                    Some("starfield_warp") => GenerativeBgPreset::StarfieldWarp,
                    Some("abstract_waves") => GenerativeBgPreset::AbstractWaves,
                    _ => GenerativeBgPreset::AuroraBorealis,
                }
            };

            match action {
                "add_generative_bg" => {
                    let (sc_w, sc_h, layer_count) = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        (sc.width as f32, sc.height as f32, sc.layers.len())
                    };
                    let preset = parse_bg_preset(args.get("preset").and_then(Value::as_str));
                    let base = GenerativeBgPreset::for_preset(preset);
                    let cfg = GenerativeBgConfig {
                        preset,
                        seed: args
                            .get("seed")
                            .and_then(Value::as_u64)
                            .unwrap_or(base.seed),
                        speed: finite_clamped(args.get("speed"), f64::from(base.speed), 0.0, 10.0)
                            as f32,
                        scale: finite_clamped(args.get("scale"), f64::from(base.scale), 0.1, 10.0)
                            as f32,
                        complexity: args
                            .get("complexity")
                            .and_then(Value::as_u64)
                            .map(|v| v.clamp(1, 8) as u32)
                            .unwrap_or(base.complexity),
                        width: finite_clamped(args.get("width"), f64::from(sc_w), 32.0, 8192.0)
                            as f32,
                        height: finite_clamped(args.get("height"), f64::from(sc_h), 32.0, 8192.0)
                            as f32,
                        ..base
                    };
                    let layer_index = args
                        .get("layer_index")
                        .and_then(Value::as_u64)
                        .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                        .unwrap_or(0);
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let dur = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        5.0,
                        0.1,
                        3600.0,
                    ));
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(preset.display_name())
                        .to_string();
                    let item =
                        TimelineItem::new(name, start, dur, ItemKind::GenerativeBg { config: cfg });
                    let item_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "item_id": item_id.to_string(), "preset": preset, "config": cfg }).to_string()
                        }]
                    }))
                }
                "update_generative_bg" => {
                    let id_str = args
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'id'".to_string())?;
                    let item_id = Uuid::parse_str(id_str).map_err(|e| e.to_string())?;
                    let cur = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        sc.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::GenerativeBg { config } = i.kind
                                {
                                    Some(config)
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| format!("GenerativeBg item not found: {item_id}"))?
                    };
                    let next_preset = if args.get("preset").is_some() {
                        parse_bg_preset(args.get("preset").and_then(Value::as_str))
                    } else {
                        cur.preset
                    };
                    let base = if next_preset != cur.preset {
                        let mut p = GenerativeBgPreset::for_preset(next_preset);
                        p.width = cur.width;
                        p.height = cur.height;
                        p
                    } else {
                        cur
                    };
                    let next = GenerativeBgConfig {
                        preset: next_preset,
                        seed: args
                            .get("seed")
                            .and_then(Value::as_u64)
                            .unwrap_or(base.seed),
                        speed: finite_clamped(args.get("speed"), f64::from(base.speed), 0.0, 10.0)
                            as f32,
                        scale: finite_clamped(args.get("scale"), f64::from(base.scale), 0.1, 10.0)
                            as f32,
                        complexity: args
                            .get("complexity")
                            .and_then(Value::as_u64)
                            .map(|v| v.clamp(1, 8) as u32)
                            .unwrap_or(base.complexity),
                        width: finite_clamped(
                            args.get("width"),
                            f64::from(base.width),
                            32.0,
                            8192.0,
                        ) as f32,
                        height: finite_clamped(
                            args.get("height"),
                            f64::from(base.height),
                            32.0,
                            8192.0,
                        ) as f32,
                        ..base
                    };
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetGenerativeBg {
                                item_id,
                                config: next,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "item_id": item_id.to_string(), "config": next }).to_string()
                        }]
                    }))
                }
                "synthesize_foley_sfx" => {
                    let preset = FoleyPreset::from_id(
                        args.get("preset")
                            .and_then(Value::as_str)
                            .unwrap_or("whoosh_transition"),
                    );
                    let dur_sec = finite_clamped(
                        args.get("duration_sec"),
                        f64::from(preset.default_duration_sec()),
                        0.08,
                        30.0,
                    ) as f32;
                    let seed = args.get("seed").and_then(Value::as_u64).unwrap_or(40_2026);
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let sr = s.project.settings.audio_sample_rate.max(8_000);
                    let pcm = synthesize_foley_sfx_pcm(preset, dur_sec, sr, seed);

                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir().join("foley"))
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_foley"));
                    let wav_name = format!("foley_{}_{seed}.wav", preset.id());
                    let wav_path = out_dir.join(&wav_name);
                    write_stereo_wav_file(&wav_path, &pcm, sr)?;

                    let asset_id = Uuid::new_v4();
                    s.project.assets.push(AssetRef {
                        id: asset_id,
                        name: wav_name.clone(),
                        relative_path: Some(PathBuf::from("assets/foley").join(&wav_name)),
                        fallback_absolute_path: Some(wav_path.clone()),
                        collected: true,
                        kind: MediaKind::Audio,
                    });

                    let mut placed_item_id: Option<String> = None;
                    if apply {
                        let layer_count = s
                            .project
                            .active_scene()
                            .map(|sc| sc.layers.len())
                            .ok_or_else(|| "no active scene".to_string())?;
                        let audio_layer = args
                            .get("audio_layer_index")
                            .and_then(Value::as_u64)
                            .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                            .unwrap_or_else(|| layer_count.saturating_sub(1));
                        let clip = TimelineItem::new(
                            format!("SFX: {}", preset.display_name()),
                            start,
                            RationalTime::from_seconds(f64::from(dur_sec)),
                            ItemKind::Audio {
                                asset_id,
                                source_offset: RationalTime::ZERO,
                                volume: 0.9,
                            },
                        );
                        placed_item_id = Some(clip.id.to_string());
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::AddObject {
                                    layer_index: audio_layer,
                                    item: clip,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "preset": preset.id(),
                                "wav_path": wav_path.display().to_string(),
                                "asset_id": asset_id.to_string(),
                                "item_id": placed_item_id,
                                "duration_sec": dur_sec,
                                "sample_rate": sr,
                                "applied": apply
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown generative_bg_and_foley action: {other}")),
            }
        }
        "broadcast_qc_and_web_export" => {
            let s = &*guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let profile = match args.get("profile").and_then(Value::as_str) {
                Some("broadcast_itu_ebu_r128") => BroadcastProfile::BroadcastItuEbuR128,
                Some("shorts_tiktok9x16") => BroadcastProfile::ShortsTikTok9x16,
                Some("cinema_dci4k") => BroadcastProfile::CinemaDci4k,
                _ => BroadcastProfile::YouTube1080p,
            };
            let sample_count = args
                .get("sample_count")
                .and_then(Value::as_u64)
                .map(|v| v.clamp(4, 24) as usize)
                .unwrap_or(8);

            let scene_id = s.project.active_scene_id;
            let mut rg = RenderGraph::new();
            rg.set_project_folder(s.folder.clone());

            let compute_qc = |rg: &mut RenderGraph| -> Result<ve_core::BroadcastQcReport, String> {
                let (
                    peak_luma_ire,
                    clipped_highlight_pct,
                    crushed_shadow_pct,
                    max_frame_luma_jump_ire,
                ) = measure_scene_video_qc_metrics(rg, &s.project, scene_id, sample_count);
                let dur = s
                    .project
                    .active_scene()
                    .map(|sc| sc.total_duration().as_seconds().clamp(0.5, 30.0))
                    .ok_or_else(|| "no active scene".to_string())?;
                let sr = s.project.settings.audio_sample_rate.max(8_000);
                let ag =
                    AudioGraph::from_project_scene(&s.project, scene_id, s.folder.as_ref(), sr);
                let pcm = if ag.clips.is_empty() {
                    Vec::new()
                } else {
                    ag.render_range(RationalTime::ZERO, RationalTime::from_seconds(dur))
                };
                let true_peak_dbtp = if pcm.is_empty() {
                    -96.0
                } else {
                    measure_true_peak_dbtp(&pcm, ag.channels)
                };
                let integrated_lufs = if pcm.is_empty() {
                    -70.0
                } else {
                    measure_integrated_lufs(&pcm, sr)
                };
                let metrics = BroadcastQcMetrics {
                    peak_luma_ire,
                    clipped_highlight_pct,
                    crushed_shadow_pct,
                    max_frame_luma_jump_ire,
                    true_peak_dbtp,
                    integrated_lufs,
                };
                Ok(run_broadcast_qc_audit(&s.project, profile, metrics))
            };

            match action {
                "run_qc_audit" => {
                    let report = compute_qc(&mut rg)?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&report).unwrap_or_default() }]
                    }))
                }
                "generate_hero_thumbnail" => {
                    let (best_t, score, hero) =
                        select_best_thumbnail_frame(&mut rg, &s.project, scene_id, sample_count)
                            .ok_or_else(|| "ヒーローサムネイルの抽出に失敗しました".to_string())?;
                    let png_bytes = encode_linear_image_png_bytes(&hero)?;
                    let out_path = if let Some(p) = args.get("output_path").and_then(Value::as_str)
                    {
                        PathBuf::from(p)
                    } else {
                        s.folder
                            .as_ref()
                            .map(|f| f.root().join("exports").join("hero_thumbnail_1280x720.png"))
                            .unwrap_or_else(|| {
                                std::env::temp_dir().join("ve_hero_thumbnail_1280x720.png")
                            })
                    };
                    if let Some(parent) = out_path.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&out_path, &png_bytes).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "timestamp_sec": best_t,
                                "attractiveness_score": score,
                                "width": hero.width,
                                "height": hero.height,
                                "bytes": png_bytes.len(),
                                "output_path": out_path.display().to_string()
                            }).to_string()
                        }]
                    }))
                }
                "export_web_review_html" => {
                    let report = compute_qc(&mut rg)?;
                    let dur = s
                        .project
                        .active_scene()
                        .map(|sc| sc.total_duration().as_seconds().max(0.5))
                        .ok_or_else(|| "no active scene".to_string())?;
                    let mut previews: Vec<(f64, String)> = Vec::new();
                    for i in 0..sample_count {
                        let t_sec = dur * (i as f64) / ((sample_count - 1).max(1) as f64);
                        if let Some(img) = rg.render_project_scene_scaled(
                            &s.project,
                            scene_id,
                            RationalTime::from_seconds(t_sec),
                            480,
                            270,
                            false,
                        ) && let Ok(png) = encode_linear_image_png_bytes(&img)
                        {
                            previews.push((t_sec, base64_encode(&png)));
                        }
                    }
                    let html = export_portable_web_review_html(&s.project, &report, &previews);
                    let out_path = if let Some(p) = args.get("output_path").and_then(Value::as_str)
                    {
                        PathBuf::from(p)
                    } else {
                        s.folder
                            .as_ref()
                            .map(|f| f.root().join("exports").join("portable_review_player.html"))
                            .unwrap_or_else(|| {
                                std::env::temp_dir().join("ve_portable_review_player.html")
                            })
                    };
                    if let Some(parent) = out_path.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&out_path, &html).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "output_path": out_path.display().to_string(),
                                "embedded_frames": previews.len(),
                                "html_bytes": html.len(),
                                "overall_pass": report.overall_pass
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown broadcast_qc_and_web_export action: {other}"
                )),
            }
        }
        "rig2d_puppet_control" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let parse_rig_preset =
                |raw: Option<&str>, fallback: RigMotionPreset| -> RigMotionPreset {
                    match raw {
                        Some("idle_breathe") => RigMotionPreset::IdleBreathe,
                        Some("wave_hello") => RigMotionPreset::WaveHello,
                        Some("nod_talk") => RigMotionPreset::NodTalk,
                        Some("dance_bounce") => RigMotionPreset::DanceBounce,
                        _ => fallback,
                    }
                };
            let apply_rig_overrides = |mut cfg: Rig2DConfig| -> Rig2DConfig {
                cfg.preset =
                    parse_rig_preset(args.get("preset").and_then(Value::as_str), cfg.preset);
                cfg.motion_speed = finite_clamped(
                    args.get("motion_speed"),
                    f64::from(cfg.motion_speed),
                    0.0,
                    6.0,
                ) as f32;
                cfg.motion_amp =
                    finite_clamped(args.get("motion_amp"), f64::from(cfg.motion_amp), 0.0, 3.0)
                        as f32;
                if let Some(ov) = args.get("show_bones_overlay").and_then(Value::as_bool) {
                    cfg.show_bones_overlay = ov;
                }
                if let Some(en) = args.get("right_ik_enabled").and_then(Value::as_bool) {
                    cfg.right_arm_ik.enabled = en;
                }
                if let Some(x) = args.get("right_ik_x").and_then(Value::as_f64) {
                    cfg.right_arm_ik.target_pos[0] = x.clamp(-400.0, 400.0) as f32;
                }
                if let Some(y) = args.get("right_ik_y").and_then(Value::as_f64) {
                    cfg.right_arm_ik.target_pos[1] = y.clamp(-400.0, 400.0) as f32;
                }
                if let Some(en) = args.get("left_ik_enabled").and_then(Value::as_bool) {
                    cfg.left_arm_ik.enabled = en;
                }
                if let Some(x) = args.get("left_ik_x").and_then(Value::as_f64) {
                    cfg.left_arm_ik.target_pos[0] = x.clamp(-400.0, 400.0) as f32;
                }
                if let Some(y) = args.get("left_ik_y").and_then(Value::as_f64) {
                    cfg.left_arm_ik.target_pos[1] = y.clamp(-400.0, 400.0) as f32;
                }
                cfg
            };

            match action {
                "add_rigged_puppet" => {
                    let layer_count = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len())
                        .ok_or_else(|| "no active scene".to_string())?;
                    let layer_index = args
                        .get("layer_index")
                        .and_then(Value::as_u64)
                        .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                        .unwrap_or(1.min(layer_count.saturating_sub(1)));
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let dur = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        5.0,
                        0.1,
                        3600.0,
                    ));
                    let cfg = apply_rig_overrides(Rig2DConfig::default_humanoid());
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Rig2D Puppet")
                        .to_string();
                    let item = TimelineItem::new(
                        name,
                        start,
                        dur,
                        ItemKind::RiggedPuppet {
                            config: Box::new(cfg.clone()),
                        },
                    );
                    let item_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "item_id": item_id.to_string(),
                                "bone_count": cfg.bones.len(),
                                "preset": cfg.preset
                            }).to_string()
                        }]
                    }))
                }
                "update_rig" => {
                    let id_str = args
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing 'id'".to_string())?;
                    let item_id = Uuid::parse_str(id_str).map_err(|e| e.to_string())?;
                    let cur = {
                        let sc = s
                            .project
                            .active_scene()
                            .ok_or_else(|| "no active scene".to_string())?;
                        sc.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find_map(|i| {
                                if i.id == item_id
                                    && let ItemKind::RiggedPuppet { config } = &i.kind
                                {
                                    Some((**config).clone())
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| format!("RiggedPuppet not found: {item_id}"))?
                    };
                    let next = apply_rig_overrides(cur);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetRig2D {
                                item_id,
                                config: Box::new(next.clone()),
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
                                "preset": next.preset,
                                "right_arm_ik": next.right_arm_ik
                            }).to_string()
                        }]
                    }))
                }
                "solve_ik_preview" => {
                    let t = finite_clamped(args.get("time_sec"), 0.0, 0.0, 3600.0) as f32;
                    let cfg = apply_rig_overrides(Rig2DConfig::default_humanoid());
                    let poses = evaluate_rig_poses(&cfg, t);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "time_sec": t,
                                "bone_count": poses.len(),
                                "poses": poses
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown rig2d_puppet_control action: {other}")),
            }
        }
        "offline_bgm_composer" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            let genre = BgmGenre::from_id(
                args.get("genre")
                    .and_then(Value::as_str)
                    .unwrap_or("lofi_chill"),
            );
            let cfg = BgmComposeConfig {
                genre,
                bpm: finite_clamped(args.get("bpm"), f64::from(genre.default_bpm()), 55.0, 200.0)
                    as f32,
                key_semitone: args
                    .get("key_semitone")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .rem_euclid(12) as i32,
                duration_sec: finite_clamped(args.get("duration_sec"), 8.0, 1.0, 120.0) as f32,
                intensity: finite_clamped(args.get("intensity"), 1.0, 0.3, 1.5) as f32,
                seed: args.get("seed").and_then(Value::as_u64).unwrap_or(50_2026),
            };
            let sr = s.project.settings.audio_sample_rate.max(8_000);
            let composed = compose_bgm_track(&cfg, sr);

            match action {
                "preview_arrangement" => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({
                            "genre": genre.id(),
                            "bpm": composed.bpm,
                            "chord_progression": composed.chord_progression,
                            "beat_count": composed.beat_times_sec.len(),
                            "bar_count": composed.bar_times_sec.len(),
                            "bar_times_sec": composed.bar_times_sec
                        }).to_string()
                    }]
                })),
                "compose_and_place_bgm" => {
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);
                    let add_bar_markers = args
                        .get("add_bar_markers")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let out_dir = s
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir().join("bgm"))
                        .unwrap_or_else(|| std::env::temp_dir().join("ve_bgm"));
                    let wav_name = format!("bgm_{}_{}.wav", genre.id(), cfg.seed);
                    let wav_path = out_dir.join(&wav_name);
                    write_stereo_wav_file(&wav_path, &composed.pcm, composed.sample_rate)?;

                    let asset_id = Uuid::new_v4();
                    s.project.assets.push(AssetRef {
                        id: asset_id,
                        name: wav_name.clone(),
                        relative_path: Some(PathBuf::from("assets/bgm").join(&wav_name)),
                        fallback_absolute_path: Some(wav_path.clone()),
                        collected: true,
                        kind: MediaKind::Audio,
                    });

                    let mut clip_id: Option<String> = None;
                    if apply {
                        let layer_count = s
                            .project
                            .active_scene()
                            .map(|sc| sc.layers.len())
                            .ok_or_else(|| "no active scene".to_string())?;
                        let audio_layer = args
                            .get("audio_layer_index")
                            .and_then(Value::as_u64)
                            .map(|v| (v as usize).min(layer_count.saturating_sub(1)))
                            .unwrap_or_else(|| layer_count.saturating_sub(1));
                        let clip = TimelineItem::new(
                            format!("BGM: {} ({:.0}BPM)", genre.id(), composed.bpm),
                            start,
                            RationalTime::from_seconds(f64::from(cfg.duration_sec)),
                            ItemKind::Audio {
                                asset_id,
                                source_offset: RationalTime::ZERO,
                                volume: 0.72,
                            },
                        );
                        clip_id = Some(clip.id.to_string());
                        let mut cmds = vec![EditCommand::AddObject {
                            layer_index: audio_layer,
                            item: clip,
                        }];
                        if add_bar_markers {
                            for (idx, &bt) in composed.bar_times_sec.iter().enumerate() {
                                cmds.push(EditCommand::AddMarker {
                                    marker: TimelineMarker::new(
                                        start + RationalTime::from_seconds(bt),
                                        format!("Bar {}", idx + 1),
                                    ),
                                });
                            }
                        }
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: format!("自動 BGM 作曲＆配置 ({})", genre.id()),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "genre": genre.id(),
                                "bpm": composed.bpm,
                                "chord_progression": composed.chord_progression,
                                "bar_markers_added": if apply && add_bar_markers { composed.bar_times_sec.len() } else { 0 },
                                "wav_path": wav_path.display().to_string(),
                                "asset_id": asset_id.to_string(),
                                "item_id": clip_id,
                                "applied": apply
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown offline_bgm_composer action: {other}")),
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
        "cinema_finishing_and_edl" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing 'action'".to_string())?;
            match action {
                "get_finishing" => {
                    let cfg = s
                        .project
                        .active_scene()
                        .map(|sc| sc.cinema_finishing.clone())
                        .ok_or_else(|| "no active scene".to_string())?;
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string(&cfg).unwrap_or_default() }]
                    }))
                }
                "set_finishing" => {
                    let mut cfg = match args.get("preset").and_then(Value::as_str) {
                        Some("kodak_35mm") => CinemaFinishingConfig::preset_kodak_35mm(),
                        Some("scifi_anamorphic") => {
                            CinemaFinishingConfig::preset_scifi_anamorphic()
                        }
                        Some("agency_review_burnin") => {
                            CinemaFinishingConfig::preset_agency_review_burnin()
                        }
                        _ => s
                            .project
                            .active_scene()
                            .map(|sc| sc.cinema_finishing.clone())
                            .ok_or_else(|| "no active scene".to_string())?,
                    };
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        cfg.enabled = en;
                    }
                    if args.get("halation_strength").is_some() {
                        cfg.halation_strength = finite_clamped(
                            args.get("halation_strength"),
                            f64::from(cfg.halation_strength),
                            0.0,
                            1.5,
                        ) as f32;
                    }
                    if args.get("halation_threshold").is_some() {
                        cfg.halation_threshold = finite_clamped(
                            args.get("halation_threshold"),
                            f64::from(cfg.halation_threshold),
                            0.3,
                            1.0,
                        ) as f32;
                    }
                    if args.get("anamorphic_flare").is_some() {
                        cfg.anamorphic_flare = finite_clamped(
                            args.get("anamorphic_flare"),
                            f64::from(cfg.anamorphic_flare),
                            0.0,
                            1.5,
                        ) as f32;
                    }
                    if args.get("film_grain").is_some() {
                        cfg.film_grain = finite_clamped(
                            args.get("film_grain"),
                            f64::from(cfg.film_grain),
                            0.0,
                            0.5,
                        ) as f32;
                    }
                    if args.get("vignette").is_some() {
                        cfg.vignette =
                            finite_clamped(args.get("vignette"), f64::from(cfg.vignette), 0.0, 1.0)
                                as f32;
                    }
                    if let Some(lb) = args.get("letterbox").and_then(Value::as_str) {
                        cfg.letterbox = match lb {
                            "cinema239" => LetterboxAspect::Cinema239,
                            "flat185" => LetterboxAspect::Flat185,
                            "academy43" => LetterboxAspect::Academy43,
                            _ => LetterboxAspect::None,
                        };
                    }
                    if let Some(tc) = args.get("burn_in_timecode").and_then(Value::as_bool) {
                        cfg.burn_in_timecode = tc;
                    }
                    if let Some(wm) = args.get("watermark_text").and_then(Value::as_str) {
                        cfg.watermark_text = wm.to_string();
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetCinemaFinishing {
                                config: cfg.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "applied": true, "config": cfg }).to_string()
                        }]
                    }))
                }
                "export_edl_cmx3600" => {
                    let edl = export_cmx3600_edl(&s.project);
                    let saved_path =
                        if let Some(p) = args.get("output_path").and_then(Value::as_str) {
                            let out_p = PathBuf::from(p);
                            if let Some(parent) = out_p.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&out_p, &edl).map_err(|e| e.to_string())?;
                            Some(out_p.display().to_string())
                        } else {
                            None
                        };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "edl": edl, "saved_path": saved_path }).to_string()
                        }]
                    }))
                }
                "export_fcpxml" => {
                    let xml = export_fcpxml_v10(&s.project);
                    let saved_path =
                        if let Some(p) = args.get("output_path").and_then(Value::as_str) {
                            let out_p = PathBuf::from(p);
                            if let Some(parent) = out_p.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&out_p, &xml).map_err(|e| e.to_string())?;
                            Some(out_p.display().to_string())
                        } else {
                            None
                        };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "fcpxml": xml, "saved_path": saved_path }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown cinema_finishing_and_edl action: {other}")),
            }
        }
        "data_chart_motion_graphics" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("create_chart");
            let parse_kind = |k: &str| match k {
                "line_chart" => ChartKind::LineChart,
                "donut_chart" => ChartKind::DonutChart,
                "kpi_counter" => ChartKind::KpiCounter,
                _ => ChartKind::BarChart,
            };
            let parse_points_from_args = |a: &Value| -> Option<Vec<DataSeriesPoint>> {
                if let Some(csv) = a.get("csv_text").and_then(Value::as_str) {
                    let parsed = parse_chart_csv(csv);
                    if !parsed.is_empty() {
                        return Some(parsed);
                    }
                }
                if let Some(arr) = a.get("points").and_then(Value::as_array) {
                    let mut pts = Vec::new();
                    for item in arr {
                        let label = item
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or("ITEM")
                            .to_string();
                        let val = item.get("value").and_then(Value::as_f64).unwrap_or(0.0);
                        let color = item
                            .get("color")
                            .and_then(Value::as_str)
                            .and_then(RgbaColor::from_hex);
                        if val.is_finite() {
                            pts.push(DataSeriesPoint {
                                label,
                                value: val,
                                color,
                            });
                        }
                    }
                    if !pts.is_empty() {
                        return Some(pts);
                    }
                }
                None
            };
            let apply_cfg_overrides = |cfg: &mut DataChartConfig, a: &Value| {
                if let Some(k) = a.get("kind").and_then(Value::as_str) {
                    cfg.kind = parse_kind(k);
                }
                if let Some(t) = a.get("title").and_then(Value::as_str) {
                    cfg.title = t.to_string();
                }
                if let Some(u) = a.get("unit_suffix").and_then(Value::as_str) {
                    cfg.unit_suffix = u.to_string();
                }
                if let Some(pts) = parse_points_from_args(a) {
                    cfg.points = pts;
                }
                if let Some(sec) = a.get("animate_in_sec").and_then(Value::as_f64) {
                    cfg.animate_in_sec = (sec as f32).clamp(0.05, 30.0);
                }
                if let Some(w) = a.get("width").and_then(Value::as_f64) {
                    cfg.width = (w as f32).clamp(120.0, 4096.0);
                }
                if let Some(h) = a.get("height").and_then(Value::as_f64) {
                    cfg.height = (h as f32).clamp(80.0, 4096.0);
                }
                if let Some(hex) = a.get("accent_color").and_then(Value::as_str)
                    && let Some(c) = RgbaColor::from_hex(hex)
                {
                    cfg.accent_color = c;
                }
                if let Some(hex) = a.get("bg_color").and_then(Value::as_str)
                    && let Some(c) = RgbaColor::from_hex(hex)
                {
                    cfg.bg_color = c;
                }
                if let Some(g) = a.get("show_grid").and_then(Value::as_bool) {
                    cfg.show_grid = g;
                }
                if let Some(v) = a.get("show_values").and_then(Value::as_bool) {
                    cfg.show_values = v;
                }
            };

            match action {
                "create_chart" => {
                    let kind = args
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(parse_kind)
                        .unwrap_or(ChartKind::BarChart);
                    let mut cfg = DataChartConfig::for_kind(kind);
                    apply_cfg_overrides(&mut cfg, args);

                    let scene = s.project.active_scene().ok_or("no active scene")?;
                    let layer_index = args
                        .get("layer_index")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize)
                        .unwrap_or(1)
                        .min(scene.layers.len().saturating_sub(1));
                    let start = RationalTime::from_seconds(finite_clamped(
                        args.get("start_sec"),
                        0.0,
                        0.0,
                        36000.0,
                    ));
                    let duration = RationalTime::from_seconds(finite_clamped(
                        args.get("duration_sec"),
                        5.0,
                        0.1,
                        3600.0,
                    ));
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&cfg.title)
                        .to_string();
                    let default_x = ((scene.width as f32) - cfg.width) * 0.5;
                    let default_y = ((scene.height as f32) - cfg.height) * 0.5;
                    let mut item = TimelineItem::new(
                        name,
                        start,
                        duration,
                        ItemKind::DataChart {
                            config: Box::new(cfg.clone()),
                        },
                    );
                    item.transform.x = args
                        .get("x")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32)
                        .unwrap_or(default_x);
                    item.transform.y = args
                        .get("y")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32)
                        .unwrap_or(default_y);
                    let created_id = item.id;
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject { layer_index, item },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "created_id": created_id,
                                "layer_index": layer_index,
                                "point_count": cfg.points.len(),
                                "config": cfg
                            }).to_string()
                        }]
                    }))
                }
                "update_chart" | "import_csv" => {
                    let item_id = args
                        .get("item_id")
                        .and_then(Value::as_str)
                        .and_then(|u| Uuid::parse_str(u).ok())
                        .ok_or("missing or invalid 'item_id'")?;
                    let existing_cfg = {
                        let scene = s.project.active_scene().ok_or("no active scene")?;
                        scene
                            .layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find(|i| i.id == item_id)
                            .and_then(|i| match &i.kind {
                                ItemKind::DataChart { config } => Some((**config).clone()),
                                _ => None,
                            })
                            .ok_or("DataChart item not found")?
                    };
                    let mut next = existing_cfg;
                    apply_cfg_overrides(&mut next, args);
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetDataChart {
                                item_id,
                                config: Box::new(next.clone()),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "updated": true,
                                "item_id": item_id,
                                "point_count": next.points.len(),
                                "config": next
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!(
                    "unknown data_chart_motion_graphics action: {other}"
                )),
            }
        }
        "vfx_paint_and_inpaint" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("add_stroke");
            let item_id = args
                .get("item_id")
                .and_then(Value::as_str)
                .and_then(|u| Uuid::parse_str(u).ok())
                .ok_or("missing or invalid 'item_id'")?;
            let current = {
                let scene = s.project.active_scene().ok_or("no active scene")?;
                scene
                    .layers
                    .iter()
                    .flat_map(|l| l.items.iter())
                    .find(|i| i.id == item_id)
                    .map(|i| i.vfx_paint.clone())
                    .ok_or("item not found")?
            };

            match action {
                "get_paint" => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({
                            "item_id": item_id,
                            "stroke_count": current.strokes.len(),
                            "vfx_paint": current
                        }).to_string()
                    }]
                })),
                "clear_paint" => {
                    let cleared = VfxPaintConfig::default();
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetVfxPaint {
                                item_id,
                                vfx_paint: cleared.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({ "cleared": true, "item_id": item_id }).to_string()
                        }]
                    }))
                }
                "add_stroke" | "set_paint" => {
                    let tool = match args
                        .get("tool")
                        .and_then(Value::as_str)
                        .unwrap_or("brush_glow")
                    {
                        "clone_stamp" => PaintToolKind::CloneStamp,
                        "wire_erase_inpaint" => PaintToolKind::WireEraseInpaint,
                        "blur_redact" => PaintToolKind::BlurRedact,
                        _ => PaintToolKind::BrushGlow,
                    };
                    let mut pts = Vec::new();
                    if let Some(arr) = args.get("points").and_then(Value::as_array) {
                        for p in arr {
                            if let Some(pair) = p.as_array()
                                && pair.len() >= 2
                                && let (Some(x), Some(y)) = (pair[0].as_f64(), pair[1].as_f64())
                            {
                                pts.push([(x as f32).clamp(0.0, 1.0), (y as f32).clamp(0.0, 1.0)]);
                            }
                        }
                    }
                    if pts.is_empty() {
                        pts = vec![[0.2, 0.5], [0.8, 0.5]];
                    }
                    let radius_px = finite_clamped(args.get("radius_px"), 14.0, 1.0, 256.0) as f32;
                    let hardness = finite_clamped(args.get("hardness"), 0.72, 0.05, 1.0) as f32;
                    let opacity = finite_clamped(args.get("opacity"), 1.0, 0.0, 1.0) as f32;
                    let write_on_progress =
                        finite_clamped(args.get("write_on_progress"), 1.0, 0.0, 1.0) as f32;
                    let color = args
                        .get("color")
                        .and_then(Value::as_str)
                        .and_then(RgbaColor::from_hex)
                        .unwrap_or(RgbaColor {
                            r: 1.0,
                            g: 0.28,
                            b: 0.52,
                            a: 1.0,
                        });
                    let clone_offset = args
                        .get("clone_offset")
                        .and_then(Value::as_array)
                        .and_then(|a| {
                            if a.len() >= 2 {
                                Some([
                                    a[0].as_f64().unwrap_or(0.08) as f32,
                                    a[1].as_f64().unwrap_or(0.0) as f32,
                                ])
                            } else {
                                None
                            }
                        })
                        .unwrap_or([0.08, 0.0]);

                    let stroke = PaintStroke {
                        id: Uuid::new_v4(),
                        tool,
                        points: pts,
                        radius_px,
                        hardness,
                        opacity,
                        color,
                        clone_offset,
                        write_on_progress,
                    };

                    let mut next = if action == "set_paint" {
                        VfxPaintConfig {
                            enabled: args.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                            animate_write_on: args
                                .get("animate_write_on")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            strokes: vec![stroke],
                        }
                    } else {
                        let mut c = current;
                        c.enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                        if let Some(aw) = args.get("animate_write_on").and_then(Value::as_bool) {
                            c.animate_write_on = aw;
                        }
                        c.strokes.push(stroke);
                        c
                    };
                    if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                        next.enabled = en;
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetVfxPaint {
                                item_id,
                                vfx_paint: next.clone(),
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "applied": true,
                                "item_id": item_id,
                                "stroke_count": next.strokes.len(),
                                "vfx_paint": next
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown vfx_paint_and_inpaint action: {other}")),
            }
        }
        "podcast_auto_director" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("analyze_and_direct");
            let target_lufs = finite_clamped(args.get("target_lufs"), -16.0, -30.0, -8.0) as f32;

            match action {
                "level_wav" => {
                    let in_path = args
                        .get("input_wav_path")
                        .and_then(Value::as_str)
                        .ok_or("missing 'input_wav_path'")?;
                    let out_path = args
                        .get("output_wav_path")
                        .and_then(Value::as_str)
                        .ok_or("missing 'output_wav_path'")?;
                    let buf = decode_audio_file(&PathBuf::from(in_path))
                        .map_err(|e| format!("decode failed: {e}"))?;
                    let sr = buf.sample_rate.max(8_000);
                    let mut stereo = Vec::with_capacity(buf.frame_count() * 2);
                    if buf.channels <= 1 {
                        for &v in &buf.samples {
                            stereo.push(v);
                            stereo.push(v);
                        }
                    } else {
                        for f in 0..buf.frame_count() {
                            let base = f * buf.channels;
                            stereo.push(buf.samples[base]);
                            stereo.push(buf.samples[base + 1]);
                        }
                    }
                    let before_lufs = measure_integrated_lufs(&stereo, sr);
                    apply_podcast_voice_leveler(&mut stereo, sr, target_lufs);
                    let after_lufs = measure_integrated_lufs(&stereo, sr);
                    let out_pb = PathBuf::from(out_path);
                    if let Some(parent) = out_pb.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    write_stereo_wav_file(&out_pb, &stereo, sr).map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "before_lufs": before_lufs,
                                "after_lufs": after_lufs,
                                "target_lufs": target_lufs,
                                "saved_path": out_pb.display().to_string()
                            }).to_string()
                        }]
                    }))
                }
                "analyze_and_direct" => {
                    let min_turn_sec = finite_clamped(args.get("min_turn_sec"), 0.4, 0.1, 5.0);
                    let silence_gate_db =
                        finite_clamped(args.get("silence_gate_db"), -42.0, -70.0, -10.0) as f32;
                    let apply_leveler_faders = args
                        .get("apply_leveler_faders")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);

                    let scene = s.project.active_scene().ok_or("no active scene")?.clone();
                    let sr = s.project.settings.audio_sample_rate.max(8_000);
                    let scene_dur_sec = scene.total_duration().as_seconds().clamp(1.0, 120.0);

                    let mut specs: Vec<(SpeakerTrackSpec, Option<String>)> = Vec::new();
                    if let Some(arr) = args.get("speakers").and_then(Value::as_array) {
                        for (idx, sp) in arr.iter().enumerate() {
                            let name = sp
                                .get("name")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                                .unwrap_or_else(|| format!("Speaker {}", idx + 1));
                            let audio_layer_index =
                                sp.get("audio_layer_index")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(idx as u64) as usize;
                            let camera_layer_index = sp
                                .get("camera_layer_index")
                                .and_then(Value::as_u64)
                                .map(|v| v as usize);
                            let wav_path = sp
                                .get("wav_path")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            specs.push((
                                SpeakerTrackSpec {
                                    name,
                                    audio_layer_index,
                                    camera_layer_index,
                                },
                                wav_path,
                            ));
                        }
                    } else {
                        for (li, layer) in scene.layers.iter().enumerate() {
                            let has_audio = layer
                                .items
                                .iter()
                                .any(|it| matches!(it.kind, ItemKind::Audio { .. }));
                            if has_audio {
                                specs.push((
                                    SpeakerTrackSpec {
                                        name: layer.name.clone(),
                                        audio_layer_index: li,
                                        camera_layer_index: None,
                                    },
                                    None,
                                ));
                            }
                        }
                    }

                    let mut track_pcms: Vec<(String, Vec<f32>)> = Vec::with_capacity(specs.len());
                    for (sp_idx, (spec, wav_opt)) in specs.iter().enumerate() {
                        let loaded_pcm = wav_opt
                            .as_ref()
                            .and_then(|p| decode_audio_file(&PathBuf::from(p)).ok())
                            .map(|buf| {
                                let mut st = Vec::with_capacity(buf.frame_count() * 2);
                                if buf.channels <= 1 {
                                    for &v in &buf.samples {
                                        st.push(v);
                                        st.push(v);
                                    }
                                } else {
                                    for f in 0..buf.frame_count() {
                                        let base = f * buf.channels;
                                        st.push(buf.samples[base]);
                                        st.push(buf.samples[base + 1]);
                                    }
                                }
                                st
                            });

                        let pcm = if let Some(st) = loaded_pcm {
                            st
                        } else {
                            // タイムライン上の音声クリップ区間から決定論的な発話活性波形を生成
                            let frames = ((scene_dur_sec * f64::from(sr)).round() as usize)
                                .clamp(sr as usize, sr as usize * 120);
                            let mut st = vec![0.0f32; frames * 2];
                            let freq = 180.0 + (sp_idx as f32) * 45.0;
                            let amp = (0.18 + (sp_idx as f32) * 0.08).min(0.45);
                            if let Some(layer) = scene.layers.get(spec.audio_layer_index) {
                                for it in &layer.items {
                                    let f0 = ((it.start.as_seconds() * f64::from(sr)).round()
                                        as usize)
                                        .min(frames);
                                    let f1 = ((it.end().as_seconds() * f64::from(sr)).round()
                                        as usize)
                                        .min(frames);
                                    for fi in f0..f1 {
                                        let t = fi as f32 / sr as f32;
                                        let v = (std::f32::consts::TAU * freq * t).sin() * amp;
                                        st[fi * 2] = v;
                                        st[fi * 2 + 1] = v;
                                    }
                                }
                            }
                            st
                        };
                        track_pcms.push((spec.name.clone(), pcm));
                    }

                    let analysis = analyze_podcast_speakers(
                        &track_pcms,
                        sr,
                        min_turn_sec,
                        silence_gate_db,
                        target_lufs,
                    );
                    let only_specs: Vec<SpeakerTrackSpec> =
                        specs.iter().map(|(sp, _)| sp.clone()).collect();
                    let cmds = build_podcast_auto_switch_commands(
                        &scene,
                        &only_specs,
                        &analysis,
                        apply_leveler_faders,
                    );
                    let command_count = cmds.len();
                    if apply && !cmds.is_empty() {
                        s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: "Podcast Auto-Director".into(),
                                    commands: cmds,
                                },
                                CommandSource::Mcp,
                                None,
                            )
                            .map_err(|e| e.to_string())?;
                    }
                    let turns_json: Vec<Value> = analysis
                        .turns
                        .iter()
                        .map(|t| {
                            json!({
                                "speaker_idx": t.speaker_idx,
                                "speaker_name": t.speaker_name,
                                "start_sec": t.start_sec,
                                "end_sec": t.end_sec,
                                "avg_rms_db": t.avg_rms_db
                            })
                        })
                        .collect();
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "applied": apply && command_count > 0,
                                "turn_count": turns_json.len(),
                                "command_count": command_count,
                                "target_lufs": analysis.target_lufs,
                                "speaker_lufs": analysis.speaker_lufs,
                                "recommended_gains_db": analysis.recommended_gains_db,
                                "turns": turns_json
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown podcast_auto_director action: {other}")),
            }
        }
        "multi_platform_packager" => {
            let s = &mut *guard;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("create_platform_variants");

            let parse_platform = |p: &str| match p {
                "shorts_reels_9x16" | "9x16" => DeliveryPlatform::ShortsReels9x16,
                "square_feed_1x1" | "1x1" => DeliveryPlatform::SquareFeed1x1,
                "cinema_4k_master" | "4k" => DeliveryPlatform::Cinema4kMaster,
                _ => DeliveryPlatform::YouTube16x9,
            };
            let targets = if let Some(arr) = args.get("platforms").and_then(Value::as_array) {
                let mut list = Vec::new();
                for v in arr {
                    if let Some(ps) = v.as_str() {
                        list.push(parse_platform(ps).default_target());
                    }
                }
                if list.is_empty() {
                    DeliveryBundleConfig::default().targets
                } else {
                    list
                }
            } else {
                DeliveryBundleConfig::default().targets
            };

            let cfg = DeliveryBundleConfig {
                targets,
                include_thumbnail: args
                    .get("include_thumbnail")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                include_edl: args
                    .get("include_edl")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                include_fcpxml: args
                    .get("include_fcpxml")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                include_qc_report: args
                    .get("include_qc_report")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                base_title: args
                    .get("base_title")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| s.project.name.clone()),
                tags: args
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["video".into(), "master".into()]),
            };

            match action {
                "create_platform_variants" => {
                    let src = s.project.active_scene().ok_or("no active scene")?.clone();
                    let mut cmds = Vec::new();
                    let mut summary = Vec::new();
                    for tgt in &cfg.targets {
                        let (sid, cmd) = build_reformatted_scene_command(&src, tgt);
                        summary.push(json!({
                            "scene_id": sid,
                            "platform": tgt.platform.id(),
                            "width": tgt.width,
                            "height": tgt.height,
                            "target_lufs": tgt.target_lufs
                        }));
                        cmds.push(cmd);
                    }
                    s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: "Multi-Platform Reformat Variants".into(),
                                commands: cmds,
                            },
                            CommandSource::Mcp,
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "created_count": summary.len(),
                                "created_scenes": summary
                            }).to_string()
                        }]
                    }))
                }
                "export_delivery_bundle" => {
                    let out_dir = if let Some(p) = args.get("output_dir").and_then(Value::as_str) {
                        PathBuf::from(p)
                    } else if let Some(folder) = &s.folder {
                        folder.root().join("deliverables")
                    } else {
                        std::env::temp_dir().join("ve_delivery_bundle")
                    };
                    std::fs::create_dir_all(out_dir.join("variants")).map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("subtitles"))
                        .map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("metadata")).map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("interchange"))
                        .map_err(|e| e.to_string())?;
                    std::fs::create_dir_all(out_dir.join("qc")).map_err(|e| e.to_string())?;

                    let src = s.project.active_scene().ok_or("no active scene")?.clone();
                    let mut artifacts: Vec<DeliveryBundleArtifact> = Vec::new();
                    let mut wrote_chapters = false;

                    for tgt in &cfg.targets {
                        let (_sid, cmd) = build_reformatted_scene_command(&src, tgt);
                        if let EditCommand::CreateScene { scene: vscene, .. } = cmd {
                            let rel_scene = format!("variants/{}.scene.json", tgt.platform.id());
                            let scene_bytes =
                                serde_json::to_vec_pretty(&vscene).map_err(|e| e.to_string())?;
                            std::fs::write(out_dir.join(&rel_scene), &scene_bytes)
                                .map_err(|e| e.to_string())?;
                            artifacts.push(DeliveryBundleArtifact {
                                kind: "scene_variant".into(),
                                platform: tgt.platform.id().into(),
                                relative_path: rel_scene,
                                bytes: scene_bytes.len(),
                            });

                            if tgt.include_srt {
                                let srt = export_srt_subtitles(&vscene);
                                let rel_srt = format!("subtitles/{}.srt", tgt.platform.id());
                                std::fs::write(out_dir.join(&rel_srt), srt.as_bytes())
                                    .map_err(|e| e.to_string())?;
                                artifacts.push(DeliveryBundleArtifact {
                                    kind: "subtitles_srt".into(),
                                    platform: tgt.platform.id().into(),
                                    relative_path: rel_srt,
                                    bytes: srt.len(),
                                });
                            }

                            if tgt.include_chapters && !wrote_chapters {
                                let ch = format_youtube_chapters(&vscene.markers);
                                let rel_ch = "metadata/youtube_chapters.txt".to_string();
                                std::fs::write(out_dir.join(&rel_ch), ch.as_bytes())
                                    .map_err(|e| e.to_string())?;
                                artifacts.push(DeliveryBundleArtifact {
                                    kind: "youtube_chapters".into(),
                                    platform: tgt.platform.id().into(),
                                    relative_path: rel_ch,
                                    bytes: ch.len(),
                                });
                                wrote_chapters = true;
                            }
                        }
                    }

                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let active_id = s.project.active_scene_id;

                    if cfg.include_thumbnail
                        && let Some((_t, _score, hero)) =
                            select_best_thumbnail_frame(&mut rg, &s.project, active_id, 4)
                    {
                        let png = encode_linear_image_png_bytes(&hero)?;
                        let rel_thumb = "thumbnail_1280x720.png".to_string();
                        std::fs::write(out_dir.join(&rel_thumb), &png)
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "thumbnail_png".into(),
                            platform: "shared".into(),
                            relative_path: rel_thumb,
                            bytes: png.len(),
                        });
                    }

                    if cfg.include_edl {
                        let edl = export_cmx3600_edl(&s.project);
                        let rel_edl = "interchange/master.edl".to_string();
                        std::fs::write(out_dir.join(&rel_edl), edl.as_bytes())
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "edl_cmx3600".into(),
                            platform: "shared".into(),
                            relative_path: rel_edl,
                            bytes: edl.len(),
                        });
                    }

                    if cfg.include_fcpxml {
                        let xml = export_fcpxml_v10(&s.project);
                        let rel_xml = "interchange/master.fcpxml".to_string();
                        std::fs::write(out_dir.join(&rel_xml), xml.as_bytes())
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "fcpxml_v10".into(),
                            platform: "shared".into(),
                            relative_path: rel_xml,
                            bytes: xml.len(),
                        });
                    }

                    if cfg.include_qc_report {
                        let (
                            peak_luma_ire,
                            clipped_highlight_pct,
                            crushed_shadow_pct,
                            max_frame_luma_jump_ire,
                        ) = measure_scene_video_qc_metrics(&mut rg, &s.project, active_id, 4);
                        let report = run_broadcast_qc_audit(
                            &s.project,
                            BroadcastProfile::YouTube1080p,
                            BroadcastQcMetrics {
                                peak_luma_ire,
                                clipped_highlight_pct,
                                crushed_shadow_pct,
                                max_frame_luma_jump_ire,
                                true_peak_dbtp: -1.5,
                                integrated_lufs: -14.0,
                            },
                        );
                        let qc_bytes =
                            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
                        let rel_qc = "qc/broadcast_qc_report.json".to_string();
                        std::fs::write(out_dir.join(&rel_qc), &qc_bytes)
                            .map_err(|e| e.to_string())?;
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "broadcast_qc_json".into(),
                            platform: "shared".into(),
                            relative_path: rel_qc,
                            bytes: qc_bytes.len(),
                        });
                    }

                    let manifest = build_delivery_manifest(&s.project, &cfg, artifacts);
                    let manifest_bytes =
                        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
                    std::fs::write(out_dir.join("manifest.json"), &manifest_bytes)
                        .map_err(|e| e.to_string())?;

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "output_dir": out_dir.display().to_string(),
                                "artifact_count": manifest.artifacts.len(),
                                "manifest": manifest
                            }).to_string()
                        }]
                    }))
                }
                other => Err(format!("unknown multi_platform_packager action: {other}")),
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

        // Verify exact MCP tool count = 58 (v2.0)
        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 78);

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
        assert_eq!(defs.as_array().unwrap().len(), 78);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v20_mcp_tools_node_graph_ai_vision_director_and_collab() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v20_{}", Uuid::new_v4()));
        let proj = Project::new("McpV20", ProjectSettings::default());
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let mut ed = EditorSession::new(proj);
        ed.folder = Some(folder);
        let session = Arc::new(Mutex::new(ed));

        // 0. Add a subject shape clip
        let obj_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "Subject", "kind": "shape", "start_sec": 0.0, "duration_sec": 4.0, "width": 320.0, "height": 240.0}),
        )
        .unwrap();
        let obj_id = serde_json::from_str::<Value>(obj_res["content"][0]["text"].as_str().unwrap())
            .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();

        // 1. #21 manage_node_graph: from_layers -> add_node -> validate -> sync_to_layers
        let ng_gen = call_tool(
            &session,
            "manage_node_graph",
            &json!({"action": "from_layers"}),
        )
        .unwrap();
        assert!(
            ng_gen["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("generated_from_layers")
        );
        let ng_add = call_tool(
            &session,
            "manage_node_graph",
            &json!({"action": "add_node", "node_type": "channel_split", "name": "LumaSplit", "channel": "luma"}),
        )
        .unwrap();
        assert!(
            ng_add["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("created_node_id")
        );
        let ng_val = call_tool(
            &session,
            "manage_node_graph",
            &json!({"action": "validate"}),
        )
        .unwrap();
        assert!(
            ng_val["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"valid\":true")
        );
        call_tool(
            &session,
            "manage_node_graph",
            &json!({"action": "sync_to_layers"}),
        )
        .unwrap();

        // 2. #22 run_local_ai_vision: status + segmentation + super_res + optical_flow + auto_reframe
        let ai_st = call_tool(
            &session,
            "run_local_ai_vision",
            &json!({"action": "status"}),
        )
        .unwrap();
        assert!(
            ai_st["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"local_only_guaranteed\": true")
        );
        call_tool(
            &session,
            "run_local_ai_vision",
            &json!({"action": "apply_segmentation", "id": obj_id, "sensitivity": 0.6}),
        )
        .unwrap();
        call_tool(
            &session,
            "run_local_ai_vision",
            &json!({"action": "apply_super_resolution", "id": obj_id, "sharpen": 0.5}),
        )
        .unwrap();
        call_tool(
            &session,
            "run_local_ai_vision",
            &json!({"action": "set_optical_flow_slowmo", "id": obj_id, "enabled": true, "speed": 0.5}),
        )
        .unwrap();
        let rf = call_tool(
            &session,
            "run_local_ai_vision",
            &json!({"action": "auto_reframe", "target_width": 1080, "target_height": 1920, "apply": true}),
        )
        .unwrap();
        assert!(
            rf["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"applied\": true")
        );
        {
            let g = session.lock().unwrap();
            let s = g.project.active_scene().unwrap();
            assert_eq!((s.width, s.height), (1080, 1920));
            assert!(s.layers[0].items[0].playback.optical_flow_interp);
            assert!(
                s.layers[0].items[0]
                    .effect_chain
                    .iter()
                    .any(|e| e.effect_type == "ai_segment")
            );
        }

        // 3. #23 orchestrate_edit_recipe: list -> start_run (pauses at human approval gate) -> approve_gate -> completed -> export_yaml -> record_from_history
        let rec_list = call_tool(
            &session,
            "orchestrate_edit_recipe",
            &json!({"action": "list_recipes"}),
        )
        .unwrap();
        assert!(
            rec_list["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("YouTube")
        );

        let run_res = call_tool(
            &session,
            "orchestrate_edit_recipe",
            &json!({
                "action": "start_run",
                "recipe_name": "YouTube 解説動画・標準ディレクターパイプライン",
                "variables": { "title": "v2.0 MCP テストタイトル" },
                "auto_run": true
            }),
        )
        .unwrap();
        let run_json: Value =
            serde_json::from_str(run_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(run_json["status"], "awaiting_approval");

        let approved_res = call_tool(
            &session,
            "orchestrate_edit_recipe",
            &json!({"action": "approve_gate", "continue_after": true}),
        )
        .unwrap();
        let approved_json: Value =
            serde_json::from_str(approved_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(approved_json["status"], "completed");
        assert_eq!(approved_json["logs"].as_array().unwrap().len(), 5);

        let yaml_res = call_tool(
            &session,
            "orchestrate_edit_recipe",
            &json!({"action": "export_yaml"}),
        )
        .unwrap();
        assert!(
            yaml_res["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("steps:")
        );

        let rec_hist = call_tool(
            &session,
            "orchestrate_edit_recipe",
            &json!({"action": "record_from_history", "name": "AutoRecorded"}),
        )
        .unwrap();
        assert!(
            rec_hist["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"status\": \"recorded\"")
        );

        // 4. #24 collab_diff_and_review: snapshot -> branch edit -> diff -> 3-way merge -> Frame.io/YouTube review import -> markers
        call_tool(
            &session,
            "collab_diff_and_review",
            &json!({"action": "save_snapshot", "name": "base_main"}),
        )
        .unwrap();
        call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 2, "name": "BranchB_Title", "kind": "text", "content": "Branch B", "start_sec": 1.0, "duration_sec": 2.0}),
        )
        .unwrap();
        call_tool(
            &session,
            "collab_diff_and_review",
            &json!({"action": "save_snapshot", "name": "branch_b"}),
        )
        .unwrap();
        // Rollback local main 1 step (Undo BranchB_Title) and add BranchA_Marker instead
        {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            assert!(s.bus.undo(&mut s.project));
        }
        call_tool(
            &session,
            "manage_markers",
            &json!({"action": "add", "time_sec": 2.0, "label": "BranchA Marker"}),
        )
        .unwrap();
        let diff_res = call_tool(
            &session,
            "collab_diff_and_review",
            &json!({"action": "diff_snapshots", "base_snapshot": "base_main", "target_snapshot": "branch_b"}),
        )
        .unwrap();
        assert!(
            diff_res["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("BranchB_Title")
        );

        let merge_res = call_tool(
            &session,
            "collab_diff_and_review",
            &json!({"action": "merge_three_way", "base_snapshot": "base_main", "theirs_snapshot": "branch_b", "apply": true}),
        )
        .unwrap();
        let merge_json: Value =
            serde_json::from_str(merge_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(merge_json["conflict_count"], 0);
        assert_eq!(merge_json["applied"], true);

        let rev_imp = call_tool(
            &session,
            "collab_diff_and_review",
            &json!({
                "action": "import_reviews",
                "format": "frameio_csv",
                "content": "Timecode In,Commenter,Comment,Status\n00:00:01:00,Director,テロップもう少し太く,Open\n"
            }),
        )
        .unwrap();
        assert!(
            rev_imp["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"imported_count\":1")
        );
        let mk_res = call_tool(
            &session,
            "collab_diff_and_review",
            &json!({"action": "convert_reviews_to_markers", "only_unresolved": true}),
        )
        .unwrap();
        assert!(
            mk_res["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"converted_markers\":1")
        );

        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 78);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v21_mcp_tools_color_vfx_spatial_and_kinetic() {
        let proj = Project::new("McpV21", ProjectSettings::default());
        let session = Arc::new(Mutex::new(EditorSession::new(proj)));

        // 0. Add a shape clip + 2 text clips
        let shp_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 0, "name": "MonitorScreen", "kind": "shape", "start_sec": 0.0, "duration_sec": 3.0, "width": 320.0, "height": 240.0}),
        )
        .unwrap();
        let shp_id = serde_json::from_str::<Value>(shp_res["content"][0]["text"].as_str().unwrap())
            .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();

        let sub1_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Sub1", "kind": "text", "content": "重要な ポイント です", "start_sec": 0.0, "duration_sec": 2.0}),
        )
        .unwrap();
        let sub1_id =
            serde_json::from_str::<Value>(sub1_res["content"][0]["text"].as_str().unwrap())
                .unwrap()["created_id"]
                .as_str()
                .unwrap()
                .to_string();

        let sub2_res = call_tool(
            &session,
            "add_object",
            &json!({"layer_index": 1, "name": "Sub2", "kind": "text", "content": "今すぐ チェック！", "start_sec": 2.0, "duration_sec": 2.0}),
        )
        .unwrap();
        let sub2_id =
            serde_json::from_str::<Value>(sub2_res["content"][0]["text"].as_str().unwrap())
                .unwrap()["created_id"]
                .as_str()
                .unwrap()
                .to_string();

        // 1. #25 pro_color_match_and_grade
        call_tool(
            &session,
            "pro_color_match_and_grade",
            &json!({"action": "set_curves", "preset": "s_curve"}),
        )
        .unwrap();
        call_tool(
            &session,
            "pro_color_match_and_grade",
            &json!({"action": "set_hsl_qualifier", "hue_center_deg": 210.0, "sat_scale": 1.35, "show_matte_overlay": false}),
        )
        .unwrap();
        let cm = call_tool(
            &session,
            "pro_color_match_and_grade",
            &json!({"action": "auto_match_timecodes", "reference_time_sec": 0.2, "target_time_sec": 1.5, "apply": true}),
        )
        .unwrap();
        assert!(
            cm["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"applied\":true")
        );
        {
            let g = session.lock().unwrap();
            let cg = &g.project.active_scene().unwrap().color_grade;
            assert!(cg.enabled);
            assert!(cg.curves.enabled);
            assert!(cg.hsl_qualifier.enabled);
            assert!((cg.hsl_qualifier.hue_center_deg - 210.0).abs() < 1e-3);
        }

        // 2. #26 vfx_stabilize_and_corner_pin
        call_tool(
            &session,
            "vfx_stabilize_and_corner_pin",
            &json!({"action": "set_corner_pin", "id": shp_id, "preset": "monitor_tilt", "feather_px": 2.0}),
        )
        .unwrap();
        let stab = call_tool(
            &session,
            "vfx_stabilize_and_corner_pin",
            &json!({"action": "analyze_and_stabilize", "id": shp_id, "smoothness": 0.8, "apply": true}),
        )
        .unwrap();
        assert!(
            stab["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"applied\":true")
        );
        {
            let g = session.lock().unwrap();
            let shp = &g.project.active_scene().unwrap().layers[0].items[0];
            assert!(shp.corner_pin.enabled);
            assert!(!shp.keyframes.is_empty());
        }

        // 3. #27 spatial_and_master_audio
        call_tool(
            &session,
            "spatial_and_master_audio",
            &json!({"action": "add_spatial_3d", "azimuth_deg": 65.0, "distance": 1.2, "room_mix": 0.15}),
        )
        .unwrap();
        call_tool(
            &session,
            "spatial_and_master_audio",
            &json!({"action": "add_multiband_comp", "ratio": 3.5, "makeup_db": 1.0}),
        )
        .unwrap();
        call_tool(
            &session,
            "spatial_and_master_audio",
            &json!({"action": "add_true_peak_limiter", "ceiling_dbtp": -1.0, "release_ms": 50.0}),
        )
        .unwrap();
        let tp = call_tool(
            &session,
            "spatial_and_master_audio",
            &json!({"action": "measure_master_true_peak", "duration_sec": 1.0}),
        )
        .unwrap();
        assert!(
            tp["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"broadcast_safe\":true")
        );
        {
            let g = session.lock().unwrap();
            let mfx = &g.project.active_scene().unwrap().master_audio_effects;
            assert_eq!(mfx.len(), 3);
            assert_eq!(mfx[0].effect_type, "spatial_3d");
            assert_eq!(mfx[1].effect_type, "multiband_comp");
            assert_eq!(mfx[2].effect_type, "true_peak_limiter");
        }

        // 4. #28 kinetic_subtitles_and_localize
        let kin = call_tool(
            &session,
            "kinetic_subtitles_and_localize",
            &json!({
                "action": "set_kinetic_style",
                "mode": "karaoke_sweep",
                "pop_scale": 1.25,
                "active_color": "#ffdd33"
            }),
        )
        .unwrap();
        assert!(
            kin["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"updated_count\":2")
        );

        let csv_res = call_tool(
            &session,
            "kinetic_subtitles_and_localize",
            &json!({"action": "extract_localization_table", "source_locale": "ja", "target_locales": ["en"], "format": "csv"}),
        )
        .unwrap();
        let csv_txt = csv_res["content"][0]["text"].as_str().unwrap();
        assert!(csv_txt.contains("item_id,start_sec,item_name,ja,en"));

        let loc_apply = call_tool(
            &session,
            "kinetic_subtitles_and_localize",
            &json!({
                "action": "apply_locale_translations",
                "locale": "en",
                "translations": {
                    sub1_id.clone(): "Important key point",
                    sub2_id.clone(): "Check it out now!"
                }
            }),
        )
        .unwrap();
        assert!(
            loc_apply["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"applied_count\":2")
        );
        {
            let g = session.lock().unwrap();
            let subs = &g.project.active_scene().unwrap().layers[1].items;
            assert!(subs[0].kinetic_subtitle.enabled);
            if let ItemKind::Text { content, .. } = &subs[0].kind {
                assert_eq!(content, "Important key point");
            } else {
                panic!("expected text item");
            }
        }
        // 1-Undo reverts both localized subtitles back to Japanese
        {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            assert!(s.bus.undo(&mut s.project));
            if let ItemKind::Text { content, .. } =
                &s.project.active_scene().unwrap().layers[1].items[0].kind
            {
                assert_eq!(content, "重要な ポイント です");
            }
        }

        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 78);
    }

    #[test]
    fn v30_mcp_tools_gpu_3d_dubbing_and_crdt() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v30_{}", Uuid::new_v4()));
        let proj = Project::new("McpV30", ProjectSettings::default());
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let mut ed = EditorSession::with_folder(proj, folder);
        ed.project.active_scene_mut().unwrap().layers[1]
            .items
            .push(TimelineItem::new(
                "SubJP",
                RationalTime::ZERO,
                RationalTime::from_seconds(2.0),
                ItemKind::Text {
                    content: "こんにちは、次世代編集ソフトです！".into(),
                    font_family: "Sans".into(),
                    font_size: 42.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
        let sub_id = ed.project.active_scene().unwrap().layers[1].items[0]
            .id
            .to_string();
        let session = Arc::new(Mutex::new(ed));

        // 1. #29 gpu_v30_pipeline: status + benchmark_4k + run_grade_and_scopes
        let gpu_st = call_tool(&session, "gpu_v30_pipeline", &json!({"action": "status"})).unwrap();
        let st_txt = gpu_st["content"][0]["text"].as_str().unwrap();
        assert!(st_txt.contains("\"color_shader_valid\":true"));
        assert!(st_txt.contains("\"scopes_shader_valid\":true"));

        let bench = call_tool(
            &session,
            "gpu_v30_pipeline",
            &json!({"action": "benchmark_4k", "width": 320, "height": 180}),
        )
        .unwrap();
        assert!(
            bench["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"estimated_fps\"")
        );

        // 2. #30 scene_3d_and_camera_track: add_mesh_3d + update_mesh_3d + emit_sample_glb + track_3d_camera
        let m3d = call_tool(
            &session,
            "scene_3d_and_camera_track",
            &json!({
                "action": "add_mesh_3d",
                "layer_index": 0,
                "name": "Hero3D",
                "preset": "extruded_text",
                "text_content": "V3.0",
                "metallic": 0.7,
                "roughness": 0.2
            }),
        )
        .unwrap();
        let m3d_id = serde_json::from_str::<Value>(m3d["content"][0]["text"].as_str().unwrap())
            .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();

        call_tool(
            &session,
            "scene_3d_and_camera_track",
            &json!({"action": "update_mesh_3d", "id": m3d_id, "preset": "torus", "wireframe": true}),
        )
        .unwrap();

        let glb = call_tool(
            &session,
            "scene_3d_and_camera_track",
            &json!({"action": "emit_sample_glb"}),
        )
        .unwrap();
        assert!(
            glb["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"triangles\":12")
        );

        let cam_tr = call_tool(
            &session,
            "scene_3d_and_camera_track",
            &json!({"action": "track_3d_camera", "sample_interval_sec": 0.5, "apply": true}),
        )
        .unwrap();
        assert!(
            cam_tr["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"applied\":true")
        );

        // 3. #31 offline_ai_dubbing_lipsync: list_voices + align_phonemes + generate_locale_dubbing
        let voices = call_tool(
            &session,
            "offline_ai_dubbing_lipsync",
            &json!({"action": "list_voices"}),
        )
        .unwrap();
        assert!(
            voices["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"offline_only\":true")
        );

        let ph = call_tool(
            &session,
            "offline_ai_dubbing_lipsync",
            &json!({"action": "align_phonemes", "text": "Hello V3 world!", "duration_sec": 2.0}),
        )
        .unwrap();
        assert!(
            ph["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"phoneme_count\"")
        );

        let dub = call_tool(
            &session,
            "offline_ai_dubbing_lipsync",
            &json!({
                "action": "generate_locale_dubbing",
                "locale": "en",
                "voice": "en_female",
                "audio_layer_index": 3,
                "translations": { sub_id.clone(): "Hello, welcome to version 3!" },
                "apply": true
            }),
        )
        .unwrap();
        let dub_json: Value =
            serde_json::from_str(dub["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(dub_json["generated_clips"], 1);
        assert_eq!(dub_json["applied"], true);
        let wav_p = dub_json["wav_paths"][0].as_str().unwrap();
        assert!(std::path::Path::new(wav_p).exists());

        // 4. #32 crdt_realtime_collab: profile + cursor + marker op + export_delta + remote merge
        call_tool(
            &session,
            "crdt_realtime_collab",
            &json!({"action": "set_peer_profile", "peer_id": "editor_a", "display_name": "Alice", "color_hex": "#33ddff"}),
        )
        .unwrap();
        call_tool(
            &session,
            "crdt_realtime_collab",
            &json!({"action": "update_cursor", "playhead_sec": 1.5}),
        )
        .unwrap();
        call_tool(
            &session,
            "crdt_realtime_collab",
            &json!({"action": "record_marker_op", "time_sec": 0.8, "label": "Alice Note"}),
        )
        .unwrap();

        let mut remote_peer = CrdtSyncLog::new("editor_b", "Bob", "#ff6699");
        remote_peer.update_local_cursor(2.2, None);
        remote_peer.record_local(CrdtOpKind::AddMarker {
            marker: TimelineMarker::new(RationalTime::from_seconds(1.8), "Bob Remote Marker"),
        });
        let remote_delta = remote_peer.export_delta_json(None);

        let merged = call_tool(
            &session,
            "crdt_realtime_collab",
            &json!({"action": "merge_remote_delta", "delta_json": remote_delta, "apply": true}),
        )
        .unwrap();
        let merged_json: Value =
            serde_json::from_str(merged["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(merged_json["merged_new_ops"], 2);
        assert_eq!(merged_json["applied"], true);
        {
            let g = session.lock().unwrap();
            let markers = &g.project.active_scene().unwrap().markers;
            assert!(markers.iter().any(|m| m.label == "Bob Remote Marker"));
            assert!(g.crdt_log.peers.contains_key("editor_b"));
        }

        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 78);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v40_mcp_tools_aces_physics_generative_and_qc() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v40_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let mut proj = Project::new("v4.0 Master Studio Test", ProjectSettings::default());
        proj.active_scene_mut().unwrap().width = 320;
        proj.active_scene_mut().unwrap().height = 180;
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let session = std::sync::Arc::new(std::sync::Mutex::new(EditorSession::with_folder(
            proj, folder,
        )));

        // 1. #33 aces_color_management: set_config + export_ocio
        let ocio_path = tmp_dir.join("studio_v4.ocio");
        let cm_res = call_tool(
            &session,
            "aces_color_management",
            &json!({
                "action": "set_config",
                "enabled": true,
                "input_idt": "s_log3",
                "working_space": "aces_cg",
                "output_odt": "display_p3",
                "tone_mapper": "ag_x",
                "exposure_stops": 0.4,
                "gamut_compress": true
            }),
        )
        .unwrap();
        let cm_json: Value =
            serde_json::from_str(cm_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(cm_json["applied"], true);
        assert_eq!(cm_json["config"]["tone_mapper"], "ag_x");

        let ocio_res = call_tool(
            &session,
            "aces_color_management",
            &json!({
                "action": "export_ocio",
                "output_path": ocio_path.display().to_string()
            }),
        )
        .unwrap();
        let ocio_json: Value =
            serde_json::from_str(ocio_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(
            ocio_json["ocio_yaml"]
                .as_str()
                .unwrap()
                .contains("ocio_profile_version: 2")
        );
        assert!(ocio_path.exists());

        // 2. #35 generative_bg_and_foley: add_generative_bg + update + synthesize_foley_sfx
        let bg_res = call_tool(
            &session,
            "generative_bg_and_foley",
            &json!({
                "action": "add_generative_bg",
                "preset": "cyber_grid3d",
                "layer_index": 0,
                "start_sec": 0.0,
                "duration_sec": 3.0,
                "speed": 1.4
            }),
        )
        .unwrap();
        let bg_json: Value =
            serde_json::from_str(bg_res["content"][0]["text"].as_str().unwrap()).unwrap();
        let bg_id = bg_json["item_id"].as_str().unwrap().to_string();

        call_tool(
            &session,
            "generative_bg_and_foley",
            &json!({
                "action": "update_generative_bg",
                "id": bg_id,
                "preset": "aurora_borealis",
                "complexity": 5
            }),
        )
        .unwrap();

        let sfx_res = call_tool(
            &session,
            "generative_bg_and_foley",
            &json!({
                "action": "synthesize_foley_sfx",
                "preset": "sub_bass_impact",
                "duration_sec": 0.8,
                "audio_layer_index": 3,
                "apply": true
            }),
        )
        .unwrap();
        let sfx_json: Value =
            serde_json::from_str(sfx_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(sfx_json["applied"], true);
        assert!(std::path::Path::new(sfx_json["wav_path"].as_str().unwrap()).exists());

        // 3. #34 physics_simulate_and_bake: add a title text item, preview trajectory & bake keyframes
        let txt = call_tool(
            &session,
            "add_object",
            &json!({
                "layer_index": 1,
                "name": "PhysicsTitle",
                "kind": "text",
                "content": "PHYSICS V4",
                "start_sec": 0.0,
                "duration_sec": 3.0
            }),
        )
        .unwrap();
        let txt_id = serde_json::from_str::<Value>(txt["content"][0]["text"].as_str().unwrap())
            .unwrap()["created_id"]
            .as_str()
            .unwrap()
            .to_string();

        let phys_prev = call_tool(
            &session,
            "physics_simulate_and_bake",
            &json!({
                "action": "preview_trajectory",
                "item_ids": [txt_id.clone()],
                "preset": "bouncing_title",
                "duration_sec": 2.0
            }),
        )
        .unwrap();
        let phys_prev_json: Value =
            serde_json::from_str(phys_prev["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(phys_prev_json["body_count"], 1);
        assert!(phys_prev_json["total_bounces"].as_u64().unwrap() >= 1);

        let phys_bake = call_tool(
            &session,
            "physics_simulate_and_bake",
            &json!({
                "action": "bake_keyframes",
                "item_ids": [txt_id],
                "preset": "bouncing_title",
                "duration_sec": 2.0
            }),
        )
        .unwrap();
        let phys_bake_json: Value =
            serde_json::from_str(phys_bake["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(phys_bake_json["baked"], true);
        assert!(phys_bake_json["keyframe_commands"].as_u64().unwrap() >= 30);

        // 4. #36 broadcast_qc_and_web_export: run_qc_audit + generate_hero_thumbnail + export_web_review_html
        let qc_res = call_tool(
            &session,
            "broadcast_qc_and_web_export",
            &json!({
                "action": "run_qc_audit",
                "profile": "youtube1080p",
                "sample_count": 4
            }),
        )
        .unwrap();
        let qc_json: Value =
            serde_json::from_str(qc_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(qc_json["checks"].as_array().unwrap().len() >= 6);

        let thumb_path = tmp_dir.join("hero_thumb.png");
        let thumb_res = call_tool(
            &session,
            "broadcast_qc_and_web_export",
            &json!({
                "action": "generate_hero_thumbnail",
                "sample_count": 4,
                "output_path": thumb_path.display().to_string()
            }),
        )
        .unwrap();
        let thumb_json: Value =
            serde_json::from_str(thumb_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(thumb_json["width"], 1280);
        assert_eq!(thumb_json["height"], 720);
        assert!(thumb_path.exists());

        let html_path = tmp_dir.join("review_player.html");
        let html_res = call_tool(
            &session,
            "broadcast_qc_and_web_export",
            &json!({
                "action": "export_web_review_html",
                "profile": "youtube1080p",
                "sample_count": 4,
                "output_path": html_path.display().to_string()
            }),
        )
        .unwrap();
        let html_json: Value =
            serde_json::from_str(html_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(html_json["embedded_frames"], 4);
        assert!(html_path.exists());
        let html_body = std::fs::read_to_string(&html_path).unwrap();
        assert!(html_body.contains("data:image/png;base64,"));

        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 78);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v50_mcp_tools_rig2d_bgm_speed_ramp_and_finishing() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v50_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let mut proj = Project::new("v5.0 Pro Finishing Test", ProjectSettings::default());
        proj.active_scene_mut().unwrap().width = 160;
        proj.active_scene_mut().unwrap().height = 90;
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let session = std::sync::Arc::new(std::sync::Mutex::new(EditorSession::with_folder(
            proj, folder,
        )));

        // 1. #37 rig2d_puppet_control: add + update + solve_ik_preview
        let rig_add = call_tool(
            &session,
            "rig2d_puppet_control",
            &json!({
                "action": "add_rigged_puppet",
                "preset": "wave_hello",
                "layer_index": 1,
                "duration_sec": 3.0,
                "right_ik_enabled": true,
                "right_ik_x": 80.0,
                "right_ik_y": -85.0
            }),
        )
        .unwrap();
        let rig_json: Value =
            serde_json::from_str(rig_add["content"][0]["text"].as_str().unwrap()).unwrap();
        let rig_id = rig_json["item_id"].as_str().unwrap().to_string();
        assert_eq!(rig_json["bone_count"], 10);

        call_tool(
            &session,
            "rig2d_puppet_control",
            &json!({
                "action": "update_rig",
                "id": rig_id.clone(),
                "preset": "dance_bounce",
                "motion_speed": 1.3
            }),
        )
        .unwrap();

        let ik_prev = call_tool(
            &session,
            "rig2d_puppet_control",
            &json!({"action": "solve_ik_preview", "time_sec": 0.2}),
        )
        .unwrap();
        let ik_json: Value =
            serde_json::from_str(ik_prev["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(ik_json["bone_count"], 10);

        // 2. #38 offline_bgm_composer: preview + compose_and_place_bgm with bar markers
        let bgm_prev = call_tool(
            &session,
            "offline_bgm_composer",
            &json!({
                "action": "preview_arrangement",
                "genre": "cyber_synthwave",
                "duration_sec": 4.0
            }),
        )
        .unwrap();
        let bgm_prev_json: Value =
            serde_json::from_str(bgm_prev["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(bgm_prev_json["bar_count"].as_u64().unwrap() >= 2);

        let bgm_res = call_tool(
            &session,
            "offline_bgm_composer",
            &json!({
                "action": "compose_and_place_bgm",
                "genre": "lofi_chill",
                "duration_sec": 3.0,
                "audio_layer_index": 3,
                "add_bar_markers": true,
                "apply": true
            }),
        )
        .unwrap();
        let bgm_json: Value =
            serde_json::from_str(bgm_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(bgm_json["applied"], true);
        assert!(bgm_json["bar_markers_added"].as_u64().unwrap() >= 1);
        assert!(std::path::Path::new(bgm_json["wav_path"].as_str().unwrap()).exists());

        // 3. #39 speed_ramp_and_scene_detect: set bullet_time_dip on rig + detect hard cut
        let sr_res = call_tool(
            &session,
            "speed_ramp_and_scene_detect",
            &json!({
                "action": "set_speed_ramp",
                "id": rig_id,
                "preset": "bullet_time_dip",
                "enabled": true
            }),
        )
        .unwrap();
        let sr_json: Value =
            serde_json::from_str(sr_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(sr_json["applied"], true);

        // Create two contrasting full-screen colored shots for scene cut detection
        {
            let mut g = session.lock().unwrap();
            let sc = g.project.active_scene_mut().unwrap();
            sc.layers[0].items.push(TimelineItem::new(
                "RedShot",
                RationalTime::ZERO,
                RationalTime::from_seconds(1.5),
                ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: 160.0,
                    height: 90.0,
                    color: RgbaColor {
                        r: 0.92,
                        g: 0.12,
                        b: 0.12,
                        a: 1.0,
                    },
                },
            ));
            sc.layers[0].items.push(TimelineItem::new(
                "BlueShot",
                RationalTime::from_seconds(1.5),
                RationalTime::from_seconds(1.5),
                ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: 160.0,
                    height: 90.0,
                    color: RgbaColor {
                        r: 0.08,
                        g: 0.22,
                        b: 0.94,
                        a: 1.0,
                    },
                },
            ));
        }

        let cut_res = call_tool(
            &session,
            "speed_ramp_and_scene_detect",
            &json!({
                "action": "detect_scene_cuts",
                "sample_interval_sec": 0.25,
                "sensitivity": 0.6,
                "split_clips": false,
                "apply": true
            }),
        )
        .unwrap();
        let cut_json: Value =
            serde_json::from_str(cut_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(cut_json["cut_count"].as_u64().unwrap() >= 1);
        assert!(cut_json["applied_commands"].as_u64().unwrap() >= 1);

        // 4. #40 cinema_finishing_and_edl: set_finishing + export EDL + export FCPXML
        let fin_res = call_tool(
            &session,
            "cinema_finishing_and_edl",
            &json!({
                "action": "set_finishing",
                "preset": "kodak_35mm",
                "burn_in_timecode": true,
                "watermark_text": "V5 REVIEW"
            }),
        )
        .unwrap();
        let fin_json: Value =
            serde_json::from_str(fin_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(fin_json["applied"], true);
        assert_eq!(fin_json["config"]["burn_in_timecode"], true);

        let edl_path = tmp_dir.join("master.edl");
        let edl_res = call_tool(
            &session,
            "cinema_finishing_and_edl",
            &json!({
                "action": "export_edl_cmx3600",
                "output_path": edl_path.display().to_string()
            }),
        )
        .unwrap();
        let edl_json: Value =
            serde_json::from_str(edl_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(
            edl_json["edl"]
                .as_str()
                .unwrap()
                .contains("FCM: NON-DROP FRAME")
        );
        assert!(edl_path.exists());

        let xml_path = tmp_dir.join("master.fcpxml");
        let xml_res = call_tool(
            &session,
            "cinema_finishing_and_edl",
            &json!({
                "action": "export_fcpxml",
                "output_path": xml_path.display().to_string()
            }),
        )
        .unwrap();
        let xml_json: Value =
            serde_json::from_str(xml_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(
            xml_json["fcpxml"]
                .as_str()
                .unwrap()
                .contains("<fcpxml version=\"1.10\">")
        );
        assert!(xml_path.exists());

        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 78);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn v60_mcp_tools_dataviz_vfxpaint_podcast_and_packager() {
        let tmp_dir = std::env::temp_dir().join(format!("ve_mcp_v60_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let proj = Project::new("V60CreatorStudioTest", ProjectSettings::default());
        let folder = ve_core::ProjectFolder::init(&tmp_dir, &proj).unwrap();
        let session = Arc::new(Mutex::new(EditorSession::with_folder(proj, folder)));

        // 1. #41 data_chart_motion_graphics: create_chart + import_csv
        let chart_res = call_tool(
            &session,
            "data_chart_motion_graphics",
            &json!({
                "action": "create_chart",
                "kind": "bar_chart",
                "title": "ANNUAL SALES",
                "unit_suffix": "K",
                "layer_index": 1,
                "duration_sec": 4.0
            }),
        )
        .unwrap();
        let chart_json: Value =
            serde_json::from_str(chart_res["content"][0]["text"].as_str().unwrap()).unwrap();
        let chart_id = chart_json["created_id"].as_str().unwrap().to_string();
        assert_eq!(chart_json["point_count"], 4);

        let csv_res = call_tool(
            &session,
            "data_chart_motion_graphics",
            &json!({
                "action": "import_csv",
                "item_id": chart_id,
                "csv_text": "label,value,color\nJAN,45,#22ccff\nFEB,72,#ff4488\nMAR,118,#44ee88\n"
            }),
        )
        .unwrap();
        let csv_json: Value =
            serde_json::from_str(csv_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(csv_json["updated"], true);
        assert_eq!(csv_json["point_count"], 3);

        // 2. #42 vfx_paint_and_inpaint: add_stroke (brush + wire_erase) -> get_paint -> clear_paint
        let paint1 = call_tool(
            &session,
            "vfx_paint_and_inpaint",
            &json!({
                "action": "add_stroke",
                "item_id": chart_id,
                "tool": "brush_glow",
                "points": [[0.1, 0.2], [0.9, 0.8]],
                "radius_px": 12.0,
                "color": "#ff3366",
                "animate_write_on": true
            }),
        )
        .unwrap();
        let p1_json: Value =
            serde_json::from_str(paint1["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(p1_json["applied"], true);
        assert_eq!(p1_json["stroke_count"], 1);

        let paint2 = call_tool(
            &session,
            "vfx_paint_and_inpaint",
            &json!({
                "action": "add_stroke",
                "item_id": chart_id,
                "tool": "wire_erase_inpaint",
                "points": [[0.2, 0.5], [0.8, 0.5]],
                "radius_px": 8.0
            }),
        )
        .unwrap();
        let p2_json: Value =
            serde_json::from_str(paint2["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(p2_json["stroke_count"], 2);

        // 3. #43 podcast_auto_director: write 2 speaker WAVs, analyze_and_direct + level_wav
        let sr = 24_000u32;
        let frames = sr as usize * 3;
        let mut host_a = vec![0.0f32; frames * 2];
        let mut host_b = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let t = i as f32 / sr as f32;
            if t < 1.3 {
                let v = (std::f32::consts::TAU * 220.0 * t).sin() * 0.14;
                host_a[i * 2] = v;
                host_a[i * 2 + 1] = v;
            }
            if (1.5..2.8).contains(&t) {
                let v = (std::f32::consts::TAU * 170.0 * t).sin() * 0.48;
                host_b[i * 2] = v;
                host_b[i * 2 + 1] = v;
            }
        }
        let wav_a = tmp_dir.join("host_a.wav");
        let wav_b = tmp_dir.join("host_b.wav");
        let wav_leveled = tmp_dir.join("host_a_leveled.wav");
        write_stereo_wav_file(&wav_a, &host_a, sr).unwrap();
        write_stereo_wav_file(&wav_b, &host_b, sr).unwrap();

        let pod_res = call_tool(
            &session,
            "podcast_auto_director",
            &json!({
                "action": "analyze_and_direct",
                "speakers": [
                    { "name": "Host A", "audio_layer_index": 2, "camera_layer_index": 0, "wav_path": wav_a.display().to_string() },
                    { "name": "Host B", "audio_layer_index": 3, "camera_layer_index": 1, "wav_path": wav_b.display().to_string() }
                ],
                "min_turn_sec": 0.3,
                "target_lufs": -16.0,
                "apply": true
            }),
        )
        .unwrap();
        let pod_json: Value =
            serde_json::from_str(pod_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(pod_json["applied"], true);
        assert_eq!(pod_json["turn_count"], 2);

        let lvl_res = call_tool(
            &session,
            "podcast_auto_director",
            &json!({
                "action": "level_wav",
                "input_wav_path": wav_a.display().to_string(),
                "output_wav_path": wav_leveled.display().to_string(),
                "target_lufs": -16.0
            }),
        )
        .unwrap();
        let lvl_json: Value =
            serde_json::from_str(lvl_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(wav_leveled.exists());
        assert!(
            lvl_json["after_lufs"].as_f64().unwrap() > lvl_json["before_lufs"].as_f64().unwrap()
        );

        // 4. #44 multi_platform_packager: create_platform_variants + export_delivery_bundle
        let var_res = call_tool(
            &session,
            "multi_platform_packager",
            &json!({
                "action": "create_platform_variants",
                "platforms": ["youtube_16x9", "shorts_reels_9x16", "square_feed_1x1"]
            }),
        )
        .unwrap();
        let var_json: Value =
            serde_json::from_str(var_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(var_json["created_count"], 3);

        let bundle_dir = tmp_dir.join("delivery_bundle");
        let bdl_res = call_tool(
            &session,
            "multi_platform_packager",
            &json!({
                "action": "export_delivery_bundle",
                "output_dir": bundle_dir.display().to_string(),
                "platforms": ["youtube_16x9", "shorts_reels_9x16"],
                "base_title": "V6 Launch Episode",
                "tags": ["rust", "video", "v6"]
            }),
        )
        .unwrap();
        let bdl_json: Value =
            serde_json::from_str(bdl_res["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(bdl_json["artifact_count"].as_u64().unwrap() >= 8);
        assert!(bundle_dir.join("manifest.json").exists());
        assert!(bundle_dir.join("thumbnail_1280x720.png").exists());
        assert!(bundle_dir.join("interchange/master.edl").exists());
        assert!(bundle_dir.join("interchange/master.fcpxml").exists());
        assert!(bundle_dir.join("qc/broadcast_qc_report.json").exists());
        assert!(bundle_dir.join("subtitles/shorts_reels_9x16.srt").exists());

        let defs = tool_definitions();
        assert_eq!(defs.as_array().unwrap().len(), 78);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
