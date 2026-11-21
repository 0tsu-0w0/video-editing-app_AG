use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::BTreeMap;
use uuid::Uuid;
use ve_audio::WaveformPeaks;
use ve_core::{
    AlignMode, CommandSource, EditCommand, FilterTargetScope, ItemId, ItemKind, RationalTime,
    RgbaColor, SceneId, SelectDirection, TextAnimator, TimelineItem, Transform2D, TransitionKind,
    build_align_commands, build_close_gaps_commands, build_fit_duration_to_layer_commands,
    build_replace_text_commands, search_text_items, select_items_from_time, select_layer_items,
};
use ve_mcp::EditorSession;

pub struct TimelineState {
    pub pixels_per_second: f32,
    pub scroll_offset_sec: f64,
    pub snap_enabled: bool,
    pub dragging_item: Option<DragOp>,
    // B-1: アルティメットプラグイン風 整列・隙間詰め・長さフィット
    pub distribute_gap_sec: f64,
    pub fit_reference_layer: usize,
    // B-5: テキスト検索・一括置換
    pub show_search_bar: bool,
    pub search_query: String,
    pub replace_text: String,
    pub search_case_sensitive: bool,
    pub search_match_index: usize,
    pub search_status: String,
    // A-3: レイヤー常駐設定エディタ
    pub editing_layer_index: Option<usize>,
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
            scroll_offset_sec: 0.0,
            snap_enabled: true,
            dragging_item: None,
            distribute_gap_sec: 0.25,
            fit_reference_layer: 3,
            show_search_bar: false,
            search_query: String::new(),
            replace_text: String::new(),
            search_case_sensitive: false,
            search_match_index: 0,
            search_status: String::new(),
            editing_layer_index: None,
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

