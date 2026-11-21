mod audio_ai_panel;
mod graph_editor;
mod ime_poc;
mod timeline;

use audio_ai_panel::AudioAi3DPanel;
use eframe::egui_wgpu;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use graph_editor::GraphEditorState;
use ime_poc::ImePocPanel;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use timeline::TimelineState;
use uuid::Uuid;
use ve_audio::{AudioEngineState, WaveformPeaks};
use ve_core::{
    AssetResolution, BlendMode, CommandSource, CubicBezier, EditCommand, EffectNode,
    FilterTargetScope, FrameRate, Interpolation, ItemKind, Keyframe, Project, ProjectSettings,
    RationalTime, RgbaColor, ShapeKind, TextAnimator, TimelineItem, TrackMatte, TrackMatteMode,
    Transform2D,
};
use ve_mcp::{EditorSession, EmbeddedMcpServer, SharedEditorSession};
use ve_media::{ExportConfig, ExportFormat, FfmpegCli};
use ve_render::{RenderGraph, ShaderReloadStatus, WgpuPreviewPipeline, register_shader_dir};

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
    AudioAi3D,
}

fn build_dock_state(mode: WorkspaceMode) -> DockState<Pane> {
    match mode {
        WorkspaceMode::MainEdit => {
            let mut dock = DockState::new(vec![Pane::Preview]);
            let surface = dock.main_surface_mut();
            let [top, _bottom] =
                surface.split_below(egui_dock::NodeIndex::root(), 0.58, vec![Pane::Timeline]);
            let [_left, _right] =
                surface.split_right(top, 0.66, vec![Pane::Inspector, Pane::AudioAi3D]);
            dock
        }
        WorkspaceMode::MotionComposite => {
            let mut dock = DockState::new(vec![Pane::Preview]);
            let surface = dock.main_surface_mut();
            let [top, bottom] =
                surface.split_below(egui_dock::NodeIndex::root(), 0.50, vec![Pane::Timeline]);
            let [_left, _right] =
                surface.split_right(top, 0.60, vec![Pane::Inspector, Pane::AudioAi3D]);
            let [_tl, _ge] = surface.split_right(bottom, 0.52, vec![Pane::GraphEditor]);
            dock
        }
    }
}

struct EditorApp {
    workspace_mode: WorkspaceMode,
    session: SharedEditorSession,
    mcp_server: Option<EmbeddedMcpServer>,
    timeline_state: TimelineState,
    graph_editor: GraphEditorState,
    audio_ai_panel: AudioAi3DPanel,
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
    custom_wgsl_path_input: String,
    custom_wgsl_msg: String,
    export_format: ExportFormat,
    export_status: String,
    preview_scale: u32,
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

