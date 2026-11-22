use egui::{Color32, Pos2, Stroke, Vec2};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use ve_audio::{
    DubbingVoicePreset, GeneratedDubClip, align_text_to_phoneme_segments,
    build_dubbing_timeline_commands, synthesize_offline_dubbing_pcm,
};
use ve_core::{
    AssetRef, CommandSource, CrdtOpKind, CrdtSyncLog, EditCommand, ItemId, ItemKind, LanCollabHost,
    MediaKind, Mesh3DConfig, Mesh3DPreset, MouthShape, RationalTime, RgbaColor, TimelineItem,
    TimelineMarker, build_cube_mesh, encode_mesh_to_glb, sync_crdt_with_lan_peer,
};
use ve_mcp::SharedEditorSession;
use ve_media::write_wav_pcm16;
use ve_render::{
    CameraTrack3DSample, GpuBenchmarkReport, GpuV30Capabilities, RenderGraph,
    analyze_3d_camera_track, benchmark_v30_gpu_pipeline, build_3d_camera_track_command,
    detect_gpu_v30_capabilities,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V30Tab {
    GpuCompute,
    Scene3D,
    OfflineDubbing,
    RealtimeCrdt,
}

/// v3.0 #29〜#32 統合ウィンドウ状態（次世代 GPU・3D ワールド・オフライン AI 吹替・P2P CRDT 共同編集）
pub struct V30PanelsState {
    pub show_window: bool,
    pub active_tab: V30Tab,

    // --- #29 GPU Compute ---
    gpu_caps: Option<GpuV30Capabilities>,
    last_bench: Option<GpuBenchmarkReport>,
    gpu_status: String,

    // --- #30 3D Mesh / glTF & Camera Tracker ---
    new_mesh_text: String,
    cam_track_interval: f32,
    cam_track_samples: Vec<CameraTrack3DSample>,
    scene3d_status: String,

    // --- #31 Offline Multi-Language AI Dubbing & Lip-Sync ---
    dub_locale: String,
    dub_voice: DubbingVoicePreset,
    dub_audio_layer: usize,
    dub_preview_text: String,
    dub_status: String,

    // --- #32 P2P / LAN CRDT Collab ---
    peer_name_input: String,
    peer_color_input: String,
    lan_bind_addr: String,
    lan_remote_addr: String,
    lan_token: String,
    lan_host: Option<LanCollabHost>,
    lan_host_log: Option<Arc<Mutex<CrdtSyncLog>>>,
    crdt_delta_text: String,
    crdt_marker_label: String,
    crdt_status: String,
}

impl Default for V30PanelsState {
    fn default() -> Self {
        Self {
            show_window: false,
            active_tab: V30Tab::GpuCompute,
            gpu_caps: None,
            last_bench: None,
            gpu_status: "WGSL Compute Shader 検証・4K ベンチマークを実行できます".into(),
            new_mesh_text: "V3.0 STUDIO".into(),
            cam_track_interval: 0.25,
            cam_track_samples: Vec::new(),
            scene3d_status:
                "3D キューブ・球体・トーラス・立体テキスト・glTF/GLB モデルを配置できます".into(),
            dub_locale: "en".into(),
            dub_voice: DubbingVoicePreset::EnFemale,
            dub_audio_layer: 3,
            dub_preview_text: "こんにちは、次世代動画編集スタジオへようこそ！".into(),
            dub_status: "完全オフラインで多言語音声 WAV と立ち絵リップシンク音素を自動生成します"
                .into(),
            peer_name_input: "Editor A".into(),
            peer_color_input: "#44ccff".into(),
            lan_bind_addr: "127.0.0.1:17890".into(),
            lan_remote_addr: "127.0.0.1:17890".into(),
            lan_token: "studio-lan-v30".into(),
            lan_host: None,
            lan_host_log: None,
            crdt_delta_text: String::new(),
            crdt_marker_label: "CRDT チェックポイント".into(),
            crdt_status: "クラウド不要の Lamport/Vector-Clock CRDT ＆ LAN TCP 同期".into(),
        }
    }
}

impl V30PanelsState {
    pub fn show_v30_window(
        &mut self,
        ctx: &egui::Context,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut open = self.show_window;
        let mut changed = false;

        egui::Window::new("🌌 v3.0 ワールド＆GPU スタジオ")
            .open(&mut open)
            .default_size([740.0, 620.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.active_tab,
                        V30Tab::GpuCompute,
                        "⚡ GPU コンピュート＆4K",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V30Tab::Scene3D,
                        "🧊 3D メッシュ/glTF ＆ カメラ追跡",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V30Tab::OfflineDubbing,
                        "🎙 オフライン吹替＆リップシンク",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V30Tab::RealtimeCrdt,
                        "🌐 P2P/CRDT 同時共同編集",
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.active_tab {
                    V30Tab::GpuCompute => self.show_gpu_tab(ui),
                    V30Tab::Scene3D => {
                        changed |= self.show_scene3d_tab(ui, session, selected_item);
                    }
                    V30Tab::OfflineDubbing => {
                        changed |= self.show_dubbing_tab(ui, session);
                    }
                    V30Tab::RealtimeCrdt => {
                        changed |= self.show_crdt_tab(ui, session, selected_item);
                    }
                });
            });

        self.show_window = open;
        changed
    }

    // =========================================================================
    // Tab 1: #29 Full GPU Compute Pipeline & 4K Benchmark
    // =========================================================================

    fn show_gpu_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("⚡ フル GPU コンピュートパイプライン（Vulkan / Metal / DX12 WGSL）");
        let caps = self
            .gpu_caps
            .get_or_insert_with(detect_gpu_v30_capabilities)
            .clone();

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let badge = if caps.gpu_available {
                    (
                        "🟢 GPU アクセラレーション有効",
                        Color32::from_rgb(80, 220, 140),
                    )
                } else {
                    (
                        "🟡 ヘッドレス / 決定論的 CPU フォールバック",
                        Color32::from_rgb(245, 195, 70),
                    )
                };
                ui.colored_label(badge.1, badge.0);
                ui.separator();
                ui.label(format!(
                    "Adapter: {} ({})",
                    caps.adapter_name, caps.backend_name
                ));
            });
            ui.horizontal_wrapped(|ui| {
                let ok_badge = |label: &str, ok: bool| {
                    if ok {
                        format!("✅ {label}")
                    } else {
                        format!("❌ {label}")
                    }
                };
                ui.label(ok_badge(
                    "COLOR_GRADE_COMPUTE_WGSL",
                    caps.color_shader_valid,
                ));
                ui.separator();
                ui.label(ok_badge(
                    "VIDEO_SCOPES_COMPUTE_WGSL",
                    caps.scopes_shader_valid,
                ));
                ui.separator();
                ui.label(ok_badge(
                    "COMPOSITE_COMPUTE_WGSL",
                    caps.composite_shader_valid,
                ));
            });
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("🔄 アダプタ＆WGSL 再検証").clicked() {
                self.gpu_caps = Some(detect_gpu_v30_capabilities());
                self.gpu_status = "WGSL シェーダー検証およびアダプタ検出を更新しました".into();
            }
            if ui.button("⚡ Full HD (1920×1080) ベンチマーク").clicked() {
                let rep = benchmark_v30_gpu_pipeline(1920, 1080);
                self.gpu_status = format!(
                    "1080p 完了: {:.2} ms ({:.1} FPS 相当, {})",
                    rep.elapsed_ms, rep.estimated_fps, rep.backend_name
                );
                self.last_bench = Some(rep);
            }
            if ui.button("🔥 4K UHD (3840×2160) ベンチマーク").clicked() {
                let rep = benchmark_v30_gpu_pipeline(3840, 2160);
                self.gpu_status = format!(
                    "4K 完了: {:.2} ms ({:.1} FPS 相当, {})",
                    rep.elapsed_ms, rep.estimated_fps, rep.backend_name
                );
                self.last_bench = Some(rep);
            }
        });

        if let Some(ref b) = self.last_bench {
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(format!(
                    "解像度: {}×{}  |  実行バックエンド: {} ({})",
                    b.width,
                    b.height,
                    b.backend_name,
                    if b.executed_on_gpu {
                        "GPU Compute"
                    } else {
                        "CPU Fallback"
                    }
                ));
                ui.label(format!(
                    "処理時間: {:.2} ms  |  推定スループット: {:.1} FPS  |  平均輝度: {:.1} IRE",
                    b.elapsed_ms, b.estimated_fps, b.avg_luma_ire
                ));
            });
        }
        ui.small(&self.gpu_status);
    }

    // =========================================================================
    // Tab 2: #30 3D Mesh / glTF 2.0 Scene Composition & 3D Camera Tracker
    // =========================================================================

    fn show_scene3d_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut changed = false;

        ui.heading("🧊 3D メッシュ / glTF 2.0 / 3D 立体押し出しテキスト配置");
        ui.horizontal_wrapped(|ui| {
            ui.label("3D テキスト:");
            ui.add(egui::TextEdit::singleline(&mut self.new_mesh_text).desired_width(140.0));
            for (lbl, preset) in [
                ("🧊 キューブ追加", Mesh3DPreset::Cube),
                ("🔮 球体追加", Mesh3DPreset::Sphere),
                ("🍩 トーラス追加", Mesh3DPreset::Torus),
                ("🔤 3D テキスト追加", Mesh3DPreset::ExtrudedText),
            ] {
                if ui.button(lbl).clicked() {
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    let cfg = Mesh3DConfig {
                        preset,
                        text_content: self.new_mesh_text.clone(),
                        ..Mesh3DConfig::default()
                    };
                    let mut item = TimelineItem::new(
                        preset.display_name(),
                        s.playhead,
                        RationalTime::from_seconds(4.0),
                        ItemKind::Mesh3D { config: cfg },
                    );
                    item.transform.x = 220.0;
                    item.transform.y = 120.0;
                    let id = item.id;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject {
                                layer_index: 0,
                                item,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        s.selected_items = vec![id];
                        changed = true;
                        self.scene3d_status = format!("{} をタイムラインへ追加しました", lbl);
                    }
                }
            }
            if ui.button("📦 サンプル GLB 書き出し＆配置").clicked() {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                let out_dir = s
                    .folder
                    .as_ref()
                    .map(|f| f.assets_dir())
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_glb"));
                if std::fs::create_dir_all(&out_dir).is_ok() {
                    let path = out_dir.join("sample_cube.glb");
                    let bytes = encode_mesh_to_glb(&build_cube_mesh());
                    if std::fs::write(&path, &bytes).is_ok() {
                        let cfg = Mesh3DConfig {
                            preset: Mesh3DPreset::GltfFile,
                            gltf_path: Some(path.display().to_string()),
                            ..Mesh3DConfig::default()
                        };
                        let mut item = TimelineItem::new(
                            "glTF Sample Cube",
                            s.playhead,
                            RationalTime::from_seconds(4.0),
                            ItemKind::Mesh3D { config: cfg },
                        );
                        item.transform.x = 220.0;
                        item.transform.y = 120.0;
                        let id = item.id;
                        if s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::AddObject {
                                    layer_index: 0,
                                    item,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                        {
                            s.selected_items = vec![id];
                            changed = true;
                            self.scene3d_status =
                                format!("GLB ({} bytes) を書き出して配置しました", bytes.len());
                        }
                    }
                }
            }
        });

        // 選択中または先頭の Mesh3D アイテム PBR 質感エディタ
        let mesh_target: Option<(ItemId, String, Mesh3DConfig)> = {
            let g = session.lock().unwrap();
            g.project.active_scene().and_then(|sc| {
                let by_sel = selected_item.and_then(|id| {
                    sc.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                        if i.id == id
                            && let ItemKind::Mesh3D { config } = &i.kind
                        {
                            Some((i.id, i.name.clone(), config.clone()))
                        } else {
                            None
                        }
                    })
                });
                by_sel.or_else(|| {
                    sc.layers.iter().flat_map(|l| l.items.iter()).find_map(|i| {
                        if let ItemKind::Mesh3D { config } = &i.kind {
                            Some((i.id, i.name.clone(), config.clone()))
                        } else {
                            None
                        }
                    })
                })
            })
        };

        if let Some((item_id, name, mut cfg)) = mesh_target {
            ui.separator();
            ui.label(format!("編集中の 3D メッシュ: {name}"));
            let mut dirty = false;

            ui.horizontal_wrapped(|ui| {
                for &p in Mesh3DPreset::ALL {
                    if ui
                        .selectable_value(&mut cfg.preset, p, p.display_name())
                        .changed()
                    {
                        dirty = true;
                    }
                }
            });

            egui::Grid::new("v30_mesh_pbr_grid")
                .num_columns(4)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Metallic (金属感)");
                    if ui
                        .add(egui::Slider::new(&mut cfg.metallic, 0.0..=1.0))
                        .changed()
                    {
                        dirty = true;
                    }
                    ui.label("Roughness (粗さ)");
                    if ui
                        .add(egui::Slider::new(&mut cfg.roughness, 0.06..=1.0))
                        .changed()
                    {
                        dirty = true;
                    }
                    ui.end_row();

                    ui.label("環境反射強度");
                    if ui
                        .add(egui::Slider::new(&mut cfg.env_reflection, 0.0..=1.0))
                        .changed()
                    {
                        dirty = true;
                    }
                    ui.label("自動スピン (°/s)");
                    if ui
                        .add(egui::Slider::new(
                            &mut cfg.auto_spin_deg_per_sec,
                            -180.0..=180.0,
                        ))
                        .changed()
                    {
                        dirty = true;
                    }
                    ui.end_row();

                    ui.label("回転 X / Y (°)");
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::DragValue::new(&mut cfg.rot_x_deg).speed(1.0))
                            .changed()
                        {
                            dirty = true;
                        }
                        if ui
                            .add(egui::DragValue::new(&mut cfg.rot_y_deg).speed(1.0))
                            .changed()
                        {
                            dirty = true;
                        }
                    });
                    ui.label("色 / ワイヤー");
                    ui.horizontal(|ui| {
                        let mut col = [cfg.base_color.r, cfg.base_color.g, cfg.base_color.b];
                        if ui.color_edit_button_rgb(&mut col).changed() {
                            cfg.base_color = RgbaColor {
                                r: col[0],
                                g: col[1],
                                b: col[2],
                                a: 1.0,
                            };
                            dirty = true;
                        }
                        if ui.checkbox(&mut cfg.wireframe, "ワイヤー").changed() {
                            dirty = true;
                        }
                    });
                    ui.end_row();
                });

            if cfg.preset == Mesh3DPreset::ExtrudedText {
                ui.horizontal(|ui| {
                    ui.label("立体テキスト:");
                    if ui.text_edit_singleline(&mut cfg.text_content).changed() {
                        dirty = true;
                    }
                    ui.label("奥行き:");
                    if ui
                        .add(egui::Slider::new(&mut cfg.extrude_depth, 0.05..=1.2))
                        .changed()
                    {
                        dirty = true;
                    }
                });
            }

            if dirty {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetMesh3D {
                            item_id,
                            config: cfg,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    changed = true;
                }
            }
        }

        ui.separator();
        ui.heading("🎥 3D カメラトラッカー（特徴点視差＋Z ドリー復元）");
        ui.horizontal_wrapped(|ui| {
            ui.label("解析間隔 (秒):");
            ui.add(egui::Slider::new(&mut self.cam_track_interval, 0.1..=1.5));
            if ui.button("🔍 3D カメラ軌跡を解析＆適用 (1-Undo)").clicked() {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                let scene_id = s.project.active_scene_id;
                let mut rg = RenderGraph::new();
                rg.set_project_folder(s.folder.clone());
                let samples = analyze_3d_camera_track(
                    &mut rg,
                    &s.project,
                    scene_id,
                    f64::from(self.cam_track_interval),
                );
                let base_cam = s
                    .project
                    .active_scene()
                    .map(|sc| sc.camera)
                    .unwrap_or_default();
                if let Some(cmd) = build_3d_camera_track_command(&base_cam, &samples)
                    && s.bus
                        .execute(&mut s.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                {
                    changed = true;
                    self.scene3d_status = format!(
                        "3D カメラトラッキング完了（{} サンプルから 3D カメラ位置を更新）",
                        samples.len()
                    );
                }
                self.cam_track_samples = samples;
            }
        });

        if self.cam_track_samples.len() >= 2 {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(420.0, 62.0), egui::Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 3.0, Color32::from_rgb(16, 20, 28));
            let n = (self.cam_track_samples.len() - 1).max(1) as f32;
            let pts: Vec<Pos2> = self
                .cam_track_samples
                .iter()
                .enumerate()
                .map(|(i, sm)| {
                    let norm_z = ((sm.cam_z + 1500.0) / 1500.0).clamp(0.0, 1.0);
                    Pos2::new(
                        rect.left() + (i as f32 / n) * rect.width(),
                        rect.bottom() - norm_z * rect.height() * 0.85 - 4.0,
                    )
                })
                .collect();
            p.add(egui::Shape::line(
                pts,
                Stroke::new(2.0, Color32::from_rgb(90, 210, 255)),
            ));
            ui.small("シアン曲線: 復元された 3D カメラ Z 深度軌跡");
        }
        ui.small(&self.scene3d_status);
        changed
    }

    // =========================================================================
    // Tab 3: #31 Offline Multi-Language AI Dubbing & Phoneme Lip-Sync
    // =========================================================================

    fn show_dubbing_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut changed = false;
        ui.heading("🎙 オフライン多言語 AI 吹替 ＆ 音声リップシンク自動生成");
        ui.colored_label(
            Color32::from_rgb(80, 220, 140),
            "🔒 完全オフライン（クラウド送信ゼロ・3フォルマント共振 48kHz WAV 合成＋母音音素自動同期）",
        );

        ui.horizontal_wrapped(|ui| {
            ui.label("吹替言語コード:");
            ui.add(egui::TextEdit::singleline(&mut self.dub_locale).desired_width(56.0));
            ui.label("配置音声レイヤー:");
            ui.add(egui::DragValue::new(&mut self.dub_audio_layer).range(0..=15));
        });
        ui.horizontal_wrapped(|ui| {
            for &v in DubbingVoicePreset::ALL {
                ui.selectable_value(&mut self.dub_voice, v, v.display_name());
            }
        });

        ui.separator();
        ui.label("音素リップシンク プレビュー:");
        ui.text_edit_singleline(&mut self.dub_preview_text);
        let preview_ph = align_text_to_phoneme_segments(
            &self.dub_preview_text,
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
        );
        ui.horizontal_wrapped(|ui| {
            for seg in preview_ph.iter().take(24) {
                let col = match seg.mouth {
                    MouthShape::A | MouthShape::Open => Color32::from_rgb(255, 110, 100),
                    MouthShape::I => Color32::from_rgb(90, 210, 255),
                    MouthShape::U => Color32::from_rgb(130, 140, 255),
                    MouthShape::E => Color32::from_rgb(255, 200, 80),
                    MouthShape::O => Color32::from_rgb(110, 230, 150),
                    MouthShape::HalfOpen => Color32::from_rgb(190, 190, 210),
                    MouthShape::Closed => Color32::from_rgb(95, 100, 112),
                };
                ui.colored_label(col, format!("[{}]", seg.phoneme));
            }
        });

        ui.add_space(6.0);
        if ui
            .button("🎙 シーン全字幕からオフライン吹替 WAV 生成＆リップシンク適用 (1-Undo)")
            .clicked()
        {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            let targets: Vec<(ItemId, String, RationalTime, RationalTime)> = s
                .project
                .active_scene()
                .map(|sc| {
                    sc.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .filter_map(|i| {
                            if let ItemKind::Text { content, .. } = &i.kind {
                                Some((i.id, content.clone(), i.start, i.duration))
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            if targets.is_empty() {
                self.dub_status = "シーン内に吹替対象の字幕アイテムがありません".into();
            } else {
                let out_dir = s
                    .folder
                    .as_ref()
                    .map(|f| f.assets_dir().join("dubbing"))
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_dubbing"));
                let _ = std::fs::create_dir_all(&out_dir);
                let sr = s.project.settings.audio_sample_rate.max(8_000);
                let mut generated = Vec::new();
                for (idx, (sub_id, txt, start, dur)) in targets.into_iter().enumerate() {
                    let phonemes = align_text_to_phoneme_segments(&txt, start, dur);
                    let pcm =
                        synthesize_offline_dubbing_pcm(&phonemes, start, dur, sr, self.dub_voice);
                    let wav_name = format!("dub_{}_{idx}.wav", self.dub_locale);
                    let wav_path = out_dir.join(&wav_name);
                    if write_wav_pcm16(&wav_path, &pcm, sr, 2).is_ok() {
                        let asset_id = Uuid::new_v4();
                        s.project.assets.push(AssetRef {
                            id: asset_id,
                            name: wav_name.clone(),
                            relative_path: Some(PathBuf::from("assets/dubbing").join(&wav_name)),
                            fallback_absolute_path: Some(wav_path),
                            collected: true,
                            kind: MediaKind::Audio,
                        });
                        generated.push(GeneratedDubClip {
                            subtitle_item_id: sub_id,
                            translated_text: txt,
                            start,
                            duration: dur,
                            audio_asset_id: asset_id,
                            clip_name: format!("Dub[{}] #{idx}", self.dub_locale),
                            phonemes,
                        });
                    }
                }
                let cnt = generated.len();
                let cmds = s
                    .project
                    .active_scene()
                    .map(|sc| build_dubbing_timeline_commands(sc, self.dub_audio_layer, &generated))
                    .unwrap_or_default();
                if !cmds.is_empty()
                    && s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!("オフライン吹替＆リップシンク ({cnt}件)"),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                {
                    changed = true;
                    self.dub_status = format!(
                        "{cnt} 件のオフライン吹替 WAV を生成し、音素リップシンクを適用しました"
                    );
                }
            }
        }
        ui.small(&self.dub_status);
        changed
    }

    // =========================================================================
    // Tab 4: #32 Real-Time P2P / LAN CRDT Multi-User Collab
    // =========================================================================

    fn show_crdt_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut changed = false;
        ui.heading("🌐 リアルタイム P2P / LAN ＆ CRDT 同時共同編集（クラウド不要）");

        ui.horizontal_wrapped(|ui| {
            ui.label("表示名:");
            ui.add(egui::TextEdit::singleline(&mut self.peer_name_input).desired_width(110.0));
            ui.label("カラー:");
            ui.add(egui::TextEdit::singleline(&mut self.peer_color_input).desired_width(72.0));
            if ui.button("📍 自分のカーソル/再生ヘッドを記録").clicked() {
                let mut g = session.lock().unwrap();
                g.crdt_log.display_name = self.peer_name_input.clone();
                g.crdt_log.color_hex = self.peer_color_input.clone();
                let ph = g.playhead.as_seconds();
                g.crdt_log.update_local_cursor(ph, selected_item);
                self.crdt_status = format!("カーソルプレゼンスを更新しました (t={ph:.2}s)");
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("CRDT マーカー:");
            ui.add(egui::TextEdit::singleline(&mut self.crdt_marker_label).desired_width(150.0));
            if ui.button("➕ CRDT マーカー操作を記録").clicked() {
                let mut g = session.lock().unwrap();
                let t = g.playhead;
                let marker = TimelineMarker::new(t, &self.crdt_marker_label);
                g.crdt_log.record_local(CrdtOpKind::AddMarker { marker });
                self.crdt_status = format!(
                    "Lamport={} でマーカー操作を記録しました",
                    g.crdt_log.lamport_clock
                );
            }
        });

        ui.separator();
        ui.heading("🔌 ローカル LAN TCP ピア同期（トークン認証付）");
        ui.horizontal_wrapped(|ui| {
            ui.label("ホスト待受:");
            ui.add(egui::TextEdit::singleline(&mut self.lan_bind_addr).desired_width(135.0));
            ui.label("トークン:");
            ui.add(egui::TextEdit::singleline(&mut self.lan_token).desired_width(115.0));
            if self.lan_host.is_none() {
                if ui.button("🟢 LAN ホスト起動").clicked() {
                    let seed_log = session.lock().unwrap().crdt_log.clone();
                    let shared = Arc::new(Mutex::new(seed_log));
                    match LanCollabHost::start(
                        &self.lan_bind_addr,
                        self.lan_token.clone(),
                        Arc::clone(&shared),
                    ) {
                        Ok(host) => {
                            self.crdt_status =
                                format!("LAN CRDT ホスト起動中: {}", host.bound_addr);
                            self.lan_remote_addr = host.bound_addr.clone();
                            self.lan_host = Some(host);
                            self.lan_host_log = Some(shared);
                        }
                        Err(e) => {
                            self.crdt_status = format!("ホスト起動失敗: {e}");
                        }
                    }
                }
            } else if ui.button("🛑 ホスト停止").clicked() {
                if let Some(h) = self.lan_host.take() {
                    h.stop();
                }
                self.lan_host_log = None;
                self.crdt_status = "LAN ホストを停止しました".into();
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("接続先ピア:");
            ui.add(egui::TextEdit::singleline(&mut self.lan_remote_addr).desired_width(150.0));
            if ui.button("🔄 LAN ピアと双方向同期＆適用").clicked() {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                match sync_crdt_with_lan_peer(
                    &self.lan_remote_addr,
                    &self.lan_token,
                    &mut s.crdt_log,
                ) {
                    Ok(added) => {
                        let converged = s.crdt_log.apply_to_project(&s.project);
                        if s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::ReplaceProjectSnapshot {
                                    project: Box::new(converged),
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                        {
                            changed = true;
                        }
                        self.crdt_status = format!(
                            "LAN 同期完了: 新規 {added} 操作をマージ (Lamport={})",
                            s.crdt_log.lamport_clock
                        );
                    }
                    Err(e) => {
                        self.crdt_status = format!("LAN 同期失敗: {e}");
                    }
                }
            }
        });

        // 在席ピアカーソル＆Lamport時計表示
        {
            let g = session.lock().unwrap();
            ui.separator();
            ui.label(format!(
                "ローカル Peer: {}  |  Lamport Clock: {}  |  操作数: {}  |  在席ピア: {}",
                g.crdt_log.local_peer_id,
                g.crdt_log.lamport_clock,
                g.crdt_log.operations.len(),
                g.crdt_log.peers.len()
            ));
            ui.horizontal_wrapped(|ui| {
                for peer in g.crdt_log.peers.values() {
                    ui.colored_label(
                        Color32::from_rgb(100, 220, 255),
                        format!("● {} ({:.2}s)", peer.display_name, peer.playhead_sec),
                    );
                }
            });
        }

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("📤 Delta JSON 書き出し").clicked() {
                let g = session.lock().unwrap();
                self.crdt_delta_text = g.crdt_log.export_delta_json(None);
                self.crdt_status = "CRDT デルタ JSON を生成しました".into();
            }
            if ui
                .button("📥 リモート Delta マージ＆1-Undo 反映")
                .clicked()
            {
                match CrdtSyncLog::parse_delta_json(&self.crdt_delta_text) {
                    Ok(ops) => {
                        let mut g = session.lock().unwrap();
                        let s = &mut *g;
                        let added = s.crdt_log.merge_remote_ops(&ops);
                        let converged = s.crdt_log.apply_to_project(&s.project);
                        if s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::ReplaceProjectSnapshot {
                                    project: Box::new(converged),
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                        {
                            changed = true;
                            self.crdt_status = format!(
                                "リモート Delta {added} 操作をマージしプロジェクトへ収束反映しました"
                            );
                        }
                    }
                    Err(e) => self.crdt_status = format!("Delta JSON パースエラー: {e}"),
                }
            }
        });

        egui::ScrollArea::vertical()
            .id_salt("v30_crdt_delta_scroll")
            .max_height(130.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.crdt_delta_text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(5),
                );
            });
        ui.small(&self.crdt_status);
        changed
    }
}
