use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use ve_core::{
    BezierMask, CommandSource, EditCommand, ItemId, ItemKind, MaskMode, RationalTime, Scene,
    TimelineItem, Transform2D,
};
use ve_mcp::EditorSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScaleCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone)]
enum GizmoDrag {
    Move {
        item_id: ItemId,
        start_scene_pos: Pos2,
        initial_tf: Transform2D,
        item_w: f32,
        item_h: f32,
        layer_offset: Vec2,
    },
    Scale {
        item_id: ItemId,
        corner: ScaleCorner,
        initial_tf: Transform2D,
        base_w: f32,
        base_h: f32,
        center_scene: Pos2,
        start_dist: f32,
        start_scene_pos: Pos2,
    },
    Rotate {
        item_id: ItemId,
        initial_tf: Transform2D,
        center_scene: Pos2,
        start_angle_deg: f32,
    },
    MaskVertex {
        item_id: ItemId,
        mask_idx: usize,
        vert_idx: usize,
        origin_x: f32,
        origin_y: f32,
        scaled_w: f32,
        scaled_h: f32,
        masks: Vec<BezierMask>,
    },
    MaskHandle {
        item_id: ItemId,
        mask_idx: usize,
        vert_idx: usize,
        is_out: bool,
        origin_x: f32,
        origin_y: f32,
        scaled_w: f32,
        scaled_h: f32,
        masks: Vec<BezierMask>,
    },
    Pan {
        start_pointer: Pos2,
        initial_pan: Vec2,
    },
}

/// プレビューキャンバス直接操作状態（Transform Gizmo・スナップガイド・セーフエリア・ズーム＆パン）
pub struct PreviewCanvasState {
    pub zoom: f32,
    pub pan: Vec2,
    pub show_safe_guides: bool,
    pub show_rule_of_thirds: bool,
    pub snap_to_guides: bool,
    drag: Option<GizmoDrag>,
    active_snap_x: Option<f32>,
    active_snap_y: Option<f32>,
}

impl Default for PreviewCanvasState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
            show_safe_guides: true,
            show_rule_of_thirds: false,
            snap_to_guides: true,
            drag: None,
            active_snap_x: None,
            active_snap_y: None,
        }
    }
}

