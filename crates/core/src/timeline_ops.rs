use crate::animation::{CubicBezier, Interpolation, Keyframe};
use crate::command::EditCommand;
use crate::project::{EffectNode, ItemId, ItemKind, Scene, TimelineItem};
use crate::time::RationalTime;
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// アイテム整列モード（アルティメットプラグイン相当）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    /// 選択中アイテムの中で最も早い開始位置に左端を揃える
    Left,
    /// 選択中アイテムの中で最も遅い終了位置に右端を揃える
    Right,
    /// 全選択アイテムの開始位置を再生ヘッドへ揃える
    Playhead,
}

/// AviUtl2 準拠の時刻基準選択方向（`Ctrl+ダブルクリック` 相当）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectDirection {
    /// 指定時刻以前（左方向）の全オブジェクト
    Before,
    /// 指定時刻以降（右方向）の全オブジェクト
    After,
}

/// 登場・退場・演出モーションプリセット（ワンクリック適用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionPreset {
    SlideInLeft,
    SlideInBottom,
    FadeInOut,
    PopBounce,
    ZoomIn,
    ImpactShake,
}

impl MotionPreset {
    pub const ALL: [Self; 6] = [
        Self::SlideInLeft,
        Self::SlideInBottom,
        Self::FadeInOut,
        Self::PopBounce,
        Self::ZoomIn,
        Self::ImpactShake,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::SlideInLeft => "左スライドイン",
            Self::SlideInBottom => "下スライドイン",
            Self::FadeInOut => "フェード入出",
            Self::PopBounce => "ポップ登場",
            Self::ZoomIn => "ズームイン",
            Self::ImpactShake => "衝撃シェイク",
        }
    }
}

/// タイムライン上のテキスト検索ヒット情報
#[derive(Debug, Clone, PartialEq)]
pub struct TextSearchMatch {
    pub item_id: ItemId,
    pub layer_index: usize,
    pub item_name: String,
    pub start: RationalTime,
    pub content: String,
}

/// 指定レイヤー内の全オブジェクト ID を返す（レイヤー名 `Ctrl+クリック` 相当）
pub fn select_layer_items(scene: &Scene, layer_index: usize) -> Vec<ItemId> {
    scene
        .layers
        .get(layer_index)
        .map(|l| l.items.iter().map(|i| i.id).collect())
        .unwrap_or_default()
}

/// 指定時刻より前または後の全オブジェクト ID を返す（端 `Ctrl+ダブルクリック` 相当）
pub fn select_items_from_time(
    scene: &Scene,
    pivot: RationalTime,
    direction: SelectDirection,
    layer_filter: Option<usize>,
) -> Vec<ItemId> {
    let mut ids = Vec::new();
    for (l_idx, layer) in scene.layers.iter().enumerate() {
        if let Some(target) = layer_filter
            && l_idx != target
        {
            continue;
        }
        for item in &layer.items {
            let matched = match direction {
                SelectDirection::After => item.start >= pivot,
                SelectDirection::Before => item.end() <= pivot || item.start <= pivot,
            };
            if matched {
                ids.push(item.id);
            }
        }
    }
    ids
}

/// 複数選択したクリップをレイヤーごとに開始時刻順へ並べ、隙間詰め（`gap_sec = 0.0`）または等間隔整列するバッチコマンドを生成する
pub fn build_close_gaps_commands(
    scene: &Scene,
    item_ids: &[ItemId],
    gap_sec: f64,
) -> Option<EditCommand> {
    let selected: HashSet<ItemId> = item_ids.iter().copied().collect();
    if selected.len() < 2 {
        return None;
    }
    let gap = RationalTime::from_seconds(gap_sec.max(0.0));
    let mut commands = Vec::new();

    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        let mut items: Vec<&TimelineItem> = layer
            .items
            .iter()
            .filter(|i| selected.contains(&i.id))
            .collect();
        if items.len() < 2 {
            continue;
        }
        items.sort_by_key(|i| i.start);
        let mut cursor = items[0].start;
        for item in items {
            if item.start != cursor {
                commands.push(EditCommand::MoveObject {
                    item_id: item.id,
                    target_layer_index: layer_idx,
                    new_start: cursor,
                });
            }
            cursor = cursor + item.duration + gap;
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some(EditCommand::Batch {
            label: if gap_sec <= 0.0 {
                "close_gaps".into()
            } else {
                format!("distribute_gap_{gap_sec:.2}s")
            },
            commands,
        })
    }
}

