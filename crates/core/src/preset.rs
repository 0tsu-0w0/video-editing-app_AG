use crate::project::{EffectNode, RgbaColor, TextStyle, TimelineItem};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// 名前付きプリセット色（AviUtl2 カラーパレット相当）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedColor {
    pub name: String,
    pub color: RgbaColor,
}

impl NamedColor {
    pub fn new(name: impl Into<String>, color: RgbaColor) -> Self {
        Self {
            name: name.into(),
            color,
        }
    }
}

/// プロジェクト共通の名前付きカラーパレット（字幕制御文字 `<@名前>` や一括色変更で参照）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorPalette {
    pub colors: Vec<NamedColor>,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self::default_presets()
    }
}

impl ColorPalette {
    pub fn default_presets() -> Self {
        Self {
            colors: vec![
                NamedColor::new("字幕白", RgbaColor::WHITE),
                NamedColor::new(
                    "強調赤",
                    RgbaColor {
                        r: 1.0,
                        g: 0.25,
                        b: 0.25,
                        a: 1.0,
                    },
                ),
                NamedColor::new(
                    "ツッコミ黄",
                    RgbaColor {
                        r: 1.0,
                        g: 0.88,
                        b: 0.2,
                        a: 1.0,
                    },
                ),
                NamedColor::new(
                    "解説シアン",
                    RgbaColor {
                        r: 0.25,
                        g: 0.85,
                        b: 1.0,
                        a: 1.0,
                    },
                ),
                NamedColor::new(
                    "ずんだ緑",
                    RgbaColor {
                        r: 0.45,
                        g: 0.92,
                        b: 0.48,
                        a: 1.0,
                    },
                ),
                NamedColor::new(
                    "かわいいピンク",
                    RgbaColor {
                        r: 1.0,
                        g: 0.55,
                        b: 0.78,
                        a: 1.0,
                    },
                ),
                NamedColor::new("黒縁", RgbaColor::BLACK),
                NamedColor::new(
                    "座布団黒",
                    RgbaColor {
                        r: 0.05,
                        g: 0.06,
                        b: 0.09,
                        a: 0.72,
                    },
                ),
            ],
        }
    }

    pub fn find_by_name(&self, name: &str) -> Option<RgbaColor> {
        self.colors.iter().find(|c| c.name == name).map(|c| c.color)
    }

    pub fn add_or_update(&mut self, name: impl Into<String>, color: RgbaColor) {
        let name = name.into();
        if let Some(existing) = self.colors.iter_mut().find(|c| c.name == name) {
            existing.color = color;
        } else {
            self.colors.push(NamedColor { name, color });
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let prev = self.colors.len();
        self.colors.retain(|c| c.name != name);
        self.colors.len() != prev
    }
}

/// AviUtl2 エイリアス（オブジェクト全体＋エフェクト＋キーフレームの再利用テンプレート）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemAlias {
    pub name: String,
    #[serde(default)]
    pub category: String,
    pub template: TimelineItem,
}

impl ItemAlias {
    pub fn from_item(
        name: impl Into<String>,
        category: impl Into<String>,
        item: &TimelineItem,
    ) -> Self {
        Self {
            name: name.into(),
            category: category.into(),
            template: item.clone(),
        }
    }

    /// 再生ヘッド位置へ新しい UUID を振り直してインスタンス化する
    pub fn instantiate_at(&self, start: crate::time::RationalTime) -> TimelineItem {
        let mut item = self.template.clone();
        item.id = Uuid::new_v4();
        item.start = start.max(crate::time::RationalTime::ZERO);
        for eff in &mut item.effect_chain {
            eff.id = Uuid::new_v4();
        }
        for eff in &mut item.audio_effects {
            eff.id = Uuid::new_v4();
        }
        item
    }
}

/// エフェクトチェーン単体のプリセット（`presets/effects/*.json` 保存・1クリック適用向け）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectPreset {
    pub name: String,
    pub effects: Vec<EffectNode>,
}

impl EffectPreset {
    pub fn instantiate(&self) -> Vec<EffectNode> {
        self.effects
            .iter()
            .cloned()
            .map(|mut e| {
                e.id = Uuid::new_v4();
                e
            })
            .collect()
    }

    /// 標準組み込みエフェクトチェーンプリセット
    pub fn default_presets() -> Vec<Self> {
        let node = |effect_type: &str, pairs: &[(&str, f32)]| EffectNode {
            id: Uuid::nil(),
            effect_type: effect_type.to_string(),
            enabled: true,
            params: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect::<BTreeMap<_, _>>(),
        };
        vec![
            Self {
                name: "シネマティック暖色".into(),
                effects: vec![
                    node(
                        "color_grade",
                        &[("exposure", 0.12), ("contrast", 1.12), ("saturation", 1.15)],
                    ),
                    node("glow", &[("intensity", 0.65), ("threshold", 0.65)]),
                ],
            },
            Self {
                name: "解説ワイプ枠 (角丸+影)".into(),
                effects: vec![
                    node("rounded_clip", &[("radius", 28.0), ("margin", 4.0)]),
                    node(
                        "drop_shadow",
                        &[
                            ("offset_x", 8.0),
                            ("offset_y", 8.0),
                            ("blur", 6.0),
                            ("opacity", 0.55),
                        ],
                    ),
                ],
            },
            Self {
                name: "強調パンチ (振動+発光)".into(),
                effects: vec![
                    node("shake", &[("amp_x", 10.0), ("amp_y", 8.0), ("freq", 22.0)]),
                    node("glow", &[("intensity", 1.1), ("threshold", 0.48)]),
                ],
            },
        ]
    }
}

