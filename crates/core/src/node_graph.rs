use crate::project::{
    BlendMode, EffectNode, ItemKind, RgbaColor, Scene, TrackMatte, TrackMatteMode,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use thiserror::Error;
use uuid::Uuid;

pub type NodeId = Uuid;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NodeGraphError {
    #[error("Node graph contains a cycle involving node {0}")]
    CycleDetected(NodeId),
    #[error("Node not found: {0}")]
    NodeNotFound(NodeId),
    #[error("Connection references unknown port '{port}' on node {node}")]
    InvalidPort { node: NodeId, port: String },
    #[error("Cannot connect a node to itself: {0}")]
    SelfConnection(NodeId),
}

/// チャンネル分離ノードで抽出する成分（RGB / Alpha / Luma / 3D Depth）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSelector {
    #[default]
    Rgba,
    Red,
    Green,
    Blue,
    Alpha,
    Luma,
    Depth,
}

impl ChannelSelector {
    pub const ALL: &'static [Self] = &[
        Self::Rgba,
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::Alpha,
        Self::Luma,
        Self::Depth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rgba => "RGBA",
            Self::Red => "R (Red)",
            Self::Green => "G (Green)",
            Self::Blue => "B (Blue)",
            Self::Alpha => "A (Alpha)",
            Self::Luma => "Y (Luma)",
            Self::Depth => "Z (3D Depth)",
        }
    }
}

/// ノードベース合成グラフ（Fusion / Nuke スタイル）のノード種別
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "node_type", rename_all = "snake_case")]
pub enum CompositeNodeKind {
    /// タイムラインの指定レイヤー出力（映像＋3D深度）を取り込む入力ノード
    LayerInput { layer_index: usize },
    /// 単色ジェネレータ
    SolidColor { color: RgbaColor },
    /// 既存 RenderGraph / プラグイン / WASM エフェクトを実行するノード
    Effect {
        effect_type: String,
        #[serde(default)]
        params: BTreeMap<String, f32>,
    },
    /// チャンネル分離（R / G / B / Alpha / Luma / 3D Depth）
    ChannelSplit { channel: ChannelSelector },
    /// チャンネル結合（`r`, `g`, `b`, `a` または `in` + `mask` でアルファ置換）
    ChannelMerge,
    /// 2D 幾何変換ノード
    Transform {
        x: f32,
        y: f32,
        scale_x: f32,
        scale_y: f32,
        rotation_deg: f32,
        opacity: f32,
    },
    /// 2 系統合成ノード（`bg` + `fg` + 任意 `mask`）
    Merge { blend_mode: BlendMode, opacity: f32 },
    /// マスク合流ノード（`in` を `mask` で Alpha/Luma 抜き）
    MaskCombine { mode: TrackMatteMode },
    /// リニア色調補正ノード
    ColorCorrect {
        exposure: f32,
        contrast: f32,
        saturation: f32,
    },
    /// 3D 深度パス連動フォグ／大気合成ノード
    DepthFog {
        near: f32,
        far: f32,
        fog_color: RgbaColor,
        strength: f32,
    },
    /// 最終シーン出力ノード
    Output,
}

impl CompositeNodeKind {
    pub fn input_ports(&self) -> &'static [&'static str] {
        match self {
            Self::LayerInput { .. } | Self::SolidColor { .. } => &[],
            Self::Effect { .. }
            | Self::ChannelSplit { .. }
            | Self::Transform { .. }
            | Self::ColorCorrect { .. }
            | Self::DepthFog { .. }
            | Self::Output => &["in"],
            Self::ChannelMerge => &["r", "g", "b", "a"],
            Self::Merge { .. } => &["bg", "fg", "mask"],
            Self::MaskCombine { .. } => &["in", "mask"],
        }
    }

    pub fn output_ports(&self) -> &'static [&'static str] {
        match self {
            Self::Output => &[],
            _ => &["out"],
        }
    }

    pub fn category_name(&self) -> &'static str {
        match self {
            Self::LayerInput { .. } | Self::SolidColor { .. } => "Input",
            Self::Effect { .. } | Self::ColorCorrect { .. } => "Effect",
            Self::ChannelSplit { .. } | Self::ChannelMerge | Self::MaskCombine { .. } => {
                "Channel/Mask"
            }
            Self::Transform { .. } | Self::DepthFog { .. } => "Transform/3D",
            Self::Merge { .. } => "Merge",
            Self::Output => "Output",
        }
    }
}

