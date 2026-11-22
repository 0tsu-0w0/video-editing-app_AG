mod asset_browser;
mod audio_ai_panel;
mod color_scopes;
mod exporter;
mod font_picker;
mod graph_editor;
mod ime_poc;
mod inspector_extras;
mod native_dialog;
mod preview_canvas;
mod project_io;
mod ram_preview;
mod render_queue;
mod timeline;
mod v09_panels;
mod v10_panels;
mod v20_panels;
mod v21_panels;
mod v30_panels;
mod v40_panels;
mod watch_folder;

use asset_browser::AssetBrowserPanel;
use audio_ai_panel::AudioAi3DPanel;
use color_scopes::ColorScopesState;
use eframe::egui_wgpu;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use exporter::{ExportJobHandle, ExportJobRequest, ExportProgressSnapshot};
use font_picker::FontPickerState;
use graph_editor::GraphEditorState;
use ime_poc::ImePocPanel;
use inspector_extras::InspectorExtrasState;
use preview_canvas::PreviewCanvasState;
use ram_preview::{RamFrameKey, RamPreviewManager};
use render_queue::RenderQueueState;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use timeline::{TimelinePlaybackAction, TimelineState};
use uuid::Uuid;
use v09_panels::V09PanelsState;
use v10_panels::{V10PanelsState, apply_user_visuals};
use v20_panels::V20PanelsState;
use v21_panels::V21PanelsState;
use v30_panels::V30PanelsState;
use v40_panels::V40PanelsState;
use ve_audio::{AudioEngineState, AudioGraph, WaveformPeaks};
use ve_core::{
    AssetResolution, BlendMode, CommandBus, CommandSource, CubicBezier, EditCommand,
    EditorUserSettings, EffectNode, FilterTargetScope, FrameRate, HistoryEntry, Interpolation,
    ItemKind, Keyframe, Project, ProjectSettings, RationalTime, RgbaColor, ScenePreset, ShapeKind,
    TextAnimator, TimelineItem, TrackMatte, TrackMatteMode, Transform2D,
    build_motion_track_keyframes_command, tr,
};
use ve_mcp::{EditorSession, EmbeddedMcpServer, SharedEditorSession};
use ve_media::{ExportConfig, ExportFormat, HwDecodeBackend, VideoEncoder};
use ve_plugin_api::WasmPluginManager;
use ve_render::{
    GpuPostProcessPass, RenderGraph, ShaderReloadStatus, WgpuPreviewPipeline, compute_video_scopes,
    register_shader_dir, track_point_in_scene,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceMode {
    /// メイン編集空間（AviUtl2ライク: カット・配置中心）
    MainEdit,
    /// モーション合成空間（AEライク: グラフエディタ・マット・エフェクトチェーン）
    MotionComposite,
    /// v2.0 #21: ノード合成空間（Fusion / Nuke スタイル: ノードグラフ配線・チャンネル分離・マスク合流）
    NodeComposite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Preview,
    Timeline,
    GraphEditor,
    NodeGraph,
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
        WorkspaceMode::NodeComposite => {
            let mut dock = DockState::new(vec![Pane::Preview]);
            let surface = dock.main_surface_mut();
            let [top, _bottom] = surface.split_below(
                egui_dock::NodeIndex::root(),
                0.44,
                vec![Pane::NodeGraph, Pane::Timeline],
            );
            let [_left, _right] = surface.split_right(
                top,
                0.62,
                vec![Pane::Inspector, Pane::AssetBrowser, Pane::AudioAi3D],
            );
            dock
        }
    }
}

struct EditorApp {
    workspace_mode: WorkspaceMode,
    session: SharedEditorSession,
    mcp_server: Option<EmbeddedMcpServer>,
    mcp_copy_status: String,
    preview_canvas: PreviewCanvasState,
    ram_preview: RamPreviewManager,
    last_ram_cached_hit: bool,
    timeline_state: TimelineState,
    graph_editor: GraphEditorState,
    asset_browser: AssetBrowserPanel,
    font_picker: FontPickerState,
    inspector_extras: InspectorExtrasState,
    color_scopes: ColorScopesState,
    v09_panels: V09PanelsState,
    v10_panels: V10PanelsState,
    v20_panels: V20PanelsState,
    v21_panels: V21PanelsState,
    v30_panels: V30PanelsState,
    v40_panels: V40PanelsState,
    user_settings: EditorUserSettings,
    hw_decode: HwDecodeBackend,
    visuals_dirty: bool,
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
    // v0.5 #3: シーン設定モーダル＆自動保存 (backups/ ローテーション)
    show_scene_settings: bool,
    scene_edit_width: u32,
    scene_edit_height: u32,
    scene_edit_fps: FrameRate,
    scene_edit_rescale: bool,
    autosave_enabled: bool,
    autosave_interval_sec: u64,
    last_autosave_rev: u64,
    last_autosave_time: std::time::Instant,
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
    // v0.6 #7: レンダーキュー（バッチ連続書き出し）
    render_queue: RenderQueueState,
    show_render_queue: bool,
    // v0.7 #9: WASM 動的プラグインマネージャー (plugins/*.wasm)
    wasm_manager: WasmPluginManager,
    preview_scale: u32,
    last_rendered_key: Option<(u64, i64, u32, u32, usize, Uuid)>,
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

