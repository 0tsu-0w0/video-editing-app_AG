//! v7.0 #46 / v8.0 #49〜#52 / v9.0 #53〜#56 / v10.0 #57〜#60 / v11.0 #61〜#64: ドメイン別に分割整理された内蔵 MCP ツール群（全 104 ツール / 11 ドメイン）
//!
//! - `edit_nle`: タイムライン基本操作・マルチカム・テキスト編集・キネティック字幕・スピードランプ＆カット検出
//! - `vfx_motion`: エフェクト・マスク・エクスプレッション・テンプレート・ノード合成・3D/glTF・物理・2D IK・チャート・VFX ペイント
//! - `color_finishing`: プライマリ色調＆3D LUT・トーンカーブ/HSL/カラーマッチ・ACEScg/OCIO・35mm シネマ仕上げ＆EDL/FCPXML
//! - `audio_voice`: 音声 DSP・ダッキング・ビート検出・3D 空間音響/True-Peak・AI 吹替・自動 BGM 作曲・ポッドキャスト自動編集
//! - `ai_collab`: ローカル AI ビジョン・AI 編集ディレクター＆レシピ・Git Diff/3-Way マージ/レビュー・P2P CRDT
//! - `deliver_system`: 書き出し・プラグイン/WASM・納品検証＆ZIP・環境設定/拡張レジストリ・放送 QC＆HTML5・マルチ納品パッケージャー
//! - `v70_unified`: 6 スタジオ Hub＆`Ctrl+K` パレット・MCP カタログ自己診断・GPU 4K/8K フレームプール・10k 区間木＆TC コンフォーム＆A/B 比較
//! - `v80_cinema_audio`: 4 点平面トラッカー＆ロトブラシ・SVG モーフィング＆Text-on-Path＆ワープ・シネマ光学 VFX・オーディオ修復＆ADR＆5.1/FOA 空間バス
//! - `v90_broadcast_ecosystem`: ライブスイッチャー＆RTMP/SRT/.veproj・VTuber 2D メッシュ＆Verlet＆VMC・OSC/MIDI/StreamDeck・スマート DAM
//! - `v100_spatial_cinema_os`: 360° 全天球＆空間 3D・2D 流体/クロス物理＆3D シャドウ/HDRI IBL・BVH/シルエットモキャップ・LAN レンダーファーム＆自律 AI プロデューサー
//! - `v110_performance_gpu`: 完全ゼロコピー GPU レンダリンググラフ・適応解像度 8K＆スマート事前レンダリング・FastBin 省メモリ監査・リアルタイムプロファイラ HUD＆ボトルネック診断

mod ai_collab;
mod audio_voice;
mod color_finishing;
mod deliver_system;
mod edit_nle;
mod v100_spatial_cinema_os;
mod v110_performance_gpu;
mod v70_unified;
mod v80_cinema_audio;
mod v90_broadcast_ecosystem;
mod vfx_motion;

#[cfg(test)]
mod tests;

