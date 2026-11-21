//! v0.8 #11: テキストベース編集（文字起こし連動リップルカット・並べ替え・日本語フィラー検出/除去）

use crate::command::EditCommand;
use crate::project::{ItemId, ItemKind, Scene, TimelineItem};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// 既定の日本語・英語フィラー（言いよどみ）語句一覧
pub const DEFAULT_FILLERS: &[&str] = &[
    "えーっと",
    "えーと",
    "ええと",
    "えっと",
    "えー",
    "あのー",
    "あの",
    "うーん",
    "うーんと",
    "そのー",
    "まー",
    "まあ",
    "なんか",
    "um",
    "uh",
    "er",
];

/// 文字起こし／字幕アイテム 1 行分の構造化エントリ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub item_id: ItemId,
    pub layer_index: usize,
    pub start: RationalTime,
    pub end: RationalTime,
    pub raw_text: String,
    pub clean_text: String,
    /// セリフ全体がフィラーのみ（または大半がフィラー）で区間カット候補かどうか
    pub is_filler_only: bool,
    /// 検出されたフィラー語句一覧
    pub matched_fillers: Vec<String>,
}

/// テキストベース削除コマンド生成結果
#[derive(Debug, Clone)]
pub struct TextEditCutResult {
    pub removed_ranges: usize,
    pub removed_sec: f64,
    pub command: EditCommand,
}

/// AviUtl2 インライン制御タグ `<...>` を除去してプレーンテキスト化する
pub fn strip_control_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn is_punct_or_space(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '、' | '。'
                | ','
                | '.'
                | '!'
                | '?'
                | '！'
                | '？'
                | '…'
                | '・'
                | 'ー'
                | '-'
                | '〜'
                | '~'
        )
}

/// テキストに含まれるフィラー語句を検出し、全体がフィラーのみかどうかも判定する
pub fn analyze_fillers(clean_text: &str, fillers: &[&str]) -> (bool, Vec<String>) {
    let mut matched = Vec::new();
    let trimmed = clean_text.trim();
    if trimmed.is_empty() {
        return (false, matched);
    }

    // 長い語句から順に判定して部分一致の重複を防止
    let mut sorted_fillers: Vec<&str> = fillers.iter().copied().filter(|f| !f.is_empty()).collect();
    sorted_fillers.sort_by_key(|f| std::cmp::Reverse(f.chars().count()));

    let lower = trimmed.to_lowercase();
    let mut residue = lower.clone();
    for f in &sorted_fillers {
        let fl = f.to_lowercase();
        let is_ascii_word = fl.is_ascii();
        let found = if is_ascii_word {
            // 英語フィラー ("um", "uh", "er") は単語境界で判定（"number" 等への誤爆防止）
            residue
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|tok| tok == fl)
        } else {
            residue.contains(&fl)
        };
        if found {
            if !matched.iter().any(|m: &String| m.eq_ignore_ascii_case(f)) {
                matched.push((*f).to_string());
            }
            if is_ascii_word {
                let mut rebuilt = String::with_capacity(residue.len());
                for tok in residue.split_inclusive(|c: char| !c.is_ascii_alphanumeric()) {
                    let word = tok.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
                    let tail = &tok[word.len()..];
                    if word == fl {
                        rebuilt.push_str(tail);
                    } else {
                        rebuilt.push_str(tok);
                    }
                }
                residue = rebuilt;
            } else {
                residue = residue.replace(&fl, "");
            }
        }
    }

    let remaining_meaningful: String = residue.chars().filter(|c| !is_punct_or_space(*c)).collect();
    let is_filler_only = !matched.is_empty() && remaining_meaningful.is_empty();
    (is_filler_only, matched)
}

