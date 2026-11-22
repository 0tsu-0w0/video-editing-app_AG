use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use std::collections::BTreeMap;
use uuid::Uuid;
use ve_core::{
    ApprovalGateStatus, BlendMode, ChannelSelector, CommandSource, CompositeNode,
    CompositeNodeKind, ConflictResolution, DiffChangeKind, EditCommand, EffectNode,
    NodeCompositeGraph, NodeId, ProjectDiff, ReviewComment, ReviewSource, ReviewStatus, RgbaColor,
    ThreeWayMergeResult, TrackMatteMode, build_review_comments_to_markers_commands,
    builtin_edit_recipes, diff_projects, export_review_comments_csv, format_smpte_timecode,
    import_review_comments_csv, import_review_comments_json, import_youtube_review_comments,
    merge_projects_three_way, parse_recipe_json_or_yaml, record_recipe_from_commands,
    serialize_recipe_yaml, sync_node_graph_to_scene_layers,
};
use ve_mcp::{
    EditorSession, SharedEditorSession, advance_recipe_step, approve_current_gate,
    reject_current_gate, run_recipe_until_gate, start_recipe_run,
};
use ve_render::{
    AutoReframeConfig, ReframeSample, RenderGraph, analyze_scene_auto_reframe,
    build_auto_reframe_commands, detect_local_vision_status,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierTab {
    AiVision,
    Director,
    CollabReview,
}

/// v2.0 #21〜#24 統合パネル状態（第3ワークスペース「ノード合成空間」＋ローカルAIビジョン＋AI編集ディレクター＋共同編集/レビュー）
pub struct V20PanelsState {
    // --- #21 Node Composite Workspace ---
    pub selected_node: Option<NodeId>,
    pub pending_wire_from: Option<(NodeId, String)>,
    pub node_pan: Vec2,
    pub node_zoom: f32,
    pub dragging_node: Option<NodeId>,
    pub node_status: String,

    // --- Frontier Studio Window (#22 / #23 / #24) ---
    pub show_frontier_window: bool,
    pub active_tab: FrontierTab,

    // #22 Local AI Vision
    pub seg_sensitivity: f32,
    pub seg_feather_px: f32,
    pub seg_skin_boost: f32,
    pub seg_despill: f32,
    pub seg_invert: bool,
    pub reframe_target_w: u32,
    pub reframe_target_h: u32,
    pub reframe_smoothing: f32,
    pub reframe_samples: Vec<ReframeSample>,
    pub sr_sharpen: f32,
    pub sr_denoise: f32,
    pub slowmo_speed: f32,
    pub ai_status: String,

    // #23 AI Editing Director
    pub selected_recipe_idx: usize,
    pub recipe_yaml_editor: String,
    pub recipe_var_title: String,
    pub recipe_var_subtitle: String,
    pub record_recipe_name: String,
    pub director_status: String,

    // #24 Collab Diff / 3-Way Merge / Review Comments
    pub snapshot_name_input: String,
    pub diff_base_snapshot: String,
    pub diff_target_snapshot: String,
    pub last_diff: Option<ProjectDiff>,
    pub merge_base_snapshot: String,
    pub merge_theirs_snapshot: String,
    pub merge_resolutions: BTreeMap<String, ConflictResolution>,
    pub last_merge: Option<ThreeWayMergeResult>,
    pub review_author: String,
    pub review_body_input: String,
    pub review_import_format: usize, // 0=Frame.io CSV, 1=Frame.io JSON, 2=YouTube Text
    pub review_import_text: String,
    pub review_export_preview: String,
    pub collab_status: String,
}

impl Default for V20PanelsState {
    fn default() -> Self {
        let first_yaml = builtin_edit_recipes()
            .first()
            .and_then(|r| serialize_recipe_yaml(r).ok())
            .unwrap_or_default();
        Self {
            selected_node: None,
            pending_wire_from: None,
            node_pan: Vec2::new(24.0, 24.0),
            node_zoom: 1.0,
            dragging_node: None,
            node_status: "ノード合成空間: 「レイヤーから生成」で既存タイムラインをノード配線へ展開できます".into(),
            show_frontier_window: false,
            active_tab: FrontierTab::AiVision,
            seg_sensitivity: 0.55,
            seg_feather_px: 1.5,
            seg_skin_boost: 0.65,
            seg_despill: 0.35,
            seg_invert: false,
            reframe_target_w: 1080,
            reframe_target_h: 1920,
            reframe_smoothing: 0.72,
            reframe_samples: Vec::new(),
            sr_sharpen: 0.55,
            sr_denoise: 0.2,
            slowmo_speed: 0.5,
            ai_status: String::new(),
            selected_recipe_idx: 0,
            recipe_yaml_editor: first_yaml,
            recipe_var_title: "v2.0 AI ディレクター自動構成".into(),
            recipe_var_subtitle: "承認ゲート＆視覚自己検査で安全に量産".into(),
            record_recipe_name: "MyMacroRecipe".into(),
            director_status: String::new(),
            snapshot_name_input: "main_v1".into(),
            diff_base_snapshot: String::new(),
            diff_target_snapshot: "(現在のプロジェクト)".into(),
            last_diff: None,
            merge_base_snapshot: String::new(),
            merge_theirs_snapshot: String::new(),
            merge_resolutions: BTreeMap::new(),
            last_merge: None,
            review_author: "Director".into(),
            review_body_input: String::new(),
            review_import_format: 0,
            review_import_text:
                "Timecode In,Timecode Out,Commenter,Comment,Status\n00:00:01:00,00:00:03:00,Reviewer,テロップ位置を少し上へ,Open\n"
                    .into(),
            review_export_preview: String::new(),
            collab_status: String::new(),
        }
    }
}

impl V20PanelsState {
    // =========================================================================
    // #21 第 3 ワークスペース: Fusion / Nuke スタイル ノード合成空間ペイン
    // =========================================================================

    /// ノード合成グラフエディタを描画し、プロジェクトに変更があれば `true` を返す
    pub fn show_node_graph_pane(&mut self, ui: &mut egui::Ui, sess: &mut EditorSession) -> bool {
        let mut changed = false;
        let Some(mut graph) = sess.project.active_scene().map(|s| s.node_graph.clone()) else {
            ui.label("アクティブシーンがありません");
            return false;
        };

        // 上部ツールバー
        let mut synced_to_layers = false;
        ui.horizontal_wrapped(|ui| {
            let mut en = graph.enabled;
            if ui
                .checkbox(&mut en, "⚡ ノード合成を有効化 (RenderGraph 出力切替)")
                .changed()
            {
                graph.enabled = en;
                if graph.nodes.is_empty()
                    && en
                    && let Some(scene) = sess.project.active_scene()
                {
                    graph = NodeCompositeGraph::from_scene_layers(scene);
                }
                changed = true;
            }
            ui.separator();
            if ui
                .button("⬇ レイヤーからノード生成")
                .on_hover_text(
                    "現在のタイムラインレイヤー構成を Fusion / Nuke スタイルのノード DAG へ変換",
                )
                .clicked()
                && let Some(scene) = sess.project.active_scene()
            {
                graph = NodeCompositeGraph::from_scene_layers(scene);
                self.selected_node = graph.output_node_id();
                self.pending_wire_from = None;
                self.node_status = format!(
                    "レイヤーから {} ノード / {} 接続を生成しました",
                    graph.nodes.len(),
                    graph.connections.len()
                );
                changed = true;
            }
            if ui
                .button("⬆ ノード変更をレイヤーへ反映")
                .on_hover_text(
                    "Transform / Effect / MaskCombine / Merge の変更をレイヤー編集空間へ双方向同期",
                )
                .clicked()
            {
                let mut next_proj = sess.project.clone();
                if let Some(scene) = next_proj.active_scene_mut() {
                    let sum = sync_node_graph_to_scene_layers(&graph, scene);
                    if sess
                        .bus
                        .execute(
                            &mut sess.project,
                            EditCommand::ReplaceProjectSnapshot {
                                project: Box::new(next_proj),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.node_status = format!(
                            "レイヤー同期完了: Offset {} / Effect {} / Matte {} / Blend {} (高度ノード {} 件は維持)",
                            sum.updated_layer_offsets,
                            sum.synced_effects,
                            sum.updated_track_mattes,
                            sum.updated_blend_modes,
                            sum.preserved_advanced_nodes
                        );
                        synced_to_layers = true;
                    }
                }
            }
            if ui.button("📐 自動整列").clicked() {
                graph.auto_layout();
                changed = true;
            }
            ui.separator();
            if ui.small_button("−").clicked() {
                self.node_zoom = (self.node_zoom - 0.15).clamp(0.45, 2.0);
            }
            ui.label(format!("{:.0}%", self.node_zoom * 100.0));
            if ui.small_button("＋").clicked() {
                self.node_zoom = (self.node_zoom + 0.15).clamp(0.45, 2.0);
            }
            if ui.small_button("視点リセット").clicked() {
                self.node_pan = Vec2::new(24.0, 24.0);
                self.node_zoom = 1.0;
            }
        });
        if synced_to_layers {
            return true;
        }

        // ノード追加パレット
        ui.horizontal_wrapped(|ui| {
            ui.weak("ノード追加:");
            let spawn_pos = [
                180.0 + (graph.nodes.len() as f32 % 5.0) * 42.0,
                90.0 + (graph.nodes.len() as f32 % 4.0) * 36.0,
            ];
            let needs_output = graph.output_node_id().is_none();
            let mut push_node = |name: &str, kind: CompositeNodeKind| {
                let id = graph.add_node(CompositeNode::new(name, spawn_pos, kind));
                self.selected_node = Some(id);
                self.node_status = format!("ノード追加: {name}");
                changed = true;
            };
            if ui.small_button("+ LayerInput").clicked() {
                push_node(
                    "LayerInput",
                    CompositeNodeKind::LayerInput { layer_index: 0 },
                );
            }
            if ui.small_button("+ Solid").clicked() {
                push_node(
                    "SolidColor",
                    CompositeNodeKind::SolidColor {
                        color: RgbaColor {
                            r: 0.18,
                            g: 0.45,
                            b: 0.85,
                            a: 1.0,
                        },
                    },
                );
            }
            if ui.small_button("+ Effect").clicked() {
                let mut params = BTreeMap::new();
                params.insert("radius".into(), 3.0);
                push_node(
                    "Effect (Blur)",
                    CompositeNodeKind::Effect {
                        effect_type: "blur".into(),
                        params,
                    },
                );
            }
            if ui.small_button("+ ChannelSplit").clicked() {
                push_node(
                    "ChannelSplit",
                    CompositeNodeKind::ChannelSplit {
                        channel: ChannelSelector::Luma,
                    },
                );
            }
            if ui.small_button("+ ChannelMerge").clicked() {
                push_node("ChannelMerge", CompositeNodeKind::ChannelMerge);
            }
            if ui.small_button("+ MaskCombine").clicked() {
                push_node(
                    "MaskCombine",
                    CompositeNodeKind::MaskCombine {
                        mode: TrackMatteMode::Alpha,
                    },
                );
            }
            if ui.small_button("+ Merge").clicked() {
                push_node(
                    "Merge",
                    CompositeNodeKind::Merge {
                        blend_mode: BlendMode::Normal,
                        opacity: 1.0,
                    },
                );
            }
            if ui.small_button("+ Transform").clicked() {
                push_node(
                    "Transform",
                    CompositeNodeKind::Transform {
                        x: 0.0,
                        y: 0.0,
                        scale_x: 1.0,
                        scale_y: 1.0,
                        rotation_deg: 0.0,
                        opacity: 1.0,
                    },
                );
            }
            if ui.small_button("+ ColorCorrect").clicked() {
                push_node(
                    "ColorCorrect",
                    CompositeNodeKind::ColorCorrect {
                        exposure: 0.0,
                        contrast: 1.0,
                        saturation: 1.0,
                    },
                );
            }
            if ui.small_button("+ DepthFog").clicked() {
                push_node(
                    "DepthFog (3D)",
                    CompositeNodeKind::DepthFog {
                        near: 400.0,
                        far: 2000.0,
                        fog_color: RgbaColor {
                            r: 0.72,
                            g: 0.82,
                            b: 0.96,
                            a: 1.0,
                        },
                        strength: 0.45,
                    },
                );
            }
            if needs_output && ui.small_button("+ Output").clicked() {
                push_node("MediaOut", CompositeNodeKind::Output);
            }
        });

        if !self.node_status.is_empty() {
            ui.weak(&self.node_status);
        }
        ui.separator();

        // 左: インタラクティブノードキャンバス / 右: 選択ノード詳細インスペクタ
        let avail = ui.available_size();
        let inspector_w = 240.0_f32.min(avail.x * 0.34);
        let canvas_w = (avail.x - inspector_w - 12.0).max(240.0);
        let panel_h = avail.y.max(240.0);

        ui.horizontal_top(|ui| {
            // --- ノードキャンバス ---
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(canvas_w, panel_h), egui::Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            // 背景＆グリッド描画
            painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 23, 29));
            let grid_step = (28.0 * self.node_zoom).max(12.0);
            let mut gx = rect.left() + (self.node_pan.x % grid_step);
            while gx < rect.right() {
                painter.line_segment(
                    [Pos2::new(gx, rect.top()), Pos2::new(gx, rect.bottom())],
                    Stroke::new(1.0, Color32::from_rgb(30, 34, 42)),
                );
                gx += grid_step;
            }
            let mut gy = rect.top() + (self.node_pan.y % grid_step);
            while gy < rect.bottom() {
                painter.line_segment(
                    [Pos2::new(rect.left(), gy), Pos2::new(rect.right(), gy)],
                    Stroke::new(1.0, Color32::from_rgb(30, 34, 42)),
                );
                gy += grid_step;
            }

            // 中ドラッグ／右ドラッグでパン
            if response.dragged_by(egui::PointerButton::Middle)
                || response.dragged_by(egui::PointerButton::Secondary)
            {
                self.node_pan += response.drag_delta();
            }

            let to_screen = |p: [f32; 2]| -> Pos2 {
                Pos2::new(
                    rect.left() + self.node_pan.x + p[0] * self.node_zoom,
                    rect.top() + self.node_pan.y + p[1] * self.node_zoom,
                )
            };

            let node_w = 166.0 * self.node_zoom;
            let header_h = 24.0 * self.node_zoom;
            let port_row_h = 18.0 * self.node_zoom;

            // 各ノードの矩形とポート中心位置を事前計算
            let mut node_rects: BTreeMap<NodeId, Rect> = BTreeMap::new();
            let mut in_ports: BTreeMap<(NodeId, String), Pos2> = BTreeMap::new();
            let mut out_ports: BTreeMap<(NodeId, String), Pos2> = BTreeMap::new();

            for n in &graph.nodes {
                let in_list = n.kind.input_ports();
                let out_list = n.kind.output_ports();
                let rows = in_list.len().max(out_list.len()).max(1) as f32;
                let h = header_h + rows * port_row_h + 12.0 * self.node_zoom;
                let tl = to_screen(n.position);
                let nrect = Rect::from_min_size(tl, Vec2::new(node_w, h));
                node_rects.insert(n.id, nrect);

                for (i, &p) in in_list.iter().enumerate() {
                    let py = tl.y + header_h + (i as f32 + 0.5) * port_row_h + 4.0 * self.node_zoom;
                    in_ports.insert((n.id, p.to_string()), Pos2::new(nrect.left(), py));
                }
                for (i, &p) in out_list.iter().enumerate() {
                    let py = tl.y + header_h + (i as f32 + 0.5) * port_row_h + 4.0 * self.node_zoom;
                    out_ports.insert((n.id, p.to_string()), Pos2::new(nrect.right(), py));
                }
            }

            // 接続ベジェ曲線を描画
            for conn in &graph.connections {
                if let (Some(&p0), Some(&p3)) = (
                    out_ports.get(&(conn.from_node, conn.from_port.clone())),
                    in_ports.get(&(conn.to_node, conn.to_port.clone())),
                ) {
                    draw_bezier_wire(&painter, p0, p3, Color32::from_rgb(120, 210, 255), 2.2);
                }
            }

            // 配線ドラッグ中の仮ワイヤーを描画
            if let Some((from_id, ref from_port)) = self.pending_wire_from
                && let Some(&p0) = out_ports.get(&(from_id, from_port.clone()))
                && let Some(cursor) = ui.ctx().pointer_hover_pos()
            {
                draw_bezier_wire(&painter, p0, cursor, Color32::from_rgb(255, 220, 90), 2.5);
            }

            // ノードボックス・ポート描画
            let font_id = egui::FontId::proportional((11.5 * self.node_zoom).clamp(8.5, 16.0));
            let small_font = egui::FontId::proportional((10.0 * self.node_zoom).clamp(7.5, 13.0));
            let port_r = (5.5 * self.node_zoom).clamp(3.5, 8.5);

            for n in &graph.nodes {
                let Some(&nrect) = node_rects.get(&n.id) else {
                    continue;
                };
                let is_sel = self.selected_node == Some(n.id);
                let header_col = node_category_color(&n.kind, n.bypassed);
                let body_col = if n.bypassed {
                    Color32::from_rgb(38, 40, 44)
                } else {
                    Color32::from_rgb(32, 37, 46)
                };
                let border_stroke = if is_sel {
                    Stroke::new(2.2, Color32::from_rgb(255, 215, 80))
                } else {
                    Stroke::new(1.0, Color32::from_rgb(72, 80, 96))
                };

                painter.rect_filled(nrect, 6.0, body_col);
                let hrect = Rect::from_min_size(nrect.min, Vec2::new(nrect.width(), header_h));
                painter.rect_filled(hrect, 6.0, header_col);
                painter.rect_stroke(nrect, 6.0, border_stroke, egui::StrokeKind::Inside);

                let title = if n.bypassed {
                    format!("⊘ {}", n.name)
                } else {
                    n.name.clone()
                };
                painter.text(
                    Pos2::new(hrect.left() + 6.0, hrect.center().y),
                    egui::Align2::LEFT_CENTER,
                    title,
                    font_id.clone(),
                    Color32::WHITE,
                );

                for &p in n.kind.input_ports() {
                    if let Some(&pos) = in_ports.get(&(n.id, p.to_string())) {
                        let connected = graph.upstream_for(n.id, p).is_some();
                        painter.circle_filled(
                            pos,
                            port_r,
                            if connected {
                                Color32::from_rgb(100, 220, 255)
                            } else {
                                Color32::from_rgb(95, 105, 120)
                            },
                        );
                        painter.text(
                            Pos2::new(pos.x + port_r + 4.0, pos.y),
                            egui::Align2::LEFT_CENTER,
                            p,
                            small_font.clone(),
                            Color32::from_gray(205),
                        );
                    }
                }
                for &p in n.kind.output_ports() {
                    if let Some(&pos) = out_ports.get(&(n.id, p.to_string())) {
                        painter.circle_filled(pos, port_r, Color32::from_rgb(255, 196, 80));
                        painter.text(
                            Pos2::new(pos.x - port_r - 4.0, pos.y),
                            egui::Align2::RIGHT_CENTER,
                            p,
                            small_font.clone(),
                            Color32::from_gray(205),
                        );
                    }
                }
            }

            // マウス操作（ポート配線 / ノード選択 / ドラッグ移動）
            let hit_r = (port_r * 2.2).max(11.0);
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let mut handled = false;
                // 出力ポートクリック → 配線開始
                for ((nid, pname), &ppos) in &out_ports {
                    if ppos.distance(pos) <= hit_r {
                        self.pending_wire_from = Some((*nid, pname.clone()));
                        self.node_status =
                            "配線中: 接続先の入力ポートをクリックしてください".into();
                        handled = true;
                        break;
                    }
                }
                // 入力ポートクリック → 配線確定または切断
                if !handled {
                    for ((nid, pname), &ppos) in &in_ports {
                        if ppos.distance(pos) <= hit_r {
                            if let Some((from_id, from_port)) = self.pending_wire_from.take() {
                                match graph.connect(from_id, from_port, *nid, pname.clone()) {
                                    Ok(()) => {
                                        self.node_status = "ノード間を接続しました".into();
                                        changed = true;
                                    }
                                    Err(err) => {
                                        self.node_status = format!("接続エラー: {err}");
                                    }
                                }
                            } else if graph.disconnect(*nid, pname) {
                                self.node_status = format!("ポート '{pname}' の接続を解除しました");
                                changed = true;
                            }
                            handled = true;
                            break;
                        }
                    }
                }
                // ノード本体クリック → 選択
                if !handled {
                    self.pending_wire_from = None;
                    self.selected_node = graph
                        .nodes
                        .iter()
                        .rev()
                        .find(|n| node_rects.get(&n.id).is_some_and(|r| r.contains(pos)))
                        .map(|n| n.id);
                }
            }

            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.dragging_node = graph
                    .nodes
                    .iter()
                    .rev()
                    .find(|n| node_rects.get(&n.id).is_some_and(|r| r.contains(pos)))
                    .map(|n| n.id);
                if self.dragging_node.is_some() {
                    self.selected_node = self.dragging_node;
                }
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(drag_id) = self.dragging_node
            {
                let delta = response.drag_delta() / self.node_zoom.max(0.2);
                if let Some(node) = graph.node_mut(drag_id) {
                    node.position[0] += delta.x;
                    node.position[1] += delta.y;
                    changed = true;
                }
            }
            if response.drag_stopped() {
                self.dragging_node = None;
            }

            // --- 右側: 選択ノードインスペクタ ---
            ui.vertical(|ui| {
                ui.set_width(inspector_w);
                ui.strong("🔧 選択ノード設定");
                ui.separator();
                if let Some(sel_id) = self.selected_node {
                    let mut remove_this = false;
                    if let Some(node) = graph.node_mut(sel_id) {
                        ui.horizontal(|ui| {
                            ui.label("名称:");
                            if ui.text_edit_singleline(&mut node.name).changed() {
                                changed = true;
                            }
                        });
                        if ui.checkbox(&mut node.bypassed, "バイパス (一時無効化)").changed() {
                            changed = true;
                        }
                        ui.separator();
                        match &mut node.kind {
                            CompositeNodeKind::LayerInput { layer_index } => {
                                let max_l = sess
                                    .project
                                    .active_scene()
                                    .map(|s| s.layers.len().saturating_sub(1))
                                    .unwrap_or(0);
                                ui.horizontal(|ui| {
                                    ui.label("Layer Index:");
                                    if ui
                                        .add(egui::DragValue::new(layer_index).range(0..=max_l))
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });
                            }
                            CompositeNodeKind::SolidColor { color } => {
                                let mut rgba = [color.r, color.g, color.b, color.a];
                                if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                                    *color = RgbaColor {
                                        r: rgba[0],
                                        g: rgba[1],
                                        b: rgba[2],
                                        a: rgba[3],
                                    };
                                    changed = true;
                                }
                            }
                            CompositeNodeKind::Effect {
                                effect_type,
                                params,
                            } => {
                                let choices = [
                                    "blur",
                                    "glow",
                                    "brightness",
                                    "invert",
                                    "mosaic",
                                    "chroma_key",
                                    "ai_segment",
                                    "ai_super_res",
                                    "vignette",
                                    "sepia",
                                ];
                                egui::ComboBox::from_id_salt("node_fx_type")
                                    .selected_text(effect_type.as_str())
                                    .show_ui(ui, |ui| {
                                        for c in choices {
                                            if ui
                                                .selectable_value(effect_type, c.to_string(), c)
                                                .changed()
                                            {
                                                changed = true;
                                            }
                                        }
                                    });
                                for (k, v) in params.iter_mut() {
                                    ui.horizontal(|ui| {
                                        ui.label(k.as_str());
                                        if ui
                                            .add(egui::DragValue::new(v).speed(0.05))
                                            .changed()
                                        {
                                            changed = true;
                                        }
                                    });
                                }
                                if ui.small_button("+ パラメータ (strength)").clicked() {
                                    params.entry("strength".into()).or_insert(0.5);
                                    changed = true;
                                }
                            }
                            CompositeNodeKind::ChannelSplit { channel } => {
                                egui::ComboBox::from_id_salt("node_ch_split")
                                    .selected_text(channel.label())
                                    .show_ui(ui, |ui| {
                                        for &ch in ChannelSelector::ALL {
                                            if ui
                                                .selectable_value(channel, ch, ch.label())
                                                .changed()
                                            {
                                                changed = true;
                                            }
                                        }
                                    });
                            }
                            CompositeNodeKind::ChannelMerge => {
                                ui.weak("入力 r / g / b / a を結合して出力します");
                            }
                            CompositeNodeKind::Transform {
                                x,
                                y,
                                scale_x,
                                scale_y,
                                rotation_deg,
                                opacity,
                            } => {
                                changed |= ui
                                    .add(egui::Slider::new(x, -960.0..=960.0).text("X"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(y, -540.0..=540.0).text("Y"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(scale_x, 0.1..=4.0).text("Scale X"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(scale_y, 0.1..=4.0).text("Scale Y"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(rotation_deg, -180.0..=180.0).text("Rot°"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(opacity, 0.0..=1.0).text("Opacity"))
                                    .changed();
                            }
                            CompositeNodeKind::Merge {
                                blend_mode,
                                opacity,
                            } => {
                                egui::ComboBox::from_id_salt("node_merge_bm")
                                    .selected_text(format!("{blend_mode:?}"))
                                    .show_ui(ui, |ui| {
                                        for bm in [
                                            BlendMode::Normal,
                                            BlendMode::Add,
                                            BlendMode::Multiply,
                                            BlendMode::Screen,
                                        ] {
                                            if ui
                                                .selectable_value(blend_mode, bm, format!("{bm:?}"))
                                                .changed()
                                            {
                                                changed = true;
                                            }
                                        }
                                    });
                                changed |= ui
                                    .add(egui::Slider::new(opacity, 0.0..=1.0).text("Opacity"))
                                    .changed();
                            }
                            CompositeNodeKind::MaskCombine { mode } => {
                                egui::ComboBox::from_id_salt("node_matte_mode")
                                    .selected_text(format!("{mode:?}"))
                                    .show_ui(ui, |ui| {
                                        for m in [
                                            TrackMatteMode::Alpha,
                                            TrackMatteMode::AlphaInverted,
                                            TrackMatteMode::Luma,
                                            TrackMatteMode::LumaInverted,
                                        ] {
                                            if ui
                                                .selectable_value(mode, m, format!("{m:?}"))
                                                .changed()
                                            {
                                                changed = true;
                                            }
                                        }
                                    });
                            }
                            CompositeNodeKind::ColorCorrect {
                                exposure,
                                contrast,
                                saturation,
                            } => {
                                changed |= ui
                                    .add(egui::Slider::new(exposure, -3.0..=3.0).text("Exposure"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(contrast, 0.2..=2.5).text("Contrast"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(saturation, 0.0..=2.5).text("Sat"))
                                    .changed();
                            }
                            CompositeNodeKind::DepthFog {
                                near,
                                far,
                                strength,
                                ..
                            } => {
                                changed |= ui
                                    .add(egui::Slider::new(near, 0.0..=2000.0).text("Near"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(far, 100.0..=5000.0).text("Far"))
                                    .changed();
                                changed |= ui
                                    .add(egui::Slider::new(strength, 0.0..=1.0).text("Fog"))
                                    .changed();
                            }
                            CompositeNodeKind::Output => {
                                ui.weak("最終合成出力ノード (MediaOut)");
                            }
                        }
                        ui.separator();
                        if !matches!(node.kind, CompositeNodeKind::Output)
                            && ui.button("🗑 このノードを削除").clicked()
                        {
                            remove_this = true;
                        }
                    }
                    if remove_this {
                        graph.remove_node(sel_id);
                        self.selected_node = None;
                        changed = true;
                    }
                } else {
                    ui.weak("キャンバス上のノードをクリックするとパラメータを編集できます。\n\n操作ヒント:\n・黄丸(out) → 青丸(in) クリックで配線\n・接続済み青丸クリックで切断\n・中/右ドラッグでキャンバス移動");
                }
            });
        });

        if changed {
            let coalesce = if self.dragging_node.is_some() {
                Some("node_graph_drag")
            } else {
                None
            };
            let _ = sess.bus.execute(
                &mut sess.project,
                EditCommand::SetNodeGraph { graph },
                CommandSource::Gui,
                coalesce,
            );
        }
        changed
    }

    // =========================================================================
    // #22 / #23 / #24: v2.0 フロンティアスタジオウィンドウ
    // =========================================================================

    pub fn show_frontier_window(
        &mut self,
        ctx: &egui::Context,
        session: &SharedEditorSession,
    ) -> bool {
        let mut open = self.show_frontier_window;
        let mut preview_dirty = false;

        egui::Window::new("🚀 v2.0 フロンティアスタジオ（ローカルAI・AIディレクター・共同編集）")
            .open(&mut open)
            .default_size([780.0, 560.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.active_tab,
                        FrontierTab::AiVision,
                        "🧠 ローカル AI ビジョン (#22)",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        FrontierTab::Director,
                        "🤖 AI 編集ディレクター (#23)",
                    );
                    ui.selectable_value(
                        &mut self.active_tab,
                        FrontierTab::CollabReview,
                        "🤝 共同編集・Diff＆レビュー (#24)",
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.active_tab {
                    FrontierTab::AiVision => {
                        let mut sess = session.lock().unwrap();
                        if self.ui_ai_vision_tab(ui, &mut sess) {
                            preview_dirty = true;
                        }
                    }
                    FrontierTab::Director => {
                        if self.ui_director_tab(ui, session) {
                            preview_dirty = true;
                        }
                    }
                    FrontierTab::CollabReview => {
                        let mut sess = session.lock().unwrap();
                        if self.ui_collab_review_tab(ui, &mut sess) {
                            preview_dirty = true;
                        }
                    }
                });
            });

        self.show_frontier_window = open;
        preview_dirty
    }

    // --- Tab 1: #22 ローカル AI ビジョン ---
    fn ui_ai_vision_tab(&mut self, ui: &mut egui::Ui, sess: &mut EditorSession) -> bool {
        let mut dirty = false;
        let models_dir = sess.folder.as_ref().map(|f| f.models_dir());
        let status = detect_local_vision_status(models_dir.as_deref());

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.colored_label(
                    Color32::from_rgb(90, 230, 150),
                    "🔒 完全オフライン推論（クラウド送信ゼロ）",
                );
                ui.weak(&status.summary);
            });
            ui.small("オプショナルなローカル ONNX モデルはプロジェクト内 `models/*.onnx` + 環境変数 `VE_LOCAL_ONNX_BIN` で自動認識されます。");
        });
        ui.add_space(6.0);

        let selected_id = sess.selected_items.first().copied();

        // 1. グリーンバック不要・自動ロトブラシ切り抜き
        ui.group(|ui| {
            ui.strong("✂ グリーンバック不要 人物・被写体切り抜き（自動ロトブラシ）");
            ui.add(egui::Slider::new(&mut self.seg_sensitivity, 0.1..=1.0).text("分離感度"));
            ui.add(
                egui::Slider::new(&mut self.seg_feather_px, 0.0..=6.0).text("輪郭フェザー (px)"),
            );
            ui.add(egui::Slider::new(&mut self.seg_skin_boost, 0.0..=1.0).text("人物肌色保護"));
            ui.add(egui::Slider::new(&mut self.seg_despill, 0.0..=1.0).text("エッジ背景色除去"));
            ui.checkbox(&mut self.seg_invert, "マット反転（背景を残す）");
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        selected_id.is_some(),
                        egui::Button::new("✂ 選択クリップへ切り抜き適用 (ai_segment)"),
                    )
                    .clicked()
                    && let Some(item_id) = selected_id
                {
                    let mut params = BTreeMap::new();
                    params.insert("sensitivity".into(), self.seg_sensitivity);
                    params.insert("edge_feather_px".into(), self.seg_feather_px);
                    params.insert("skin_boost".into(), self.seg_skin_boost);
                    params.insert("despill_strength".into(), self.seg_despill);
                    params.insert("invert".into(), if self.seg_invert { 1.0 } else { 0.0 });
                    let eff = EffectNode {
                        id: Uuid::new_v4(),
                        effect_type: "ai_segment".into(),
                        enabled: true,
                        params,
                    };
                    if sess
                        .bus
                        .execute(
                            &mut sess.project,
                            EditCommand::AddEffect {
                                item_id,
                                effect: eff,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.ai_status =
                            "自動ロトブラシ切り抜き (ai_segment) を適用しました".into();
                        dirty = true;
                    }
                }
                if selected_id.is_none() {
                    ui.weak("※ タイムラインで対象クリップを選択してください");
                }
            });
        });

        ui.add_space(6.0);

        // 2. 被写体・顔追尾 16:9 -> 9:16 自動リフレーム
        ui.group(|ui| {
            ui.strong("📱 被写体・顔追尾 インテリジェント自動リフレーム");
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("9:16 Shorts (1080×1920)").clicked() {
                    self.reframe_target_w = 1080;
                    self.reframe_target_h = 1920;
                }
                if ui.small_button("1:1 SNS (1080×1080)").clicked() {
                    self.reframe_target_w = 1080;
                    self.reframe_target_h = 1080;
                }
                if ui.small_button("4:5 縦投稿 (1080×1350)").clicked() {
                    self.reframe_target_w = 1080;
                    self.reframe_target_h = 1350;
                }
                ui.label(format!("目標: {}×{}", self.reframe_target_w, self.reframe_target_h));
            });
            ui.add(
                egui::Slider::new(&mut self.reframe_smoothing, 0.0..=0.95)
                    .text("カメラ追従スムージング"),
            );
            ui.horizontal(|ui| {
                if ui.button("🔍 被写体軌跡を解析").clicked() {
                    let cfg = AutoReframeConfig {
                        target_width: self.reframe_target_w,
                        target_height: self.reframe_target_h,
                        smoothing_strength: self.reframe_smoothing,
                        ..Default::default()
                    };
                    let mut rg = RenderGraph::new();
                    rg.set_project_folder(sess.folder.clone());
                    self.reframe_samples = analyze_scene_auto_reframe(
                        &mut rg,
                        &sess.project,
                        sess.project.active_scene_id,
                        &cfg,
                    );
                    self.ai_status = format!(
                        "被写体・顔追尾解析完了: {} サンプル検出",
                        self.reframe_samples.len()
                    );
                }
                if ui.button("🎯 自動リフレームを適用 (1-Undo)").clicked() {
                    let cfg = AutoReframeConfig {
                        target_width: self.reframe_target_w,
                        target_height: self.reframe_target_h,
                        smoothing_strength: self.reframe_smoothing,
                        ..Default::default()
                    };
                    if self.reframe_samples.is_empty() {
                        let mut rg = RenderGraph::new();
                        rg.set_project_folder(sess.folder.clone());
                        self.reframe_samples = analyze_scene_auto_reframe(
                            &mut rg,
                            &sess.project,
                            sess.project.active_scene_id,
                            &cfg,
                        );
                    }
                    if let Some(scene) = sess.project.active_scene() {
                        let cmds = build_auto_reframe_commands(scene, &cfg, &self.reframe_samples);
                        let n = cmds.len();
                        if sess
                            .bus
                            .execute(
                                &mut sess.project,
                                EditCommand::Batch {
                                    label: format!(
                                        "AI 自動リフレーム ({}x{})",
                                        cfg.target_width, cfg.target_height
                                    ),
                                    commands: cmds,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                        {
                            self.ai_status = format!(
                                "自動リフレームを適用しました ({}×{}, キーフレーム含む {n} コマンド)",
                                cfg.target_width, cfg.target_height
                            );
                            dirty = true;
                        }
                    }
                }
            });

            if !self.reframe_samples.is_empty() {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(340.0, 52.0), egui::Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 4.0, Color32::from_rgb(24, 28, 36));
                let mid_y = rect.center().y;
                p.line_segment(
                    [Pos2::new(rect.left(), mid_y), Pos2::new(rect.right(), mid_y)],
                    Stroke::new(1.0, Color32::from_gray(70)),
                );
                let n = self.reframe_samples.len().max(2);
                for i in 0..self.reframe_samples.len().saturating_sub(1) {
                    let x0 = rect.left() + (i as f32 / (n - 1) as f32) * rect.width();
                    let x1 = rect.left() + ((i + 1) as f32 / (n - 1) as f32) * rect.width();
                    let y0 = rect.top() + self.reframe_samples[i].focus_x_norm * rect.height();
                    let y1 = rect.top() + self.reframe_samples[i + 1].focus_x_norm * rect.height();
                    p.line_segment(
                        [Pos2::new(x0, y0), Pos2::new(x1, y1)],
                        Stroke::new(2.0, Color32::from_rgb(80, 220, 255)),
                    );
                }
                ui.small("シアン曲線: 時間経過に伴う被写体 X 注視位置（中央線 = 50%）");
            }
        });

        ui.add_space(6.0);

        // 3. ローカル超解像 & オプティカルフロー滑らかスローモーション
        ui.group(|ui| {
            ui.strong("🔎 ローカル超解像 ＆ 🌊 オプティカルフロー・スロー補間");
            ui.add(egui::Slider::new(&mut self.sr_sharpen, 0.0..=1.8).text("超解像エッジ復元"));
            ui.add(egui::Slider::new(&mut self.sr_denoise, 0.0..=1.0).text("平坦部ノイズ除去"));
            if ui
                .add_enabled(
                    selected_id.is_some(),
                    egui::Button::new("🔎 選択クリップに超解像ディテール復元を適用 (ai_super_res)"),
                )
                .clicked()
                && let Some(item_id) = selected_id
            {
                let mut params = BTreeMap::new();
                params.insert("sharpen".into(), self.sr_sharpen);
                params.insert("denoise".into(), self.sr_denoise);
                let eff = EffectNode {
                    id: Uuid::new_v4(),
                    effect_type: "ai_super_res".into(),
                    enabled: true,
                    params,
                };
                if sess
                    .bus
                    .execute(
                        &mut sess.project,
                        EditCommand::AddEffect {
                            item_id,
                            effect: eff,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.ai_status = "ローカル超解像 (ai_super_res) を適用しました".into();
                    dirty = true;
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut self.slowmo_speed, 0.2..=0.8).text("スロー倍率"));
                if ui
                    .add_enabled(
                        selected_id.is_some(),
                        egui::Button::new("🌊 オプティカルフロー補間スロー適用"),
                    )
                    .clicked()
                    && let Some(item_id) = selected_id
                {
                    let cmds = vec![
                        EditCommand::SetPlaybackSpeed {
                            item_id,
                            speed: self.slowmo_speed,
                            loop_playback: false,
                        },
                        EditCommand::SetOpticalFlowInterp {
                            item_id,
                            enabled: true,
                        },
                    ];
                    if sess
                        .bus
                        .execute(
                            &mut sess.project,
                            EditCommand::Batch {
                                label: "オプティカルフロー補間スロー".into(),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.ai_status = format!(
                            "オプティカルフロー中間フレーム補間スロー ({:.2}x) を有効化しました",
                            self.slowmo_speed
                        );
                        dirty = true;
                    }
                }
            });
        });

        if !self.ai_status.is_empty() {
            ui.add_space(4.0);
            ui.colored_label(Color32::from_rgb(130, 220, 255), &self.ai_status);
        }
        dirty
    }

    // --- Tab 2: #23 マルチエージェント AI 編集ディレクター ---
    fn ui_director_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        let (all_recipes, run_snapshot) = {
            let g = session.lock().unwrap();
            let mut list = builtin_edit_recipes();
            if let Some(folder) = g.folder.as_ref() {
                list.extend(folder.load_recipes());
            }
            (list, g.recipe_run.clone())
        };
        if self.selected_recipe_idx >= all_recipes.len() {
            self.selected_recipe_idx = 0;
        }

        ui.group(|ui| {
            ui.strong("📚 編集レシピ ライブラリ（YAML / JSON 再現・共有マクロ）");
            ui.horizontal_wrapped(|ui| {
                let current_name = all_recipes
                    .get(self.selected_recipe_idx)
                    .map(|r| r.name.as_str())
                    .unwrap_or("(なし)");
                egui::ComboBox::from_id_salt("director_recipe_combo")
                    .selected_text(current_name)
                    .show_ui(ui, |ui| {
                        for (i, r) in all_recipes.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.selected_recipe_idx, i, &r.name)
                                .clicked()
                                && let Ok(y) = serialize_recipe_yaml(r)
                            {
                                self.recipe_yaml_editor = y;
                            }
                        }
                    });
                if ui.button("📋 YAML をエディタへ読込").clicked()
                    && let Some(r) = all_recipes.get(self.selected_recipe_idx)
                    && let Ok(y) = serialize_recipe_yaml(r)
                {
                    self.recipe_yaml_editor = y;
                }
            });

            ui.horizontal(|ui| {
                ui.label("タイトル変数:");
                ui.text_edit_singleline(&mut self.recipe_var_title);
                ui.label("字幕変数:");
                ui.text_edit_singleline(&mut self.recipe_var_subtitle);
            });

            ui.horizontal_wrapped(|ui| {
                if ui.button("▶ 承認ゲートまで実行").clicked() {
                    match parse_recipe_json_or_yaml(&self.recipe_yaml_editor) {
                        Ok(mut recipe) => {
                            recipe
                                .variables
                                .insert("title".into(), self.recipe_var_title.clone());
                            recipe
                                .variables
                                .insert("subtitle".into(), self.recipe_var_subtitle.clone());
                            match start_recipe_run(session, recipe)
                                .and_then(|_| run_recipe_until_gate(session))
                            {
                                Ok(st) => {
                                    self.director_status = st.last_message;
                                    dirty = true;
                                }
                                Err(e) => self.director_status = format!("実行エラー: {e}"),
                            }
                        }
                        Err(e) => self.director_status = format!("YAML/JSON 構文エラー: {e}"),
                    }
                }
                if ui.button("⏭ 1ステップ実行").clicked() {
                    let has_run = session.lock().unwrap().recipe_run.is_some();
                    if !has_run
                        && let Ok(recipe) = parse_recipe_json_or_yaml(&self.recipe_yaml_editor)
                    {
                        let _ = start_recipe_run(session, recipe);
                    }
                    match advance_recipe_step(session, false) {
                        Ok(st) => {
                            self.director_status = st.last_message;
                            dirty = true;
                        }
                        Err(e) => self.director_status = format!("エラー: {e}"),
                    }
                }
                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut self.record_recipe_name).desired_width(120.0),
                );
                if ui.button("📼 操作履歴からレシピ記録").clicked() {
                    let mut g = session.lock().unwrap();
                    let cmds = g.bus.recent_commands(25);
                    let rec = record_recipe_from_commands(
                        self.record_recipe_name.clone(),
                        "GUI 編集履歴から記録した再利用レシピ",
                        &cmds,
                    );
                    if let Ok(y) = serialize_recipe_yaml(&rec) {
                        self.recipe_yaml_editor = y;
                    }
                    if let Some(folder) = g.folder.as_ref() {
                        let _ = folder.save_recipe(&rec);
                    }
                    g.active_recipe = Some(rec.clone());
                    self.director_status = format!(
                        "{} ステップのレシピ「{}」を記録しました",
                        rec.steps.len(),
                        rec.name
                    );
                }
                if ui.button("💾 recipes/ へ保存").clicked() {
                    match parse_recipe_json_or_yaml(&self.recipe_yaml_editor) {
                        Ok(rec) => {
                            let g = session.lock().unwrap();
                            if let Some(folder) = g.folder.as_ref() {
                                match folder.save_recipe(&rec) {
                                    Ok(p) => {
                                        self.director_status =
                                            format!("レシピ保存完了: {}", p.display());
                                    }
                                    Err(e) => self.director_status = format!("保存エラー: {e}"),
                                }
                            } else {
                                self.director_status =
                                    "プロジェクトフォルダを開くと recipes/ へ保存できます".into();
                            }
                        }
                        Err(e) => self.director_status = format!("構文エラー: {e}"),
                    }
                }
            });
        });

        // 人間承認ゲートカード & 実行ログ
        if let Some(run) = run_snapshot {
            ui.add_space(6.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.strong(format!(
                        "🎬 実行状況: {} ({}/{})",
                        run.recipe_name, run.current_step_index, run.total_steps
                    ));
                    let badge_col = match run.status {
                        ApprovalGateStatus::AwaitingApproval => Color32::from_rgb(255, 200, 60),
                        ApprovalGateStatus::Completed => Color32::from_rgb(90, 230, 140),
                        ApprovalGateStatus::Failed | ApprovalGateStatus::Rejected => {
                            Color32::from_rgb(255, 100, 100)
                        }
                        _ => Color32::from_rgb(120, 200, 255),
                    };
                    ui.colored_label(badge_col, format!("[{:?}]", run.status));
                });
                ui.label(&run.last_message);

                if run.status == ApprovalGateStatus::AwaitingApproval {
                    ui.add_space(4.0);
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgba_unmultiplied(70, 52, 16, 180))
                        .show(ui, |ui| {
                            ui.colored_label(
                                Color32::from_rgb(255, 224, 110),
                                format!(
                                    "🛡 人間承認ゲート: {}",
                                    run.pending_gate_title
                                        .as_deref()
                                        .unwrap_or("次のステップ実行許可")
                                ),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("✅ 承認して続行").clicked() {
                                    match approve_current_gate(session, true) {
                                        Ok(st) => {
                                            self.director_status = st.last_message;
                                            dirty = true;
                                        }
                                        Err(e) => self.director_status = format!("エラー: {e}"),
                                    }
                                }
                                if ui.button("⏩ このステップをスキップ").clicked()
                                    && let Ok(st) =
                                        reject_current_gate(session, false, true, "手動スキップ")
                                {
                                    self.director_status = st.last_message;
                                }
                                if ui.button("↩ 却下＋直前をUndo").clicked()
                                    && let Ok(st) = reject_current_gate(
                                        session,
                                        true,
                                        false,
                                        "人間レビュー却下",
                                    )
                                {
                                    self.director_status = st.last_message;
                                    dirty = true;
                                }
                            });
                        });
                }

                if !run.logs.is_empty() {
                    ui.separator();
                    ui.weak("実行＆視覚自己検査ログ:");
                    for log in &run.logs {
                        ui.horizontal_wrapped(|ui| {
                            let icon = if log.succeeded { "✅" } else { "❌" };
                            ui.label(format!(
                                "{icon} #{} {} — {}",
                                log.step_index + 1,
                                log.role.display_label(),
                                log.title
                            ));
                            if let Some(vr) = &log.visual_report {
                                let col = if vr.passed {
                                    Color32::from_rgb(110, 235, 160)
                                } else {
                                    Color32::from_rgb(255, 190, 80)
                                };
                                ui.colored_label(col, format!("[👁 {}]", vr.advice));
                            }
                        });
                    }
                }
            });
        }

        ui.add_space(6.0);
        ui.collapsing(
            "📝 レシピ YAML / JSON エディタ（共有・直接編集可能）",
            |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.recipe_yaml_editor)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY),
                );
            },
        );

        if !self.director_status.is_empty() {
            ui.colored_label(Color32::from_rgb(140, 220, 255), &self.director_status);
        }
        dirty
    }

    // --- Tab 3: #24 共同編集・視覚 Diff・3-Way マージ・タイムコードレビュー ---
    fn ui_collab_review_tab(&mut self, ui: &mut egui::Ui, sess: &mut EditorSession) -> bool {
        let mut dirty = false;
        let snapshots = sess
            .folder
            .as_ref()
            .map(|f| f.list_snapshots())
            .unwrap_or_default();

        // 1. スナップショット保存 & 視覚的タイムライン Diff
        ui.group(|ui| {
            ui.strong("🔀 Git-Friendly スナップショット ＆ 視覚的タイムライン Diff");
            ui.horizontal_wrapped(|ui| {
                ui.label("スナップショット名:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.snapshot_name_input).desired_width(140.0),
                );
                if ui.button("📸 現在状態を snapshots/ へ保存").clicked() {
                    if let Some(folder) = sess.folder.as_ref() {
                        match folder.save_snapshot(&self.snapshot_name_input, &sess.project) {
                            Ok(p) => {
                                self.collab_status =
                                    format!("スナップショット保存: {}", p.display());
                                if self.diff_base_snapshot.is_empty() {
                                    self.diff_base_snapshot = self.snapshot_name_input.clone();
                                    self.merge_base_snapshot = self.snapshot_name_input.clone();
                                }
                            }
                            Err(e) => self.collab_status = format!("保存エラー: {e}"),
                        }
                    } else {
                        self.collab_status =
                            "プロジェクトフォルダを開くと snapshots/ を利用できます".into();
                    }
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Base:");
                egui::ComboBox::from_id_salt("diff_base_combo")
                    .selected_text(if self.diff_base_snapshot.is_empty() {
                        "(選択)"
                    } else {
                        &self.diff_base_snapshot
                    })
                    .show_ui(ui, |ui| {
                        for sn in &snapshots {
                            ui.selectable_value(&mut self.diff_base_snapshot, sn.clone(), sn);
                        }
                    });
                ui.label("Target:");
                egui::ComboBox::from_id_salt("diff_target_combo")
                    .selected_text(&self.diff_target_snapshot)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.diff_target_snapshot,
                            "(現在のプロジェクト)".into(),
                            "(現在のプロジェクト)",
                        );
                        for sn in &snapshots {
                            ui.selectable_value(&mut self.diff_target_snapshot, sn.clone(), sn);
                        }
                    });
                if ui.button("🔍 タイムライン Diff 比較").clicked()
                    && let Some(folder) = sess.folder.as_ref()
                {
                    match folder.load_snapshot(&self.diff_base_snapshot) {
                        Ok(base_proj) => {
                            let target_res = if self.diff_target_snapshot == "(現在のプロジェクト)"
                            {
                                Ok(sess.project.clone())
                            } else {
                                folder.load_snapshot(&self.diff_target_snapshot)
                            };
                            match target_res {
                                Ok(target_proj) => {
                                    let d = diff_projects(&base_proj, &target_proj);
                                    self.collab_status =
                                        format!("Diff 完了: 合計 {} 件の変更", d.total_changes());
                                    self.last_diff = Some(d);
                                }
                                Err(e) => self.collab_status = format!("Target 読込エラー: {e}"),
                            }
                        }
                        Err(e) => self.collab_status = format!("Base 読込エラー: {e}"),
                    }
                }
            });

            if let Some(diff) = &self.last_diff {
                if diff.is_empty() {
                    ui.colored_label(Color32::from_rgb(110, 230, 150), "✅ 差分なし（完全一致）");
                } else {
                    ui.small(format!(
                        "変更サマリー: Scene {} / Layer {} / Item {} / Marker {}",
                        diff.scene_diffs.len(),
                        diff.layer_diffs.len(),
                        diff.item_diffs.len(),
                        diff.marker_diffs.len()
                    ));
                    for it in diff.item_diffs.iter().take(12) {
                        let (sym, col) = match it.kind {
                            DiffChangeKind::Added => ("+ 追加", Color32::from_rgb(100, 235, 145)),
                            DiffChangeKind::Removed => ("- 削除", Color32::from_rgb(255, 115, 115)),
                            DiffChangeKind::Modified => ("~ 変更", Color32::from_rgb(255, 210, 90)),
                        };
                        ui.horizontal(|ui| {
                            ui.colored_label(col, sym);
                            if ui.small_button(format!("@{:.2}s", it.start_sec)).clicked() {
                                sess.playhead = ve_core::RationalTime::from_seconds(it.start_sec);
                                dirty = true;
                            }
                            ui.label(format!(
                                "L{} {} ({})",
                                it.layer_index + 1,
                                it.item_name,
                                it.changed_fields.join(", ")
                            ));
                        });
                    }
                }
            }
        });

        ui.add_space(6.0);

        // 2. 3-Way 意味的ブランチマージ
        ui.group(|ui| {
            ui.strong("🌿 3-Way 意味的ブランチマージ (Base + Ours[現在] + Theirs)");
            ui.horizontal_wrapped(|ui| {
                ui.label("Base:");
                egui::ComboBox::from_id_salt("merge_base_combo")
                    .selected_text(if self.merge_base_snapshot.is_empty() {
                        "(選択)"
                    } else {
                        &self.merge_base_snapshot
                    })
                    .show_ui(ui, |ui| {
                        for sn in &snapshots {
                            ui.selectable_value(&mut self.merge_base_snapshot, sn.clone(), sn);
                        }
                    });
                ui.label("Theirs:");
                egui::ComboBox::from_id_salt("merge_theirs_combo")
                    .selected_text(if self.merge_theirs_snapshot.is_empty() {
                        "(選択)"
                    } else {
                        &self.merge_theirs_snapshot
                    })
                    .show_ui(ui, |ui| {
                        for sn in &snapshots {
                            ui.selectable_value(&mut self.merge_theirs_snapshot, sn.clone(), sn);
                        }
                    });
                if ui.button("🔬 マージ試算").clicked()
                    && let Some(folder) = sess.folder.as_ref()
                {
                    match (
                        folder.load_snapshot(&self.merge_base_snapshot),
                        folder.load_snapshot(&self.merge_theirs_snapshot),
                    ) {
                        (Ok(b), Ok(t)) => {
                            let res = merge_projects_three_way(
                                &b,
                                &sess.project,
                                &t,
                                &self.merge_resolutions,
                            );
                            self.collab_status = format!(
                                "3-Way 試算: 自動マージ {} 件 / 競合 {} 件",
                                res.auto_merged_from_theirs,
                                res.conflicts.len()
                            );
                            self.last_merge = Some(res);
                        }
                        _ => {
                            self.collab_status =
                                "Base / Theirs スナップショットを選択してください".into()
                        }
                    }
                }
            });

            if let Some(m) = self.last_merge.clone() {
                if !m.conflicts.is_empty() {
                    ui.colored_label(
                        Color32::from_rgb(255, 170, 80),
                        format!("⚠ {} 件の競合を解決してください:", m.conflicts.len()),
                    );
                    for c in &m.conflicts {
                        let entry = self
                            .merge_resolutions
                            .entry(c.conflict_id.clone())
                            .or_insert(ConflictResolution::KeepOurs);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(&c.description);
                            ui.radio_value(entry, ConflictResolution::KeepOurs, "Ours 維持");
                            ui.radio_value(entry, ConflictResolution::KeepTheirs, "Theirs 採用");
                            ui.radio_value(
                                entry,
                                ConflictResolution::KeepBothDuplicate,
                                "両方保持(複製)",
                            );
                        });
                    }
                }
                if ui
                    .button("🔀 マージ結果を適用 (Ctrl+Z で元に戻せます)")
                    .clicked()
                    && let Some(folder) = sess.folder.as_ref()
                    && let (Ok(b), Ok(t)) = (
                        folder.load_snapshot(&self.merge_base_snapshot),
                        folder.load_snapshot(&self.merge_theirs_snapshot),
                    )
                {
                    let final_m =
                        merge_projects_three_way(&b, &sess.project, &t, &self.merge_resolutions);
                    if sess
                        .bus
                        .execute(
                            &mut sess.project,
                            EditCommand::ReplaceProjectSnapshot {
                                project: Box::new(final_m.merged_project),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.collab_status = format!(
                            "3-Way マージ適用完了（自動統合 {} 件）",
                            final_m.auto_merged_from_theirs
                        );
                        self.last_merge = None;
                        dirty = true;
                    }
                }
            }
        });

        ui.add_space(6.0);

        // 3. タイムコード付きレビューコメント（Frame.io / YouTube 互換）
        ui.group(|ui| {
            ui.strong("💬 タイムコード付きレビューコメント（Frame.io / YouTube 互換）");
            let fps = sess
                .project
                .active_scene()
                .map(|s| s.fps)
                .unwrap_or(sess.project.settings.fps);
            let mut comments = sess
                .project
                .active_scene()
                .map(|s| s.review_comments.clone())
                .unwrap_or_default();
            let mut comments_changed = false;

            ui.horizontal(|ui| {
                ui.label("投稿者:");
                ui.add(egui::TextEdit::singleline(&mut self.review_author).desired_width(90.0));
                ui.label("コメント:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.review_body_input).desired_width(240.0),
                );
                if ui.button("➕ 現在位置へ追加").clicked()
                    && !self.review_body_input.trim().is_empty()
                {
                    comments.push(ReviewComment::new(
                        sess.playhead,
                        self.review_author.clone(),
                        self.review_body_input.trim().to_string(),
                        ReviewSource::Internal,
                    ));
                    self.review_body_input.clear();
                    comments_changed = true;
                }
            });

            // コメント一覧（タイムコードクリックでシーク＋ステータス切替）
            let mut seek_to = None;
            for c in &mut comments {
                ui.horizontal_wrapped(|ui| {
                    let tc = format_smpte_timecode(c.time, fps);
                    if ui.small_button(format!("⏱ {tc}")).clicked() {
                        seek_to = Some(c.time);
                    }
                    ui.strong(&c.author);
                    ui.label(&c.body);
                    let next_status = match c.status {
                        ReviewStatus::Open => ReviewStatus::InProgress,
                        ReviewStatus::InProgress => ReviewStatus::Resolved,
                        ReviewStatus::Resolved => ReviewStatus::Open,
                    };
                    if ui.small_button(c.status.label()).clicked() {
                        c.status = next_status;
                        comments_changed = true;
                    }
                });
            }
            if let Some(t) = seek_to {
                sess.playhead = t;
                dirty = true;
            }

            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("📍 未解決コメントをタイムラインマーカーへ自動変換")
                    .clicked()
                {
                    let cmds = build_review_comments_to_markers_commands(&comments, true);
                    let cnt = cmds.len();
                    if cnt > 0
                        && sess
                            .bus
                            .execute(
                                &mut sess.project,
                                EditCommand::Batch {
                                    label: format!("レビュー→マーカー ({cnt}件)"),
                                    commands: cmds,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                    {
                        self.collab_status =
                            format!("未解決レビュー {cnt} 件をタイムラインマーカーへ変換しました");
                        dirty = true;
                    }
                }
                if ui.button("📤 Frame.io CSV 書き出し").clicked() {
                    self.review_export_preview = export_review_comments_csv(&comments, fps);
                    self.collab_status = "Frame.io 互換 CSV を生成しました".into();
                }
            });

            ui.collapsing(
                "📥 Frame.io (CSV/JSON) / YouTube タイムスタンプ一括インポート",
                |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.review_import_format, 0, "Frame.io CSV");
                        ui.radio_value(&mut self.review_import_format, 1, "Frame.io JSON");
                        ui.radio_value(&mut self.review_import_format, 2, "YouTube コメント");
                    });
                    ui.add(
                        egui::TextEdit::multiline(&mut self.review_import_text)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                    if ui.button("📥 インポート実行").clicked() {
                        let res = match self.review_import_format {
                            1 => import_review_comments_json(&self.review_import_text, fps),
                            2 => Ok(import_youtube_review_comments(
                                &self.review_import_text,
                                fps,
                            )),
                            _ => Ok(import_review_comments_csv(&self.review_import_text, fps)),
                        };
                        match res {
                            Ok(imp) => {
                                let cnt = imp.len();
                                comments.extend(imp);
                                comments_changed = true;
                                self.collab_status =
                                    format!("レビューコメント {cnt} 件をインポートしました");
                            }
                            Err(e) => self.collab_status = format!("インポートエラー: {e}"),
                        }
                    }
                    if !self.review_export_preview.is_empty() {
                        ui.label("書き出し済み Frame.io CSV:");
                        ui.add(
                            egui::TextEdit::multiline(&mut self.review_export_preview)
                                .font(egui::TextStyle::Monospace)
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                    }
                },
            );

            if comments_changed {
                let _ = sess.bus.execute(
                    &mut sess.project,
                    EditCommand::SetReviewComments { comments },
                    CommandSource::Gui,
                    None,
                );
                dirty = true;
            }
        });

        if !self.collab_status.is_empty() {
            ui.colored_label(Color32::from_rgb(130, 225, 195), &self.collab_status);
        }
        dirty
    }
}

