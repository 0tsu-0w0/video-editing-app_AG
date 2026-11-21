use crate::animation::{CubicBezier, Keyframe, evaluate_track};
use crate::camera::{Camera3D, Transform3D};
use crate::lipsync::{CharacterSpriteConfig, PhonemeSegment};
use crate::preset::ColorPalette;
use crate::time::{FrameRate, RationalTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

fn default_one_f32() -> f32 {
    1.0
}

fn default_transition_softness() -> f32 {
    0.08
}

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

    /// `#RRGGBB` または `#RRGGBBAA`（先頭 `#` 任意）を `RgbaColor` へ変換する
    pub fn from_hex(hex: &str) -> Option<Self> {
        let s = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
        match s.len() {
            6 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                Some(Self {
                    r: f32::from(r) / 255.0,
                    g: f32::from(g) / 255.0,
                    b: f32::from(b) / 255.0,
                    a: 1.0,
                })
            }
            8 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                let a = u8::from_str_radix(&s[6..8], 16).ok()?;
                Some(Self {
                    r: f32::from(r) / 255.0,
                    g: f32::from(g) / 255.0,
                    b: f32::from(b) / 255.0,
                    a: f32::from(a) / 255.0,
                })
            }
            _ => None,
        }
    }

    pub fn to_hex_rgb(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            (self.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.b.clamp(0.0, 1.0) * 255.0).round() as u8
        )
    }

    pub fn to_hex_rgba(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            (self.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.b.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.a.clamp(0.0, 1.0) * 255.0).round() as u8
        )
    }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
    Screen,
}

/// トラックマット（アルファ／ルミナンスおよび各反転）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackMatteMode {
    Alpha,
    AlphaInverted,
    Luma,
    LumaInverted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackMatte {
    pub mode: TrackMatteMode,
    pub source_layer_index: usize,
}

/// テキストの文字ごとアニメーター（仕様書 2.2）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextAnimator {
    pub enabled: bool,
    /// 各文字の出現前オフセット（出現完了時に 0 へ補間）
    pub start_offset_x: f32,
    pub start_offset_y: f32,
    /// 各文字の出現前スケール（出現完了時に 1.0 へ補間）
    pub start_scale: f32,
    /// 各文字の出現前回転角（度）
    pub start_rotation_deg: f32,
    /// 各文字の出現前不透明度（出現完了時に 1.0 へ補間）
    pub start_opacity: f32,
    /// 1文字あたりの出現ディレイ（秒）
    pub delay_per_char_sec: f32,
    /// 1文字あたりのアニメーション時間（秒）
    pub duration_per_char_sec: f32,
    pub easing: CubicBezier,
}

