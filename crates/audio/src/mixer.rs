use std::sync::Arc;
use ve_core::RationalTime;
use ve_media::AudioBuffer;

#[derive(Debug, Clone)]
pub struct AudioClip {
    pub timeline_start: RationalTime,
    pub duration: RationalTime,
    pub source_offset: RationalTime,
    pub gain: f32,
    pub muted: bool,
    pub buffer: Arc<AudioBuffer>,
}

/// プレビュー再生・FFmpeg書き出し共通の Audio Graph（リサンプリング＋ステレオミキサー）
#[derive(Debug, Clone)]
pub struct AudioGraph {
    pub sample_rate: u32,
    pub channels: usize,
    pub master_gain: f32,
    pub clips: Vec<AudioClip>,
}

impl AudioGraph {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channels: 2,
            master_gain: 1.0,
            clips: Vec::new(),
        }
    }

    pub fn add_clip(&mut self, clip: AudioClip) {
        self.clips.push(clip);
    }

    /// 指定サンプル位置 `start_sample` から `frame_count` フレーム分のステレオPCMをレンダリングする
    pub fn render_block(&self, start_sample: i64, frame_count: usize, out: &mut [f32]) {
        let total_samples = frame_count * self.channels;
        debug_assert!(out.len() >= total_samples);
        out[..total_samples].fill(0.0);

        if self.sample_rate == 0 || frame_count == 0 {
            return;
        }

        let block_start_t = RationalTime::from_audio_samples(start_sample, self.sample_rate);
        let block_end_t =
            RationalTime::from_audio_samples(start_sample + frame_count as i64, self.sample_rate);

        for clip in &self.clips {
            if clip.muted
                || clip.gain <= 0.0
                || clip.buffer.sample_rate == 0
                || clip.buffer.channels == 0
            {
                continue;
            }
            let clip_end = clip.timeline_start + clip.duration;
            if clip_end <= block_start_t || clip.timeline_start >= block_end_t {
                continue;
            }

            let src_sr = clip.buffer.sample_rate as f64;
            let dst_sr = self.sample_rate as f64;
            let src_frames = clip.buffer.frame_count();
            if src_frames == 0 {
                continue;
            }
            let src_ch = clip.buffer.channels;
            let clip_start_sec = clip.timeline_start.as_seconds();
            let clip_end_sec = clip_end.as_seconds();
            let offset_sec = clip.source_offset.as_seconds();

            for i in 0..frame_count {
                let timeline_sec = (start_sample + i as i64) as f64 / dst_sr;
                if timeline_sec < clip_start_sec || timeline_sec >= clip_end_sec {
                    continue;
                }
                let src_sec = (timeline_sec - clip_start_sec) + offset_sec;
                if src_sec < 0.0 {
                    continue;
                }
                let src_pos = src_sec * src_sr;
                let idx0 = src_pos.floor() as usize;
                if idx0 >= src_frames {
                    continue;
                }
                let idx1 = (idx0 + 1).min(src_frames - 1);
                let frac = (src_pos - idx0 as f64) as f32;

                let (l, r) = if src_ch == 1 {
                    let s0 = clip.buffer.samples[idx0];
                    let s1 = clip.buffer.samples[idx1];
                    let v = s0 + (s1 - s0) * frac;
                    (v, v)
                } else {
                    let l0 = clip.buffer.samples[idx0 * src_ch];
                    let r0 = clip.buffer.samples[idx0 * src_ch + 1];
                    let l1 = clip.buffer.samples[idx1 * src_ch];
                    let r1 = clip.buffer.samples[idx1 * src_ch + 1];
                    (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac)
                };

                let base = i * self.channels;
                out[base] += l * clip.gain;
                if self.channels > 1 {
                    out[base + 1] += r * clip.gain;
                }
            }
        }

        // マスターゲイン適用とクリッピング防止
        for s in &mut out[..total_samples] {
            *s = (*s * self.master_gain).clamp(-1.0, 1.0);
        }
    }

    /// 書き出し用: タイムライン全体の音声を一括レンダリングする
    pub fn render_range(&self, start: RationalTime, duration: RationalTime) -> Vec<f32> {
        let start_sample = start.to_audio_samples(self.sample_rate);
        let frames = duration.to_audio_samples(self.sample_rate).max(0) as usize;
        let mut out = vec![0.0f32; frames * self.channels];
        self.render_block(start_sample, frames, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_44100_into_48000_and_mixes_tracks() {
        // 44.1kHz モノラル一定値 0.25 のクリップ
        let buf_44k = Arc::new(AudioBuffer {
            sample_rate: 44_100,
            channels: 1,
            samples: vec![0.25; 44_100],
        });
        // 48kHz ステレオ一定値 0.30 のクリップ
        let buf_48k = Arc::new(AudioBuffer {
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.30; 48_000 * 2],
        });

        let mut graph = AudioGraph::new(48_000);
        graph.add_clip(AudioClip {
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            buffer: buf_44k,
        });
        graph.add_clip(AudioClip {
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            buffer: buf_48k,
        });

        let rendered = graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(0.1));
        assert_eq!(rendered.len(), 4_800 * 2);
        for &s in &rendered[10..100] {
            assert!((s - 0.55).abs() < 1e-4, "expected ~0.55, got {s}");
        }
    }
}