/// ノードグラフ上の 1 ノード
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompositeNode {
    pub id: NodeId,
    pub name: String,
    /// ノードキャンバス上の座標 `[x, y]`
    pub position: [f32; 2],
    #[serde(default)]
    pub bypassed: bool,
    pub kind: CompositeNodeKind,
}

impl CompositeNode {
    pub fn new(name: impl Into<String>, position: [f32; 2], kind: CompositeNodeKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            position,
            bypassed: false,
            kind,
        }
    }
}

/// ノード間の有向エッジ（ポート単位で配線）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeConnection {
    pub from_node: NodeId,
    pub from_port: String,
    pub to_node: NodeId,
    pub to_port: String,
}

/// v2.0 #21: シーンに紐づくノードベース合成グラフ（Fusion / Nuke スタイル）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeCompositeGraph {
    /// true のとき `RenderGraph` が固定レイヤー順ではなく本ノードグラフで最終合成を実行する
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub nodes: Vec<CompositeNode>,
    #[serde(default)]
    pub connections: Vec<NodeConnection>,
}

/// ノードグラフからレイヤー編集空間へ双方向同期した結果サマリー
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeToLayerSyncSummary {
    pub updated_layer_offsets: usize,
    pub updated_track_mattes: usize,
    pub updated_blend_modes: usize,
    pub synced_effects: usize,
    pub updated_visibility: usize,
    pub preserved_advanced_nodes: usize,
}