impl PreviewCanvasState {
    /// プレビュー領域とインタラクティブ Transform Gizmo を描画し、変換が発生した場合は `true` を返す
    pub fn show(
        &mut self,
        ui: &mut Ui,
        tex_id: Option<egui::TextureId>,
        session: &mut EditorSession,
        ram_cached_now: bool,
    ) -> bool {
        let mut modified = false;

        let (scene_w, scene_h) = session
            .project
            .active_scene()
            .map(|s| (s.width.max(1) as f32, s.height.max(1) as f32))
            .unwrap_or((1280.0, 720.0));
        let is_vertical = scene_h > scene_w;

        // プレビュー操作ツールバー（ズーム・フィット・ガイド・スナップ）
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Fit").clicked() {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
            for (label, z) in [("50%", 0.5), ("100%", 1.0), ("150%", 1.5), ("200%", 2.0)] {
                if ui
                    .selectable_label((self.zoom - z).abs() < 0.05, label)
                    .clicked()
                {
                    self.zoom = z;
                }
            }
            ui.separator();
            ui.checkbox(&mut self.show_safe_guides, "📐 セーフ枠");
            ui.checkbox(&mut self.show_rule_of_thirds, "▦ 3分割");
            ui.checkbox(&mut self.snap_to_guides, "🧲 中央吸着");
            ui.separator();
            ui.weak(format!(
                "{}×{}{}",
                scene_w as u32,
                scene_h as u32,
                if is_vertical { " (9:16縦)" } else { "" }
            ));
            if ram_cached_now {
                ui.colored_label(Color32::from_rgb(96, 224, 130), "● RAMキャッシュ済");
            }
        });

        let avail = ui.available_size().max(Vec2::new(160.0, 120.0));
        let (viewport_rect, response) = ui.allocate_exact_size(avail, Sense::click_and_drag());
        let painter = ui.painter_at(viewport_rect);
        painter.rect_filled(viewport_rect, 2.0, Color32::from_rgb(16, 18, 22));

        // ホイールズーム＆中ボタン/Altパン
        if response.hovered() {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.2 {
                let factor = if scroll_y > 0.0 { 1.08 } else { 0.92 };
                self.zoom = (self.zoom * factor).clamp(0.25, 4.0);
            }
        }

        // シーンの実際のアスペクト比で映像描画矩形を計算
        let aspect = scene_w / scene_h;
        let fit_w = (viewport_rect.width() - 16.0)
            .min((viewport_rect.height() - 16.0) * aspect)
            .max(64.0);
        let fit_h = fit_w / aspect;
        let disp_w = fit_w * self.zoom;
        let disp_h = fit_h * self.zoom;
        let img_center = viewport_rect.center() + self.pan;
        let image_rect = Rect::from_center_size(img_center, Vec2::new(disp_w, disp_h));

        // プレビュー映像本体
        if let Some(tid) = tex_id {
            painter.image(
                tid,
                image_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            painter.rect_filled(image_rect, 2.0, Color32::from_rgb(28, 32, 40));
            painter.text(
                image_rect.center(),
                egui::Align2::CENTER_CENTER,
                "wgpu プレビュー初期化待機中",
                egui::FontId::proportional(13.0),
                Color32::LIGHT_GRAY,
            );
        }
        painter.rect_stroke(
            image_rect,
            0.0,
            Stroke::new(1.0, Color32::from_gray(78)),
            StrokeKind::Outside,
        );

        let scene_to_screen = |sx: f32, sy: f32| -> Pos2 {
            Pos2::new(
                image_rect.left() + (sx / scene_w) * image_rect.width(),
                image_rect.top() + (sy / scene_h) * image_rect.height(),
            )
        };
        let screen_to_scene = |p: Pos2| -> Pos2 {
            Pos2::new(
                ((p.x - image_rect.left()) / image_rect.width().max(1.0)) * scene_w,
                ((p.y - image_rect.top()) / image_rect.height().max(1.0)) * scene_h,
            )
        };

        // ガイド線オーバーレイ（アクション/タイトルセーフ・3分割・縦動画 Shorts/TikTok UI 警告枠）
        if self.show_safe_guides {
            draw_safe_guides(&painter, image_rect, is_vertical);
        }
        if self.show_rule_of_thirds {
            draw_rule_of_thirds(&painter, image_rect);
        }

        let playhead = session.playhead;
        let shift_held = ui.input(|i| i.modifiers.shift);
        let alt_held = ui.input(|i| i.modifiers.alt);
        let middle_down = ui.input(|i| i.pointer.middle_down());

        // 現在時刻に表示中の選択アイテムのギズモ幾何計算
        let selected_geo = session.selected_items.first().copied().and_then(|sel_id| {
            session
                .project
                .active_scene()
                .and_then(|scene| find_visible_item_geometry(scene, sel_id, playhead))
        });

        // ギズモ描画（バウンディングボックス・4隅スケールハンドル・回転ハンドル）＆ v0.7 ベジェマスク頂点
        let mut handle_rects: Option<GizmoScreenHandles> = None;
        let selected_masks: Vec<BezierMask> = selected_geo
            .as_ref()
            .and_then(|geo| {
                session.project.active_scene().and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == geo.item_id)
                        .map(|i| i.masks.clone())
                })
            })
            .unwrap_or_default();

        if let Some(ref geo) = selected_geo {
            let handles = compute_screen_handles(geo, &scene_to_screen);
            draw_gizmo(&painter, &handles, geo.tf.rotation_deg);
            if !selected_masks.is_empty() {
                draw_bezier_mask_overlay(&painter, geo, &selected_masks, &scene_to_screen);
            }
            handle_rects = Some(handles);
        }

        // スナップ発動中の吸着ガイド線を描画
        if let Some(sx) = self.active_snap_x {
            let p0 = scene_to_screen(sx, 0.0);
            let p1 = scene_to_screen(sx, scene_h);
            painter.line_segment([p0, p1], Stroke::new(1.5, Color32::from_rgb(0, 230, 255)));
        }
        if let Some(sy) = self.active_snap_y {
            let p0 = scene_to_screen(0.0, sy);
            let p1 = scene_to_screen(scene_w, sy);
            painter.line_segment([p0, p1], Stroke::new(1.5, Color32::from_rgb(0, 230, 255)));
        }

        // ポインタ操作開始（パン / マスク頂点・ハンドル / 回転 / スケール / 移動 / クリック選択）
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            if middle_down || alt_held {
                self.drag = Some(GizmoDrag::Pan {
                    start_pointer: pos,
                    initial_pan: self.pan,
                });
            } else if let (Some(geo), Some(handles)) = (&selected_geo, &handle_rects) {
                let s_pos = screen_to_scene(pos);
                let hit_r = 10.0;
                if let Some(mask_drag) =
                    hit_test_bezier_masks(geo, &selected_masks, pos, hit_r, &scene_to_screen)
                {
                    self.drag = Some(mask_drag);
                    session.bus.set_gui_drag_locked(true);
                } else if handles.rotate_knob.distance(pos) <= hit_r {
                    let start_ang = (s_pos.y - geo.center_y)
                        .atan2(s_pos.x - geo.center_x)
                        .to_degrees();
                    self.drag = Some(GizmoDrag::Rotate {
                        item_id: geo.item_id,
                        initial_tf: geo.tf,
                        center_scene: Pos2::new(geo.center_x, geo.center_y),
                        start_angle_deg: start_ang,
                    });
                    session.bus.set_gui_drag_locked(true);
                } else if let Some(corner) = handles.hit_corner(pos, hit_r) {
                    let center = Pos2::new(geo.center_x, geo.center_y);
                    self.drag = Some(GizmoDrag::Scale {
                        item_id: geo.item_id,
                        corner,
                        initial_tf: geo.tf,
                        base_w: geo.base_w,
                        base_h: geo.base_h,
                        center_scene: center,
                        start_dist: s_pos.distance(center).max(8.0),
                        start_scene_pos: s_pos,
                    });
                    session.bus.set_gui_drag_locked(true);
                } else if handles.contains_point(pos) {
                    self.drag = Some(GizmoDrag::Move {
                        item_id: geo.item_id,
                        start_scene_pos: s_pos,
                        initial_tf: geo.tf,
                        item_w: geo.scaled_w,
                        item_h: geo.scaled_h,
                        layer_offset: Vec2::new(geo.layer_offset_x, geo.layer_offset_y),
                    });
                    session.bus.set_gui_drag_locked(true);
                } else if let Some(hit_id) = session
                    .project
                    .active_scene()
                    .and_then(|s| hit_test_scene(s, playhead, s_pos))
                {
                    // 別オブジェクト上を直接ドラッグ開始 → 即選択＆移動開始
                    session.selected_items = vec![hit_id];
                    if let Some(new_geo) = session
                        .project
                        .active_scene()
                        .and_then(|s| find_visible_item_geometry(s, hit_id, playhead))
                    {
                        self.drag = Some(GizmoDrag::Move {
                            item_id: hit_id,
                            start_scene_pos: s_pos,
                            initial_tf: new_geo.tf,
                            item_w: new_geo.scaled_w,
                            item_h: new_geo.scaled_h,
                            layer_offset: Vec2::new(new_geo.layer_offset_x, new_geo.layer_offset_y),
                        });
                        session.bus.set_gui_drag_locked(true);
                    }
                }
            } else if let Some(hit_id) = session
                .project
                .active_scene()
                .and_then(|s| hit_test_scene(s, playhead, screen_to_scene(pos)))
            {
                session.selected_items = vec![hit_id];
                let s_pos = screen_to_scene(pos);
                if let Some(new_geo) = session
                    .project
                    .active_scene()
                    .and_then(|s| find_visible_item_geometry(s, hit_id, playhead))
                {
                    self.drag = Some(GizmoDrag::Move {
                        item_id: hit_id,
                        start_scene_pos: s_pos,
                        initial_tf: new_geo.tf,
                        item_w: new_geo.scaled_w,
                        item_h: new_geo.scaled_h,
                        layer_offset: Vec2::new(new_geo.layer_offset_x, new_geo.layer_offset_y),
                    });
                    session.bus.set_gui_drag_locked(true);
                }
            }
        } else if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && image_rect.contains(pos)
        {
            let s_pos = screen_to_scene(pos);
            if let Some(hit_id) = session
                .project
                .active_scene()
                .and_then(|s| hit_test_scene(s, playhead, s_pos))
            {
                session.selected_items = vec![hit_id];
            } else {
                session.selected_items.clear();
            }
        }

        // ドラッグ中のリアルタイム更新
        self.active_snap_x = None;
        self.active_snap_y = None;
        if let Some(drag) = self.drag.clone() {
            let was_edit = !matches!(drag, GizmoDrag::Pan { .. });
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                match drag {
                    GizmoDrag::Pan {
                        start_pointer,
                        initial_pan,
                    } => {
                        self.pan = initial_pan + (pos - start_pointer);
                    }
                    GizmoDrag::Move {
                        item_id,
                        start_scene_pos,
                        initial_tf,
                        item_w,
                        item_h,
                        layer_offset,
                    } => {
                        let cur_scene = screen_to_scene(pos);
                        let delta = cur_scene - start_scene_pos;
                        let mut new_x = initial_tf.x + delta.x;
                        let mut new_y = initial_tf.y + delta.y;

                        if self.snap_to_guides {
                            let snap_tol = (scene_w.max(scene_h) * 0.012).max(8.0);
                            let (sx, guide_x) = snap_axis_with_bounds(
                                new_x + layer_offset.x,
                                item_w,
                                scene_w,
                                snap_tol,
                            );
                            let (sy, guide_y) = snap_axis_with_bounds(
                                new_y + layer_offset.y,
                                item_h,
                                scene_h,
                                snap_tol,
                            );
                            new_x = sx - layer_offset.x;
                            new_y = sy - layer_offset.y;
                            self.active_snap_x = guide_x;
                            self.active_snap_y = guide_y;
                        }

                        let mut tf = initial_tf;
                        tf.x = new_x;
                        tf.y = new_y;
                        if session
                            .bus
                            .execute(
                                &mut session.project,
                                EditCommand::SetTransform {
                                    item_id,
                                    transform: tf,
                                },
                                CommandSource::Gui,
                                Some("preview_gizmo_move"),
                            )
                            .is_ok()
                        {
                            modified = true;
                        }
                    }
                    GizmoDrag::Scale {
                        item_id,
                        corner,
                        initial_tf,
                        base_w,
                        base_h,
                        center_scene,
                        start_dist,
                        start_scene_pos,
                    } => {
                        let cur_scene = screen_to_scene(pos);
                        let (new_sx, new_sy) = compute_scaled_factors(
                            initial_tf.scale_x,
                            initial_tf.scale_y,
                            corner,
                            center_scene,
                            start_scene_pos,
                            cur_scene,
                            start_dist,
                            shift_held,
                        );
                        // 中心位置がぶれないように左上基準座標を補正
                        let old_w = base_w * initial_tf.scale_x;
                        let old_h = base_h * initial_tf.scale_y;
                        let new_w = base_w * new_sx;
                        let new_h = base_h * new_sy;
                        let mut tf = initial_tf;
                        tf.scale_x = new_sx;
                        tf.scale_y = new_sy;
                        tf.x = initial_tf.x + (old_w - new_w) * 0.5;
                        tf.y = initial_tf.y + (old_h - new_h) * 0.5;
                        if session
                            .bus
                            .execute(
                                &mut session.project,
                                EditCommand::SetTransform {
                                    item_id,
                                    transform: tf,
                                },
                                CommandSource::Gui,
                                Some("preview_gizmo_scale"),
                            )
                            .is_ok()
                        {
                            modified = true;
                        }
                    }
                    GizmoDrag::Rotate {
                        item_id,
                        initial_tf,
                        center_scene,
                        start_angle_deg,
                    } => {
                        let cur_scene = screen_to_scene(pos);
                        let cur_ang = (cur_scene.y - center_scene.y)
                            .atan2(cur_scene.x - center_scene.x)
                            .to_degrees();
                        let mut deg = initial_tf.rotation_deg + (cur_ang - start_angle_deg);
                        if shift_held {
                            deg = (deg / 15.0).round() * 15.0;
                        }
                        let mut tf = initial_tf;
                        tf.rotation_deg = deg;
                        if session
                            .bus
                            .execute(
                                &mut session.project,
                                EditCommand::SetTransform {
                                    item_id,
                                    transform: tf,
                                },
                                CommandSource::Gui,
                                Some("preview_gizmo_rotate"),
                            )
                            .is_ok()
                        {
                            modified = true;
                        }
                    }
                    GizmoDrag::MaskVertex {
                        item_id,
                        mask_idx,
                        vert_idx,
                        origin_x,
                        origin_y,
                        scaled_w,
                        scaled_h,
                        mut masks,
                    } => {
                        let cur_scene = screen_to_scene(pos);
                        let nx = ((cur_scene.x - origin_x) / scaled_w.max(1.0)).clamp(-0.5, 1.5);
                        let ny = ((cur_scene.y - origin_y) / scaled_h.max(1.0)).clamp(-0.5, 1.5);
                        if let Some(m) = masks.get_mut(mask_idx)
                            && let Some(v) = m.vertices.get_mut(vert_idx)
                        {
                            v.x = nx;
                            v.y = ny;
                            if session
                                .bus
                                .execute(
                                    &mut session.project,
                                    EditCommand::SetMasks {
                                        item_id,
                                        masks: masks.clone(),
                                    },
                                    CommandSource::Gui,
                                    Some("preview_mask_drag"),
                                )
                                .is_ok()
                            {
                                modified = true;
                            }
                            self.drag = Some(GizmoDrag::MaskVertex {
                                item_id,
                                mask_idx,
                                vert_idx,
                                origin_x,
                                origin_y,
                                scaled_w,
                                scaled_h,
                                masks,
                            });
                        }
                    }
                    GizmoDrag::MaskHandle {
                        item_id,
                        mask_idx,
                        vert_idx,
                        is_out,
                        origin_x,
                        origin_y,
                        scaled_w,
                        scaled_h,
                        mut masks,
                    } => {
                        let cur_scene = screen_to_scene(pos);
                        let nx = (cur_scene.x - origin_x) / scaled_w.max(1.0);
                        let ny = (cur_scene.y - origin_y) / scaled_h.max(1.0);
                        if let Some(m) = masks.get_mut(mask_idx)
                            && let Some(v) = m.vertices.get_mut(vert_idx)
                        {
                            let dx = (nx - v.x).clamp(-1.0, 1.0);
                            let dy = (ny - v.y).clamp(-1.0, 1.0);
                            if is_out {
                                v.out_handle = [dx, dy];
                                if shift_held {
                                    v.in_handle = [-dx, -dy];
                                }
                            } else {
                                v.in_handle = [dx, dy];
                                if shift_held {
                                    v.out_handle = [-dx, -dy];
                                }
                            }
                            if session
                                .bus
                                .execute(
                                    &mut session.project,
                                    EditCommand::SetMasks {
                                        item_id,
                                        masks: masks.clone(),
                                    },
                                    CommandSource::Gui,
                                    Some("preview_mask_handle_drag"),
                                )
                                .is_ok()
                            {
                                modified = true;
                            }
                            self.drag = Some(GizmoDrag::MaskHandle {
                                item_id,
                                mask_idx,
                                vert_idx,
                                is_out,
                                origin_x,
                                origin_y,
                                scaled_w,
                                scaled_h,
                                masks,
                            });
                        }
                    }
                }
            }
            if response.drag_stopped() {
                self.drag = None;
                self.active_snap_x = None;
                self.active_snap_y = None;
                if was_edit {
                    session.bus.set_gui_drag_locked(false);
                }
            }
        }

        modified
    }
}

