//! v6.0 #42: 非破壊 VFX ペイント ＆ クローンスタンプ / ワイヤー・不要物消し（Inpainting）＆ プライバシーぼかしパス
//! Pure Rust・ゼロ `unsafe` で、クリップ単位のベクターストローク描画（アニメ書き込み対応）、
//! クローンスタンプ複製、境界補間型ワイヤー消しインペインティング、局所モザイク秘匿を提供する。

use crate::project::RgbaColor;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintToolKind {
    #[default]
    BrushGlow,
    CloneStamp,
    WireEraseInpaint,
    BlurRedact,
}

impl PaintToolKind {
    pub const ALL: &'static [Self] = &[
        Self::BrushGlow,
        Self::CloneStamp,
        Self::WireEraseInpaint,
        Self::BlurRedact,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::BrushGlow => "🖌 グローブラシ手描き (Write-On 対応)",
            Self::CloneStamp => "🩹 クローンスタンプ複製",
            Self::WireEraseInpaint => "✨ ワイヤー・不要物消し (Inpaint)",
            Self::BlurRedact => "🔒 プライバシーぼかし / モザイク秘匿",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintStroke {
    pub id: Uuid,
    #[serde(default)]
    pub tool: PaintToolKind,
    /// 正規化ストローク頂点列 (`[0.0..=1.0, 0.0..=1.0]`)
    pub points: Vec<[f32; 2]>,
    /// ブラシ半径（px）
    #[serde(default = "default_radius")]
    pub radius_px: f32,
    /// エッジ硬さ (`0.1..=1.0`)
    #[serde(default = "default_hardness")]
    pub hardness: f32,
    /// 不透明度 (`0.0..=1.0`)
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_paint_color")]
    pub color: RgbaColor,
    /// クローンスタンプ時の参照元正規化オフセット (`[dx, dy]`)
    #[serde(default = "default_clone_offset")]
    pub clone_offset: [f32; 2],
    /// Write-On 描画進行上限 (`0.0..=1.0`, 既定 `1.0` 全長)
    #[serde(default = "default_opacity")]
    pub write_on_progress: f32,
}

fn default_radius() -> f32 {
    14.0
}
fn default_hardness() -> f32 {
    0.75
}
fn default_opacity() -> f32 {
    1.0
}
fn default_paint_color() -> RgbaColor {
    RgbaColor {
        r: 1.0,
        g: 0.28,
        b: 0.52,
        a: 1.0,
    }
}
fn default_clone_offset() -> [f32; 2] {
    [0.08, 0.0]
}

impl PaintStroke {
    pub fn brush_glow(points: Vec<[f32; 2]>, radius_px: f32, color: RgbaColor) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool: PaintToolKind::BrushGlow,
            points,
            radius_px: radius_px.clamp(2.0, 128.0),
            hardness: 0.65,
            opacity: 1.0,
            color,
            clone_offset: [0.0, 0.0],
            write_on_progress: 1.0,
        }
    }

    pub fn wire_erase(points: Vec<[f32; 2]>, radius_px: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool: PaintToolKind::WireEraseInpaint,
            points,
            radius_px: radius_px.clamp(2.0, 96.0),
            hardness: 0.7,
            opacity: 1.0,
            color: RgbaColor::WHITE,
            clone_offset: [0.0, 0.0],
            write_on_progress: 1.0,
        }
    }

    pub fn clone_stamp(points: Vec<[f32; 2]>, radius_px: f32, clone_offset: [f32; 2]) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool: PaintToolKind::CloneStamp,
            points,
            radius_px: radius_px.clamp(2.0, 128.0),
            hardness: 0.6,
            opacity: 1.0,
            color: RgbaColor::WHITE,
            clone_offset,
            write_on_progress: 1.0,
        }
    }

    pub fn blur_redact(points: Vec<[f32; 2]>, radius_px: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool: PaintToolKind::BlurRedact,
            points,
            radius_px: radius_px.clamp(4.0, 128.0),
            hardness: 0.85,
            opacity: 1.0,
            color: RgbaColor::WHITE,
            clone_offset: [0.0, 0.0],
            write_on_progress: 1.0,
        }
    }
}

/// クリップ単位の非破壊 VFX ペイント＆インペインティング設定（`TimelineItem::vfx_paint`）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VfxPaintConfig {
    #[serde(default)]
    pub enabled: bool,
    /// true のときクリップ進行に合わせてグローブラシがリアルタイムに書き進まれる（Write-On）
    #[serde(default)]
    pub animate_write_on: bool,
    #[serde(default)]
    pub strokes: Vec<PaintStroke>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfx_paint_strokes_build_and_roundtrip_serde() {
        let cfg = VfxPaintConfig {
            enabled: true,
            animate_write_on: true,
            strokes: vec![
                PaintStroke::brush_glow(vec![[0.1, 0.2], [0.8, 0.7]], 16.0, RgbaColor::WHITE),
                PaintStroke::wire_erase(vec![[0.2, 0.5], [0.9, 0.5]], 10.0),
                PaintStroke::clone_stamp(vec![[0.4, 0.4]], 18.0, [0.12, -0.04]),
                PaintStroke::blur_redact(vec![[0.3, 0.3], [0.6, 0.3]], 20.0),
            ],
        };
        let raw = serde_json::to_string(&cfg).unwrap();
        let dec: VfxPaintConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(cfg, dec);
        assert_eq!(dec.strokes.len(), 4);
    }
}
