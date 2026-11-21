mod ime_poc;
mod timeline;

use eframe::egui_wgpu;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use ime_poc::ImePocPanel;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use timeline::TimelineState;
use uuid::Uuid;
use ve_audio::{AudioEngineState, WaveformPeaks};
use ve_core::{
    AssetResolution, CommandSource, EditCommand, FilterTargetScope, FrameRate, ItemKind, Project,
    ProjectSettings, RationalTime, RgbaColor, ShapeKind, TimelineItem, Transform2D,
};
use ve_mcp::{EditorSession, EmbeddedMcpServer, SharedEditorSession};
use ve_media::{ExportConfig, ExportFormat, FfmpegCli};
use ve_render::{RenderGraph, WgpuPreviewPipeline};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Preview,
    Timeline,
    InspectorIme,
}

struct EditorApp {
    session: SharedEditorSession,
    mcp_server: Option<EmbeddedMcpServer>,
    timeline_state: TimelineState,
    waveforms: BTreeMap<Uuid, WaveformPeaks>,
    render_graph: RenderGraph,
    preview_pipeline: Option<WgpuPreviewPipeline>,
    preview_tex_id: Option<egui::TextureId>,
    audio_state: Arc<AudioEngineState>,
    #[cfg(windows)]
    _live_audio: Option<ve_audio::output::live::LiveAudioStream>,
    dock_state: DockState<Pane>,
    ime_panel: ImePocPanel,
    relink_inputs: BTreeMap<Uuid, String>,
    export_status: String,
    preview_scale: u32,
}

