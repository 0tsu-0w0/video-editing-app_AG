use egui::Ui;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;
use ve_ai::{
    ApiProvider, ClaudeRefiner, CloudUploadGuard, CredentialStore, GeminiTranscriber,
    LocalTtsClient, LocalWhisperTranscriber, OsCredentialStore, SubtitleSegment, TtsEngine,
    import_tts_file_pair, segments_to_subtitle_items,
};
use ve_audio::{WaveformBucket, WaveformPeaks};
use ve_core::{
    AssetRef, CharacterSpriteConfig, CommandSource, EditCommand, ItemId, ItemKind, LipSyncMode,
    MediaKind, MouthShape, PhonemeSegment, PsdCharacterConfig, RationalTime, RgbaColor,
    ScriptBuildConfig, TextAnimator, TimelineItem, Transform2D, build_script_timeline_commands,
    export_ass_subtitles, export_otio_json, export_srt_subtitles, parse_ass_subtitles,
    parse_aviutl_exo, parse_script_auto, parse_srt_subtitles, subtitle_entries_to_commands,
    wrap_subtitle_japanese,
};
use ve_mcp::EditorSession;
use ve_media::decode_audio_file;

struct SubtitleDiffRow {
    item_id: ItemId,
    start_sec: f64,
    before: String,
    after: String,
    accepted: bool,
}

pub struct AudioAi3DPanel {
    cred_store: OsCredentialStore,
    guard: CloudUploadGuard,
    gemini_key_input: String,
    claude_key_input: String,
    whisper_model_path: String,
    transcribe_wav_path: String,
    cloud_confirm_checked: bool,
    refine_wrap_chars: usize,
    refine_diffs: Vec<SubtitleDiffRow>,
    tts_pair_wav_path: String,
    tts_engine: TtsEngine,
    tts_speaker_id: u32,
    tts_text: String,
    // v0.6 #5: 台本 (CSV / Markdown) -> タイムライン一括生成
    script_text: String,
    script_chars_per_sec: f64,
    script_pause_gap_sec: f64,
    script_show_speaker: bool,
    script_use_tts: bool,
    // v0.6 #6: SRT / ASS / AviUtl .exo / OTIO 互換
    interop_export_stem: String,
    status_msg: String,
}

impl Default for AudioAi3DPanel {
    fn default() -> Self {
        Self {
            cred_store: OsCredentialStore::new(),
            guard: CloudUploadGuard::default(),
            gemini_key_input: String::new(),
            claude_key_input: String::new(),
            whisper_model_path: String::new(),
            transcribe_wav_path: String::new(),
            cloud_confirm_checked: false,
            refine_wrap_chars: 18,
            refine_diffs: Vec::new(),
            tts_pair_wav_path: String::new(),
            tts_engine: TtsEngine::Voicevox,
            tts_speaker_id: 3,
            tts_text: "こんにちは！次世代ハイブリッド動画編集ソフトです。".to_string(),
            script_text: "**霊夢** [smile]: 今日は v0.6 の新機能を解説するよ！\n**魔理沙** [surprise]: 台本から自動で字幕タイムラインが組めるのかぜ！？\n**霊夢**: 話者ごとに自動で色分けされて、Undoも1発だよ。".to_string(),
            script_chars_per_sec: 6.5,
            script_pause_gap_sec: 0.25,
            script_show_speaker: false,
            script_use_tts: false,
            interop_export_stem: "project_export".to_string(),
            status_msg: "待機中".to_string(),
        }
    }
}

