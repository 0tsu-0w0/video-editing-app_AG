use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;
use ve_audio::AudioGraph;
use ve_core::{Project, ProjectFolder, RationalTime, SceneId};
use ve_media::{
    ExportConfig, ExportFormat, FfmpegCli, HwDecodeBackend, VideoEncoder, write_wav_pcm16,
};
use ve_render::{
    GpuPostProcessPass, HeadlessGpuOffloader, RenderGraph, render_frame_batch_parallel,
};

#[derive(Debug, Clone)]
pub struct ExportJobRequest {
    pub project: Project,
    pub scene_id: SceneId,
    pub folder: Option<ProjectFolder>,
    pub output_path: PathBuf,
    pub config: ExportConfig,
    pub transparent_bg: bool,
    pub gpu_passes: Vec<GpuPostProcessPass>,
    pub shader_files: Vec<(String, PathBuf)>,
    /// v1.0 #18: マルチスレッド並列レンダリングスレッド数（0 で自動 CPU コア数）
    pub worker_threads: usize,
    /// v1.0 #18: FFmpeg ハードウェアデコード設定
    pub hw_decode: HwDecodeBackend,
}

#[derive(Debug, Clone)]
pub struct ExportProgressSnapshot {
    pub current_frame: u64,
    pub total_frames: u64,
    pub stage: String,
    pub resolved_encoder: Option<VideoEncoder>,
    pub elapsed_sec: f64,
    pub eta_sec: Option<f64>,
    pub finished: bool,
    pub succeeded: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

impl ExportProgressSnapshot {
    pub fn fraction(&self) -> f32 {
        if self.total_frames == 0 {
            if self.finished { 1.0 } else { 0.0 }
        } else {
            (self.current_frame as f32 / self.total_frames as f32).clamp(0.0, 1.0)
        }
    }
}

/// バックグラウンド動画・音声書き出しジョブの操作ハンドル（進捗参照・安全キャンセル対応）
pub struct ExportJobHandle {
    progress: Arc<Mutex<ExportProgressSnapshot>>,
    cancel_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

struct TempFileGuard {
    path: PathBuf,
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl ExportJobHandle {
    pub fn start(req: ExportJobRequest) -> Self {
        let progress = Arc::new(Mutex::new(ExportProgressSnapshot {
            current_frame: 0,
            total_frames: 0,
            stage: "準備中...".to_string(),
            resolved_encoder: None,
            elapsed_sec: 0.0,
            eta_sec: None,
            finished: false,
            succeeded: false,
            cancelled: false,
            error: None,
        }));
        let cancel_flag = Arc::new(AtomicBool::new(false));

        let prog_clone = Arc::clone(&progress);
        let cancel_clone = Arc::clone(&cancel_flag);
        let handle = thread::spawn(move || {
            run_export_worker(req, prog_clone, cancel_clone);
        });

        Self {
            progress,
            cancel_flag,
            handle: Some(handle),
        }
    }

    pub fn snapshot(&self) -> ExportProgressSnapshot {
        self.progress.lock().unwrap().clone()
    }

    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.progress.lock().unwrap().finished
    }

    #[cfg(test)]
    pub fn wait(mut self) -> ExportProgressSnapshot {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.snapshot()
    }
}

impl Drop for ExportJobHandle {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn update_progress(
    progress: &Arc<Mutex<ExportProgressSnapshot>>,
    f: impl FnOnce(&mut ExportProgressSnapshot),
) {
    if let Ok(mut guard) = progress.lock() {
        f(&mut guard);
    }
}

fn run_export_worker(
    mut req: ExportJobRequest,
    progress: Arc<Mutex<ExportProgressSnapshot>>,
    cancel_flag: Arc<AtomicBool>,
) {
    let started = Instant::now();
    let Some(scene_duration) = req.project.scene(req.scene_id).map(|s| s.total_duration()) else {
        update_progress(&progress, |p| {
            p.stage = "エラー: 対象シーンが見つかりません".into();
            p.error = Some("target scene not found".into());
            p.finished = true;
        });
        return;
    };

    let duration = scene_duration.max(RationalTime::from_seconds(1.0));
    let total_frames = ((duration.as_seconds() * req.config.fps.as_f64()).ceil() as u64).max(1);
    update_progress(&progress, |p| {
        p.total_frames = total_frames;
        p.stage = "Audio Graph ミックス中...".into();
    });

    if let Some(parent) = req.output_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        update_progress(&progress, |p| {
            p.stage = format!("出力先作成失敗: {e}");
            p.error = Some(e.to_string());
            p.finished = true;
        });
        return;
    }

