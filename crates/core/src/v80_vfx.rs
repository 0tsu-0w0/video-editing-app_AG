//! v8.0 #49 / #51 / #52 コアデータモデル:
//! - #49: 4 点平面ホモグラフィトラッカー (`PlanarTrackConfig` / `PlanarTrackResult`) ＆
//!   エッジ吸着キーフレームロトブラシ (`RotoBrushConfig` / `RotoKeyframe`)
//! - #51: シネマ光学 VFX (`OpticalVfxConfig`: 擬似深度ボケ Rack Focus・ゴッドレイ・Brown-Conrady レンズ歪み＆物理色収差)
//! - #52: ADR カウント収録セッション (`AdrSessionConfig` / `AdrCue` / `AdrTake`) ＆
//!   5.1ch サラウンド / FOA Ambisonics (AmbiX) 空間音声バス (`SpatialBusConfig`)
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロ。

use crate::animation::{Interpolation, Keyframe};
use crate::command::EditCommand;
use crate::project::{
    AssetId, BezierMask, CornerPinConfig, ItemId, MaskMode, MaskVertex, RgbaColor,
};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// #49: 4 点平面トラッカー（Planar Homography Tracker）＆ キーフレームロトブラシ
// ============================================================================

fn default_planar_quad() -> [[f32; 2]; 4] {
    [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]]
}

fn default_step_frames() -> u32 {
    2
}

fn default_patch_radius() -> u32 {
    8
}

fn default_search_radius() -> u32 {
    18
}

fn default_ransac_iters() -> u32 {
    64
}

fn default_ransac_inlier_px() -> f32 {
    2.5
}

/// 4 点平面ホモグラフィ追跡の設定（正規化シーン座標 0..=1）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanarTrackConfig {
    pub start_time: RationalTime,
    pub end_time: RationalTime,
    #[serde(default = "default_step_frames")]
    pub step_frames: u32,
    /// 開始フレームにおける追跡平面の 4 隅 `[TL, TR, BR, BL]`（正規化 0..=1）
    #[serde(default = "default_planar_quad")]
    pub initial_quad: [[f32; 2]; 4],
    /// 各パッチ半径 (px)
    #[serde(default = "default_patch_radius")]
    pub patch_radius: u32,
    /// フレーム間探索半径 (px)
    #[serde(default = "default_search_radius")]
    pub search_radius: u32,
    /// RANSAC 反復回数
    #[serde(default = "default_ransac_iters")]
    pub ransac_iterations: u32,
    /// RANSAC インライア再投影誤差閾値 (px)
    #[serde(default = "default_ransac_inlier_px")]
    pub ransac_inlier_px: f32,
}

impl Default for PlanarTrackConfig {
    fn default() -> Self {
        Self {
            start_time: RationalTime::ZERO,
            end_time: RationalTime::from_seconds(2.0),
            step_frames: default_step_frames(),
            initial_quad: default_planar_quad(),
            patch_radius: default_patch_radius(),
            search_radius: default_search_radius(),
            ransac_iterations: default_ransac_iters(),
            ransac_inlier_px: default_ransac_inlier_px(),
        }
    }
}

/// 1 サンプル時刻における平面追跡結果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanarTrackFrame {
    pub time: RationalTime,
    /// 追跡された平面 4 隅 `[TL, TR, BR, BL]`（正規化 0..=1）
    pub quad: [[f32; 2]; 4],
    /// 開始フレームからの累積 3×3 ホモグラフィ行列（行優先）
    pub homography: [f64; 9],
    /// RANSAC インライアパッチ数
    pub inlier_count: usize,
    /// 0.0..=1.0 の追跡信頼度
    pub confidence: f32,
}

/// 4 点平面ホモグラフィ追跡の全体レポート
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanarTrackResult {
    pub frames: Vec<PlanarTrackFrame>,
    pub mean_confidence: f32,
    pub mean_inliers: f32,
}

