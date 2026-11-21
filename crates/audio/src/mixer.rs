use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use ve_core::{AssetId, AssetResolution, ItemKind, Project, ProjectFolder, RationalTime, SceneId};
use ve_media::{AudioBuffer, FfmpegCli, decode_audio_file};

#[derive(Debug, Clone)]
pub struct AudioClip {
    pub timeline_start: RationalTime,
    pub duration: RationalTime,
    pub source_offset: RationalTime,
    pub gain: f32,
    pub muted: bool,
    pub buffer: Arc<AudioBuffer>,
}

/// プレビュー再生・FFmpeg書き出し共通の Audio Graph（リサンプリング＋ステレオミキサー）
#[derive(Debug, Clone)]
pub struct AudioGraph {
    pub sample_rate: u32,
    pub channels: usize,
    pub master_gain: f32,
    pub clips: Vec<AudioClip>,
}

impl AudioGraph {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channels: 2,
            master_gain: 1.0,
            clips: Vec::new(),
        }
    }

    pub fn add_clip(&mut self, clip: AudioClip) {
        self.clips.push(clip);
    }

    /// プロジェクトの指定シーンから Audio / Video / Precomp クリップを収集し、
    /// プレビュー再生と動画書き出しで完全に一致する `AudioGraph` を構築する。
    pub fn from_project_scene(
        project: &Project,
        scene_id: SceneId,
        folder: Option<&ProjectFolder>,
        sample_rate: u32,
    ) -> Self {
        let mut graph = Self::new(sample_rate.max(8_000));
        let mut decoded_cache: HashMap<AssetId, Option<Arc<AudioBuffer>>> = HashMap::new();
        let ffmpeg = FfmpegCli::default();
        graph.collect_scene_clips(
            project,
            scene_id,
            folder,
            RationalTime::ZERO,
            1.0,
            0,
            &ffmpeg,
            &mut decoded_cache,
        );
        graph
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_scene_clips(
        &mut self,
        project: &Project,
        scene_id: SceneId,
        folder: Option<&ProjectFolder>,
        time_offset: RationalTime,
        gain_mul: f32,
        depth: u8,
        ffmpeg: &FfmpegCli,
        decoded_cache: &mut HashMap<AssetId, Option<Arc<AudioBuffer>>>,
    ) {
        if depth >= 4 {
            return;
        }
        let Some(scene) = project.scene(scene_id) else {
            return;
        };
        for layer in &scene.layers {
            if layer.muted {
                continue;
            }
            for item in &layer.items {
                match &item.kind {
                    ItemKind::Audio {
                        asset_id,
                        source_offset,
                        volume,
                    }
                    | ItemKind::Video {
                        asset_id,
                        source_offset,
                        volume,
                    } => {
                        let gain = (*volume).max(0.0) * gain_mul;
                        if gain <= 0.0 {
                            continue;
                        }
                        let buf_opt = decoded_cache.entry(*asset_id).or_insert_with(|| {
                            let path = resolve_project_asset_path(project, *asset_id, folder)?;
                            decode_media_audio_with_fallback(&path, self.sample_rate, ffmpeg)
                                .map(Arc::new)
                        });
                        if let Some(buffer) = buf_opt.as_ref() {
                            self.clips.push(AudioClip {
                                timeline_start: time_offset + item.start,
                                duration: item.duration,
                                source_offset: *source_offset,
                                gain,
                                muted: false,
                                buffer: Arc::clone(buffer),
                            });
                        }
                    }
                    ItemKind::Precomp {
                        scene_id: nested_id,
                    } => {
                        self.collect_scene_clips(
                            project,
                            *nested_id,
                            folder,
                            time_offset + item.start,
                            gain_mul,
                            depth + 1,
                            ffmpeg,
                            decoded_cache,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    /// 指定サンプル位置 `start_sample` から `frame_count` フレーム分のステレオPCMをレンダリングする
    pub fn render_block(&self, start_sample: i64, frame_count: usize, out: &mut [f32]) {
        let total_samples = frame_count * self.channels;
        debug_assert!(out.len() >= total_samples);
        out[..total_samples].fill(0.0);

        if self.sample_rate == 0 || frame_count == 0 {
            return;
        }

        let block_start_t = RationalTime::from_audio_samples(start_sample, self.sample_rate);
        let block_end_t =
            RationalTime::from_audio_samples(start_sample + frame_count as i64, self.sample_rate);

        for clip in &self.clips {
            if clip.muted
                || clip.gain <= 0.0
                || clip.buffer.sample_rate == 0
                || clip.buffer.channels == 0
            {
                continue;
            }
            let clip_end = clip.timeline_start + clip.duration;
            if clip_end <= block_start_t || clip.timeline_start >= block_end_t {
                continue;
            }

            let src_sr = clip.buffer.sample_rate as f64;
            let dst_sr = self.sample_rate as f64;
            let src_frames = clip.buffer.frame_count();
            if src_frames == 0 {
                continue;
            }
            let src_ch = clip.buffer.channels;
            let clip_start_sec = clip.timeline_start.as_seconds();
            let clip_end_sec = clip_end.as_seconds();
            let offset_sec = clip.source_offset.as_seconds();

            let block_start_sec = start_sample as f64 / dst_sr;
            let i_start = if block_start_sec < clip_start_sec {
                ((clip_start_sec - block_start_sec) * dst_sr)
                    .ceil()
                    .max(0.0) as usize
            } else {
                0
            };
            let i_end = (((clip_end_sec - block_start_sec) * dst_sr).ceil().max(0.0) as usize)
                .min(frame_count);
            if i_start >= i_end {
                continue;
            }

            let step = src_sr / dst_sr;
            let mut src_pos = ((start_sample + i_start as i64) as f64 / dst_sr - clip_start_sec
                + offset_sec)
                * src_sr;

            for i in i_start..i_end {
                if src_pos >= 0.0 {
                    let idx0 = src_pos as usize;
                    if idx0 >= src_frames {
                        break;
                    }
                    let idx1 = (idx0 + 1).min(src_frames - 1);
                    let frac = (src_pos - idx0 as f64) as f32;

                    let (l, r) = if src_ch == 1 {
                        let s0 = clip.buffer.samples[idx0];
                        let s1 = clip.buffer.samples[idx1];
                        let v = s0 + (s1 - s0) * frac;
                        (v, v)
                    } else {
                        let base0 = idx0 * src_ch;
                        let base1 = idx1 * src_ch;
                        let l0 = clip.buffer.samples[base0];
                        let r0 = clip.buffer.samples[base0 + 1];
                        let l1 = clip.buffer.samples[base1];
                        let r1 = clip.buffer.samples[base1 + 1];
                        (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac)
                    };

                    let base = i * self.channels;
                    out[base] += l * clip.gain;
                    if self.channels > 1 {
                        out[base + 1] += r * clip.gain;
                    }
                }
                src_pos += step;
            }
        }

        // マスターゲイン適用とクリッピング防止
        for s in &mut out[..total_samples] {
            *s = (*s * self.master_gain).clamp(-1.0, 1.0);
        }
    }

    /// 書き出し用: タイムライン全体の音声を一括レンダリングする
    pub fn render_range(&self, start: RationalTime, duration: RationalTime) -> Vec<f32> {
        let start_sample = start.to_audio_samples(self.sample_rate);
        let frames = duration.to_audio_samples(self.sample_rate).max(0) as usize;
        let mut out = vec![0.0f32; frames * self.channels];
        self.render_block(start_sample, frames, &mut out);
        out
    }
}

fn resolve_project_asset_path(
    project: &Project,
    asset_id: AssetId,
    folder: Option<&ProjectFolder>,
) -> Option<PathBuf> {
    let asset = project.asset(asset_id)?;
    if let Some(f) = folder {
        match f.resolve_asset(asset) {
            AssetResolution::Found(p) => Some(p),
            AssetResolution::Offline => None,
        }
    } else {
        asset
            .fallback_absolute_path
            .as_ref()
            .filter(|p| p.exists())
            .cloned()
    }
}

fn decode_media_audio_with_fallback(
    path: &std::path::Path,
    target_sample_rate: u32,
    ffmpeg: &FfmpegCli,
) -> Option<AudioBuffer> {
    if let Ok(buf) = decode_audio_file(path)
        && !buf.samples.is_empty()
    {
        return Some(buf);
    }
    static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let seq = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp_wav =
        std::env::temp_dir().join(format!("ve_audio_extract_{}_{seq}.wav", std::process::id()));
    let decoded = if ffmpeg
        .extract_audio_wav(path, &tmp_wav, target_sample_rate)
        .is_ok()
    {
        decode_audio_file(&tmp_wav).ok()
    } else {
        None
    };
    let _ = std::fs::remove_file(&tmp_wav);
    decoded.filter(|b| !b.samples.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use uuid::Uuid;
    use ve_core::{AssetRef, MediaKind, ProjectSettings, TimelineItem};
    use ve_media::write_wav_pcm16;

    #[test]
    fn resamples_44100_into_48000_and_mixes_tracks() {
        // 44.1kHz モノラル一定値 0.25 のクリップ
        let buf_44k = Arc::new(AudioBuffer {
            sample_rate: 44_100,
            channels: 1,
            samples: vec![0.25; 44_100],
        });
        // 48kHz ステレオ一定値 0.30 のクリップ
        let buf_48k = Arc::new(AudioBuffer {
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.30; 48_000 * 2],
        });

        let mut graph = AudioGraph::new(48_000);
        graph.add_clip(AudioClip {
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            buffer: buf_44k,
        });
        graph.add_clip(AudioClip {
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            buffer: buf_48k,
        });

        let rendered = graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(0.1));
        assert_eq!(rendered.len(), 4_800 * 2);
        for &s in &rendered[10..100] {
            assert!((s - 0.55).abs() < 1e-4, "expected ~0.55, got {s}");
        }
    }

    #[test]
    fn builds_audio_graph_from_project_scene_clips() {
        let tmp = tempdir().unwrap();
        let wav_path = tmp.path().join("bgm.wav");
        write_wav_pcm16(&wav_path, &vec![0.4f32; 48_000], 48_000, 1).unwrap();

        let mut project = Project::new("AudioScene", ProjectSettings::default());
        let asset_id = Uuid::new_v4();
        project.assets.push(AssetRef {
            id: asset_id,
            name: "bgm.wav".into(),
            kind: MediaKind::Audio,
            relative_path: None,
            fallback_absolute_path: Some(wav_path),
            collected: false,
        });
        project.active_scene_mut().unwrap().layers[3]
            .items
            .push(TimelineItem::new(
                "BGM",
                RationalTime::ZERO,
                RationalTime::from_seconds(0.5),
                ItemKind::Audio {
                    asset_id,
                    source_offset: RationalTime::ZERO,
                    volume: 0.5,
                },
            ));

        let graph = AudioGraph::from_project_scene(&project, project.active_scene_id, None, 48_000);
        assert_eq!(graph.clips.len(), 1);
        let pcm = graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(0.1));
        assert!((pcm[20] - 0.2).abs() < 0.01, "got {}", pcm[20]);
    }
}
