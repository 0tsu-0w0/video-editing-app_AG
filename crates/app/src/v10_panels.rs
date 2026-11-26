//! v1.0 統合プロダクションUIモジュール:
//! - #17 マルチカメラ編集スタジオ（音声相互相関自動同期・2×2/3×3グリッド・数字キー `1`〜`9` ライブスイッチング）
//! - #19 プロオーディオ トラックミキサー（パン/ソロ/バス/リバーブセンド・スペクトル減算NR・ディエッサー・ボーカル/BGM分離・マイク直接アフレコ録音）
//! - #19 プロジェクト検証チェックリスト ＆ Pure-Rust ZIP アーカイブパッケージ化
//! - #20 カスタムキーバインド（AviUtl2/Premiere/DaVinci/AE＋JSON入出力）・多言語(日/英)・テーマ/フォント倍率・HWデコード設定・SHA-256署名検証付き公式拡張レジストリブラウザ

use egui::{Color32, Context, RichText, Ui};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use ve_audio::{VoiceoverRecorder, estimate_audio_offset_xcorr, separate_vocal_bgm_stems};
use ve_core::{
    AssetRef, AssetResolution, AudioEffectNode, CommandSource, DeliveryReport, EditCommand,
    EditorAction, EditorKeymap, EditorUserSettings, Interpolation, IssueSeverity, ItemKind,
    Keyframe, KeymapPreset, MediaKind, MulticamCutPoint, MulticamGridLayout, RationalTime,
    UiLanguage, UiThemePreset, archive_project_to_zip, build_multicam_switch_cut_commands,
    build_multicam_sync_align_commands, official_registry_catalog, tr, validate_project_delivery,
    verify_and_install_extension,
};
use ve_mcp::EditorSession;
use ve_media::{
    FfmpegCli, HwDecodeBackend, ProxyCodec, ProxyProfile, decode_audio_file, write_wav_pcm16,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudioTab {
    Multicam,
    AudioMixer,
    DeliveryZip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    AppearanceAndKeymap,
    Performance,
    ExtensionRegistry,
}

pub struct V10PanelsState {
    pub show_studio_window: bool,
    pub studio_tab: StudioTab,
    pub show_settings_window: bool,
    pub settings_tab: SettingsTab,

    // #17 Multicam state
    pub multicam_grid_active: bool,
    pub multicam_layout: MulticamGridLayout,
    pub multicam_angle_layers: Vec<usize>,
    pub multicam_active_angle: usize,
    pub multicam_live_recording: bool,
    pub multicam_cuts: Vec<MulticamCutPoint>,
    pub multicam_output_layer: usize,
    pub multicam_status: String,

    // #19 Audio Mixer & Voiceover state
    pub voiceover: VoiceoverRecorder,
    pub voiceover_layer: usize,
    pub voiceover_start: RationalTime,
    pub audio_mixer_status: String,

    // #19 Delivery & ZIP Archive state
    pub delivery_report: Option<DeliveryReport>,
    pub delivery_status: String,
    pub archive_name_input: String,

    // #20 Settings, Keymap JSON & Official Extension Registry
    pub keymap_json_buf: String,
    pub settings_status: String,
    pub registry_status: String,
    pub proxy_profile: ProxyProfile,
    pub detected_hw_decoders: Vec<HwDecodeBackend>,
}

impl Default for V10PanelsState {
    fn default() -> Self {
        let default_km = EditorKeymap::for_preset(KeymapPreset::AviUtl2);
        let keymap_json_buf = default_km.to_json_pretty().unwrap_or_default();
        let detected_hw_decoders = FfmpegCli::default().detect_available_hw_decoders();
        Self {
            show_studio_window: false,
            studio_tab: StudioTab::Multicam,
            show_settings_window: false,
            settings_tab: SettingsTab::AppearanceAndKeymap,
            multicam_grid_active: false,
            multicam_layout: MulticamGridLayout::Grid2x2,
            multicam_angle_layers: vec![0, 1],
            multicam_active_angle: 0,
            multicam_live_recording: false,
            multicam_cuts: Vec::new(),
            multicam_output_layer: 2,
            multicam_status: String::new(),
            voiceover: VoiceoverRecorder::new(),
            voiceover_layer: 3,
            voiceover_start: RationalTime::ZERO,
            audio_mixer_status: String::new(),
            delivery_report: None,
            delivery_status: String::new(),
            archive_name_input: "project_release_v1".to_string(),
            keymap_json_buf,
            settings_status: String::new(),
            registry_status: String::new(),
            proxy_profile: ProxyProfile::default(),
            detected_hw_decoders,
        }
    }
}

impl V10PanelsState {
    /// プレビューペインでの数字キー `1`〜`9` 押下をマルチカムライブスイッチングとして処理する
    pub fn handle_multicam_hotkeys(&mut self, ctx: &Context, playhead: RationalTime) -> bool {
        if !self.multicam_grid_active || ctx.egui_wants_keyboard_input() {
            return false;
        }
        let max_angles = self
            .multicam_angle_layers
            .len()
            .min(self.multicam_layout.max_angles())
            .max(1);
        let keys = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        let pressed = ctx.input(|i| {
            if i.modifiers.command || i.modifiers.alt {
                return None;
            }
            for (idx, key) in keys.iter().enumerate() {
                if idx < max_angles && i.key_pressed(*key) {
                    return Some(idx);
                }
            }
            None
        });
        if let Some(angle) = pressed {
            self.multicam_active_angle = angle;
            if self.multicam_live_recording {
                self.multicam_cuts.push(MulticamCutPoint {
                    time: playhead,
                    active_angle: angle,
                });
                self.multicam_status = format!(
                    "🎬 CAM{} へ切替記録 ({:.2}s / 計 {} カット)",
                    angle + 1,
                    playhead.as_seconds(),
                    self.multicam_cuts.len()
                );
            }
            return true;
        }
        false
    }

    /// プレビュー上にマルチカム 2×2 / 3×3 ライブスイッチンググリッドオーバーレイを描画する
    pub fn draw_multicam_grid_overlay(
        &mut self,
        ui: &mut Ui,
        rect: egui::Rect,
        session: &EditorSession,
    ) {
        if !self.multicam_grid_active {
            return;
        }
        let (cols, rows) = self.multicam_layout.cols_rows();
        let painter = ui.painter();
        let cell_w = rect.width() / cols as f32;
        let cell_h = rect.height() / rows as f32;

        let scene = session.project.active_scene();
        let mut clicked_angle: Option<usize> = None;

        for idx in 0..(cols * rows) {
            let c = idx % cols;
            let r = idx / cols;
            let cell = egui::Rect::from_min_size(
                egui::Pos2::new(
                    rect.left() + c as f32 * cell_w,
                    rect.top() + r as f32 * cell_h,
                ),
                egui::Vec2::new(cell_w, cell_h),
            )
            .shrink(2.0);

            let is_active = idx == self.multicam_active_angle;
            let layer_opt = self.multicam_angle_layers.get(idx).copied();
            let layer_name = layer_opt
                .and_then(|li| scene.and_then(|s| s.layers.get(li)))
                .map(|l| l.name.as_str())
                .unwrap_or("(未割当)");

            let border_color = if is_active {
                if self.multicam_live_recording {
                    Color32::from_rgb(255, 65, 65)
                } else {
                    Color32::from_rgb(80, 230, 140)
                }
            } else {
                Color32::from_rgba_unmultiplied(180, 195, 220, 120)
            };
            painter.rect_stroke(
                cell,
                3.0,
                egui::Stroke::new(if is_active { 3.0 } else { 1.0 }, border_color),
                egui::StrokeKind::Inside,
            );
            // カメラ番号ラベルバッジ
            let badge_rect = egui::Rect::from_min_size(
                cell.left_top() + egui::Vec2::new(6.0, 6.0),
                egui::Vec2::new((cell.width() - 12.0).min(170.0), 20.0),
            );
            painter.rect_filled(
                badge_rect,
                3.0,
                if is_active {
                    Color32::from_rgba_unmultiplied(210, 45, 55, 225)
                } else {
                    Color32::from_rgba_unmultiplied(18, 22, 30, 190)
                },
            );
            painter.text(
                badge_rect.left_center() + egui::Vec2::new(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("[{}] CAM{} : {layer_name}", idx + 1, idx + 1),
                egui::FontId::proportional(11.0),
                Color32::WHITE,
            );

            if layer_opt.is_some()
                && ui.input(|i| {
                    i.pointer.primary_clicked()
                        && i.pointer.interact_pos().is_some_and(|p| cell.contains(p))
                })
            {
                clicked_angle = Some(idx);
            }
        }

        if let Some(angle) = clicked_angle {
            self.multicam_active_angle = angle;
            if self.multicam_live_recording {
                self.multicam_cuts.push(MulticamCutPoint {
                    time: session.playhead,
                    active_angle: angle,
                });
            }
        }
    }

    /// v1.0 プロダクションスタジオ統合ウィンドウ（マルチカム / トラックミキサー＆録音 / 検証＆ZIP）
    /// 戻り値: `true` ならプレビュー再描画が必要
    #[allow(dead_code)]
    pub fn show_studio_window(
        &mut self,
        ctx: &Context,
        session: &mut EditorSession,
        lang: UiLanguage,
    ) -> bool {
        if !self.show_studio_window {
            return false;
        }
        let mut dirty = false;
        let mut open = self.show_studio_window;
        let title = match lang {
            UiLanguage::Japanese => {
                "🎬 v1.0 プロダクションスタジオ (マルチカム / ミキサー / 納品検証)"
            }
            UiLanguage::English => "🎬 v1.0 Production Studio (Multicam / Mixer / Delivery)",
        };
        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .default_width(700.0)
            .default_height(490.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.studio_tab,
                        StudioTab::Multicam,
                        tr(lang, "multicam_title"),
                    );
                    ui.selectable_value(
                        &mut self.studio_tab,
                        StudioTab::AudioMixer,
                        tr(lang, "mixer_title"),
                    );
                    ui.selectable_value(
                        &mut self.studio_tab,
                        StudioTab::DeliveryZip,
                        tr(lang, "delivery_title"),
                    );
                });
                ui.separator();

                match self.studio_tab {
                    StudioTab::Multicam => {
                        if self.show_multicam_tab(ui, session) {
                            dirty = true;
                        }
                    }
                    StudioTab::AudioMixer => {
                        if self.show_audio_mixer_tab(ui, session) {
                            dirty = true;
                        }
                    }
                    StudioTab::DeliveryZip => {
                        self.show_delivery_zip_tab(ui, session);
                    }
                }
            });
        self.show_studio_window = open;
        dirty
    }

    pub fn show_multicam_tab(&mut self, ui: &mut Ui, session: &mut EditorSession) -> bool {
        let mut dirty = false;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.multicam_grid_active,
                "📺 プレビューにマルチカムグリッドを表示（数字キー 1〜9 で切替）",
            );
            egui::ComboBox::from_id_salt("multicam_grid_layout")
                .selected_text(match self.multicam_layout {
                    MulticamGridLayout::Grid2x2 => "2×2 (最大4カメラ)",
                    MulticamGridLayout::Grid3x3 => "3×3 (最大9カメラ)",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.multicam_layout,
                        MulticamGridLayout::Grid2x2,
                        "2×2 (最大4カメラ)",
                    );
                    ui.selectable_value(
                        &mut self.multicam_layout,
                        MulticamGridLayout::Grid3x3,
                        "3×3 (最大9カメラ)",
                    );
                });
        });

        let layer_names: Vec<String> = session
            .project
            .active_scene()
            .map(|s| s.layers.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default();

        ui.label(RichText::new("カメラアングルに割り当てるレイヤー:").strong());
        ui.horizontal_wrapped(|ui| {
            let mut set: BTreeSet<usize> = self.multicam_angle_layers.iter().copied().collect();
            for (li, name) in layer_names.iter().enumerate() {
                let mut checked = set.contains(&li);
                if ui
                    .checkbox(&mut checked, format!("L{li}: {name}"))
                    .changed()
                {
                    if checked {
                        set.insert(li);
                    } else {
                        set.remove(&li);
                    }
                }
            }
            self.multicam_angle_layers = set.into_iter().collect();
        });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("🎙 音声相互相関 (Cross-Correlation) で自動同期")
                .on_hover_text("CAM1 の音声波形を基準に他カメラのズレをサブフレーム精度で算出して一括整列します")
                .clicked()
            {
                dirty |= self.run_multicam_audio_sync(session);
            }

            let rec_btn = if self.multicam_live_recording {
                RichText::new("⏹ ライブ切替記録を停止").color(Color32::from_rgb(255, 110, 110))
            } else {
                RichText::new("🔴 ライブスイッチング記録開始 (1〜9)")
            };
            if ui.button(rec_btn).clicked() {
                self.multicam_live_recording = !self.multicam_live_recording;
                if self.multicam_live_recording {
                    self.multicam_grid_active = true;
                    self.multicam_cuts.push(MulticamCutPoint {
                        time: session.playhead,
                        active_angle: self.multicam_active_angle,
                    });
                    self.multicam_status =
                        "🔴 ライブ記録中: 再生しながら数字キー 1〜9 またはグリッドをクリックして切替".into();
                }
            }
            if ui.button("🗑 切替点クリア").clicked() {
                self.multicam_cuts.clear();
                self.multicam_status = "切替履歴をクリアしました".into();
            }
        });

        // 手動アングルボタン列
        if !self.multicam_angle_layers.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label("アクティブカメラ:");
                for idx in 0..self.multicam_angle_layers.len().min(9) {
                    let active = self.multicam_active_angle == idx;
                    if ui
                        .selectable_label(active, format!("CAM {} [{}]", idx + 1, idx + 1))
                        .clicked()
                    {
                        self.multicam_active_angle = idx;
                        if self.multicam_live_recording {
                            self.multicam_cuts.push(MulticamCutPoint {
                                time: session.playhead,
                                active_angle: idx,
                            });
                        }
                    }
                }
            });
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("切替結果の出力レイヤー:");
            let max_l = layer_names.len().saturating_sub(1);
            self.multicam_output_layer = self.multicam_output_layer.min(max_l);
            egui::ComboBox::from_id_salt("multicam_out_layer")
                .selected_text(format!(
                    "L{}: {}",
                    self.multicam_output_layer,
                    layer_names
                        .get(self.multicam_output_layer)
                        .map(String::as_str)
                        .unwrap_or("")
                ))
                .show_ui(ui, |ui| {
                    for (li, name) in layer_names.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.multicam_output_layer,
                            li,
                            format!("L{li}: {name}"),
                        );
                    }
                });

            if ui
                .add_enabled(
                    !self.multicam_angle_layers.is_empty(),
                    egui::Button::new("✅ タイムラインへ確定 (1-Undo)"),
                )
                .clicked()
            {
                let range_end = session
                    .project
                    .active_scene()
                    .map(|s| s.total_duration().max(RationalTime::from_seconds(1.0)))
                    .unwrap_or_else(|| RationalTime::from_seconds(5.0));
                if let Some(cmd) = session.project.active_scene().and_then(|s| {
                    build_multicam_switch_cut_commands(
                        s,
                        &self.multicam_angle_layers,
                        &self.multicam_cuts,
                        self.multicam_output_layer,
                        RationalTime::ZERO,
                        range_end,
                    )
                }) {
                    if session
                        .bus
                        .execute(&mut session.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                    {
                        self.multicam_status = format!(
                            "✅ {} 個の切替点を L{} へ書き出しました (Ctrl+Z 可)",
                            self.multicam_cuts.len().max(1),
                            self.multicam_output_layer
                        );
                        dirty = true;
                    }
                } else {
                    self.multicam_status = "出力対象のカメラクリップが見つかりません".into();
                }
            }
        });

        if !self.multicam_cuts.is_empty() {
            ui.label(format!("記録済み切替点 ({} 件):", self.multicam_cuts.len()));
            egui::ScrollArea::vertical()
                .max_height(110.0)
                .show(ui, |ui| {
                    for (i, c) in self.multicam_cuts.iter().enumerate() {
                        ui.monospace(format!(
                            "#{:02}  {:7.3}s  ->  CAM {}",
                            i + 1,
                            c.time.as_seconds(),
                            c.active_angle + 1
                        ));
                    }
                });
        }
        if !self.multicam_status.is_empty() {
            ui.colored_label(Color32::from_rgb(130, 220, 175), &self.multicam_status);
        }
        dirty
    }

    fn run_multicam_audio_sync(&mut self, session: &mut EditorSession) -> bool {
        if self.multicam_angle_layers.len() < 2 {
            self.multicam_status = "2 つ以上のカメラレイヤーを選択してください".into();
            return false;
        }
        let Some(scene) = session.project.active_scene() else {
            return false;
        };
        let mut clips: Vec<(usize, Uuid, Option<std::path::PathBuf>)> = Vec::new();
        for &li in &self.multicam_angle_layers {
            if let Some(layer) = scene.layers.get(li)
                && let Some(item) = layer.items.first()
            {
                let path = match &item.kind {
                    ItemKind::Video { asset_id, .. } | ItemKind::Audio { asset_id, .. } => {
                        session.project.asset(*asset_id).and_then(|a| {
                            if let Some(f) = session.folder.as_ref() {
                                match f.resolve_asset(a) {
                                    AssetResolution::Found(p) => Some(p),
                                    AssetResolution::Offline => None,
                                }
                            } else {
                                a.fallback_absolute_path.clone()
                            }
                        })
                    }
                    _ => None,
                };
                clips.push((li, item.id, path));
            }
        }
        if clips.len() < 2 {
            self.multicam_status = "選択レイヤー上に同期可能なクリップが 2 件未満です".into();
            return false;
        }
        let ref_buf = clips[0]
            .2
            .as_deref()
            .and_then(|p| decode_audio_file(p).ok());
        let mut offsets = vec![(clips[0].1, 0.0f64)];
        let mut summary = Vec::new();
        for (idx, (li, item_id, path_opt)) in clips.into_iter().enumerate().skip(1) {
            let est = match (
                ref_buf.as_ref(),
                path_opt.as_deref().and_then(|p| decode_audio_file(p).ok()),
            ) {
                (Some(rb), Some(tb)) => estimate_audio_offset_xcorr(rb, &tb, 10.0),
                _ => Default::default(),
            };
            offsets.push((item_id, est.offset_sec));
            summary.push(format!(
                "CAM{}(L{}): {:+.3}s (相関 {:.2})",
                idx + 1,
                li,
                est.offset_sec,
                est.correlation
            ));
        }
        if let Some(cmd) = session
            .project
            .active_scene()
            .and_then(|s| build_multicam_sync_align_commands(s, &offsets))
            && session
                .bus
                .execute(&mut session.project, cmd, CommandSource::Gui, None)
                .is_ok()
        {
            self.multicam_status = format!("✅ 自動同期完了: {}", summary.join(" / "));
            return true;
        }
        false
    }

    pub fn show_audio_mixer_tab(&mut self, ui: &mut Ui, session: &mut EditorSession) -> bool {
        let mut dirty = false;
        struct MixerStripSnapshot {
            idx: usize,
            name: String,
            volume: f32,
            pan: f32,
            solo: bool,
            muted: bool,
            bus: String,
            send_reverb: f32,
        }
        let layers_snapshot: Vec<MixerStripSnapshot> = session
            .project
            .active_scene()
            .map(|s| {
                s.layers
                    .iter()
                    .enumerate()
                    .map(|(idx, l)| MixerStripSnapshot {
                        idx,
                        name: l.name.clone(),
                        volume: l.volume,
                        pan: l.pan,
                        solo: l.solo,
                        muted: l.muted,
                        bus: l.bus.clone(),
                        send_reverb: l.send_reverb,
                    })
                    .collect()
            })
            .unwrap_or_default();

        ui.label(
            RichText::new(
                "🎛 トラックミキサー（フェーダー / パン / ソロ / バス / Aux リバーブセンド）",
            )
            .strong(),
        );
        let mut pending_mixer_cmd: Option<EditCommand> = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for strip in layers_snapshot {
                    let li = strip.idx;
                    let name = strip.name;
                    let mut vol = strip.volume;
                    let mut pan = strip.pan;
                    let mut solo = strip.solo;
                    let mut muted = strip.muted;
                    let mut bus = strip.bus;
                    let mut send_rev = strip.send_reverb;
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(128.0);
                        ui.label(RichText::new(format!("L{li} {name}")).strong().size(11.5));
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(
                                    solo,
                                    RichText::new("S").color(if solo {
                                        Color32::YELLOW
                                    } else {
                                        Color32::GRAY
                                    }),
                                )
                                .on_hover_text("ソロ (このレイヤーのみ出力)")
                                .clicked()
                            {
                                solo = !solo;
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    muted,
                                    RichText::new("M").color(if muted {
                                        Color32::from_rgb(255, 95, 95)
                                    } else {
                                        Color32::GRAY
                                    }),
                                )
                                .on_hover_text("ミュート")
                                .clicked()
                            {
                                muted = !muted;
                                changed = true;
                            }
                            egui::ComboBox::from_id_salt(format!("mixer_bus_{li}"))
                                .width(64.0)
                                .selected_text(&bus)
                                .show_ui(ui, |ui| {
                                    for b in ["Master", "Voice", "BGM", "SFX"] {
                                        if ui.selectable_value(&mut bus, b.to_string(), b).changed()
                                        {
                                            changed = true;
                                        }
                                    }
                                });
                        });
                        if ui
                            .add(
                                egui::Slider::new(&mut vol, 0.0..=2.0)
                                    .text("Vol")
                                    .fixed_decimals(2),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        if ui
                            .add(
                                egui::Slider::new(&mut pan, -1.0..=1.0)
                                    .text("Pan")
                                    .fixed_decimals(2),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        if ui
                            .add(
                                egui::Slider::new(&mut send_rev, 0.0..=1.0)
                                    .text("Rev")
                                    .fixed_decimals(2),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        if changed {
                            pending_mixer_cmd = Some(EditCommand::SetLayerMixer {
                                layer_index: li,
                                volume: vol,
                                pan,
                                solo,
                                muted,
                                bus,
                                send_reverb: send_rev,
                            });
                        }
                    });
                }
            });
        });

        if let Some(cmd) = pending_mixer_cmd
            && session
                .bus
                .execute(&mut session.project, cmd, CommandSource::Gui, None)
                .is_ok()
        {
            dirty = true;
        }

        ui.separator();
        // 選択中音声クリップの高度なポストプロダクション
        ui.label(
            RichText::new("✨ 選択クリップ 音声ポストプロダクション（Pure Rust オフライン処理）")
                .strong(),
        );
        let selected_id = session.selected_items.first().copied();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    selected_id.is_some(),
                    egui::Button::new("🔇 スペクトル減算ノイズ除去を追加"),
                )
                .clicked()
                && let Some(item_id) = selected_id
                && session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddAudioEffect {
                            item_id: Some(item_id),
                            effect: AudioEffectNode::with_defaults("noise_reduction"),
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
            {
                self.audio_mixer_status =
                    "✅ スペクトル減算ノイズリダクションを追加しました".into();
                dirty = true;
            }
            if ui
                .add_enabled(
                    selected_id.is_some(),
                    egui::Button::new("🐍 ディエッサー (歯擦音除去) を追加"),
                )
                .clicked()
                && let Some(item_id) = selected_id
                && session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::AddAudioEffect {
                            item_id: Some(item_id),
                            effect: AudioEffectNode::with_defaults("de_esser"),
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
            {
                self.audio_mixer_status = "✅ ディエッサーを追加しました".into();
                dirty = true;
            }
            if ui
                .add_enabled(
                    selected_id.is_some(),
                    egui::Button::new("📈 現在位置に Vol/Pan オートメーション点"),
                )
                .clicked()
                && let Some(item_id) = selected_id
                && let Some(item_start) = session.project.active_scene().and_then(|s| {
                    s.layers
                        .iter()
                        .flat_map(|l| l.items.iter())
                        .find(|i| i.id == item_id)
                        .map(|i| i.start)
                })
            {
                let local_t = (session.playhead - item_start).max(RationalTime::ZERO);
                let batch = EditCommand::Batch {
                    label: "add_audio_automation_kf".into(),
                    commands: vec![
                        EditCommand::SetKeyframe {
                            item_id,
                            property: "volume".into(),
                            keyframe: Keyframe {
                                time: local_t,
                                value: 1.0,
                                interpolation: Interpolation::Linear,
                            },
                        },
                        EditCommand::SetKeyframe {
                            item_id,
                            property: "pan".into(),
                            keyframe: Keyframe {
                                time: local_t,
                                value: 0.0,
                                interpolation: Interpolation::Linear,
                            },
                        },
                    ],
                };
                if session
                    .bus
                    .execute(&mut session.project, batch, CommandSource::Gui, None)
                    .is_ok()
                {
                    self.audio_mixer_status = format!(
                        "📈 {:.2}s に Volume/Pan オートメーション点を追加",
                        local_t.as_seconds()
                    );
                    dirty = true;
                }
            }
            if ui
                .add_enabled(
                    selected_id.is_some(),
                    egui::Button::new("🎼 ボーカル / BGM オフライン分離"),
                )
                .clicked()
                && let Some(item_id) = selected_id
            {
                dirty |= self.run_stem_separation_on_item(session, item_id);
            }
        });

        ui.separator();
        // マイク直接アフレコ録音
        ui.label(RichText::new("🎙 マイク直接アフレコ録音（タイムラインへ自動配置）").strong());
        ui.horizontal_wrapped(|ui| {
            let max_l = session
                .project
                .active_scene()
                .map(|s| s.layers.len().saturating_sub(1))
                .unwrap_or(0);
            self.voiceover_layer = self.voiceover_layer.min(max_l);
            ui.label("配置レイヤー:");
            ui.add(egui::DragValue::new(&mut self.voiceover_layer).range(0..=max_l));

            if !self.voiceover.is_recording() {
                if ui.button("🔴 マイク録音開始").clicked() {
                    self.voiceover_start = session.playhead;
                    match self.voiceover.start_live_mic() {
                        Ok(()) => {
                            self.audio_mixer_status = "🔴 マイク録音中...".into();
                        }
                        Err(e) => {
                            self.audio_mixer_status = format!("⚠ マイク起動不可: {e}");
                        }
                    }
                }
            } else {
                let peak = self.voiceover.current_peak_level();
                let dur = self.voiceover.recorded_duration_sec();
                ui.add(
                    egui::ProgressBar::new(peak)
                        .desired_width(120.0)
                        .text(format!("REC {dur:.1}s")),
                );
                if ui.button("⏹ 停止してタイムラインへ追加").clicked() {
                    let ts = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let filename = format!("voiceover_{ts}.wav");
                    let out_path = session
                        .folder
                        .as_ref()
                        .map(|f| f.assets_dir().join(&filename))
                        .unwrap_or_else(|| std::env::temp_dir().join(&filename));
                    match self.voiceover.stop_and_save_wav(&out_path) {
                        Ok(rec_dur) => {
                            let asset_id = Uuid::new_v4();
                            session.project.assets.push(AssetRef {
                                id: asset_id,
                                name: filename.clone(),
                                kind: MediaKind::Audio,
                                relative_path: session
                                    .folder
                                    .as_ref()
                                    .map(|_| std::path::PathBuf::from("assets").join(&filename)),
                                fallback_absolute_path: Some(out_path),
                                collected: session.folder.is_some(),
                            });
                            let clip = ve_core::TimelineItem::new(
                                format!("🎙 {filename}"),
                                self.voiceover_start,
                                RationalTime::from_seconds(rec_dur.max(0.2)),
                                ItemKind::Audio {
                                    asset_id,
                                    source_offset: RationalTime::ZERO,
                                    volume: 1.0,
                                },
                            );
                            let _ = session.bus.execute(
                                &mut session.project,
                                EditCommand::AddObject {
                                    layer_index: self.voiceover_layer,
                                    item: clip,
                                },
                                CommandSource::Gui,
                                None,
                            );
                            self.audio_mixer_status =
                                format!("✅ アフレコ保存＆配置完了 ({rec_dur:.2}s)");
                            dirty = true;
                        }
                        Err(e) => {
                            self.audio_mixer_status = format!("❌ 録音保存失敗: {e}");
                        }
                    }
                }
                if ui.button("破棄").clicked() {
                    self.voiceover.cancel();
                    self.audio_mixer_status = "録音をキャンセルしました".into();
                }
            }
        });

        if !self.audio_mixer_status.is_empty() {
            ui.colored_label(Color32::from_rgb(130, 220, 175), &self.audio_mixer_status);
        }
        dirty
    }

    fn run_stem_separation_on_item(&mut self, session: &mut EditorSession, item_id: Uuid) -> bool {
        let found = session.project.active_scene().and_then(|s| {
            s.layers
                .iter()
                .flat_map(|l| l.items.iter())
                .find(|i| i.id == item_id)
                .and_then(|it| match &it.kind {
                    ItemKind::Audio { asset_id, .. } | ItemKind::Video { asset_id, .. } => {
                        Some((*asset_id, it.start, it.duration))
                    }
                    _ => None,
                })
        });
        let Some((asset_id, start, duration)) = found else {
            self.audio_mixer_status = "音声/動画クリップを選択してください".into();
            return false;
        };
        let Some(asset) = session.project.asset(asset_id).cloned() else {
            return false;
        };
        let path_opt = if let Some(f) = session.folder.as_ref() {
            match f.resolve_asset(&asset) {
                AssetResolution::Found(p) => Some(p),
                AssetResolution::Offline => None,
            }
        } else {
            asset.fallback_absolute_path.clone()
        };
        let Some(src_path) = path_opt else {
            self.audio_mixer_status = "対象素材ファイルがオフラインです".into();
            return false;
        };
        let Ok(buf) = decode_audio_file(&src_path) else {
            self.audio_mixer_status = "音声デコードに失敗しました".into();
            return false;
        };
        let stems = separate_vocal_bgm_stems(&buf);
        let out_dir = session
            .folder
            .as_ref()
            .map(|f| f.assets_dir())
            .unwrap_or_else(|| std::env::temp_dir().join("ve_stems"));
        let _ = std::fs::create_dir_all(&out_dir);
        let base: String = asset
            .name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .take(28)
            .collect();
        let vocal_name = format!("{base}_vocals.wav");
        let bgm_name = format!("{base}_bgm.wav");
        let vocal_path = out_dir.join(&vocal_name);
        let bgm_path = out_dir.join(&bgm_name);
        if write_wav_pcm16(
            &vocal_path,
            &stems.vocals.samples,
            stems.vocals.sample_rate,
            stems.vocals.channels as u16,
        )
        .is_err()
            || write_wav_pcm16(
                &bgm_path,
                &stems.accompaniment.samples,
                stems.accompaniment.sample_rate,
                stems.accompaniment.channels as u16,
            )
            .is_err()
        {
            self.audio_mixer_status = "分離 WAV の書き出しに失敗しました".into();
            return false;
        }
        let v_id = Uuid::new_v4();
        let b_id = Uuid::new_v4();
        session.project.assets.push(AssetRef {
            id: v_id,
            name: vocal_name.clone(),
            kind: MediaKind::Audio,
            relative_path: session
                .folder
                .as_ref()
                .map(|_| std::path::PathBuf::from("assets").join(&vocal_name)),
            fallback_absolute_path: Some(vocal_path),
            collected: session.folder.is_some(),
        });
        session.project.assets.push(AssetRef {
            id: b_id,
            name: bgm_name.clone(),
            kind: MediaKind::Audio,
            relative_path: session
                .folder
                .as_ref()
                .map(|_| std::path::PathBuf::from("assets").join(&bgm_name)),
            fallback_absolute_path: Some(bgm_path),
            collected: session.folder.is_some(),
        });
        let max_l = session
            .project
            .active_scene()
            .map(|s| s.layers.len().saturating_sub(1))
            .unwrap_or(0);
        let batch = EditCommand::Batch {
            label: "separate_vocal_bgm_stems".into(),
            commands: vec![
                EditCommand::AddObject {
                    layer_index: 3.min(max_l),
                    item: ve_core::TimelineItem::new(
                        format!("🎤 {vocal_name}"),
                        start,
                        duration,
                        ItemKind::Audio {
                            asset_id: v_id,
                            source_offset: RationalTime::ZERO,
                            volume: 1.0,
                        },
                    ),
                },
                EditCommand::AddObject {
                    layer_index: 2.min(max_l),
                    item: ve_core::TimelineItem::new(
                        format!("🎵 {bgm_name}"),
                        start,
                        duration,
                        ItemKind::Audio {
                            asset_id: b_id,
                            source_offset: RationalTime::ZERO,
                            volume: 1.0,
                        },
                    ),
                },
            ],
        };
        let _ = session
            .bus
            .execute(&mut session.project, batch, CommandSource::Gui, None);
        self.audio_mixer_status =
            "✅ ボーカル / BGM を分離しタイムラインへ配置しました (Ctrl+Z 可)".into();
        true
    }

    pub fn show_delivery_zip_tab(&mut self, ui: &mut Ui, session: &mut EditorSession) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("🔍 書き出し前検証チェックリストを実行").clicked() {
                let rep = validate_project_delivery(&session.project, session.folder.as_ref());
                self.delivery_status = if rep.passed {
                    format!(
                        "✅ 書き出し準備完了 (エラー 0 / 警告 {} / 情報 {})",
                        rep.warning_count, rep.info_count
                    )
                } else {
                    format!("⚠ {} 件のエラーを検出しました（修正推奨）", rep.error_count)
                };
                self.delivery_report = Some(rep);
            }
            ui.separator();
            ui.label("ZIP名:");
            ui.add(egui::TextEdit::singleline(&mut self.archive_name_input).desired_width(150.0));
            if ui.button("📦 Pure-Rust ZIP アーカイブ出力").clicked() {
                if let Some(folder) = session.folder.as_ref() {
                    let safe: String = self
                        .archive_name_input
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                        .take(48)
                        .collect();
                    let stem = if safe.is_empty() {
                        "project_archive".to_string()
                    } else {
                        safe
                    };
                    let out_dir = folder.exports_dir();
                    let _ = std::fs::create_dir_all(&out_dir);
                    let zip_path = out_dir.join(format!("{stem}.zip"));
                    match archive_project_to_zip(folder, &zip_path) {
                        Ok(sum) => {
                            self.delivery_status = format!(
                                "✅ ZIP パッケージ作成完了: {} ({} ファイル / {:.1} KB)",
                                sum.zip_path.display(),
                                sum.file_count,
                                sum.total_bytes as f64 / 1024.0
                            );
                        }
                        Err(e) => {
                            self.delivery_status = format!("❌ ZIP 作成失敗: {e}");
                        }
                    }
                } else {
                    self.delivery_status =
                        "⚠ 先にプロジェクトフォルダを保存（💾 保存）してください".into();
                }
            }
        });

        if !self.delivery_status.is_empty() {
            ui.colored_label(Color32::from_rgb(130, 220, 175), &self.delivery_status);
        }

        ui.separator();
        if let Some(ref rep) = self.delivery_report {
            if rep.issues.is_empty() {
                ui.colored_label(
                    Color32::from_rgb(100, 230, 150),
                    "🎉 問題は見つかりませんでした。そのまま安全に書き出し・受け渡し可能です。",
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for issue in &rep.issues {
                            let (icon, col) = match issue.severity {
                                IssueSeverity::Error => {
                                    ("❌ ERROR", Color32::from_rgb(255, 100, 100))
                                }
                                IssueSeverity::Warning => {
                                    ("⚠ WARN", Color32::from_rgb(255, 205, 90))
                                }
                                IssueSeverity::Info => ("ℹ INFO", Color32::from_rgb(130, 195, 255)),
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(col, RichText::new(icon).strong());
                                ui.monospace(&issue.category);
                                ui.label(&issue.message);
                                if let Some(iid) = issue.item_id
                                    && ui.small_button("選択").clicked()
                                {
                                    session.selected_items = vec![iid];
                                }
                            });
                        }
                    });
            }
        } else {
            ui.weak("「書き出し前検証チェックリストを実行」を押すと、オフライン素材・クリップ重複・音量クリッピング・式構文・奇数解像度を一括診断します。");
        }
    }

    /// v1.0 #20: 環境設定・カスタムキーバインド・パフォーマンス・公式 SHA-256 拡張レジストリウィンドウ
    /// 戻り値: `(settings_changed, extension_installed)`
    pub fn show_settings_and_registry_window(
        &mut self,
        ctx: &Context,
        settings: &mut EditorUserSettings,
        hw_decode: &mut HwDecodeBackend,
        disk_cache_stats: Option<(usize, u64, u64, u64)>,
        session: &EditorSession,
    ) -> (bool, bool) {
        if !self.show_settings_window {
            return (false, false);
        }
        let mut settings_changed = false;
        let mut extension_installed = false;
        let mut open = self.show_settings_window;

        let title = match settings.language {
            UiLanguage::Japanese => "⚙ v1.0 環境設定・キーバインド・公式拡張レジストリ",
            UiLanguage::English => "⚙ v1.0 Settings, Keymap & Official Extension Registry",
        };

        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .default_width(720.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::AppearanceAndKeymap,
                        tr(settings.language, "settings_title"),
                    );
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::Performance,
                        "⚡ パフォーマンス / HWデコード",
                    );
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::ExtensionRegistry,
                        tr(settings.language, "registry_title"),
                    );
                });
                ui.separator();

                match self.settings_tab {
                    SettingsTab::AppearanceAndKeymap => {
                        // 言語・テーマ・フォント倍率
                        ui.horizontal(|ui| {
                            ui.label("UI Language / 表示言語:");
                            if ui
                                .selectable_value(
                                    &mut settings.language,
                                    UiLanguage::Japanese,
                                    "日本語 (ja)",
                                )
                                .changed()
                            {
                                settings_changed = true;
                            }
                            if ui
                                .selectable_value(
                                    &mut settings.language,
                                    UiLanguage::English,
                                    "English (en)",
                                )
                                .changed()
                            {
                                settings_changed = true;
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label("テーマ / Theme:");
                            for theme in [
                                UiThemePreset::DarkStudio,
                                UiThemePreset::MidnightOled,
                                UiThemePreset::HighContrast,
                                UiThemePreset::LightDaylight,
                            ] {
                                if ui
                                    .selectable_value(
                                        &mut settings.theme,
                                        theme,
                                        theme.display_name(settings.language),
                                    )
                                    .changed()
                                {
                                    settings_changed = true;
                                }
                            }
                        });
                        if ui
                            .add(
                                egui::Slider::new(&mut settings.font_scale, 0.85..=1.45)
                                    .text("フォント倍率 (アクセシビリティ)")
                                    .fixed_decimals(2),
                            )
                            .changed()
                        {
                            settings_changed = true;
                        }

                        ui.separator();
                        ui.label(
                            RichText::new("⌨ カスタムキーバインド プリセット (Premiere / DaVinci / AviUtl2 / AE)")
                                .strong(),
                        );
                        ui.horizontal_wrapped(|ui| {
                            for &preset in KeymapPreset::ALL {
                                if ui
                                    .selectable_label(
                                        settings.keymap.preset == preset,
                                        preset.display_name(),
                                    )
                                    .clicked()
                                {
                                    settings.keymap = EditorKeymap::for_preset(preset);
                                    self.keymap_json_buf =
                                        settings.keymap.to_json_pretty().unwrap_or_default();
                                    self.settings_status = format!(
                                        "キーバインドを「{}」へ切替えました",
                                        preset.display_name()
                                    );
                                    settings_changed = true;
                                }
                            }
                        });

                        egui::Grid::new("keymap_preview_grid")
                            .num_columns(2)
                            .striped(true)
                            .show(ui, |ui| {
                                for &action in EditorAction::ALL {
                                    ui.label(format!("{action:?}"));
                                    let entry = settings
                                        .keymap
                                        .bindings
                                        .entry(action)
                                        .or_default();
                                    if ui
                                        .add(egui::TextEdit::singleline(entry).desired_width(140.0))
                                        .changed()
                                    {
                                        settings_changed = true;
                                    }
                                    ui.end_row();
                                }
                            });

                        ui.horizontal(|ui| {
                            if ui.button("📤 JSON へ書き出し").clicked() {
                                self.keymap_json_buf =
                                    settings.keymap.to_json_pretty().unwrap_or_default();
                                self.settings_status = "キーバインドを JSON へ出力しました".into();
                            }
                            if ui.button("📥 JSON から読込").clicked() {
                                match EditorKeymap::from_json(&self.keymap_json_buf) {
                                    Ok(km) => {
                                        settings.keymap = km;
                                        settings_changed = true;
                                        self.settings_status =
                                            "✅ JSON キーバインドを読み込みました".into();
                                    }
                                    Err(e) => {
                                        self.settings_status = format!("❌ JSON 解析エラー: {e}");
                                    }
                                }
                            }
                            if ui.button("💾 設定を保存").clicked() {
                                if let Some(f) = session.folder.as_ref() {
                                    match f.save_user_settings(settings) {
                                        Ok(()) => {
                                            self.settings_status = format!(
                                                "✅ 設定保存: {}",
                                                f.user_settings_path().display()
                                            );
                                        }
                                        Err(e) => {
                                            self.settings_status = format!("❌ 保存失敗: {e}");
                                        }
                                    }
                                } else {
                                    self.settings_status =
                                        "設定をメモリへ反映しました（プロジェクト保存後に settings.json へ永続化）"
                                            .into();
                                }
                            }
                        });
                        egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.keymap_json_buf)
                                    .code_editor()
                                    .desired_width(f32::INFINITY),
                            );
                        });
                        if !self.settings_status.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(130, 220, 175),
                                &self.settings_status,
                            );
                        }
                    }
                    SettingsTab::Performance => {
                        ui.label(RichText::new("🚀 ハードウェアデコード (HW Decode) & 並列書き出し").strong());
                        ui.horizontal(|ui| {
                            ui.label("HW デコーダ:");
                            egui::ComboBox::from_id_salt("v10_hw_decode_combo")
                                .selected_text(hw_decode.display_name())
                                .show_ui(ui, |ui| {
                                    for b in [
                                        HwDecodeBackend::Auto,
                                        HwDecodeBackend::Software,
                                        HwDecodeBackend::D3d11va,
                                        HwDecodeBackend::Dxva2,
                                        HwDecodeBackend::Vaapi,
                                        HwDecodeBackend::VideoToolbox,
                                    ] {
                                        if ui
                                            .selectable_value(hw_decode, b, b.display_name())
                                            .changed()
                                        {
                                            settings_changed = true;
                                        }
                                    }
                                });
                        });
                        let det_names: Vec<&str> = self
                            .detected_hw_decoders
                            .iter()
                            .map(|b| b.display_name())
                            .collect();
                        ui.weak(format!(
                            "この環境で検出された FFmpeg HW デコーダ: {}",
                            if det_names.is_empty() {
                                "なし (Software CPU 自動フォールバック)".to_string()
                            } else {
                                det_names.join(", ")
                            }
                        ));

                        ui.separator();
                        let mut threads = settings.export_worker_threads as u32;
                        if ui
                            .add(
                                egui::Slider::new(&mut threads, 0..=16)
                                    .text("並列書き出しワーカー数 (0=自動CPU数)"),
                            )
                            .changed()
                        {
                            settings.export_worker_threads = threads as usize;
                            settings_changed = true;
                        }
                        if ui
                            .add(
                                egui::Slider::new(&mut settings.disk_frame_cache_mib, 64..=4096)
                                    .text("SSD 永続フレームキャッシュ上限 (MiB / cache/frames/)"),
                            )
                            .changed()
                        {
                            settings_changed = true;
                        }
                        if let Some((entries, bytes, hits, misses)) = disk_cache_stats {
                            ui.monospace(format!(
                                "SSD cache/frames/: {entries} 枚 / {:.1} MiB | Hits={hits} Misses={misses}",
                                bytes as f64 / (1024.0 * 1024.0)
                            ));
                        } else {
                            ui.weak("プロジェクトフォルダを開くと cache/frames/ 永続キャッシュが自動有効化されます。");
                        }

                        ui.separator();
                        ui.label(RichText::new("🎞 バックグラウンドプロキシ生成プロファイル").strong());
                        ui.horizontal(|ui| {
                            ui.label("解像度:");
                            for h in [360u32, 540, 720, 1080] {
                                ui.selectable_value(
                                    &mut self.proxy_profile.max_height,
                                    h,
                                    format!("{h}p"),
                                );
                            }
                            ui.label("コーデック:");
                            ui.selectable_value(
                                &mut self.proxy_profile.codec,
                                ProxyCodec::H264Fast,
                                "H.264 UltraFast",
                            );
                            ui.selectable_value(
                                &mut self.proxy_profile.codec,
                                ProxyCodec::MjpegIntra,
                                "MJPEG Intra (高速シーク)",
                            );
                        });
                    }
                    SettingsTab::ExtensionRegistry => {
                        ui.label(
                            RichText::new(
                                "🔐 公式拡張レジストリ（全パッケージ SHA-256 署名検証・サンドボックス権限明示）",
                            )
                            .strong(),
                        );
                        let catalog = official_registry_catalog();
                        egui::ScrollArea::vertical().max_height(350.0).show(ui, |ui| {
                            for (entry, payload) in catalog {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&entry.name).strong());
                                        ui.weak(format!(
                                            "v{} | {} | {}",
                                            entry.version, entry.author, entry.license
                                        ));
                                    });
                                    ui.label(&entry.description);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.colored_label(
                                            Color32::from_rgb(120, 215, 255),
                                            format!("SHA-256: {}…", &entry.sha256[..16]),
                                        );
                                        for perm in &entry.permissions {
                                            ui.colored_label(
                                                Color32::from_rgb(160, 235, 165),
                                                format!("[{}]", perm.label()),
                                            );
                                        }
                                    });
                                    if ui
                                        .button("⬇ SHA-256 検証してインストール")
                                        .clicked()
                                    {
                                        if let Some(folder) = session.folder.as_ref() {
                                            match verify_and_install_extension(
                                                &entry,
                                                &payload,
                                                folder.root(),
                                            ) {
                                                Ok(installed) => {
                                                    self.registry_status = format!(
                                                        "✅ 署名検証OK・導入完了: {}",
                                                        installed.display()
                                                    );
                                                    extension_installed = true;
                                                }
                                                Err(e) => {
                                                    self.registry_status =
                                                        format!("❌ 検証/導入エラー: {e}");
                                                }
                                            }
                                        } else {
                                            self.registry_status =
                                                "⚠ 先にプロジェクトフォルダを保存してください".into();
                                        }
                                    }
                                });
                            }
                        });
                        if !self.registry_status.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(130, 220, 175),
                                &self.registry_status,
                            );
                        }
                    }
                }
            });

        self.show_settings_window = open;
        (settings_changed, extension_installed)
    }
}

