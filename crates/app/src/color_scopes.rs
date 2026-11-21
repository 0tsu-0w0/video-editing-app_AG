//! v0.8 #13: カラーグレーディング UI（3-Way カラーホイール・`.cube` 3D LUT）＆ リアルタイムビデオスコープ描画

use crate::native_dialog;
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use ve_core::{CommandSource, EditCommand, SceneColorGrade};
use ve_mcp::EditorSession;
use ve_render::{
    BUILTIN_LUT_PRESETS, HIST_BINS, PARADE_BINS, PARADE_COLS, VECTORSCOPE_SIZE, VideoScopeData,
    WAVEFORM_BINS, WAVEFORM_COLS, emit_sample_cube_luts,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeViewMode {
    Waveform,
    RgbParade,
    Vectorscope,
    Histogram,
    Quad,
}

pub struct ColorScopesState {
    pub show_scopes: bool,
    pub mode: ScopeViewMode,
    pub latest_scopes: VideoScopeData,
    pub lut_path_input: String,
    pub lut_status: String,
}

impl Default for ColorScopesState {
    fn default() -> Self {
        Self {
            show_scopes: false,
            mode: ScopeViewMode::Quad,
            latest_scopes: VideoScopeData::default(),
            lut_path_input: String::new(),
            lut_status: String::new(),
        }
    }
}

impl ColorScopesState {
    /// インスペクタ用: シーン全体の 3-Way カラーグレーディング ＆ `.cube` 3D LUT 編集 UI
    /// 変更があった場合 `true` を返す（プレビュー再描画トリガー用）
    pub fn show_color_grading_ui(&mut self, ui: &mut Ui, session: &mut EditorSession) -> bool {
        let mut dirty = false;
        ui.collapsing(
            "🎨 カラーグレーディング（3-Way ホイール / .cube 3D LUT）",
            |ui| {
                let Some(mut grade) = session
                    .project
                    .active_scene()
                    .map(|s| s.color_grade.clone())
                else {
                    return;
                };
                let prev = grade.clone();

                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut grade.enabled, "カラーグレーディングを有効化");
                    if ui.small_button("🔄 リセット").clicked() {
                        grade = SceneColorGrade::default();
                    }
                    ui.toggle_value(&mut self.show_scopes, "📊 スコープ表示");
                });

                if grade.enabled {
                    // 1. 基本補正（露出・コントラスト・彩度・色温度・ティント）
                    ui.add(egui::Slider::new(&mut grade.exposure, -3.0..=3.0).text("露出 (EV)"));
                    ui.add(egui::Slider::new(&mut grade.contrast, 0.2..=2.5).text("コントラスト"));
                    ui.add(egui::Slider::new(&mut grade.saturation, 0.0..=2.5).text("彩度"));
                    ui.add(
                        egui::Slider::new(&mut grade.temperature, -1.0..=1.0)
                            .text("色温度 (青←→暖)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut grade.tint, -1.0..=1.0).text("ティント (緑←→桃)"),
                    );

                    // 2. 3-Way カラーホイール (Lift / Gamma / Gain)
                    ui.separator();
                    ui.weak("3-Way カラーホイール（ドラッグで色味調整 / 右クリックでリセット）:");
                    ui.horizontal(|ui| {
                        draw_color_wheel(ui, "Lift (暗部)", &mut grade.lift, 0.0, 0.35);
                        draw_color_wheel(ui, "Gamma (中間)", &mut grade.gamma, 0.0, 0.35);
                        draw_color_wheel(ui, "Gain (明部)", &mut grade.gain, 1.0, 0.65);
                    });

                    // 3. `.cube` 3D LUT
                    ui.separator();
                    ui.strong("🎞 .cube 3D LUT（四面体補間）");
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(grade.lut_path.is_none(), "LUT なし")
                            .clicked()
                        {
                            grade.lut_path = None;
                        }
                        for &(preset_id, label) in BUILTIN_LUT_PRESETS {
                            let active = grade.lut_path.as_deref() == Some(preset_id);
                            if ui.selectable_label(active, label).clicked() {
                                grade.lut_path = Some(preset_id.to_string());
                            }
                        }
                    });

                    // プロジェクト luts/*.cube 一覧
                    let project_luts = session
                        .folder
                        .as_ref()
                        .map(|f| f.list_luts())
                        .unwrap_or_default();
                    if !project_luts.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.weak("luts/:");
                            for p in project_luts {
                                let name = p
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                let path_str = p.display().to_string();
                                let active = grade.lut_path.as_deref() == Some(path_str.as_str());
                                if ui.selectable_label(active, name).clicked() {
                                    grade.lut_path = Some(path_str);
                                }
                            }
                        });
                    }

                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.lut_path_input)
                                .desired_width(135.0)
                                .hint_text(".cube ファイルパス..."),
                        );
                        if ui.small_button("📂 .cube").clicked()
                            && let Some(p) =
                                native_dialog::pick_file(".cube 3D LUT を選択", &["cube"])
                        {
                            let s = p.display().to_string();
                            self.lut_path_input = s.clone();
                            grade.lut_path = Some(s);
                            self.lut_status = "LUT を読み込みました".into();
                        }
                        if ui.small_button("適用").clicked()
                            && !self.lut_path_input.trim().is_empty()
                        {
                            grade.lut_path = Some(self.lut_path_input.trim().to_string());
                        }
                        if ui
                            .small_button("📦 サンプル3種")
                            .on_hover_text("luts/ へ標準 .cube LUT 3種を書き出す")
                            .clicked()
                        {
                            let dir = session
                                .folder
                                .as_ref()
                                .map(|f| f.luts_dir())
                                .unwrap_or_else(|| std::path::PathBuf::from("luts"));
                            match emit_sample_cube_luts(&dir) {
                                Ok(paths) => {
                                    self.lut_status = format!(
                                        "{} へサンプル .cube {} 種を生成しました",
                                        dir.display(),
                                        paths.len()
                                    );
                                }
                                Err(e) => {
                                    self.lut_status = format!("LUT 生成エラー: {e}");
                                }
                            }
                        }
                    });

                    if grade.lut_path.is_some() {
                        ui.add(
                            egui::Slider::new(&mut grade.lut_intensity, 0.0..=1.0)
                                .text("LUT 適用強度"),
                        );
                    }
                    if !self.lut_status.is_empty() {
                        ui.weak(&self.lut_status);
                    }
                }

                if grade != prev
                    && session
                        .bus
                        .execute(
                            &mut session.project,
                            EditCommand::SetSceneColorGrade { grade },
                            CommandSource::Gui,
                            Some("scene_color_grade"),
                        )
                        .is_ok()
                {
                    dirty = true;
                }
            },
        );
        dirty
    }

    /// プレビュー下部または独立枠へ描画するリアルタイムビデオスコープ UI
    pub fn show_video_scopes_panel(&mut self, ui: &mut Ui) {
        let d = &self.latest_scopes;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("📊 ビデオスコープ (Rec.709)");
                ui.selectable_value(&mut self.mode, ScopeViewMode::Quad, "4分割");
                ui.selectable_value(&mut self.mode, ScopeViewMode::Waveform, "Waveform");
                ui.selectable_value(&mut self.mode, ScopeViewMode::RgbParade, "RGBパレード");
                ui.selectable_value(&mut self.mode, ScopeViewMode::Vectorscope, "ベクトル");
                ui.selectable_value(&mut self.mode, ScopeViewMode::Histogram, "ヒストグラム");
                ui.separator();
                ui.monospace(format!(
                    "Avg:{:.0}IRE Peak:{:.0}IRE Clip:{:.1}% Crush:{:.1}%",
                    d.avg_luma_ire, d.peak_luma_ire, d.clipped_highlight_pct, d.crushed_shadow_pct
                ));
            });

            let avail_w = ui.available_width().max(240.0);
            let scope_h = 132.0;
            match self.mode {
                ScopeViewMode::Waveform => {
                    draw_waveform_scope(ui, d, Vec2::new(avail_w, scope_h));
                }
                ScopeViewMode::RgbParade => {
                    draw_parade_scope(ui, d, Vec2::new(avail_w, scope_h));
                }
                ScopeViewMode::Vectorscope => {
                    draw_vectorscope(ui, d, Vec2::new(scope_h * 1.4, scope_h));
                }
                ScopeViewMode::Histogram => {
                    draw_histogram_scope(ui, d, Vec2::new(avail_w, scope_h));
                }
                ScopeViewMode::Quad => {
                    let cell_w = ((avail_w - 18.0) * 0.25).max(80.0);
                    ui.horizontal(|ui| {
                        draw_waveform_scope(ui, d, Vec2::new(cell_w, scope_h));
                        draw_parade_scope(ui, d, Vec2::new(cell_w, scope_h));
                        draw_vectorscope(ui, d, Vec2::new(cell_w, scope_h));
                        draw_histogram_scope(ui, d, Vec2::new(cell_w, scope_h));
                    });
                }
            }
        });
    }
}

