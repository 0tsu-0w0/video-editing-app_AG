//! v1.0 #17: 音声相互相関（Cross-Correlation）によるマルチカメラ自動同期エンジン

use ve_media::AudioBuffer;

/// 相互相関による同期オフセット推定結果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioSyncEstimate {
    /// ターゲットクリップをタイムライン上でシフトすべき時間（秒）。
    /// 例えば基準カメラで t=1.0s に鳴ったクラップ音が、ターゲットカメラで t=1.25s に録音されている場合、
    /// ターゲットを `-0.25s` シフトすれば基準と完全一致する。
    pub offset_sec: f64,
    /// 正規化相互相関係数（-1.0 ..= 1.0）
    pub correlation: f32,
    /// ピーク鋭さに基づく信頼度（0.0 ..= 1.0）
    pub confidence: f32,
}

impl Default for AudioSyncEstimate {
    fn default() -> Self {
        Self {
            offset_sec: 0.0,
            correlation: 0.0,
            confidence: 0.0,
        }
    }
}

/// モノラル化＋ボックスカー平均アンチエイリアシング付きダウンサンプリング
fn mixdown_and_downsample_mono(
    buf: &AudioBuffer,
    target_sr: u32,
    max_duration_sec: f64,
) -> Vec<f32> {
    if buf.sample_rate == 0 || buf.channels == 0 || buf.samples.is_empty() || target_sr == 0 {
        return Vec::new();
    }
    let src_frames = buf.frame_count();
    let max_src_frames = ((max_duration_sec * buf.sample_rate as f64).round() as usize)
        .min(src_frames)
        .max(1);
    let ch = buf.channels;
    let inv_ch = 1.0 / ch as f32;

    if buf.sample_rate == target_sr {
        let mut mono = Vec::with_capacity(max_src_frames);
        for f in 0..max_src_frames {
            let base = f * ch;
            let mut acc = 0.0f32;
            for c in 0..ch {
                acc += buf.samples[base + c];
            }
            mono.push(acc * inv_ch);
        }
        return mono;
    }

    let ratio = buf.sample_rate as f64 / target_sr as f64;
    let out_len = ((max_src_frames as f64) / ratio).floor().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let s0 = (i as f64 * ratio).floor() as usize;
        let s1 = (((i + 1) as f64 * ratio).ceil() as usize)
            .max(s0 + 1)
            .min(max_src_frames);
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for f in s0..s1 {
            let base = f * ch;
            for c in 0..ch {
                sum += buf.samples[base + c];
            }
            count += 1;
        }
        out.push(if count > 0 {
            (sum * inv_ch) / count as f32
        } else {
            0.0
        });
    }
    out
}

/// 広帯域音声でもエイリアシングしない RMS 振幅エンベロープ列を指定レート（例: 500 Hz）で抽出する
fn extract_rms_envelope(buf: &AudioBuffer, env_sr: u32, max_duration_sec: f64) -> Vec<f32> {
    if buf.sample_rate == 0 || buf.channels == 0 || buf.samples.is_empty() || env_sr == 0 {
        return Vec::new();
    }
    let src_frames = buf.frame_count();
    let max_src_frames = ((max_duration_sec * buf.sample_rate as f64).round() as usize)
        .min(src_frames)
        .max(1);
    let ch = buf.channels;
    let inv_ch = 1.0 / ch as f32;
    let ratio = buf.sample_rate as f64 / env_sr as f64;
    let out_len = ((max_src_frames as f64) / ratio).floor().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let s0 = (i as f64 * ratio).floor() as usize;
        let s1 = (((i + 1) as f64 * ratio).ceil() as usize)
            .max(s0 + 1)
            .min(max_src_frames);
        let mut sum_sq = 0.0f64;
        let mut count = 0usize;
        for f in s0..s1 {
            let base = f * ch;
            let mut acc = 0.0f32;
            for c in 0..ch {
                acc += buf.samples[base + c];
            }
            let m = f64::from(acc * inv_ch);
            sum_sq += m * m;
            count += 1;
        }
        out.push(if count > 0 {
            (sum_sq / count as f64).sqrt() as f32
        } else {
            0.0
        });
    }
    out
}

