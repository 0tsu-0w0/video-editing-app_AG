//! v3.0 #31: オフライン多言語 AI 吹替 ＆ 音声リップシンク自動生成
//! Pure Rust・ゼロ `unsafe`・クラウド送信ゼロで、多言語テキストからの音素アライメント（`PhonemeSegment`）
//! および 3 フォルマント共振音声合成（48kHz ステレオ PCM）を実行し、立ち絵・映像口パクと自動同期する。

use std::f32::consts::PI;
use ve_core::{
    AssetId, EditCommand, ItemId, ItemKind, LocalizationTable, MouthShape, PhonemeSegment,
    RationalTime, Scene, TimelineItem, strip_control_tags,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DubbingVoicePreset {
    #[default]
    JpFemale,
    JpMale,
    EnFemale,
    EnMale,
    NarratorWarm,
}

impl DubbingVoicePreset {
    pub const ALL: &'static [Self] = &[
        Self::JpFemale,
        Self::JpMale,
        Self::EnFemale,
        Self::EnMale,
        Self::NarratorWarm,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::JpFemale => "jp_female",
            Self::JpMale => "jp_male",
            Self::EnFemale => "en_female",
            Self::EnMale => "en_male",
            Self::NarratorWarm => "narrator_warm",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s {
            "jp_male" => Self::JpMale,
            "en_female" => Self::EnFemale,
            "en_male" => Self::EnMale,
            "narrator_warm" => Self::NarratorWarm,
            _ => Self::JpFemale,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::JpFemale => "🎙 日本語・女性アナウンサー (230Hz)",
            Self::JpMale => "🎙 日本語・男性ナレーター (125Hz)",
            Self::EnFemale => "🎙 English Female Studio (215Hz)",
            Self::EnMale => "🎙 English Male Broadcast (118Hz)",
            Self::NarratorWarm => "🎙 ウォーム解説ボイス (155Hz)",
        }
    }

    pub fn base_pitch_hz(self) -> f32 {
        match self {
            Self::JpFemale => 232.0,
            Self::JpMale => 126.0,
            Self::EnFemale => 216.0,
            Self::EnMale => 118.0,
            Self::NarratorWarm => 156.0,
        }
    }

    /// フォルマント周波数スケール係数（女性声は声道が短いためやや高め）
    pub fn formant_scale(self) -> f32 {
        match self {
            Self::JpFemale | Self::EnFemale => 1.14,
            Self::JpMale | Self::EnMale => 0.94,
            Self::NarratorWarm => 1.0,
        }
    }
}

