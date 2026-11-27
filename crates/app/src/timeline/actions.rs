//! タイムライン編集アクション（ミニマップ・分割・リップル削除・コピペ・ジャンプカット・ビート検出・描画ヘルパー）

use super::*;

impl TimelineState {
    /// タイムライン全体を俯瞰・クリックジャンプできるミニマップバーを描画する
    pub(super) fn draw_minimap(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        total_scene_sec: f64,
        visible_span_sec: f64,
        avail_w: f32,
        header_w: f32,
    ) {
        let map_h = 26.0;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(avail_w, map_h), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, Color32::from_rgb(18, 20, 26));

        let label_rect = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.left() + header_w - 2.0, rect.bottom()),
        );
        painter.rect_filled(label_rect, 2.0, Color32::from_rgb(30, 34, 42));
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("🗺 マップ ({total_scene_sec:.1}s)"),
            egui::FontId::proportional(10.5),
            Color32::from_gray(190),
        );

        let bar_rect = Rect::from_min_max(
            Pos2::new(rect.left() + header_w, rect.top() + 2.0),
            Pos2::new(rect.right(), rect.bottom() - 2.0),
        );
        let span = total_scene_sec.max(visible_span_sec).max(1.0);
        if let Some(scene) = session.project.active_scene() {
            let l_cnt = scene.layers.len().max(1) as f32;
            let row_h = (bar_rect.height() / l_cnt).max(2.0);
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                let y = bar_rect.top() + l_idx as f32 * row_h;
                let step = (layer.items.len() / 1000).max(1);
                for item in layer.items.iter().step_by(step) {
                    let x0 = bar_rect.left()
                        + (item.start.as_seconds() / span) as f32 * bar_rect.width();
                    let x1 = (bar_rect.left()
                        + (item.end().as_seconds() / span) as f32 * bar_rect.width())
                    .max(x0 + 2.0);
                    let r = Rect::from_min_max(
                        Pos2::new(x0, y),
                        Pos2::new(x1.min(bar_rect.right()), (y + row_h - 1.0).max(y + 1.0)),
                    );
                    painter.rect_filled(r, 1.0, clip_color_for_kind(&item.kind));
                }
            }
            // ミニマップ上のマーカー表示
            for marker in &scene.markers {
                let mx =
                    bar_rect.left() + (marker.time.as_seconds() / span) as f32 * bar_rect.width();
                if mx >= bar_rect.left() && mx <= bar_rect.right() {
                    painter.line_segment(
                        [
                            Pos2::new(mx, bar_rect.top()),
                            Pos2::new(mx, bar_rect.top() + 6.0),
                        ],
                        Stroke::new(2.0, rgba_to_color32(marker.color)),
                    );
                }
            }
        }

        // 現在の表示ウィンドウ枠
        let vx0 = bar_rect.left() + (self.scroll_offset_sec / span) as f32 * bar_rect.width();
        let vx1 = (bar_rect.left()
            + ((self.scroll_offset_sec + visible_span_sec) / span) as f32 * bar_rect.width())
        .min(bar_rect.right());
        let vp_rect = Rect::from_min_max(
            Pos2::new(vx0.clamp(bar_rect.left(), bar_rect.right()), bar_rect.top()),
            Pos2::new(vx1.max(vx0 + 6.0), bar_rect.bottom()),
        );
        painter.rect_filled(
            vp_rect,
            1.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 26),
        );
        painter.rect_stroke(
            vp_rect,
            1.0,
            Stroke::new(1.0, Color32::from_gray(200)),
            StrokeKind::Inside,
        );

        // ミニマップ上の再生ヘッド位置
        let ph_x =
            bar_rect.left() + (session.playhead.as_seconds() / span) as f32 * bar_rect.width();
        if ph_x >= bar_rect.left() && ph_x <= bar_rect.right() {
            painter.line_segment(
                [
                    Pos2::new(ph_x, bar_rect.top()),
                    Pos2::new(ph_x, bar_rect.bottom()),
                ],
                Stroke::new(1.5, Color32::from_rgb(235, 72, 72)),
            );
        }

        // ミニマップクリック・ドラッグで即ジャンプ＆スクロール
        if (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
            && pos.x >= bar_rect.left()
        {
            let frac = ((pos.x - bar_rect.left()) / bar_rect.width().max(1.0)).clamp(0.0, 1.0);
            let target_sec = f64::from(frac) * span;
            session.playhead = RationalTime::from_seconds(target_sec);
            self.scroll_offset_sec = (target_sec - visible_span_sec * 0.35).max(0.0);
        }
    }

    pub(super) fn precompose_selected(&self, session: &mut EditorSession) {
        if session.selected_items.is_empty() {
            return;
        }
        let new_scene_id = Uuid::new_v4();
        let precomp_item_id = Uuid::new_v4();
        let next_num = session.project.scenes.len();
        let cmd = EditCommand::PrecomposeSelection {
            item_ids: session.selected_items.clone(),
            new_scene_id,
            precomp_name: format!("Precomp {next_num}"),
            precomp_item_id,
            target_layer_index: 0,
        };
        if session
            .bus
            .execute(&mut session.project, cmd, CommandSource::Gui, None)
            .is_ok()
        {
            session.selected_items = vec![precomp_item_id];
        }
    }

    pub(super) fn split_selected_at_playhead(&self, session: &mut EditorSession) {
        let split_at = session.playhead;
        let cmds: Vec<EditCommand> = session
            .selected_items
            .iter()
            .copied()
            .map(|id| EditCommand::SplitObject {
                item_id: id,
                split_at,
                new_right_id: Uuid::new_v4(),
            })
            .collect();
        if cmds.is_empty() {
            return;
        }
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::Batch {
                label: "split_at_playhead".into(),
                commands: cmds,
            },
            CommandSource::Gui,
            None,
        );
    }

    pub(super) fn delete_selected(&self, session: &mut EditorSession) {
        let cmds: Vec<EditCommand> = session
            .selected_items
            .iter()
            .copied()
            .map(|item_id| EditCommand::DeleteObject { item_id })
            .collect();
        if cmds.is_empty() {
            return;
        }
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::Batch {
                    label: "delete_selected".into(),
                    commands: cmds,
                },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            session.selected_items.clear();
        }
    }

    pub(super) fn ripple_delete_selected(&self, session: &mut EditorSession) {
        let Some(scene) = session.project.active_scene().cloned() else {
            return;
        };
        if let Some(cmd) = build_ripple_delete_commands(&scene, &session.selected_items)
            && session
                .bus
                .execute(&mut session.project, cmd, CommandSource::Gui, None)
                .is_ok()
        {
            session.selected_items.clear();
        }
    }

    pub(super) fn select_all_in_scene(&self, session: &mut EditorSession) {
        if let Some(scene) = session.project.active_scene() {
            session.selected_items = scene
                .layers
                .iter()
                .flat_map(|l| l.items.iter().map(|i| i.id))
                .collect();
        }
    }

    pub(super) fn copy_selected(&mut self, session: &EditorSession) {
        if session.selected_items.is_empty() {
            return;
        }
        let sel: BTreeSet<ItemId> = session.selected_items.iter().copied().collect();
        let mut copied = Vec::new();
        if let Some(scene) = session.project.active_scene() {
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                for item in &layer.items {
                    if sel.contains(&item.id) {
                        copied.push((l_idx, item.clone()));
                    }
                }
            }
        }
        if !copied.is_empty() {
            self.clipboard_items = copied;
        }
    }

    pub(super) fn paste_clipboard_at_playhead(&self, session: &mut EditorSession) {
        if self.clipboard_items.is_empty() {
            return;
        }
        let Some(layer_count) = session.project.active_scene().map(|s| s.layers.len()) else {
            return;
        };
        if layer_count == 0 {
            return;
        }
        let min_start = self
            .clipboard_items
            .iter()
            .map(|(_, it)| it.start)
            .min()
            .unwrap_or(RationalTime::ZERO);
        let target_base = session.playhead.max(RationalTime::ZERO);

        let mut cmds = Vec::with_capacity(self.clipboard_items.len());
        let mut new_ids = Vec::with_capacity(self.clipboard_items.len());
        for (l_idx, src) in &self.clipboard_items {
            let rel = (src.start - min_start).max(RationalTime::ZERO);
            let item = clone_item_with_fresh_ids(src, target_base + rel);
            new_ids.push(item.id);
            cmds.push(EditCommand::AddObject {
                layer_index: (*l_idx).min(layer_count - 1),
                item,
            });
        }
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::Batch {
                    label: "paste_clips".into(),
                    commands: cmds,
                },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            session.selected_items = new_ids;
        }
    }

    pub(super) fn duplicate_selected(&self, session: &mut EditorSession) {
        if session.selected_items.is_empty() {
            return;
        }
        let sel: BTreeSet<ItemId> = session.selected_items.iter().copied().collect();
        let mut selected_pairs: Vec<(usize, TimelineItem)> = Vec::new();
        let layer_count = if let Some(scene) = session.project.active_scene() {
            for (l_idx, layer) in scene.layers.iter().enumerate() {
                for item in &layer.items {
                    if sel.contains(&item.id) {
                        selected_pairs.push((l_idx, item.clone()));
                    }
                }
            }
            scene.layers.len()
        } else {
            return;
        };
        if selected_pairs.is_empty() || layer_count == 0 {
            return;
        }

        let min_start = selected_pairs
            .iter()
            .map(|(_, it)| it.start)
            .min()
            .unwrap_or(RationalTime::ZERO);
        let max_end = selected_pairs
            .iter()
            .map(|(_, it)| it.end())
            .max()
            .unwrap_or(min_start + RationalTime::from_seconds(1.0));
        let span = (max_end - min_start).max(RationalTime::from_seconds(0.1));

        let mut cmds = Vec::with_capacity(selected_pairs.len());
        let mut new_ids = Vec::with_capacity(selected_pairs.len());
        for (l_idx, src) in &selected_pairs {
            let item = clone_item_with_fresh_ids(src, src.start + span);
            new_ids.push(item.id);
            cmds.push(EditCommand::AddObject {
                layer_index: (*l_idx).min(layer_count - 1),
                item,
            });
        }
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::Batch {
                    label: "duplicate_clips".into(),
                    commands: cmds,
                },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            session.selected_items = new_ids;
        }
    }

    pub(super) fn add_marker_at_playhead(&mut self, session: &mut EditorSession) {
        let next_idx = session
            .project
            .active_scene()
            .map(|s| s.markers.len() + 1)
            .unwrap_or(1);
        let label = if self.marker_label_input.trim().is_empty() {
            format!("チャプター {next_idx}")
        } else {
            self.marker_label_input.trim().to_string()
        };
        let color = MARKER_PALETTE
            .get(self.marker_color_idx % MARKER_PALETTE.len())
            .map(|(_, c)| *c)
            .unwrap_or(RgbaColor::WHITE);
        let marker = TimelineMarker {
            id: Uuid::new_v4(),
            time: session.playhead,
            label: label.clone(),
            color,
        };
        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::AddMarker { marker },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            self.marker_label_input.clear();
            self.marker_status = format!(
                "マーカー「{label}」を {:.2}s へ追加しました",
                session.playhead.as_seconds()
            );
        }
    }

    pub(super) fn execute_jump_cut(
        &mut self,
        session: &mut EditorSession,
        waveforms: &BTreeMap<Uuid, WaveformPeaks>,
    ) {
        let Some(scene) = session.project.active_scene().cloned() else {
            self.jump_cut_status = "アクティブシーンがありません".into();
            return;
        };
        let total_sec = scene.total_duration().as_seconds();
        if total_sec <= 0.05 {
            self.jump_cut_status = "カット対象のクリップがありません".into();
            return;
        }
        let audio_layer = self
            .jump_cut_audio_layer
            .min(scene.layers.len().saturating_sub(1));

        // 1. 実メディアの AudioGraph から 50ms 単位の RMS 列を生成。
        //    実ファイルが無い場合（デモ波形など）は waveforms キャッシュへフォールバック。
        let sr = session.project.settings.audio_sample_rate.max(8_000);
        let audio_graph = AudioGraph::from_project_scene(
            &session.project,
            session.project.active_scene_id,
            session.folder.as_ref(),
            sr,
        );
        let bucket_sec = 0.05_f64;
        let bucket_count = (total_sec / bucket_sec).ceil().max(1.0) as usize;
        let mut rms_series = vec![0.0_f32; bucket_count];
        let mut has_audio_signal = false;

        if !audio_graph.clips.is_empty() {
            let pcm = audio_graph.render_range(RationalTime::ZERO, scene.total_duration());
            let frames_per_bucket = ((sr as f64) * bucket_sec).round().max(1.0) as usize;
            let stereo_frames = pcm.as_chunks::<2>().0;
            for (b_idx, rms_slot) in rms_series.iter_mut().enumerate() {
                let start_f = b_idx * frames_per_bucket;
                if start_f >= stereo_frames.len() {
                    break;
                }
                let end_f = ((b_idx + 1) * frames_per_bucket).min(stereo_frames.len());
                let slice = &stereo_frames[start_f..end_f];
                if slice.is_empty() {
                    continue;
                }
                let sum_sq: f64 = slice
                    .iter()
                    .map(|[l, r]| {
                        let mono = 0.5 * (f64::from(*l) + f64::from(*r));
                        mono * mono
                    })
                    .sum();
                *rms_slot = (sum_sq / slice.len() as f64).sqrt() as f32;
                if *rms_slot > 0.0 {
                    has_audio_signal = true;
                }
            }
        }

        if !has_audio_signal && let Some(layer) = scene.layers.get(audio_layer) {
            for item in &layer.items {
                if let ItemKind::Audio {
                    asset_id,
                    source_offset,
                    volume,
                } = &item.kind
                    && let Some(wf) = waveforms.get(asset_id)
                    && wf.sample_rate > 0
                    && wf.samples_per_bucket > 0
                {
                    let wf_bucket_sec = (wf.samples_per_bucket as f64) / f64::from(wf.sample_rate);
                    let start_s = item.start.as_seconds();
                    let end_s = item.end().as_seconds();
                    let src_off = source_offset.as_seconds();
                    for (b_idx, rms_slot) in rms_series.iter_mut().enumerate() {
                        let t = (b_idx as f64 + 0.5) * bucket_sec;
                        if t < start_s || t >= end_s {
                            continue;
                        }
                        let wf_idx = ((src_off + (t - start_s)) / wf_bucket_sec).floor() as usize;
                        if let Some(b) = wf.buckets.get(wf_idx) {
                            let v = b.rms * volume.max(0.0);
                            if v > *rms_slot {
                                *rms_slot = v;
                                has_audio_signal = true;
                            }
                        }
                    }
                }
            }
        }

        if !has_audio_signal {
            self.jump_cut_status = format!(
                "L{audio_layer} に解析可能な音声波形がありません（先に音声ファイルを配置してください）"
            );
            return;
        }

        let linked: Vec<usize> = self
            .jump_cut_linked_layers
            .iter()
            .copied()
            .filter(|&l| l < scene.layers.len())
            .collect();
        let cfg = JumpCutConfig {
            rms_threshold: self.jump_cut_rms_threshold,
            min_silence_sec: self.jump_cut_min_silence_sec,
            pad_before_sec: self.jump_cut_pad_before_sec,
            pad_after_sec: self.jump_cut_pad_after_sec,
            ripple_linked_layers: linked.clone(),
        };
        let intervals = detect_speech_intervals_from_rms(&rms_series, bucket_sec, total_sec, &cfg);
        if intervals.is_empty() {
            self.jump_cut_status =
                "しきい値以上の有音区間が検出されませんでした（RMS閾値を下げてください）".into();
            return;
        }
        match build_jump_cut_commands(&scene, audio_layer, &intervals, &linked) {
            Some((kept_count, removed_sec, cmd)) => {
                match session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None)
                {
                    Ok(_) => {
                        session.selected_items.clear();
                        self.jump_cut_status = format!(
                            "無音カット完了: 発話 {kept_count} 区間を保持 / 約 {removed_sec:.2} 秒を自動カット（Ctrl+Z で復元可）"
                        );
                    }
                    Err(e) => {
                        self.jump_cut_status = format!("無音カット実行エラー: {e}");
                    }
                }
            }
            None => {
                self.jump_cut_status = "カット可能な無音区間はありませんでした".into();
            }
        }
    }

    pub(super) fn execute_beat_detection(
        &mut self,
        session: &EditorSession,
        waveforms: &BTreeMap<Uuid, WaveformPeaks>,
    ) {
        let Some(scene) = session.project.active_scene() else {
            self.beat_status = "アクティブシーンがありません".into();
            return;
        };
        let total_dur = scene.total_duration();
        if total_dur.as_seconds() < 0.5 {
            self.beat_status = "解析対象の音声クリップがありません".into();
            return;
        }
        let cfg = BeatDetectionConfig {
            min_bpm: self.beat_min_bpm,
            max_bpm: self.beat_max_bpm,
            sensitivity: self.beat_sensitivity,
            beats_per_bar: self.beat_per_bar.max(1),
        };
        let sr = session.project.settings.audio_sample_rate.max(8_000);
        let graph = AudioGraph::from_project_scene(
            &session.project,
            session.project.active_scene_id,
            session.folder.as_ref(),
            sr,
        );
        let mut res = if !graph.clips.is_empty() {
            let capped = total_dur.min(RationalTime::from_seconds(300.0));
            let pcm = graph.render_range(RationalTime::ZERO, capped);
            detect_audio_beats(&pcm, sr, &cfg)
        } else {
            ve_audio::BeatAnalysisResult {
                bpm: 0.0,
                confidence: 0.0,
                beats: Vec::new(),
                onset_times_sec: Vec::new(),
            }
        };

        // 実ファイル未ロード（デモ波形等）の場合は指定レイヤーの WaveformPeaks からフォールバック検出
        if res.beats.is_empty()
            && let Some(layer) = scene.layers.get(self.beat_audio_layer)
        {
            for item in &layer.items {
                if let ItemKind::Audio { asset_id, .. } = &item.kind
                    && let Some(wf) = waveforms.get(asset_id)
                {
                    let mut wf_res = detect_beats_from_waveform(wf, &cfg);
                    let offset = item.start.as_seconds();
                    for b in &mut wf_res.beats {
                        b.time_sec += offset;
                    }
                    if !wf_res.beats.is_empty() {
                        res = wf_res;
                        break;
                    }
                }
            }
        }

        if res.beats.is_empty() {
            self.beat_grid.clear();
            self.detected_bpm = 0.0;
            self.beat_status =
                "ビートを検出できませんでした（BGM/音声ファイルを配置してください）".into();
        } else {
            self.detected_bpm = res.bpm;
            self.beat_grid = res
                .beats
                .iter()
                .map(|b| (b.time_sec, b.is_downbeat))
                .collect();
            self.beat_status = format!(
                "検出完了: {:.1} BPM（{} 拍 / 信頼度 {:.0}%）",
                res.bpm,
                res.beats.len(),
                res.confidence * 100.0
            );
        }
    }
}