impl Default for TextAnimator {
    fn default() -> Self {
        Self {
            enabled: false,
            start_offset_x: 0.0,
            start_offset_y: 28.0,
            start_scale: 0.7,
            start_rotation_deg: 0.0,
            start_opacity: 0.0,
            delay_per_char_sec: 0.06,
            duration_per_char_sec: 0.35,
            easing: CubicBezier::EASE_OUT,
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

/// 音声 DSP エフェクトノード（Utsbox Audio Effect 相当）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioEffectNode {
    pub id: EffectId,
    /// `"low_cut"`, `"high_cut"`, `"eq_3band"`, `"compressor"`, `"limiter"`, `"delay"`, `"reverb"`, `"radio_voice"`
    pub effect_type: String,
    pub enabled: bool,
    pub params: BTreeMap<String, f32>,
}

impl AudioEffectNode {
    pub fn with_defaults(effect_type: impl Into<String>) -> Self {
        let effect_type = effect_type.into();
        let mut params = BTreeMap::new();
        match effect_type.as_str() {
            "low_cut" => {
                params.insert("cutoff_hz".into(), 100.0);
            }
            "high_cut" => {
                params.insert("cutoff_hz".into(), 8000.0);
            }
            "eq_3band" => {
                params.insert("low_db".into(), 0.0);
                params.insert("mid_db".into(), 0.0);
                params.insert("high_db".into(), 0.0);
            }
            "compressor" => {
                params.insert("threshold_db".into(), -18.0);
                params.insert("ratio".into(), 4.0);
                params.insert("makeup_db".into(), 3.0);
            }
            "limiter" => {
                params.insert("ceiling_db".into(), -1.0);
            }
            "delay" => {
                params.insert("delay_ms".into(), 240.0);
                params.insert("feedback".into(), 0.35);
                params.insert("mix".into(), 0.3);
            }
            "reverb" => {
                params.insert("room_size".into(), 0.55);
                params.insert("damping".into(), 0.4);
                params.insert("mix".into(), 0.25);
            }
            "radio_voice" => {
                params.insert("drive".into(), 2.5);
                params.insert("mix".into(), 0.85);
            }
            _ => {}
        }
        Self {
            id: Uuid::new_v4(),
            effect_type,
            enabled: true,
            params,
        }
    }
}

/// 字幕リッチ装飾スタイル（縁取り・ドロップシャドウ・座布団背景）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    #[serde(default)]
    pub outline_width: f32,
    #[serde(default = "default_black_color")]
    pub outline_color: RgbaColor,
    #[serde(default)]
    pub shadow_offset: [f32; 2],
    #[serde(default)]
    pub shadow_blur: f32,
    #[serde(default = "default_shadow_color")]
    pub shadow_color: RgbaColor,
    #[serde(default)]
    pub box_enabled: bool,
    #[serde(default = "default_box_color")]
    pub box_color: RgbaColor,
    #[serde(default = "default_box_padding")]
    pub box_padding: [f32; 2],
    #[serde(default = "default_box_radius")]
    pub box_radius: f32,
}

fn default_black_color() -> RgbaColor {
    RgbaColor::BLACK
}

fn default_shadow_color() -> RgbaColor {
    RgbaColor {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.65,
    }
}

fn default_box_color() -> RgbaColor {
    RgbaColor {
        r: 0.05,
        g: 0.06,
        b: 0.09,
        a: 0.72,
    }
}

fn default_box_padding() -> [f32; 2] {
    [16.0, 10.0]
}

fn default_box_radius() -> f32 {
    10.0
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            outline_width: 0.0,
            outline_color: default_black_color(),
            shadow_offset: [0.0, 0.0],
            shadow_blur: 0.0,
            shadow_color: default_shadow_color(),
            box_enabled: false,
            box_color: default_box_color(),
            box_padding: default_box_padding(),
            box_radius: default_box_radius(),
        }
    }
}

/// 動画・音声の再生速度・ループ・ソース尺設定（AviUtl2 再生速度＆時間制御相当）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSettings {
    /// 再生倍率（正: 通常/倍速/スロー、負: 逆再生。0.0 は静止）
    #[serde(default = "default_one_f32")]
    pub speed: f32,
    #[serde(default)]
    pub loop_playback: bool,
    /// 既知の元素材長（ループ折り返し・逆再生基準に使用）
    #[serde(default)]
    pub source_duration: Option<RationalTime>,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            speed: 1.0,
            loop_playback: false,
            source_duration: None,
        }
    }
}

/// AviUtl2 シーンチェンジ（トランジション）種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    CrossFade,
    WipeLeft,
    WipeRight,
    WipeUp,
    WipeDown,
    SlideLeft,
    SlideRight,
    Flash,
    RandomLines,
}

