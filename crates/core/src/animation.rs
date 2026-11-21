use crate::time::RationalTime;
use serde::{Deserialize, Serialize};

/// 3次ベジェ曲線（P0=(0,0), P1=(x1,y1), P2=(x2,y2), P3=(1,1)）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl Default for CubicBezier {
    fn default() -> Self {
        Self::EASE_IN_OUT
    }
}

impl CubicBezier {
    pub const LINEAR: Self = Self {
        x1: 0.0,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    pub const EASE_OUT: Self = Self {
        x1: 0.0,
        y1: 0.0,
        x2: 0.2,
        y2: 1.0,
    };
    pub const EASE_IN_OUT: Self = Self {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };

    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self {
            x1: x1.clamp(0.0, 1.0),
            y1,
            x2: x2.clamp(0.0, 1.0),
            y2,
        }
    }

    #[inline]
    fn sample_x(self, u: f32) -> f32 {
        let inv = 1.0 - u;
        3.0 * inv * inv * u * self.x1 + 3.0 * inv * u * u * self.x2 + u * u * u
    }

    #[inline]
    fn sample_y(self, u: f32) -> f32 {
        let inv = 1.0 - u;
        3.0 * inv * inv * u * self.y1 + 3.0 * inv * u * u * self.y2 + u * u * u
    }

    #[inline]
    fn sample_dx(self, u: f32) -> f32 {
        let inv = 1.0 - u;
        3.0 * inv * inv * self.x1
            + 6.0 * inv * u * (self.x2 - self.x1)
            + 3.0 * u * u * (1.0 - self.x2)
    }

    #[inline]
    fn sample_dy(self, u: f32) -> f32 {
        let inv = 1.0 - u;
        3.0 * inv * inv * self.y1
            + 6.0 * inv * u * (self.y2 - self.y1)
            + 3.0 * u * u * (1.0 - self.y2)
    }

    /// 入力進捗 `x` (0..=1) に対応するパラメータ `u` を求める
    fn solve_u(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        // Newton-Raphson
        let mut u = x;
        for _ in 0..6 {
            let err = self.sample_x(u) - x;
            if err.abs() < 1e-5 {
                return u;
            }
            let dx = self.sample_dx(u);
            if dx.abs() < 1e-6 {
                break;
            }
            u = (u - err / dx).clamp(0.0, 1.0);
        }
        // フォールバック二分探索
        let mut lo = 0.0f32;
        let mut hi = 1.0f32;
        for _ in 0..16 {
            let mid = 0.5 * (lo + hi);
            if self.sample_x(mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// 値グラフ評価: 進捗 `t` (0..=1) に対するイージング済み進捗 y
    pub fn evaluate(self, t: f32) -> f32 {
        let u = self.solve_u(t);
        self.sample_y(u)
    }

    /// 速度グラフ評価: 進捗 `t` (0..=1) における正規化微分 dy/dt
    pub fn derivative(self, t: f32) -> f32 {
        let u = self.solve_u(t);
        let dx = self.sample_dx(u);
        let dy = self.sample_dy(u);
        if dx.abs() < 1e-4 {
            dy / 1e-4_f32.copysign(dx)
        } else {
            dy / dx
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Interpolation {
    Hold,
    Linear,
    Bezier(CubicBezier),
}

/// AviUtl2の中間点とAEのキーフレームを統一表現するデータモデル
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// アイテム先頭からのローカル時刻
    pub time: RationalTime,
    pub value: f32,
    pub interpolation: Interpolation,
}

/// キーフレーム列を評価し、指定ローカル時刻の値を返す（値グラフ用）
pub fn evaluate_track(base_value: f32, keyframes: &[Keyframe], local_time: RationalTime) -> f32 {
    if keyframes.is_empty() {
        return base_value;
    }
    if local_time <= keyframes[0].time {
        return keyframes[0].value;
    }
    let last = keyframes.last().unwrap();
    if local_time >= last.time {
        return last.value;
    }

    for win in keyframes.windows(2) {
        let k0 = win[0];
        let k1 = win[1];
        if local_time >= k0.time && local_time <= k1.time {
            let span = (k1.time - k0.time).ticks as f64;
            if span <= 0.0 {
                return k1.value;
            }
            let t = ((local_time - k0.time).ticks as f64 / span) as f32;
            let eased = match k0.interpolation {
                Interpolation::Hold => 0.0,
                Interpolation::Linear => t,
                Interpolation::Bezier(b) => b.evaluate(t),
            };
            return k0.value + (k1.value - k0.value) * eased;
        }
    }

    last.value
}

/// 指定ローカル時刻における変化速度（単位/秒）を返す（速度グラフ用）
pub fn evaluate_velocity(keyframes: &[Keyframe], local_time: RationalTime) -> f32 {
    if keyframes.len() < 2 {
        return 0.0;
    }
    if local_time < keyframes[0].time || local_time > keyframes.last().unwrap().time {
        return 0.0;
    }

    for win in keyframes.windows(2) {
        let k0 = win[0];
        let k1 = win[1];
        if local_time >= k0.time && local_time <= k1.time {
            let dt_sec = (k1.time - k0.time).as_seconds() as f32;
            if dt_sec <= 1e-6 {
                return 0.0;
            }
            let dv = k1.value - k0.value;
            let t = ((local_time - k0.time).as_seconds() as f32 / dt_sec).clamp(0.0, 1.0);
            let slope = match k0.interpolation {
                Interpolation::Hold => 0.0,
                Interpolation::Linear => 1.0,
                Interpolation::Bezier(b) => b.derivative(t),
            };
            return (dv / dt_sec) * slope;
        }
    }

    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_value_and_velocity_curves() {
        let b = CubicBezier::EASE_IN_OUT;
        assert!((b.evaluate(0.0) - 0.0).abs() < 1e-4);
        assert!((b.evaluate(0.5) - 0.5).abs() < 1e-3);
        assert!((b.evaluate(1.0) - 1.0).abs() < 1e-4);
        // Ease-in-out は中央 (t=0.5) の速度が両端より大きい
        assert!(b.derivative(0.5) > b.derivative(0.05));
        assert!(b.derivative(0.5) > b.derivative(0.95));
    }

    #[test]
    fn evaluate_track_and_velocity_across_keyframes() {
        let kfs = vec![
            Keyframe {
                time: RationalTime::ZERO,
                value: 0.0,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
            },
            Keyframe {
                time: RationalTime::from_seconds(2.0),
                value: 200.0,
                interpolation: Interpolation::Linear,
            },
        ];
        let mid_v = evaluate_track(0.0, &kfs, RationalTime::from_seconds(1.0));
        assert!((mid_v - 100.0).abs() < 1.0);

        let mid_vel = evaluate_velocity(&kfs, RationalTime::from_seconds(1.0));
        // Ease-in-out 中央は平均速度 100/sec より速い
        assert!(mid_vel > 120.0, "expected mid_vel > 120, got {mid_vel}");
    }
}