/// 平面追跡結果から対象アイテムの `CornerPinConfig` と 8 本のキーフレーム列（`pin_tl_x`〜`pin_bl_y`）を
/// 1-Undo `EditCommand::Batch` として生成する。
pub fn build_planar_track_corner_pin_command(
    item_id: ItemId,
    item_start: RationalTime,
    result: &PlanarTrackResult,
    feather_px: f32,
) -> Option<EditCommand> {
    let first = result.frames.first()?;
    let mut cmds = Vec::new();
    cmds.push(EditCommand::SetCornerPin {
        item_id,
        corner_pin: CornerPinConfig {
            enabled: true,
            tl: first.quad[0],
            tr: first.quad[1],
            br: first.quad[2],
            bl: first.quad[3],
            feather_px: feather_px.clamp(0.0, 16.0),
        },
    });

    const PROP_NAMES: [&str; 8] = [
        "pin_tl_x", "pin_tl_y", "pin_tr_x", "pin_tr_y", "pin_br_x", "pin_br_y", "pin_bl_x",
        "pin_bl_y",
    ];
    for f in &result.frames {
        let local_t = (f.time - item_start).max(RationalTime::ZERO);
        let vals = [
            f.quad[0][0],
            f.quad[0][1],
            f.quad[1][0],
            f.quad[1][1],
            f.quad[2][0],
            f.quad[2][1],
            f.quad[3][0],
            f.quad[3][1],
        ];
        for (prop, &v) in PROP_NAMES.iter().zip(vals.iter()) {
            cmds.push(EditCommand::SetKeyframe {
                item_id,
                property: (*prop).to_string(),
                keyframe: Keyframe {
                    time: local_t,
                    value: v,
                    interpolation: Interpolation::Linear,
                },
            });
        }
    }

    Some(EditCommand::Batch {
        label: format!("bake_planar_track_{}f", result.frames.len()),
        commands: cmds,
    })
}

fn default_roto_feather() -> f32 {
    2.0
}

fn default_roto_snap_radius() -> u32 {
    10
}

fn default_roto_edge_weight() -> f32 {
    0.75
}

/// エッジ吸着キーフレームロトブラシの 1 キーフレーム輪郭
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotoKeyframe {
    /// クリップ内ローカル時刻
    pub time: RationalTime,
    /// 正規化クリップ座標 (0..=1) の閉ベジェ頂点列
    pub vertices: Vec<MaskVertex>,
    #[serde(default = "default_roto_feather")]
    pub feather_px: f32,
    #[serde(default)]
    pub expansion_px: f32,
}

/// 非破壊キーフレームロトブラシ（エッジ吸着＋オプティカルフロー伝播対応）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotoBrushConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub invert: bool,
    #[serde(default)]
    pub mode: MaskMode,
    /// Sobel エッジ吸着探索半径 (px)
    #[serde(default = "default_roto_snap_radius")]
    pub edge_snap_radius_px: u32,
    /// エッジ勾配強度と距離ペナルティのバランス (0.1..=1.0)
    #[serde(default = "default_roto_edge_weight")]
    pub edge_weight: f32,
    /// 時刻順に並んだロト輪郭キーフレーム列
    #[serde(default)]
    pub keyframes: Vec<RotoKeyframe>,
}

impl Default for RotoBrushConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            invert: false,
            mode: MaskMode::Add,
            edge_snap_radius_px: default_roto_snap_radius(),
            edge_weight: default_roto_edge_weight(),
            keyframes: Vec::new(),
        }
    }
}

impl RotoBrushConfig {
    /// デフォルトの楕円ロト初期輪郭（8 頂点）を生成する
    pub fn default_ellipse_vertices(
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        count: usize,
    ) -> Vec<MaskVertex> {
        let n = count.clamp(4, 32);
        let handle_len = (4.0 / 3.0) * (std::f32::consts::PI / (2.0 * n as f32)).tan();
        let mut verts = Vec::with_capacity(n);
        for i in 0..n {
            let theta = (i as f32 / n as f32) * std::f32::consts::TAU;
            let cos_t = theta.cos();
            let sin_t = theta.sin();
            let x = (cx + rx * cos_t).clamp(0.01, 0.99);
            let y = (cy + ry * sin_t).clamp(0.01, 0.99);
            let tx = -rx * sin_t * handle_len;
            let ty = ry * cos_t * handle_len;
            verts.push(MaskVertex {
                x,
                y,
                in_handle: [-tx, -ty],
                out_handle: [tx, ty],
            });
        }
        verts
    }