impl NodeCompositeGraph {
    pub fn node(&self, id: NodeId) -> Option<&CompositeNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut CompositeNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn output_node_id(&self) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|n| matches!(n.kind, CompositeNodeKind::Output))
            .map(|n| n.id)
    }

    pub fn add_node(&mut self, node: CompositeNode) -> NodeId {
        let id = node.id;
        self.nodes.push(node);
        id
    }

    pub fn remove_node(&mut self, id: NodeId) -> bool {
        let before = self.nodes.len();
        self.nodes.retain(|n| n.id != id);
        self.connections
            .retain(|c| c.from_node != id && c.to_node != id);
        self.nodes.len() != before
    }

    /// ポート検証・サイクル検出付きでノード間を接続する（同一入力ポートへの既存接続は置換）
    pub fn connect(
        &mut self,
        from_node: NodeId,
        from_port: impl Into<String>,
        to_node: NodeId,
        to_port: impl Into<String>,
    ) -> Result<(), NodeGraphError> {
        if from_node == to_node {
            return Err(NodeGraphError::SelfConnection(from_node));
        }
        let from_port = from_port.into();
        let to_port = to_port.into();

        let src = self
            .node(from_node)
            .ok_or(NodeGraphError::NodeNotFound(from_node))?;
        if !src.kind.output_ports().contains(&from_port.as_str()) {
            return Err(NodeGraphError::InvalidPort {
                node: from_node,
                port: from_port,
            });
        }
        let dst = self
            .node(to_node)
            .ok_or(NodeGraphError::NodeNotFound(to_node))?;
        if !dst.kind.input_ports().contains(&to_port.as_str()) {
            return Err(NodeGraphError::InvalidPort {
                node: to_node,
                port: to_port,
            });
        }

        let prev = self.connections.clone();
        // 同一入力ポートへは 1 本の配線のみ許可
        self.connections
            .retain(|c| !(c.to_node == to_node && c.to_port == to_port));
        self.connections.push(NodeConnection {
            from_node,
            from_port,
            to_node,
            to_port,
        });

        if let Err(err) = self.topological_order() {
            self.connections = prev;
            return Err(err);
        }
        Ok(())
    }

    pub fn disconnect(&mut self, to_node: NodeId, to_port: &str) -> bool {
        let before = self.connections.len();
        self.connections
            .retain(|c| !(c.to_node == to_node && c.to_port == to_port));
        self.connections.len() != before
    }

    /// 指定ノードの指定入力ポートに接続している上流ノード ID を返す
    pub fn upstream_for(&self, to_node: NodeId, to_port: &str) -> Option<NodeId> {
        self.connections
            .iter()
            .find(|c| c.to_node == to_node && c.to_port == to_port)
            .map(|c| c.from_node)
    }

    /// Kahn のアルゴリズムによる決定論的トポロジカルソート＆サイクル検証
    pub fn topological_order(&self) -> Result<Vec<NodeId>, NodeGraphError> {
        let mut indegree: HashMap<NodeId, usize> =
            self.nodes.iter().map(|n| (n.id, 0usize)).collect();
        let mut adj: HashMap<NodeId, Vec<NodeId>> =
            self.nodes.iter().map(|n| (n.id, Vec::new())).collect();

        for conn in &self.connections {
            if !indegree.contains_key(&conn.from_node) {
                return Err(NodeGraphError::NodeNotFound(conn.from_node));
            }
            if !indegree.contains_key(&conn.to_node) {
                return Err(NodeGraphError::NodeNotFound(conn.to_node));
            }
            if let Some(list) = adj.get_mut(&conn.from_node) {
                list.push(conn.to_node);
            }
            if let Some(deg) = indegree.get_mut(&conn.to_node) {
                *deg += 1;
            }
        }

        // ノード定義順の決定論性を維持してキュー初期化
        let mut queue: VecDeque<NodeId> = VecDeque::new();
        for n in &self.nodes {
            if indegree.get(&n.id).copied().unwrap_or(0) == 0 {
                queue.push_back(n.id);
            }
        }

        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(u) = queue.pop_front() {
            order.push(u);
            if let Some(neighbors) = adj.get(&u) {
                for &v in neighbors {
                    if let Some(deg) = indegree.get_mut(&v) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(v);
                        }
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            let cycle_node = self
                .nodes
                .iter()
                .find(|n| indegree.get(&n.id).copied().unwrap_or(0) > 0)
                .map(|n| n.id)
                .unwrap_or_else(Uuid::nil);
            return Err(NodeGraphError::CycleDetected(cycle_node));
        }

        Ok(order)
    }

    /// 既存レイヤー構成（`Scene`）から Fusion / Nuke スタイルのノード合成グラフを自動生成する（双方向変換: Layer -> Node）
    pub fn from_scene_layers(scene: &Scene) -> Self {
        let mut graph = Self {
            enabled: true,
            nodes: Vec::new(),
            connections: Vec::new(),
        };

        // 視覚要素を持つレイヤー（または先頭 3 レイヤー）を抽出
        let mut visual_indices: Vec<usize> = scene
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let is_audio_only = !l.items.is_empty()
                    && l.items
                        .iter()
                        .all(|it| matches!(it.kind, ItemKind::Audio { .. }));
                !is_audio_only
            })
            .map(|(idx, _)| idx)
            .collect();

        if visual_indices.is_empty() && !scene.layers.is_empty() {
            visual_indices.push(0);
        }

        // トラックマット供給元として消費されるレイヤーをマーク
        let mut consumed_matte: HashMap<usize, bool> = HashMap::new();
        for layer in &scene.layers {
            if let Some(tm) = layer.track_matte {
                consumed_matte.insert(tm.source_layer_index, true);
            }
        }

        let mut layer_node_ids: HashMap<usize, NodeId> = HashMap::new();
        for (row, &layer_idx) in visual_indices.iter().enumerate() {
            let layer = &scene.layers[layer_idx];
            let mut node = CompositeNode::new(
                format!("L{}: {}", layer_idx + 1, layer.name),
                [40.0, 50.0 + row as f32 * 130.0],
                CompositeNodeKind::LayerInput {
                    layer_index: layer_idx,
                },
            );
            node.bypassed = !layer.visible;
            let nid = graph.add_node(node);
            layer_node_ids.insert(layer_idx, nid);
        }

        // 各レイヤーのトラックマット・オフセットを中間ノードへ展開
        let mut prepared_streams: Vec<(usize, NodeId, BlendMode)> = Vec::new();
        for (row, &layer_idx) in visual_indices.iter().enumerate() {
            if consumed_matte.get(&layer_idx).copied().unwrap_or(false) {
                continue;
            }
            let layer = &scene.layers[layer_idx];
            let mut cur_id = layer_node_ids[&layer_idx];
            let base_y = 50.0 + row as f32 * 130.0;

            if layer.offset_x.abs() > 0.01 || layer.offset_y.abs() > 0.01 {
                let tf_id = graph.add_node(CompositeNode::new(
                    format!("Transform L{}", layer_idx + 1),
                    [240.0, base_y],
                    CompositeNodeKind::Transform {
                        x: layer.offset_x,
                        y: layer.offset_y,
                        scale_x: 1.0,
                        scale_y: 1.0,
                        rotation_deg: 0.0,
                        opacity: 1.0,
                    },
                ));
                let _ = graph.connect(cur_id, "out", tf_id, "in");
                cur_id = tf_id;
            }

            if let Some(tm) = layer.track_matte
                && let Some(&matte_node) = layer_node_ids.get(&tm.source_layer_index)
            {
                let mask_id = graph.add_node(CompositeNode::new(
                    format!("Matte L{}", layer_idx + 1),
                    [420.0, base_y],
                    CompositeNodeKind::MaskCombine { mode: tm.mode },
                ));
                let _ = graph.connect(cur_id, "out", mask_id, "in");
                let _ = graph.connect(matte_node, "out", mask_id, "mask");
                cur_id = mask_id;
            }

            let blend = layer
                .items
                .iter()
                .find(|it| !matches!(it.kind, ItemKind::Audio { .. }))
                .map(|it| it.blend_mode)
                .unwrap_or(BlendMode::Normal);
            prepared_streams.push((layer_idx, cur_id, blend));
        }

        // 左→右へ Merge チェーンで合流し Output へ接続
        let mut acc_node: Option<NodeId> = None;
        for (step, (layer_idx, stream_id, blend_mode)) in prepared_streams.into_iter().enumerate() {
            if let Some(bg_id) = acc_node {
                let merge_id = graph.add_node(CompositeNode::new(
                    format!("Merge L{}", layer_idx + 1),
                    [
                        620.0 + (step.saturating_sub(1)) as f32 * 190.0,
                        120.0 + step as f32 * 40.0,
                    ],
                    CompositeNodeKind::Merge {
                        blend_mode,
                        opacity: 1.0,
                    },
                ));
                let _ = graph.connect(bg_id, "out", merge_id, "bg");
                let _ = graph.connect(stream_id, "out", merge_id, "fg");
                acc_node = Some(merge_id);
            } else {
                acc_node = Some(stream_id);
            }
        }

        let out_x = 660.0 + (graph.nodes.len() as f32 * 40.0).min(480.0);
        let output_id = graph.add_node(CompositeNode::new(
            "MediaOut (Scene Output)",
            [out_x, 180.0],
            CompositeNodeKind::Output,
        ));
        if let Some(last_id) = acc_node {
            let _ = graph.connect(last_id, "out", output_id, "in");
        }

        graph.auto_layout();
        graph
    }

    /// トポロジカル深度に基づきノード座標を左→右の綺麗な DAG レイアウトに整列する
    pub fn auto_layout(&mut self) {
        let Ok(order) = self.topological_order() else {
            return;
        };
        let mut depth: HashMap<NodeId, usize> = HashMap::new();
        for &nid in &order {
            let max_parent = self
                .connections
                .iter()
                .filter(|c| c.to_node == nid)
                .filter_map(|c| depth.get(&c.from_node).copied())
                .max()
                .map(|d| d + 1)
                .unwrap_or(0);
            depth.insert(nid, max_parent);
        }

        let mut col_counts: BTreeMap<usize, usize> = BTreeMap::new();
        for &nid in &order {
            let col = depth.get(&nid).copied().unwrap_or(0);
            let row = *col_counts.entry(col).or_insert(0);
            *col_counts.entry(col).or_insert(0) += 1;
            if let Some(node) = self.node_mut(nid) {
                node.position = [40.0 + col as f32 * 220.0, 50.0 + row as f32 * 130.0];
            }
        }
    }
}

