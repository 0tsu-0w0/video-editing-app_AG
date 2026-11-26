//! v8.0 #52: オーディオ修復（ハム除去 / デクリック / 適応 De-Reverb）＆ ADR カウント収録ビープ ＆ 5.1ch / FOA Ambisonics バス
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで放送・映画ポストプロダクション品質の音響修復＆空間バスを実現する:
//! - 縦続 2 次 IIR ノッチフィルタによる電源ハム除去 (`apply_hum_removal`: 50/60Hz 基本波＋高調波 K 次)
//! - 中央値・MAD 外れ値検出＋3 次エルミート補間によるインパルスデクリック (`apply_declick`)
//! - 適応後期残響エンベロープ減算による De-Reverb (`apply_adaptive_dereverb`)
//! - ADR 3-2-1 同期ビープ音生成 (`synthesize_adr_preroll_beeps`) ＆ タイムラインキュー配置 (`build_adr_cue_timeline_commands`)
//! - ITU-R BS.775 5.1ch サラウンド (`L, R, C, LFE, Ls, Rs`) ＆ 1 次 Ambisonics FOA B-Format AmbiX (`W, Y, Z, X` SN3D/ACN) ⇔ バイノーラル復号

use crate::dsp::{db_to_linear, linear_to_db};
use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};
use std::path::Path;
use ve_core::{
    AdrCue, EditCommand, ItemKind, RgbaColor, SpatialBusConfig, TextAnimator, TimelineItem,
    TimelineMarker,
};

// ============================================================================
// 1. 縦続ノッチ IIR 電源ハム除去 (50Hz / 60Hz + 高調波)
// ============================================================================

/// 2 次 RBJ オーディオノッチフィルタ（双二次 IIR）
#[derive(Debug, Clone, Copy)]
struct NotchBiquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl NotchBiquad {
    fn new(sample_rate: f32, freq_hz: f32, q: f32) -> Self {
        let f = freq_hz.clamp(10.0, sample_rate * 0.48);
        let q = q.clamp(1.0, 120.0);
        let w0 = TAU * f / sample_rate.max(1.0);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: 1.0 / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: 1.0 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// 2 次 Butterworth ローパスフィルタ（5.1ch LFE サブウーファー抽出用）
#[derive(Debug, Clone, Copy)]
struct LowPassBiquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl LowPassBiquad {
    fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        let f = cutoff_hz.clamp(20.0, sample_rate * 0.45);
        let w0 = TAU * f / sample_rate.max(1.0);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) * 0.5) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// 電源ハム（50Hz / 60Hz 基本波および最大 12 次までの高調波）を縦続ノッチ IIR で除去する。
/// 戻り値: `(適用ノッチ数, 除去前 RMS dB, 除去後 RMS dB)`。
pub fn apply_hum_removal(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    base_hz: f32,
    harmonics: usize,
    q: f32,
    mix: f32,
) -> (usize, f32, f32) {
    if samples.is_empty() || channels == 0 || sample_rate == 0 {
        return (0, -70.0, -70.0);
    }
    let before_db = buffer_rms_db(samples);
    let sr = sample_rate as f32;
    let f0 = base_hz.clamp(40.0, 120.0);
    let max_h = harmonics.clamp(1, 12);
    let wet = mix.clamp(0.0, 1.0);

    let mut freqs = Vec::new();
    for k in 1..=max_h {
        let fk = f0 * k as f32;
        if fk < sr * 0.45 {
            freqs.push(fk);
        }
    }
    if freqs.is_empty() || wet <= 1e-4 {
        return (0, before_db, before_db);
    }

    // チャンネルごとに全高調波の縦続 2 段ノッチ（深い減衰）を配置
    let mut banks: Vec<Vec<NotchBiquad>> = (0..channels)
        .map(|_| {
            let mut chain = Vec::with_capacity(freqs.len() * 2);
            for &fk in &freqs {
                chain.push(NotchBiquad::new(sr, fk, q));
                chain.push(NotchBiquad::new(sr, fk, q * 1.15));
            }
            chain
        })
        .collect();

    for frame in samples.chunks_exact_mut(channels) {
        for (ch, s) in frame.iter_mut().enumerate() {
            let dry = *s;
            let mut v = dry;
            for f in &mut banks[ch] {
                v = f.process(v);
            }
            *s = dry * (1.0 - wet) + v * wet;
        }
    }

    let after_db = buffer_rms_db(samples);
    (freqs.len(), before_db, after_db)
}

// ============================================================================
// 2. 中央値＋MAD 予測インパルスデクリック（リップノイズ・ポップ・レコード傷）
// ============================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeclickReport {
    pub clicks_repaired: usize,
    pub samples_interpolated: usize,
    pub peak_before: f32,
    pub peak_after: f32,
}

