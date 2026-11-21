mod ime_poc;

use eframe::egui_wgpu;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use ime_poc::ImePocPanel;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use ve_audio::AudioEngineState;
use ve_core::{
    FilterTargetScope, FrameRate, ItemKind, Project, ProjectSettings, RationalTime, RgbaColor,
    ShapeKind, TimelineItem, Transform2D,
};
use ve_media::{ExportConfig, ExportFormat, FfmpegCli};
use ve_render::{RenderGraph, WgpuPreviewPipeline};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Preview,
    Timeline,
    InspectorIme,
}

struct EditorApp {
    project: Project,
    render_graph: RenderGraph,
    preview_pipeline: Option<WgpuPreviewPipeline>,
    preview_tex_id: Option<egui::TextureId>,
    audio_state: Arc<AudioEngineState>,
    #[cfg(windows)]
    _live_audio: Option<ve_audio::output::live::LiveAudioStream>,
    dock_state: DockState<Pane>,
    ime_panel: ImePocPanel,
    filter_enabled: bool,
    export_status: String,
    preview_scale: u32,
}

impl EditorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings = ProjectSettings::default();
        settings.width = 1280;
        settings.height = 720;
        settings.fps = FrameRate::FPS_60;

        let mut project = Project::new("Phase 1a PoC Project", settings.clone());
        if let Some(scene) = project.active_scene_mut() {
            // Layer 0: 背景シェイプ
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

            // Layer 1: 日本語テキストオブジェクト（cosmic-text + swash）
            scene.layers[1].items.push(TimelineItem {
                id: Uuid::new_v4(),
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
                    content: "次世代ハイブリッド動画編集ソフト\nPhase 1a コアエンジン PoC".to_string(),
                    font_family: "sans-serif".to_string(),
                    font_size: 56.0,
                    color: RgbaColor::WHITE,
                },
            });

            // Layer 2: AviUtl2風フィルタオブジェクト（直上レイヤーへ適用）
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
        let [top, _bottom] = surface.split_below(egui_dock::NodeIndex::root(), 0.65, vec![Pane::Timeline]);
        let [_left, _right] = surface.split_right(top, 0.68, vec![Pane::InspectorIme]);

        Self {
            project,
            render_graph: RenderGraph::new(),
            preview_pipeline,
            preview_tex_id,
            audio_state,
            #[cfg(windows)]
            _live_audio: live_audio,
            dock_state,
            ime_panel: ImePocPanel::new(),
            filter_enabled: true,
            export_status: "待機中".to_string(),
            preview_scale: 2,
        }
    }

    fn export_mp4_poc(&mut self) {
        let Some(scene) = self.project.active_scene().cloned() else {
            return;
        };
        let out_path = PathBuf::from("phase1a_poc_export.mp4");
        let cli = FfmpegCli::default();
        let config = ExportConfig {
            width: 640,
            height: 360,
            fps: FrameRate::FPS_30,
            format: ExportFormat::Mp4H264,
            crf: 20,
        };

        let mut scaled_scene = scene;
        scaled_scene.width = config.width;
        scaled_scene.height = config.height;

        match cli.start_export(&out_path, &config, None) {
            Ok(mut session) => {
                for frame_idx in 0..60 {
                    let t = RationalTime::from_frame(frame_idx, config.fps);
                    let linear = self.render_graph.render_scene(&scaled_scene, t);
                    let rgba8 = linear.to_rgba8_srgb();
                    if let Err(e) = session.write_rgba8_frame(&rgba8) {
                        self.export_status = format!("書き出しエラー: {e}");
                        return;
                    }
                }
                match session.finish() {
                    Ok(()) => {
                        self.export_status = format!("書き出し完了: {}", out_path.display());
                    }
                    Err(e) => {
                        self.export_status = format!("FFmpeg終了エラー: {e}");
                    }
                }
            }
            Err(e) => {
                self.export_status = format!("FFmpeg起動失敗: {e}");
            }
        }
    }
}

struct EditorTabViewer<'a> {
    app: &'a mut EditorApp,
    current_time: RationalTime,
}

