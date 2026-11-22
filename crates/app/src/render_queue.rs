use crate::exporter::{ExportJobHandle, ExportJobRequest};
use egui::{Color32, Context, RichText};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use ve_core::{
    CommandBus, CommandSource, EditCommand, Project, ProjectFolder, SceneId, export_ass_subtitles,
    export_otio_json, export_srt_subtitles,
};
use ve_mcp::EditorSession;
use ve_media::{ExportConfig, ExportFormat, VideoEncoder};
use ve_render::GpuPostProcessPass;

/// v0.6 #7: レンダーキューで実行するジョブ種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueJobKind {
    Mp4H264,
    Mp4Shorts9x16,
    Mp4H265,
    WebmAlpha,
    WavAudioOnly,
    SubtitlesSrt,
    SubtitlesAss,
    OpenTimelineIo,
}

impl QueueJobKind {
    pub const ALL: [Self; 8] = [
        Self::Mp4H264,
        Self::Mp4Shorts9x16,
        Self::Mp4H265,
        Self::WebmAlpha,
        Self::WavAudioOnly,
        Self::SubtitlesSrt,
        Self::SubtitlesAss,
        Self::OpenTimelineIo,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Mp4H264 => "MP4 (H.264 / 現在解像度)",
            Self::Mp4Shorts9x16 => "MP4 (9:16 縦動画 自動リフレーム)",
            Self::Mp4H265 => "MP4 (H.265 高圧縮)",
            Self::WebmAlpha => "WebM (VP9 透過アルファ)",
            Self::WavAudioOnly => "WAV (48kHz 音声のみ)",
            Self::SubtitlesSrt => "SRT 字幕ファイル (.srt)",
            Self::SubtitlesAss => "ASS スタイル付字幕 (.ass)",
            Self::OpenTimelineIo => "OpenTimelineIO (.otio)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4H264 | Self::Mp4Shorts9x16 | Self::Mp4H265 => "mp4",
            Self::WebmAlpha => "webm",
            Self::WavAudioOnly => "wav",
            Self::SubtitlesSrt => "srt",
            Self::SubtitlesAss => "ass",
            Self::OpenTimelineIo => "otio",
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::Mp4H264 => "h264",
            Self::Mp4Shorts9x16 => "shorts_9x16",
            Self::Mp4H265 => "h265",
            Self::WebmAlpha => "alpha",
            Self::WavAudioOnly => "audio",
            Self::SubtitlesSrt => "subtitles",
            Self::SubtitlesAss => "subtitles",
            Self::OpenTimelineIo => "timeline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueJobStatus {
    Pending,
    Running,
    Succeeded,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct RenderQueueItem {
    pub id: Uuid,
    pub label: String,
    pub scene_id: SceneId,
    pub kind: QueueJobKind,
    pub output_path: PathBuf,
    pub status: QueueJobStatus,
    pub progress: f32,
    pub detail: String,
}

/// v0.6 #7: マルチジョブ・バッチ連続書き出しレンダーキュー
pub struct RenderQueueState {
    pub jobs: Vec<RenderQueueItem>,
    pub running: bool,
    pub new_job_kind: QueueJobKind,
    pub new_job_stem: String,
    pub encoder: VideoEncoder,
    active_job_id: Option<Uuid>,
    active_handle: Option<ExportJobHandle>,
}

impl Default for RenderQueueState {
    fn default() -> Self {
        Self {
            jobs: Vec::new(),
            running: false,
            new_job_kind: QueueJobKind::Mp4H264,
            new_job_stem: "output".to_string(),
            encoder: VideoEncoder::Auto,
            active_job_id: None,
            active_handle: None,
        }
    }
}

impl RenderQueueState {
    pub fn sanitize_stem(raw: &str) -> String {
        let s: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .take(48)
            .collect();
        if s.is_empty() {
            "output".to_string()
        } else {
            s
        }
    }

    pub fn enqueue(
        &mut self,
        scene_id: SceneId,
        scene_name: &str,
        kind: QueueJobKind,
        output_path: PathBuf,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.jobs.push(RenderQueueItem {
            id,
            label: format!("{} [{}]", scene_name, kind.display_name()),
            scene_id,
            kind,
            output_path,
            status: QueueJobStatus::Pending,
            progress: 0.0,
            detail: "待機中".to_string(),
        });
        id
    }

    /// 1クリックで「本編MP4 + Shorts 9:16 + WAV + SRT字幕 + OTIO」を一括登録
    pub fn enqueue_youtube_bundle(
        &mut self,
        scene_id: SceneId,
        scene_name: &str,
        out_dir: &Path,
        stem: &str,
    ) {
        let safe = Self::sanitize_stem(stem);
        for kind in [
            QueueJobKind::Mp4H264,
            QueueJobKind::Mp4Shorts9x16,
            QueueJobKind::WavAudioOnly,
            QueueJobKind::SubtitlesSrt,
            QueueJobKind::OpenTimelineIo,
        ] {
            let path = out_dir.join(format!("{safe}_{}.{}", kind.suffix(), kind.extension()));
            self.enqueue(scene_id, scene_name, kind, path);
        }
    }

    pub fn pending_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.status == QueueJobStatus::Pending)
            .count()
    }