/// DC オフセットを除去しつつ正規化相互相関を指定ラグ範囲 `[min_lag, max_lag]` で計算する。
/// 戻り値: `(best_lag_samples, best_corr, second_best_corr)`
///
/// ラグ `lag` の定義: `ref[t]` と `target[t - lag]` を比較する。
/// つまり `lag > 0` は「ターゲットを `+lag` 後ろへシフトすると基準に一致する」ことを意味する。
fn search_ncc_lag(
    reference: &[f32],
    target: &[f32],
    min_lag: isize,
    max_lag: isize,
) -> (isize, f32, f32) {
    if reference.is_empty() || target.is_empty() || min_lag > max_lag {
        return (0, 0.0, 0.0);
    }
    let ref_mean = reference.iter().map(|&v| f64::from(v)).sum::<f64>() / reference.len() as f64;
    let tgt_mean = target.iter().map(|&v| f64::from(v)).sum::<f64>() / target.len() as f64;
    let ref_zm: Vec<f32> = reference
        .iter()
        .map(|&v| (f64::from(v) - ref_mean) as f32)
        .collect();
    let tgt_zm: Vec<f32> = target
        .iter()
        .map(|&v| (f64::from(v) - tgt_mean) as f32)
        .collect();

    let mut best_lag = 0isize;
    let mut best_corr = -1.0f32;
    let mut second_corr = -1.0f32;

    let ref_len = ref_zm.len() as isize;
    let tgt_len = tgt_zm.len() as isize;

    for lag in min_lag..=max_lag {
        // t in reference corresponds to u = t - lag in target
        let t_start = lag.max(0);
        let t_end = ref_len.min(tgt_len + lag);
        let overlap = t_end - t_start;
        if overlap < 16 {
            continue;
        }
        let mut dot = 0.0f64;
        let mut er = 0.0f64;
        let mut et = 0.0f64;
        let ts = t_start as usize;
        let us = (t_start - lag) as usize;
        let len = overlap as usize;
        for i in 0..len {
            let rv = f64::from(ref_zm[ts + i]);
            let tv = f64::from(tgt_zm[us + i]);
            dot += rv * tv;
            er += rv * rv;
            et += tv * tv;
        }
        let denom = (er * et).sqrt();
        let corr = if denom > 1e-9 {
            (dot / denom) as f32
        } else {
            0.0
        };
        if corr > best_corr {
            if (lag - best_lag).abs() > 2 {
                second_corr = second_corr.max(best_corr);
            }
            best_corr = corr;
            best_lag = lag;
        } else if (lag - best_lag).abs() > 2 && corr > second_corr {
            second_corr = corr;
        }
    }

    (best_lag, best_corr.max(0.0), second_corr.max(0.0))
}