/// アクティブシーン内のテキストオブジェクトを時系列の文字起こし一覧として抽出する
pub fn extract_transcript_entries(
    scene: &Scene,
    subtitle_layer_filter: Option<usize>,
    custom_fillers: Option<&[&str]>,
) -> Vec<TranscriptEntry> {
    let fillers = custom_fillers.unwrap_or(DEFAULT_FILLERS);
    let mut entries = Vec::new();
    for (l_idx, layer) in scene.layers.iter().enumerate() {
        if let Some(target) = subtitle_layer_filter
            && l_idx != target
        {
            continue;
        }
        for item in &layer.items {
            if let ItemKind::Text { content, .. } = &item.kind {
                let clean = strip_control_tags(content);
                let (is_filler_only, matched_fillers) = analyze_fillers(&clean, fillers);
                entries.push(TranscriptEntry {
                    item_id: item.id,
                    layer_index: l_idx,
                    start: item.start,
                    end: item.end(),
                    raw_text: content.clone(),
                    clean_text: clean,
                    is_filler_only,
                    matched_fillers,
                });
            }
        }
    }
    entries.sort_by_key(|e| (e.start, e.layer_index));
    entries
}

/// 字幕内のフィラー語句（例: `"えーと、今日は"` → `"今日は"`）を非破壊 1 ステップ Undo で一括除去する
pub fn build_strip_filler_words_commands(
    scene: &Scene,
    subtitle_layer_filter: Option<usize>,
    custom_fillers: Option<&[&str]>,
) -> Option<(usize, EditCommand)> {
    let fillers = custom_fillers.unwrap_or(DEFAULT_FILLERS);
    let mut sorted_fillers: Vec<&str> = fillers.iter().copied().filter(|f| !f.is_empty()).collect();
    sorted_fillers.sort_by_key(|f| std::cmp::Reverse(f.chars().count()));

    let mut commands = Vec::new();
    for (l_idx, layer) in scene.layers.iter().enumerate() {
        if let Some(target) = subtitle_layer_filter
            && l_idx != target
        {
            continue;
        }
        for item in &layer.items {
            if let ItemKind::Text { content, .. } = &item.kind {
                let mut next = content.clone();
                for f in &sorted_fillers {
                    if !f.is_ascii() {
                        // 日本語フィラー直後の読点も含めて綺麗に除去
                        next = next.replace(&format!("{f}、"), "");
                        next = next.replace(&format!("{f}，"), "");
                        next = next.replace(f, "");
                    }
                }
                let cleaned = next
                    .trim_start_matches(['、', '。', '…', ' ', '　'])
                    .trim_end()
                    .to_string();
                let has_meaningful = cleaned.chars().any(|c| !is_punct_or_space(c));
                if cleaned != *content && has_meaningful {
                    commands.push(EditCommand::SetTextContent {
                        item_id: item.id,
                        content: cleaned,
                    });
                }
            }
        }
    }
    if commands.is_empty() {
        None
    } else {
        let count = commands.len();
        Some((
            count,
            EditCommand::Batch {
                label: "strip_filler_words".into(),
                commands,
            },
        ))
    }
}

/// クリップの指定時間区間 `[cut_start, cut_end]` を切り出し、新開始位置 `new_start` に配置するスライスを生成する
fn slice_item_to_window(
    item: &TimelineItem,
    win_start: RationalTime,
    win_end: RationalTime,
    new_start: RationalTime,
) -> Option<TimelineItem> {
    let overlap_start = item.start.max(win_start);
    let overlap_end = item.end().min(win_end);
    if overlap_end <= overlap_start {
        return None;
    }
    let trim_left = overlap_start - item.start;
    let new_dur = overlap_end - overlap_start;
    let mut sliced = item.clone();
    sliced.id = Uuid::new_v4();
    sliced.start = new_start + (overlap_start - win_start);
    sliced.duration = new_dur;

    match &mut sliced.kind {
        ItemKind::Video { source_offset, .. } | ItemKind::Audio { source_offset, .. } => {
            *source_offset = *source_offset + trim_left;
        }
        _ => {}
    }
    if trim_left.ticks > 0 {
        for kfs in sliced.keyframes.values_mut() {
            for kf in kfs.iter_mut() {
                kf.time = RationalTime::from_ticks((kf.time.ticks - trim_left.ticks).max(0));
            }
        }
    }
    for eff in &mut sliced.effect_chain {
        eff.id = Uuid::new_v4();
    }
    for eff in &mut sliced.audio_effects {
        eff.id = Uuid::new_v4();
    }
    for mask in &mut sliced.masks {
        mask.id = Uuid::new_v4();
    }
    Some(sliced)
}

