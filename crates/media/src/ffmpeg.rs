use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use thiserror::Error;
use ve_core::FrameRate;

/// 動画フレーム1辺の安全上限（8K解像度）
pub const MAX_VIDEO_DIMENSION: u32 = 8192;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("FFmpeg/FFprobe execution error: {0}")]
    Process(#[from] std::io::Error),
    #[error("FFprobe JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("FFmpeg exited with non-zero status: {0}")]
    NonZeroExit(String),
    #[error("Audio decode error: {0}")]
    AudioDecode(String),
    #[error("Invalid media argument: {0}")]
    InvalidArgument(String),
}

/// `-` で始まる相対パスが FFmpeg/FFprobe のCLIオプションとして誤解釈される引数インジェクションを防ぐ
fn sanitize_ffmpeg_path(path: &Path) -> PathBuf {
    if path.is_relative()
        && let Some(s) = path.to_str()
        && s.starts_with('-')
    {
        return Path::new(".").join(path);
    }
    path.to_path_buf()
}

fn checked_rgba_frame_bytes(width: u32, height: u32) -> Result<usize, MediaError> {
    if width == 0 || height == 0 || width > MAX_VIDEO_DIMENSION || height > MAX_VIDEO_DIMENSION {
        return Err(MediaError::InvalidArgument(format!(
            "dimensions {width}x{height} out of valid range 1..={MAX_VIDEO_DIMENSION}"
        )));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| MediaError::InvalidArgument("frame buffer size overflow".into()))
}

#[derive(Debug, Clone)]
pub struct MediaProbe {
    pub duration_sec: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<FrameRate>,
    pub audio_sample_rate: Option<u32>,
    pub audio_channels: Option<u16>,
    pub color_space: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Mp4H264,
    Mp4H265,
    WebmVp9Alpha,
    PngSequence,
    WavAudioOnly,
}

/// ハードウェア／ソフトウェアエンコーダー選択（NVENC / QSV / AMF 自動検出＋CPUフォールバック）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VideoEncoder {
    #[default]
    Auto,
    Software,
    Nvenc,
    Qsv,
    Amf,
}

impl VideoEncoder {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "自動 (HW優先)",
            Self::Software => "Software (CPU)",
            Self::Nvenc => "NVIDIA NVENC",
            Self::Qsv => "Intel QSV",
            Self::Amf => "AMD AMF",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub format: ExportFormat,
    pub crf: u8,
    pub encoder: VideoEncoder,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: FrameRate::FPS_60,
            format: ExportFormat::Mp4H264,
            crf: 18,
            encoder: VideoEncoder::Auto,
        }
    }
}

/// FFmpeg / FFprobe 外部CLI（サイドカー）ラッパー
#[derive(Debug, Clone)]
pub struct FfmpegCli {
    pub ffmpeg_bin: PathBuf,
    pub ffprobe_bin: PathBuf,
}

impl Default for FfmpegCli {
    fn default() -> Self {
        Self {
            ffmpeg_bin: PathBuf::from("ffmpeg"),
            ffprobe_bin: PathBuf::from("ffprobe"),
        }
    }
}

#[derive(Deserialize)]
struct FfprobeOutput {
    format: Option<FfprobeFormat>,
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u16>,
    color_space: Option<String>,
}

fn parse_rational_fps(s: &str) -> Option<FrameRate> {
    let (num_s, den_s) = s.split_once('/')?;
    let num: u32 = num_s.parse().ok()?;
    let den: u32 = den_s.parse().ok()?;
    if num > 0 && den > 0 {
        Some(FrameRate::new(num, den))
    } else {
        None
    }
}

