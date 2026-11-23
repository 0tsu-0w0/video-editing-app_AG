//! v5.0 #39: スピードランプ（Time-Remapping 曲線解析積分）＆ 自動シーンカット検出コマンドビルダー
//! Pure Rust・ゼロ `unsafe` で、任意の速度倍率カーブ（スローモーション・バレットタイム・ヒーロー加速）の
//! 厳密な積分タイムリマップと、検出ショット境界への 1-Undo マーカー／自動分割コマンド生成を提供する。

use crate::command::EditCommand;
use crate::project::{ItemKind, Scene, TimelineMarker};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedRampPreset {
    #[default]
    Uniform,
    BulletTimeDip,
    RampUpHero,
    EaseInSlowMo,
    FlashBurst,
}

impl SpeedRampPreset {
    pub const ALL: &'static [Self] = &[
        Self::Uniform,
        Self::BulletTimeDip,
        Self::RampUpHero,
        Self::EaseInSlowMo,
        Self::FlashBurst,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Uniform => "➡ 等速 (1.0x 均一)",
            Self::BulletTimeDip => "🎯 バレットタイム (2.4x → 0.25x → 2.4x)",
            Self::RampUpHero => "🚀 ヒーロー加速 (0.4x → 2.8x)",
            Self::EaseInSlowMo => "🌊 滑らかスロー突入 (1.8x → 0.35x)",
            Self::FlashBurst => "⚡ フラッシュ早送りスパイク (1.0x → 4.0x → 1.0x)",
        }
    }

    pub fn default_config(self) -> SpeedRampConfig {
        match self {
            Self::Uniform => SpeedRampConfig {
                enabled: false,
                preset: self,
                knots: vec![
                    SpeedRampKnot {
                        norm_time: 0.0,
                        speed: 1.0,
                    },
                    SpeedRampKnot {
                        norm_time: 1.0,
                        speed: 1.0,
                    },
                ],
                smooth_blend: true,
            },
            Self::BulletTimeDip => SpeedRampConfig {
                enabled: true,
                preset: self,
                knots: vec![
                    SpeedRampKnot {
                        norm_time: 0.0,
                        speed: 2.4,
                    },
                    SpeedRampKnot {
                        norm_time: 0.32,
                        speed: 0.25,
                    },
                    SpeedRampKnot {
                        norm_time: 0.68,
                        speed: 0.25,
                    },
                    SpeedRampKnot {
                        norm_time: 1.0,
                        speed: 2.4,
                    },
                ],
                smooth_blend: true,
            },
            Self::RampUpHero => SpeedRampConfig {
                enabled: true,
                preset: self,
                knots: vec![
                    SpeedRampKnot {
                        norm_time: 0.0,
                        speed: 0.4,
                    },
                    SpeedRampKnot {
                        norm_time: 0.45,
                        speed: 0.75,
                    },
                    SpeedRampKnot {
                        norm_time: 1.0,
                        speed: 2.8,
                    },
                ],
                smooth_blend: true,
            },
            Self::EaseInSlowMo => SpeedRampConfig {
                enabled: true,
                preset: self,
                knots: vec![
                    SpeedRampKnot {
                        norm_time: 0.0,
                        speed: 1.8,
                    },
                    SpeedRampKnot {
                        norm_time: 0.35,
                        speed: 0.35,
                    },
                    SpeedRampKnot {
                        norm_time: 1.0,
                        speed: 0.35,
                    },
                ],
                smooth_blend: true,
            },
            Self::FlashBurst => SpeedRampConfig {
                enabled: true,
                preset: self,
                knots: vec![
                    SpeedRampKnot {
                        norm_time: 0.0,
                        speed: 1.0,
                    },
                    SpeedRampKnot {
                        norm_time: 0.42,
                        speed: 1.0,
                    },
                    SpeedRampKnot {
                        norm_time: 0.50,
                        speed: 4.0,
                    },
                    SpeedRampKnot {
                        norm_time: 0.58,
                        speed: 1.0,
                    },
                    SpeedRampKnot {
                        norm_time: 1.0,
                        speed: 1.0,
                    },
                ],
                smooth_blend: true,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpeedRampKnot {
    /// クリップ内の正規化位置 `[0.0, 1.0]`
    pub norm_time: f32,
    /// その位置での瞬間再生速度倍率 (`0.05..=8.0`)
    pub speed: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedRampConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub preset: SpeedRampPreset,
    #[serde(default = "default_speed_knots")]
    pub knots: Vec<SpeedRampKnot>,
    #[serde(default = "default_true")]
    pub smooth_blend: bool,
}

fn default_true() -> bool {
    true
}

fn default_speed_knots() -> Vec<SpeedRampKnot> {
    vec![
        SpeedRampKnot {
            norm_time: 0.0,
            speed: 1.0,
        },
        SpeedRampKnot {
            norm_time: 1.0,
            speed: 1.0,
        },
    ]
}

impl Default for SpeedRampConfig {
    fn default() -> Self {
        SpeedRampPreset::Uniform.default_config()
    }
}

impl SpeedRampConfig {
    /// ノットを正規化時間昇順で正規化・クランプする
    pub fn normalized(&self) -> Vec<SpeedRampKnot> {
        let mut pts: Vec<SpeedRampKnot> = self
            .knots
            .iter()
            .map(|k| SpeedRampKnot {
                norm_time: k.norm_time.clamp(0.0, 1.0),
                speed: k.speed.clamp(0.05, 8.0),
            })
            .collect();
        pts.sort_by(|a, b| a.norm_time.total_cmp(&b.norm_time));
        if pts.is_empty() {
            return default_speed_knots();
        }
        if pts[0].norm_time > 1e-4 {
            pts.insert(
                0,
                SpeedRampKnot {
                    norm_time: 0.0,
                    speed: pts[0].speed,
                },
            );
        }
        if let Some(&last) = pts.last()
            && last.norm_time < 1.0 - 1e-4
        {
            pts.push(SpeedRampKnot {
                norm_time: 1.0,
                speed: last.speed,
            });
        }
        pts
    }
}

/// 正規化クリップ位置 `u in [0, 1]` における瞬間再生速度倍率を返す
pub fn evaluate_speed_at_norm_time(cfg: &SpeedRampConfig, u: f32) -> f32 {
    if !cfg.enabled {
        return 1.0;
    }
    let pts = cfg.normalized();
    let x = u.clamp(0.0, 1.0);
    for seg in pts.windows(2) {
        let a = seg[0];
        let b = seg[1];
        if x <= b.norm_time || (b.norm_time - 1.0).abs() < 1e-5 {
            let span = (b.norm_time - a.norm_time).max(1e-5);
            let mut frac = ((x - a.norm_time) / span).clamp(0.0, 1.0);
            if cfg.smooth_blend {
                frac = frac * frac * (3.0 - 2.0 * frac);
            }
            return a.speed + (b.speed - a.speed) * frac;
        }
    }
    pts.last().map(|k| k.speed).unwrap_or(1.0)
}

/// スピードランプ曲線の厳密解析積分により、タイムライン経過時間 `local_sec` に対応する
/// リマップ後のソース再生時間（秒）を算出する（滑らか・単調増加・ドリフトゼロ）。
pub fn evaluate_speed_ramp_source_time(
    local_sec: f64,
    clip_dur_sec: f64,
    cfg: &SpeedRampConfig,
) -> f64 {
    if !cfg.enabled || clip_dur_sec <= 1e-6 || local_sec <= 0.0 {
        return local_sec.max(0.0);
    }
    let pts = cfg.normalized();
    let u_target = (local_sec / clip_dur_sec).clamp(0.0, 1.0);
    let mut integral_u = 0.0f64;

    for seg in pts.windows(2) {
        let a = seg[0];
        let b = seg[1];
        let t0 = f64::from(a.norm_time);
        let t1 = f64::from(b.norm_time);
        if u_target <= t0 {
            break;
        }
        let span = t1 - t0;
        if span <= 1e-6 {
            continue;
        }
        let s0 = f64::from(a.speed);
        let ds = f64::from(b.speed) - s0;
        let hi = u_target.min(t1);
        let f = ((hi - t0) / span).clamp(0.0, 1.0);
        // smoothstep(t) = 3t^2 - 2t^3 の解析不定積分は t^3 - 0.5*t^4
        let blend_area = if cfg.smooth_blend {
            let f2 = f * f;
            f2 * f - 0.5 * f2 * f2
        } else {
            0.5 * f * f
        };
        integral_u += span * (s0 * f + ds * blend_area);
        if u_target <= t1 {
            break;
        }
    }

    // クリップ尺を超えた時刻は終端速度で外挿
    let extra = (local_sec - clip_dur_sec).max(0.0);
    let end_speed = f64::from(pts.last().map(|k| k.speed).unwrap_or(1.0));
    integral_u * clip_dur_sec + extra * end_speed
}

/// 自動シーンカット検出で発見された 1 つのショット境界
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DetectedSceneCut {
    pub time_sec: f64,
    pub confidence: f32,
    pub hist_chi_sq: f32,
    pub luma_delta_ire: f32,
}

/// 検出されたショット境界からタイムラインマーカー追加（およびオプションで対象レイヤーのクリップ自動分割）
/// を行う 1-Undo コマンド列を構築する
pub fn build_scene_cut_commands(
    scene: &Scene,
    cuts: &[DetectedSceneCut],
    split_clips: bool,
    target_layer_index: Option<usize>,
) -> Vec<EditCommand> {
    let mut cmds = Vec::new();
    let mut sorted: Vec<DetectedSceneCut> = cuts.to_vec();
    sorted.sort_by(|a, b| a.time_sec.total_cmp(&b.time_sec));

    for (idx, cut) in sorted.iter().enumerate() {
        cmds.push(EditCommand::AddMarker {
            marker: TimelineMarker::new(
                RationalTime::from_seconds(cut.time_sec),
                format!("Cut #{} ({:.0}%)", idx + 1, cut.confidence * 100.0),
            ),
        });
    }

    if split_clips {
        // 後ろの時刻から逆順に分割することで、前半クリップ ID への連続分割がすべて成立する
        for cut in sorted.iter().rev() {
            let split_t = RationalTime::from_seconds(cut.time_sec);
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                if let Some(only_l) = target_layer_index
                    && l_idx != only_l
                {
                    continue;
                }
                for item in &layer.items {
                    if matches!(
                        item.kind,
                        ItemKind::Video { .. }
                            | ItemKind::Image { .. }
                            | ItemKind::GenerativeBg { .. }
                    ) && split_t > item.start + RationalTime::from_seconds(0.05)
                        && split_t < item.end() - RationalTime::from_seconds(0.05)
                    {
                        cmds.push(EditCommand::SplitObject {
                            item_id: item.id,
                            split_at: split_t,
                            new_right_id: Uuid::new_v4(),
                        });
                    }
                }
            }
        }
    }

    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Project, ProjectSettings, TimelineItem};

    #[test]
    fn speed_ramp_integral_is_strictly_monotone_and_bullet_time_slows_middle() {
        let cfg = SpeedRampPreset::BulletTimeDip.default_config();
        let dur = 4.0;
        let mut prev = -1.0;
        for step in 0..=40 {
            let t = dur * (step as f64 / 40.0);
            let mapped = evaluate_speed_ramp_source_time(t, dur, &cfg);
            assert!(
                mapped > prev,
                "remapped time must strictly increase: t={t} prev={prev} mapped={mapped}"
            );
            prev = mapped;
        }
        // 中央 (u=0.5) は 0.25x スロー、冒頭 (u=0.05) は 2.x 高速
        assert!(evaluate_speed_at_norm_time(&cfg, 0.5) < 0.3);
        assert!(evaluate_speed_at_norm_time(&cfg, 0.02) > 2.0);
    }

    #[test]
    fn build_scene_cut_commands_emits_markers_and_reverse_order_splits() {
        let mut proj = Project::new("CutTest", ProjectSettings::default());
        let item = TimelineItem::new(
            "ClipA",
            RationalTime::ZERO,
            RationalTime::from_seconds(6.0),
            ItemKind::GenerativeBg {
                config: crate::generative::GenerativeBgConfig::default(),
            },
        );
        proj.active_scene_mut().unwrap().layers[0].items.push(item);
        let cuts = vec![
            DetectedSceneCut {
                time_sec: 1.5,
                confidence: 0.92,
                hist_chi_sq: 0.85,
                luma_delta_ire: 28.0,
            },
            DetectedSceneCut {
                time_sec: 4.0,
                confidence: 0.88,
                hist_chi_sq: 0.72,
                luma_delta_ire: 22.0,
            },
        ];
        let cmds = build_scene_cut_commands(proj.active_scene().unwrap(), &cuts, true, Some(0));
        // 2 markers + 2 splits
        assert_eq!(cmds.len(), 4);
        assert!(matches!(cmds[0], EditCommand::AddMarker { .. }));
        assert!(matches!(cmds[2], EditCommand::SplitObject { .. }));
    }
}
