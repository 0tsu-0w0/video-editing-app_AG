use ve_media::AudioBuffer;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveformBucket {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

/// タイムライン高精細描画＆口パク音量連動用の事前解析波形データ
#[derive(Debug, Clone)]
pub struct WaveformPeaks {
    pub sample_rate: u32,
    pub samples_per_bucket: usize,
    pub buckets: Vec<WaveformBucket>,
}

impl WaveformPeaks {
    pub fn from_buffer(buffer: &AudioBuffer, buckets_per_second: usize) -> Self {
        let buckets_per_second = buckets_per_second.max(1);
        let samples_per_bucket = (buffer.sample_rate as usize / buckets_per_second).max(1);
        let frames = buffer.frame_count();
        let ch = buffer.channels.max(1);
        let mut buckets = Vec::with_capacity(frames.div_ceil(samples_per_bucket));

        let mut frame = 0;
        while frame < frames {
            let end = (frame + samples_per_bucket).min(frames);
            let mut min_v = 0.0f32;
            let mut max_v = 0.0f32;
            let mut sum_sq = 0.0f64;
            let mut count = 0usize;

            for f in frame..end {
                let mut mono = 0.0f32;
                for c in 0..ch {
                    mono += buffer.samples[f * ch + c];
                }
                mono /= ch as f32;
                min_v = min_v.min(mono);
                max_v = max_v.max(mono);
                sum_sq += f64::from(mono) * f64::from(mono);
                count += 1;
            }
            let rms = if count > 0 {
                (sum_sq / count as f64).sqrt() as f32
            } else {
                0.0
            };
            buckets.push(WaveformBucket {
                min: min_v,
                max: max_v,
                rms,
            });
            frame = end;
        }

        Self {
            sample_rate: buffer.sample_rate,
            samples_per_bucket,
            buckets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_peaks_and_rms_buckets() {
        let sr = 48_000u32;
        let mut samples = Vec::with_capacity(sr as usize);
        for i in 0..sr as usize {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.8;
            samples.push(v);
        }
        let buf = AudioBuffer {
            sample_rate: sr,
            channels: 1,
            samples,
        };
        let peaks = WaveformPeaks::from_buffer(&buf, 100);
        assert_eq!(peaks.buckets.len(), 100);
        assert!(peaks.buckets[0].max > 0.75);
        assert!(peaks.buckets[0].min < -0.75);
        assert!(peaks.buckets[0].rms > 0.5);
    }
}