pub(crate) use crate::session::SharedEditorSession;
use serde::{Deserialize, Serialize};
pub(crate) use serde_json::{Value, json};
pub(crate) use std::collections::BTreeMap;
pub(crate) use std::path::PathBuf;
pub(crate) use uuid::Uuid;
pub(crate) use ve_ai::{
    ApiProvider, ClaudeRefiner, CloudUploadGuard, OsCredentialStore, SubtitleSegment,
    import_tts_file_pair,
};
pub(crate) use ve_audio::{
    AudioGraph, BeatDetectionConfig, BgmComposeConfig, BgmGenre, DubbingVoicePreset, FoleyPreset,
    GeneratedDubClip, SpeakerTrackSpec, align_text_to_phoneme_segments, analyze_podcast_speakers,
    apply_podcast_voice_leveler, build_dubbing_timeline_commands,
    build_podcast_auto_switch_commands, compose_bgm_track, detect_audio_beats,
    estimate_audio_offset_xcorr, measure_integrated_lufs, measure_true_peak_dbtp,
    separate_vocal_bgm_stems, synthesize_foley_sfx_pcm, synthesize_offline_dubbing_pcm,
    write_stereo_wav_file,
};
pub(crate) use ve_core::{
    AbCompareMode, AbReferenceSource, AlignMode, AssetRef, AudioEffectNode, BezierMask, BlendMode,
    BroadcastProfile, BroadcastQcMetrics, ChannelSelector, CharacterSpriteConfig, ChartKind,
    CinemaFinishingConfig, ColorCurves, ColorManagementConfig, ColorSpace, CommandSource,
    CompositeNode, CompositeNodeKind, ConflictResolution, ConformDirection, CornerPinConfig,
    CrdtOpKind, CrdtSyncLog, CubicBezier, DataChartConfig, DataSeriesPoint, DeliveryBundleArtifact,
    DeliveryBundleConfig, DeliveryPlatform, EditCommand, EditorKeymap, EffectNode, EffectPreset,
    FrameRate, GenerativeBgConfig, GenerativeBgPreset, HighlightConfig, Interpolation, ItemKind,
    JumpCutConfig, Keyframe, KeymapPreset, KineticHighlightMode, KineticSubtitleConfig,
    LetterboxAspect, Light3DType, LipSyncMode, LocalizationEntry, LocalizationTable, MaskMode,
    MaskVertex, MediaKind, Mesh3DConfig, Mesh3DPreset, MotionPreset, MulticamCutPoint,
    NodeCompositeGraph, PaintStroke, PaintToolKind, ParticleConfig, ParticlePreset, PhysicsConfig,
    PhysicsPreset, ProjectTemplate, PsdCharacterConfig, RationalTime, ReviewComment, ReviewSource,
    ReviewStatus, RgbaColor, Rig2DConfig, RigMotionPreset, SceneColorGrade, SceneIntervalIndex,
    ScriptBuildConfig, ShapeKind, SpeedRampKnot, SpeedRampPreset, StudioHubId, TemplateParamValue,
    TextAnimator, TextStylePreset, TimelineItem, TimelineMarker, ToneMapper, TrackMatte,
    TrackMatteMode, UiLanguage, UiThemePreset, VectorShapeConfig, VfxPaintConfig,
    archive_project_to_zip, base64_encode, benchmark_interval_index, build_align_commands,
    build_apply_locale_commands, build_bake_expression_command, build_beat_cut_commands,
    build_beat_markers_command, build_close_gaps_commands, build_cube_mesh,
    build_delivery_manifest, build_fit_duration_to_layer_commands,
    build_instantiate_template_command, build_jump_cut_commands, build_motion_preset_commands,
    build_multicam_switch_cut_commands, build_multicam_sync_align_commands,
    build_physics_bake_commands, build_reformatted_scene_command, build_replace_text_commands,
    build_review_comments_to_markers_commands, build_scene_cut_commands,
    build_script_timeline_commands, build_shorts_clip_scene_command,
    build_strip_filler_words_commands, build_text_based_delete_commands,
    build_text_based_reorder_commands, build_timeline_conform_command, builtin_motion_templates,
    builtin_project_templates, commands_for_hub, compile_expression,
    detect_speech_intervals_from_rms, diff_projects, encode_mesh_to_glb, evaluate_item_property,
    evaluate_rig_poses, export_ass_subtitles, export_cmx3600_edl, export_fcpxml_v10,
    export_localization_csv, export_ocio_config_yaml, export_otio_json,
    export_portable_web_review_html, export_review_comments_csv, export_review_comments_json,
    export_srt_subtitles, extract_highlight_candidates, extract_localization_table,
    extract_motion_template_from_scene, extract_transcript_entries, format_diff_report,
    format_youtube_chapters, fuzzy_search_commands, import_localization_csv,
    import_review_comments_csv, import_review_comments_json, import_youtube_review_comments,
    merge_projects_three_way, official_registry_catalog, parse_ass_subtitles, parse_aviutl_exo,
    parse_chart_csv, parse_gltf_or_glb, parse_script_auto, parse_srt_subtitles,
    plan_timeline_conform, run_broadcast_qc_audit, search_text_items, simulate_scene_physics,
    studio_palette_catalog, subtitle_entries_to_commands, sync_node_graph_to_scene_layers,
    validate_project_delivery, verify_and_install_extension, wrap_subtitle_japanese,
};
pub(crate) use ve_media::{
    ExportConfig, ExportFormat, FfmpegCli, HwDecodeBackend, VideoEncoder, decode_audio_file,
    write_wav_pcm16,
};
pub(crate) use ve_plugin_api::{PluginRegistry, WasmPluginManager, emit_sample_wasm_plugins};
pub(crate) use ve_render::{
    AutoReframeConfig, FastBlendMaskParams, FastMaskShape, GpuV70FastPipeline, LocalVisionBackend,
    RenderGraph, StabilizeConfig, analyze_3d_camera_track, analyze_camera_shake,
    analyze_scene_auto_reframe, benchmark_v30_gpu_pipeline, benchmark_v70_frame_pool,
    build_3d_camera_track_command, build_auto_reframe_commands, build_stabilize_commands,
    compute_auto_color_match, detect_gpu_v30_capabilities, detect_gpu_v70_status,
    detect_local_vision_status, detect_scene_cuts_in_scene, emit_sample_cube_luts,
    encode_linear_image_png_bytes, execute_v70_aces_or_cpu, execute_v70_blend_mask_or_cpu,
    measure_scene_video_qc_metrics, render_ab_compare_scene, run_gpu_or_cpu_grade_and_scopes,
    select_best_thumbnail_frame, validate_wgsl,
};

pub(crate) const MAX_BATCH_COMMANDS: usize = 500;
pub(crate) const ALLOWED_TTS_AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a", "aac"];

