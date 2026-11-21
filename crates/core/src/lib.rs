pub mod animation;
pub mod cache;
pub mod camera;
pub mod command;
pub mod encoding;
pub mod folder;
pub mod lipsync;
pub mod preset;
pub mod project;
pub mod time;
pub mod timeline_ops;

pub use animation::{CubicBezier, Interpolation, Keyframe, evaluate_track, evaluate_velocity};
pub use cache::{CacheError, PrecompCacheManager, content_hash_hex};
pub use camera::{Camera3D, Projected3D, Transform3D};
pub use command::{CommandBus, CommandError, CommandSource, DEFAULT_MAX_UNDO_DEPTH, EditCommand};
pub use encoding::base64_encode;
pub use folder::{AssetResolution, FolderError, HistoryEntry, ProjectFolder};
pub use lipsync::{
    CharacterSpriteConfig, LipSyncMode, MouthShape, PhonemeSegment, PsdCharacterConfig,
    evaluate_vowel_mouth, phoneme_to_mouth, resolve_mouth_png, rms_to_mouth,
};
pub use preset::{ColorPalette, EffectPreset, ItemAlias, NamedColor};
pub use project::{
    AssetId, AssetRef, AudioEffectNode, BlendMode, EffectId, EffectNode, FilterTargetScope, ItemId,
    ItemKind, Layer, LayerId, MediaKind, PlaybackSettings, Project, ProjectSettings, RgbaColor,
    Scene, SceneId, ShapeKind, TextAnimator, TextStyle, TimelineItem, TrackMatte, TrackMatteMode,
    Transform2D, TransitionKind,
};
pub use time::{FrameRate, RationalTime, TICKS_PER_SECOND, TimeInput};
pub use timeline_ops::{
    AlignMode, MotionPreset, SelectDirection, TextSearchMatch, build_align_commands,
    build_close_gaps_commands, build_fit_duration_to_layer_commands, build_motion_preset_commands,
    build_replace_text_commands, search_text_items, select_items_from_time, select_layer_items,
};