/// 2 段階正規化相互相関（Stage 1: 500Hz RMS エンベロープ粗探索 → Stage 2: 8000Hz 波形精密探索）により、
/// 2 つの音声トラック間の同期オフセット（サブフレーム精度 0.125ms）を高速算出する。
pub fn estimate_audio_offset_xcorr(
    reference: &AudioBuffer,
    target: &AudioBuffer,
    max_lag_sec: f64,
) -> AudioSyncEstimate {
    let max_lag = max_lag_sec.clamp(0.05, 60.0);
    // 長尺ファイルでも高速に同期できるよう先頭最大 45 秒を解析
    let analysis_window_sec = (max_lag * 2.5).clamp(10.0, 45.0);

    // Stage 1: 500 Hz RMS エンベロープ粗探索（2ms 分解能・エイリアシングフリー）
    let coarse_sr = 500u32;
    let ref_coarse = extract_rms_envelope(reference, coarse_sr, analysis_window_sec);
    let tgt_coarse = extract_rms_envelope(target, coarse_sr, analysis_window_sec);
    if ref_coarse.len() < 32 || tgt_coarse.len() < 32 {
        return AudioSyncEstimate::default();
    }
    let coarse_max_lag = (max_lag * coarse_sr as f64).round() as isize;
    let (coarse_lag, coarse_corr, _) =
        search_ncc_lag(&ref_coarse, &tgt_coarse, -coarse_max_lag, coarse_max_lag);
    if coarse_corr < 0.05 {
        return AudioSyncEstimate::default();
    }
    let coarse_sec = coarse_lag as f64 / coarse_sr as f64;

    // Stage 2: 8000 Hz 精密探索（粗探索結果の前後 ±0.035s = ±280 サンプルを 0.125ms 分解能で精査）
    let fine_sr = 8_000u32;
    let ref_fine = mixdown_and_downsample_mono(reference, fine_sr, analysis_window_sec);
    let tgt_fine = mixdown_and_downsample_mono(target, fine_sr, analysis_window_sec);
    let center = (coarse_sec * fine_sr as f64).round() as isize;
    let radius = (0.035 * fine_sr as f64).round() as isize;
    let hard_max = (max_lag * fine_sr as f64).round() as isize;
    let fine_min = (center - radius).max(-hard_max);
    let fine_max = (center + radius).min(hard_max);

    let (fine_lag, fine_corr, second_corr) =
        search_ncc_lag(&ref_fine, &tgt_fine, fine_min, fine_max);
    // 位相反転や強い室内残響で波形相関がエンベロープ相関より低い場合は粗探索結果を採用
    let (offset_sec, best_corr) = if fine_corr >= coarse_corr * 0.5 {
        (fine_lag as f64 / fine_sr as f64, fine_corr)
    } else {
        (coarse_sec, coarse_corr)
    };
    let sharpness = ((best_corr - second_corr).max(0.0) * 2.5).min(1.0);
    let confidence = (best_corr * 0.7 + sharpness * 0.3).clamp(0.0, 1.0);

    AudioSyncEstimate {
        offset_sec,
        correlation: best_corr,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn make_chirp_burst(sr: u32, duration_sec: f64, burst_start_sec: f64) -> AudioBuffer {
        let len = (sr as f64 * duration_sec).round() as usize;
        let mut samples = vec![0.0f32; len];
        let start = (sr as f64 * burst_start_sec).round() as usize;
        let burst_len = (sr as f64 * 0.25).round() as usize;
        for i in 0..burst_len {
            if start + i >= len {
                break;
            }
            let t = i as f32 / sr as f32;
            let env = (PI * i as f32 / burst_len as f32).sin();
            // 広帯域スイープ（300Hz → 2200Hz）で鋭い相互相関ピークを生成
            let phase = 2.0 * PI * (300.0 * t + 3800.0 * t * t);
            samples[start + i] = phase.sin() * env * 0.85;
        }
        AudioBuffer {
            sample_rate: sr,
            channels: 1,
            samples,
        }
    }

    #[test]
    fn xcorr_recovers_positive_and_negative_subframe_offsets() {
        let sr = 48_000;
        let reference = make_chirp_burst(sr, 2.0, 0.800);
        // target のクラップ音が 0.525s（基準より 0.275s 早い）にある -> target を +0.275s 後ろへシフトすべき
        let target_early = make_chirp_burst(sr, 2.0, 0.525);
        let est_pos = estimate_audio_offset_xcorr(&reference, &target_early, 1.0);
        assert!(
            (est_pos.offset_sec - 0.275).abs() < 0.002,
            "expected +0.275s, got {est_pos:?}"
        );
        assert!(est_pos.correlation > 0.9, "est={est_pos:?}");

        // target のクラップ音が 1.150s（基準より 0.350s 遅い）にある -> target を -0.350s 前へシフトすべき
        let target_late = make_chirp_burst(sr, 2.0, 1.150);
        let est_neg = estimate_audio_offset_xcorr(&reference, &target_late, 1.0);
        assert!(
            (est_neg.offset_sec - (-0.350)).abs() < 0.002,
            "expected -0.350s, got {est_neg:?}"
        );
        assert!(est_neg.correlation > 0.9, "est={est_neg:?}");
    }

    #[test]
    fn xcorr_handles_silence_safely() {
        let silence = AudioBuffer {
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.0; 48_000 * 2],
        };
        let est = estimate_audio_offset_xcorr(&silence, &silence, 1.0);
        assert_eq!(est.offset_sec, 0.0);
        assert_eq!(est.correlation, 0.0);
    }
}