    /// クリップ内ローカル時刻 `local_time` における補間済み `BezierMask` を評価する
    pub fn evaluate_mask_at(&self, local_time: RationalTime) -> Option<BezierMask> {
        if !self.enabled || self.keyframes.is_empty() {
            return None;
        }
        let mut sorted: Vec<&RotoKeyframe> = self.keyframes.iter().collect();
        sorted.sort_by_key(|k| k.time);

        if sorted.len() == 1 || local_time <= sorted[0].time {
            return Some(self.kf_to_mask(sorted[0]));
        }
        let last = *sorted.last()?;
        if local_time >= last.time {
            return Some(self.kf_to_mask(last));
        }

        for pair in sorted.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if local_time >= a.time && local_time <= b.time {
                let span = (b.time.ticks - a.time.ticks).max(1) as f64;
                let t = ((local_time.ticks - a.time.ticks) as f64 / span).clamp(0.0, 1.0) as f32;
                let feather = a.feather_px + (b.feather_px - a.feather_px) * t;
                let expansion = a.expansion_px + (b.expansion_px - a.expansion_px) * t;
                let verts = expand_roto_vertices(
                    &interpolate_roto_vertices(&a.vertices, &b.vertices, t),
                    expansion,
                );
                return Some(BezierMask {
                    id: Uuid::nil(),
                    name: "RotoBrush".to_string(),
                    enabled: true,
                    mode: self.mode,
                    inverted: self.invert,
                    feather_px: feather.max(0.0),
                    opacity: 1.0,
                    vertices: verts,
                });
            }
        }
        Some(self.kf_to_mask(last))
    }

    fn kf_to_mask(&self, kf: &RotoKeyframe) -> BezierMask {
        BezierMask {
            id: Uuid::nil(),
            name: "RotoBrush".to_string(),
            enabled: true,
            mode: self.mode,
            inverted: self.invert,
            feather_px: kf.feather_px.max(0.0),
            opacity: 1.0,
            vertices: expand_roto_vertices(&kf.vertices, kf.expansion_px),
        }
    }
}

fn expand_roto_vertices(verts: &[MaskVertex], expansion_px: f32) -> Vec<MaskVertex> {
    if verts.is_empty() || expansion_px.abs() < 1e-4 {
        return verts.to_vec();
    }
    let inv = 1.0 / verts.len() as f32;
    let cx = verts.iter().map(|v| v.x).sum::<f32>() * inv;
    let cy = verts.iter().map(|v| v.y).sum::<f32>() * inv;
    let delta = (expansion_px / 512.0).clamp(-0.25, 0.25);
    verts
        .iter()
        .map(|v| {
            let dx = v.x - cx;
            let dy = v.y - cy;
            let len = dx.hypot(dy).max(1e-4);
            MaskVertex {
                x: (v.x + (dx / len) * delta).clamp(0.0, 1.0),
                y: (v.y + (dy / len) * delta).clamp(0.0, 1.0),
                in_handle: v.in_handle,
                out_handle: v.out_handle,
            }
        })
        .collect()
}

fn interpolate_roto_vertices(a: &[MaskVertex], b: &[MaskVertex], t: f32) -> Vec<MaskVertex> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    let n = a.len().max(b.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let va = &a[i * a.len() / n];
        let vb = &b[i * b.len() / n];
        out.push(MaskVertex {
            x: va.x + (vb.x - va.x) * t,
            y: va.y + (vb.y - va.y) * t,
            in_handle: [
                va.in_handle[0] + (vb.in_handle[0] - va.in_handle[0]) * t,
                va.in_handle[1] + (vb.in_handle[1] - va.in_handle[1]) * t,
            ],
            out_handle: [
                va.out_handle[0] + (vb.out_handle[0] - va.out_handle[0]) * t,
                va.out_handle[1] + (vb.out_handle[1] - va.out_handle[1]) * t,
            ],
        });
    }
    out
}

