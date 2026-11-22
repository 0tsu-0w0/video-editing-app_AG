use crate::command::EditCommand;
use crate::project::{
    ItemId, Layer, LayerId, Project, RgbaColor, Scene, SceneId, TimelineItem, TimelineMarker,
};
use crate::time::{FrameRate, RationalTime};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

// =========================================================================
// 1. Semantic Timeline Visual Diff (Git-Friendly NLE)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemDiff {
    pub scene_id: SceneId,
    pub scene_name: String,
    pub layer_index: usize,
    pub item_id: ItemId,
    pub item_name: String,
    pub kind: DiffChangeKind,
    pub start_sec: f64,
    pub duration_sec: f64,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerDiff {
    pub scene_id: SceneId,
    pub layer_id: LayerId,
    pub layer_name: String,
    pub kind: DiffChangeKind,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkerDiff {
    pub scene_id: SceneId,
    pub marker_id: Uuid,
    pub label: String,
    pub time_sec: f64,
    pub kind: DiffChangeKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneDiff {
    pub scene_id: SceneId,
    pub scene_name: String,
    pub kind: DiffChangeKind,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectDiff {
    pub base_name: String,
    pub target_name: String,
    pub scene_diffs: Vec<SceneDiff>,
    pub layer_diffs: Vec<LayerDiff>,
    pub item_diffs: Vec<ItemDiff>,
    pub marker_diffs: Vec<MarkerDiff>,
    pub assets_added: usize,
    pub assets_removed: usize,
}

impl ProjectDiff {
    pub fn total_changes(&self) -> usize {
        self.scene_diffs.len()
            + self.layer_diffs.len()
            + self.item_diffs.len()
            + self.marker_diffs.len()
            + self.assets_added
            + self.assets_removed
    }

    pub fn is_empty(&self) -> bool {
        self.total_changes() == 0
    }
}

/// 2 つのプロジェクト（ベース vs 比較先）のタイムライン意味差分を算出する
pub fn diff_projects(base: &Project, target: &Project) -> ProjectDiff {
    let mut out = ProjectDiff {
        base_name: base.name.clone(),
        target_name: target.name.clone(),
        ..Default::default()
    };

    let base_assets: BTreeSet<Uuid> = base.assets.iter().map(|a| a.id).collect();
    let target_assets: BTreeSet<Uuid> = target.assets.iter().map(|a| a.id).collect();
    out.assets_added = target_assets.difference(&base_assets).count();
    out.assets_removed = base_assets.difference(&target_assets).count();

    let base_scenes: HashMap<SceneId, &Scene> = base.scenes.iter().map(|s| (s.id, s)).collect();
    let target_scenes: HashMap<SceneId, &Scene> = target.scenes.iter().map(|s| (s.id, s)).collect();

    for ts in &target.scenes {
        match base_scenes.get(&ts.id) {
            None => {
                out.scene_diffs.push(SceneDiff {
                    scene_id: ts.id,
                    scene_name: ts.name.clone(),
                    kind: DiffChangeKind::Added,
                    changed_fields: vec!["scene_added".into()],
                });
                for (lidx, layer) in ts.layers.iter().enumerate() {
                    for it in &layer.items {
                        out.item_diffs.push(ItemDiff {
                            scene_id: ts.id,
                            scene_name: ts.name.clone(),
                            layer_index: lidx,
                            item_id: it.id,
                            item_name: it.name.clone(),
                            kind: DiffChangeKind::Added,
                            start_sec: it.start.as_seconds(),
                            duration_sec: it.duration.as_seconds(),
                            changed_fields: vec!["new_scene_item".into()],
                        });
                    }
                }
            }
            Some(bs) => {
                let mut scene_fields = Vec::new();
                if bs.name != ts.name {
                    scene_fields.push(format!("name: {} -> {}", bs.name, ts.name));
                }
                if bs.width != ts.width || bs.height != ts.height {
                    scene_fields.push(format!(
                        "resolution: {}x{} -> {}x{}",
                        bs.width, bs.height, ts.width, ts.height
                    ));
                }
                if bs.fps != ts.fps {
                    scene_fields.push(format!(
                        "fps: {:.2} -> {:.2}",
                        bs.fps.as_f64(),
                        ts.fps.as_f64()
                    ));
                }
                if bs.color_grade != ts.color_grade {
                    scene_fields.push("color_grade".into());
                }
                if bs.motion_blur != ts.motion_blur {
                    scene_fields.push("motion_blur".into());
                }
                if bs.audio_ducking != ts.audio_ducking {
                    scene_fields.push("audio_ducking".into());
                }
                if bs.node_graph != ts.node_graph {
                    scene_fields.push("node_graph".into());
                }
                if !scene_fields.is_empty() {
                    out.scene_diffs.push(SceneDiff {
                        scene_id: ts.id,
                        scene_name: ts.name.clone(),
                        kind: DiffChangeKind::Modified,
                        changed_fields: scene_fields,
                    });
                }

                diff_scene_layers_and_items(bs, ts, &mut out);
                diff_scene_markers(bs, ts, &mut out);
            }
        }
    }

    for bs in &base.scenes {
        if !target_scenes.contains_key(&bs.id) {
            out.scene_diffs.push(SceneDiff {
                scene_id: bs.id,
                scene_name: bs.name.clone(),
                kind: DiffChangeKind::Removed,
                changed_fields: vec!["scene_removed".into()],
            });
        }
    }

    out
}

fn diff_scene_layers_and_items(bs: &Scene, ts: &Scene, out: &mut ProjectDiff) {
    let base_layers: HashMap<LayerId, &Layer> = bs.layers.iter().map(|l| (l.id, l)).collect();
    let target_layers: HashMap<LayerId, &Layer> = ts.layers.iter().map(|l| (l.id, l)).collect();

    for tl in &ts.layers {
        match base_layers.get(&tl.id) {
            None => out.layer_diffs.push(LayerDiff {
                scene_id: ts.id,
                layer_id: tl.id,
                layer_name: tl.name.clone(),
                kind: DiffChangeKind::Added,
                changed_fields: vec!["layer_added".into()],
            }),
            Some(bl) => {
                let mut lf = Vec::new();
                if bl.name != tl.name {
                    lf.push("name".into());
                }
                if bl.visible != tl.visible {
                    lf.push("visible".into());
                }
                if bl.muted != tl.muted {
                    lf.push("muted".into());
                }
                if bl.solo != tl.solo {
                    lf.push("solo".into());
                }
                if (bl.volume - tl.volume).abs() > 1e-4 {
                    lf.push("volume".into());
                }
                if (bl.pan - tl.pan).abs() > 1e-4 {
                    lf.push("pan".into());
                }
                if (bl.offset_x - tl.offset_x).abs() > 1e-3
                    || (bl.offset_y - tl.offset_y).abs() > 1e-3
                {
                    lf.push("offset".into());
                }
                if bl.track_matte != tl.track_matte {
                    lf.push("track_matte".into());
                }
                if !lf.is_empty() {
                    out.layer_diffs.push(LayerDiff {
                        scene_id: ts.id,
                        layer_id: tl.id,
                        layer_name: tl.name.clone(),
                        kind: DiffChangeKind::Modified,
                        changed_fields: lf,
                    });
                }
            }
        }
    }
    for bl in &bs.layers {
        if !target_layers.contains_key(&bl.id) {
            out.layer_diffs.push(LayerDiff {
                scene_id: bs.id,
                layer_id: bl.id,
                layer_name: bl.name.clone(),
                kind: DiffChangeKind::Removed,
                changed_fields: vec!["layer_removed".into()],
            });
        }
    }

    // アイテム単位の差分（レイヤー移動も含めて ItemId で照合）
    let mut base_items: HashMap<ItemId, (usize, &TimelineItem)> = HashMap::new();
    for (lidx, l) in bs.layers.iter().enumerate() {
        for it in &l.items {
            base_items.insert(it.id, (lidx, it));
        }
    }
    let mut seen_ids: BTreeSet<ItemId> = BTreeSet::new();
    for (lidx, l) in ts.layers.iter().enumerate() {
        for it in &l.items {
            seen_ids.insert(it.id);
            match base_items.get(&it.id) {
                None => out.item_diffs.push(ItemDiff {
                    scene_id: ts.id,
                    scene_name: ts.name.clone(),
                    layer_index: lidx,
                    item_id: it.id,
                    item_name: it.name.clone(),
                    kind: DiffChangeKind::Added,
                    start_sec: it.start.as_seconds(),
                    duration_sec: it.duration.as_seconds(),
                    changed_fields: vec!["added".into()],
                }),
                Some(&(old_lidx, old_it)) => {
                    let fields = diff_item_fields(old_lidx, old_it, lidx, it);
                    if !fields.is_empty() {
                        out.item_diffs.push(ItemDiff {
                            scene_id: ts.id,
                            scene_name: ts.name.clone(),
                            layer_index: lidx,
                            item_id: it.id,
                            item_name: it.name.clone(),
                            kind: DiffChangeKind::Modified,
                            start_sec: it.start.as_seconds(),
                            duration_sec: it.duration.as_seconds(),
                            changed_fields: fields,
                        });
                    }
                }
            }
        }
    }
    for (id, (old_lidx, old_it)) in base_items {
        if !seen_ids.contains(&id) {
            out.item_diffs.push(ItemDiff {
                scene_id: bs.id,
                scene_name: bs.name.clone(),
                layer_index: old_lidx,
                item_id: id,
                item_name: old_it.name.clone(),
                kind: DiffChangeKind::Removed,
                start_sec: old_it.start.as_seconds(),
                duration_sec: old_it.duration.as_seconds(),
                changed_fields: vec!["removed".into()],
            });
        }
    }
    out.item_diffs.sort_by(|a, b| {
        a.start_sec
            .total_cmp(&b.start_sec)
            .then_with(|| a.layer_index.cmp(&b.layer_index))
    });
}

fn diff_item_fields(
    old_layer: usize,
    old_it: &TimelineItem,
    new_layer: usize,
    new_it: &TimelineItem,
) -> Vec<String> {
    let mut fields = Vec::new();
    if old_layer != new_layer {
        fields.push(format!("layer: L{} -> L{}", old_layer + 1, new_layer + 1));
    }
    if old_it.name != new_it.name {
        fields.push("name".into());
    }
    if old_it.start != new_it.start {
        fields.push(format!(
            "start: {:.2}s -> {:.2}s",
            old_it.start.as_seconds(),
            new_it.start.as_seconds()
        ));
    }
    if old_it.duration != new_it.duration {
        fields.push(format!(
            "duration: {:.2}s -> {:.2}s",
            old_it.duration.as_seconds(),
            new_it.duration.as_seconds()
        ));
    }
    if old_it.transform != new_it.transform {
        fields.push("transform".into());
    }
    if old_it.transform_3d != new_it.transform_3d {
        fields.push("transform_3d".into());
    }
    if old_it.blend_mode != new_it.blend_mode {
        fields.push("blend_mode".into());
    }
    if old_it.keyframes != new_it.keyframes {
        fields.push("keyframes".into());
    }
    if old_it.effect_chain != new_it.effect_chain {
        fields.push("effects".into());
    }
    if old_it.audio_effects != new_it.audio_effects {
        fields.push("audio_effects".into());
    }
    if old_it.expressions != new_it.expressions {
        fields.push("expressions".into());
    }
    if old_it.masks != new_it.masks {
        fields.push("masks".into());
    }
    if old_it.playback != new_it.playback {
        fields.push("playback".into());
    }
    if old_it.text_style != new_it.text_style {
        fields.push("text_style".into());
    }
    if old_it.kind != new_it.kind {
        fields.push("content/kind".into());
    }
    fields
}

fn diff_scene_markers(bs: &Scene, ts: &Scene, out: &mut ProjectDiff) {
    let base_m: HashMap<Uuid, &TimelineMarker> = bs.markers.iter().map(|m| (m.id, m)).collect();
    let target_m: HashMap<Uuid, &TimelineMarker> = ts.markers.iter().map(|m| (m.id, m)).collect();
    for tm in &ts.markers {
        match base_m.get(&tm.id) {
            None => out.marker_diffs.push(MarkerDiff {
                scene_id: ts.id,
                marker_id: tm.id,
                label: tm.label.clone(),
                time_sec: tm.time.as_seconds(),
                kind: DiffChangeKind::Added,
            }),
            Some(bm) if *bm != tm => out.marker_diffs.push(MarkerDiff {
                scene_id: ts.id,
                marker_id: tm.id,
                label: tm.label.clone(),
                time_sec: tm.time.as_seconds(),
                kind: DiffChangeKind::Modified,
            }),
            _ => {}
        }
    }
    for bm in &bs.markers {
        if !target_m.contains_key(&bm.id) {
            out.marker_diffs.push(MarkerDiff {
                scene_id: bs.id,
                marker_id: bm.id,
                label: bm.label.clone(),
                time_sec: bm.time.as_seconds(),
                kind: DiffChangeKind::Removed,
            });
        }
    }
}

/// タイムライン差分を人間・PR 向けに読みやすいテキストへ整形する
pub fn format_diff_report(diff: &ProjectDiff) -> String {
    if diff.is_empty() {
        return format!("差分なし ({} == {})", diff.base_name, diff.target_name);
    }
    let mut lines = vec![format!(
        "=== Timeline Diff: {} -> {} (合計 {} 件) ===",
        diff.base_name,
        diff.target_name,
        diff.total_changes()
    )];
    for sd in &diff.scene_diffs {
        let sym = diff_symbol(sd.kind);
        lines.push(format!(
            "{sym} [Scene] {} ({})",
            sd.scene_name,
            sd.changed_fields.join(", ")
        ));
    }
    for ld in &diff.layer_diffs {
        let sym = diff_symbol(ld.kind);
        lines.push(format!(
            "{sym} [Layer] {} ({})",
            ld.layer_name,
            ld.changed_fields.join(", ")
        ));
    }
    for id in &diff.item_diffs {
        let sym = diff_symbol(id.kind);
        lines.push(format!(
            "{sym} [Item L{} @{:.2}s] {} ({})",
            id.layer_index + 1,
            id.start_sec,
            id.item_name,
            id.changed_fields.join(", ")
        ));
    }
    for md in &diff.marker_diffs {
        let sym = diff_symbol(md.kind);
        lines.push(format!("{sym} [Marker @{:.2}s] {}", md.time_sec, md.label));
    }
    lines.join("\n")
}

fn diff_symbol(kind: DiffChangeKind) -> &'static str {
    match kind {
        DiffChangeKind::Added => "+",
        DiffChangeKind::Removed => "-",
        DiffChangeKind::Modified => "~",
    }
}

// =========================================================================
// 2. 3-Way Semantic Branch Merge (Git-Friendly NLE)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    ItemBothModified,
    ItemModifiedVsDeleted,
    SceneSettingsConflict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeConflict {
    pub conflict_id: String,
    pub scene_id: SceneId,
    pub item_id: Option<ItemId>,
    pub kind: ConflictKind,
    pub description: String,
    pub ours_summary: String,
    pub theirs_summary: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    #[default]
    KeepOurs,
    KeepTheirs,
    /// 両ブランチの変更を残す（アイテム衝突時は相手のアイテムを新 ID で複製追加）
    KeepBothDuplicate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreeWayMergeResult {
    pub merged_project: Project,
    pub auto_merged_from_theirs: usize,
    pub conflicts: Vec<MergeConflict>,
}

/// ベース・自ブランチ (`ours`)・相手ブランチ (`theirs`) の 3-Way 意味的マージを実行する
pub fn merge_projects_three_way(
    base: &Project,
    ours: &Project,
    theirs: &Project,
    resolutions: &BTreeMap<String, ConflictResolution>,
) -> ThreeWayMergeResult {
    let mut merged = ours.clone();
    let mut auto_merged = 0usize;
    let mut conflicts = Vec::new();

    // 1. アセットのユニオンマージ
    let existing_assets: BTreeSet<Uuid> = merged.assets.iter().map(|a| a.id).collect();
    for asset in &theirs.assets {
        if !existing_assets.contains(&asset.id) {
            merged.assets.push(asset.clone());
            auto_merged += 1;
        }
    }

    let base_scenes: HashMap<SceneId, &Scene> = base.scenes.iter().map(|s| (s.id, s)).collect();
    let ours_scene_ids: BTreeSet<SceneId> = merged.scenes.iter().map(|s| s.id).collect();

    // 2. 相手ブランチで新規追加されたシーンを取り込む
    for ts in &theirs.scenes {
        if !ours_scene_ids.contains(&ts.id) && !base_scenes.contains_key(&ts.id) {
            merged.scenes.push(ts.clone());
            auto_merged += 1;
        }
    }

    // 3. 共通シーンの意味的 3-Way マージ
    for os in &mut merged.scenes {
        let Some(ts) = theirs.scenes.iter().find(|s| s.id == os.id) else {
            continue;
        };
        let bs = base_scenes.get(&os.id).copied();

        // シーン設定（カラーグレーディング・モーションブラー・ダッキング・ノードグラフ）
        if let Some(b) = bs {
            if os.color_grade == b.color_grade && ts.color_grade != b.color_grade {
                os.color_grade = ts.color_grade.clone();
                auto_merged += 1;
            } else if os.color_grade != b.color_grade
                && ts.color_grade != b.color_grade
                && os.color_grade != ts.color_grade
            {
                let cid = format!("scene_grade_{}", os.id);
                let res = resolutions.get(&cid).copied();
                if res == Some(ConflictResolution::KeepTheirs) {
                    os.color_grade = ts.color_grade.clone();
                } else if res.is_none() {
                    conflicts.push(MergeConflict {
                        conflict_id: cid,
                        scene_id: os.id,
                        item_id: None,
                        kind: ConflictKind::SceneSettingsConflict,
                        description: format!(
                            "シーン「{}」のカラーグレーディングが両ブランチで変更されました",
                            os.name
                        ),
                        ours_summary: format!("Ours (LUT={:?})", os.color_grade.lut_path),
                        theirs_summary: format!("Theirs (LUT={:?})", ts.color_grade.lut_path),
                    });
                }
            }

            if os.node_graph == b.node_graph && ts.node_graph != b.node_graph {
                os.node_graph = ts.node_graph.clone();
                auto_merged += 1;
            }
            if os.motion_blur == b.motion_blur && ts.motion_blur != b.motion_blur {
                os.motion_blur = ts.motion_blur;
                auto_merged += 1;
            }
            if os.audio_ducking == b.audio_ducking && ts.audio_ducking != b.audio_ducking {
                os.audio_ducking = ts.audio_ducking;
                auto_merged += 1;
            }
        }

        // レイヤー数が足りない場合は拡張
        while os.layers.len() < ts.layers.len() {
            let idx = os.layers.len();
            let mut new_layer = ts.layers[idx].clone();
            new_layer.items.clear();
            os.layers.push(new_layer);
        }

        // マーカー＆レビューコメントのユニオン＋更新マージ
        merge_markers_three_way(bs, os, ts, &mut auto_merged);
        merge_reviews_three_way(os, ts, &mut auto_merged);

        // アイテム単位の 3-Way マージ
        let mut base_items: HashMap<ItemId, (usize, &TimelineItem)> = HashMap::new();
        if let Some(b) = bs {
            for (li, l) in b.layers.iter().enumerate() {
                for it in &l.items {
                    base_items.insert(it.id, (li, it));
                }
            }
        }
        let mut ours_items: HashMap<ItemId, (usize, TimelineItem)> = HashMap::new();
        for (li, l) in os.layers.iter().enumerate() {
            for it in &l.items {
                ours_items.insert(it.id, (li, it.clone()));
            }
        }
        let mut theirs_items: HashMap<ItemId, (usize, &TimelineItem)> = HashMap::new();
        for (li, l) in ts.layers.iter().enumerate() {
            for it in &l.items {
                theirs_items.insert(it.id, (li, it));
            }
        }

        let mut all_ids: BTreeSet<ItemId> = BTreeSet::new();
        all_ids.extend(base_items.keys().copied());
        all_ids.extend(ours_items.keys().copied());
        all_ids.extend(theirs_items.keys().copied());

        for id in all_ids {
            let b_opt = base_items.get(&id).copied();
            let o_opt = ours_items.get(&id).cloned();
            let t_opt = theirs_items.get(&id).copied();

            match (b_opt, o_opt, t_opt) {
                // 相手ブランチで新規追加
                (None, None, Some((t_li, t_item))) => {
                    let target_li = t_li.min(os.layers.len().saturating_sub(1));
                    os.layers[target_li].items.push(t_item.clone());
                    auto_merged += 1;
                }
                // 相手ブランチのみが削除（自ブランチは変更なし）
                (Some((b_li, b_item)), Some((o_li, o_item)), None)
                    if o_li == b_li && &o_item == b_item =>
                {
                    remove_item_from_scene(os, id);
                    auto_merged += 1;
                }
                // 自ブランチは変更、相手ブランチは削除 -> 競合
                (Some(_), Some((_, o_item)), None) => {
                    let cid = format!("item_mod_del_{id}");
                    match resolutions.get(&cid).copied() {
                        Some(ConflictResolution::KeepTheirs) => {
                            remove_item_from_scene(os, id);
                        }
                        Some(
                            ConflictResolution::KeepOurs | ConflictResolution::KeepBothDuplicate,
                        ) => {}
                        None => conflicts.push(MergeConflict {
                            conflict_id: cid,
                            scene_id: os.id,
                            item_id: Some(id),
                            kind: ConflictKind::ItemModifiedVsDeleted,
                            description: format!(
                                "「{}」が Ours で編集、Theirs で削除されています",
                                o_item.name
                            ),
                            ours_summary: format!("編集保持 (@{:.2}s)", o_item.start.as_seconds()),
                            theirs_summary: "削除".into(),
                        }),
                    }
                }
                // 自ブランチは削除、相手ブランチは変更 -> 競合
                (Some((b_li, b_item)), None, Some((t_li, t_item)))
                    if !(t_li == b_li && t_item == b_item) =>
                {
                    let cid = format!("item_del_mod_{id}");
                    match resolutions.get(&cid).copied() {
                        Some(
                            ConflictResolution::KeepTheirs | ConflictResolution::KeepBothDuplicate,
                        ) => {
                            let target_li = t_li.min(os.layers.len().saturating_sub(1));
                            os.layers[target_li].items.push(t_item.clone());
                        }
                        Some(ConflictResolution::KeepOurs) => {}
                        None => conflicts.push(MergeConflict {
                            conflict_id: cid,
                            scene_id: os.id,
                            item_id: Some(id),
                            kind: ConflictKind::ItemModifiedVsDeleted,
                            description: format!(
                                "「{}」が Ours で削除、Theirs で編集されています",
                                t_item.name
                            ),
                            ours_summary: "削除".into(),
                            theirs_summary: format!(
                                "編集保持 (@{:.2}s)",
                                t_item.start.as_seconds()
                            ),
                        }),
                    }
                }
                // 両方に存在：相手だけ変更された場合は自動採用、両方異なる変更は競合
                (Some((b_li, b_item)), Some((o_li, o_item)), Some((t_li, t_item))) => {
                    let ours_changed = o_li != b_li || &o_item != b_item;
                    let theirs_changed = t_li != b_li || t_item != b_item;
                    if !ours_changed && theirs_changed {
                        remove_item_from_scene(os, id);
                        let target_li = t_li.min(os.layers.len().saturating_sub(1));
                        os.layers[target_li].items.push(t_item.clone());
                        auto_merged += 1;
                    } else if ours_changed && theirs_changed && (o_li != t_li || &o_item != t_item)
                    {
                        let cid = format!("item_both_{id}");
                        match resolutions.get(&cid).copied() {
                            Some(ConflictResolution::KeepTheirs) => {
                                remove_item_from_scene(os, id);
                                let target_li = t_li.min(os.layers.len().saturating_sub(1));
                                os.layers[target_li].items.push(t_item.clone());
                            }
                            Some(ConflictResolution::KeepBothDuplicate) => {
                                let mut dup = t_item.clone();
                                dup.id = Uuid::new_v4();
                                dup.name = format!("{} (Theirs)", dup.name);
                                let target_li = t_li.min(os.layers.len().saturating_sub(1));
                                os.layers[target_li].items.push(dup);
                            }
                            Some(ConflictResolution::KeepOurs) => {}
                            None => conflicts.push(MergeConflict {
                                conflict_id: cid,
                                scene_id: os.id,
                                item_id: Some(id),
                                kind: ConflictKind::ItemBothModified,
                                description: format!(
                                    "「{}」が両ブランチで同時に変更されました ({})",
                                    o_item.name,
                                    diff_item_fields(o_li, &o_item, t_li, t_item).join(", ")
                                ),
                                ours_summary: format!(
                                    "Ours: L{} @{:.2}s ({:.2}s)",
                                    o_li + 1,
                                    o_item.start.as_seconds(),
                                    o_item.duration.as_seconds()
                                ),
                                theirs_summary: format!(
                                    "Theirs: L{} @{:.2}s ({:.2}s)",
                                    t_li + 1,
                                    t_item.start.as_seconds(),
                                    t_item.duration.as_seconds()
                                ),
                            }),
                        }
                    }
                }
                _ => {}
            }
        }
    }

    ThreeWayMergeResult {
        merged_project: merged,
        auto_merged_from_theirs: auto_merged,
        conflicts,
    }
}

fn remove_item_from_scene(scene: &mut Scene, id: ItemId) {
    for layer in &mut scene.layers {
        layer.items.retain(|i| i.id != id);
    }
}

fn merge_markers_three_way(
    bs: Option<&Scene>,
    os: &mut Scene,
    ts: &Scene,
    auto_merged: &mut usize,
) {
    let base_ids: BTreeSet<Uuid> = bs
        .map(|s| s.markers.iter().map(|m| m.id).collect())
        .unwrap_or_default();
    let ours_ids: BTreeSet<Uuid> = os.markers.iter().map(|m| m.id).collect();
    for tm in &ts.markers {
        if !ours_ids.contains(&tm.id) && !base_ids.contains(&tm.id) {
            os.markers.push(tm.clone());
            *auto_merged += 1;
        }
    }
    os.markers.sort_by_key(|m| m.time);
}

fn merge_reviews_three_way(os: &mut Scene, ts: &Scene, auto_merged: &mut usize) {
    let ours_ids: BTreeSet<Uuid> = os.review_comments.iter().map(|c| c.id).collect();
    for tc in &ts.review_comments {
        if !ours_ids.contains(&tc.id) {
            os.review_comments.push(tc.clone());
            *auto_merged += 1;
        }
    }
    os.review_comments.sort_by_key(|c| c.time);
}

// =========================================================================
// 3. Timecode Review Comments (Frame.io / YouTube Review Interop)
// =========================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    Open,
    InProgress,
    Resolved,
}

impl ReviewStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "🔴 未対応 (Open)",
            Self::InProgress => "🟡 対応中 (InProgress)",
            Self::Resolved => "🟢 解決済み (Resolved)",
        }
    }

    pub fn marker_color(self) -> RgbaColor {
        match self {
            Self::Open => RgbaColor {
                r: 0.96,
                g: 0.34,
                b: 0.34,
                a: 1.0,
            },
            Self::InProgress => RgbaColor {
                r: 0.96,
                g: 0.76,
                b: 0.24,
                a: 1.0,
            },
            Self::Resolved => RgbaColor {
                r: 0.26,
                g: 0.84,
                b: 0.54,
                a: 1.0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSource {
    #[default]
    Internal,
    FrameIo,
    YouTube,
    GenericCsv,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewReply {
    pub author: String,
    pub body: String,
    pub created_at: String,
}

/// タイムコード付きレビューコメント（Frame.io / YouTube 限定公開レビュー互換）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: Uuid,
    pub time: RationalTime,
    #[serde(default)]
    pub end_time: Option<RationalTime>,
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub status: ReviewStatus,
    #[serde(default)]
    pub source: ReviewSource,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub replies: Vec<ReviewReply>,
}

impl ReviewComment {
    pub fn new(
        time: RationalTime,
        author: impl Into<String>,
        body: impl Into<String>,
        source: ReviewSource,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            time: time.max(RationalTime::ZERO),
            end_time: None,
            author: author.into(),
            body: body.into(),
            status: ReviewStatus::Open,
            source,
            created_at: String::new(),
            replies: Vec::new(),
        }
    }
}

/// `HH:MM:SS:FF` (SMPTE), `HH:MM:SS;FF`, `HH:MM:SS.mmm`, `MM:SS`, または秒数を `RationalTime` へ変換する
pub fn parse_timecode_to_rational(raw: &str, fps: FrameRate) -> Option<RationalTime> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(sec) = s.parse::<f64>()
        && sec.is_finite()
        && sec >= 0.0
    {
        return Some(RationalTime::from_seconds(sec));
    }
    let normalized = s.replace(';', ":");
    let parts: Vec<&str> = normalized.split(':').collect();
    match parts.len() {
        2 => {
            let m: f64 = parts[0].trim().parse().ok()?;
            let sec: f64 = parts[1].trim().parse().ok()?;
            Some(RationalTime::from_seconds((m * 60.0 + sec).max(0.0)))
        }
        3 => {
            let h: f64 = parts[0].trim().parse().ok()?;
            let m: f64 = parts[1].trim().parse().ok()?;
            let sec: f64 = parts[2].trim().parse().ok()?;
            Some(RationalTime::from_seconds(
                (h * 3600.0 + m * 60.0 + sec).max(0.0),
            ))
        }
        4 => {
            let h: f64 = parts[0].trim().parse().ok()?;
            let m: f64 = parts[1].trim().parse().ok()?;
            let sec: f64 = parts[2].trim().parse().ok()?;
            let frames: f64 = parts[3].trim().parse().ok()?;
            let total_sec = h * 3600.0 + m * 60.0 + sec + frames / fps.as_f64().max(1.0);
            Some(RationalTime::from_seconds(total_sec.max(0.0)))
        }
        _ => None,
    }
}

/// `RationalTime` を SMPTE タイムコード `HH:MM:SS:FF` へ整形する
pub fn format_smpte_timecode(time: RationalTime, fps: FrameRate) -> String {
    let fps_int = fps.as_f64().round().max(1.0) as i64;
    let total_frames = time.to_frame_floor(fps).max(0);
    let ff = total_frames % fps_int;
    let total_sec = total_frames / fps_int;
    let ss = total_sec % 60;
    let mm = (total_sec / 60) % 60;
    let hh = total_sec / 3600;
    format!("{hh:02}:{mm:02}:{ss:02}:{ff:02}")
}

/// Frame.io 互換 CSV（または汎用タイムコード CSV）をインポートする
pub fn import_review_comments_csv(csv_text: &str, fps: FrameRate) -> Vec<ReviewComment> {
    let mut lines = csv_text.lines().filter(|l| !l.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return Vec::new();
    };
    let headers: Vec<String> = parse_csv_row(header_line)
        .into_iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();

    let find_col = |candidates: &[&str]| -> Option<usize> {
        headers
            .iter()
            .position(|h| candidates.iter().any(|c| h.contains(c)))
    };

    let tc_in_col = find_col(&["timecode in", "timecode", "timestamp", "time", "start"]);
    let tc_out_col = find_col(&["timecode out", "end"]);
    let comment_col = find_col(&["comment", "body", "text", "note", "feedback"]);
    let author_col = find_col(&["commenter", "author", "user", "name"]);
    let status_col = find_col(&["status", "completed", "resolved"]);

    let mut out = Vec::new();
    for line in lines {
        let cols = parse_csv_row(line);
        let tc_raw = tc_in_col
            .and_then(|i| cols.get(i))
            .map(String::as_str)
            .unwrap_or("");
        let Some(time) = parse_timecode_to_rational(tc_raw, fps) else {
            continue;
        };
        let body = comment_col
            .and_then(|i| cols.get(i))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if body.is_empty() {
            continue;
        }
        let author = author_col
            .and_then(|i| cols.get(i))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Reviewer".into());
        let end_time = tc_out_col
            .and_then(|i| cols.get(i))
            .and_then(|raw| parse_timecode_to_rational(raw, fps))
            .filter(|t| *t > time);
        let status = status_col
            .and_then(|i| cols.get(i))
            .map(|s| parse_review_status(s))
            .unwrap_or(ReviewStatus::Open);
        let source = if headers.iter().any(|h| h.contains("timecode in")) {
            ReviewSource::FrameIo
        } else {
            ReviewSource::GenericCsv
        };
        let mut c = ReviewComment::new(time, author, body, source);
        c.end_time = end_time;
        c.status = status;
        out.push(c);
    }
    out.sort_by_key(|c| c.time);
    out
}

/// Frame.io JSON / 本アプリ JSON レビューコメントをインポートする
pub fn import_review_comments_json(
    json_text: &str,
    fps: FrameRate,
) -> Result<Vec<ReviewComment>, String> {
    if let Ok(direct) = serde_json::from_str::<Vec<ReviewComment>>(json_text) {
        return Ok(direct);
    }
    let val: serde_json::Value =
        serde_json::from_str(json_text).map_err(|e| format!("invalid review JSON: {e}"))?;
    let arr = val
        .as_array()
        .or_else(|| val.get("comments").and_then(|v| v.as_array()))
        .ok_or_else(|| "expected JSON array or { \"comments\": [...] }".to_string())?;

    let mut out = Vec::new();
    for entry in arr {
        let time = entry
            .get("timestamp_sec")
            .or_else(|| entry.get("timestamp"))
            .and_then(|v| {
                v.as_f64()
                    .map(RationalTime::from_seconds)
                    .or_else(|| v.as_str().and_then(|s| parse_timecode_to_rational(s, fps)))
            })
            .or_else(|| {
                entry
                    .get("timecode")
                    .or_else(|| entry.get("timecode_in"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| parse_timecode_to_rational(s, fps))
            })
            .unwrap_or(RationalTime::ZERO);

        let body = entry
            .get("text")
            .or_else(|| entry.get("body"))
            .or_else(|| entry.get("comment"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if body.is_empty() {
            continue;
        }
        let author = entry
            .get("author")
            .or_else(|| entry.get("owner_name"))
            .or_else(|| entry.get("commenter"))
            .and_then(|v| v.as_str())
            .unwrap_or("Reviewer")
            .to_string();
        let completed = entry
            .get("completed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let status = if completed {
            ReviewStatus::Resolved
        } else {
            entry
                .get("status")
                .and_then(|v| v.as_str())
                .map(parse_review_status)
                .unwrap_or(ReviewStatus::Open)
        };
        let mut comment = ReviewComment::new(time, author, body, ReviewSource::FrameIo);
        comment.status = status;
        out.push(comment);
    }
    out.sort_by_key(|c| c.time);
    Ok(out)
}

/// YouTube 限定公開レビュー等のタイムスタンプ付きコメントテキストを一括インポートする
/// 例: `"00:12 字幕の色をもう少し明るく\n1:05:10 @田中 BGM を -3dB 下げてください"`
pub fn import_youtube_review_comments(text: &str, fps: FrameRate) -> Vec<ReviewComment> {
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, |c: char| c.is_whitespace() || c == '-' || c == '—');
        let first = parts.next().unwrap_or("").trim();
        let rest = parts
            .next()
            .unwrap_or("")
            .trim()
            .trim_start_matches(['-', ':', '—', ' ']);
        if rest.is_empty() {
            continue;
        }
        if let Some(time) = parse_timecode_to_rational(first, fps) {
            let (author, body) = if let Some(after_at) = rest.strip_prefix('@') {
                let mut sp = after_at.splitn(2, char::is_whitespace);
                let a = sp.next().unwrap_or("YouTube Reviewer").trim();
                let b = sp.next().unwrap_or("").trim();
                if b.is_empty() {
                    ("YouTube Reviewer".to_string(), rest.to_string())
                } else {
                    (a.to_string(), b.to_string())
                }
            } else {
                ("YouTube Reviewer".to_string(), rest.to_string())
            };
            out.push(ReviewComment::new(
                time,
                author,
                body,
                ReviewSource::YouTube,
            ));
        }
    }
    out.sort_by_key(|c| c.time);
    out
}

/// Frame.io 互換 CSV 形式へレビューコメントを書き出す
pub fn export_review_comments_csv(comments: &[ReviewComment], fps: FrameRate) -> String {
    let mut out = String::from("Timecode In,Timecode Out,Commenter,Comment,Status\n");
    for c in comments {
        let tc_in = format_smpte_timecode(c.time, fps);
        let tc_out = c
            .end_time
            .map(|t| format_smpte_timecode(t, fps))
            .unwrap_or_default();
        let status = match c.status {
            ReviewStatus::Open => "Open",
            ReviewStatus::InProgress => "InProgress",
            ReviewStatus::Resolved => "Resolved",
        };
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            tc_in,
            tc_out,
            escape_csv_cell(&c.author),
            escape_csv_cell(&c.body),
            status
        ));
    }
    out
}

