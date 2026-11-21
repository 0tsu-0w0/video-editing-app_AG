//! v0.8 #12: BGM ビート自動検出（Pure-Rust オンセット強度・自己相関 BPM 推定・拍位置精密アライメント）

use crate::waveform::WaveformPeaks;

/// ビート自動検出パラメータ
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatDetectionConfig {
    /// 推定対象 BPM の下限（既定 70.0）
    pub min_bpm: f32,
    /// 推定対象 BPM の上限（既定 180.0）
    pub max_bpm: f32,
    /// オンセット検出のローカル平均に対する倍率しきい値（既定 1.35）
    pub sensitivity: f32,
    /// 1 小節あたりの拍数（既定 4 拍子）
    pub beats_per_bar: usize,
}

impl Default for BeatDetectionConfig {
    fn default() -> Self {
        Self {
            min_bpm: 70.0,
            max_bpm: 180.0,
            sensitivity: 1.35,
            beats_per_bar: 4,
        }
    }
}

/// 検出された 1 拍分の情報
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatPoint {
    pub time_sec: f64,
    pub strength: f32,
    /// 小節頭（ダウンビート）かどうか
    pub is_downbeat: bool,
}

/// ビート解析結果（BPM・信頼度・ビート列・生オンセット時刻）
#[derive(Debug, Clone, PartialEq)]
pub struct BeatAnalysisResult {
    pub bpm: f32,
    pub confidence: f32,
    pub beats: Vec<BeatPoint>,
    pub onset_times_sec: Vec<f64>,
}

/// ステレオインターリーブ PCM からオンセット・BPM・拍位置（小節頭付き）を自動検出する
pub fn detect_audio_beats(
    stereo_pcm: &[f32],
    sample_rate: u32,
    cfg: &BeatDetectionConfig,
) -> BeatAnalysisResult {
    let sr = sample_rate.max(8_000);
    let frames = stereo_pcm.as_chunks::<2>().0;
    let total_sec = frames.len() as f64 / f64::from(sr);
    if frames.len() < (sr as usize / 2) {
        return BeatAnalysisResult {
            bpm: 0.0,
            confidence: 0.0,
            beats: Vec::new(),
            onset_times_sec: Vec::new(),
        };
    }

    // 10ms ホップ（100 Hz フレームレート）で低域＋広帯域のエネルギー差分（オンセット包絡）を算出
    let hop = ((sr as usize) / 100).max(64);
    let win = hop * 2;
    let hop_sec = hop as f64 / f64::from(sr);
    let num_frames = frames.len().saturating_sub(win) / hop + 1;
    if num_frames < 16 {
        return BeatAnalysisResult {
            bpm: 0.0,
            confidence: 0.0,
            beats: Vec::new(),
            onset_times_sec: Vec::new(),
        };
    }

    // 簡易 1次ローパス（~200Hz）でキック・ベースの低域トランジェントを強調
    let dt = 1.0 / f32::from(sr as u16).max(8000.0);
    let rc = 1.0 / (2.0 * std::f32::consts::PI * 220.0);
    let lp_alpha = (dt / (rc + dt)).clamp(0.005, 0.5);

    let mut mono = Vec::with_capacity(frames.len());
    let mut low = Vec::with_capacity(frames.len());
    let mut lp_state = 0.0f32;
    for &[l, r] in frames {
        let m = 0.5 * (l + r);
        lp_state += lp_alpha * (m - lp_state);
        mono.push(m);
        low.push(lp_state);
    }

    let mut onset_env = vec![0.0f32; num_frames];
    let mut prev_low_e = 0.0f32;
    let mut prev_full_e = 0.0f32;
    for (idx, slot) in onset_env.iter_mut().enumerate() {
        let s0 = idx * hop;
        let s1 = (s0 + win).min(mono.len());
        let n = (s1 - s0).max(1) as f32;
        let mut low_sq = 0.0f32;
        let mut full_sq = 0.0f32;
        for i in s0..s1 {
            let lv = low[i];
            let mv = mono[i];
            low_sq += lv * lv;
            full_sq += mv * mv;
        }
        let low_e = (low_sq / n).sqrt();
        let full_e = (full_sq / n).sqrt();
        let flux_low = (low_e - prev_low_e).max(0.0);
        let flux_full = (full_e - prev_full_e).max(0.0);
        // 低域キックに 1.6 倍の重みを乗せて安定検出
        *slot = flux_low * 1.6 + flux_full;
        prev_low_e = low_e;
        prev_full_e = full_e;
    }

    analyze_onset_envelope(&onset_env, hop_sec, total_sec, cfg)
}