/// テキストベース削除: 指定した字幕 ID 群の区間を、連動する全レイヤー（映像・音声・字幕等）から
/// まとめてカット＆左へリップル詰めする 1 ステップ Undo コマンドを生成する。
pub fn build_text_based_delete_commands(
    scene: &Scene,
    target_text_ids: &[ItemId],
    linked_layers: &[usize],
) -> Option<TextEditCutResult> {
    if target_text_ids.is_empty() {
        return None;
    }
    let target_set: HashSet<ItemId> = target_text_ids.iter().copied().collect();
    let mut raw_cuts: Vec<(RationalTime, RationalTime)> = Vec::new();
    let mut affected_layers: HashSet<usize> = linked_layers
        .iter()
        .copied()
        .filter(|&l| l < scene.layers.len())
        .collect();

    for (l_idx, layer) in scene.layers.iter().enumerate() {
        for item in &layer.items {
            if target_set.contains(&item.id) && item.end() > item.start {
                raw_cuts.push((item.start, item.end()));
                affected_layers.insert(l_idx);
            }
        }
    }
    if raw_cuts.is_empty() {
        return None;
    }

    // 重複・隣接するカット区間を統合
    raw_cuts.sort_by_key(|(s, _)| *s);
    let mut merged_cuts: Vec<(RationalTime, RationalTime)> = Vec::new();
    for (s, e) in raw_cuts {
        if let Some(last) = merged_cuts.last_mut()
            && s <= last.1
        {
            if e > last.1 {
                last.1 = e;
            }
        } else {
            merged_cuts.push((s, e));
        }
    }

    let total_end = scene.total_duration();
    // 保持区間（全タイムライン \ merged_cuts）を構築
    let mut kept_windows: Vec<(RationalTime, RationalTime)> = Vec::new();
    let mut cursor = RationalTime::ZERO;
    for &(cs, ce) in &merged_cuts {
        if cs > cursor {
            kept_windows.push((cursor, cs));
        }
        cursor = cursor.max(ce);
    }
    if total_end > cursor {
        kept_windows.push((cursor, total_end));
    }

    let removed_sec: f64 = merged_cuts
        .iter()
        .map(|(s, e)| (*e - *s).as_seconds())
        .sum();

    let mut layer_indices: Vec<usize> = affected_layers.into_iter().collect();
    layer_indices.sort_unstable();

    let mut commands = Vec::new();
    for l_idx in layer_indices {
        let layer = &scene.layers[l_idx];
        for item in &layer.items {
            // カット対象 ID 本体は完全削除
            if target_set.contains(&item.id) {
                commands.push(EditCommand::DeleteObject { item_id: item.id });
                continue;
            }
            // このアイテムに対してカット区間が影響する（重なる or 自分より前にカットがある）か判定
            let first_cut_start = merged_cuts[0].0;
            if item.end() <= first_cut_start {
                continue;
            }
            commands.push(EditCommand::DeleteObject { item_id: item.id });
            let mut dst_cursor = RationalTime::ZERO;
            for &(ws, we) in &kept_windows {
                if let Some(sliced) = slice_item_to_window(item, ws, we, dst_cursor) {
                    commands.push(EditCommand::AddObject {
                        layer_index: l_idx,
                        item: sliced,
                    });
                }
                dst_cursor = dst_cursor + (we - ws);
            }
        }
    }

    if commands.is_empty() {
        return None;
    }

    Some(TextEditCutResult {
        removed_ranges: merged_cuts.len(),
        removed_sec,
        command: EditCommand::Batch {
            label: "text_based_delete".into(),
            commands,
        },
    })
}

