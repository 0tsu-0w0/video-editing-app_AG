use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use ve_core::{PhonemeSegment, RationalTime, phoneme_to_mouth};
use ve_media::decode_audio_file;

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Audio decode error: {0}")]
    Media(#[from] ve_media::MediaError),
    #[error("TTS error: {0}")]
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsEngine {
    Voicevox,
    Coeiroink,
    AiVoice,
}

impl TtsEngine {
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Voicevox => "http://127.0.0.1:50021",
            Self::Coeiroink => "http://127.0.0.1:50032",
            Self::AiVoice => "http://127.0.0.1:50080",
        }
    }
}

/// ファイルペア自動取込またはローカルTTS APIから得られる一括配置バンドル
#[derive(Debug, Clone)]
pub struct TtsImportBundle {
    pub wav_path: Option<PathBuf>,
    pub wav_bytes: Option<Vec<u8>>,
    pub duration: RationalTime,
    pub subtitle_text: String,
    pub phonemes: Vec<PhonemeSegment>,
}

/// モード3A: ファイルペア自動取込（`voice.wav` と同名の `.txt` / `.lab` を一括解析）
pub fn import_tts_file_pair(wav_path: &Path) -> Result<TtsImportBundle, TtsError> {
    let audio = decode_audio_file(wav_path)?;
    let duration = RationalTime::from_seconds(audio.duration_sec().max(0.1));

    let txt_path = wav_path.with_extension("txt");
    let subtitle_text = if txt_path.exists() {
        fs::read_to_string(&txt_path)?.trim().to_string()
    } else {
        wav_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Voice".to_string())
    };

    let lab_path = wav_path.with_extension("lab");
    let phonemes = if lab_path.exists() {
        let lab_text = fs::read_to_string(&lab_path)?;
        parse_lab_phonemes(&lab_text)
    } else {
        Vec::new()
    };

    Ok(TtsImportBundle {
        wav_path: Some(wav_path.to_path_buf()),
        wav_bytes: None,
        duration,
        subtitle_text,
        phonemes,
    })
}

/// HTS / OpenJTalk / CeVIO / VOICEVOX 形式の音素ラベル (`.lab`) をパースする。
/// 100ナノ秒整数形式（`0 1200000 a`）と浮動小数点秒形式（`0.00 0.12 a`）の両方に対応。
pub fn parse_lab_phonemes(content: &str) -> Vec<PhonemeSegment> {
    let mut segments = Vec::new();
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let (start_sec, end_sec) = if parts[0].contains('.') || parts[1].contains('.') {
            let Ok(s) = parts[0].parse::<f64>() else {
                continue;
            };
            let Ok(e) = parts[1].parse::<f64>() else {
                continue;
            };
            (s, e)
        } else {
            let Ok(s_100ns) = parts[0].parse::<i64>() else {
                continue;
            };
            let Ok(e_100ns) = parts[1].parse::<i64>() else {
                continue;
            };
            (s_100ns as f64 / 10_000_000.0, e_100ns as f64 / 10_000_000.0)
        };
        if end_sec <= start_sec {
            continue;
        }
        let phoneme = parts[2].to_string();
        let mouth = phoneme_to_mouth(&phoneme);
        segments.push(PhonemeSegment {
            start: RationalTime::from_seconds(start_sec.max(0.0)),
            end: RationalTime::from_seconds(end_sec.max(0.0)),
            phoneme,
            mouth,
        });
    }
    segments
}

/// VOICEVOX `/audio_query` レスポンス JSON から母音タイミング列（`PhonemeSegment`）を直接抽出する
pub fn parse_voicevox_audio_query(query: &Value) -> (RationalTime, Vec<PhonemeSegment>) {
    let speed = query
        .get("speedScale")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .max(0.1);
    let pre = query
        .get("prePhonemeLength")
        .and_then(Value::as_f64)
        .unwrap_or(0.1);
    let post = query
        .get("postPhonemeLength")
        .and_then(Value::as_f64)
        .unwrap_or(0.1);

    let mut cursor = pre / speed;
    let mut segments = Vec::new();

    if let Some(phrases) = query.get("accent_phrases").and_then(Value::as_array) {
        for phrase in phrases {
            if let Some(moras) = phrase.get("moras").and_then(Value::as_array) {
                for mora in moras {
                    if let Some(c_len) = mora.get("consonant_length").and_then(Value::as_f64) {
                        let c_dur = c_len / speed;
                        let c_sym = mora.get("consonant").and_then(Value::as_str).unwrap_or("c");
                        if c_dur > 0.0 {
                            segments.push(PhonemeSegment {
                                start: RationalTime::from_seconds(cursor),
                                end: RationalTime::from_seconds(cursor + c_dur),
                                phoneme: c_sym.to_string(),
                                mouth: phoneme_to_mouth(c_sym),
                            });
                            cursor += c_dur;
                        }
                    }
                    if let Some(v_len) = mora.get("vowel_length").and_then(Value::as_f64) {
                        let v_dur = v_len / speed;
                        let v_sym = mora.get("vowel").and_then(Value::as_str).unwrap_or("a");
                        if v_dur > 0.0 {
                            segments.push(PhonemeSegment {
                                start: RationalTime::from_seconds(cursor),
                                end: RationalTime::from_seconds(cursor + v_dur),
                                phoneme: v_sym.to_string(),
                                mouth: phoneme_to_mouth(v_sym),
                            });
                            cursor += v_dur;
                        }
                    }
                }
            }
            if let Some(pause) = phrase.get("pause_mora").and_then(Value::as_object)
                && let Some(p_len) = pause.get("vowel_length").and_then(Value::as_f64)
            {
                let p_dur = p_len / speed;
                if p_dur > 0.0 {
                    segments.push(PhonemeSegment {
                        start: RationalTime::from_seconds(cursor),
                        end: RationalTime::from_seconds(cursor + p_dur),
                        phoneme: "pau".to_string(),
                        mouth: phoneme_to_mouth("pau"),
                    });
                    cursor += p_dur;
                }
            }
        }
    }

    let total = RationalTime::from_seconds((cursor + post / speed).max(0.1));
    (total, segments)
}