/// 局所 2 階差分と中央値絶対偏差（MAD）によるインパルスノイズ検出＋3 次エルミート補間修復
pub fn apply_declick(
    samples: &mut [f32],
    channels: usize,
    sensitivity: f32,
    max_click_samples: usize,
) -> DeclickReport {
    if samples.len() < channels * 16 || channels == 0 {
        return DeclickReport::default();
    }
    let frames = samples.len() / channels;
    let peak_before = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    let thresh_mult = sensitivity.clamp(2.0, 12.0);
    let max_run = max_click_samples.clamp(1, 32);
    let half_win = 12usize;

    let mut clicks_repaired = 0usize;
    let mut samples_interpolated = 0usize;

    let mut ch_buf = vec![0.0_f32; frames];
    let mut diff2 = vec![0.0_f32; frames];
    let mut scratch = Vec::with_capacity(half_win * 2 + 1);
    let mut flagged = vec![false; frames];

    for ch in 0..channels {
        for f in 0..frames {
            ch_buf[f] = samples[f * channels + ch];
        }
        // 2 階差分（滑らかな音声波形では小さく、鋭いクリックで巨大になる）
        diff2[0] = 0.0;
        diff2[frames - 1] = 0.0;
        for f in 1..(frames - 1) {
            diff2[f] = (ch_buf[f] - 0.5 * (ch_buf[f - 1] + ch_buf[f + 1])).abs();
        }

        flagged.fill(false);
        for f in 2..(frames - 2) {
            let lo = f.saturating_sub(half_win);
            let hi = (f + half_win + 1).min(frames);
            scratch.clear();
            scratch.extend_from_slice(&diff2[lo..hi]);
            scratch.sort_by(|a, b| a.total_cmp(b));
            let median = scratch[scratch.len() / 2].max(1e-4);
            if diff2[f] > median * thresh_mult && diff2[f] > 0.035 {
                flagged[f] = true;
            }
        }

        // 連続したフラグ区間（<= max_run）を前後正常サンプルの 3 次エルミート補間で滑らか修復
        let mut f = 2usize;
        while f < frames - 2 {
            if !flagged[f] {
                f += 1;
                continue;
            }
            let start = f;
            while f < frames - 2 && flagged[f] {
                f += 1;
            }
            let end = f; // exclusive
            let run_len = end - start;
            if run_len <= max_run && start >= 2 && end + 1 < frames {
                let y0 = ch_buf[start - 1];
                let y1 = ch_buf[end];
                let m0 = (ch_buf[start - 1] - ch_buf[start - 2]) * run_len as f32;
                let m1 = (ch_buf[end + 1] - ch_buf[end]) * run_len as f32;
                for k in 0..run_len {
                    let t = (k + 1) as f32 / (run_len + 1) as f32;
                    let t2 = t * t;
                    let t3 = t2 * t;
                    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
                    let h10 = t3 - 2.0 * t2 + t;
                    let h01 = -2.0 * t3 + 3.0 * t2;
                    let h11 = t3 - t2;
                    ch_buf[start + k] =
                        (h00 * y0 + h10 * m0 + h01 * y1 + h11 * m1).clamp(-1.0, 1.0);
                }
                clicks_repaired += 1;
                samples_interpolated += run_len;
            }
        }

        for (fi, &v) in ch_buf.iter().enumerate() {
            samples[fi * channels + ch] = v;
        }
    }

    let peak_after = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    DeclickReport {
        clicks_repaired,
        samples_interpolated,
        peak_before,
        peak_after,
    }
}

