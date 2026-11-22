//! v4.0 #35: オフライン・プロシージャル効果音（Foley / SFX）シンセサイザー
//! Pure Rust・ゼロ `unsafe`・外部依存ゼロで、トランジション用ウーッシュ・シネマティック低音インパクト・
//! UI ポップ音・グリッチ電子音・環境雨音・テンションライザーの 48kHz ステレオ PCM / WAV を即時生成する。

use std::f32::consts::{PI, TAU};
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FoleyPreset {
    #[default]
    WhooshTransition,
    SubBassImpact,
    UiPopClick,
    GlitchZap,
    AmbientRain,
    RiserTension,
}

impl FoleyPreset {
    pub const ALL: &'static [Self] = &[
        Self::WhooshTransition,
        Self::SubBassImpact,
        Self::UiPopClick,
        Self::GlitchZap,
        Self::AmbientRain,
        Self::RiserTension,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::WhooshTransition => "whoosh_transition",
            Self::SubBassImpact => "sub_bass_impact",
            Self::UiPopClick => "ui_pop_click",
            Self::GlitchZap => "glitch_zap",
            Self::AmbientRain => "ambient_rain",
            Self::RiserTension => "riser_tension",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "sub_bass_impact" => Self::SubBassImpact,
            "ui_pop_click" => Self::UiPopClick,
            "glitch_zap" => Self::GlitchZap,
            "ambient_rain" => Self::AmbientRain,
            "riser_tension" => Self::RiserTension,
            _ => Self::WhooshTransition,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::WhooshTransition => "💨 ウーッシュ遷移 (Whoosh Transition)",
            Self::SubBassImpact => "💥 シネマティック衝撃音 (Sub-Bass Impact)",
            Self::UiPopClick => "✨ UI ポップ / クリック (UI Pop Click)",
            Self::GlitchZap => "⚡ サイバー・グリッチ電子音 (Glitch Zap)",
            Self::AmbientRain => "🌧 環境音・雨粒 (Ambient Rain)",
            Self::RiserTension => "📈 テンション上昇ライザー (Cinematic Riser)",
        }
    }

    pub fn default_duration_sec(self) -> f32 {
        match self {
            Self::WhooshTransition => 0.85,
            Self::SubBassImpact => 1.6,
            Self::UiPopClick => 0.22,
            Self::GlitchZap => 0.55,
            Self::AmbientRain => 4.0,
            Self::RiserTension => 2.5,
        }
    }
}

/// 決定論的 SplitMix64 擬似乱数生成器（`[-1.0, 1.0]`）
struct SplitMixRng {
    state: u64,
}

impl SplitMixRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[inline]
    fn next_unit_signed(&mut self) -> f32 {
        let u = ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32);
        u * 2.0 - 1.0
    }
}

