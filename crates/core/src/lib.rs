pub mod animation;
pub mod cache;
pub mod camera;
pub mod command;
pub mod encoding;
pub mod folder;
pub mod lipsync;
pub mod project;
pub mod time;

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
pub use project::{
    AssetId, AssetRef, BlendMode, EffectId, EffectNode, FilterTargetScope, ItemId, ItemKind, Layer,
    LayerId, MediaKind, Project, ProjectSettings, RgbaColor, Scene, SceneId, ShapeKind,
    TextAnimator, TimelineItem, TrackMatte, TrackMatteMode, Transform2D,
};
pub use time::{FrameRate, RationalTime, TICKS_PER_SECOND, TimeInput};
