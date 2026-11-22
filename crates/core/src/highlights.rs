use crate::animation::{Interpolation, Keyframe};
use crate::command::EditCommand;
use crate::project::{
    AudioDuckingConfig, ItemKind, Layer, Project, Scene, SceneId, TextStyle, TimelineItem,
    TrackPointSample,
};
use crate::time::RationalTime;
use crate::timeline_ops::wrap_subtitle_japanese;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// v0.7 #10: AI ハイライト自動抽出（見どころ区間スコアリング）設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HighlightConfig {
    pub audio_layer_index: usize,
    pub target_clip_count: usize,
    pub clip_duration_sec: f64,
    pub min_gap_sec: f64,
    pub weight_audio_energy: f32,
    pub weight_speech_density: f32,
    pub weight_markers_and_keywords: f32,
    pub excitation_keywords: Vec<String>,
}

impl Default for HighlightConfig {
    fn default() -> Self {
        Self {
            audio_layer_index: 3,
            target_clip_count: 3,
            clip_duration_sec: 15.0,
            min_gap_sec: 3.0,
            weight_audio_energy: 0.50,
            weight_speech_density: 0.30,
            weight_markers_and_keywords: 0.20,
            excitation_keywords: vec![
                "！".into(),
                "!".into(),
                "すごい".into(),
                "ヤバい".into(),
                "やばい".into(),
                "神".into(),
                "爆笑".into(),
                "重要".into(),
                "ポイント".into(),
                "結論".into(),
                "まとめ".into(),
                "驚".into(),
            ],
        }
    }
}

/// v0.7 #10: 抽出されたハイライト候補区間
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HighlightCandidate {
    pub start: RationalTime,
    pub end: RationalTime,
    pub score: f32,
    pub reason: String,
    pub suggested_title: String,
}