/// ドラッグ操作可能な円形 3-Way カラーホイール（Lift / Gamma / Gain）
fn draw_color_wheel(ui: &mut Ui, label: &str, rgb: &mut [f32; 3], neutral: f32, range: f32) {
    ui.vertical(|ui| {
        ui.weak(label);
        let size = 76.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let center = rect.center();
        let radius = size * 0.44;

        painter.circle_filled(center, radius, Color32::from_rgb(26, 28, 36));
        painter.circle_stroke(center, radius, Stroke::new(1.2, Color32::from_gray(95)));
        // RGB 色相方位の小さな目安ドット（赤=右上、緑=下、青=左上）
        for (ang_deg, col) in [
            (-30.0f32, Color32::from_rgb(230, 80, 80)),
            (90.0, Color32::from_rgb(80, 220, 100)),
            (210.0, Color32::from_rgb(80, 140, 240)),
        ] {
            let rad = ang_deg.to_radians();
            let p = Pos2::new(
                center.x + rad.cos() * (radius - 4.0),
                center.y + rad.sin() * (radius - 4.0),
            );
            painter.circle_filled(p, 2.5, col);
        }
        // 十字線
        painter.line_segment(
            [
                Pos2::new(center.x - radius, center.y),
                Pos2::new(center.x + radius, center.y),
            ],
            Stroke::new(0.7, Color32::from_gray(55)),
        );
        painter.line_segment(
            [
                Pos2::new(center.x, center.y - radius),
                Pos2::new(center.x, center.y + radius),
            ],
            Stroke::new(0.7, Color32::from_gray(55)),
        );

        // 現在の RGB オフセットから 2D ホイール座標 (dx, dy) を逆算
        let dr = rgb[0] - neutral;
        let dg = rgb[1] - neutral;
        let db = rgb[2] - neutral;
        let cur_x = ((dr - db) / (range * 1.4).max(1e-4)).clamp(-1.0, 1.0);
        let cur_y = ((dg - 0.5 * (dr + db)) / (range * 1.4).max(1e-4)).clamp(-1.0, 1.0);
        let dot_pos = Pos2::new(center.x + cur_x * radius, center.y + cur_y * radius);
        painter.line_segment([center, dot_pos], Stroke::new(1.2, Color32::from_gray(180)));
        painter.circle_filled(dot_pos, 4.5, Color32::from_rgb(255, 226, 96));
        painter.circle_stroke(dot_pos, 4.5, Stroke::new(1.0, Color32::BLACK));

        if resp.secondary_clicked() {
            *rgb = [neutral, neutral, neutral];
        } else if (resp.dragged() || resp.clicked())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let mut vx = (pos.x - center.x) / radius;
            let mut vy = (pos.y - center.y) / radius;
            let len = vx.hypot(vy);
            if len > 1.0 {
                vx /= len;
                vy /= len;
            }
            // ホイール方向を R/G/B へ直感的にマッピング
            let r_off = (vx * 0.85 - vy * 0.35) * range;
            let g_off = (vy * 0.85) * range;
            let b_off = (-vx * 0.85 - vy * 0.35) * range;
            rgb[0] = (neutral + r_off).clamp(neutral - range, neutral + range);
            rgb[1] = (neutral + g_off).clamp(neutral - range, neutral + range);
            rgb[2] = (neutral + b_off).clamp(neutral - range, neutral + range);
        }
    });
}