fn node_category_color(kind: &CompositeNodeKind, bypassed: bool) -> Color32 {
    if bypassed {
        return Color32::from_rgb(76, 80, 88);
    }
    match kind {
        CompositeNodeKind::LayerInput { .. } | CompositeNodeKind::SolidColor { .. } => {
            Color32::from_rgb(42, 108, 178)
        }
        CompositeNodeKind::Effect { .. } | CompositeNodeKind::ColorCorrect { .. } => {
            Color32::from_rgb(120, 68, 178)
        }
        CompositeNodeKind::ChannelSplit { .. }
        | CompositeNodeKind::ChannelMerge
        | CompositeNodeKind::MaskCombine { .. } => Color32::from_rgb(36, 138, 146),
        CompositeNodeKind::Transform { .. } | CompositeNodeKind::DepthFog { .. } => {
            Color32::from_rgb(66, 94, 172)
        }
        CompositeNodeKind::Merge { .. } => Color32::from_rgb(184, 118, 34),
        CompositeNodeKind::Output => Color32::from_rgb(38, 146, 88),
    }
}

fn draw_bezier_wire(painter: &egui::Painter, p0: Pos2, p3: Pos2, color: Color32, width: f32) {
    let dx = ((p3.x - p0.x).abs() * 0.5).max(40.0);
    let p1 = Pos2::new(p0.x + dx, p0.y);
    let p2 = Pos2::new(p3.x - dx, p3.y);
    let steps = 20usize;
    let mut prev = p0;
    for s in 1..=steps {
        let t = s as f32 / steps as f32;
        let u = 1.0 - t;
        let b0 = u * u * u;
        let b1 = 3.0 * u * u * t;
        let b2 = 3.0 * u * t * t;
        let b3 = t * t * t;
        let cur = Pos2::new(
            b0 * p0.x + b1 * p1.x + b2 * p2.x + b3 * p3.x,
            b0 * p0.y + b1 * p1.y + b2 * p2.y + b3 * p3.y,
        );
        painter.line_segment([prev, cur], Stroke::new(width, color));
        prev = cur;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v20_panels_default_has_valid_yaml_recipe() {
        let state = V20PanelsState::default();
        let parsed = parse_recipe_json_or_yaml(&state.recipe_yaml_editor).unwrap();
        assert!(!parsed.steps.is_empty());
        assert_eq!(state.reframe_target_w, 1080);
        assert_eq!(state.reframe_target_h, 1920);
    }
}