        let mut project = Project::new("Hybrid Motion Project", settings.clone());
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
                    content: "次世代ハイブリッド動画編集ソフト\nハイブリッド編集＆AI連携"
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
        let mut wasm_manager = WasmPluginManager::new(PathBuf::from("plugins"));
        let _ = wasm_manager.poll_and_sync(&mut render_graph.plugins);

        let (proxy_tx, proxy_rx) = mpsc::channel();

        Self {
            workspace_mode,
            session,
            mcp_server,
            mcp_copy_status: String::new(),
            preview_canvas: PreviewCanvasState::default(),
            ram_preview: RamPreviewManager::new(),
            last_ram_cached_hit: false,
            timeline_state: TimelineState::default(),
            graph_editor: GraphEditorState::default(),
            asset_browser: AssetBrowserPanel::new(),
            font_picker: FontPickerState::new(),
            inspector_extras: InspectorExtrasState::default(),
            color_scopes: ColorScopesState::default(),
            v09_panels: V09PanelsState::default(),
            v10_panels: V10PanelsState::default(),
            v20_panels: V20PanelsState::default(),
            v21_panels: V21PanelsState::default(),
            v30_panels: V30PanelsState::default(),
            v40_panels: V40PanelsState::default(),
            user_settings: EditorUserSettings::default(),
            hw_decode: HwDecodeBackend::Auto,
            visuals_dirty: true,
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
            show_scene_settings: false,
            scene_edit_width: 1280,
            scene_edit_height: 720,
            scene_edit_fps: FrameRate::FPS_60,
            scene_edit_rescale: true,
            autosave_enabled: true,
            autosave_interval_sec: 60,
            last_autosave_rev: 0,
            last_autosave_time: std::time::Instant::now(),
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
            render_queue: RenderQueueState::default(),
            show_render_queue: false,
            wasm_manager,
            preview_scale: 2,
            last_rendered_key: None,
            preview_dirty: true,
            last_shader_poll: std::time::Instant::now(),
        }
    }

    /// シーン解像度とプレビュー縮小率から、アスペクト比を維持しつつ最大 960px に収まるプレビュー解像度を算出する
    fn compute_preview_dims(&self, scene_w: u32, scene_h: u32) -> (u32, u32) {
        let div = self.preview_scale.max(1);
        let mut pw = (scene_w / div).max(16);
        let mut ph = (scene_h / div).max(16);
        let max_side = pw.max(ph);
        if max_side > 960 {
            let s = 960.0 / max_side as f64;
            pw = ((pw as f64 * s).round() as u32).max(16);
            ph = ((ph as f64 * s).round() as u32).max(16);
        }
        (pw, ph)
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
                let max_bytes = self.user_settings.disk_frame_cache_mib * 1024 * 1024;
                self.render_graph.set_project_folder(Some(folder.clone()));
                self.render_graph
                    .enable_disk_frame_cache(folder.cache_frames_dir(), max_bytes);
                let _ = folder.save_user_settings(&self.user_settings);
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
                self.user_settings = opened.folder.load_user_settings();
                self.v10_panels.keymap_json_buf = self
                    .user_settings
                    .keymap
                    .to_json_pretty()
                    .unwrap_or_default();
                self.visuals_dirty = true;
                let max_bytes = self.user_settings.disk_frame_cache_mib * 1024 * 1024;
                self.ram_preview.disk_cache_max_bytes = max_bytes;
                self.render_graph.clear_media_caches();
                self.render_graph
                    .set_project_folder(Some(opened.folder.clone()));
                self.render_graph
                    .enable_disk_frame_cache(opened.folder.cache_frames_dir(), max_bytes);
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
            ExportFormat::Mp4H265HdrPq10Bit => ("export_h265_hdr10.mp4", false),
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
            worker_threads: self.user_settings.export_worker_threads,
            hw_decode: self.hw_decode,
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
            Pane::NodeGraph => "🔀 ノード合成グラフ (Fusion/Nuke)".into(),
            Pane::Inspector => match self.app.workspace_mode {
                WorkspaceMode::MainEdit => "インスペクタ / MCP / 日本語IME".into(),
                WorkspaceMode::MotionComposite => "AEモーション・エフェクト・マット".into(),
                WorkspaceMode::NodeComposite => "ノード合成インスペクタ / MCP".into(),
            },
            Pane::AssetBrowser => "🗂 アセット・エイリアス・監視".into(),
            Pane::AudioAi3D => "🎙 音声自動化・立ち絵・3D".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Pane::Preview => {
                if self.app.color_scopes.show_scopes {
                    let scopes_h = 170.0;
                    let total_h = ui.available_height();
                    let preview_h = (total_h - scopes_h).max(160.0);
                    let resp =
                        ui.allocate_ui(egui::Vec2::new(ui.available_width(), preview_h), |ui| {
                            let mut sess = self.app.session.lock().unwrap();
                            if self.app.preview_canvas.show(
                                ui,
                                self.app.preview_tex_id,
                                &mut sess,
                                self.app.last_ram_cached_hit,
                            ) {
                                self.app.preview_dirty = true;
                            }
                        });
                    if self.app.v10_panels.multicam_grid_active {
                        let sess = self.app.session.lock().unwrap();
                        self.app.v10_panels.draw_multicam_grid_overlay(
                            ui,
                            resp.response.rect,
                            &sess,
                        );
                    }
                    self.app.color_scopes.show_video_scopes_panel(ui);
                } else {
                    let avail = ui.available_rect_before_wrap();
                    let mut sess = self.app.session.lock().unwrap();
                    if self.app.preview_canvas.show(
                        ui,
                        self.app.preview_tex_id,
                        &mut sess,
                        self.app.last_ram_cached_hit,
                    ) {
                        self.app.preview_dirty = true;
                    }
                    if self.app.v10_panels.multicam_grid_active {
                        self.app
                            .v10_panels
                            .draw_multicam_grid_overlay(ui, avail, &sess);
                    }
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
                    let (pw, ph) = sess
                        .project
                        .active_scene()
                        .map(|s| self.app.compute_preview_dims(s.width, s.height))
                        .unwrap_or((640, 360));
                    let ram_ranges = self.app.ram_preview.cached_frame_ranges(
                        sess.bus.revision(),
                        sess.project.active_scene_id,
                        pw,
                        ph,
                    );

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
                            ExportFormat::Mp4H264 => "MP4 (H.264 Rec.709)",
                            ExportFormat::Mp4H265 => "MP4 (H.265 Rec.709)",
                            ExportFormat::Mp4H265HdrPq10Bit => "MP4 (H.265 HDR10 PQ 10-bit)",
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
                                    "MP4 (H.264 Rec.709)",
                                );
                                ui.selectable_value(
                                    &mut self.app.export_format,
                                    ExportFormat::Mp4H265,
                                    "MP4 (H.265 Rec.709)",
                                );
                                ui.selectable_value(
                                    &mut self.app.export_format,
                                    ExportFormat::Mp4H265HdrPq10Bit,
                                    "MP4 (H.265 HDR10 PQ 10-bit)",
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
                        let rq_label = if self.app.render_queue.running {
                            format!("📋 キュー (実行中 {})", self.app.render_queue.jobs.len())
                        } else if !self.app.render_queue.jobs.is_empty() {
                            format!("📋 キュー ({})", self.app.render_queue.jobs.len())
                        } else {
                            "📋 キュー".to_string()
                        };
                        if ui
                            .selectable_label(self.app.show_render_queue, rq_label)
                            .on_hover_text("バッチ連続書き出しレンダーキューを開く")
                            .clicked()
                        {
                            self.app.show_render_queue = !self.app.show_render_queue;
                        }
                        ui.label(&self.app.export_status);
                    });

                    ui.separator();
                    let pb_action = self.app.timeline_state.show(
                        ui,
                        &mut sess,
                        &self.app.waveforms,
                        &ram_ranges,
                    );

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
            Pane::NodeGraph => {
                let mut sess = self.app.session.lock().unwrap();
                if self.app.v20_panels.show_node_graph_pane(ui, &mut sess) {
                    self.app.preview_dirty = true;
                }
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

                        // v0.9 #15: モーショングラフィックス テンプレート公開パラメータ専用編集
                        if let Some(ref binding) = item.template_binding {
                            let mut g = self.app.session.lock().unwrap();
                            if self.app.v09_panels.show_template_instance_ui(
                                ui,
                                &mut g,
                                item.id,
                                binding,
                            ) {
                                self.app.ram_preview.clear();
                                self.app.preview_dirty = true;
                            }
                        }

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

                        // v0.7.1 + v0.9: 専用オブジェクトインスペクタ
                        match item.kind.clone() {
                            ItemKind::FilterObject {
                                effect_type,
                                scope,
                                params,
                            } => {
                                let manifests = self.app.render_graph.plugins.list_manifests();
                                let mut g = self.app.session.lock().unwrap();
                                inspector_extras::show_filter_object_ui(
                                    ui,
                                    &mut g,
                                    item.id,
                                    effect_type,
                                    scope,
                                    params,
                                    &manifests,
                                );
                            }
                            ItemKind::GroupControl { target_layer_count } => {
                                let mut g = self.app.session.lock().unwrap();
                                inspector_extras::show_group_control_ui(
                                    ui,
                                    &mut g,
                                    item.id,
                                    target_layer_count,
                                );
                            }
                            ItemKind::AudioSpectrum {
                                linked_audio_layer,
                                width,
                                height,
                                bar_count,
                                color,
                                peak_color,
                                sensitivity,
                            } => {
                                let mut g = self.app.session.lock().unwrap();
                                inspector_extras::show_audio_spectrum_ui(
                                    ui,
                                    &mut g,
                                    item.id,
                                    linked_audio_layer,
                                    width,
                                    height,
                                    bar_count,
                                    color,
                                    peak_color,
                                    sensitivity,
                                );
                            }
                            ItemKind::ParticleEmitter { config } => {
                                let mut g = self.app.session.lock().unwrap();
                                if V09PanelsState::show_particle_inspector_ui(
                                    ui, &mut g, item.id, config,
                                ) {
                                    self.app.ram_preview.clear();
                                    self.app.preview_dirty = true;
                                }
                            }
                            ItemKind::VectorShape { config } => {
                                let mut g = self.app.session.lock().unwrap();
                                if V09PanelsState::show_vector_shape_inspector_ui(
                                    ui, &mut g, item.id, config,
                                ) {
                                    self.app.ram_preview.clear();
                                    self.app.preview_dirty = true;
                                }
                            }
                            _ => {}
                        }

                        // v0.9 #14: Pure-Rust エクスプレッションエディタ
                        {
                            let mut g = self.app.session.lock().unwrap();
                            if self.app.v09_panels.show_expression_editor_ui(
                                ui, &mut g, layer_idx, &item,
                            ) {
                                self.app.ram_preview.clear();
                                self.app.preview_dirty = true;
                            }
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
                            color,
                            mut animator,
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

                            // 字幕リッチ装飾（縁取り・ドロップシャドウ・座布団背景・スタイルプリセット）
                            {
                                let mut g = self.app.session.lock().unwrap();
                                inspector_extras::show_text_style_ui(
                                    ui,
                                    &mut self.app.inspector_extras,
                                    &mut g,
                                    item.id,
                                    &font_family,
                                    font_size,
                                    color,
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

                        // v0.6 #4: オーディオリアクティブ（音声エネルギー連動アニメーション）
                        // v0.7 #8: 自由ベジェマスク ＆ 自動モーショントラッキング
                        // v0.7 #9: WASM 動的プラグイン管理
                        // B-3: 音声DSPエフェクト（クリップ＆マスター）＋ BGM自動ダッキング ＋ LUFS ラウドネス正規化
                        let track_req = {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_bezier_masks_ui(ui, &mut g, &item);
                            let req = inspector_extras::show_motion_tracker_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &g,
                                &item,
                            );
                            inspector_extras::show_audio_reactive_ui(ui, &mut g, &item);
                            if inspector_extras::show_wasm_plugins_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                                Some(item.id),
                                &mut self.app.wasm_manager,
                                &mut self.app.render_graph.plugins,
                            ) {
                                self.app.preview_dirty = true;
                            }
                            inspector_extras::show_audio_dsp_and_lufs_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                                Some(&item),
                            );
                            req
                        };
                        if let Some(cfg) = track_req {
                            let (proj_clone, scene_id) = {
                                let g = self.app.session.lock().unwrap();
                                (g.project.clone(), g.project.active_scene_id)
                            };
                            let samples = track_point_in_scene(
                                &mut self.app.render_graph,
                                &proj_clone,
                                scene_id,
                                &cfg,
                            );
                            let init_tf = item.eval_transform(cfg.start_time);
                            let off_x = init_tf.x - cfg.anchor_x;
                            let off_y = init_tf.y - cfg.anchor_y;
                            if let Some(cmd) =
                                build_motion_track_keyframes_command(&item, &samples, off_x, off_y)
                            {
                                let mut g = self.app.session.lock().unwrap();
                                let s = &mut *g;
                                if s.bus
                                    .execute(&mut s.project, cmd, CommandSource::Gui, None)
                                    .is_ok()
                                {
                                    self.app.preview_dirty = true;
                                    self.app.inspector_extras.track_status = format!(
                                        "追跡完了: {} 点の X/Y キーフレームを焼き込みました",
                                        samples.len()
                                    );
                                }
                            } else {
                                self.app.inspector_extras.track_status =
                                    "追跡サンプルを取得できませんでした".into();
                            }
                        }
                    } else {
                        // 未選択時でもカラーパレット・WASMプラグイン・マスター音声DSP/LUFSは操作可能
                        {
                            let mut g = self.app.session.lock().unwrap();
                            inspector_extras::show_color_palette_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                            );
                            if inspector_extras::show_wasm_plugins_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                                None,
                                &mut self.app.wasm_manager,
                                &mut self.app.render_graph.plugins,
                            ) {
                                self.app.preview_dirty = true;
                            }
                            inspector_extras::show_audio_dsp_and_lufs_ui(
                                ui,
                                &mut self.app.inspector_extras,
                                &mut g,
                                None,
                            );
                        }
                        ui.separator();
                        ui.weak("タイムラインまたはプレビューでオブジェクトを選択すると、変形・装飾・エフェクト等を編集できます。");
                    }

                    ui.separator();
                    {
                        let mut g = self.app.session.lock().unwrap();
                        if self.app.color_scopes.show_color_grading_ui(ui, &mut g) {
                            self.app.ram_preview.clear();
                            self.app.preview_dirty = true;
                        }
                        if V09PanelsState::show_scene_lighting_and_motion_blur_ui(ui, &mut g) {
                            self.app.ram_preview.clear();
                            self.app.preview_dirty = true;
                        }
                        let tpl_action = self.app.v09_panels.show_template_gallery_ui(ui, &mut g);
                        if let Some(new_proj) = tpl_action.replaced_project {
                            g.project = new_proj;
                            g.bus = CommandBus::default();
                            g.selected_items.clear();
                            g.playhead = RationalTime::ZERO;
                            self.app.render_graph.clear_media_caches();
                            self.app.ram_preview.clear();
                            self.app.preview_dirty = true;
                        } else if tpl_action.dirty {
                            self.app.ram_preview.clear();
                            self.app.preview_dirty = true;
                        }
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
                                        if ui.button("📂").clicked()
                                            && let Some(p) = native_dialog::pick_file(
                                                "再リンク先ファイルを選択",
                                                &[],
                                            )
                                        {
                                            *entry = p.display().to_string();
                                        }
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
                        if ui.button("📂").clicked()
                            && let Some(p) =
                                native_dialog::pick_file("カスタム WGSL を選択", &["wgsl"])
                        {
                            self.app.custom_wgsl_path_input = p.display().to_string();
                        }
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
        if self.visuals_dirty {
            apply_user_visuals(&ctx, &self.user_settings);
            self.render_graph.set_hw_decode(self.hw_decode);
            self.ram_preview.hw_decode = self.hw_decode;
            self.ram_preview.disk_cache_max_bytes =
                self.user_settings.disk_frame_cache_mib * 1024 * 1024;
            self.visuals_dirty = false;
        }
        {
            let ph = self.session.lock().unwrap().playhead;
            if self.v10_panels.handle_multicam_hotkeys(&ctx, ph) {
                self.preview_dirty = true;
            }
        }
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

        // v0.6 #7: レンダーキュー（バッチ連続書き出し）の進捗ポーリング＆順次実行
        if self.render_queue.running {
            let (proj_clone, folder) = {
                let sess = self.session.lock().unwrap();
                (sess.project.clone(), sess.folder.clone())
            };
            let gpu_passes = self.build_active_gpu_passes();
            let shader_files: Vec<(String, PathBuf)> = self
                .render_graph
                .hot_shaders
                .list()
                .into_iter()
                .map(|e| (e.id.clone(), e.path.clone()))
                .collect();
            if self
                .render_queue
                .tick(&proj_clone, folder.as_ref(), &gpu_passes, &shader_files)
            {
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

        // カスタム WGSL シェーダー ＆ WASM プラグイン (plugins/*.wasm) の自動ホットリロードポーリング（500ms 間隔）
        if self.last_shader_poll.elapsed() >= std::time::Duration::from_millis(500) {
            self.last_shader_poll = std::time::Instant::now();
            let reloaded = self.render_graph.hot_shaders.poll_reload();
            if !reloaded.is_empty() {
                self.custom_wgsl_msg = format!("自動ホットリロード: {}", reloaded.join(", "));
                self.preview_dirty = true;
            }
            let wasm_events = self
                .wasm_manager
                .poll_and_sync(&mut self.render_graph.plugins);
            if !wasm_events.is_empty() {
                self.inspector_extras.wasm_status = wasm_events.join(" / ");
                self.preview_dirty = true;
            }
        }

        // 定期自動保存（backups/ ローテーション保護）
        if self.autosave_enabled
            && self.last_autosave_time.elapsed()
                >= std::time::Duration::from_secs(self.autosave_interval_sec.max(10))
        {
            let maybe_save = {
                let sess = self.session.lock().unwrap();
                let rev = sess.bus.revision();
                if rev != self.last_autosave_rev
                    && let Some(folder) = sess.folder.clone()
                {
                    Some((folder, sess.project.clone(), rev))
                } else {
                    None
                }
            };
            self.last_autosave_time = std::time::Instant::now();
            if let Some((folder, proj, rev)) = maybe_save {
                match folder.save_auto_backup(&proj, 10) {
                    Ok(_) => {
                        self.last_autosave_rev = rev;
                        self.project_status = "自動バックアップを backups/ へ保存しました".into();
                    }
                    Err(e) => {
                        self.project_status = format!("自動保存エラー: {e}");
                    }
                }
            }
        }

        // 上部バー: Dual-Workspace UX 切替 + シーン切替 + シーン設定 + RAM先読み + プロジェクト/自動保存
        egui::Panel::top("workspace_switcher_bar").show(ui, |ui| {
            let (scenes, active_id, cur_w, cur_h, cur_fps, cur_rev, backup_list) = {
                let s = self.session.lock().unwrap();
                let list: Vec<(Uuid, String)> = s
                    .project
                    .scenes
                    .iter()
                    .map(|sc| (sc.id, sc.name.clone()))
                    .collect();
                let (w, h, f) = s
                    .project
                    .active_scene()
                    .map(|sc| (sc.width, sc.height, sc.fps))
                    .unwrap_or((1280, 720, FrameRate::FPS_60));
                let backups = s
                    .folder
                    .as_ref()
                    .map(|fl| fl.list_backups())
                    .unwrap_or_default();
                (
                    list,
                    s.project.active_scene_id,
                    w,
                    h,
                    f,
                    s.bus.revision(),
                    backups,
                )
            };
            let (pw, ph) = self.compute_preview_dims(cur_w, cur_h);
            let ram_cnt = self.ram_preview.cached_count(cur_rev, active_id, pw, ph);

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
                if ui
                    .selectable_label(
                        self.workspace_mode == WorkspaceMode::NodeComposite,
                        "🔀 ノード合成空間 (Fusion/Nuke)",
                    )
                    .on_hover_text("v2.0 #21: 第3ワークスペース — チャンネル分離・マスク合流・3D深度合成をノード配線")
                    .clicked()
                {
                    self.switch_workspace(WorkspaceMode::NodeComposite);
                }

                ui.separator();
                ui.label("シーン:");
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
                        self.preview_dirty = true;
                    }
                }
                if ui
                    .button(format!("⚙ {}×{} {:.0}fps", cur_w, cur_h, cur_fps.as_f64()))
                    .on_hover_text("シーン解像度・FPS・縦動画(9:16)プリセット設定を開く")
                    .clicked()
                {
                    self.scene_edit_width = cur_w;
                    self.scene_edit_height = cur_h;
                    self.scene_edit_fps = cur_fps;
                    self.show_scene_settings = !self.show_scene_settings;
                }

                ui.separator();
                if ui
                    .checkbox(&mut self.render_graph.use_proxies, "軽量プロキシ")
                    .changed()
                {
                    self.ram_preview.clear();
                    self.preview_dirty = true;
                }
                if ui
                    .checkbox(
                        &mut self.ram_preview.enabled,
                        format!("⚡ RAM先読み ({ram_cnt}枚)"),
                    )
                    .changed()
                {
                    self.ram_preview.clear();
                }
                if ui
                    .small_button("🧹")
                    .on_hover_text("RAM プレビューキャッシュをクリア")
                    .clicked()
                {
                    self.ram_preview.clear();
                    self.preview_dirty = true;
                }
                if ui
                    .toggle_value(&mut self.color_scopes.show_scopes, "📊 スコープ")
                    .on_hover_text("Waveform / RGBパレード / ベクトルスコープ / ヒストグラムを表示")
                    .changed()
                    && self.color_scopes.show_scopes
                {
                    self.preview_dirty = true;
                }
                if ui
                    .toggle_value(
                        &mut self.v10_panels.show_studio_window,
                        tr(self.user_settings.language, "multicam_title"),
                    )
                    .on_hover_text("v1.0 プロダクションスタジオ（マルチカム / トラックミキサー＆マイク録音 / 納品検証＆ZIP）")
                    .clicked()
                {
                    // toggle_value already flipped show_studio_window
                }
                if ui
                    .toggle_value(
                        &mut self.v20_panels.show_frontier_window,
                        "🚀 AI・共同編集",
                    )
                    .on_hover_text("v2.0 フロンティアスタジオ（ローカルAI切り抜き/リフレーム/超解像・AI編集ディレクター・タイムラインDiff＆レビュー）")
                    .clicked()
                {
                    // toggle_value already flipped show_frontier_window
                }
                if ui
                    .toggle_value(
                        &mut self.v21_panels.show_window,
                        "🎞 シネマ＆クリエイター",
                    )
                    .on_hover_text("v2.1 シネマ＆クリエイタースタジオ（RGBトーンカーブ/HSL二次補正/自動カラーマッチ・自動手ぶれ補正/4点コーナーピン・3D空間オーディオ/True-Peakマスタリング・キネティック字幕/多言語切替）")
                    .clicked()
                {
                    // toggle_value already flipped show_window
                }
                if ui
                    .toggle_value(
                        &mut self.v30_panels.show_window,
                        "🌌 v3.0 ワールド＆GPU",
                    )
                    .on_hover_text("v3.0 ワールド＆GPU スタジオ（フルGPUコンピュート＆4Kベンチ・3Dメッシュ/glTF 2.0＆カメラトラッカー・オフライン多言語AI吹替＆リップシンク・P2P/LAN CRDT同時共同編集）")
                    .clicked()
                {
                    // toggle_value already flipped show_window
                }
                if ui
                    .toggle_value(
                        &mut self.v40_panels.show_window,
                        "🏆 v4.0 マスター",
                    )
                    .on_hover_text("v4.0 マスタースタジオ（ACEScg/OpenColorIOカラー管理・2D剛体物理演算＆キーフレーム自動ベイク・プロシージャル背景＆Foley SFX合成・放送QC監査＆ヒーローサムネイル＆HTML5レビュー書き出し）")
                    .clicked()
                {
                    // toggle_value already flipped show_window
                }
                if ui
                    .toggle_value(
                        &mut self.v10_panels.show_settings_window,
                        tr(self.user_settings.language, "settings_title"),
                    )
                    .on_hover_text("v1.0 環境設定・カスタムキーバインド・HWデコード・公式SHA-256拡張レジストリ")
                    .clicked()
                {
                    // toggle_value already flipped show_settings_window
                }
                ui.weak(format!(
                    "Precomp: {}/{} | Proxy={}",
                    self.render_graph.cache_hits,
                    self.render_graph.cache_misses,
                    self.render_graph.proxy_count()
                ));
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("📁 プロジェクト:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.project_dir_input).desired_width(200.0),
                );
                if ui
                    .button("📂")
                    .on_hover_text("OSフォルダ選択ダイアログを開く")
                    .clicked()
                    && let Some(dir) = native_dialog::pick_folder("プロジェクトフォルダを選択")
                {
                    self.project_dir_input = dir.display().to_string();
                }
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
                ui.checkbox(&mut self.autosave_enabled, "自動保存");
                if !backup_list.is_empty() {
                    ui.menu_button(
                        format!("🕰 バックアップ({})", backup_list.len()),
                        |ui| {
                            let mut restore_path: Option<PathBuf> = None;
                            for bp in &backup_list {
                                let label = bp
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| bp.display().to_string());
                                if ui.button(label).clicked() {
                                    restore_path = Some(bp.clone());
                                    ui.close();
                                }
                            }
                            if let Some(path) = restore_path {
                                let mut sess = self.session.lock().unwrap();
                                if let Some(folder) = sess.folder.clone() {
                                    match folder.load_backup(&path) {
                                        Ok(restored) => {
                                            sess.project = restored;
                                            sess.selected_items.clear();
                                            self.ram_preview.clear();
                                            self.preview_dirty = true;
                                            self.project_status = format!(
                                                "バックアップから復元しました: {}",
                                                path.display()
                                            );
                                        }
                                        Err(e) => {
                                            self.project_status =
                                                format!("バックアップ復元失敗: {e}");
                                        }
                                    }
                                }
                            }
                        },
                    );
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
                                self.ram_preview.clear();
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

        // バックグラウンド RAM プレビュー先読み更新 + wgpu Ping-Pong GPU 描画（動的リサイズ対応）
        let active_gpu_passes = self.build_active_gpu_passes();
        let (revision, active_id, scene_w, scene_h, scene_fps, folder_snap) = {
            let sess = self.session.lock().unwrap();
            let (sw, sh, sfps) = sess
                .project
                .active_scene()
                .map(|s| (s.width, s.height, s.fps))
                .unwrap_or((1280, 720, FrameRate::FPS_60));
            (
                sess.bus.revision(),
                sess.project.active_scene_id,
                sw,
                sh,
                sfps,
                sess.folder.clone(),
            )
        };
        let (pw, ph) = self.compute_preview_dims(scene_w, scene_h);
        let cur_frame_idx = current_time.to_frame_floor(scene_fps).max(0);

        if self.ram_preview.enabled {
            let proj_for_ram = self.session.lock().unwrap().project.clone();
            let proxies = self.render_graph.proxy_paths_snapshot();
            self.ram_preview.update_target(
                &proj_for_ram,
                folder_snap.as_ref(),
                &proxies,
                &self.waveforms,
                self.render_graph.use_proxies,
                revision,
                active_id,
                cur_frame_idx,
                pw,
                ph,
            );
        }

        if let Some(wgpu_state) = frame.wgpu_render_state() {
            // シーン解像度・アスペクト比変更時は WgpuPreviewPipeline と egui テクスチャを自動再構築
            let needs_resize = self
                .preview_pipeline
                .as_ref()
                .is_none_or(|p| p.width != pw || p.height != ph);
            if needs_resize {
                let new_pipe = WgpuPreviewPipeline::new(&wgpu_state.device, pw, ph);
                if let Some(tex_id) = self.preview_tex_id {
                    wgpu_state
                        .renderer
                        .write()
                        .update_egui_texture_from_wgpu_texture(
                            &wgpu_state.device,
                            &new_pipe.output_view,
                            wgpu::FilterMode::Linear,
                            tex_id,
                        );
                } else {
                    let tex_id = wgpu_state.renderer.write().register_native_texture(
                        &wgpu_state.device,
                        &new_pipe.output_view,
                        wgpu::FilterMode::Linear,
                    );
                    self.preview_tex_id = Some(tex_id);
                }
                self.preview_pipeline = Some(new_pipe);
                self.preview_dirty = true;
            }

            if let Some(pipe) = self.preview_pipeline.as_mut() {
                if pipe.sync_hot_shaders(&wgpu_state.device, &self.render_graph.hot_shaders) > 0 {
                    self.preview_dirty = true;
                }
                let render_key = (
                    revision,
                    current_time.ticks,
                    pw,
                    ph,
                    self.waveforms.len(),
                    active_id,
                );

                if self.preview_dirty || self.last_rendered_key != Some(render_key) {
                    let ram_key = RamFrameKey {
                        revision,
                        scene_id: active_id,
                        frame_idx: cur_frame_idx,
                        width: pw,
                        height: ph,
                    };
                    let cached_img = if self.ram_preview.enabled {
                        self.ram_preview.get_frame(ram_key)
                    } else {
                        None
                    };
                    let rendered = if let Some(hit) = cached_img {
                        self.last_ram_cached_hit = true;
                        Some(hit)
                    } else {
                        self.last_ram_cached_hit = false;
                        let proj_clone = self.session.lock().unwrap().project.clone();
                        self.render_graph
                            .render_project_scene_scaled(
                                &proj_clone,
                                active_id,
                                current_time,
                                pw,
                                ph,
                                false,
                            )
                            .map(|img| {
                                let arc = Arc::new(img);
                                if self.ram_preview.enabled {
                                    self.ram_preview.insert_frame(ram_key, Arc::clone(&arc));
                                }
                                arc
                            })
                    };

                    if let Some(linear_img) = rendered {
                        if self.color_scopes.show_scopes {
                            self.color_scopes.latest_scopes = compute_video_scopes(&linear_img);
                        }
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
        }

        let mut dock_state = std::mem::replace(&mut self.dock_state, DockState::new(vec![]));
        let mut viewer = EditorTabViewer { app: self };
        DockArea::new(&mut dock_state)
            .style(Style::from_egui(ctx.global_style().as_ref()))
            .show_inside(ui, &mut viewer);
        self.dock_state = dock_state;

        // v0.5 #3: シーン設定モーダル（1080p / 4K / 9:16 Shorts / 1:1 プリセット＋スマートリフレーム）
        if self.show_scene_settings {
            let mut open = true;
            let mut apply_now = false;
            egui::Window::new("⚙ シーン設定・縦動画プリセット")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label("ワンクリック プリセット:");
                    for &preset in ScenePreset::ALL {
                        let (pw, ph) = preset.dimensions();
                        let selected = self.scene_edit_width == pw && self.scene_edit_height == ph;
                        if ui
                            .selectable_label(selected, preset.display_name())
                            .clicked()
                        {
                            self.scene_edit_width = pw;
                            self.scene_edit_height = ph;
                            self.scene_edit_fps = preset.default_fps();
                        }
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("幅:");
                        ui.add(
                            egui::DragValue::new(&mut self.scene_edit_width)
                                .range(16..=8192)
                                .suffix("px"),
                        );
                        ui.label("高さ:");
                        ui.add(
                            egui::DragValue::new(&mut self.scene_edit_height)
                                .range(16..=8192)
                                .suffix("px"),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("フレームレート:");
                        let fps_choices = [
                            ("23.976 fps", FrameRate::FPS_23_976),
                            ("24 fps", FrameRate::FPS_24),
                            ("29.97 fps", FrameRate::FPS_29_97),
                            ("30 fps", FrameRate::FPS_30),
                            ("60 fps", FrameRate::FPS_60),
                        ];
                        let cur_label = format!("{:.3} fps", self.scene_edit_fps.as_f64());
                        egui::ComboBox::from_id_salt("scene_settings_fps_combo")
                            .selected_text(cur_label)
                            .show_ui(ui, |ui| {
                                for (lbl, f) in fps_choices {
                                    ui.selectable_value(&mut self.scene_edit_fps, f, lbl);
                                }
                            });
                    });
                    ui.checkbox(
                        &mut self.scene_edit_rescale,
                        "既存アイテムの座標・サイズを自動リフレーム（比率追従）",
                    );
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("✅ 適用 (Ctrl+Z で元に戻せます)").clicked() {
                            apply_now = true;
                        }
                        if ui.button("キャンセル").clicked() {
                            self.show_scene_settings = false;
                        }
                    });
                });
            if !open {
                self.show_scene_settings = false;
            }
            if apply_now {
                let mut g = self.session.lock().unwrap();
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetSceneSettings {
                            width: self.scene_edit_width,
                            height: self.scene_edit_height,
                            fps: self.scene_edit_fps,
                            rescale_items: self.scene_edit_rescale,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.ram_preview.clear();
                    self.preview_dirty = true;
                    self.project_status = format!(
                        "シーン設定を変更しました: {}×{} ({:.2}fps)",
                        self.scene_edit_width,
                        self.scene_edit_height,
                        self.scene_edit_fps.as_f64()
                    );
                    self.show_scene_settings = false;
                }
            }
        }

        // v0.6 #7: レンダーキュー（バッチ連続書き出し）ウィンドウ
        if self.show_render_queue {
            let sess = self.session.lock().unwrap();
            self.render_queue
                .show_window(&ctx, &mut self.show_render_queue, &sess);
        }

        // v1.0 #17/#19: プロダクションスタジオ（マルチカム / トラックミキサー＆アフレコ / 納品検証＆ZIP）
        if self.v10_panels.show_studio_window {
            let mut sess = self.session.lock().unwrap();
            if self
                .v10_panels
                .show_studio_window(&ctx, &mut sess, self.user_settings.language)
            {
                self.preview_dirty = true;
            }
        }

        // v1.0 #18/#20: 環境設定・キーバインド・HWデコード・SHA-256公式拡張レジストリ
        if self.v10_panels.show_settings_window {
            let disk_stats = self.render_graph.disk_frame_cache_stats();
            let sess = self.session.lock().unwrap();
            let (settings_changed, ext_installed) =
                self.v10_panels.show_settings_and_registry_window(
                    &ctx,
                    &mut self.user_settings,
                    &mut self.hw_decode,
                    disk_stats,
                    &sess,
                );
            if settings_changed {
                self.visuals_dirty = true;
            }
            if ext_installed {
                if let Some(ref folder) = sess.folder {
                    let _ = register_shader_dir(
                        &mut self.render_graph.hot_shaders,
                        &folder.root().join("shaders"),
                    );
                }
                self.preview_dirty = true;
            }
        }

        // v2.0 #22/#23/#24: フロンティアスタジオ（ローカルAIビジョン・AI編集ディレクター・共同編集＆レビュー）
        if self.v20_panels.show_frontier_window
            && self.v20_panels.show_frontier_window(&ctx, &self.session)
        {
            self.ram_preview.clear();
            self.preview_dirty = true;
        }

        // v2.1 #25〜#28: シネマ＆クリエイタースタジオ（トーンカーブ/HSL/カラーマッチ・手ぶれ補正/4点ピン・3D空間音響/True-Peak・キネティック字幕/多言語切替）
        if self.v21_panels.show_window {
            let selected_item = self
                .session
                .lock()
                .ok()
                .and_then(|s| s.selected_items.first().copied());
            if self
                .v21_panels
                .show_cinema_creator_window(&ctx, &self.session, selected_item)
            {
                self.ram_preview.clear();
                self.preview_dirty = true;
            }
        }

        // v3.0 #29〜#32: ワールド＆GPU スタジオ（フルGPUコンピュート・3Dメッシュ/glTF＆カメラ追跡・オフライン多言語吹替＆リップシンク・P2P/LAN CRDT共同編集）
        if self.v30_panels.show_window {
            let selected_item = self
                .session
                .lock()
                .ok()
                .and_then(|s| s.selected_items.first().copied());
            if self
                .v30_panels
                .show_v30_window(&ctx, &self.session, selected_item)
            {
                self.ram_preview.clear();
                self.preview_dirty = true;
            }
        }

        // v4.0 #33〜#36: マスタースタジオ（ACEScg/OCIO・2D剛体物理演算＆ベイク・プロシージャル背景＆Foley SFX・放送QC＆HTML5レビュー）
        if self.v40_panels.show_window {
            let selected_item = self
                .session
                .lock()
                .ok()
                .and_then(|s| s.selected_items.first().copied());
            if self
                .v40_panels
                .show_v40_window(&ctx, &self.session, selected_item)
            {
                self.ram_preview.clear();
                self.preview_dirty = true;
            }
        }

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
            .with_title("Next-Gen Hybrid Video Editor v4.0.0 (OSS Edition)"),
        wgpu_options: egui_wgpu::WgpuConfiguration::default(),
        ..Default::default()
    };

    eframe::run_native(
        "ve-app",
        options,
        Box::new(|cc| Ok(Box::new(EditorApp::new(cc)))),
    )
}