fn draw_waveform_scope(ui: &mut Ui, data: &VideoScopeData, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(12, 14, 18));
    draw_ire_graticule(&painter, rect, "Waveform (IRE)");

    let max_v = f32::from(data.waveform_max.max(1));
    let cw = rect.width() / WAVEFORM_COLS as f32;
    let bh = rect.height() / WAVEFORM_BINS as f32;
    for bin in 0..WAVEFORM_BINS {
        let y = rect.bottom() - (bin as f32 + 1.0) * bh;
        let row = bin * WAVEFORM_COLS;
        for col in 0..WAVEFORM_COLS {
            let v = data.waveform[row + col];
            if v == 0 {
                continue;
            }
            let intensity = (f32::from(v) / max_v).sqrt().clamp(0.15, 1.0);
            let alpha = (intensity * 235.0) as u8;
            let x = rect.left() + col as f32 * cw;
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw.max(1.2), bh.max(1.4))),
                0.0,
                Color32::from_rgba_unmultiplied(92, 235, 156, alpha),
            );
        }
    }
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        StrokeKind::Inside,
    );
}

fn draw_parade_scope(ui: &mut Ui, data: &VideoScopeData, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(12, 14, 18));
    draw_ire_graticule(&painter, rect, "RGB Parade");

    let max_v = f32::from(data.parade_max.max(1));
    let third_w = rect.width() / 3.0;
    let cw = third_w / PARADE_COLS as f32;
    let bh = rect.height() / PARADE_BINS as f32;

    let channels = [
        (data.parade_r.as_slice(), 245u8, 78u8, 78u8),
        (data.parade_g.as_slice(), 78u8, 230u8, 112u8),
        (data.parade_b.as_slice(), 82u8, 152u8, 250u8),
    ];
    for (ch_idx, (grid, cr, cg, cb)) in channels.iter().enumerate() {
        let x_base = rect.left() + ch_idx as f32 * third_w;
        for bin in 0..PARADE_BINS {
            let y = rect.bottom() - (bin as f32 + 1.0) * bh;
            let row = bin * PARADE_COLS;
            for col in 0..PARADE_COLS {
                let v = grid[row + col];
                if v == 0 {
                    continue;
                }
                let intensity = (f32::from(v) / max_v).sqrt().clamp(0.18, 1.0);
                let x = x_base + col as f32 * cw;
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw.max(1.0), bh.max(1.4))),
                    0.0,
                    Color32::from_rgba_unmultiplied(*cr, *cg, *cb, (intensity * 235.0) as u8),
                );
            }
        }
    }
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        StrokeKind::Inside,
    );
}