impl FfmpegCli {
    pub fn new(ffmpeg_bin: impl Into<PathBuf>, ffprobe_bin: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg_bin: ffmpeg_bin.into(),
            ffprobe_bin: ffprobe_bin.into(),
        }
    }

    pub fn probe(&self, path: &Path) -> Result<MediaProbe, MediaError> {
        let safe_path = sanitize_ffmpeg_path(path);
        let output = Command::new(&self.ffprobe_bin)
            .args([
                "-v",
                "quiet",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(&safe_path)
            .output()?;

        if !output.status.success() {
            return Err(MediaError::NonZeroExit(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout)?;
        let duration_sec = parsed
            .format
            .and_then(|f| f.duration)
            .and_then(|d| d.parse::<f64>().ok())
            .unwrap_or(0.0);

        let mut width = None;
        let mut height = None;
        let mut fps = None;
        let mut audio_sample_rate = None;
        let mut audio_channels = None;
        let mut color_space = None;

        for stream in parsed.streams {
            match stream.codec_type.as_deref() {
                Some("video") if width.is_none() => {
                    width = stream.width;
                    height = stream.height;
                    fps = stream.r_frame_rate.as_deref().and_then(parse_rational_fps);
                    color_space = stream.color_space;
                }
                Some("audio") if audio_sample_rate.is_none() => {
                    audio_sample_rate = stream.sample_rate.and_then(|s| s.parse().ok());
                    audio_channels = stream.channels;
                }
                _ => {}
            }
        }

        Ok(MediaProbe {
            duration_sec,
            width,
            height,
            fps,
            audio_sample_rate,
            audio_channels,
            color_space,
        })
    }

    /// 指定時刻の1フレームを RGBA8（sRGB）としてデコードする（Phase 1a PoC用）
    pub fn decode_frame_rgba8(
        &self,
        path: &Path,
        time_sec: f64,
        target_width: u32,
        target_height: u32,
    ) -> Result<Vec<u8>, MediaError> {
        if !time_sec.is_finite() || time_sec < 0.0 {
            return Err(MediaError::InvalidArgument(format!(
                "invalid decode timestamp: {time_sec}"
            )));
        }
        let expected = checked_rgba_frame_bytes(target_width, target_height)?;
        let safe_path = sanitize_ffmpeg_path(path);

        let output = Command::new(&self.ffmpeg_bin)
            .args(["-v", "error", "-ss", &format!("{time_sec:.6}"), "-i"])
            .arg(&safe_path)
            .args([
                "-frames:v",
                "1",
                "-vf",
                &format!("scale={target_width}:{target_height}"),
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .output()?;

        if !output.status.success() {
            return Err(MediaError::NonZeroExit(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        if output.stdout.len() < expected {
            return Err(MediaError::NonZeroExit(format!(
                "short frame read: got {} bytes, expected {expected}",
                output.stdout.len()
            )));
        }
        Ok(output.stdout[..expected].to_vec())
    }

    /// 静止画ファイル（JPEG / WebP / BMP / PNG 等）をアスペクト比維持で RGBA8 デコードする
    pub fn decode_image_rgba8(
        &self,
        path: &Path,
        max_width: u32,
        max_height: u32,
    ) -> Result<(u32, u32, Vec<u8>), MediaError> {
        let max_w = max_width.clamp(1, MAX_VIDEO_DIMENSION);
        let max_h = max_height.clamp(1, MAX_VIDEO_DIMENSION);
        let probe = self.probe(path)?;
        let nat_w = probe.width.unwrap_or(max_w).clamp(1, MAX_VIDEO_DIMENSION);
        let nat_h = probe.height.unwrap_or(max_h).clamp(1, MAX_VIDEO_DIMENSION);
        let scale = (max_w as f64 / nat_w as f64)
            .min(max_h as f64 / nat_h as f64)
            .min(1.0);
        let out_w = ((nat_w as f64 * scale).round() as u32).clamp(1, max_w);
        let out_h = ((nat_h as f64 * scale).round() as u32).clamp(1, max_h);
        let rgba = self.decode_frame_rgba8(path, 0.0, out_w, out_h)?;
        Ok((out_w, out_h, rgba))
    }

    /// 高速シーク用の軽量 H.264 プロキシ動画を `cache/proxy/` へ生成する（短GOP 15フレーム）
    pub fn generate_proxy(
        &self,
        src_path: &Path,
        proxy_path: &Path,
        max_height: u32,
    ) -> Result<PathBuf, MediaError> {
        if !(64..=2160).contains(&max_height) {
            return Err(MediaError::InvalidArgument(format!(
                "proxy max_height {max_height} out of valid range 64..=2160"
            )));
        }
        if let Some(parent) = proxy_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let safe_src = sanitize_ffmpeg_path(src_path);
        let safe_dst = sanitize_ffmpeg_path(proxy_path);
        let scale_filter = format!("scale=-2:'min(ih,{max_height})'");
        let output = Command::new(&self.ffmpeg_bin)
            .args(["-y", "-v", "error", "-i"])
            .arg(&safe_src)
            .args([
                "-vf",
                &scale_filter,
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "24",
                "-g",
                "15",
                "-pix_fmt",
                "yuv420p",
                "-an",
            ])
            .arg(&safe_dst)
            .output()?;
        if !output.status.success() {
            return Err(MediaError::NonZeroExit(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(proxy_path.to_path_buf())
    }

    /// 動画・音声コンテナから 16-bit PCM ステレオ WAV を抽出する（AudioGraph・波形生成フォールバック用）
    pub fn extract_audio_wav(
        &self,
        src_path: &Path,
        out_wav: &Path,
        sample_rate: u32,
    ) -> Result<(), MediaError> {
        if sample_rate == 0 || sample_rate > 384_000 {
            return Err(MediaError::InvalidArgument(format!(
                "invalid sample_rate: {sample_rate}"
            )));
        }
        let safe_src = sanitize_ffmpeg_path(src_path);
        let safe_out = sanitize_ffmpeg_path(out_wav);
        let output = Command::new(&self.ffmpeg_bin)
            .args(["-y", "-v", "error", "-i"])
            .arg(&safe_src)
            .args([
                "-vn",
                "-acodec",
                "pcm_s16le",
                "-ar",
                &sample_rate.to_string(),
                "-ac",
                "2",
            ])
            .arg(&safe_out)
            .output()?;
        if !output.status.success() {
            return Err(MediaError::NonZeroExit(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(())
    }

    /// FFmpeg ビルドに含まれるハードウェアエンコーダー候補を列挙する
    pub fn detect_available_hw_encoders(&self) -> Vec<VideoEncoder> {
        let Ok(out) = Command::new(&self.ffmpeg_bin)
            .args(["-hide_banner", "-encoders"])
            .output()
        else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut list = Vec::new();
        if text.contains("h264_nvenc") {
            list.push(VideoEncoder::Nvenc);
        }
        if text.contains("h264_qsv") {
            list.push(VideoEncoder::Qsv);
        }
        if text.contains("h264_amf") {
            list.push(VideoEncoder::Amf);
        }
        list
    }

    /// 実際に GPU ドライバ経由で指定エンコーダーが動作するかを 1 フレーム無出力エンコードで検証する
    pub fn probe_encoder_usable(&self, codec: &str) -> bool {
        Command::new(&self.ffmpeg_bin)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:s=64x64:d=0.04",
                "-frames:v",
                "1",
                "-c:v",
                codec,
                "-f",
                "null",
                "-",
            ])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// 指定フォーマットと希望エンコーダーから、実機で安全に使えるエンコーダーを解決する（不可時は Software へフォールバック）
    pub fn resolve_video_encoder(
        &self,
        format: ExportFormat,
        preferred: VideoEncoder,
    ) -> VideoEncoder {
        if !matches!(format, ExportFormat::Mp4H264 | ExportFormat::Mp4H265)
            || preferred == VideoEncoder::Software
        {
            return VideoEncoder::Software;
        }
        let codec_for = |enc: VideoEncoder| -> Option<&'static str> {
            match (format, enc) {
                (ExportFormat::Mp4H264, VideoEncoder::Nvenc) => Some("h264_nvenc"),
                (ExportFormat::Mp4H264, VideoEncoder::Qsv) => Some("h264_qsv"),
                (ExportFormat::Mp4H264, VideoEncoder::Amf) => Some("h264_amf"),
                (ExportFormat::Mp4H265, VideoEncoder::Nvenc) => Some("hevc_nvenc"),
                (ExportFormat::Mp4H265, VideoEncoder::Qsv) => Some("hevc_qsv"),
                (ExportFormat::Mp4H265, VideoEncoder::Amf) => Some("hevc_amf"),
                _ => None,
            }
        };

        let candidates: Vec<VideoEncoder> = if preferred == VideoEncoder::Auto {
            self.detect_available_hw_encoders()
        } else {
            vec![preferred]
        };
        for cand in candidates {
            if let Some(codec) = codec_for(cand)
                && self.probe_encoder_usable(codec)
            {
                return cand;
            }
        }
        VideoEncoder::Software
    }

    /// 動画書き出しパイプを開始する（stdin へ RGBA8 フレームを順次書き込む）
    pub fn start_export(
        &self,
        output_path: &Path,
        config: &ExportConfig,
        audio_wav_path: Option<&Path>,
    ) -> Result<FfmpegExportSession, MediaError> {
        if config.format == ExportFormat::WavAudioOnly {
            return Err(MediaError::InvalidArgument(
                "WavAudioOnly does not use video export pipe; write WAV directly".into(),
            ));
        }
        let expected_frame_bytes = checked_rgba_frame_bytes(config.width, config.height)?;
        if config.fps.num == 0 || config.fps.den == 0 {
            return Err(MediaError::InvalidArgument(
                "export fps numerator/denominator must be non-zero".into(),
            ));
        }
        let crf = config.crf.min(51);
        let crf_str = crf.to_string();
        let resolved_encoder = self.resolve_video_encoder(config.format, config.encoder);
        let safe_out = sanitize_ffmpeg_path(output_path);
        let fps_str = format!("{}/{}", config.fps.num, config.fps.den);
        let size_str = format!("{}x{}", config.width, config.height);

        let mut cmd = Command::new(&self.ffmpeg_bin);
        cmd.args([
            "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &size_str, "-r",
            &fps_str, "-i", "pipe:0",
        ]);

        if let Some(wav) = audio_wav_path {
            cmd.arg("-i").arg(sanitize_ffmpeg_path(wav));
        }

        match config.format {
            ExportFormat::Mp4H264 => {
                match resolved_encoder {
                    VideoEncoder::Nvenc => {
                        cmd.args([
                            "-c:v",
                            "h264_nvenc",
                            "-pix_fmt",
                            "yuv420p",
                            "-cq",
                            &crf_str,
                            "-preset",
                            "p4",
                            "-movflags",
                            "+faststart",
                        ]);
                    }
                    VideoEncoder::Qsv => {
                        cmd.args([
                            "-c:v",
                            "h264_qsv",
                            "-pix_fmt",
                            "nv12",
                            "-global_quality",
                            &crf_str,
                            "-movflags",
                            "+faststart",
                        ]);
                    }
                    VideoEncoder::Amf => {
                        cmd.args([
                            "-c:v",
                            "h264_amf",
                            "-pix_fmt",
                            "yuv420p",
                            "-qp_i",
                            &crf_str,
                            "-qp_p",
                            &crf_str,
                            "-movflags",
                            "+faststart",
                        ]);
                    }
                    VideoEncoder::Auto | VideoEncoder::Software => {
                        cmd.args([
                            "-c:v",
                            "libx264",
                            "-pix_fmt",
                            "yuv420p",
                            "-crf",
                            &crf_str,
                            "-preset",
                            "fast",
                            "-movflags",
                            "+faststart",
                        ]);
                    }
                }
                if audio_wav_path.is_some() {
                    cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
                }
            }
            ExportFormat::Mp4H265 => {
                match resolved_encoder {
                    VideoEncoder::Nvenc => {
                        cmd.args([
                            "-c:v",
                            "hevc_nvenc",
                            "-pix_fmt",
                            "yuv420p",
                            "-cq",
                            &crf_str,
                            "-preset",
                            "p4",
                        ]);
                    }
                    VideoEncoder::Qsv => {
                        cmd.args([
                            "-c:v",
                            "hevc_qsv",
                            "-pix_fmt",
                            "nv12",
                            "-global_quality",
                            &crf_str,
                        ]);
                    }
                    VideoEncoder::Amf => {
                        cmd.args([
                            "-c:v", "hevc_amf", "-pix_fmt", "yuv420p", "-qp_i", &crf_str, "-qp_p",
                            &crf_str,
                        ]);
                    }
                    VideoEncoder::Auto | VideoEncoder::Software => {
                        cmd.args(["-c:v", "libx265", "-pix_fmt", "yuv420p", "-crf", &crf_str]);
                    }
                }
                if audio_wav_path.is_some() {
                    cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
                }
            }
            ExportFormat::WebmVp9Alpha => {
                cmd.args([
                    "-c:v",
                    "libvpx-vp9",
                    "-pix_fmt",
                    "yuva420p",
                    "-crf",
                    &crf_str,
                    "-b:v",
                    "0",
                ]);
                if audio_wav_path.is_some() {
                    cmd.args(["-c:a", "libopus", "-shortest"]);
                }
            }
            ExportFormat::PngSequence => {
                cmd.args(["-c:v", "png", "-pix_fmt", "rgba"]);
            }
            ExportFormat::WavAudioOnly => unreachable!(),
        }

        cmd.arg(&safe_out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| MediaError::NonZeroExit("failed to open ffmpeg stdin".into()))?;

        Ok(FfmpegExportSession {
            child,
            stdin: Some(stdin),
            expected_frame_bytes,
            resolved_encoder,
        })
    }
}

pub struct FfmpegExportSession {
    child: Child,
    stdin: Option<ChildStdin>,
    expected_frame_bytes: usize,
    resolved_encoder: VideoEncoder,
}

impl FfmpegExportSession {
    pub fn resolved_encoder(&self) -> VideoEncoder {
        self.resolved_encoder
    }

    pub fn write_rgba8_frame(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        if rgba.len() != self.expected_frame_bytes {
            return Err(MediaError::InvalidArgument(format!(
                "frame buffer length mismatch: got {} bytes, expected {}",
                rgba.len(),
                self.expected_frame_bytes
            )));
        }
        if let Some(stdin) = self.stdin.as_mut() {
            stdin.write_all(rgba)?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), MediaError> {
        drop(self.stdin.take());
        let output = self.child.wait_with_output()?;
        if !output.status.success() {
            return Err(MediaError::NonZeroExit(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(())
    }

    /// 書き出しキャンセル時に FFmpeg 子プロセスを確実に終了・回収する
    pub fn cancel(mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_decode::write_wav_pcm16;
    use tempfile::tempdir;

    #[test]
    fn ffmpeg_roundtrip_export_probe_and_decode() {
        let cli = FfmpegCli::default();
        let tmp = tempdir().unwrap();
        let mp4_path = tmp.path().join("test_out.mp4");

        let config = ExportConfig {
            width: 160,
            height: 90,
            fps: FrameRate::FPS_30,
            format: ExportFormat::Mp4H264,
            crf: 23,
            encoder: VideoEncoder::Auto,
        };

        let mut session = cli.start_export(&mp4_path, &config, None).unwrap();
        // 15フレーム（0.5秒）の単色フレームを書き出す
        let mut frame = vec![0u8; 160 * 90 * 4];
        for px in frame.as_chunks_mut::<4>().0 {
            px[0] = 64;
            px[1] = 160;
            px[2] = 240;
            px[3] = 255;
        }
        for _ in 0..15 {
            session.write_rgba8_frame(&frame).unwrap();
        }
        session.finish().unwrap();

        let probe = cli.probe(&mp4_path).unwrap();
        assert_eq!(probe.width, Some(160));
        assert_eq!(probe.height, Some(90));
        assert!(probe.duration_sec >= 0.4);

        let decoded = cli.decode_frame_rgba8(&mp4_path, 0.1, 160, 90).unwrap();
        assert_eq!(decoded.len(), 160 * 90 * 4);
    }

    #[test]
    fn ffmpeg_proxy_generation_and_audio_mux_extract_roundtrip() {
        let cli = FfmpegCli::default();
        let tmp = tempdir().unwrap();
        let wav_path = tmp.path().join("audio.wav");
        let mp4_path = tmp.path().join("muxed.mp4");
        let proxy_path = tmp.path().join("proxy").join("muxed_proxy.mp4");
        let extracted_wav = tmp.path().join("extracted.wav");

        // 0.5秒のステレオ音声 WAV を作成
        let sr = 48_000u32;
        let samples = vec![0.25f32; (sr as usize / 2) * 2];
        write_wav_pcm16(&wav_path, &samples, sr, 2).unwrap();

        let config = ExportConfig {
            width: 320,
            height: 180,
            fps: FrameRate::FPS_30,
            format: ExportFormat::Mp4H264,
            crf: 23,
            encoder: VideoEncoder::Software,
        };
        let mut session = cli
            .start_export(&mp4_path, &config, Some(&wav_path))
            .unwrap();
        let frame = vec![128u8; 320 * 180 * 4];
        for _ in 0..15 {
            session.write_rgba8_frame(&frame).unwrap();
        }
        session.finish().unwrap();

        // 映像＋音声の両ストリームが Mux されていることを確認
        let probe = cli.probe(&mp4_path).unwrap();
        assert_eq!(probe.width, Some(320));
        assert_eq!(probe.height, Some(180));
        assert!(probe.audio_sample_rate.is_some());

        // プロキシ生成 (高さ 90px へ縮小)
        cli.generate_proxy(&mp4_path, &proxy_path, 90).unwrap();
        let proxy_probe = cli.probe(&proxy_path).unwrap();
        assert_eq!(proxy_probe.height, Some(90));

        // 音声抽出
        cli.extract_audio_wav(&mp4_path, &extracted_wav, 48_000)
            .unwrap();
        assert!(extracted_wav.exists());
    }

    #[test]
    fn ffmpeg_transparent_png_sequence_export() {
        let cli = FfmpegCli::default();
        let tmp = tempdir().unwrap();
        let pattern = tmp.path().join("frame_%04d.png");

        let config = ExportConfig {
            width: 64,
            height: 36,
            fps: FrameRate::FPS_30,
            format: ExportFormat::PngSequence,
            crf: 18,
            encoder: VideoEncoder::Software,
        };
        let mut session = cli.start_export(&pattern, &config, None).unwrap();
        let mut frame = vec![0u8; 64 * 36 * 4];
        // 半透明ピクセル
        for px in frame.as_chunks_mut::<4>().0 {
            px[0] = 255;
            px[1] = 128;
            px[2] = 0;
            px[3] = 128;
        }
        for _ in 0..3 {
            session.write_rgba8_frame(&frame).unwrap();
        }
        session.finish().unwrap();

        assert!(tmp.path().join("frame_0001.png").exists());
        assert!(tmp.path().join("frame_0003.png").exists());
    }

    #[test]
    fn sanitizes_dash_prefixed_paths_and_rejects_invalid_bounds() {
        assert_eq!(
            sanitize_ffmpeg_path(Path::new("-malicious.mp4")),
            PathBuf::from("./-malicious.mp4")
        );
        assert_eq!(
            sanitize_ffmpeg_path(Path::new("normal.mp4")),
            PathBuf::from("normal.mp4")
        );

        let cli = FfmpegCli::default();
        assert!(matches!(
            cli.decode_frame_rgba8(Path::new("in.mp4"), -1.0, 64, 64),
            Err(MediaError::InvalidArgument(_))
        ));
        assert!(matches!(
            cli.decode_frame_rgba8(Path::new("in.mp4"), f64::NAN, 64, 64),
            Err(MediaError::InvalidArgument(_))
        ));
        assert!(matches!(
            cli.decode_frame_rgba8(Path::new("in.mp4"), 0.0, 0, 64),
            Err(MediaError::InvalidArgument(_))
        ));
        assert!(matches!(
            cli.decode_frame_rgba8(Path::new("in.mp4"), 0.0, MAX_VIDEO_DIMENSION + 1, 64),
            Err(MediaError::InvalidArgument(_))
        ));
    }
}
