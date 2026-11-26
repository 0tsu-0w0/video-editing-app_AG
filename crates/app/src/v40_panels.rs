use egui::{Color32, Pos2, Stroke, Vec2};
use std::path::PathBuf;
use uuid::Uuid;
use ve_audio::{
    AudioGraph, FoleyPreset, measure_integrated_lufs, measure_true_peak_dbtp,
    synthesize_foley_sfx_pcm, write_stereo_wav_file,
};
use ve_core::{
    AssetRef, BroadcastProfile, BroadcastQcMetrics, BroadcastQcReport, ColorManagementConfig,
    ColorSpace, CommandSource, EditCommand, GenerativeBgConfig, GenerativeBgPreset, ItemId,
    ItemKind, MediaKind, PhysicsBodyTrajectory, PhysicsConfig, PhysicsPreset, QcStatus,
    RationalTime, TimelineItem, ToneMapper, base64_encode, build_physics_bake_commands,
    export_ocio_config_yaml, export_portable_web_review_html, run_broadcast_qc_audit,
    simulate_scene_physics,
};
use ve_mcp::SharedEditorSession;
use ve_render::{
    RenderGraph, encode_linear_image_png_bytes, measure_scene_video_qc_metrics,
    select_best_thumbnail_frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V40Tab {
    AcesColor,
    PhysicsSim,
    GenerativeStudio,
    BroadcastQc,
}

/// v4.0 #33〜#36 マスタースタジオ統合ウィンドウ状態
pub struct V40PanelsState {
    pub show_window: bool,
    pub active_tab: V40Tab,

    // --- #33 ACEScg / OCIO ---
    ocio_preview_yaml: String,
    aces_status: String,

    // --- #34 Rigid-Body Physics ---
    physics_cfg: PhysicsConfig,
    physics_selected_only: bool,
    last_trajectories: Vec<PhysicsBodyTrajectory>,
    physics_status: String,

    // --- #35 Generative Backgrounds & Foley SFX ---
    bg_preset: GenerativeBgPreset,
    bg_seed: u64,
    bg_speed: f32,
    bg_scale: f32,
    bg_complexity: u32,
    bg_duration_sec: f32,
    bg_layer_index: usize,
    foley_preset: FoleyPreset,
    foley_duration_sec: f32,
    foley_seed: u64,
    foley_audio_layer: usize,
    gen_status: String,

    // --- #36 Broadcast QC, Hero Thumbnail & Portable Web Review ---
    qc_profile: BroadcastProfile,
    qc_samples: usize,
    last_qc_report: Option<BroadcastQcReport>,
    last_thumb_info: String,
    qc_status: String,
}

impl Default for V40PanelsState {
    fn default() -> Self {
        Self {
            show_window: false,
            active_tab: V40Tab::AcesColor,
            ocio_preview_yaml: String::new(),
            aces_status: "ACEScg (AP1) 広色域・カメラ Log IDT・AgX/ACES トーンマップを制御します"
                .into(),
            physics_cfg: PhysicsConfig::default(),
            physics_selected_only: false,
            last_trajectories: Vec::new(),
            physics_status:
                "決定論的 60Hz リジッドボディ物理演算でタイトル・図形を自動アニメーション化します"
                    .into(),
            bg_preset: GenerativeBgPreset::AuroraBorealis,
            bg_seed: 40_2026,
            bg_speed: 1.0,
            bg_scale: 1.0,
            bg_complexity: 4,
            bg_duration_sec: 5.0,
            bg_layer_index: 0,
            foley_preset: FoleyPreset::WhooshTransition,
            foley_duration_sec: FoleyPreset::WhooshTransition.default_duration_sec(),
            foley_seed: 40_2026,
            foley_audio_layer: 3,
            gen_status: "オフラインでプロシージャル動画背景＆48kHz Foley 効果音を生成します".into(),
            qc_profile: BroadcastProfile::YouTube1080p,
            qc_samples: 8,
            last_qc_report: None,
            last_thumb_info: String::new(),
            qc_status:
                "ITU-R BS.1770 / BT.1702 QC 監査・1280×720 サムネイル抽出・HTML5 レビュー書き出し"
                    .into(),
        }
    }
}

impl V40PanelsState {
    #[allow(dead_code)]
    pub fn show_v40_window(
        &mut self,
        ctx: &egui::Context,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        let mut open = self.show_window;
        egui::Window::new("🏆 v4.0 マスタースタジオ (ACEScg・物理演算・生成背景/SFX・放送QC)")
            .open(&mut open)
            .default_width(660.0)
            .default_height(600.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.active_tab,
                        V40Tab::AcesColor,
                        "🎨 ACEScg / OCIO",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V40Tab::PhysicsSim,
                        "🏀 物理演算エンジン",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V40Tab::GenerativeStudio,
                        "🌌 背景＆Foley SFX",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V40Tab::BroadcastQc,
                        "📡 放送QC＆HTML5レビュー",
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.active_tab {
                    V40Tab::AcesColor => {
                        if self.show_aces_tab(ui, session) {
                            mutated = true;
                        }
                    }
                    V40Tab::PhysicsSim => {
                        if self.show_physics_tab(ui, session, selected_item) {
                            mutated = true;
                        }
                    }
                    V40Tab::GenerativeStudio => {
                        if self.show_generative_tab(ui, session, selected_item) {
                            mutated = true;
                        }
                    }
                    V40Tab::BroadcastQc => {
                        self.show_qc_tab(ui, session);
                    }
                });
            });
        self.show_window = open;
        mutated
    }

    pub fn show_aces_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut mutated = false;
        let Ok(cur_cfg) = session.lock().map(|s| {
            s.project
                .active_scene()
                .map(|sc| sc.color_management)
                .unwrap_or_default()
        }) else {
            return false;
        };

        let mut cfg = cur_cfg;
        ui.heading("🎨 ACEScg (AP1) / OpenColorIO v2 プロカラー管理 (#33)");
        ui.label(
            "カメラ Log（Sony S-Log3 / Panasonic V-Log / ARRI LogC3）の IDT から ACES 1.3 ガマット圧縮、AgX/ACES フィルムトーンマップ、Display P3 / Rec.2020 ODT まで完全オフラインで適用します。",
        );

        ui.horizontal_wrapped(|ui| {
            if ui.button("🎥 S-Log3 → ACEScg → AgX").clicked() {
                cfg = ColorManagementConfig {
                    enabled: true,
                    input_idt: ColorSpace::SLog3,
                    working_space: ColorSpace::AcesCg,
                    output_odt: ColorSpace::LinearSrgb,
                    tone_mapper: ToneMapper::AgX,
                    exposure_stops: 0.2,
                    gamut_compress: true,
                };
            }
            if ui.button("🎬 ARRI LogC3 → ACES Filmic").clicked() {
                cfg = ColorManagementConfig {
                    enabled: true,
                    input_idt: ColorSpace::LogC3,
                    working_space: ColorSpace::AcesCg,
                    output_odt: ColorSpace::DisplayP3,
                    tone_mapper: ToneMapper::AcesFitted,
                    exposure_stops: 0.0,
                    gamut_compress: true,
                };
            }
            if ui.button("📺 HDR10 Rec.2020 マスター").clicked() {
                cfg = ColorManagementConfig {
                    enabled: true,
                    input_idt: ColorSpace::LinearSrgb,
                    working_space: ColorSpace::AcesCg,
                    output_odt: ColorSpace::Rec2020,
                    tone_mapper: ToneMapper::ReinhardExtended,
                    exposure_stops: 0.0,
                    gamut_compress: true,
                };
            }
        });
        ui.separator();

        ui.checkbox(
            &mut cfg.enabled,
            "シーンで ACEScg / OCIO カラー管理を有効化",
        );
        egui::ComboBox::from_label("入力色空間 (IDT)")
            .selected_text(cfg.input_idt.display_name())
            .show_ui(ui, |ui| {
                for &sp in ColorSpace::INPUT_SPACES {
                    ui.selectable_value(&mut cfg.input_idt, sp, sp.display_name());
                }
            });
        egui::ComboBox::from_label("ワーキング空間")
            .selected_text(cfg.working_space.display_name())
            .show_ui(ui, |ui| {
                for &sp in &[
                    ColorSpace::AcesCg,
                    ColorSpace::LinearSrgb,
                    ColorSpace::DisplayP3,
                    ColorSpace::Rec2020,
                ] {
                    ui.selectable_value(&mut cfg.working_space, sp, sp.display_name());
                }
            });
        egui::ComboBox::from_label("出力色空間 (ODT)")
            .selected_text(cfg.output_odt.display_name())
            .show_ui(ui, |ui| {
                for &sp in ColorSpace::OUTPUT_SPACES {
                    ui.selectable_value(&mut cfg.output_odt, sp, sp.display_name());
                }
            });
        egui::ComboBox::from_label("トーンマッパー")
            .selected_text(cfg.tone_mapper.display_name())
            .show_ui(ui, |ui| {
                for &tm in ToneMapper::ALL {
                    ui.selectable_value(&mut cfg.tone_mapper, tm, tm.display_name());
                }
            });
        ui.add(
            egui::Slider::new(&mut cfg.exposure_stops, -4.0..=4.0).text("シーンリニア露出 (EV)"),
        );
        ui.checkbox(
            &mut cfg.gamut_compress,
            "ACES 1.3 Reference Gamut Compression（ネオン色相破綻防止）",
        );

        if cfg != cur_cfg
            && let Ok(mut guard) = session.lock()
        {
            let s = &mut *guard;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetColorManagement { config: cfg },
                    CommandSource::Gui,
                    Some("v40_color_mgmt"),
                )
                .is_ok()
            {
                self.aces_status = format!(
                    "カラー管理を更新: {} → {} ({})",
                    cfg.input_idt.display_name(),
                    cfg.output_odt.display_name(),
                    cfg.tone_mapper.display_name()
                );
                mutated = true;
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("📄 .ocio v2 設定 YAML を生成＆保存").clicked() {
                let yaml = export_ocio_config_yaml(&cfg);
                self.ocio_preview_yaml = yaml.clone();
                let out_path = session
                    .lock()
                    .ok()
                    .and_then(|s| {
                        s.folder
                            .as_ref()
                            .map(|f| f.root().join("exports").join("studio_v4.ocio"))
                    })
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_studio_v4.ocio"));
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&out_path, &yaml) {
                    Ok(()) => {
                        self.aces_status =
                            format!("OCIO v2 設定を保存しました: {}", out_path.display());
                    }
                    Err(e) => {
                        self.aces_status = format!("保存エラー: {e}");
                    }
                }
            }
        });
        if !self.ocio_preview_yaml.is_empty() {
            ui.add(
                egui::TextEdit::multiline(&mut self.ocio_preview_yaml)
                    .desired_rows(7)
                    .code_editor(),
            );
        }
        ui.colored_label(Color32::LIGHT_GREEN, &self.aces_status);
        mutated
    }

    pub fn show_physics_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        ui.heading("🏀 2D リジッドボディ物理演算シミュレーション (#34)");
        ui.label(
            "重力・床/壁バウンス・剛体同士の弾性衝突と回転トルクを 60Hz 決定論計算し、1-Undo キーフレームへ自動ベイクします。",
        );

        let prev_preset = self.physics_cfg.preset;
        egui::ComboBox::from_label("物理プリセット")
            .selected_text(self.physics_cfg.preset.display_name())
            .show_ui(ui, |ui| {
                for &p in PhysicsPreset::ALL {
                    ui.selectable_value(&mut self.physics_cfg.preset, p, p.display_name());
                }
            });
        if self.physics_cfg.preset != prev_preset {
            self.physics_cfg = self.physics_cfg.preset.default_config();
        }

        ui.checkbox(
            &mut self.physics_selected_only,
            "選択中クリップのみ対象（OFF でシーン内の全ビジュアル要素を衝突計算）",
        );

        ui.add(
            egui::Slider::new(&mut self.physics_cfg.gravity_y, -2500.0..=3000.0)
                .text("重力 Y (px/s²)"),
        );
        ui.add(
            egui::Slider::new(&mut self.physics_cfg.restitution, 0.0..=1.0)
                .text("反発係数 (Bounce)"),
        );
        ui.add(egui::Slider::new(&mut self.physics_cfg.friction, 0.5..=1.0).text("床摩擦"));
        ui.add(
            egui::Slider::new(&mut self.physics_cfg.initial_vx, -1200.0..=1200.0)
                .text("初速 X (px/s)"),
        );
        ui.add(
            egui::Slider::new(&mut self.physics_cfg.initial_vy, -1200.0..=1200.0)
                .text("初速 Y (px/s)"),
        );
        ui.add(
            egui::Slider::new(&mut self.physics_cfg.initial_spin_deg, -360.0..=360.0)
                .text("初期回転角速度 (deg/s)"),
        );
        ui.add(
            egui::Slider::new(&mut self.physics_cfg.central_attract, 0.0..=1800.0)
                .text("中心ボテックス引力"),
        );
        ui.add(
            egui::Slider::new(&mut self.physics_cfg.duration_sec, 0.5..=12.0)
                .text("シミュレーション尺 (秒)"),
        );
        ui.add(
            egui::Slider::new(&mut self.physics_cfg.floor_ratio, 0.3..=0.96)
                .text("床ライン位置比率"),
        );

        let collect_targets = |s: &ve_mcp::EditorSession| -> Vec<ItemId> {
            if self.physics_selected_only
                && let Some(id) = selected_item
            {
                return vec![id];
            }
            s.project
                .active_scene()
                .map(|sc| {
                    sc.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .filter(|i| {
                            !matches!(
                                i.kind,
                                ItemKind::Audio { .. }
                                    | ItemKind::FilterObject { .. }
                                    | ItemKind::Transition { .. }
                                    | ItemKind::GroupControl { .. }
                                    | ItemKind::GenerativeBg { .. }
                            )
                        })
                        .take(12)
                        .map(|i| i.id)
                        .collect()
                })
                .unwrap_or_default()
        };

        ui.horizontal(|ui| {
            if ui.button("▶ 軌跡プレビュー計算").clicked()
                && let Ok(s) = session.lock()
            {
                let ids = collect_targets(&s);
                if let Some(sc) = s.project.active_scene() {
                    let traj = simulate_scene_physics(sc, &ids, &self.physics_cfg);
                    let bounces: u32 = traj.iter().map(|t| t.bounce_count).sum();
                    self.physics_status = format!(
                        "計算完了: {} 剛体 / 合計 {} 回バウンス・衝突",
                        traj.len(),
                        bounces
                    );
                    self.last_trajectories = traj;
                }
            }
            if ui.button("🔥 1-Undo キーフレームへ自動ベイク").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                let ids = collect_targets(s);
                let baked = s
                    .project
                    .active_scene()
                    .map(|sc| build_physics_bake_commands(sc, &ids, &self.physics_cfg));
                if let Some((traj, cmds)) = baked {
                    let count = cmds.len();
                    let bodies = traj.len();
                    self.last_trajectories = traj;
                    if !cmds.is_empty() {
                        match s.bus.execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!("2D 剛体物理ベイク ({bodies}体)"),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        ) {
                            Ok(()) => {
                                self.physics_status =
                                    format!("ベイク完了: {bodies}体・{count}キーフレームを 1-Undo で焼き込みました");
                                mutated = true;
                            }
                            Err(e) => {
                                self.physics_status = format!("ベイク失敗: {e}");
                            }
                        }
                    } else {
                        self.physics_status = "対象オブジェクトが見つかりませんでした".into();
                    }
                }
            }
        });

        // 2D 軌跡可視化キャンバス
        if !self.last_trajectories.is_empty() {
            let (sc_w, sc_h) = session
                .lock()
                .ok()
                .and_then(|s| {
                    s.project
                        .active_scene()
                        .map(|sc| (sc.width as f32, sc.height as f32))
                })
                .unwrap_or((1280.0, 720.0));
            let (rect, _) = ui.allocate_exact_size(Vec2::new(600.0, 170.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, Color32::from_rgb(14, 18, 28));

            let floor_y = rect.top() + rect.height() * self.physics_cfg.floor_ratio;
            painter.line_segment(
                [
                    Pos2::new(rect.left(), floor_y),
                    Pos2::new(rect.right(), floor_y),
                ],
                Stroke::new(1.5, Color32::from_rgb(240, 160, 60)),
            );

            let palette = [
                Color32::from_rgb(76, 224, 179),
                Color32::from_rgb(96, 178, 255),
                Color32::from_rgb(255, 130, 180),
                Color32::from_rgb(250, 210, 80),
            ];
            for (idx, tr) in self.last_trajectories.iter().enumerate() {
                let col = palette[idx % palette.len()];
                let pts: Vec<Pos2> = tr
                    .samples
                    .iter()
                    .map(|sm| {
                        let nx = (sm.x / sc_w.max(1.0)).clamp(0.0, 1.0);
                        let ny = (sm.y / sc_h.max(1.0)).clamp(0.0, 1.0);
                        Pos2::new(
                            rect.left() + nx * rect.width(),
                            rect.top() + ny * rect.height(),
                        )
                    })
                    .collect();
                if pts.len() >= 2 {
                    painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.8, col)));
                    painter.circle_filled(pts[0], 3.5, Color32::WHITE);
                    if let Some(&end) = pts.last() {
                        painter.circle_filled(end, 4.5, col);
                    }
                }
            }
        }

        ui.colored_label(Color32::LIGHT_GREEN, &self.physics_status);
        mutated
    }

    pub fn show_generative_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        ui.heading("🌌 オフライン・プロシージャル背景ジェネレーター (#35)");
        egui::ComboBox::from_label("背景プリセット")
            .selected_text(self.bg_preset.display_name())
            .show_ui(ui, |ui| {
                for &p in GenerativeBgPreset::ALL {
                    ui.selectable_value(&mut self.bg_preset, p, p.display_name());
                }
            });
        ui.add(egui::Slider::new(&mut self.bg_speed, 0.0..=5.0).text("アニメーション速度"));
        ui.add(egui::Slider::new(&mut self.bg_scale, 0.2..=4.0).text("パターン拡大率"));
        ui.add(egui::Slider::new(&mut self.bg_complexity, 1..=6).text("ディテール層数"));
        ui.add(egui::Slider::new(&mut self.bg_duration_sec, 1.0..=60.0).text("クリップ尺 (秒)"));
        ui.add(egui::Slider::new(&mut self.bg_layer_index, 0..=3).text("配置レイヤー"));

        ui.horizontal(|ui| {
            if ui.button("✨ プロシージャル背景クリップを追加").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                let (w, h, max_layer) = s
                    .project
                    .active_scene()
                    .map(|sc| {
                        (
                            sc.width as f32,
                            sc.height as f32,
                            sc.layers.len().saturating_sub(1),
                        )
                    })
                    .unwrap_or((1280.0, 720.0, 0));
                let base = GenerativeBgPreset::for_preset(self.bg_preset);
                let cfg = GenerativeBgConfig {
                    preset: self.bg_preset,
                    seed: self.bg_seed,
                    speed: self.bg_speed,
                    scale: self.bg_scale,
                    complexity: self.bg_complexity,
                    width: w,
                    height: h,
                    ..base
                };
                let item = TimelineItem::new(
                    self.bg_preset.display_name(),
                    RationalTime::ZERO,
                    RationalTime::from_seconds(f64::from(self.bg_duration_sec)),
                    ItemKind::GenerativeBg { config: cfg },
                );
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::AddObject {
                            layer_index: self.bg_layer_index.min(max_layer),
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.gen_status = format!(
                        "プロシージャル背景を追加しました: {}",
                        self.bg_preset.display_name()
                    );
                    mutated = true;
                }
            }

            if let Some(sel_id) = selected_item
                && ui.button("🔄 選択中背景へ設定を適用").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                let base = GenerativeBgPreset::for_preset(self.bg_preset);
                let (w, h) = s
                    .project
                    .active_scene()
                    .map(|sc| (sc.width as f32, sc.height as f32))
                    .unwrap_or((1280.0, 720.0));
                let cfg = GenerativeBgConfig {
                    preset: self.bg_preset,
                    seed: self.bg_seed,
                    speed: self.bg_speed,
                    scale: self.bg_scale,
                    complexity: self.bg_complexity,
                    width: w,
                    height: h,
                    ..base
                };
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetGenerativeBg {
                            item_id: sel_id,
                            config: cfg,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.gen_status = "選択中のプロシージャル背景を更新しました".into();
                    mutated = true;
                } else {
                    self.gen_status = "選択クリップはプロシージャル背景ではありません".into();
                }
            }
        });

        ui.separator();
        ui.heading("🔊 オフライン・プロシージャル Foley / SFX シンセサイザー (#35)");
        let prev_foley = self.foley_preset;
        egui::ComboBox::from_label("効果音プリセット")
            .selected_text(self.foley_preset.display_name())
            .show_ui(ui, |ui| {
                for &p in FoleyPreset::ALL {
                    ui.selectable_value(&mut self.foley_preset, p, p.display_name());
                }
            });
        if self.foley_preset != prev_foley {
            self.foley_duration_sec = self.foley_preset.default_duration_sec();
        }
        ui.add(egui::Slider::new(&mut self.foley_duration_sec, 0.1..=10.0).text("SFX 尺 (秒)"));
        ui.add(egui::Slider::new(&mut self.foley_audio_layer, 0..=3).text("配置音声レイヤー"));

        if ui
            .button("🎵 48kHz ステレオ WAV 合成＆タイムライン配置")
            .clicked()
            && let Ok(mut guard) = session.lock()
        {
            let s = &mut *guard;
            let sr = s.project.settings.audio_sample_rate.max(8_000);
            let pcm = synthesize_foley_sfx_pcm(
                self.foley_preset,
                self.foley_duration_sec,
                sr,
                self.foley_seed,
            );
            self.foley_seed = self.foley_seed.wrapping_add(1);
            let out_dir = s
                .folder
                .as_ref()
                .map(|f| f.assets_dir().join("foley"))
                .unwrap_or_else(|| std::env::temp_dir().join("ve_foley"));
            let wav_name = format!("foley_{}_{}.wav", self.foley_preset.id(), self.foley_seed);
            let wav_path = out_dir.join(&wav_name);
            match write_stereo_wav_file(&wav_path, &pcm, sr) {
                Ok(()) => {
                    let asset_id = Uuid::new_v4();
                    s.project.assets.push(AssetRef {
                        id: asset_id,
                        name: wav_name.clone(),
                        relative_path: Some(PathBuf::from("assets/foley").join(&wav_name)),
                        fallback_absolute_path: Some(wav_path.clone()),
                        collected: true,
                        kind: MediaKind::Audio,
                    });
                    let max_layer = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len().saturating_sub(1))
                        .unwrap_or(0);
                    let clip = TimelineItem::new(
                        format!("SFX: {}", self.foley_preset.display_name()),
                        RationalTime::ZERO,
                        RationalTime::from_seconds(f64::from(self.foley_duration_sec)),
                        ItemKind::Audio {
                            asset_id,
                            source_offset: RationalTime::ZERO,
                            volume: 0.9,
                        },
                    );
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject {
                                layer_index: self.foley_audio_layer.min(max_layer),
                                item: clip,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.gen_status =
                            format!("SFX を生成＆配置しました: {}", wav_path.display());
                        mutated = true;
                    }
                }
                Err(e) => {
                    self.gen_status = format!("WAV 書き出しエラー: {e}");
                }
            }
        }

        ui.colored_label(Color32::LIGHT_GREEN, &self.gen_status);
        mutated
    }

    pub fn show_qc_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) {
        ui.heading("📡 放送コンプライアンス QC・ヒーローサムネイル・HTML5 レビュー (#36)");
        egui::ComboBox::from_label("QC ターゲット規格")
            .selected_text(self.qc_profile.display_name())
            .show_ui(ui, |ui| {
                for &p in BroadcastProfile::ALL {
                    ui.selectable_value(&mut self.qc_profile, p, p.display_name());
                }
            });
        ui.add(egui::Slider::new(&mut self.qc_samples, 4..=20).text("解析サンプルフレーム数"));

        ui.horizontal_wrapped(|ui| {
            if ui.button("🔍 放送 QC 監査を実行").clicked()
                && let Ok(s) = session.lock()
            {
                let report = run_session_qc(&s, self.qc_profile, self.qc_samples);
                self.qc_status = format!(
                    "QC 完了: {} (PASS {} / WARN {} / FAIL {})",
                    if report.overall_pass {
                        "合格"
                    } else {
                        "要修正"
                    },
                    report.pass_count,
                    report.warning_count,
                    report.fail_count
                );
                self.last_qc_report = Some(report);
            }

            if ui
                .button("🖼 高CTR ヒーローサムネイル自動抽出 (1280×720)")
                .clicked()
                && let Ok(s) = session.lock()
            {
                let mut rg = RenderGraph::new();
                rg.set_project_folder(s.folder.clone());
                if let Some((best_t, score, hero)) = select_best_thumbnail_frame(
                    &mut rg,
                    &s.project,
                    s.project.active_scene_id,
                    self.qc_samples,
                ) && let Ok(png) = encode_linear_image_png_bytes(&hero)
                {
                    let out_path = s
                        .folder
                        .as_ref()
                        .map(|f| f.root().join("exports").join("hero_thumbnail_1280x720.png"))
                        .unwrap_or_else(|| {
                            std::env::temp_dir().join("ve_hero_thumbnail_1280x720.png")
                        });
                    if let Some(parent) = out_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&out_path, &png) {
                        Ok(()) => {
                            self.last_thumb_info = format!(
                                "ベストフレーム: {:.2}s / 魅力度スコア {:.1}/100 → {}",
                                best_t,
                                score,
                                out_path.display()
                            );
                            self.qc_status = "ヒーローサムネイル PNG を書き出しました".into();
                        }
                        Err(e) => {
                            self.qc_status = format!("PNG 保存エラー: {e}");
                        }
                    }
                }
            }

            if ui
                .button("🌐 ゼロサーバー HTML5 レビュー書き出し")
                .clicked()
                && let Ok(s) = session.lock()
            {
                let report = run_session_qc(&s, self.qc_profile, self.qc_samples);
                let mut rg = RenderGraph::new();
                rg.set_project_folder(s.folder.clone());
                let scene_id = s.project.active_scene_id;
                let dur = s
                    .project
                    .active_scene()
                    .map(|sc| sc.total_duration().as_seconds().max(0.5))
                    .unwrap_or(1.0);
                let mut frames = Vec::new();
                for i in 0..self.qc_samples {
                    let t = dur * (i as f64) / ((self.qc_samples - 1).max(1) as f64);
                    if let Some(img) = rg.render_project_scene_scaled(
                        &s.project,
                        scene_id,
                        RationalTime::from_seconds(t),
                        480,
                        270,
                        false,
                    ) && let Ok(png) = encode_linear_image_png_bytes(&img)
                    {
                        frames.push((t, base64_encode(&png)));
                    }
                }
                let html = export_portable_web_review_html(&s.project, &report, &frames);
                let out_path = s
                    .folder
                    .as_ref()
                    .map(|f| f.root().join("exports").join("portable_review_player.html"))
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_portable_review_player.html"));
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&out_path, &html) {
                    Ok(()) => {
                        self.qc_status = format!(
                            "自己完結型 HTML5 レビュープレイヤーを保存 ({}枚埋込): {}",
                            frames.len(),
                            out_path.display()
                        );
                        self.last_qc_report = Some(report);
                    }
                    Err(e) => {
                        self.qc_status = format!("HTML 保存エラー: {e}");
                    }
                }
            }
        });

        if !self.last_thumb_info.is_empty() {
            ui.colored_label(Color32::from_rgb(130, 220, 255), &self.last_thumb_info);
        }

        if let Some(ref rep) = self.last_qc_report {
            ui.separator();
            ui.label(format!(
                "True-Peak: {:.2} dBTP | Integrated: {:.1} LUFS | Peak Luma: {:.1} IRE | Max Flash Jump: {:.1} IRE",
                rep.metrics.true_peak_dbtp,
                rep.metrics.integrated_lufs,
                rep.metrics.peak_luma_ire,
                rep.metrics.max_frame_luma_jump_ire
            ));
            for chk in &rep.checks {
                let (badge, col) = match chk.status {
                    QcStatus::Pass => ("PASS", Color32::from_rgb(80, 220, 120)),
                    QcStatus::Warning => ("WARN", Color32::from_rgb(250, 204, 21)),
                    QcStatus::Fail => ("FAIL", Color32::from_rgb(248, 113, 113)),
                };
                ui.horizontal(|ui| {
                    ui.colored_label(col, format!("[{badge}]"));
                    ui.strong(&chk.code);
                    ui.label(&chk.detail);
                });
            }
        }

        ui.colored_label(Color32::LIGHT_GREEN, &self.qc_status);
    }
}

