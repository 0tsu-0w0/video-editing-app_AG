use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use ve_core::{AudioDuckingConfig, AudioEffectNode};

/// リアルタイム音量メーター・ラウドネス測定結果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioMeterSnapshot {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
    pub clipped: bool,
    pub momentary_lufs: f32,
}

impl Default for AudioMeterSnapshot {
    fn default() -> Self {
        Self {
            peak_l: 0.0,
            peak_r: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
            clipped: false,
            momentary_lufs: -70.0,
        }
    }
}

/// 音声コールバックスレッドからロックフリーでメーター値を書き込めるアトミック構造体
#[derive(Debug)]
pub struct AtomicAudioMeter {
    peak_l: AtomicU32,
    peak_r: AtomicU32,
    rms_l: AtomicU32,
    rms_r: AtomicU32,
    lufs: AtomicU32,
    clipped: AtomicBool,
}

impl Default for AtomicAudioMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicAudioMeter {
    pub fn new() -> Self {
        Self {
            peak_l: AtomicU32::new(0.0f32.to_bits()),
            peak_r: AtomicU32::new(0.0f32.to_bits()),
            rms_l: AtomicU32::new(0.0f32.to_bits()),
            rms_r: AtomicU32::new(0.0f32.to_bits()),
            lufs: AtomicU32::new((-70.0f32).to_bits()),
            clipped: AtomicBool::new(false),
        }
    }

    pub fn store(&self, snap: AudioMeterSnapshot) {
        self.peak_l.store(snap.peak_l.to_bits(), Ordering::Relaxed);
        self.peak_r.store(snap.peak_r.to_bits(), Ordering::Relaxed);
        self.rms_l.store(snap.rms_l.to_bits(), Ordering::Relaxed);
        self.rms_r.store(snap.rms_r.to_bits(), Ordering::Relaxed);
        self.lufs
            .store(snap.momentary_lufs.to_bits(), Ordering::Relaxed);
        self.clipped.store(snap.clipped, Ordering::Relaxed);
    }

    pub fn load(&self) -> AudioMeterSnapshot {
        AudioMeterSnapshot {
            peak_l: f32::from_bits(self.peak_l.load(Ordering::Relaxed)),
            peak_r: f32::from_bits(self.peak_r.load(Ordering::Relaxed)),
            rms_l: f32::from_bits(self.rms_l.load(Ordering::Relaxed)),
            rms_r: f32::from_bits(self.rms_r.load(Ordering::Relaxed)),
            momentary_lufs: f32::from_bits(self.lufs.load(Ordering::Relaxed)),
            clipped: self.clipped.load(Ordering::Relaxed),
        }
    }
}

#[inline]
pub fn linear_to_db(v: f32) -> f32 {
    if v <= 1e-6 { -120.0 } else { 20.0 * v.log10() }
}

#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// ステレオブロックのリアルタイム Peak / RMS / 簡易 Momentary LUFS を計測する
pub fn measure_block_meter(stereo: &[f32]) -> AudioMeterSnapshot {
    if stereo.len() < 2 {
        return AudioMeterSnapshot::default();
    }
    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    let mut sum_sq_l = 0.0f64;
    let mut sum_sq_r = 0.0f64;
    let mut frames = 0usize;

    for &[l, r] in stereo.as_chunks::<2>().0 {
        let al = l.abs();
        let ar = r.abs();
        if al > peak_l {
            peak_l = al;
        }
        if ar > peak_r {
            peak_r = ar;
        }
        sum_sq_l += f64::from(l) * f64::from(l);
        sum_sq_r += f64::from(r) * f64::from(r);
        frames += 1;
    }

    if frames == 0 {
        return AudioMeterSnapshot::default();
    }
    let inv = 1.0 / frames as f64;
    let ms_l = sum_sq_l * inv;
    let ms_r = sum_sq_r * inv;
    let mean_sq = ms_l + ms_r;
    let lufs = if mean_sq <= 1e-10 {
        -70.0
    } else {
        (-0.691 + 10.0 * mean_sq.log10()).max(-70.0) as f32
    };

    AudioMeterSnapshot {
        peak_l,
        peak_r,
        rms_l: ms_l.sqrt() as f32,
        rms_r: ms_r.sqrt() as f32,
        clipped: peak_l >= 0.995 || peak_r >= 0.995,
        momentary_lufs: lufs,
    }
}

