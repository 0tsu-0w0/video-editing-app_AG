//! v4.0 #35: オフライン・プロシージャル背景ジェネレーター設定（オーロラ・シネマボケ・サイバーグリッド・リキッドマーブル・ワープ星雲・アブストラクト波）

use crate::project::RgbaColor;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerativeBgPreset {
    #[default]
    AuroraBorealis,
    CinemaBokeh,
    CyberGrid3D,
    LiquidMarble,
    StarfieldWarp,
    AbstractWaves,
}

impl GenerativeBgPreset {
    pub const ALL: &'static [Self] = &[
        Self::AuroraBorealis,
        Self::CinemaBokeh,
        Self::CyberGrid3D,
        Self::LiquidMarble,
        Self::StarfieldWarp,
        Self::AbstractWaves,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::AuroraBorealis => "🌌 オーロラ・ボレアリス",
            Self::CinemaBokeh => "✨ シネマティック玉ボケ",
            Self::CyberGrid3D => "🕹 サイバー3Dグリッド",
            Self::LiquidMarble => "🎨 リキッドマーブル流体",
            Self::StarfieldWarp => "🌠 スターフィールドワープ",
            Self::AbstractWaves => "🌊 アブストラクトグラデ波",
        }
    }

    pub fn for_preset(preset: Self) -> GenerativeBgConfig {
        let mut cfg = GenerativeBgConfig {
            preset,
            ..GenerativeBgConfig::default()
        };
        match preset {
            Self::AuroraBorealis => {
                cfg.primary_color = RgbaColor {
                    r: 0.04,
                    g: 0.08,
                    b: 0.18,
                    a: 1.0,
                };
                cfg.secondary_color = RgbaColor {
                    r: 0.12,
                    g: 0.88,
                    b: 0.62,
                    a: 1.0,
                };
                cfg.accent_color = RgbaColor {
                    r: 0.56,
                    g: 0.24,
                    b: 0.96,
                    a: 1.0,
                };
            }
            Self::CinemaBokeh => {
                cfg.primary_color = RgbaColor {
                    r: 0.06,
                    g: 0.05,
                    b: 0.10,
                    a: 1.0,
                };
                cfg.secondary_color = RgbaColor {
                    r: 1.0,
                    g: 0.68,
                    b: 0.25,
                    a: 1.0,
                };
                cfg.accent_color = RgbaColor {
                    r: 0.25,
                    g: 0.72,
                    b: 1.0,
                    a: 1.0,
                };
            }
            Self::CyberGrid3D => {
                cfg.primary_color = RgbaColor {
                    r: 0.05,
                    g: 0.02,
                    b: 0.14,
                    a: 1.0,
                };
                cfg.secondary_color = RgbaColor {
                    r: 1.0,
                    g: 0.16,
                    b: 0.62,
                    a: 1.0,
                };
                cfg.accent_color = RgbaColor {
                    r: 0.12,
                    g: 0.92,
                    b: 1.0,
                    a: 1.0,
                };
            }
            Self::LiquidMarble => {
                cfg.primary_color = RgbaColor {
                    r: 0.10,
                    g: 0.14,
                    b: 0.28,
                    a: 1.0,
                };
                cfg.secondary_color = RgbaColor {
                    r: 0.95,
                    g: 0.42,
                    b: 0.52,
                    a: 1.0,
                };
                cfg.accent_color = RgbaColor {
                    r: 0.98,
                    g: 0.84,
                    b: 0.44,
                    a: 1.0,
                };
            }
            Self::StarfieldWarp => {
                cfg.primary_color = RgbaColor {
                    r: 0.02,
                    g: 0.03,
                    b: 0.08,
                    a: 1.0,
                };
                cfg.secondary_color = RgbaColor {
                    r: 0.78,
                    g: 0.92,
                    b: 1.0,
                    a: 1.0,
                };
                cfg.accent_color = RgbaColor {
                    r: 0.45,
                    g: 0.62,
                    b: 1.0,
                    a: 1.0,
                };
            }
            Self::AbstractWaves => {
                cfg.primary_color = RgbaColor {
                    r: 0.08,
                    g: 0.18,
                    b: 0.36,
                    a: 1.0,
                };
                cfg.secondary_color = RgbaColor {
                    r: 0.20,
                    g: 0.66,
                    b: 0.96,
                    a: 1.0,
                };
                cfg.accent_color = RgbaColor {
                    r: 0.94,
                    g: 0.98,
                    b: 1.0,
                    a: 1.0,
                };
            }
        }
        cfg
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GenerativeBgConfig {
    #[serde(default)]
    pub preset: GenerativeBgPreset,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default = "default_scale")]
    pub scale: f32,
    #[serde(default = "default_complexity")]
    pub complexity: u32,
    #[serde(default = "default_bg_w")]
    pub width: f32,
    #[serde(default = "default_bg_h")]
    pub height: f32,
    #[serde(default = "default_primary")]
    pub primary_color: RgbaColor,
    #[serde(default = "default_secondary")]
    pub secondary_color: RgbaColor,
    #[serde(default = "default_accent")]
    pub accent_color: RgbaColor,
}

fn default_seed() -> u64 {
    40_2026
}
fn default_speed() -> f32 {
    1.0
}
fn default_scale() -> f32 {
    1.0
}
fn default_complexity() -> u32 {
    4
}
fn default_bg_w() -> f32 {
    1280.0
}
fn default_bg_h() -> f32 {
    720.0
}
fn default_primary() -> RgbaColor {
    RgbaColor {
        r: 0.04,
        g: 0.08,
        b: 0.18,
        a: 1.0,
    }
}
fn default_secondary() -> RgbaColor {
    RgbaColor {
        r: 0.12,
        g: 0.88,
        b: 0.62,
        a: 1.0,
    }
}
fn default_accent() -> RgbaColor {
    RgbaColor {
        r: 0.56,
        g: 0.24,
        b: 0.96,
        a: 1.0,
    }
}

impl Default for GenerativeBgConfig {
    fn default() -> Self {
        Self {
            preset: GenerativeBgPreset::AuroraBorealis,
            seed: default_seed(),
            speed: default_speed(),
            scale: default_scale(),
            complexity: default_complexity(),
            width: default_bg_w(),
            height: default_bg_h(),
            primary_color: default_primary(),
            secondary_color: default_secondary(),
            accent_color: default_accent(),
        }
    }
}