/// テキスト文字列とクリップ尺から、母音・無音区間付きの音素タイミング列（`PhonemeSegment`）をオフライン生成する
pub fn align_text_to_phoneme_segments(
    text: &str,
    clip_start: RationalTime,
    clip_duration: RationalTime,
) -> Vec<PhonemeSegment> {
    let clean = strip_control_tags(text);
    let dur_sec = clip_duration.as_seconds().max(0.15);

    // 各文字を母音・ポーズ音素へマッピング（日本語仮名・漢字推定・英語母音対応）
    let mut tokens: Vec<(&'static str, MouthShape, f32)> = Vec::new();
    for ch in clean.chars() {
        if ch.is_whitespace()
            || matches!(ch, '、' | ',' | '。' | '.' | '!' | '！' | '?' | '？' | '…')
        {
            tokens.push(("sil", MouthShape::Closed, 0.75));
            continue;
        }
        let (ph, mouth, weight) = classify_char_vowel(ch);
        tokens.push((ph, mouth, weight));
    }

    if tokens.is_empty() {
        return vec![PhonemeSegment {
            start: clip_start,
            end: clip_start + clip_duration,
            phoneme: "sil".into(),
            mouth: MouthShape::Closed,
        }];
    }

    // 末尾に自然な口閉じポーズを付与
    if tokens.last().map(|t| t.1) != Some(MouthShape::Closed) {
        tokens.push(("sil", MouthShape::Closed, 0.5));
    }

    let total_w: f32 = tokens.iter().map(|t| t.2).sum::<f32>().max(1e-4);
    let mut cursor = 0.0f64;
    let mut out = Vec::with_capacity(tokens.len());

    for (ph, mouth, w) in tokens {
        let seg_dur = dur_sec * f64::from(w / total_w);
        let s = clip_start + RationalTime::from_seconds(cursor);
        let e = clip_start + RationalTime::from_seconds((cursor + seg_dur).min(dur_sec));
        cursor += seg_dur;
        if e > s {
            out.push(PhonemeSegment {
                start: s,
                end: e,
                phoneme: ph.to_string(),
                mouth,
            });
        }
    }
    out
}

fn classify_char_vowel(ch: char) -> (&'static str, MouthShape, f32) {
    match ch.to_ascii_lowercase() {
        'a' | 'あ' | 'ぁ' | 'か' | 'が' | 'さ' | 'ざ' | 'た' | 'だ' | 'な' | 'は' | 'ば' | 'ぱ'
        | 'ま' | 'や' | 'ゃ' | 'ら' | 'わ' | 'ア' | 'ァ' | 'カ' | 'ガ' | 'サ' | 'ザ' | 'タ'
        | 'ダ' | 'ナ' | 'ハ' | 'バ' | 'パ' | 'マ' | 'ヤ' | 'ャ' | 'ラ' | 'ワ' => {
            ("a", MouthShape::A, 1.15)
        }
        'i' | 'い' | 'ぃ' | 'き' | 'ぎ' | 'し' | 'じ' | 'ち' | 'ぢ' | 'に' | 'ひ' | 'び' | 'ぴ'
        | 'み' | 'り' | 'イ' | 'ィ' | 'キ' | 'ギ' | 'シ' | 'ジ' | 'チ' | 'ヂ' | 'ニ' | 'ヒ'
        | 'ビ' | 'ピ' | 'ミ' | 'リ' => ("i", MouthShape::I, 1.0),
        'u' | 'う' | 'ぅ' | 'く' | 'ぐ' | 'す' | 'ず' | 'つ' | 'づ' | 'ぬ' | 'ふ' | 'ぶ' | 'ぷ'
        | 'む' | 'ゆ' | 'ゅ' | 'る' | 'ウ' | 'ゥ' | 'ク' | 'グ' | 'ス' | 'ズ' | 'ツ' | 'ヅ'
        | 'ヌ' | 'フ' | 'ブ' | 'プ' | 'ム' | 'ユ' | 'ュ' | 'ル' => {
            ("u", MouthShape::U, 1.0)
        }
        'e' | 'え' | 'ぇ' | 'け' | 'げ' | 'せ' | 'ぜ' | 'て' | 'で' | 'ね' | 'へ' | 'べ' | 'ぺ'
        | 'め' | 'れ' | 'エ' | 'ェ' | 'ケ' | 'ゲ' | 'セ' | 'ゼ' | 'テ' | 'デ' | 'ネ' | 'ヘ'
        | 'ベ' | 'ペ' | 'メ' | 'レ' => ("e", MouthShape::E, 1.05),
        'o' | 'お' | 'ぉ' | 'こ' | 'ご' | 'そ' | 'ぞ' | 'と' | 'ど' | 'の' | 'ほ' | 'ぼ' | 'ぽ'
        | 'も' | 'よ' | 'ょ' | 'ろ' | 'を' | 'オ' | 'ォ' | 'コ' | 'ゴ' | 'ソ' | 'ゾ' | 'ト'
        | 'ド' | 'ノ' | 'ホ' | 'ボ' | 'ポ' | 'モ' | 'ヨ' | 'ョ' | 'ロ' | 'ヲ' => {
            ("o", MouthShape::O, 1.1)
        }
        'ん' | 'ン' | 'っ' | 'ッ' | 'm' | 'p' | 'b' => ("m", MouthShape::Closed, 0.7),
        'ー' => ("a", MouthShape::HalfOpen, 0.9),
        c if ('\u{4E00}'..='\u{9FFF}').contains(&c) => {
            // 漢字はコードポイントから決定論的に 5 母音へバランス配分（2 モーラ相当の重み）
            match (c as u32) % 5 {
                0 => ("a", MouthShape::A, 1.35),
                1 => ("i", MouthShape::I, 1.25),
                2 => ("u", MouthShape::U, 1.25),
                3 => ("e", MouthShape::E, 1.30),
                _ => ("o", MouthShape::O, 1.35),
            }
        }
        c if c.is_ascii_alphabetic() => ("c", MouthShape::HalfOpen, 0.65),
        _ => ("o", MouthShape::HalfOpen, 0.8),
    }
}

/// 音素区間列から 3 フォルマント共振音声合成（48kHz ステレオ PCM）をオフライン生成する
pub fn synthesize_offline_dubbing_pcm(
    phonemes: &[PhonemeSegment],
    clip_start: RationalTime,
    clip_duration: RationalTime,
    sample_rate: u32,
    voice: DubbingVoicePreset,
) -> Vec<f32> {
    let sr = sample_rate.max(8_000);
    let dur_sec = clip_duration.as_seconds().clamp(0.1, 60.0);
    let total_frames = ((dur_sec * sr as f64).round() as usize).max(1);
    let mut stereo = vec![0.0f32; total_frames * 2];

    let f0_base = voice.base_pitch_hz();
    let f_scale = voice.formant_scale();
    let mut glottal_phase = 0.0f32;

    // 3 並列 2 次共振バンドパスフィルタ状態 (F1, F2, F3)
    let mut bq = [[0.0f32; 2]; 3];

    // 音素検索用カーソル
    let mut seg_idx = 0usize;

    for (frame_i, frame) in stereo.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let local_sec = frame_i as f64 / sr as f64;
        let abs_t = clip_start + RationalTime::from_seconds(local_sec);
        while seg_idx + 1 < phonemes.len() && phonemes[seg_idx].end <= abs_t {
            seg_idx += 1;
        }
        let seg = phonemes.get(seg_idx);
        let (mouth, seg_env) = if let Some(s) = seg {
            let s_dur = (s.end - s.start).as_seconds().max(1e-4);
            let frac = ((abs_t - s.start).as_seconds() / s_dur).clamp(0.0, 1.0) as f32;
            // スムーズな音節エンベロープ（クリックノイズ防止）
            let env = (frac * 6.0).min(1.0) * ((1.0 - frac) * 6.0).min(1.0);
            (s.mouth, env)
        } else {
            (MouthShape::Closed, 0.0)
        };

        let voiced = mouth != MouthShape::Closed;
        // 自然なイントネーション（文頭やや高め → 文末下降＋微小ビブラート）
        let progress = (local_sec / dur_sec).clamp(0.0, 1.0) as f32;
        let vib = (2.0 * PI * 5.2 * local_sec as f32).sin() * 0.012;
        let f0 = f0_base * (1.08 - 0.22 * progress + vib);
        glottal_phase = (glottal_phase + f0 / sr as f32).fract();

        // 声帯パルス（帯域制限ノコギリ＋軟調丸み）
        let excitation = if voiced {
            let saw = 2.0 * glottal_phase - 1.0;
            let soft = (PI * glottal_phase).sin();
            (0.55 * saw + 0.45 * soft) * seg_env * 0.42
        } else {
            0.0
        };

        let (f1, f2, f3) = vowel_formants_hz(mouth);
        let freqs = [f1 * f_scale, f2 * f_scale, f3 * f_scale];
        let bws = [90.0f32, 130.0f32, 220.0f32];
        let gains = [1.0f32, 0.62f32, 0.28f32];

        let mut sample = 0.0f32;
        for (state, ((freq, bw), gain)) in bq.iter_mut().zip(freqs.into_iter().zip(bws).zip(gains))
        {
            let r = (-PI * bw / sr as f32).exp();
            let cos_w = (2.0 * PI * freq.min(sr as f32 * 0.44) / sr as f32).cos();
            let y = (1.0 - r) * excitation + 2.0 * r * cos_w * state[0] - r * r * state[1];
            state[1] = state[0];
            state[0] = y;
            sample += y * gain;
        }

        let out = (sample * 0.85).tanh() * 0.65;
        frame[0] = out;
        frame[1] = out;
    }

    stereo
}