/// ITU-R BS.1770 K-weighting 近似フィルタによる Integrated LUFS を計測する
pub fn measure_integrated_lufs(stereo: &[f32], sample_rate: u32) -> f32 {
    if stereo.len() < 2 || sample_rate == 0 {
        return -70.0;
    }
    let sr = sample_rate as f32;
    // Stage 1: High-shelf (+4dB @ 1681Hz), Stage 2: RLB High-pass (38Hz)
    let mut shelf_l = Biquad::high_shelf(sr, 1681.0, 4.0);
    let mut shelf_r = Biquad::high_shelf(sr, 1681.0, 4.0);
    let mut rlb_l = Biquad::high_pass(sr, 38.0, 0.5);
    let mut rlb_r = Biquad::high_pass(sr, 38.0, 0.5);

    // 400ms ブロック単位で絶対ゲート (-70 LUFS) を適用
    let block_frames = ((sample_rate as usize) * 2 / 5).max(1);
    let mut block_powers: Vec<f64> = Vec::new();
    let mut acc_l = 0.0f64;
    let mut acc_r = 0.0f64;
    let mut count = 0usize;

    for &[l, r] in stereo.as_chunks::<2>().0 {
        let fl = rlb_l.process(shelf_l.process(l));
        let fr = rlb_r.process(shelf_r.process(r));
        acc_l += f64::from(fl) * f64::from(fl);
        acc_r += f64::from(fr) * f64::from(fr);
        count += 1;
        if count >= block_frames {
            let z = (acc_l + acc_r) / count as f64;
            if z > 1e-7 {
                block_powers.push(z);
            }
            acc_l = 0.0;
            acc_r = 0.0;
            count = 0;
        }
    }
    if count > 0 {
        let z = (acc_l + acc_r) / count as f64;
        if z > 1e-7 {
            block_powers.push(z);
        }
    }

    if block_powers.is_empty() {
        return -70.0;
    }
    let avg_all = block_powers.iter().sum::<f64>() / block_powers.len() as f64;
    // 相対ゲート（-10 LU 以下を除外）
    let rel_threshold = avg_all * 0.1;
    let gated: Vec<f64> = block_powers
        .into_iter()
        .filter(|&z| z >= rel_threshold)
        .collect();
    let final_z = if gated.is_empty() {
        avg_all
    } else {
        gated.iter().sum::<f64>() / gated.len() as f64
    };
    if final_z <= 1e-10 {
        -70.0
    } else {
        (-0.691 + 10.0 * final_z.log10()).max(-70.0) as f32
    }
}

/// 現在 LUFS から目標 LUFS（例: YouTube 基準 -14.0 LUFS）へ正規化するためのリニアゲイン倍率を算出する
pub fn compute_lufs_gain(current_lufs: f32, target_lufs: f32) -> f32 {
    if !current_lufs.is_finite() || !target_lufs.is_finite() || current_lufs <= -65.0 {
        return 1.0;
    }
    let diff_db = (target_lufs - current_lufs).clamp(-24.0, 18.0);
    db_to_linear(diff_db)
}

/// ステレオ／モノラル PCM バッファへ音声 DSP エフェクトチェーンを一括適用する
pub fn apply_audio_effect_chain(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    chain: &[AudioEffectNode],
) {
    if samples.is_empty() || channels == 0 || sample_rate == 0 {
        return;
    }
    for node in chain {
        if !node.enabled {
            continue;
        }
        apply_audio_effect(samples, channels, sample_rate, node);
    }
}

