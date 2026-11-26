use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
use ve_audio::{
    AudioGraph, BeatDetectionConfig, WaveformPeaks, detect_audio_beats, detect_beats_from_waveform,
};
use ve_core::{
    AlignMode, CachedSceneIntervalIndex, CommandSource, EditCommand, FilterTargetScope, ItemId,
    ItemKind, JumpCutConfig, ParticleConfig, RationalTime, RgbaColor, SceneId, SelectDirection,
    TextAnimator, TimelineItem, TimelineMarker, Transform2D, TransitionKind, VectorShapeConfig,
    build_align_commands, build_beat_cut_commands, build_beat_markers_command,
    build_close_gaps_commands, build_fit_duration_to_layer_commands, build_jump_cut_commands,
    build_replace_text_commands, build_ripple_delete_commands, build_snap_to_beats_commands,
    detect_speech_intervals_from_rms, format_youtube_chapters, search_text_items,
    select_items_from_time, select_layer_items,
};
use ve_mcp::EditorSession;

/// J/K/L シャトル再生・フレームステップ等の再生制御アクション
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelinePlaybackAction {
    None,
    /// `J`: 逆再生 / 速度アップ (-1x -> -2x -> -4x)
    ShuttleReverse,
    /// `K`: 一時停止＆1x リセット
    ShuttlePause,
    /// `L`: 正再生 / 倍速アップ (1x -> 2x -> 4x)
    ShuttleForward,
}

pub struct TimelineState {
    pub pixels_per_second: f32,
    pub scroll_offset_sec: f64,
    pub snap_enabled: bool,
    pub dragging_item: Option<DragOp>,
    // v0.4 #7: ラバーバンド（矩形）選択
    pub marquee_origin: Option<Pos2>,
    // v0.4 #7 / v0.7.1: マーカー追加・編集設定
    pub marker_label_input: String,
    pub marker_color_idx: usize,
    pub marker_status: String,
    pub editing_marker_id: Option<Uuid>,
    // v0.7.1: クリップコピー＆ペースト用クリップボード `(layer_index, TimelineItem)`
    pub clipboard_items: Vec<(usize, TimelineItem)>,
    // v0.4 #3: 無音区間自動カット（ジェットカット）
    pub show_jump_cut_bar: bool,
    pub jump_cut_audio_layer: usize,
    pub jump_cut_rms_threshold: f32,
    pub jump_cut_min_silence_sec: f64,
    pub jump_cut_pad_before_sec: f64,
    pub jump_cut_pad_after_sec: f64,
    pub jump_cut_linked_layers: BTreeSet<usize>,
    pub jump_cut_status: String,
    // v0.8 #12: BGM ビート自動検出 ＆ ビートスナップ編集
    pub show_beat_bar: bool,
    pub beat_audio_layer: usize,
    pub beat_min_bpm: f32,
    pub beat_max_bpm: f32,
    pub beat_sensitivity: f32,
    pub beat_per_bar: usize,
    pub detected_bpm: f32,
    pub beat_grid: Vec<(f64, bool)>,
    pub beat_snap_enabled: bool,
    pub beat_status: String,
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
    // v7.0 #48: 10,000+ クリップ区間木 O(log N) キャッシュ
    pub interval_index: CachedSceneIntervalIndex,
}

#[derive(Debug, Clone)]
pub enum DragMode {
    Move {
        grab_offset_sec: f64,
        anchor_layer: usize,
        anchor_start_sec: f64,
        /// ドラッグ開始時の全選択対象 `(item_id, initial_layer, initial_start_sec)`
        group_items: Vec<(ItemId, usize, f64)>,
    },
    TrimRight,
}

#[derive(Debug, Clone)]
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
            marquee_origin: None,
            marker_label_input: String::new(),
            marker_color_idx: 0,
            marker_status: String::new(),
            editing_marker_id: None,
            clipboard_items: Vec::new(),
            show_jump_cut_bar: false,
            jump_cut_audio_layer: 3,
            jump_cut_rms_threshold: 0.025,
            jump_cut_min_silence_sec: 0.45,
            jump_cut_pad_before_sec: 0.08,
            jump_cut_pad_after_sec: 0.12,
            jump_cut_linked_layers: [0, 1, 2].into_iter().collect(),
            jump_cut_status: String::new(),
            show_beat_bar: false,
            beat_audio_layer: 3,
            beat_min_bpm: 70.0,
            beat_max_bpm: 180.0,
            beat_sensitivity: 1.35,
            beat_per_bar: 4,
            detected_bpm: 0.0,
            beat_grid: Vec::new(),
            beat_snap_enabled: true,
            beat_status: String::new(),
            distribute_gap_sec: 0.25,
            fit_reference_layer: 3,
            show_search_bar: false,
            search_query: String::new(),
            replace_text: String::new(),
            search_case_sensitive: false,
            search_match_index: 0,
            search_status: String::new(),
            editing_layer_index: None,
            interval_index: CachedSceneIntervalIndex::default(),
        }
    }
}

const MARKER_PALETTE: &[(&str, RgbaColor)] = &[
    (
        "黄",
        RgbaColor {
            r: 0.98,
            g: 0.82,
            b: 0.20,
            a: 1.0,
        },
    ),
    (
        "水",
        RgbaColor {
            r: 0.25,
            g: 0.80,
            b: 0.98,
            a: 1.0,
        },
    ),
    (
        "緑",
        RgbaColor {
            r: 0.30,
            g: 0.86,
            b: 0.48,
            a: 1.0,
        },
    ),
    (
        "桃",
        RgbaColor {
            r: 0.96,
            g: 0.42,
            b: 0.65,
            a: 1.0,
        },
    ),
    (
        "赤",
        RgbaColor {
            r: 0.96,
            g: 0.28,
            b: 0.28,
            a: 1.0,
        },
    ),
];

#[derive(Default)]
struct TimelineShortcutInput {
    undo: bool,
    redo: bool,
    split: bool,
    delete: bool,
    ripple_delete: bool,
    find: bool,
    marker: bool,
    copy: bool,
    paste: bool,
    duplicate: bool,
    select_all: bool,
    group: bool,
    unlink: bool,
    jump_home: bool,
    jump_end: bool,
    step_frames: i64,
    shuttle: Option<TimelinePlaybackAction>,
}