fn vowel_formants_hz(mouth: MouthShape) -> (f32, f32, f32) {
    match mouth {
        MouthShape::A | MouthShape::Open => (730.0, 1180.0, 2600.0),
        MouthShape::I => (300.0, 2250.0, 3050.0),
        MouthShape::U => (340.0, 920.0, 2350.0),
        MouthShape::E => (500.0, 1850.0, 2650.0),
        MouthShape::O => (460.0, 860.0, 2450.0),
        MouthShape::HalfOpen => (540.0, 1450.0, 2550.0),
        MouthShape::Closed => (280.0, 1100.0, 2300.0),
    }
}

/// 生成済み吹替音声アセット群＋音素アライメントをシーンへ反映する 1-Undo コマンド列を構築する
pub struct GeneratedDubClip {
    pub subtitle_item_id: ItemId,
    pub translated_text: String,
    pub start: RationalTime,
    pub duration: RationalTime,
    pub audio_asset_id: AssetId,
    pub clip_name: String,
    pub phonemes: Vec<PhonemeSegment>,
}

/// 吹替字幕切替・音声クリップ配置・音素リップシンク同期を 1 ステップ Undo 可能なコマンド列へまとめる
pub fn build_dubbing_timeline_commands(
    scene: &Scene,
    audio_layer_index: usize,
    generated: &[GeneratedDubClip],
) -> Vec<EditCommand> {
    let mut cmds = Vec::new();
    let target_layer = audio_layer_index.min(scene.layers.len().saturating_sub(1));

    // 全吹替クリップの音素列を結合（立ち絵の母音連動口パク用）
    let mut combined_phonemes: Vec<PhonemeSegment> = Vec::new();
    for g in generated {
        cmds.push(EditCommand::SetTextContent {
            item_id: g.subtitle_item_id,
            content: g.translated_text.clone(),
        });
        let mut audio_item = TimelineItem::new(
            g.clip_name.clone(),
            g.start,
            g.duration,
            ItemKind::Audio {
                asset_id: g.audio_asset_id,
                source_offset: RationalTime::ZERO,
                volume: 1.0,
            },
        );
        audio_item.phoneme_segments = g.phonemes.clone();
        cmds.push(EditCommand::AddObject {
            layer_index: target_layer,
            item: audio_item,
        });
        combined_phonemes.extend(g.phonemes.clone());
    }

    combined_phonemes.sort_by_key(|p| p.start);

    // シーン内の全 CharacterSprite にも結合音素列を同期し、自動的に多言語リップシンク連動させる
    if !combined_phonemes.is_empty() {
        for layer in &scene.layers {
            for item in &layer.items {
                if matches!(item.kind, ItemKind::CharacterSprite { .. }) {
                    cmds.push(EditCommand::SetItemPhonemes {
                        item_id: item.id,
                        phonemes: combined_phonemes.clone(),
                    });
                }
            }
        }
    }

    cmds
}