fn apply_audio_effect(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    node: &AudioEffectNode,
) {
    let sr = sample_rate as f32;
    match node.effect_type.as_str() {
        "low_cut" => {
            let cutoff = node
                .params
                .get("cutoff_hz")
                .copied()
                .unwrap_or(100.0)
                .clamp(20.0, sr * 0.45);
            let mut filters: Vec<Biquad> = (0..channels)
                .map(|_| Biquad::high_pass(sr, cutoff, 0.707))
                .collect();
            for frame in samples.chunks_exact_mut(channels) {
                for (ch, s) in frame.iter_mut().enumerate() {
                    *s = filters[ch].process(*s);
                }
            }
        }
        "high_cut" => {
            let cutoff = node
                .params
                .get("cutoff_hz")
                .copied()
                .unwrap_or(8000.0)
                .clamp(200.0, sr * 0.45);
            let mut filters: Vec<Biquad> = (0..channels)
                .map(|_| Biquad::low_pass(sr, cutoff, 0.707))
                .collect();
            for frame in samples.chunks_exact_mut(channels) {
                for (ch, s) in frame.iter_mut().enumerate() {
                    *s = filters[ch].process(*s);
                }
            }
        }
        "eq_3band" => {
            let low_db = node
                .params
                .get("low_db")
                .copied()
                .unwrap_or(0.0)
                .clamp(-18.0, 18.0);
            let mid_db = node
                .params
                .get("mid_db")
                .copied()
                .unwrap_or(0.0)
                .clamp(-18.0, 18.0);
            let high_db = node
                .params
                .get("high_db")
                .copied()
                .unwrap_or(0.0)
                .clamp(-18.0, 18.0);
            let mut lows: Vec<Biquad> = (0..channels)
                .map(|_| Biquad::low_shelf(sr, 200.0, low_db))
                .collect();
            let mut mids: Vec<Biquad> = (0..channels)
                .map(|_| Biquad::peaking_eq(sr, 1200.0, 1.0, mid_db))
                .collect();
            let mut highs: Vec<Biquad> = (0..channels)
                .map(|_| Biquad::high_shelf(sr, 5000.0, high_db))
                .collect();
            for frame in samples.chunks_exact_mut(channels) {
                for (ch, s) in frame.iter_mut().enumerate() {
                    let v = lows[ch].process(*s);
                    let v = mids[ch].process(v);
                    *s = highs[ch].process(v);
                }
            }
        }
        "compressor" => {
            let threshold_db = node
                .params
                .get("threshold_db")
                .copied()
                .unwrap_or(-18.0)
                .clamp(-60.0, 0.0);
            let ratio = node
                .params
                .get("ratio")
                .copied()
                .unwrap_or(4.0)
                .clamp(1.0, 20.0);
            let makeup = db_to_linear(
                node.params
                    .get("makeup_db")
                    .copied()
                    .unwrap_or(3.0)
                    .clamp(-12.0, 24.0),
            );
            let attack_coeff = (-1.0 / (0.010 * sr)).exp();
            let release_coeff = (-1.0 / (0.100 * sr)).exp();
            let mut env = 0.0f32;
            for frame in samples.chunks_exact_mut(channels) {
                let peak = frame.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
                let coeff = if peak > env {
                    attack_coeff
                } else {
                    release_coeff
                };
                env = coeff * env + (1.0 - coeff) * peak;
                let env_db = linear_to_db(env);
                let gain_db = if env_db > threshold_db {
                    (threshold_db + (env_db - threshold_db) / ratio) - env_db
                } else {
                    0.0
                };
                let gain = db_to_linear(gain_db) * makeup;
                for s in frame.iter_mut() {
                    *s *= gain;
                }
            }
        }
        "limiter" => {
            let ceiling = db_to_linear(
                node.params
                    .get("ceiling_db")
                    .copied()
                    .unwrap_or(-1.0)
                    .clamp(-18.0, 0.0),
            );
            let release_coeff = (-1.0 / (0.050 * sr)).exp();
            let mut gain = 1.0f32;
            for frame in samples.chunks_exact_mut(channels) {
                let peak = frame.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
                let target_gain = if peak > ceiling && peak > 1e-6 {
                    ceiling / peak
                } else {
                    1.0
                };
                if target_gain < gain {
                    gain = target_gain;
                } else {
                    gain = release_coeff * gain + (1.0 - release_coeff) * target_gain;
                }
                for s in frame.iter_mut() {
                    *s = (*s * gain).clamp(-ceiling, ceiling);
                }
            }
        }
        "delay" => {
            let delay_ms = node
                .params
                .get("delay_ms")
                .copied()
                .unwrap_or(240.0)
                .clamp(10.0, 2000.0);
            let feedback = node
                .params
                .get("feedback")
                .copied()
                .unwrap_or(0.35)
                .clamp(0.0, 0.88);
            let mix = node
                .params
                .get("mix")
                .copied()
                .unwrap_or(0.3)
                .clamp(0.0, 1.0);
            let delay_frames = ((delay_ms / 1000.0) * sr).round().max(1.0) as usize;
            let mut ring = vec![0.0f32; delay_frames * channels];
            let mut write_idx = 0usize;
            for frame in samples.chunks_exact_mut(channels) {
                let base = write_idx * channels;
                for (ch, s) in frame.iter_mut().enumerate() {
                    let delayed = ring[base + ch];
                    let dry = *s;
                    ring[base + ch] = dry + delayed * feedback;
                    *s = dry * (1.0 - mix) + delayed * mix;
                }
                write_idx = (write_idx + 1) % delay_frames;
            }
        }
        "reverb" => {
            let room = node
                .params
                .get("room_size")
                .copied()
                .unwrap_or(0.55)
                .clamp(0.1, 0.92);
            let damp = node
                .params
                .get("damping")
                .copied()
                .unwrap_or(0.4)
                .clamp(0.0, 0.9);
            let mix = node
                .params
                .get("mix")
                .copied()
                .unwrap_or(0.25)
                .clamp(0.0, 1.0);
            // Schroeder 4並列コムフィルタ（44.1kHz基準長を現在サンプルレートへスケール）
            let scale = sr / 44_100.0;
            let comb_lens = [
                (1116.0 * scale) as usize,
                (1188.0 * scale) as usize,
                (1277.0 * scale) as usize,
                (1356.0 * scale) as usize,
            ];
            let mut combs: Vec<Vec<CombFilter>> = (0..channels)
                .map(|ch| {
                    comb_lens
                        .iter()
                        .map(|&len| CombFilter::new((len + ch * 23).max(8)))
                        .collect()
                })
                .collect();
            let feedback = 0.55 + room * 0.35;
            for frame in samples.chunks_exact_mut(channels) {
                for (ch, s) in frame.iter_mut().enumerate() {
                    let dry = *s;
                    let mut wet = 0.0f32;
                    for comb in &mut combs[ch] {
                        wet += comb.process(dry, feedback, damp);
                    }
                    wet *= 0.25;
                    *s = dry * (1.0 - mix) + wet * mix;
                }
            }
        }
        "radio_voice" => {
            let drive = node
                .params
                .get("drive")
                .copied()
                .unwrap_or(2.5)
                .clamp(1.0, 10.0);
            let mix = node
                .params
                .get("mix")
                .copied()
                .unwrap_or(0.85)
                .clamp(0.0, 1.0);
            let mut hpf: Vec<Biquad> = (0..channels)
                .map(|_| Biquad::high_pass(sr, 380.0, 0.8))
                .collect();
            let mut lpf: Vec<Biquad> = (0..channels)
                .map(|_| Biquad::low_pass(sr, 2800.0, 0.8))
                .collect();
            for frame in samples.chunks_exact_mut(channels) {
                for (ch, s) in frame.iter_mut().enumerate() {
                    let dry = *s;
                    let bp = lpf[ch].process(hpf[ch].process(dry));
                    let sat = (bp * drive).tanh();
                    *s = dry * (1.0 - mix) + sat * mix;
                }
            }
        }
        _ => {}
    }
}