/// 双方向変換（Node -> Layer）: ノード合成グラフ上のレイヤー互換変更（可視性・Transform・Effect・MaskCombine・Merge ブレンドモード）を `Scene.layers` へ反映する
pub fn sync_node_graph_to_scene_layers(
    graph: &NodeCompositeGraph,
    scene: &mut Scene,
) -> NodeToLayerSyncSummary {
    let mut summary = NodeToLayerSyncSummary::default();

    // 各ノードの直上 LayerInput インデックスを追跡するマップ
    let mut node_layer_origin: HashMap<NodeId, usize> = HashMap::new();
    let order = graph
        .topological_order()
        .unwrap_or_else(|_| graph.nodes.iter().map(|n| n.id).collect());

    for nid in order {
        let Some(node) = graph.node(nid) else {
            continue;
        };
        match &node.kind {
            CompositeNodeKind::LayerInput { layer_index } => {
                node_layer_origin.insert(nid, *layer_index);
                if let Some(layer) = scene.layers.get_mut(*layer_index) {
                    let desired_visible = !node.bypassed;
                    if layer.visible != desired_visible {
                        layer.visible = desired_visible;
                        summary.updated_visibility += 1;
                    }
                }
            }
            CompositeNodeKind::Transform { x, y, .. } => {
                if let Some(src_id) = graph.upstream_for(nid, "in")
                    && let Some(&layer_idx) = node_layer_origin.get(&src_id)
                {
                    node_layer_origin.insert(nid, layer_idx);
                    if !node.bypassed
                        && let Some(layer) = scene.layers.get_mut(layer_idx)
                    {
                        layer.offset_x = *x;
                        layer.offset_y = *y;
                        summary.updated_layer_offsets += 1;
                    }
                } else {
                    summary.preserved_advanced_nodes += 1;
                }
            }
            CompositeNodeKind::Effect {
                effect_type,
                params,
            } => {
                if let Some(src_id) = graph.upstream_for(nid, "in")
                    && let Some(&layer_idx) = node_layer_origin.get(&src_id)
                {
                    node_layer_origin.insert(nid, layer_idx);
                    if !node.bypassed
                        && let Some(layer) = scene.layers.get_mut(layer_idx)
                    {
                        for item in &mut layer.items {
                            if let Some(existing) = item
                                .effect_chain
                                .iter_mut()
                                .find(|e| e.effect_type == *effect_type)
                            {
                                existing.params = params.clone();
                                existing.enabled = true;
                            } else {
                                item.effect_chain.push(EffectNode {
                                    id: Uuid::new_v4(),
                                    effect_type: effect_type.clone(),
                                    enabled: true,
                                    params: params.clone(),
                                });
                            }
                        }
                        summary.synced_effects += 1;
                    }
                } else {
                    summary.preserved_advanced_nodes += 1;
                }
            }
            CompositeNodeKind::MaskCombine { mode } => {
                let in_layer = graph
                    .upstream_for(nid, "in")
                    .and_then(|id| node_layer_origin.get(&id).copied());
                let mask_layer = graph
                    .upstream_for(nid, "mask")
                    .and_then(|id| node_layer_origin.get(&id).copied());
                if let (Some(target_idx), Some(src_idx)) = (in_layer, mask_layer) {
                    node_layer_origin.insert(nid, target_idx);
                    if !node.bypassed
                        && target_idx != src_idx
                        && let Some(layer) = scene.layers.get_mut(target_idx)
                    {
                        layer.track_matte = Some(TrackMatte {
                            mode: *mode,
                            source_layer_index: src_idx,
                        });
                        summary.updated_track_mattes += 1;
                    }
                } else {
                    summary.preserved_advanced_nodes += 1;
                }
            }
            CompositeNodeKind::Merge { blend_mode, .. } => {
                let fg_layer = graph
                    .upstream_for(nid, "fg")
                    .and_then(|id| node_layer_origin.get(&id).copied());
                if let Some(layer_idx) = fg_layer
                    && !node.bypassed
                    && let Some(layer) = scene.layers.get_mut(layer_idx)
                {
                    for item in &mut layer.items {
                        item.blend_mode = *blend_mode;
                    }
                    summary.updated_blend_modes += 1;
                }
            }
            CompositeNodeKind::ChannelSplit { .. }
            | CompositeNodeKind::ChannelMerge
            | CompositeNodeKind::SolidColor { .. }
            | CompositeNodeKind::ColorCorrect { .. }
            | CompositeNodeKind::DepthFog { .. } => {
                summary.preserved_advanced_nodes += 1;
            }
            CompositeNodeKind::Output => {}
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ShapeKind, TimelineItem};
    use crate::time::{FrameRate, RationalTime};

    #[test]
    fn from_scene_layers_builds_valid_dag_and_roundtrips_back_to_layers() {
        let mut scene = Scene::new("NodeTest", 1280, 720, FrameRate::FPS_60);
        scene.layers[0].items.push(TimelineItem::new(
            "BG",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 400.0,
                height: 300.0,
                color: RgbaColor::WHITE,
            },
        ));
        scene.layers[1].items.push(TimelineItem::new(
            "FG",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 200.0,
                height: 200.0,
                color: RgbaColor::WHITE,
            },
        ));
        scene.layers[1].offset_x = 42.0;

        let mut graph = NodeCompositeGraph::from_scene_layers(&scene);
        assert!(graph.enabled);
        assert!(graph.output_node_id().is_some());
        let order = graph.topological_order().expect("valid DAG");
        assert_eq!(order.len(), graph.nodes.len());

        // Transform ノードのオフセットを変更し、Merge を Screen に変更してレイヤーへ逆同期
        for node in &mut graph.nodes {
            if let CompositeNodeKind::Transform { x, .. } = &mut node.kind {
                *x = 128.0;
            }
            if let CompositeNodeKind::Merge { blend_mode, .. } = &mut node.kind {
                *blend_mode = BlendMode::Screen;
            }
        }
        let sync = sync_node_graph_to_scene_layers(&graph, &mut scene);
        assert_eq!(sync.updated_layer_offsets, 1);
        assert!((scene.layers[1].offset_x - 128.0).abs() < 1e-4);
        assert!(sync.updated_blend_modes >= 1);
        assert_eq!(scene.layers[1].items[0].blend_mode, BlendMode::Screen);
    }

    #[test]
    fn rejects_cycles_and_invalid_ports() {
        let mut graph = NodeCompositeGraph::default();
        let a = graph.add_node(CompositeNode::new(
            "BlurA",
            [0.0, 0.0],
            CompositeNodeKind::Effect {
                effect_type: "blur".into(),
                params: BTreeMap::new(),
            },
        ));
        let b = graph.add_node(CompositeNode::new(
            "BlurB",
            [200.0, 0.0],
            CompositeNodeKind::Effect {
                effect_type: "blur".into(),
                params: BTreeMap::new(),
            },
        ));
        assert!(graph.connect(a, "out", b, "in").is_ok());
        // 逆向き配線はサイクルとして拒否され、既存接続は保持される
        assert!(matches!(
            graph.connect(b, "out", a, "in"),
            Err(NodeGraphError::CycleDetected(_))
        ));
        assert_eq!(graph.connections.len(), 1);
        assert!(matches!(
            graph.connect(a, "nope", b, "in"),
            Err(NodeGraphError::InvalidPort { .. })
        ));
    }
}