pub(super) fn clip_color_for_kind(kind: &ItemKind) -> Color32 {
    match kind {
        ItemKind::Text { .. } => Color32::from_rgb(72, 130, 196),
        ItemKind::FilterObject { .. } => Color32::from_rgb(165, 102, 185),
        ItemKind::Transition { .. } => Color32::from_rgb(214, 118, 52),
        ItemKind::GroupControl { .. } => Color32::from_rgb(196, 140, 56),
        ItemKind::CharacterSprite { .. } => Color32::from_rgb(208, 102, 140),
        ItemKind::AudioSpectrum { .. } => Color32::from_rgb(46, 178, 212),
        ItemKind::ParticleEmitter { .. } => Color32::from_rgb(218, 152, 42),
        ItemKind::VectorShape { .. } => Color32::from_rgb(172, 86, 214),
        ItemKind::SvgMorph { .. } => Color32::from_rgb(204, 92, 232),
        ItemKind::VtuberAvatar { .. } => Color32::from_rgb(242, 108, 178),
        ItemKind::FluidSim { .. } => Color32::from_rgb(248, 132, 46),
        ItemKind::ClothSim { .. } => Color32::from_rgb(198, 78, 118),
        ItemKind::Mesh3D { .. } => Color32::from_rgb(68, 146, 238),
        ItemKind::GenerativeBg { .. } => Color32::from_rgb(48, 186, 152),
        ItemKind::RiggedPuppet { .. } => Color32::from_rgb(232, 92, 148),
        ItemKind::DataChart { .. } => Color32::from_rgb(56, 198, 232),
        ItemKind::Precomp { .. } => Color32::from_rgb(42, 152, 148),
        ItemKind::Audio { .. } => Color32::from_rgb(64, 152, 108),
        _ => Color32::from_rgb(88, 104, 138),
    }
}