struct CombFilter {
    buf: Vec<f32>,
    idx: usize,
    lp_state: f32,
}

impl CombFilter {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            lp_state: 0.0,
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.lp_state = out * (1.0 - damp) + self.lp_state * damp;
        self.buf[self.idx] = input + self.lp_state * feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

/// 2次 IIR バイクワッドフィルタ（Transposed Direct Form II）
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        let inv = 1.0 / a0.max(1e-8);
        Self {
            b0: b0 * inv,
            b1: b1 * inv,
            b2: b2 * inv,
            a1: a1 * inv,
            a2: a2 * inv,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn high_pass(sr: f32, freq: f32, q: f32) -> Self {
        let w0 = 2.0 * PI * (freq.clamp(10.0, sr * 0.48) / sr);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));
        Self::from_coeffs(
            (1.0 + cos_w0) * 0.5,
            -(1.0 + cos_w0),
            (1.0 + cos_w0) * 0.5,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    fn low_pass(sr: f32, freq: f32, q: f32) -> Self {
        let w0 = 2.0 * PI * (freq.clamp(10.0, sr * 0.48) / sr);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));
        Self::from_coeffs(
            (1.0 - cos_w0) * 0.5,
            1.0 - cos_w0,
            (1.0 - cos_w0) * 0.5,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    fn peaking_eq(sr: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * (freq.clamp(10.0, sr * 0.48) / sr);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));
        Self::from_coeffs(
            1.0 + alpha * a,
            -2.0 * cos_w0,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos_w0,
            1.0 - alpha / a,
        )
    }

    fn low_shelf(sr: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * (freq.clamp(10.0, sr * 0.48) / sr);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let sqrt_a = a.sqrt();
        let alpha = sin_w0 * 0.5 * 2.0_f32.sqrt();
        Self::from_coeffs(
            a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
            a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
            (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
            (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
        )
    }

    fn high_shelf(sr: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * (freq.clamp(10.0, sr * 0.48) / sr);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let sqrt_a = a.sqrt();
        let alpha = sin_w0 * 0.5 * 2.0_f32.sqrt();
        Self::from_coeffs(
            a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
            a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
            (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
            (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
        )
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// v0.6 #4: 周波数帯域スペクトラム解析結果（Goertzel / 対数バンド分割）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpectrumSnapshot {
    pub bands: Vec<f32>,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub rms: f32,
}

/// Pure-Rust Hann窓付き Goertzel アルゴリズムでモノラル PCM から対数間隔の周波数帯域エネルギーを解析する
pub fn analyze_spectrum_bands(
    mono: &[f32],
    sample_rate: u32,
    band_count: usize,
) -> SpectrumSnapshot {
    let count = band_count.clamp(1, 64);
    if mono.is_empty() || sample_rate == 0 {
        return SpectrumSnapshot {
            bands: vec![0.0; count],
            ..Default::default()
        };
    }

    // 最大 1024 サンプルの末尾ウィンドウを使用（高速かつ低遅延）
    let win_len = mono.len().min(1024);
    let slice = &mono[mono.len() - win_len..];
    let n_f = win_len as f32;

    let mut sum_sq = 0.0f64;
    let mut windowed = Vec::with_capacity(win_len);
    for (i, &s) in slice.iter().enumerate() {
        sum_sq += f64::from(s) * f64::from(s);
        let hann = if win_len > 1 {
            0.5 * (1.0 - (2.0 * PI * i as f32 / (win_len - 1) as f32).cos())
        } else {
            1.0
        };
        windowed.push(s * hann);
    }
    let rms = (sum_sq / win_len as f64).sqrt() as f32;

    let min_hz = 60.0f32;
    let max_hz = (sample_rate as f32 * 0.45).clamp(200.0, 12_000.0);
    let ratio = (max_hz / min_hz).max(1.01);
    let sr = sample_rate as f32;

    let mut bands = Vec::with_capacity(count);
    for b in 0..count {
        let t = if count > 1 {
            b as f32 / (count - 1) as f32
        } else {
            0.5
        };
        let target_hz = min_hz * ratio.powf(t);
        let omega = 2.0 * PI * target_hz / sr;
        let coeff = 2.0 * omega.cos();
        let mut s1 = 0.0f32;
        let mut s2 = 0.0f32;
        for &x in &windowed {
            let s0 = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
        // Hann 窓のコヒーレントゲイン (0.5) を補正して [0, 1] 正規化
        let mag = (power.sqrt() * 4.0 / n_f).clamp(0.0, 1.0);
        bands.push(mag);
    }

    let low_end = (count / 3).max(1);
    let mid_end = ((count * 2) / 3).max(low_end).min(count);
    let avg = |sl: &[f32]| -> f32 {
        if sl.is_empty() {
            0.0
        } else {
            sl.iter().sum::<f32>() / sl.len() as f32
        }
    };
    let low = avg(&bands[..low_end]);
    let mid = avg(&bands[low_end..mid_end]);
    let high = avg(&bands[mid_end..]);

    SpectrumSnapshot {
        bands,
        low,
        mid,
        high,
        rms,
    }
}

/// v0.6 #4: 発話（サイドチェイン）信号のエンベロープに応じて BGM ステレオバッファを滑らかに自動減衰する
pub fn apply_sidechain_ducking(
    voice_stereo: &[f32],
    bgm_stereo: &mut [f32],
    sample_rate: u32,
    cfg: &AudioDuckingConfig,
) {
    if !cfg.enabled || sample_rate == 0 || voice_stereo.len() < 2 || bgm_stereo.len() < 2 {
        return;
    }
    let sr = sample_rate as f32;
    let threshold = cfg.threshold_rms.clamp(0.001, 0.9);
    let duck_target = db_to_linear(cfg.reduction_db.clamp(-48.0, 0.0));
    let attack_coeff = (-1.0 / (cfg.attack_sec.clamp(0.002, 2.0) * sr)).exp();
    let release_coeff = (-1.0 / (cfg.release_sec.clamp(0.01, 5.0) * sr)).exp();

    // 発話のゼロ交差でチャタリングしないよう、高速 RMS 検出エンベロープ（10ms / 45ms）を使用
    let det_attack = (-1.0 / (0.010 * sr)).exp();
    let det_release = (-1.0 / (0.045 * sr)).exp();
    let mut det_env = 0.0f32;
    let mut gain = 1.0f32;

    let frames = voice_stereo.len().min(bgm_stereo.len()) / 2;
    for i in 0..frames {
        let base = i * 2;
        let vl = voice_stereo[base];
        let vr = voice_stereo[base + 1];
        let v_rms = ((vl * vl + vr * vr) * 0.5).sqrt();
        let dc = if v_rms > det_env {
            det_attack
        } else {
            det_release
        };
        det_env = dc * det_env + (1.0 - dc) * v_rms;

        let target_gain = if det_env >= threshold {
            duck_target
        } else {
            1.0
        };
        let gc = if target_gain < gain {
            attack_coeff
        } else {
            release_coeff
        };
        gain = gc * gain + (1.0 - gc) * target_gain;

        bgm_stereo[base] *= gain;
        bgm_stereo[base + 1] *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_stereo(freq: f32, amp: f32, sr: u32, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = (2.0 * PI * freq * i as f32 / sr as f32).sin() * amp;
            out.push(s);
            out.push(s);
        }
        out
    }

    #[test]
    fn low_cut_attenuates_dc_and_low_rumble() {
        let sr = 48_000;
        let mut low = sine_stereo(40.0, 0.8, sr, sr as usize / 2);
        let mut high = sine_stereo(2000.0, 0.8, sr, sr as usize / 2);
        let mut eff = AudioEffectNode::with_defaults("low_cut");
        eff.params.insert("cutoff_hz".into(), 300.0);

        apply_audio_effect_chain(&mut low, 2, sr, std::slice::from_ref(&eff));
        apply_audio_effect_chain(&mut high, 2, sr, &[eff]);

        let low_rms = measure_block_meter(&low[sr as usize / 4..]).rms_l;
        let high_rms = measure_block_meter(&high[sr as usize / 4..]).rms_l;
        assert!(low_rms < high_rms * 0.25, "low={low_rms}, high={high_rms}");
    }

    #[test]
    fn limiter_clamps_peaks_to_ceiling() {
        let sr = 48_000;
        let mut buf = sine_stereo(440.0, 1.5, sr, 4800);
        let mut lim = AudioEffectNode::with_defaults("limiter");
        lim.params.insert("ceiling_db".into(), -6.0);
        apply_audio_effect_chain(&mut buf, 2, sr, &[lim]);
        let ceiling = db_to_linear(-6.0) + 1e-3;
        assert!(buf.iter().all(|s| s.abs() <= ceiling));
    }

    #[test]
    fn integrated_lufs_and_normalization_hit_target() {
        let sr = 48_000;
        let mut buf = sine_stereo(1000.0, 0.1, sr, sr as usize);
        let before = measure_integrated_lufs(&buf, sr);
        assert!(before < -15.0 && before > -35.0, "before={before}");
        let gain = compute_lufs_gain(before, -14.0);
        for s in &mut buf {
            *s *= gain;
        }
        let after = measure_integrated_lufs(&buf, sr);
        assert!((after - (-14.0)).abs() < 0.8, "after={after}");
    }

    #[test]
    fn spectrum_analyzer_separates_low_and_high_sine_waves() {
        let sr = 48_000;
        let low_mono: Vec<f32> = (0..1024)
            .map(|i| (2.0 * PI * 120.0 * i as f32 / sr as f32).sin() * 0.8)
            .collect();
        let high_mono: Vec<f32> = (0..1024)
            .map(|i| (2.0 * PI * 6000.0 * i as f32 / sr as f32).sin() * 0.8)
            .collect();

        let low_spec = analyze_spectrum_bands(&low_mono, sr, 18);
        let high_spec = analyze_spectrum_bands(&high_mono, sr, 18);

        assert_eq!(low_spec.bands.len(), 18);
        assert!(low_spec.low > low_spec.high * 3.0, "low_spec={low_spec:?}");
        assert!(
            high_spec.high > high_spec.low * 3.0,
            "high_spec={high_spec:?}"
        );
    }

    #[test]
    fn sidechain_ducking_attenuates_bgm_only_during_voice() {
        let sr = 48_000;
        // 1.5s: 0.0..0.5s 無音 -> 0.5..1.0s 発話 -> 1.0..1.5s 無音
        let total_frames = sr as usize * 3 / 2;
        let mut voice = vec![0.0f32; total_frames * 2];
        for i in (sr as usize / 2)..(sr as usize) {
            let s = (2.0 * PI * 300.0 * i as f32 / sr as f32).sin() * 0.6;
            voice[i * 2] = s;
            voice[i * 2 + 1] = s;
        }
        let mut bgm = sine_stereo(440.0, 0.5, sr, total_frames);
        let cfg = AudioDuckingConfig {
            enabled: true,
            voice_layer_index: 3,
            bgm_layer_index: 2,
            threshold_rms: 0.05,
            reduction_db: -12.0,
            attack_sec: 0.015,
            release_sec: 0.05,
        };
        apply_sidechain_ducking(&voice, &mut bgm, sr, &cfg);

        let before_rms = measure_block_meter(&bgm[0..(sr as usize / 2) * 2]).rms_l;
        let during_rms =
            measure_block_meter(&bgm[(sr as usize * 3 / 4) * 2..(sr as usize) * 2]).rms_l;
        let after_rms =
            measure_block_meter(&bgm[(sr as usize * 5 / 4) * 2..total_frames * 2]).rms_l;

        assert!(
            during_rms < before_rms * 0.45,
            "before={before_rms}, during={during_rms}, after={after_rms}"
        );
        assert!(
            after_rms > during_rms * 2.0,
            "BGM should recover after voice stops: during={during_rms}, after={after_rms}"
        );
    }
}
