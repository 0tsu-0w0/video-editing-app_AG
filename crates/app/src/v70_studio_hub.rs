//! v7.0 #45〜#48 / v8.0 #49〜#52 / v9.0 #53〜#56 / v10.0 #57〜#60 / v11.0 #61〜#64: 役割別 6 スタジオ Hub UX 統合 ＆ `Ctrl+K` ファジー検索コマンドパレット
//! ＋ GPU 4K/8K フレームプールパネル ＋ 10,000+ 区間木＆プロキシ⇔原本 TC コンフォームパネル
//! ＋ A/B スプリット比較ビューパネル ＋ MCP モジュールカタログ (104 ツール / 11 ドメイン) インスペクタ

use crate::color_scopes::ColorScopesState;
use crate::render_queue::RenderQueueState;
use crate::v09_panels::V09PanelsState;
use crate::v10_panels::V10PanelsState;
use crate::v20_panels::V20PanelsState;
use crate::v21_panels::V21PanelsState;
use crate::v30_panels::V30PanelsState;
use crate::v40_panels::V40PanelsState;
use crate::v50_panels::V50PanelsState;
use crate::v60_panels::V60PanelsState;
use crate::v80_panels::V80PanelsState;
use crate::v90_panels::V90PanelsState;
use crate::v100_panels::V100PanelsState;
use crate::v110_panels::V110PanelsState;
use egui::{Color32, Context, RichText, Ui};
use std::collections::BTreeMap;
use ve_core::{
    AbCompareMode, AbReferenceSource, AssetConformMeta, CommandSource, ConformDirection,
    ConformReport, EditCommand, IntervalIndexBenchmark, ItemId, Project, StudioHubId, UiLanguage,
    benchmark_interval_index, build_timeline_conform_command, commands_for_hub,
    fuzzy_search_commands, infer_is_proxy_name, normalize_reel_stem, plan_timeline_conform,
    studio_palette_catalog,
};
use ve_mcp::{EmbeddedMcpServer, McpDomainSummary, SharedEditorSession, tool_domain_catalog};
use ve_render::{
    FramePoolBenchmarkReport, GpuV70Status, LinearImage, RenderGraph, benchmark_v70_frame_pool,
    detect_gpu_v70_status,
};

/// 6 スタジオ Hub ウィンドウおよび `Ctrl+K` コマンドパレットから `EditorApp` へ返すアクション
#[derive(Debug, Default)]
pub struct StudioHubFrameOutcome {
    pub preview_dirty: bool,
    pub clear_ram_preview: bool,
    pub replaced_project: Option<Project>,
    pub switch_workspace_idx: Option<u8>, // 0=MainEdit, 1=MotionComposite, 2=NodeComposite
    pub open_render_queue: bool,
    pub toggle_scopes: bool,
    pub toggle_proxies: bool,
    pub open_scene_settings: bool,
    pub open_user_settings: bool,
    pub toggle_jump_cut_bar: bool,
    pub toggle_beat_bar: bool,
    pub toggle_search_bar: bool,
    pub trigger_undo: bool,
    pub trigger_redo: bool,
    pub trigger_save: bool,
    pub capture_still_a: bool,
}

/// v7.0 #45〜#48 統合スタジオ Hub ＆ `Ctrl+K` パレット状態
pub struct V70StudioHubState {
    /// 6 スタジオ Hub ウィンドウが開いているか
    pub show_hub_window: bool,
    /// 現在アクティブな制作ロール Hub
    pub active_hub: StudioHubId,
    /// 各 Hub で最後に開いていたサブツール command_id
    pub hub_active_tool: BTreeMap<StudioHubId, String>,

    // --- #45 `Ctrl+K` ファジー検索コマンドパレット ---
    pub show_palette: bool,
    pub palette_query: String,
    pub palette_selected_idx: usize,
    pub palette_focus_requested: bool,
    pub recent_command_ids: Vec<String>,

    // --- #46 MCP モジュールカタログ インスペクタ ---
    pub mcp_catalog_cache: Vec<McpDomainSummary>,
    pub mcp_search_query: String,
    pub mcp_selected_domain: Option<String>,
    pub mcp_copy_msg: String,

    // --- #47 GPU-Resident 4K/8K フレームプール ---
    pub gpu_v70_status: Option<GpuV70Status>,
    pub gpu_pool_budget_mib: u64,
    pub gpu_bench_preset: usize, // 0=1080p, 1=4K, 2=8K
    pub last_gpu_pool_bench: Option<FramePoolBenchmarkReport>,
    pub gpu_pool_status_msg: String,

    // --- #48 10,000+ 区間木 & プロキシ⇔原本 TC コンフォーム & A/B 比較 ---
    pub index_bench_clips: usize,
    pub last_index_bench: Option<IntervalIndexBenchmark>,
    pub conform_direction: ConformDirection,
    pub last_conform_report: Option<ConformReport>,
    pub conform_status_msg: String,
    pub frozen_still_a: Option<LinearImage>,
    pub ab_status_msg: String,
}

impl Default for V70StudioHubState {
    fn default() -> Self {
        let mut hub_active_tool = BTreeMap::new();
        for &hub in StudioHubId::ALL {
            if let Some(first) = commands_for_hub(hub).first() {
                hub_active_tool.insert(hub, first.id.to_string());
            }
        }
        Self {
            show_hub_window: false,
            active_hub: StudioHubId::Edit,
            hub_active_tool,
            show_palette: false,
            palette_query: String::new(),
            palette_selected_idx: 0,
            palette_focus_requested: false,
            recent_command_ids: Vec::new(),
            mcp_catalog_cache: tool_domain_catalog(),
            mcp_search_query: String::new(),
            mcp_selected_domain: None,
            mcp_copy_msg: String::new(),
            gpu_v70_status: None,
            gpu_pool_budget_mib: 512,
            gpu_bench_preset: 1, // 4K default
            last_gpu_pool_bench: None,
            gpu_pool_status_msg:
                "4K/8K フレームプールの VRAM/RAM 再利用と WGSL 高速パスを監視・検証できます".into(),
            index_bench_clips: 10_000,
            last_index_bench: None,
            conform_direction: ConformDirection::ProxyToOriginal,
            last_conform_report: None,
            conform_status_msg:
                "リール名＆SMPTE 開始タイムコードからプロキシ⇔カメラ原本を 1-Undo 自動コンフォーム"
                    .into(),
            frozen_still_a: None,
            ab_status_msg:
                "Before/After 色補正比較・静止画 A キャプチャ・差分強調ワイプを制御します".into(),
        }
    }
}

