use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 立ち絵の口形状（3段階PNG差分 or 5母音PNG差分に対応）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouthShape {
    Closed,
    HalfOpen,
    Open,
    A,
    I,
    U,
    E,
    O,
}

/// 立ち絵自動口パクの連動方式（仕様書 2.3 B）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LipSyncMode {
    /// 指定した音声レイヤーのRMS音量エンベロープと連動
    VolumeRms {
        closed_threshold: f32,
        open_threshold: f32,
    },
    /// 文字起こし結果やTTS音素情報と連動し、発話母音に応じた口形状を自動選択
    VowelPhoneme,
}

impl Default for LipSyncMode {
    fn default() -> Self {
        Self::VolumeRms {
            closed_threshold: 0.04,
            open_threshold: 0.18,
        }
    }
}

/// TTS (.lab / VOICEVOX) や文字起こし由来の音素タイミング区間
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhonemeSegment {
    pub start: RationalTime,
    pub end: RationalTime,
    pub phoneme: String,
    pub mouth: MouthShape,
}

/// Phase 3 PNG差分立ち絵オブジェクトの設定（連動音声レイヤー選択のみの設定レス設計）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CharacterSpriteConfig {
    /// 連動する音声レイヤーのインデックス（選択するだけで即動作）
    pub linked_audio_layer: Option<usize>,
    pub mode: LipSyncMode,
    /// 口形状ごとの PNG 差分ファイルパス
    pub png_paths: BTreeMap<MouthShape, PathBuf>,
    /// 立ち絵側へ直接紐づけられた音素トラック（未指定時は連動レイヤーの音声アイテムから取得）
    #[serde(default)]
    pub phoneme_override: Vec<PhonemeSegment>,
}

/// 日本語音素記号（HTS .lab / VOICEVOX / OpenJTalk）を口形状へマッピングする
pub fn phoneme_to_mouth(phoneme: &str) -> MouthShape {
    let p = phoneme.trim().to_ascii_lowercase();
    match p.as_str() {
        "" | "sil" | "pau" | "sp" | "cl" | "q" | "_" => MouthShape::Closed,
        "a" | "aa" => MouthShape::A,
        "i" | "ii" | "y" => MouthShape::I,
        "u" | "uu" | "w" => MouthShape::U,
        "e" | "ee" => MouthShape::E,
        "o" | "oo" => MouthShape::O,
        "n" | "m" | "b" | "p" => MouthShape::Closed,
        _ => MouthShape::HalfOpen,
    }
}

/// RMS 音量エンベロープを閾値に応じて口形状へマッピングする
pub fn rms_to_mouth(rms: f32, closed_threshold: f32, open_threshold: f32) -> MouthShape {
    if rms < closed_threshold {
        MouthShape::Closed
    } else if rms < open_threshold {
        MouthShape::HalfOpen
    } else {
        MouthShape::Open
    }
}

/// 指定時刻の音素区間から母音口形状を評価する
pub fn evaluate_vowel_mouth(segments: &[PhonemeSegment], time: RationalTime) -> MouthShape {
    for seg in segments {
        if time >= seg.start && time < seg.end {
            return seg.mouth;
        }
    }
    MouthShape::Closed
}

/// 指定された口形状に対応する PNG パスを解決する（母音PNGが無い場合は Open/HalfOpen/Closed へフォールバック）
pub fn resolve_mouth_png(
    png_paths: &BTreeMap<MouthShape, PathBuf>,
    desired: MouthShape,
) -> Option<&PathBuf> {
    if let Some(p) = png_paths.get(&desired) {
        return Some(p);
    }
    let fallback_order: &[MouthShape] = match desired {
        MouthShape::A | MouthShape::O => {
            &[MouthShape::Open, MouthShape::HalfOpen, MouthShape::Closed]
        }
        MouthShape::I | MouthShape::U | MouthShape::E => {
            &[MouthShape::HalfOpen, MouthShape::Open, MouthShape::Closed]
        }
        MouthShape::Open => &[MouthShape::A, MouthShape::HalfOpen, MouthShape::Closed],
        MouthShape::HalfOpen => &[MouthShape::E, MouthShape::Open, MouthShape::Closed],
        MouthShape::Closed => &[MouthShape::HalfOpen],
    };
    for &fb in fallback_order {
        if let Some(p) = png_paths.get(&fb) {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_phonemes_and_rms_to_mouth_shapes() {
        assert_eq!(phoneme_to_mouth("a"), MouthShape::A);
        assert_eq!(phoneme_to_mouth("I"), MouthShape::I);
        assert_eq!(phoneme_to_mouth("pau"), MouthShape::Closed);
        assert_eq!(phoneme_to_mouth("k"), MouthShape::HalfOpen);

        assert_eq!(rms_to_mouth(0.01, 0.04, 0.18), MouthShape::Closed);
        assert_eq!(rms_to_mouth(0.10, 0.04, 0.18), MouthShape::HalfOpen);
        assert_eq!(rms_to_mouth(0.30, 0.04, 0.18), MouthShape::Open);
    }

    #[test]
    fn vowel_fallback_uses_open_or_half_open_png() {
        let mut paths = BTreeMap::new();
        paths.insert(MouthShape::Closed, PathBuf::from("closed.png"));
        paths.insert(MouthShape::HalfOpen, PathBuf::from("half.png"));
        paths.insert(MouthShape::Open, PathBuf::from("open.png"));

        assert_eq!(
            resolve_mouth_png(&paths, MouthShape::A),
            Some(&PathBuf::from("open.png"))
        );
        assert_eq!(
            resolve_mouth_png(&paths, MouthShape::I),
            Some(&PathBuf::from("half.png"))
        );
    }
}