/// 既に生成済みの `WaveformPeaks` のみからビートを推定するフォールバック経路
pub fn detect_beats_from_waveform(
    peaks: &WaveformPeaks,
    cfg: &BeatDetectionConfig,
) -> BeatAnalysisResult {
    if peaks.buckets.len() < 16 || peaks.sample_rate == 0 || peaks.samples_per_bucket == 0 {
        return BeatAnalysisResult {
            bpm: 0.0,
            confidence: 0.0,
            beats: Vec::new(),
            onset_times_sec: Vec::new(),
        };
    }
    let hop_sec = (peaks.samples_per_bucket as f64) / f64::from(peaks.sample_rate);
    let total_sec = peaks.buckets.len() as f64 * hop_sec;
    let mut onset_env = vec![0.0f32; peaks.buckets.len()];
    let mut prev = 0.0f32;
    for (i, b) in peaks.buckets.iter().enumerate() {
        let e = b.rms.max(b.max.abs() * 0.5);
        onset_env[i] = (e - prev).max(0.0);
        prev = e;
    }
    analyze_onset_envelope(&onset_env, hop_sec, total_sec, cfg)
}

fn analyze_onset_envelope(
    onset_env: &[f32],
    hop_sec: f64,
    total_sec: f64,
    cfg: &BeatDetectionConfig,
) -> BeatAnalysisResult {
    let n = onset_env.len();
    let max_val = onset_env.iter().copied().fold(0.0f32, f32::max);
    if max_val < 1e-5 || hop_sec <= 1e-6 {
        return BeatAnalysisResult {
            bpm: 0.0,
            confidence: 0.0,
            beats: Vec::new(),
            onset_times_sec: Vec::new(),
        };
    }

    // 正規化
    let norm: Vec<f32> = onset_env.iter().map(|v| *v / max_val).collect();

    // 1. ローカル移動平均しきい値によるオンセットピーク抽出（最小間隔 160ms）
    let win_radius = ((0.25 / hop_sec).round() as usize).clamp(2, 40);
    let min_gap_frames = ((0.16 / hop_sec).round() as usize).max(1);
    let sens = cfg.sensitivity.clamp(0.8, 3.5);
    let mut onset_times_sec = Vec::new();
    let mut last_peak: Option<usize> = None;
    for i in 1..n.saturating_sub(1) {
        let v = norm[i];
        if v < 0.08 || v < norm[i - 1] || v < norm[i + 1] {
            continue;
        }
        let lo = i.saturating_sub(win_radius);
        let hi = (i + win_radius + 1).min(n);
        let local_mean = norm[lo..hi].iter().sum::<f32>() / (hi - lo).max(1) as f32;
        if v >= (local_mean * sens).max(0.10) && last_peak.is_none_or(|p| i - p >= min_gap_frames) {
            onset_times_sec.push(i as f64 * hop_sec);
            last_peak = Some(i);
        }
    }

    // 2. 自己相関による BPM（支配的ビート周期）推定
    let min_bpm = cfg.min_bpm.clamp(40.0, 220.0);
    let max_bpm = cfg.max_bpm.clamp(min_bpm + 5.0, 260.0);
    let min_lag = ((60.0 / f64::from(max_bpm)) / hop_sec).floor().max(2.0) as usize;
    let max_lag = ((60.0 / f64::from(min_bpm)) / hop_sec)
        .ceil()
        .min((n / 2).max(min_lag + 1) as f64) as usize;

    let mut best_lag = min_lag;
    let mut best_score = 0.0f64;
    let mut sum_scores = 0.0f64;
    let mut score_count = 0usize;

    for lag in min_lag..=max_lag {
        let mut acc = 0.0f64;
        let len = n.saturating_sub(lag);
        if len == 0 {
            continue;
        }
        for i in 0..len {
            acc += f64::from(norm[i]) * f64::from(norm[i + lag]);
        }
        // ハーモニック補強（2拍先も参照してオクターブ誤検出を抑制）
        let dbl = lag * 2;
        if dbl < n {
            let mut acc2 = 0.0f64;
            let len2 = n - dbl;
            for i in 0..len2 {
                acc2 += f64::from(norm[i]) * f64::from(norm[i + dbl]);
            }
            acc = acc / len as f64 + 0.5 * (acc2 / len2 as f64);
        } else {
            acc /= len as f64;
        }
        // 音楽的に一般的な 110..135 BPM 付近を軽く優先（タイブレーク）
        let cand_bpm = 60.0 / (lag as f64 * hop_sec);
        let prior = 1.0 + 0.04 * (-((cand_bpm - 120.0) / 55.0).powi(2)).exp();
        let weighted = acc * prior;

        sum_scores += weighted;
        score_count += 1;
        if weighted > best_score {
            best_score = weighted;
            best_lag = lag;
        }
    }

    if best_score <= 1e-6 {
        return BeatAnalysisResult {
            bpm: 0.0,
            confidence: 0.0,
            beats: Vec::new(),
            onset_times_sec,
        };
    }

    let mean_score = (sum_scores / score_count.max(1) as f64).max(1e-9);
    let confidence = (((best_score / mean_score) - 1.0) / 3.0).clamp(0.0, 1.0) as f32;
    let period_sec = best_lag as f64 * hop_sec;
    let bpm = (60.0 / period_sec) as f32;

    // 3. 位相アライメント: [0..best_lag) の中でオンセット強度総和が最大になる開始フレームを探索
    let mut best_phase = 0usize;
    let mut best_phase_score = -1.0f64;
    for phase in 0..best_lag {
        let mut s = 0.0f64;
        let mut idx = phase;
        while idx < n {
            // 近傍 ±1 フレームの最大値を拾う
            let local_max = norm[idx.saturating_sub(1)..=(idx + 1).min(n - 1)]
                .iter()
                .copied()
                .fold(0.0f32, f32::max);
            s += f64::from(local_max);
            idx += best_lag;
        }
        if s > best_phase_score {
            best_phase_score = s;
            best_phase = phase;
        }
    }

    // 4. 各ビートを ±35ms 内の最も強い局所オンセットへ微調整してスナップ精度を最大化
    let refine_radius = ((0.035 / hop_sec).round() as usize).clamp(1, 6);
    let bpb = cfg.beats_per_bar.max(1);
    let mut beats = Vec::new();
    let mut k = 0usize;
    loop {
        let nominal_sec = (best_phase as f64 * hop_sec) + (k as f64 * period_sec);
        if nominal_sec >= total_sec - 0.01 {
            break;
        }
        let nominal_idx = (nominal_sec / hop_sec).round() as usize;
        let lo = nominal_idx.saturating_sub(refine_radius).min(n - 1);
        let hi = (nominal_idx + refine_radius + 1).min(n);
        let mut best_i = nominal_idx.min(n - 1);
        let mut best_v = norm[best_i];
        for (i, &v) in norm[lo..hi].iter().enumerate() {
            if v > best_v {
                best_v = v;
                best_i = lo + i;
            }
        }
        let refined_sec = if best_v > 0.15 {
            best_i as f64 * hop_sec
        } else {
            nominal_sec
        };
        beats.push(BeatPoint {
            time_sec: refined_sec,
            strength: best_v,
            is_downbeat: k.is_multiple_of(bpb),
        });
        k += 1;
    }

    BeatAnalysisResult {
        bpm,
        confidence,
        beats,
        onset_times_sec,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_120_bpm_synthetic_kick_track_accurately() {
        let sr = 48_000u32;
        let dur_sec = 8.0f64;
        let total_frames = (dur_sec * f64::from(sr)) as usize;
        let mut pcm = vec![0.0f32; total_frames * 2];

        // 120 BPM = 0.5 秒ごとに短い 80Hz キックパルスを生成
        let beat_interval = 0.5f64;
        let pulse_len = (0.06 * f64::from(sr)) as usize;
        let mut t = 0.25f64;
        let mut beat_idx = 0usize;
        while t < dur_sec - 0.1 {
            let start_f = (t * f64::from(sr)) as usize;
            let amp = if beat_idx.is_multiple_of(4) {
                0.95
            } else {
                0.65
            };
            for i in 0..pulse_len {
                if start_f + i >= total_frames {
                    break;
                }
                let env = (-(i as f32) / (0.015 * sr as f32)).exp();
                let phase = 2.0 * std::f32::consts::PI * 80.0 * (i as f32 / sr as f32);
                let sample = amp * env * phase.sin();
                pcm[(start_f + i) * 2] = sample;
                pcm[(start_f + i) * 2 + 1] = sample;
            }
            t += beat_interval;
            beat_idx += 1;
        }

        let res = detect_audio_beats(&pcm, sr, &BeatDetectionConfig::default());
        assert!(
            (res.bpm - 120.0).abs() <= 2.5,
            "expected ~120 BPM, got {}",
            res.bpm
        );
        assert!(res.beats.len() >= 14, "beats={}", res.beats.len());
        assert!(res.beats[0].is_downbeat);
        // 最初のビートが 0.25s 付近にロックしていること
        assert!(
            (res.beats[0].time_sec - 0.25).abs() < 0.05,
            "first beat={}",
            res.beats[0].time_sec
        );
    }

    #[test]
    fn handles_silence_gracefully_without_panicking() {
        let pcm = vec![0.0f32; 48_000 * 4];
        let res = detect_audio_beats(&pcm, 48_000, &BeatDetectionConfig::default());
        assert_eq!(res.bpm, 0.0);
        assert!(res.beats.is_empty());
    }
}
