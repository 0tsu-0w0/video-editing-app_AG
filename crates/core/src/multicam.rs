//! v1.0 #17: マルチカメラ編集（同期整列・2×2/3×3 ライブスイッチングカット組み立て）

use crate::command::EditCommand;
use crate::project::{ItemId, Scene, TimelineItem};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// マルチカムプレビューのグリッドレイアウト
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MulticamGridLayout {
    #[default]
    Grid2x2,
    Grid3x3,
}

impl MulticamGridLayout {
    pub fn cols_rows(self) -> (usize, usize) {
        match self {
            Self::Grid2x2 => (2, 2),
            Self::Grid3x3 => (3, 3),
        }
    }

    pub fn max_angles(self) -> usize {
        let (c, r) = self.cols_rows();
        c * r
    }
}

/// マルチカムの 1 カメラアングル情報
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MulticamAngle {
    pub camera_index: usize,
    pub label: String,
    pub layer_index: usize,
    pub item_id: ItemId,
    /// 基準カメラに対する算出オフセット（秒、正値なら後ろへずらして同期）
    pub sync_offset_sec: f64,
    pub confidence: f32,
}

/// ライブスイッチングで記録されたカメラ切替点
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MulticamCutPoint {
    pub time: RationalTime,
    /// `0`-based アングル番号（キーボード `1`〜`9` は `0`〜`8` に対応）
    pub active_angle: usize,
}

/// 音声相互相関で求めた `(item_id, offset_sec)` をもとに複数カメラクリップを一括整列する
/// 1 ステップ Undo 可能な `EditCommand::Batch` を生成する。
/// 最小開始時刻が負にならないよう全体を自動シフトしてクリップ欠損を防ぐ。
pub fn build_multicam_sync_align_commands(
    scene: &Scene,
    offsets: &[(ItemId, f64)],
) -> Option<EditCommand> {
    if offsets.is_empty() {
        return None;
    }

    let mut targets: Vec<(usize, ItemId, f64)> = Vec::new();
    let mut min_new_start = f64::INFINITY;

    for &(item_id, delta_sec) in offsets {
        if !delta_sec.is_finite() {
            continue;
        }
        if let Some((l_idx, item)) = scene
            .layers
            .iter()
            .enumerate()
            .find_map(|(li, l)| l.items.iter().find(|i| i.id == item_id).map(|it| (li, it)))
        {
            let proposed = item.start.as_seconds() + delta_sec;
            if proposed < min_new_start {
                min_new_start = proposed;
            }
            targets.push((l_idx, item_id, proposed));
        }
    }

    if targets.is_empty() {
        return None;
    }

    let shift_floor = if min_new_start < 0.0 {
        -min_new_start
    } else {
        0.0
    };

    let commands: Vec<EditCommand> = targets
        .into_iter()
        .map(|(l_idx, item_id, proposed)| EditCommand::MoveObject {
            item_id,
            target_layer_index: l_idx,
            new_start: RationalTime::from_seconds((proposed + shift_floor).max(0.0)),
        })
        .collect();

    Some(EditCommand::Batch {
        label: "multicam_audio_sync_align".to_string(),
        commands,
    })
}