// ============================================================================
// #51: シネマ光学 VFX（擬似深度ボケ Rack Focus・ゴッドレイ・Brown-Conrady レンズ歪み＆色収差）
// ============================================================================

/// 擬似深度マップの生成モード
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PseudoDepthMode {
    /// 縦方向チルトシフト深度（手前=下端 `0.0` → 奥=上端 `1.0`）
    #[default]
    VerticalTiltShift,
    /// 中央被写体フォーカス（中心 `0.0` → 周辺 `1.0`）
    RadialCenter,
    /// 輝度・コントラスト顕著性に基づく擬似深度（高輝度/高エッジ=手前）
    LuminanceSaliency,
}

impl PseudoDepthMode {
    pub const ALL: &'static [Self] = &[
        Self::VerticalTiltShift,
        Self::RadialCenter,
        Self::LuminanceSaliency,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::VerticalTiltShift => "チルトシフト深度 (手前下→奥上)",
            Self::RadialCenter => "中央被写体 (中心→周辺)",
            Self::LuminanceSaliency => "輝度・エッジ顕著性深度",
        }
    }
}

/// シネマ光学 VFX ワンクリックプリセット
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpticalVfxPreset {
    AnamorphicRackFocus,
    CathedralGodRays,
    ActionCamDeFish,
    VintagePrismLens,
}

impl OpticalVfxPreset {
    pub const ALL: &'static [Self] = &[
        Self::AnamorphicRackFocus,
        Self::CathedralGodRays,
        Self::ActionCamDeFish,
        Self::VintagePrismLens,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::AnamorphicRackFocus => "🎥 シネマ Rack Focus 玉ボケ",
            Self::CathedralGodRays => "☀ 大聖堂ゴッドレイ体積光",
            Self::ActionCamDeFish => "📷 アクションカム魚眼歪み補正 (De-Fish)",
            Self::VintagePrismLens => "🌈 ヴィンテージ広角レンズ＆物理色収差",
        }
    }
}

fn default_focus_depth() -> f32 {
    0.35
}
fn default_focus_band() -> f32 {
    0.16
}
fn default_bokeh_radius() -> f32 {
    8.0
}
fn default_bokeh_highlight() -> f32 {
    1.2
}
fn default_bokeh_blades() -> u32 {
    6
}
fn default_light_pos() -> [f32; 2] {
    [0.5, 0.15]
}
fn default_god_rays_intensity() -> f32 {
    0.65
}
fn default_god_rays_decay() -> f32 {
    0.94
}
fn default_god_rays_threshold() -> f32 {
    0.55
}
fn default_god_rays_samples() -> u32 {
    28
}
fn default_god_rays_tint() -> RgbaColor {
    RgbaColor {
        r: 1.0,
        g: 0.90,
        b: 0.68,
        a: 1.0,
    }
}
fn default_true() -> bool {
    true
}