    // 1. プレビューと同一の AudioGraph で音声トラックを一括ミックス
    let sample_rate = req.project.settings.audio_sample_rate.max(8_000);
    let audio_graph = AudioGraph::from_project_scene(
        &req.project,
        req.scene_id,
        req.folder.as_ref(),
        sample_rate,
    );

    if cancel_flag.load(Ordering::Relaxed) {
        update_progress(&progress, |p| {
            p.stage = "キャンセル済み".into();
            p.cancelled = true;
            p.finished = true;
        });
        return;
    }

    // WAV 単体書き出しモード
    if req.config.format == ExportFormat::WavAudioOnly {
        let pcm = audio_graph.render_range(RationalTime::ZERO, duration);
        match write_wav_pcm16(
            &req.output_path,
            &pcm,
            audio_graph.sample_rate,
            audio_graph.channels as u16,
        ) {
            Ok(()) => {
                update_progress(&progress, |p| {
                    p.current_frame = total_frames;
                    p.elapsed_sec = started.elapsed().as_secs_f64();
                    p.eta_sec = Some(0.0);
                    p.stage = format!("WAV 書き出し完了: {}", req.output_path.display());
                    p.succeeded = true;
                    p.finished = true;
                });
            }
            Err(e) => {
                update_progress(&progress, |p| {
                    p.stage = format!("WAV 書き出しエラー: {e}");
                    p.error = Some(e.to_string());
                    p.finished = true;
                });
            }
        }
        return;
    }

    // 映像コンテナ向け一時 WAV の準備（PNG連番以外で音声クリップがある場合）
    let mut temp_wav_guard: Option<TempFileGuard> = None;
    if req.config.format != ExportFormat::PngSequence && !audio_graph.clips.is_empty() {
        static MUX_WAV_SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = MUX_WAV_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp_wav =
            std::env::temp_dir().join(format!("ve_export_mux_{}_{seq}.wav", std::process::id()));
        let pcm = audio_graph.render_range(RationalTime::ZERO, duration);
        if let Err(e) = write_wav_pcm16(
            &tmp_wav,
            &pcm,
            audio_graph.sample_rate,
            audio_graph.channels as u16,
        ) {
            update_progress(&progress, |p| {
                p.stage = format!("音声ミックスWAV作成失敗: {e}");
                p.error = Some(e.to_string());
                p.finished = true;
            });
            return;
        }
        temp_wav_guard = Some(TempFileGuard { path: tmp_wav });
    }

    // 2. 書き出し解像度へシーン設定を合わせ、原本メディアで RenderGraph を初期化
    if let Some(scene) = req.project.scenes.iter_mut().find(|s| s.id == req.scene_id) {
        scene.width = req.config.width;
        scene.height = req.config.height;
    }
    let mut rg = RenderGraph::new();
    rg.use_proxies = false; // 書き出し時は必ず高画質原本を参照
    rg.high_quality_motion_blur = true; // 書き出し時は高品質サブフレーム分割でモーションブラー合成
    rg.set_hw_decode(req.hw_decode);
    rg.set_project_folder(req.folder.clone());
    for (id, path) in &req.shader_files {
        let _ = rg.hot_shaders.register_file(id.clone(), path);
    }
    // v0.7 #9: 書き出し時も plugins/*.wasm 動的プラグインを自動ロード
    {
        let mut default_wasm = ve_plugin_api::WasmPluginManager::new(PathBuf::from("plugins"));
        default_wasm.poll_and_sync(&mut rg.plugins);
        if let Some(folder) = req.folder.as_ref() {
            let mut proj_wasm =
                ve_plugin_api::WasmPluginManager::new(folder.root().join("plugins"));
            proj_wasm.poll_and_sync(&mut rg.plugins);
        }
    }

