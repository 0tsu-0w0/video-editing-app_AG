use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use ve_core::{
    CommandSource, CubicBezier, EditCommand, Interpolation, Keyframe, RationalTime, evaluate_track,
    evaluate_velocity,
};
use ve_mcp::EditorSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphMode {
    Value,
    Speed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragHandle {
    P1(usize),
    P2(usize),
}

pub struct GraphEditorState {
    pub mode: GraphMode,
    pub property: String,
    active_handle: Option<DragHandle>,
}

impl Default for GraphEditorState {
    fn default() -> Self {
        Self {
            mode: GraphMode::Value,
            property: "x".to_string(),
            active_handle: None,
        }
    }
}

impl GraphEditorState {
    pub fn show(&mut self, ui: &mut Ui, session: &mut EditorSession) {
        let Some(&item_id) = session.selected_items.first() else {
            ui.weak("タイムラインでオブジェクトを選択するとグラフエディタを編集できます。");
            return;
        };

        let item_opt = session.project.active_scene().and_then(|s| {
            s.layers
                .iter()
                .flat_map(|l| l.items.iter())
                .find(|i| i.id == item_id)
                .cloned()
        });
        let Some(item) = item_opt else {
            ui.weak("選択中のオブジェクトがアクティブシーンにありません。");
            return;
        };

        // ヘッダーコントロール
        ui.horizontal(|ui| {
            ui.label("グラフ:");
            ui.selectable_value(&mut self.mode, GraphMode::Value, "値グラフ");
            ui.selectable_value(&mut self.mode, GraphMode::Speed, "速度グラフ");
            ui.separator();

            egui::ComboBox::from_id_salt("graph_prop_combo")
                .selected_text(&self.property)
                .show_ui(ui, |ui| {
                    for p in ["x", "y", "scale_x", "scale_y", "rotation_deg", "opacity"] {
                        ui.selectable_value(&mut self.property, p.to_string(), p);
                    }
                });

            let local_t = (session.playhead - item.start)
                .max(RationalTime::ZERO)
                .min(item.duration);
            if ui.button("◆ 現在位置にキー追加").clicked() {
                let base = base_property_value(&item.transform, &self.property);
                let cur_val = item
                    .keyframes
                    .get(&self.property)
                    .map(|kfs| evaluate_track(base, kfs, local_t))
                    .unwrap_or(base);
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::SetKeyframe {
                        item_id,
                        property: self.property.clone(),
                        keyframe: Keyframe {
                            time: local_t,
                            value: cur_val,
                            interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                        },
                    },
                    CommandSource::Gui,
                    None,
                );
            }
            if ui.button("サンプルEase設定").clicked() {
                let base = base_property_value(&item.transform, &self.property);
                let target = if self.property == "opacity" {
                    0.0
                } else {
                    base + 180.0
                };
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::Batch {
                        label: "seed_bezier_keyframes".into(),
                        commands: vec![
                            EditCommand::SetKeyframe {
                                item_id,
                                property: self.property.clone(),
                                keyframe: Keyframe {
                                    time: RationalTime::ZERO,
                                    value: base,
                                    interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                                },
                            },
                            EditCommand::SetKeyframe {
                                item_id,
                                property: self.property.clone(),
                                keyframe: Keyframe {
                                    time: item.duration,
                                    value: target,
                                    interpolation: Interpolation::Linear,
                                },
                            },
                        ],
                    },
                    CommandSource::Gui,
                    None,
                );
            }
        });

        let kfs = item
            .keyframes
            .get(&self.property)
            .cloned()
            .unwrap_or_default();
        let base_val = base_property_value(&item.transform, &self.property);
        let dur_sec = item.duration.as_seconds().max(0.1) as f32;

        // キャンバス描画領域
        let avail = Vec2::new(
            ui.available_width().max(260.0),
            ui.available_height().clamp(160.0, 260.0),
        );
        let (rect, response) = ui.allocate_exact_size(avail, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 23, 30));

        let pad = 24.0;
        let plot = Rect::from_min_max(
            Pos2::new(rect.left() + pad, rect.top() + pad),
            Pos2::new(rect.right() - pad, rect.bottom() - pad),
        );
        painter.rect_filled(plot, 2.0, Color32::from_rgb(27, 31, 40));

        // サンプル点からY軸レンジ決定
        let samples = 80usize;
        let mut pts_raw: Vec<(f32, f32)> = Vec::with_capacity(samples + 1);
        for i in 0..=samples {
            let t_sec = (i as f32 / samples as f32) * dur_sec;
            let rt = RationalTime::from_seconds(f64::from(t_sec));
            let y = match self.mode {
                GraphMode::Value => evaluate_track(base_val, &kfs, rt),
                GraphMode::Speed => evaluate_velocity(&kfs, rt),
            };
            pts_raw.push((t_sec, y));
        }

        let mut min_y = pts_raw
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::INFINITY, f32::min);
        let mut max_y = pts_raw
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::NEG_INFINITY, f32::max);
        if (max_y - min_y).abs() < 1e-3 {
            min_y -= 1.0;
            max_y += 1.0;
        } else {
            let margin = (max_y - min_y) * 0.15;
            min_y -= margin;
            max_y += margin;
        }

        let to_screen = |t_sec: f32, val: f32| -> Pos2 {
            let nx = (t_sec / dur_sec).clamp(0.0, 1.0);
            let ny = ((val - min_y) / (max_y - min_y)).clamp(0.0, 1.0);
            Pos2::new(
                plot.left() + nx * plot.width(),
                plot.bottom() - ny * plot.height(),
            )
        };

        // グリッド線
        for i in 0..=4 {
            let y = plot.top() + (i as f32 / 4.0) * plot.height();
            painter.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(1.0, Color32::from_rgb(42, 47, 58)),
            );
        }

        // カーブ描画
        let curve_color = match self.mode {
            GraphMode::Value => Color32::from_rgb(92, 198, 255),
            GraphMode::Speed => Color32::from_rgb(255, 168, 72),
        };
        let screen_pts: Vec<Pos2> = pts_raw.iter().map(|&(t, y)| to_screen(t, y)).collect();
        for win in screen_pts.windows(2) {
            painter.line_segment([win[0], win[1]], Stroke::new(2.0, curve_color));
        }

        // 再生ヘッド位置
        let play_local = (session.playhead - item.start).as_seconds() as f32;
        if play_local >= 0.0 && play_local <= dur_sec {
            let px = plot.left() + (play_local / dur_sec) * plot.width();
            painter.line_segment(
                [Pos2::new(px, plot.top()), Pos2::new(px, plot.bottom())],
                Stroke::new(1.5, Color32::from_rgb(235, 72, 72)),
            );
        }

        // キーフレーム点と 3次ベジェハンドル（ドラッグ操作対応）
        let mut updated_kf: Option<Keyframe> = None;
        for (idx, kf) in kfs.iter().enumerate() {
            let t0 = kf.time.as_seconds() as f32;
            let p0 = to_screen(
                t0,
                match self.mode {
                    GraphMode::Value => kf.value,
                    GraphMode::Speed => evaluate_velocity(&kfs, kf.time),
                },
            );
            painter.circle_filled(p0, 5.0, Color32::from_rgb(255, 224, 96));

            if self.mode == GraphMode::Value
                && let Some(next_kf) = kfs.get(idx + 1)
                && let Interpolation::Bezier(mut bez) = kf.interpolation
            {
                let t1 = next_kf.time.as_seconds() as f32;
                let dt = (t1 - t0).max(1e-4);
                let dv = next_kf.value - kf.value;
                let h1 = to_screen(t0 + bez.x1 * dt, kf.value + bez.y1 * dv);
                let h2 = to_screen(t0 + bez.x2 * dt, kf.value + bez.y2 * dv);
                let p3 = to_screen(t1, next_kf.value);

                painter.line_segment([p0, h1], Stroke::new(1.0, Color32::from_rgb(160, 220, 140)));
                painter.line_segment([p3, h2], Stroke::new(1.0, Color32::from_rgb(160, 220, 140)));
                painter.circle_filled(h1, 5.5, Color32::from_rgb(110, 235, 150));
                painter.circle_filled(h2, 5.5, Color32::from_rgb(110, 235, 150));

                if response.drag_started()
                    && let Some(ptr) = response.interact_pointer_pos()
                {
                    if ptr.distance(h1) <= 12.0 {
                        self.active_handle = Some(DragHandle::P1(idx));
                        session.bus.set_gui_drag_locked(true);
                    } else if ptr.distance(h2) <= 12.0 {
                        self.active_handle = Some(DragHandle::P2(idx));
                        session.bus.set_gui_drag_locked(true);
                    }
                }

                if response.dragged()
                    && let Some(ptr) = response.interact_pointer_pos()
                {
                    let ptr_t = ((ptr.x - plot.left()) / plot.width()) * dur_sec;
                    let ptr_v = min_y + ((plot.bottom() - ptr.y) / plot.height()) * (max_y - min_y);
                    let nx = ((ptr_t - t0) / dt).clamp(0.01, 0.99);
                    let ny = if dv.abs() > 1e-4 {
                        ((ptr_v - kf.value) / dv).clamp(-0.5, 1.5)
                    } else {
                        0.5
                    };
                    match self.active_handle {
                        Some(DragHandle::P1(i)) if i == idx => {
                            bez.x1 = nx;
                            bez.y1 = ny;
                            updated_kf = Some(Keyframe {
                                interpolation: Interpolation::Bezier(bez),
                                ..*kf
                            });
                        }
                        Some(DragHandle::P2(i)) if i == idx => {
                            bez.x2 = nx;
                            bez.y2 = ny;
                            updated_kf = Some(Keyframe {
                                interpolation: Interpolation::Bezier(bez),
                                ..*kf
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Some(kf) = updated_kf {
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetKeyframe {
                    item_id,
                    property: self.property.clone(),
                    keyframe: kf,
                },
                CommandSource::Gui,
                Some("graph_bezier_drag"),
            );
        }

        if response.drag_stopped() && self.active_handle.is_some() {
            self.active_handle = None;
            session.bus.set_gui_drag_locked(false);
        }

        painter.text(
            Pos2::new(plot.left() + 4.0, plot.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!("max: {max_y:.1}"),
            egui::FontId::monospace(10.0),
            Color32::from_gray(170),
        );
        painter.text(
            Pos2::new(plot.left() + 4.0, plot.bottom() - 14.0),
            egui::Align2::LEFT_TOP,
            format!("min: {min_y:.1}"),
            egui::FontId::monospace(10.0),
            Color32::from_gray(170),
        );
    }
}

fn base_property_value(tf: &ve_core::Transform2D, property: &str) -> f32 {
    match property {
        "x" => tf.x,
        "y" => tf.y,
        "scale_x" => tf.scale_x,
        "scale_y" => tf.scale_y,
        "rotation_deg" => tf.rotation_deg,
        "opacity" => tf.opacity,
        _ => 0.0,
    }
}