impl TimelineState {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        waveforms: &BTreeMap<Uuid, WaveformPeaks>,
        ram_cached_ranges: &[(i64, i64)],
    ) -> TimelinePlaybackAction {
        let fps = session.project.settings.fps;
        let mut playback_action = TimelinePlaybackAction::None;

        // ショートカット:
        // Ctrl+Z/Y, Ctrl+C/V/D/A/G, Ctrl+Shift+G, Ctrl+F, Delete/Backspace (+Shift=リップル), S, M, Home/End, ←→(Shift=10f), J/K/L
        // テキスト入力フォーカス中はすべて抑止する
        let text_editing = ui.ctx().egui_wants_keyboard_input();
        let keys = if text_editing {
            TimelineShortcutInput::default()
        } else {
            ui.input(|i| {
                let cmd = i.modifiers.command;
                let shift = i.modifiers.shift;
                let del_pressed =
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace);
                let step_frames = if !cmd && i.key_pressed(egui::Key::ArrowLeft) {
                    if shift { -10 } else { -1 }
                } else if !cmd && i.key_pressed(egui::Key::ArrowRight) {
                    if shift { 10 } else { 1 }
                } else {
                    0
                };
                let shuttle = if !cmd && i.key_pressed(egui::Key::J) {
                    Some(TimelinePlaybackAction::ShuttleReverse)
                } else if !cmd && i.key_pressed(egui::Key::K) {
                    Some(TimelinePlaybackAction::ShuttlePause)
                } else if !cmd && i.key_pressed(egui::Key::L) {
                    Some(TimelinePlaybackAction::ShuttleForward)
                } else {
                    None
                };
                TimelineShortcutInput {
                    undo: cmd && !shift && i.key_pressed(egui::Key::Z),
                    redo: (cmd && i.key_pressed(egui::Key::Y))
                        || (cmd && shift && i.key_pressed(egui::Key::Z)),
                    split: !cmd && i.key_pressed(egui::Key::S),
                    delete: del_pressed && !shift,
                    ripple_delete: del_pressed && shift,
                    find: cmd && i.key_pressed(egui::Key::F),
                    marker: !cmd && i.key_pressed(egui::Key::M),
                    copy: cmd && !shift && i.key_pressed(egui::Key::C),
                    paste: cmd && !shift && i.key_pressed(egui::Key::V),
                    duplicate: cmd && !shift && i.key_pressed(egui::Key::D),
                    select_all: cmd && !shift && i.key_pressed(egui::Key::A),
                    group: cmd && !shift && i.key_pressed(egui::Key::G),
                    unlink: cmd && shift && i.key_pressed(egui::Key::G),
                    jump_home: !cmd && i.key_pressed(egui::Key::Home),
                    jump_end: !cmd && i.key_pressed(egui::Key::End),
                    step_frames,
                    shuttle,
                }
            })
        };
        if keys.find {
            self.show_search_bar = !self.show_search_bar;
        }
        if keys.jump_home {
            session.playhead = RationalTime::ZERO;
            self.scroll_offset_sec = 0.0;
            playback_action = TimelinePlaybackAction::ShuttlePause;
        } else if keys.jump_end
            && let Some(end_t) = session.project.active_scene().map(|s| s.total_duration())
        {
            session.playhead = end_t;
            self.scroll_offset_sec = (end_t.as_seconds() - 2.0).max(0.0);
            playback_action = TimelinePlaybackAction::ShuttlePause;
        }
        if keys.step_frames != 0 {
            let cur_f = session.playhead.to_frame_round(fps);
            let next_f = (cur_f + keys.step_frames).max(0);
            session.playhead = RationalTime::from_frame(next_f, fps);
            playback_action = TimelinePlaybackAction::ShuttlePause;
        }
        if let Some(shuttle_act) = keys.shuttle {
            playback_action = shuttle_act;
        }
        if keys.marker {
            self.add_marker_at_playhead(session);
        }
        if keys.select_all {
            self.select_all_in_scene(session);
        }
        if keys.copy {
            self.copy_selected(session);
        }
        if keys.paste {
            self.paste_clipboard_at_playhead(session);
        }
        if keys.duplicate {
            self.duplicate_selected(session);
        }
        if keys.group && !session.selected_items.is_empty() {
            let ids = session.selected_items.clone();
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetGroupId {
                    item_ids: ids,
                    group_id: Some(Uuid::new_v4()),
                },
                CommandSource::Gui,
                None,
            );
        }
        if keys.unlink && !session.selected_items.is_empty() {
            let ids = session.selected_items.clone();
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetGroupId {
                    item_ids: ids,
                    group_id: None,
                },
                CommandSource::Gui,
                None,
            );
        }

        // ===== 1. メイン編集ツールバー =====
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(session.bus.can_undo(), egui::Button::new("↶ Undo"))
                .clicked()
                || keys.undo
            {
                session.bus.undo(&mut session.project);
            }
            if ui
                .add_enabled(session.bus.can_redo(), egui::Button::new("↷ Redo"))
                .clicked()
                || keys.redo
            {
                session.bus.redo(&mut session.project);
            }
            ui.separator();

            if ui.button("✂ 分割 (S)").clicked() || keys.split {
                self.split_selected_at_playhead(session);
            }
            if ui
                .add_enabled(
                    !session.selected_items.is_empty(),
                    egui::Button::new("📋 複製 (Ctrl+D)"),
                )
                .clicked()
            {
                self.duplicate_selected(session);
            }
            if ui
                .add_enabled(
                    !session.selected_items.is_empty(),
                    egui::Button::new("🗑 削除"),
                )
                .clicked()
                || keys.delete
            {
                self.delete_selected(session);
            }
            if ui
                .add_enabled(
                    !session.selected_items.is_empty(),
                    egui::Button::new("⇤ リップル削除 (Shift+Del)"),
                )
                .clicked()
                || keys.ripple_delete
            {
                self.ripple_delete_selected(session);
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
            // v1.0 #19: グルーピング＆A/Vリンク解除・スリップ/ロール/スライドトリム
            if ui
                .add_enabled(
                    session.selected_items.len() >= 2,
                    egui::Button::new("🔗 グループ (Ctrl+G)"),
                )
                .on_hover_text("選択中の複数クリップ（映像＋音声等）をグループ化します")
                .clicked()
            {
                let ids = session.selected_items.clone();
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::SetGroupId {
                        item_ids: ids,
                        group_id: Some(Uuid::new_v4()),
                    },
                    CommandSource::Gui,
                    None,
                );
            }
            if ui
                .add_enabled(
                    !session.selected_items.is_empty(),
                    egui::Button::new("🔓 リンク解除"),
                )
                .on_hover_text("グループ／A-V リンクを解除します (Ctrl+Shift+G)")
                .clicked()
            {
                let ids = session.selected_items.clone();
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::SetGroupId {
                        item_ids: ids,
                        group_id: None,
                    },
                    CommandSource::Gui,
                    None,
                );
            }
            if let Some(first_id) = session.selected_items.first().copied() {
                ui.menu_button("✂ プロトリム", |ui| {
                    let step = RationalTime::from_seconds(0.1);
                    let neg_step = RationalTime::from_seconds(-0.1);
                    ui.label("スリップ (尺固定で素材開始点シフト)");
                    ui.horizontal(|ui| {
                        if ui.button("◀ -0.1s").clicked() {
                            let _ = session.bus.execute(
                                &mut session.project,
                                EditCommand::SlipTrim {
                                    item_id: first_id,
                                    delta: neg_step,
                                },
                                CommandSource::Gui,
                                None,
                            );
                        }
                        if ui.button("+0.1s ▶").clicked() {
                            let _ = session.bus.execute(
                                &mut session.project,
                                EditCommand::SlipTrim {
                                    item_id: first_id,
                                    delta: step,
                                },
                                CommandSource::Gui,
                                None,
                            );
                        }
                    });
                    let neighbors = session.project.active_scene().and_then(|s| {
                        for layer in &s.layers {
                            if layer.items.iter().any(|i| i.id == first_id) {
                                let mut sorted: Vec<(RationalTime, Uuid)> =
                                    layer.items.iter().map(|i| (i.start, i.id)).collect();
                                sorted.sort_by_key(|(st, _)| *st);
                                let pos = sorted.iter().position(|(_, id)| *id == first_id)?;
                                let left = pos.checked_sub(1).map(|p| sorted[p].1);
                                let right = sorted.get(pos + 1).map(|(_, id)| *id);
                                return Some((left, right));
                            }
                        }
                        None
                    });
                    if let Some((left_nb, right_nb)) = neighbors {
                        if let Some(r_id) = right_nb {
                            ui.separator();
                            ui.label("ロール (右隣接境界を同期移動)");
                            ui.horizontal(|ui| {
                                if ui.button("◀ -0.1s").clicked() {
                                    let _ = session.bus.execute(
                                        &mut session.project,
                                        EditCommand::RollTrim {
                                            left_item_id: first_id,
                                            right_item_id: r_id,
                                            delta: neg_step,
                                        },
                                        CommandSource::Gui,
                                        None,
                                    );
                                }
                                if ui.button("+0.1s ▶").clicked() {
                                    let _ = session.bus.execute(
                                        &mut session.project,
                                        EditCommand::RollTrim {
                                            left_item_id: first_id,
                                            right_item_id: r_id,
                                            delta: step,
                                        },
                                        CommandSource::Gui,
                                        None,
                                    );
                                }
                            });
                        }
                        ui.separator();
                        ui.label("スライド (前後尺を自動伸縮)");
                        ui.horizontal(|ui| {
                            if ui.button("◀ -0.1s").clicked() {
                                let _ = session.bus.execute(
                                    &mut session.project,
                                    EditCommand::SlideTrim {
                                        item_id: first_id,
                                        left_neighbor_id: left_nb,
                                        right_neighbor_id: right_nb,
                                        delta: neg_step,
                                    },
                                    CommandSource::Gui,
                                    None,
                                );
                            }
                            if ui.button("+0.1s ▶").clicked() {
                                let _ = session.bus.execute(
                                    &mut session.project,
                                    EditCommand::SlideTrim {
                                        item_id: first_id,
                                        left_neighbor_id: left_nb,
                                        right_neighbor_id: right_nb,
                                        delta: step,
                                    },
                                    CommandSource::Gui,
                                    None,
                                );
                            }
                        });
                    }
                });
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
            if ui
                .button("＋ スペクトラム")
                .on_hover_text("音声レイヤー連動のオーディオスペクトラムビジュアライザーを配置")
                .clicked()
            {
                let (scene_w, scene_h, audio_layer) = session
                    .project
                    .active_scene()
                    .map(|s| {
                        (
                            s.width as f32,
                            s.height as f32,
                            s.layers.len().saturating_sub(1),
                        )
                    })
                    .unwrap_or((1280.0, 720.0, 3));
                let mut item = TimelineItem::new(
                    "オーディオスペクトラム",
                    session.playhead,
                    RationalTime::from_seconds(5.0),
                    ItemKind::AudioSpectrum {
                        linked_audio_layer: audio_layer.min(3),
                        width: (scene_w * 0.60).round().clamp(160.0, 1400.0),
                        height: (scene_h * 0.22).round().clamp(64.0, 400.0),
                        bar_count: 24,
                        color: RgbaColor {
                            r: 0.16,
                            g: 0.82,
                            b: 1.0,
                            a: 0.95,
                        },
                        peak_color: RgbaColor {
                            r: 1.0,
                            g: 0.35,
                            b: 0.65,
                            a: 1.0,
                        },
                        sensitivity: 1.8,
                    },
                );
                item.transform.x = (scene_w * 0.20).round();
                item.transform.y = (scene_h * 0.68).round();
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
            if ui
                .button("＋ パーティクル")
                .on_hover_text(
                    "決定論的 2D/3D パーティクルエミッター（紙吹雪・雪・火花・集中線等）を配置",
                )
                .clicked()
            {
                let item = TimelineItem::new(
                    "✨ パーティクル (紙吹雪)",
                    session.playhead,
                    RationalTime::from_seconds(5.0),
                    ItemKind::ParticleEmitter {
                        config: ParticleConfig::default(),
                    },
                );
                let new_id = item.id;
                let target_layer = session
                    .project
                    .active_scene()
                    .map(|s| 1.min(s.layers.len().saturating_sub(1)))
                    .unwrap_or(0);
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: target_layer,
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
            if ui
                .button("＋ ベクター")
                .on_hover_text(
                    "自由ベジェベクターシェイプ（グラデーション塗り・破線・トリムパス対応）を配置",
                )
                .clicked()
            {
                let (cx, cy) = session
                    .project
                    .active_scene()
                    .map(|s| {
                        (
                            (s.width as f32 - 360.0) * 0.5,
                            (s.height as f32 - 360.0) * 0.5,
                        )
                    })
                    .unwrap_or((460.0, 180.0));
                let mut item = TimelineItem::new(
                    "✒ ベクターシェイプ",
                    session.playhead,
                    RationalTime::from_seconds(4.0),
                    ItemKind::VectorShape {
                        config: VectorShapeConfig::star(),
                    },
                );
                item.transform.x = cx.max(0.0);
                item.transform.y = cy.max(0.0);
                let new_id = item.id;
                let target_layer = session
                    .project
                    .active_scene()
                    .map(|s| 1.min(s.layers.len().saturating_sub(1)))
                    .unwrap_or(0);
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddObject {
                            layer_index: target_layer,
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
            ui.toggle_value(&mut self.show_jump_cut_bar, "⚡ 無音カット");
            ui.toggle_value(&mut self.show_beat_bar, "🥁 ビート検出");
            ui.toggle_value(&mut self.show_search_bar, "🔍 検索/置換 (Ctrl+F)");
            if ui
                .button("🚩 マーカー (M)")
                .on_hover_text("現在の再生ヘッド位置にチャプターマーカーを追加")
                .clicked()
            {
                self.add_marker_at_playhead(session);
            }
            let marker_count = session
                .project
                .active_scene()
                .map(|s| s.markers.len())
                .unwrap_or(0);
            if ui
                .add_enabled(
                    marker_count > 0,
                    egui::Button::new(format!("📋 YouTubeチャプター ({marker_count})")),
                )
                .clicked()
                && let Some(scene) = session.project.active_scene()
            {
                let text = format_youtube_chapters(&scene.markers);
                ui.ctx().copy_text(text);
                self.marker_status = format!("YouTubeチャプター {marker_count} 件をコピーしました");
            }
            ui.checkbox(&mut self.snap_enabled, "スナップ");
            ui.add(
                egui::Slider::new(&mut self.pixels_per_second, 30.0..=400.0).text("ズーム(px/s)"),
            );
        });

        // マーカーラベル・色クイック設定＆選択中マーカー編集（マーカー存在時または入力中・操作後）
        let marker_count = session
            .project
            .active_scene()
            .map(|s| s.markers.len())
            .unwrap_or(0);
        if marker_count > 0 || !self.marker_status.is_empty() || !self.marker_label_input.is_empty()
        {
            ui.horizontal_wrapped(|ui| {
                ui.label("🚩 次マーカー名:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.marker_label_input)
                        .hint_text("空欄時は「チャプター N」")
                        .desired_width(140.0),
                );
                for (idx, (name, _)) in MARKER_PALETTE.iter().enumerate() {
                    ui.selectable_value(&mut self.marker_color_idx, idx, *name);
                }
                ui.weak("（ルーラー上の旗: 左クリック=移動＆編集 / 右クリック=削除）");
                if !self.marker_status.is_empty() {
                    ui.colored_label(Color32::LIGHT_GREEN, &self.marker_status);
                }
            });

            if let Some(edit_id) = self.editing_marker_id {
                let marker_snap = session
                    .project
                    .active_scene()
                    .and_then(|s| s.markers.iter().find(|m| m.id == edit_id).cloned());
                if let Some(mut m) = marker_snap {
                    let prev_label = m.label.clone();
                    let prev_color = m.color;
                    let mut remove_now = false;
                    let mut close_now = false;
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(format!("✏ マーカー編集 ({:.2}s):", m.time.as_seconds()));
                        ui.add(
                            egui::TextEdit::singleline(&mut m.label)
                                .desired_width(160.0)
                                .hint_text("チャプター名"),
                        );
                        for (name, c) in MARKER_PALETTE {
                            if ui.selectable_label(m.color == *c, *name).clicked() {
                                m.color = *c;
                            }
                        }
                        if ui.small_button("🗑 削除").clicked() {
                            remove_now = true;
                        }
                        if ui.small_button("閉じる").clicked() {
                            close_now = true;
                        }
                    });
                    if remove_now {
                        let _ = session.bus.execute(
                            &mut session.project,
                            EditCommand::RemoveMarker { marker_id: edit_id },
                            CommandSource::Gui,
                            None,
                        );
                        self.editing_marker_id = None;
                        self.marker_status = "マーカーを削除しました".into();
                    } else if m.label != prev_label || m.color != prev_color {
                        let _ = session.bus.execute(
                            &mut session.project,
                            EditCommand::UpdateMarker {
                                marker_id: edit_id,
                                label: m.label,
                                color: m.color,
                            },
                            CommandSource::Gui,
                            Some("marker_edit"),
                        );
                    }
                    if close_now {
                        self.editing_marker_id = None;
                    }
                } else {
                    self.editing_marker_id = None;
                }
            }
        }

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
        if layer_count > 0 && self.jump_cut_audio_layer >= layer_count {
            self.jump_cut_audio_layer = layer_count - 1;
        }
        if layer_count > 0 && self.beat_audio_layer >= layer_count {
            self.beat_audio_layer = layer_count - 1;
        }

        // ===== 2B. 無音区間自動カット（ジェットカット）設定バー（v0.4 #3） =====
        if self.show_jump_cut_bar {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("⚡ 無音区間自動カット (Jump Cut):");
                    ui.label("解析音声:");
                    egui::ComboBox::from_id_salt("jump_cut_audio_layer")
                        .width(64.0)
                        .selected_text(format!("L{}", self.jump_cut_audio_layer))
                        .show_ui(ui, |ui| {
                            for idx in 0..layer_count {
                                ui.selectable_value(
                                    &mut self.jump_cut_audio_layer,
                                    idx,
                                    format!("L{idx}"),
                                );
                            }
                        });
                    ui.label("RMS閾値:");
                    ui.add(
                        egui::DragValue::new(&mut self.jump_cut_rms_threshold)
                            .speed(0.002)
                            .range(0.002..=0.30)
                            .fixed_decimals(3),
                    );
                    ui.label("最小無音:");
                    ui.add(
                        egui::DragValue::new(&mut self.jump_cut_min_silence_sec)
                            .speed(0.05)
                            .range(0.10..=3.0)
                            .suffix("s"),
                    );
                    ui.label("語頭余白:");
                    ui.add(
                        egui::DragValue::new(&mut self.jump_cut_pad_before_sec)
                            .speed(0.01)
                            .range(0.0..=0.5)
                            .suffix("s"),
                    );
                    ui.label("語尾余白:");
                    ui.add(
                        egui::DragValue::new(&mut self.jump_cut_pad_after_sec)
                            .speed(0.01)
                            .range(0.0..=0.5)
                            .suffix("s"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("連動リップルレイヤー:");
                    for idx in 0..layer_count {
                        let mut linked = self.jump_cut_linked_layers.contains(&idx)
                            || idx == self.jump_cut_audio_layer;
                        let enabled = idx != self.jump_cut_audio_layer;
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Checkbox::new(&mut linked, format!("L{idx}")),
                            )
                            .changed()
                        {
                            if linked {
                                self.jump_cut_linked_layers.insert(idx);
                            } else {
                                self.jump_cut_linked_layers.remove(&idx);
                            }
                        }
                    }
                    if ui
                        .button("✂ 無音を一括カット＆詰め (1ステップUndo)")
                        .clicked()
                    {
                        self.execute_jump_cut(session, waveforms);
                    }
                    if !self.jump_cut_status.is_empty() {
                        ui.colored_label(Color32::from_rgb(255, 224, 128), &self.jump_cut_status);
                    }
                });
            });
        }

        // ===== 2C. BGM ビート自動検出 ＆ ビートスナップ編集バー（v0.8 #12） =====
        if self.show_beat_bar {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("🥁 BGM ビート自動検出 (音ハメ編集):");
                    ui.label("参照:");
                    egui::ComboBox::from_id_salt("beat_audio_layer")
                        .width(64.0)
                        .selected_text(format!("L{}", self.beat_audio_layer))
                        .show_ui(ui, |ui| {
                            for idx in 0..layer_count {
                                ui.selectable_value(
                                    &mut self.beat_audio_layer,
                                    idx,
                                    format!("L{idx}"),
                                );
                            }
                        });
                    ui.label("BPM帯:");
                    ui.add(egui::DragValue::new(&mut self.beat_min_bpm).range(40.0..=180.0));
                    ui.label("〜");
                    ui.add(egui::DragValue::new(&mut self.beat_max_bpm).range(80.0..=240.0));
                    ui.label("拍子:");
                    ui.add(egui::DragValue::new(&mut self.beat_per_bar).range(1..=8));
                    if ui.button("🔍 BPM・ビート解析").clicked() {
                        self.execute_beat_detection(session, waveforms);
                    }
                    ui.checkbox(&mut self.beat_snap_enabled, "🧲 ビート吸着");
                });
                ui.horizontal_wrapped(|ui| {
                    let has_beats = !self.beat_grid.is_empty();
                    if self.detected_bpm > 0.0 {
                        ui.monospace(format!(
                            "{:.1} BPM ({}拍)",
                            self.detected_bpm,
                            self.beat_grid.len()
                        ));
                    }
                    if ui
                        .add_enabled(has_beats, egui::Button::new("🚩 小節マーカー配置").small())
                        .clicked()
                        && let Some(cmd) = build_beat_markers_command(&self.beat_grid, true)
                        && session
                            .bus
                            .execute(&mut session.project, cmd, CommandSource::Gui, None)
                            .is_ok()
                    {
                        self.beat_status = "小節頭マーカーを一括配置しました (Ctrl+Z可)".into();
                    }
                    if ui
                        .add_enabled(has_beats, egui::Button::new("🚩 全拍マーカー").small())
                        .clicked()
                        && let Some(cmd) = build_beat_markers_command(&self.beat_grid, false)
                        && session
                            .bus
                            .execute(&mut session.project, cmd, CommandSource::Gui, None)
                            .is_ok()
                    {
                        self.beat_status = "全ビートマーカーを一括配置しました (Ctrl+Z可)".into();
                    }
                    if ui
                        .add_enabled(
                            has_beats && has_sel,
                            egui::Button::new("✂ 選択クリップをビートでカット").small(),
                        )
                        .clicked()
                        && let Some(scene) = session.project.active_scene().cloned()
                    {
                        let beat_secs: Vec<f64> = self.beat_grid.iter().map(|(t, _)| *t).collect();
                        if let Some((cuts, cmd)) =
                            build_beat_cut_commands(&scene, &session.selected_items, &beat_secs)
                        {
                            if session
                                .bus
                                .execute(&mut session.project, cmd, CommandSource::Gui, None)
                                .is_ok()
                            {
                                self.beat_status = format!(
                                    "ビート位置で {cuts} 箇所を自動カットしました (Ctrl+Z可)"
                                );
                            }
                        } else {
                            self.beat_status =
                                "選択クリップ内にカット可能なビート位置がありません".into();
                        }
                    }
                    if ui
                        .add_enabled(
                            has_beats && has_sel,
                            egui::Button::new("📐 ビートへ整列").small(),
                        )
                        .clicked()
                        && let Some(scene) = session.project.active_scene().cloned()
                    {
                        let beat_secs: Vec<f64> = self.beat_grid.iter().map(|(t, _)| *t).collect();
                        if let Some(cmd) = build_snap_to_beats_commands(
                            &scene,
                            &session.selected_items,
                            &beat_secs,
                        ) && session
                            .bus
                            .execute(&mut session.project, cmd, CommandSource::Gui, None)
                            .is_ok()
                        {
                            self.beat_status = "選択クリップを最寄りビートへ整列しました".into();
                        }
                    }
                    if ui
                        .add_enabled(has_beats, egui::Button::new("🧹 グリッド消去").small())
                        .clicked()
                    {
                        self.beat_grid.clear();
                        self.detected_bpm = 0.0;
                        self.beat_status.clear();
                    }
                    if !self.beat_status.is_empty() {
                        ui.colored_label(Color32::from_rgb(132, 232, 255), &self.beat_status);
                    }
                });
            });
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

        // Shift + ホイールで横スクロール、Ctrl + ホイールで再生ヘッド中心のタイムラインズーム
        if response.hovered() {
            let (scroll_delta, ctrl_held) =
                ui.input(|i| (i.smooth_scroll_delta, i.modifiers.command));
            if ctrl_held && scroll_delta.y.abs() > 0.1 {
                let factor = if scroll_delta.y > 0.0 { 1.12 } else { 0.89 };
                let old_pps = self.pixels_per_second;
                let new_pps = (old_pps * factor).clamp(30.0, 400.0);
                let ph_sec = session.playhead.as_seconds();
                let ph_offset_px = (ph_sec - self.scroll_offset_sec) * f64::from(old_pps);
                let track_w = f64::from((canvas_rect.right() - track_origin_x).max(100.0));
                let target_px = if (0.0..=track_w).contains(&ph_offset_px) {
                    ph_offset_px
                } else {
                    track_w * 0.5
                };
                self.pixels_per_second = new_pps;
                self.scroll_offset_sec = (ph_sec - target_px / f64::from(new_pps)).max(0.0);
            } else if scroll_delta.x.abs() > 0.1 {
                self.scroll_offset_sec =
                    (self.scroll_offset_sec - f64::from(scroll_delta.x) / f64::from(pps)).max(0.0);
            }
        }
        let pps = self.pixels_per_second;
        let scroll_sec = self.scroll_offset_sec.max(0.0);

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

        // AE 風 RAM プレビュー先読みキャッシュ済みバー（ルーラー下端の緑色ライン）
        let fps_f = fps.as_f64().max(1.0);
        for &(f0, f1) in ram_cached_ranges {
            let s0 = f0 as f64 / fps_f;
            let s1 = (f1 + 1) as f64 / fps_f;
            let rx0 = (track_origin_x + (s0 - scroll_sec) as f32 * pps).max(track_origin_x);
            let rx1 = (track_origin_x + (s1 - scroll_sec) as f32 * pps).min(canvas_rect.right());
            if rx1 > rx0 {
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(rx0, ruler_rect.bottom() - 3.5),
                        Pos2::new(rx1, ruler_rect.bottom()),
                    ),
                    0.0,
                    Color32::from_rgb(72, 214, 118),
                );
            }
        }

        // v0.8 #12: 検出済み BGM ビートグリッド縦線（小節頭=ゴールド、拍=薄シアン）
        for &(b_sec, is_downbeat) in &self.beat_grid {
            let bx = track_origin_x + (b_sec - scroll_sec) as f32 * pps;
            if bx < track_origin_x || bx > canvas_rect.right() {
                continue;
            }
            let (col, width) = if is_downbeat {
                (Color32::from_rgba_unmultiplied(250, 200, 56, 115), 1.4)
            } else {
                (Color32::from_rgba_unmultiplied(72, 205, 248, 55), 1.0)
            };
            painter.line_segment(
                [
                    Pos2::new(bx, ruler_rect.bottom()),
                    Pos2::new(bx, canvas_rect.bottom()),
                ],
                Stroke::new(width, col),
            );
            if is_downbeat {
                painter.line_segment(
                    [
                        Pos2::new(bx, ruler_rect.bottom() - 7.0),
                        Pos2::new(bx, ruler_rect.bottom()),
                    ],
                    Stroke::new(1.8, Color32::from_rgb(250, 200, 56)),
                );
            }
        }

        // ルーラー上のタイムラインマーカー描画＆クリック操作（左クリック=ジャンプ＆編集、右クリック=削除）
        let mut jump_to_marker: Option<(Uuid, RationalTime)> = None;
        let mut remove_marker_id: Option<Uuid> = None;
        if let Some(scene) = session.project.active_scene() {
            for marker in &scene.markers {
                let mx = track_origin_x + (marker.time.as_seconds() - scroll_sec) as f32 * pps;
                if mx < track_origin_x - 4.0 || mx > canvas_rect.right() + 4.0 {
                    continue;
                }
                let mcol = rgba_to_color32(marker.color);
                let flag_rect = Rect::from_min_max(
                    Pos2::new(mx - 5.0, ruler_rect.top() + 1.0),
                    Pos2::new(
                        (mx + 64.0).min(canvas_rect.right()),
                        ruler_rect.bottom() - 2.0,
                    ),
                );
                // マーカー縦ガイドライン
                painter.line_segment(
                    [
                        Pos2::new(mx, ruler_rect.bottom()),
                        Pos2::new(mx, canvas_rect.bottom()),
                    ],
                    Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(mcol.r(), mcol.g(), mcol.b(), 110),
                    ),
                );
                // 旗ヘッド（三角＋ラベルピル）
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(mx, ruler_rect.top() + 2.0),
                        Pos2::new(mx + 6.0, ruler_rect.bottom() - 3.0),
                    ),
                    2.0,
                    mcol,
                );
                painter.text(
                    Pos2::new(mx + 8.0, ruler_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &marker.label,
                    egui::FontId::proportional(10.0),
                    mcol,
                );
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && flag_rect.contains(pos)
                {
                    jump_to_marker = Some((marker.id, marker.time));
                } else if response.secondary_clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && flag_rect.contains(pos)
                {
                    remove_marker_id = Some(marker.id);
                }
            }
        }

        // スナップ候補時刻の収集（再生ヘッド＋マーカー＋ビートグリッド＋ドラッグ対象外の全クリップ端）
        let dragged_ids: BTreeSet<ItemId> = match &self.dragging_item {
            Some(DragOp {
                item_id,
                mode: DragMode::Move { group_items, .. },
            }) => {
                let mut set: BTreeSet<ItemId> = group_items.iter().map(|(id, _, _)| *id).collect();
                set.insert(*item_id);
                set
            }
            Some(DragOp { item_id, .. }) => [*item_id].into_iter().collect(),
            None => BTreeSet::new(),
        };
        // v7.0 #48: 大規模タイムライン（64+ クリップ）は区間木 O(log N + K) で表示窓とスナップ候補を高速抽出
        let vis_span_sec = ((canvas_rect.right() - track_origin_x).max(1.0) / pps.max(1.0)) as f64;
        let vis_start_rt = RationalTime::from_seconds((scroll_sec - 0.5).max(0.0));
        let vis_end_rt = RationalTime::from_seconds(scroll_sec + vis_span_sec + 0.5);
        let snap_start_rt = RationalTime::from_seconds((scroll_sec - 2.0).max(0.0));
        let snap_end_rt = RationalTime::from_seconds(scroll_sec + vis_span_sec + 2.0);
        let rev_key = session.bus.revision().max(1);

        let mut snap_targets: Vec<f64> = vec![session.playhead.as_seconds()];
        if self.beat_snap_enabled {
            for &(b_sec, _) in &self.beat_grid {
                snap_targets.push(b_sec);
            }
        }
        let mut visible_indices_by_layer: Option<Vec<Vec<usize>>> = None;
        if let Some(scene) = session.project.active_scene() {
            for m in &scene.markers {
                snap_targets.push(m.time.as_seconds());
            }
            let total_scene_items: usize = scene.layers.iter().map(|l| l.items.len()).sum();
            if total_scene_items >= 64 {
                let idx = self.interval_index.get_or_build(scene, rev_key);
                for hit in idx.query_range(snap_start_rt, snap_end_rt) {
                    if let Some(item) = scene
                        .layers
                        .get(hit.layer_index)
                        .and_then(|l| l.items.get(hit.item_index))
                        && !dragged_ids.contains(&item.id)
                    {
                        snap_targets.push(item.start.as_seconds());
                        snap_targets.push(item.end().as_seconds());
                    }
                }
                let mut per_layer = Vec::with_capacity(scene.layers.len());
                for layer_idx in 0..scene.layers.len() {
                    let mut indices =
                        idx.query_layer_indices_in_range(layer_idx, vis_start_rt, vis_end_rt);
                    indices.sort_unstable();
                    per_layer.push(indices);
                }
                visible_indices_by_layer = Some(per_layer);
            } else {
                for layer in &scene.layers {
                    for item in &layer.items {
                        if !dragged_ids.contains(&item.id) {
                            snap_targets.push(item.start.as_seconds());
                            snap_targets.push(item.end().as_seconds());
                        }
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
        // ラバーバンド矩形選択用のクリップ矩形一覧
        let mut clip_rects: Vec<(ItemId, Rect)> = Vec::new();

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

                let culled_indices = visible_indices_by_layer
                    .as_ref()
                    .and_then(|v| v.get(layer_idx));
                let iter_len = culled_indices.map_or(layer.items.len(), Vec::len);
                for slot in 0..iter_len {
                    let Some(item) = culled_indices
                        .map_or_else(|| layer.items.get(slot), |idxs| layer.items.get(idxs[slot]))
                    else {
                        continue;
                    };
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
                    clip_rects.push((item.id, clip_rect));

                    let selected = session.selected_items.contains(&item.id);
                    let fill = clip_color_for_kind(&item.kind);
                    painter.rect_filled(clip_rect, 4.0, fill);

                    // 音声クリップの場合は波形ピークを描画
                    if let ItemKind::Audio { asset_id, .. } = &item.kind
                        && let Some(wf) = waveforms.get(asset_id)
                    {
                        draw_waveform(&painter, clip_rect, wf);
                    }

                    // v1.0 #19: ボリューム（黄色）・パン（シアン）オートメーションカーブをクリップ上へ直接描画
                    if let Some(vol_kfs) = item.keyframes.get("volume")
                        && !vol_kfs.is_empty()
                    {
                        let mut pts: Vec<Pos2> = vol_kfs
                            .iter()
                            .map(|kf| {
                                let kx = (x0 + kf.time.as_seconds() as f32 * pps)
                                    .clamp(clip_rect.left(), clip_rect.right());
                                let norm = (kf.value / 2.0).clamp(0.0, 1.0);
                                let ky = clip_rect.bottom() - norm * clip_rect.height();
                                Pos2::new(kx, ky)
                            })
                            .collect();
                        pts.sort_by(|a, b| a.x.total_cmp(&b.x));
                        if pts.len() >= 2 {
                            painter.add(egui::Shape::line(
                                pts.clone(),
                                Stroke::new(1.8, Color32::from_rgb(255, 224, 76)),
                            ));
                        }
                        for p in pts {
                            painter.circle_filled(p, 3.0, Color32::from_rgb(255, 224, 76));
                        }
                    }
                    if let Some(pan_kfs) = item.keyframes.get("pan")
                        && !pan_kfs.is_empty()
                    {
                        let mut pts: Vec<Pos2> = pan_kfs
                            .iter()
                            .map(|kf| {
                                let kx = (x0 + kf.time.as_seconds() as f32 * pps)
                                    .clamp(clip_rect.left(), clip_rect.right());
                                let norm = ((kf.value + 1.0) * 0.5).clamp(0.0, 1.0);
                                let ky = clip_rect.bottom() - norm * clip_rect.height();
                                Pos2::new(kx, ky)
                            })
                            .collect();
                        pts.sort_by(|a, b| a.x.total_cmp(&b.x));
                        if pts.len() >= 2 {
                            painter.add(egui::Shape::line(
                                pts.clone(),
                                Stroke::new(1.6, Color32::from_rgb(80, 225, 255)),
                            ));
                        }
                        for p in pts {
                            painter.circle_filled(p, 2.8, Color32::from_rgb(80, 225, 255));
                        }
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
                    if item.group_id.is_some() {
                        speed_tag.push_str(" 🔗");
                    }
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
                                anchor_layer: layer_idx,
                                anchor_start_sec: item.start.as_seconds(),
                                group_items: Vec::new(),
                            }
                        };
                        clicked_item = Some((item.id, mode));
                    }
                }
            }
        }

        // マーカー操作の反映
        if let Some(mid) = remove_marker_id {
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::RemoveMarker { marker_id: mid },
                CommandSource::Gui,
                None,
            );
            if self.editing_marker_id == Some(mid) {
                self.editing_marker_id = None;
            }
            self.marker_status = "マーカーを削除しました".into();
        } else if let Some((mid, mtime)) = jump_to_marker {
            session.playhead = mtime;
            self.editing_marker_id = Some(mid);
            return playback_action;
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
            return playback_action;
        }
        if let Some((pivot, dir)) = ctrl_dbl_select
            && let Some(scene) = session.project.active_scene()
        {
            session.selected_items = select_items_from_time(scene, pivot, dir, None);
            return playback_action;
        }

        if let Some(scene_id) = enter_precomp_scene {
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SwitchScene { scene_id },
                CommandSource::Gui,
                None,
            );
            session.selected_items.clear();
            return playback_action;
        }

        // ポインタ操作の処理（選択・複数クリップ同時ドラッグ移動・伸縮・ラバーバンド矩形選択・ルーラーシーク）
        if let Some((item_id, mut mode)) = clicked_item {
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
                // 複数選択中のドラッグ時は全選択アイテムの初期座標をスナップショット
                if let DragMode::Move {
                    ref mut group_items,
                    ..
                } = mode
                    && let Some(scene) = session.project.active_scene()
                {
                    let sel_set: BTreeSet<ItemId> =
                        session.selected_items.iter().copied().collect();
                    for (l_idx, layer) in scene.layers.iter().enumerate() {
                        for it in &layer.items {
                            if sel_set.contains(&it.id) {
                                group_items.push((it.id, l_idx, it.start.as_seconds()));
                            }
                        }
                    }
                }
                self.dragging_item = Some(DragOp { item_id, mode });
                session.bus.set_gui_drag_locked(true);
            }
        } else if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
            && pos.x >= track_origin_x
        {
            if pos.y < ruler_rect.bottom() {
                // ルーラー上ドラッグ: 再生ヘッドスクラブ
                let sec = (scroll_sec + ((pos.x - track_origin_x) / pps) as f64).max(0.0);
                session.playhead = RationalTime::from_seconds(sec);
            } else {
                // 空トラック上ドラッグ: ラバーバンド矩形選択開始
                self.marquee_origin = Some(pos);
            }
        } else if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && pos.x >= track_origin_x
        {
            let sec = (scroll_sec + ((pos.x - track_origin_x) / pps) as f64).max(0.0);
            session.playhead = RationalTime::from_seconds(sec);
        }

        // ラバーバンド矩形選択の更新＆描画
        if let Some(origin) = self.marquee_origin {
            if let Some(cur) = response.interact_pointer_pos() {
                let mrect = Rect::from_two_pos(origin, cur).intersect(canvas_rect);
                painter.rect_filled(
                    mrect,
                    2.0,
                    Color32::from_rgba_unmultiplied(72, 162, 255, 48),
                );
                painter.rect_stroke(
                    mrect,
                    2.0,
                    Stroke::new(1.2, Color32::from_rgb(108, 192, 255)),
                    StrokeKind::Inside,
                );
                if response.dragged() || response.drag_stopped() {
                    let mut picked: Vec<ItemId> = if shift_held {
                        session.selected_items.clone()
                    } else {
                        Vec::new()
                    };
                    for (cid, crect) in &clip_rects {
                        if crect.intersects(mrect) && !picked.contains(cid) {
                            picked.push(*cid);
                        }
                    }
                    session.selected_items = picked;
                }
            }
            if response.drag_stopped() {
                self.marquee_origin = None;
            }
        } else if self.dragging_item.is_none()
            && response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
            && pos.x >= track_origin_x
            && pos.y < ruler_rect.bottom()
        {
            let sec = (scroll_sec + ((pos.x - track_origin_x) / pps) as f64).max(0.0);
            session.playhead = RationalTime::from_seconds(sec);
        }

        if let Some(drag) = self.dragging_item.clone() {
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let raw_sec = (scroll_sec + ((pos.x - track_origin_x) / pps) as f64).max(0.0);
                let target_layer = (((pos.y - (canvas_rect.top() + ruler_h)) / track_h)
                    .floor()
                    .max(0.0) as usize)
                    .min(layer_count.saturating_sub(1));

                match drag.mode {
                    DragMode::Move {
                        grab_offset_sec,
                        anchor_layer,
                        anchor_start_sec,
                        group_items,
                    } => {
                        let mut cand_sec = (raw_sec - grab_offset_sec).max(0.0);
                        if self.snap_enabled {
                            cand_sec = snap_value(cand_sec, &snap_targets, 8.0 / pps as f64);
                        }
                        if group_items.len() <= 1 {
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
                        } else {
                            // 複数選択クリップの一括相対移動（最小時刻 >= 0、レイヤー範囲内へ安全クランプ）
                            let min_init_sec = group_items
                                .iter()
                                .map(|(_, _, s)| *s)
                                .fold(f64::INFINITY, f64::min);
                            let raw_delta_sec = cand_sec - anchor_start_sec;
                            let delta_sec = raw_delta_sec.max(-min_init_sec);

                            let min_init_l =
                                group_items.iter().map(|(_, l, _)| *l).min().unwrap_or(0) as isize;
                            let max_init_l =
                                group_items.iter().map(|(_, l, _)| *l).max().unwrap_or(0) as isize;
                            let max_layer_i = layer_count.saturating_sub(1) as isize;
                            let raw_delta_l = target_layer as isize - anchor_layer as isize;
                            let delta_l = raw_delta_l.clamp(-min_init_l, max_layer_i - max_init_l);

                            let cmds: Vec<EditCommand> = group_items
                                .iter()
                                .map(|(id, init_l, init_s)| EditCommand::MoveObject {
                                    item_id: *id,
                                    target_layer_index: (*init_l as isize + delta_l)
                                        .clamp(0, max_layer_i)
                                        as usize,
                                    new_start: RationalTime::from_seconds(
                                        (*init_s + delta_sec).max(0.0),
                                    ),
                                })
                                .collect();
                            let _ = session.bus.execute(
                                &mut session.project,
                                EditCommand::Batch {
                                    label: "timeline_move_multi".into(),
                                    commands: cmds,
                                },
                                CommandSource::Gui,
                                Some("timeline_move"),
                            );
                        }
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

        playback_action
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
                let step = (layer.items.len() / 1000).max(1);
                for item in layer.items.iter().step_by(step) {
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
            // ミニマップ上のマーカー表示
            for marker in &scene.markers {
                let mx =
                    bar_rect.left() + (marker.time.as_seconds() / span) as f32 * bar_rect.width();
                if mx >= bar_rect.left() && mx <= bar_rect.right() {
                    painter.line_segment(
                        [
                            Pos2::new(mx, bar_rect.top()),
                            Pos2::new(mx, bar_rect.top() + 6.0),
                        ],
                        Stroke::new(2.0, rgba_to_color32(marker.color)),
                    );
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

    fn ripple_delete_selected(&self, session: &mut EditorSession) {
        let Some(scene) = session.project.active_scene().cloned() else {
            return;
        };
        if let Some(cmd) = build_ripple_delete_commands(&scene, &session.selected_items)
            && session
                .bus
                .execute(&mut session.project, cmd, CommandSource::Gui, None)
                .is_ok()
        {
            session.selected_items.clear();
        }
    }

    fn select_all_in_scene(&self, session: &mut EditorSession) {
        if let Some(scene) = session.project.active_scene() {
            session.selected_items = scene
                .layers
                .iter()
                .flat_map(|l| l.items.iter().map(|i| i.id))
                .collect();
        }
    }

    fn copy_selected(&mut self, session: &EditorSession) {
        if session.selected_items.is_empty() {
            return;
        }
        let sel: BTreeSet<ItemId> = session.selected_items.iter().copied().collect();
        let mut copied = Vec::new();
        if let Some(scene) = session.project.active_scene() {
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                for item in &layer.items {
                    if sel.contains(&item.id) {
                        copied.push((l_idx, item.clone()));
                    }
                }
            }
        }
        if !copied.is_empty() {
            self.clipboard_items = copied;
        }
    }

    fn paste_clipboard_at_playhead(&self, session: &mut EditorSession) {
        if self.clipboard_items.is_empty() {
            return;
        }
        let Some(layer_count) = session.project.active_scene().map(|s| s.layers.len()) else {
            return;
        };
        if layer_count == 0 {
            return;
        }
        let min_start = self
            .clipboard_items
            .iter()
            .map(|(_, it)| it.start)
            .min()
            .unwrap_or(RationalTime::ZERO);
        let target_base = session.playhead.max(RationalTime::ZERO);

        let mut cmds = Vec::with_capacity(self.clipboard_items.len());
        let mut new_ids = Vec::with_capacity(self.clipboard_items.len());
        for (l_idx, src) in &self.clipboard_items {
            let rel = (src.start - min_start).max(RationalTime::ZERO);
            let item = clone_item_with_fresh_ids(src, target_base + rel);
            new_ids.push(item.id);
            cmds.push(EditCommand::AddObject {
                layer_index: (*l_idx).min(layer_count - 1),
                item,
            });
        }
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::Batch {
                    label: "paste_clips".into(),
                    commands: cmds,
                },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            session.selected_items = new_ids;
        }
    }

    fn duplicate_selected(&self, session: &mut EditorSession) {
        if session.selected_items.is_empty() {
            return;
        }
        let sel: BTreeSet<ItemId> = session.selected_items.iter().copied().collect();
        let mut selected_pairs: Vec<(usize, TimelineItem)> = Vec::new();
        let layer_count = if let Some(scene) = session.project.active_scene() {
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                for item in &layer.items {
                    if sel.contains(&item.id) {
                        selected_pairs.push((l_idx, item.clone()));
                    }
                }
            }
            scene.layers.len()
        } else {
            return;
        };
        if selected_pairs.is_empty() || layer_count == 0 {
            return;
        }

        let min_start = selected_pairs
            .iter()
            .map(|(_, it)| it.start)
            .min()
            .unwrap_or(RationalTime::ZERO);
        let max_end = selected_pairs
            .iter()
            .map(|(_, it)| it.end())
            .max()
            .unwrap_or(min_start + RationalTime::from_seconds(1.0));
        let span = (max_end - min_start).max(RationalTime::from_seconds(0.1));

        let mut cmds = Vec::with_capacity(selected_pairs.len());
        let mut new_ids = Vec::with_capacity(selected_pairs.len());
        for (l_idx, src) in &selected_pairs {
            let item = clone_item_with_fresh_ids(src, src.start + span);
            new_ids.push(item.id);
            cmds.push(EditCommand::AddObject {
                layer_index: (*l_idx).min(layer_count - 1),
                item,
            });
        }
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::Batch {
                    label: "duplicate_clips".into(),
                    commands: cmds,
                },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            session.selected_items = new_ids;
        }
    }

    fn add_marker_at_playhead(&mut self, session: &mut EditorSession) {
        let next_idx = session
            .project
            .active_scene()
            .map(|s| s.markers.len() + 1)
            .unwrap_or(1);
        let label = if self.marker_label_input.trim().is_empty() {
            format!("チャプター {next_idx}")
        } else {
            self.marker_label_input.trim().to_string()
        };
        let color = MARKER_PALETTE
            .get(self.marker_color_idx % MARKER_PALETTE.len())
            .map(|(_, c)| *c)
            .unwrap_or(RgbaColor::WHITE);
        let marker = TimelineMarker {
            id: Uuid::new_v4(),
            time: session.playhead,
            label: label.clone(),
            color,
        };
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::AddMarker { marker },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            self.marker_label_input.clear();
            self.marker_status = format!(
                "マーカー「{label}」を {:.2}s へ追加しました",
                session.playhead.as_seconds()
            );
        }
    }

    fn execute_jump_cut(
        &mut self,
        session: &mut EditorSession,
        waveforms: &BTreeMap<Uuid, WaveformPeaks>,
    ) {
        let Some(scene) = session.project.active_scene().cloned() else {
            self.jump_cut_status = "アクティブシーンがありません".into();
            return;
        };
        let total_sec = scene.total_duration().as_seconds();
        if total_sec <= 0.05 {
            self.jump_cut_status = "カット対象のクリップがありません".into();
            return;
        }
        let audio_layer = self
            .jump_cut_audio_layer
            .min(scene.layers.len().saturating_sub(1));

        // 1. 実メディアの AudioGraph から 50ms 単位の RMS 列を生成。
        //    実ファイルが無い場合（デモ波形など）は waveforms キャッシュへフォールバック。
        let sr = session.project.settings.audio_sample_rate.max(8_000);
        let audio_graph = AudioGraph::from_project_scene(
            &session.project,
            session.project.active_scene_id,
            session.folder.as_ref(),
            sr,
        );
        let bucket_sec = 0.05_f64;
        let bucket_count = (total_sec / bucket_sec).ceil().max(1.0) as usize;
        let mut rms_series = vec![0.0_f32; bucket_count];
        let mut has_audio_signal = false;

        if !audio_graph.clips.is_empty() {
            let pcm = audio_graph.render_range(RationalTime::ZERO, scene.total_duration());
            let frames_per_bucket = ((sr as f64) * bucket_sec).round().max(1.0) as usize;
            let stereo_frames = pcm.as_chunks::<2>().0;
            for (b_idx, rms_slot) in rms_series.iter_mut().enumerate() {
                let start_f = b_idx * frames_per_bucket;
                if start_f >= stereo_frames.len() {
                    break;
                }
                let end_f = ((b_idx + 1) * frames_per_bucket).min(stereo_frames.len());
                let slice = &stereo_frames[start_f..end_f];
                if slice.is_empty() {
                    continue;
                }
                let sum_sq: f64 = slice
                    .iter()
                    .map(|[l, r]| {
                        let mono = 0.5 * (f64::from(*l) + f64::from(*r));
                        mono * mono
                    })
                    .sum();
                *rms_slot = (sum_sq / slice.len() as f64).sqrt() as f32;
                if *rms_slot > 0.0 {
                    has_audio_signal = true;
                }
            }
        }

        if !has_audio_signal && let Some(layer) = scene.layers.get(audio_layer) {
            for item in &layer.items {
                if let ItemKind::Audio {
                    asset_id,
                    source_offset,
                    volume,
                } = &item.kind
                    && let Some(wf) = waveforms.get(asset_id)
                    && wf.sample_rate > 0
                    && wf.samples_per_bucket > 0
                {
                    let wf_bucket_sec = (wf.samples_per_bucket as f64) / f64::from(wf.sample_rate);
                    let start_s = item.start.as_seconds();
                    let end_s = item.end().as_seconds();
                    let src_off = source_offset.as_seconds();
                    for (b_idx, rms_slot) in rms_series.iter_mut().enumerate() {
                        let t = (b_idx as f64 + 0.5) * bucket_sec;
                        if t < start_s || t >= end_s {
                            continue;
                        }
                        let wf_idx = ((src_off + (t - start_s)) / wf_bucket_sec).floor() as usize;
                        if let Some(b) = wf.buckets.get(wf_idx) {
                            let v = b.rms * volume.max(0.0);
                            if v > *rms_slot {
                                *rms_slot = v;
                                has_audio_signal = true;
                            }
                        }
                    }
                }
            }
        }

        if !has_audio_signal {
            self.jump_cut_status = format!(
                "L{audio_layer} に解析可能な音声波形がありません（先に音声ファイルを配置してください）"
            );
            return;
        }

        let linked: Vec<usize> = self
            .jump_cut_linked_layers
            .iter()
            .copied()
            .filter(|&l| l < scene.layers.len())
            .collect();
        let cfg = JumpCutConfig {
            rms_threshold: self.jump_cut_rms_threshold,
            min_silence_sec: self.jump_cut_min_silence_sec,
            pad_before_sec: self.jump_cut_pad_before_sec,
            pad_after_sec: self.jump_cut_pad_after_sec,
            ripple_linked_layers: linked.clone(),
        };
        let intervals = detect_speech_intervals_from_rms(&rms_series, bucket_sec, total_sec, &cfg);
        if intervals.is_empty() {
            self.jump_cut_status =
                "しきい値以上の有音区間が検出されませんでした（RMS閾値を下げてください）".into();
            return;
        }
        match build_jump_cut_commands(&scene, audio_layer, &intervals, &linked) {
            Some((kept_count, removed_sec, cmd)) => {
                match session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None)
                {
                    Ok(_) => {
                        session.selected_items.clear();
                        self.jump_cut_status = format!(
                            "無音カット完了: 発話 {kept_count} 区間を保持 / 約 {removed_sec:.2} 秒を自動カット（Ctrl+Z で復元可）"
                        );
                    }
                    Err(e) => {
                        self.jump_cut_status = format!("無音カット実行エラー: {e}");
                    }
                }
            }
            None => {
                self.jump_cut_status = "カット可能な無音区間はありませんでした".into();
            }
        }
    }

    fn execute_beat_detection(
        &mut self,
        session: &EditorSession,
        waveforms: &BTreeMap<Uuid, WaveformPeaks>,
    ) {
        let Some(scene) = session.project.active_scene() else {
            self.beat_status = "アクティブシーンがありません".into();
            return;
        };
        let total_dur = scene.total_duration();
        if total_dur.as_seconds() < 0.5 {
            self.beat_status = "解析対象の音声クリップがありません".into();
            return;
        }
        let cfg = BeatDetectionConfig {
            min_bpm: self.beat_min_bpm,
            max_bpm: self.beat_max_bpm,
            sensitivity: self.beat_sensitivity,
            beats_per_bar: self.beat_per_bar.max(1),
        };
        let sr = session.project.settings.audio_sample_rate.max(8_000);
        let graph = AudioGraph::from_project_scene(
            &session.project,
            session.project.active_scene_id,
            session.folder.as_ref(),
            sr,
        );
        let mut res = if !graph.clips.is_empty() {
            let capped = total_dur.min(RationalTime::from_seconds(300.0));
            let pcm = graph.render_range(RationalTime::ZERO, capped);
            detect_audio_beats(&pcm, sr, &cfg)
        } else {
            ve_audio::BeatAnalysisResult {
                bpm: 0.0,
                confidence: 0.0,
                beats: Vec::new(),
                onset_times_sec: Vec::new(),
            }
        };

        // 実ファイル未ロード（デモ波形等）の場合は指定レイヤーの WaveformPeaks からフォールバック検出
        if res.beats.is_empty()
            && let Some(layer) = scene.layers.get(self.beat_audio_layer)
        {
            for item in &layer.items {
                if let ItemKind::Audio { asset_id, .. } = &item.kind
                    && let Some(wf) = waveforms.get(asset_id)
                {
                    let mut wf_res = detect_beats_from_waveform(wf, &cfg);
                    let offset = item.start.as_seconds();
                    for b in &mut wf_res.beats {
                        b.time_sec += offset;
                    }
                    if !wf_res.beats.is_empty() {
                        res = wf_res;
                        break;
                    }
                }
            }
        }

        if res.beats.is_empty() {
            self.beat_grid.clear();
            self.detected_bpm = 0.0;
            self.beat_status =
                "ビートを検出できませんでした（BGM/音声ファイルを配置してください）".into();
        } else {
            self.detected_bpm = res.bpm;
            self.beat_grid = res
                .beats
                .iter()
                .map(|b| (b.time_sec, b.is_downbeat))
                .collect();
            self.beat_status = format!(
                "検出完了: {:.1} BPM（{} 拍 / 信頼度 {:.0}%）",
                res.bpm,
                res.beats.len(),
                res.confidence * 100.0
            );
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
        ItemKind::AudioSpectrum { .. } => Color32::from_rgb(46, 178, 212),
        ItemKind::ParticleEmitter { .. } => Color32::from_rgb(218, 152, 42),
        ItemKind::VectorShape { .. } => Color32::from_rgb(172, 86, 214),
        ItemKind::SvgMorph { .. } => Color32::from_rgb(204, 92, 232),
        ItemKind::VtuberAvatar { .. } => Color32::from_rgb(242, 108, 178),
        ItemKind::FluidSim { .. } => Color32::from_rgb(248, 132, 46),
        ItemKind::ClothSim { .. } => Color32::from_rgb(198, 78, 118),
        ItemKind::Mesh3D { .. } => Color32::from_rgb(68, 146, 238),
        ItemKind::GenerativeBg { .. } => Color32::from_rgb(48, 186, 152),
        ItemKind::RiggedPuppet { .. } => Color32::from_rgb(232, 92, 148),
        ItemKind::DataChart { .. } => Color32::from_rgb(56, 198, 232),
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

fn clone_item_with_fresh_ids(src: &TimelineItem, new_start: RationalTime) -> TimelineItem {
    let mut item = src.clone();
    item.id = Uuid::new_v4();
    item.start = new_start.max(RationalTime::ZERO);
    for eff in &mut item.effect_chain {
        eff.id = Uuid::new_v4();
    }
    for eff in &mut item.audio_effects {
        eff.id = Uuid::new_v4();
    }
    for mask in &mut item.masks {
        mask.id = Uuid::new_v4();
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::{Project, ProjectSettings, ShapeKind};

    #[test]
    fn copy_paste_duplicate_and_select_all_preserve_offsets_and_undo() {
        let mut proj = Project::new("TlShortcutTest", ProjectSettings::default());
        let a = TimelineItem::new(
            "ClipA",
            RationalTime::from_seconds(1.0),
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 60.0,
                color: RgbaColor::WHITE,
            },
        );
        let b = TimelineItem::new(
            "ClipB",
            RationalTime::from_seconds(2.5),
            RationalTime::from_seconds(1.0),
            ItemKind::Text {
                content: "字幕".into(),
                font_family: "sans-serif".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        proj.active_scene_mut().unwrap().layers[0].items.push(a);
        proj.active_scene_mut().unwrap().layers[1].items.push(b);

        let mut sess = EditorSession::new(proj);
        let mut tl = TimelineState::default();

        tl.select_all_in_scene(&mut sess);
        assert_eq!(sess.selected_items.len(), 2);

        tl.copy_selected(&sess);
        assert_eq!(tl.clipboard_items.len(), 2);

        sess.playhead = RationalTime::from_seconds(10.0);
        tl.paste_clipboard_at_playhead(&mut sess);
        {
            let sc = sess.project.active_scene().unwrap();
            assert_eq!(sc.layers[0].items.len(), 2);
            assert_eq!(sc.layers[1].items.len(), 2);
            assert!((sc.layers[0].items[1].start.as_seconds() - 10.0).abs() < 1e-6);
            assert!((sc.layers[1].items[1].start.as_seconds() - 11.5).abs() < 1e-6);
            assert_ne!(sc.layers[0].items[0].id, sc.layers[0].items[1].id);
        }

        // Ctrl+D on newly pasted items duplicates them right after their combined end (12.5s)
        tl.duplicate_selected(&mut sess);
        {
            let sc = sess.project.active_scene().unwrap();
            assert_eq!(sc.layers[0].items.len(), 3);
            assert!((sc.layers[0].items[2].start.as_seconds() - 12.5).abs() < 1e-6);
        }

        assert!(sess.bus.undo(&mut sess.project));
        assert_eq!(
            sess.project.active_scene().unwrap().layers[0].items.len(),
            2
        );
    }
}
