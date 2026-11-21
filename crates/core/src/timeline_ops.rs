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

/// 無音区間自動カット（ジェットカット / Jump Cut）設定
#[derive(Debug, Clone, PartialEq)]
pub struct JumpCutConfig {
    /// 無音判定の RMS しきい値（既定 0.025）
    pub rms_threshold: f32,
    /// カット対象とする最小無音長（秒、既定 0.45s）
    pub min_silence_sec: f64,
    /// 発話語頭の食い込み防止マージン（秒、既定 0.08s）
    pub pad_before_sec: f64,
    /// 発話語尾の食い込み防止マージン（秒、既定 0.12s）
    pub pad_after_sec: f64,
    /// 音声に連動して同時にカット＆詰めを行うレイヤーインデックス群
    pub ripple_linked_layers: Vec<usize>,
}

impl Default for JumpCutConfig {
    fn default() -> Self {
        Self {
            rms_threshold: 0.025,
            min_silence_sec: 0.45,
            pad_before_sec: 0.08,
            pad_after_sec: 0.12,
            ripple_linked_layers: vec![0, 1, 2],
        }
    }
}

/// 保持すべき発話区間（タイムライン時刻）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechInterval {
    pub start: RationalTime,
    pub end: RationalTime,
}

/// 波形 RMS バケット列から「保持すべき有音区間」を抽出する（短すぎる無音の結合・前後パディング対応）
pub fn detect_speech_intervals_from_rms(
    rms_series: &[f32],
    bucket_sec: f64,
    total_sec: f64,
    cfg: &JumpCutConfig,
) -> Vec<SpeechInterval> {
    if rms_series.is_empty() || bucket_sec <= 0.0 || total_sec <= 0.0 {
        return Vec::new();
    }
    let threshold = cfg.rms_threshold.max(0.0);
    let mut raw: Vec<(f64, f64)> = Vec::new();
    let mut cur_start: Option<f64> = None;

    for (idx, &rms) in rms_series.iter().enumerate() {
        let t0 = idx as f64 * bucket_sec;
        if t0 >= total_sec {
            break;
        }
        if rms >= threshold {
            if cur_start.is_none() {
                cur_start = Some(t0);
            }
        } else if let Some(s) = cur_start.take() {
            raw.push((s, t0.min(total_sec)));
        }
    }
    if let Some(s) = cur_start {
        raw.push((s, total_sec));
    }
    if raw.is_empty() {
        return Vec::new();
    }

    // min_silence_sec 未満の短い間は発話継続として結合
    let min_gap = cfg.min_silence_sec.max(0.0);
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in raw {
        if let Some(last) = merged.last_mut()
            && s - last.1 < min_gap
        {
            last.1 = last.1.max(e);
            continue;
        }
        merged.push((s, e));
    }

    // 前後マージン（pad_before / pad_after）を付与して重複を再マージ
    let pad_b = cfg.pad_before_sec.max(0.0);
    let pad_a = cfg.pad_after_sec.max(0.0);
    let mut padded: Vec<(f64, f64)> = Vec::new();
    for (s, e) in merged {
        let ps = (s - pad_b).max(0.0);
        let pe = (e + pad_a).min(total_sec);
        if let Some(last) = padded.last_mut()
            && ps <= last.1
        {
            last.1 = last.1.max(pe);
            continue;
        }
        if pe - ps > 0.02 {
            padded.push((ps, pe));
        }
    }

    padded
        .into_iter()
        .map(|(s, e)| SpeechInterval {
            start: RationalTime::from_seconds(s),
            end: RationalTime::from_seconds(e),
        })
        .collect()
}