impl TransitionKind {
    pub const ALL: [Self; 9] = [
        Self::CrossFade,
        Self::WipeLeft,
        Self::WipeRight,
        Self::WipeUp,
        Self::WipeDown,
        Self::SlideLeft,
        Self::SlideRight,
        Self::Flash,
        Self::RandomLines,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::CrossFade => "クロスフェード",
            Self::WipeLeft => "ワイプ (左→右)",
            Self::WipeRight => "ワイプ (右→左)",
            Self::WipeUp => "ワイプ (上→下)",
            Self::WipeDown => "ワイプ (下→上)",
            Self::SlideLeft => "スライドイン (左)",
            Self::SlideRight => "スライドイン (右)",
            Self::Flash => "発光 (フラッシュ)",
            Self::RandomLines => "ランダムライン",
        }
    }
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
    RoundedRectangle,
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
        #[serde(default)]
        animator: TextAnimator,
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
    /// AviUtl2風シーンチェンジ（トランジションオブジェクト）
    Transition {
        transition: TransitionKind,
        scope: FilterTargetScope,
        #[serde(default = "default_transition_softness")]
        softness: f32,
    },
    /// AviUtl2風グループ制御（指定レイヤー数分の位置・拡大率・回転・不透明度を一括制御）
    GroupControl {
        target_layer_count: usize,
    },
    /// Phase 3: PNG差分立ち絵オブジェクト（音量連動／母音連動自動口パク）
    CharacterSprite {
        config: CharacterSpriteConfig,
        width: f32,
        height: f32,
    },
    /// v0.6 #4: オーディオスペクトラム ビジュアライザー（音声レイヤーの周波数帯域バー描画）
    AudioSpectrum {
        linked_audio_layer: usize,
        width: f32,
        height: f32,
        bar_count: usize,
        color: RgbaColor,
        peak_color: RgbaColor,
        sensitivity: f32,
    },
    Precomp {
        scene_id: SceneId,
    },
}

/// オーディオリアクティブ連動対象の周波数帯域
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioReactiveBand {
    #[default]
    Rms,
    Low,
    Mid,
    High,
}

/// v0.6 #4: 音声エネルギーに応じて拡大率・Yバウンス・不透明度を自動連動する設定
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioReactiveConfig {
    pub enabled: bool,
    pub linked_audio_layer: usize,
    pub band: AudioReactiveBand,
    pub sensitivity: f32,
    /// エネルギーに応じた拡大率加算量（例: 0.25 で最大 +25% 拡大）
    pub scale_amount: f32,
    /// エネルギーに応じた Y 座標バウンス（px、負値で上方向へ跳ねる）
    pub y_bounce_px: f32,
    /// エネルギーに応じた不透明度ブースト
    pub opacity_amount: f32,
}

impl Default for AudioReactiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            linked_audio_layer: 3,
            band: AudioReactiveBand::Low,
            sensitivity: 2.0,
            scale_amount: 0.18,
            y_bounce_px: -24.0,
            opacity_amount: 0.0,
        }
    }
}

/// v0.6 #4: 発話レイヤー（サイドチェイン）に応じて BGM レイヤーを自動減衰するダッキング設定
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioDuckingConfig {
    pub enabled: bool,
    /// 発話（声）が入っているサイドチェイン入力レイヤー
    pub voice_layer_index: usize,
    /// 自動減衰させる BGM レイヤー
    pub bgm_layer_index: usize,
    /// 発話検出の RMS しきい値（既定 0.03）
    pub threshold_rms: f32,
    /// 減衰量 dB（負値、既定 -12.0 dB）
    pub reduction_db: f32,
    /// アタック時間（秒、既定 0.03s）
    pub attack_sec: f32,
    /// リリース時間（秒、既定 0.25s）
    pub release_sec: f32,
}

impl Default for AudioDuckingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            voice_layer_index: 3,
            bgm_layer_index: 2,
            threshold_rms: 0.03,
            reduction_db: -12.0,
            attack_sec: 0.03,
            release_sec: 0.25,
        }
    }
}

/// v0.7 #8: ベジェマスクの合成モード（加算／減算）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    #[default]
    Add,
    Subtract,
}

/// v0.7 #8: 自由ベジェマスクの頂点（アイテムローカル正規化座標 0.0..=1.0）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaskVertex {
    pub x: f32,
    pub y: f32,
    /// 頂点からの相対インハンドル（[0, 0] で鋭角コーナー）
    pub in_handle: [f32; 2],
    /// 頂点からの相対アウトハンドル（[0, 0] で鋭角コーナー）
    pub out_handle: [f32; 2],
}

impl MaskVertex {
    pub fn corner(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            in_handle: [0.0, 0.0],
            out_handle: [0.0, 0.0],
        }
    }
}

/// v0.7 #8: 閉曲線自由ベジェマスク（フェザー・反転・加算/減算対応）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BezierMask {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub mode: MaskMode,
    pub inverted: bool,
    pub feather_px: f32,
    pub opacity: f32,
    pub vertices: Vec<MaskVertex>,
}