/// シーンの音声エネルギー（`(time_sec, rms)` 列）・字幕密度・強調キーワード・マーカーを複合評価し、
/// 見どころハイライト区間を非重複（NMS）で抽出する。
pub fn extract_highlight_candidates(
    scene: &Scene,
    rms_samples: &[(f64, f32)],
    cfg: &HighlightConfig,
) -> Vec<HighlightCandidate> {
    let total_sec = scene.total_duration().as_seconds();
    if total_sec <= 0.2 {
        return Vec::new();
    }
    let clip_dur = cfg.clip_duration_sec.clamp(1.5, 120.0).min(total_sec);
    let step_sec = (clip_dur * 0.25).clamp(0.5, 2.0);
    let target_count = cfg.target_clip_count.clamp(1, 20);
    let min_gap = cfg.min_gap_sec.max(0.0);

    // 字幕（Text）アイテムを収集
    let mut texts: Vec<(f64, f64, &str)> = Vec::new();
    for layer in &scene.layers {
        if !layer.visible {
            continue;
        }
        for item in &layer.items {
            if let ItemKind::Text { content, .. } = &item.kind {
                texts.push((item.start.as_seconds(), item.end().as_seconds(), content));
            }
        }
    }

    let max_rms = rms_samples
        .iter()
        .map(|(_, r)| *r)
        .fold(0.0_f32, f32::max)
        .max(1e-4);

    let mut windows: Vec<HighlightCandidate> = Vec::new();
    let max_start = (total_sec - clip_dur).max(0.0);
    let mut start_sec = 0.0;
    let mut safety = 0usize;

    while start_sec <= max_start + 1e-6 && safety < 5000 {
        safety += 1;
        let end_sec = (start_sec + clip_dur).min(total_sec);

        // 1. 音声盛り上がりスコア（ウィンドウ内 RMS ピーク＋平均）
        let mut sum_rms = 0.0f32;
        let mut peak_rms = 0.0f32;
        let mut cnt_rms = 0usize;
        for &(t, r) in rms_samples {
            if t >= start_sec && t <= end_sec {
                sum_rms += r;
                peak_rms = peak_rms.max(r);
                cnt_rms += 1;
            }
        }
        let audio_score = if cnt_rms > 0 {
            let avg = sum_rms / cnt_rms as f32;
            ((avg * 0.55 + peak_rms * 0.45) / max_rms).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // 2. 発話・字幕密度 & 強調キーワード
        let mut char_count = 0usize;
        let mut keyword_hits = 0usize;
        let mut first_sub_snippet: Option<String> = None;
        for &(ts, te, content) in &texts {
            if te > start_sec && ts < end_sec {
                let clean = content.trim();
                char_count += clean.chars().count();
                if first_sub_snippet.is_none() && !clean.is_empty() {
                    let short: String = clean
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(18)
                        .collect();
                    if !short.is_empty() {
                        first_sub_snippet = Some(short);
                    }
                }
                for kw in &cfg.excitation_keywords {
                    if !kw.is_empty() && clean.contains(kw.as_str()) {
                        keyword_hits += 1;
                    }
                }
            }
        }
        let chars_per_sec = char_count as f64 / clip_dur.max(1.0);
        let density_score = ((chars_per_sec / 8.0) as f32).clamp(0.0, 1.0);

        // 3. マーカー & キーワードボーナス
        let mut marker_labels: Vec<&str> = Vec::new();
        for m in &scene.markers {
            let mt = m.time.as_seconds();
            if mt >= start_sec && mt <= end_sec {
                marker_labels.push(m.label.as_str());
            }
        }
        let marker_kw_score =
            ((marker_labels.len() as f32 * 0.55) + (keyword_hits as f32 * 0.30)).clamp(0.0, 1.0);

        let w_sum =
            (cfg.weight_audio_energy + cfg.weight_speech_density + cfg.weight_markers_and_keywords)
                .max(1e-4);
        let total_score = (audio_score * cfg.weight_audio_energy
            + density_score * cfg.weight_speech_density
            + marker_kw_score * cfg.weight_markers_and_keywords)
            / w_sum;

        let mut reasons = Vec::new();
        if audio_score >= 0.55 {
            reasons.push(format!("音量ピーク({:.0}%)", audio_score * 100.0));
        }
        if density_score >= 0.45 {
            reasons.push("高密度トーク".to_string());
        }
        if !marker_labels.is_empty() {
            reasons.push(format!("マーカー「{}」", marker_labels[0]));
        }
        if keyword_hits > 0 {
            reasons.push(format!("強調表現×{keyword_hits}"));
        }
        if reasons.is_empty() {
            reasons.push("標準シーン候補".to_string());
        }

        let suggested_title = marker_labels
            .first()
            .map(|s| (*s).to_string())
            .or(first_sub_snippet)
            .unwrap_or_else(|| format!("Shorts {:.0}s-{:.0}s", start_sec, end_sec));

        windows.push(HighlightCandidate {
            start: RationalTime::from_seconds(start_sec),
            end: RationalTime::from_seconds(end_sec),
            score: total_score.clamp(0.0, 1.0),
            reason: reasons.join(" + "),
            suggested_title,
        });

        if (max_start - start_sec).abs() < 1e-6 {
            break;
        }
        start_sec = (start_sec + step_sec).min(max_start);
    }

    // スコア降順ソート + NMS（重複＆min_gap未満を抑制）
    windows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.start.cmp(&b.start))
    });

    let mut selected: Vec<HighlightCandidate> = Vec::with_capacity(target_count);
    for cand in windows {
        let overlaps = selected.iter().any(|picked| {
            let cs = cand.start.as_seconds();
            let ce = cand.end.as_seconds();
            let ps = picked.start.as_seconds() - min_gap;
            let pe = picked.end.as_seconds() + min_gap;
            ce > ps && cs < pe
        });
        if !overlaps {
            selected.push(cand);
            if selected.len() >= target_count {
                break;
            }
        }
    }
    selected
}

