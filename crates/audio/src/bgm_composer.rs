//! v5.0 #38: オフライン自動プロシージャル BGM コンポーザー（音楽理論・コード進行・マルチトラック合成）
//! Pure Rust・ゼロ `unsafe`・著作権フリー完全オフラインで、ドラム・ベース・コードパッド・メロディの
//! 48kHz ステレオ BGM を決定論合成し、ビート／小節マーカー時刻を同時生成する。

use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BgmGenre {
    #[default]
    LoFiChill,
    UpbeatPop,
    CinematicEpic,
    CyberSynthwave,
    AcousticExplainer,
    SuspensePulse,
}

impl BgmGenre {
    pub const ALL: &'static [Self] = &[
        Self::LoFiChill,
        Self::UpbeatPop,
        Self::CinematicEpic,
        Self::CyberSynthwave,
        Self::AcousticExplainer,
        Self::SuspensePulse,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::LoFiChill => "lofi_chill",
            Self::UpbeatPop => "upbeat_pop",
            Self::CinematicEpic => "cinematic_epic",
            Self::CyberSynthwave => "cyber_synthwave",
            Self::AcousticExplainer => "acoustic_explainer",
            Self::SuspensePulse => "suspense_pulse",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s {
            "upbeat_pop" => Self::UpbeatPop,
            "cinematic_epic" => Self::CinematicEpic,
            "cyber_synthwave" => Self::CyberSynthwave,
            "acoustic_explainer" => Self::AcousticExplainer,
            "suspense_pulse" => Self::SuspensePulse,
            _ => Self::LoFiChill,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::LoFiChill => "☕ Lo-Fi チルホップ (82 BPM)",
            Self::UpbeatPop => "🎉 アップビート・ポップ (124 BPM)",
            Self::CinematicEpic => "🎬 シネマティック壮大オーケストラ調 (96 BPM)",
            Self::CyberSynthwave => "🕹 サイバー・シンセウェーブ (114 BPM)",
            Self::AcousticExplainer => "📚 解説・Vlog 向け爽やかアコースティック (106 BPM)",
            Self::SuspensePulse => "🕵 サスペンス・パルステック (118 BPM)",
        }
    }

    pub fn default_bpm(self) -> f32 {
        match self {
            Self::LoFiChill => 82.0,
            Self::UpbeatPop => 124.0,
            Self::CinematicEpic => 96.0,
            Self::CyberSynthwave => 114.0,
            Self::AcousticExplainer => 106.0,
            Self::SuspensePulse => 118.0,
        }
    }

    pub fn progression_label(self) -> &'static str {
        match self {
            Self::LoFiChill => "ii7 - V7 - Imaj7 - vi7",
            Self::UpbeatPop => "I - V - vi - IV",
            Self::CinematicEpic => "vi - IV - I - V",
            Self::CyberSynthwave => "i - VI - III - VII",
            Self::AcousticExplainer => "I - iii - IV - V",
            Self::SuspensePulse => "i - bII - i - V7",
        }
    }

    /// 4 小節分のコード構成音（ルート基準半音オフセット 3 音）
    fn chords(self) -> [[i32; 3]; 4] {
        match self {
            Self::LoFiChill => [[2, 5, 9], [7, 11, 14], [0, 4, 7], [9, 12, 16]],
            Self::UpbeatPop => [[0, 4, 7], [7, 11, 14], [9, 12, 16], [5, 9, 12]],
            Self::CinematicEpic => [[9, 12, 16], [5, 9, 12], [0, 4, 7], [7, 11, 14]],
            Self::CyberSynthwave => [[0, 3, 7], [8, 12, 15], [3, 7, 10], [10, 14, 17]],
            Self::AcousticExplainer => [[0, 4, 7], [4, 7, 11], [5, 9, 12], [7, 11, 14]],
            Self::SuspensePulse => [[0, 3, 7], [1, 5, 8], [0, 3, 7], [7, 11, 14]],
        }
    }

    /// メロディ生成用の 5〜7 音スケール（半音オフセット）
    fn scale(self) -> &'static [i32] {
        match self {
            Self::LoFiChill | Self::UpbeatPop | Self::AcousticExplainer => &[0, 2, 4, 7, 9, 12, 14],
            Self::CinematicEpic => &[0, 2, 4, 5, 7, 9, 11, 12],
            Self::CyberSynthwave => &[0, 3, 5, 7, 10, 12, 15],
            Self::SuspensePulse => &[0, 1, 3, 5, 7, 8, 10, 12],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BgmComposeConfig {
    pub genre: BgmGenre,
    pub bpm: f32,
    /// キー（0=C, 2=D, 4=E, 5=F, 7=G, 9=A）
    pub key_semitone: i32,
    pub duration_sec: f32,
    /// アレンジ密度・エネルギー (0.3..=1.5)
    pub intensity: f32,
    pub seed: u64,
}

impl Default for BgmComposeConfig {
    fn default() -> Self {
        Self {
            genre: BgmGenre::LoFiChill,
            bpm: BgmGenre::LoFiChill.default_bpm(),
            key_semitone: 0,
            duration_sec: 12.0,
            intensity: 1.0,
            seed: 50_2026,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComposedBgmResult {
    pub pcm: Vec<f32>,
    pub sample_rate: u32,
    pub bpm: f32,
    pub beat_times_sec: Vec<f64>,
    pub bar_times_sec: Vec<f64>,
    pub chord_progression: String,
}

#[inline]
fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
}

#[inline]
fn hash_step(step: usize, seed: u64) -> u64 {
    let mut h = (step as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(seed ^ 0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 30)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 27)
}

#[inline]
fn hash_noise(i: usize, seed: u64) -> f32 {
    let u = ((hash_step(i, seed) >> 40) as f32) / ((1u32 << 24) as f32);
    u * 2.0 - 1.0
}

/// 指定ジャンル・BPM・キー・尺から完全オフラインでマルチトラック 48kHz ステレオ BGM と
/// 正確な拍／小節マーカー時刻一覧を自動作曲・合成する
pub fn compose_bgm_track(cfg: &BgmComposeConfig, sample_rate: u32) -> ComposedBgmResult {
    let sr = sample_rate.clamp(8_000, 192_000);
    let sr_f = sr as f32;
    let bpm = cfg.bpm.clamp(55.0, 200.0);
    let dur = cfg.duration_sec.clamp(1.0, 120.0);
    let intensity = cfg.intensity.clamp(0.3, 1.5);
    let key = cfg.key_semitone.rem_euclid(12);
    let frames = ((dur * sr_f).round() as usize).max(256);

    let beat_sec = 60.0 / f64::from(bpm);
    let mut beat_times_sec = Vec::new();
    let mut bar_times_sec = Vec::new();
    let mut beat_idx = 0usize;
    while (beat_idx as f64) * beat_sec < f64::from(dur) {
        let bt = (beat_idx as f64) * beat_sec;
        beat_times_sec.push(bt);
        if beat_idx.is_multiple_of(4) {
            bar_times_sec.push(bt);
        }
        beat_idx += 1;
    }

    let chords = cfg.genre.chords();
    let scale = cfg.genre.scale();
    let beat_dur_f = 60.0 / bpm;
    let eighth_dur_f = beat_dur_f * 0.5;
    let bar_dur_f = beat_dur_f * 4.0;

    // 16 ステップ分の決定論メロディモチーフを事前生成
    let mut motif_notes = [None::<f32>; 16];
    for (step, slot) in motif_notes.iter_mut().enumerate() {
        let h = hash_step(step, cfg.seed);
        let density_ok = match cfg.genre {
            BgmGenre::CinematicEpic | BgmGenre::SuspensePulse => step.is_multiple_of(2),
            BgmGenre::LoFiChill => !h.is_multiple_of(3),
            _ => !h.is_multiple_of(4),
        };
        if density_ok {
            let deg = scale[(h as usize) % scale.len()];
            let octave_base = match cfg.genre {
                BgmGenre::CinematicEpic => 60,
                BgmGenre::SuspensePulse => 57,
                _ => 67,
            };
            *slot = Some(midi_to_hz((octave_base + key + deg) as f32));
        }
    }

    let mut pcm = vec![0.0f32; frames * 2];
    let mut pad_ph_l = [0.0f32; 3];
    let mut pad_ph_r = [0.0f32; 3];
    let mut bass_ph = 0.0f32;
    let mut mel_ph = 0.0f32;
    let mut lp_pad_l = 0.0f32;
    let mut lp_pad_r = 0.0f32;

    // 簡易ステレオエコー（約 0.28s 遅延）
    let delay_frames = ((sr_f * 0.28) as usize).clamp(64, 32_000);
    let mut delay_l = vec![0.0f32; delay_frames];
    let mut delay_r = vec![0.0f32; delay_frames];
    let mut delay_pos = 0usize;

    for (i, lr) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let t = i as f32 / sr_f;
        let fade_in = (t / 0.25).min(1.0);
        let fade_out = ((dur - t) / 0.45).clamp(0.0, 1.0);
        let master_env = fade_in * fade_out;

        let bar_idx = (t / bar_dur_f).floor() as usize;
        let chord = chords[bar_idx % 4];
        let beat_pos = (t / beat_dur_f) % 4.0;
        let beat_frac = beat_pos.fract();
        let beat_num = beat_pos.floor() as usize;
        let eighth_idx = (t / eighth_dur_f).floor() as usize;
        let eighth_frac = ((t / eighth_dur_f).fract()) * eighth_dur_f;

        // 1. ドラムグルーヴ（キック・スネア・ハイハット）
        let kick_hit = match cfg.genre {
            BgmGenre::CinematicEpic => beat_num == 0 || beat_num == 2,
            BgmGenre::LoFiChill => beat_num == 0 || (beat_num == 2 && beat_frac > 0.48),
            _ => beat_num == 0 || beat_num == 2,
        };
        let kick_t = if cfg.genre == BgmGenre::LoFiChill && beat_num == 2 && beat_frac > 0.48 {
            (beat_frac - 0.5) * beat_dur_f
        } else {
            beat_frac * beat_dur_f
        };
        let kick = if kick_hit && (0.0..0.24).contains(&kick_t) {
            let kf = 46.0 + 115.0 * (-kick_t * 28.0).exp();
            (TAU * kf * kick_t).sin() * (-kick_t * 14.0).exp() * 0.42 * intensity
        } else {
            0.0
        };

        let snare_hit = beat_num == 1 || beat_num == 3;
        let snare_t = beat_frac * beat_dur_f;
        let snare = if snare_hit && snare_t < 0.16 && cfg.genre != BgmGenre::CinematicEpic {
            let n = hash_noise(i, cfg.seed ^ 0x1111);
            let body = (TAU * 185.0 * snare_t).sin() * (-snare_t * 26.0).exp() * 0.35;
            (n * 0.65 + body) * (-snare_t * 22.0).exp() * 0.26 * intensity
        } else {
            0.0
        };

        let hat_t = eighth_frac;
        let hat = if hat_t < 0.045 {
            let amp_hat = if eighth_idx.is_multiple_of(2) {
                0.08
            } else {
                0.045
            };
            hash_noise(i, cfg.seed ^ 0x2222) * (-hat_t * 85.0).exp() * amp_hat * intensity
        } else {
            0.0
        };

        // サイドチェイン風ダッキング（キック発音直後にパッドを軽く揺らす）
        let pump = if kick_hit && kick_t < 0.16 {
            0.68 + 0.32 * (kick_t / 0.16)
        } else {
            1.0
        };

        // 2. ベースライン
        let bass_midi = (36 + key + chord[0]) as f32;
        let bass_hz = midi_to_hz(bass_midi);
        bass_ph = (bass_ph + TAU * bass_hz / sr_f) % TAU;
        let bass_env = (-eighth_frac * 3.8).exp() * 0.7 + 0.3;
        let bass_wave = match cfg.genre {
            BgmGenre::CyberSynthwave | BgmGenre::SuspensePulse => {
                (bass_ph.sin() * 0.65 + (bass_ph * 2.0).sin() * 0.35).tanh()
            }
            _ => bass_ph.sin() * 0.85 + (bass_ph * 2.0).sin() * 0.15,
        };
        let bass = bass_wave * bass_env * 0.28 * intensity;

        // 3. ステレオコードパッド（左右微細デチューン）
        let mut raw_pad_l = 0.0f32;
        let mut raw_pad_r = 0.0f32;
        for (c_idx, &semi) in chord.iter().enumerate() {
            let hz = midi_to_hz((48 + key + semi) as f32);
            pad_ph_l[c_idx] = (pad_ph_l[c_idx] + TAU * (hz * 0.997) / sr_f) % TAU;
            pad_ph_r[c_idx] = (pad_ph_r[c_idx] + TAU * (hz * 1.003) / sr_f) % TAU;
            let wl = pad_ph_l[c_idx].sin() * 0.72 + (pad_ph_l[c_idx] * 2.0).sin() * 0.16;
            let wr = pad_ph_r[c_idx].sin() * 0.72 + (pad_ph_r[c_idx] * 2.0).sin() * 0.16;
            raw_pad_l += wl;
            raw_pad_r += wr;
        }
        let lp_coeff = match cfg.genre {
            BgmGenre::LoFiChill => 0.09,
            BgmGenre::CyberSynthwave => 0.35,
            _ => 0.18,
        };
        lp_pad_l += lp_coeff * (raw_pad_l * 0.14 - lp_pad_l);
        lp_pad_r += lp_coeff * (raw_pad_r * 0.14 - lp_pad_r);

        // 4. メロディモチーフ＋ステレオエコー
        let mel_sig = if let Some(m_hz) = motif_notes[eighth_idx % 16] {
            mel_ph = (mel_ph + TAU * m_hz / sr_f) % TAU;
            let m_env = (eighth_frac / 0.012).min(1.0) * (-eighth_frac * 5.2).exp();
            let tone = mel_ph.sin() * 0.75 + (mel_ph * 3.0).sin() * 0.15;
            tone * m_env * 0.22 * intensity
        } else {
            0.0
        };

        let echo_l = delay_l[delay_pos];
        let echo_r = delay_r[delay_pos];
        delay_l[delay_pos] = mel_sig * 0.45 + echo_r * 0.28;
        delay_r[delay_pos] = mel_sig * 0.32 + echo_l * 0.28;
        delay_pos = (delay_pos + 1) % delay_frames;

        let mix_l =
            (kick + snare * 0.9 + hat * 0.75 + bass + lp_pad_l * pump + mel_sig + echo_l * 0.45)
                * master_env;
        let mix_r = (kick
            + snare * 1.05
            + hat * 1.15
            + bass
            + lp_pad_r * pump
            + mel_sig * 0.92
            + echo_r * 0.45)
            * master_env;

        // 放送安全ソフトリミット（約 -1.2 dBTP 以内）
        lr[0] = (mix_l * 1.15).tanh() * 0.85;
        lr[1] = (mix_r * 1.15).tanh() * 0.85;
    }

    ComposedBgmResult {
        pcm,
        sample_rate: sr,
        bpm,
        beat_times_sec,
        bar_times_sec,
        chord_progression: cfg.genre.progression_label().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_bgm_genres_compose_deterministic_audible_broadcast_safe_tracks() {
        for &genre in BgmGenre::ALL {
            let cfg = BgmComposeConfig {
                genre,
                bpm: genre.default_bpm(),
                duration_sec: 2.5,
                ..BgmComposeConfig::default()
            };
            let a = compose_bgm_track(&cfg, 48_000);
            let b = compose_bgm_track(&cfg, 48_000);
            assert_eq!(a.pcm, b.pcm, "genre {:?} must be deterministic", genre);
            assert!(!a.beat_times_sec.is_empty());
            assert!(!a.bar_times_sec.is_empty());

            let peak = a.pcm.iter().fold(0.0f32, |acc, &s| {
                assert!(s.is_finite());
                acc.max(s.abs())
            });
            assert!(
                peak > 0.15 && peak <= 0.88,
                "genre {:?} peak out of broadcast-safe audible bounds: {peak}",
                genre
            );
        }
    }
}