/// v1.0 #20: `UiThemePreset` と `font_scale` を egui の `Context` へ即時反映する
pub fn apply_user_visuals(ctx: &Context, settings: &EditorUserSettings) {
    let egui_theme = match settings.theme {
        UiThemePreset::LightDaylight => egui::Theme::Light,
        _ => egui::Theme::Dark,
    };
    ctx.set_theme(egui_theme);

    let mut visuals = match settings.theme {
        UiThemePreset::DarkStudio => egui::Visuals::dark(),
        UiThemePreset::MidnightOled => {
            let mut v = egui::Visuals::dark();
            v.panel_fill = Color32::from_rgb(6, 7, 10);
            v.window_fill = Color32::from_rgb(10, 11, 15);
            v
        }
        UiThemePreset::HighContrast => {
            let mut v = egui::Visuals::dark();
            v.panel_fill = Color32::BLACK;
            v.window_fill = Color32::from_rgb(8, 8, 8);
            v.override_text_color = Some(Color32::from_rgb(255, 255, 240));
            v.selection.bg_fill = Color32::from_rgb(255, 200, 0);
            v
        }
        UiThemePreset::LightDaylight => egui::Visuals::light(),
    };
    visuals.window_corner_radius = egui::CornerRadius::same(6);

    let scale = settings.font_scale.clamp(0.85, 1.45);
    let mut style = (*ctx.style_of(egui_theme)).clone();
    style.visuals = visuals;
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(13.5 * scale),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(13.0 * scale),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(12.5 * scale),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(17.5 * scale),
    );
    ctx.set_style_of(egui_theme, style);
}