#[derive(Debug, Clone)]
struct ItemGeometry {
    item_id: ItemId,
    tf: Transform2D,
    layer_offset_x: f32,
    layer_offset_y: f32,
    base_w: f32,
    base_h: f32,
    scaled_w: f32,
    scaled_h: f32,
    center_x: f32,
    center_y: f32,
}

fn estimate_item_base_size(item: &TimelineItem, scene: &Scene) -> Option<(f32, f32)> {
    match &item.kind {
        ItemKind::Shape { width, height, .. }
        | ItemKind::CharacterSprite { width, height, .. }
        | ItemKind::AudioSpectrum { width, height, .. } => Some((*width, *height)),
        ItemKind::Text {
            content, font_size, ..
        } => {
            let lines: Vec<&str> = content.lines().collect();
            let line_count = lines.len().max(1) as f32;
            let max_chars = lines
                .iter()
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            let pad_x = item.text_style.box_padding[0] * 2.0;
            let pad_y = item.text_style.box_padding[1] * 2.0;
            let w = (max_chars * font_size * 0.85 + pad_x + 24.0).clamp(60.0, scene.width as f32);
            let h = (line_count * font_size * 1.35 + pad_y + 16.0).clamp(32.0, scene.height as f32);
            Some((w, h))
        }
        ItemKind::Image { .. } | ItemKind::Video { .. } | ItemKind::Precomp { .. } => {
            Some((scene.width as f32, scene.height as f32))
        }
        ItemKind::Audio { .. }
        | ItemKind::FilterObject { .. }
        | ItemKind::Transition { .. }
        | ItemKind::GroupControl { .. } => None,
    }
}