    pub fn start_all(&mut self) {
        if self.pending_count() > 0 || self.active_handle.is_some() {
            self.running = true;
        }
    }

    pub fn cancel_running(&mut self) {
        self.running = false;
        if let Some(handle) = self.active_handle.as_ref() {
            handle.cancel();
        }
        if let Some(active_id) = self.active_job_id
            && let Some(job) = self.jobs.iter_mut().find(|j| j.id == active_id)
            && job.status == QueueJobStatus::Running
        {
            job.status = QueueJobStatus::Cancelled;
            job.detail = "キャンセル要求中...".to_string();
        }
    }

    pub fn clear_finished(&mut self) {
        self.jobs
            .retain(|j| matches!(j.status, QueueJobStatus::Pending | QueueJobStatus::Running));
    }

    pub fn move_up(&mut self, idx: usize) {
        if idx > 0
            && idx < self.jobs.len()
            && self.jobs[idx].status == QueueJobStatus::Pending
            && self.jobs[idx - 1].status == QueueJobStatus::Pending
        {
            self.jobs.swap(idx, idx - 1);
        }
    }

    pub fn move_down(&mut self, idx: usize) {
        if idx + 1 < self.jobs.len()
            && self.jobs[idx].status == QueueJobStatus::Pending
            && self.jobs[idx + 1].status == QueueJobStatus::Pending
        {
            self.jobs.swap(idx, idx + 1);
        }
    }

    pub fn remove_job(&mut self, idx: usize) {
        if idx < self.jobs.len() && self.jobs[idx].status != QueueJobStatus::Running {
            self.jobs.remove(idx);
        }
    }

