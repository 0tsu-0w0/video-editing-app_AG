//! v7.0 #48: 大規模タイムライン最適化（10,000+ クリップ対応 フラット配列型 Augmented Interval Tree）
//!
//! - 構築計算量: $O(N \log N)$（中央値分割による完全平衡二分探索木・ヒープ断片化ゼロの単一 `Vec` 格納）
//! - 時刻刺し検索 (`query_at`) ＆ 区間重複検索 (`query_range`): $O(\log N + K)$
//! - `TimelineItem::contains`（半開区間 `[start, end)`）と 100% 同一の境界判定および `(layer_index, item_index)` 決定論順序を保証

use crate::project::{ItemId, Layer, Scene, SceneId};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// 区間木検索でヒットしたクリップ参照
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedClipRef {
    pub layer_index: usize,
    pub item_index: usize,
    pub item_id: ItemId,
    pub start: RationalTime,
    pub end: RationalTime,
}

#[derive(Debug, Clone, Copy)]
struct IntervalNode {
    start_ticks: i64,
    end_ticks: i64,
    subtree_min_start: i64,
    subtree_max_end: i64,
    left: Option<u32>,
    right: Option<u32>,
    layer_index: usize,
    item_index: usize,
    item_id: ItemId,
}

/// 単一レイヤーまたはシーン全体で共用できるフラット配列型 Augmented Interval Tree
#[derive(Debug, Clone, Default)]
pub struct FlatIntervalTree {
    nodes: Vec<IntervalNode>,
    root: Option<u32>,
}

impl FlatIntervalTree {
    fn from_raw_entries(mut entries: Vec<IndexedClipRef>) -> Self {
        if entries.is_empty() {
            return Self::default();
        }
        entries.sort_by(|a, b| {
            a.start
                .ticks
                .cmp(&b.start.ticks)
                .then_with(|| a.end.ticks.cmp(&b.end.ticks))
                .then_with(|| a.layer_index.cmp(&b.layer_index))
                .then_with(|| a.item_index.cmp(&b.item_index))
        });
        let mut nodes = Vec::with_capacity(entries.len());
        let root = Self::build_balanced(&entries, &mut nodes);
        Self { nodes, root }
    }