// ============================================================================
// 3. 適応後期残響エンベロープ減算 De-Reverb
// ============================================================================

/// 室内残響（後期リバーブテイル）を 3 帯域（低・中・高）の高速アタック／緩慢リバーブ減衰エンベロープ差分から
/// 適応抑圧する Pure-Rust De-Reverb。直接音のアタックを保ちつつ尾引き残響だけを減衰させる。
pub fn apply_adaptive_dereverb(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    strength: f32,
    decay_ms: f32,
    floor_db: f32,
) {
    if samples.is_empty() || channels == 0 || sample_rate == 0 {
        return;
    }
    let sr = sample_rate as f32;
    let amount = strength.clamp(0.0, 1.0);
    if amount <= 1e-4 {
        return;
    }
    let min_gain = db_to_linear(floor_db.clamp(-42.0, -3.0));
    let decay_sec = (decay_ms.clamp(30.0, 600.0)) * 0.001;
    let fast_coeff = (-1.0 / (0.006 * sr)).exp();
    let late_coeff = (-1.0 / (decay_sec * sr)).exp();
    let gain_smooth = (-1.0 / (0.008 * sr)).exp();

    // 簡易 2 分割クロスオーバー（低域 <= 350Hz / 中高域 > 350Hz）で各帯域独立に残響尾を抑圧
    let mut lp_split: Vec<LowPassBiquad> = (0..channels)
        .map(|_| LowPassBiquad::new(sr, 350.0))
        .collect();
    let mut env_fast_lo = vec![0.0_f32; channels];
    let mut env_late_lo = vec![0.0_f32; channels];
    let mut gain_lo = vec![1.0_f32; channels];
    let mut env_fast_hi = vec![0.0_f32; channels];
    let mut env_late_hi = vec![0.0_f32; channels];
    let mut gain_hi = vec![1.0_f32; channels];

    for frame in samples.chunks_exact_mut(channels) {
        for (ch, s) in frame.iter_mut().enumerate() {
            let x = *s;
            let lo = lp_split[ch].process(x);
            let hi = x - lo;

            let p_lo = lo * lo;
            env_fast_lo[ch] = fast_coeff * env_fast_lo[ch] + (1.0 - fast_coeff) * p_lo;
            env_late_lo[ch] = if p_lo > env_late_lo[ch] {
                // 初期反射を取り込みつつ緩やかに立ち上がる
                0.96 * env_late_lo[ch] + 0.04 * p_lo
            } else {
                late_coeff * env_late_lo[ch]
            };
            let late_ratio_lo = (env_late_lo[ch] / (env_fast_lo[ch] + 1e-7)).clamp(0.0, 1.5);
            // 残響尾（fast が減衰し late が残っている状態）でゲインを絞る
            let target_g_lo = (1.0 - amount * (late_ratio_lo - 0.35).max(0.0)).clamp(min_gain, 1.0);
            gain_lo[ch] = gain_smooth * gain_lo[ch] + (1.0 - gain_smooth) * target_g_lo;

            let p_hi = hi * hi;
            env_fast_hi[ch] = fast_coeff * env_fast_hi[ch] + (1.0 - fast_coeff) * p_hi;
            env_late_hi[ch] = if p_hi > env_late_hi[ch] {
                0.95 * env_late_hi[ch] + 0.05 * p_hi
            } else {
                late_coeff * env_late_hi[ch]
            };
            let late_ratio_hi = (env_late_hi[ch] / (env_fast_hi[ch] + 1e-7)).clamp(0.0, 1.5);
            let target_g_hi =
                (1.0 - amount * 1.15 * (late_ratio_hi - 0.30).max(0.0)).clamp(min_gain, 1.0);
            gain_hi[ch] = gain_smooth * gain_hi[ch] + (1.0 - gain_smooth) * target_g_hi;

            *s = lo * gain_lo[ch] + hi * gain_hi[ch];
        }
    }
}

// ============================================================================
// 4. ADR カウントダウン 3-2-1 同期ビープ音 ＆ タイムラインキュー配置
// ============================================================================