    /// 毎フレーム呼び出して進行中ジョブをポーリングし、完了したら次の待機ジョブを順次起動する。
    /// 実行中（再描画が必要）の場合は `true` を返す。
    pub fn tick(
        &mut self,
        project: &Project,
        folder: Option<&ProjectFolder>,
        gpu_passes: &[GpuPostProcessPass],
        shader_files: &[(String, PathBuf)],
    ) -> bool {
        // 1. 実行中のバックグラウンド書き出しをポーリング
        if let Some(handle) = self.active_handle.as_ref() {
            let snap = handle.snapshot();
            if let Some(active_id) = self.active_job_id
                && let Some(job) = self.jobs.iter_mut().find(|j| j.id == active_id)
            {
                job.progress = snap.fraction();
                job.detail = snap.stage.clone();
                if snap.finished {
                    if snap.succeeded {
                        job.progress = 1.0;
                        job.status = QueueJobStatus::Succeeded;
                    } else if snap.cancelled {
                        job.status = QueueJobStatus::Cancelled;
                    } else {
                        job.status = QueueJobStatus::Failed(
                            snap.error.unwrap_or_else(|| snap.stage.clone()),
                        );
                    }
                }
            }
            if snap.finished {
                self.active_handle = None;
                self.active_job_id = None;
            } else {
                return true;
            }
        }

        if !self.running {
            return false;
        }

        // 2. 次の待機中ジョブを起動（軽量テキスト系は即時実行し、動画/音声は ExportJobHandle へ委譲）
        loop {
            let Some(next_idx) = self
                .jobs
                .iter()
                .position(|j| j.status == QueueJobStatus::Pending)
            else {
                self.running = false;
                return false;
            };

            let (job_id, scene_id, kind, output_path) = {
                let j = &mut self.jobs[next_idx];
                j.status = QueueJobStatus::Running;
                j.progress = 0.0;
                j.detail = "開始中...".to_string();
                (j.id, j.scene_id, j.kind, j.output_path.clone())
            };

            match kind {
                QueueJobKind::SubtitlesSrt
                | QueueJobKind::SubtitlesAss
                | QueueJobKind::OpenTimelineIo => {
                    let result = run_instant_text_export(project, scene_id, kind, &output_path);
                    let j = &mut self.jobs[next_idx];
                    match result {
                        Ok(msg) => {
                            j.progress = 1.0;
                            j.status = QueueJobStatus::Succeeded;
                            j.detail = msg;
                        }
                        Err(e) => {
                            j.status = QueueJobStatus::Failed(e.clone());
                            j.detail = format!("失敗: {e}");
                        }
                    }
                    // 連続して次のジョブへ進む
                    continue;
                }
                QueueJobKind::Mp4H264
                | QueueJobKind::Mp4Shorts9x16
                | QueueJobKind::Mp4H265
                | QueueJobKind::WebmAlpha
                | QueueJobKind::WavAudioOnly => {
                    let mut job_project = project.clone();
                    if kind == QueueJobKind::Mp4Shorts9x16 {
                        job_project.active_scene_id = scene_id;
                        if let Some(fps) = job_project.scene(scene_id).map(|s| s.fps) {
                            let mut bus = CommandBus::default();
                            let _ = bus.execute(
                                &mut job_project,
                                EditCommand::SetSceneSettings {
                                    width: 1080,
                                    height: 1920,
                                    fps,
                                    rescale_items: true,
                                },
                                CommandSource::Gui,
                                None,
                            );
                        }
                    }
                    let Some(scene) = job_project.scene(scene_id) else {
                        let j = &mut self.jobs[next_idx];
                        j.status = QueueJobStatus::Failed("対象シーンが存在しません".into());
                        j.detail = "対象シーンなし".into();
                        continue;
                    };
                    let (format, transparent_bg) = match kind {
                        QueueJobKind::Mp4H264 | QueueJobKind::Mp4Shorts9x16 => {
                            (ExportFormat::Mp4H264, false)
                        }
                        QueueJobKind::Mp4H265 => (ExportFormat::Mp4H265, false),
                        QueueJobKind::WebmAlpha => (ExportFormat::WebmVp9Alpha, true),
                        QueueJobKind::WavAudioOnly => (ExportFormat::WavAudioOnly, false),
                        _ => unreachable!(),
                    };
                    let req = ExportJobRequest {
                        config: ExportConfig {
                            width: scene.width.max(16),
                            height: scene.height.max(16),
                            fps: scene.fps,
                            format,
                            crf: 18,
                            encoder: self.encoder,
                        },
                        project: job_project,
                        scene_id,
                        folder: folder.cloned(),
                        output_path,
                        transparent_bg,
                        gpu_passes: gpu_passes.to_vec(),
                        shader_files: shader_files.to_vec(),
                        worker_threads: 0,
                        hw_decode: ve_media::HwDecodeBackend::Auto,
                    };
                    self.active_job_id = Some(job_id);
                    self.active_handle = Some(ExportJobHandle::start(req));
                    return true;
                }
            }
        }
    }

