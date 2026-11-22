use egui::{Color32, Pos2, Stroke, Vec2};
use uuid::Uuid;
use ve_audio::{AudioGraph, measure_integrated_lufs, measure_true_peak_dbtp};
use ve_core::{
    AudioEffectNode, ColorCurves, CommandSource, CornerPinConfig, EditCommand, HslQualifier,
    ItemId, ItemKind, KineticHighlightMode, KineticSubtitleConfig, RationalTime, RgbaColor,
    build_apply_locale_commands, export_localization_csv, extract_localization_table,
    import_localization_csv, tokenize_kinetic_words,
};
use ve_mcp::SharedEditorSession;
use ve_render::{
    CameraMotionSample, RenderGraph, StabilizeConfig, analyze_camera_shake,
    build_stabilize_commands, compute_auto_color_match, evaluate_tone_curve_lut,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CinemaCreatorTab {
    ProColor,
    VfxStabilizeCornerPin,
    SpatialMasterAudio,
    KineticSubLocalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurveChannel {
    Master,
    Red,
    Green,
    Blue,
}

/// v2.1 #25〜#28 統合ウィンドウ状態（シネマ＆クリエイタースタジオ）
pub struct V21PanelsState {
    pub show_window: bool,
    pub active_tab: CinemaCreatorTab,

    // --- #25 Pro Color ---
    curve_channel: CurveChannel,
    dragging_curve_pt: Option<usize>,
    match_ref_sec: f64,
    match_tgt_sec: f64,
    color_status: String,

    // --- #26 Stabilizer & Corner Pin ---
    stab_smoothness: f32,
    stab_max_shift_pct: f32,
    stab_interval_sec: f32,
    stab_rotation: bool,
    stab_auto_crop: bool,
    last_shake_samples: Vec<CameraMotionSample>,
    dragging_pin_corner: Option<usize>,
    vfx_status: String,

    // --- #27 Spatial 3D & Mastering ---
    audio_target_clip: bool,
    spatial_azimuth_deg: f32,
    spatial_elevation_deg: f32,
    spatial_distance: f32,
    spatial_room_mix: f32,
    mb_low_thresh: f32,
    mb_mid_thresh: f32,
    mb_high_thresh: f32,
    mb_ratio: f32,
    mb_makeup: f32,
    tp_ceiling_dbtp: f32,
    tp_release_ms: f32,
    last_measured_dbtp: Option<f32>,
    last_measured_lufs: Option<f32>,
    audio_status: String,

    // --- #28 Kinetic Subtitles & Localization ---
    kinetic_cfg: KineticSubtitleConfig,
    kinetic_keywords_input: String,
    kinetic_preview_progress: f32,
    loc_source_locale: String,
    loc_target_locales: String,
    loc_apply_locale: String,
    loc_csv_editor: String,
    sub_status: String,
}

impl Default for V21PanelsState {
    fn default() -> Self {
        let kinetic_cfg = KineticSubtitleConfig {
            enabled: true,
            ..KineticSubtitleConfig::default()
        };
        let kinetic_keywords_input = kinetic_cfg.auto_emphasize_keywords.join(", ");
        Self {
            show_window: false,
            active_tab: CinemaCreatorTab::ProColor,
            curve_channel: CurveChannel::Master,
            dragging_curve_pt: None,
            match_ref_sec: 0.0,
            match_tgt_sec: 2.0,
            color_status: "トーンカーブ・HSL二次補正・自動ショットカラーマッチを調整できます"
                .into(),
            stab_smoothness: 0.78,
            stab_max_shift_pct: 8.0,
            stab_interval_sec: 0.2,
            stab_rotation: true,
            stab_auto_crop: true,
            last_shake_samples: Vec::new(),
            dragging_pin_corner: None,
            vfx_status: "タイムラインでクリップを選択して手ぶれ補正・4点コーナーピンを適用できます"
                .into(),
            audio_target_clip: false,
            spatial_azimuth_deg: 35.0,
            spatial_elevation_deg: 0.0,
            spatial_distance: 1.5,
            spatial_room_mix: 0.14,
            mb_low_thresh: -18.0,
            mb_mid_thresh: -16.0,
            mb_high_thresh: -20.0,
            mb_ratio: 3.0,
            mb_makeup: 1.5,
            tp_ceiling_dbtp: -1.0,
            tp_release_ms: 60.0,
            last_measured_dbtp: None,
            last_measured_lufs: None,
            audio_status:
                "3Dバイノーラル空間定位・3バンドコンプ・-1.0 dBTP リミッターを設定できます".into(),
            kinetic_cfg,
            kinetic_keywords_input,
            kinetic_preview_progress: 0.42,
            loc_source_locale: "ja".into(),
            loc_target_locales: "en, zh, ko".into(),
            loc_apply_locale: "en".into(),
            loc_csv_editor: String::new(),
            sub_status: "ワード単位キネティック字幕＆多言語CSVローカライズを管理できます".into(),
        }
    }
}

impl V21PanelsState {
    /// v2.1 シネマ＆クリエイタースタジオウィンドウを描画する。
    /// プロジェクト状態が更新された場合は `true` を返す（プレビュー再描画用）。
    pub fn show_cinema_creator_window(
        &mut self,
        ctx: &egui::Context,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut open = self.show_window;
        let mut changed = false;

        egui::Window::new("🎞 シネマ＆クリエイタースタジオ (v2.1)")
            .open(&mut open)
            .default_size([720.0, 620.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.active_tab,
                        CinemaCreatorTab::ProColor,
                        "🎨 トーンカーブ / HSL / カラーマッチ",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        CinemaCreatorTab::VfxStabilizeCornerPin,
                        "📹 手ぶれ補正 / 4点コーナーピン",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        CinemaCreatorTab::SpatialMasterAudio,
                        "🎧 3D空間オーディオ / マスタリング",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        CinemaCreatorTab::KineticSubLocalize,
                        "💬 キネティック字幕 / 多言語切替",
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.active_tab {
                    CinemaCreatorTab::ProColor => {
                        changed |= self.show_pro_color_tab(ui, session);
                    }
                    CinemaCreatorTab::VfxStabilizeCornerPin => {
                        changed |= self.show_vfx_tab(ui, session, selected_item);
                    }
                    CinemaCreatorTab::SpatialMasterAudio => {
                        changed |= self.show_spatial_master_tab(ui, session, selected_item);
                    }
                    CinemaCreatorTab::KineticSubLocalize => {
                        changed |= self.show_kinetic_localize_tab(ui, session, selected_item);
                    }
                });
            });

        self.show_window = open;
        changed
    }

    // =========================================================================
    // Tab 1: #25 Pro Color (RGB Tone Curves + HSL Qualifier + Auto Color Match)
    // =========================================================================

    fn show_pro_color_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut changed = false;
        let mut grade = {
            let g = session.lock().unwrap();
            g.project
                .active_scene()
                .map(|s| s.color_grade.clone())
                .unwrap_or_default()
        };
        let mut grade_dirty = false;

        ui.heading("🎨 RGB / マスタートーンカーブ（Fritsch-Carlson 単調スプライン）");
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut grade.curves.enabled, "カーブ有効")
                .changed()
            {
                if grade.curves.enabled {
                    grade.enabled = true;
                }
                grade_dirty = true;
            }
            ui.separator();
            ui.selectable_value(&mut self.curve_channel, CurveChannel::Master, "Master");
            ui.selectable_value(&mut self.curve_channel, CurveChannel::Red, "R");
            ui.selectable_value(&mut self.curve_channel, CurveChannel::Green, "G");
            ui.selectable_value(&mut self.curve_channel, CurveChannel::Blue, "B");
            ui.separator();
            if ui.button("直線リセット").clicked() {
                grade.curves = ColorCurves {
                    enabled: grade.curves.enabled,
                    ..ColorCurves::default()
                };
                grade_dirty = true;
            }
            if ui.button("S-Curve").clicked() {
                grade.curves = ColorCurves::preset_s_curve();
                grade.enabled = true;
                grade_dirty = true;
            }
            if ui.button("フィルムフェード").clicked() {
                grade.curves = ColorCurves::preset_film_fade();
                grade.enabled = true;
                grade_dirty = true;
            }
            if ui.button("ウォームシネマ").clicked() {
                grade.curves = ColorCurves::preset_warm_cinema();
                grade.enabled = true;
                grade_dirty = true;
            }
        });

        ui.horizontal(|ui| {
            // インタラクティブ曲線キャンバス (260 x 200)
            let desired = Vec2::new(260.0, 200.0);
            let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, Color32::from_rgb(18, 20, 26));
            painter.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, Color32::from_rgb(64, 70, 84)),
                egui::StrokeKind::Inside,
            );

            // 4x4 グリッド＋対角基準線
            for step in 1..4 {
                let frac = step as f32 * 0.25;
                let gx = rect.left() + rect.width() * frac;
                let gy = rect.top() + rect.height() * frac;
                painter.line_segment(
                    [Pos2::new(gx, rect.top()), Pos2::new(gx, rect.bottom())],
                    Stroke::new(0.8, Color32::from_rgb(38, 42, 52)),
                );
                painter.line_segment(
                    [Pos2::new(rect.left(), gy), Pos2::new(rect.right(), gy)],
                    Stroke::new(0.8, Color32::from_rgb(38, 42, 52)),
                );
            }
            painter.line_segment(
                [rect.left_bottom(), rect.right_top()],
                Stroke::new(1.0, Color32::from_rgb(60, 64, 76)),
            );

            let (pts, stroke_color) = match self.curve_channel {
                CurveChannel::Master => {
                    (&mut grade.curves.master, Color32::from_rgb(235, 238, 245))
                }
                CurveChannel::Red => (&mut grade.curves.red, Color32::from_rgb(245, 92, 92)),
                CurveChannel::Green => (&mut grade.curves.green, Color32::from_rgb(92, 224, 128)),
                CurveChannel::Blue => (&mut grade.curves.blue, Color32::from_rgb(96, 168, 250)),
            };

            let to_screen = |p: [f32; 2]| -> Pos2 {
                Pos2::new(
                    rect.left() + p[0].clamp(0.0, 1.0) * rect.width(),
                    rect.bottom() - p[1].clamp(0.0, 1.0) * rect.height(),
                )
            };
            let from_screen = |pos: Pos2| -> [f32; 2] {
                [
                    ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                    ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
                ]
            };

            // ドラッグ／追加／右クリック削除のハンドリング
            if response.drag_started()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let mut hit = None;
                let mut best_d = 14.0f32;
                for (i, pt) in pts.iter().enumerate() {
                    let d = to_screen(*pt).distance(pos);
                    if d < best_d {
                        best_d = d;
                        hit = Some(i);
                    }
                }
                if let Some(idx) = hit {
                    self.dragging_curve_pt = Some(idx);
                } else if pts.len() < 12 {
                    let np = from_screen(pos);
                    pts.push(np);
                    pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
                    self.dragging_curve_pt = pts
                        .iter()
                        .position(|p| (p[0] - np[0]).abs() < 1e-4 && (p[1] - np[1]).abs() < 1e-4);
                    grade.curves.enabled = true;
                    grade.enabled = true;
                    grade_dirty = true;
                }
            }
            if response.dragged()
                && let (Some(idx), Some(pos)) =
                    (self.dragging_curve_pt, response.interact_pointer_pos())
                && idx < pts.len()
            {
                let mut np = from_screen(pos);
                let last_idx = pts.len() - 1;
                if idx == 0 {
                    np[0] = 0.0;
                } else if idx == last_idx {
                    np[0] = 1.0;
                } else {
                    let lo = pts[idx - 1][0] + 0.02;
                    let hi = pts[idx + 1][0] - 0.02;
                    np[0] = np[0].clamp(lo, hi.max(lo));
                }
                pts[idx] = np;
                grade.curves.enabled = true;
                grade.enabled = true;
                grade_dirty = true;
            }
            if response.drag_stopped() {
                self.dragging_curve_pt = None;
            }
            if response.secondary_clicked()
                && let Some(pos) = response.interact_pointer_pos()
                && pts.len() > 2
            {
                let mut hit = None;
                let mut best_d = 14.0f32;
                for (i, pt) in pts.iter().enumerate().skip(1).take(pts.len() - 2) {
                    let d = to_screen(*pt).distance(pos);
                    if d < best_d {
                        best_d = d;
                        hit = Some(i);
                    }
                }
                if let Some(idx) = hit {
                    pts.remove(idx);
                    grade_dirty = true;
                }
            }

            // 評価 LUT から滑らかなスプライン曲線を描画
            let lut = evaluate_tone_curve_lut(pts);
            let poly: Vec<Pos2> = (0..64)
                .map(|i| {
                    let idx = (i * 255) / 63;
                    let x = idx as f32 / 255.0;
                    to_screen([x, lut[idx]])
                })
                .collect();
            painter.add(egui::Shape::line(poly, Stroke::new(2.2, stroke_color)));

            for (i, pt) in pts.iter().enumerate() {
                let c = to_screen(*pt);
                let fill = if self.dragging_curve_pt == Some(i) {
                    Color32::YELLOW
                } else {
                    stroke_color
                };
                painter.circle_filled(c, 4.8, fill);
                painter.circle_stroke(c, 4.8, Stroke::new(1.0, Color32::BLACK));
            }

            ui.vertical(|ui| {
                ui.label("操作ヒント:");
                ui.small("• 制御点をドラッグして調整");
                ui.small("• 曲線上の空き領域をドラッグで点追加");
                ui.small("• 内側の点を右クリックで削除");
                ui.add_space(6.0);
                ui.label(format!("制御点数: {}", pts.len()));
            });
        });

        ui.separator();
        ui.heading("🎯 HSL 二次色補正クオリファイア（特定色相の抽出＆部分補正）");
        let q = &mut grade.hsl_qualifier;
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut q.enabled, "HSL クオリファイア有効")
                .changed()
            {
                if q.enabled {
                    grade.enabled = true;
                }
                grade_dirty = true;
            }
            if ui
                .checkbox(&mut q.show_matte_overlay, "👁 抽出マット可視化")
                .changed()
            {
                grade_dirty = true;
            }
            if ui.button("肌色 (25°)").clicked() {
                *q = HslQualifier {
                    enabled: true,
                    hue_center_deg: 25.0,
                    hue_width_deg: 24.0,
                    sat_min: 0.12,
                    ..*q
                };
                grade.enabled = true;
                grade_dirty = true;
            }
            if ui.button("青空 (210°)").clicked() {
                *q = HslQualifier {
                    enabled: true,
                    hue_center_deg: 210.0,
                    hue_width_deg: 35.0,
                    sat_min: 0.15,
                    sat_scale: 1.25,
                    ..*q
                };
                grade.enabled = true;
                grade_dirty = true;
            }
            if ui.button("新緑 (120°)").clicked() {
                *q = HslQualifier {
                    enabled: true,
                    hue_center_deg: 120.0,
                    hue_width_deg: 32.0,
                    sat_min: 0.14,
                    ..*q
                };
                grade.enabled = true;
                grade_dirty = true;
            }
        });

        egui::Grid::new("v21_hsl_grid")
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("中心色相 (°)");
                if ui
                    .add(egui::Slider::new(&mut q.hue_center_deg, 0.0..=360.0))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.label("色相幅 (±°)");
                if ui
                    .add(egui::Slider::new(&mut q.hue_width_deg, 5.0..=120.0))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.end_row();

                ui.label("最低彩度");
                if ui
                    .add(egui::Slider::new(&mut q.sat_min, 0.0..=1.0))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.label("ソフト幅");
                if ui
                    .add(egui::Slider::new(&mut q.softness, 0.02..=1.0))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.end_row();

                ui.label("色相シフト (°)");
                if ui
                    .add(egui::Slider::new(&mut q.hue_shift_deg, -180.0..=180.0))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.label("彩度倍率");
                if ui
                    .add(egui::Slider::new(&mut q.sat_scale, 0.0..=2.5))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.end_row();

                ui.label("輝度オフセット");
                if ui
                    .add(egui::Slider::new(&mut q.luma_offset, -0.5..=0.5))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.label("最低輝度");
                if ui
                    .add(egui::Slider::new(&mut q.luma_min, 0.0..=1.0))
                    .changed()
                {
                    grade_dirty = true;
                }
                ui.end_row();
            });

        if grade_dirty {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetSceneColorGrade { grade },
                    CommandSource::Gui,
                    None,
                )
                .is_ok()
            {
                changed = true;
                self.color_status = "カラーグレーディングを更新しました（1-Undo 対応）".into();
            }
        }

        ui.separator();
        ui.heading("⚖ 自動ショット間カラーマッチ（Auto Shot Color Match）");
        ui.horizontal_wrapped(|ui| {
            ui.label("参照時刻 (s):");
            ui.add(
                egui::DragValue::new(&mut self.match_ref_sec)
                    .speed(0.1)
                    .range(0.0..=3600.0),
            );
            if ui.small_button("現在位置").clicked() {
                self.match_ref_sec = session.lock().unwrap().playhead.as_seconds();
            }
            ui.label("対象時刻 (s):");
            ui.add(
                egui::DragValue::new(&mut self.match_tgt_sec)
                    .speed(0.1)
                    .range(0.0..=3600.0),
            );
            if ui.small_button("現在位置").clicked() {
                self.match_tgt_sec = session.lock().unwrap().playhead.as_seconds();
            }
            if ui.button("✨ 自動カラーマッチ適用").clicked() {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                let scene_id = s.project.active_scene_id;
                let mut rg = RenderGraph::new();
                rg.set_project_folder(s.folder.clone());
                if let (Some(ref_img), Some(tgt_img)) = (
                    rg.render_project_scene_scaled(
                        &s.project,
                        scene_id,
                        RationalTime::from_seconds(self.match_ref_sec),
                        320,
                        180,
                        false,
                    ),
                    rg.render_project_scene_scaled(
                        &s.project,
                        scene_id,
                        RationalTime::from_seconds(self.match_tgt_sec),
                        320,
                        180,
                        false,
                    ),
                ) {
                    let matched = compute_auto_color_match(&ref_img, &tgt_img);
                    let mut next = s
                        .project
                        .active_scene()
                        .map(|sc| sc.color_grade.clone())
                        .unwrap_or_default();
                    next.enabled = true;
                    next.lift = matched.lift;
                    next.gain = matched.gain;
                    next.saturation = matched.saturation;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetSceneColorGrade { grade: next },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        changed = true;
                        self.color_status = format!(
                            "カラーマッチ適用完了 (ref={:.2}s → tgt={:.2}s, sat={:.2})",
                            self.match_ref_sec, self.match_tgt_sec, matched.saturation
                        );
                    }
                } else {
                    self.color_status = "フレーム描画に失敗しました".into();
                }
            }
        });
        ui.small(&self.color_status);
        changed
    }

    // =========================================================================
    // Tab 2: #26 Auto Video Stabilizer + Planar 4-Point Corner Pin
    // =========================================================================

    fn show_vfx_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut changed = false;
        let target_info: Option<(ItemId, String, CornerPinConfig)> = {
            let g = session.lock().unwrap();
            g.project.active_scene().and_then(|sc| {
                let find_by_id = selected_item.and_then(|id| {
                    sc.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == id)
                });
                let item = find_by_id.or_else(|| {
                    sc.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| !matches!(i.kind, ItemKind::Audio { .. }))
                })?;
                Some((item.id, item.name.clone(), item.corner_pin))
            })
        };

        let Some((target_id, target_name, mut pin)) = target_info else {
            ui.label("対象となる映像・画像・シェイプクリップをタイムラインに追加してください。");
            return false;
        };

        ui.label(format!("対象クリップ: {target_name}"));
        ui.separator();

        // --- 1. Auto 2D Video Stabilizer ---
        ui.heading("📹 自動 2D 手ぶれ補正（ゼロ位相平滑化 ＋ 自動クロップ拡大）");
        egui::Grid::new("v21_stab_grid")
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("平滑化強度");
                ui.add(egui::Slider::new(&mut self.stab_smoothness, 0.1..=0.95));
                ui.label("最大補正幅 (%)");
                ui.add(egui::Slider::new(&mut self.stab_max_shift_pct, 1.0..=25.0));
                ui.end_row();

                ui.label("解析間隔 (秒)");
                ui.add(egui::Slider::new(&mut self.stab_interval_sec, 0.05..=1.0));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.stab_rotation, "回転補正");
                    ui.checkbox(&mut self.stab_auto_crop, "自動クロップ拡大");
                });
                ui.end_row();
            });

        if ui
            .button("🔍 手ぶれ解析 ＆ キーフレーム一括適用 (1-Undo)")
            .clicked()
        {
            let cfg = StabilizeConfig {
                sample_interval_sec: f64::from(self.stab_interval_sec),
                smoothness: self.stab_smoothness,
                max_shift_ratio: self.stab_max_shift_pct * 0.01,
                stabilize_rotation: self.stab_rotation,
                auto_crop_zoom: self.stab_auto_crop,
            };
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            let scene_id = s.project.active_scene_id;
            let mut rg = RenderGraph::new();
            rg.set_project_folder(s.folder.clone());
            let samples = analyze_camera_shake(&mut rg, &s.project, scene_id, &cfg);
            let cmds = s
                .project
                .active_scene()
                .map(|sc| build_stabilize_commands(sc, target_id, &samples, &cfg))
                .unwrap_or_default();
            let cmd_cnt = cmds.len();
            if !cmds.is_empty()
                && s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::Batch {
                            label: format!("自動手ぶれ補正 ({} サンプル)", samples.len()),
                            commands: cmds,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
            {
                changed = true;
                self.vfx_status = format!(
                    "手ぶれ補正を適用しました: {} サンプル / {} キーフレームコマンド生成",
                    samples.len(),
                    cmd_cnt
                );
            } else {
                self.vfx_status = "解析完了（対象クリップ範囲に生成可能なサンプルなし）".into();
            }
            self.last_shake_samples = samples;
        }

        // ブレ解析結果スパークライン表示
        if self.last_shake_samples.len() >= 2 {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(420.0, 68.0), egui::Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 3.0, Color32::from_rgb(18, 20, 26));
            let mid_y = rect.center().y;
            p.line_segment(
                [
                    Pos2::new(rect.left(), mid_y),
                    Pos2::new(rect.right(), mid_y),
                ],
                Stroke::new(0.8, Color32::from_rgb(56, 62, 74)),
            );
            let max_abs = self
                .last_shake_samples
                .iter()
                .map(|sm| sm.raw_x.abs().max(sm.comp_x.abs()))
                .fold(5.0f32, f32::max);
            let n = (self.last_shake_samples.len() - 1).max(1) as f32;
            let raw_pts: Vec<Pos2> = self
                .last_shake_samples
                .iter()
                .enumerate()
                .map(|(i, sm)| {
                    Pos2::new(
                        rect.left() + (i as f32 / n) * rect.width(),
                        mid_y - (sm.raw_x / max_abs) * (rect.height() * 0.42),
                    )
                })
                .collect();
            let comp_pts: Vec<Pos2> = self
                .last_shake_samples
                .iter()
                .enumerate()
                .map(|(i, sm)| {
                    Pos2::new(
                        rect.left() + (i as f32 / n) * rect.width(),
                        mid_y - (sm.comp_x / max_abs) * (rect.height() * 0.42),
                    )
                })
                .collect();
            p.add(egui::Shape::line(
                raw_pts,
                Stroke::new(1.5, Color32::from_rgb(240, 108, 92)),
            ));
            p.add(egui::Shape::line(
                comp_pts,
                Stroke::new(1.8, Color32::from_rgb(80, 214, 154)),
            ));
            ui.small("赤: 生カメラブレ軌跡 X / 緑: ゼロ位相打ち消し補正量 X");
        }

        ui.separator();

        // --- 2. Planar 4-Point Corner Pin Composite ---
        ui.heading("📐 平面 4 点コーナーピン射影合成（3×3 ホモグラフィ）");
        let mut pin_dirty = false;
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut pin.enabled, "コーナーピン有効").changed() {
                pin_dirty = true;
            }
            if ui.button("リセット").clicked() {
                pin = CornerPinConfig::default();
                pin_dirty = true;
            }
            if ui.button("モニター斜め投影").clicked() {
                pin = CornerPinConfig {
                    enabled: true,
                    tl: [0.14, 0.12],
                    tr: [0.88, 0.08],
                    br: [0.92, 0.86],
                    bl: [0.10, 0.90],
                    feather_px: 1.5,
                };
                pin_dirty = true;
            }
            if ui.button("左看板パース").clicked() {
                pin = CornerPinConfig {
                    enabled: true,
                    tl: [0.06, 0.08],
                    tr: [0.76, 0.18],
                    br: [0.76, 0.82],
                    bl: [0.06, 0.92],
                    feather_px: 1.5,
                };
                pin_dirty = true;
            }
            if ui.button("床面遠近投影").clicked() {
                pin = CornerPinConfig {
                    enabled: true,
                    tl: [0.24, 0.35],
                    tr: [0.76, 0.35],
                    br: [0.96, 0.94],
                    bl: [0.04, 0.94],
                    feather_px: 2.0,
                };
                pin_dirty = true;
            }
        });

        ui.horizontal(|ui| {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(260.0, 175.0), egui::Sense::drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, Color32::from_rgb(18, 20, 26));
            let inner = rect.shrink(14.0);
            painter.rect_stroke(
                inner,
                0.0,
                Stroke::new(0.8, Color32::from_rgb(56, 62, 74)),
                egui::StrokeKind::Inside,
            );

            let to_pos = |pt: [f32; 2]| -> Pos2 {
                Pos2::new(
                    inner.left() + pt[0] * inner.width(),
                    inner.top() + pt[1] * inner.height(),
                )
            };
            let from_pos = |pos: Pos2| -> [f32; 2] {
                [
                    ((pos.x - inner.left()) / inner.width()).clamp(-0.2, 1.2),
                    ((pos.y - inner.top()) / inner.height()).clamp(-0.2, 1.2),
                ]
            };

            let mut corners = [pin.tl, pin.tr, pin.br, pin.bl];
            if response.drag_started()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let mut best = None;
                let mut best_d = 18.0f32;
                for (i, c) in corners.iter().enumerate() {
                    let d = to_pos(*c).distance(pos);
                    if d < best_d {
                        best_d = d;
                        best = Some(i);
                    }
                }
                self.dragging_pin_corner = best;
            }
            if response.dragged()
                && let (Some(idx), Some(pos)) =
                    (self.dragging_pin_corner, response.interact_pointer_pos())
            {
                corners[idx] = from_pos(pos);
                pin.tl = corners[0];
                pin.tr = corners[1];
                pin.br = corners[2];
                pin.bl = corners[3];
                pin.enabled = true;
                pin_dirty = true;
            }
            if response.drag_stopped() {
                self.dragging_pin_corner = None;
            }

            let screen_pts = [
                to_pos(corners[0]),
                to_pos(corners[1]),
                to_pos(corners[2]),
                to_pos(corners[3]),
            ];
            let quad_col = Color32::from_rgb(90, 200, 250);
            for i in 0..4 {
                painter.line_segment(
                    [screen_pts[i], screen_pts[(i + 1) % 4]],
                    Stroke::new(2.0, quad_col),
                );
            }
            // 遠近法確認用の対角クロスヘア
            painter.line_segment(
                [screen_pts[0], screen_pts[2]],
                Stroke::new(0.8, Color32::from_rgba_unmultiplied(90, 200, 250, 90)),
            );
            painter.line_segment(
                [screen_pts[1], screen_pts[3]],
                Stroke::new(0.8, Color32::from_rgba_unmultiplied(90, 200, 250, 90)),
            );

            let labels = ["TL", "TR", "BR", "BL"];
            for (i, (&sp, &lbl)) in screen_pts.iter().zip(labels.iter()).enumerate() {
                let col = if self.dragging_pin_corner == Some(i) {
                    Color32::YELLOW
                } else {
                    Color32::from_rgb(255, 130, 80)
                };
                painter.circle_filled(sp, 6.0, col);
                painter.text(
                    sp + Vec2::new(7.0, -7.0),
                    egui::Align2::LEFT_BOTTOM,
                    lbl,
                    egui::FontId::monospace(10.0),
                    Color32::WHITE,
                );
            }

            ui.vertical(|ui| {
                ui.label("エッジフェザー (px)");
                if ui
                    .add(egui::Slider::new(&mut pin.feather_px, 0.0..=16.0))
                    .changed()
                {
                    pin_dirty = true;
                }
                ui.small(format!("TL: ({:.2}, {:.2})", pin.tl[0], pin.tl[1]));
                ui.small(format!("TR: ({:.2}, {:.2})", pin.tr[0], pin.tr[1]));
                ui.small(format!("BR: ({:.2}, {:.2})", pin.br[0], pin.br[1]));
                ui.small(format!("BL: ({:.2}, {:.2})", pin.bl[0], pin.bl[1]));
            });
        });

        if pin_dirty {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetCornerPin {
                        item_id: target_id,
                        corner_pin: pin,
                    },
                    CommandSource::Gui,
                    None,
                )
                .is_ok()
            {
                changed = true;
                self.vfx_status = "4点コーナーピン射影設定を更新しました".into();
            }
        }
        ui.small(&self.vfx_status);
        changed
    }

    // =========================================================================
    // Tab 3: #27 3D Spatial Audio Radar + Mastering Suite (Multiband + True-Peak)
    // =========================================================================

    fn show_spatial_master_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("適用先:");
            ui.selectable_value(&mut self.audio_target_clip, false, "マスターバス (全体)");
            ui.add_enabled_ui(selected_item.is_some(), |ui| {
                ui.selectable_value(&mut self.audio_target_clip, true, "選択中クリップ");
            });
        });
        let target_opt = if self.audio_target_clip {
            selected_item
        } else {
            None
        };

        ui.separator();
        ui.heading("🎧 3D バイノーラル空間オーディオパンナー（ITD + ILD + 頭部回折）");
        ui.horizontal(|ui| {
            // インタラクティブ 360° トップダウンレーダーキャンバス (180x180)
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(180.0, 180.0), egui::Sense::click_and_drag());
            let p = ui.painter_at(rect);
            let center = rect.center();
            let max_r = rect.width().min(rect.height()) * 0.44;
            p.rect_filled(rect, 6.0, Color32::from_rgb(16, 20, 28));
            for ring in [0.33f32, 0.66, 1.0] {
                p.circle_stroke(
                    center,
                    max_r * ring,
                    Stroke::new(0.9, Color32::from_rgb(48, 58, 76)),
                );
            }
            p.line_segment(
                [
                    Pos2::new(center.x - max_r, center.y),
                    Pos2::new(center.x + max_r, center.y),
                ],
                Stroke::new(0.8, Color32::from_rgb(44, 52, 66)),
            );
            p.line_segment(
                [
                    Pos2::new(center.x, center.y - max_r),
                    Pos2::new(center.x, center.y + max_r),
                ],
                Stroke::new(0.8, Color32::from_rgb(44, 52, 66)),
            );
            // 中央のリスナー頭部マーク
            p.circle_filled(center, 9.0, Color32::from_rgb(90, 130, 190));
            p.text(
                Pos2::new(center.x, rect.top() + 8.0),
                egui::Align2::CENTER_TOP,
                "FRONT (0°)",
                egui::FontId::monospace(9.0),
                Color32::LIGHT_GRAY,
            );

            if (response.dragged() || response.clicked())
                && let Some(pos) = response.interact_pointer_pos()
            {
                let dx = pos.x - center.x;
                let dy = center.y - pos.y; // 上=+Y (正面)
                self.spatial_azimuth_deg = dx.atan2(dy).to_degrees().clamp(-180.0, 180.0);
                let norm_r = (dx.hypot(dy) / max_r).clamp(0.08, 1.0);
                self.spatial_distance = (norm_r * 8.0).clamp(0.3, 10.0);
            }

            let az = self.spatial_azimuth_deg.to_radians();
            let dist_norm = (self.spatial_distance / 8.0).clamp(0.08, 1.0);
            let orb = Pos2::new(
                center.x + az.sin() * dist_norm * max_r,
                center.y - az.cos() * dist_norm * max_r,
            );
            p.line_segment(
                [center, orb],
                Stroke::new(1.5, Color32::from_rgb(80, 220, 190)),
            );
            p.circle_filled(orb, 7.5, Color32::from_rgb(255, 190, 60));

            ui.vertical(|ui| {
                ui.add(
                    egui::Slider::new(&mut self.spatial_azimuth_deg, -180.0..=180.0)
                        .text("方位角 (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut self.spatial_elevation_deg, -90.0..=90.0)
                        .text("仰角 (°)"),
                );
                ui.add(egui::Slider::new(&mut self.spatial_distance, 0.3..=10.0).text("距離 (m)"));
                ui.add(egui::Slider::new(&mut self.spatial_room_mix, 0.0..=0.6).text("初期反射"));
                if ui.button("🎧 3D バイノーラルをチェーンへ追加").clicked() {
                    let mut node = AudioEffectNode::with_defaults("spatial_3d");
                    node.params
                        .insert("azimuth_deg".into(), self.spatial_azimuth_deg);
                    node.params
                        .insert("elevation_deg".into(), self.spatial_elevation_deg);
                    node.params.insert("distance".into(), self.spatial_distance);
                    node.params.insert("room_mix".into(), self.spatial_room_mix);
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddAudioEffect {
                                item_id: target_opt,
                                effect: node,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        changed = true;
                        self.audio_status = format!(
                            "3D 空間オーディオ追加 (方位 {:.0}°, 距離 {:.1}m)",
                            self.spatial_azimuth_deg, self.spatial_distance
                        );
                    }
                }
            });
        });

        ui.separator();
        ui.heading(
            "🎛 マスタリング DSP スイート（3バンド・マルチバンドコンプ ＆ 4× True-Peak リミッター）",
        );
        egui::Grid::new("v21_master_grid")
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("Low (<250Hz) dB");
                ui.add(egui::Slider::new(&mut self.mb_low_thresh, -40.0..=0.0));
                ui.label("Mid (250-3.5k) dB");
                ui.add(egui::Slider::new(&mut self.mb_mid_thresh, -40.0..=0.0));
                ui.end_row();

                ui.label("High (>3.5kHz) dB");
                ui.add(egui::Slider::new(&mut self.mb_high_thresh, -40.0..=0.0));
                ui.label("Ratio / Makeup");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.mb_ratio)
                            .range(1.0..=12.0)
                            .speed(0.1)
                            .suffix(":1"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.mb_makeup)
                            .range(-6.0..=12.0)
                            .speed(0.1)
                            .suffix("dB"),
                    );
                });
                ui.end_row();

                ui.label("True-Peak 上限");
                ui.add(egui::Slider::new(&mut self.tp_ceiling_dbtp, -6.0..=-0.1).suffix(" dBTP"));
                ui.label("Release (ms)");
                ui.add(egui::Slider::new(&mut self.tp_release_ms, 10.0..=300.0));
                ui.end_row();
            });

        ui.horizontal_wrapped(|ui| {
            if ui.button("🎛 3バンドコンプを追加").clicked() {
                let mut node = AudioEffectNode::with_defaults("multiband_comp");
                node.params
                    .insert("low_thresh_db".into(), self.mb_low_thresh);
                node.params
                    .insert("mid_thresh_db".into(), self.mb_mid_thresh);
                node.params
                    .insert("high_thresh_db".into(), self.mb_high_thresh);
                node.params.insert("ratio".into(), self.mb_ratio);
                node.params.insert("makeup_db".into(), self.mb_makeup);
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::AddAudioEffect {
                            item_id: target_opt,
                            effect: node,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    changed = true;
                    self.audio_status = "3バンド・マルチバンドコンプレッサーを追加しました".into();
                }
            }
            if ui
                .button("🧱 True-Peak リミッターを追加 (-1.0 dBTP 準拠)")
                .clicked()
            {
                let mut node = AudioEffectNode::with_defaults("true_peak_limiter");
                node.params
                    .insert("ceiling_dbtp".into(), self.tp_ceiling_dbtp);
                node.params.insert("release_ms".into(), self.tp_release_ms);
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::AddAudioEffect {
                            item_id: target_opt,
                            effect: node,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    changed = true;
                    self.audio_status = format!(
                        "4× True-Peak リミッター追加 (上限 {:.1} dBTP)",
                        self.tp_ceiling_dbtp
                    );
                }
            }
            if ui.button("📊 マスター True-Peak / LUFS 計測").clicked() {
                let g = session.lock().unwrap();
                let scene_id = g.project.active_scene_id;
                let dur = g
                    .project
                    .active_scene()
                    .map(|sc| sc.total_duration().as_seconds().clamp(0.5, 15.0))
                    .unwrap_or(1.0);
                let sr = g.project.settings.audio_sample_rate.max(8_000);
                let ag =
                    AudioGraph::from_project_scene(&g.project, scene_id, g.folder.as_ref(), sr);
                let pcm = ag.render_range(RationalTime::ZERO, RationalTime::from_seconds(dur));
                let tp = measure_true_peak_dbtp(&pcm, ag.channels);
                let lufs = measure_integrated_lufs(&pcm, sr);
                self.last_measured_dbtp = Some(tp);
                self.last_measured_lufs = Some(lufs);
                self.audio_status = format!(
                    "計測完了 ({dur:.1}s): True-Peak = {tp:.2} dBTP / Integrated = {lufs:.1} LUFS"
                );
            }
        });

        if let (Some(tp), Some(lufs)) = (self.last_measured_dbtp, self.last_measured_lufs) {
            let safe = tp <= -1.0;
            let badge_col = if safe {
                Color32::from_rgb(76, 210, 128)
            } else {
                Color32::from_rgb(245, 92, 92)
            };
            ui.horizontal(|ui| {
                ui.colored_label(
                    badge_col,
                    format!(
                        "{} True-Peak: {:.2} dBTP  |  Integrated: {:.1} LUFS",
                        if safe {
                            "✅ 放送・配信安全"
                        } else {
                            "⚠ -1.0 dBTP 超過"
                        },
                        tp,
                        lufs
                    ),
                );
            });
        }
        ui.small(&self.audio_status);
        changed
    }

    // =========================================================================
    // Tab 4: #28 Dynamic Kinetic Subtitles + Multi-Language Localization CSV
    // =========================================================================

    fn show_kinetic_localize_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut changed = false;

        ui.heading("💬 ワード単位ダイナミック・キネティック字幕（Shorts / TikTok 最適化）");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.kinetic_cfg.enabled, "キネティック有効");
            for &mode in KineticHighlightMode::ALL {
                ui.selectable_value(&mut self.kinetic_cfg.mode, mode, mode.display_name());
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("ポップ拡大率:");
            ui.add(egui::Slider::new(
                &mut self.kinetic_cfg.pop_scale,
                1.0..=1.8,
            ));
            let mut active_arr = [
                self.kinetic_cfg.active_color.r,
                self.kinetic_cfg.active_color.g,
                self.kinetic_cfg.active_color.b,
            ];
            ui.label("アクティブ色:");
            if ui.color_edit_button_rgb(&mut active_arr).changed() {
                self.kinetic_cfg.active_color = RgbaColor {
                    r: active_arr[0],
                    g: active_arr[1],
                    b: active_arr[2],
                    a: 1.0,
                };
            }
            let mut emph_arr = [
                self.kinetic_cfg.emphasis_color.r,
                self.kinetic_cfg.emphasis_color.g,
                self.kinetic_cfg.emphasis_color.b,
            ];
            ui.label("強調語色:");
            if ui.color_edit_button_rgb(&mut emph_arr).changed() {
                self.kinetic_cfg.emphasis_color = RgbaColor {
                    r: emph_arr[0],
                    g: emph_arr[1],
                    b: emph_arr[2],
                    a: 1.0,
                };
            }
        });

        ui.horizontal(|ui| {
            ui.label("自動強調キーワード:");
            if ui
                .text_edit_singleline(&mut self.kinetic_keywords_input)
                .changed()
            {
                self.kinetic_cfg.auto_emphasize_keywords = self
                    .kinetic_keywords_input
                    .split([',', '、'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        });

        // ライブプレビューバー
        ui.horizontal(|ui| {
            ui.label("プレビュー進行:");
            ui.add(egui::Slider::new(
                &mut self.kinetic_preview_progress,
                0.0..=1.0,
            ));
        });
        let sample_spans = tokenize_kinetic_words(
            "ここが 重要な ポイント です！今すぐチェック",
            &self.kinetic_cfg.auto_emphasize_keywords,
        );
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(18, 20, 26))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let p = self.kinetic_preview_progress;
                    for sp in &sample_spans {
                        let active =
                            p >= sp.start_frac && (p < sp.end_frac || sp.end_frac >= 0.999);
                        let col = if active {
                            rgba_to_color32(self.kinetic_cfg.active_color)
                        } else if sp.is_emphasized {
                            rgba_to_color32(self.kinetic_cfg.emphasis_color)
                        } else {
                            rgba_to_color32(self.kinetic_cfg.inactive_color)
                        };
                        let sz = if active {
                            16.0 * self.kinetic_cfg.pop_scale
                        } else {
                            16.0
                        };
                        ui.label(egui::RichText::new(&sp.text).size(sz).color(col).strong());
                    }
                });
            });

        ui.horizontal_wrapped(|ui| {
            if let Some(sel_id) = selected_item
                && ui.button("✨ 選択字幕に適用").clicked()
            {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetKineticSubtitle {
                            item_id: sel_id,
                            kinetic: self.kinetic_cfg.clone(),
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    changed = true;
                    self.sub_status = "選択字幕にキネティック設定を適用しました".into();
                }
            }
            if ui.button("⚡ シーン全字幕へ一括適用 (1-Undo)").clicked() {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                let ids: Vec<Uuid> = s
                    .project
                    .active_scene()
                    .map(|sc| {
                        sc.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .filter(|i| matches!(i.kind, ItemKind::Text { .. }))
                            .map(|i| i.id)
                            .collect()
                    })
                    .unwrap_or_default();
                if ids.is_empty() {
                    self.sub_status = "シーン内に字幕・テキストアイテムがありません".into();
                } else {
                    let cnt = ids.len();
                    let cmds = ids
                        .into_iter()
                        .map(|id| EditCommand::SetKineticSubtitle {
                            item_id: id,
                            kinetic: self.kinetic_cfg.clone(),
                        })
                        .collect();
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!("キネティック字幕一括適用 ({cnt}件)"),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        changed = true;
                        self.sub_status = format!("{cnt} 件の字幕に一括適用しました");
                    }
                }
            }
        });

        ui.separator();
        ui.heading("🌐 多言語字幕ローカライズ表（CSV 入出力 ＆ 1-Undo 言語切替）");
        ui.horizontal_wrapped(|ui| {
            ui.label("基準言語:");
            ui.add(egui::TextEdit::singleline(&mut self.loc_source_locale).desired_width(48.0));
            ui.label("追加列:");
            ui.add(egui::TextEdit::singleline(&mut self.loc_target_locales).desired_width(120.0));
            if ui.button("📋 シーンからCSV抽出").clicked() {
                let g = session.lock().unwrap();
                if let Some(sc) = g.project.active_scene() {
                    let mut tbl = extract_localization_table(sc, &self.loc_source_locale);
                    for loc in self
                        .loc_target_locales
                        .split([',', ' '])
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        if !tbl.locales.iter().any(|l| l == loc) {
                            tbl.locales.push(loc.to_string());
                        }
                    }
                    self.loc_csv_editor = export_localization_csv(&tbl);
                    self.sub_status = format!("{} 件の字幕を CSV 抽出しました", tbl.entries.len());
                }
            }
            ui.separator();
            ui.label("切替先言語:");
            ui.add(egui::TextEdit::singleline(&mut self.loc_apply_locale).desired_width(48.0));
            if ui.button("🌐 指定言語へ一括切替 (1-Undo)").clicked() {
                let tbl = import_localization_csv(&self.loc_csv_editor);
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                let cmds = s
                    .project
                    .active_scene()
                    .map(|sc| build_apply_locale_commands(sc, &tbl, &self.loc_apply_locale))
                    .unwrap_or_default();
                let cnt = cmds.len();
                if cnt > 0
                    && s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!(
                                    "字幕言語切替 [{}] ({cnt}件)",
                                    self.loc_apply_locale
                                ),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                {
                    changed = true;
                    self.sub_status = format!(
                        "言語 [{}] へ {} 件の字幕を一括切替しました（Ctrl+Z で元言語へ戻せます）",
                        self.loc_apply_locale, cnt
                    );
                } else {
                    self.sub_status = format!(
                        "言語 [{}] の翻訳セルが CSV 内に見つかりません",
                        self.loc_apply_locale
                    );
                }
            }
        });

        egui::ScrollArea::vertical()
            .id_salt("v21_loc_csv_scroll")
            .max_height(150.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.loc_csv_editor)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(6),
                );
            });
        ui.small(&self.sub_status);
        changed
    }
}

fn rgba_to_color32(c: RgbaColor) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}
