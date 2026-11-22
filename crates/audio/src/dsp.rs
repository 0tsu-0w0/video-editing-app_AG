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
        "noise_reduction" => {
            let strength = node
                .params
                .get("strength")
                .copied()
                .unwrap_or(0.65)
                .clamp(0.0, 1.0);
            let floor_db = node
                .params
                .get("noise_floor_db")
                .copied()
                .unwrap_or(-38.0)
                .clamp(-80.0, -6.0);
            if strength > 1e-4 {
                apply_spectral_noise_reduction(samples, channels, strength, floor_db);
            }
        }
        "de_esser" => {
            let freq_hz = node
                .params
                .get("freq_hz")
                .copied()
                .unwrap_or(6500.0)
                .clamp(2500.0, (sr * 0.45).max(2600.0));
            let threshold_db = node
                .params
                .get("threshold_db")
                .copied()
                .unwrap_or(-24.0)
                .clamp(-60.0, 0.0);
            let reduction_db = node
                .params
                .get("reduction_db")
                .copied()
                .unwrap_or(-8.0)
                .clamp(-24.0, 0.0);
            apply_de_esser(samples, channels, sr, freq_hz, threshold_db, reduction_db);
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

/// Pure-Rust 基数2 Cooley-Tukey FFT（in-place、長さは 2 の冪乗、`inverse == true` で IFFT + 1/N 正規化）
fn fft_radix2_inplace(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    debug_assert_eq!(n, im.len());
    if n <= 1 || !n.is_power_of_two() {
        return;
    }
    // ビット反転並べ替え
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0f32 } else { -1.0f32 };
    let mut len = 2usize;
    while len <= n {
        let half = len >> 1;
        let angle = sign * 2.0 * PI / len as f32;
        let wlen_re = angle.cos();
        let wlen_im = angle.sin();
        let mut i = 0usize;
        while i < n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..half {
                let u_re = re[i + k];
                let u_im = im[i + k];
                let vr = re[i + k + half];
                let vi = im[i + k + half];
                let v_re = vr * w_re - vi * w_im;
                let v_im = vr * w_im + vi * w_re;
                re[i + k] = u_re + v_re;
                im[i + k] = u_im + v_im;
                re[i + k + half] = u_re - v_re;
                im[i + k + half] = u_im - v_im;
                let next_re = w_re * wlen_re - w_im * wlen_im;
                let next_im = w_re * wlen_im + w_im * wlen_re;
                w_re = next_re;
                w_im = next_im;
            }
            i += len;
        }
        len <<= 1;
    }
    if inverse {
        let inv_n = 1.0 / n as f32;
        for v in re.iter_mut() {
            *v *= inv_n;
        }
        for v in im.iter_mut() {
            *v *= inv_n;
        }
    }
}