/// 複数選択したクリップの左端揃え・右端揃え・再生ヘッド吸着バッチコマンドを生成する
pub fn build_align_commands(
    scene: &Scene,
    item_ids: &[ItemId],
    mode: AlignMode,
    playhead: RationalTime,
) -> Option<EditCommand> {
    let selected: HashSet<ItemId> = item_ids.iter().copied().collect();
    if selected.is_empty() {
        return None;
    }
    let mut found: Vec<(usize, &TimelineItem)> = Vec::new();
    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        for item in &layer.items {
            if selected.contains(&item.id) {
                found.push((layer_idx, item));
            }
        }
    }
    if found.is_empty() {
        return None;
    }

    let mut commands = Vec::new();
    match mode {
        AlignMode::Left => {
            let min_start = found.iter().map(|(_, i)| i.start).min()?;
            for (layer_idx, item) in found {
                if item.start != min_start {
                    commands.push(EditCommand::MoveObject {
                        item_id: item.id,
                        target_layer_index: layer_idx,
                        new_start: min_start,
                    });
                }
            }
        }
        AlignMode::Right => {
            let max_end = found.iter().map(|(_, i)| i.end()).max()?;
            for (layer_idx, item) in found {
                let new_start = (max_end - item.duration).max(RationalTime::ZERO);
                if item.start != new_start {
                    commands.push(EditCommand::MoveObject {
                        item_id: item.id,
                        target_layer_index: layer_idx,
                        new_start,
                    });
                }
            }
        }
        AlignMode::Playhead => {
            let target = playhead.max(RationalTime::ZERO);
            for (layer_idx, item) in found {
                if item.start != target {
                    commands.push(EditCommand::MoveObject {
                        item_id: item.id,
                        target_layer_index: layer_idx,
                        new_start: target,
                    });
                }
            }
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some(EditCommand::Batch {
            label: format!("align_{mode:?}").to_ascii_lowercase(),
            commands,
        })
    }
}

/// 選択中クリップの開始位置と長さを、参照レイヤー（例: 音声レイヤー）上の対応クリップへ一括フィットさせる。
/// 重複する参照クリップがあれば最も重なりが大きいクリップへ合わせ、無ければ時系列順の同インデックス参照クリップへ合わせる。
pub fn build_fit_duration_to_layer_commands(
    scene: &Scene,
    target_item_ids: &[ItemId],
    reference_layer_index: usize,
) -> Option<EditCommand> {
    let ref_layer = scene.layers.get(reference_layer_index)?;
    if ref_layer.items.is_empty() || target_item_ids.is_empty() {
        return None;
    }
    let mut ref_items: Vec<&TimelineItem> = ref_layer.items.iter().collect();
    ref_items.sort_by_key(|i| i.start);

    let selected: HashSet<ItemId> = target_item_ids.iter().copied().collect();
    let mut targets: Vec<&TimelineItem> = scene
        .layers
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != reference_layer_index)
        .flat_map(|(_, l)| l.items.iter())
        .filter(|i| selected.contains(&i.id))
        .collect();
    if targets.is_empty() {
        return None;
    }
    targets.sort_by_key(|i| i.start);

    let mut commands = Vec::new();
    for (order_idx, item) in targets.iter().enumerate() {
        // 最大オーバーラップする参照クリップを優先検索
        let matched_ref = ref_items
            .iter()
            .filter_map(|r| {
                let ov_start = item.start.max(r.start);
                let ov_end = item.end().min(r.end());
                let ov = (ov_end - ov_start).ticks;
                if ov > 0 { Some((ov, *r)) } else { None }
            })
            .max_by_key(|(ov, _)| *ov)
            .map(|(_, r)| r)
            .or_else(|| ref_items.get(order_idx).copied());

        if let Some(ref_clip) = matched_ref
            && (item.start != ref_clip.start || item.duration != ref_clip.duration)
        {
            commands.push(EditCommand::TrimObject {
                item_id: item.id,
                new_start: ref_clip.start,
                new_duration: ref_clip.duration,
            });
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some(EditCommand::Batch {
            label: format!("fit_to_layer_{reference_layer_index}"),
            commands,
        })
    }
}