/// 指定プリセット・尺・サンプルレート・シードからステレオインターリーブ PCM (`Vec<f32>`) をオフライン合成する
pub fn synthesize_foley_sfx_pcm(
    preset: FoleyPreset,
    duration_sec: f32,
    sample_rate: u32,
    seed: u64,
) -> Vec<f32> {
    let sr = sample_rate.clamp(8_000, 192_000) as f32;
    let dur = duration_sec.clamp(0.08, 30.0);
    let frames = ((dur * sr).round() as usize).max(64);
    let mut pcm = vec![0.0f32; frames * 2];
    let mut rng_l = SplitMixRng::new(seed ^ 0xA5A5_1234);
    let mut rng_r = SplitMixRng::new(seed.wrapping_mul(31) ^ 0x5A5A_9876);

    match preset {
        FoleyPreset::WhooshTransition => {
            let mut lp_l = 0.0f32;
            let mut lp_r = 0.0f32;
            let mut bp_l = 0.0f32;
            let mut bp_r = 0.0f32;
            let mut phase = 0.0f32;
            for (i, lr) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let u = i as f32 / (frames as f32);
                // ベル型エンベロープ（0.45 付近にピーク）
                let env = (u * PI).sin().powf(1.65);
                // パンが左 → 右へ横切る演出
                let pan = (u * 1.4 - 0.7).clamp(-0.85, 0.85);
                let g_l = ((1.0 - pan) * 0.5).sqrt();
                let g_r = ((1.0 + pan) * 0.5).sqrt();

                let cutoff = 260.0 + 2800.0 * env;
                let alpha = (TAU * cutoff / sr).clamp(0.01, 0.65);
                let n_l = rng_l.next_unit_signed();
                let n_r = rng_r.next_unit_signed();
                lp_l += alpha * (n_l - lp_l);
                lp_r += alpha * (n_r - lp_r);
                bp_l += (alpha * 0.35) * (lp_l - bp_l);
                bp_r += (alpha * 0.35) * (lp_r - bp_r);
                let air_l = lp_l - bp_l * 0.6;
                let air_r = lp_r - bp_r * 0.6;

                phase += TAU * (110.0 + 180.0 * env) / sr;
                let wind_tone = phase.sin() * 0.16;

                lr[0] = ((air_l * 0.85 + wind_tone) * env * g_l * 1.35).clamp(-0.92, 0.92);
                lr[1] = ((air_r * 0.85 + wind_tone) * env * g_r * 1.35).clamp(-0.92, 0.92);
            }
        }
        FoleyPreset::SubBassImpact => {
            let mut phase = 0.0f32;
            let mut sub_phase = 0.0f32;
            let mut lp_noise = 0.0f32;
            for (i, lr) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let freq = 38.0 + 130.0 * (-t * 14.0).exp();
                phase += TAU * freq / sr;
                sub_phase += TAU * (freq * 0.5) / sr;

                let attack = (t / 0.006).min(1.0);
                let body_env = attack * (-t * 2.4).exp();
                let punch_env = attack * (-t * 28.0).exp();

                let noise = rng_l.next_unit_signed();
                lp_noise += 0.18 * (noise - lp_noise);

                let boom = (phase.sin() * 0.78 + sub_phase.sin() * 0.38).tanh();
                let stereo_air = rng_r.next_unit_signed() * punch_env * 0.12;
                let base = boom * body_env * 0.88 + lp_noise * punch_env * 0.55;

                lr[0] = (base + stereo_air).clamp(-0.94, 0.94);
                lr[1] = (base - stereo_air).clamp(-0.94, 0.94);
            }
        }
        FoleyPreset::UiPopClick => {
            let mut p1 = 0.0f32;
            let mut p2 = 0.0f32;
            for (i, lr) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let f1 = 520.0 + 880.0 * (1.0 - (-t * 55.0).exp());
                let f2 = 1640.0 + 620.0 * (-t * 70.0).exp();
                p1 += TAU * f1 / sr;
                p2 += TAU * f2 / sr;

                let attack = (t / 0.0025).min(1.0);
                let env1 = attack * (-t * 24.0).exp();
                let env2 = attack * (-t * 55.0).exp();
                let click = rng_l.next_unit_signed() * (-t * 190.0).exp() * 0.18;

                let sig = p1.sin() * env1 * 0.62 + p2.sin() * env2 * 0.28 + click;
                lr[0] = (sig * 0.98).clamp(-0.90, 0.90);
                lr[1] = (sig * 0.94 + p2.cos() * env2 * 0.06).clamp(-0.90, 0.90);
            }
        }
        FoleyPreset::GlitchZap => {
            let mut carrier = 0.0f32;
            let mut mod_p = 0.0f32;
            let mut step_freq = 440.0f32;
            let mut gate = 1.0f32;
            let step_samples = ((sr * 0.028) as usize).max(16);
            for (i, lr) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                if i % step_samples == 0 {
                    let r = rng_l.next_unit_signed().abs();
                    step_freq = 180.0 + r * 1950.0;
                    gate = if rng_r.next_unit_signed() > -0.45 {
                        1.0
                    } else {
                        0.15
                    };
                }
                let u = i as f32 / frames as f32;
                let env = (1.0 - u).powf(0.6) * ((u * 25.0).min(1.0));
                mod_p += TAU * (step_freq * 1.73) / sr;
                carrier += TAU * (step_freq + mod_p.sin() * 420.0) / sr;
                // デジタル量子化クランチ
                let raw = (carrier.sin() * 6.0).round() / 6.0;
                let zap_r = ((carrier * 1.015).sin() * 5.0).round() / 5.0;
                let crackle = rng_l.next_unit_signed() * 0.15;

                lr[0] = ((raw * 0.68 + crackle) * gate * env).clamp(-0.92, 0.92);
                lr[1] = ((zap_r * 0.68 - crackle) * gate * env).clamp(-0.92, 0.92);
            }
        }
        FoleyPreset::AmbientRain => {
            let mut lp_l = 0.0f32;
            let mut lp_r = 0.0f32;
            let mut drop_env_l = 0.0f32;
            let mut drop_env_r = 0.0f32;
            let mut drop_phase_l = 0.0f32;
            let mut drop_phase_r = 0.0f32;
            let mut drop_freq_l = 1200.0f32;
            let mut drop_freq_r = 1500.0f32;
            for (i, lr) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let u = i as f32 / frames as f32;
                let fade = (u * 12.0).min(1.0) * ((1.0 - u) * 12.0).min(1.0);
                let n_l = rng_l.next_unit_signed();
                let n_r = rng_r.next_unit_signed();
                lp_l += 0.22 * (n_l - lp_l);
                lp_r += 0.22 * (n_r - lp_r);

                // ランダムな水滴の跳ね音
                if rng_l.next_u64().is_multiple_of(3800) {
                    drop_env_l = 0.28;
                    drop_freq_l = 950.0 + (rng_l.next_unit_signed().abs() * 1400.0);
                }
                if rng_r.next_u64().is_multiple_of(4100) {
                    drop_env_r = 0.26;
                    drop_freq_r = 1050.0 + (rng_r.next_unit_signed().abs() * 1500.0);
                }
                drop_phase_l += TAU * drop_freq_l / sr;
                drop_phase_r += TAU * drop_freq_r / sr;
                drop_env_l *= 0.9965;
                drop_env_r *= 0.9965;

                let s_l = lp_l * 0.42 + drop_phase_l.sin() * drop_env_l;
                let s_r = lp_r * 0.42 + drop_phase_r.sin() * drop_env_r;
                lr[0] = (s_l * fade).clamp(-0.88, 0.88);
                lr[1] = (s_r * fade).clamp(-0.88, 0.88);
            }
        }
        FoleyPreset::RiserTension => {
            let mut p_root = 0.0f32;
            let mut p_fifth = 0.0f32;
            let mut p_detune = 0.0f32;
            let mut lp_l = 0.0f32;
            let mut lp_r = 0.0f32;
            for (i, lr) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let u = i as f32 / frames as f32;
                let rise = u * u;
                let tail_cut = if u > 0.96 {
                    ((1.0 - u) / 0.04).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let env = (0.08 + 0.92 * rise) * tail_cut;
                let f = 110.0 * 2.0_f32.powf(u * 2.6);
                p_root += TAU * f / sr;
                p_fifth += TAU * (f * 1.498) / sr;
                p_detune += TAU * (f * 1.007) / sr;

                let alpha = (0.04 + 0.55 * rise).clamp(0.02, 0.7);
                lp_l += alpha * (rng_l.next_unit_signed() - lp_l);
                lp_r += alpha * (rng_r.next_unit_signed() - lp_r);

                let synth_l = (p_root.sin() * 0.42 + p_fifth.sin() * 0.25 + lp_l * 0.42) * env;
                let synth_r = (p_detune.sin() * 0.42 + p_fifth.cos() * 0.25 + lp_r * 0.42) * env;
                lr[0] = synth_l.clamp(-0.92, 0.92);
                lr[1] = synth_r.clamp(-0.92, 0.92);
            }
        }
    }

    pcm
}