/// v7.0 #46: ドメイン別 MCP モジュール統計サマリー
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDomainSummary {
    pub domain_id: String,
    pub label: String,
    pub tool_count: usize,
    pub tool_names: Vec<String>,
}

fn extract_names(defs: &[Value]) -> Vec<String> {
    defs.iter()
        .filter_map(|v| v.get("name").and_then(Value::as_str).map(String::from))
        .collect()
}

/// 分割された全 MCP サブモジュールのドメイン別カタログを返す
pub fn tool_domain_catalog() -> Vec<McpDomainSummary> {
    let groups: &[(&str, &str, Vec<Value>)] = &[
        (
            edit_nle::DOMAIN_ID,
            edit_nle::DOMAIN_LABEL,
            edit_nle::definitions(),
        ),
        (
            vfx_motion::DOMAIN_ID,
            vfx_motion::DOMAIN_LABEL,
            vfx_motion::definitions(),
        ),
        (
            color_finishing::DOMAIN_ID,
            color_finishing::DOMAIN_LABEL,
            color_finishing::definitions(),
        ),
        (
            audio_voice::DOMAIN_ID,
            audio_voice::DOMAIN_LABEL,
            audio_voice::definitions(),
        ),
        (
            ai_collab::DOMAIN_ID,
            ai_collab::DOMAIN_LABEL,
            ai_collab::definitions(),
        ),
        (
            deliver_system::DOMAIN_ID,
            deliver_system::DOMAIN_LABEL,
            deliver_system::definitions(),
        ),
        (
            v70_unified::DOMAIN_ID,
            v70_unified::DOMAIN_LABEL,
            v70_unified::definitions(),
        ),
        (
            v80_cinema_audio::DOMAIN_ID,
            v80_cinema_audio::DOMAIN_LABEL,
            v80_cinema_audio::definitions(),
        ),
        (
            v90_broadcast_ecosystem::DOMAIN_ID,
            v90_broadcast_ecosystem::DOMAIN_LABEL,
            v90_broadcast_ecosystem::definitions(),
        ),
        (
            v100_spatial_cinema_os::DOMAIN_ID,
            v100_spatial_cinema_os::DOMAIN_LABEL,
            v100_spatial_cinema_os::definitions(),
        ),
        (
            v110_performance_gpu::DOMAIN_ID,
            v110_performance_gpu::DOMAIN_LABEL,
            v110_performance_gpu::definitions(),
        ),
    ];

    groups
        .iter()
        .map(|(id, label, defs)| {
            let names = extract_names(defs);
            McpDomainSummary {
                domain_id: (*id).to_string(),
                label: (*label).to_string(),
                tool_count: names.len(),
                tool_names: names,
            }
        })
        .collect()
}

/// 全ドメインの MCP ツール定義（全 104 ツール / 11 ドメイン）を JSON 配列として返す
pub fn tool_definitions() -> Value {
    let mut tools = Vec::with_capacity(104);
    tools.extend(edit_nle::definitions());
    tools.extend(vfx_motion::definitions());
    tools.extend(color_finishing::definitions());
    tools.extend(audio_voice::definitions());
    tools.extend(ai_collab::definitions());
    tools.extend(deliver_system::definitions());
    tools.extend(v70_unified::definitions());
    tools.extend(v80_cinema_audio::definitions());
    tools.extend(v90_broadcast_ecosystem::definitions());
    tools.extend(v100_spatial_cinema_os::definitions());
    tools.extend(v110_performance_gpu::definitions());
    Value::Array(tools)
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

    if let Some(res) = edit_nle::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = vfx_motion::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = color_finishing::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = audio_voice::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = ai_collab::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = deliver_system::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = v70_unified::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = v80_cinema_audio::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = v90_broadcast_ecosystem::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = v100_spatial_cinema_os::try_call(&mut guard, fps, name, args) {
        return res;
    }
    if let Some(res) = v110_performance_gpu::try_call(&mut guard, fps, name, args) {
        return res;
    }

    Err(format!("unknown tool: {name}"))
}

pub(crate) fn finite_clamped(val: Option<&Value>, default: f64, min: f64, max: f64) -> f64 {
    match val.and_then(Value::as_f64) {
        Some(v) if v.is_finite() => v.clamp(min, max),
        _ => default,
    }
}

pub(crate) fn parse_time_arg(
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

pub(crate) fn parse_uuid(args: &Value, key: &str) -> Result<Uuid, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument '{key}'"))?;
    Uuid::parse_str(raw).map_err(|e| format!("invalid uuid for '{key}': {e}"))
}

pub(crate) fn parse_params_map(val: Option<&Value>) -> BTreeMap<String, f32> {
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

pub(crate) fn parse_template_param_values(
    val: Option<&Value>,
) -> BTreeMap<String, TemplateParamValue> {
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

pub(crate) fn encode_rgba8_to_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
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