/// 映画アフレコ（ADR）標準の 3-2-1 同期ビープ音（1000Hz × 3 回・収録開始点 4 拍目は無音）を
/// ステレオ PCM（-12 dBFS, 48kHz 等）で合成する。
pub fn synthesize_adr_preroll_beeps(sample_rate: u32, preroll_sec: f64) -> Vec<f32> {
    let sr = sample_rate.max(8_000);
    let dur = preroll_sec.clamp(1.0, 5.0);
    let total_frames = (dur * sr as f64).round() as usize;
    let mut out = vec![0.0_f32; total_frames * 2];
    let beep_frames = ((0.065 * sr as f64).round() as usize).max(16);
    let beats = dur.floor() as usize;
    let amp = 0.25_f32; // 約 -12 dBFS

    for b in 0..beats {
        let start_f = (b as f64 * sr as f64).round() as usize;
        for i in 0..beep_frames {
            let idx = start_f + i;
            if idx >= total_frames {
                break;
            }
            let t = i as f32 / sr as f32;
            let env = (PI * i as f32 / beep_frames as f32).sin();
            let s = (TAU * 1000.0 * t).sin() * env * amp;
            out[idx * 2] = s;
            out[idx * 2 + 1] = s;
        }
    }
    out
}

/// ADR キュー一覧からタイムラインマーカー＆ガイド台詞テロップを 1-Undo `EditCommand::Batch` へ構築する
pub fn build_adr_cue_timeline_commands(
    cues: &[AdrCue],
    subtitle_layer: usize,
) -> Option<EditCommand> {
    if cues.is_empty() {
        return None;
    }
    let mut cmds = Vec::with_capacity(cues.len() * 2);
    for cue in cues {
        cmds.push(EditCommand::AddMarker {
            marker: TimelineMarker {
                id: cue.id,
                time: cue.start_time,
                label: format!("ADR #{} [{}]", cue.cue_number, cue.character),
                color: RgbaColor {
                    r: 0.96,
                    g: 0.26,
                    b: 0.34,
                    a: 1.0,
                },
            },
        });
        let guide = TimelineItem::new(
            format!("ADR#{} {}", cue.cue_number, cue.character),
            cue.start_time,
            cue.duration,
            ItemKind::Text {
                content: format!("[{}] {}", cue.character, cue.line_text),
                font_family: "sans-serif".to_string(),
                font_size: 40.0,
                color: RgbaColor {
                    r: 1.0,
                    g: 0.92,
                    b: 0.45,
                    a: 1.0,
                },
                animator: TextAnimator::default(),
            },
        );
        cmds.push(EditCommand::AddObject {
            layer_index: subtitle_layer,
            item: guide,
        });
    }
    Some(EditCommand::Batch {
        label: format!("adr_place_{}_cues", cues.len()),
        commands: cmds,
    })
}

// ============================================================================
// 5. ITU-R BS.775 5.1ch サラウンド ＆ 1 次 Ambisonics FOA B-Format (AmbiX W,Y,Z,X)
// ============================================================================

/// ITU-R BS.775 5.1ch インターリーブバッファ（チャンネル順: `L, R, C, LFE, Ls, Rs`）
#[derive(Debug, Clone)]
pub struct Surround51Buffer {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

impl Surround51Buffer {
    pub fn frame_count(&self) -> usize {
        self.samples.len() / 6
    }