/// ライブスイッチングの切替時刻列から、指定出力レイヤーへ切れ目なく切り替わる
/// マルチカム編集済みクリップ列を 1 ステップ Undo で生成する。
pub fn build_multicam_switch_cut_commands(
    scene: &Scene,
    angle_layers: &[usize],
    cuts: &[MulticamCutPoint],
    output_layer_index: usize,
    range_start: RationalTime,
    range_end: RationalTime,
) -> Option<EditCommand> {
    if angle_layers.is_empty() || range_end <= range_start {
        return None;
    }

    let mut sorted_cuts: Vec<MulticamCutPoint> = cuts
        .iter()
        .copied()
        .filter(|c| c.time >= range_start && c.time < range_end)
        .collect();
    sorted_cuts.sort_by_key(|c| c.time);

    // 同一時刻の重複切替点は後勝ちに整理
    let mut deduped: Vec<MulticamCutPoint> = Vec::new();
    for c in sorted_cuts {
        if let Some(last) = deduped.last_mut()
            && last.time == c.time
        {
            last.active_angle = c.active_angle;
            continue;
        }
        deduped.push(c);
    }

    if deduped.is_empty() || deduped[0].time > range_start {
        deduped.insert(
            0,
            MulticamCutPoint {
                time: range_start,
                active_angle: 0,
            },
        );
    }

    let mut commands = Vec::new();
    for (idx, cut) in deduped.iter().enumerate() {
        let seg_start = cut.time;
        let seg_end = deduped
            .get(idx + 1)
            .map(|n| n.time)
            .unwrap_or(range_end)
            .min(range_end);
        if seg_end <= seg_start {
            continue;
        }
        let angle = cut.active_angle % angle_layers.len();
        let src_layer_idx = angle_layers[angle];
        let Some(src_layer) = scene.layers.get(src_layer_idx) else {
            continue;
        };
        // 区間内に重なるカメラクリップをトリム複製して出力レイヤーへ並べる
        for src_item in &src_layer.items {
            let ov_start = seg_start.max(src_item.start);
            let ov_end = seg_end.min(src_item.end());
            if ov_end <= ov_start {
                continue;
            }
            let trim_in = ov_start - src_item.start;
            let mut cloned: TimelineItem = src_item.clone();
            cloned.id = Uuid::new_v4();
            cloned.name = format!("[CAM{}] {}", angle + 1, src_item.name);
            cloned.start = ov_start;
            cloned.duration = ov_end - ov_start;
            match &mut cloned.kind {
                crate::project::ItemKind::Video { source_offset, .. }
                | crate::project::ItemKind::Audio { source_offset, .. } => {
                    *source_offset = *source_offset + trim_in;
                }
                _ => {}
            }
            commands.push(EditCommand::AddObject {
                layer_index: output_layer_index,
                item: cloned,
            });
        }
    }

    if commands.is_empty() {
        return None;
    }

    Some(EditCommand::Batch {
        label: "multicam_live_switch_cuts".to_string(),
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::{ItemKind, Layer, Project, ProjectSettings, RgbaColor, ShapeKind};

    #[test]
    fn multicam_sync_align_and_live_switch_cuts_work_with_single_undo() {
        let mut proj = Project::new("MulticamTest", ProjectSettings::default());
        let s = proj.active_scene_mut().unwrap();
        let cam1 = TimelineItem::new(
            "CamA",
            RationalTime::from_seconds(0.0),
            RationalTime::from_seconds(6.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        );
        let cam2 = TimelineItem::new(
            "CamB",
            RationalTime::from_seconds(0.5),
            RationalTime::from_seconds(6.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        );
        let c1_id = cam1.id;
        let c2_id = cam2.id;
        s.layers = vec![
            Layer {
                items: vec![cam1],
                ..Layer::new("Cam1")
            },
            Layer {
                items: vec![cam2],
                ..Layer::new("Cam2")
            },
            Layer::new("SwitchOut"),
        ];

        let mut bus = CommandBus::default();
        // CamB を -0.8s ずらす -> 負にならないよう全体 +0.3s シフト
        let sync_cmd = build_multicam_sync_align_commands(
            proj.active_scene().unwrap(),
            &[(c1_id, 0.0), (c2_id, -0.8)],
        )
        .unwrap();
        bus.execute(&mut proj, sync_cmd, CommandSource::Gui, None)
            .unwrap();
        let s_after = proj.active_scene().unwrap();
        assert!((s_after.layers[0].items[0].start.as_seconds() - 0.3).abs() < 1e-4);
        assert!((s_after.layers[1].items[0].start.as_seconds() - 0.0).abs() < 1e-4);

        // ライブスイッチングカット組み立て
        let cut_cmd = build_multicam_switch_cut_commands(
            s_after,
            &[0, 1],
            &[
                MulticamCutPoint {
                    time: RationalTime::from_seconds(0.5),
                    active_angle: 0,
                },
                MulticamCutPoint {
                    time: RationalTime::from_seconds(2.0),
                    active_angle: 1,
                },
                MulticamCutPoint {
                    time: RationalTime::from_seconds(4.0),
                    active_angle: 0,
                },
            ],
            2,
            RationalTime::from_seconds(0.5),
            RationalTime::from_seconds(5.5),
        )
        .unwrap();
        bus.execute(&mut proj, cut_cmd, CommandSource::Gui, None)
            .unwrap();
        let out_items = &proj.active_scene().unwrap().layers[2].items;
        assert_eq!(out_items.len(), 3);
        assert!(out_items[0].name.starts_with("[CAM1]"));
        assert!(out_items[1].name.starts_with("[CAM2]"));
        assert!(out_items[2].name.starts_with("[CAM1]"));

        assert!(bus.undo(&mut proj));
        assert!(proj.active_scene().unwrap().layers[2].items.is_empty());
    }
}