/// シーン単位の非破壊シネマ光学 VFX 設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpticalVfxConfig {
    #[serde(default)]
    pub enabled: bool,
    // --- 擬似深度ボケ Rack Focus ---
    #[serde(default)]
    pub bokeh_enabled: bool,
    #[serde(default)]
    pub depth_mode: PseudoDepthMode,
    /// 合焦深度 (`0.0..=1.0`)
    #[serde(default = "default_focus_depth")]
    pub focus_depth: f32,
    /// ピントが合う深度幅 (`0.02..=0.5`)
    #[serde(default = "default_focus_band")]
    pub focus_band: f32,
    /// 最大錯乱円ボケ半径 (`0.0..=20.0` px)
    #[serde(default = "default_bokeh_radius")]
    pub bokeh_radius_px: f32,
    /// ハイライト玉ボケ強調 (`0.0..=3.0`)
    #[serde(default = "default_bokeh_highlight")]
    pub bokeh_highlight_boost: f32,
    /// 絞り羽根枚数 (`0`=真円、`5..=8`=多角形ボケ)
    #[serde(default = "default_bokeh_blades")]
    pub bokeh_blades: u32,
    // --- ゴッドレイ（体積光散乱） ---
    #[serde(default)]
    pub god_rays_enabled: bool,
    /// 光源スクリーン位置（正規化 0..=1）
    #[serde(default = "default_light_pos")]
    pub light_pos: [f32; 2],
    #[serde(default = "default_god_rays_intensity")]
    pub god_rays_intensity: f32,
    #[serde(default = "default_god_rays_decay")]
    pub god_rays_decay: f32,
    #[serde(default = "default_god_rays_threshold")]
    pub god_rays_threshold: f32,
    #[serde(default = "default_god_rays_samples")]
    pub god_rays_samples: u32,
    #[serde(default = "default_god_rays_tint")]
    pub god_rays_tint: RgbaColor,
    // --- Brown-Conrady レンズ歪み補正＆物理色収差 ---
    #[serde(default)]
    pub lens_enabled: bool,
    /// 放射歪み 2 次係数 $k_1$（負=樽型補正/糸巻き化、正=糸巻き補正/樽型化）
    #[serde(default)]
    pub k1: f32,
    /// 放射歪み 4 次係数 $k_2$
    #[serde(default)]
    pub k2: f32,
    /// 接線歪み係数 $p_1$
    #[serde(default)]
    pub p1: f32,
    /// 接線歪み係数 $p_2$
    #[serde(default)]
    pub p2: f32,
    /// 歪み補正時の周辺黒枠を自動ズーム補償するか
    #[serde(default = "default_true")]
    pub lens_zoom_compensate: bool,
    /// 物理波長分散色収差強度 (`-0.05..=0.05`)
    #[serde(default)]
    pub chromatic_aberration: f32,
}

impl Default for OpticalVfxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bokeh_enabled: false,
            depth_mode: PseudoDepthMode::VerticalTiltShift,
            focus_depth: default_focus_depth(),
            focus_band: default_focus_band(),
            bokeh_radius_px: default_bokeh_radius(),
            bokeh_highlight_boost: default_bokeh_highlight(),
            bokeh_blades: default_bokeh_blades(),
            god_rays_enabled: false,
            light_pos: default_light_pos(),
            god_rays_intensity: default_god_rays_intensity(),
            god_rays_decay: default_god_rays_decay(),
            god_rays_threshold: default_god_rays_threshold(),
            god_rays_samples: default_god_rays_samples(),
            god_rays_tint: default_god_rays_tint(),
            lens_enabled: false,
            k1: 0.0,
            k2: 0.0,
            p1: 0.0,
            p2: 0.0,
            lens_zoom_compensate: true,
            chromatic_aberration: 0.0,
        }
    }
}

impl OpticalVfxConfig {
    pub fn from_preset(preset: OpticalVfxPreset) -> Self {
        match preset {
            OpticalVfxPreset::AnamorphicRackFocus => Self {
                enabled: true,
                bokeh_enabled: true,
                depth_mode: PseudoDepthMode::RadialCenter,
                focus_depth: 0.18,
                focus_band: 0.15,
                bokeh_radius_px: 10.0,
                bokeh_highlight_boost: 1.8,
                bokeh_blades: 6,
                lens_enabled: true,
                k1: 0.04,
                chromatic_aberration: 0.008,
                ..Self::default()
            },
            OpticalVfxPreset::CathedralGodRays => Self {
                enabled: true,
                god_rays_enabled: true,
                light_pos: [0.5, 0.12],
                god_rays_intensity: 0.85,
                god_rays_decay: 0.95,
                god_rays_threshold: 0.45,
                god_rays_samples: 36,
                god_rays_tint: RgbaColor {
                    r: 1.0,
                    g: 0.88,
                    b: 0.62,
                    a: 1.0,
                },
                ..Self::default()
            },
            OpticalVfxPreset::ActionCamDeFish => Self {
                enabled: true,
                lens_enabled: true,
                k1: -0.26,
                k2: 0.06,
                p1: 0.0,
                p2: 0.0,
                lens_zoom_compensate: true,
                chromatic_aberration: 0.0,
                ..Self::default()
            },
            OpticalVfxPreset::VintagePrismLens => Self {
                enabled: true,
                bokeh_enabled: true,
                depth_mode: PseudoDepthMode::VerticalTiltShift,
                focus_depth: 0.48,
                focus_band: 0.18,
                bokeh_radius_px: 6.5,
                bokeh_highlight_boost: 1.35,
                bokeh_blades: 7,
                lens_enabled: true,
                k1: 0.12,
                k2: -0.02,
                lens_zoom_compensate: true,
                chromatic_aberration: 0.018,
                ..Self::default()
            },
        }
    }

