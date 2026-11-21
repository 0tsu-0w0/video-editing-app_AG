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
}

#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub format: ExportFormat,
    pub crf: u8,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: FrameRate::FPS_60,
            format: ExportFormat::Mp4H264,
            crf: 18,
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

    /// 動画書き出しパイプを開始する（stdin へ RGBA8 フレームを順次書き込む）
    pub fn start_export(
        &self,
        output_path: &Path,
        config: &ExportConfig,
        audio_wav_path: Option<&Path>,
    ) -> Result<FfmpegExportSession, MediaError> {
        let expected_frame_bytes = checked_rgba_frame_bytes(config.width, config.height)?;
        if config.fps.num == 0 || config.fps.den == 0 {
            return Err(MediaError::InvalidArgument(
                "export fps numerator/denominator must be non-zero".into(),
            ));
        }
        let crf = config.crf.min(51);
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
                cmd.args([
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-crf",
                    &crf.to_string(),
                    "-preset",
                    "fast",
                    "-movflags",
                    "+faststart",
                ]);
                if audio_wav_path.is_some() {
                    cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
                }
            }
            ExportFormat::Mp4H265 => {
                cmd.args([
                    "-c:v",
                    "libx265",
                    "-pix_fmt",
                    "yuv420p",
                    "-crf",
                    &crf.to_string(),
                ]);
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
                    &crf.to_string(),
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
        })
    }
}

pub struct FfmpegExportSession {
    child: Child,
    stdin: Option<ChildStdin>,
    expected_frame_bytes: usize,
}

impl FfmpegExportSession {
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