    pub fn show_window(&mut self, ctx: &Context, open: &mut bool, session: &EditorSession) {
        let mut window_open = *open;
        egui::Window::new("📋 レンダーキュー（バッチ連続書き出し v0.6）")
            .open(&mut window_open)
            .resizable(true)
            .default_width(640.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                let out_dir = session
                    .folder
                    .as_ref()
                    .map(|f| f.exports_dir())
                    .unwrap_or_else(|| PathBuf::from("exports"));
                let (scene_id, scene_name) = session
                    .project
                    .active_scene()
                    .map(|s| (s.id, s.name.clone()))
                    .unwrap_or_else(|| (session.project.active_scene_id, "Main".to_string()));

                ui.horizontal_wrapped(|ui| {
                    ui.label("形式:");
                    egui::ComboBox::from_id_salt("rq_kind_combo")
                        .selected_text(self.new_job_kind.display_name())
                        .show_ui(ui, |ui| {
                            for kind in QueueJobKind::ALL {
                                ui.selectable_value(
                                    &mut self.new_job_kind,
                                    kind,
                                    kind.display_name(),
                                );
                            }
                        });
                    ui.label("ファイル名:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_job_stem)
                            .desired_width(110.0)
                            .hint_text("output"),
                    );
                    if ui.button("＋ キュー追加").clicked() {
                        let safe = Self::sanitize_stem(&self.new_job_stem);
                        let path = out_dir.join(format!(
                            "{safe}_{}.{}",
                            self.new_job_kind.suffix(),
                            self.new_job_kind.extension()
                        ));
                        self.enqueue(scene_id, &scene_name, self.new_job_kind, path);
                    }
                    if ui
                        .button("📦 YouTube一括セット追加 (16:9+9:16+WAV+SRT+OTIO)")
                        .clicked()
                    {
                        self.enqueue_youtube_bundle(
                            scene_id,
                            &scene_name,
                            &out_dir,
                            &self.new_job_stem.clone(),
                        );
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    ui.label("エンコーダ:");
                    egui::ComboBox::from_id_salt("rq_encoder_combo")
                        .selected_text(self.encoder.display_name())
                        .show_ui(ui, |ui| {
                            for enc in [
                                VideoEncoder::Auto,
                                VideoEncoder::Software,
                                VideoEncoder::Nvenc,
                                VideoEncoder::Qsv,
                                VideoEncoder::Amf,
                            ] {
                                ui.selectable_value(&mut self.encoder, enc, enc.display_name());
                            }
                        });
                    ui.separator();
                    if self.running {
                        if ui.button("⏹ 連続書き出し停止").clicked() {
                            self.cancel_running();
                        }
                    } else if ui
                        .add_enabled(
                            self.pending_count() > 0,
                            egui::Button::new(format!(
                                "▶ 連続書き出し開始 ({}件待機)",
                                self.pending_count()
                            )),
                        )
                        .clicked()
                    {
                        self.start_all();
                    }
                    if ui.button("🧹 完了済みを消去").clicked() {
                        self.clear_finished();
                    }
                });

                ui.weak(format!("出力先フォルダ: {}", out_dir.display()));
                ui.separator();

                let total = self.jobs.len();
                let done = self
                    .jobs
                    .iter()
                    .filter(|j| j.status == QueueJobStatus::Succeeded)
                    .count();
                if total > 0 {
                    ui.add(
                        egui::ProgressBar::new(done as f32 / total as f32)
                            .text(format!("全体進捗: {done}/{total} 完了")),
                    );
                } else {
                    ui.weak("キューは空です。上のボタンからジョブを追加してください。");
                }

                ui.add_space(4.0);
                let mut action_up: Option<usize> = None;
                let mut action_down: Option<usize> = None;
                let mut action_del: Option<usize> = None;

                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for (idx, job) in self.jobs.iter().enumerate() {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (badge, color) = match &job.status {
                                        QueueJobStatus::Pending => {
                                            ("⏳ 待機", Color32::from_gray(190))
                                        }
                                        QueueJobStatus::Running => {
                                            ("⚙ 実行中", Color32::from_rgb(110, 200, 255))
                                        }
                                        QueueJobStatus::Succeeded => {
                                            ("✅ 完了", Color32::from_rgb(100, 230, 130))
                                        }
                                        QueueJobStatus::Failed(_) => {
                                            ("❌ 失敗", Color32::from_rgb(245, 110, 110))
                                        }
                                        QueueJobStatus::Cancelled => {
                                            ("⏹ 中止", Color32::from_rgb(235, 190, 90))
                                        }
                                    };
                                    ui.label(RichText::new(badge).color(color).strong());
                                    ui.strong(&job.label);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if job.status != QueueJobStatus::Running
                                                && ui.small_button("✖").clicked()
                                            {
                                                action_del = Some(idx);
                                            }
                                            if job.status == QueueJobStatus::Pending {
                                                if ui.small_button("↓").clicked() {
                                                    action_down = Some(idx);
                                                }
                                                if ui.small_button("↑").clicked() {
                                                    action_up = Some(idx);
                                                }
                                            }
                                        },
                                    );
                                });
                                ui.weak(job.output_path.display().to_string());
                                ui.add(
                                    egui::ProgressBar::new(job.progress)
                                        .desired_height(14.0)
                                        .text(&job.detail),
                                );
                            });
                        }
                    });

                if let Some(i) = action_up {
                    self.move_up(i);
                }
                if let Some(i) = action_down {
                    self.move_down(i);
                }
                if let Some(i) = action_del {
                    self.remove_job(i);
                }
            });
        *open = window_open;
    }
}

