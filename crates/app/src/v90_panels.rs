//! v9.0 #53〜#56 Live Broadcast, VTuber & Hardware Ecosystem スタジオパネル
//!
//! 既存の 6 スタジオ Hub（`Deliver` / `VfxMotion` / `AiCollab` / `Edit` Hub）＋ `Ctrl+K` パレットへ統合:
//! - #53 `ui_live_switcher_tab`: 2 バスライブスイッチャー＆RTMP/SRT/スティンガー/`.veproj` 同時生成
//! - #54 `ui_vtuber_mesh_tab`: VTuber 2D 三角メッシュ変形・Verlet 物理揺れ・UDP OSC / VMC 顔トラッキング
//! - #55 `ui_hardware_ctrl_tab`: OSC / MIDI-over-UDP / Stream Deck WebSocket ハードウェア連携ブリッジ
//! - #56 `ui_smart_dam_tab`: ローカル スマート DAM（代表色/BPM/LUFS 索引・類似ショット検索・一括自動再リンク）

use egui::{Color32, RichText, Ui, Vec2};
use std::path::PathBuf;
use ve_core::{
    CommandSource, DamAssetRecord, EditCommand, HardwareBridgeServer, HardwareControllerConfig,
    HardwareControllerPreset, HardwareDispatchOutcome, HardwareEvent, HardwareProtocol, ItemId,
    ItemKind, MediaKind, OscArg, RationalTime, SimilarShotHit, StingerKind, StreamProtocol,
    TimelineItem, VmcTrackFrame, VmcUdpReceiver, VtuberAvatarPreset, VtuberMeshConfig,
    build_live_switcher_veproj_commands, build_relink_assets_command,
    build_vtuber_vmc_bake_command, compute_file_head_fingerprint, encode_osc_message,
    evaluate_smart_bin, export_live_session_veproj_json, extract_visual_features_from_rgba8,
    find_similar_shots, map_hardware_event_to_outcome, relink_missing_assets_from_dir,
};
use ve_mcp::SharedEditorSession;
use ve_media::build_live_stream_ffmpeg_args;

/// v9.0 #53〜#56 Hub 内サブパネル状態
pub struct V90PanelsState {
    // --- #53 ライブスイッチャー ---
    pub live_stinger_layer: usize,
    pub live_note: String,
    pub live_status: String,

    // --- #54 VTuber 2D メッシュ & Verlet & VMC ---
    pub vtuber_cfg: VtuberMeshConfig,
    pub vtuber_layer: usize,
    pub vtuber_duration_sec: f32,
    pub vmc_receiver: Option<VmcUdpReceiver>,
    pub vmc_recorded_frames: Vec<VmcTrackFrame>,
    pub vtuber_status: String,

    // --- #55 OSC / MIDI / Stream Deck ハードウェアブリッジ ---
    pub hw_bridge: Option<HardwareBridgeServer>,
    pub hw_status: String,

    // --- #56 ローカル スマート DAM ---
    pub dam_selected_bin: Option<usize>,
    pub dam_tag_filter: String,
    pub dam_similar_hits: Vec<SimilarShotHit>,
    pub dam_relink_dir: String,
    pub dam_status: String,
}

impl Default for V90PanelsState {
    fn default() -> Self {
        Self {
            live_stinger_layer: 3,
            live_note: "Live Take".into(),
            live_status: "PGM/PVW 2 バス切替・RTMP/SRT 引数生成・.veproj NLE 同時生成を制御します"
                .into(),

            vtuber_cfg: VtuberMeshConfig::default(),
            vtuber_layer: 1,
            vtuber_duration_sec: 6.0,
            vmc_receiver: None,
            vmc_recorded_frames: Vec::new(),
            vtuber_status:
                "2D 三角メッシュ変形・Verlet 髪揺れ・UDP OSC / VMC (port 39539) 顔トラッキング対応"
                    .into(),

            hw_bridge: None,
            hw_status:
                "OSC (UDP) / MIDI-over-UDP / Stream Deck (RFC 6455 WebSocket) を CommandBus へ統合"
                    .into(),

            dam_selected_bin: None,
            dam_tag_filter: String::new(),
            dam_similar_hits: Vec::new(),
            dam_relink_dir: std::env::temp_dir().display().to_string(),
            dam_status: "5 色代表パレット・暖色度・16D 類似検索・SHA-256 指紋一括再リンク".into(),
        }
    }
}

