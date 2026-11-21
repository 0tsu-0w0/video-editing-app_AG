use egui::Ui;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;
use ve_ai::{
    ApiProvider, CloudUploadGuard, CredentialStore, LocalTtsClient, LocalWhisperTranscriber,
    OsCredentialStore, SubtitleSegment, TtsEngine, import_tts_file_pair,
    segments_to_subtitle_items,
};
use ve_audio::{WaveformBucket, WaveformPeaks};
use ve_core::{
    AssetRef, CharacterSpriteConfig, CommandSource, EditCommand, ItemKind, LipSyncMode, MediaKind,
    MouthShape, PhonemeSegment, PsdCharacterConfig, RationalTime, RgbaColor, TextAnimator,
    TimelineItem, Transform2D,
};
use ve_mcp::EditorSession;
use ve_media::decode_audio_file;

pub struct AudioAi3DPanel {
    cred_store: OsCredentialStore,
    guard: CloudUploadGuard,
    gemini_key_input: String,
    claude_key_input: String,
    whisper_model_path: String,
    transcribe_wav_path: String,
    cloud_confirm_checked: bool,
    tts_pair_wav_path: String,
    tts_engine: TtsEngine,
    tts_speaker_id: u32,
    tts_text: String,
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
            tts_pair_wav_path: String::new(),
            tts_engine: TtsEngine::Voicevox,
            tts_speaker_id: 3,
            tts_text: "こんにちは！次世代ハイブリッド動画編集ソフトです。".to_string(),
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
            });
            ui.horizontal(|ui| {
                ui.label("Whisperモデル:");
                ui.text_edit_singleline(&mut self.whisper_model_path);
            });

            let est = self
                .guard
                .estimate_audio_transcription(ApiProvider::Gemini, 60.0);
            ui.checkbox(
                &mut self.cloud_confirm_checked,
                format!("外部送信・概算コスト確認済み ({})", est.summary_message),
            );

            ui.horizontal(|ui| {
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
            ui.monospace(format!("ステータス: {}", self.status_msg));
        });
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
}