/// 発話区間列 `speech_intervals` 以外の無音区間を対象音声レイヤー＆連動レイヤーから一括カット＆左詰めする
/// `EditCommand::Batch`（1回の Ctrl+Z で全復元可能）を構築する。
/// 戻り値: `(保持区間数, 削除した無音合計秒数, Batchコマンド)`
pub fn build_jump_cut_commands(
    scene: &Scene,
    audio_layer_index: usize,
    speech_intervals: &[SpeechInterval],
    ripple_linked_layers: &[usize],
) -> Option<(usize, f64, EditCommand)> {
    if speech_intervals.is_empty() {
        return None;
    }
    let mut sorted = speech_intervals.to_vec();
    sorted.sort_by_key(|iv| iv.start);

    // 各保持区間を詰めた後の新開始時刻テーブルを構築
    let mut mapped: Vec<(SpeechInterval, RationalTime)> = Vec::with_capacity(sorted.len());
    let mut out_cursor = sorted[0].start.min(RationalTime::ZERO);
    let mut kept_total_sec = 0.0f64;
    for iv in &sorted {
        if iv.end <= iv.start {
            continue;
        }
        let dur = iv.end - iv.start;
        mapped.push((*iv, out_cursor));
        out_cursor = out_cursor + dur;
        kept_total_sec += dur.as_seconds();
    }
    if mapped.is_empty() {
        return None;
    }

    let orig_span_sec = (sorted.last().unwrap().end - sorted.first().unwrap().start)
        .as_seconds()
        .max(0.0);
    let removed_sec = (orig_span_sec - kept_total_sec).max(0.0);

    // 対象レイヤー集合（重複排除）
    let mut target_layers: Vec<usize> = vec![audio_layer_index];
    for &l in ripple_linked_layers {
        if l < scene.layers.len() && !target_layers.contains(&l) {
            target_layers.push(l);
        }
    }

    let mut commands = Vec::new();
    for &layer_idx in &target_layers {
        let Some(layer) = scene.layers.get(layer_idx) else {
            continue;
        };
        for item in &layer.items {
            let mut slices: Vec<TimelineItem> = Vec::new();
            for (iv, new_iv_start) in &mapped {
                let ov_start = item.start.max(iv.start);
                let ov_end = item.end().min(iv.end);
                if ov_end <= ov_start {
                    continue;
                }
                let slice_dur = ov_end - ov_start;
                let offset_into_item = ov_start - item.start;
                let offset_into_iv = ov_start - iv.start;
                let mut cloned = item.clone();
                cloned.id = Uuid::new_v4();
                cloned.start = *new_iv_start + offset_into_iv;
                cloned.duration = slice_dur;
                match &mut cloned.kind {
                    ItemKind::Video { source_offset, .. }
                    | ItemKind::Audio { source_offset, .. } => {
                        let speed = f64::from(item.playback.speed).abs().max(0.01);
                        let delta_src =
                            RationalTime::from_seconds(offset_into_item.as_seconds() * speed);
                        *source_offset = *source_offset + delta_src;
                    }
                    _ => {}
                }
                slices.push(cloned);
            }

            // 元アイテムを削除し、切り出した有音スライス群を追加
            commands.push(EditCommand::DeleteObject { item_id: item.id });
            for slice in slices {
                commands.push(EditCommand::AddObject {
                    layer_index: layer_idx,
                    item: slice,
                });
            }
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some((
            mapped.len(),
            removed_sec,
            EditCommand::Batch {
                label: "auto_jump_cut".into(),
                commands,
            },
        ))
    }
}

/// 選択中アイテムを削除し、同じレイヤー上で後続するクリップを削除尺分だけ左へ詰める（リップル削除）
pub fn build_ripple_delete_commands(scene: &Scene, item_ids: &[ItemId]) -> Option<EditCommand> {
    let selected: HashSet<ItemId> = item_ids.iter().copied().collect();
    if selected.is_empty() {
        return None;
    }

    let mut commands = Vec::new();
    for (layer_idx, layer) in scene.layers.iter().enumerate() {
        let deleted: Vec<&TimelineItem> = layer
            .items
            .iter()
            .filter(|i| selected.contains(&i.id))
            .collect();
        if deleted.is_empty() {
            continue;
        }
        for d in &deleted {
            commands.push(EditCommand::DeleteObject { item_id: d.id });
        }
        // 各非選択アイテムについて、自分より前で削除されたアイテム群の合計尺分だけ左へ詰める
        for item in &layer.items {
            if selected.contains(&item.id) {
                continue;
            }
            let shift_ticks: i64 = deleted
                .iter()
                .filter(|d| d.start < item.start)
                .map(|d| d.duration.ticks)
                .sum();
            if shift_ticks > 0 {
                let new_start = RationalTime::from_ticks((item.start.ticks - shift_ticks).max(0));
                commands.push(EditCommand::MoveObject {
                    item_id: item.id,
                    target_layer_index: layer_idx,
                    new_start,
                });
            }
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some(EditCommand::Batch {
            label: "ripple_delete".into(),
            commands,
        })
    }
}

/// タイムラインマーカー列から YouTube 概要欄へそのまま貼れるチャプターテキストを生成する
pub fn format_youtube_chapters(markers: &[crate::project::TimelineMarker]) -> String {
    if markers.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&crate::project::TimelineMarker> = markers.iter().collect();
    sorted.sort_by_key(|m| m.time);

    let max_sec = sorted.last().map(|m| m.time.as_seconds()).unwrap_or(0.0);
    let include_hours = max_sec >= 3600.0;

    let fmt_ts = |secs: f64| -> String {
        let total = secs.max(0.0).floor() as u64;
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        if include_hours {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m:02}:{s:02}")
        }
    };

    let mut lines = Vec::new();
    if sorted[0].time.as_seconds() >= 1.0 {
        lines.push(format!("{} イントロ", fmt_ts(0.0)));
    }
    for m in sorted {
        let label = if m.label.trim().is_empty() {
            "チャプター"
        } else {
            m.label.trim()
        };
        lines.push(format!("{} {label}", fmt_ts(m.time.as_seconds())));
    }
    lines.join("\n")
}

/// 字幕テキストを日本語で読みやすい文字数（既定18文字）へ自動改行整形する（オフライン差分プレビュー＆MCP用）
pub fn wrap_subtitle_japanese(text: &str, max_chars_per_line: usize) -> String {
    let limit = max_chars_per_line.max(6);
    let cleaned = text.replace('\r', "").replace("  ", " ");
    let mut out_lines = Vec::new();
    for raw_line in cleaned.split('\n') {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() <= limit {
            out_lines.push(trimmed.to_string());
            continue;
        }
        let mut start = 0usize;
        while start < chars.len() {
            let end = (start + limit).min(chars.len());
            if end == chars.len() {
                out_lines.push(chars[start..end].iter().collect());
                break;
            }
            // 句読点やスペースが終端付近にあればそこで自然に改行
            let break_pos = (start + (limit / 2)..=end)
                .rev()
                .find(|&idx| {
                    idx < chars.len()
                        && matches!(
                            chars[idx - 1],
                            '。' | '、' | '！' | '？' | '!' | '?' | ' ' | '　'
                        )
                })
                .unwrap_or(end);
            out_lines.push(
                chars[start..break_pos]
                    .iter()
                    .collect::<String>()
                    .trim()
                    .to_string(),
            );
            start = break_pos;
        }
    }
    out_lines.join("\n")
}

/// v0.8 #12: 選択クリップ群をビート時刻で一括自動分割する（1ステップ Undo）
pub fn build_beat_cut_commands(
    scene: &Scene,
    item_ids: &[ItemId],
    beat_times_sec: &[f64],
) -> Option<(usize, EditCommand)> {
    if item_ids.is_empty() || beat_times_sec.is_empty() {
        return None;
    }
    let selected: HashSet<ItemId> = item_ids.iter().copied().collect();
    let mut beats: Vec<f64> = beat_times_sec
        .iter()
        .copied()
        .filter(|t| t.is_finite() && *t > 0.0)
        .collect();
    beats.sort_by(|a, b| a.total_cmp(b));
    beats.dedup_by(|a, b| (*a - *b).abs() < 0.02);

    let mut commands = Vec::new();
    let mut cuts = 0usize;
    for layer in &scene.layers {
        for item in &layer.items {
            if !selected.contains(&item.id) {
                continue;
            }
            let min_sec = item.start.as_seconds() + 0.05;
            let max_sec = item.end().as_seconds() - 0.05;
            let mut cur_id = item.id;
            let mut cur_start = min_sec;
            for &b_sec in &beats {
                if b_sec > cur_start && b_sec < max_sec {
                    let right_id = Uuid::new_v4();
                    commands.push(EditCommand::SplitObject {
                        item_id: cur_id,
                        split_at: RationalTime::from_seconds(b_sec),
                        new_right_id: right_id,
                    });
                    cur_id = right_id;
                    cur_start = b_sec + 0.05;
                    cuts += 1;
                }
            }
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some((
            cuts,
            EditCommand::Batch {
                label: "beat_auto_cut".into(),
                commands,
            },
        ))
    }
}

/// v0.8 #12: 選択クリップの開始位置を最寄りのビート位置へスナップ整列する（1ステップ Undo）
pub fn build_snap_to_beats_commands(
    scene: &Scene,
    item_ids: &[ItemId],
    beat_times_sec: &[f64],
) -> Option<EditCommand> {
    if item_ids.is_empty() || beat_times_sec.is_empty() {
        return None;
    }
    let selected: HashSet<ItemId> = item_ids.iter().copied().collect();
    let mut commands = Vec::new();
    for (l_idx, layer) in scene.layers.iter().enumerate() {
        for item in &layer.items {
            if !selected.contains(&item.id) {
                continue;
            }
            let s = item.start.as_seconds();
            if let Some(&nearest) = beat_times_sec
                .iter()
                .filter(|t| t.is_finite() && **t >= 0.0)
                .min_by(|a, b| (s - **a).abs().total_cmp(&(s - **b).abs()))
            {
                let target = RationalTime::from_seconds(nearest);
                if target != item.start {
                    commands.push(EditCommand::MoveObject {
                        item_id: item.id,
                        target_layer_index: l_idx,
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
            label: "snap_to_beats".into(),
            commands,
        })
    }
}

/// v0.8 #12: 検出されたビート位置（`(time_sec, is_downbeat)`）をタイムラインマーカーとして一括配置する
pub fn build_beat_markers_command(
    beats: &[(f64, bool)],
    downbeats_only: bool,
) -> Option<EditCommand> {
    use crate::project::{RgbaColor, TimelineMarker};
    let gold = RgbaColor {
        r: 0.98,
        g: 0.80,
        b: 0.22,
        a: 1.0,
    };
    let cyan = RgbaColor {
        r: 0.25,
        g: 0.82,
        b: 0.98,
        a: 1.0,
    };

    let mut commands = Vec::new();
    let mut bar = 0usize;
    let mut beat_in_bar = 0usize;
    for &(t, is_downbeat) in beats {
        if !t.is_finite() || t < 0.0 {
            continue;
        }
        if is_downbeat {
            bar += 1;
            beat_in_bar = 1;
        } else {
            if bar == 0 {
                bar = 1;
            }
            beat_in_bar += 1;
        }
        if downbeats_only && !is_downbeat {
            continue;
        }
        let label = if is_downbeat {
            format!("♩ Bar {bar}")
        } else {
            format!("· {bar}.{beat_in_bar}")
        };
        commands.push(EditCommand::AddMarker {
            marker: TimelineMarker {
                id: Uuid::new_v4(),
                time: RationalTime::from_seconds(t),
                label,
                color: if is_downbeat { gold } else { cyan },
            },
        });
    }
    if commands.is_empty() {
        None
    } else {
        Some(EditCommand::Batch {
            label: "add_beat_markers".into(),
            commands,
        })
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

    #[test]
    fn jump_cut_removes_silence_and_syncs_linked_layers_with_single_undo() {
        // 10秒間の波形: [0.5..2.0s] と [6.0..8.0s] が発話、他は無音
        let mut rms = vec![0.002f32; 100]; // 100ms per bucket
        for b in &mut rms[5..20] {
            *b = 0.2;
        }
        for b in &mut rms[60..80] {
            *b = 0.25;
        }
        let cfg = JumpCutConfig {
            rms_threshold: 0.05,
            min_silence_sec: 0.5,
            pad_before_sec: 0.1,
            pad_after_sec: 0.1,
            ripple_linked_layers: vec![0, 1],
        };
        let intervals = detect_speech_intervals_from_rms(&rms, 0.1, 10.0, &cfg);
        assert_eq!(intervals.len(), 2);

        let mut proj = Project::new("JumpCutTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        {
            let scene = proj.active_scene_mut().unwrap();
            scene.layers[0].items.push(TimelineItem::new(
                "MainVideo",
                RationalTime::ZERO,
                RationalTime::from_seconds(10.0),
                ItemKind::Video {
                    asset_id: Uuid::new_v4(),
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            ));
            scene.layers[3].items.push(TimelineItem::new(
                "MainVoice",
                RationalTime::ZERO,
                RationalTime::from_seconds(10.0),
                ItemKind::Audio {
                    asset_id: Uuid::new_v4(),
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            ));
        }

        let (kept, removed_sec, cmd) = build_jump_cut_commands(
            proj.active_scene().unwrap(),
            3,
            &intervals,
            &cfg.ripple_linked_layers,
        )
        .expect("jump cut commands");
        assert_eq!(kept, 2);
        assert!(removed_sec > 3.0);

        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        let scene = proj.active_scene().unwrap();
        // 映像・音声どちらも2スライスになり、合計尺が短縮されている
        assert_eq!(scene.layers[0].items.len(), 2);
        assert_eq!(scene.layers[3].items.len(), 2);
        assert!(scene.total_duration().as_seconds() < 5.0);
        // 2つ目のスライスの source_offset が発話開始位置へ進んでいる
        if let ItemKind::Video { source_offset, .. } = scene.layers[0].items[1].kind {
            assert!(source_offset.as_seconds() > 5.5);
        } else {
            panic!("expected video slice");
        }

        // 1回の Undo で元の10秒1本クリップへ完全復元
        assert!(bus.undo(&mut proj));
        assert_eq!(proj.active_scene().unwrap().layers[0].items.len(), 1);
        assert!((proj.active_scene().unwrap().total_duration().as_seconds() - 10.0).abs() < 1e-4);
    }

    #[test]
    fn ripple_delete_closes_gap_for_subsequent_clips() {
        let mut proj = Project::new("RippleTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let a = make_text_item("A", 0.0, 2.0, "A");
        let b = make_text_item("B", 2.0, 3.0, "B");
        let c = make_text_item("C", 5.0, 2.0, "C");
        let b_id = b.id;
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .extend([a, b, c]);

        let cmd =
            build_ripple_delete_commands(proj.active_scene().unwrap(), &[b_id]).expect("ripple");
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        let items = &proj.active_scene().unwrap().layers[1].items;
        assert_eq!(items.len(), 2);
        assert!((items[1].start.as_seconds() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn formats_youtube_chapters_and_wraps_japanese_subtitles() {
        use crate::project::TimelineMarker;

        let markers = vec![
            TimelineMarker::new(RationalTime::from_seconds(75.0), "本編解説"),
            TimelineMarker::new(RationalTime::from_seconds(10.0), "オープニング"),
        ];
        let chapters = format_youtube_chapters(&markers);
        assert_eq!(
            chapters,
            "00:00 イントロ\n00:10 オープニング\n01:15 本編解説"
        );

        let wrapped = wrap_subtitle_japanese(
            "今日は次世代ハイブリッド動画編集ソフトの最新機能を分かりやすく解説します。",
            18,
        );
        assert!(wrapped.contains('\n'));
        assert!(wrapped.lines().all(|l| l.chars().count() <= 18));
    }

    #[test]
    fn beat_auto_cut_snap_and_markers_work_with_single_undo() {
        let mut proj = Project::new("BeatOpsTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let clip = make_text_item("LongClip", 0.12, 2.5, "音ハメ");
        let clip_id = clip.id;
        proj.active_scene_mut().unwrap().layers[1].items.push(clip);

        let beats = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5];
        let snap_cmd =
            build_snap_to_beats_commands(proj.active_scene().unwrap(), &[clip_id], &beats).unwrap();
        bus.execute(&mut proj, snap_cmd, CommandSource::Gui, None)
            .unwrap();
        assert!(
            (proj.active_scene().unwrap().layers[1].items[0]
                .start
                .as_seconds()
                - 0.0)
                .abs()
                < 1e-6
        );

        let (cuts, cut_cmd) =
            build_beat_cut_commands(proj.active_scene().unwrap(), &[clip_id], &beats).unwrap();
        assert_eq!(cuts, 4);
        bus.execute(&mut proj, cut_cmd, CommandSource::Gui, None)
            .unwrap();
        assert_eq!(proj.active_scene().unwrap().layers[1].items.len(), 5);

        let beat_pairs: Vec<(f64, bool)> = beats
            .iter()
            .enumerate()
            .map(|(i, &t)| (t, i % 4 == 0))
            .collect();
        let mk_cmd = build_beat_markers_command(&beat_pairs, true).unwrap();
        bus.execute(&mut proj, mk_cmd, CommandSource::Gui, None)
            .unwrap();
        assert_eq!(proj.active_scene().unwrap().markers.len(), 2);
        assert_eq!(proj.active_scene().unwrap().markers[0].label, "♩ Bar 1");

        assert!(bus.undo(&mut proj));
        assert!(proj.active_scene().unwrap().markers.is_empty());
    }
}