/// 多言語ローカライズ表から指定言語の吹替対象行 `(item_id, text, start, duration)` を抽出する
pub fn collect_dubbing_targets_from_table(
    scene: &Scene,
    table: &LocalizationTable,
    target_locale: &str,
) -> Vec<(ItemId, String, RationalTime, RationalTime)> {
    let loc = target_locale.trim();
    let mut out = Vec::new();
    for entry in &table.entries {
        let Some(translated) = entry.translations.get(loc).filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        if let Some(item) = scene
            .layers
            .iter()
            .flat_map(|l| l.items.iter())
            .find(|i| i.id == entry.item_id)
        {
            out.push((item.id, translated.clone(), item.start, item.duration));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_japanese_and_english_text_to_timed_phonemes_and_synthesizes_audible_pcm() {
        let start = RationalTime::from_seconds(1.0);
        let dur = RationalTime::from_seconds(2.0);
        let ph_ja = align_text_to_phoneme_segments("こんにちは、世界！", start, dur);
        assert!(ph_ja.len() >= 5);
        assert!(ph_ja.iter().any(|p| p.mouth == MouthShape::O));
        assert!(ph_ja.iter().any(|p| p.mouth == MouthShape::A));

        let ph_en = align_text_to_phoneme_segments("Hello world, welcome!", start, dur);
        assert!(ph_en.len() >= 10);

        let pcm = synthesize_offline_dubbing_pcm(
            &ph_ja,
            start,
            dur,
            48_000,
            DubbingVoicePreset::JpFemale,
        );
        assert_eq!(pcm.len(), 48_000 * 2 * 2);
        let peak = pcm.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(
            peak > 0.05 && peak <= 1.0,
            "synthesized speech PCM must be audible and non-clipping, peak={peak}"
        );
    }
}