impl BezierMask {
    /// 矩形プリセットマスク（0.15..0.85）
    pub fn rectangle() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "矩形マスク".to_string(),
            enabled: true,
            mode: MaskMode::Add,
            inverted: false,
            feather_px: 0.0,
            opacity: 1.0,
            vertices: vec![
                MaskVertex::corner(0.15, 0.15),
                MaskVertex::corner(0.85, 0.15),
                MaskVertex::corner(0.85, 0.85),
                MaskVertex::corner(0.15, 0.85),
            ],
        }
    }

    /// 楕円プリセットマスク（3次ベジェ曲線による正確な円近似 κ=0.55228475）
    pub fn ellipse() -> Self {
        let r = 0.38_f32;
        let k = 0.552_284_8 * r;
        Self {
            id: Uuid::new_v4(),
            name: "楕円マスク".to_string(),
            enabled: true,
            mode: MaskMode::Add,
            inverted: false,
            feather_px: 4.0,
            opacity: 1.0,
            vertices: vec![
                MaskVertex {
                    x: 0.5,
                    y: 0.5 - r,
                    in_handle: [-k, 0.0],
                    out_handle: [k, 0.0],
                },
                MaskVertex {
                    x: 0.5 + r,
                    y: 0.5,
                    in_handle: [0.0, -k],
                    out_handle: [0.0, k],
                },
                MaskVertex {
                    x: 0.5,
                    y: 0.5 + r,
                    in_handle: [k, 0.0],
                    out_handle: [-k, 0.0],
                },
                MaskVertex {
                    x: 0.5 - r,
                    y: 0.5,
                    in_handle: [0.0, k],
                    out_handle: [0.0, -k],
                },
            ],
        }
    }

    /// ひし形プリセットマスク
    pub fn diamond() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "ひし形マスク".to_string(),
            enabled: true,
            mode: MaskMode::Add,
            inverted: false,
            feather_px: 2.0,
            opacity: 1.0,
            vertices: vec![
                MaskVertex::corner(0.5, 0.10),
                MaskVertex::corner(0.90, 0.5),
                MaskVertex::corner(0.5, 0.90),
                MaskVertex::corner(0.10, 0.5),
            ],
        }
    }

    /// 3次ベジェ閉曲線をポリゴン頂点列（正規化座標）へ分割展開する
    pub fn flatten_polygon(&self, samples_per_segment: usize) -> Vec<[f32; 2]> {
        let n = self.vertices.len();
        if n < 3 {
            return Vec::new();
        }
        let steps = samples_per_segment.clamp(2, 64);
        let mut pts = Vec::with_capacity(n * steps);
        for i in 0..n {
            let v0 = self.vertices[i];
            let v1 = self.vertices[(i + 1) % n];
            let p0 = [v0.x, v0.y];
            let p1 = [v0.x + v0.out_handle[0], v0.y + v0.out_handle[1]];
            let p2 = [v1.x + v1.in_handle[0], v1.y + v1.in_handle[1]];
            let p3 = [v1.x, v1.y];
            for s in 0..steps {
                let t = s as f32 / steps as f32;
                let u = 1.0 - t;
                let b0 = u * u * u;
                let b1 = 3.0 * u * u * t;
                let b2 = 3.0 * u * t * t;
                let b3 = t * t * t;
                pts.push([
                    b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
                    b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
                ]);
            }
        }
        pts
    }
}

/// v0.7 #8: 1ポイント自動モーショントラッキング設定
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionTrackConfig {
    pub start_time: RationalTime,
    pub end_time: RationalTime,
    /// 何フレームおきに追跡サンプルを取得するか（既定 2 フレーム）
    pub step_frames: u32,
    /// 開始フレームでの追跡アンカー位置（シーン座標 px）
    pub anchor_x: f32,
    pub anchor_y: f32,
    /// テンプレートパッチ半径（px、既定 12）
    pub patch_radius: u32,
    /// フレーム間探索半径（px、既定 28）
    pub search_radius: u32,
}

impl Default for MotionTrackConfig {
    fn default() -> Self {
        Self {
            start_time: RationalTime::ZERO,
            end_time: RationalTime::from_seconds(2.0),
            step_frames: 2,
            anchor_x: 640.0,
            anchor_y: 360.0,
            patch_radius: 12,
            search_radius: 28,
        }
    }
}