/// シーン内の全テキストオブジェクトからキーワード検索する
pub fn search_text_items(scene: &Scene, query: &str, case_sensitive: bool) -> Vec<TextSearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut matches = Vec::new();
    for (layer_index, layer) in scene.layers.iter().enumerate() {
        for item in &layer.items {
            if let ItemKind::Text { content, .. } = &item.kind {
                let hay = if case_sensitive {
                    content.clone()
                } else {
                    content.to_lowercase()
                };
                if hay.contains(&needle) {
                    matches.push(TextSearchMatch {
                        item_id: item.id,
                        layer_index,
                        item_name: item.name.clone(),
                        start: item.start,
                        content: content.clone(),
                    });
                }
            }
        }
    }
    matches.sort_by_key(|m| m.start);
    matches
}

/// シーン内の全テキストオブジェクトを一括置換するバッチコマンドを生成する（1回の Ctrl+Z で全件 Undo 可能）
pub fn build_replace_text_commands(
    scene: &Scene,
    query: &str,
    replacement: &str,
    case_sensitive: bool,
) -> Option<(usize, EditCommand)> {
    if query.is_empty() {
        return None;
    }
    let mut commands = Vec::new();
    let mut total_replacements = 0usize;

    for layer in &scene.layers {
        for item in &layer.items {
            if let ItemKind::Text { content, .. } = &item.kind {
                let (new_content, count) = if case_sensitive {
                    let c = content.matches(query).count();
                    (content.replace(query, replacement), c)
                } else {
                    replace_case_insensitive(content, query, replacement)
                };
                if count > 0 {
                    total_replacements += count;
                    commands.push(EditCommand::SetTextContent {
                        item_id: item.id,
                        content: new_content,
                    });
                }
            }
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some((
            total_replacements,
            EditCommand::Batch {
                label: "replace_all_text".into(),
                commands,
            },
        ))
    }
}

fn replace_case_insensitive(text: &str, query: &str, replacement: &str) -> (String, usize) {
    let q_chars: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q_chars.is_empty() {
        return (text.to_string(), 0);
    }
    // 元の文字列のバイト位置対応を維持しながら大文字小文字無視で置換
    let mut out = String::with_capacity(text.len());
    let mut count = 0usize;
    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0usize;
    while i < char_indices.len() {
        let mut matched = false;
        if i + q_chars.len() <= char_indices.len() {
            let slice_matches = (0..q_chars.len()).all(|j| {
                let mut lc = char_indices[i + j].1.to_lowercase();
                lc.next() == Some(q_chars[j]) && lc.next().is_none()
            });
            if slice_matches {
                out.push_str(replacement);
                count += 1;
                i += q_chars.len();
                matched = true;
            }
        }
        if !matched {
            out.push(char_indices[i].1);
            i += 1;
        }
    }
    (out, count)
}

/// 選択オブジェクトへ登場・退場イージングまたは衝撃シェイクプリセットを適用するバッチコマンドを生成する
pub fn build_motion_preset_commands(item: &TimelineItem, preset: MotionPreset) -> EditCommand {
    let id = item.id;
    let dur = item.duration.as_seconds().max(0.2);
    let intro = RationalTime::from_seconds((dur * 0.25).clamp(0.15, 0.5));
    let outro_start = RationalTime::from_seconds((dur - 0.35).max(intro.as_seconds()));
    let end_t = item.duration;
    let mut commands = Vec::new();

    match preset {
        MotionPreset::SlideInLeft => {
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "x".into(),
                keyframe: Keyframe {
                    time: RationalTime::ZERO,
                    value: item.transform.x - 160.0,
                    interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "x".into(),
                keyframe: Keyframe {
                    time: intro,
                    value: item.transform.x,
                    interpolation: Interpolation::Linear,
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: RationalTime::ZERO,
                    value: 0.0,
                    interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: intro,
                    value: item.transform.opacity,
                    interpolation: Interpolation::Linear,
                },
            });
        }
        MotionPreset::SlideInBottom => {
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "y".into(),
                keyframe: Keyframe {
                    time: RationalTime::ZERO,
                    value: item.transform.y + 90.0,
                    interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "y".into(),
                keyframe: Keyframe {
                    time: intro,
                    value: item.transform.y,
                    interpolation: Interpolation::Linear,
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: RationalTime::ZERO,
                    value: 0.0,
                    interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: intro,
                    value: item.transform.opacity,
                    interpolation: Interpolation::Linear,
                },
            });
        }
        MotionPreset::FadeInOut => {
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: RationalTime::ZERO,
                    value: 0.0,
                    interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: intro,
                    value: item.transform.opacity,
                    interpolation: Interpolation::Linear,
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: outro_start,
                    value: item.transform.opacity,
                    interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                },
            });
            commands.push(EditCommand::SetKeyframe {
                item_id: id,
                property: "opacity".into(),
                keyframe: Keyframe {
                    time: end_t,
                    value: 0.0,
                    interpolation: Interpolation::Linear,
                },
            });
        }
        MotionPreset::PopBounce => {
            let mid = RationalTime::from_seconds((intro.as_seconds() * 0.65).max(0.08));
            for prop in ["scale_x", "scale_y"] {
                let base = if prop == "scale_x" {
                    item.transform.scale_x
                } else {
                    item.transform.scale_y
                };
                commands.push(EditCommand::SetKeyframe {
                    item_id: id,
                    property: prop.into(),
                    keyframe: Keyframe {
                        time: RationalTime::ZERO,
                        value: base * 0.2,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                    },
                });
                commands.push(EditCommand::SetKeyframe {
                    item_id: id,
                    property: prop.into(),
                    keyframe: Keyframe {
                        time: mid,
                        value: base * 1.12,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                    },
                });
                commands.push(EditCommand::SetKeyframe {
                    item_id: id,
                    property: prop.into(),
                    keyframe: Keyframe {
                        time: intro,
                        value: base,
                        interpolation: Interpolation::Linear,
                    },
                });
            }
        }
        MotionPreset::ZoomIn => {
            for prop in ["scale_x", "scale_y"] {
                let base = if prop == "scale_x" {
                    item.transform.scale_x
                } else {
                    item.transform.scale_y
                };
                commands.push(EditCommand::SetKeyframe {
                    item_id: id,
                    property: prop.into(),
                    keyframe: Keyframe {
                        time: RationalTime::ZERO,
                        value: base * 0.75,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                    },
                });
                commands.push(EditCommand::SetKeyframe {
                    item_id: id,
                    property: prop.into(),
                    keyframe: Keyframe {
                        time: intro,
                        value: base,
                        interpolation: Interpolation::Linear,
                    },
                });
            }
        }
        MotionPreset::ImpactShake => {
            let mut params = BTreeMap::new();
            params.insert("amp_x".into(), 18.0);
            params.insert("amp_y".into(), 14.0);
            params.insert("freq".into(), 26.0);
            params.insert("seed".into(), 7.0);
            commands.push(EditCommand::AddEffect {
                item_id: id,
                effect: EffectNode {
                    id: Uuid::new_v4(),
                    effect_type: "shake".into(),
                    enabled: true,
                    params,
                },
            });
        }
    }

    EditCommand::Batch {
        label: format!("motion_preset_{preset:?}").to_ascii_lowercase(),
        commands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::{Project, ProjectSettings, RgbaColor, TextAnimator};

    fn make_text_item(name: &str, start_s: f64, dur_s: f64, content: &str) -> TimelineItem {
        TimelineItem::new(
            name,
            RationalTime::from_seconds(start_s),
            RationalTime::from_seconds(dur_s),
            ItemKind::Text {
                content: content.into(),
                font_family: "sans-serif".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        )
    }

    #[test]
    fn close_gaps_and_equal_spacing_with_single_undo() {
        let mut proj = Project::new("AlignTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let a = make_text_item("A", 1.0, 2.0, "一");
        let b = make_text_item("B", 5.0, 1.5, "二");
        let c = make_text_item("C", 10.0, 1.0, "三");
        let ids = vec![a.id, b.id, c.id];
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .extend([a, b, c]);

        let cmd =
            build_close_gaps_commands(proj.active_scene().unwrap(), &ids, 0.25).expect("commands");
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        let items = &proj.active_scene().unwrap().layers[1].items;
        assert!((items[0].start.as_seconds() - 1.0).abs() < 1e-6);
        assert!((items[1].start.as_seconds() - 3.25).abs() < 1e-6);
        assert!((items[2].start.as_seconds() - 5.0).abs() < 1e-6);

        assert!(bus.undo(&mut proj));
        assert!(
            (proj.active_scene().unwrap().layers[1].items[2]
                .start
                .as_seconds()
                - 10.0)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn fit_subtitles_duration_to_audio_layer() {
        let mut proj = Project::new("FitTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let sub1 = make_text_item("Sub1", 0.0, 1.0, "セリフ1");
        let sub2 = make_text_item("Sub2", 3.5, 1.0, "セリフ2");
        let audio1 = TimelineItem::new(
            "Voice1",
            RationalTime::from_seconds(0.2),
            RationalTime::from_seconds(2.4),
            ItemKind::Audio {
                asset_id: Uuid::new_v4(),
                source_offset: RationalTime::ZERO,
                volume: 1.0,
            },
        );
        let audio2 = TimelineItem::new(
            "Voice2",
            RationalTime::from_seconds(3.0),
            RationalTime::from_seconds(1.8),
            ItemKind::Audio {
                asset_id: Uuid::new_v4(),
                source_offset: RationalTime::ZERO,
                volume: 1.0,
            },
        );
        let sub_ids = vec![sub1.id, sub2.id];
        {
            let scene = proj.active_scene_mut().unwrap();
            scene.layers[1].items.extend([sub1, sub2]);
            scene.layers[3].items.extend([audio1, audio2]);
        }

        let cmd = build_fit_duration_to_layer_commands(proj.active_scene().unwrap(), &sub_ids, 3)
            .expect("fit commands");
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        let subs = &proj.active_scene().unwrap().layers[1].items;
        assert_eq!(subs[0].start, RationalTime::from_seconds(0.2));
        assert_eq!(subs[0].duration, RationalTime::from_seconds(2.4));
        assert_eq!(subs[1].start, RationalTime::from_seconds(3.0));
        assert_eq!(subs[1].duration, RationalTime::from_seconds(1.8));
    }

    #[test]
    fn search_and_replace_text_across_scene() {
        let mut proj = Project::new("SearchTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let t1 = make_text_item("T1", 0.0, 2.0, "ずんだもんとメタん");
        let t2 = make_text_item("T2", 2.0, 2.0, "メタんの解説");
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .extend([t1, t2]);

        let hits = search_text_items(proj.active_scene().unwrap(), "メタん", true);
        assert_eq!(hits.len(), 2);

        let (count, cmd) =
            build_replace_text_commands(proj.active_scene().unwrap(), "メタん", "めたん", true)
                .unwrap();
        assert_eq!(count, 2);
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();

        let hits_after = search_text_items(proj.active_scene().unwrap(), "めたん", true);
        assert_eq!(hits_after.len(), 2);
        assert!(search_text_items(proj.active_scene().unwrap(), "メタん", true).is_empty());
    }

    #[test]
    fn select_after_and_before_pivot_time() {
        let mut proj = Project::new("SelTest", ProjectSettings::default());
        let a = make_text_item("A", 0.0, 1.0, "A");
        let b = make_text_item("B", 2.0, 1.0, "B");
        let c = make_text_item("C", 4.0, 1.0, "C");
        let (a_id, b_id, c_id) = (a.id, b.id, c.id);
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .extend([a, b, c]);

        let after = select_items_from_time(
            proj.active_scene().unwrap(),
            RationalTime::from_seconds(2.0),
            SelectDirection::After,
            None,
        );
        assert_eq!(after, vec![b_id, c_id]);

        let before = select_items_from_time(
            proj.active_scene().unwrap(),
            RationalTime::from_seconds(2.0),
            SelectDirection::Before,
            Some(1),
        );
        assert_eq!(before, vec![a_id, b_id]);
    }
}
