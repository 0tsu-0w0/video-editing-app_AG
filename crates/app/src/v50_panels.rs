use egui::{Color32, Pos2, Stroke, Vec2};
use std::path::PathBuf;
use uuid::Uuid;
use ve_audio::{BgmComposeConfig, BgmGenre, compose_bgm_track, write_stereo_wav_file};
use ve_core::{
    AssetRef, CinemaFinishingConfig, CommandSource, DetectedSceneCut, EditCommand, ItemId,
    ItemKind, LetterboxAspect, MediaKind, RationalTime, Rig2DConfig, RigMotionPreset,
    SpeedRampConfig, SpeedRampPreset, TimelineItem, TimelineMarker, build_scene_cut_commands,
    evaluate_rig_poses, evaluate_speed_at_norm_time, export_cmx3600_edl, export_fcpxml_v10,
};
use ve_mcp::SharedEditorSession;
use ve_render::{RenderGraph, detect_scene_cuts_in_scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V50Tab {
    Rig2D,
    BgmComposer,
    SpeedRampCuts,
    CinemaFinishing,
}

/// v5.0 #37〜#40 プロフィニッシング＆モーションリギング統合ウィンドウ状態
pub struct V50PanelsState {
    pub show_window: bool,
    pub active_tab: V50Tab,

    // --- #37 2D Bone / IK Rig ---
    rig_cfg: Rig2DConfig,
    rig_duration_sec: f32,
    rig_layer_index: usize,
    rig_preview_time: f32,
    rig_status: String,

    // --- #38 Offline BGM Composer ---
    bgm_cfg: BgmComposeConfig,
    bgm_audio_layer: usize,
    bgm_add_bar_markers: bool,
    bgm_status: String,

    // --- #39 Speed Ramp & Scene Cut Detection ---
    ramp_cfg: SpeedRampConfig,
    cut_interval_sec: f64,
    cut_sensitivity: f32,
    cut_split_clips: bool,
    last_cuts: Vec<DetectedSceneCut>,
    ramp_status: String,

    // --- #40 Cinema Finishing & EDL/FCPXML ---
    interchange_preview: String,
    finishing_status: String,
}

impl Default for V50PanelsState {
    fn default() -> Self {
        Self {
            show_window: false,
            active_tab: V50Tab::Rig2D,
            rig_cfg: Rig2DConfig::default_humanoid(),
            rig_duration_sec: 5.0,
            rig_layer_index: 1,
            rig_preview_time: 0.0,
            rig_status: "10 ボーン階層 FK＋2-Bone 解析 IK＋髪・リボン二次揺れパペットを制御します"
                .into(),
            bgm_cfg: BgmComposeConfig::default(),
            bgm_audio_layer: 3,
            bgm_add_bar_markers: true,
            bgm_status:
                "完全オフライン・著作権フリーで 6 ジャンルの 48kHz ステレオ BGM を自動作曲します"
                    .into(),
            ramp_cfg: SpeedRampPreset::BulletTimeDip.default_config(),
            cut_interval_sec: 0.25,
            cut_sensitivity: 0.55,
            cut_split_clips: true,
            last_cuts: Vec::new(),
            ramp_status: "スピードランプ曲線適用 ＆ ヒストグラム χ² 自動シーンカット検出".into(),
            interchange_preview: String::new(),
            finishing_status:
                "35mm ハレーション・アナモルフィックフレア・グレイン・EDL/FCPXML 出力".into(),
        }
    }
}

impl V50PanelsState {
    #[allow(dead_code)]
    pub fn show_v50_window(
        &mut self,
        ctx: &egui::Context,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        let mut open = self.show_window;
        egui::Window::new(
            "🚀 v5.0 プロフィニッシング＆リギング (2D IK・自動BGM・スピードランプ・EDL/FCPXML)",
        )
        .open(&mut open)
        .default_width(680.0)
        .default_height(610.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.active_tab, V50Tab::Rig2D, "🦴 2D ボーン＆IK");
                ui.selectable_value(&mut self.active_tab, V50Tab::BgmComposer, "🎼 自動BGM作曲");
                ui.selectable_value(
                    &mut self.active_tab,
                    V50Tab::SpeedRampCuts,
                    "⚡ 速度曲線＆カット検出",
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    V50Tab::CinemaFinishing,
                    "🎬 シネマ仕上げ＆EDL",
                );
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| match self.active_tab {
                V50Tab::Rig2D => {
                    if self.show_rig_tab(ui, session, selected_item) {
                        mutated = true;
                    }
                }
                V50Tab::BgmComposer => {
                    if self.show_bgm_tab(ui, session) {
                        mutated = true;
                    }
                }
                V50Tab::SpeedRampCuts => {
                    if self.show_speed_ramp_tab(ui, session, selected_item) {
                        mutated = true;
                    }
                }
                V50Tab::CinemaFinishing => {
                    if self.show_finishing_tab(ui, session) {
                        mutated = true;
                    }
                }
            });
        });
        self.show_window = open;
        mutated
    }

    pub fn show_rig_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        ui.heading("🦴 2D スケルトンボーン ＆ 2-Bone IK パペットリグ (#37)");
        egui::ComboBox::from_label("モーションプリセット")
            .selected_text(self.rig_cfg.preset.display_name())
            .show_ui(ui, |ui| {
                for &p in RigMotionPreset::ALL {
                    ui.selectable_value(&mut self.rig_cfg.preset, p, p.display_name());
                }
            });
        ui.add(egui::Slider::new(&mut self.rig_cfg.motion_speed, 0.0..=4.0).text("モーション速度"));
        ui.add(egui::Slider::new(&mut self.rig_cfg.motion_amp, 0.0..=2.5).text("揺れ振幅"));
        ui.checkbox(
            &mut self.rig_cfg.show_bones_overlay,
            "ボーン軸・関節・IK ターゲットを重ね表示",
        );

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.rig_cfg.right_arm_ik.enabled, "右腕 2-Bone IK");
            ui.add(
                egui::Slider::new(&mut self.rig_cfg.right_arm_ik.target_pos[0], -150.0..=150.0)
                    .text("R-X"),
            );
            ui.add(
                egui::Slider::new(&mut self.rig_cfg.right_arm_ik.target_pos[1], -180.0..=140.0)
                    .text("R-Y"),
            );
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.rig_cfg.left_arm_ik.enabled, "左腕 2-Bone IK");
            ui.add(
                egui::Slider::new(&mut self.rig_cfg.left_arm_ik.target_pos[0], -150.0..=150.0)
                    .text("L-X"),
            );
            ui.add(
                egui::Slider::new(&mut self.rig_cfg.left_arm_ik.target_pos[1], -180.0..=140.0)
                    .text("L-Y"),
            );
        });
        ui.add(
            egui::Slider::new(&mut self.rig_preview_time, 0.0..=4.0).text("プレビュー時刻 (秒)"),
        );

        // ライブスケルトン可視化キャンバス
        let poses = evaluate_rig_poses(&self.rig_cfg, self.rig_preview_time);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(600.0, 170.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(14, 18, 28));
        let cx = rect.center().x;
        let cy = rect.center().y + 10.0;
        let sc = 0.44f32;
        let to_screen = |p: [f32; 2]| Pos2::new(cx + p[0] * sc, cy + p[1] * sc);
        for pose in &poses {
            let p0 = to_screen(pose.start);
            let p1 = to_screen(pose.end);
            painter.line_segment([p0, p1], Stroke::new(2.2, Color32::from_rgb(60, 230, 245)));
            painter.circle_filled(p0, 3.2, Color32::from_rgb(255, 215, 70));
        }
        for ik in [self.rig_cfg.right_arm_ik, self.rig_cfg.left_arm_ik] {
            if ik.enabled && ik.lower_bone_idx < poses.len() {
                let end = to_screen(poses[ik.lower_bone_idx].end);
                painter.circle_stroke(end, 6.0, Stroke::new(1.8, Color32::from_rgb(255, 65, 180)));
            }
        }

        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut self.rig_duration_sec, 1.0..=30.0).text("尺(秒)"));
            ui.add(egui::Slider::new(&mut self.rig_layer_index, 0..=3).text("レイヤー"));
        });
        ui.horizontal(|ui| {
            if ui.button("✨ パペットリグをタイムラインへ追加").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                let max_l = s
                    .project
                    .active_scene()
                    .map(|sc| sc.layers.len().saturating_sub(1))
                    .unwrap_or(0);
                let item = TimelineItem::new(
                    format!("Rig2D: {}", self.rig_cfg.preset.display_name()),
                    RationalTime::ZERO,
                    RationalTime::from_seconds(f64::from(self.rig_duration_sec)),
                    ItemKind::RiggedPuppet {
                        config: Box::new(self.rig_cfg.clone()),
                    },
                );
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::AddObject {
                            layer_index: self.rig_layer_index.min(max_l),
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.rig_status = "2D スケルトンパペットを追加しました".into();
                    mutated = true;
                }
            }
            if let Some(sel_id) = selected_item
                && ui.button("🔄 選択中パペットへ適用").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetRig2D {
                            item_id: sel_id,
                            config: Box::new(self.rig_cfg.clone()),
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.rig_status = "選択パペットのリグ設定を更新しました".into();
                    mutated = true;
                } else {
                    self.rig_status = "選択クリップは 2D パペットリグではありません".into();
                }
            }
        });
        ui.colored_label(Color32::LIGHT_GREEN, &self.rig_status);
        mutated
    }

    pub fn show_bgm_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut mutated = false;
        ui.heading("🎼 オフライン自動プロシージャル BGM コンポーザー (#38)");
        ui.label(
            "著作権フリー＆クラウド不要で、ドラム・ベース・コードパッド・メロディを 48kHz ステレオ WAV 合成し、小節ビートマーカーと同時に 1-Undo 配置します。",
        );

        let prev_genre = self.bgm_cfg.genre;
        egui::ComboBox::from_label("BGM ジャンル")
            .selected_text(self.bgm_cfg.genre.display_name())
            .show_ui(ui, |ui| {
                for &g in BgmGenre::ALL {
                    ui.selectable_value(&mut self.bgm_cfg.genre, g, g.display_name());
                }
            });
        if self.bgm_cfg.genre != prev_genre {
            self.bgm_cfg.bpm = self.bgm_cfg.genre.default_bpm();
        }
        ui.label(format!(
            "コード進行: {}",
            self.bgm_cfg.genre.progression_label()
        ));

        ui.add(egui::Slider::new(&mut self.bgm_cfg.bpm, 60.0..=180.0).text("テンポ (BPM)"));
        ui.add(
            egui::Slider::new(&mut self.bgm_cfg.key_semitone, 0..=11)
                .text("キー移調 (0=C .. 11=B)"),
        );
        ui.add(egui::Slider::new(&mut self.bgm_cfg.duration_sec, 4.0..=60.0).text("曲の長さ (秒)"));
        ui.add(egui::Slider::new(&mut self.bgm_cfg.intensity, 0.4..=1.4).text("アレンジ強度"));
        ui.add(egui::Slider::new(&mut self.bgm_audio_layer, 0..=3).text("配置音声レイヤー"));
        ui.checkbox(
            &mut self.bgm_add_bar_markers,
            "小節頭（4拍ごと）のビートマーカーを同時配置",
        );

        if ui
            .button("🎵 自動作曲して 48kHz WAV ＆ タイムライン配置")
            .clicked()
            && let Ok(mut guard) = session.lock()
        {
            let s = &mut *guard;
            let sr = s.project.settings.audio_sample_rate.max(8_000);
            let res = compose_bgm_track(&self.bgm_cfg, sr);
            self.bgm_cfg.seed = self.bgm_cfg.seed.wrapping_add(1);

            let out_dir = s
                .folder
                .as_ref()
                .map(|f| f.assets_dir().join("bgm"))
                .unwrap_or_else(|| std::env::temp_dir().join("ve_bgm"));
            let wav_name = format!("bgm_{}_{}.wav", self.bgm_cfg.genre.id(), self.bgm_cfg.seed);
            let wav_path = out_dir.join(&wav_name);
            match write_stereo_wav_file(&wav_path, &res.pcm, res.sample_rate) {
                Ok(()) => {
                    let asset_id = Uuid::new_v4();
                    s.project.assets.push(AssetRef {
                        id: asset_id,
                        name: wav_name.clone(),
                        relative_path: Some(PathBuf::from("assets/bgm").join(&wav_name)),
                        fallback_absolute_path: Some(wav_path.clone()),
                        collected: true,
                        kind: MediaKind::Audio,
                    });
                    let max_l = s
                        .project
                        .active_scene()
                        .map(|sc| sc.layers.len().saturating_sub(1))
                        .unwrap_or(0);
                    let mut cmds = vec![EditCommand::AddObject {
                        layer_index: self.bgm_audio_layer.min(max_l),
                        item: TimelineItem::new(
                            format!("BGM: {} ({:.0}BPM)", self.bgm_cfg.genre.id(), res.bpm),
                            RationalTime::ZERO,
                            RationalTime::from_seconds(f64::from(self.bgm_cfg.duration_sec)),
                            ItemKind::Audio {
                                asset_id,
                                source_offset: RationalTime::ZERO,
                                volume: 0.72,
                            },
                        ),
                    }];
                    if self.bgm_add_bar_markers {
                        for (idx, &bt) in res.bar_times_sec.iter().enumerate() {
                            cmds.push(EditCommand::AddMarker {
                                marker: TimelineMarker::new(
                                    RationalTime::from_seconds(bt),
                                    format!("Bar {}", idx + 1),
                                ),
                            });
                        }
                    }
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!("自動 BGM 作曲 ({})", self.bgm_cfg.genre.id()),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.bgm_status = format!(
                            "作曲＆配置完了: {}（{} 小節マーカー・{}）",
                            wav_path.display(),
                            res.bar_times_sec.len(),
                            res.chord_progression
                        );
                        mutated = true;
                    }
                }
                Err(e) => {
                    self.bgm_status = format!("WAV 書き出しエラー: {e}");
                }
            }
        }
        ui.colored_label(Color32::LIGHT_GREEN, &self.bgm_status);
        mutated
    }

    pub fn show_speed_ramp_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut mutated = false;
        ui.heading("⚡ スピードランプ（Time-Remapping 解析積分曲線）(#39)");
        let prev_preset = self.ramp_cfg.preset;
        egui::ComboBox::from_label("速度曲線プリセット")
            .selected_text(self.ramp_cfg.preset.display_name())
            .show_ui(ui, |ui| {
                for &p in SpeedRampPreset::ALL {
                    ui.selectable_value(&mut self.ramp_cfg.preset, p, p.display_name());
                }
            });
        if self.ramp_cfg.preset != prev_preset {
            self.ramp_cfg = self.ramp_cfg.preset.default_config();
        }
        ui.checkbox(&mut self.ramp_cfg.enabled, "スピードランプを有効化");
        ui.checkbox(
            &mut self.ramp_cfg.smooth_blend,
            "3次 Smoothstep 滑らか加減速補間",
        );

        // 速度曲線グラフ可視化
        let (rect, _) = ui.allocate_exact_size(Vec2::new(600.0, 120.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(14, 18, 28));
        // 1.0x 基準線
        let y_one = rect.bottom() - (1.0 / 4.5) * rect.height();
        painter.line_segment(
            [
                Pos2::new(rect.left(), y_one),
                Pos2::new(rect.right(), y_one),
            ],
            Stroke::new(1.0, Color32::from_rgb(80, 92, 115)),
        );
        let pts: Vec<Pos2> = (0..=64)
            .map(|i| {
                let u = i as f32 / 64.0;
                let spd = evaluate_speed_at_norm_time(&self.ramp_cfg, u);
                let x = rect.left() + u * rect.width();
                let y = rect.bottom() - ((spd / 4.5).clamp(0.02, 0.98)) * rect.height();
                Pos2::new(x, y)
            })
            .collect();
        painter.add(egui::Shape::line(
            pts,
            Stroke::new(2.2, Color32::from_rgb(255, 195, 60)),
        ));

        if let Some(sel_id) = selected_item {
            if ui.button("🚀 選択クリップへスピードランプを適用").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetSpeedRamp {
                            item_id: sel_id,
                            speed_ramp: self.ramp_cfg.clone(),
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.ramp_status = format!(
                        "スピードランプを適用しました: {}",
                        self.ramp_cfg.preset.display_name()
                    );
                    mutated = true;
                }
            }
        } else {
            ui.label("※タイムラインでクリップを選択するとスピードランプを適用できます");
        }

        ui.separator();
        ui.heading("✂ インテリジェント自動シーンカット検出（Scene Edit Detection）");
        ui.add(
            egui::Slider::new(&mut self.cut_interval_sec, 0.1..=0.8)
                .text("解析サンプリング間隔 (秒)"),
        );
        ui.add(egui::Slider::new(&mut self.cut_sensitivity, 0.1..=1.0).text("カット検出感度"));
        ui.checkbox(
            &mut self.cut_split_clips,
            "検出ショット境界で映像クリップを自動分割する",
        );

        ui.horizontal(|ui| {
            if ui.button("🔍 シーンカットをスキャン").clicked()
                && let Ok(s) = session.lock()
            {
                let mut rg = RenderGraph::new();
                rg.set_project_folder(s.folder.clone());
                self.last_cuts = detect_scene_cuts_in_scene(
                    &mut rg,
                    &s.project,
                    s.project.active_scene_id,
                    self.cut_interval_sec,
                    self.cut_sensitivity,
                );
                self.ramp_status =
                    format!("{} 箇所のショット境界を検出しました", self.last_cuts.len());
            }
            if !self.last_cuts.is_empty()
                && ui.button("⚡ マーカー＆自動分割を 1-Undo 適用").clicked()
                && let Ok(mut guard) = session.lock()
            {
                let s = &mut *guard;
                let cmds = s
                    .project
                    .active_scene()
                    .map(|sc| {
                        build_scene_cut_commands(sc, &self.last_cuts, self.cut_split_clips, None)
                    })
                    .unwrap_or_default();
                let n_cmds = cmds.len();
                if !cmds.is_empty()
                    && s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: format!("自動シーンカット適用 ({}件)", self.last_cuts.len()),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                {
                    self.ramp_status = format!("{n_cmds} コマンドを 1-Undo で適用しました");
                    mutated = true;
                }
            }
        });
        for (i, c) in self.last_cuts.iter().take(8).enumerate() {
            ui.label(format!(
                "Cut #{}: {:.2}s (信頼度 {:.0}% / χ²={:.2} / ΔLuma={:.1} IRE)",
                i + 1,
                c.time_sec,
                c.confidence * 100.0,
                c.hist_chi_sq,
                c.luma_delta_ire
            ));
        }

        ui.colored_label(Color32::LIGHT_GREEN, &self.ramp_status);
        mutated
    }

    pub fn show_finishing_tab(&mut self, ui: &mut egui::Ui, session: &SharedEditorSession) -> bool {
        let mut mutated = false;
        let Ok(cur_cfg) = session.lock().map(|s| {
            s.project
                .active_scene()
                .map(|sc| sc.cinema_finishing.clone())
                .unwrap_or_default()
        }) else {
            return false;
        };
        let mut cfg = cur_cfg.clone();

        ui.heading(
            "🎬 プロ・シネマフィニッシング（ハレーション・フレア・グレイン・バーンイン）(#40)",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("🎞 Kodak 35mm フィルム").clicked() {
                cfg = CinemaFinishingConfig::preset_kodak_35mm();
            }
            if ui.button("🌌 Sci-Fi アナモルフィック").clicked() {
                cfg = CinemaFinishingConfig::preset_scifi_anamorphic();
            }
            if ui.button("📋 代理店レビュー TC バーンイン").clicked() {
                cfg = CinemaFinishingConfig::preset_agency_review_burnin();
            }
        });
        ui.separator();

        ui.checkbox(&mut cfg.enabled, "シネマフィニッシングパスを有効化");
        ui.add(
            egui::Slider::new(&mut cfg.halation_strength, 0.0..=1.5).text("35mm ハレーション強度"),
        );
        ui.add(
            egui::Slider::new(&mut cfg.halation_threshold, 0.35..=0.98)
                .text("ハレーション発火しきい値"),
        );
        ui.add(
            egui::Slider::new(&mut cfg.anamorphic_flare, 0.0..=1.5)
                .text("アナモルフィック水平フレア"),
        );
        ui.add(egui::Slider::new(&mut cfg.film_grain, 0.0..=0.45).text("35mm フィルムグレイン"));
        ui.add(egui::Slider::new(&mut cfg.vignette, 0.0..=1.0).text("周辺ビネット"));
        egui::ComboBox::from_label("シネマレターボックス")
            .selected_text(cfg.letterbox.display_name())
            .show_ui(ui, |ui| {
                for &lb in LetterboxAspect::ALL {
                    ui.selectable_value(&mut cfg.letterbox, lb, lb.display_name());
                }
            });
        ui.checkbox(
            &mut cfg.burn_in_timecode,
            "SMPTE タイムコードを映像へバーンイン",
        );
        ui.horizontal(|ui| {
            ui.label("透かしバーンイン文字列:");
            ui.text_edit_singleline(&mut cfg.watermark_text);
        });

        if cfg != cur_cfg
            && let Ok(mut guard) = session.lock()
        {
            let s = &mut *guard;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetCinemaFinishing { config: cfg },
                    CommandSource::Gui,
                    Some("v50_finishing"),
                )
                .is_ok()
            {
                self.finishing_status = "シネマフィニッシング設定を更新しました".into();
                mutated = true;
            }
        }

        ui.separator();
        ui.heading("📦 業界標準プロ交換フォーマット書き出し (CMX 3600 EDL / FCPXML v1.10)");
        ui.horizontal(|ui| {
            if ui.button("📄 CMX 3600 EDL を書き出し").clicked()
                && let Ok(s) = session.lock()
            {
                let edl = export_cmx3600_edl(&s.project);
                self.interchange_preview = edl.clone();
                let out_path = s
                    .folder
                    .as_ref()
                    .map(|f| f.root().join("exports").join("timeline_cmx3600.edl"))
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_timeline_cmx3600.edl"));
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&out_path, &edl) {
                    Ok(()) => {
                        self.finishing_status =
                            format!("CMX 3600 EDL を保存しました: {}", out_path.display());
                    }
                    Err(e) => {
                        self.finishing_status = format!("EDL 保存エラー: {e}");
                    }
                }
            }
            if ui
                .button("🎬 Final Cut Pro XML (FCPXML v1.10) を書き出し")
                .clicked()
                && let Ok(s) = session.lock()
            {
                let xml = export_fcpxml_v10(&s.project);
                self.interchange_preview = xml.clone();
                let out_path = s
                    .folder
                    .as_ref()
                    .map(|f| f.root().join("exports").join("timeline.fcpxml"))
                    .unwrap_or_else(|| std::env::temp_dir().join("ve_timeline.fcpxml"));
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&out_path, &xml) {
                    Ok(()) => {
                        self.finishing_status =
                            format!("FCPXML v1.10 を保存しました: {}", out_path.display());
                    }
                    Err(e) => {
                        self.finishing_status = format!("FCPXML 保存エラー: {e}");
                    }
                }
            }
        });
        if !self.interchange_preview.is_empty() {
            ui.add(
                egui::TextEdit::multiline(&mut self.interchange_preview)
                    .desired_rows(7)
                    .code_editor(),
            );
        }
        ui.colored_label(Color32::LIGHT_GREEN, &self.finishing_status);
        mutated
    }
}