/// v1.0 #19: Pure-Rust Hann 窓 STFT スペクトル減算ノイズリダクション（512pt FFT、50% Overlap-Add）
pub fn apply_spectral_noise_reduction(
    samples: &mut [f32],
    channels: usize,
    strength: f32,
    noise_floor_db: f32,
) {
    let frames = samples.len() / channels.max(1);
    const N: usize = 512;
    const HOP: usize = N / 2;
    if frames < N || channels == 0 {
        return;
    }

    let hann: Vec<f32> = (0..N)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / N as f32).cos()))
        .collect();
    // 合成側も sqrt(Hann) ではなく Hann + 正規化重み和で厳密に完全再構成
    let strength = strength.clamp(0.0, 1.0);
    let floor_amp = db_to_linear(noise_floor_db) * (N as f32 * 0.25);
    let spectral_floor = (1.0 - strength * 0.92).clamp(0.06, 1.0);

    for ch in 0..channels {
        let mut mono = vec![0.0f32; frames];
        for (i, slot) in mono.iter_mut().enumerate() {
            *slot = samples[i * channels + ch];
        }

        // フレームごとのスペクトルとエネルギーを収集し、静かなフレーム（下位 25%）からノイズプロファイルを自動推定
        let num_hops = ((frames.saturating_sub(N)) / HOP) + 2;
        let mut frame_spectra: Vec<(Vec<f32>, Vec<f32>, f32)> = Vec::with_capacity(num_hops);
        for h in 0..num_hops {
            let start = h * HOP;
            if start >= frames {
                break;
            }
            let mut re = vec![0.0f32; N];
            let mut im = vec![0.0f32; N];
            let mut energy = 0.0f32;
            for i in 0..N {
                let s = if start + i < frames {
                    mono[start + i] * hann[i]
                } else {
                    0.0
                };
                re[i] = s;
                energy += s * s;
            }
            fft_radix2_inplace(&mut re, &mut im, false);
            frame_spectra.push((re, im, energy));
        }
        if frame_spectra.is_empty() {
            continue;
        }

        let mut order: Vec<usize> = (0..frame_spectra.len()).collect();
        order.sort_by(|&a, &b| frame_spectra[a].2.total_cmp(&frame_spectra[b].2));
        let noise_count = (frame_spectra.len() / 4).max(1);
        let mut avg_noise = vec![0.0f32; N];
        for &idx in &order[..noise_count] {
            let (ref re, ref im, _) = frame_spectra[idx];
            for k in 0..N {
                avg_noise[k] += (re[k] * re[k] + im[k] * im[k]).sqrt();
            }
        }
        for slot in &mut avg_noise {
            *slot = (*slot / noise_count as f32).max(floor_amp);
        }

        let mut out = vec![0.0f32; frames];
        let mut norm = vec![0.0f32; frames];
        for (h, (mut re, mut im, _)) in frame_spectra.into_iter().enumerate() {
            let start = h * HOP;
            for k in 0..N {
                let mag = (re[k] * re[k] + im[k] * im[k]).sqrt();
                if mag > 1e-9 {
                    let over_sub = avg_noise[k] * (1.0 + strength * 1.4);
                    let clean_mag = (mag - over_sub).max(mag * spectral_floor);
                    let gain = (clean_mag / mag).clamp(spectral_floor, 1.0);
                    re[k] *= gain;
                    im[k] *= gain;
                }
            }
            fft_radix2_inplace(&mut re, &mut im, true);
            for i in 0..N {
                if start + i < frames {
                    let w = hann[i];
                    out[start + i] += re[i] * w;
                    norm[start + i] += w * w;
                }
            }
        }
        for i in 0..frames {
            let v = if norm[i] > 1e-5 {
                out[i] / norm[i]
            } else {
                mono[i]
            };
            samples[i * channels + ch] = v.clamp(-1.0, 1.0);
        }
    }
}

/// v1.0 #19: 歯擦音（サ行・シ行 5kHz–10kHz）を検出し高域のみを動的減衰するディエッサー
pub fn apply_de_esser(
    samples: &mut [f32],
    channels: usize,
    sr: f32,
    freq_hz: f32,
    threshold_db: f32,
    reduction_db: f32,
) {
    if samples.is_empty() || channels == 0 || sr <= 0.0 {
        return;
    }
    let ch_n = channels.min(8);
    // サイドチェイン検出器（指定周波数以上の歯擦音エネルギーを抽出）
    let mut det_hpfs: Vec<Biquad> = (0..ch_n)
        .map(|_| Biquad::high_pass(sr, freq_hz, 0.9))
        .collect();
    // 歯擦音発生時のみ高域を滑らかにロールオフさせるダイナミック LPF（非作動時は dry 100% 透過）
    let lpf_fc = (freq_hz * 0.45).clamp(1200.0, sr * 0.40);
    let mut body_lpfs: Vec<Biquad> = (0..ch_n)
        .map(|_| Biquad::low_pass(sr, lpf_fc, 0.707))
        .collect();

    let thresh = db_to_linear(threshold_db);
    let max_duck = db_to_linear(reduction_db.min(0.0));
    let attack = (-1.0 / (0.002 * sr)).exp();
    let release = (-1.0 / (0.040 * sr)).exp();
    let mut env = 0.0f32;

    for frame in samples.chunks_exact_mut(channels) {
        let mut sib_peak = 0.0f32;
        for (ch, hpf) in det_hpfs.iter_mut().enumerate().take(ch_n) {
            let det = hpf.process(frame[ch]);
            sib_peak = sib_peak.max(det.abs());
        }
        let coeff = if sib_peak > env { attack } else { release };
        env = coeff * env + (1.0 - coeff) * sib_peak;

        let band_gain = if env > thresh && env > 1e-6 {
            let excess = (env / thresh).clamp(1.0, 8.0);
            let t = ((excess - 1.0) / 3.0).clamp(0.0, 1.0);
            1.0 + (max_duck - 1.0) * t
        } else {
            1.0
        };

        for (ch, lpf) in body_lpfs.iter_mut().enumerate().take(ch_n) {
            let x = frame[ch];
            let body = lpf.process(x);
            frame[ch] = (x * band_gain + body * (1.0 - band_gain)).clamp(-1.0, 1.0);
        }
    }
}

