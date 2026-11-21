use crate::time::{FrameRate, RationalTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

pub type AssetId = Uuid;
pub type SceneId = Uuid;
pub type LayerId = Uuid;
pub type ItemId = Uuid;
pub type EffectId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RgbaColor {
    /// sRGB 0.0..=1.0
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl RgbaColor {
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation_deg: f32,
    pub opacity: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_deg: 0.0,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    Font,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRef {
    pub id: AssetId,
    pub name: String,
    pub kind: MediaKind,
    /// プロジェクトフォルダ基準の相対パス（優先解決）
    pub relative_path: Option<PathBuf>,
    /// 別ドライブ等でのフォールバック絶対パス
    pub fallback_absolute_path: Option<PathBuf>,
    /// プロジェクト収集（assets/へコピー済み）かどうか
    pub collected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectNode {
    pub id: EffectId,
    pub effect_type: String,
    pub enabled: bool,
    pub params: BTreeMap<String, f32>,
}

/// AviUtl2風フィルタオブジェクトの適用範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum FilterTargetScope {
    /// 直上（または直下）の1レイヤーのみ
    ImmediateLayer,
    /// 指定したレイヤー数分へ適用
    LayerRange { count: usize },
    /// 以降の全対象レイヤーへ適用
    AllRemaining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemKind {
    Video {
        asset_id: AssetId,
        source_offset: RationalTime,
        volume: f32,
    },
    Audio {
        asset_id: AssetId,
        source_offset: RationalTime,
        volume: f32,
    },
    Image {
        asset_id: AssetId,
    },
    Text {
        content: String,
        font_family: String,
        font_size: f32,
        color: RgbaColor,
    },
    Shape {
        shape: ShapeKind,
        width: f32,
        height: f32,
        color: RgbaColor,
    },
    /// AviUtl2方式のタイムライン上フィルタオブジェクト（個別エフェクトチェーンとは別系統）
    FilterObject {
        effect_type: String,
        scope: FilterTargetScope,
        params: BTreeMap<String, f32>,
    },
    Precomp {
        scene_id: SceneId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: ItemId,
    pub name: String,
    pub start: RationalTime,
    pub duration: RationalTime,
    pub transform: Transform2D,
    /// オブジェクト個別のAE風エフェクトチェーン
    pub effect_chain: Vec<EffectNode>,
    pub kind: ItemKind,
}

impl TimelineItem {
    pub fn end(&self) -> RationalTime {
        self.start + self.duration
    }

    pub fn contains(&self, t: RationalTime) -> bool {
        t >= self.start && t < self.end()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub muted: bool,
    pub items: Vec<TimelineItem>,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            visible: true,
            muted: false,
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: SceneId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub layers: Vec<Layer>,
}

impl Scene {
    pub fn new(name: impl Into<String>, width: u32, height: u32, fps: FrameRate) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            width,
            height,
            fps,
            layers: vec![
                Layer::new("Layer 1"),
                Layer::new("Layer 2"),
                Layer::new("Layer 3"),
            ],
        }
    }

    pub fn total_duration(&self) -> RationalTime {
        self.layers
            .iter()
            .flat_map(|l| l.items.iter())
            .map(TimelineItem::end)
            .max()
            .unwrap_or(RationalTime::ZERO)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub audio_sample_rate: u32,
    /// キャッシュ容量上限（バイト、既定 8 GiB）
    pub cache_max_bytes: u64,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: FrameRate::FPS_60,
            audio_sample_rate: 48_000,
            cache_max_bytes: 8 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub name: String,
    pub settings: ProjectSettings,
    pub assets: Vec<AssetRef>,
    pub scenes: Vec<Scene>,
    pub active_scene_id: SceneId,
}

impl Project {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    pub fn new(name: impl Into<String>, settings: ProjectSettings) -> Self {
        let main_scene = Scene::new("Main", settings.width, settings.height, settings.fps);
        let active_scene_id = main_scene.id;
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            name: name.into(),
            settings,
            assets: Vec::new(),
            scenes: vec![main_scene],
            active_scene_id,
        }
    }

    pub fn active_scene(&self) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == self.active_scene_id)
    }

    pub fn active_scene_mut(&mut self) -> Option<&mut Scene> {
        self.scenes
            .iter_mut()
            .find(|s| s.id == self.active_scene_id)
    }
}