impl V70StudioHubState {
    pub fn open_hub(&mut self, hub: StudioHubId) {
        if self.show_hub_window && self.active_hub == hub {
            self.show_hub_window = false;
        } else {
            self.active_hub = hub;
            self.show_hub_window = true;
        }
    }

    pub fn open_hub_tool(&mut self, hub: StudioHubId, command_id: &str) {
        self.active_hub = hub;
        self.hub_active_tool.insert(hub, command_id.to_string());
        self.show_hub_window = true;
    }

    pub fn open_palette(&mut self) {
        self.show_palette = true;
        self.palette_query.clear();
        self.palette_selected_idx = 0;
        self.palette_focus_requested = true;
    }

    fn record_recent(&mut self, cmd_id: &str) {
        self.recent_command_ids.retain(|id| id != cmd_id);
        self.recent_command_ids.insert(0, cmd_id.to_string());
        self.recent_command_ids.truncate(12);
    }

    /// グローバルショートカット検知（`Ctrl+K` / `Ctrl+Shift+P` でコマンドパレット、`Alt+W` で A/B 比較トグル、`F1`/`F2`/`F3` で 3 空間切替）
    pub fn handle_global_shortcuts(
        &mut self,
        ctx: &Context,
        session: &SharedEditorSession,
        outcome: &mut StudioHubFrameOutcome,
    ) {
        let wants_kb = ctx.egui_wants_keyboard_input();
        let (open_palette_key, toggle_ab_key, ws_key) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            let shift = i.modifiers.shift;
            let alt = i.modifiers.alt;
            let pal = (cmd && !shift && i.key_pressed(egui::Key::K))
                || (cmd && shift && i.key_pressed(egui::Key::P));
            let ab = !wants_kb && alt && !cmd && i.key_pressed(egui::Key::W);
            let ws = if !wants_kb && !cmd && !alt {
                if i.key_pressed(egui::Key::F1) {
                    Some(0u8)
                } else if i.key_pressed(egui::Key::F2) {
                    Some(1u8)
                } else if i.key_pressed(egui::Key::F3) {
                    Some(2u8)
                } else {
                    None
                }
            } else {
                None
            };
            (pal, ab, ws)
        });

        if open_palette_key {
            if self.show_palette {
                self.show_palette = false;
            } else {
                self.open_palette();
            }
        }
        if let Some(w_idx) = ws_key {
            outcome.switch_workspace_idx = Some(w_idx);
        }
        if toggle_ab_key {
            self.toggle_ab_compare(session, outcome);
        }
    }

    pub fn toggle_ab_compare(
        &mut self,
        session: &SharedEditorSession,
        outcome: &mut StudioHubFrameOutcome,
    ) {
        if let Ok(mut g) = session.lock() {
            let mut cfg = g
                .project
                .active_scene()
                .map(|s| s.ab_compare.clone())
                .unwrap_or_default();
            cfg.mode = if cfg.is_active() {
                AbCompareMode::Off
            } else {
                AbCompareMode::VerticalSplit
            };
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAbCompare { ab_compare: cfg },
                    CommandSource::Gui,
                    None,
                )
                .is_ok()
            {
                outcome.preview_dirty = true;
            }
        }
    }

    /// コマンド ID を実行して対応する Hub / 空間 / クイックアクションへ遷移する
    pub fn dispatch_palette_command(
        &mut self,
        cmd_id: &str,
        session: &SharedEditorSession,
        outcome: &mut StudioHubFrameOutcome,
    ) {
        self.record_recent(cmd_id);
        self.show_palette = false;

        if let Some(entry) = studio_palette_catalog().iter().find(|e| e.id == cmd_id)
            && let Some(hub) = entry.hub
        {
            if cmd_id == "hub.deliver.render_queue" {
                outcome.open_render_queue = true;
            }
            self.open_hub_tool(hub, cmd_id);
            return;
        }

        match cmd_id {
            "workspace.main_edit" => outcome.switch_workspace_idx = Some(0),
            "workspace.motion_composite" => outcome.switch_workspace_idx = Some(1),
            "workspace.node_composite" => outcome.switch_workspace_idx = Some(2),
            "action.jump_cut_bar" => outcome.toggle_jump_cut_bar = true,
            "action.beat_detect_bar" => outcome.toggle_beat_bar = true,
            "action.search_replace_bar" => outcome.toggle_search_bar = true,
            "action.toggle_scopes" => outcome.toggle_scopes = true,
            "action.toggle_ab_split" => self.toggle_ab_compare(session, outcome),
            "action.toggle_proxies" => outcome.toggle_proxies = true,
            "action.clear_ram_cache" => {
                outcome.clear_ram_preview = true;
                outcome.preview_dirty = true;
            }
            "action.scene_settings" => outcome.open_scene_settings = true,
            "action.user_settings" => outcome.open_user_settings = true,
            "action.undo" => outcome.trigger_undo = true,
            "action.redo" => outcome.trigger_redo = true,
            "action.save_project" => outcome.trigger_save = true,
            _ => {}
        }
    }

    /// `Ctrl+K` モーダルコマンドパレットを描画する
    pub fn show_command_palette_modal(
        &mut self,
        ctx: &Context,
        lang: UiLanguage,
        session: &SharedEditorSession,
        outcome: &mut StudioHubFrameOutcome,
    ) {
        if !self.show_palette {
            return;
        }
        let hits = fuzzy_search_commands(&self.palette_query, &self.recent_command_ids, 14);
        if hits.is_empty() {
            self.palette_selected_idx = 0;
        } else if self.palette_selected_idx >= hits.len() {
            self.palette_selected_idx = hits.len() - 1;
        }

        let (esc, down, up, enter) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if esc {
            self.show_palette = false;
            return;
        }
        if down && !hits.is_empty() {
            self.palette_selected_idx = (self.palette_selected_idx + 1) % hits.len();
        }
        if up && !hits.is_empty() {
            self.palette_selected_idx = if self.palette_selected_idx == 0 {
                hits.len() - 1
            } else {
                self.palette_selected_idx - 1
            };
        }
        let mut execute_id: Option<String> = None;
        if enter && let Some(sel) = hits.get(self.palette_selected_idx) {
            execute_id = Some(sel.entry.id.to_string());
        }

        let mut open = self.show_palette;
        egui::Window::new("🔍 コマンドパレット (Ctrl+K / Ctrl+Shift+P)")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 56.0])
            .collapsible(false)
            .resizable(false)
            .fixed_size([640.0, 440.0])
            .show(ctx, |ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .desired_width(f32::INFINITY)
                        .hint_text(
                            "機能名・日本語・英語・#番号でファジー検索（例: ワイヤー消し / ACES / #48 / コンフォーム / BGM）...",
                        ),
                );
                if self.palette_focus_requested {
                    edit.request_focus();
                    self.palette_focus_requested = false;
                }
                if edit.changed() {
                    self.palette_selected_idx = 0;
                }
                ui.horizontal(|ui| {
                    ui.weak(format!(
                        "登録コマンド {} 件 | ↑↓ 選択 / Enter 実行 / Esc 閉じる",
                        studio_palette_catalog().len()
                    ));
                });
                ui.separator();

                egui::ScrollArea::vertical().max_height(355.0).show(ui, |ui| {
                    if hits.is_empty() {
                        ui.weak("一致するコマンドが見つかりません。別キーワードで検索してください。");
                    }
                    for (idx, hit) in hits.iter().enumerate() {
                        let is_sel = idx == self.palette_selected_idx;
                        let hub_badge = hit
                            .entry
                            .hub
                            .map(|h| format!("[{}]", h.button_label(lang)))
                            .unwrap_or_else(|| "[Core]".to_string());
                        let shortcut = hit
                            .entry
                            .shortcut_hint
                            .map(|s| format!("  <{s}>"))
                            .unwrap_or_default();
                        let label = format!(
                            "{}  {}  ({}){}",
                            hit.entry.title(lang),
                            hub_badge,
                            hit.entry.roadmap_tag,
                            shortcut
                        );
                        let resp = ui.selectable_label(is_sel, label);
                        if resp.clicked() {
                            execute_id = Some(hit.entry.id.to_string());
                        }
                    }
                });
            });

        if !open {
            self.show_palette = false;
        }
        if let Some(cid) = execute_id {
            self.dispatch_palette_command(&cid, session, outcome);
        }
    }

    /// 役割別 6 スタジオ Hub 統合ウィンドウを描画する（v0.9〜v9.0 全機能を 1 箇所からゼロ欠落アクセス）
    #[allow(clippy::too_many_arguments)]
    pub fn show_studio_hub_window(
        &mut self,
        ctx: &Context,
        lang: UiLanguage,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
        mcp_server: Option<&EmbeddedMcpServer>,
        render_graph: &mut RenderGraph,
        render_queue: &RenderQueueState,
        color_scopes: &mut ColorScopesState,
        v09_panels: &mut V09PanelsState,
        v10_panels: &mut V10PanelsState,
        v20_panels: &mut V20PanelsState,
        v21_panels: &mut V21PanelsState,
        v30_panels: &mut V30PanelsState,
        v40_panels: &mut V40PanelsState,
        v50_panels: &mut V50PanelsState,
        v60_panels: &mut V60PanelsState,
        v80_panels: &mut V80PanelsState,
        v90_panels: &mut V90PanelsState,
        v100_panels: &mut V100PanelsState,
        v110_panels: &mut V110PanelsState,
        outcome: &mut StudioHubFrameOutcome,
    ) {
        if !self.show_hub_window {
            return;
        }

        let mut open = self.show_hub_window;
        let title = format!(
            "🎛 Unified Studio Hub — {} (v11.0)",
            self.active_hub.button_label(lang)
        );
        egui::Window::new(title)
            .id(egui::Id::NULL.with("v70_unified_studio_hub_window"))
            .open(&mut open)
            .default_width(760.0)
            .default_height(650.0)
            .resizable(true)
            .show(ctx, |ui| {
                // 上段: 6 スタジオ Hub スイッチャー
                ui.horizontal_wrapped(|ui| {
                    for &hub in StudioHubId::ALL {
                        if ui
                            .selectable_label(self.active_hub == hub, hub.button_label(lang))
                            .on_hover_text(hub.summary(lang))
                            .clicked()
                        {
                            self.active_hub = hub;
                        }
                    }
                    ui.separator();
                    if ui.small_button("🔍 Ctrl+K 検索").clicked() {
                        self.open_palette();
                    }
                });
                ui.weak(self.active_hub.summary(lang));
                ui.separator();

                // 中段: 選択中 Hub に属する全ツールサブタブ（機能ゼロ欠落）
                let hub_cmds = commands_for_hub(self.active_hub);
                let active_cmd_id = self
                    .hub_active_tool
                    .entry(self.active_hub)
                    .or_insert_with(|| {
                        hub_cmds
                            .first()
                            .map(|c| c.id.to_string())
                            .unwrap_or_default()
                    })
                    .clone();
                let mut next_cmd_id = active_cmd_id.clone();

                ui.horizontal_wrapped(|ui| {
                    for entry in &hub_cmds {
                        let short_label = format!("{} ({})", entry.title(lang), entry.roadmap_tag);
                        if ui
                            .selectable_label(active_cmd_id == entry.id, short_label)
                            .clicked()
                        {
                            next_cmd_id = entry.id.to_string();
                        }
                    }
                });
                if next_cmd_id != active_cmd_id {
                    self.hub_active_tool
                        .insert(self.active_hub, next_cmd_id.clone());
                }
                ui.separator();

                // 下段: アクティブツールの本体パネルをインライン描画
                egui::ScrollArea::vertical().show(ui, |ui| {
                    match next_cmd_id.as_str() {
                        // ===== Edit Hub =====
                        "hub.edit.multicam" => {
                            let mut sess = session.lock().unwrap();
                            if v10_panels.show_multicam_tab(ui, &mut sess) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.edit.speed_ramp" => {
                            if v50_panels.show_speed_ramp_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.edit.kinetic_sub" => {
                            if v21_panels.show_kinetic_localize_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.edit.templates" => {
                            let mut sess = session.lock().unwrap();
                            let act = v09_panels.show_template_gallery_ui(ui, &mut sess);
                            if let Some(proj) = act.replaced_project {
                                outcome.replaced_project = Some(proj);
                            }
                            if act.dirty {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.edit.conform_index" => {
                            self.show_interval_index_and_conform_ui(ui, session, outcome);
                        }
                        "hub.edit.smart_dam" => {
                            if v90_panels.ui_smart_dam_tab(ui, session) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.edit.adaptive_prerender" => {
                            if v110_panels.ui_adaptive_prerender_tab(ui, session) {
                                outcome.preview_dirty = true;
                            }
                        }

                        // ===== VFX & Motion Hub =====
                        "hub.vfx.stabilize_pin" => {
                            if v21_panels.show_vfx_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.paint_inpaint" => {
                            if v60_panels.ui_vfx_paint_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.scene3d" => {
                            if v30_panels.show_scene3d_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.rig2d_ik" => {
                            if v50_panels.show_rig_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.physics2d" => {
                            if v40_panels.show_physics_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.data_chart" => {
                            if v60_panels.ui_dataviz_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.generative_bg_foley" => {
                            if v40_panels.show_generative_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.lighting_mblur" => {
                            let mut sess = session.lock().unwrap();
                            if V09PanelsState::show_scene_lighting_and_motion_blur_ui(ui, &mut sess)
                            {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.gpu_pool" => {
                            self.show_gpu_v70_frame_pool_ui(ui, render_graph);
                            ui.separator();
                            ui.collapsing(
                                "⚡ v3.0 GPU カラー＆原子ヒストグラム ベンチマーク (#29)",
                                |ui| {
                                    v30_panels.show_gpu_tab(ui);
                                },
                            );
                        }
                        "hub.vfx.planar_roto" => {
                            if v80_panels.ui_planar_roto_tab(
                                ui,
                                session,
                                selected_item,
                                render_graph,
                            ) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.motion_foundry" => {
                            if v80_panels.ui_motion_foundry_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.optical_vfx" => {
                            if v80_panels.ui_optical_vfx_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.vtuber_mesh" => {
                            if v90_panels.ui_vtuber_mesh_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.spatial_360" => {
                            if v100_panels.ui_spatial_360_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.fluid_cloth_ibl" => {
                            if v100_panels.ui_fluid_cloth_ibl_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.mocap_retarget" => {
                            if v100_panels.ui_mocap_retarget_tab(
                                ui,
                                session,
                                selected_item,
                                render_graph,
                            ) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.vfx.gpu_graph_v110" => {
                            if v110_panels.ui_gpu_render_graph_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }

                        // ===== Color Hub =====
                        "hub.color.primary_lut" => {
                            ui.horizontal(|ui| {
                                if ui
                                    .toggle_value(
                                        &mut color_scopes.show_scopes,
                                        "📊 リアルタイムビデオスコープ表示",
                                    )
                                    .changed()
                                    && color_scopes.show_scopes
                                {
                                    outcome.preview_dirty = true;
                                }
                            });
                            let mut sess = session.lock().unwrap();
                            if color_scopes.show_color_grading_ui(ui, &mut sess) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.color.curves_hsl_match" => {
                            if v21_panels.show_pro_color_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.color.aces_ocio" => {
                            if v40_panels.show_aces_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.color.cinema_finishing" => {
                            if v50_panels.show_finishing_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.color.ab_compare" => {
                            self.show_ab_compare_ui(ui, session, outcome);
                        }

                        // ===== Audio Hub =====
                        "hub.audio.mixer_nr_stems" => {
                            let mut sess = session.lock().unwrap();
                            if v10_panels.show_audio_mixer_tab(ui, &mut sess) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.audio.spatial_master" => {
                            if v21_panels.show_spatial_master_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.audio.bgm_composer" => {
                            if v50_panels.show_bgm_tab(ui, session) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.audio.offline_dubbing" => {
                            if v30_panels.show_dubbing_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.audio.podcast_director" => {
                            if v60_panels.ui_podcast_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.audio.restoration_adr" => {
                            if v80_panels.ui_audio_restoration_adr_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }

                        // ===== AI & Collab Hub =====
                        "hub.ai.local_vision" => {
                            let mut sess = session.lock().unwrap();
                            if v20_panels.ui_ai_vision_tab(ui, &mut sess) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.ai.director_recipe" => {
                            if v20_panels.ui_director_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.ai.collab_diff_review" => {
                            let mut sess = session.lock().unwrap();
                            if v20_panels.ui_collab_review_tab(ui, &mut sess) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.ai.crdt_lan" => {
                            if v30_panels.show_crdt_tab(ui, session, selected_item) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.ai.mcp_inspector" => {
                            self.show_mcp_catalog_inspector_ui(ui, mcp_server);
                        }
                        "hub.ai.hardware_ctrl" => {
                            if v90_panels.ui_hardware_ctrl_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.ai.cinema_producer" => {
                            if v100_panels.ui_cinema_producer_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }

                        // ===== Deliver Hub =====
                        "hub.deliver.profiler_hud" => {
                            if v110_panels.ui_profiler_hud_tab(ui, session, render_graph) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.deliver.memory_fastbin" => {
                            if v110_panels.ui_memory_fastbin_tab(ui, session, render_graph) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.deliver.render_farm" => {
                            if v100_panels.ui_render_farm_tab(ui, session) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.deliver.live_switcher" => {
                            if v90_panels.ui_live_switcher_tab(ui, session) {
                                outcome.preview_dirty = true;
                                outcome.clear_ram_preview = true;
                            }
                        }
                        "hub.deliver.multi_packager" => {
                            if v60_panels.ui_multi_deliver_tab(ui, session) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.deliver.broadcast_qc_html" => {
                            v40_panels.show_qc_tab(ui, session);
                        }
                        "hub.deliver.edl_fcpxml" => {
                            if v50_panels.show_finishing_tab(ui, session) {
                                outcome.preview_dirty = true;
                            }
                        }
                        "hub.deliver.validate_zip" => {
                            let mut sess = session.lock().unwrap();
                            v10_panels.show_delivery_zip_tab(ui, &mut sess);
                        }
                        "hub.deliver.render_queue" => {
                            ui.heading("📋 バッチ連続書き出しレンダーキュー (#7)");
                            ui.label(format!(
                                "登録ジョブ数: {} 件 | 実行中: {}",
                                render_queue.jobs.len(),
                                if render_queue.running {
                                    "はい"
                                } else {
                                    "待機中"
                                }
                            ));
                            if ui.button("🚀 レンダーキュー専用ウィンドウを開く").clicked()
                            {
                                outcome.open_render_queue = true;
                            }
                        }
                        _ => {
                            ui.weak("左上のツールタブから機能を選択してください。");
                        }
                    }
                });
            });

        if !open {
            self.show_hub_window = false;
        }
    }

    // =========================================================================
    // #46: 内蔵 MCP モジュール分割カタログ インスペクタ (82 Tools / 7 Domains)
    // =========================================================================
    fn show_mcp_catalog_inspector_ui(
        &mut self,
        ui: &mut Ui,
        mcp_server: Option<&EmbeddedMcpServer>,
    ) {
        let total: usize = self.mcp_catalog_cache.iter().map(|d| d.tool_count).sum();
        ui.heading(format!(
            "🔌 内蔵 MCP ドメイン別ツールカタログ（全 {total} ツール / {} モジュール） (#46)",
            self.mcp_catalog_cache.len()
        ));

        if let Some(mcp) = mcp_server {
            ui.horizontal_wrapped(|ui| {
                ui.monospace(mcp.endpoint_url());
                if ui.button("📋 URL コピー").clicked() {
                    ui.ctx().copy_text(mcp.endpoint_url());
                    self.mcp_copy_msg = "MCP Endpoint URL をコピーしました".into();
                }
                if ui.button("🔑 Bearer トークン").clicked() {
                    ui.ctx().copy_text(mcp.bearer_token().to_string());
                    self.mcp_copy_msg = "Bearer トークンをコピーしました".into();
                }
                if ui.button("⚙ クライアント設定 JSON").clicked() {
                    ui.ctx().copy_text(mcp.client_config_json());
                    self.mcp_copy_msg = "MCP クライアント設定 JSON をコピーしました".into();
                }
            });
            if !self.mcp_copy_msg.is_empty() {
                ui.colored_label(Color32::LIGHT_GREEN, &self.mcp_copy_msg);
            }
        }
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(
                    self.mcp_selected_domain.is_none(),
                    format!("すべて ({total})"),
                )
                .clicked()
            {
                self.mcp_selected_domain = None;
            }
            for dom in &self.mcp_catalog_cache {
                let selected = self.mcp_selected_domain.as_deref() == Some(dom.domain_id.as_str());
                if ui
                    .selectable_label(selected, format!("{} ({})", dom.domain_id, dom.tool_count))
                    .on_hover_text(&dom.label)
                    .clicked()
                {
                    self.mcp_selected_domain = Some(dom.domain_id.clone());
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("フィルタ:");
            ui.add(
                egui::TextEdit::singleline(&mut self.mcp_search_query)
                    .hint_text("ツール名検索 (例: conform, gpu, podcast, aces)..."),
            );
        });
        ui.separator();

        let q = self.mcp_search_query.trim().to_lowercase();
        for dom in &self.mcp_catalog_cache {
            if let Some(ref filter) = self.mcp_selected_domain
                && &dom.domain_id != filter
            {
                continue;
            }
            let matched: Vec<&String> = dom
                .tool_names
                .iter()
                .filter(|n| q.is_empty() || n.to_lowercase().contains(&q))
                .collect();
            if matched.is_empty() {
                continue;
            }
            ui.strong(format!(
                "📁 crates/mcp/src/tools/{}.rs — {} ({} 件)",
                dom.domain_id,
                dom.label,
                matched.len()
            ));
            ui.horizontal_wrapped(|ui| {
                for name in matched {
                    ui.monospace(name);
                }
            });
            ui.add_space(4.0);
        }
    }

    // =========================================================================
    // #47: GPU-Resident 4K/8K 合成フレームプール ＆ WGSL 高速パス UI
    // =========================================================================
    fn show_gpu_v70_frame_pool_ui(&mut self, ui: &mut Ui, render_graph: &mut RenderGraph) {
        ui.heading(
            "⚡ #47 GPU-Resident 4K/8K フレームプール ＆ WGSL ブレンド/マスク/色管理高速パス",
        );

        let status = self
            .gpu_v70_status
            .get_or_insert_with(detect_gpu_v70_status)
            .clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "アダプタ: {} ({})",
                status.adapter_name, status.backend_name
            ));
            ui.colored_label(
                if status.blend_mask_shader_valid {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::LIGHT_RED
                },
                format!(
                    "Blend/Mask WGSL: {}",
                    if status.blend_mask_shader_valid {
                        "OK"
                    } else {
                        "ERR"
                    }
                ),
            );
            ui.colored_label(
                if status.aces_color_shader_valid {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::LIGHT_RED
                },
                format!(
                    "ACEScg WGSL: {}",
                    if status.aces_color_shader_valid {
                        "OK"
                    } else {
                        "ERR"
                    }
                ),
            );
        });

        let metrics = render_graph.frame_pool.metrics(0);
        ui.group(|ui| {
            ui.strong("ライブ RenderGraph フレームプール統計");
            ui.label(format!(
                "再利用ヒット率: {:.1}% (新規確保 {} / 再利用 {} / LRU Evict {})",
                metrics.hit_rate() * 100.0,
                metrics.allocations_created,
                metrics.pool_reuses,
                metrics.evictions
            ));
            ui.label(format!(
                "常駐使用量: {:.1} MiB / 予算 {:.0} MiB (ピーク {:.1} MiB | アイドル {} 枚)",
                metrics.resident_bytes as f64 / (1024.0 * 1024.0),
                metrics.budget_bytes as f64 / (1024.0 * 1024.0),
                metrics.peak_resident_bytes as f64 / (1024.0 * 1024.0),
                metrics.idle_cached
            ));
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Slider::new(&mut self.gpu_pool_budget_mib, 64..=4096)
                            .text("プール上限 (MiB)"),
                    )
                    .changed()
                {
                    render_graph
                        .frame_pool
                        .set_budget_mib(self.gpu_pool_budget_mib);
                }
                if ui.button("🧹 プール解放").clicked() {
                    render_graph.frame_pool.clear();
                    self.gpu_pool_status_msg = "常駐フレームプールを解放しました".into();
                }
            });
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("ベンチ解像度:");
            ui.selectable_value(&mut self.gpu_bench_preset, 0, "1080p");
            ui.selectable_value(&mut self.gpu_bench_preset, 1, "4K (3840×2160)");
            ui.selectable_value(&mut self.gpu_bench_preset, 2, "8K (7680×4320)");
            if ui
                .button("🚀 プール＆WGSL 高速パス ベンチマーク実行")
                .clicked()
            {
                let (w, h, iters, budget) = match self.gpu_bench_preset {
                    0 => (1920, 1080, 4, 512),
                    2 => (7680, 4320, 2, 1536),
                    _ => (3840, 2160, 3, 768),
                };
                let rep = benchmark_v70_frame_pool(w, h, iters, budget);
                self.gpu_pool_status_msg = format!(
                    "完了: {}×{} ({}) | {:.1} ms ({:.1} fps) | 再利用ヒット率 {:.1}%",
                    rep.width,
                    rep.height,
                    rep.backend_name,
                    rep.elapsed_ms,
                    rep.estimated_fps,
                    rep.pool_metrics.hit_rate() * 100.0
                );
                self.last_gpu_pool_bench = Some(rep);
            }
        });
        if let Some(ref rep) = self.last_gpu_pool_bench {
            ui.monospace(format!(
                "Backend={} | GPU={} | {}x{} x{} -> {:.2} ms ({:.1} fps) | Reuses={}/{} | GpuBufs={}",
                rep.backend_name,
                rep.executed_on_gpu,
                rep.width,
                rep.height,
                rep.iterations,
                rep.elapsed_ms,
                rep.estimated_fps,
                rep.pool_metrics.pool_reuses,
                rep.pool_metrics.allocations_created + rep.pool_metrics.pool_reuses,
                rep.pool_metrics.gpu_resident_buffers
            ));
        }
        ui.weak(&self.gpu_pool_status_msg);

        ui.separator();
        ui.collapsing(
            "🚀 v11.0 WGSL コンピュート（ゼロコピー連結・360°・2D 流体）＆ 全 9 シェーダー Naga 検証",
            |ui| {
                static CACHED_CHECKS: std::sync::OnceLock<Vec<ve_render::WgslValidationSummary>> =
                    std::sync::OnceLock::new();
                let checks = CACHED_CHECKS
                    .get_or_init(ve_render::validate_all_builtin_wgsl_shaders);
                if ui
                    .button("⚡ 全 9 WGSL シェーダー検証 ＋ 360° GPU/CPU ベンチ実行")
                    .clicked()
                {
                    let rep = ve_render::benchmark_v100_gpu_or_cpu(640, 360);
                    let valid_cnt = checks.iter().filter(|c| c.valid).count();
                    self.gpu_pool_status_msg = format!(
                        "v10.0 WGSL 検証: {}/{} valid (all_ok={}) | {} ({}) | 360° OverCapture 640×360 = {:.2} ms (GPU={})",
                        valid_cnt,
                        rep.validated_shader_count,
                        rep.all_shaders_valid,
                        rep.adapter_name,
                        rep.backend_name,
                        rep.spatial_360_ms,
                        rep.used_gpu
                    );
                }
                for s in checks {
                    let icon = if s.valid { "✅" } else { "❌" };
                    ui.monospace(format!("{icon} [{}] {}", s.milestone, s.name));
                }
            },
        );
    }

    // =========================================================================
    // #48 (前半): 10,000+ 区間木 O(log N) ＆ プロキシ⇔原本 TC コンフォーム UI
    // =========================================================================
    fn show_interval_index_and_conform_ui(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        outcome: &mut StudioHubFrameOutcome,
    ) {
        ui.heading("🌲 #48 大規模タイムライン区間木 O(log N) ＆ プロキシ⇔原本 TC 自動コンフォーム");

        // --- Part 1: 10,000+ クリップ区間木ベンチマーク ---
        ui.group(|ui| {
            ui.strong("1. 10,000+ クリップ Augmented Interval Tree 性能検証");
            let active_clips: usize = session
                .lock()
                .ok()
                .and_then(|s| {
                    s.project
                        .active_scene()
                        .map(|sc| sc.layers.iter().map(|l| l.items.len()).sum())
                })
                .unwrap_or(0);
            ui.label(format!(
                "現在シーンのクリップ数: {active_clips} 件（64 件以上で RenderGraph ＆ Timeline が自動的に $O(\\log N)$ 区間木検索へ切替）"
            ));
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.index_bench_clips)
                        .range(1_000..=30_000)
                        .speed(500)
                        .suffix(" クリップ"),
                );
                if ui.button("⚡ 区間木 vs 線形走査 ベンチマーク実行").clicked() {
                    self.last_index_bench = Some(benchmark_interval_index(self.index_bench_clips, 100));
                }
            });
            if let Some(ref rep) = self.last_index_bench {
                ui.colored_label(
                    if rep.exact_parity_verified {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::LIGHT_RED
                    },
                    format!(
                        "{} クリップ ({} レイヤー) | 構築 {:.2} ms | 区間木 {:.1} µs vs 線形 {:.1} µs ({:.1}x 高速) | 平均訪問 {:.1} ノード | 100%一致={}",
                        rep.clip_count,
                        rep.layer_count,
                        rep.build_ms,
                        rep.tree_query_total_us,
                        rep.linear_scan_total_us,
                        rep.speedup_factor,
                        rep.avg_visited_nodes,
                        rep.exact_parity_verified
                    ),
                );
            }
        });

        ui.add_space(6.0);

        // --- Part 2: プロキシ ⇔ カメラ原本 SMPTE タイムコード自動コンフォーム ---
        ui.group(|ui| {
            ui.strong("2. プロキシ ⇔ カメラ原本 SMPTE タイムコード自動コンフォーム");
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(
                    &mut self.conform_direction,
                    ConformDirection::ProxyToOriginal,
                    ConformDirection::ProxyToOriginal.display_name(),
                );
                ui.selectable_value(
                    &mut self.conform_direction,
                    ConformDirection::OriginalToProxy,
                    ConformDirection::OriginalToProxy.display_name(),
                );
            });

            // アセットごとのリール名・開始 TC・プロキシ種別のインライン編集テーブル
            let asset_rows: Vec<(ve_core::AssetId, String, AssetConformMeta)> = {
                let g = session.lock().unwrap();
                g.project
                    .assets
                    .iter()
                    .map(|a| {
                        let meta = g
                            .project
                            .conform_meta
                            .get(&a.id)
                            .cloned()
                            .unwrap_or_else(|| AssetConformMeta {
                                reel_name: Some(normalize_reel_stem(&a.name)),
                                start_timecode: Some("00:00:00:00".into()),
                                is_proxy_media: infer_is_proxy_name(&a.name),
                                linked_counterpart_id: None,
                            });
                        (a.id, a.name.clone(), meta)
                    })
                    .collect()
            };

            if asset_rows.is_empty() {
                ui.weak("プロジェクトにメディアアセットがありません（動画/音声を D&D 取込後にコンフォーム可能です）。");
            } else {
                egui::Grid::new("v70_conform_asset_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("アセット名");
                        ui.strong("リール名");
                        ui.strong("開始 TC");
                        ui.strong("プロキシ");
                        ui.end_row();

                        for (aid, name, mut meta) in asset_rows {
                            let before = meta.clone();
                            let mut reel_str = meta
                                .reel_name
                                .clone()
                                .unwrap_or_else(|| normalize_reel_stem(&name));
                            let mut tc_str = meta
                                .start_timecode
                                .clone()
                                .unwrap_or_else(|| "00:00:00:00".into());

                            ui.label(&name);
                            ui.add(egui::TextEdit::singleline(&mut reel_str).desired_width(110.0));
                            ui.add(egui::TextEdit::singleline(&mut tc_str).desired_width(92.0));
                            ui.checkbox(&mut meta.is_proxy_media, "Proxy");
                            ui.end_row();

                            meta.reel_name = if reel_str.trim().is_empty() {
                                None
                            } else {
                                Some(reel_str.trim().to_string())
                            };
                            meta.start_timecode = if tc_str.trim().is_empty() {
                                None
                            } else {
                                Some(tc_str.trim().to_string())
                            };
                            if meta != before {
                                let mut g = session.lock().unwrap();
                                let s = &mut *g;
                                let _ = s.bus.execute(
                                    &mut s.project,
                                    EditCommand::SetAssetConformMeta {
                                        asset_id: aid,
                                        meta,
                                    },
                                    CommandSource::Gui,
                                    Some("conform_meta_edit"),
                                );
                            }
                        }
                    });
            }

            ui.horizontal(|ui| {
                if ui.button("🔍 コンフォーム計画をプレビュー (Dry-Run)").clicked() {
                    let g = session.lock().unwrap();
                    let rep = plan_timeline_conform(
                        &g.project,
                        g.project.active_scene_id,
                        self.conform_direction,
                    );
                    self.conform_status_msg = format!(
                        "プレビュー: 対象 {} クリップ / 警告 {} 件 / 既に一致 {} 件",
                        rep.actions.len(),
                        rep.issues.len(),
                        rep.skipped_already_target
                    );
                    self.last_conform_report = Some(rep);
                }
                if ui.button("✅ 1-Undo 自動コンフォームを実行").clicked() {
                    let mut g = session.lock().unwrap();
                    let rep = plan_timeline_conform(
                        &g.project,
                        g.project.active_scene_id,
                        self.conform_direction,
                    );
                    if let Some(cmd) = build_timeline_conform_command(&rep) {
                        let s = &mut *g;
                        if s.bus
                            .execute(&mut s.project, cmd, CommandSource::Gui, None)
                            .is_ok()
                        {
                            outcome.preview_dirty = true;
                            outcome.clear_ram_preview = true;
                            self.conform_status_msg = format!(
                                "コンフォーム完了: {} クリップの参照アセット＆TCオフセットを更新しました (Ctrl+Z で元に戻せます)",
                                rep.actions.len()
                            );
                        }
                    } else {
                        self.conform_status_msg =
                            "切替対象のクリップが見つかりませんでした（リール名・プロキシ種別を確認してください）"
                                .into();
                    }
                    self.last_conform_report = Some(rep);
                }
            });

            if let Some(ref rep) = self.last_conform_report {
                for act in &rep.actions {
                    ui.colored_label(
                        Color32::LIGHT_GREEN,
                        format!(
                            "✔ L{} '{}' : {} → {} | TC In {} | Offset {:.2}s → {:.2}s",
                            act.layer_index,
                            act.item_name,
                            act.from_asset_name,
                            act.to_asset_name,
                            act.clip_source_tc_in,
                            act.old_source_offset.as_seconds(),
                            act.new_source_offset.as_seconds(),
                        ),
                    );
                }
                for iss in &rep.issues {
                    ui.colored_label(
                        Color32::YELLOW,
                        format!("⚠ '{}': {}", iss.item_name, iss.message),
                    );
                }
            }
            ui.weak(&self.conform_status_msg);
        });
    }

    // =========================================================================
    // #48 (後半): A/B スプリット比較ビュー設定 UI
    // =========================================================================
    fn show_ab_compare_ui(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        outcome: &mut StudioHubFrameOutcome,
    ) {
        ui.heading("🌓 #48 A/B スプリット比較ビュー（プレビュー上ドラッグワイプ対応 / Alt+W）");

        let (mut cfg, scene_choices, cur_playhead) = {
            let g = session.lock().unwrap();
            let cfg = g
                .project
                .active_scene()
                .map(|s| s.ab_compare.clone())
                .unwrap_or_default();
            let scenes: Vec<(ve_core::SceneId, String)> = g
                .project
                .scenes
                .iter()
                .map(|s| (s.id, s.name.clone()))
                .collect();
            (cfg, scenes, g.playhead.as_seconds())
        };
        let before = cfg.clone();

        ui.horizontal_wrapped(|ui| {
            ui.label("表示モード:");
            for &m in AbCompareMode::ALL {
                ui.selectable_value(&mut cfg.mode, m, m.display_name());
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("A 側比較ソース:");
            for &src in AbReferenceSource::ALL {
                ui.selectable_value(&mut cfg.reference, src, src.display_name());
            }
        });

        ui.add(
            egui::Slider::new(&mut cfg.split_ratio, 0.05..=0.95)
                .text("スプリット位置 (プレビュー上でも直接ドラッグ可能)"),
        );
        if cfg.mode == AbCompareMode::DifferenceAmplified {
            ui.add(
                egui::Slider::new(&mut cfg.diff_gain, 1.0..=16.0).text("ピクセル差分増幅ゲイン"),
            );
        }
        if cfg.reference == AbReferenceSource::ReferenceTimecode {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut cfg.reference_sec)
                        .range(0.0..=86_400.0)
                        .speed(0.1)
                        .suffix(" 秒"),
                );
                if ui.button("現在再生ヘッド秒をセット").clicked() {
                    cfg.reference_sec = cur_playhead;
                }
            });
        }
        if cfg.reference == AbReferenceSource::AlternateScene {
            let cur_lbl = cfg
                .reference_scene_id
                .and_then(|id| scene_choices.iter().find(|(sid, _)| *sid == id))
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "比較シーンを選択".into());
            egui::ComboBox::from_label("比較対象シーン")
                .selected_text(cur_lbl)
                .show_ui(ui, |ui| {
                    for (sid, sname) in &scene_choices {
                        ui.selectable_value(&mut cfg.reference_scene_id, Some(*sid), sname);
                    }
                });
        }

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut cfg.show_divider, "分割境界線＆A/B バッジを表示");
            ui.checkbox(&mut cfg.swap_sides, "A / B を左右（上下）反転");
            if ui
                .button("📸 現在フレームを静止画 A へキャプチャ")
                .clicked()
            {
                cfg.reference = AbReferenceSource::FrozenStill;
                if !cfg.is_active() {
                    cfg.mode = AbCompareMode::VerticalSplit;
                }
                outcome.capture_still_a = true;
                self.ab_status_msg =
                    format!("現在時刻 ({cur_playhead:.2}s) を静止画 A へキャプチャしました");
            }
            if self.frozen_still_a.is_some() && ui.button("🗑 静止画 A をクリア").clicked()
            {
                self.frozen_still_a = None;
                outcome.preview_dirty = true;
            }
        });

        if cfg != before {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetAbCompare { ab_compare: cfg },
                    CommandSource::Gui,
                    Some("ab_compare_ui"),
                )
                .is_ok()
            {
                outcome.preview_dirty = true;
            }
        }

        let still_badge = match self.frozen_still_a.as_ref() {
            Some(img) => format!("静止画 A 保持中 ({}×{})", img.width, img.height),
            None => "静止画 A 未キャプチャ".into(),
        };
        ui.label(RichText::new(format!("{} | {}", self.ab_status_msg, still_badge)).weak());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studio_hub_state_maps_all_six_hubs_and_104_mcp_tools() {
        let mut hub = V70StudioHubState::default();
        assert_eq!(hub.hub_active_tool.len(), 6);
        let total_mcp: usize = hub.mcp_catalog_cache.iter().map(|d| d.tool_count).sum();
        assert_eq!(total_mcp, 104);
        assert_eq!(hub.mcp_catalog_cache.len(), 11);

        let catalog = studio_palette_catalog();
        for id in [
            "hub.vfx.gpu_graph_v110",
            "hub.edit.adaptive_prerender",
            "hub.deliver.memory_fastbin",
            "hub.deliver.profiler_hud",
        ] {
            assert!(
                catalog.iter().any(|e| e.id == id),
                "missing v11.0 palette entry: {id}"
            );
        }

        hub.open_hub(StudioHubId::Color);
        assert!(hub.show_hub_window);
        assert_eq!(hub.active_hub, StudioHubId::Color);
        // Clicking the same active hub toggles it closed
        hub.open_hub(StudioHubId::Color);
        assert!(!hub.show_hub_window);
    }
}