fn draw_vectorscope(ui: &mut Ui, data: &VideoScopeData, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(12, 14, 18));

    let center = rect.center();
    let rad = rect.width().min(rect.height()) * 0.45;
    painter.circle_stroke(center, rad, Stroke::new(1.0, Color32::from_gray(64)));
    painter.circle_stroke(center, rad * 0.5, Stroke::new(0.7, Color32::from_gray(44)));
    painter.line_segment(
        [
            Pos2::new(center.x - rad, center.y),
            Pos2::new(center.x + rad, center.y),
        ],
        Stroke::new(0.7, Color32::from_gray(44)),
    );
    painter.line_segment(
        [
            Pos2::new(center.x, center.y - rad),
            Pos2::new(center.x, center.y + rad),
        ],
        Stroke::new(0.7, Color32::from_gray(44)),
    );
    // スキントーンライン（I 軸、約 123° 方向）
    let skin_ang = (-57.0f32).to_radians();
    painter.line_segment(
        [
            center,
            Pos2::new(
                center.x - skin_ang.cos() * rad,
                center.y + skin_ang.sin() * rad,
            ),
        ],
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(230, 170, 110, 120)),
    );

    let max_v = f32::from(data.vectorscope_max.max(1));
    let scale = (rad * 2.0) / VECTORSCOPE_SIZE as f32;
    let origin = Pos2::new(center.x - rad, center.y - rad);
    for vy in 0..VECTORSCOPE_SIZE {
        let row = vy * VECTORSCOPE_SIZE;
        for vx in 0..VECTORSCOPE_SIZE {
            let v = data.vectorscope[row + vx];
            if v == 0 {
                continue;
            }
            let intensity = (f32::from(v) / max_v).sqrt().clamp(0.2, 1.0);
            let p = Pos2::new(origin.x + vx as f32 * scale, origin.y + vy as f32 * scale);
            painter.rect_filled(
                Rect::from_min_size(p, Vec2::splat(scale.max(1.5))),
                0.0,
                Color32::from_rgba_unmultiplied(110, 232, 224, (intensity * 240.0) as u8),
            );
        }
    }

    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 3.0),
        egui::Align2::LEFT_TOP,
        "Vectorscope",
        egui::FontId::monospace(9.5),
        Color32::from_gray(165),
    );
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        StrokeKind::Inside,
    );
}

