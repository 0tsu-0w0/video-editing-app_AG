pub mod animation;
pub mod cache;
pub mod camera;
pub mod command;
pub mod encoding;
pub mod folder;
pub mod highlights;
pub mod interop;
pub mod lipsync;
pub mod preset;
pub mod project;
pub mod script_import;
pub mod text_edit;
pub mod time;
pub mod timeline_ops;

pub use animation::{CubicBezier, Interpolation, Keyframe, evaluate_track, evaluate_velocity};
pub use cache::{CacheError, PrecompCacheManager, content_hash_hex};
pub use camera::{Camera3D, Projected3D, Transform3D};
pub use command::{CommandBus, CommandError, CommandSource, DEFAULT_MAX_UNDO_DEPTH, EditCommand};
pub use encoding::base64_encode;
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
pub use lipsync::{
    CharacterSpriteConfig, LipSyncMode, MouthShape, PhonemeSegment, PsdCharacterConfig,
    evaluate_vowel_mouth, phoneme_to_mouth, resolve_mouth_png, rms_to_mouth,
};
pub use preset::{ColorPalette, EffectPreset, ItemAlias, NamedColor, TextStylePreset};
pub use project::{
    AssetId, AssetRef, AudioDuckingConfig, AudioEffectNode, AudioReactiveBand, AudioReactiveConfig,
    BezierMask, BlendMode, EffectId, EffectNode, FilterTargetScope, ItemId, ItemKind, Layer,
    LayerId, MaskMode, MaskVertex, MediaKind, MotionTrackConfig, PlaybackSettings, Project,
    ProjectSettings, RgbaColor, Scene, SceneColorGrade, SceneId, ScenePreset, ShapeKind,
    TextAnimator, TextStyle, TimelineItem, TimelineMarker, TrackMatte, TrackMatteMode,
    TrackPointSample, Transform2D, TransitionKind,
};
pub use script_import::{
    ScriptBuildConfig, ScriptLine, build_script_timeline_commands, parse_script_auto,
    parse_script_csv, parse_script_markdown,
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