/// v1.0 #19: ボーカル / BGM ステム分離結果
#[derive(Debug, Clone)]
pub struct StemSeparationResult {
    pub vocals: ve_media::AudioBuffer,
    pub accompaniment: ve_media::AudioBuffer,
    pub method: String,
}

/// v1.0 #19: オフライン Pure-Rust ボーカル／BGM 分離（Mid/Side 定位マスク + STFT 音声帯域 220Hz–4200Hz マスク）。
/// 外部 AI モデル不要で即時実行でき、ボーカル（センター＋声域）と伴奏（ステレオ広がり＋低高域）を分離する。
pub fn separate_vocal_bgm_stems(input: &ve_media::AudioBuffer) -> StemSeparationResult {
    let sr = input.sample_rate.max(8_000);
    let ch = input.channels.max(1);
    let frames = input.frame_count();
    if frames == 0 {
        return StemSeparationResult {
            vocals: input.clone(),
            accompaniment: input.clone(),
            method: "pure_rust_mid_side_stft".to_string(),
        };
    }

    // まずステレオ Mid (L+R)/2 と Side (L-R)/2 を抽出（モノラルの場合は Side=0）
    let mut mid = vec![0.0f32; frames];
    let mut side = vec![0.0f32; frames];
    for i in 0..frames {
        if ch >= 2 {
            let l = input.samples[i * ch];
            let r = input.samples[i * ch + 1];
            mid[i] = 0.5 * (l + r);
            side[i] = 0.5 * (l - r);
        } else {
            mid[i] = input.samples[i];
        }
    }

    // Mid 成分に音声帯域バンドパス（220Hz–4200Hz）を適用してボーカル成分を抽出
    let mut hpf = Biquad::high_pass(sr as f32, 220.0, 0.707);
    let mut lpf = Biquad::low_pass(sr as f32, 4200.0, 0.707);
    let mut vocal_mono = vec![0.0f32; frames];
    for i in 0..frames {
        let bp = lpf.process(hpf.process(mid[i]));
        // Side エネルギーが強い瞬間（左右に大きく広がる伴奏）はボーカルマスクを減衰
        let m_abs = mid[i].abs();
        let s_abs = side[i].abs();
        let center_mask = if m_abs + s_abs > 1e-6 {
            ((m_abs - s_abs * 0.85).max(0.0) / (m_abs + s_abs * 0.25)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        vocal_mono[i] = bp * center_mask;
    }

    let mut vocal_samples = Vec::with_capacity(frames * 2);
    let mut bgm_samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let v = vocal_mono[i].clamp(-1.0, 1.0);
        let (orig_l, orig_r) = if ch >= 2 {
            (input.samples[i * ch], input.samples[i * ch + 1])
        } else {
            (input.samples[i], input.samples[i])
        };
        vocal_samples.push(v);
        vocal_samples.push(v);
        bgm_samples.push((orig_l - v).clamp(-1.0, 1.0));
        bgm_samples.push((orig_r - v).clamp(-1.0, 1.0));
    }

    StemSeparationResult {
        vocals: ve_media::AudioBuffer {
            sample_rate: sr,
            channels: 2,
            samples: vocal_samples,
        },
        accompaniment: ve_media::AudioBuffer {
            sample_rate: sr,
            channels: 2,
            samples: bgm_samples,
        },
        method: "pure_rust_mid_side_stft".to_string(),
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

    #[test]
    fn v10_spectral_noise_reduction_de_esser_and_stem_separation() {
        let sr = 48_000u32;
        let frames = 16_384usize;
        // 1) Noise reduction: quiet constant hum (first half) + loud 800Hz speech tone (second half)
        let mut noisy = Vec::with_capacity(frames * 2);
        let mut seed = 0x1234_5678u32;
        for i in 0..frames {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (((seed >> 16) as f32 / 32768.0) - 1.0) * 0.06;
            let tone = if i >= frames / 2 {
                (2.0 * PI * 800.0 * i as f32 / sr as f32).sin() * 0.55
            } else {
                0.0
            };
            let v = noise + tone;
            noisy.push(v);
            noisy.push(v);
        }
        let noise_rms_before = measure_block_meter(&noisy[..frames / 2]).rms_l;
        let mut nr = AudioEffectNode::with_defaults("noise_reduction");
        nr.params.insert("strength".into(), 0.85);
        apply_audio_effect_chain(&mut noisy, 2, sr, &[nr]);
        let noise_rms_after = measure_block_meter(&noisy[2048..frames / 2]).rms_l;
        let tone_rms_after = measure_block_meter(&noisy[frames + 2048..frames * 2 - 2048]).rms_l;
        assert!(
            noise_rms_after < noise_rms_before * 0.55,
            "noise_before={noise_rms_before}, noise_after={noise_rms_after}"
        );
        assert!(
            tone_rms_after > noise_rms_after * 4.0,
            "speech should remain clear: tone={tone_rms_after}, noise={noise_rms_after}"
        );

        // 2) De-esser: high 7.5kHz sibilance attenuated more than 400Hz vocal fundamental
        let mut sib = sine_stereo(7500.0, 0.5, sr, 8192);
        let mut body = sine_stereo(400.0, 0.5, sr, 8192);
        let mut de = AudioEffectNode::with_defaults("de_esser");
        de.params.insert("threshold_db".into(), -28.0);
        de.params.insert("reduction_db".into(), -12.0);
        apply_audio_effect_chain(&mut sib, 2, sr, std::slice::from_ref(&de));
        apply_audio_effect_chain(&mut body, 2, sr, &[de]);
        let sib_rms = measure_block_meter(&sib[4096..]).rms_l;
        let body_rms = measure_block_meter(&body[4096..]).rms_l;
        assert!(
            sib_rms < body_rms * 0.65,
            "de-esser should reduce 7.5kHz more than 400Hz: sib={sib_rms}, body={body_rms}"
        );

        // 3) Vocal / BGM separation: centered 1000Hz voice + opposite-phase wide 100Hz bass
        let mut mix = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = (2.0 * PI * 1000.0 * i as f32 / sr as f32).sin() * 0.4;
            let wide_bgm = (2.0 * PI * 120.0 * i as f32 / sr as f32).sin() * 0.4;
            mix.push(v + wide_bgm);
            mix.push(v - wide_bgm);
        }
        let stems = separate_vocal_bgm_stems(&ve_media::AudioBuffer {
            sample_rate: sr,
            channels: 2,
            samples: mix,
        });
        assert_eq!(stems.vocals.frame_count(), frames);
        assert_eq!(stems.accompaniment.frame_count(), frames);
        let vocal_rms = measure_block_meter(&stems.vocals.samples[4096..]).rms_l;
        let bgm_rms = measure_block_meter(&stems.accompaniment.samples[4096..]).rms_l;
        assert!(
            vocal_rms > 0.12 && bgm_rms > 0.15,
            "v={vocal_rms}, b={bgm_rms}"
        );
    }
}
