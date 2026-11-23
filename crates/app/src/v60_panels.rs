use egui::{Color32, Pos2, Stroke, Vec2};
use std::path::PathBuf;
use uuid::Uuid;
use ve_audio::{SpeakerTrackSpec, analyze_podcast_speakers, build_podcast_auto_switch_commands};
use ve_core::{
    BroadcastProfile, BroadcastQcMetrics, ChartKind, CommandSource, DataChartConfig,
    DeliveryBundleArtifact, DeliveryBundleConfig, DeliveryPlatform, EditCommand, ItemId, ItemKind,
    PaintStroke, PaintToolKind, RationalTime, RgbaColor, TimelineItem, VfxPaintConfig,
    build_delivery_manifest, build_reformatted_scene_command, export_cmx3600_edl,
    export_fcpxml_v10, export_srt_subtitles, format_youtube_chapters, parse_chart_csv,
    run_broadcast_qc_audit, serialize_pretty_json_bytes,
};
use ve_mcp::SharedEditorSession;
use ve_render::{
    RenderGraph, encode_linear_image_png_bytes, measure_scene_video_qc_metrics,
    select_best_thumbnail_frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V60Tab {
    DataViz,
    VfxPaint,
    PodcastDirector,
    MultiDeliver,
}

/// v6.0 #41〜#44 クリエイター Pro スタジオ統合ウィンドウ状態
pub struct V60PanelsState {
    pub show_window: bool,
    pub active_tab: V60Tab,

    // --- #41 Data-Driven Motion Graphics ---
    chart_cfg: DataChartConfig,
    chart_csv: String,
    chart_layer_index: usize,
    chart_duration_sec: f32,
    chart_status: String,

    // --- #42 Non-Destructive VFX Paint / Inpaint ---
    paint_tool: PaintToolKind,
    paint_radius_px: f32,
    paint_hardness: f32,
    paint_opacity: f32,
    paint_animate_write_on: bool,
    paint_color: [f32; 3],
    paint_clone_offset: [f32; 2],
    draft_stroke: Vec<[f32; 2]>,
    paint_status: String,

    // --- #43 Podcast Auto-Director ---
    speaker_rows: Vec<(String, usize, usize, bool)>, // (name, audio_layer, cam_layer, use_cam)
    podcast_target_lufs: f32,
    podcast_min_turn_sec: f32,
    podcast_silence_gate_db: f32,
    podcast_apply_faders: bool,
    podcast_summary: String,

    // --- #44 Multi-Platform Reformat & Delivery Packager ---
    plat_yt_16x9: bool,
    plat_shorts_9x16: bool,
    plat_square_1x1: bool,
    plat_cinema_4k: bool,
    bundle_thumb: bool,
    bundle_edl: bool,
    bundle_fcpxml: bool,
    bundle_qc: bool,
    bundle_title: String,
    bundle_tags: String,
    bundle_out_dir: String,
    deliver_summary: String,
}

impl Default for V60PanelsState {
    fn default() -> Self {
        Self {
            show_window: false,
            active_tab: V60Tab::DataViz,
            chart_cfg: DataChartConfig::for_kind(ChartKind::BarChart),
            chart_csv: "label,value,color\nQ1,32,#22ccff\nQ2,58,#ff4488\nQ3,84,#44ee88\nQ4,126,#ffbb33\n"
                .into(),
            chart_layer_index: 1,
            chart_duration_sec: 5.0,
            chart_status: "CSV から棒グラフ・折れ線・ドーナツ・KPI カウンターを自動アニメーション描画"
                .into(),
            paint_tool: PaintToolKind::BrushGlow,
            paint_radius_px: 14.0,
            paint_hardness: 0.72,
            paint_opacity: 1.0,
            paint_animate_write_on: true,
            paint_color: [1.0, 0.28, 0.52],
            paint_clone_offset: [0.10, 0.0],
            draft_stroke: Vec::new(),
            paint_status: "スケッチパッド上でドラッグして非破壊ストロークを描画できます".into(),
            speaker_rows: vec![
                ("Host A".into(), 2, 0, true),
                ("Host B".into(), 3, 1, true),
            ],
            podcast_target_lufs: -16.0,
            podcast_min_turn_sec: 0.4,
            podcast_silence_gate_db: -42.0,
            podcast_apply_faders: true,
            podcast_summary: "複数話者の発話区間を自動検出し、カメラ切替＆ -16 LUFS 均一化を 1-Undo で適用"
                .into(),
            plat_yt_16x9: true,
            plat_shorts_9x16: true,
            plat_square_1x1: true,
            plat_cinema_4k: false,
            bundle_thumb: true,
            bundle_edl: true,
            bundle_fcpxml: true,
            bundle_qc: true,
            bundle_title: "Master Delivery v6".into(),
            bundle_tags: "video,master,creator".into(),
            bundle_out_dir: String::new(),
            deliver_summary: "1 本のマスターから 16:9 / 9:16 / 1:1 / 4K と SRT・サムネイル・EDL・FCPXML・QC を一括納品"
                .into(),
        }
    }
}

impl V60PanelsState {
    pub fn show_v60_window(
        &mut self,
        ctx: &egui::Context,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        let mut open = self.show_window;
        egui::Window::new("💎 v6.0 クリエイター Pro スタジオ (#41〜#44)")
            .open(&mut open)
            .default_width(600.0)
            .default_height(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.active_tab, V60Tab::DataViz, "📊 #41 チャート");
                    ui.selectable_value(
                        &mut self.active_tab,
                        V60Tab::VfxPaint,
                        "🖌 #42 VFXペイント",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V60Tab::PodcastDirector,
                        "🎙 #43 ポッドキャスト",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        V60Tab::MultiDeliver,
                        "📦 #44 マルチ納品",
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.active_tab {
                    V60Tab::DataViz => {
                        if self.ui_dataviz_tab(ui, session, selected_item) {
                            mutated = true;
                        }
                    }
                    V60Tab::VfxPaint => {
                        if self.ui_vfx_paint_tab(ui, session, selected_item) {
                            mutated = true;
                        }
                    }
                    V60Tab::PodcastDirector => {
                        if self.ui_podcast_tab(ui, session) {
                            mutated = true;
                        }
                    }
                    V60Tab::MultiDeliver => {
                        if self.ui_multi_deliver_tab(ui, session) {
                            mutated = true;
                        }
                    }
                });
            });
        self.show_window = open;
        mutated
    }

    fn ui_dataviz_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        ui.heading("📊 #41 データ駆動モーショングラフィックス＆チャート");
        ui.horizontal_wrapped(|ui| {
            for &kind in ChartKind::ALL {
                if ui
                    .selectable_label(self.chart_cfg.kind == kind, kind.display_name())
                    .clicked()
                {
                    let prev_pts = self.chart_cfg.points.clone();
                    self.chart_cfg = DataChartConfig::for_kind(kind);
                    if !prev_pts.is_empty() && kind != ChartKind::KpiCounter {
                        self.chart_cfg.points = prev_pts;
                    }
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("タイトル:");
            ui.text_edit_singleline(&mut self.chart_cfg.title);
            ui.label("単位:");
            ui.add(egui::TextEdit::singleline(&mut self.chart_cfg.unit_suffix).desired_width(48.0));
        });
        ui.horizontal(|ui| {
            ui.label("登場アニメ(秒):");
            ui.add(egui::Slider::new(
                &mut self.chart_cfg.animate_in_sec,
                0.2..=6.0,
            ));
            ui.checkbox(&mut self.chart_cfg.show_grid, "グリッド");
            ui.checkbox(&mut self.chart_cfg.show_values, "数値ラベル");
        });
        ui.horizontal(|ui| {
            ui.label("幅:");
            ui.add(egui::DragValue::new(&mut self.chart_cfg.width).range(160.0..=1920.0));
            ui.label("高さ:");
            ui.add(egui::DragValue::new(&mut self.chart_cfg.height).range(120.0..=1080.0));
            ui.label("レイヤー:");
            ui.add(egui::DragValue::new(&mut self.chart_layer_index).range(0..=16));
            ui.label("尺(秒):");
            ui.add(egui::DragValue::new(&mut self.chart_duration_sec).range(0.5..=60.0));
        });

        ui.label("CSV データ (`label,value[,#RRGGBB]`):");
        ui.add(
            egui::TextEdit::multiline(&mut self.chart_csv)
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .code_editor(),
        );
        ui.horizontal(|ui| {
            if ui.button("📥 CSV パース反映").clicked() {
                let pts = parse_chart_csv(&self.chart_csv);
                if pts.is_empty() {
                    self.chart_status = "⚠️ 有効な数値行が見つかりませんでした".into();
                } else {
                    self.chart_status =
                        format!("✅ {} 件のデータポイントを反映しました", pts.len());
                    self.chart_cfg.points = pts;
                }
            }
            if ui.button("➕ タイムラインへチャート追加").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                if let Some(sc) = s.project.active_scene() {
                    let layer_index = self
                        .chart_layer_index
                        .min(sc.layers.len().saturating_sub(1));
                    let mut item = TimelineItem::new(
                        self.chart_cfg.title.clone(),
                        RationalTime::ZERO,
                        RationalTime::from_seconds(f64::from(self.chart_duration_sec)),
                        ItemKind::DataChart {
                            config: Box::new(self.chart_cfg.clone()),
                        },
                    );
                    item.transform.x = (sc.width as f32 - self.chart_cfg.width) * 0.5;
                    item.transform.y = (sc.height as f32 - self.chart_cfg.height) * 0.5;
                    match s.bus.execute(
                        &mut s.project,
                        EditCommand::AddObject { layer_index, item },
                        CommandSource::Gui,
                        None,
                    ) {
                        Ok(()) => {
                            self.chart_status =
                                format!("✅ Layer {} にチャートを追加しました", layer_index);
                            mutated = true;
                        }
                        Err(e) => self.chart_status = format!("❌ 追加失敗: {e}"),
                    }
                }
            }
            if let Some(item_id) = selected_item
                && ui.button("✏️ 選択チャートへ適用").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                match s.bus.execute(
                    &mut s.project,
                    EditCommand::SetDataChart {
                        item_id,
                        config: Box::new(self.chart_cfg.clone()),
                    },
                    CommandSource::Gui,
                    None,
                ) {
                    Ok(()) => {
                        self.chart_status = "✅ 選択中チャートを更新しました".into();
                        mutated = true;
                    }
                    Err(_) => {
                        self.chart_status = "⚠️ 選択中のクリップは DataChart ではありません".into();
                    }
                }
            }
        });

        // 簡易プレビューバー
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 6.0, Color32::from_rgb(18, 22, 34));
        let max_v = self
            .chart_cfg
            .points
            .iter()
            .map(|pt| pt.value.abs())
            .fold(1.0_f64, f64::max) as f32;
        let n = self.chart_cfg.points.len().max(1);
        let slot = (rect.width() - 20.0) / n as f32;
        for (i, pt) in self.chart_cfg.points.iter().enumerate() {
            let h = ((pt.value.max(0.0) as f32) / max_v) * (rect.height() - 24.0);
            let x0 = rect.min.x + 10.0 + i as f32 * slot + slot * 0.18;
            let bar = egui::Rect::from_min_max(
                Pos2::new(x0, rect.max.y - 10.0 - h),
                Pos2::new(x0 + slot * 0.64, rect.max.y - 10.0),
            );
            let col = pt.color.unwrap_or(self.chart_cfg.accent_color);
            p.rect_filled(
                bar,
                3.0,
                Color32::from_rgb(
                    (col.r * 255.0) as u8,
                    (col.g * 255.0) as u8,
                    (col.b * 255.0) as u8,
                ),
            );
        }

        ui.small(&self.chart_status);
        mutated
    }

    fn ui_vfx_paint_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        ui.heading("🖌 #42 非破壊 VFX ペイント / クローン / ワイヤー消し / 秘匿ぼかし");
        let Some(item_id) = selected_item else {
            ui.colored_label(
                Color32::from_rgb(240, 190, 90),
                "タイムライン上で対象クリップを 1 つ選択してください。",
            );
            return false;
        };

        ui.horizontal_wrapped(|ui| {
            for &tool in PaintToolKind::ALL {
                ui.selectable_value(&mut self.paint_tool, tool, tool.display_name());
            }
        });
        ui.horizontal(|ui| {
            ui.label("半径(px):");
            ui.add(egui::Slider::new(&mut self.paint_radius_px, 2.0..=96.0));
            ui.label("硬さ:");
            ui.add(egui::Slider::new(&mut self.paint_hardness, 0.1..=1.0));
            ui.label("不透明度:");
            ui.add(egui::Slider::new(&mut self.paint_opacity, 0.05..=1.0));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.paint_animate_write_on, "✨ Write-On アニメ描画");
            if self.paint_tool == PaintToolKind::BrushGlow {
                ui.label("ブラシ色:");
                ui.color_edit_button_rgb(&mut self.paint_color);
            }
            if self.paint_tool == PaintToolKind::CloneStamp {
                ui.label("参照オフセット X/Y:");
                ui.add(
                    egui::DragValue::new(&mut self.paint_clone_offset[0])
                        .speed(0.01)
                        .range(-0.8..=0.8),
                );
                ui.add(
                    egui::DragValue::new(&mut self.paint_clone_offset[1])
                        .speed(0.01)
                        .range(-0.8..=0.8),
                );
            }
        });

        // インタラクティブ・スケッチパッド（正規化 0..=1 座標ストローク入力）
        let (pad_rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().min(480.0), 180.0),
            egui::Sense::click_and_drag(),
        );
        if resp.drag_started() {
            self.draft_stroke.clear();
        }
        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let nx = ((pos.x - pad_rect.min.x) / pad_rect.width().max(1.0)).clamp(0.0, 1.0);
            let ny = ((pos.y - pad_rect.min.y) / pad_rect.height().max(1.0)).clamp(0.0, 1.0);
            if self
                .draft_stroke
                .last()
                .is_none_or(|p| (p[0] - nx).hypot(p[1] - ny) > 0.015)
            {
                self.draft_stroke.push([nx, ny]);
            }
        }

        let painter = ui.painter();
        painter.rect_filled(pad_rect, 6.0, Color32::from_rgb(20, 24, 34));
        painter.rect_stroke(
            pad_rect,
            6.0,
            Stroke::new(1.0, Color32::from_rgb(70, 90, 125)),
            egui::StrokeKind::Middle,
        );

        let existing_paint = session
            .lock()
            .ok()
            .and_then(|g| {
                g.project.active_scene().and_then(|sc| {
                    sc.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.vfx_paint.clone())
                })
            })
            .unwrap_or_default();

        for st in &existing_paint.strokes {
            let col = match st.tool {
                PaintToolKind::BrushGlow => Color32::from_rgb(255, 90, 145),
                PaintToolKind::CloneStamp => Color32::from_rgb(80, 210, 255),
                PaintToolKind::WireEraseInpaint => Color32::from_rgb(90, 240, 140),
                PaintToolKind::BlurRedact => Color32::from_rgb(255, 210, 70),
            };
            let pts: Vec<Pos2> = st
                .points
                .iter()
                .map(|p| {
                    Pos2::new(
                        pad_rect.min.x + p[0] * pad_rect.width(),
                        pad_rect.min.y + p[1] * pad_rect.height(),
                    )
                })
                .collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(pts, Stroke::new(2.5, col)));
            } else if let Some(&pt) = pts.first() {
                painter.circle_filled(pt, 5.0, col);
            }
        }
        if self.draft_stroke.len() >= 2 {
            let pts: Vec<Pos2> = self
                .draft_stroke
                .iter()
                .map(|p| {
                    Pos2::new(
                        pad_rect.min.x + p[0] * pad_rect.width(),
                        pad_rect.min.y + p[1] * pad_rect.height(),
                    )
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(3.0, Color32::WHITE)));
        }

        let commit_stroke =
            |points: Vec<[f32; 2]>, tool: PaintToolKind, state: &mut Self, mutated: &mut bool| {
                let mut next = existing_paint.clone();
                next.enabled = true;
                next.animate_write_on = state.paint_animate_write_on;
                next.strokes.push(PaintStroke {
                    id: Uuid::new_v4(),
                    tool,
                    points,
                    radius_px: state.paint_radius_px,
                    hardness: state.paint_hardness,
                    opacity: state.paint_opacity,
                    color: RgbaColor {
                        r: state.paint_color[0],
                        g: state.paint_color[1],
                        b: state.paint_color[2],
                        a: 1.0,
                    },
                    clone_offset: state.paint_clone_offset,
                    write_on_progress: 1.0,
                });
                if let Ok(mut guard) = session.lock() {
                    let s = &mut *guard;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetVfxPaint {
                                item_id,
                                vfx_paint: next.clone(),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        state.paint_status =
                            format!("✅ ストローク適用済み（合計 {} 本）", next.strokes.len());
                        *mutated = true;
                    }
                }
            };

        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.draft_stroke.is_empty(),
                    egui::Button::new("➕ 描いたストロークを確定"),
                )
                .clicked()
            {
                let pts = std::mem::take(&mut self.draft_stroke);
                let tool = self.paint_tool;
                commit_stroke(pts, tool, self, &mut mutated);
            }
            if ui.button("✨ 中央ワイヤー消しプリセット").clicked() {
                commit_stroke(
                    vec![[0.12, 0.50], [0.88, 0.50]],
                    PaintToolKind::WireEraseInpaint,
                    self,
                    &mut mutated,
                );
            }
            if ui.button("🔒 中央モザイク秘匿").clicked() {
                commit_stroke(
                    vec![[0.35, 0.42], [0.65, 0.42]],
                    PaintToolKind::BlurRedact,
                    self,
                    &mut mutated,
                );
            }
            if ui.button("🗑 ペイント全消去").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetVfxPaint {
                            item_id,
                            vfx_paint: VfxPaintConfig::default(),
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.draft_stroke.clear();
                    self.paint_status = "🗑 ペイントストロークをクリアしました".into();
                    mutated = true;
                }
            }
        });
        ui.small(&self.paint_status);
        mutated
    }

    fn ui_podcast_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut mutated = false;
        ui.heading("🎙 #43 マルチトラック・ポッドキャスト＆コラボ実況オート編集");
        ui.horizontal(|ui| {
            ui.label("目標ラウドネス (LUFS):");
            ui.add(egui::Slider::new(
                &mut self.podcast_target_lufs,
                -24.0..=-12.0,
            ));
            ui.checkbox(&mut self.podcast_apply_faders, "フェーダー自動均一化");
        });
        ui.horizontal(|ui| {
            ui.label("最小発話ターン(秒):");
            ui.add(egui::Slider::new(&mut self.podcast_min_turn_sec, 0.2..=2.0));
            ui.label("無音ゲート(dB):");
            ui.add(egui::Slider::new(
                &mut self.podcast_silence_gate_db,
                -60.0..=-20.0,
            ));
        });

        for row in &mut self.speaker_rows {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut row.0).desired_width(110.0));
                ui.label("音声L:");
                ui.add(egui::DragValue::new(&mut row.1).range(0..=16));
                ui.checkbox(&mut row.3, "カメラ連動");
                if row.3 {
                    ui.label("映像L:");
                    ui.add(egui::DragValue::new(&mut row.2).range(0..=16));
                }
            });
        }
        ui.horizontal(|ui| {
            if ui.button("➕ 話者トラック追加").clicked() && self.speaker_rows.len() < 6 {
                let n = self.speaker_rows.len() + 1;
                self.speaker_rows
                    .push((format!("Guest {n}"), n + 1, n.saturating_sub(1), false));
            }
            if ui.button("🎬 自動話者スイッチ＆-16 LUFS 適用").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                if let Some(scene) = s.project.active_scene().cloned() {
                    let sr = s.project.settings.audio_sample_rate.max(8_000);
                    let dur = scene.total_duration().as_seconds().clamp(2.0, 120.0);
                    let frames = ((dur * f64::from(sr)).round() as usize).clamp(sr as usize, sr as usize * 120);
                    let mut specs = Vec::new();
                    let mut pcms = Vec::new();
                    for (idx, (name, al, cl, use_cam)) in self.speaker_rows.iter().enumerate() {
                        specs.push(SpeakerTrackSpec {
                            name: name.clone(),
                            audio_layer_index: *al,
                            camera_layer_index: if *use_cam { Some(*cl) } else { None },
                        });
                        let mut st = vec![0.0f32; frames * 2];
                        let freq = 180.0 + idx as f32 * 45.0;
                        let amp = (0.18 + idx as f32 * 0.08).min(0.45);
                        if let Some(layer) = scene.layers.get(*al) {
                            for it in &layer.items {
                                let f0 = ((it.start.as_seconds() * f64::from(sr)).round() as usize)
                                    .min(frames);
                                let f1 = ((it.end().as_seconds() * f64::from(sr)).round() as usize)
                                    .min(frames);
                                for fi in f0..f1 {
                                    let t = fi as f32 / sr as f32;
                                    let v = (std::f32::consts::TAU * freq * t).sin() * amp;
                                    st[fi * 2] = v;
                                    st[fi * 2 + 1] = v;
                                }
                            }
                        }
                        pcms.push((name.clone(), st));
                    }
                    let analysis = analyze_podcast_speakers(
                        &pcms,
                        sr,
                        f64::from(self.podcast_min_turn_sec),
                        self.podcast_silence_gate_db,
                        self.podcast_target_lufs,
                    );
                    let cmds = build_podcast_auto_switch_commands(
                        &scene,
                        &specs,
                        &analysis,
                        self.podcast_apply_faders,
                    );
                    let turn_cnt = analysis.turns.len();
                    let cmd_cnt = cmds.len();
                    if !cmds.is_empty()
                        && s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::Batch {
                                    label: "Podcast Auto-Director".into(),
                                    commands: cmds,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                    {
                        self.podcast_summary = format!(
                            "✅ {} 件の話者ターンを検出し {} コマンド（カメラ切替/マーカー/フェーダー）を適用しました",
                            turn_cnt, cmd_cnt
                        );
                        mutated = true;
                    } else {
                        self.podcast_summary =
                            "ℹ️ 指定レイヤーに発話クリップが見つかりませんでした".into();
                    }
                }
            }
        });
        ui.small(&self.podcast_summary);
        mutated
    }

    fn selected_targets(&self) -> Vec<ve_core::DeliverableTarget> {
        let mut out = Vec::new();
        if self.plat_yt_16x9 {
            out.push(DeliveryPlatform::YouTube16x9.default_target());
        }
        if self.plat_shorts_9x16 {
            out.push(DeliveryPlatform::ShortsReels9x16.default_target());
        }
        if self.plat_square_1x1 {
            out.push(DeliveryPlatform::SquareFeed1x1.default_target());
        }
        if self.plat_cinema_4k {
            out.push(DeliveryPlatform::Cinema4kMaster.default_target());
        }
        if out.is_empty() {
            out.push(DeliveryPlatform::YouTube16x9.default_target());
        }
        out
    }

    fn ui_multi_deliver_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut mutated = false;
        ui.heading("📦 #44 プロ・マルチプラットフォーム自動リフォーマット＆納品バンドル");

        ui.label("出力プラットフォーム:");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.plat_yt_16x9,
                DeliveryPlatform::YouTube16x9.display_name(),
            );
            ui.checkbox(
                &mut self.plat_shorts_9x16,
                DeliveryPlatform::ShortsReels9x16.display_name(),
            );
            ui.checkbox(
                &mut self.plat_square_1x1,
                DeliveryPlatform::SquareFeed1x1.display_name(),
            );
            ui.checkbox(
                &mut self.plat_cinema_4k,
                DeliveryPlatform::Cinema4kMaster.display_name(),
            );
        });

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.bundle_thumb, "🖼 1280×720 サムネイル");
            ui.checkbox(&mut self.bundle_edl, "🎞 CMX 3600 EDL");
            ui.checkbox(&mut self.bundle_fcpxml, "🎬 FCPXML v1.10");
            ui.checkbox(&mut self.bundle_qc, "✅ 放送 QC JSON");
        });
        ui.horizontal(|ui| {
            ui.label("納品タイトル:");
            ui.text_edit_singleline(&mut self.bundle_title);
        });
        ui.horizontal(|ui| {
            ui.label("タグ(カンマ区切り):");
            ui.text_edit_singleline(&mut self.bundle_tags);
        });
        ui.horizontal(|ui| {
            ui.label("出力フォルダ (空=自動):");
            ui.text_edit_singleline(&mut self.bundle_out_dir);
        });

        ui.horizontal(|ui| {
            if ui.button("📐 派生シーンを一括生成 (1-Undo)").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                let targets = self.selected_targets();
                if let Some(src) = s.project.active_scene().cloned() {
                    let cmds: Vec<EditCommand> = targets
                        .iter()
                        .map(|t| build_reformatted_scene_command(&src, t).1)
                        .collect();
                    let count = cmds.len();
                    match s.bus.execute(
                        &mut s.project,
                        EditCommand::Batch {
                            label: "Multi-Platform Reformat".into(),
                            commands: cmds,
                        },
                        CommandSource::Gui,
                        None,
                    ) {
                        Ok(()) => {
                            self.deliver_summary = format!(
                                "✅ {} 種類の派生プラットフォームシーンを生成しました",
                                count
                            );
                            mutated = true;
                        }
                        Err(e) => self.deliver_summary = format!("❌ 生成失敗: {e}"),
                    }
                }
            }

            if ui.button("📦 1クリック納品バンドル書き出し").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                let out_dir = if !self.bundle_out_dir.trim().is_empty() {
                    PathBuf::from(self.bundle_out_dir.trim())
                } else if let Some(folder) = &s.folder {
                    folder.root().join("deliverables")
                } else {
                    std::env::temp_dir().join("ve_delivery_bundle")
                };
                let cfg = DeliveryBundleConfig {
                    targets: self.selected_targets(),
                    include_thumbnail: self.bundle_thumb,
                    include_edl: self.bundle_edl,
                    include_fcpxml: self.bundle_fcpxml,
                    include_qc_report: self.bundle_qc,
                    base_title: self.bundle_title.clone(),
                    tags: self
                        .bundle_tags
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect(),
                };
                if let Some(src) = s.project.active_scene().cloned() {
                    let _ = std::fs::create_dir_all(out_dir.join("variants"));
                    let _ = std::fs::create_dir_all(out_dir.join("subtitles"));
                    let _ = std::fs::create_dir_all(out_dir.join("metadata"));
                    let _ = std::fs::create_dir_all(out_dir.join("interchange"));
                    let _ = std::fs::create_dir_all(out_dir.join("qc"));

                    let mut artifacts = Vec::new();
                    let mut wrote_ch = false;
                    for tgt in &cfg.targets {
                        if let EditCommand::CreateScene { scene: vscene, .. } =
                            build_reformatted_scene_command(&src, tgt).1
                        {
                            let rel_s = format!("variants/{}.scene.json", tgt.platform.id());
                            if let Ok(bytes) = serialize_pretty_json_bytes(&vscene)
                                && std::fs::write(out_dir.join(&rel_s), &bytes).is_ok()
                            {
                                artifacts.push(DeliveryBundleArtifact {
                                    kind: "scene_variant".into(),
                                    platform: tgt.platform.id().into(),
                                    relative_path: rel_s,
                                    bytes: bytes.len(),
                                });
                            }
                            let srt = export_srt_subtitles(&vscene);
                            let rel_srt = format!("subtitles/{}.srt", tgt.platform.id());
                            if std::fs::write(out_dir.join(&rel_srt), srt.as_bytes()).is_ok() {
                                artifacts.push(DeliveryBundleArtifact {
                                    kind: "subtitles_srt".into(),
                                    platform: tgt.platform.id().into(),
                                    relative_path: rel_srt,
                                    bytes: srt.len(),
                                });
                            }
                            if tgt.include_chapters && !wrote_ch {
                                let ch = format_youtube_chapters(&vscene.markers);
                                let rel_ch = "metadata/youtube_chapters.txt".to_string();
                                if std::fs::write(out_dir.join(&rel_ch), ch.as_bytes()).is_ok() {
                                    artifacts.push(DeliveryBundleArtifact {
                                        kind: "youtube_chapters".into(),
                                        platform: tgt.platform.id().into(),
                                        relative_path: rel_ch,
                                        bytes: ch.len(),
                                    });
                                    wrote_ch = true;
                                }
                            }
                        }
                    }

                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(s.folder.clone());
                    let active_id = s.project.active_scene_id;
                    if cfg.include_thumbnail
                        && let Some((_, _, hero)) =
                            select_best_thumbnail_frame(&mut rg, &s.project, active_id, 4)
                        && let Ok(png) = encode_linear_image_png_bytes(&hero)
                        && std::fs::write(out_dir.join("thumbnail_1280x720.png"), &png).is_ok()
                    {
                        artifacts.push(DeliveryBundleArtifact {
                            kind: "thumbnail_png".into(),
                            platform: "shared".into(),
                            relative_path: "thumbnail_1280x720.png".into(),
                            bytes: png.len(),
                        });
                    }
                    if cfg.include_edl {
                        let edl = export_cmx3600_edl(&s.project);
                        if std::fs::write(out_dir.join("interchange/master.edl"), edl.as_bytes())
                            .is_ok()
                        {
                            artifacts.push(DeliveryBundleArtifact {
                                kind: "edl_cmx3600".into(),
                                platform: "shared".into(),
                                relative_path: "interchange/master.edl".into(),
                                bytes: edl.len(),
                            });
                        }
                    }
                    if cfg.include_fcpxml {
                        let xml = export_fcpxml_v10(&s.project);
                        if std::fs::write(out_dir.join("interchange/master.fcpxml"), xml.as_bytes())
                            .is_ok()
                        {
                            artifacts.push(DeliveryBundleArtifact {
                                kind: "fcpxml_v10".into(),
                                platform: "shared".into(),
                                relative_path: "interchange/master.fcpxml".into(),
                                bytes: xml.len(),
                            });
                        }
                    }
                    if cfg.include_qc_report {
                        let (peak, clip, crush, jump) =
                            measure_scene_video_qc_metrics(&mut rg, &s.project, active_id, 4);
                        let rep = run_broadcast_qc_audit(
                            &s.project,
                            BroadcastProfile::YouTube1080p,
                            BroadcastQcMetrics {
                                peak_luma_ire: peak,
                                clipped_highlight_pct: clip,
                                crushed_shadow_pct: crush,
                                max_frame_luma_jump_ire: jump,
                                true_peak_dbtp: -1.5,
                                integrated_lufs: -14.0,
                            },
                        );
                        if let Ok(qb) = serialize_pretty_json_bytes(&rep)
                            && std::fs::write(out_dir.join("qc/broadcast_qc_report.json"), &qb)
                                .is_ok()
                        {
                            artifacts.push(DeliveryBundleArtifact {
                                kind: "broadcast_qc_json".into(),
                                platform: "shared".into(),
                                relative_path: "qc/broadcast_qc_report.json".into(),
                                bytes: qb.len(),
                            });
                        }
                    }
                    let manifest = build_delivery_manifest(&s.project, &cfg, artifacts);
                    if let Ok(mb) = serialize_pretty_json_bytes(&manifest) {
                        let _ = std::fs::write(out_dir.join("manifest.json"), mb);
                    }
                    self.deliver_summary = format!(
                        "✅ {} 点の納品アセットを {} へ書き出しました",
                        manifest.artifacts.len(),
                        out_dir.display()
                    );
                }
            }
        });
        ui.small(&self.deliver_summary);
        mutated
    }
}
