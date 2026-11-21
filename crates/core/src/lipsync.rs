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

/// Phase 4: PSD ネイティブ立ち絵レイヤー切り替え設定（目パチ・口パク差分）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PsdCharacterConfig {
    pub psd_path: PathBuf,
    /// 体・髪・服など常時表示するベースレイヤー名（空の場合は目/口差分以外の可視レイヤーを使用）
    #[serde(default)]
    pub base_layers: Vec<String>,
    /// 口形状と PSD 内レイヤー名の対応表（例: Closed -> "口_閉じ", A -> "口_あ"）
    #[serde(default)]
    pub mouth_layers: BTreeMap<MouthShape, String>,
    /// 開き目レイヤー名（例: "目_開き"）
    #[serde(default)]
    pub eye_open_layer: Option<String>,
    /// 閉じ目レイヤー名（例: "目_閉じ"）
    #[serde(default)]
    pub eye_closed_layer: Option<String>,
    /// 自動目パチの周期（秒、0以下で無効）
    #[serde(default = "default_blink_interval")]
    pub blink_interval_sec: f32,
    /// 自動目パチの閉じ時間（秒）
    #[serde(default = "default_blink_duration")]
    pub blink_duration_sec: f32,
}

fn default_blink_interval() -> f32 {
    3.5
}

fn default_blink_duration() -> f32 {
    0.14
}

impl Default for PsdCharacterConfig {
    fn default() -> Self {
        Self {
            psd_path: PathBuf::new(),
            base_layers: Vec::new(),
            mouth_layers: BTreeMap::new(),
            eye_open_layer: None,
            eye_closed_layer: None,
            blink_interval_sec: default_blink_interval(),
            blink_duration_sec: default_blink_duration(),
        }
    }
}

impl PsdCharacterConfig {
    /// 指定時刻に目パチ（閉じ目）状態かどうかを決定論的に評価する
    pub fn is_eye_closed_at(&self, time_sec: f64) -> bool {
        if self.blink_interval_sec <= 0.0 || self.blink_duration_sec <= 0.0 {
            return false;
        }
        let interval = f64::from(self.blink_interval_sec);
        let dur = f64::from(self.blink_duration_sec).min(interval * 0.5);
        let phase = time_sec.rem_euclid(interval);
        phase >= (interval - dur)
    }
}

/// Phase 3/4 立ち絵オブジェクトの設定（PNG差分方式 / PSDネイティブ対応）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CharacterSpriteConfig {
    /// 連動する音声レイヤーのインデックス（選択するだけで即動作）
    pub linked_audio_layer: Option<usize>,
    pub mode: LipSyncMode,
    /// Phase 3: 口形状ごとの PNG 差分ファイルパス
    pub png_paths: BTreeMap<MouthShape, PathBuf>,
    /// Phase 4: PSD ネイティブ立ち絵設定（設定時は PNG より優先してレイヤー直接合成）
    #[serde(default)]
    pub psd_config: Option<PsdCharacterConfig>,
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