        Self {
            workspace_mode,
            session,
            mcp_server,
            timeline_state: TimelineState::default(),
            graph_editor: GraphEditorState::default(),
            audio_ai_panel: AudioAi3DPanel::default(),
            waveforms: BTreeMap::new(),
            render_graph,
            preview_pipeline,
            preview_tex_id,
            audio_state,
            #[cfg(windows)]
            _live_audio: live_audio,
            dock_state,
            ime_panel: ImePocPanel::new(),
            relink_inputs: BTreeMap::new(),
            custom_wgsl_path_input: "shaders/custom_chromatic_aberration.wgsl".to_string(),
            custom_wgsl_msg: format!("監視中 WGSL: {loaded_shaders} 件"),
            export_format: ExportFormat::Mp4H264,
            export_status: "待機中".to_string(),
            preview_scale: 2,
        }
    }

    fn switch_workspace(&mut self, mode: WorkspaceMode) {
        if self.workspace_mode != mode {
            self.workspace_mode = mode;
            self.dock_state = build_dock_state(mode);
        }
    }

    fn export_current_scene(&mut self) {
        let mut proj_clone = {
            let guard = self.session.lock().unwrap();
            guard.project.clone()
        };
        let active_id = proj_clone.active_scene_id;
        let (out_path, transparent_bg) = match self.export_format {
            ExportFormat::Mp4H264 => (PathBuf::from("phase3_export_h264.mp4"), false),
            ExportFormat::Mp4H265 => (PathBuf::from("phase3_export_h265.mp4"), false),
            ExportFormat::WebmVp9Alpha => (PathBuf::from("phase3_export_alpha.webm"), true),
            ExportFormat::PngSequence => (PathBuf::from("phase3_frame_%04d.png"), true),
        };

        let config = ExportConfig {
            width: 640,
            height: 360,
            fps: FrameRate::FPS_30,
            format: self.export_format,
            crf: 20,
        };
        if let Some(s) = proj_clone.active_scene_mut() {
            s.width = config.width;
            s.height = config.height;
        }

        let cli = FfmpegCli::default();
        match cli.start_export(&out_path, &config, None) {
            Ok(mut session) => {
                for frame_idx in 0..60 {
                    let t = RationalTime::from_frame(frame_idx, config.fps);
                    if let Some(linear) = self.render_graph.render_project_scene(
                        &proj_clone,
                        active_id,
                        t,
                        transparent_bg,
                    ) && let Err(e) = session.write_rgba8_frame(&linear.to_rgba8_srgb())
                    {
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
            Pane::Timeline => "タイムライン".into(),
            Pane::GraphEditor => "グラフエディタ (値 / 速度)".into(),
            Pane::Inspector => match self.app.workspace_mode {
                WorkspaceMode::MainEdit => "インスペクタ / MCP / 日本語IME".into(),
                WorkspaceMode::MotionComposite => "AEモーション・エフェクト・マット".into(),
            },
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
                {
                    let mut sess = self.app.session.lock().unwrap();
                    let fps = sess.project.settings.fps;
                    let frame = sess.playhead.to_frame_floor(fps);
                    let secs = sess.playhead.as_seconds();

                    ui.horizontal_wrapped(|ui| {
                        let is_playing = self.app.audio_state.clock.read().unwrap().is_playing();
                        if ui
                            .button(if is_playing {
                                "⏸ 一時停止"
                            } else {
                                "▶ 再生"
                            })
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

                        let fmt_label = match self.app.export_format {
                            ExportFormat::Mp4H264 => "MP4 (H.264)",
                            ExportFormat::Mp4H265 => "MP4 (H.265)",
                            ExportFormat::WebmVp9Alpha => "WebM (VP9+透過)",
                            ExportFormat::PngSequence => "PNG連番 (透過)",
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
                            });
                        if ui.button("書き出し").clicked() {
                            export_clicked = true;
                        }
                        ui.label(&self.app.export_status);
                    });

                    ui.separator();
                    self.app
                        .timeline_state
                        .show(ui, &mut sess, &self.app.waveforms);

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
                    self.app.export_current_scene();
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
                        ui.horizontal(|ui| {
                            ui.monospace(&url);
                            if ui.button("URL").clicked() {
                                ui.ctx().copy_text(url);
                            }
                            if ui.button("トークン").clicked() {
                                ui.ctx().copy_text(mcp.bearer_token().to_string());
                            }
                        });
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

                        // WGSL エフェクトチェーン追加
                        ui.horizontal_wrapped(|ui| {
                            ui.label("エフェクト追加:");
                            for (label, eff_type, k, v) in [
                                ("ブラー", "blur", "radius", 3.0),
                                ("グロー", "glow", "intensity", 1.2),
                                ("色調補正", "color_grade", "exposure", 0.35),
                                ("クリップ", "clip_rect", "left", 0.1),
                            ] {
                                if ui.button(label).clicked() {
                                    let mut params = BTreeMap::new();
                                    params.insert(k.to_string(), v);
                                    let mut g = self.app.session.lock().unwrap();
                                    let s = &mut *g;
                                    let _ = s.bus.execute(
                                        &mut s.project,
                                        EditCommand::AddEffect {
                                            item_id: item.id,
                                            effect: EffectNode {
                                                id: Uuid::new_v4(),
                                                effect_type: eff_type.to_string(),
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
                        if !item.effect_chain.is_empty() {
                            for eff in &item.effect_chain {
                                ui.monospace(format!("  • {} {:?}", eff.effect_type, eff.params));
                            }
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

                        // テキストオブジェクトの場合は文字ごとアニメーター＋日本語IME
                        if let ItemKind::Text {
                            mut content,
                            mut animator,
                            ..
                        } = item.kind
                        {
                            ui.separator();
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
                            let before = content.clone();
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
                    } else {
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
                                        }
                                    }
                                }
                            });
                        }
                    }
                    ui.separator();
                    ui.heading("カスタム WGSL ホットリロード (Phase 4)");
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

        // カスタム WGSL シェーダーの自動ホットリロードポーリング
        let reloaded = self.render_graph.hot_shaders.poll_reload();
        if !reloaded.is_empty() {
            self.custom_wgsl_msg = format!("自動ホットリロード: {}", reloaded.join(", "));
        }

        // 上部バー: Dual-Workspace UX 切替 + シーンパンくずリスト + Precomp キャッシュ統計
        egui::Panel::top("workspace_switcher_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
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
                ui.weak(format!(
                    "Precomp Cache: hit={} / miss={}",
                    self.render_graph.cache_hits, self.render_graph.cache_misses
                ));
            });
        });

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

        // 立ち絵の音量連動口パク用に最新波形データを RenderGraph へ同期
        for (aid, wf) in &self.waveforms {
            if !self.render_graph.asset_waveforms.contains_key(aid) {
                self.render_graph.register_waveform(*aid, wf.clone());
            }
        }

        // Render Graph 評価（Precomp + 口パク(PNG/PSD) + 3Dカメラ + プラグイン） → wgpu Linear f16
        if let (Some(wgpu_state), Some(pipe)) =
            (frame.wgpu_render_state(), self.preview_pipeline.as_ref())
        {
            let mut proj_clone = {
                let sess = self.session.lock().unwrap();
                sess.project.clone()
            };
            let active_id = proj_clone.active_scene_id;
            if let Some(s) = proj_clone.active_scene_mut() {
                s.width /= self.preview_scale.max(1);
                s.height /= self.preview_scale.max(1);
            }
            if let Some(linear_img) =
                self.render_graph
                    .render_project_scene(&proj_clone, active_id, current_time, false)
            {
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
            .with_inner_size([1480.0, 900.0])
            .with_title("Next-Gen Hybrid Video Editor v0.1.0 (OSS Edition)"),
        wgpu_options: egui_wgpu::WgpuConfiguration::default(),
        ..Default::default()
    };

    eframe::run_native(
        "ve-app",
        options,
        Box::new(|cc| Ok(Box::new(EditorApp::new(cc)))),
    )
}