/// 字幕スタイルプリセット（`presets/text_styles/*.json` 保存・1クリック適用向け）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStylePreset {
    pub name: String,
    pub font_family: String,
    pub font_size: f32,
    pub color: RgbaColor,
    pub text_style: TextStyle,
}

impl TextStylePreset {
    /// YouTube / ゆっくり解説 / シネマ向けの標準字幕スタイルプリセット
    pub fn default_presets() -> Vec<Self> {
        vec![
            Self {
                name: "YouTube 太縁テロップ".into(),
                font_family: "sans-serif".into(),
                font_size: 56.0,
                color: RgbaColor::WHITE,
                text_style: TextStyle {
                    outline_width: 5.0,
                    outline_color: RgbaColor::BLACK,
                    shadow_offset: [4.0, 4.0],
                    shadow_blur: 3.0,
                    shadow_color: RgbaColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.72,
                    },
                    box_enabled: false,
                    ..TextStyle::default()
                },
            },
            Self {
                name: "ゆっくり座布団字幕".into(),
                font_family: "sans-serif".into(),
                font_size: 44.0,
                color: RgbaColor::WHITE,
                text_style: TextStyle {
                    outline_width: 2.5,
                    outline_color: RgbaColor::BLACK,
                    shadow_offset: [0.0, 2.0],
                    shadow_blur: 2.0,
                    shadow_color: RgbaColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.55,
                    },
                    box_enabled: true,
                    box_color: RgbaColor {
                        r: 0.04,
                        g: 0.06,
                        b: 0.11,
                        a: 0.78,
                    },
                    box_padding: [18.0, 10.0],
                    box_radius: 12.0,
                },
            },
            Self {
                name: "強調ツッコミ黄×赤縁".into(),
                font_family: "sans-serif".into(),
                font_size: 64.0,
                color: RgbaColor {
                    r: 1.0,
                    g: 0.90,
                    b: 0.18,
                    a: 1.0,
                },
                text_style: TextStyle {
                    outline_width: 6.0,
                    outline_color: RgbaColor {
                        r: 0.78,
                        g: 0.08,
                        b: 0.10,
                        a: 1.0,
                    },
                    shadow_offset: [5.0, 5.0],
                    shadow_blur: 4.0,
                    shadow_color: RgbaColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.80,
                    },
                    box_enabled: false,
                    ..TextStyle::default()
                },
            },
            Self {
                name: "シネマ繊細字幕".into(),
                font_family: "serif".into(),
                font_size: 38.0,
                color: RgbaColor {
                    r: 0.96,
                    g: 0.95,
                    b: 0.92,
                    a: 1.0,
                },
                text_style: TextStyle {
                    outline_width: 0.0,
                    outline_color: RgbaColor::BLACK,
                    shadow_offset: [0.0, 3.0],
                    shadow_blur: 4.0,
                    shadow_color: RgbaColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.70,
                    },
                    box_enabled: false,
                    ..TextStyle::default()
                },
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ItemKind, TextAnimator};
    use crate::time::RationalTime;

    #[test]
    fn color_palette_crud_and_lookup() {
        let mut pal = ColorPalette::default_presets();
        assert!(pal.find_by_name("強調赤").is_some());
        pal.add_or_update(
            "カスタム青",
            RgbaColor {
                r: 0.1,
                g: 0.2,
                b: 0.9,
                a: 1.0,
            },
        );
        assert!(pal.find_by_name("カスタム青").is_some());
        assert!(pal.remove("カスタム青"));
        assert!(pal.find_by_name("カスタム青").is_none());
    }

    #[test]
    fn alias_instantiate_assigns_fresh_ids_and_start() {
        let mut src = TimelineItem::new(
            "強調字幕",
            RationalTime::from_seconds(2.0),
            RationalTime::from_seconds(3.0),
            ItemKind::Text {
                content: "<#ff3333>重要</>テスト".into(),
                font_family: "sans-serif".into(),
                font_size: 48.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        src.effect_chain.push(EffectNode {
            id: Uuid::new_v4(),
            effect_type: "glow".into(),
            enabled: true,
            params: Default::default(),
        });

        let alias = ItemAlias::from_item("重要テロップ", "字幕", &src);
        let inst = alias.instantiate_at(RationalTime::from_seconds(10.0));
        assert_ne!(inst.id, src.id);
        assert_eq!(inst.start, RationalTime::from_seconds(10.0));
        assert_eq!(inst.duration, src.duration);
        assert_ne!(inst.effect_chain[0].id, src.effect_chain[0].id);
    }

    #[test]
    fn builtin_text_style_and_effect_presets_are_valid() {
        let text_presets = TextStylePreset::default_presets();
        assert_eq!(text_presets.len(), 4);
        assert!(text_presets.iter().any(|p| p.text_style.box_enabled));

        let fx_presets = EffectPreset::default_presets();
        assert_eq!(fx_presets.len(), 3);
        let inst_a = fx_presets[0].instantiate();
        let inst_b = fx_presets[0].instantiate();
        assert_eq!(inst_a.len(), 2);
        assert_ne!(inst_a[0].id, inst_b[0].id);
    }
}
