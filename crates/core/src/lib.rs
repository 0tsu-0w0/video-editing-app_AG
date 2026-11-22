pub mod animation;
pub mod cache;
pub mod camera;
pub mod collab;
pub mod command;
pub mod delivery;
pub mod encoding;
pub mod expression;
pub mod folder;
pub mod highlights;
pub mod interop;
pub mod kinetic_sub;
pub mod lipsync;
pub mod multicam;
pub mod node_graph;
pub mod preset;
pub mod project;
pub mod recipe;
pub mod registry;
pub mod script_import;
pub mod settings;
pub mod template;
pub mod text_edit;
pub mod time;
pub mod timeline_ops;

pub use animation::{CubicBezier, Interpolation, Keyframe, evaluate_track, evaluate_velocity};
pub use cache::{CacheError, PersistentFrameCache, PrecompCacheManager, content_hash_hex};
pub use camera::{Camera3D, Light3D, Light3DType, Projected3D, Transform3D};
pub use collab::{
    ConflictKind, ConflictResolution, DiffChangeKind, ItemDiff, LayerDiff, MarkerDiff,
    MergeConflict, ProjectDiff, ReviewComment, ReviewReply, ReviewSource, ReviewStatus, SceneDiff,
    ThreeWayMergeResult, build_review_comments_to_markers_commands, diff_projects,
    export_review_comments_csv, export_review_comments_json, format_diff_report,
    format_smpte_timecode, import_review_comments_csv, import_review_comments_json,
    import_youtube_review_comments, merge_projects_three_way, parse_timecode_to_rational,
};
pub use command::{CommandBus, CommandError, CommandSource, DEFAULT_MAX_UNDO_DEPTH, EditCommand};
pub use delivery::{
    ArchivePackageSummary, DeliveryIssue, DeliveryReport, IssueSeverity, archive_project_to_zip,
    crc32_ieee, validate_project_delivery,
};
pub use encoding::base64_encode;
pub use expression::{
    CompiledExpression, EXPRESSION_PROPERTIES, ExpressionContext, ExpressionError, LayerLookupFn,
    build_bake_expression_command, compile_expression, evaluate_expression, evaluate_item_property,
    evaluate_item_transform, expression_presets, item_base_property, item_expressions_use_audio,
};
pub use folder::{AssetResolution, FolderError, HistoryEntry, ProjectFolder};
pub use highlights::{
    HighlightCandidate, HighlightConfig, build_motion_track_keyframes_command,
    build_shorts_clip_scene_command, extract_highlight_candidates,
};
pub use interop::{
    SubtitleEntry, decode_exo_utf16le_hex, export_ass_subtitles, export_otio_json,
    export_srt_subtitles, parse_ass_subtitles, parse_aviutl_exo, parse_srt_subtitles,
    subtitle_entries_to_commands,
};
pub use kinetic_sub::{
    KineticHighlightMode, KineticSubtitleConfig, KineticWordSpan, LocalizationEntry,
    LocalizationTable, build_apply_locale_commands, export_localization_csv,
    extract_localization_table, generate_kinetic_inline_markup, import_localization_csv,
    tokenize_kinetic_words,
};
pub use lipsync::{
    CharacterSpriteConfig, LipSyncMode, MouthShape, PhonemeSegment, PsdCharacterConfig,
    evaluate_vowel_mouth, phoneme_to_mouth, resolve_mouth_png, rms_to_mouth,
};
pub use multicam::{
    MulticamAngle, MulticamCutPoint, MulticamGridLayout, build_multicam_switch_cut_commands,
    build_multicam_sync_align_commands,
};
pub use node_graph::{
    ChannelSelector, CompositeNode, CompositeNodeKind, NodeCompositeGraph, NodeConnection,
    NodeGraphError, NodeId, NodeToLayerSyncSummary, sync_node_graph_to_scene_layers,
};
pub use preset::{ColorPalette, EffectPreset, ItemAlias, NamedColor, TextStylePreset};
pub use project::{
    AssetId, AssetRef, AudioDuckingConfig, AudioEffectNode, AudioReactiveBand, AudioReactiveConfig,
    BezierMask, BlendMode, ColorCurves, CornerPinConfig, EffectId, EffectNode, FilterTargetScope,
    HslQualifier, ItemId, ItemKind, Layer, LayerId, MaskMode, MaskVertex, MediaKind,
    MotionBlurConfig, MotionTrackConfig, ParticleConfig, ParticlePreset, PlaybackSettings, Project,
    ProjectSettings, RgbaColor, Scene, SceneColorGrade, SceneId, ScenePreset, ShapeKind,
    TextAnimator, TextStyle, TimelineItem, TimelineMarker, TrackMatte, TrackMatteMode,
    TrackPointSample, Transform2D, TransitionKind, VectorFillMode, VectorShapeConfig,
};
pub use recipe::{
    AgentRole, ApprovalGateStatus, EditRecipe, RecipeError, RecipeRunState, RecipeStep,
    RecipeStepLog, VisualCheckExpectation, VisualCheckReport, builtin_edit_recipes,
    parse_recipe_json_or_yaml, record_recipe_from_commands, resolve_template_vars,
    serialize_recipe_json, serialize_recipe_yaml,
};
pub use registry::{
    ExtensionKind, RegistryEntry, SandboxPermission, official_registry_catalog, sha256_hex,
    verify_and_install_extension,
};
pub use script_import::{
    ScriptBuildConfig, ScriptLine, build_script_timeline_commands, parse_script_auto,
    parse_script_csv, parse_script_markdown,
};
pub use settings::{
    EditorAction, EditorKeymap, EditorUserSettings, KeymapPreset, UiLanguage, UiThemePreset, tr,
};
pub use template::{
    ExposedParam, ExposedParamKind, MotionTemplate, ProjectTemplate, ResponsiveTimeRegion,
    TemplateBinding, TemplateParamValue, apply_responsive_time_stretch,
    apply_template_values_to_scene, build_instantiate_template_command, builtin_motion_templates,
    builtin_project_templates, extract_motion_template_from_scene, map_responsive_time,
};
pub use text_edit::{
    DEFAULT_FILLERS, TextEditCutResult, TranscriptEntry, analyze_fillers,
    build_strip_filler_words_commands, build_text_based_delete_commands,
    build_text_based_reorder_commands, extract_transcript_entries, strip_control_tags,
};
pub use time::{FrameRate, RationalTime, TICKS_PER_SECOND, TimeInput};
pub use timeline_ops::{
    AlignMode, JumpCutConfig, MotionPreset, SelectDirection, SpeechInterval, TextSearchMatch,
    build_align_commands, build_beat_cut_commands, build_beat_markers_command,
    build_close_gaps_commands, build_fit_duration_to_layer_commands, build_jump_cut_commands,
    build_motion_preset_commands, build_replace_text_commands, build_ripple_delete_commands,
    build_snap_to_beats_commands, detect_speech_intervals_from_rms, format_youtube_chapters,
    search_text_items, select_items_from_time, select_layer_items, wrap_subtitle_japanese,
};
