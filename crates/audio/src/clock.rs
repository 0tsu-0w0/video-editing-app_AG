use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::time::Instant;
use ve_core::{FrameRate, RationalTime};

/// 音声出力サンプル位置をマスタークロックとして A/V 同期および J/K/L シャトル再生を行うクロック
#[derive(Debug)]
pub struct AudioMasterClock {
    sample_rate: u32,
    played_samples: AtomicI64,
    playing: AtomicBool,
    /// J/K/L シャトル再生レート（-4.0..=4.0、既定 1.0）
    playback_rate: AtomicU32,
    fallback_epoch: Option<Instant>,
    fallback_start_samples: i64,
}

impl AudioMasterClock {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            played_samples: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            playback_rate: AtomicU32::new(1.0f32.to_bits()),
            fallback_epoch: None,
            fallback_start_samples: 0,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn playback_rate(&self) -> f32 {
        f32::from_bits(self.playback_rate.load(Ordering::Relaxed))
    }

    /// J/K/L シャトル再生速度を設定する（負=逆再生、正=通常/倍速）
    pub fn set_playback_rate(&mut self, rate: f32) {
        let clamped = if rate.is_finite() {
            rate.clamp(-8.0, 8.0)
        } else {
            1.0
        };
        let current = self.current_sample();
        self.played_samples.store(current, Ordering::Release);
        self.fallback_start_samples = current;
        if self.is_playing() {
            self.fallback_epoch = Some(Instant::now());
        }
        self.playback_rate
            .store(clamped.to_bits(), Ordering::Release);
    }

    pub fn play(&mut self) {
        if self.playback_rate().abs() < 0.01 {
            self.playback_rate
                .store(1.0f32.to_bits(), Ordering::Release);
        }
        self.fallback_start_samples = self.played_samples.load(Ordering::Relaxed);
        self.fallback_epoch = Some(Instant::now());
        self.playing.store(true, Ordering::Release);
    }

    pub fn pause(&mut self) {
        let current = self.current_sample();
        self.played_samples.store(current, Ordering::Release);
        self.fallback_epoch = None;
        self.playing.store(false, Ordering::Release);
    }

    pub fn seek(&mut self, time: RationalTime) {
        let sample = time.to_audio_samples(self.sample_rate).max(0);
        self.played_samples.store(sample, Ordering::Release);
        self.fallback_start_samples = sample;
        if self.is_playing() {
            self.fallback_epoch = Some(Instant::now());
        }
    }

    /// 音声デバイスコールバックから呼ばれ、再生済みサンプル数を進める（レート対応）
    pub fn advance_by_audio_callback(&self, frames: usize) {
        if self.playing.load(Ordering::Relaxed) {
            let rate = f64::from(self.playback_rate());
            let delta = (frames as f64 * rate).round() as i64;
            let cur = self.played_samples.load(Ordering::Acquire);
            let next = (cur + delta).max(0);
            self.played_samples.store(next, Ordering::Release);
        }
    }

    /// 音声デバイス未接続時（ヘッドレス／フォールバック）の時刻更新
    pub fn tick_fallback_clock(&self) {
        if !self.playing.load(Ordering::Relaxed) {
            return;
        }
        if let Some(epoch) = self.fallback_epoch {
            let elapsed = epoch.elapsed().as_secs_f64();
            let rate = f64::from(self.playback_rate());
            let delta = (elapsed * self.sample_rate as f64 * rate).round() as i64;
            let target = (self.fallback_start_samples + delta).max(0);
            self.played_samples.store(target, Ordering::Release);
        }
    }

    pub fn current_sample(&self) -> i64 {
        self.played_samples.load(Ordering::Acquire)
    }

    pub fn current_time(&self) -> RationalTime {
        RationalTime::from_audio_samples(self.current_sample(), self.sample_rate)
    }

    /// A/V 同期: 現在の音声マスタークロックに対応する映像フレーム番号を返す
    pub fn current_video_frame(&self, fps: FrameRate) -> i64 {
        self.current_time().to_frame_floor(fps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_callback_drives_video_frame_sync() {
        let mut clock = AudioMasterClock::new(48_000);
        clock.play();
        // 48,000 samples = ちょうど 1.0 秒 = 60fps でフレーム 60
        clock.advance_by_audio_callback(48_000);
        assert_eq!(clock.current_video_frame(FrameRate::FPS_60), 60);
        assert_eq!(clock.current_video_frame(FrameRate::FPS_30), 30);
    }

    #[test]
    fn jkl_shuttle_rates_advance_and_rewind_safely() {
        let mut clock = AudioMasterClock::new(48_000);
        clock.set_playback_rate(2.0);
        clock.play();
        clock.advance_by_audio_callback(24_000); // 2x => 48,000 samples (1.0s)
        assert_eq!(clock.current_video_frame(FrameRate::FPS_60), 60);

        clock.set_playback_rate(-1.0);
        clock.advance_by_audio_callback(24_000); // -1x => back 0.5s
        assert_eq!(clock.current_video_frame(FrameRate::FPS_60), 30);

        // 0未満には巻き戻らない
        clock.advance_by_audio_callback(96_000);
        assert_eq!(clock.current_sample(), 0);
    }
}