    pub fn is_active(&self) -> bool {
        self.enabled && (self.bokeh_enabled || self.god_rays_enabled || self.lens_enabled)
    }
}

// ============================================================================
// #52: ADR カウント収録セッション ＆ 5.1ch / FOA Ambisonics (AmbiX) 空間音声バス
// ============================================================================

/// ADR アフレコの収録テイク記録
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdrTake {
    pub take_number: u32,
    pub asset_id: Option<AssetId>,
    pub label: String,
    pub rms_db: f32,
    /// 1..=5 評価（ベストテイク選定用）
    pub rating: u8,
}

fn default_adr_preroll() -> f64 {
    3.0
}

/// 1 台詞分の ADR キュー（開始・尺・プリロール・複数テイク）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdrCue {
    pub id: Uuid,
    pub cue_number: u32,
    pub character: String,
    pub line_text: String,
    pub start_time: RationalTime,
    pub duration: RationalTime,
    /// カウントダウン（3-2-1 ビープ＆映像ストリーマ）のプリロール秒数（既定 3.0s）
    #[serde(default = "default_adr_preroll")]
    pub preroll_sec: f64,
    #[serde(default)]
    pub takes: Vec<AdrTake>,
    #[serde(default)]
    pub selected_take: Option<usize>,
}

impl AdrCue {
    pub fn new(
        cue_number: u32,
        character: impl Into<String>,
        line_text: impl Into<String>,
        start_time: RationalTime,
        duration: RationalTime,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            cue_number,
            character: character.into(),
            line_text: line_text.into(),
            start_time: start_time.max(RationalTime::ZERO),
            duration: duration.max(RationalTime::from_seconds(0.2)),
            preroll_sec: default_adr_preroll(),
            takes: Vec::new(),
            selected_take: None,
        }
    }
}

/// シーン単位の ADR カウント収録セッション状態
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdrSessionConfig {
    #[serde(default)]
    pub enabled: bool,
    /// プレビューへ 3-2-1 映像ストリーマ＆台詞ガイドを重ねるか
    #[serde(default = "default_true")]
    pub show_countdown_streamer: bool,
    /// プリロール時に 3-2-1 ビープ音（1kHz）を生成するか
    #[serde(default = "default_true")]
    pub beep_enabled: bool,
    #[serde(default)]
    pub cues: Vec<AdrCue>,
    #[serde(default)]
    pub active_cue_idx: usize,
}

impl Default for AdrSessionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            show_countdown_streamer: true,
            beep_enabled: true,
            cues: Vec::new(),
            active_cue_idx: 0,
        }
    }
}

/// 指定再生時刻において進行中の ADR プリロール／収録状態
#[derive(Debug, Clone, PartialEq)]
pub struct AdrCountdownState {
    pub cue_number: u32,
    pub character: String,
    pub line_text: String,
    /// プリロール中なら `Some((残り秒 3/2/1, 0.0..=1.0 進行率))`、収録中なら `None`
    pub countdown_beat: Option<(u32, f32)>,
    pub recording_active: bool,
    /// 収録区間内の進行率 `0.0..=1.0`
    pub record_progress: f32,
}