pub fn export_review_comments_json(comments: &[ReviewComment]) -> String {
    serde_json::to_string_pretty(comments).unwrap_or_else(|_| "[]".into())
}

/// レビューコメント群をタイムラインマーカー（ステータス別カラー付き）へ自動変換するコマンド列を生成する
pub fn build_review_comments_to_markers_commands(
    comments: &[ReviewComment],
    only_unresolved: bool,
) -> Vec<EditCommand> {
    comments
        .iter()
        .filter(|c| !only_unresolved || c.status != ReviewStatus::Resolved)
        .map(|c| {
            let mut m = TimelineMarker::new(c.time, format!("[{}] {}", c.author, c.body));
            m.color = c.status.marker_color();
            EditCommand::AddMarker { marker: m }
        })
        .collect()
}

fn parse_review_status(s: &str) -> ReviewStatus {
    let lower = s.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "resolved" | "done" | "completed" | "true" | "yes" | "closed"
    ) {
        ReviewStatus::Resolved
    } else if matches!(lower.as_str(), "in_progress" | "inprogress" | "wip") {
        ReviewStatus::InProgress
    } else {
        ReviewStatus::Open
    }
}

fn escape_csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn parse_csv_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    cells.push(cur);
    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ItemKind, ProjectSettings, ShapeKind};

    #[test]
    fn semantic_diff_and_three_way_merge_handles_parallel_edits_and_conflicts() {
        let mut base = Project::new("CollabBase", ProjectSettings::default());
        let item_a = TimelineItem::new(
            "Subtitle A",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 200.0,
                height: 80.0,
                color: RgbaColor::WHITE,
            },
        );
        let item_b = TimelineItem::new(
            "BGM Clip",
            RationalTime::from_seconds(1.0),
            RationalTime::from_seconds(4.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 40.0,
                color: RgbaColor::BLACK,
            },
        );
        let id_a = item_a.id;
        let id_b = item_b.id;
        base.scenes[0].layers[0].items.push(item_a);
        base.scenes[0].layers[1].items.push(item_b);

        // Ours は item_a を編集
        let mut ours = base.clone();
        ours.scenes[0].layers[0].items[0].transform.x = 120.0;

        // Theirs は item_b を編集＋新規アイテムを追加＋カラーグレーディング ON
        let mut theirs = base.clone();
        theirs.scenes[0].layers[1].items[0].start = RationalTime::from_seconds(1.5);
        theirs.scenes[0].layers[2].items.push(TimelineItem::new(
            "New Badge",
            RationalTime::from_seconds(2.0),
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 60.0,
                height: 60.0,
                color: RgbaColor::WHITE,
            },
        ));
        theirs.scenes[0].color_grade.enabled = true;

        let diff = diff_projects(&base, &theirs);
        assert!(diff.total_changes() >= 3);
        assert!(format_diff_report(&diff).contains("New Badge"));

        // 独立編集なので競合ゼロで自動マージ成功
        let clean = merge_projects_three_way(&base, &ours, &theirs, &BTreeMap::new());
        assert!(clean.conflicts.is_empty());
        assert!(clean.auto_merged_from_theirs >= 3);
        assert!(clean.merged_project.scenes[0].color_grade.enabled);
        assert_eq!(
            clean.merged_project.scenes[0].layers[0].items[0]
                .transform
                .x,
            120.0
        );
        assert_eq!(
            clean.merged_project.scenes[0].layers[1].items[0].start,
            RationalTime::from_seconds(1.5)
        );
        assert_eq!(clean.merged_project.scenes[0].layers[2].items.len(), 1);

        // 今度は両方が item_a を別値へ変更 -> 競合検出＆KeepBothDuplicate で解決
        theirs.scenes[0].layers[0].items[0].transform.x = 300.0;
        let conflicted = merge_projects_three_way(&base, &ours, &theirs, &BTreeMap::new());
        assert_eq!(conflicted.conflicts.len(), 1);
        assert_eq!(conflicted.conflicts[0].item_id, Some(id_a));
        assert_ne!(id_a, id_b);

        let mut res = BTreeMap::new();
        res.insert(
            conflicted.conflicts[0].conflict_id.clone(),
            ConflictResolution::KeepBothDuplicate,
        );
        let resolved = merge_projects_three_way(&base, &ours, &theirs, &res);
        assert!(resolved.conflicts.is_empty());
        assert_eq!(resolved.merged_project.scenes[0].layers[0].items.len(), 2);
    }

    #[test]
    fn imports_frameio_csv_json_and_youtube_review_comments_and_builds_markers() {
        let csv = "Timecode In,Timecode Out,Commenter,Comment,Status\n\
                   00:00:02:30,00:00:04:00,Alice,\"字幕フォントを少し大きく, 太字で\",Open\n\
                   00:01:05:00,,Bob,色味OKです,Resolved\n";
        let imported = import_review_comments_csv(csv, FrameRate::FPS_60);
        assert_eq!(imported.len(), 2);
        assert!((imported[0].time.as_seconds() - 2.5).abs() < 1e-3);
        assert_eq!(imported[0].author, "Alice");
        assert_eq!(imported[0].status, ReviewStatus::Open);
        assert_eq!(imported[1].status, ReviewStatus::Resolved);

        let exported_csv = export_review_comments_csv(&imported, FrameRate::FPS_60);
        let reimported = import_review_comments_csv(&exported_csv, FrameRate::FPS_60);
        assert_eq!(reimported.len(), 2);
        assert_eq!(reimported[0].body, imported[0].body);

        let yt = "00:15 @ディレクター ここでSE入れてください\n01:20 音量完璧\n";
        let yt_comments = import_youtube_review_comments(yt, FrameRate::FPS_30);
        assert_eq!(yt_comments.len(), 2);
        assert_eq!(yt_comments[0].author, "ディレクター");
        assert!((yt_comments[1].time.as_seconds() - 80.0).abs() < 1e-3);

        let cmds = build_review_comments_to_markers_commands(&imported, true);
        assert_eq!(cmds.len(), 1, "only unresolved comment converted to marker");
    }
}
