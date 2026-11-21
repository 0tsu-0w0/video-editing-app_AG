use crate::dsp::{
    apply_audio_effect_chain, apply_sidechain_ducking, compute_lufs_gain, measure_integrated_lufs,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use ve_core::{
    AssetId, AssetResolution, AudioDuckingConfig, AudioEffectNode, ItemKind, Keyframe, Project,
    ProjectFolder, RationalTime, SceneId, evaluate_track,
};
use ve_media::{AudioBuffer, FfmpegCli, decode_audio_file};

#[derive(Debug, Clone)]
pub struct AudioClip {
    pub layer_index: usize,
    pub timeline_start: RationalTime,
    pub duration: RationalTime,
    pub source_offset: RationalTime,
    pub gain: f32,
    pub muted: bool,
    pub speed: f32,
    pub loop_playback: bool,
    pub time_remap_kfs: Vec<Keyframe>,
    pub effects: Vec<AudioEffectNode>,
    pub buffer: Arc<AudioBuffer>,
}

/// プレビュー再生・FFmpeg書き出し共通の Audio Graph（リサンプリング＋速度/ループ＋DSP＋ダッキング＋LUFS＋ステレオミキサー）
#[derive(Debug, Clone)]
pub struct AudioGraph {
    pub sample_rate: u32,
    pub channels: usize,
    pub master_gain: f32,
    pub master_effects: Vec<AudioEffectNode>,
    pub target_lufs: Option<f32>,
    pub ducking: AudioDuckingConfig,
    pub clips: Vec<AudioClip>,
}

impl AudioGraph {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channels: 2,
            master_gain: 1.0,
            master_effects: Vec::new(),
            target_lufs: None,
            ducking: AudioDuckingConfig::default(),
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
        if let Some(scene) = project.scene(scene_id) {
            graph.master_effects = scene.master_audio_effects.clone();
            graph.target_lufs = scene.target_lufs;
            graph.ducking = scene.audio_ducking;
        }
        let mut decoded_cache: HashMap<AssetId, Option<Arc<AudioBuffer>>> = HashMap::new();
        let ffmpeg = FfmpegCli::default();
        graph.collect_scene_clips(
            project,
            scene_id,
            folder,
            RationalTime::ZERO,
            1.0,
            None,
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
        parent_layer_index: Option<usize>,
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
        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            if layer.muted {
                continue;
            }
            let layer_gain = layer.volume.max(0.0);
            if layer_gain <= 0.0 {
                continue;
            }
            let effective_layer = parent_layer_index.unwrap_or(layer_idx);
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
                        let gain = (*volume).max(0.0) * layer_gain * gain_mul;
                        if gain <= 0.0 {
                            continue;
                        }
                        let buf_opt = decoded_cache.entry(*asset_id).or_insert_with(|| {
                            let path = resolve_project_asset_path(project, *asset_id, folder)?;
                            decode_media_audio_with_fallback(&path, self.sample_rate, ffmpeg)
                                .map(Arc::new)
                        });
                        if let Some(buffer) = buf_opt.as_ref() {
                            let time_remap_kfs = item
                                .keyframes
                                .get("time_remap")
                                .cloned()
                                .unwrap_or_default();
                            self.clips.push(AudioClip {
                                layer_index: effective_layer,
                                timeline_start: time_offset + item.start,
                                duration: item.duration,
                                source_offset: *source_offset,
                                gain,
                                muted: false,
                                speed: item.playback.speed,
                                loop_playback: item.playback.loop_playback,
                                time_remap_kfs,
                                effects: item.audio_effects.clone(),
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
                            gain_mul * layer_gain,
                            Some(effective_layer),
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

        let ducking_active = self.ducking.enabled
            && self.channels == 2
            && self.ducking.voice_layer_index != self.ducking.bgm_layer_index;
        let mut voice_bus = if ducking_active {
            vec![0.0f32; total_samples]
        } else {
            Vec::new()
        };
        let mut bgm_bus = if ducking_active {
            vec![0.0f32; total_samples]
        } else {
            Vec::new()
        };

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

            let has_clip_fx = clip.effects.iter().any(|e| e.enabled);
            let mut clip_scratch = if has_clip_fx {
                vec![0.0f32; (i_end - i_start) * self.channels]
            } else {
                Vec::new()
            };

            let target_bus: &mut [f32] = if ducking_active {
                if clip.layer_index == self.ducking.voice_layer_index {
                    &mut voice_bus
                } else if clip.layer_index == self.ducking.bgm_layer_index {
                    &mut bgm_bus
                } else {
                    &mut out[..total_samples]
                }
            } else {
                &mut out[..total_samples]
            };

            let speed = f64::from(clip.speed);
            let src_frames_f = src_frames as f64;
            let reverse_anchor_frames = (src_frames_f - 1.0).max(0.0);
            let has_remap = !clip.time_remap_kfs.is_empty();

            let step = (src_sr / dst_sr) * speed;
            let local_start_sec = (start_sample + i_start as i64) as f64 / dst_sr - clip_start_sec;
            let mut src_pos = if speed < 0.0 {
                reverse_anchor_frames - local_start_sec * speed.abs() * src_sr + offset_sec * src_sr
            } else {
                (local_start_sec * speed + offset_sec) * src_sr
            };

            for i in i_start..i_end {
                let sample_pos = if has_remap {
                    let local_sec = (start_sample + i as i64) as f64 / dst_sr - clip_start_sec;
                    let mapped_sec = f64::from(evaluate_track(
                        (local_sec + offset_sec) as f32,
                        &clip.time_remap_kfs,
                        RationalTime::from_seconds(local_sec),
                    ));
                    mapped_sec * src_sr
                } else {
                    src_pos
                };

                let wrapped_pos = if clip.loop_playback && src_frames_f > 0.0 {
                    Some(sample_pos.rem_euclid(src_frames_f))
                } else if sample_pos >= 0.0 && sample_pos < src_frames_f {
                    Some(sample_pos)
                } else {
                    None
                };

                if let Some(pos) = wrapped_pos {
                    let idx0 = (pos as usize).min(src_frames - 1);
                    let idx1 = if clip.loop_playback {
                        (idx0 + 1) % src_frames
                    } else {
                        (idx0 + 1).min(src_frames - 1)
                    };
                    let frac = (pos - idx0 as f64).clamp(0.0, 1.0) as f32;

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

                    if has_clip_fx {
                        let base = (i - i_start) * self.channels;
                        clip_scratch[base] = l * clip.gain;
                        if self.channels > 1 {
                            clip_scratch[base + 1] = r * clip.gain;
                        }
                    } else {
                        let base = i * self.channels;
                        target_bus[base] += l * clip.gain;
                        if self.channels > 1 {
                            target_bus[base + 1] += r * clip.gain;
                        }
                    }
                }
                src_pos += step;
            }

            if has_clip_fx {
                apply_audio_effect_chain(
                    &mut clip_scratch,
                    self.channels,
                    self.sample_rate,
                    &clip.effects,
                );
                for (rel_i, frame) in clip_scratch.chunks_exact(self.channels).enumerate() {
                    let base = (i_start + rel_i) * self.channels;
                    target_bus[base] += frame[0];
                    if self.channels > 1 {
                        target_bus[base + 1] += frame[1];
                    }
                }
            }
        }

        // v0.6 #4: 発話レイヤーに応じて BGM レイヤーをサイドチェイン減衰して合成
        if ducking_active {
            apply_sidechain_ducking(&voice_bus, &mut bgm_bus, self.sample_rate, &self.ducking);
            for ((dst, v), b) in out[..total_samples]
                .iter_mut()
                .zip(voice_bus.iter())
                .zip(bgm_bus.iter())
            {
                *dst += *v + *b;
            }
        }

        // マスター音声 DSP エフェクトチェーン適用
        if self.master_effects.iter().any(|e| e.enabled) {
            apply_audio_effect_chain(
                &mut out[..total_samples],
                self.channels,
                self.sample_rate,
                &self.master_effects,
            );
        }

        // マスターゲイン適用とクリッピング防止
        for s in &mut out[..total_samples] {
            *s = (*s * self.master_gain).clamp(-1.0, 1.0);
        }
    }

    /// 書き出し用: タイムライン全体の音声を一括レンダリングする（`target_lufs` 指定時はラウドネス正規化）
    pub fn render_range(&self, start: RationalTime, duration: RationalTime) -> Vec<f32> {
        let start_sample = start.to_audio_samples(self.sample_rate);
        let frames = duration.to_audio_samples(self.sample_rate).max(0) as usize;
        let mut out = vec![0.0f32; frames * self.channels];
        self.render_block(start_sample, frames, &mut out);
        if let Some(target_lufs) = self.target_lufs
            && self.channels == 2
        {
            let measured = measure_integrated_lufs(&out, self.sample_rate);
            let gain = compute_lufs_gain(measured, target_lufs);
            if (gain - 1.0).abs() > 1e-3 {
                for s in &mut out {
                    *s = (*s * gain).clamp(-1.0, 1.0);
                }
            }
        }
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
            layer_index: 0,
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            speed: 1.0,
            loop_playback: false,
            time_remap_kfs: Vec::new(),
            effects: Vec::new(),
            buffer: buf_44k,
        });
        graph.add_clip(AudioClip {
            layer_index: 1,
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            speed: 1.0,
            loop_playback: false,
            time_remap_kfs: Vec::new(),
            effects: Vec::new(),
            buffer: buf_48k,
        });

        let rendered = graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(0.1));
        assert_eq!(rendered.len(), 4_800 * 2);
        for &s in &rendered[10..100] {
            assert!((s - 0.55).abs() < 1e-4, "expected ~0.55, got {s}");
        }
    }

    #[test]
    fn supports_loop_playback_and_reverse_speed() {
        let ramp: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        let buf = Arc::new(AudioBuffer {
            sample_rate: 1000,
            channels: 1,
            samples: ramp,
        });

        // 2x speed with loop over 1.0s (covers two full cycles)
        let mut graph = AudioGraph::new(1000);
        graph.add_clip(AudioClip {
            layer_index: 0,
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            speed: 2.0,
            loop_playback: true,
            time_remap_kfs: Vec::new(),
            effects: Vec::new(),
            buffer: Arc::clone(&buf),
        });
        let pcm = graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(1.0));
        // At t=0.75s with 2x speed, src_pos = 1500 % 1000 = 500 -> value ~0.5
        assert!((pcm[750 * 2] - 0.5).abs() < 0.02, "got {}", pcm[750 * 2]);

        // Reverse playback (-1x) starts near end (~0.999) and decreases toward 0.0
        let mut rev_graph = AudioGraph::new(1000);
        rev_graph.add_clip(AudioClip {
            layer_index: 0,
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            speed: -1.0,
            loop_playback: false,
            time_remap_kfs: Vec::new(),
            effects: Vec::new(),
            buffer: buf,
        });
        let rev = rev_graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(1.0));
        assert!(rev[0] > 0.95 && rev[900 * 2] < 0.15);
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
        {
            let layer = &mut project.active_scene_mut().unwrap().layers[3];
            layer.volume = 0.5;
            layer.items.push(TimelineItem::new(
                "BGM",
                RationalTime::ZERO,
                RationalTime::from_seconds(0.5),
                ItemKind::Audio {
                    asset_id,
                    source_offset: RationalTime::ZERO,
                    volume: 0.5,
                },
            ));
        }

        let graph = AudioGraph::from_project_scene(&project, project.active_scene_id, None, 48_000);
        assert_eq!(graph.clips.len(), 1);
        let pcm = graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(0.1));
        // 0.4 * clip(0.5) * layer(0.5) = 0.1
        assert!((pcm[20] - 0.1).abs() < 0.01, "got {}", pcm[20]);
    }

    #[test]
    fn sidechain_ducking_in_audio_graph_ducks_bgm_layer_on_voice() {
        let sr = 48_000;
        let bgm_buf = Arc::new(AudioBuffer {
            sample_rate: sr,
            channels: 1,
            samples: vec![0.4; sr as usize],
        });
        let voice_buf = Arc::new(AudioBuffer {
            sample_rate: sr,
            channels: 1,
            samples: vec![0.25; sr as usize / 2],
        });

        let mut graph = AudioGraph::new(sr);
        graph.ducking = AudioDuckingConfig {
            enabled: true,
            voice_layer_index: 3,
            bgm_layer_index: 2,
            threshold_rms: 0.05,
            reduction_db: -18.0,
            attack_sec: 0.01,
            release_sec: 0.03,
        };
        // BGM on layer 2 from 0..1s
        graph.add_clip(AudioClip {
            layer_index: 2,
            timeline_start: RationalTime::ZERO,
            duration: RationalTime::from_seconds(1.0),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            speed: 1.0,
            loop_playback: false,
            time_remap_kfs: Vec::new(),
            effects: Vec::new(),
            buffer: bgm_buf,
        });
        // Voice on layer 3 from 0.25..0.75s
        graph.add_clip(AudioClip {
            layer_index: 3,
            timeline_start: RationalTime::from_seconds(0.25),
            duration: RationalTime::from_seconds(0.5),
            source_offset: RationalTime::ZERO,
            gain: 1.0,
            muted: false,
            speed: 1.0,
            loop_playback: false,
            time_remap_kfs: Vec::new(),
            effects: Vec::new(),
            buffer: voice_buf,
        });

        let pcm = graph.render_range(RationalTime::ZERO, RationalTime::from_seconds(1.0));
        // Before voice (t=0.1s): full BGM (~0.4)
        let before = pcm[(sr as usize / 10) * 2];
        // During voice (t=0.5s): voice(0.25) + ducked BGM(0.4 * 0.125 = 0.05) ~= 0.30 < un-ducked (0.65)
        let during = pcm[(sr as usize / 2) * 2];
        assert!((before - 0.4).abs() < 0.02, "before={before}");
        assert!(during < 0.38, "expected ducked sum < 0.38, got {during}");
    }
}
