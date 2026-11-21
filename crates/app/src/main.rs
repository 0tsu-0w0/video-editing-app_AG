mod asset_browser;
mod audio_ai_panel;
mod exporter;
mod font_picker;
mod graph_editor;
mod ime_poc;
mod inspector_extras;
mod project_io;
mod timeline;
mod watch_folder;

use asset_browser::AssetBrowserPanel;
use audio_ai_panel::AudioAi3DPanel;
use eframe::egui_wgpu;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use exporter::{ExportJobHandle, ExportJobRequest, ExportProgressSnapshot};
use font_picker::FontPickerState;
use graph_editor::GraphEditorState;
use ime_poc::ImePocPanel;
use inspector_extras::InspectorExtrasState;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use timeline::{TimelinePlaybackAction, TimelineState};
use uuid::Uuid;
use ve_audio::{AudioEngineState, AudioGraph, WaveformPeaks};
use ve_core::{
    AssetResolution, BlendMode, CommandBus, CommandSource, CubicBezier, EditCommand, EffectNode,
    FilterTargetScope, FrameRate, HistoryEntry, Interpolation, ItemKind, Keyframe, Project,
    ProjectSettings, RationalTime, RgbaColor, ShapeKind, TextAnimator, TimelineItem, TrackMatte,
    TrackMatteMode, Transform2D,
};
use ve_mcp::{EditorSession, EmbeddedMcpServer, SharedEditorSession};
use ve_media::{ExportConfig, ExportFormat, VideoEncoder};
use ve_render::{
    GpuPostProcessPass, RenderGraph, ShaderReloadStatus, WgpuPreviewPipeline, register_shader_dir,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceMode {
    /// メイン編集空間（AviUtl2ライク: カット・配置中心）
    MainEdit,
    /// モーション合成空間（AEライク: グラフエディタ・マット・エフェクトチェーン）
    MotionComposite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Preview,
    Timeline,
    GraphEditor,
    Inspector,
    AssetBrowser,
    AudioAi3D,
}

fn build_dock_state(mode: WorkspaceMode) -> DockState<Pane> {
    match mode {
        WorkspaceMode::MainEdit => {
            let mut dock = DockState::new(vec![Pane::Preview]);
            let surface = dock.main_surface_mut();
            let [top, _bottom] =
                surface.split_below(egui_dock::NodeIndex::root(), 0.56, vec![Pane::Timeline]);
            let [_left, _right] = surface.split_right(
                top,
                0.64,
                vec![Pane::Inspector, Pane::AssetBrowser, Pane::AudioAi3D],
            );
            dock
        }
        WorkspaceMode::MotionComposite => {
            let mut dock = DockState::new(vec![Pane::Preview]);
            let surface = dock.main_surface_mut();
            let [top, bottom] =
                surface.split_below(egui_dock::NodeIndex::root(), 0.50, vec![Pane::Timeline]);
            let [_left, _right] = surface.split_right(
                top,
                0.58,
                vec![Pane::Inspector, Pane::AssetBrowser, Pane::AudioAi3D],
            );
            let [_tl, _ge] = surface.split_right(bottom, 0.52, vec![Pane::GraphEditor]);
            dock
        }
    }
}

struct EditorApp {
    workspace_mode: WorkspaceMode,
    session: SharedEditorSession,
    mcp_server: Option<EmbeddedMcpServer>,
    mcp_copy_status: String,
    timeline_state: TimelineState,
    graph_editor: GraphEditorState,
    asset_browser: AssetBrowserPanel,
    font_picker: FontPickerState,
    inspector_extras: InspectorExtrasState,
    audio_ai_panel: AudioAi3DPanel,
    waveforms: BTreeMap<Uuid, WaveformPeaks>,
    render_graph: RenderGraph,
    preview_pipeline: Option<WgpuPreviewPipeline>,
    preview_tex_id: Option<egui::TextureId>,
    audio_state: Arc<AudioEngineState>,
    #[cfg(not(windows))]
    meter_scratch: Vec<f32>,
    #[cfg(windows)]
    _live_audio: Option<ve_audio::output::live::LiveAudioStream>,
    dock_state: DockState<Pane>,
    ime_panel: ImePocPanel,
    relink_inputs: BTreeMap<Uuid, String>,
    custom_wgsl_path_input: String,
    custom_wgsl_msg: String,
    // v0.4 #5: wgpu GPU オフロード＆カスタム WGSL ポストプロセス設定
    gpu_offload_enabled: bool,
    gpu_active_custom_shader: Option<String>,
    gpu_custom_strength: f32,
    gpu_exposure: f32,
    gpu_contrast: f32,
    gpu_saturation: f32,
    gpu_blur_radius: f32,
    gpu_glow_intensity: f32,
    project_dir_input: String,
    project_status: String,
    pending_recovery: Vec<HistoryEntry>,
    proxy_tx: mpsc::Sender<(Uuid, PathBuf)>,
    proxy_rx: mpsc::Receiver<(Uuid, PathBuf)>,
    export_format: ExportFormat,
    export_encoder: VideoEncoder,
    export_job: Option<ExportJobHandle>,
    last_export_snapshot: Option<ExportProgressSnapshot>,
    export_status: String,
    preview_scale: u32,
    last_rendered_key: Option<(u64, i64, u32, usize, Uuid)>,
    preview_dirty: bool,
    last_shader_poll: std::time::Instant,
}

impl EditorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = ProjectSettings {
            width: 1280,
            height: 720,
            fps: FrameRate::FPS_60,
            ..Default::default()
        };

        let mut project = Project::new("Phase 2 Hybrid Motion Project", settings.clone());
        let mut initial_selected = Vec::new();
        if let Some(scene) = project.active_scene_mut() {
            let mut bg = TimelineItem::new(
                "背景パネル",
                RationalTime::ZERO,
                RationalTime::from_seconds(5.0),
                ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: 1120.0,
                    height: 560.0,
                    color: RgbaColor {
                        r: 0.12,
                        g: 0.18,
                        b: 0.28,
                        a: 1.0,
                    },
                },
            );
            bg.transform = Transform2D {
                x: 80.0,
                y: 80.0,
                ..Default::default()
            };
            scene.layers[0].items.push(bg);

            let mut title = TimelineItem::new(
                "タイトル字幕",
                RationalTime::ZERO,
                RationalTime::from_seconds(5.0),
                ItemKind::Text {
                    content: "次世代ハイブリッド動画編集ソフト\nPhase 2 AE融合＆ワンクリック結合"
                        .to_string(),
                    font_family: "sans-serif".to_string(),
                    font_size: 52.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator {
                        enabled: true,
                        ..Default::default()
                    },
                },
            );
            title.transform = Transform2D {
                x: 120.0,
                y: 250.0,
                ..Default::default()
            };
            title.keyframes.insert(
                "y".to_string(),
                vec![
                    Keyframe {
                        time: RationalTime::ZERO,
                        value: 290.0,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                    },
                    Keyframe {
                        time: RationalTime::from_seconds(1.0),
                        value: 250.0,
                        interpolation: Interpolation::Linear,
                    },
                ],
            );
            initial_selected.push(title.id);
            scene.layers[1].items.push(title);

            let mut params = BTreeMap::new();
            params.insert("gain".to_string(), 1.15);
            scene.layers[2].items.push(TimelineItem::new(
                "明るさフィルタ (AviUtl2方式)",
                RationalTime::ZERO,
                RationalTime::from_seconds(5.0),
                ItemKind::FilterObject {
                    effect_type: "brightness".to_string(),
                    scope: FilterTargetScope::ImmediateLayer,
                    params,
                },
            ));
        }

        let mut editor_session = EditorSession::new(project);
        editor_session.selected_items = initial_selected;
        let session = Arc::new(Mutex::new(editor_session));
        let mcp_server = EmbeddedMcpServer::start(Arc::clone(&session)).ok();

        let audio_state = AudioEngineState::new(settings.audio_sample_rate);
        #[cfg(windows)]
        let live_audio = ve_audio::output::live::start_live_output(Arc::clone(&audio_state)).ok();

        let (preview_pipeline, preview_tex_id) =
            if let Some(wgpu_state) = cc.wgpu_render_state.as_ref() {
                let pipe = WgpuPreviewPipeline::new(&wgpu_state.device, 640, 360);
                let tex_id = wgpu_state.renderer.write().register_native_texture(
                    &wgpu_state.device,
                    &pipe.output_view,
                    wgpu::FilterMode::Linear,
                );
                (Some(pipe), Some(tex_id))
            } else {
                (None, None)
            };

        let workspace_mode = WorkspaceMode::MainEdit;
        let dock_state = build_dock_state(workspace_mode);
        let mut render_graph = RenderGraph::new();
        let loaded_shaders = register_shader_dir(
            &mut render_graph.hot_shaders,
            std::path::Path::new("shaders"),
        );

        let (proxy_tx, proxy_rx) = mpsc::channel();

        Self {
            workspace_mode,
            session,
            mcp_server,
            mcp_copy_status: String::new(),
            timeline_state: TimelineState::default(),
            graph_editor: GraphEditorState::default(),
            asset_browser: AssetBrowserPanel::new(),
            font_picker: FontPickerState::new(),
            inspector_extras: InspectorExtrasState::default(),
            audio_ai_panel: AudioAi3DPanel::default(),
            waveforms: BTreeMap::new(),
            render_graph,
            preview_pipeline,
            preview_tex_id,
            audio_state,
            #[cfg(not(windows))]
            meter_scratch: vec![0.0f32; 512 * 2],
            #[cfg(windows)]
            _live_audio: live_audio,
            dock_state,
            ime_panel: ImePocPanel::new(),
            relink_inputs: BTreeMap::new(),
            custom_wgsl_path_input: "shaders/custom_chromatic_aberration.wgsl".to_string(),
            custom_wgsl_msg: format!("監視中 WGSL: {loaded_shaders} 件"),
            gpu_offload_enabled: true,
            gpu_active_custom_shader: None,
            gpu_custom_strength: 0.008,
            gpu_exposure: 0.0,
            gpu_contrast: 1.0,
            gpu_saturation: 1.0,
            gpu_blur_radius: 0.0,
            gpu_glow_intensity: 0.0,
            project_dir_input: "./MyVideoProject".to_string(),
            project_status: "未保存プロジェクト（D&Dで素材読込可）".to_string(),
            pending_recovery: Vec::new(),
            proxy_tx,
            proxy_rx,
            export_format: ExportFormat::Mp4H264,
            export_encoder: VideoEncoder::Auto,
            export_job: None,
            last_export_snapshot: None,
            export_status: "待機中".to_string(),
            preview_scale: 2,
            last_rendered_key: None,
            preview_dirty: true,
            last_shader_poll: std::time::Instant::now(),
        }
    }

    /// 現在有効な wgpu Ping-Pong ポストプロセス＆カスタム WGSL パス列を構築する
    fn build_active_gpu_passes(&self) -> Vec<GpuPostProcessPass> {
        if !self.gpu_offload_enabled {
            return Vec::new();
        }
        let mut passes = Vec::new();
        if self.gpu_exposure.abs() > 0.001
            || (self.gpu_contrast - 1.0).abs() > 0.001
            || (self.gpu_saturation - 1.0).abs() > 0.001
        {
            passes.push(GpuPostProcessPass::custom(
                "builtin:color_grade",
                [
                    self.gpu_exposure,
                    self.gpu_contrast,
                    self.gpu_saturation,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
            ));
        }
        if self.gpu_blur_radius > 0.05 {
            passes.push(GpuPostProcessPass::custom(
                "builtin:blur",
                [self.gpu_blur_radius, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ));
        }
        if self.gpu_glow_intensity > 0.01 {
            passes.push(GpuPostProcessPass::custom(
                "builtin:glow",
                [3.0, 0.55, self.gpu_glow_intensity, 0.0, 0.0, 0.0, 0.0, 0.0],
            ));
        }
        if let Some(ref sid) = self.gpu_active_custom_shader {
            passes.push(GpuPostProcessPass::custom(
                sid.clone(),
                [self.gpu_custom_strength, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ));
        }
        passes
    }

    fn switch_workspace(&mut self, mode: WorkspaceMode) {
        if self.workspace_mode != mode {
            self.workspace_mode = mode;
            self.dock_state = build_dock_state(mode);
        }
    }

    /// 再生開始前にタイムライン上の Audio / Video クリップから AudioGraph を最新同期する
    fn sync_live_audio_graph(&self) {
        let (project, scene_id, folder) = {
            let sess = self.session.lock().unwrap();
            (
                sess.project.clone(),
                sess.project.active_scene_id,
                sess.folder.clone(),
            )
        };
        let sr = project.settings.audio_sample_rate.max(8_000);
        let built = AudioGraph::from_project_scene(&project, scene_id, folder.as_ref(), sr);
        if let Ok(mut graph) = self.audio_state.graph.write() {
            *graph = built;
        }
    }

    fn handle_new_project_folder(&mut self) {
        let dir = PathBuf::from(self.project_dir_input.trim());
        if dir.as_os_str().is_empty() {
            self.project_status = "プロジェクトフォルダパスを入力してください".into();
            return;
        }
        let mut sess = self.session.lock().unwrap();
        match project_io::create_new_project_folder(&dir, &sess.project) {
            Ok(folder) => {
                sess.bus = CommandBus::with_folder(folder.clone());
                sess.folder = Some(folder.clone());
                self.render_graph.set_project_folder(Some(folder.clone()));
                self.asset_browser.sync_with_project_folder(&folder);
                self.pending_recovery.clear();
                self.preview_dirty = true;
                self.project_status = format!("新規プロジェクト作成: {}", folder.root().display());
            }
            Err(e) => self.project_status = format!("作成失敗: {e}"),
        }
    }

    fn handle_open_project_folder(&mut self) {
        let dir = PathBuf::from(self.project_dir_input.trim());
        match project_io::open_project_folder(&dir) {
            Ok(opened) => {
                let mut sess = self.session.lock().unwrap();
                sess.project = opened.project;
                sess.bus = CommandBus::with_folder(opened.folder.clone());
                sess.folder = Some(opened.folder.clone());
                sess.selected_items.clear();
                sess.playhead = RationalTime::ZERO;
                self.render_graph.clear_media_caches();
                self.render_graph
                    .set_project_folder(Some(opened.folder.clone()));
                self.asset_browser.sync_with_project_folder(&opened.folder);
                self.pending_recovery = opened.pending_recovery;
                self.preview_dirty = true;
                self.project_status = if self.pending_recovery.is_empty() {
                    format!("読込完了: {}", dir.display())
                } else {
                    format!(
                        "読込完了（未反映履歴 {} 件あり — 上部バナーから復旧可能）",
                        self.pending_recovery.len()
                    )
                };
            }
            Err(e) => self.project_status = format!("読込失敗: {e}"),
        }
    }

    fn handle_save_project(&mut self) {
        let sess = self.session.lock().unwrap();
        match project_io::save_project_snapshot(&sess) {
            Ok(path) => {
                self.pending_recovery.clear();
                self.project_status = format!("保存完了: {}", path.display());
            }
            Err(e) => self.project_status = format!("保存失敗: {e}"),
        }
    }

    fn handle_collect_assets(&mut self) {
        let mut sess = self.session.lock().unwrap();
        match project_io::collect_project_assets(&mut sess) {
            Ok((ok, offline)) => {
                self.preview_dirty = true;
                self.project_status =
                    format!("プロジェクト収集完了: 成功 {ok} 件 / オフライン {offline} 件");
            }
            Err(e) => self.project_status = format!("収集失敗: {e}"),
        }
    }

    fn start_background_export(&mut self) {
        if self.export_job.as_ref().is_some_and(|j| !j.is_finished()) {
            return;
        }
        let (proj_clone, folder) = {
            let guard = self.session.lock().unwrap();
            (guard.project.clone(), guard.folder.clone())
        };
        let active_id = proj_clone.active_scene_id;
        let out_dir = folder
            .as_ref()
            .map(|f| f.exports_dir())
            .unwrap_or_else(|| PathBuf::from("."));
        let (file_name, transparent_bg) = match self.export_format {
            ExportFormat::Mp4H264 => ("export_h264.mp4", false),
            ExportFormat::Mp4H265 => ("export_h265.mp4", false),
            ExportFormat::WebmVp9Alpha => ("export_alpha.webm", true),
            ExportFormat::PngSequence => ("export_frame_%04d.png", true),
            ExportFormat::WavAudioOnly => ("export_audio.wav", false),
        };
        let output_path = out_dir.join(file_name);

        let (width, height, fps) = proj_clone
            .scene(active_id)
            .map(|s| (s.width.max(16), s.height.max(16), s.fps))
            .unwrap_or((1280, 720, FrameRate::FPS_60));

        let config = ExportConfig {
            width,
            height,
            fps,
            format: self.export_format,
            crf: 18,
            encoder: self.export_encoder,
        };
        let gpu_passes = self.build_active_gpu_passes();
        let shader_files: Vec<(String, PathBuf)> = self
            .render_graph
            .hot_shaders
            .list()
            .into_iter()
            .map(|e| (e.id.clone(), e.path.clone()))
            .collect();

        self.export_job = Some(ExportJobHandle::start(ExportJobRequest {
            project: proj_clone,
            scene_id: active_id,
            folder,
            output_path,
            config,
            transparent_bg,
            gpu_passes,
            shader_files,
        }));
        self.export_status = "バックグラウンド書き出し開始...".to_string();
    }
}

struct EditorTabViewer<'a> {
    app: &'a mut EditorApp,
}

impl TabViewer for EditorTabViewer<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::NULL.with(*tab as u8)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Pane::Preview => "プレビュー (wgpu Linear f16)".into(),
            Pane::Timeline => "タイムライン".into(),
            Pane::GraphEditor => "グラフエディタ (値 / 速度)".into(),
            Pane::Inspector => match self.app.workspace_mode {
                WorkspaceMode::MainEdit => "インスペクタ / MCP / 日本語IME".into(),
                WorkspaceMode::MotionComposite => "AEモーション・エフェクト・マット".into(),
            },
            Pane::AssetBrowser => "🗂 アセット・エイリアス・監視".into(),
            Pane::AudioAi3D => "🎙 音声自動化・立ち絵・3D".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Pane::Preview => {
                if let Some(tex_id) = self.app.preview_tex_id {
                    let avail = ui.available_size();
                    let aspect = 16.0 / 9.0;
                    let w = avail.x.min(avail.y * aspect).max(160.0);
                    let h = w / aspect;
                    ui.centered_and_justified(|ui| {
                        ui.image(egui::load::SizedTexture::new(tex_id, egui::vec2(w, h)));
                    });
                } else {
                    ui.label("wgpu プレビューテクスチャを初期化できませんでした。");
                }
            }
            Pane::Timeline => {
                let mut export_clicked = false;
                let mut cancel_export_clicked = false;
                let mut play_started = false;
                let export_snap = self.app.last_export_snapshot.clone();
                let is_exporting = self
                    .app
                    .export_job
                    .as_ref()
                    .is_some_and(|j| !j.is_finished());
                let space_pressed = !ui.ctx().egui_wants_keyboard_input()
                    && ui.input(|i| !i.modifiers.command && i.key_pressed(egui::Key::Space));
                {
                    let mut sess = self.app.session.lock().unwrap();
                    let fps = sess.project.settings.fps;
                    let frame = sess.playhead.to_frame_floor(fps);
                    let secs = sess.playhead.as_seconds();

                    ui.horizontal_wrapped(|ui| {
                        let (is_playing, cur_rate) = {
                            let c = self.app.audio_state.clock.read().unwrap();
                            (c.is_playing(), c.playback_rate())
                        };
                        if ui
                            .button(if is_playing {
                                "⏸ 一時停止 (Space)"
                            } else {
                                "▶ 再生 (Space)"
                            })
                            .clicked()
                            || space_pressed
                        {
                            let mut clock = self.app.audio_state.clock.write().unwrap();
                            if is_playing {
                                clock.pause();
                                clock.set_playback_rate(1.0);
                            } else {
                                clock.set_playback_rate(1.0);
                                clock.seek(sess.playhead);
                                clock.play();
                                play_started = true;
                            }
                        }
                        if is_playing && (cur_rate - 1.0).abs() > 0.05 {
                            let badge = if cur_rate < 0.0 {
                                format!("⏪ {:.0}x", cur_rate)
                            } else {
                                format!("⏩ {:.0}x", cur_rate)
                            };
                            ui.colored_label(egui::Color32::from_rgb(255, 212, 82), badge);
                        }
                        if ui.button("⏮ 先頭").clicked() {
                            sess.playhead = RationalTime::ZERO;
                            let mut clock = self.app.audio_state.clock.write().unwrap();
                            clock.set_playback_rate(1.0);
                            clock.seek(RationalTime::ZERO);
                        }
                        ui.monospace(format!(
                            "{secs:.3}s | Frame {frame} ({:.2}fps)",
                            fps.as_f64()
                        ));
                        inspector_extras::draw_audio_meter_bar(
                            ui,
                            self.app.audio_state.meter.load(),
                        );
                        ui.separator();

                        let fmt_label = match self.app.export_format {
                            ExportFormat::Mp4H264 => "MP4 (H.264)",
                            ExportFormat::Mp4H265 => "MP4 (H.265)",
                            ExportFormat::WebmVp9Alpha => "WebM (VP9+透過)",
                            ExportFormat::PngSequence => "PNG連番 (透過)",
                            ExportFormat::WavAudioOnly => "WAV (音声のみ)",
                        };
                        egui::ComboBox::from_id_salt("export_format_combo")
                            .selected_text(fmt_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.app.export_format,
                                    ExportFormat::Mp4H264,
                                    "MP4 (H.264)",
                                );
                                ui.selectable_value(
                                    &mut self.app.export_format,
                                    ExportFormat::Mp4H265,
                                    "MP4 (H.265)",
                                );
                                ui.selectable_value(
                                    &mut self.app.export_format,
                                    ExportFormat::WebmVp9Alpha,
                                    "WebM (VP9+透過)",
                                );
                                ui.selectable_value(
                                    &mut self.app.export_format,
                                    ExportFormat::PngSequence,
                                    "PNG連番 (透過)",
                                );
                                ui.selectable_value(
                                    &mut self.app.export_format,
                                    ExportFormat::WavAudioOnly,
                                    "WAV (音声のみ)",
                                );
                            });

                        egui::ComboBox::from_id_salt("export_encoder_combo")
                            .selected_text(self.app.export_encoder.display_name())
                            .show_ui(ui, |ui| {
                                for enc in [
                                    VideoEncoder::Auto,
                                    VideoEncoder::Software,
                                    VideoEncoder::Nvenc,
                                    VideoEncoder::Qsv,
                                    VideoEncoder::Amf,
                                ] {
                                    ui.selectable_value(
                                        &mut self.app.export_encoder,
                                        enc,
                                        enc.display_name(),
                                    );
                                }
                            });

                        if is_exporting {
                            if let Some(ref snap) = export_snap {
                                let eta_text = snap
                                    .eta_sec
                                    .map(|e| format!(" / 残り {e:.1}s"))
                                    .unwrap_or_default();
                                ui.add(
                                    egui::ProgressBar::new(snap.fraction())
                                        .desired_width(180.0)
                                        .text(format!(
                                            "{:.0}% ({}/{}){}",
                                            snap.fraction() * 100.0,
                                            snap.current_frame,
                                            snap.total_frames,
                                            eta_text
                                        )),
                                );
                            }
                            if ui.button("✖ キャンセル").clicked() {
                                cancel_export_clicked = true;
                            }
                        } else if ui.button("🚀 書き出し").clicked() {
                            export_clicked = true;
                        }
                        ui.label(&self.app.export_status);
                    });

                    ui.separator();
                    let pb_action =
                        self.app
                            .timeline_state
                            .show(ui, &mut sess, &self.app.waveforms);

                    match pb_action {
                        TimelinePlaybackAction::ShuttleReverse => {
                            let mut clock = self.app.audio_state.clock.write().unwrap();
                            let next_rate = if clock.is_playing() && clock.playback_rate() < 0.0 {
                                match clock.playback_rate().round() as i32 {
                                    -1 => -2.0,
                                    _ => -4.0,
                                }
                            } else {
                                -1.0
                            };
                            clock.set_playback_rate(next_rate);
                            if !clock.is_playing() {
                                clock.seek(sess.playhead);
                                clock.play();
                                play_started = true;
                            }
                        }
                        TimelinePlaybackAction::ShuttlePause => {
                            let mut clock = self.app.audio_state.clock.write().unwrap();
                            clock.pause();
                            clock.set_playback_rate(1.0);
                            clock.seek(sess.playhead);
                        }
                        TimelinePlaybackAction::ShuttleForward => {
                            let mut clock = self.app.audio_state.clock.write().unwrap();
                            let next_rate = if clock.is_playing() && clock.playback_rate() > 0.0 {
                                match clock.playback_rate().round() as i32 {
                                    1 => 2.0,
                                    _ => 4.0,
                                }
                            } else {
                                1.0
                            };
                            clock.set_playback_rate(next_rate);
                            if !clock.is_playing() {
                                clock.seek(sess.playhead);
                                clock.play();
                                play_started = true;
                            }
                        }
                        TimelinePlaybackAction::None => {
                            if !self.app.audio_state.clock.read().unwrap().is_playing() {
                                self.app
                                    .audio_state
                                    .clock
                                    .write()
                                    .unwrap()
                                    .seek(sess.playhead);
                            }
                        }
                    }
                }
                if play_started {
                    self.app.sync_live_audio_graph();
                }
                if export_clicked {
                    self.app.start_background_export();
                }
                if cancel_export_clicked && let Some(job) = self.app.export_job.as_ref() {
                    job.cancel();
                }
            }
            Pane::GraphEditor => {
                let mut sess = self.app.session.lock().unwrap();
                self.app.graph_editor.show(ui, &mut sess);
            }
            Pane::Inspector => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // 内蔵 MCP ステータス
                    ui.heading("内蔵 MCP (Streamable HTTP)");
                    if let Some(mcp) = &self.app.mcp_server {
                        let url = mcp.endpoint_url();
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(&url);
                            if ui.button("URL").clicked() {
                                ui.ctx().copy_text(url);
                                self.app.mcp_copy_status = "MCP URLをコピーしました".into();
                            }
                            if ui.button("トークン").clicked() {
                                ui.ctx().copy_text(mcp.bearer_token().to_string());
                                self.app.mcp_copy_status = "Bearerトークンをコピーしました".into();
                            }
                            if ui
                                .button("📋 MCPクライアント設定JSONをコピー")
                                .on_hover_text(
                                    "Claude Desktop / Cursor / Codex 等へそのまま貼り付け可能な JSON 設定をコピー",
                                )
                                .clicked()
                            {
                                ui.ctx().copy_text(mcp.client_config_json());
                                self.app.mcp_copy_status =
                                    "MCPクライアント設定JSONをクリップボードへコピーしました"
                                        .into();
                            }
                        });
                        if !self.app.mcp_copy_status.is_empty() {
                            ui.colored_label(
                                egui::Color32::LIGHT_GREEN,
                                &self.app.mcp_copy_status,
                            );
                        }
                    }
                    ui.separator();

                    // 選択中アイテムの詳細（ブレンドモード / 文字ごとアニメーター / エフェクトチェーン）
                    let selected_snapshot = {
                        let sess = self.app.session.lock().unwrap();
                        sess.selected_items.first().copied().and_then(|sel_id| {
                            sess.project.active_scene().and_then(|scene| {
                                scene.layers.iter().enumerate().find_map(|(l_idx, layer)| {
                                    layer
                                        .items
                                        .iter()
                                        .find(|i| i.id == sel_id)
                                        .map(|it| (l_idx, layer.track_matte, it.clone()))
                                })
                            })
                        })
                    };

                    if let Some((layer_idx, track_matte, item)) = selected_snapshot {
                        ui.heading(format!("選択中: {}", item.name));

                        // ブレンドモード
                        let mut blend = item.blend_mode;
                        egui::ComboBox::from_label("ブレンドモード")
                            .selected_text(format!("{blend:?}"))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut blend, BlendMode::Normal, "Normal");
                                ui.selectable_value(&mut blend, BlendMode::Add, "Add");
                                ui.selectable_value(&mut blend, BlendMode::Multiply, "Multiply");
                                ui.selectable_value(&mut blend, BlendMode::Screen, "Screen");
                            });
                        if blend != item.blend_mode {
                            let mut g = self.app.session.lock().unwrap();
                            let s = &mut *g;
                            let _ = s.bus.execute(
                                &mut s.project,
                                EditCommand::SetBlendMode {
                                    item_id: item.id,
                                    blend_mode: blend,
                                },
                                CommandSource::Gui,
                                None,
                            );
                        }

                        // レイヤーのトラックマット設定
                        let mut matte_choice = track_matte.map(|m| m.mode);
                        egui::ComboBox::from_label(format!("L{layer_idx} トラックマット"))
                            .selected_text(
                                matte_choice
                                    .map(|m| format!("{m:?}"))
                                    .unwrap_or_else(|| "なし".into()),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut matte_choice, None, "なし");
                                ui.selectable_value(
                                    &mut matte_choice,
                                    Some(TrackMatteMode::Alpha),
                                    "Alpha (上レイヤー)",
                                );
                                ui.selectable_value(
                                    &mut matte_choice,
                                    Some(TrackMatteMode::AlphaInverted),
                                    "Alpha反転",
                                );
                                ui.selectable_value(
                                    &mut matte_choice,
                                    Some(TrackMatteMode::Luma),
                                    "Luma (輝度)",
                                );
                                ui.selectable_value(
                                    &mut matte_choice,
                                    Some(TrackMatteMode::LumaInverted),
                                    "Luma反転",
                                );
                            });
                        if matte_choice != track_matte.map(|m| m.mode) {
                            let mut g = self.app.session.lock().unwrap();
                            let s = &mut *g;
                            let new_matte = matte_choice.map(|mode| TrackMatte {
                                mode,
                                source_layer_index: layer_idx + 1,
                            });
                            let _ = s.bus.execute(
                                &mut s.project,
                                EditCommand::SetTrackMatte {
                                    layer_index: layer_idx,
                                    matte: new_matte,
                                },
                                CommandSource::Gui,
                                None,
                            );
                        }

                        // A-1: 名前付きカラーパレット（複数選択オブジェクト一括色変更対応）
                        {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_color_palette_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                            );
                        }

                        // A-4: シーンチェンジ（トランジション）設定
                        if let ItemKind::Transition {
                            transition,
                            softness,
                            ..
                        } = item.kind
                        {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_transition_ui(
                                ui, &mut g, item.id, transition, softness,
                            );
                        }

                        // A-5: 動画・音声の再生速度・逆再生・ループ・時間制御
                        {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_playback_speed_ui(ui, &mut g, &item);
                        }

                        // A-6 + B-7: 映像エフェクト追加・並び替え・コピペ・実務頻出フィルタ・演出プリセット
                        {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_video_effects_and_presets_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                                &item,
                            );
                        }

                        // Phase 4: プラグイン API 経由のエフェクト追加（PluginRegistry 動的列挙）
                        let manifests = self.app.render_graph.plugins.list_manifests();
                        ui.horizontal_wrapped(|ui| {
                            ui.label("プラグイン適用:");
                            for m in manifests {
                                if ui.button(&m.name).clicked() {
                                    let mut params = BTreeMap::new();
                                    for p in &m.params {
                                        params.insert(p.key.clone(), p.default_value);
                                    }
                                    let mut g = self.app.session.lock().unwrap();
                                    let s = &mut *g;
                                    let _ = s.bus.execute(
                                        &mut s.project,
                                        EditCommand::AddEffect {
                                            item_id: item.id,
                                            effect: EffectNode {
                                                id: Uuid::new_v4(),
                                                effect_type: m.id.clone(),
                                                enabled: true,
                                                params,
                                            },
                                        },
                                        CommandSource::Gui,
                                        None,
                                    );
                                }
                            }
                        });

                        // テキストオブジェクトの場合はフォント視覚化ピッカー・字幕装飾・文字ごとアニメーター・制御文字補完＋日本語IME
                        if let ItemKind::Text {
                            mut content,
                            font_family,
                            mut font_size,
                            mut animator,
                            ..
                        } = item.kind.clone()
                        {
                            ui.separator();
                            let prev_size = font_size;
                            ui.horizontal(|ui| {
                                ui.label(format!("フォント: {font_family}"));
                                ui.add(
                                    egui::Slider::new(&mut font_size, 10.0..=180.0).text("サイズ"),
                                );
                            });
                            if (font_size - prev_size).abs() > 0.05 {
                                let mut g = self.app.session.lock().unwrap();
                                let s = &mut *g;
                                let _ = s.bus.execute(
                                    &mut s.project,
                                    EditCommand::SetTextFont {
                                        item_id: item.id,
                                        font_family: font_family.clone(),
                                        font_size,
                                    },
                                    CommandSource::Gui,
                                    Some("text_font_size"),
                                );
                            }

                            // B-2: フォント視覚化ピッカー＆お気に入りフォント
                            ui.collapsing(
                                "🔤 フォント視覚化ピッカー（★お気に入り対応）",
                                |ui| {
                                    let mut g = self.app.session.lock().unwrap();
                                    self.app.font_picker.show(
                                        ui,
                                        &mut g,
                                        self.app.render_graph.text_renderer_mut(),
                                        item.id,
                                        &font_family,
                                        font_size,
                                    );
                                },
                            );

                            // 字幕リッチ装飾（縁取り・ドロップシャドウ・座布団背景）
                            {
                                let mut g = self.app.session.lock().unwrap();
                                inspector_extras::show_text_style_ui(
                                    ui,
                                    &mut g,
                                    item.id,
                                    item.text_style,
                                );
                            }

                            let prev_anim = animator;
                            ui.checkbox(&mut animator.enabled, "文字ごとアニメーター有効");
                            if animator.enabled {
                                ui.add(
                                    egui::Slider::new(&mut animator.delay_per_char_sec, 0.0..=0.3)
                                        .text("文字ディレイ(秒)"),
                                );
                                ui.add(
                                    egui::Slider::new(&mut animator.start_offset_y, -80.0..=80.0)
                                        .text("出現Yオフセット"),
                                );
                            }
                            if animator != prev_anim {
                                let mut g = self.app.session.lock().unwrap();
                                let s = &mut *g;
                                let _ = s.bus.execute(
                                    &mut s.project,
                                    EditCommand::SetTextAnimator {
                                        item_id: item.id,
                                        animator,
                                    },
                                    CommandSource::Gui,
                                    Some("text_animator_edit"),
                                );
                            }

                            ui.separator();
                            // A-2: AviUtl2 インライン制御文字＆Ctrl+Space補完
                            let before = content.clone();
                            {
                                let g = self.app.session.lock().unwrap();
                                inspector_extras::show_inline_tag_helper(
                                    ui,
                                    &mut self.app.inspector_extras,
                                    &g,
                                    &mut content,
                                );
                            }
                            self.app.ime_panel.ui(ui, &mut content);
                            if content != before {
                                let mut g = self.app.session.lock().unwrap();
                                let s = &mut *g;
                                let _ = s.bus.execute(
                                    &mut s.project,
                                    EditCommand::SetTextContent {
                                        item_id: item.id,
                                        content,
                                    },
                                    CommandSource::Gui,
                                    Some("inspector_text_edit"),
                                );
                            }
                        }

                        // B-3: 音声DSPエフェクト（クリップ＆マスター）＋ LUFS ラウドネス正規化
                        {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_audio_dsp_and_lufs_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                                Some(&item),
                            );
                        }
                    } else {
                        // 未選択時でもカラーパレットとマスター音声DSP/LUFSは操作可能
                        {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_color_palette_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                            );
                            inspector_extras::show_audio_dsp_and_lufs_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                                None,
                            );
                        }
                        let mut dummy =
                            String::from("（タイムラインでオブジェクトを選択してください）");
                        self.app.ime_panel.ui(ui, &mut dummy);
                    }

                    ui.separator();
                    ui.heading("アセット管理・オフライン再リンク");
                    let mut sess = self.app.session.lock().unwrap();
                    if sess.project.assets.is_empty() {
                        ui.weak("外部参照アセットはまだありません。");
                    } else {
                        let folder = sess.folder.clone();
                        for asset in &mut sess.project.assets {
                            let status = folder
                                .as_ref()
                                .map(|f| f.resolve_asset(asset))
                                .unwrap_or_else(|| {
                                    if asset
                                        .fallback_absolute_path
                                        .as_ref()
                                        .is_some_and(|p| p.exists())
                                    {
                                        AssetResolution::Found(
                                            asset.fallback_absolute_path.clone().unwrap(),
                                        )
                                    } else {
                                        AssetResolution::Offline
                                    }
                                });
                            ui.horizontal(|ui| {
                                ui.label(&asset.name);
                                match status {
                                    AssetResolution::Found(_) => {
                                        ui.colored_label(egui::Color32::GREEN, "OK");
                                    }
                                    AssetResolution::Offline => {
                                        ui.colored_label(egui::Color32::RED, "OFFLINE");
                                        let entry =
                                            self.app.relink_inputs.entry(asset.id).or_default();
                                        ui.text_edit_singleline(entry);
                                        if ui.button("再リンク").clicked() && !entry.is_empty()
                                        {
                                            asset.fallback_absolute_path =
                                                Some(PathBuf::from(entry.trim()));
                                            self.app.preview_dirty = true;
                                        }
                                    }
                                }
                            });
                        }
                    }
                    ui.separator();
                    ui.heading("⚡ wgpu GPU オフロード＆カスタム WGSL ホットリロード (v0.4)");
                    let compiled_custom_cnt = self
                        .app
                        .preview_pipeline
                        .as_ref()
                        .map(|p| p.compiled_custom_shader_count())
                        .unwrap_or(0);
                    let mut gpu_changed = false;
                    gpu_changed |= ui
                        .checkbox(
                            &mut self.app.gpu_offload_enabled,
                            format!(
                                "Rgba16Float Ping-Pong GPU パイプライン有効（コンパイル済カスタム: {compiled_custom_cnt}）"
                            ),
                        )
                        .changed();
                    if self.app.gpu_offload_enabled {
                        let shader_entries = self.app.render_graph.hot_shaders.list();
                        let cur_shader_label = self
                            .app
                            .gpu_active_custom_shader
                            .clone()
                            .unwrap_or_else(|| "なし（組み込みのみ）".into());
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_label("アクティブ WGSL")
                                .selected_text(cur_shader_label)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_value(
                                            &mut self.app.gpu_active_custom_shader,
                                            None,
                                            "なし（組み込みのみ）",
                                        )
                                        .changed()
                                    {
                                        gpu_changed = true;
                                    }
                                    for entry in &shader_entries {
                                        if entry.fragment_entry_point.is_some()
                                            && ui
                                                .selectable_value(
                                                    &mut self.app.gpu_active_custom_shader,
                                                    Some(entry.id.clone()),
                                                    &entry.id,
                                                )
                                                .changed()
                                        {
                                            gpu_changed = true;
                                        }
                                    }
                                });
                            if ui.small_button("リセット").clicked() {
                                self.app.gpu_active_custom_shader = None;
                                self.app.gpu_custom_strength = 0.008;
                                self.app.gpu_exposure = 0.0;
                                self.app.gpu_contrast = 1.0;
                                self.app.gpu_saturation = 1.0;
                                self.app.gpu_blur_radius = 0.0;
                                self.app.gpu_glow_intensity = 0.0;
                                gpu_changed = true;
                            }
                        });
                        if self.app.gpu_active_custom_shader.is_some() {
                            gpu_changed |= ui
                                .add(
                                    egui::Slider::new(&mut self.app.gpu_custom_strength, 0.0..=0.08)
                                        .fixed_decimals(4)
                                        .text("カスタム強度 (p0)"),
                                )
                                .changed();
                        }
                        gpu_changed |= ui
                            .add(
                                egui::Slider::new(&mut self.app.gpu_exposure, -2.0..=2.0)
                                    .text("GPU 露出 (EV)"),
                            )
                            .changed();
                        gpu_changed |= ui
                            .add(
                                egui::Slider::new(&mut self.app.gpu_contrast, 0.2..=2.5)
                                    .text("GPU コントラスト"),
                            )
                            .changed();
                        gpu_changed |= ui
                            .add(
                                egui::Slider::new(&mut self.app.gpu_saturation, 0.0..=2.5)
                                    .text("GPU 彩度"),
                            )
                            .changed();
                        gpu_changed |= ui
                            .add(
                                egui::Slider::new(&mut self.app.gpu_blur_radius, 0.0..=12.0)
                                    .text("GPU 2-Pass ブラー"),
                            )
                            .changed();
                        gpu_changed |= ui
                            .add(
                                egui::Slider::new(&mut self.app.gpu_glow_intensity, 0.0..=3.0)
                                    .text("GPU グロー強度"),
                            )
                            .changed();
                    }
                    if gpu_changed {
                        self.app.preview_dirty = true;
                    }

                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.app.custom_wgsl_path_input);
                        if ui.button("監視登録").clicked()
                            && !self.app.custom_wgsl_path_input.trim().is_empty()
                        {
                            let p = PathBuf::from(self.app.custom_wgsl_path_input.trim());
                            let id = p
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "custom".into());
                            match self.app.render_graph.hot_shaders.register_file(id, &p) {
                                Ok(e) => {
                                    self.app.custom_wgsl_msg =
                                        format!("登録完了: {} (rev {})", e.id, e.revision);
                                    self.app.preview_dirty = true;
                                }
                                Err(err) => {
                                    self.app.custom_wgsl_msg = err;
                                }
                            }
                        }
                        if ui.button("今すぐ検証").clicked() {
                            let reloaded = self.app.render_graph.hot_shaders.poll_reload();
                            self.app.custom_wgsl_msg = if reloaded.is_empty() {
                                "変更なし".into()
                            } else {
                                self.app.preview_dirty = true;
                                format!("ホットリロード完了: {}", reloaded.join(", "))
                            };
                        }
                    });
                    ui.weak(&self.app.custom_wgsl_msg);
                    for entry in self.app.render_graph.hot_shaders.list() {
                        match &entry.status {
                            ShaderReloadStatus::Loaded {
                                revision,
                                entry_points,
                            } => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_GREEN,
                                    format!("✔ {} (rev {}) {:?}", entry.id, revision, entry_points),
                                );
                            }
                            ShaderReloadStatus::Error {
                                last_good_revision,
                                message,
                            } => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_RED,
                                    format!(
                                        "✖ {} (rev {} 維持): {}",
                                        entry.id, last_good_revision, message
                                    ),
                                );
                            }
                        }
                    }
                });
            }
            Pane::AssetBrowser => {
                let mut sess = self.app.session.lock().unwrap();
                if self.app.asset_browser.show(
                    ui,
                    &mut sess,
                    &mut self.app.waveforms,
                    Some(&self.app.proxy_tx),
                    &self.app.audio_state,
                ) {
                    self.app.preview_dirty = true;
                }
            }
            Pane::AudioAi3D => {
                let mut sess = self.app.session.lock().unwrap();
                self.app
                    .audio_ai_panel
                    .show(ui, &mut sess, &mut self.app.waveforms);
            }
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ime_panel.capture_events(&ctx);

        // バックグラウンド生成された軽量プロキシを RenderGraph へ登録
        while let Ok((aid, proxy_path)) = self.proxy_rx.try_recv() {
            self.render_graph.register_proxy(aid, proxy_path);
            self.preview_dirty = true;
            self.project_status = format!(
                "軽量プロキシ生成完了 (登録済み: {} 件)",
                self.render_graph.proxy_count()
            );
        }

        // バックグラウンド書き出しジョブの進捗ポーリング
        if let Some(job) = self.export_job.as_ref() {
            let snap = job.snapshot();
            self.export_status = snap.stage.clone();
            let finished = snap.finished;
            self.last_export_snapshot = Some(snap);
            if finished {
                self.export_job = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
        }

        // グローバル ドラッグ＆ドロップ素材自動取込（動画 / 音声 / 画像 / PSD / TTSペア）
        let dropped_paths: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped_paths.is_empty() {
            let mut sess = self.session.lock().unwrap();
            let summary = project_io::import_dropped_files(
                &dropped_paths,
                &mut sess,
                &mut self.waveforms,
                Some(&self.proxy_tx),
            );
            if summary.imported_count > 0 {
                self.preview_dirty = true;
            }
            if !summary.messages.is_empty() {
                self.project_status = summary.messages.join(" / ");
            }
        }

        // B-6: かんしくん風 フォルダ監視による新着ファイル自動配置（再生ヘッド自動送り対応）
        if self.asset_browser.is_watching() {
            let arrived = self.asset_browser.poll_watch_folder();
            if !arrived.is_empty() {
                let mut sess = self.session.lock().unwrap();
                for path in arrived {
                    let summary = project_io::import_dropped_files(
                        std::slice::from_ref(&path),
                        &mut sess,
                        &mut self.waveforms,
                        Some(&self.proxy_tx),
                    );
                    if summary.imported_count > 0 {
                        self.preview_dirty = true;
                        if self.asset_browser.watch_auto_advance
                            && let Some(end_t) = summary.max_end
                        {
                            sess.playhead = end_t;
                        }
                    }
                    if !summary.messages.is_empty() {
                        self.project_status = format!("[監視] {}", summary.messages.join(" / "));
                    }
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // カスタム WGSL シェーダーの自動ホットリロードポーリング（500ms 間隔にスロットルして毎フレームI/Oを回避）
        if self.last_shader_poll.elapsed() >= std::time::Duration::from_millis(500) {
            self.last_shader_poll = std::time::Instant::now();
            let reloaded = self.render_graph.hot_shaders.poll_reload();
            if !reloaded.is_empty() {
                self.custom_wgsl_msg = format!("自動ホットリロード: {}", reloaded.join(", "));
                self.preview_dirty = true;
            }
        }

        // 上部バー: Dual-Workspace UX 切替 + プロジェクト開閉・保存・収集 + シーン切替
        egui::Panel::top("workspace_switcher_bar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("空間切替:");
                if ui
                    .selectable_label(
                        self.workspace_mode == WorkspaceMode::MainEdit,
                        "🎬 メイン編集空間 (AviUtl2)",
                    )
                    .clicked()
                {
                    self.switch_workspace(WorkspaceMode::MainEdit);
                }
                if ui
                    .selectable_label(
                        self.workspace_mode == WorkspaceMode::MotionComposite,
                        "✨ モーション合成空間 (AE)",
                    )
                    .clicked()
                {
                    self.switch_workspace(WorkspaceMode::MotionComposite);
                }

                ui.separator();
                ui.label("シーン:");
                let (scenes, active_id) = {
                    let s = self.session.lock().unwrap();
                    let list: Vec<(Uuid, String)> = s
                        .project
                        .scenes
                        .iter()
                        .map(|sc| (sc.id, sc.name.clone()))
                        .collect();
                    (list, s.project.active_scene_id)
                };
                for (sid, sname) in scenes {
                    if ui.selectable_label(sid == active_id, sname).clicked() && sid != active_id {
                        let mut g = self.session.lock().unwrap();
                        let s = &mut *g;
                        let _ = s.bus.execute(
                            &mut s.project,
                            EditCommand::SwitchScene { scene_id: sid },
                            CommandSource::Gui,
                            None,
                        );
                        s.selected_items.clear();
                    }
                }

                ui.separator();
                if ui
                    .checkbox(&mut self.render_graph.use_proxies, "軽量プロキシ優先")
                    .changed()
                {
                    self.preview_dirty = true;
                }
                ui.weak(format!(
                    "Cache: hit={} / miss={} | Proxy={}",
                    self.render_graph.cache_hits,
                    self.render_graph.cache_misses,
                    self.render_graph.proxy_count()
                ));
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("📁 プロジェクト:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.project_dir_input).desired_width(220.0),
                );
                if ui.button("新規").clicked() {
                    self.handle_new_project_folder();
                }
                if ui.button("開く").clicked() {
                    self.handle_open_project_folder();
                }
                if ui.button("💾 保存").clicked() {
                    self.handle_save_project();
                }
                if ui.button("📦 素材収集").clicked() {
                    self.handle_collect_assets();
                }
                ui.separator();
                ui.weak(&self.project_status);
            });
        });

        // クラッシュ復旧バナー: `history.jsonl` に未保存の操作履歴が残っている場合に表示
        if !self.pending_recovery.is_empty() {
            egui::Panel::top("crash_recovery_banner").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!(
                            "⚠ 前回の未保存操作ジャーナル (history.jsonl: {} 件) を検出しました",
                            self.pending_recovery.len()
                        ),
                    );
                    if ui.button("🔄 復旧リプレイ").clicked() {
                        let entries = std::mem::take(&mut self.pending_recovery);
                        let mut sess = self.session.lock().unwrap();
                        match CommandBus::replay_history(&mut sess.project, &entries) {
                            Ok(n) => {
                                self.preview_dirty = true;
                                self.project_status =
                                    format!("クラッシュ復旧完了: {n} 件の操作を再適用しました");
                            }
                            Err(e) => {
                                self.project_status = format!("復旧リプレイ失敗: {e}");
                            }
                        }
                    }
                    if ui.button("破棄").clicked() {
                        self.pending_recovery.clear();
                        let sess = self.session.lock().unwrap();
                        if let Some(folder) = &sess.folder {
                            let _ = folder.clear_history();
                        }
                        self.project_status = "未反映履歴を破棄しました".into();
                    }
                });
            });
        }

        // 再生中の場合は音声マスタークロックから playhead を更新
        let (is_playing, current_time, current_sample) = {
            let clock = self.audio_state.clock.read().unwrap();
            if clock.is_playing() {
                clock.tick_fallback_clock();
                ctx.request_repaint();
                let t = clock.current_time();
                let s = clock.current_sample();
                if let Ok(mut sess) = self.session.lock() {
                    sess.playhead = t;
                }
                (true, t, s)
            } else {
                (false, self.session.lock().unwrap().playhead, 0)
            }
        };

        // 非 Windows 環境（フォールバッククロック）でも VU / Peak / LUFS メーターをリアルタイム更新
        #[cfg(not(windows))]
        {
            if is_playing {
                if let Ok(graph) = self.audio_state.graph.try_read() {
                    let frames = self.meter_scratch.len() / 2;
                    graph.render_block(current_sample, frames, &mut self.meter_scratch);
                    self.audio_state
                        .meter
                        .store(ve_audio::measure_block_meter(&self.meter_scratch));
                }
            } else {
                self.audio_state
                    .meter
                    .store(ve_audio::AudioMeterSnapshot::default());
            }
        }
        #[cfg(windows)]
        let _ = (is_playing, current_sample);

        // 立ち絵の音量連動口パク用に最新波形データを RenderGraph へ同期
        for (aid, wf) in &self.waveforms {
            if !self.render_graph.asset_waveforms.contains_key(aid) {
                self.render_graph.register_waveform(*aid, wf.clone());
                self.preview_dirty = true;
            }
        }

        // Render Graph 評価＋wgpu Ping-Pong GPU オフロード（アイドル時は完全スキップ）
        let active_gpu_passes = self.build_active_gpu_passes();
        if let (Some(wgpu_state), Some(pipe)) =
            (frame.wgpu_render_state(), self.preview_pipeline.as_mut())
        {
            if pipe.sync_hot_shaders(&wgpu_state.device, &self.render_graph.hot_shaders) > 0 {
                self.preview_dirty = true;
            }
            let (revision, active_id) = {
                let sess = self.session.lock().unwrap();
                (sess.bus.revision(), sess.project.active_scene_id)
            };
            let render_key = (
                revision,
                current_time.ticks,
                self.preview_scale,
                self.waveforms.len(),
                active_id,
            );

            if self.preview_dirty || self.last_rendered_key != Some(render_key) {
                let mut proj_clone = {
                    let sess = self.session.lock().unwrap();
                    sess.project.clone()
                };
                if let Some(s) = proj_clone.active_scene_mut() {
                    s.width = (s.width / self.preview_scale.max(1)).max(1);
                    s.height = (s.height / self.preview_scale.max(1)).max(1);
                }
                if let Some(linear_img) = self.render_graph.render_project_scene(
                    &proj_clone,
                    active_id,
                    current_time,
                    false,
                ) {
                    pipe.upload_process_and_present(
                        &wgpu_state.device,
                        &wgpu_state.queue,
                        &linear_img,
                        &active_gpu_passes,
                        current_time.as_seconds() as f32,
                    );
                    self.last_rendered_key = Some(render_key);
                    self.preview_dirty = false;
                }
            }
        }

        let mut dock_state = std::mem::replace(&mut self.dock_state, DockState::new(vec![]));
        let mut viewer = EditorTabViewer { app: self };
        DockArea::new(&mut dock_state)
            .style(Style::from_egui(ctx.global_style().as_ref()))
            .show_inside(ui, &mut viewer);
        self.dock_state = dock_state;

        // ドラッグホバー中の視覚ガイドオーバーレイ
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering_files {
            egui::Area::new(egui::Id::NULL.with("drop_files_overlay"))
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(egui::Color32::from_rgba_unmultiplied(20, 32, 48, 235))
                        .show(ui, |ui| {
                            ui.heading("📥 ファイルをドロップしてタイムラインへ追加");
                            ui.label("対応: 動画 (MP4/MOV/MKV/WebM) ・ 音声 (WAV/MP3/FLAC/TTSペア) ・ 画像 (PNG/JPEG/WebP) ・ PSD立ち絵");
                        });
                });
        }
    }
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1480.0, 900.0])
            .with_title("Next-Gen Hybrid Video Editor v0.4.0 (OSS Edition)"),
        wgpu_options: egui_wgpu::WgpuConfiguration::default(),
        ..Default::default()
    };

    eframe::run_native(
        "ve-app",
        options,
        Box::new(|cc| Ok(Box::new(EditorApp::new(cc)))),
    )
}