/// v0.7 #8: 追跡された1サンプル点
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackPointSample {
    pub time: RationalTime,
    pub x: f32,
    pub y: f32,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: ItemId,
    pub name: String,
    pub start: RationalTime,
    pub duration: RationalTime,
    pub transform: Transform2D,
    /// Phase 3: 3D 空間配置（Z座標・3軸回転）
    #[serde(default)]
    pub transform_3d: Transform3D,
    #[serde(default)]
    pub blend_mode: BlendMode,
    /// プロパティ名（"x", "y", "scale_x", "scale_y", "rotation_deg", "opacity", "z", "time_remap"）ごとのキーフレーム列
    #[serde(default)]
    pub keyframes: BTreeMap<String, Vec<Keyframe>>,
    /// TTS / 文字起こし由来の音素タイミング列（立ち絵の母音連動口パク参照用）
    #[serde(default)]
    pub phoneme_segments: Vec<PhonemeSegment>,
    /// オブジェクト個別のAE風エフェクトチェーン
    pub effect_chain: Vec<EffectNode>,
    /// 動画・音声の再生速度・ループ・時間制御設定
    #[serde(default)]
    pub playback: PlaybackSettings,
    /// 字幕のリッチ装飾スタイル（縁取り・シャドウ・座布団背景）
    #[serde(default)]
    pub text_style: TextStyle,
    /// クリップ単位の音声 DSP エフェクトチェーン
    #[serde(default)]
    pub audio_effects: Vec<AudioEffectNode>,
    /// v0.6 #4: オーディオリアクティブ（音声エネルギー連動アニメーション）
    #[serde(default)]
    pub audio_reactive: AudioReactiveConfig,
    /// v0.7 #8: 自由ベジェマスク列（加算・減算・フェザー・反転）
    #[serde(default)]
    pub masks: Vec<BezierMask>,
    pub kind: ItemKind,
}