fn find_visible_item_geometry(
    scene: &Scene,
    item_id: ItemId,
    time: RationalTime,
) -> Option<ItemGeometry> {
    for layer in &scene.layers {
        if !layer.visible {
            continue;
        }
        for item in &layer.items {
            if item.id == item_id && item.contains(time) {
                let (base_w, base_h) = estimate_item_base_size(item, scene)?;
                let tf = item.eval_transform(time);
                let scaled_w = (base_w * tf.scale_x.max(0.05)).max(8.0);
                let scaled_h = (base_h * tf.scale_y.max(0.05)).max(8.0);
                let x = tf.x + layer.offset_x;
                let y = tf.y + layer.offset_y;
                return Some(ItemGeometry {
                    item_id,
                    tf,
                    layer_offset_x: layer.offset_x,
                    layer_offset_y: layer.offset_y,
                    base_w,
                    base_h,
                    scaled_w,
                    scaled_h,
                    center_x: x + scaled_w * 0.5,
                    center_y: y + scaled_h * 0.5,
                });
            }
        }
    }
    None
}

/// 最前面（高レイヤー番号）から順にクリック位置の可視アイテムをヒットテストする
fn hit_test_scene(scene: &Scene, time: RationalTime, scene_pos: Pos2) -> Option<ItemId> {
    for layer in scene.layers.iter().rev() {
        if !layer.visible {
            continue;
        }
        for item in layer.items.iter().rev() {
            if !item.contains(time) {
                continue;
            }
            let Some((base_w, base_h)) = estimate_item_base_size(item, scene) else {
                continue;
            };
            let tf = item.eval_transform(time);
            let sw = (base_w * tf.scale_x.max(0.05)).max(8.0);
            let sh = (base_h * tf.scale_y.max(0.05)).max(8.0);
            let cx = tf.x + layer.offset_x + sw * 0.5;
            let cy = tf.y + layer.offset_y + sh * 0.5;
            let rad = (-tf.rotation_deg).to_radians();
            let dx = scene_pos.x - cx;
            let dy = scene_pos.y - cy;
            let lx = dx * rad.cos() - dy * rad.sin();
            let ly = dx * rad.sin() + dy * rad.cos();
            if lx.abs() <= sw * 0.5 && ly.abs() <= sh * 0.5 {
                return Some(item.id);
            }
        }
    }
    None
}