impl V90PanelsState {
    // =========================================================================
    // #53: ライブスイッチャー ＆ NLE 同時収録（RTMP / SRT / スティンガー / .veproj）
    // =========================================================================
    pub fn ui_live_switcher_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        ui.heading("📡 ライブスイッチャー ＆ NLE 同時収録（RTMP / SRT / .veproj 同時生成） (#53)");
        ui.separator();

        let (mut sw, playhead_sec) = {
            let g = session.lock().unwrap();
            let s = g
                .project
                .active_scene()
                .map(|sc| sc.live_switcher.clone())
                .unwrap_or_default();
            (s, g.playhead.as_seconds())
        };
        let mut sw_changed = false;

        ui.horizontal_wrapped(|ui| {
            sw_changed |= ui
                .checkbox(&mut sw.enabled, RichText::new("スイッチャー有効").strong())
                .changed();
            sw_changed |= ui
                .checkbox(
                    &mut sw.recording_active,
                    RichText::new("🔴 同時収録 REC").color(Color32::from_rgb(255, 96, 96)),
                )
                .changed();
            sw_changed |= ui
                .checkbox(&mut sw.show_tally_hud, "タリー HUD 表示")
                .changed();
            sw_changed |= ui
                .checkbox(&mut sw.auto_journal_veproj, ".veproj ジャーナル自動記録")
                .changed();
        });

        // PGM / PVW 2 バス 放送スイッチャーバンク
        ui.group(|ui| {
            let mut clicked_pgm: Option<(usize, String)> = None;
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(Color32::from_rgb(255, 92, 92), "PGM (ON-AIR):");
                for ch in &sw.sources {
                    let active = sw.program_channel == ch.index;
                    let lbl = RichText::new(&ch.label).color(if active {
                        Color32::WHITE
                    } else {
                        Color32::LIGHT_GRAY
                    });
                    if ui
                        .add(egui::Button::new(lbl).fill(if active {
                            Color32::from_rgb(190, 36, 42)
                        } else {
                            Color32::from_rgb(44, 48, 58)
                        }))
                        .clicked()
                    {
                        clicked_pgm = Some((ch.index, ch.label.clone()));
                    }
                }
            });
            if let Some((idx, label)) = clicked_pgm {
                sw.enabled = true;
                sw.cut_to_channel(idx, playhead_sec);
                self.live_status = format!("✂ PGM を {label} へカット切替しました");
                sw_changed = true;
            }