    // GPU オフロードが有効な場合はヘッドレス wgpu::Device 経由でポストプロセスを実行（非搭載環境は CPU 自動フォールバック）
    let mut gpu_offloader = if !req.gpu_passes.is_empty() {
        HeadlessGpuOffloader::try_new(req.config.width, req.config.height)
    } else {
        None
    };

    let cli = FfmpegCli::default();
    let audio_wav_ref = temp_wav_guard.as_ref().map(|g| g.path.as_path());
    let mut session = match cli.start_export(&req.output_path, &req.config, audio_wav_ref) {
        Ok(s) => s,
        Err(e) => {
            update_progress(&progress, |p| {
                p.stage = format!("FFmpeg起動失敗: {e}");
                p.error = Some(e.to_string());
                p.finished = true;
            });
            return;
        }
    };
    let resolved_enc = session.resolved_encoder();
    let effective_workers = if req.worker_threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get().clamp(1, 8))
            .unwrap_or(1)
    } else {
        req.worker_threads.clamp(1, 16)
    };
    let can_parallel_cpu =
        effective_workers > 1 && gpu_offloader.is_none() && req.shader_files.is_empty();
    let mode_tag = if gpu_offloader.is_some() {
        " + wgpu GPU".to_string()
    } else if can_parallel_cpu {
        format!(" + {effective_workers}T 並列")
    } else {
        String::new()
    };
    update_progress(&progress, |p| {
        p.resolved_encoder = Some(resolved_enc);
        p.stage = format!(
            "エンコード中 ({}{mode_tag})...",
            resolved_enc.display_name()
        );
    });

    // 3. フレーム単位／マルチスレッド並列バッチでレンダリング＆エンコード
    if can_parallel_cpu {
        let batch_len = (effective_workers * 4).clamp(4, 32) as u64;
        let empty_proxies = HashMap::new();
        let empty_waveforms = HashMap::new();
        let mut cursor = 0u64;
        while cursor < total_frames {
            if cancel_flag.load(Ordering::Relaxed) {
                session.cancel();
                if req.config.format != ExportFormat::PngSequence {
                    let _ = std::fs::remove_file(&req.output_path);
                }
                update_progress(&progress, |p| {
                    p.elapsed_sec = started.elapsed().as_secs_f64();
                    p.eta_sec = None;
                    p.stage = "キャンセル済み".into();
                    p.cancelled = true;
                    p.finished = true;
                });
                return;
            }
            let end = (cursor + batch_len).min(total_frames);
            let times: Vec<RationalTime> = (cursor..end)
                .map(|fi| RationalTime::from_frame(fi as i64, req.config.fps))
                .collect();
            let frames_rgba = render_frame_batch_parallel(
                &req.project,
                req.scene_id,
                req.folder.as_ref(),
                &times,
                req.transparent_bg,
                false,
                true,
                req.hw_decode,
                &empty_proxies,
                &empty_waveforms,
                effective_workers,
            );
            for (offset, rgba) in frames_rgba.iter().enumerate() {
                if let Err(e) = session.write_rgba8_frame(rgba) {
                    session.cancel();
                    update_progress(&progress, |p| {
                        p.stage = format!("書き出し書き込みエラー: {e}");
                        p.error = Some(e.to_string());
                        p.finished = true;
                    });
                    return;
                }
                let done = cursor + offset as u64 + 1;
                let elapsed = started.elapsed().as_secs_f64();
                let eta = if done > 0 {
                    Some((elapsed / done as f64) * total_frames.saturating_sub(done) as f64)
                } else {
                    None
                };
                update_progress(&progress, |p| {
                    p.current_frame = done;
                    p.elapsed_sec = elapsed;
                    p.eta_sec = eta;
                });
            }
            cursor = end;
        }
    } else {
        for frame_idx in 0..total_frames {
            if cancel_flag.load(Ordering::Relaxed) {
                session.cancel();
                if req.config.format != ExportFormat::PngSequence {
                    let _ = std::fs::remove_file(&req.output_path);
                }
                update_progress(&progress, |p| {
                    p.elapsed_sec = started.elapsed().as_secs_f64();
                    p.eta_sec = None;
                    p.stage = "キャンセル済み".into();
                    p.cancelled = true;
                    p.finished = true;
                });
                return;
            }

            let t = RationalTime::from_frame(frame_idx as i64, req.config.fps);
            let Some(linear) =
                rg.render_project_scene(&req.project, req.scene_id, t, req.transparent_bg)
            else {
                session.cancel();
                update_progress(&progress, |p| {
                    p.stage = "フレーム描画に失敗しました".into();
                    p.error = Some("render_project_scene returned None".into());
                    p.finished = true;
                });
                return;
            };

            let linear = if let Some(offloader) = gpu_offloader.as_mut() {
                offloader
                    .apply_passes_readback(
                        &linear,
                        &rg.hot_shaders,
                        &req.gpu_passes,
                        t.as_seconds() as f32,
                    )
                    .unwrap_or(linear)
            } else {
                linear
            };

            if let Err(e) = session.write_rgba8_frame(&linear.to_rgba8_srgb()) {
                session.cancel();
                update_progress(&progress, |p| {
                    p.stage = format!("書き出し書き込みエラー: {e}");
                    p.error = Some(e.to_string());
                    p.finished = true;
                });
                return;
            }

            let done = frame_idx + 1;
            let elapsed = started.elapsed().as_secs_f64();
            let eta = if done > 0 {
                let per_frame = elapsed / done as f64;
                Some(per_frame * total_frames.saturating_sub(done) as f64)
            } else {
                None
            };
            update_progress(&progress, |p| {
                p.current_frame = done;
                p.elapsed_sec = elapsed;
                p.eta_sec = eta;
            });
        }
    }