/// モード3B: VOICEVOX / COEIROINK / A.I.VOICE ローカルAPIクライアント
#[derive(Debug, Clone)]
pub struct LocalTtsClient {
    pub engine: TtsEngine,
    pub base_url: String,
    pub speaker_id: u32,
}

impl LocalTtsClient {
    pub fn new(engine: TtsEngine, speaker_id: u32) -> Self {
        Self {
            engine,
            base_url: engine.default_base_url().to_string(),
            speaker_id,
        }
    }

    /// テキストから音声WAV・字幕・母音タイミングを1アクションで生成する（VOICEVOX互換 API）
    pub fn synthesize(&self, text: &str) -> Result<TtsImportBundle, TtsError> {
        let client = reqwest::blocking::Client::new();
        let query_url = format!("{}/audio_query", self.base_url.trim_end_matches('/'));
        let query: Value = client
            .post(&query_url)
            .query(&[("text", text), ("speaker", &self.speaker_id.to_string())])
            .send()?
            .error_for_status()?
            .json()?;

        let (duration, phonemes) = parse_voicevox_audio_query(&query);

        let synth_url = format!("{}/synthesis", self.base_url.trim_end_matches('/'));
        let wav_bytes = client
            .post(&synth_url)
            .query(&[("speaker", &self.speaker_id.to_string())])
            .json(&query)
            .send()?
            .error_for_status()?
            .bytes()?
            .to_vec();

        Ok(TtsImportBundle {
            wav_path: None,
            wav_bytes: Some(wav_bytes),
            duration,
            subtitle_text: text.to_string(),
            phonemes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;
    use ve_core::MouthShape;
    use ve_media::write_wav_pcm16;

    #[test]
    fn imports_wav_txt_lab_file_pair_automatically() {
        let tmp = tempdir().unwrap();
        let wav_path = tmp.path().join("voice_01.wav");
        let txt_path = tmp.path().join("voice_01.txt");
        let lab_path = tmp.path().join("voice_01.lab");

        write_wav_pcm16(&wav_path, &vec![0.0f32; 48_000], 48_000, 1).unwrap();
        fs::write(&txt_path, "こんにちは！").unwrap();
        // HTS 100ns 形式
        fs::write(
            &lab_path,
            "0 1000000 sil\n1000000 3000000 o\n3000000 5000000 i\n5000000 8000000 a\n",
        )
        .unwrap();

        let bundle = import_tts_file_pair(&wav_path).unwrap();
        assert_eq!(bundle.subtitle_text, "こんにちは！");
        assert_eq!(bundle.phonemes.len(), 4);
        assert_eq!(bundle.phonemes[0].mouth, MouthShape::Closed);
        assert_eq!(bundle.phonemes[1].mouth, MouthShape::O);
        assert_eq!(bundle.phonemes[2].mouth, MouthShape::I);
        assert_eq!(bundle.phonemes[3].mouth, MouthShape::A);
    }

    #[test]
    fn extracts_vowel_timings_from_voicevox_audio_query() {
        let query = json!({
            "speedScale": 1.0,
            "prePhonemeLength": 0.1,
            "postPhonemeLength": 0.1,
            "accent_phrases": [{
                "moras": [
                    { "consonant": "k", "consonant_length": 0.05, "vowel": "o", "vowel_length": 0.15 },
                    { "consonant": "n", "consonant_length": 0.04, "vowel": "a", "vowel_length": 0.20 }
                ],
                "pause_mora": { "vowel": "pau", "vowel_length": 0.1 }
            }]
        });
        let (dur, phonemes) = parse_voicevox_audio_query(&query);
        assert!(dur.as_seconds() > 0.7);
        assert_eq!(phonemes.len(), 5);
        assert_eq!(phonemes[1].mouth, MouthShape::O);
        assert_eq!(phonemes[3].mouth, MouthShape::A);
        assert_eq!(phonemes[4].mouth, MouthShape::Closed);
    }
}