    pub fn channel_rms_db(&self) -> [f32; 6] {
        let frames = self.frame_count().max(1);
        let mut sums = [0.0_f64; 6];
        for fr in self.samples.as_chunks::<6>().0 {
            for c in 0..6 {
                sums[c] += f64::from(fr[c]) * f64::from(fr[c]);
            }
        }
        std::array::from_fn(|c| linear_to_db((sums[c] / frames as f64).sqrt() as f32))
    }
}

/// ステレオ PCM を `SpatialBusConfig`（方位角・センター台詞寄与・LFE クロスオーバー・広がり）に基づき
/// ITU-R BS.775 5.1ch サラウンド (`L, R, C, LFE, Ls, Rs`) へエンコードする
pub fn encode_stereo_to_51_surround(
    stereo: &[f32],
    sample_rate: u32,
    cfg: &SpatialBusConfig,
) -> Surround51Buffer {
    let frames = stereo.len() / 2;
    let sr = sample_rate.max(8_000) as f32;
    let mut out = vec![0.0_f32; frames * 6];
    let mut lfe_lp = LowPassBiquad::new(sr, cfg.lfe_crossover_hz.clamp(40.0, 160.0));

    let az = cfg.azimuth_deg.clamp(-180.0, 180.0).to_radians();
    // 5 仮想スピーカーの標準 ITU-R BS.775 配置角（度）: L=-30, R=+30, C=0, Ls=-110, Rs=+110
    let spk_angles: [f32; 5] = [
        -30.0_f32.to_radians(),
        30.0_f32.to_radians(),
        0.0_f32,
        -110.0_f32.to_radians(),
        110.0_f32.to_radians(),
    ];
    let mut gains = [0.0_f32; 5];
    let mut sum_sq = 0.0_f32;
    for (i, &sa) in spk_angles.iter().enumerate() {
        let d = (az - sa).cos().max(0.0);
        let g = d * d;
        gains[i] = g;
        sum_sq += g * g;
    }
    let norm = if sum_sq > 1e-6 {
        1.0 / sum_sq.sqrt()
    } else {
        1.0
    };
    for g in &mut gains {
        *g *= norm;
    }

    let c_boost = cfg.center_dialogue_boost.clamp(0.0, 1.0);
    let width = cfg.room_width.clamp(0.0, 1.0);

    for f in 0..frames {
        let l = stereo[f * 2];
        let r = stereo[f * 2 + 1];
        let mid = 0.5 * (l + r);
        let side = 0.5 * (l - r);

        let lfe = lfe_lp.process(mid) * 0.7;
        let base_c = mid * c_boost * FRAC_1_SQRT_2;
        let front_l = l * (1.0 - c_boost * 0.35) * (0.5 + 0.5 * gains[0]) + mid * gains[0] * 0.35;
        let front_r = r * (1.0 - c_boost * 0.35) * (0.5 + 0.5 * gains[1]) + mid * gains[1] * 0.35;
        let center = (base_c + mid * gains[2] * 0.5).clamp(-1.0, 1.0);
        let sur_l = (mid * gains[3] + side * width * 0.45).clamp(-1.0, 1.0);
        let sur_r = (mid * gains[4] - side * width * 0.45).clamp(-1.0, 1.0);

        let base = f * 6;
        out[base] = front_l.clamp(-1.0, 1.0);
        out[base + 1] = front_r.clamp(-1.0, 1.0);
        out[base + 2] = center;
        out[base + 3] = lfe.clamp(-1.0, 1.0);
        out[base + 4] = sur_l;
        out[base + 5] = sur_r;
    }

    Surround51Buffer {
        sample_rate: sample_rate.max(8_000),
        samples: out,
    }
}

/// ITU-R BS.775 標準係数（`C: -3dB`, `Ls/Rs: -3dB`）で 5.1ch サラウンドを 2.0 ステレオへダウンミックスする
pub fn downmix_51_to_stereo_itu(buf: &Surround51Buffer) -> Vec<f32> {
    let frames = buf.frame_count();
    let mut out = vec![0.0_f32; frames * 2];
    for f in 0..frames {
        let base = f * 6;
        let l = buf.samples[base];
        let r = buf.samples[base + 1];
        let c = buf.samples[base + 2];
        let lfe = buf.samples[base + 3];
        let ls = buf.samples[base + 4];
        let rs = buf.samples[base + 5];
        out[f * 2] = (l + FRAC_1_SQRT_2 * c + FRAC_1_SQRT_2 * ls + 0.25 * lfe).clamp(-1.0, 1.0);
        out[f * 2 + 1] = (r + FRAC_1_SQRT_2 * c + FRAC_1_SQRT_2 * rs + 0.25 * lfe).clamp(-1.0, 1.0);
    }
    out
}

/// 1 次 Ambisonics FOA B-Format AmbiX バッファ（ACN / SN3D 順: `0=W, 1=Y, 2=Z, 3=X`）
#[derive(Debug, Clone)]
pub struct FoaAmbiXBuffer {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

impl FoaAmbiXBuffer {
    pub fn frame_count(&self) -> usize {
        self.samples.len() / 4
    }