/// 選択したハイライト区間から **9:16 縦動画 (1080×1920) ショートシーン** を自動切り抜き＆字幕リレイアウトし、
/// 1 ステップ Undo 可能な `EditCommand::CreateScene` を生成する。
pub fn build_shorts_clip_scene_command(
    project: &Project,
    source_scene_id: SceneId,
    candidate: &HighlightCandidate,
    shorts_name: impl Into<String>,
) -> Option<(SceneId, EditCommand)> {
    let src = project.scene(source_scene_id)?;
    let clip_start = candidate.start.max(RationalTime::ZERO);
    let clip_end = candidate.end;
    if clip_end <= clip_start {
        return None;
    }

    let target_w = 1080u32;
    let target_h = 1920u32;
    let sx = target_w as f32 / src.width.max(1) as f32;
    let sy = target_h as f32 / src.height.max(1) as f32;
    let uniform = sx.min(sy);
    // 16:9映像を9:16中央へ見やすくフィットさせるスケールとYオフセット
    let fit_y_offset = (target_h as f32 - src.height as f32 * uniform) * 0.5;

    let mut new_layers: Vec<Layer> = Vec::with_capacity(src.layers.len());
    for layer in &src.layers {
        let mut cloned_layer = Layer {
            id: Uuid::new_v4(),
            name: layer.name.clone(),
            visible: layer.visible,
            muted: layer.muted,
            solo: layer.solo,
            offset_x: layer.offset_x * uniform,
            offset_y: layer.offset_y * uniform,
            volume: layer.volume,
            pan: layer.pan,
            bus: layer.bus.clone(),
            send_reverb: layer.send_reverb,
            color_tag: layer.color_tag,
            track_matte: layer.track_matte,
            items: Vec::new(),
        };

        for item in &layer.items {
            let item_end = item.end();
            if item_end <= clip_start || item.start >= clip_end {
                continue;
            }
            let trim_head = (clip_start - item.start).max(RationalTime::ZERO);
            let new_start = (item.start - clip_start).max(RationalTime::ZERO);
            let clamped_end = item_end.min(clip_end);
            let new_dur =
                (clamped_end - item.start.max(clip_start)).max(RationalTime::from_seconds(0.05));

            let mut cut = item.clone();
            cut.id = Uuid::new_v4();
            cut.start = new_start;
            cut.duration = new_dur;

            // 先頭カット分だけキーフレーム時刻をシフト
            for kfs in cut.keyframes.values_mut() {
                for kf in kfs.iter_mut() {
                    kf.time = (kf.time - trim_head).max(RationalTime::ZERO);
                }
            }

            match &mut cut.kind {
                ItemKind::Video { source_offset, .. } | ItemKind::Audio { source_offset, .. } => {
                    *source_offset = *source_offset + trim_head;
                    cut.transform.x *= uniform;
                    cut.transform.y = cut.transform.y * uniform + fit_y_offset;
                }
                ItemKind::Text {
                    content, font_size, ..
                } => {
                    // Shorts/TikTok の下・右 UI 被りを避けたセーフゾーンへ自動リレイアウト＆13文字改行
                    *content = wrap_subtitle_japanese(content, 13);
                    *font_size = (*font_size * 1.15).clamp(42.0, 72.0);
                    cut.transform.x = (target_w as f32 * 0.09).round();
                    cut.transform.y = (target_h as f32 * 0.62).round();
                    cut.text_style = TextStyle {
                        outline_width: cut.text_style.outline_width.max(3.5),
                        shadow_offset: [2.0, 2.0],
                        box_enabled: true,
                        ..cut.text_style
                    };
                }
                ItemKind::Shape { width, height, .. }
                | ItemKind::CharacterSprite { width, height, .. }
                | ItemKind::AudioSpectrum { width, height, .. } => {
                    *width = (*width * uniform).max(8.0);
                    *height = (*height * uniform).max(8.0);
                    cut.transform.x *= uniform;
                    cut.transform.y = cut.transform.y * uniform + fit_y_offset;
                }
                _ => {
                    cut.transform.x *= sx;
                    cut.transform.y *= sy;
                }
            }
            cloned_layer.items.push(cut);
        }
        new_layers.push(cloned_layer);
    }

    let scene_id = Uuid::new_v4();
    let shorts_scene = Scene {
        id: scene_id,
        name: shorts_name.into(),
        width: target_w,
        height: target_h,
        fps: src.fps,
        camera: src.camera,
        master_audio_effects: src.master_audio_effects.clone(),
        target_lufs: src.target_lufs,
        audio_ducking: AudioDuckingConfig {
            ..src.audio_ducking
        },
        color_grade: src.color_grade.clone(),
        color_management: src.color_management,
        motion_blur: src.motion_blur,
        markers: Vec::new(),
        node_graph: crate::node_graph::NodeCompositeGraph::default(),
        review_comments: Vec::new(),
        layers: new_layers,
    };

    Some((
        scene_id,
        EditCommand::CreateScene {
            scene: shorts_scene,
            switch_to: true,
        },
    ))
}

