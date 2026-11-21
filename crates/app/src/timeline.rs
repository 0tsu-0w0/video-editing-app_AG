use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::BTreeMap;
use uuid::Uuid;
use ve_audio::WaveformPeaks;
use ve_core::{
    CommandSource, EditCommand, FilterTargetScope, ItemId, ItemKind, RationalTime, RgbaColor,
    Transform2D,
};
use ve_mcp::EditorSession;

pub struct TimelineState {
    pub pixels_per_second: f32,
    pub snap_enabled: bool,
    pub dragging_item: Option<DragOp>,
}

#[derive(Debug, Clone, Copy)]
pub enum DragMode {
    Move { grab_offset_sec: f64 },
    TrimRight,
}

#[derive(Debug, Clone, Copy)]
pub struct DragOp {
    pub item_id: ItemId,
    pub mode: DragMode,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            pixels_per_second: 120.0,
            snap_enabled: true,
            dragging_item: None,
        }
    }
}

impl TimelineState {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        waveforms: &BTreeMap<Uuid, WaveformPeaks>,
    ) {
        let fps = session.project.settings.fps;

        // ショートカット: Ctrl+Z / Ctrl+Y / Delete / S (分割)
        let (undo_key, redo_key, split_key, delete_key) = ui.input(|i| {
            let undo = i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z);
            let redo = (i.modifiers.command && i.key_pressed(egui::Key::Y))
                || (i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z));
            let split = !i.modifiers.command && i.key_pressed(egui::Key::S);
            let del = i.key_pressed(egui::Key::Delete);
            (undo, redo, split, del)
        });

        // ツールバー
        ui.horizontal(|ui| {
            if ui
                .add_enabled(session.bus.can_undo(), egui::Button::new("↶ Undo (Ctrl+Z)"))
                .clicked()
                || undo_key
            {
                session.bus.undo(&mut session.project);
            }
            if ui
                .add_enabled(session.bus.can_redo(), egui::Button::new("↷ Redo (Ctrl+Y)"))
                .clicked()
                || redo_key
            {
                session.bus.redo(&mut session.project);
            }
            ui.separator();

            if ui.button("✂ 分割 (S)").clicked() || split_key {
                self.split_selected_at_playhead(session);
            }
            if ui
                .add_enabled(
                    !session.selected_items.is_empty(),
                    egui::Button::new("🗑 削除 (Del)"),
                )
                .clicked()
                || delete_key
            {
                self.delete_selected(session);
            }
            ui.separator();

            if ui.button("＋ 字幕追加").clicked() {
                let item = ve_core::TimelineItem {
                    id: Uuid::new_v4(),
                    name: "新規字幕".to_string(),
                    start: session.playhead,
                    duration: RationalTime::from_seconds(2.0),
                    transform: Transform2D {
                        x: 160.0,
                        y: 480.0,
                        ..Default::default()
                    },
                    effect_chain: vec![],
                    kind: ItemKind::Text {
                        content: "新しい字幕テキスト".to_string(),
                        font_family: "sans-serif".to_string(),
                        font_size: 44.0,
                        color: RgbaColor::WHITE,
                    },
                };
                let new_id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 1,
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    session.selected_items = vec![new_id];
                }
            }
            if ui.button("＋ フィルタ追加").clicked() {
                let mut params = BTreeMap::new();
                params.insert("gain".to_string(), 1.3);
                let item = ve_core::TimelineItem {
                    id: Uuid::new_v4(),
                    name: "明るさフィルタ".to_string(),
                    start: session.playhead,
                    duration: RationalTime::from_seconds(2.0),
                    transform: Transform2D::default(),
                    effect_chain: vec![],
                    kind: ItemKind::FilterObject {
                        effect_type: "brightness".to_string(),
                        scope: FilterTargetScope::ImmediateLayer,
                        params,
                    },
                };
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::AddObject {
                        layer_index: 2,
                        item,
                    },
                    CommandSource::Gui,
                    None,
                );
            }

            ui.separator();
            ui.checkbox(&mut self.snap_enabled, "スナップ");
            ui.add(
                egui::Slider::new(&mut self.pixels_per_second, 40.0..=300.0).text("ズーム(px/s)"),
            );
        });

        ui.separator();

        // タイムラインキャンバス描画
        let layer_count = session
            .project
            .active_scene()
            .map(|s| s.layers.len())
            .unwrap_or(0);
        let header_w = 110.0;
        let ruler_h = 24.0;
        let track_h = 36.0;
        let total_h = ruler_h + track_h * (layer_count.max(1) as f32) + 8.0;

        let (canvas_rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(400.0), total_h),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 2.0, Color32::from_rgb(24, 26, 32));

        let track_origin_x = canvas_rect.left() + header_w;
        let pps = self.pixels_per_second;

        // ルーラー描画＆クリックでシーク
        let ruler_rect = Rect::from_min_max(
            Pos2::new(track_origin_x, canvas_rect.top()),
            Pos2::new(canvas_rect.right(), canvas_rect.top() + ruler_h),
        );
        painter.rect_filled(ruler_rect, 0.0, Color32::from_rgb(36, 40, 48));
        let max_sec = ((ruler_rect.width() / pps).ceil() as i32).max(1);
        for sec in 0..=max_sec {
            let x = track_origin_x + sec as f32 * pps;
            painter.line_segment(
                [
                    Pos2::new(x, ruler_rect.top()),
                    Pos2::new(x, canvas_rect.bottom()),
                ],
                Stroke::new(1.0, Color32::from_rgb(45, 49, 58)),
            );
            painter.text(
                Pos2::new(x + 3.0, ruler_rect.top() + 4.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "{sec}s ({}f)",
                    RationalTime::from_seconds(sec as f64).to_frame_round(fps)
                ),
                egui::FontId::monospace(10.0),
                Color32::from_gray(185),
            );
        }

        // スナップ候補時刻の収集（再生ヘッド＋全クリップ端）
        let mut snap_targets: Vec<f64> = vec![session.playhead.as_seconds()];
        if let Some(scene) = session.project.active_scene() {
            for layer in &scene.layers {
                for item in &layer.items {
                    if self.dragging_item.map(|d| d.item_id) != Some(item.id) {
                        snap_targets.push(item.start.as_seconds());
                        snap_targets.push(item.end().as_seconds());
                    }
                }
            }
        }

        // レイヤー行＆クリップ描画
        let mut clicked_item: Option<(ItemId, DragMode)> = None;
        if let Some(scene) = session.project.active_scene() {
            for (layer_idx, layer) in scene.layers.iter().enumerate() {
                let y0 = canvas_rect.top() + ruler_h + layer_idx as f32 * track_h;
                let row_rect = Rect::from_min_max(
                    Pos2::new(canvas_rect.left(), y0),
                    Pos2::new(canvas_rect.right(), y0 + track_h - 2.0),
                );
                let header_rect = Rect::from_min_max(
                    Pos2::new(canvas_rect.left(), y0),
                    Pos2::new(track_origin_x - 2.0, y0 + track_h - 2.0),
                );
                painter.rect_filled(header_rect, 2.0, Color32::from_rgb(38, 42, 52));
                painter.text(
                    Pos2::new(header_rect.left() + 8.0, header_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("L{layer_idx} {}", layer.name),
                    egui::FontId::proportional(12.0),
                    if layer.visible {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(120)
                    },
                );

                painter.line_segment(
                    [
                        Pos2::new(track_origin_x, row_rect.bottom()),
                        Pos2::new(canvas_rect.right(), row_rect.bottom()),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(42, 46, 56)),
                );

                for item in &layer.items {
                    let x0 = track_origin_x + item.start.as_seconds() as f32 * pps;
                    let x1 = (track_origin_x + item.end().as_seconds() as f32 * pps).max(x0 + 8.0);
                    let clip_rect = Rect::from_min_max(
                        Pos2::new(x0, y0 + 2.0),
                        Pos2::new(x1, y0 + track_h - 5.0),
                    );
                    let selected = session.selected_items.contains(&item.id);
                    let fill = match &item.kind {
                        ItemKind::Text { .. } => Color32::from_rgb(72, 130, 196),
                        ItemKind::FilterObject { .. } => Color32::from_rgb(165, 102, 185),
                        ItemKind::Audio { .. } => Color32::from_rgb(64, 152, 108),
                        _ => Color32::from_rgb(88, 104, 138),
                    };
                    painter.rect_filled(clip_rect, 4.0, fill);

                    // 音声クリップの場合は波形ピークを描画
                    if let ItemKind::Audio { asset_id, .. } = &item.kind
                        && let Some(wf) = waveforms.get(asset_id)
                    {
                        draw_waveform(&painter, clip_rect, wf);
                    }

                    painter.rect_stroke(
                        clip_rect,
                        4.0,
                        Stroke::new(
                            if selected { 2.0 } else { 1.0 },
                            if selected {
                                Color32::from_rgb(255, 214, 72)
                            } else {
                                Color32::from_rgb(200, 210, 225)
                            },
                        ),
                        StrokeKind::Inside,
                    );
                    painter.text(
                        Pos2::new(clip_rect.left() + 6.0, clip_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &item.name,
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );

                    // クリック／ドラッグ開始の当たり判定
                    if (response.drag_started() || response.clicked())
                        && let Some(pos) = response.interact_pointer_pos()
                        && clip_rect.contains(pos)
                    {
                        let is_right_edge = (clip_rect.right() - pos.x).abs() <= 8.0;
                        let mode = if is_right_edge {
                            DragMode::TrimRight
                        } else {
                            let ptr_sec = ((pos.x - track_origin_x) / pps) as f64;
                            DragMode::Move {
                                grab_offset_sec: ptr_sec - item.start.as_seconds(),
                            }
                        };
                        clicked_item = Some((item.id, mode));
                    }
                }
            }
        }

        // ポインタ操作の処理（選択・ドラッグ移動・伸縮・ルーラーシーク）
        let shift_held = ui.input(|i| i.modifiers.shift);
        if let Some((item_id, mode)) = clicked_item {
            if shift_held {
                if !session.selected_items.contains(&item_id) {
                    session.selected_items.push(item_id);
                }
            } else {
                session.selected_items = vec![item_id];
            }
            if response.drag_started() {
                self.dragging_item = Some(DragOp { item_id, mode });
                session.bus.set_gui_drag_locked(true);
            }
        } else if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && pos.x >= track_origin_x
        {
            let sec = ((pos.x - track_origin_x) / pps).max(0.0) as f64;
            session.playhead = RationalTime::from_seconds(sec);
        }

        if let Some(drag) = self.dragging_item {
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let raw_sec = ((pos.x - track_origin_x) / pps).max(0.0) as f64;
                let target_layer = (((pos.y - (canvas_rect.top() + ruler_h)) / track_h)
                    .floor()
                    .max(0.0) as usize)
                    .min(layer_count.saturating_sub(1));

                match drag.mode {
                    DragMode::Move { grab_offset_sec } => {
                        let mut cand_sec = (raw_sec - grab_offset_sec).max(0.0);
                        if self.snap_enabled {
                            cand_sec = snap_value(cand_sec, &snap_targets, 8.0 / pps as f64);
                        }
                        let _ = session.bus.execute(
                            &mut session.project,
                            EditCommand::MoveObject {
                                item_id: drag.item_id,
                                target_layer_index: target_layer,
                                new_start: RationalTime::from_seconds(cand_sec),
                            },
                            CommandSource::Gui,
                            Some("timeline_move"),
                        );
                    }
                    DragMode::TrimRight => {
                        let item_start = session.project.active_scene().and_then(|s| {
                            s.layers
                                .iter()
                                .flat_map(|l| l.items.iter())
                                .find(|i| i.id == drag.item_id)
                                .map(|i| i.start)
                        });
                        if let Some(start) = item_start {
                            let mut end_sec = raw_sec.max(start.as_seconds() + 0.1);
                            if self.snap_enabled {
                                end_sec = snap_value(end_sec, &snap_targets, 8.0 / pps as f64);
                            }
                            let dur = (end_sec - start.as_seconds()).max(0.1);
                            let _ = session.bus.execute(
                                &mut session.project,
                                EditCommand::TrimObject {
                                    item_id: drag.item_id,
                                    new_start: start,
                                    new_duration: RationalTime::from_seconds(dur),
                                },
                                CommandSource::Gui,
                                Some("timeline_trim"),
                            );
                        }
                    }
                }
            }
            if response.drag_stopped() {
                self.dragging_item = None;
                session.bus.set_gui_drag_locked(false);
            }
        }

        // 再生ヘッド（赤い縦線）描画
        let playhead_x = track_origin_x + session.playhead.as_seconds() as f32 * pps;
        if playhead_x >= track_origin_x && playhead_x <= canvas_rect.right() {
            painter.line_segment(
                [
                    Pos2::new(playhead_x, canvas_rect.top()),
                    Pos2::new(playhead_x, canvas_rect.bottom()),
                ],
                Stroke::new(2.0, Color32::from_rgb(235, 72, 72)),
            );
        }
    }

    fn split_selected_at_playhead(&self, session: &mut EditorSession) {
        let split_at = session.playhead;
        let cmds: Vec<EditCommand> = session
            .selected_items
            .iter()
            .copied()
            .map(|id| EditCommand::SplitObject {
                item_id: id,
                split_at,
                new_right_id: Uuid::new_v4(),
            })
            .collect();
        if cmds.is_empty() {
            return;
        }
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::Batch {
                label: "split_at_playhead".into(),
                commands: cmds,
            },
            CommandSource::Gui,
            None,
        );
    }

    fn delete_selected(&self, session: &mut EditorSession) {
        let cmds: Vec<EditCommand> = session
            .selected_items
            .iter()
            .copied()
            .map(|item_id| EditCommand::DeleteObject { item_id })
            .collect();
        if cmds.is_empty() {
            return;
        }
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::Batch {
                    label: "delete_selected".into(),
                    commands: cmds,
                },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            session.selected_items.clear();
        }
    }
}

fn snap_value(value: f64, targets: &[f64], threshold: f64) -> f64 {
    let mut best = value;
    let mut best_dist = threshold;
    for &t in targets {
        let d = (value - t).abs();
        if d < best_dist {
            best_dist = d;
            best = t;
        }
    }
    best
}

fn draw_waveform(painter: &egui::Painter, rect: Rect, wf: &WaveformPeaks) {
    if wf.buckets.is_empty() {
        return;
    }
    let mid_y = rect.center().y;
    let half_h = rect.height() * 0.42;
    let steps = (rect.width() as usize).clamp(1, wf.buckets.len());
    for i in 0..steps {
        let b_idx = i * wf.buckets.len() / steps;
        let b = wf.buckets[b_idx];
        let x = rect.left() + (i as f32 / steps as f32) * rect.width();
        let y0 = mid_y - b.max * half_h;
        let y1 = mid_y - b.min * half_h;
        painter.line_segment(
            [Pos2::new(x, y0), Pos2::new(x, y1)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(220, 255, 230, 180)),
        );
    }
}