struct GizmoScreenHandles {
    corners: [Pos2; 4], // TL, TR, BR, BL
    center: Pos2,
    top_mid: Pos2,
    rotate_knob: Pos2,
}

impl GizmoScreenHandles {
    fn hit_corner(&self, pos: Pos2, radius: f32) -> Option<ScaleCorner> {
        let map = [
            (self.corners[0], ScaleCorner::TopLeft),
            (self.corners[1], ScaleCorner::TopRight),
            (self.corners[2], ScaleCorner::BottomRight),
            (self.corners[3], ScaleCorner::BottomLeft),
        ];
        for (cpos, corner) in map {
            if cpos.distance(pos) <= radius {
                return Some(corner);
            }
        }
        None
    }

    fn contains_point(&self, pos: Pos2) -> bool {
        // 凸四角形（回転済みボックス）の内外判定
        let mut sign = 0.0_f32;
        for i in 0..4 {
            let a = self.corners[i];
            let b = self.corners[(i + 1) % 4];
            let cross = (b.x - a.x) * (pos.y - a.y) - (b.y - a.y) * (pos.x - a.x);
            if cross.abs() > 1e-4 {
                if sign == 0.0 {
                    sign = cross.signum();
                } else if sign != cross.signum() {
                    return false;
                }
            }
        }
        true
    }
}