/// テキストベース並べ替え: 指定字幕の区間を隣接する前後の字幕区間と入れ替え、
/// 連動する映像・音声・字幕クリップを 1 ステップ Undo でまとめてスワップする。
pub fn build_text_based_reorder_commands(
    scene: &Scene,
    move_text_id: ItemId,
    move_up: bool,
    linked_layers: &[usize],
) -> Option<EditCommand> {
    let entries = extract_transcript_entries(scene, None, None);
    let pos = entries.iter().position(|e| e.item_id == move_text_id)?;
    let neighbor_pos = if move_up {
        pos.checked_sub(1)?
    } else {
        let next = pos + 1;
        if next >= entries.len() {
            return None;
        }
        next
    };

    let (first, second) = if pos < neighbor_pos {
        (&entries[pos], &entries[neighbor_pos])
    } else {
        (&entries[neighbor_pos], &entries[pos])
    };
    if first.end > second.start {
        // 完全に重なっている字幕同士は安全のため入れ替え不可
        return None;
    }

    let a_start = first.start;
    let a_end = first.end;
    let b_start = second.start;
    let b_end = second.end;
    let gap = b_start - a_end;
    let b_dur = b_end - b_start;

    // 入れ替え後の配置: [a_start .. a_start + b_dur] に旧 B、gap 後に旧 A
    let new_b_start = a_start;
    let new_a_start = a_start + b_dur + gap;

    let mut affected_layers: HashSet<usize> = linked_layers
        .iter()
        .copied()
        .filter(|&l| l < scene.layers.len())
        .collect();
    affected_layers.insert(first.layer_index);
    affected_layers.insert(second.layer_index);
    let mut layer_indices: Vec<usize> = affected_layers.into_iter().collect();
    layer_indices.sort_unstable();

    let total_end = scene.total_duration().max(b_end);
    // タイムラインを 5 区間に分割: [0..a_start], [a_start..a_end], [a_end..b_start], [b_start..b_end], [b_end..total_end]
    // 並べ替え後の出力マッピング (src_win_start, src_win_end, dst_start)
    let windows: Vec<(RationalTime, RationalTime, RationalTime)> = vec![
        (RationalTime::ZERO, a_start, RationalTime::ZERO),
        (b_start, b_end, new_b_start),
        (a_end, b_start, a_start + b_dur),
        (a_start, a_end, new_a_start),
        (b_end, total_end, b_end),
    ];

    let mut commands = Vec::new();
    for l_idx in layer_indices {
        let layer = &scene.layers[l_idx];
        for item in &layer.items {
            // [a_start, b_end] に一切かからないアイテムはそのまま維持
            if item.end() <= a_start || item.start >= b_end {
                continue;
            }
            commands.push(EditCommand::DeleteObject { item_id: item.id });
            for &(ws, we, dst) in &windows {
                if we <= ws {
                    continue;
                }
                if let Some(sliced) = slice_item_to_window(item, ws, we, dst) {
                    commands.push(EditCommand::AddObject {
                        layer_index: l_idx,
                        item: sliced,
                    });
                }
            }
        }
    }

    if commands.is_empty() {
        None
    } else {
        Some(EditCommand::Batch {
            label: "text_based_reorder".into(),
            commands,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::{Project, ProjectSettings, RgbaColor, TextAnimator};

    fn make_text(name: &str, start_s: f64, dur_s: f64, text: &str) -> TimelineItem {
        TimelineItem::new(
            name,
            RationalTime::from_seconds(start_s),
            RationalTime::from_seconds(dur_s),
            ItemKind::Text {
                content: text.to_string(),
                font_family: "sans-serif".to_string(),
                font_size: 44.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        )
    }

    #[test]
    fn filler_detection_and_word_stripping_handle_japanese_fillers() {
        let mut proj = Project::new("FillerTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        {
            let sc = proj.active_scene_mut().unwrap();
            sc.layers[1]
                .items
                .push(make_text("S1", 0.0, 1.0, "えーと…"));
            sc.layers[1].items.push(make_text(
                "S2",
                1.0,
                2.0,
                "あのー、今日は動画編集を紹介します",
            ));
            sc.layers[1]
                .items
                .push(make_text("S3", 3.0, 2.0, "number 1 の機能です"));
        }

        let entries = extract_transcript_entries(proj.active_scene().unwrap(), Some(1), None);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_filler_only, "S1 should be pure filler");
        assert!(!entries[1].is_filler_only);
        assert!(!entries[1].matched_fillers.is_empty());
        assert!(
            entries[2].matched_fillers.is_empty(),
            "'number' must not trigger English 'um' filler"
        );

        let (count, cmd) =
            build_strip_filler_words_commands(proj.active_scene().unwrap(), Some(1), None).unwrap();
        assert_eq!(count, 1);
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        if let ItemKind::Text { content, .. } =
            &proj.active_scene().unwrap().layers[1].items[1].kind
        {
            assert_eq!(content, "今日は動画編集を紹介します");
        } else {
            panic!("expected text");
        }
    }

    #[test]
    fn text_based_delete_ripple_cuts_video_audio_and_subtitles_in_one_undo() {
        let mut proj = Project::new("TextDelTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let s1 = make_text("S1", 0.0, 2.0, "こんにちは");
        let s2 = make_text("S2", 2.0, 2.0, "えーと");
        let s3 = make_text("S3", 4.0, 2.0, "本編です");
        let s2_id = s2.id;
        {
            let sc = proj.active_scene_mut().unwrap();
            sc.layers[0].items.push(TimelineItem::new(
                "MainVid",
                RationalTime::ZERO,
                RationalTime::from_seconds(6.0),
                ItemKind::Video {
                    asset_id: Uuid::new_v4(),
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            ));
            sc.layers[1].items.extend([s1, s2, s3]);
            sc.layers[3].items.push(TimelineItem::new(
                "MainAud",
                RationalTime::ZERO,
                RationalTime::from_seconds(6.0),
                ItemKind::Audio {
                    asset_id: Uuid::new_v4(),
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            ));
        }

        let res =
            build_text_based_delete_commands(proj.active_scene().unwrap(), &[s2_id], &[0, 1, 3])
                .expect("delete result");
        assert_eq!(res.removed_ranges, 1);
        assert!((res.removed_sec - 2.0).abs() < 1e-6);

        bus.execute(&mut proj, res.command, CommandSource::Gui, None)
            .unwrap();
        let sc = proj.active_scene().unwrap();
        assert_eq!(sc.layers[1].items.len(), 2);
        assert!((sc.layers[1].items[1].start.as_seconds() - 2.0).abs() < 1e-6);
        assert_eq!(sc.layers[0].items.len(), 2);
        assert_eq!(sc.layers[3].items.len(), 2);
        // 後半動画スライスの source_offset が 4.0s になっていること
        if let ItemKind::Video { source_offset, .. } = sc.layers[0].items[1].kind {
            assert!((source_offset.as_seconds() - 4.0).abs() < 1e-6);
        } else {
            panic!("expected video");
        }
        assert!((sc.total_duration().as_seconds() - 4.0).abs() < 1e-6);

        // 1回の Undo で完全復元
        assert!(bus.undo(&mut proj));
        assert_eq!(proj.active_scene().unwrap().layers[1].items.len(), 3);
        assert!((proj.active_scene().unwrap().total_duration().as_seconds() - 6.0).abs() < 1e-6);
    }

    #[test]
    fn text_based_reorder_swaps_adjacent_segments_with_linked_audio() {
        let mut proj = Project::new("TextReorderTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let s1 = make_text("First", 0.0, 2.0, "最初");
        let s2 = make_text("Second", 2.0, 3.0, "二番目");
        let s2_id = s2.id;
        {
            let sc = proj.active_scene_mut().unwrap();
            sc.layers[1].items.extend([s1, s2]);
            sc.layers[3].items.push(TimelineItem::new(
                "Voice",
                RationalTime::ZERO,
                RationalTime::from_seconds(5.0),
                ItemKind::Audio {
                    asset_id: Uuid::new_v4(),
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            ));
        }

        let cmd =
            build_text_based_reorder_commands(proj.active_scene().unwrap(), s2_id, true, &[1, 3])
                .expect("reorder cmd");
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();

        let entries = extract_transcript_entries(proj.active_scene().unwrap(), Some(1), None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].clean_text, "二番目");
        assert!((entries[0].start.as_seconds() - 0.0).abs() < 1e-6);
        assert_eq!(entries[1].clean_text, "最初");
        assert!((entries[1].start.as_seconds() - 3.0).abs() < 1e-6);

        // 音声レイヤーも入れ替わり、先頭スライスが元 2.0s〜 の source_offset を持つ
        let mut audio_items = proj.active_scene().unwrap().layers[3].items.clone();
        audio_items.sort_by_key(|i| i.start);
        assert_eq!(audio_items.len(), 2);
        if let ItemKind::Audio { source_offset, .. } = audio_items[0].kind {
            assert!((source_offset.as_seconds() - 2.0).abs() < 1e-6);
        } else {
            panic!("expected audio");
        }
    }
}