/// v0.7 #8: 自動モーショントラッキング結果 (`TrackPointSample` 列) を
/// 指定アイテムの標準 `"x"` / `"y"` キーフレームとして 1 ステップ Undo で焼き込む。
pub fn build_motion_track_keyframes_command(
    target_item: &TimelineItem,
    samples: &[TrackPointSample],
    offset_x: f32,
    offset_y: f32,
) -> Option<EditCommand> {
    if samples.is_empty() {
        return None;
    }
    let mut commands = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let local_t = (s.time - target_item.start).max(RationalTime::ZERO);
        commands.push(EditCommand::SetKeyframe {
            item_id: target_item.id,
            property: "x".to_string(),
            keyframe: Keyframe {
                time: local_t,
                value: s.x + offset_x,
                interpolation: Interpolation::Linear,
            },
        });
        commands.push(EditCommand::SetKeyframe {
            item_id: target_item.id,
            property: "y".to_string(),
            keyframe: Keyframe {
                time: local_t,
                value: s.y + offset_y,
                interpolation: Interpolation::Linear,
            },
        });
    }
    Some(EditCommand::Batch {
        label: "bake_motion_track_keyframes".into(),
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::{ProjectSettings, RgbaColor, TextAnimator, TimelineMarker};

    #[test]
    fn extract_highlights_ranks_loud_marked_and_dense_segments_highest() {
        let mut proj = Project::new("HighlightTest", ProjectSettings::default());
        {
            let scene = proj.active_scene_mut().unwrap();
            // 0..30s の長い映像
            scene.layers[0].items.push(TimelineItem::new(
                "MainClip",
                RationalTime::ZERO,
                RationalTime::from_seconds(30.0),
                ItemKind::Text {
                    content: "通常解説トーク".into(),
                    font_family: "sans-serif".into(),
                    font_size: 40.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
            // 18..24s に強調字幕＆マーカーを集中配置
            scene.layers[1].items.push(TimelineItem::new(
                "ExciteSub",
                RationalTime::from_seconds(18.0),
                RationalTime::from_seconds(5.0),
                ItemKind::Text {
                    content: "ここが一番ヤバい！すごい神ポイント！結論まとめ！".into(),
                    font_family: "sans-serif".into(),
                    font_size: 48.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
            scene.markers.push(TimelineMarker::new(
                RationalTime::from_seconds(20.0),
                "神展開",
            ));
        }

        // 音声 RMS: 18..24s が大盛り上がり (0.85)、他は静か (0.08)
        let mut rms = Vec::new();
        for sec in 0..30 {
            let v = if (18..=24).contains(&sec) { 0.85 } else { 0.08 };
            rms.push((sec as f64 + 0.5, v));
        }

        let cfg = HighlightConfig {
            target_clip_count: 2,
            clip_duration_sec: 8.0,
            min_gap_sec: 2.0,
            ..Default::default()
        };
        let cands = extract_highlight_candidates(proj.active_scene().unwrap(), &rms, &cfg);
        assert!(!cands.is_empty());
        // 1位は 18..24s を含む区間になること
        let best = &cands[0];
        assert!(
            best.start.as_seconds() <= 20.0 && best.end.as_seconds() >= 20.0,
            "best={best:?}"
        );
        assert!(best.score > 0.65, "best={best:?}");
        assert!(best.suggested_title.contains("神展開"));
    }

    #[test]
    fn build_shorts_clip_scene_trims_offsets_and_relayouts_subtitles_for_9x16() {
        let mut proj = Project::new("ShortsCutTest", ProjectSettings::default());
        let vid_asset = Uuid::new_v4();
        {
            let scene = proj.active_scene_mut().unwrap();
            scene.layers[0].items.push(TimelineItem::new(
                "LongVideo",
                RationalTime::ZERO,
                RationalTime::from_seconds(30.0),
                ItemKind::Video {
                    asset_id: vid_asset,
                    source_offset: RationalTime::ZERO,
                    volume: 1.0,
                },
            ));
            scene.layers[1].items.push(TimelineItem::new(
                "Sub",
                RationalTime::from_seconds(12.0),
                RationalTime::from_seconds(4.0),
                ItemKind::Text {
                    content: "これはショート動画用に自動で読みやすく改行される長いテロップです。"
                        .into(),
                    font_family: "sans-serif".into(),
                    font_size: 44.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
        }

        let cand = HighlightCandidate {
            start: RationalTime::from_seconds(10.0),
            end: RationalTime::from_seconds(20.0),
            score: 0.9,
            reason: "テスト".into(),
            suggested_title: "ハイライト1".into(),
        };
        let (new_sid, cmd) =
            build_shorts_clip_scene_command(&proj, proj.active_scene_id, &cand, "Shorts #1")
                .unwrap();

        let mut bus = CommandBus::new();
        let orig_sid = proj.active_scene_id;
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        assert_eq!(proj.active_scene_id, new_sid);

        let shorts = proj.active_scene().unwrap();
        assert_eq!((shorts.width, shorts.height), (1080, 1920));
        // 動画は 10s 分 source_offset が進み、尺は 10s になっていること
        let v_item = &shorts.layers[0].items[0];
        assert_eq!(v_item.start, RationalTime::ZERO);
        assert!((v_item.duration.as_seconds() - 10.0).abs() < 1e-4);
        if let ItemKind::Video { source_offset, .. } = v_item.kind {
            assert!((source_offset.as_seconds() - 10.0).abs() < 1e-4);
        } else {
            panic!("expected video");
        }
        // 字幕は start=2.0s にシフトされ、13文字基準で改行＆9:16セーフ位置に配置される
        let sub_item = &shorts.layers[1].items[0];
        assert!((sub_item.start.as_seconds() - 2.0).abs() < 1e-4);
        if let ItemKind::Text { content, .. } = &sub_item.kind {
            assert!(
                content.contains('\n'),
                "expected wrapped subtitle: {content}"
            );
        }

        // Undo で元のメインシーンへ完全復元されること
        assert!(bus.undo(&mut proj));
        assert_eq!(proj.active_scene_id, orig_sid);
        assert_eq!(proj.scenes.len(), 1);
    }
}