    fn build_balanced(slice: &[IndexedClipRef], out: &mut Vec<IntervalNode>) -> Option<u32> {
        if slice.is_empty() {
            return None;
        }
        let mid = slice.len() / 2;
        let cur = slice[mid];
        let idx = out.len() as u32;
        out.push(IntervalNode {
            start_ticks: cur.start.ticks,
            end_ticks: cur.end.ticks,
            subtree_min_start: cur.start.ticks,
            subtree_max_end: cur.end.ticks,
            left: None,
            right: None,
            layer_index: cur.layer_index,
            item_index: cur.item_index,
            item_id: cur.item_id,
        });

        let left = Self::build_balanced(&slice[..mid], out);
        let right = Self::build_balanced(&slice[mid + 1..], out);

        let mut min_s = cur.start.ticks;
        let mut max_e = cur.end.ticks;
        if let Some(li) = left {
            let ln = &out[li as usize];
            min_s = min_s.min(ln.subtree_min_start);
            max_e = max_e.max(ln.subtree_max_end);
        }
        if let Some(ri) = right {
            let rn = &out[ri as usize];
            min_s = min_s.min(rn.subtree_min_start);
            max_e = max_e.max(rn.subtree_max_end);
        }
        let node = &mut out[idx as usize];
        node.left = left;
        node.right = right;
        node.subtree_min_start = min_s;
        node.subtree_max_end = max_e;
        Some(idx)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 指定時刻 `t` を含む（`start <= t < end`）全エントリを検索し、`(hits, visited_nodes)` を返す
    pub fn query_point(&self, time: RationalTime) -> (Vec<IndexedClipRef>, usize) {
        let mut hits = Vec::new();
        let mut visited = 0usize;
        if let Some(root) = self.root {
            self.query_point_rec(root, time.ticks, &mut hits, &mut visited);
        }
        hits.sort_by(|a, b| {
            a.layer_index
                .cmp(&b.layer_index)
                .then_with(|| a.item_index.cmp(&b.item_index))
        });
        (hits, visited)
    }

    fn query_point_rec(
        &self,
        node_idx: u32,
        t: i64,
        hits: &mut Vec<IndexedClipRef>,
        visited: &mut usize,
    ) {
        let n = &self.nodes[node_idx as usize];
        if t < n.subtree_min_start || t >= n.subtree_max_end {
            return;
        }
        *visited += 1;

        if let Some(left) = n.left
            && self.nodes[left as usize].subtree_max_end > t
        {
            self.query_point_rec(left, t, hits, visited);
        }

        if t >= n.start_ticks && t < n.end_ticks {
            hits.push(IndexedClipRef {
                layer_index: n.layer_index,
                item_index: n.item_index,
                item_id: n.item_id,
                start: RationalTime::from_ticks(n.start_ticks),
                end: RationalTime::from_ticks(n.end_ticks),
            });
        }

        // 右部分木はすべて `start_ticks >= n.start_ticks` のため、`t >= n.start_ticks` のときのみ探索すれば十分
        if t >= n.start_ticks
            && let Some(right) = n.right
            && self.nodes[right as usize].subtree_max_end > t
        {
            self.query_point_rec(right, t, hits, visited);
        }
    }

    /// 半開区間 `[range_start, range_end)` と重なる（`item.start < range_end && item.end > range_start`）全エントリを検索する
    /// （`range_start == range_end` の場合は時刻点クエリと同様に `item.start <= range_start && item.end > range_start` として扱う）
    pub fn query_overlap(
        &self,
        range_start: RationalTime,
        range_end: RationalTime,
    ) -> (Vec<IndexedClipRef>, usize) {
        let s = range_start.ticks;
        let e = range_end.ticks.max(s);
        if s == e {
            return self.query_point(range_start);
        }
        let mut hits = Vec::new();
        let mut visited = 0usize;
        if let Some(root) = self.root {
            self.query_overlap_rec(root, s, e, &mut hits, &mut visited);
        }
        hits.sort_by(|a, b| {
            a.layer_index
                .cmp(&b.layer_index)
                .then_with(|| a.item_index.cmp(&b.item_index))
        });
        (hits, visited)
    }

    fn query_overlap_rec(
        &self,
        node_idx: u32,
        range_s: i64,
        range_e: i64,
        hits: &mut Vec<IndexedClipRef>,
        visited: &mut usize,
    ) {
        let n = &self.nodes[node_idx as usize];
        if n.subtree_max_end <= range_s || n.subtree_min_start >= range_e {
            return;
        }
        *visited += 1;

        if let Some(left) = n.left
            && self.nodes[left as usize].subtree_max_end > range_s
        {
            self.query_overlap_rec(left, range_s, range_e, hits, visited);
        }

        if n.start_ticks < range_e && n.end_ticks > range_s {
            hits.push(IndexedClipRef {
                layer_index: n.layer_index,
                item_index: n.item_index,
                item_id: n.item_id,
                start: RationalTime::from_ticks(n.start_ticks),
                end: RationalTime::from_ticks(n.end_ticks),
            });
        }

        // 右部分木はすべて `start_ticks >= n.start_ticks` のため、`n.start_ticks < range_e` のときのみ探索すれば十分
        if n.start_ticks < range_e
            && let Some(right) = n.right
            && self.nodes[right as usize].subtree_max_end > range_s
        {
            self.query_overlap_rec(right, range_s, range_e, hits, visited);
        }
    }
}

/// シーン全体＋各レイヤー別の $O(\log N)$ 区間木インデックス
#[derive(Debug, Clone, Default)]
pub struct SceneIntervalIndex {
    scene_tree: FlatIntervalTree,
    layer_trees: Vec<FlatIntervalTree>,
    total_clips: usize,
}

impl SceneIntervalIndex {
    pub fn build(scene: &Scene) -> Self {
        let mut all_entries = Vec::new();
        let mut layer_trees = Vec::with_capacity(scene.layers.len());

        for (layer_idx, layer) in scene.layers.iter().enumerate() {
            let layer_entries = collect_layer_entries(layer_idx, layer);
            all_entries.extend_from_slice(&layer_entries);
            layer_trees.push(FlatIntervalTree::from_raw_entries(layer_entries));
        }

        let total_clips = all_entries.len();
        let scene_tree = FlatIntervalTree::from_raw_entries(all_entries);
        Self {
            scene_tree,
            layer_trees,
            total_clips,
        }
    }

    pub fn total_clips(&self) -> usize {
        self.total_clips
    }

    pub fn layer_count(&self) -> usize {
        self.layer_trees.len()
    }

    pub fn query_at(&self, time: RationalTime) -> Vec<IndexedClipRef> {
        self.scene_tree.query_point(time).0
    }