    pub fn channel_rms_db(&self) -> [f32; 4] {
        let frames = self.frame_count().max(1);
        let mut sums = [0.0_f64; 4];
        for fr in self.samples.as_chunks::<4>().0 {
            for c in 0..4 {
                sums[c] += f64::from(fr[c]) * f64::from(fr[c]);
            }
        }
        std::array::from_fn(|c| linear_to_db((sums[c] / frames as f64).sqrt() as f32))
    }
}

/// ステレオ PCM を 1 次 Ambisonics FOA B-Format (AmbiX ACN/SN3D: `W, Y, Z, X`) へエンコードする。
/// 右手系規約: `az = 0°` が正面 (+X)、`az = -90°` が左 (+Y)、`el = +90°` が天頂 (+Z)。
pub fn encode_stereo_to_foa_ambix(
    stereo: &[f32],
    sample_rate: u32,
    cfg: &SpatialBusConfig,
) -> FoaAmbiXBuffer {
    let frames = stereo.len() / 2;
    let mut out = vec![0.0_f32; frames * 4];
    let az_base = -cfg.azimuth_deg.clamp(-180.0, 180.0).to_radians();
    let el = cfg.elevation_deg.clamp(-90.0, 90.0).to_radians();
    let spread = (cfg.room_width.clamp(0.0, 1.0) * 35.0).to_radians();

    let az_l = az_base + spread;
    let az_r = az_base - spread;
    let cos_el = el.cos();
    let sin_el = el.sin();

    // SN3D 1 次球面調和基底: W = 1, Y = sin(az)cos(el), Z = sin(el), X = cos(az)cos(el)
    let yl = az_l.sin() * cos_el;
    let xl = az_l.cos() * cos_el;
    let yr = az_r.sin() * cos_el;
    let xr = az_r.cos() * cos_el;

    for f in 0..frames {
        let l = stereo[f * 2] * 0.5;
        let r = stereo[f * 2 + 1] * 0.5;
        let base = f * 4;
        out[base] = (l + r) * FRAC_1_SQRT_2; // W
        out[base + 1] = l * yl + r * yr; // Y (Left-Right)
        out[base + 2] = (l + r) * sin_el; // Z (Up-Down)
        out[base + 3] = l * xl + r * xr; // X (Front-Back)
    }

    FoaAmbiXBuffer {
        sample_rate: sample_rate.max(8_000),
        samples: out,
    }
}

/// FOA AmbiX 音場をリスナーの頭部ヨー／ピッチ角だけ 3D 回転する（VR/ヘッドトラッキング対応）
pub fn rotate_foa_ambix(buf: &mut FoaAmbiXBuffer, head_yaw_deg: f32, head_pitch_deg: f32) {
    if head_yaw_deg.abs() < 1e-4 && head_pitch_deg.abs() < 1e-4 {
        return;
    }
    let yaw = head_yaw_deg.to_radians();
    let pitch = head_pitch_deg.to_radians();
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let (cp, sp) = (pitch.cos(), pitch.sin());

    for fr in buf.samples.as_chunks_mut::<4>().0 {
        let y = fr[1];
        let z = fr[2];
        let x = fr[3];
        // Yaw around Z axis
        let x1 = x * cy - y * sy;
        let y1 = x * sy + y * cy;
        // Pitch around Y axis
        let x2 = x1 * cp + z * sp;
        let z2 = -x1 * sp + z * cp;
        fr[1] = y1;
        fr[2] = z2;
        fr[3] = x2;
    }
}

/// FOA AmbiX (`W, Y, Z, X`) を仮想 4 方位カーディオイド＋ITD（両耳時間差）＋頭部陰影 LPF で
/// バイノーラルステレオへ復号する
pub fn decode_foa_ambix_to_binaural_stereo(buf: &FoaAmbiXBuffer) -> Vec<f32> {
    let frames = buf.frame_count();
    let mut out = vec![0.0_f32; frames * 2];
    let sr = buf.sample_rate.max(8_000) as f32;
    // 約 0.35ms の対側耳遅延リングバッファ（ITD）
    let itd_delay = ((0.00035 * sr).round() as usize).clamp(1, 32);
    let mut dl_ring = vec![0.0_f32; itd_delay];
    let mut dr_ring = vec![0.0_f32; itd_delay];
    let mut lp_back_l = LowPassBiquad::new(sr, 4500.0);
    let mut lp_back_r = LowPassBiquad::new(sr, 4500.0);

    for f in 0..frames {
        let base = f * 4;
        let w = buf.samples[base];
        let y = buf.samples[base + 1];
        let z = buf.samples[base + 2];
        let x = buf.samples[base + 3];

        // 仮想左耳 (+Y)・右耳 (-Y) カーディオイド復号＋仰角微妙カラーレーション
        let left_direct = (w + 0.72 * y + 0.38 * x + 0.12 * z).clamp(-1.0, 1.0);
        let right_direct = (w - 0.72 * y + 0.38 * x + 0.12 * z).clamp(-1.0, 1.0);
        // 後方成分（-X）は耳介陰影として高域減衰
        let back_l = lp_back_l.process((w + 0.5 * y - 0.5 * x) * 0.25);
        let back_r = lp_back_r.process((w - 0.5 * y - 0.5 * x) * 0.25);

        let slot = f % itd_delay;
        let cross_r_to_l = dr_ring[slot] * 0.18;
        let cross_l_to_r = dl_ring[slot] * 0.18;
        dl_ring[slot] = left_direct;
        dr_ring[slot] = right_direct;

        out[f * 2] = (left_direct + back_l + cross_r_to_l).clamp(-1.0, 1.0);
        out[f * 2 + 1] = (right_direct + back_r + cross_l_to_r).clamp(-1.0, 1.0);
    }
    out
}

/// 任意チャンネル数（2ch / 4ch FOA AmbiX / 6ch 5.1ch）の 16-bit PCM WAV ファイルを書き出す
pub fn write_multichannel_wav_file(
    path: &Path,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
) -> Result<usize, String> {
    if channels == 0 || sample_rate == 0 {
        return Err("channels と sample_rate は 1 以上が必要です".to_string());
    }
    let ch = usize::from(channels);
    let usable = (samples.len() / ch) * ch;
    let data_bytes = (usable * 2) as u32;
    let byte_rate = sample_rate * u32::from(channels) * 2;
    let block_align = channels * 2;

    let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in &samples[..usable] {
        let q = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        bytes.extend_from_slice(&q.to_le_bytes());
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成失敗: {e}"))?;
    }
    std::fs::write(path, &bytes).map_err(|e| format!("WAV 書込失敗: {e}"))?;
    Ok(bytes.len())
}

fn buffer_rms_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -70.0;
    }
    let ms = samples
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / samples.len() as f64;
    linear_to_db(ms.sqrt() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hum_removal_attenuates_50hz_and_harmonics_by_over_20db() {
        let sr = 48_000u32;
        let frames = sr as usize; // 1 second
        let mut buf = vec![0.0_f32; frames * 2];
        for i in 0..frames {
            let t = i as f32 / sr as f32;
            let hum = 0.4 * (TAU * 50.0 * t).sin()
                + 0.2 * (TAU * 100.0 * t).sin()
                + 0.1 * (TAU * 150.0 * t).sin();
            buf[i * 2] = hum;
            buf[i * 2 + 1] = hum;
        }
        let (notches, before_db, after_db) = apply_hum_removal(&mut buf, 2, sr, 50.0, 5, 22.0, 1.0);
        assert_eq!(notches, 5);
        // 後半 0.5 秒の定常減衰を検証
        let tail_db = buffer_rms_db(&buf[frames..]);
        assert!(
            before_db - tail_db > 20.0,
            "before={before_db:.1}dB after={after_db:.1}dB tail={tail_db:.1}dB"
        );
    }

    #[test]
    fn declick_repairs_synthetic_impulses_and_preserves_sine() {
        let sr = 48_000u32;
        let frames = 4096usize;
        let mut buf = vec![0.0_f32; frames * 2];
        for i in 0..frames {
            let s = 0.2 * (TAU * 440.0 * i as f32 / sr as f32).sin();
            buf[i * 2] = s;
            buf[i * 2 + 1] = s;
        }
        // 鋭いリップクリックを 3 箇所へ注入
        for &pos in &[500usize, 1500, 2800] {
            buf[pos * 2] = 0.98;
            buf[pos * 2 + 1] = -0.96;
        }
        let rep = apply_declick(&mut buf, 2, 4.0, 6);
        assert!(rep.clicks_repaired >= 6, "rep={rep:?}");
        assert!(rep.peak_after < 0.35, "peak_after={}", rep.peak_after);
    }

    #[test]
    fn adaptive_dereverb_suppresses_late_reverb_tail() {
        let sr = 48_000u32;
        let frames = sr as usize;
        let mut buf = vec![0.0_f32; frames * 2];
        // 先頭 0.15s に直接音、その後 0.85s に減衰する残響テイルを模擬
        for i in 0..frames {
            let t = i as f32 / sr as f32;
            let s = if t < 0.15 {
                0.5 * (TAU * 800.0 * t).sin()
            } else {
                0.35 * (-(t - 0.15) / 0.25).exp() * (TAU * 800.0 * t).sin()
            };
            buf[i * 2] = s;
            buf[i * 2 + 1] = s;
        }
        let tail_before = buffer_rms_db(&buf[(sr as usize / 2) * 2..]);
        apply_adaptive_dereverb(&mut buf, 2, sr, 0.85, 180.0, -24.0);
        let tail_after = buffer_rms_db(&buf[(sr as usize / 2) * 2..]);
        assert!(
            tail_after < tail_before - 2.0,
            "before={tail_before:.2} after={tail_after:.2}"
        );
    }

    #[test]
    fn surround_51_and_foa_ambix_encode_rotate_and_decode_cleanly() {
        let sr = 48_000u32;
        let frames = 2400usize;
        let mut stereo = vec![0.0_f32; frames * 2];
        for i in 0..frames {
            let t = i as f32 / sr as f32;
            // 低域 (60Hz) + 声帯域 (600Hz)
            let s = 0.3 * (TAU * 60.0 * t).sin() + 0.3 * (TAU * 600.0 * t).sin();
            stereo[i * 2] = s;
            stereo[i * 2 + 1] = s;
        }

        let cfg_left = SpatialBusConfig {
            azimuth_deg: -60.0,
            elevation_deg: 20.0,
            lfe_crossover_hz: 90.0,
            center_dialogue_boost: 0.5,
            room_width: 0.6,
            head_yaw_deg: 0.0,
            head_pitch_deg: 0.0,
            ..Default::default()
        };
        let s51 = encode_stereo_to_51_surround(&stereo, sr, &cfg_left);
        assert_eq!(s51.frame_count(), frames);
        let rms51 = s51.channel_rms_db();
        assert!(
            rms51[0] > rms51[1],
            "left azimuth should favor L over R: {rms51:?}"
        );
        assert!(rms51[3] > -40.0, "LFE should carry 60Hz energy: {rms51:?}");
        let down = downmix_51_to_stereo_itu(&s51);
        assert_eq!(down.len(), stereo.len());

        let mut foa = encode_stereo_to_foa_ambix(&stereo, sr, &cfg_left);
        assert_eq!(foa.frame_count(), frames);
        let rms_foa = foa.channel_rms_db();
        assert!(rms_foa[0] > -25.0 && rms_foa[1] > -30.0 && rms_foa[2] > -35.0);
        rotate_foa_ambix(&mut foa, 45.0, 10.0);
        let bin = decode_foa_ambix_to_binaural_stereo(&foa);
        assert_eq!(bin.len(), stereo.len());

        let beeps = synthesize_adr_preroll_beeps(sr, 3.0);
        assert_eq!(beeps.len(), sr as usize * 3 * 2);
        assert!(beeps.iter().any(|&v| v.abs() > 0.1));
    }
}
