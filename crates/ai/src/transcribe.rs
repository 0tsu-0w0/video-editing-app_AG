use crate::credentials::{ApiProvider, CredentialError, CredentialStore};
use crate::guard::{CloudUploadGuard, GuardError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;
use ve_core::{
    ItemKind, PhonemeSegment, RationalTime, RgbaColor, TextAnimator, TimelineItem, Transform2D,
};
use ve_media::AudioBuffer;

#[derive(Debug, Error)]
pub enum TranscribeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Credential error: {0}")]
    Credential(#[from] CredentialError),
    #[error("Upload guard blocked request: {0}")]
    Guard(#[from] GuardError),
    #[error("Transcription failed: {0}")]
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleSegment {
    pub start: RationalTime,
    pub end: RationalTime,
    pub text: String,
    #[serde(default)]
    pub phonemes: Vec<PhonemeSegment>,
}

/// 字幕セグメント列をタイムラインのテキストオブジェクト群へ変換する（CommandBus 投入用）
pub fn segments_to_subtitle_items(
    segments: &[SubtitleSegment],
    font_size: f32,
    y_position: f32,
) -> Vec<TimelineItem> {
    segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let dur = (seg.end - seg.start).max(RationalTime::from_seconds(0.1));
            let mut item = TimelineItem::new(
                format!("字幕 {}", i + 1),
                seg.start,
                dur,
                ItemKind::Text {
                    content: seg.text.clone(),
                    font_family: "sans-serif".to_string(),
                    font_size,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            );
            item.transform = Transform2D {
                x: 120.0,
                y: y_position,
                ..Default::default()
            };
            item.phoneme_segments = seg.phonemes.clone();
            item
        })
        .collect()
}

/// モード1: ローカル標準モード（whisper.cpp CLI / GPU推論対応＋オフラインVADフォールバック）
#[derive(Debug, Clone)]
pub struct LocalWhisperTranscriber {
    pub whisper_bin: PathBuf,
    pub model_path: Option<PathBuf>,
    pub language: String,
}

impl Default for LocalWhisperTranscriber {
    fn default() -> Self {
        Self {
            whisper_bin: PathBuf::from("whisper-cli"),
            model_path: None,
            language: "ja".to_string(),
        }
    }
}

impl LocalWhisperTranscriber {
    /// ローカル `whisper-cli`（whisper.cpp）を実行して SRT/JSON タイムスタンプ付き字幕を生成する
    pub fn transcribe_wav(&self, wav_path: &Path) -> Result<Vec<SubtitleSegment>, TranscribeError> {
        let model = self.model_path.as_ref().ok_or_else(|| {
            TranscribeError::Failed("Whisper モデルファイル (.bin) が未設定です".into())
        })?;
        let output = Command::new(&self.whisper_bin)
            .args(["-m"])
            .arg(model)
            .args(["-l", &self.language, "-osrt", "-f"])
            .arg(wav_path)
            .output()?;
        if !output.status.success() {
            return Err(TranscribeError::Failed(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let srt_path = PathBuf::from(format!("{}.srt", wav_path.display()));
        let srt_text = if srt_path.exists() {
            fs::read_to_string(&srt_path)?
        } else {
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        Ok(parse_srt_segments(&srt_text))
    }
}

/// SRT 形式文字列を `SubtitleSegment` 列へパースする
pub fn parse_srt_segments(srt: &str) -> Vec<SubtitleSegment> {
    let mut segments = Vec::new();
    for block in srt.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let Some(arrow_idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let parts: Vec<&str> = lines[arrow_idx].split("-->").collect();
        if parts.len() != 2 {
            continue;
        }
        if let (Some(start), Some(end)) = (
            parse_srt_timestamp(parts[0].trim()),
            parse_srt_timestamp(parts[1].trim()),
        ) {
            let text = lines[arrow_idx + 1..].join("\n");
            if !text.is_empty() {
                segments.push(SubtitleSegment {
                    start,
                    end,
                    text,
                    phonemes: Vec::new(),
                });
            }
        }
    }
    segments
}

fn parse_srt_timestamp(s: &str) -> Option<RationalTime> {
    let cleaned = s.replace(',', ".");
    let parts: Vec<&str> = cleaned.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    Some(RationalTime::from_seconds(h * 3600.0 + m * 60.0 + sec))
}

/// オフライン簡易エネルギー VAD（Whisper モデル未導入環境での発話区間自動検出用）
pub fn segment_audio_by_vad(
    buffer: &AudioBuffer,
    rms_threshold: f32,
    min_segment_sec: f64,
) -> Vec<SubtitleSegment> {
    if buffer.sample_rate == 0 || buffer.channels == 0 {
        return Vec::new();
    }
    let win_frames = (buffer.sample_rate as usize / 20).max(1); // 50ms 窓
    let total_frames = buffer.frame_count();
    let ch = buffer.channels;

    let mut segments = Vec::new();
    let mut active_start: Option<usize> = None;
    let mut idx = 0usize;

    while idx < total_frames {
        let end = (idx + win_frames).min(total_frames);
        let mut sum_sq = 0.0f64;
        for f in idx..end {
            let mut mono = 0.0f32;
            for c in 0..ch {
                mono += buffer.samples[f * ch + c];
            }
            mono /= ch as f32;
            sum_sq += f64::from(mono) * f64::from(mono);
        }
        let rms = (sum_sq / (end - idx).max(1) as f64).sqrt() as f32;

        if rms >= rms_threshold {
            if active_start.is_none() {
                active_start = Some(idx);
            }
        } else if let Some(s) = active_start.take() {
            let start_sec = s as f64 / buffer.sample_rate as f64;
            let end_sec = idx as f64 / buffer.sample_rate as f64;
            if end_sec - start_sec >= min_segment_sec {
                segments.push(SubtitleSegment {
                    start: RationalTime::from_seconds(start_sec),
                    end: RationalTime::from_seconds(end_sec),
                    text: format!("（発話区間 {}）", segments.len() + 1),
                    phonemes: Vec::new(),
                });
            }
        }
        idx = end;
    }

    if let Some(s) = active_start {
        let start_sec = s as f64 / buffer.sample_rate as f64;
        let end_sec = total_frames as f64 / buffer.sample_rate as f64;
        if end_sec - start_sec >= min_segment_sec {
            segments.push(SubtitleSegment {
                start: RationalTime::from_seconds(start_sec),
                end: RationalTime::from_seconds(end_sec),
                text: format!("（発話区間 {}）", segments.len() + 1),
                phonemes: Vec::new(),
            });
        }
    }

    segments
}

/// モード2A: Gemini API（音声ネイティブ解析による高精度書き起こし・タイムスタンプ抽出）
pub struct GeminiTranscriber {
    pub model: String,
    pub endpoint_base: String,
    pub guard: CloudUploadGuard,
}

impl Default for GeminiTranscriber {
    fn default() -> Self {
        Self {
            model: "gemini-2.5-flash".to_string(),
            endpoint_base: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            guard: CloudUploadGuard::default(),
        }
    }
}

impl GeminiTranscriber {
    pub fn transcribe_audio_bytes(
        &self,
        store: &dyn CredentialStore,
        audio_bytes: &[u8],
        mime_type: &str,
        duration_sec: f64,
        user_confirmed: bool,
    ) -> Result<Vec<SubtitleSegment>, TranscribeError> {
        self.guard
            .verify_before_send(ApiProvider::Gemini, duration_sec, user_confirmed)?;
        let api_key = store.get_api_key(ApiProvider::Gemini)?;

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.endpoint_base, self.model, api_key
        );
        let b64 = base64_encode(audio_bytes);
        let body = json!({
            "contents": [{
                "parts": [
                    {
                        "text": "音声を日本語で正確に書き起こし、JSON配列 [{\"start_sec\": 0.0, \"end_sec\": 1.5, \"text\": \"...\"}] のみで返してください。"
                    },
                    {
                        "inline_data": {
                            "mime_type": mime_type,
                            "data": b64
                        }
                    }
                ]
            }],
            "generationConfig": {
                "responseMimeType": "application/json"
            }
        });

        let client = reqwest::blocking::Client::new();
        let resp: Value = client
            .post(url)
            .json(&body)
            .send()?
            .error_for_status()?
            .json()?;
        let text = resp["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| TranscribeError::Failed("Gemini response missing text".into()))?;
        parse_llm_segments_json(text)
    }
}

/// モード2B: Claude API（文脈補正・誤字修正・読みやすい字幕分割）
pub struct ClaudeRefiner {
    pub model: String,
    pub endpoint_url: String,
    pub guard: CloudUploadGuard,
}

impl Default for ClaudeRefiner {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            endpoint_url: "https://api.anthropic.com/v1/messages".to_string(),
            guard: CloudUploadGuard::default(),
        }
    }
}

impl ClaudeRefiner {
    pub fn refine_subtitles(
        &self,
        store: &dyn CredentialStore,
        segments: &[SubtitleSegment],
        duration_sec: f64,
        user_confirmed: bool,
    ) -> Result<Vec<SubtitleSegment>, TranscribeError> {
        self.guard
            .verify_before_send(ApiProvider::Claude, duration_sec, user_confirmed)?;
        let api_key = store.get_api_key(ApiProvider::Claude)?;

        let raw_items: Vec<Value> = segments
            .iter()
            .map(|s| {
                json!({
                    "start_sec": s.start.as_seconds(),
                    "end_sec": s.end.as_seconds(),
                    "text": s.text,
                })
            })
            .collect();

        let prompt = format!(
            "以下の字幕JSONの誤字修正・文脈補正を行い、1行18文字以内の読みやすい字幕に整形して同じJSON配列形式のみで返してください:\n{}",
            serde_json::to_string(&raw_items)?
        );

        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": prompt }]
        });

        let client = reqwest::blocking::Client::new();
        let resp: Value = client
            .post(&self.endpoint_url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()?
            .error_for_status()?
            .json()?;

        let text = resp["content"][0]["text"]
            .as_str()
            .ok_or_else(|| TranscribeError::Failed("Claude response missing text".into()))?;
        parse_llm_segments_json(text)
    }
}

#[derive(Deserialize)]
struct RawLlmSegment {
    start_sec: f64,
    end_sec: f64,
    text: String,
}

pub fn parse_llm_segments_json(raw: &str) -> Result<Vec<SubtitleSegment>, TranscribeError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: Vec<RawLlmSegment> = serde_json::from_str(trimmed)?;
    Ok(parsed
        .into_iter()
        .map(|s| SubtitleSegment {
            start: RationalTime::from_seconds(s.start_sec.max(0.0)),
            end: RationalTime::from_seconds(s.end_sec.max(s.start_sec)),
            text: s.text,
            phonemes: Vec::new(),
        })
        .collect())
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::InMemoryCredentialStore;

    #[test]
    fn parses_srt_and_llm_json_subtitles() {
        let srt = "1\n00:00:00,500 --> 00:00:02,000\nこんにちは\n\n2\n00:00:02,500 --> 00:00:04,000\n動画編集テスト\n";
        let segs = parse_srt_segments(srt);
        assert_eq!(segs.len(), 2);
        assert!((segs[0].start.as_seconds() - 0.5).abs() < 1e-3);
        assert_eq!(segs[1].text, "動画編集テスト");

        let llm_json =
            "```json\n[{\"start_sec\": 0.0, \"end_sec\": 1.2, \"text\": \"補正済み字幕\"}]\n```";
        let parsed = parse_llm_segments_json(llm_json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].text, "補正済み字幕");
    }

    #[test]
    fn vad_detects_active_speech_interval() {
        let sr = 16_000u32;
        let mut samples = vec![0.0f32; sr as usize * 2];
        // 0.5s .. 1.2s に音声サイン波を配置
        for (i, s) in samples
            .iter_mut()
            .enumerate()
            .take((sr as f32 * 1.2) as usize)
            .skip((sr as f32 * 0.5) as usize)
        {
            *s = (2.0 * std::f32::consts::PI * 300.0 * i as f32 / sr as f32).sin() * 0.6;
        }
        let buf = AudioBuffer {
            sample_rate: sr,
            channels: 1,
            samples,
        };
        let segs = segment_audio_by_vad(&buf, 0.1, 0.2);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start.as_seconds() - 0.5).abs() < 0.1);
    }

    #[test]
    fn gemini_refuses_send_without_explicit_confirmation() {
        let store = InMemoryCredentialStore::new();
        store.set_api_key(ApiProvider::Gemini, "dummy-key").unwrap();
        let gemini = GeminiTranscriber::default();
        let err = gemini
            .transcribe_audio_bytes(&store, b"RIFF", "audio/wav", 30.0, false)
            .unwrap_err();
        assert!(matches!(err, TranscribeError::Guard(_)));
    }
}