impl TimelineItem {
    pub fn new(
        name: impl Into<String>,
        start: RationalTime,
        duration: RationalTime,
        kind: ItemKind,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            start,
            duration,
            transform: Transform2D::default(),
            transform_3d: Transform3D::default(),
            blend_mode: BlendMode::Normal,
            keyframes: BTreeMap::new(),
            phoneme_segments: Vec::new(),
            effect_chain: Vec::new(),
            playback: PlaybackSettings::default(),
            text_style: TextStyle::default(),
            audio_effects: Vec::new(),
            audio_reactive: AudioReactiveConfig::default(),
            masks: Vec::new(),
            kind,
        }
    }

    pub fn end(&self) -> RationalTime {
        self.start + self.duration
    }

    pub fn contains(&self, t: RationalTime) -> bool {
        t >= self.start && t < self.end()
    }

    /// 再生速度・逆再生・ループ・`time_remap` キーフレームを考慮して元素材の参照時刻（秒）を算出する
    pub fn evaluate_source_time_sec(
        &self,
        local_time: RationalTime,
        source_offset: RationalTime,
        fallback_source_dur_sec: Option<f64>,
    ) -> f64 {
        let loop_len = self
            .playback
            .source_duration
            .map(RationalTime::as_seconds)
            .or(fallback_source_dur_sec)
            .filter(|d| *d > 1e-4);

        let raw_sec = if let Some(kfs) = self.keyframes.get("time_remap")
            && !kfs.is_empty()
        {
            f64::from(evaluate_track(
                (local_time + source_offset).as_seconds() as f32,
                kfs,
                local_time,
            ))
        } else {
            let speed = f64::from(self.playback.speed);
            if speed < 0.0 {
                let anchor = loop_len.unwrap_or_else(|| self.duration.as_seconds().max(0.0));
                (anchor - local_time.as_seconds() * speed.abs()) + source_offset.as_seconds()
            } else {
                source_offset.as_seconds() + local_time.as_seconds() * speed
            }
        };

        if self.playback.loop_playback
            && let Some(len) = loop_len
        {
            raw_sec.rem_euclid(len)
        } else {
            raw_sec.max(0.0)
        }
    }

    /// 指定グローバル時刻でのアニメーション済み Transform2D を評価する
    pub fn eval_transform(&self, global_time: RationalTime) -> Transform2D {
        let local = global_time - self.start;
        let eval = |prop: &str, base: f32| -> f32 {
            self.keyframes
                .get(prop)
                .map(|kfs| evaluate_track(base, kfs, local))
                .unwrap_or(base)
        };
        Transform2D {
            x: eval("x", self.transform.x),
            y: eval("y", self.transform.y),
            scale_x: eval("scale_x", self.transform.scale_x),
            scale_y: eval("scale_y", self.transform.scale_y),
            rotation_deg: eval("rotation_deg", self.transform.rotation_deg),
            opacity: eval("opacity", self.transform.opacity).clamp(0.0, 1.0),
        }
    }

    /// 指定グローバル時刻でのアニメーション済み Transform3D を評価する
    pub fn eval_transform_3d(&self, global_time: RationalTime) -> Transform3D {
        let local = global_time - self.start;
        let eval = |prop: &str, base: f32| -> f32 {
            self.keyframes
                .get(prop)
                .map(|kfs| evaluate_track(base, kfs, local))
                .unwrap_or(base)
        };
        Transform3D {
            enable_3d: self.transform_3d.enable_3d,
            z: eval("z", self.transform_3d.z),
            rotation_x_deg: eval("rotation_x_deg", self.transform_3d.rotation_x_deg),
            rotation_y_deg: eval("rotation_y_deg", self.transform_3d.rotation_y_deg),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub muted: bool,
    /// AviUtl2 準拠のレイヤー一括 X/Y オフセット
    #[serde(default)]
    pub offset_x: f32,
    #[serde(default)]
    pub offset_y: f32,
    /// AviUtl2 準拠のレイヤー一括音量倍率（既定 1.0）
    #[serde(default = "default_one_f32")]
    pub volume: f32,
    /// レイヤー識別用のカラータグ
    #[serde(default)]
    pub color_tag: Option<RgbaColor>,
    #[serde(default)]
    pub track_matte: Option<TrackMatte>,
    pub items: Vec<TimelineItem>,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            visible: true,
            muted: false,
            offset_x: 0.0,
            offset_y: 0.0,
            volume: 1.0,
            color_tag: None,
            track_matte: None,
            items: Vec::new(),
        }
    }
}

/// タイムラインマーカー（`M` キー追加・クリックジャンプ・YouTube チャプター自動出力対応）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineMarker {
    pub id: Uuid,
    pub time: RationalTime,
    pub label: String,
    #[serde(default = "default_marker_color")]
    pub color: RgbaColor,
}

fn default_marker_color() -> RgbaColor {
    RgbaColor {
        r: 0.25,
        g: 0.85,
        b: 0.55,
        a: 1.0,
    }
}

impl TimelineMarker {
    pub fn new(time: RationalTime, label: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            time: time.max(RationalTime::ZERO),
            label: label.into(),
            color: default_marker_color(),
        }
    }
}

/// v0.8 #13: 非破壊シーンカラーグレーディング（3-Way Lift/Gamma/Gain・色温度/ティント・.cube 3D LUT）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneColorGrade {
    pub enabled: bool,
    /// 露出補正 (EV: -3.0..=3.0)
    pub exposure: f32,
    /// コントラスト (0.2..=2.5, 既定 1.0)
    pub contrast: f32,
    /// 彩度 (0.0..=2.5, 既定 1.0)
    pub saturation: f32,
    /// 色温度 (-1.0 青〜+1.0 暖色)
    pub temperature: f32,
    /// ティント (-1.0 緑〜+1.0 マゼンタ)
    pub tint: f32,
    /// 3-Way カラーホイール Lift（シャドウ RGB オフセット -0.5..=0.5）
    pub lift: [f32; 3],
    /// 3-Way カラーホイール Gamma（ミッドトーン RGB オフセット -0.5..=0.5）
    pub gamma: [f32; 3],
    /// 3-Way カラーホイール Gain（ハイライト RGB 乗算 0.2..=2.5, 既定 [1,1,1]）
    pub gain: [f32; 3],
    /// 適用する `.cube` 3D LUT（ファイルパスまたは `builtin:<id>`）
    #[serde(default)]
    pub lut_path: Option<String>,
    /// 3D LUT 適用強度 (0.0..=1.0)
    #[serde(default = "default_one_f32")]
    pub lut_intensity: f32,
}

