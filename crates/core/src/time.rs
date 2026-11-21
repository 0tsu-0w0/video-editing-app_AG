use serde::{Deserialize, Serialize};
use std::ops::{Add, Sub};

/// 1秒あたりのティック数（705,600,000 = Flicks）。
/// 24/25/30/50/60/120fps、NTSC (23.976/29.97/59.94fps)、
/// および 44.1kHz / 48kHz / 96kHz 音声サンプルをすべて誤差ゼロの整数で表現できる。
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// 有理数フレームレート（例: 60fps = 60/1, 29.97fps = 30000/1001）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const FPS_24: Self = Self { num: 24, den: 1 };
    pub const FPS_23_976: Self = Self {
        num: 24_000,
        den: 1_001,
    };
    pub const FPS_29_97: Self = Self {
        num: 30_000,
        den: 1_001,
    };
    pub const FPS_30: Self = Self { num: 30, den: 1 };
    pub const FPS_59_94: Self = Self {
        num: 60_000,
        den: 1_001,
    };
    pub const FPS_60: Self = Self { num: 60, den: 1 };

    pub const fn new(num: u32, den: u32) -> Self {
        assert!(
            num > 0 && den > 0,
            "FrameRate numerator and denominator must be > 0"
        );
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// 1フレームあたりのティック数（整数丸め；標準FPSはすべて厳密一致）
    pub fn ticks_per_frame(self) -> i64 {
        let num = i128::from(TICKS_PER_SECOND) * i128::from(self.den);
        let den = i128::from(self.num);
        ((num + den / 2) / den) as i64
    }
}

/// 内部時間表現（有理数タイムベース: `ticks / TICKS_PER_SECOND` 秒）
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct RationalTime {
    pub ticks: i64,
}

impl RationalTime {
    pub const ZERO: Self = Self { ticks: 0 };

    pub const fn from_ticks(ticks: i64) -> Self {
        Self { ticks }
    }

    pub fn from_seconds(seconds: f64) -> Self {
        Self {
            ticks: (seconds * TICKS_PER_SECOND as f64).round() as i64,
        }
    }

    pub fn as_seconds(self) -> f64 {
        self.ticks as f64 / TICKS_PER_SECOND as f64
    }

    pub fn from_frame(frame: i64, fps: FrameRate) -> Self {
        let ticks = (i128::from(frame) * i128::from(TICKS_PER_SECOND) * i128::from(fps.den))
            / i128::from(fps.num);
        Self {
            ticks: ticks as i64,
        }
    }

    pub fn to_frame_floor(self, fps: FrameRate) -> i64 {
        let num = i128::from(self.ticks) * i128::from(fps.num);
        let den = i128::from(TICKS_PER_SECOND) * i128::from(fps.den);
        num.div_euclid(den) as i64
    }

    pub fn to_frame_round(self, fps: FrameRate) -> i64 {
        let num = i128::from(self.ticks) * i128::from(fps.num);
        let den = i128::from(TICKS_PER_SECOND) * i128::from(fps.den);
        if num >= 0 {
            ((num + den / 2) / den) as i64
        } else {
            ((num - den / 2) / den) as i64
        }
    }

    pub fn from_audio_samples(samples: i64, sample_rate: u32) -> Self {
        let ticks = (i128::from(samples) * i128::from(TICKS_PER_SECOND)) / i128::from(sample_rate);
        Self {
            ticks: ticks as i64,
        }
    }

    pub fn to_audio_samples(self, sample_rate: u32) -> i64 {
        let samples =
            (i128::from(self.ticks) * i128::from(sample_rate)) / i128::from(TICKS_PER_SECOND);
        samples as i64
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            ticks: self.ticks.max(other.ticks),
        }
    }

    pub fn min(self, other: Self) -> Self {
        Self {
            ticks: self.ticks.min(other.ticks),
        }
    }
}

impl Add for RationalTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            ticks: self.ticks.saturating_add(rhs.ticks),
        }
    }
}

impl Sub for RationalTime {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            ticks: self.ticks.saturating_sub(rhs.ticks),
        }
    }
}

/// UI・MCP入出力向けの時間指定（フレーム番号・秒・内部ティックの相互変換）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "snake_case")]
pub enum TimeInput {
    Frames(i64),
    Seconds(f64),
    Ticks(i64),
}

impl TimeInput {
    pub fn resolve(self, fps: FrameRate) -> RationalTime {
        match self {
            Self::Frames(f) => RationalTime::from_frame(f, fps),
            Self::Seconds(s) => RationalTime::from_seconds(s),
            Self::Ticks(t) => RationalTime::from_ticks(t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_frame_roundtrip_all_standard_fps() {
        let rates = [
            FrameRate::FPS_24,
            FrameRate::FPS_23_976,
            FrameRate::FPS_29_97,
            FrameRate::FPS_30,
            FrameRate::FPS_59_94,
            FrameRate::FPS_60,
        ];
        for fps in rates {
            for frame in [0, 1, 29, 30, 59, 60, 1000, 3600] {
                let t = RationalTime::from_frame(frame, fps);
                assert_eq!(t.to_frame_round(fps), frame, "fps={fps:?}, frame={frame}");
                assert_eq!(t.to_frame_floor(fps), frame, "fps={fps:?}, frame={frame}");
            }
        }
    }

    #[test]
    fn exact_audio_sample_roundtrip() {
        for sr in [44_100u32, 48_000, 96_000] {
            for samples in [0i64, 1, 441, 480, sr as i64, sr as i64 * 10] {
                let t = RationalTime::from_audio_samples(samples, sr);
                assert_eq!(
                    t.to_audio_samples(sr),
                    samples,
                    "sr={sr}, samples={samples}"
                );
            }
        }
    }

    #[test]
    fn time_input_frames_and_seconds() {
        let fps = FrameRate::FPS_60;
        let from_frames = TimeInput::Frames(90).resolve(fps);
        let from_secs = TimeInput::Seconds(1.5).resolve(fps);
        assert_eq!(from_frames, from_secs);
        assert!((from_frames.as_seconds() - 1.5).abs() < 1e-9);
    }
}