impl EditorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings = ProjectSettings::default();
        settings.width = 1280;
        settings.height = 720;
        settings.fps = FrameRate::FPS_60;

        let mut project = Project::new("Phase 1b NLE + MCP Project", settings.clone());
        let mut initial_selected = Vec::new();
        if let Some(scene) = project.active_scene_mut() {
            scene.layers[0].items.push(TimelineItem {
                id: Uuid::new_v4(),
                name: "背景パネル".to_string(),
                start: RationalTime::ZERO,
                duration: RationalTime::from_seconds(5.0),
                transform: Transform2D {
                    x: 80.0,
                    y: 80.0,
                    ..Default::default()
                },
                effect_chain: vec![],
                kind: ItemKind::Shape {
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
            });

            let title_id = Uuid::new_v4();
            initial_selected.push(title_id);
            scene.layers[1].items.push(TimelineItem {
                id: title_id,
                name: "タイトル字幕".to_string(),
                start: RationalTime::ZERO,
                duration: RationalTime::from_seconds(5.0),
                transform: Transform2D {
                    x: 140.0,
                    y: 260.0,
                    ..Default::default()
                },
                effect_chain: vec![],
                kind: ItemKind::Text {
                    content: "次世代ハイブリッド動画編集ソフト\nPhase 1b NLE + 内蔵MCP".to_string(),
                    font_family: "sans-serif".to_string(),
                    font_size: 56.0,
                    color: RgbaColor::WHITE,
                },
            });

            let mut params = BTreeMap::new();
            params.insert("gain".to_string(), 1.25);
            scene.layers[2].items.push(TimelineItem {
                id: Uuid::new_v4(),
                name: "明るさフィルタ (AviUtl2方式)".to_string(),
                start: RationalTime::ZERO,
                duration: RationalTime::from_seconds(5.0),
                transform: Transform2D::default(),
                effect_chain: vec![],
                kind: ItemKind::FilterObject {
                    effect_type: "brightness".to_string(),
                    scope: FilterTargetScope::ImmediateLayer,
                    params,
                },
            });
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

        let mut dock_state = DockState::new(vec![Pane::Preview]);
        let surface = dock_state.main_surface_mut();
        let [top, _bottom] =
            surface.split_below(egui_dock::NodeIndex::root(), 0.58, vec![Pane::Timeline]);
        let [_left, _right] = surface.split_right(top, 0.66, vec![Pane::InspectorIme]);

        Self {
            session,
            mcp_server,
            timeline_state: TimelineState::default(),
            waveforms: BTreeMap::new(),
            render_graph: RenderGraph::new(),
            preview_pipeline,
            preview_tex_id,
            audio_state,
            #[cfg(windows)]
            _live_audio: live_audio,
            dock_state,
            ime_panel: ImePocPanel::new(),
            relink_inputs: BTreeMap::new(),
            export_status: "待機中".to_string(),
            preview_scale: 2,
        }
    }

    fn export_mp4_poc(&mut self) {
        let scene = {
            let guard = self.session.lock().unwrap();
            guard.project.active_scene().cloned()
        };
        let Some(mut scaled_scene) = scene else {
            return;
        };
        let out_path = PathBuf::from("phase1b_export.mp4");
        let cli = FfmpegCli::default();
        let config = ExportConfig {
            width: 640,
            height: 360,
            fps: FrameRate::FPS_30,
            format: ExportFormat::Mp4H264,
            crf: 20,
        };
        scaled_scene.width = config.width;
        scaled_scene.height = config.height;

        match cli.start_export(&out_path, &config, None) {
            Ok(mut session) => {
                for frame_idx in 0..60 {
                    let t = RationalTime::from_frame(frame_idx, config.fps);
                    let linear = self.render_graph.render_scene(&scaled_scene, t);
                    if let Err(e) = session.write_rgba8_frame(&linear.to_rgba8_srgb()) {
                        self.export_status = format!("書き出しエラー: {e}");
                        return;
                    }
                }
                match session.finish() {
                    Ok(()) => self.export_status = format!("書き出し完了: {}", out_path.display()),
                    Err(e) => self.export_status = format!("FFmpeg終了エラー: {e}"),
                }
            }
            Err(e) => self.export_status = format!("FFmpeg起動失敗: {e}"),
        }
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
            Pane::Timeline => "タイムライン (AviUtl2 NLE)".into(),
            Pane::InspectorIme => "インスペクタ / MCP / 日本語IME".into(),
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
                {
                    let mut sess = self.app.session.lock().unwrap();
                    let fps = sess.project.settings.fps;
                    let frame = sess.playhead.to_frame_floor(fps);
                    let secs = sess.playhead.as_seconds();

                    ui.horizontal(|ui| {
                        let is_playing = self.app.audio_state.clock.read().unwrap().is_playing();
                        if ui
                            .button(if is_playing { "⏸ 一時停止" } else { "▶ 再生" })
                            .clicked()
                        {
                            let mut clock = self.app.audio_state.clock.write().unwrap();
                            if is_playing {
                                clock.pause();
                            } else {
                                clock.seek(sess.playhead);
                                clock.play();
                            }
                        }
                        if ui.button("⏮ 先頭").clicked() {
                            sess.playhead = RationalTime::ZERO;
                            self.app
                                .audio_state
                                .clock
                                .write()
                                .unwrap()
                                .seek(RationalTime::ZERO);
                        }
                        ui.monospace(format!(
                            "{secs:.3}s | Frame {frame} ({:.2}fps)",
                            fps.as_f64()
                        ));
                        ui.separator();
                        if ui.button("MP4 書き出し").clicked() {
                            export_clicked = true;
                        }
                        ui.label(&self.app.export_status);
                    });

                    ui.separator();
                    self.app
                        .timeline_state
                        .show(ui, &mut sess, &self.app.waveforms);

                    // タイムライン上の手動シークをクロックへ同期
                    if !self.app.audio_state.clock.read().unwrap().is_playing() {
                        self.app
                            .audio_state
                            .clock
                            .write()
                            .unwrap()
                            .seek(sess.playhead);
                    }
                }
                if export_clicked {
                    self.app.export_mp4_poc();
                }
            }
            Pane::InspectorIme => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // 内蔵 MCP サーバーステータス
                    ui.heading("内蔵 MCP サーバー (Streamable HTTP)");
                    if let Some(mcp) = &self.app.mcp_server {
                        let url = mcp.endpoint_url();
                        ui.horizontal(|ui| {
                            ui.monospace(&url);
                            if ui.button("URLコピー").clicked() {
                                ui.ctx().copy_text(url);
                            }
                            if ui.button("トークンコピー").clicked() {
                                ui.ctx().copy_text(mcp.bearer_token().to_string());
                            }
                        });
                    } else {
                        ui.colored_label(egui::Color32::RED, "MCPサーバー起動失敗");
                    }
                    ui.separator();

                    // 選択中テキストオブジェクトの CommandBus 経由編集 + 日本語IME PoC
                    let mut text_edit_state: Option<(Uuid, String)> = None;
                    {
                        let sess = self.app.session.lock().unwrap();
                        if let Some(&sel_id) = sess.selected_items.first() {
                            if let Some(scene) = sess.project.active_scene() {
                                for layer in &scene.layers {
                                    if let Some(item) =
                                        layer.items.iter().find(|i| i.id == sel_id)
                                    {
                                        if let ItemKind::Text { content, .. } = &item.kind {
                                            text_edit_state = Some((sel_id, content.clone()));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some((item_id, mut content)) = text_edit_state {
                        let before = content.clone();
                        self.app.ime_panel.ui(ui, &mut content);
                        if content != before {
                            let mut sess_guard = self.app.session.lock().unwrap();
                            let sess = &mut *sess_guard;
                            let _ = sess.bus.execute(
                                &mut sess.project,
                                EditCommand::SetTextContent { item_id, content },
                                CommandSource::Gui,
                                Some("inspector_text_edit"),
                            );
                        }
                    } else {
                        let mut dummy = String::from("（タイムラインでテキストオブジェクトを選択してください）");
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
                                        let entry = self
                                            .app
                                            .relink_inputs
                                            .entry(asset.id)
                                            .or_default();
                                        ui.text_edit_singleline(entry);
                                        if ui.button("再リンク").clicked() && !entry.is_empty() {
                                            asset.fallback_absolute_path =
                                                Some(PathBuf::from(entry.trim()));
                                        }
                                    }
                                }
                            });
                        }
                    }
                });
            }
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ime_panel.capture_events(&ctx);

        // 再生中の場合は音声マスタークロックから playhead を更新
        let current_time = {
            let clock = self.audio_state.clock.read().unwrap();
            if clock.is_playing() {
                clock.tick_fallback_clock();
                ctx.request_repaint();
                let t = clock.current_time();
                if let Ok(mut sess) = self.session.lock() {
                    sess.playhead = t;
                }
                t
            } else {
                self.session.lock().unwrap().playhead
            }
        };

        // Render Graph 評価 → wgpu Linear f16 テクスチャへアップロード
        if let (Some(wgpu_state), Some(pipe)) =
            (frame.wgpu_render_state(), self.preview_pipeline.as_ref())
        {
            let scene_opt = {
                let sess = self.session.lock().unwrap();
                sess.project.active_scene().cloned()
            };
            if let Some(mut preview_scene) = scene_opt {
                preview_scene.width = preview_scene.width / self.preview_scale.max(1);
                preview_scene.height = preview_scene.height / self.preview_scale.max(1);
                let linear_img = self.render_graph.render_scene(&preview_scene, current_time);
                pipe.upload_and_present(&wgpu_state.device, &wgpu_state.queue, &linear_img);
            }
        }

        let mut dock_state = std::mem::replace(&mut self.dock_state, DockState::new(vec![]));
        let mut viewer = EditorTabViewer { app: self };
        DockArea::new(&mut dock_state)
            .style(Style::from_egui(ctx.global_style().as_ref()))
            .show_inside(ui, &mut viewer);
        self.dock_state = dock_state;
    }
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 880.0])
            .with_title("Next-Gen Hybrid Video Editor — Phase 1b (NLE + Command Bus + MCP)"),
        wgpu_options: egui_wgpu::WgpuConfiguration::default(),
        ..Default::default()
    };

    eframe::run_native(
        "ve-app",
        options,
        Box::new(|cc| Ok(Box::new(EditorApp::new(cc)))),
    )
}