fn run_instant_text_export(
    project: &Project,
    scene_id: SceneId,
    kind: QueueJobKind,
    output_path: &Path,
) -> Result<String, String> {
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let payload = match kind {
        QueueJobKind::SubtitlesSrt => {
            let scene = project
                .scene(scene_id)
                .ok_or("対象シーンが見つかりません")?;
            export_srt_subtitles(scene)
        }
        QueueJobKind::SubtitlesAss => {
            let scene = project
                .scene(scene_id)
                .ok_or("対象シーンが見つかりません")?;
            export_ass_subtitles(scene)
        }
        QueueJobKind::OpenTimelineIo => {
            export_otio_json(project, scene_id).ok_or("OTIO エクスポートに失敗しました")?
        }
        _ => return Err("unexpected instant job kind".into()),
    };
    std::fs::write(output_path, payload).map_err(|e| e.to_string())?;
    Ok(format!("書き出し完了: {}", output_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use ve_core::{ItemKind, ProjectSettings, RationalTime, RgbaColor, TextAnimator, TimelineItem};

    #[test]
    fn render_queue_executes_srt_ass_otio_and_wav_batch() {
        let tmp = tempfile::tempdir().unwrap();
        let mut proj = Project::new("BatchQueueTest", ProjectSettings::default());
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .push(TimelineItem::new(
                "Sub1",
                RationalTime::ZERO,
                RationalTime::from_seconds(1.2),
                ItemKind::Text {
                    content: "レンダーキュー一括テスト".into(),
                    font_family: "sans-serif".into(),
                    font_size: 44.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));

        let sid = proj.active_scene_id;
        let mut queue = RenderQueueState::default();
        let srt_p = tmp.path().join("out.srt");
        let ass_p = tmp.path().join("out.ass");
        let otio_p = tmp.path().join("out.otio");
        let wav_p = tmp.path().join("out.wav");

        queue.enqueue(sid, "Main", QueueJobKind::SubtitlesSrt, srt_p.clone());
        queue.enqueue(sid, "Main", QueueJobKind::SubtitlesAss, ass_p.clone());
        queue.enqueue(sid, "Main", QueueJobKind::OpenTimelineIo, otio_p.clone());
        queue.enqueue(sid, "Main", QueueJobKind::WavAudioOnly, wav_p.clone());
        queue.start_all();

        let deadline = Instant::now() + Duration::from_secs(10);
        while queue.tick(&proj, None, &[], &[]) {
            assert!(Instant::now() < deadline, "render queue timed out");
            std::thread::sleep(Duration::from_millis(15));
        }

        assert!(!queue.running);
        assert!(
            queue
                .jobs
                .iter()
                .all(|j| j.status == QueueJobStatus::Succeeded),
            "jobs={:?}",
            queue.jobs
        );
        assert!(
            std::fs::read_to_string(&srt_p)
                .unwrap()
                .contains("レンダーキュー一括テスト")
        );
        assert!(
            std::fs::read_to_string(&ass_p)
                .unwrap()
                .contains("[V4+ Styles]")
        );
        assert!(
            std::fs::read_to_string(&otio_p)
                .unwrap()
                .contains("Timeline.1")
        );
        assert!(std::fs::metadata(&wav_p).unwrap().len() > 44);
    }
}