            let mut clicked_pvw: Option<usize> = None;
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(Color32::from_rgb(84, 224, 128), "PVW (NEXT):  ");
                for ch in &sw.sources {
                    let active = sw.preview_channel == ch.index;
                    if ui
                        .add(egui::Button::new(&ch.label).fill(if active {
                            Color32::from_rgb(32, 138, 72)
                        } else {
                            Color32::from_rgb(44, 48, 58)
                        }))
                        .clicked()
                    {
                        clicked_pvw = Some(ch.index);
                    }
                }
            });
            if let Some(idx) = clicked_pvw {
                sw.preview_channel = idx;
                sw_changed = true;
            }

            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(RichText::new("✂ CUT (即切替)").strong())
                    .clicked()
                {
                    sw.enabled = true;
                    sw.trigger_take(playhead_sec, false, self.live_note.clone());
                    self.live_status = format!(
                        "✂ CUT 完了 (履歴 {} 件) → PGM CH{}",
                        sw.events.len(),
                        sw.program_channel + 1
                    );
                    sw_changed = true;
                }
                if ui
                    .button(
                        RichText::new("⚡ AUTO STINGER (遷移切替)")
                            .strong()
                            .color(Color32::from_rgb(110, 220, 255)),
                    )
                    .clicked()
                {
                    sw.enabled = true;
                    sw.trigger_take(playhead_sec, true, self.live_note.clone());
                    self.live_status = format!(
                        "⚡ STINGER 切替完了 ({}) → PGM CH{}",
                        sw.stinger.kind.display_name(),
                        sw.program_channel + 1
                    );
                    sw_changed = true;
                }
                ui.label("メモ:");
                ui.add(egui::TextEdit::singleline(&mut self.live_note).desired_width(120.0));
            });

            ui.horizontal_wrapped(|ui| {
                for &sk in StingerKind::ALL {
                    if ui
                        .selectable_label(sw.stinger.kind == sk, sk.display_name())
                        .clicked()
                    {
                        sw.stinger.kind = sk;
                        sw_changed = true;
                    }
                }
                sw_changed |= ui
                    .add(
                        egui::Slider::new(&mut sw.stinger.duration_sec, 0.15..=2.0)
                            .text("遷移尺 s"),
                    )
                    .changed();
            });
        });

        // RTMP / SRT 配信先設定
        ui.collapsing(
            RichText::new("🌐 RTMP / RTMPS / SRT 配信出力設定").strong(),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    for &proto in StreamProtocol::ALL {
                        if ui
                            .selectable_label(
                                sw.stream_target.protocol == proto,
                                proto.display_name(),
                            )
                            .clicked()
                        {
                            sw.stream_target.protocol = proto;
                            sw_changed = true;
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("配信 URL:");
                    sw_changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut sw.stream_target.endpoint_url)
                                .desired_width(280.0),
                        )
                        .changed();
                    sw_changed |= ui
                        .add(
                            egui::DragValue::new(&mut sw.stream_target.video_bitrate_kbps)
                                .prefix("映像: ")
                                .suffix(" kbps")
                                .range(500..=50_000),
                        )
                        .changed();
                    sw_changed |= ui
                        .add(
                            egui::DragValue::new(&mut sw.stream_target.srt_latency_ms)
                                .prefix("SRT 遅延: ")
                                .suffix(" ms")
                                .range(20..=4000),
                        )
                        .changed();
                });
                if ui.small_button("🔍 FFmpeg 配信引数を検証").clicked() {
                    match build_live_stream_ffmpeg_args(&sw.stream_target, "", None) {
                        Ok(argv) => {
                            self.live_status = format!(
                                "✅ FFmpeg 配信コマンド検証OK ({} 引数): {}",
                                argv.len(),
                                argv.last().cloned().unwrap_or_default()
                            );
                        }
                        Err(e) => self.live_status = format!("❌ URL/設定エラー: {e}"),
                    }
                }
            },
        );

        if sw_changed {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetLiveSwitcher {
                        live_switcher: sw.clone(),
                    },
                    CommandSource::Gui,
                    None,
                )
                .is_ok()
            {
                dirty = true;
            }
        }

        // NLE 同時生成＆ベイク
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("記録済みスイッチング履歴: {} 件", sw.events.len()));
            ui.add(
                egui::DragValue::new(&mut self.live_stinger_layer)
                    .prefix("スティンガー層: ")
                    .range(0..=15),
            );
            if ui
                .add_enabled(
                    !sw.events.is_empty(),
                    egui::Button::new("📌 NLE タイムラインへ一括ベイク (1-Undo)"),
                )
                .clicked()
            {
                let mut g = session.lock().unwrap();
                if let Some(cmd) =
                    build_live_switcher_veproj_commands(&g.project, &sw, self.live_stinger_layer)
                {
                    let s = &mut *g;
                    if s.bus
                        .execute(&mut s.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                    {
                        self.live_status = format!(
                            "✅ {} 件のライブ切替をタイムラインマーカー・遷移・カメラキーフレームへ反映しました",
                            sw.events.len()
                        );
                        dirty = true;
                    }
                }
            }
            if ui.button("💾 .veproj 同時生成エクスポート").clicked() {
                let (json_res, out_path) = {
                    let g = session.lock().unwrap();
                    let p = g
                        .folder
                        .as_ref()
                        .map(|f| f.exports_dir().join("live_session_master.veproj.json"))
                        .unwrap_or_else(|| {
                            std::env::temp_dir().join("ve_live_session_master.veproj.json")
                        });
                    (
                        export_live_session_veproj_json(&g.project, &sw, self.live_stinger_layer),
                        p,
                    )
                };
                match json_res {
                    Ok(body) => match std::fs::write(&out_path, &body) {
                        Ok(_) => {
                            self.live_status = format!(
                                "✅ 編集用 .veproj 書き出し完了 ({} bytes) → {}",
                                body.len(),
                                out_path.display()
                            );
                        }
                        Err(e) => self.live_status = format!("❌ 保存失敗: {e}"),
                    },
                    Err(e) => self.live_status = format!("❌ 生成失敗: {e}"),
                }
            }
        });
        ui.weak(&self.live_status);
        dirty
    }

    // =========================================================================
    // #54: VTuber 2D 三角メッシュ変形・Verlet 物理揺れ・UDP OSC / VMC 連携
    // =========================================================================
    pub fn ui_vtuber_mesh_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut dirty = false;
        ui.heading("🦊 VTuber 2D 三角メッシュ変形・Verlet 物理揺れ・UDP OSC / VMC 連携 (#54)");
        ui.separator();

        // VMC UDP レシーバが稼働中なら非ブロッキングで最新フレームを反映
        if let Some(ref mut rx) = self.vmc_receiver {
            let applied = rx.poll_into(&mut self.vtuber_cfg.blend_shapes);
            if applied > 0 {
                let playhead = session
                    .lock()
                    .map(|g| g.playhead)
                    .unwrap_or(RationalTime::ZERO);
                self.vmc_recorded_frames.push(VmcTrackFrame {
                    time: playhead,
                    shapes: self.vtuber_cfg.blend_shapes,
                });
                self.vtuber_status = format!(
                    "📡 VMC 受信中 (127.0.0.1:{}, 累計 {} パケット / 録画 {} フレーム)",
                    rx.bound_port,
                    rx.packets_received,
                    self.vmc_recorded_frames.len()
                );
                dirty = true;
            }
        }

        ui.horizontal_wrapped(|ui| {
            for &preset in VtuberAvatarPreset::ALL {
                if ui
                    .selectable_label(self.vtuber_cfg.preset == preset, preset.display_name())
                    .clicked()
                {
                    self.vtuber_cfg.preset = preset;
                }
            }
            ui.checkbox(&mut self.vtuber_cfg.show_wireframe, "三角メッシュ線表示");
            ui.checkbox(&mut self.vtuber_cfg.auto_idle_animate, "自動まばたき＆呼吸");
        });

        ui.group(|ui| {
            ui.label(RichText::new("😊 ブレンドシェイプ ＆ 擬似 3D 頭部姿勢").strong());
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.blend_shapes.head_yaw_deg, -35.0..=35.0)
                        .text("顔左右 Yaw°"),
                );
                ui.add(
                    egui::Slider::new(
                        &mut self.vtuber_cfg.blend_shapes.head_pitch_deg,
                        -25.0..=25.0,
                    )
                    .text("顔上下 Pitch°"),
                );
                ui.add(
                    egui::Slider::new(
                        &mut self.vtuber_cfg.blend_shapes.head_roll_deg,
                        -25.0..=25.0,
                    )
                    .text("傾き Roll°"),
                );
            });
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.blend_shapes.mouth_open, 0.0..=1.0)
                        .text("口開き A"),
                );
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.blend_shapes.mouth_smile, 0.0..=1.0)
                        .text("スマイル"),
                );
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.blend_shapes.blink_l, 0.0..=1.0)
                        .text("左目パチ"),
                );
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.blend_shapes.blink_r, 0.0..=1.0)
                        .text("右目パチ"),
                );
            });
        });

        ui.group(|ui| {
            ui.label(RichText::new("💨 Verlet 距離拘束 物理揺れ（髪・衣装）").strong());
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.vtuber_cfg.physics.enabled, "Verlet 物理有効");
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.physics.wind_amp, 0.0..=0.8)
                        .text("風ゆれ振幅"),
                );
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.physics.stiffness, 0.1..=1.0)
                        .text("復元剛性"),
                );
                ui.add(
                    egui::Slider::new(&mut self.vtuber_cfg.physics.damping, 0.5..=0.98)
                        .text("減衰"),
                );
            });
        });

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.vtuber_layer)
                    .prefix("レイヤー: ")
                    .range(0..=15),
            );
            ui.add(
                egui::DragValue::new(&mut self.vtuber_duration_sec)
                    .prefix("尺: ")
                    .suffix("s")
                    .range(0.5..=120.0),
            );
            if ui.button("➕ VTuber アバター追加 (1-Undo)").clicked() {
                let mut g = session.lock().unwrap();
                let playhead = g.playhead;
                let (cx, cy) = g
                    .project
                    .active_scene()
                    .map(|s| (s.width as f32 * 0.5, s.height as f32 * 0.5))
                    .unwrap_or((960.0, 540.0));
                let mut item = TimelineItem::new(
                    "VTuber Avatar",
                    playhead,
                    RationalTime::from_seconds(f64::from(self.vtuber_duration_sec)),
                    ItemKind::VtuberAvatar {
                        config: Box::new(self.vtuber_cfg.clone()),
                    },
                );
                item.transform.x = cx;
                item.transform.y = cy;
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::AddObject {
                            layer_index: self.vtuber_layer,
                            item,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.vtuber_status = "✅ VTuber 2D 三角メッシュアバターを配置しました".into();
                    dirty = true;
                }
            }

            if let Some(item_id) = selected_item
                && ui.button("🔄 選択クリップへ設定適用 (1-Undo)").clicked()
            {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetVtuberAvatar {
                            item_id,
                            config: Box::new(self.vtuber_cfg.clone()),
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.vtuber_status =
                        "✅ VTuber ブレンドシェイプ＆物理設定を更新しました".into();
                    dirty = true;
                }
            }
        });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let listening = self.vmc_receiver.is_some();
            if ui
                .button(if listening {
                    "⏹ VMC UDP 受信停止"
                } else {
                    "📡 VMC UDP 受信開始 (port 39539)"
                })
                .clicked()
            {
                if listening {
                    self.vmc_receiver = None;
                    self.vtuber_status = "VMC UDP 受信を停止しました".into();
                } else {
                    match VmcUdpReceiver::bind_localhost(self.vtuber_cfg.vmc_udp_port)
                        .or_else(|_| VmcUdpReceiver::bind_localhost(0))
                    {
                        Ok(rx) => {
                            self.vtuber_status = format!(
                                "✅ VMC OSC-over-UDP 受信待機中: 127.0.0.1:{}",
                                rx.bound_port
                            );
                            self.vmc_receiver = Some(rx);
                        }
                        Err(e) => self.vtuber_status = format!("❌ UDP バインド失敗: {e}"),
                    }
                }
            }

            if let Some(ref rx) = self.vmc_receiver
                && ui.small_button("🧪 VMC ループバック送信テスト").clicked()
                && let Ok(sock) = std::net::UdpSocket::bind(("127.0.0.1", 0))
            {
                let pkt = encode_osc_message(
                    "/VMC/Ext/Blend/Val",
                    &[OscArg::String("A".into()), OscArg::Float(0.85)],
                );
                let _ = sock.send_to(&pkt, ("127.0.0.1", rx.bound_port));
            }

            if let Some(item_id) = selected_item
                && !self.vmc_recorded_frames.is_empty()
                && ui
                    .button(format!(
                        "🎞 VMC 録画 {}f をキーフレームベイク (1-Undo)",
                        self.vmc_recorded_frames.len()
                    ))
                    .clicked()
            {
                let mut g = session.lock().unwrap();
                let item_start = g
                    .project
                    .active_scene()
                    .and_then(|s| {
                        s.layers
                            .iter()
                            .flat_map(|l| l.items.iter())
                            .find(|i| i.id == item_id)
                            .map(|i| i.start)
                    })
                    .unwrap_or(RationalTime::ZERO);
                if let Some(cmd) =
                    build_vtuber_vmc_bake_command(item_id, item_start, &self.vmc_recorded_frames)
                {
                    let s = &mut *g;
                    if s.bus
                        .execute(&mut s.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                    {
                        self.vtuber_status = format!(
                            "✅ VMC {} フレームを 6 表情キーフレームへ焼き込みました",
                            self.vmc_recorded_frames.len()
                        );
                        self.vmc_recorded_frames.clear();
                        dirty = true;
                    }
                }
            }
        });
        ui.weak(&self.vtuber_status);
        dirty
    }

    // =========================================================================
    // #55: OSC / MIDI-over-UDP / Stream Deck WebSocket ハードウェアコントローラー連携
    // =========================================================================
    pub fn ui_hardware_ctrl_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        ui.heading("🎛 OSC / MIDI-over-UDP / Stream Deck WebSocket ハードウェア連携 (#55)");
        ui.separator();

        // 稼働中の 3 プロトコル統合ブリッジからイベントを非ブロッキングでドレイン＆実行
        if let Some(ref mut srv) = self.hw_bridge {
            let incoming = srv.poll_events();
            if !incoming.is_empty() {
                let mut g = session.lock().unwrap();
                let cfg = g.project.hardware_ctrl.clone();
                let playhead = g.playhead;
                let mut executed = 0usize;
                for ev in &incoming {
                    if let Some(HardwareDispatchOutcome::EditCommand(cmd)) =
                        map_hardware_event_to_outcome(&cfg, ev, &g.project, playhead)
                    {
                        let s = &mut *g;
                        if s.bus
                            .execute(&mut s.project, *cmd, CommandSource::Gui, None)
                            .is_ok()
                        {
                            executed += 1;
                            dirty = true;
                        }
                    }
                }
                self.hw_status = format!(
                    "📡 受信: {} 件 (CommandBus 適用 {} 件 / 累計 {} 件)",
                    incoming.len(),
                    executed,
                    srv.events_received
                );
            }
        }

        let cfg = {
            let g = session.lock().unwrap();
            g.project.hardware_ctrl.clone()
        };

        ui.horizontal_wrapped(|ui| {
            ui.label("プリセット適用:");
            for &preset in HardwareControllerPreset::ALL {
                if ui.small_button(preset.display_name()).clicked() {
                    let mut next = HardwareControllerConfig::from_preset(preset);
                    next.enabled = true;
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetHardwareController {
                                hardware_ctrl: next,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.hw_status =
                            format!("✅ プリセット '{}' を適用しました", preset.display_name());
                        dirty = true;
                    }
                }
            }
        });

        ui.horizontal_wrapped(|ui| {
            let running = self.hw_bridge.is_some();
            if ui
                .button(if running {
                    "⏹ 3 プロトコルブリッジ停止"
                } else {
                    "🚀 OSC + MIDI + Stream Deck WebSocket ブリッジ起動"
                })
                .clicked()
            {
                if running {
                    self.hw_bridge = None;
                    self.hw_status = "ハードウェアブリッジを停止しました".into();
                } else {
                    match HardwareBridgeServer::bind_localhost(
                        cfg.osc_udp_port,
                        cfg.midi_udp_port,
                        cfg.stream_deck_ws_port,
                    )
                    .or_else(|_| HardwareBridgeServer::bind_localhost(0, 0, 0))
                    {
                        Ok(srv) => {
                            self.hw_status = format!(
                                "✅ ブリッジ稼働中 | OSC UDP :{} | MIDI UDP :{} | Stream Deck WS :{}",
                                srv.osc_port, srv.midi_port, srv.ws_port
                            );
                            self.hw_bridge = Some(srv);
                        }
                        Err(e) => self.hw_status = format!("❌ ブリッジ起動失敗: {e}"),
                    }
                }
            }

            // 1 クリック ループバックテスト
            if ui.small_button("📡 OSC 露出テスト (+1.0)").clicked() {
                let ev = HardwareEvent {
                    protocol: HardwareProtocol::OscUdp,
                    pattern: "/ve/color/exposure".into(),
                    value_01: 0.75,
                };
                let mut g = session.lock().unwrap();
                if let Some(HardwareDispatchOutcome::EditCommand(cmd)) =
                    map_hardware_event_to_outcome(&cfg, &ev, &g.project, g.playhead)
                {
                    let s = &mut *g;
                    if s.bus
                        .execute(&mut s.project, *cmd, CommandSource::Gui, None)
                        .is_ok()
                    {
                        self.hw_status = "✅ OSC /ve/color/exposure を適用しました".into();
                        dirty = true;
                    }
                }
            }
            if ui.small_button("🎛 Stream Deck CAM 2 切替テスト").clicked() {
                let ev = HardwareEvent {
                    protocol: HardwareProtocol::StreamDeckWs,
                    pattern: "deck:cam_2".into(),
                    value_01: 1.0,
                };
                let mut g = session.lock().unwrap();
                if let Some(HardwareDispatchOutcome::EditCommand(cmd)) =
                    map_hardware_event_to_outcome(&cfg, &ev, &g.project, g.playhead)
                {
                    let s = &mut *g;
                    if s.bus
                        .execute(&mut s.project, *cmd, CommandSource::Gui, None)
                        .is_ok()
                    {
                        self.hw_status = "✅ Stream Deck deck:cam_2 を実行しました".into();
                        dirty = true;
                    }
                }
            }
        });

        ui.separator();
        ui.label(
            RichText::new(format!(
                "登録バインディング一覧（{} 件）",
                cfg.bindings.len()
            ))
            .strong(),
        );
        egui::Grid::new("v90_hw_bindings_grid")
            .striped(true)
            .show(ui, |ui| {
                ui.strong("プロトコル");
                ui.strong("パターン");
                ui.strong("名称");
                ui.strong("出力範囲");
                ui.end_row();
                for b in &cfg.bindings {
                    ui.label(b.protocol.display_name());
                    ui.monospace(&b.match_pattern);
                    ui.label(&b.name);
                    ui.monospace(format!("{:.1}..{:.1}", b.min_out, b.max_out));
                    ui.end_row();
                }
            });
        ui.weak(&self.hw_status);
        dirty
    }

    // =========================================================================
    // #56: ローカル スマート DAM（代表色/BPM/LUFS 索引・類似ショット検索・一括自動再リンク）
    // =========================================================================
    pub fn ui_smart_dam_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        ui.heading("🗂 ローカル スマート DAM（代表色/BPM/LUFS 索引・類似ショット検索・一括自動再リンク） (#56)");
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            if ui.button("📥 プロジェクト素材を索引化 (SHA-256)").clicked() {
                let mut g = session.lock().unwrap();
                let mut dam = g.project.smart_dam.clone();
                let mut cnt = 0usize;
                for asset in &g.project.assets {
                    let p_opt = asset.fallback_absolute_path.as_deref();
                    let (sz, hash) = p_opt
                        .and_then(|p| compute_file_head_fingerprint(p).ok())
                        .unwrap_or((0, String::new()));
                    dam.upsert_record(DamAssetRecord {
                        asset_id: Some(asset.id),
                        file_path: p_opt
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| asset.name.clone()),
                        file_name: asset.name.clone(),
                        kind: asset.kind,
                        file_size_bytes: sz,
                        head_sha256: hash,
                        dominant_colors: Vec::new(),
                        avg_luma: 0.5,
                        avg_saturation: 0.3,
                        warmth_score: 0.0,
                        edge_energy: 0.2,
                        feature_vec: [0.25; 16],
                        bpm: None,
                        integrated_lufs: None,
                        duration_sec: None,
                        tags: Vec::new(),
                    });
                    cnt += 1;
                }
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetSmartDamIndex { smart_dam: dam },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.dam_status = format!("✅ {cnt} 件のアセットを Smart DAM へ索引化しました");
                    dirty = true;
                }
            }

            if ui
                .small_button("🌅 サンプル夕景/夜景/BGM レコード投入")
                .clicked()
            {
                let mut g = session.lock().unwrap();
                let mut dam = g.project.smart_dam.clone();
                let samples = [
                    (
                        "sunset_coast.mp4",
                        [246u8, 128, 44],
                        MediaKind::Video,
                        None,
                        None,
                        "sunset",
                    ),
                    (
                        "tokyo_blue_night.mp4",
                        [32u8, 92, 232],
                        MediaKind::Video,
                        None,
                        None,
                        "night",
                    ),
                    (
                        "upbeat_bgm_124.wav",
                        [140u8, 220, 180],
                        MediaKind::Audio,
                        Some(124.0),
                        Some(-14.2),
                        "bgm",
                    ),
                ];
                for (name, rgb, kind, bpm, lufs, tag) in samples {
                    let mut rgba = vec![0u8; 16 * 16 * 4];
                    for px in rgba.as_chunks_mut::<4>().0 {
                        *px = [rgb[0], rgb[1], rgb[2], 255];
                    }
                    let (dom, luma, sat, warmth, edge, feat) =
                        extract_visual_features_from_rgba8(16, 16, &rgba);
                    dam.upsert_record(DamAssetRecord {
                        asset_id: None,
                        file_path: format!("/media/{name}"),
                        file_name: name.into(),
                        kind,
                        file_size_bytes: 0,
                        head_sha256: String::new(),
                        dominant_colors: dom,
                        avg_luma: luma,
                        avg_saturation: sat,
                        warmth_score: warmth,
                        edge_energy: edge,
                        feature_vec: feat,
                        bpm,
                        integrated_lufs: lufs,
                        duration_sec: Some(10.0),
                        tags: vec![tag.into()],
                    });
                }
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetSmartDamIndex { smart_dam: dam },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    self.dam_status = "✅ 3 件のサンプル DAM レコードを登録しました".into();
                    dirty = true;
                }
            }
        });

        let dam = {
            let g = session.lock().unwrap();
            g.project.smart_dam.clone()
        };

        // スマートビン切替チップ + タグ／ファイル名フィルタ
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(
                    self.dam_selected_bin.is_none(),
                    format!("すべて ({})", dam.records.len()),
                )
                .clicked()
            {
                self.dam_selected_bin = None;
            }
            for (idx, bin) in dam.smart_bins.iter().enumerate() {
                let cnt = evaluate_smart_bin(&dam, bin).len();
                if ui
                    .selectable_label(
                        self.dam_selected_bin == Some(idx),
                        format!("{} ({cnt})", bin.name),
                    )
                    .clicked()
                {
                    self.dam_selected_bin = Some(idx);
                }
            }
            ui.label("🏷 絞込:");
            ui.add(
                egui::TextEdit::singleline(&mut self.dam_tag_filter)
                    .hint_text("ファイル名 / タグ")
                    .desired_width(130.0),
            );
        });

        let needle = self.dam_tag_filter.trim().to_ascii_lowercase();
        let visible: Vec<&DamAssetRecord> = if let Some(idx) = self.dam_selected_bin
            && let Some(rule) = dam.smart_bins.get(idx)
        {
            evaluate_smart_bin(&dam, rule)
        } else {
            dam.records.iter().collect()
        }
        .into_iter()
        .filter(|rec| {
            needle.is_empty()
                || rec.file_name.to_ascii_lowercase().contains(&needle)
                || rec
                    .tags
                    .iter()
                    .any(|t| t.to_ascii_lowercase().contains(&needle))
        })
        .collect();

        ui.separator();
        for rec in &visible {
            ui.horizontal_wrapped(|ui| {
                // 5 色代表パレットスウォッチを描画
                for sw in rec.dominant_colors.iter().take(5) {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(r, 2.0, Color32::from_rgb(sw.r, sw.g, sw.b));
                }
                ui.strong(&rec.file_name);
                ui.weak(format!("暖色:{:+.2}", rec.warmth_score));
                if let Some(bpm) = rec.bpm {
                    ui.monospace(format!("{bpm:.0}BPM"));
                }
                if let Some(lufs) = rec.integrated_lufs {
                    ui.monospace(format!("{lufs:.1}LUFS"));
                }
                if ui.small_button("🔍 類似ショット検索").clicked() {
                    self.dam_similar_hits =
                        find_similar_shots(&dam, &rec.feature_vec, Some(&rec.file_path), 5);
                    self.dam_status = format!(
                        "🔍 '{}' の類似ショット {} 件をコサイン類似度順で表示中",
                        rec.file_name,
                        self.dam_similar_hits.len()
                    );
                }
            });
        }

        if !self.dam_similar_hits.is_empty() {
            ui.group(|ui| {
                ui.label(
                    RichText::new("🎯 類似ショットランキング（16 次元コサイン類似度）").strong(),
                );
                for hit in &self.dam_similar_hits {
                    ui.monospace(format!("{:.1}%  {}", hit.similarity * 100.0, hit.file_name));
                }
            });
        }

        // 一括自動再リンク
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("再リンク探索フォルダ:");
            ui.add(egui::TextEdit::singleline(&mut self.dam_relink_dir).desired_width(260.0));
            if ui
                .button("🔗 行方不明素材を一括自動再リンク (SHA-256, 1-Undo)")
                .clicked()
            {
                let mut g = session.lock().unwrap();
                let plan = relink_missing_assets_from_dir(
                    &g.project,
                    &g.project.smart_dam,
                    &PathBuf::from(&self.dam_relink_dir),
                );
                if let Some(cmd) = build_relink_assets_command(&plan) {
                    let s = &mut *g;
                    if s.bus
                        .execute(&mut s.project, cmd, CommandSource::Gui, None)
                        .is_ok()
                    {
                        self.dam_status = format!(
                            "✅ 再リンク完了: {} / {} 件解決 (走査 {} ファイル)",
                            plan.matches.len(),
                            plan.missing_assets,
                            plan.scanned_files
                        );
                        dirty = true;
                    }
                } else {
                    self.dam_status = format!(
                        "リンク切れ: {} 件 / 走査 {} ファイル（一致候補なし）",
                        plan.missing_assets, plan.scanned_files
                    );
                }
            }
        });
        ui.weak(&self.dam_status);
        dirty
    }
}