    pub fn query_at_with_stats(&self, time: RationalTime) -> (Vec<IndexedClipRef>, usize) {
        self.scene_tree.query_point(time)
    }

    pub fn query_range(&self, start: RationalTime, end: RationalTime) -> Vec<IndexedClipRef> {
        self.scene_tree.query_overlap(start, end).0
    }

    pub fn query_range_with_stats(
        &self,
        start: RationalTime,
        end: RationalTime,
    ) -> (Vec<IndexedClipRef>, usize) {
        self.scene_tree.query_overlap(start, end)
    }

    /// 指定レイヤー上で時刻 `time` にアクティブな `item_index` 列を元のアイテム昇順で返す（`RenderGraph` 高速パス用）
    pub fn query_layer_indices_at(&self, layer_index: usize, time: RationalTime) -> Vec<usize> {
        let Some(tree) = self.layer_trees.get(layer_index) else {
            return Vec::new();
        };
        tree.query_point(time)
            .0
            .into_iter()
            .map(|r| r.item_index)
            .collect()
    }

    /// 指定レイヤー上でビューポート区間 `[start, end)` と重なる `item_index` 列を返す（`Timeline` 描画カリング用）
    pub fn query_layer_indices_in_range(
        &self,
        layer_index: usize,
        start: RationalTime,
        end: RationalTime,
    ) -> Vec<usize> {
        let Some(tree) = self.layer_trees.get(layer_index) else {
            return Vec::new();
        };
        tree.query_overlap(start, end)
            .0
            .into_iter()
            .map(|r| r.item_index)
            .collect()
    }
}

fn collect_layer_entries(layer_idx: usize, layer: &Layer) -> Vec<IndexedClipRef> {
    let mut entries = Vec::with_capacity(layer.items.len());
    for (item_idx, item) in layer.items.iter().enumerate() {
        let end = item.end();
        if end > item.start {
            entries.push(IndexedClipRef {
                layer_index: layer_idx,
                item_index: item_idx,
                item_id: item.id,
                start: item.start,
                end,
            });
        }
    }
    entries
}

/// `CommandBus::revision()` とシーン署名に連動して再構築を自動スキップするキャッシュ付き区間木
#[derive(Debug, Clone, Default)]
pub struct CachedSceneIntervalIndex {
    cached_key: Option<(SceneId, u64, usize, usize)>,
    index: SceneIntervalIndex,
    pub rebuild_count: u64,
}

impl CachedSceneIntervalIndex {
    pub fn get_or_build(&mut self, scene: &Scene, revision: u64) -> &SceneIntervalIndex {
        let item_total: usize = scene.layers.iter().map(|l| l.items.len()).sum();
        let key = (scene.id, revision, scene.layers.len(), item_total);
        if self.cached_key != Some(key) {
            self.index = SceneIntervalIndex::build(scene);
            self.cached_key = Some(key);
            self.rebuild_count += 1;
        }
        &self.index
    }

    pub fn invalidate(&mut self) {
        self.cached_key = None;
    }
}

/// 10,000+ クリップ区間木 vs 線形走査の実測ベンチマークレポート
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalIndexBenchmark {
    pub clip_count: usize,
    pub layer_count: usize,
    pub query_count: usize,
    pub build_ms: f64,
    pub tree_query_total_us: f64,
    pub linear_scan_total_us: f64,
    pub speedup_factor: f64,
    pub avg_visited_nodes: f64,
    pub exact_parity_verified: bool,
}

/// 任意のクリップ数（既定 10,000+）の合成タイムラインを生成し、$O(\log N)$ 区間木と $O(N)$ 線形走査の速度・完全一致性を検証する
pub fn benchmark_interval_index(clip_count: usize, query_count: usize) -> IntervalIndexBenchmark {
    use crate::project::{ItemKind, RgbaColor, ShapeKind, TimelineItem};
    use crate::time::FrameRate;

    let clips = clip_count.max(16);
    let queries = query_count.max(8);
    let layer_count = 8usize;
    let mut scene = Scene::new("10kBenchScene", 1920, 1080, FrameRate::FPS_60);
    while scene.layers.len() < layer_count {
        scene
            .layers
            .push(Layer::new(format!("Layer {}", scene.layers.len() + 1)));
    }

    for i in 0..clips {
        let l_idx = i % layer_count;
        let slot = (i / layer_count) as f64;
        let start_sec = slot * 1.25 + (l_idx as f64) * 0.15;
        let dur_sec = 0.95 + ((i % 5) as f64) * 0.25;
        let item = TimelineItem::new(
            format!("Clip_{i}"),
            RationalTime::from_seconds(start_sec),
            RationalTime::from_seconds(dur_sec),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 320.0,
                height: 180.0,
                color: RgbaColor::WHITE,
            },
        );
        scene.layers[l_idx].items.push(item);
    }