pub(super) fn rgba_to_color32(c: RgbaColor) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

pub(super) fn snap_value(value: f64, targets: &[f64], threshold: f64) -> f64 {
    let mut best = value;
    let mut best_dist = threshold;
    for &t in targets {
        let d = (value - t).abs();
        if d < best_dist {
            best_dist = d;
            best = t;
        }
    }
    best
}

pub(super) fn draw_waveform(painter: &egui::Painter, rect: Rect, wf: &WaveformPeaks) {
    if wf.buckets.is_empty() {
        return;
    }
    let mid_y = rect.center().y;
    let half_h = rect.height() * 0.42;
    let steps = (rect.width() as usize).clamp(1, wf.buckets.len());
    for i in 0..steps {
        let b_idx = i * wf.buckets.len() / steps;
        let b = wf.buckets[b_idx];
        let x = rect.left() + (i as f32 / steps as f32) * rect.width();
        let y0 = mid_y - b.max * half_h;
        let y1 = mid_y - b.min * half_h;
        painter.line_segment(
            [Pos2::new(x, y0), Pos2::new(x, y1)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(220, 255, 230, 180)),
        );
    }
}

pub(super) fn clone_item_with_fresh_ids(
    src: &TimelineItem,
    new_start: RationalTime,
) -> TimelineItem {
    let mut item = src.clone();
    item.id = Uuid::new_v4();
    item.start = new_start.max(RationalTime::ZERO);
    for eff in &mut item.effect_chain {
        eff.id = Uuid::new_v4();
    }
    for eff in &mut item.audio_effects {
        eff.id = Uuid::new_v4();
    }
    for mask in &mut item.masks {
        mask.id = Uuid::new_v4();
    }
    item
}