fn compute_screen_handles(
    geo: &ItemGeometry,
    scene_to_screen: &impl Fn(f32, f32) -> Pos2,
) -> GizmoScreenHandles {
    let rad = geo.tf.rotation_deg.to_radians();
    let cos = rad.cos();
    let sin = rad.sin();
    let rot = |dx: f32, dy: f32| -> Pos2 {
        let sx = geo.center_x + dx * cos - dy * sin;
        let sy = geo.center_y + dx * sin + dy * cos;
        scene_to_screen(sx, sy)
    };
    let hw = geo.scaled_w * 0.5;
    let hh = geo.scaled_h * 0.5;
    let corners = [rot(-hw, -hh), rot(hw, -hh), rot(hw, hh), rot(-hw, hh)];
    let center = scene_to_screen(geo.center_x, geo.center_y);
    let top_mid = Pos2::new(
        (corners[0].x + corners[1].x) * 0.5,
        (corners[0].y + corners[1].y) * 0.5,
    );
    let up_dir = (top_mid - center).normalized();
    let rotate_knob = top_mid + up_dir * 24.0;
    GizmoScreenHandles {
        corners,
        center,
        top_mid,
        rotate_knob,
    }
}

fn draw_gizmo(painter: &egui::Painter, h: &GizmoScreenHandles, rotation_deg: f32) {
    let accent = Color32::from_rgb(255, 210, 64);
    for i in 0..4 {
        painter.line_segment(
            [h.corners[i], h.corners[(i + 1) % 4]],
            Stroke::new(1.8, accent),
        );
    }
    // 回転ステム＆ノブ
    painter.line_segment([h.top_mid, h.rotate_knob], Stroke::new(1.5, accent));
    painter.circle_filled(h.rotate_knob, 5.5, Color32::from_rgb(56, 189, 248));
    painter.circle_stroke(h.rotate_knob, 5.5, Stroke::new(1.5, Color32::WHITE));

    // 4隅スケールハンドル
    for &c in &h.corners {
        let r = Rect::from_center_size(c, Vec2::splat(9.0));
        painter.rect_filled(r, 1.5, Color32::WHITE);
        painter.rect_stroke(r, 1.5, Stroke::new(1.5, accent), StrokeKind::Outside);
    }

    // 中心クロスヘア
    painter.line_segment(
        [
            Pos2::new(h.center.x - 6.0, h.center.y),
            Pos2::new(h.center.x + 6.0, h.center.y),
        ],
        Stroke::new(1.2, accent),
    );
    painter.line_segment(
        [
            Pos2::new(h.center.x, h.center.y - 6.0),
            Pos2::new(h.center.x, h.center.y + 6.0),
        ],
        Stroke::new(1.2, accent),
    );
    if rotation_deg.abs() > 0.1 {
        painter.text(
            h.rotate_knob + Vec2::new(10.0, -4.0),
            egui::Align2::LEFT_CENTER,
            format!("{rotation_deg:.1}°"),
            egui::FontId::monospace(10.5),
            Color32::from_rgb(130, 225, 255),
        );
    }
}