fn draw_histogram_scope(ui: &mut Ui, data: &VideoScopeData, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(12, 14, 18));

    let max_v = data.hist_max.max(1) as f32;
    let bw = rect.width() / HIST_BINS as f32;

    let draw_curve = |bins: &[u32; HIST_BINS], color: Color32| {
        let mut pts = Vec::with_capacity(HIST_BINS);
        for (i, &v) in bins.iter().enumerate() {
            let frac = (v as f32 / max_v).sqrt().clamp(0.0, 1.0);
            let x = rect.left() + (i as f32 + 0.5) * bw;
            let y = rect.bottom() - frac * (rect.height() - 14.0);
            pts.push(Pos2::new(x, y));
        }
        for seg in pts.windows(2) {
            painter.line_segment([seg[0], seg[1]], Stroke::new(1.4, color));
        }
    };

    draw_curve(
        &data.hist_luma,
        Color32::from_rgba_unmultiplied(215, 215, 220, 175),
    );
    draw_curve(
        &data.hist_r,
        Color32::from_rgba_unmultiplied(245, 86, 86, 210),
    );
    draw_curve(
        &data.hist_g,
        Color32::from_rgba_unmultiplied(84, 232, 118, 210),
    );
    draw_curve(
        &data.hist_b,
        Color32::from_rgba_unmultiplied(92, 162, 250, 210),
    );

    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 3.0),
        egui::Align2::LEFT_TOP,
        "Histogram (RGBY)",
        egui::FontId::monospace(9.5),
        Color32::from_gray(165),
    );
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        StrokeKind::Inside,
    );
}

fn draw_ire_graticule(painter: &egui::Painter, rect: Rect, title: &str) {
    for frac in [0.25f32, 0.5, 0.75] {
        let y = rect.bottom() - rect.height() * frac;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(0.6, Color32::from_gray(42)),
        );
    }
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 3.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::monospace(9.5),
        Color32::from_gray(165),
    );
}