impl Default for SceneColorGrade {
    fn default() -> Self {
        Self {
            enabled: false,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            lift: [0.0, 0.0, 0.0],
            gamma: [0.0, 0.0, 0.0],
            gain: [1.0, 1.0, 1.0],
            lut_path: None,
            lut_intensity: 1.0,
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
    #[serde(default)]
    pub camera: Camera3D,
    /// シーンマスター音声 DSP エフェクトチェーン
    #[serde(default)]
    pub master_audio_effects: Vec<AudioEffectNode>,
    /// 書き出し時の目標ラウドネス正規化（例: `Some(-14.0)` で YouTube 基準 -14 LUFS）
    #[serde(default)]
    pub target_lufs: Option<f32>,
    /// v0.6 #4: 発話レイヤー連動 BGM 自動ダッキング設定
    #[serde(default)]
    pub audio_ducking: AudioDuckingConfig,
    /// v0.8 #13: シーン全体のカラーグレーディング＆`.cube` 3D LUT 設定
    #[serde(default)]
    pub color_grade: SceneColorGrade,
    /// タイムラインマーカー一覧（YouTube チャプター生成対応）
    #[serde(default)]
    pub markers: Vec<TimelineMarker>,
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
            camera: Camera3D::default(),
            master_audio_effects: Vec::new(),
            target_lufs: None,
            audio_ducking: AudioDuckingConfig::default(),
            color_grade: SceneColorGrade::default(),
            markers: Vec::new(),
            layers: vec![
                Layer::new("Layer 1"),
                Layer::new("Layer 2"),
                Layer::new("Layer 3"),
                Layer::new("Audio 1"),
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

/// 動画サイズ・アスペクト比プリセット（横動画 / 4K / YouTube Shorts・TikTok 縦動画 / 正方形）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePreset {
    Hd720,
    FullHd1080,
    Uhd4k,
    Shorts9x16,
    Square1x1,
}

impl ScenePreset {
    pub const ALL: &'static [Self] = &[
        Self::Hd720,
        Self::FullHd1080,
        Self::Uhd4k,
        Self::Shorts9x16,
        Self::Square1x1,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Hd720 => "HD 720p (1280×720 / 16:9)",
            Self::FullHd1080 => "Full HD 1080p (1920×1080 / 16:9)",
            Self::Uhd4k => "4K UHD (3840×2160 / 16:9)",
            Self::Shorts9x16 => "📱 Shorts / TikTok 縦 (1080×1920 / 9:16)",
            Self::Square1x1 => "⬜ 正方形 SNS (1080×1080 / 1:1)",
        }
    }

    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Hd720 => (1280, 720),
            Self::FullHd1080 => (1920, 1080),
            Self::Uhd4k => (3840, 2160),
            Self::Shorts9x16 => (1080, 1920),
            Self::Square1x1 => (1080, 1080),
        }
    }

    pub fn default_fps(self) -> FrameRate {
        match self {
            Self::Uhd4k => FrameRate::FPS_30,
            _ => FrameRate::FPS_60,
        }
    }

    pub fn is_vertical(self) -> bool {
        let (w, h) = self.dimensions();
        h > w
    }
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
    /// AviUtl2 名前付きカラーパレット
    #[serde(default)]
    pub color_palette: ColorPalette,
    /// お気に入りフォント一覧（フォント視覚化ピッカーでピン留め表示）
    #[serde(default)]
    pub favorite_fonts: Vec<String>,
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
            color_palette: ColorPalette::default(),
            favorite_fonts: Vec::new(),
        }
    }

    pub fn scene(&self, id: SceneId) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    pub fn active_scene(&self) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == self.active_scene_id)
    }

    pub fn active_scene_mut(&mut self) -> Option<&mut Scene> {
        self.scenes
            .iter_mut()
            .find(|s| s.id == self.active_scene_id)
    }

    pub fn asset(&self, id: AssetId) -> Option<&AssetRef> {
        self.assets.iter().find(|a| a.id == id)
    }
}