fn draw_safe_guides(painter: &egui::Painter, rect: Rect, is_vertical: bool) {
    // Action Safe (90%) & Title Safe (80%)
    let action = rect.shrink2(rect.size() * 0.05);
    let title = rect.shrink2(rect.size() * 0.10);
    painter.rect_stroke(
        action,
        0.0,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(180, 210, 255, 70)),
        StrokeKind::Inside,
    );
    painter.rect_stroke(
        title,
        0.0,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(180, 210, 255, 45)),
        StrokeKind::Inside,
    );

    // 9:16 縦動画の場合は Shorts / TikTok UI 被り警告エリアを薄赤で可視化
    if is_vertical {
        let right_ui = Rect::from_min_max(
            Pos2::new(
                rect.right() - rect.width() * 0.16,
                rect.top() + rect.height() * 0.45,
            ),
            Pos2::new(rect.right(), rect.bottom() - rect.height() * 0.18),
        );
        let bottom_ui = Rect::from_min_max(
            Pos2::new(rect.left(), rect.bottom() - rect.height() * 0.18),
            rect.max,
        );
        let warn_fill = Color32::from_rgba_unmultiplied(235, 72, 72, 34);
        painter.rect_filled(right_ui, 0.0, warn_fill);
        painter.rect_filled(bottom_ui, 0.0, warn_fill);
        painter.text(
            bottom_ui.center(),
            egui::Align2::CENTER_CENTER,
            "Shorts / TikTok 説明欄 UI 被り注意エリア",
            egui::FontId::proportional(10.0),
            Color32::from_rgba_unmultiplied(255, 160, 160, 180),
        );
    }
}

fn draw_rule_of_thirds(painter: &egui::Painter, rect: Rect) {
    let col = Color32::from_rgba_unmultiplied(255, 255, 255, 65);
    for frac in [1.0 / 3.0, 2.0 / 3.0] {
        let x = rect.left() + rect.width() * frac;
        let y = rect.top() + rect.height() * frac;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, col),
        );
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, col),
        );
    }
}

fn draw_bezier_mask_overlay(
    painter: &egui::Painter,
    geo: &ItemGeometry,
    masks: &[BezierMask],
    scene_to_screen: &impl Fn(f32, f32) -> Pos2,
) {
    let ox = geo.tf.x + geo.layer_offset_x;
    let oy = geo.tf.y + geo.layer_offset_y;
    let sw = geo.scaled_w.max(1.0);
    let sh = geo.scaled_h.max(1.0);

    for mask in masks {
        if !mask.enabled || mask.vertices.len() < 3 {
            continue;
        }
        let stroke_col = match mask.mode {
            MaskMode::Add => Color32::from_rgb(255, 200, 64),
            MaskMode::Subtract => Color32::from_rgb(255, 92, 136),
        };
        let poly = mask.flatten_polygon(12);
        if poly.len() >= 3 {
            let pts: Vec<Pos2> = poly
                .iter()
                .map(|[nx, ny]| scene_to_screen(ox + nx * sw, oy + ny * sh))
                .collect();
            for i in 0..pts.len() {
                painter.line_segment(
                    [pts[i], pts[(i + 1) % pts.len()]],
                    Stroke::new(1.8, stroke_col),
                );
            }
        }
        let tan_col = Color32::from_rgba_unmultiplied(255, 230, 150, 190);
        for v in &mask.vertices {
            let vp = scene_to_screen(ox + v.x * sw, oy + v.y * sh);
            // ベジェ接線ハンドル（長さが 0 より大きい場合のみ描画）
            if v.in_handle[0].hypot(v.in_handle[1]) > 1e-4 {
                let hp = scene_to_screen(
                    ox + (v.x + v.in_handle[0]) * sw,
                    oy + (v.y + v.in_handle[1]) * sh,
                );
                painter.line_segment([vp, hp], Stroke::new(1.0, tan_col));
                painter.circle_filled(hp, 3.5, tan_col);
            }
            if v.out_handle[0].hypot(v.out_handle[1]) > 1e-4 {
                let hp = scene_to_screen(
                    ox + (v.x + v.out_handle[0]) * sw,
                    oy + (v.y + v.out_handle[1]) * sh,
                );
                painter.line_segment([vp, hp], Stroke::new(1.0, tan_col));
                painter.circle_filled(hp, 3.5, tan_col);
            }
            let r = Rect::from_center_size(vp, Vec2::splat(8.5));
            painter.rect_filled(r, 1.5, stroke_col);
            painter.rect_stroke(
                r,
                1.5,
                Stroke::new(1.0, Color32::BLACK),
                StrokeKind::Outside,
            );
        }
    }
}