        // ショートカット: Ctrl+Z / Ctrl+Y / Ctrl+F / Delete / S (分割) — テキスト入力フォーカス中は無効化
        let text_editing = ui.ctx().egui_wants_keyboard_input();
        let (undo_key, redo_key, split_key, delete_key, find_key) = if text_editing {
            (false, false, false, false, false)
        } else {
            ui.input(|i| {
                let undo = i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z);
                let redo = (i.modifiers.command && i.key_pressed(egui::Key::Y))
                    || (i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z));
                let split = !i.modifiers.command && i.key_pressed(egui::Key::S);
                let del = i.key_pressed(egui::Key::Delete);
                let find = i.modifiers.command && i.key_pressed(egui::Key::F);
                (undo, redo, split, del, find)
            })
        };
        if find_key {
            self.show_search_bar = !self.show_search_bar;
        }

        // ===== 1. メイン編集ツールバー =====
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(session.bus.can_undo(), egui::Button::new("↶ Undo"))
                .clicked()
                || undo_key
            {
                session.bus.undo(&mut session.project);
            }
            if ui
                .add_enabled(session.bus.can_redo(), egui::Button::new("↷ Redo"))
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
                    egui::Button::new("🗑 削除"),
                )
                .clicked()
                || delete_key
            {
                self.delete_selected(session);
            }
            if ui
                .add_enabled(
                    !session.selected_items.is_empty(),
                    egui::Button::new("📦 ワンクリック結合 (Precompose)"),
                )
                .clicked()
            {
                self.precompose_selected(session);
            }
            ui.separator();

            if ui.button("＋ 字幕").clicked() {
                let mut item = TimelineItem::new(
                    "新規字幕",
                    session.playhead,
                    RationalTime::from_seconds(2.0),
                    ItemKind::Text {
                        content: "新しい字幕テキスト".to_string(),
                        font_family: "sans-serif".to_string(),
                        font_size: 44.0,
                        color: RgbaColor::WHITE,
                        animator: TextAnimator::default(),
                    },
                );
                item.transform = Transform2D {
                    x: 160.0,
                    y: 480.0,
                    ..Default::default()
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
            if ui.button("＋ フィルタ").clicked() {
                let mut params = BTreeMap::new();
                params.insert("gain".to_string(), 1.3);
                let item = TimelineItem::new(
                    "明るさフィルタ",
                    session.playhead,
                    RationalTime::from_seconds(2.0),
                    ItemKind::FilterObject {
                        effect_type: "brightness".to_string(),
                        scope: FilterTargetScope::ImmediateLayer,
                        params,
                    },
                );
                let new_id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 2,
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
            if ui.button("＋ シーンチェンジ").clicked() {
                let item = TimelineItem::new(
                    "ワイプ (左→右)",
                    session.playhead,
                    RationalTime::from_seconds(1.0),
                    ItemKind::Transition {
                        transition: TransitionKind::WipeLeft,
                        scope: FilterTargetScope::ImmediateLayer,
                        softness: 0.08,
                    },
                );
                let new_id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 2,
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
            if ui.button("＋ グループ制御").clicked() {
                let item = TimelineItem::new(
                    "グループ制御 (2レイヤー)",
                    session.playhead,
                    RationalTime::from_seconds(3.0),
                    ItemKind::GroupControl {
                        target_layer_count: 2,
                    },
                );
                let new_id = item.id;
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: 2,
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

            ui.separator();
            ui.toggle_value(&mut self.show_search_bar, "🔍 検索/置換 (Ctrl+F)");
            ui.checkbox(&mut self.snap_enabled, "スナップ");
            ui.add(
                egui::Slider::new(&mut self.pixels_per_second, 30.0..=300.0).text("ズーム(px/s)"),
            );
        });

        // ===== 2. アイテム整列・隙間詰め・長さフィットバー（アルティメットプラグイン相当） =====
        let has_sel = !session.selected_items.is_empty();
        let has_multi = session.selected_items.len() >= 2;
        let layer_count = session
            .project
            .active_scene()
            .map(|s| s.layers.len())
            .unwrap_or(0);
        if layer_count > 0 && self.fit_reference_layer >= layer_count {
            self.fit_reference_layer = layer_count - 1;
        }

        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("選択: {}件", session.selected_items.len()));
            if ui
                .add_enabled(has_multi, egui::Button::new("🧲 隙間詰め(0s)").small())
                .clicked()
                && let Some(scene) = session.project.active_scene().cloned()
                && let Some(cmd) = build_close_gaps_commands(&scene, &session.selected_items, 0.0)
            {
                let _ = session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None);
            }
            ui.add(
                egui::DragValue::new(&mut self.distribute_gap_sec)
                    .speed(0.05)
                    .range(0.0..=10.0)
                    .suffix("s"),
            );
            if ui
                .add_enabled(has_multi, egui::Button::new("📏 等間隔整列").small())
                .clicked()
                && let Some(scene) = session.project.active_scene().cloned()
                && let Some(cmd) = build_close_gaps_commands(
                    &scene,
                    &session.selected_items,
                    self.distribute_gap_sec,
                )
            {
                let _ = session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None);
            }
            if ui
                .add_enabled(has_multi, egui::Button::new("⇤ 左揃え").small())
                .clicked()
                && let Some(scene) = session.project.active_scene().cloned()
                && let Some(cmd) = build_align_commands(
                    &scene,
                    &session.selected_items,
                    AlignMode::Left,
                    session.playhead,
                )
            {
                let _ = session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None);
            }
            if ui
                .add_enabled(has_multi, egui::Button::new("⇥ 右揃え").small())
                .clicked()
                && let Some(scene) = session.project.active_scene().cloned()
                && let Some(cmd) = build_align_commands(
                    &scene,
                    &session.selected_items,
                    AlignMode::Right,
                    session.playhead,
                )
            {
                let _ = session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None);
            }
            if ui
                .add_enabled(has_sel, egui::Button::new("📍 ヘッドへ吸着").small())
                .clicked()
                && let Some(scene) = session.project.active_scene().cloned()
                && let Some(cmd) = build_align_commands(
                    &scene,
                    &session.selected_items,
                    AlignMode::Playhead,
                    session.playhead,
                )
            {
                let _ = session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None);
            }
            ui.separator();
            ui.label("参照:");
            egui::ComboBox::from_id_salt("fit_ref_layer_combo")
                .width(72.0)
                .selected_text(format!("L{}", self.fit_reference_layer))
                .show_ui(ui, |ui| {
                    for idx in 0..layer_count {
                        ui.selectable_value(&mut self.fit_reference_layer, idx, format!("L{idx}"));
                    }
                });
            if ui
                .add_enabled(has_sel, egui::Button::new("↔ 長さフィット").small())
                .on_hover_text("選択中の字幕等を、参照レイヤー（例: 音声）の尺へ一括フィット")
                .clicked()
                && let Some(scene) = session.project.active_scene().cloned()
                && let Some(cmd) = build_fit_duration_to_layer_commands(
                    &scene,
                    &session.selected_items,
                    self.fit_reference_layer,
                )
            {
                let _ = session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None);
            }
            ui.weak(
                "ヒント: レイヤーCtrl+クリック=全選択 / 端Ctrl+ダブルクリック=以前・以降全選択",
            );
        });

        // ===== 3. テキスト検索＆一括置換バー（オブジェクトエクスプローラー相当） =====
        if self.show_search_bar {
            ui.horizontal_wrapped(|ui| {
                ui.label("検索:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("字幕テキスト検索...")
                        .desired_width(140.0),
                );
                ui.label("置換:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.replace_text)
                        .hint_text("置換後文字列...")
                        .desired_width(140.0),
                );
                ui.checkbox(&mut self.search_case_sensitive, "大小区別");

                let hits = session
                    .project
                    .active_scene()
                    .map(|s| search_text_items(s, &self.search_query, self.search_case_sensitive))
                    .unwrap_or_default();

                ui.weak(format!("一致: {}件", hits.len()));
                if ui
                    .add_enabled(!hits.is_empty(), egui::Button::new("◀ 前").small())
                    .clicked()
                {
                    self.search_match_index = if self.search_match_index == 0 {
                        hits.len() - 1
                    } else {
                        self.search_match_index - 1
                    };
                    if let Some(m) = hits.get(self.search_match_index) {
                        session.playhead = m.start;
                        session.selected_items = vec![m.item_id];
                        self.scroll_offset_sec = (m.start.as_seconds() - 1.0).max(0.0);
                    }
                }
                if ui
                    .add_enabled(!hits.is_empty(), egui::Button::new("次 ▶").small())
                    .clicked()
                {
                    self.search_match_index = (self.search_match_index + 1) % hits.len();
                    if let Some(m) = hits.get(self.search_match_index) {
                        session.playhead = m.start;
                        session.selected_items = vec![m.item_id];
                        self.scroll_offset_sec = (m.start.as_seconds() - 1.0).max(0.0);
                    }
                }
                if ui
                    .add_enabled(!hits.is_empty(), egui::Button::new("🔁 すべて置換").small())
                    .clicked()
                    && let Some(scene) = session.project.active_scene().cloned()
                    && let Some((count, cmd)) = build_replace_text_commands(
                        &scene,
                        &self.search_query,
                        &self.replace_text,
                        self.search_case_sensitive,
                    )
                    && session
                        .bus
                        .execute(&mut session.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                {
                    self.search_status =
                        format!("{count} 箇所を一括置換しました (Ctrl+Z で元に戻せます)");
                }
                if !self.search_status.is_empty() {
                    ui.weak(&self.search_status);
                }
            });
        }

        // ===== 4. レイヤー常駐設定エディタ（⚙ クリック時のみ表示） =====
        if let Some(l_idx) = self.editing_layer_index {
            let layer_snap = session
                .project
                .active_scene()
                .and_then(|s| s.layers.get(l_idx).cloned());
            if let Some(mut l) = layer_snap {
                let prev = l.clone();
                let mut close_editor = false;
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("⚙ L{l_idx} レイヤー設定:"));
                    ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(90.0));
                    ui.checkbox(&mut l.visible, "表示");
                    ui.checkbox(&mut l.muted, "ミュート");
                    ui.label("X:");
                    ui.add(egui::DragValue::new(&mut l.offset_x).speed(1.0));
                    ui.label("Y:");
                    ui.add(egui::DragValue::new(&mut l.offset_y).speed(1.0));
                    ui.label("音量:");
                    ui.add(
                        egui::Slider::new(&mut l.volume, 0.0..=2.0)
                            .fixed_decimals(2)
                            .show_value(true),
                    );
                    ui.label("色:");
                    if ui.small_button("無").clicked() {
                        l.color_tag = None;
                    }
                    for (label, c) in [
                        (
                            "青",
                            RgbaColor {
                                r: 0.25,
                                g: 0.55,
                                b: 0.95,
                                a: 1.0,
                            },
                        ),
                        (
                            "緑",
                            RgbaColor {
                                r: 0.28,
                                g: 0.78,
                                b: 0.45,
                                a: 1.0,
                            },
                        ),
                        (
                            "黄",
                            RgbaColor {
                                r: 0.95,
                                g: 0.78,
                                b: 0.22,
                                a: 1.0,
                            },
                        ),
                        (
                            "赤",
                            RgbaColor {
                                r: 0.92,
                                g: 0.32,
                                b: 0.32,
                                a: 1.0,
                            },
                        ),
                        (
                            "紫",
                            RgbaColor {
                                r: 0.68,
                                g: 0.42,
                                b: 0.90,
                                a: 1.0,
                            },
                        ),
                    ] {
                        if ui.small_button(label).clicked() {
                            l.color_tag = Some(c);
                        }
                    }
                    if ui.small_button("閉じる").clicked() {
                        close_editor = true;
                    }
                });
                if l != prev {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetLayerSettings {
                            layer_index: l_idx,
                            name: l.name,
                            visible: l.visible,
                            muted: l.muted,
                            offset_x: l.offset_x,
                            offset_y: l.offset_y,
                            volume: l.volume,
                            color_tag: l.color_tag,
                        },
                        CommandSource::Gui,
                        Some("layer_settings_edit"),
                    );
                }
                if close_editor {
                    self.editing_layer_index = None;
                }
            } else {
                self.editing_layer_index = None;
            }
        }

        ui.separator();

        // ===== 5. タイムラインミニマップ（アルティメットプラグイン「タイムラインマップ」相当） =====
        let total_scene_sec = session
            .project
            .active_scene()
            .map(|s| s.total_duration().as_seconds())
            .unwrap_or(10.0)
            .max(10.0);
        let header_w = 156.0;
        let avail_w = ui.available_width().max(400.0);
        let visible_span_sec = ((avail_w - header_w).max(60.0) / self.pixels_per_second) as f64;
        self.draw_minimap(
            ui,
            session,
            total_scene_sec,
            visible_span_sec,
            avail_w,
            header_w,
        );

        // ===== 6. メインタイムラインキャンバス =====
        let ruler_h = 24.0;
        let track_h = 36.0;
        let total_h = ruler_h + track_h * (layer_count.max(1) as f32) + 8.0;

        let (canvas_rect, response) =
            ui.allocate_exact_size(Vec2::new(avail_w, total_h), Sense::click_and_drag());
        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 2.0, Color32::from_rgb(24, 26, 32));

        let track_origin_x = canvas_rect.left() + header_w;
        let pps = self.pixels_per_second;
        let scroll_sec = self.scroll_offset_sec.max(0.0);

        // Shift + ホイールで横スクロール、Ctrl + ホイールでズーム
        if response.hovered() {
            let (scroll_delta, ctrl_held) =
                ui.input(|i| (i.smooth_scroll_delta, i.modifiers.command));
            if ctrl_held && scroll_delta.y.abs() > 0.1 {
                let factor = if scroll_delta.y > 0.0 { 1.12 } else { 0.89 };
                self.pixels_per_second = (self.pixels_per_second * factor).clamp(30.0, 300.0);
            } else if scroll_delta.x.abs() > 0.1 {
                self.scroll_offset_sec =
                    (self.scroll_offset_sec - f64::from(scroll_delta.x) / f64::from(pps)).max(0.0);
            }
        }

        // ルーラー描画
        let ruler_rect = Rect::from_min_max(
            Pos2::new(track_origin_x, canvas_rect.top()),
            Pos2::new(canvas_rect.right(), canvas_rect.top() + ruler_h),
        );
        painter.rect_filled(ruler_rect, 0.0, Color32::from_rgb(36, 40, 48));
        let first_sec = scroll_sec.floor() as i32;
        let last_sec =
            ((scroll_sec + f64::from(ruler_rect.width() / pps)).ceil() as i32).max(first_sec + 1);
        for sec in first_sec..=last_sec {
            let x = track_origin_x + (f64::from(sec) - scroll_sec) as f32 * pps;
            if x < track_origin_x - 2.0 || x > canvas_rect.right() {
                continue;
            }
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
                    RationalTime::from_seconds(f64::from(sec)).to_frame_round(fps)
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

        let ctrl_held = ui.input(|i| i.modifiers.command);
        let shift_held = ui.input(|i| i.modifiers.shift);

        // レイヤー行＆クリップ描画
        let mut clicked_item: Option<(ItemId, DragMode)> = None;
        let mut enter_precomp_scene: Option<SceneId> = None;
        let mut ctrl_select_layer: Option<usize> = None;
        let mut ctrl_dbl_select: Option<(RationalTime, SelectDirection)> = None;
        let mut toggle_layer_vis: Option<usize> = None;
        let mut toggle_layer_mute: Option<usize> = None;
        let mut open_layer_settings: Option<usize> = None;

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

                // レイヤーカラータグ縦バー（A-3）
                if let Some(tag) = layer.color_tag {
                    let stripe = Rect::from_min_max(
                        header_rect.min,
                        Pos2::new(header_rect.left() + 5.0, header_rect.bottom()),
                    );
                    painter.rect_filled(stripe, 1.0, rgba_to_color32(tag));
                }

                // ヘッダー内ミニアイコン当たり判定（表示 / ミュート / 設定⚙）
                let vis_rect = Rect::from_min_max(
                    Pos2::new(header_rect.right() - 62.0, header_rect.top() + 4.0),
                    Pos2::new(header_rect.right() - 42.0, header_rect.bottom() - 4.0),
                );
                let mute_rect = Rect::from_min_max(
                    Pos2::new(header_rect.right() - 42.0, header_rect.top() + 4.0),
                    Pos2::new(header_rect.right() - 22.0, header_rect.bottom() - 4.0),
                );
                let gear_rect = Rect::from_min_max(
                    Pos2::new(header_rect.right() - 22.0, header_rect.top() + 4.0),
                    Pos2::new(header_rect.right() - 2.0, header_rect.bottom() - 4.0),
                );

                let matte_badge = if layer.track_matte.is_some() {
                    "[M]"
                } else {
                    ""
                };
                painter.text(
                    Pos2::new(header_rect.left() + 8.0, header_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("L{layer_idx} {}{matte_badge}", layer.name),
                    egui::FontId::proportional(11.0),
                    if layer.visible {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(120)
                    },
                );
                painter.text(
                    vis_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if layer.visible { "👁" } else { "·" },
                    egui::FontId::proportional(11.0),
                    if layer.visible {
                        Color32::LIGHT_GRAY
                    } else {
                        Color32::DARK_GRAY
                    },
                );
                painter.text(
                    mute_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if layer.muted { "🔇" } else { "🔊" },
                    egui::FontId::proportional(11.0),
                    if layer.muted {
                        Color32::from_rgb(235, 100, 100)
                    } else {
                        Color32::LIGHT_GRAY
                    },
                );
                painter.text(
                    gear_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "⚙",
                    egui::FontId::proportional(11.0),
                    if self.editing_layer_index == Some(layer_idx) {
                        Color32::YELLOW
                    } else {
                        Color32::from_gray(165)
                    },
                );

                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && header_rect.contains(pos)
                {
                    if vis_rect.contains(pos) {
                        toggle_layer_vis = Some(layer_idx);
                    } else if mute_rect.contains(pos) {
                        toggle_layer_mute = Some(layer_idx);
                    } else if gear_rect.contains(pos) {
                        open_layer_settings = Some(layer_idx);
                    } else if ctrl_held {
                        ctrl_select_layer = Some(layer_idx);
                    }
                }

                painter.line_segment(
                    [
                        Pos2::new(track_origin_x, row_rect.bottom()),
                        Pos2::new(canvas_rect.right(), row_rect.bottom()),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(42, 46, 56)),
                );

                for item in &layer.items {
                    let x0 = track_origin_x + (item.start.as_seconds() - scroll_sec) as f32 * pps;
                    let x1 = (track_origin_x + (item.end().as_seconds() - scroll_sec) as f32 * pps)
                        .max(x0 + 8.0);
                    if x1 < track_origin_x || x0 > canvas_rect.right() {
                        continue;
                    }
                    let vis_x0 = x0.max(track_origin_x);
                    let clip_rect = Rect::from_min_max(
                        Pos2::new(vis_x0, y0 + 2.0),
                        Pos2::new(x1.min(canvas_rect.right()), y0 + track_h - 5.0),
                    );
                    let selected = session.selected_items.contains(&item.id);
                    let fill = clip_color_for_kind(&item.kind);
                    painter.rect_filled(clip_rect, 4.0, fill);

                    // 音声クリップの場合は波形ピークを描画
                    if let ItemKind::Audio { asset_id, .. } = &item.kind
                        && let Some(wf) = waveforms.get(asset_id)
                    {
                        draw_waveform(&painter, clip_rect, wf);
                    }

                    // キーフレームマーカー（黄色ドット）
                    for kfs in item.keyframes.values() {
                        for kf in kfs {
                            let kx = x0 + kf.time.as_seconds() as f32 * pps;
                            if kx >= clip_rect.left() && kx <= clip_rect.right() {
                                painter.circle_filled(
                                    Pos2::new(kx, clip_rect.bottom() - 5.0),
                                    2.5,
                                    Color32::from_rgb(255, 232, 92),
                                );
                            }
                        }
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

                    let mut speed_tag = String::new();
                    if (item.playback.speed - 1.0).abs() > 0.01 {
                        speed_tag.push_str(&format!(" [{:.2}x]", item.playback.speed));
                    }
                    if item.playback.loop_playback {
                        speed_tag.push_str(" 🔁");
                    }
                    let label = match &item.kind {
                        ItemKind::Precomp { .. } => {
                            format!("📦 {} (ダブルクリック編集)", item.name)
                        }
                        ItemKind::Transition { transition, .. } => {
                            format!("🎞 {}", transition.display_name())
                        }
                        _ => format!("{}{speed_tag}", item.name),
                    };
                    painter.text(
                        Pos2::new(clip_rect.left() + 6.0, clip_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );

                    // ダブルクリック判定: Ctrl+ダブルクリック=以前/以降全選択 (AviUtl2), 通常ダブルクリック=Precomp編集
                    if response.double_clicked()
                        && let Some(pos) = response.interact_pointer_pos()
                        && clip_rect.contains(pos)
                    {
                        if ctrl_held {
                            let dir = if pos.x >= clip_rect.center().x {
                                SelectDirection::After
                            } else {
                                SelectDirection::Before
                            };
                            ctrl_dbl_select = Some((item.start, dir));
                        } else if let ItemKind::Precomp { scene_id } = item.kind {
                            enter_precomp_scene = Some(scene_id);
                        }
                    }

                    // クリック／ドラッグ開始の当たり判定
                    if (response.drag_started() || response.clicked())
                        && let Some(pos) = response.interact_pointer_pos()
                        && clip_rect.contains(pos)
                    {
                        let is_right_edge = (x1 - pos.x).abs() <= 8.0;
                        let mode = if is_right_edge {
                            DragMode::TrimRight
                        } else {
                            let ptr_sec = scroll_sec + ((pos.x - track_origin_x) / pps) as f64;
                            DragMode::Move {
                                grab_offset_sec: ptr_sec - item.start.as_seconds(),
                            }
                        };
                        clicked_item = Some((item.id, mode));
                    }
                }
            }
        }

        // レイヤーヘッダーのクイック操作反映
        if let Some(l_idx) = toggle_layer_vis
            && let Some(l) = session
                .project
                .active_scene()
                .and_then(|s| s.layers.get(l_idx).cloned())
        {
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetLayerSettings {
                    layer_index: l_idx,
                    name: l.name,
                    visible: !l.visible,
                    muted: l.muted,
                    offset_x: l.offset_x,
                    offset_y: l.offset_y,
                    volume: l.volume,
                    color_tag: l.color_tag,
                },
                CommandSource::Gui,
                None,
            );
        }
        if let Some(l_idx) = toggle_layer_mute
            && let Some(l) = session
                .project
                .active_scene()
                .and_then(|s| s.layers.get(l_idx).cloned())
        {
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetLayerSettings {
                    layer_index: l_idx,
                    name: l.name,
                    visible: l.visible,
                    muted: !l.muted,
                    offset_x: l.offset_x,
                    offset_y: l.offset_y,
                    volume: l.volume,
                    color_tag: l.color_tag,
                },
                CommandSource::Gui,
                None,
            );
        }
        if let Some(l_idx) = open_layer_settings {
            self.editing_layer_index = if self.editing_layer_index == Some(l_idx) {
                None
            } else {
                Some(l_idx)
            };
        }
        if let Some(l_idx) = ctrl_select_layer
            && let Some(scene) = session.project.active_scene()
        {
            session.selected_items = select_layer_items(scene, l_idx);
            return;
        }
        if let Some((pivot, dir)) = ctrl_dbl_select
            && let Some(scene) = session.project.active_scene()
        {
            session.selected_items = select_items_from_time(scene, pivot, dir, None);
            return;
        }

        if let Some(scene_id) = enter_precomp_scene {
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SwitchScene { scene_id },
                CommandSource::Gui,
                None,
            );
            session.selected_items.clear();
            return;
        }

        // ポインタ操作の処理（選択・ドラッグ移動・伸縮・ルーラーシーク）
        if let Some((item_id, mode)) = clicked_item {
            if shift_held {
                if let Some(pos) = session.selected_items.iter().position(|id| *id == item_id) {
                    if !response.drag_started() {
                        session.selected_items.remove(pos);
                    }
                } else {
                    session.selected_items.push(item_id);
                }
            } else if !session.selected_items.contains(&item_id) || !response.drag_started() {
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
            let sec = (scroll_sec + ((pos.x - track_origin_x) / pps) as f64).max(0.0);
            session.playhead = RationalTime::from_seconds(sec);
        }

        if let Some(drag) = self.dragging_item {
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let raw_sec = (scroll_sec + ((pos.x - track_origin_x) / pps) as f64).max(0.0);
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
        let playhead_x = track_origin_x + (session.playhead.as_seconds() - scroll_sec) as f32 * pps;
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

    /// タイムライン全体を俯瞰・クリックジャンプできるミニマップバーを描画する
    fn draw_minimap(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        total_scene_sec: f64,
        visible_span_sec: f64,
        avail_w: f32,
        header_w: f32,
    ) {
        let map_h = 26.0;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(avail_w, map_h), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, Color32::from_rgb(18, 20, 26));

        let label_rect = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.left() + header_w - 2.0, rect.bottom()),
        );
        painter.rect_filled(label_rect, 2.0, Color32::from_rgb(30, 34, 42));
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("🗺 マップ ({total_scene_sec:.1}s)"),
            egui::FontId::proportional(10.5),
            Color32::from_gray(190),
        );

        let bar_rect = Rect::from_min_max(
            Pos2::new(rect.left() + header_w, rect.top() + 2.0),
            Pos2::new(rect.right(), rect.bottom() - 2.0),
        );
        let span = total_scene_sec.max(visible_span_sec).max(1.0);
        if let Some(scene) = session.project.active_scene() {
            let l_cnt = scene.layers.len().max(1) as f32;
            let row_h = (bar_rect.height() / l_cnt).max(2.0);
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                let y = bar_rect.top() + l_idx as f32 * row_h;
                for item in &layer.items {
                    let x0 = bar_rect.left()
                        + (item.start.as_seconds() / span) as f32 * bar_rect.width();
                    let x1 = (bar_rect.left()
                        + (item.end().as_seconds() / span) as f32 * bar_rect.width())
                    .max(x0 + 2.0);
                    let r = Rect::from_min_max(
                        Pos2::new(x0, y),
                        Pos2::new(x1.min(bar_rect.right()), (y + row_h - 1.0).max(y + 1.0)),
                    );
                    painter.rect_filled(r, 1.0, clip_color_for_kind(&item.kind));
                }
            }
        }

        // 現在の表示ウィンドウ枠
        let vx0 = bar_rect.left() + (self.scroll_offset_sec / span) as f32 * bar_rect.width();
        let vx1 = (bar_rect.left()
            + ((self.scroll_offset_sec + visible_span_sec) / span) as f32 * bar_rect.width())
        .min(bar_rect.right());
        let vp_rect = Rect::from_min_max(
            Pos2::new(vx0.clamp(bar_rect.left(), bar_rect.right()), bar_rect.top()),
            Pos2::new(vx1.max(vx0 + 6.0), bar_rect.bottom()),
        );
        painter.rect_filled(
            vp_rect,
            1.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 26),
        );
        painter.rect_stroke(
            vp_rect,
            1.0,
            Stroke::new(1.0, Color32::from_gray(200)),
            StrokeKind::Inside,
        );

        // ミニマップ上の再生ヘッド位置
        let ph_x =
            bar_rect.left() + (session.playhead.as_seconds() / span) as f32 * bar_rect.width();
        if ph_x >= bar_rect.left() && ph_x <= bar_rect.right() {
            painter.line_segment(
                [
                    Pos2::new(ph_x, bar_rect.top()),
                    Pos2::new(ph_x, bar_rect.bottom()),
                ],
                Stroke::new(1.5, Color32::from_rgb(235, 72, 72)),
            );
        }

        // ミニマップクリック・ドラッグで即ジャンプ＆スクロール
        if (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
            && pos.x >= bar_rect.left()
        {
            let frac = ((pos.x - bar_rect.left()) / bar_rect.width().max(1.0)).clamp(0.0, 1.0);
            let target_sec = f64::from(frac) * span;
            session.playhead = RationalTime::from_seconds(target_sec);
            self.scroll_offset_sec = (target_sec - visible_span_sec * 0.35).max(0.0);
        }
    }

    fn precompose_selected(&self, session: &mut EditorSession) {
        if session.selected_items.is_empty() {
            return;
        }
        let new_scene_id = Uuid::new_v4();
        let precomp_item_id = Uuid::new_v4();
        let next_num = session.project.scenes.len();
        let cmd = EditCommand::PrecomposeSelection {
            item_ids: session.selected_items.clone(),
            new_scene_id,
            precomp_name: format!("Precomp {next_num}"),
            precomp_item_id,
            target_layer_index: 0,
        };
        if session
            .bus
            .execute(&mut session.project, cmd, CommandSource::Gui, None)
            .is_ok()
        {
            session.selected_items = vec![precomp_item_id];
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

fn clip_color_for_kind(kind: &ItemKind) -> Color32 {
    match kind {
        ItemKind::Text { .. } => Color32::from_rgb(72, 130, 196),
        ItemKind::FilterObject { .. } => Color32::from_rgb(165, 102, 185),
        ItemKind::Transition { .. } => Color32::from_rgb(214, 118, 52),
        ItemKind::GroupControl { .. } => Color32::from_rgb(196, 140, 56),
        ItemKind::CharacterSprite { .. } => Color32::from_rgb(208, 102, 140),
        ItemKind::Precomp { .. } => Color32::from_rgb(42, 152, 148),
        ItemKind::Audio { .. } => Color32::from_rgb(64, 152, 108),
        _ => Color32::from_rgb(88, 104, 138),
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
