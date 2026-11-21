use crate::ffmpeg::MediaError;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// インターリーブ済み浮動小数点（-1.0..=1.0）PCMバッファ
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn frame_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels
        }
    }

    pub fn duration_sec(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frame_count() as f64 / self.sample_rate as f64
        }
    }
}

/// `symphonia` 0.6 を使って音声ファイルを Pure Rust でデコードする
pub fn decode_audio_file(path: &Path) -> Result<AudioBuffer, MediaError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| MediaError::AudioDecode(e.to_string()))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| MediaError::AudioDecode("no default audio track".into()))?;
    let track_id = track.id;

    let audio_params = match &track.codec_params {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err(MediaError::AudioDecode("missing audio codec parameters".into())),
    };

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| MediaError::AudioDecode(e.to_string()))?;

    let mut sample_rate = audio_params.sample_rate.unwrap_or(48_000);
    let mut channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);
    let mut samples: Vec<f32> = Vec::new();
    let mut chunk: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => continue,
            Err(e) => return Err(MediaError::AudioDecode(e.to_string())),
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(MediaError::AudioDecode(e.to_string())),
        };

        let spec = decoded.spec();
        sample_rate = spec.rate();
        channels = spec.channels().count();

        chunk.clear();
        decoded.copy_to_vec_interleaved::<f32>(&mut chunk);
        samples.extend_from_slice(&chunk);
    }

    Ok(AudioBuffer {
        sample_rate,
        channels,
        samples,
    })
}

/// Audio Graph の出力を 16-bit PCM WAV ファイルとして書き出す（FFmpeg mux 用）
pub fn write_wav_pcm16(
    path: &Path,
    interleaved_f32: &[f32],
    sample_rate: u32,
    channels: u16,
) -> Result<(), MediaError> {
    let mut file = File::create(path)?;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample / 8);
    let block_align = channels * (bits_per_sample / 8);
    let data_bytes = (interleaved_f32.len() * 2) as u32;
    let riff_size = 36 + data_bytes;

    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&bits_per_sample.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;

    for &s in interleaved_f32 {
        let clamped = s.clamp(-1.0, 1.0);
        let pcm = (clamped * i16::MAX as f32).round() as i16;
        file.write_all(&pcm.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn wav_write_and_symphonia_decode_roundtrip() {
        let tmp = tempdir().unwrap();
        let wav_path = tmp.path().join("tone.wav");

        let sr = 44_100u32;
        let frames = 4_410usize; // 0.1 sec
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.5;
            samples.push(v);
            samples.push(v);
        }

        write_wav_pcm16(&wav_path, &samples, sr, 2).unwrap();
        let decoded = decode_audio_file(&wav_path).unwrap();
        assert_eq!(decoded.sample_rate, sr);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.frame_count(), frames);
    }
}