fn hit_test_bezier_masks(
    geo: &ItemGeometry,
    masks: &[BezierMask],
    pointer: Pos2,
    hit_r: f32,
    scene_to_screen: &impl Fn(f32, f32) -> Pos2,
) -> Option<GizmoDrag> {
    let ox = geo.tf.x + geo.layer_offset_x;
    let oy = geo.tf.y + geo.layer_offset_y;
    let sw = geo.scaled_w.max(1.0);
    let sh = geo.scaled_h.max(1.0);

    for (mi, mask) in masks.iter().enumerate() {
        if !mask.enabled {
            continue;
        }
        for (vi, v) in mask.vertices.iter().enumerate() {
            let vp = scene_to_screen(ox + v.x * sw, oy + v.y * sh);
            if vp.distance(pointer) <= hit_r {
                return Some(GizmoDrag::MaskVertex {
                    item_id: geo.item_id,
                    mask_idx: mi,
                    vert_idx: vi,
                    origin_x: ox,
                    origin_y: oy,
                    scaled_w: sw,
                    scaled_h: sh,
                    masks: masks.to_vec(),
                });
            }
            if v.out_handle[0].hypot(v.out_handle[1]) > 1e-4 {
                let hp = scene_to_screen(
                    ox + (v.x + v.out_handle[0]) * sw,
                    oy + (v.y + v.out_handle[1]) * sh,
                );
                if hp.distance(pointer) <= hit_r {
                    return Some(GizmoDrag::MaskHandle {
                        item_id: geo.item_id,
                        mask_idx: mi,
                        vert_idx: vi,
                        is_out: true,
                        origin_x: ox,
                        origin_y: oy,
                        scaled_w: sw,
                        scaled_h: sh,
                        masks: masks.to_vec(),
                    });
                }
            }
            if v.in_handle[0].hypot(v.in_handle[1]) > 1e-4 {
                let hp = scene_to_screen(
                    ox + (v.x + v.in_handle[0]) * sw,
                    oy + (v.y + v.in_handle[1]) * sh,
                );
                if hp.distance(pointer) <= hit_r {
                    return Some(GizmoDrag::MaskHandle {
                        item_id: geo.item_id,
                        mask_idx: mi,
                        vert_idx: vi,
                        is_out: false,
                        origin_x: ox,
                        origin_y: oy,
                        scaled_w: sw,
                        scaled_h: sh,
                        masks: masks.to_vec(),
                    });
                }
            }
        }
    }
    None
}

/// 1軸方向についてアイテム中心・左/上端・右/下端をキャンバス中央＆セーフ枠へ吸着させる
fn snap_axis_with_bounds(
    pos: f32,
    size: f32,
    canvas_len: f32,
    tolerance: f32,
) -> (f32, Option<f32>) {
    let center = pos + size * 0.5;
    let canvas_center = canvas_len * 0.5;
    if (center - canvas_center).abs() <= tolerance {
        return (canvas_center - size * 0.5, Some(canvas_center));
    }
    let safe_lo = canvas_len * 0.10;
    let safe_hi = canvas_len * 0.90;
    if (pos - safe_lo).abs() <= tolerance {
        return (safe_lo, Some(safe_lo));
    }
    if ((pos + size) - safe_hi).abs() <= tolerance {
        return (safe_hi - size, Some(safe_hi));
    }
    if pos.abs() <= tolerance {
        return (0.0, Some(0.0));
    }
    if ((pos + size) - canvas_len).abs() <= tolerance {
        return (canvas_len - size, Some(canvas_len));
    }
    (pos, None)
}

#[allow(clippy::too_many_arguments)]
fn compute_scaled_factors(
    init_sx: f32,
    init_sy: f32,
    _corner: ScaleCorner,
    center: Pos2,
    start_pos: Pos2,
    cur_pos: Pos2,
    start_dist: f32,
    lock_aspect: bool,
) -> (f32, f32) {
    if lock_aspect {
        let ratio = (cur_pos.distance(center) / start_dist.max(4.0)).clamp(0.05, 12.0);
        (
            (init_sx * ratio).clamp(0.05, 12.0),
            (init_sy * ratio).clamp(0.05, 12.0),
        )
    } else {
        let start_dx = (start_pos.x - center.x).abs().max(8.0);
        let start_dy = (start_pos.y - center.y).abs().max(8.0);
        let cur_dx = (cur_pos.x - center.x).abs().max(4.0);
        let cur_dy = (cur_pos.y - center.y).abs().max(4.0);
        (
            (init_sx * (cur_dx / start_dx)).clamp(0.05, 12.0),
            (init_sy * (cur_dy / start_dy)).clamp(0.05, 12.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_axis_snaps_item_center_to_canvas_center() {
        // 1280px キャンバスで幅 200px のアイテム: x=536 => center=636 (中央640から4px以内)
        let (snapped, guide) = snap_axis_with_bounds(536.0, 200.0, 1280.0, 10.0);
        assert!((snapped - 540.0).abs() < 1e-4);
        assert_eq!(guide, Some(640.0));
    }

    #[test]
    fn corner_scaling_respects_shift_aspect_lock() {
        let (sx, sy) = compute_scaled_factors(
            1.0,
            1.0,
            ScaleCorner::BottomRight,
            Pos2::new(100.0, 100.0),
            Pos2::new(200.0, 150.0),
            Pos2::new(300.0, 200.0),
            Pos2::new(200.0, 150.0).distance(Pos2::new(100.0, 100.0)),
            true,
        );
        assert!((sx - 2.0).abs() < 0.02);
        assert!((sy - 2.0).abs() < 0.02);
    }
}