    let total_dur_sec = scene.total_duration().as_seconds().max(1.0);

    let t_build = Instant::now();
    let index = SceneIntervalIndex::build(&scene);
    let build_ms = t_build.elapsed().as_secs_f64() * 1000.0;

    let sample_times: Vec<RationalTime> = (0..queries)
        .map(|q| {
            let frac = (q as f64 + 0.37) / (queries as f64);
            RationalTime::from_seconds(frac * total_dur_sec)
        })
        .collect();

    let t_tree = Instant::now();
    let mut total_visited = 0usize;
    let mut tree_results = Vec::with_capacity(queries);
    for &qt in &sample_times {
        let (hits, visited) = index.query_at_with_stats(qt);
        total_visited += visited;
        tree_results.push(hits);
    }
    let tree_us = t_tree.elapsed().as_secs_f64() * 1_000_000.0;

    let t_lin = Instant::now();
    let mut linear_results = Vec::with_capacity(queries);
    for &qt in &sample_times {
        let mut lin_hits = Vec::new();
        for (l_idx, layer) in scene.layers.iter().enumerate() {
            for (i_idx, item) in layer.items.iter().enumerate() {
                if item.contains(qt) {
                    lin_hits.push(IndexedClipRef {
                        layer_index: l_idx,
                        item_index: i_idx,
                        item_id: item.id,
                        start: item.start,
                        end: item.end(),
                    });
                }
            }
        }
        linear_results.push(lin_hits);
    }
    let linear_us = t_lin.elapsed().as_secs_f64() * 1_000_000.0;

    let exact_parity_verified = tree_results == linear_results;
    let speedup_factor = if tree_us > 0.001 {
        linear_us / tree_us
    } else {
        1.0
    };

    IntervalIndexBenchmark {
        clip_count: clips,
        layer_count,
        query_count: queries,
        build_ms,
        tree_query_total_us: tree_us,
        linear_scan_total_us: linear_us,
        speedup_factor,
        avg_visited_nodes: (total_visited as f64) / (queries as f64),
        exact_parity_verified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ItemKind, RgbaColor, ShapeKind, TimelineItem};
    use crate::time::FrameRate;

    #[test]
    fn interval_tree_12000_clips_matches_linear_scan_100_percent_in_point_and_range() {
        let bench = benchmark_interval_index(12_000, 120);
        assert_eq!(bench.clip_count, 12_000);
        assert!(bench.exact_parity_verified);
        // 12,000 ノードに対して平均訪問ノード数は対数オーダー（十分小さい）であること
        assert!(
            bench.avg_visited_nodes < 120.0,
            "expected logarithmic node visits, got {}",
            bench.avg_visited_nodes
        );

        // Range overlap parity check on 2,000 overlapping clips
        let mut scene = Scene::new("OverlapScene", 1920, 1080, FrameRate::FPS_60);
        for i in 0..2_000 {
            let l = i % 4;
            let s = (i as f64) * 0.17;
            let d = 0.5 + ((i % 7) as f64) * 0.21;
            scene.layers[l].items.push(TimelineItem::new(
                format!("C{i}"),
                RationalTime::from_seconds(s),
                RationalTime::from_seconds(d),
                ItemKind::Shape {
                    shape: ShapeKind::Ellipse,
                    width: 64.0,
                    height: 64.0,
                    color: RgbaColor::WHITE,
                },
            ));
        }
        let idx = SceneIntervalIndex::build(&scene);
        for w in 0..25 {
            let rs = RationalTime::from_seconds(w as f64 * 12.5);
            let re = RationalTime::from_seconds(w as f64 * 12.5 + 8.3);
            let (tree_hits, visited) = idx.query_range_with_stats(rs, re);
            assert!(visited < 250, "visited too many nodes: {visited}");

            let mut expected = Vec::new();
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                for (i_idx, item) in layer.items.iter().enumerate() {
                    if item.start < re && item.end() > rs {
                        expected.push(IndexedClipRef {
                            layer_index: l_idx,
                            item_index: i_idx,
                            item_id: item.id,
                            start: item.start,
                            end: item.end(),
                        });
                    }
                }
            }
            assert_eq!(tree_hits, expected);
        }
    }
}