impl AudioAi3DPanel {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        waveforms: &mut BTreeMap<Uuid, WaveformPeaks>,
    ) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            // ===== 1. 立ち絵自動口パク =====
            ui.heading("1. 立ち絵自動口パク（PNG差分）");
            if ui
                .button("＋ デモ立ち絵＆音声（波形・母音付）を配置")
                .clicked()
            {
                self.spawn_lipsync_demo(session, waveforms);
            }

            let selected_sprite = session.selected_items.first().copied().and_then(|sel_id| {
                session.project.active_scene().and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == sel_id)
                        .and_then(|i| match &i.kind {
                            ItemKind::CharacterSprite { config, .. } => {
                                Some((i.id, config.clone()))
                            }
                            _ => None,
                        })
                })
            });

            if let Some((item_id, mut cfg)) = selected_sprite {
                let prev_cfg = cfg.clone();
                let layer_names: Vec<String> = session
                    .project
                    .active_scene()
                    .map(|s| {
                        s.layers
                            .iter()
                            .enumerate()
                            .map(|(idx, l)| format!("L{idx}: {}", l.name))
                            .collect()
                    })
                    .unwrap_or_default();

                // 設定レスUX: 連動する音声レイヤーを選ぶだけ
                let cur_label = cfg
                    .linked_audio_layer
                    .and_then(|idx| layer_names.get(idx).cloned())
                    .unwrap_or_else(|| "未連動".to_string());
                egui::ComboBox::from_label("連動する音声レイヤー")
                    .selected_text(cur_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cfg.linked_audio_layer, None, "未連動");
                        for (idx, name) in layer_names.iter().enumerate() {
                            ui.selectable_value(&mut cfg.linked_audio_layer, Some(idx), name);
                        }
                    });

                ui.horizontal(|ui| {
                    ui.label("連動方式:");
                    let is_rms = matches!(cfg.mode, LipSyncMode::VolumeRms { .. });
                    if ui.selectable_label(is_rms, "音量連動 (RMS)").clicked() && !is_rms {
                        cfg.mode = LipSyncMode::default();
                    }
                    if ui
                        .selectable_label(!is_rms, "母音連動 (音素/TTS)")
                        .clicked()
                        && is_rms
                    {
                        cfg.mode = LipSyncMode::VowelPhoneme;
                    }
                });

                if let LipSyncMode::VolumeRms {
                    ref mut closed_threshold,
                    ref mut open_threshold,
                } = cfg.mode
                {
                    ui.add(egui::Slider::new(closed_threshold, 0.0..=0.5).text("閉じ口しきい値"));
                    ui.add(egui::Slider::new(open_threshold, 0.05..=0.8).text("開き口しきい値"));
                }

                egui::CollapsingHeader::new("PNG差分パス設定（任意・未指定時はアバター描画）")
                    .show(ui, |ui| {
                        for shape in [
                            MouthShape::Closed,
                            MouthShape::HalfOpen,
                            MouthShape::Open,
                            MouthShape::A,
                            MouthShape::I,
                            MouthShape::U,
                            MouthShape::E,
                            MouthShape::O,
                        ] {
                            let mut path_str = cfg
                                .png_paths
                                .get(&shape)
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label(format!("{shape:?}:"));
                                if ui.text_edit_singleline(&mut path_str).changed() {
                                    if path_str.trim().is_empty() {
                                        cfg.png_paths.remove(&shape);
                                    } else {
                                        cfg.png_paths.insert(shape, PathBuf::from(path_str.trim()));
                                    }
                                }
                            });
                        }
                    });

                egui::CollapsingHeader::new("PSD ネイティブ立ち絵設定（Phase 4: 目パチ・口パク）")
                    .show(ui, |ui| {
                        let mut use_psd = cfg.psd_config.is_some();
                        if ui
                            .checkbox(&mut use_psd, "PSD レイヤー直接読み込みを有効化")
                            .changed()
                        {
                            cfg.psd_config = if use_psd {
                                Some(PsdCharacterConfig::default())
                            } else {
                                None
                            };
                        }
                        if let Some(psd) = cfg.psd_config.as_mut() {
                            let mut psd_str = psd.psd_path.to_string_lossy().into_owned();
                            ui.horizontal(|ui| {
                                ui.label("PSDパス:");
                                if ui.text_edit_singleline(&mut psd_str).changed() {
                                    psd.psd_path = PathBuf::from(psd_str.trim());
                                }
                                if ui.button("📂").clicked()
                                    && let Some(p) =
                                        crate::native_dialog::pick_file("PSD立ち絵を選択", &["psd"])
                                {
                                    psd.psd_path = p;
                                }
                            });
                            let mut eye_open = psd.eye_open_layer.clone().unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label("開き目レイヤー:");
                                if ui.text_edit_singleline(&mut eye_open).changed() {
                                    psd.eye_open_layer = if eye_open.trim().is_empty() {
                                        None
                                    } else {
                                        Some(eye_open.trim().to_string())
                                    };
                                }
                            });
                            let mut eye_closed = psd.eye_closed_layer.clone().unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label("閉じ目レイヤー:");
                                if ui.text_edit_singleline(&mut eye_closed).changed() {
                                    psd.eye_closed_layer = if eye_closed.trim().is_empty() {
                                        None
                                    } else {
                                        Some(eye_closed.trim().to_string())
                                    };
                                }
                            });
                            ui.add(
                                egui::Slider::new(&mut psd.blink_interval_sec, 0.0..=10.0)
                                    .text("目パチ周期(秒)"),
                            );
                            for shape in [
                                MouthShape::Closed,
                                MouthShape::HalfOpen,
                                MouthShape::Open,
                                MouthShape::A,
                                MouthShape::I,
                                MouthShape::U,
                                MouthShape::E,
                                MouthShape::O,
                            ] {
                                let mut lname =
                                    psd.mouth_layers.get(&shape).cloned().unwrap_or_default();
                                ui.horizontal(|ui| {
                                    ui.label(format!("口 {shape:?}:"));
                                    if ui.text_edit_singleline(&mut lname).changed() {
                                        if lname.trim().is_empty() {
                                            psd.mouth_layers.remove(&shape);
                                        } else {
                                            psd.mouth_layers
                                                .insert(shape, lname.trim().to_string());
                                        }
                                    }
                                });
                            }
                        }
                    });

                if cfg != prev_cfg {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetCharacterSprite {
                            item_id,
                            config: cfg,
                        },
                        CommandSource::Gui,
                        Some("sprite_config"),
                    );
                }
            } else {
                ui.weak("立ち絵オブジェクトを選択すると口パク連動設定が表示されます。");
            }

            ui.separator();

            // ===== 2. 3Dカメラ制御 & 3D空間配置 =====
            ui.heading("2. 3Dカメラ制御＆被写界深度");
            if let Some(scene) = session.project.active_scene() {
                let mut cam = scene.camera;
                let prev_cam = cam;
                ui.checkbox(&mut cam.enabled, "3Dカメラ有効");
                if cam.enabled {
                    ui.horizontal(|ui| {
                        ui.label("位置:");
                        ui.add(egui::DragValue::new(&mut cam.position[0]).prefix("X:"));
                        ui.add(egui::DragValue::new(&mut cam.position[1]).prefix("Y:"));
                        ui.add(egui::DragValue::new(&mut cam.position[2]).prefix("Z:"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("注視点:");
                        ui.add(egui::DragValue::new(&mut cam.target[0]).prefix("X:"));
                        ui.add(egui::DragValue::new(&mut cam.target[1]).prefix("Y:"));
                        ui.add(egui::DragValue::new(&mut cam.target[2]).prefix("Z:"));
                    });
                    ui.add(egui::Slider::new(&mut cam.fov_deg, 15.0..=110.0).text("視野角 (FOV)"));
                    ui.checkbox(&mut cam.dof_enabled, "被写界深度 (ボケ)");
                    if cam.dof_enabled {
                        ui.add(
                            egui::Slider::new(&mut cam.focus_distance, 100.0..=3000.0)
                                .text("合焦距離"),
                        );
                        ui.add(egui::Slider::new(&mut cam.aperture, 0.0..=0.2).text("絞り強度"));
                    }
                }
                if cam != prev_cam {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetCamera3D { camera: cam },
                        CommandSource::Gui,
                        Some("camera_3d_edit"),
                    );
                }
            }

            // 選択オブジェクトの 3D 配置
            if let Some(&sel_id) = session.selected_items.first()
                && let Some(mut tf3d) = session.project.active_scene().and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == sel_id)
                        .map(|i| i.transform_3d)
                })
            {
                let prev = tf3d;
                ui.checkbox(&mut tf3d.enable_3d, "選択オブジェクトを 3D レイヤー化");
                if tf3d.enable_3d {
                    ui.add(egui::Slider::new(&mut tf3d.z, -600.0..=1200.0).text("Z 座標"));
                    ui.add(
                        egui::Slider::new(&mut tf3d.rotation_x_deg, -85.0..=85.0).text("X 回転"),
                    );
                    ui.add(
                        egui::Slider::new(&mut tf3d.rotation_y_deg, -85.0..=85.0).text("Y 回転"),
                    );
                }
                if tf3d != prev {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetTransform3D {
                            item_id: sel_id,
                            transform_3d: tf3d,
                        },
                        CommandSource::Gui,
                        Some("transform_3d_edit"),
                    );
                }
            }

            ui.separator();

            // ===== 3. 合成音声（TTS）直接連携 =====
            ui.heading("3. 合成音声 (TTS) 連携");
            ui.label("A. ファイルペア自動取込 (voice.wav + 同名 .txt / .lab):");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.tts_pair_wav_path);
                if ui.button("📂").clicked()
                    && let Some(p) = crate::native_dialog::pick_file(
                        "TTS音声ファイルを選択",
                        &["wav", "mp3", "flac"],
                    )
                {
                    self.tts_pair_wav_path = p.display().to_string();
                }
                if ui.button("一括取込").clicked() && !self.tts_pair_wav_path.trim().is_empty()
                {
                    self.import_pair(session, waveforms);
                }
            });

            ui.add_space(4.0);
            ui.label("B. ローカル TTS API 連携 (VOICEVOX / COEIROINK / A.I.VOICE):");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("tts_engine_combo")
                    .selected_text(format!("{:?}", self.tts_engine))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.tts_engine, TtsEngine::Voicevox, "VOICEVOX");
                        ui.selectable_value(
                            &mut self.tts_engine,
                            TtsEngine::Coeiroink,
                            "COEIROINK",
                        );
                        ui.selectable_value(&mut self.tts_engine, TtsEngine::AiVoice, "A.I.VOICE");
                    });
                ui.add(egui::DragValue::new(&mut self.tts_speaker_id).prefix("Speaker:"));
            });
            ui.text_edit_singleline(&mut self.tts_text);
            if ui
                .button("🎙 音声・字幕・母音タイミングを1アクション生成")
                .clicked()
            {
                let client = LocalTtsClient::new(self.tts_engine, self.tts_speaker_id);
                match client.synthesize(&self.tts_text) {
                    Ok(bundle) => {
                        let start = session.playhead;
                        let mut sub = TimelineItem::new(
                            "TTS字幕",
                            start,
                            bundle.duration,
                            ItemKind::Text {
                                content: bundle.subtitle_text,
                                font_family: "sans-serif".to_string(),
                                font_size: 46.0,
                                color: RgbaColor::WHITE,
                                animator: TextAnimator::default(),
                            },
                        );
                        sub.phoneme_segments = bundle.phonemes;
                        let _ = session.bus.execute(
                            &mut session.project,
                            EditCommand::AddObject {
                                layer_index: 1,
                                item: sub,
                            },
                            CommandSource::Gui,
                            None,
                        );
                        self.status_msg =
                            "ローカルTTSから字幕＋母音タイミングを生成しました".into();
                    }
                    Err(e) => {
                        self.status_msg = format!("ローカルTTS接続失敗: {e}");
                    }
                }
            }

            ui.separator();

            // ===== 4. 自動文字起こし & OS資格情報ストア =====
            ui.heading("4. 自動文字起こし (Whisper / Gemini / Claude)");
            ui.weak(
                "※ APIキーは OS資格情報ストアへ安全保管し、project.json には一切保存しません。",
            );
            ui.horizontal(|ui| {
                ui.label("Gemini Key:");
                ui.add(egui::TextEdit::singleline(&mut self.gemini_key_input).password(true));
                if ui.button("OS保存").clicked() && !self.gemini_key_input.is_empty() {
                    match self
                        .cred_store
                        .set_api_key(ApiProvider::Gemini, &self.gemini_key_input)
                    {
                        Ok(()) => {
                            self.gemini_key_input.clear();
                            self.status_msg =
                                "Gemini APIキーをOS資格情報ストアへ保存しました".into();
                        }
                        Err(e) => self.status_msg = format!("保存エラー: {e}"),
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Claude Key:");
                ui.add(egui::TextEdit::singleline(&mut self.claude_key_input).password(true));
                if ui.button("OS保存").clicked() && !self.claude_key_input.is_empty() {
                    match self
                        .cred_store
                        .set_api_key(ApiProvider::Claude, &self.claude_key_input)
                    {
                        Ok(()) => {
                            self.claude_key_input.clear();
                            self.status_msg =
                                "Claude APIキーをOS資格情報ストアへ保存しました".into();
                        }
                        Err(e) => self.status_msg = format!("保存エラー: {e}"),
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("WAVパス:");
                ui.text_edit_singleline(&mut self.transcribe_wav_path);
                if ui.button("📂").clicked()
                    && let Some(p) = crate::native_dialog::pick_file(
                        "文字起こし音声を選択",
                        &["wav", "mp3", "m4a", "flac"],
                    )
                {
                    self.transcribe_wav_path = p.display().to_string();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Whisperモデル:");
                ui.text_edit_singleline(&mut self.whisper_model_path);
                if ui.button("📂").clicked()
                    && let Some(p) = crate::native_dialog::pick_file(
                        "Whisperモデル (.bin) を選択",
                        &["bin", "gguf"],
                    )
                {
                    self.whisper_model_path = p.display().to_string();
                }
            });

            let est = self
                .guard
                .estimate_audio_transcription(ApiProvider::Gemini, 60.0);
            ui.checkbox(
                &mut self.cloud_confirm_checked,
                format!("外部送信・概算コスト確認済み ({})", est.summary_message),
            );

            ui.horizontal_wrapped(|ui| {
                if ui.button("ローカル Whisper 実行").clicked() {
                    let wt = LocalWhisperTranscriber {
                        model_path: if self.whisper_model_path.trim().is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(self.whisper_model_path.trim()))
                        },
                        ..Default::default()
                    };
                    match wt.transcribe_wav(&PathBuf::from(self.transcribe_wav_path.trim())) {
                        Ok(segs) => {
                            let items = segments_to_subtitle_items(&segs, 44.0, 560.0);
                            let count = items.len();
                            let _ = session.bus.execute(
                                &mut session.project,
                                EditCommand::ImportSubtitles {
                                    layer_index: 1,
                                    items,
                                },
                                CommandSource::Gui,
                                None,
                            );
                            self.status_msg = format!("Whisper 字幕 {count} 件を配置しました");
                        }
                        Err(e) => self.status_msg = format!("Whisper 実行エラー: {e}"),
                    }
                }
                if ui.button("☁ Gemini 書き起こし実行").clicked() {
                    let wav_path = PathBuf::from(self.transcribe_wav_path.trim());
                    match std::fs::read(&wav_path) {
                        Ok(bytes) => {
                            let duration_sec = decode_audio_file(&wav_path)
                                .map(|b| b.duration_sec())
                                .unwrap_or(60.0);
                            let gemini = GeminiTranscriber {
                                guard: self.guard.clone(),
                                ..Default::default()
                            };
                            match gemini.transcribe_audio_bytes(
                                &self.cred_store,
                                &bytes,
                                "audio/wav",
                                duration_sec,
                                self.cloud_confirm_checked,
                            ) {
                                Ok(segs) => {
                                    let items = segments_to_subtitle_items(&segs, 44.0, 560.0);
                                    let count = items.len();
                                    let _ = session.bus.execute(
                                        &mut session.project,
                                        EditCommand::ImportSubtitles {
                                            layer_index: 1,
                                            items,
                                        },
                                        CommandSource::Gui,
                                        None,
                                    );
                                    self.status_msg =
                                        format!("Gemini 字幕 {count} 件を配置しました");
                                }
                                Err(e) => self.status_msg = format!("Gemini 実行エラー: {e}"),
                            }
                        }
                        Err(e) => self.status_msg = format!("WAV 読み込みエラー: {e}"),
                    }
                }
                if ui.button("サンプル字幕生成 (テスト)").clicked() {
                    let demo_segs = vec![
                        SubtitleSegment {
                            start: RationalTime::ZERO,
                            end: RationalTime::from_seconds(2.0),
                            text: "自動文字起こしサンプル 1".into(),
                            phonemes: vec![],
                        },
                        SubtitleSegment {
                            start: RationalTime::from_seconds(2.1),
                            end: RationalTime::from_seconds(4.2),
                            text: "Ctrl+Z で一括Undoできます".into(),
                            phonemes: vec![],
                        },
                    ];
                    let items = segments_to_subtitle_items(&demo_segs, 44.0, 560.0);
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::ImportSubtitles {
                            layer_index: 1,
                            items,
                        },
                        CommandSource::Gui,
                        None,
                    );
                    self.status_msg = "サンプル字幕を配置しました".into();
                }
            });

            ui.separator();
            // ===== 4B. Claude 字幕文脈補正＆改行最適化（差分確認・1ステップUndo適用） =====
            ui.strong("✨ Claude 字幕文脈補正＆改行プレビュー（差分確認）");
            ui.horizontal_wrapped(|ui| {
                ui.label("1行目安:");
                ui.add(
                    egui::DragValue::new(&mut self.refine_wrap_chars)
                        .range(8..=40)
                        .suffix("文字"),
                );
                if ui.button("✨ Claude で字幕を文脈補正").clicked() {
                    self.preview_claude_refinement(session);
                }
                if ui.button("📝 自動改行プレビュー (オフライン)").clicked() {
                    self.preview_offline_wrap(session);
                }
            });

            if !self.refine_diffs.is_empty() {
                let changed_count = self
                    .refine_diffs
                    .iter()
                    .filter(|r| r.before != r.after)
                    .count();
                ui.weak(format!(
                    "差分プレビュー: 全 {} 件中 {} 件に変更あり（チェック済みを一括適用）",
                    self.refine_diffs.len(),
                    changed_count
                ));
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    for row in &mut self.refine_diffs {
                        let is_changed = row.before != row.after;
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut row.accepted, format!("{:.2}s", row.start_sec));
                            ui.vertical(|ui| {
                                ui.colored_label(
                                    if is_changed {
                                        egui::Color32::from_rgb(235, 130, 130)
                                    } else {
                                        egui::Color32::from_gray(150)
                                    },
                                    format!("Before: {}", row.before.replace('\n', " ⏎ ")),
                                );
                                ui.horizontal(|ui| {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(130, 230, 155),
                                        "After:",
                                    );
                                    ui.text_edit_singleline(&mut row.after);
                                });
                            });
                        });
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        if ui.button("✅ 選択差分を一括適用 (Undo可)").clicked() {
                            self.apply_accepted_diffs(session);
                        }
                        if ui.small_button("変更行のみ選択").clicked() {
                            for r in &mut self.refine_diffs {
                                r.accepted = r.before != r.after;
                            }
                        }
                        if ui.small_button("閉じる").clicked() {
                            self.refine_diffs.clear();
                        }
                    });
                });
            }

            ui.separator();

            // ===== 5. 台本（CSV / Markdown）→ タイムライン一括自動生成 (v0.6 #5) =====
            ui.heading("5. 台本（CSV / Markdown）→ タイムライン一括生成");
            ui.weak("形式例: `話者,セリフ,表情,背景画像` または `**霊夢** [smile]: セリフ`");
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("📂 .csv / .md 読込").clicked()
                    && let Some(p) = crate::native_dialog::pick_file(
                        "台本ファイル (.csv / .md / .txt)",
                        &["csv", "md", "txt"],
                    )
                {
                    match std::fs::read_to_string(&p) {
                        Ok(txt) => {
                            self.script_text = txt;
                            self.status_msg = format!("台本読込完了: {}", p.display());
                        }
                        Err(e) => self.status_msg = format!("台本読込エラー: {e}"),
                    }
                }
                if ui.small_button("📄 CSVサンプル").clicked() {
                    self.script_text =
                        "話者,セリフ,表情,背景画像\n霊夢,こんにちは！解説を始めるよ。,smile,\n魔理沙,台本CSVから自動でタイムラインを組むんだぜ！,surprise,\nずんだもん,話者カラーと座布団も自動で付くのだ！,happy,"
                            .to_string();
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.script_text)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label("読上速度:");
                ui.add(
                    egui::DragValue::new(&mut self.script_chars_per_sec)
                        .speed(0.2)
                        .range(2.0..=20.0)
                        .suffix("文字/秒"),
                );
                ui.label("間の秒数:");
                ui.add(
                    egui::DragValue::new(&mut self.script_pause_gap_sec)
                        .speed(0.05)
                        .range(0.0..=5.0)
                        .suffix("s"),
                );
                ui.checkbox(&mut self.script_show_speaker, "話者名表示");
                ui.checkbox(&mut self.script_use_tts, "ローカルTTS実尺連携");
            });
            if ui
                .button("🎬 台本からタイムラインを一括生成 (1ステップUndo)")
                .clicked()
            {
                self.build_timeline_from_script(session);
            }

            ui.separator();

            // ===== 6. 字幕・他ソフト互換（SRT / ASS / AviUtl .exo / OTIO） (v0.6 #6) =====
            ui.heading("6. 字幕・他ソフト互換 (SRT / ASS / AviUtl .exo / OTIO)");
            ui.horizontal_wrapped(|ui| {
                if ui.button("📥 SRT 読込").clicked()
                    && let Some(p) = crate::native_dialog::pick_file("SRT字幕ファイル", &["srt"])
                {
                    self.import_interop_file(session, &p, "srt");
                }
                if ui.button("📥 ASS 読込").clicked()
                    && let Some(p) = crate::native_dialog::pick_file("ASS字幕ファイル", &["ass"])
                {
                    self.import_interop_file(session, &p, "ass");
                }
                if ui.button("📥 AviUtl .exo 読込").clicked()
                    && let Some(p) =
                        crate::native_dialog::pick_file("AviUtl オブジェクトファイル", &["exo"])
                {
                    self.import_interop_file(session, &p, "exo");
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("書出名:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.interop_export_stem)
                        .desired_width(120.0),
                );
                if ui.button("📤 SRT 書出").clicked() {
                    self.export_interop_file(session, "srt");
                }
                if ui.button("📤 ASS 書出").clicked() {
                    self.export_interop_file(session, "ass");
                }
                if ui.button("📤 OTIO (.otio) 書出").clicked() {
                    self.export_interop_file(session, "otio");
                }
            });

            ui.separator();
            ui.monospace(format!("ステータス: {}", self.status_msg));
        });
    }

    fn collect_scene_text_items(
        session: &EditorSession,
    ) -> Vec<(ItemId, RationalTime, RationalTime, String)> {
        let mut rows = Vec::new();
        if let Some(scene) = session.project.active_scene() {
            for layer in &scene.layers {
                for item in &layer.items {
                    if let ItemKind::Text { content, .. } = &item.kind {
                        rows.push((item.id, item.start, item.end(), content.clone()));
                    }
                }
            }
        }
        rows.sort_by_key(|(_, start, _, _)| *start);
        rows
    }

    fn preview_offline_wrap(&mut self, session: &EditorSession) {
        let items = Self::collect_scene_text_items(session);
        if items.is_empty() {
            self.status_msg = "シーン内に字幕テキストがありません".into();
            return;
        }
        let max_chars = self.refine_wrap_chars.max(6);
        self.refine_diffs = items
            .into_iter()
            .map(|(item_id, start, _, before)| {
                let after = wrap_subtitle_japanese(&before, max_chars);
                let changed = before != after;
                SubtitleDiffRow {
                    item_id,
                    start_sec: start.as_seconds(),
                    before,
                    after,
                    accepted: changed,
                }
            })
            .collect();
        let changed = self
            .refine_diffs
            .iter()
            .filter(|r| r.before != r.after)
            .count();
        self.status_msg = format!(
            "オフライン自動改行プレビュー生成完了（{} 件中 {} 件に変更案）",
            self.refine_diffs.len(),
            changed
        );
    }

    fn preview_claude_refinement(&mut self, session: &EditorSession) {
        let items = Self::collect_scene_text_items(session);
        if items.is_empty() {
            self.status_msg = "シーン内に字幕テキストがありません".into();
            return;
        }
        let segments: Vec<SubtitleSegment> = items
            .iter()
            .map(|(_, start, end, text)| SubtitleSegment {
                start: *start,
                end: *end,
                text: text.clone(),
                phonemes: Vec::new(),
            })
            .collect();
        let duration_sec = items
            .iter()
            .map(|(_, _, end, _)| end.as_seconds())
            .fold(1.0_f64, f64::max);

        let refiner = ClaudeRefiner {
            guard: self.guard.clone(),
            ..Default::default()
        };
        match refiner.refine_subtitles(
            &self.cred_store,
            &segments,
            duration_sec,
            self.cloud_confirm_checked,
        ) {
            Ok(refined) => {
                self.refine_diffs = items
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (item_id, start, _, before))| {
                        let after = refined
                            .get(idx)
                            .map(|s| s.text.clone())
                            .unwrap_or_else(|| before.clone());
                        let changed = before != after;
                        SubtitleDiffRow {
                            item_id,
                            start_sec: start.as_seconds(),
                            before,
                            after,
                            accepted: changed,
                        }
                    })
                    .collect();
                let changed = self
                    .refine_diffs
                    .iter()
                    .filter(|r| r.before != r.after)
                    .count();
                self.status_msg = format!(
                    "Claude 文脈補正プレビュー完了（{} 件中 {} 件に補正案 — 確認後に適用してください）",
                    self.refine_diffs.len(),
                    changed
                );
            }
            Err(e) => {
                self.status_msg = format!("Claude 文脈補正エラー: {e}");
            }
        }
    }

    fn apply_accepted_diffs(&mut self, session: &mut EditorSession) {
        let commands: Vec<EditCommand> = self
            .refine_diffs
            .iter()
            .filter(|r| r.accepted && r.before != r.after)
            .map(|r| EditCommand::SetTextContent {
                item_id: r.item_id,
                content: r.after.clone(),
            })
            .collect();
        let count = commands.len();
        if count == 0 {
            self.status_msg = "適用対象の変更行がありません".into();
            return;
        }
        match session.bus.execute(
            &mut session.project,
            EditCommand::Batch {
                label: "apply_subtitle_refinement".into(),
                commands,
            },
            CommandSource::Gui,
            None,
        ) {
            Ok(_) => {
                self.refine_diffs.clear();
                self.status_msg =
                    format!("字幕 {count} 件の補正差分を一括適用しました（Ctrl+Z で元に戻せます）");
            }
            Err(e) => {
                self.status_msg = format!("差分適用エラー: {e}");
            }
        }
    }

    fn import_pair(
        &mut self,
        session: &mut EditorSession,
        waveforms: &mut BTreeMap<Uuid, WaveformPeaks>,
    ) {
        let wav_path = PathBuf::from(self.tts_pair_wav_path.trim());
        match import_tts_file_pair(&wav_path) {
            Ok(bundle) => {
                let asset_id = Uuid::new_v4();
                if let Ok(buf) = decode_audio_file(&wav_path) {
                    waveforms.insert(asset_id, WaveformPeaks::from_buffer(&buf, 100));
                }
                session.project.assets.push(AssetRef {
                    id: asset_id,
                    name: wav_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "voice.wav".into()),
                    kind: MediaKind::Audio,
                    relative_path: None,
                    fallback_absolute_path: Some(wav_path),
                    collected: false,
                });
                let start = session.playhead;
                let mut audio_item = TimelineItem::new(
                    "TTS音声",
                    start,
                    bundle.duration,
                    ItemKind::Audio {
                        asset_id,
                        source_offset: RationalTime::ZERO,
                        volume: 1.0,
                    },
                );
                audio_item.phoneme_segments = bundle.phonemes.clone();
                let sub_item = TimelineItem::new(
                    "TTS字幕",
                    start,
                    bundle.duration,
                    ItemKind::Text {
                        content: bundle.subtitle_text.clone(),
                        font_family: "sans-serif".to_string(),
                        font_size: 46.0,
                        color: RgbaColor::WHITE,
                        animator: TextAnimator::default(),
                    },
                );
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::Batch {
                        label: "import_tts_file_pair".into(),
                        commands: vec![
                            EditCommand::AddObject {
                                layer_index: 3,
                                item: audio_item,
                            },
                            EditCommand::AddObject {
                                layer_index: 1,
                                item: sub_item,
                            },
                        ],
                    },
                    CommandSource::Gui,
                    None,
                );
                self.status_msg = format!(
                    "TTSペア取込完了（字幕「{}」/ 音素 {} 個）",
                    bundle.subtitle_text,
                    bundle.phonemes.len()
                );
            }
            Err(e) => {
                self.status_msg = format!("TTSペア取込エラー: {e}");
            }
        }
    }

    fn spawn_lipsync_demo(
        &mut self,
        session: &mut EditorSession,
        waveforms: &mut BTreeMap<Uuid, WaveformPeaks>,
    ) {
        let audio_asset = Uuid::new_v4();
        // 5秒分の疑似発話波形（周期的に声が出るRMSエンベロープ）
        let mut buckets = Vec::with_capacity(250);
        for i in 0..250 {
            let phase = (i as f32 * 0.25).sin().abs();
            let rms = if (i / 15) % 2 == 0 {
                phase * 0.35
            } else {
                0.01
            };
            buckets.push(WaveformBucket {
                min: -rms,
                max: rms,
                rms,
            });
        }
        waveforms.insert(
            audio_asset,
            WaveformPeaks {
                sample_rate: 48_000,
                samples_per_bucket: 960,
                buckets,
            },
        );

        let mut audio_item = TimelineItem::new(
            "デモ発話音声 (L3)",
            RationalTime::ZERO,
            RationalTime::from_seconds(5.0),
            ItemKind::Audio {
                asset_id: audio_asset,
                source_offset: RationalTime::ZERO,
                volume: 1.0,
            },
        );
        // 母音タイムラインも同時に付与（音量連動／母音連動どちらも即確認可能）
        let vowels = [
            ("a", MouthShape::A),
            ("i", MouthShape::I),
            ("u", MouthShape::U),
            ("e", MouthShape::E),
            ("o", MouthShape::O),
            ("pau", MouthShape::Closed),
        ];
        for i in 0..18 {
            let (sym, mouth) = vowels[i % vowels.len()];
            audio_item.phoneme_segments.push(PhonemeSegment {
                start: RationalTime::from_seconds(i as f64 * 0.25),
                end: RationalTime::from_seconds((i + 1) as f64 * 0.25),
                phoneme: sym.to_string(),
                mouth,
            });
        }

        let mut sprite = TimelineItem::new(
            "立ち絵 (自動口パク)",
            RationalTime::ZERO,
            RationalTime::from_seconds(5.0),
            ItemKind::CharacterSprite {
                config: CharacterSpriteConfig {
                    linked_audio_layer: Some(3),
                    mode: LipSyncMode::default(),
                    png_paths: BTreeMap::new(),
                    psd_config: None,
                    phoneme_override: Vec::new(),
                },
                width: 240.0,
                height: 300.0,
            },
        );
        sprite.transform = Transform2D {
            x: 920.0,
            y: 360.0,
            ..Default::default()
        };
        let sprite_id = sprite.id;

        if session
            .bus
            .execute(
                &mut session.project,
                EditCommand::Batch {
                    label: "spawn_lipsync_demo".into(),
                    commands: vec![
                        EditCommand::AddObject {
                            layer_index: 3,
                            item: audio_item,
                        },
                        EditCommand::AddObject {
                            layer_index: 2,
                            item: sprite,
                        },
                    ],
                },
                CommandSource::Gui,
                None,
            )
            .is_ok()
        {
            session.selected_items = vec![sprite_id];
            self.status_msg =
                "デモ立ち絵＋音声を配置しました（再生すると口パクが連動します）".into();
        }
    }

    fn build_timeline_from_script(&mut self, session: &mut EditorSession) {
        let lines = parse_script_auto(&self.script_text);
        if lines.is_empty() {
            self.status_msg = "台本から有効なセリフ行を検出できませんでした".into();
            return;
        }
        let Some((scene_w, scene_h, max_layer)) = session
            .project
            .active_scene()
            .map(|s| (s.width, s.height, s.layers.len().saturating_sub(1)))
        else {
            return;
        };

        let mut tts_durations: Vec<f64> = Vec::new();
        let mut tts_hits = 0usize;
        if self.script_use_tts {
            let client = LocalTtsClient::new(self.tts_engine, self.tts_speaker_id);
            for line in &lines {
                match client.synthesize(&line.text) {
                    Ok(bundle) => {
                        tts_durations.push(bundle.duration.as_seconds());
                        tts_hits += 1;
                    }
                    Err(_) => tts_durations.push(0.0),
                }
            }
        }
        let durations_opt = (!tts_durations.is_empty()).then_some(tts_durations.as_slice());

        let cfg = ScriptBuildConfig {
            start_time: session.playhead,
            chars_per_sec: self.script_chars_per_sec,
            pause_gap_sec: self.script_pause_gap_sec,
            subtitle_layer: 1usize.min(max_layer),
            show_speaker_prefix: self.script_show_speaker,
            ..Default::default()
        };

        let Some((count, end_time, mut cmd)) =
            build_script_timeline_commands(&lines, durations_opt, &cfg, scene_w, scene_h)
        else {
            self.status_msg = "タイムライン生成に失敗しました".into();
            return;
        };

        // 台本に背景画像パスが指定されていれば Layer 0 へ同時配置
        if let EditCommand::Batch {
            ref mut commands, ..
        } = cmd
        {
            let mut bg_cmds = Vec::new();
            for (idx, line) in lines.iter().enumerate() {
                let Some(img_str) = line.image_path.as_deref() else {
                    continue;
                };
                let p = PathBuf::from(img_str);
                if !p.exists() {
                    continue;
                }
                if let Some(EditCommand::AddObject { item: sub_item, .. }) = commands.get(idx) {
                    let asset_id = Uuid::new_v4();
                    let fname = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("bg.png")
                        .to_string();
                    session.project.assets.push(AssetRef {
                        id: asset_id,
                        name: fname.clone(),
                        kind: MediaKind::Image,
                        relative_path: None,
                        fallback_absolute_path: Some(p),
                        collected: false,
                    });
                    bg_cmds.push(EditCommand::AddObject {
                        layer_index: 0,
                        item: TimelineItem::new(
                            format!("背景: {fname}"),
                            sub_item.start,
                            sub_item.duration,
                            ItemKind::Image { asset_id },
                        ),
                    });
                }
            }
            commands.extend(bg_cmds);
        }

        match session
            .bus
            .execute(&mut session.project, cmd, CommandSource::Gui, None)
        {
            Ok(()) => {
                self.status_msg = if self.script_use_tts {
                    format!(
                        "台本 {count} 行を配置完了（終了 {:.1}s / TTS連携 {tts_hits}件 / Ctrl+Zで一括Undo）",
                        end_time.as_seconds()
                    )
                } else {
                    format!(
                        "台本 {count} 行を自動配置しました（終了 {:.1}s / Ctrl+Zで一括Undo）",
                        end_time.as_seconds()
                    )
                };
            }
            Err(e) => self.status_msg = format!("台本配置エラー: {e}"),
        }
    }

    fn import_interop_file(
        &mut self,
        session: &mut EditorSession,
        path: &std::path::Path,
        fmt: &str,
    ) {
        let Ok(content) = std::fs::read_to_string(path) else {
            self.status_msg = format!("ファイル読込失敗: {}", path.display());
            return;
        };
        let Some((scene_w, scene_h, scene_fps, max_layer)) = session
            .project
            .active_scene()
            .map(|s| (s.width, s.height, s.fps, s.layers.len().saturating_sub(1)))
        else {
            return;
        };
        let target_layer = 1usize.min(max_layer);
        let result = match fmt {
            "srt" => {
                let entries = parse_srt_subtitles(&content);
                let n = entries.len();
                subtitle_entries_to_commands(&entries, target_layer, scene_w, scene_h)
                    .map(|c| (n, c))
            }
            "ass" => {
                let entries = parse_ass_subtitles(&content);
                let n = entries.len();
                subtitle_entries_to_commands(&entries, target_layer, scene_w, scene_h)
                    .map(|c| (n, c))
            }
            "exo" => {
                let parsed = parse_aviutl_exo(&content, scene_fps);
                let n = parsed.len();
                if n == 0 {
                    None
                } else {
                    let commands = parsed
                        .into_iter()
                        .map(|(l_idx, item)| EditCommand::AddObject {
                            layer_index: l_idx.min(max_layer),
                            item,
                        })
                        .collect();
                    Some((
                        n,
                        EditCommand::Batch {
                            label: "import_aviutl_exo".into(),
                            commands,
                        },
                    ))
                }
            }
            _ => None,
        };
        let Some((count, cmd)) = result else {
            self.status_msg = format!(".{fmt} から有効なオブジェクトを検出できませんでした");
            return;
        };
        match session
            .bus
            .execute(&mut session.project, cmd, CommandSource::Gui, None)
        {
            Ok(()) => {
                self.status_msg = format!(
                    "{} を取込完了（{count} 件 / Ctrl+Zで一括Undo）",
                    path.display()
                );
            }
            Err(e) => self.status_msg = format!("取込エラー: {e}"),
        }
    }

    fn export_interop_file(&mut self, session: &EditorSession, fmt: &str) {
        let out_dir = session
            .folder
            .as_ref()
            .map(|f| f.exports_dir())
            .unwrap_or_else(|| PathBuf::from("exports"));
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            self.status_msg = format!("出力フォルダ作成失敗: {e}");
            return;
        }
        let stem = crate::render_queue::RenderQueueState::sanitize_stem(&self.interop_export_stem);
        let payload = match fmt {
            "srt" => session
                .project
                .active_scene()
                .map(export_srt_subtitles)
                .unwrap_or_default(),
            "ass" => session
                .project
                .active_scene()
                .map(export_ass_subtitles)
                .unwrap_or_default(),
            "otio" => export_otio_json(&session.project, session.project.active_scene_id)
                .unwrap_or_default(),
            _ => return,
        };
        let out_path = out_dir.join(format!("{stem}.{fmt}"));
        match std::fs::write(&out_path, payload) {
            Ok(()) => {
                self.status_msg = format!("書出完了: {}", out_path.display());
            }
            Err(e) => self.status_msg = format!("書出エラー: {e}"),
        }
    }
}