/// インターリーブステレオ `f32` PCM を標準 16-bit PCM WAV バイナリへエンコードする
pub fn encode_stereo_wav_pcm16(pcm: &[f32], sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * (channels as u32) * (bits_per_sample as u32 / 8);
    let block_align = channels * (bits_per_sample / 8);
    let data_bytes = (pcm.len() * 2) as u32;
    let riff_size = 36u32.saturating_add(data_bytes);

    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());

    for &sample in pcm {
        let clamped = sample.clamp(-1.0, 1.0);
        let q = (clamped * 32767.0).round() as i16;
        out.extend_from_slice(&q.to_le_bytes());
    }
    out
}

/// 生成したステレオ PCM を指定パスへ 16-bit WAV ファイルとして保存する
pub fn write_stereo_wav_file(path: &Path, pcm: &[f32], sample_rate: u32) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create wav directory: {e}"))?;
    }
    let bytes = encode_stereo_wav_pcm16(pcm, sample_rate);
    std::fs::write(path, bytes).map_err(|e| format!("failed to write wav file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_foley_presets_generate_audible_non_clipping_deterministic_stereo_wav() {
        for &preset in FoleyPreset::ALL {
            let pcm_a = synthesize_foley_sfx_pcm(preset, 0.35, 48_000, 12345);
            let pcm_b = synthesize_foley_sfx_pcm(preset, 0.35, 48_000, 12345);
            assert_eq!(pcm_a, pcm_b, "preset {:?} must be deterministic", preset);
            assert!(!pcm_a.is_empty());
            assert_eq!(pcm_a.len() % 2, 0);

            let peak = pcm_a.iter().fold(0.0f32, |acc, &s| {
                assert!(s.is_finite());
                acc.max(s.abs())
            });
            assert!(
                peak > 0.05 && peak <= 0.96,
                "preset {:?} peak out of expected audible safe range: {peak}",
                preset
            );

            let wav = encode_stereo_wav_pcm16(&pcm_a, 48_000);
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(&wav[8..12], b"WAVE");
            assert_eq!(wav.len(), 44 + pcm_a.len() * 2);
        }
    }
}