impl AdrSessionConfig {
    /// 時刻 `time` における映像ストリーマ表示用の ADR 状態を評価する
    pub fn evaluate_countdown_at(&self, time: RationalTime) -> Option<AdrCountdownState> {
        if !self.enabled || !self.show_countdown_streamer || self.cues.is_empty() {
            return None;
        }
        let t_sec = time.as_seconds();
        for cue in &self.cues {
            let cue_start = cue.start_time.as_seconds();
            let cue_end = (cue.start_time + cue.duration).as_seconds();
            let preroll = cue.preroll_sec.clamp(0.5, 5.0);
            if t_sec >= cue_start - preroll && t_sec < cue_start {
                let remain = (cue_start - t_sec).max(0.0);
                let beat = remain.ceil().clamp(1.0, 5.0) as u32;
                let progress = (1.0 - remain / preroll).clamp(0.0, 1.0) as f32;
                return Some(AdrCountdownState {
                    cue_number: cue.cue_number,
                    character: cue.character.clone(),
                    line_text: cue.line_text.clone(),
                    countdown_beat: Some((beat, progress)),
                    recording_active: false,
                    record_progress: 0.0,
                });
            }
            if t_sec >= cue_start && t_sec <= cue_end {
                let dur = (cue_end - cue_start).max(1e-4);
                let prog = ((t_sec - cue_start) / dur).clamp(0.0, 1.0) as f32;
                return Some(AdrCountdownState {
                    cue_number: cue.cue_number,
                    character: cue.character.clone(),
                    line_text: cue.line_text.clone(),
                    countdown_beat: None,
                    recording_active: true,
                    record_progress: prog,
                });
            }
        }
        None
    }
}

/// 空間音声マスタリングバス種別
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialAudioBusMode {
    /// 標準 2.0 ステレオ
    #[default]
    Stereo,
    /// ITU-R BS.775 5.1ch サラウンド (`L, R, C, LFE, Ls, Rs`)
    Surround51,
    /// 1 次 Ambisonics FOA B-Format AmbiX (ACN / SN3D: `W, Y, Z, X` ⇔ バイノーラル復号)
    AmbisonicsFoaAmbiX,
}

impl SpatialAudioBusMode {
    pub const ALL: &'static [Self] = &[Self::Stereo, Self::Surround51, Self::AmbisonicsFoaAmbiX];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Stereo => "2.0 ステレオ",
            Self::Surround51 => "🔊 5.1ch サラウンド (ITU-R BS.775)",
            Self::AmbisonicsFoaAmbiX => "🌐 FOA Ambisonics (AmbiX W,Y,Z,X ⇔ バイノーラル)",
        }
    }

    pub fn channel_count(self) -> usize {
        match self {
            Self::Stereo => 2,
            Self::Surround51 => 6,
            Self::AmbisonicsFoaAmbiX => 4,
        }
    }
}

fn default_lfe_crossover() -> f32 {
    90.0
}
fn default_center_boost() -> f32 {
    0.5
}
fn default_room_width() -> f32 {
    0.65
}

/// 5.1ch サラウンド / FOA Ambisonics (AmbiX) 空間音声バス設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpatialBusConfig {
    #[serde(default)]
    pub mode: SpatialAudioBusMode,
    /// 音源方位角（度: `-180..=180`, 0=正面, -90=左, +90=右）
    #[serde(default)]
    pub azimuth_deg: f32,
    /// 音源仰角（度: `-90..=90`, Ambisonics Z 成分）
    #[serde(default)]
    pub elevation_deg: f32,
    /// 5.1ch LFE サブウーファーローパス周波数 (Hz)
    #[serde(default = "default_lfe_crossover")]
    pub lfe_crossover_hz: f32,
    /// 5.1ch センター台詞寄与 (`0.0..=1.0`)
    #[serde(default = "default_center_boost")]
    pub center_dialogue_boost: f32,
    /// 音場広がり幅 (`0.0..=1.0`)
    #[serde(default = "default_room_width")]
    pub room_width: f32,
    /// FOA Ambisonics リスナー頭部ヨー回転（度: `-180..=180`）
    #[serde(default)]
    pub head_yaw_deg: f32,
    /// FOA Ambisonics リスナー頭部ピッチ回転（度: `-90..=90`）
    #[serde(default)]
    pub head_pitch_deg: f32,
}