impl TabViewer for EditorTabViewer<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::NULL.with(*tab as u8)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Pane::Preview => "プレビュー (wgpu Linear f16)".into(),
            Pane::Timeline => "タイムライン (AviUtl2領域)".into(),
            Pane::InspectorIme => "インスペクタ / 日本語IME PoC".into(),
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
                let fps = self.app.project.settings.fps;
                let frame = self.current_time.to_frame_floor(fps);
                let mut secs = self.current_time.as_seconds() as f32;

                ui.horizontal(|ui| {
                    let is_playing = self.app.audio_state.clock.read().unwrap().is_playing();
                    if ui.button(if is_playing { "⏸ 一時停止" } else { "▶ 再生" }).clicked() {
                        let mut clock = self.app.audio_state.clock.write().unwrap();
                        if is_playing {
                            clock.pause();
                        } else {
                            clock.play();
                        }
                    }
                    if ui.button("⏮ 先頭").clicked() {
                        self.app.audio_state.clock.write().unwrap().seek(RationalTime::ZERO);
                    }
                    ui.label(format!("Time: {secs:.3}s | Frame: {frame} ({:.2}fps)", fps.as_f64()));
                    ui.separator();
                    if ui.button("MP4 書き出し (2秒PoC)").clicked() {
                        self.app.export_mp4_poc();
                    }
                    ui.label(&self.app.export_status);
                });

                if ui
                    .add(egui::Slider::new(&mut secs, 0.0..=5.0).text("再生位置 (秒)"))
                    .changed()
                {
                    self.app
                        .audio_state
                        .clock
                        .write()
                        .unwrap()
                        .seek(RationalTime::from_seconds(f64::from(secs)));
                }

                ui.separator();
                if let Some(scene) = self.app.project.active_scene() {
                    for (idx, layer) in scene.layers.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.monospace(format!("L{idx} [{}]", layer.name));
                            for item in &layer.items {
                                let label = format!(
                                    "{} ({:.1}s..{:.1}s)",
                                    item.name,
                                    item.start.as_seconds(),
                                    item.end().as_seconds()
                                );
                                let _ = ui.selectable_label(item.contains(self.current_time), label);
                            }
                        });
                    }
                }
            }
            Pane::InspectorIme => {
                ui.checkbox(&mut self.app.filter_enabled, "Layer 2 フィルタオブジェクト有効");
                if let Some(scene) = self.app.project.active_scene_mut() {
                    if let Some(layer2) = scene.layers.get_mut(2) {
                        layer2.visible = self.app.filter_enabled;
                    }
                    if let Some(text_item) = scene
                        .layers
                        .get_mut(1)
                        .and_then(|l| l.items.first_mut())
                    {
                        if let ItemKind::Text { content, .. } = &mut text_item.kind {
                            self.app.ime_panel.ui(ui, content);
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ime_panel.capture_events(&ctx);

        // 音声マスタークロック更新（フォールバック時）
        let current_time = {
            let clock = self.audio_state.clock.read().unwrap();
            clock.tick_fallback_clock();
            if clock.is_playing() {
                ctx.request_repaint();
            }
            clock.current_time()
        };

        // Render Graph 評価 → wgpu Linear f16 テクスチャへアップロード
        if let (Some(wgpu_state), Some(pipe), Some(scene)) = (
            frame.wgpu_render_state(),
            self.preview_pipeline.as_ref(),
            self.project.active_scene(),
        ) {
            let mut preview_scene = scene.clone();
            preview_scene.width = self.project.settings.width / self.preview_scale.max(1);
            preview_scene.height = self.project.settings.height / self.preview_scale.max(1);
            let linear_img = self.render_graph.render_scene(&preview_scene, current_time);
            pipe.upload_and_present(&wgpu_state.device, &wgpu_state.queue, &linear_img);
        }

        let mut dock_state = std::mem::replace(&mut self.dock_state, DockState::new(vec![]));
        let mut viewer = EditorTabViewer {
            app: self,
            current_time,
        };
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
            .with_inner_size([1440.0, 860.0])
            .with_title("Next-Gen Hybrid Video Editor — Phase 1a PoC"),
        wgpu_options: egui_wgpu::WgpuConfiguration::default(),
        ..Default::default()
    };

    eframe::run_native(
        "ve-app",
        options,
        Box::new(|cc| Ok(Box::new(EditorApp::new(cc)))),
    )
}