fn run_session_qc(
    s: &ve_mcp::EditorSession,
    profile: BroadcastProfile,
    sample_count: usize,
) -> BroadcastQcReport {
    let scene_id = s.project.active_scene_id;
    let mut rg = RenderGraph::new();
    rg.set_project_folder(s.folder.clone());
    let (peak_luma_ire, clipped_highlight_pct, crushed_shadow_pct, max_frame_luma_jump_ire) =
        measure_scene_video_qc_metrics(&mut rg, &s.project, scene_id, sample_count);
    let dur = s
        .project
        .active_scene()
        .map(|sc| sc.total_duration().as_seconds().clamp(0.5, 30.0))
        .unwrap_or(1.0);
    let sr = s.project.settings.audio_sample_rate.max(8_000);
    let ag = AudioGraph::from_project_scene(&s.project, scene_id, s.folder.as_ref(), sr);
    let pcm = if ag.clips.is_empty() {
        Vec::new()
    } else {
        ag.render_range(RationalTime::ZERO, RationalTime::from_seconds(dur))
    };
    let true_peak_dbtp = if pcm.is_empty() {
        -96.0
    } else {
        measure_true_peak_dbtp(&pcm, ag.channels)
    };
    let integrated_lufs = if pcm.is_empty() {
        -70.0
    } else {
        measure_integrated_lufs(&pcm, sr)
    };
    run_broadcast_qc_audit(
        &s.project,
        profile,
        BroadcastQcMetrics {
            peak_luma_ire,
            clipped_highlight_pct,
            crushed_shadow_pct,
            max_frame_luma_jump_ire,
            true_peak_dbtp,
            integrated_lufs,
        },
    )
}