impl Default for SpatialBusConfig {
    fn default() -> Self {
        Self {
            mode: SpatialAudioBusMode::Stereo,
            azimuth_deg: 0.0,
            elevation_deg: 0.0,
            lfe_crossover_hz: default_lfe_crossover(),
            center_dialogue_boost: default_center_boost(),
            room_width: default_room_width(),
            head_yaw_deg: 0.0,
            head_pitch_deg: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roto_brush_interpolates_smoothly_across_keyframes() {
        let mut roto = RotoBrushConfig {
            enabled: true,
            ..Default::default()
        };
        roto.keyframes.push(RotoKeyframe {
            time: RationalTime::ZERO,
            vertices: RotoBrushConfig::default_ellipse_vertices(0.3, 0.5, 0.15, 0.2, 8),
            feather_px: 2.0,
            expansion_px: 0.0,
        });
        roto.keyframes.push(RotoKeyframe {
            time: RationalTime::from_seconds(2.0),
            vertices: RotoBrushConfig::default_ellipse_vertices(0.7, 0.5, 0.15, 0.2, 8),
            feather_px: 6.0,
            expansion_px: 2.0,
        });

        let mid = roto
            .evaluate_mask_at(RationalTime::from_seconds(1.0))
            .expect("mid mask");
        let avg_x = mid.vertices.iter().map(|v| v.x).sum::<f32>() / mid.vertices.len() as f32;
        assert!((avg_x - 0.5).abs() < 1e-3, "avg_x={avg_x}");
        assert!((mid.feather_px - 4.0).abs() < 1e-3);
        let max_dx = mid
            .vertices
            .iter()
            .map(|v| (v.x - avg_x).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_dx > 0.15,
            "expanded radius should exceed base 0.15: {max_dx}"
        );
    }

    #[test]
    fn adr_countdown_evaluates_preroll_and_recording_windows() {
        let mut adr = AdrSessionConfig {
            enabled: true,
            ..Default::default()
        };
        adr.cues.push(AdrCue::new(
            1,
            "Hero",
            "ここで決める！",
            RationalTime::from_seconds(5.0),
            RationalTime::from_seconds(2.0),
        ));

        let pre = adr
            .evaluate_countdown_at(RationalTime::from_seconds(2.4))
            .expect("preroll active");
        assert_eq!(pre.countdown_beat.unwrap().0, 3);
        assert!(!pre.recording_active);

        let rec = adr
            .evaluate_countdown_at(RationalTime::from_seconds(6.0))
            .expect("recording active");
        assert!(rec.recording_active);
        assert!((rec.record_progress - 0.5).abs() < 1e-3);
    }

    #[test]
    fn planar_track_command_builder_emits_corner_pin_and_8_tracks() {
        let res = PlanarTrackResult {
            frames: vec![
                PlanarTrackFrame {
                    time: RationalTime::ZERO,
                    quad: [[0.1, 0.1], [0.9, 0.1], [0.9, 0.9], [0.1, 0.9]],
                    homography: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    inlier_count: 14,
                    confidence: 0.95,
                },
                PlanarTrackFrame {
                    time: RationalTime::from_seconds(1.0),
                    quad: [[0.2, 0.15], [0.85, 0.12], [0.88, 0.85], [0.18, 0.88]],
                    homography: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    inlier_count: 12,
                    confidence: 0.91,
                },
            ],
            mean_confidence: 0.93,
            mean_inliers: 13.0,
        };
        let cmd =
            build_planar_track_corner_pin_command(Uuid::new_v4(), RationalTime::ZERO, &res, 1.5)
                .expect("command");
        match cmd {
            EditCommand::Batch { commands, .. } => assert_eq!(commands.len(), 17),
            _ => panic!("expected batch"),
        }
    }
}