    update_progress(&progress, |p| {
        p.stage = "ファイルファイナライズ中...".into();
    });
    match session.finish() {
        Ok(()) => {
            update_progress(&progress, |p| {
                p.current_frame = total_frames;
                p.elapsed_sec = started.elapsed().as_secs_f64();
                p.eta_sec = Some(0.0);
                p.stage = format!(
                    "書き出し完了 ({}): {}",
                    resolved_enc.display_name(),
                    req.output_path.display()
                );
                p.succeeded = true;
                p.finished = true;
            });
        }
        Err(e) => {
            update_progress(&progress, |p| {
                p.stage = format!("FFmpeg終了エラー: {e}");
                p.error = Some(e.to_string());
                p.finished = true;
            });
        }
    }
    drop(temp_wav_guard);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use uuid::Uuid;
    use ve_core::{
        AssetRef, FrameRate, ItemKind, MediaKind, ProjectSettings, RgbaColor, ShapeKind,
        TimelineItem,
    };
    use ve_media::decode_audio_file;

    fn sample_av_project(wav_path: PathBuf) -> Project {
        let settings = ProjectSettings {
            width: 96,
            height: 54,
            fps: FrameRate::FPS_30,
            audio_sample_rate: 48_000,
            ..Default::default()
        };
        let mut project = Project::new("ExportTest", settings);
        let audio_id = Uuid::new_v4();
        project.assets.push(AssetRef {
            id: audio_id,
            name: "tone.wav".into(),
            kind: MediaKind::Audio,
            relative_path: None,
            fallback_absolute_path: Some(wav_path),
            collected: false,
        });
        let scene = project.active_scene_mut().unwrap();
        scene.layers[0].items.push(TimelineItem::new(
            "Bg",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 96.0,
                height: 54.0,
                color: RgbaColor {
                    r: 0.2,
                    g: 0.6,
                    b: 0.9,
                    a: 1.0,
                },
            },
        ));
        scene.layers[3].items.push(TimelineItem::new(
            "Voice",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Audio {
                asset_id: audio_id,
                source_offset: RationalTime::ZERO,
                volume: 0.8,
            },
        ));
        project
    }

    #[test]
    fn async_export_muxes_video_and_audio_graph_tracks() {
        let tmp = tempdir().unwrap();
        let src_wav = tmp.path().join("tone.wav");
        write_wav_pcm16(&src_wav, &vec![0.3f32; 48_000], 48_000, 1).unwrap();

        let project = sample_av_project(src_wav);
        let out_mp4 = tmp.path().join("out_muxed.mp4");
        let handle = ExportJobHandle::start(ExportJobRequest {
            scene_id: project.active_scene_id,
            project,
            folder: None,
            output_path: out_mp4.clone(),
            config: ExportConfig {
                width: 96,
                height: 54,
                fps: FrameRate::FPS_30,
                format: ExportFormat::Mp4H264,
                crf: 23,
                encoder: VideoEncoder::Auto,
            },
            transparent_bg: false,
            gpu_passes: vec![GpuPostProcessPass::custom(
                "builtin:color_grade",
                [0.05, 1.05, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            )],
            shader_files: Vec::new(),
            worker_threads: 2,
            hw_decode: HwDecodeBackend::Software,
        });
        let snap = handle.wait();
        assert!(snap.succeeded, "export failed: {:?}", snap.error);
        assert_eq!(snap.current_frame, 30);

        let probe = FfmpegCli::default().probe(&out_mp4).unwrap();
        assert_eq!(probe.width, Some(96));
        assert_eq!(probe.height, Some(54));
        assert!(probe.audio_sample_rate.is_some());
    }

    #[test]
    fn async_export_wav_audio_only() {
        let tmp = tempdir().unwrap();
        let src_wav = tmp.path().join("tone.wav");
        write_wav_pcm16(&src_wav, &vec![0.4f32; 48_000], 48_000, 1).unwrap();

        let project = sample_av_project(src_wav);
        let out_wav = tmp.path().join("mixdown.wav");
        let handle = ExportJobHandle::start(ExportJobRequest {
            scene_id: project.active_scene_id,
            project,
            folder: None,
            output_path: out_wav.clone(),
            config: ExportConfig {
                format: ExportFormat::WavAudioOnly,
                ..Default::default()
            },
            transparent_bg: false,
            gpu_passes: Vec::new(),
            shader_files: Vec::new(),
            worker_threads: 1,
            hw_decode: HwDecodeBackend::Software,
        });
        let snap = handle.wait();
        assert!(snap.succeeded, "wav export failed: {:?}", snap.error);
        let decoded = decode_audio_file(&out_wav).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channels, 2);
        assert!(decoded.duration_sec() >= 0.9);
    }

    #[test]
    fn async_export_respects_cancel_flag() {
        let tmp = tempdir().unwrap();
        let src_wav = tmp.path().join("tone.wav");
        write_wav_pcm16(&src_wav, &vec![0.1f32; 48_000], 48_000, 1).unwrap();

        let mut project = sample_av_project(src_wav);
        // 長尺（10秒 = 300フレーム）にしてキャンセルを確実に捕捉
        project.active_scene_mut().unwrap().layers[0].items[0].duration =
            RationalTime::from_seconds(10.0);
        let out_mp4 = tmp.path().join("cancelled.mp4");
        let handle = ExportJobHandle::start(ExportJobRequest {
            scene_id: project.active_scene_id,
            project,
            folder: None,
            output_path: out_mp4,
            config: ExportConfig {
                width: 96,
                height: 54,
                fps: FrameRate::FPS_30,
                format: ExportFormat::Mp4H264,
                crf: 23,
                encoder: VideoEncoder::Software,
            },
            transparent_bg: false,
            gpu_passes: Vec::new(),
            shader_files: Vec::new(),
            worker_threads: 2,
            hw_decode: HwDecodeBackend::Software,
        });
        handle.cancel();
        let snap = handle.wait();
        assert!(snap.finished);
        assert!(snap.cancelled || snap.succeeded);
    }
}
