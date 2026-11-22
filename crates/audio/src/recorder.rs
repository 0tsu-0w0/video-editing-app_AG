//! v1.0 #19: マイクからの直接アフレコ録音（Voiceover Recorder）

use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use ve_media::write_wav_pcm16;

#[derive(Debug, Error)]
pub enum VoiceoverError {
    #[error("No audio input microphone device available")]
    NoInputDevice,
    #[error("Voiceover live microphone capture requires cpal-output feature on this OS")]
    LiveCaptureUnavailable,
    #[error("Recorder is not currently recording")]
    NotRecording,
    #[error("Audio capture backend error: {0}")]
    Backend(String),
    #[error("Failed to save recorded WAV: {0}")]
    Io(String),
}

/// マイク入力または外部 PCM バッファから音声を録音し、WAV ファイルとして書き出すレコーダー
pub struct VoiceoverRecorder {
    sample_rate: u32,
    channels: u16,
    recording: bool,
    samples: Arc<Mutex<Vec<f32>>>,
    #[cfg(any(windows, feature = "cpal-output"))]
    stream: Option<cpal::Stream>,
}

impl Default for VoiceoverRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceoverRecorder {
    pub fn new() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 1,
            recording: false,
            samples: Arc::new(Mutex::new(Vec::new())),
            #[cfg(any(windows, feature = "cpal-output"))]
            stream: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// 録音済みの長さ（秒）
    pub fn recorded_duration_sec(&self) -> f64 {
        let len = self.samples.lock().map(|v| v.len()).unwrap_or(0);
        let denom = (self.sample_rate.max(1) as f64) * (self.channels.max(1) as f64);
        len as f64 / denom
    }

    /// 最新の録音レベル（ピーク 0.0..=1.0）
    pub fn current_peak_level(&self) -> f32 {
        let guard = match self.samples.lock() {
            Ok(g) => g,
            Err(_) => return 0.0,
        };
        let tail = guard.len().saturating_sub(1024);
        guard[tail..]
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max)
            .clamp(0.0, 1.0)
    }

    /// OS の既定マイク入力デバイスからリアルタイム録音を開始する
    #[cfg(any(windows, feature = "cpal-output"))]
    pub fn start_live_mic(&mut self) -> Result<(), VoiceoverError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(VoiceoverError::NoInputDevice)?;
        let config = device
            .default_input_config()
            .map_err(|e| VoiceoverError::Backend(e.to_string()))?;

        self.sample_rate = config.sample_rate().0;
        self.channels = config.channels();
        if let Ok(mut guard) = self.samples.lock() {
            guard.clear();
        }
        let sink = Arc::clone(&self.samples);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _| {
                    if let Ok(mut buf) = sink.lock() {
                        buf.extend_from_slice(data);
                    }
                },
                |err| eprintln!("voiceover input stream error: {err}"),
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _| {
                    if let Ok(mut buf) = sink.lock() {
                        buf.extend(data.iter().map(|&s| f32::from(s) / 32768.0));
                    }
                },
                |err| eprintln!("voiceover input stream error: {err}"),
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.into(),
                move |data: &[u16], _| {
                    if let Ok(mut buf) = sink.lock() {
                        buf.extend(data.iter().map(|&s| (f32::from(s) - 32768.0) / 32768.0));
                    }
                },
                |err| eprintln!("voiceover input stream error: {err}"),
                None,
            ),
            other => {
                return Err(VoiceoverError::Backend(format!(
                    "unsupported input sample format: {other:?}"
                )));
            }
        }
        .map_err(|e| VoiceoverError::Backend(e.to_string()))?;

        stream
            .play()
            .map_err(|e| VoiceoverError::Backend(e.to_string()))?;
        self.stream = Some(stream);
        self.recording = true;
        Ok(())
    }

    #[cfg(not(any(windows, feature = "cpal-output")))]
    pub fn start_live_mic(&mut self) -> Result<(), VoiceoverError> {
        Err(VoiceoverError::LiveCaptureUnavailable)
    }

    /// テスト・ヘッドレス・仮想デバイス用のバッファ録音モードを開始する
    pub fn start_buffer_capture(&mut self, sample_rate: u32, channels: u16) {
        self.sample_rate = sample_rate.max(8_000);
        self.channels = channels.clamp(1, 2);
        if let Ok(mut guard) = self.samples.lock() {
            guard.clear();
        }
        self.recording = true;
    }

    /// バッファ録音モード中に PCM サンプルを追記する
    pub fn push_samples(&self, chunk: &[f32]) {
        if !self.recording {
            return;
        }
        if let Ok(mut guard) = self.samples.lock() {
            guard.extend_from_slice(chunk);
        }
    }

    /// 録音を停止し、16-bit PCM WAV ファイルとして保存する。戻り値は録音秒数。
    pub fn stop_and_save_wav(&mut self, output_path: &Path) -> Result<f64, VoiceoverError> {
        if !self.recording {
            return Err(VoiceoverError::NotRecording);
        }
        #[cfg(any(windows, feature = "cpal-output"))]
        {
            self.stream = None;
        }
        self.recording = false;

        let captured = self
            .samples
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        if let Some(parent) = output_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let duration_sec = captured.len() as f64
            / ((self.sample_rate.max(1) as f64) * (self.channels.max(1) as f64));
        write_wav_pcm16(output_path, &captured, self.sample_rate, self.channels)
            .map_err(|e| VoiceoverError::Io(e.to_string()))?;
        Ok(duration_sec)
    }

    /// 録音を破棄して停止する
    pub fn cancel(&mut self) {
        #[cfg(any(windows, feature = "cpal-output"))]
        {
            self.stream = None;
        }
        self.recording = false;
        if let Ok(mut guard) = self.samples.lock() {
            guard.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;
    use tempfile::tempdir;
    use ve_media::decode_audio_file;

    #[test]
    fn voiceover_buffer_capture_saves_valid_wav() {
        let tmp = tempdir().unwrap();
        let wav_path = tmp.path().join("assets").join("voiceover_take1.wav");

        let mut rec = VoiceoverRecorder::new();
        rec.start_buffer_capture(48_000, 1);
        assert!(rec.is_recording());

        let chunk: Vec<f32> = (0..24_000)
            .map(|i| (2.0 * PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        rec.push_samples(&chunk);
        assert!((rec.recorded_duration_sec() - 0.5).abs() < 1e-3);
        assert!(rec.current_peak_level() > 0.4);

        let dur = rec.stop_and_save_wav(&wav_path).unwrap();
        assert!(!rec.is_recording());
        assert!((dur - 0.5).abs() < 1e-3);
        assert!(wav_path.exists());

        let decoded = decode_audio_file(&wav_path).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.frame_count(), 24_000);
    }
}
