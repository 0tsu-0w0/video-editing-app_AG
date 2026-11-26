//! v8.0 #49〜#52 Pro Cinema VFX, Motion Foundry & Audio Restoration スタジオパネル
//!
//! 既存の 6 スタジオ Hub (`VfxMotion` Hub / `Audio` Hub) ＋ `Ctrl+K` パレットへ統合される 4 つの UI パネル:
//! - #49 `ui_planar_roto_tab`: 4 点平面ホモグラフィトラッカー＆エッジ吸着キーフレームロトブラシ
//! - #50 `ui_motion_foundry_tab`: SVG `<path>` インポート＆等弧長モーフィング・Text-on-Path・4×4 双三次グリッドワープ＆変位
//! - #51 `ui_optical_vfx_tab`: シネマ光学 VFX（擬似深度 Rack Focus ボケ・ゴッドレイ体積光・Brown-Conrady レンズ歪み＆色収差）
//! - #52 `ui_audio_restoration_adr_tab`: オーディオ修復 DSP・ADR 3-2-1 カウント収録・5.1ch / FOA Ambisonics 空間音声バス

use egui::{Color32, Pos2, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use ve_audio::{
    apply_adaptive_dereverb, apply_declick, apply_hum_removal, build_adr_cue_timeline_commands,
    decode_foa_ambix_to_binaural_stereo, downmix_51_to_stereo_itu, encode_stereo_to_51_surround,
    encode_stereo_to_foa_ambix, rotate_foa_ambix, synthesize_adr_preroll_beeps,
    write_multichannel_wav_file,
};
use ve_core::{
    AdrCue, AudioEffectNode, CommandSource, EditCommand, ItemId, ItemKind, OpticalVfxConfig,
    OpticalVfxPreset, PlanarTrackConfig, PseudoDepthMode, RationalTime, RgbaColor, RotoBrushConfig,
    RotoKeyframe, SpatialAudioBusMode, SvgMorphConfig, SvgShapePreset, TextOnPathConfig,
    TimelineItem, VectorFillMode, WarpDisplaceConfig, WarpPreset,
    build_planar_track_corner_pin_command, parse_svg_path_to_vertices,
};
use ve_mcp::SharedEditorSession;
use ve_render::{
    RenderGraph, propagate_roto_brush_in_scene, snap_roto_vertices_to_edges,
    track_planar_quad_in_scene,
};

/// v8.0 #49〜#52 Hub 内サブパネル状態
pub struct V80PanelsState {
    // --- #49 4点平面トラッカー & ロトブラシ ---
    pub planar_quad: [[f32; 2]; 4],
    pub planar_start_sec: f64,
    pub planar_end_sec: f64,
    pub planar_step_frames: u32,
    pub planar_feather_px: f32,
    pub planar_dragging_corner: Option<usize>,
    pub planar_status: String,

    pub roto_cx: f32,
    pub roto_cy: f32,
    pub roto_rx: f32,
    pub roto_ry: f32,
    pub roto_vertices: usize,
    pub roto_snap_radius: u32,
    pub roto_edge_weight: f32,
    pub roto_feather_px: f32,
    pub roto_expansion_px: f32,
    pub roto_invert: bool,
    pub roto_status: String,

    // --- #50 SVG モーフィング / Text-on-Path / グリッドワープ ---
    pub svg_morph_cfg: SvgMorphConfig,
    pub svg_layer_index: usize,
    pub svg_duration_sec: f32,
    pub svg_fill_rgb: [f32; 3],
    pub svg_stroke_rgb: [f32; 3],
    pub svg_status: String,

    pub text_path_cfg: TextOnPathConfig,
    pub warp_cfg: WarpDisplaceConfig,

    // --- #51 シネマ光学 VFX ---
    pub optical_status: String,

    // --- #52 オーディオ修復 & ADR & 5.1/FOA 空間バス ---
    pub restore_target_clip: bool,
    pub hum_base_hz: f32,
    pub hum_harmonics: usize,
    pub hum_q: f32,
    pub declick_sensitivity: f32,
    pub dereverb_strength: f32,
    pub dereverb_decay_ms: f32,
    pub restore_status: String,

    pub adr_character: String,
    pub adr_line_text: String,
    pub adr_start_sec: f64,
    pub adr_duration_sec: f64,
    pub adr_preroll_sec: f64,
    pub adr_subtitle_layer: usize,
    pub adr_status: String,

    pub spatial_status: String,
}

impl Default for V80PanelsState {
    fn default() -> Self {
        Self {
            planar_quad: PlanarTrackConfig::default().initial_quad,
            planar_start_sec: 0.0,
            planar_end_sec: 1.5,
            planar_step_frames: 2,
            planar_feather_px: 1.5,
            planar_dragging_corner: None,
            planar_status:
                "4隅ハンドルをドラッグして追跡平面を指定し、CornerPin へ 1-Undo ベイクできます"
                    .into(),

            roto_cx: 0.5,
            roto_cy: 0.5,
            roto_rx: 0.22,
            roto_ry: 0.28,
            roto_vertices: 10,
            roto_snap_radius: 12,
            roto_edge_weight: 0.78,
            roto_feather_px: 2.0,
            roto_expansion_px: 0.0,
            roto_invert: false,
            roto_status: "Sobel エッジ吸着＆オプティカルフロー時間軸伝播でロト切り抜きを作成"
                .into(),

            svg_morph_cfg: SvgMorphConfig::default(),
            svg_layer_index: 1,
            svg_duration_sec: 4.0,
            svg_fill_rgb: [0.22, 0.78, 1.0],
            svg_stroke_rgb: [1.0, 1.0, 1.0],
            svg_status: "SVG <path d> を貼り付けるかプリセットから等弧長モーフィングを作成".into(),

            text_path_cfg: TextOnPathConfig {
                enabled: true,
                ..TextOnPathConfig::default()
            },
            warp_cfg: WarpDisplaceConfig {
                enabled: true,
                ..WarpDisplaceConfig::default()
            },

            optical_status:
                "擬似深度ボケ (Rack Focus)・ゴッドレイ・Brown-Conrady レンズ歪み＆色収差を非破壊制御"
                    .into(),

            restore_target_clip: false,
            hum_base_hz: 50.0,
            hum_harmonics: 5,
            hum_q: 24.0,
            declick_sensitivity: 4.0,
            dereverb_strength: 0.65,
            dereverb_decay_ms: 160.0,
            restore_status:
                "50/60Hz 縦続ノッチ・MAD+Hermite デクリック・適応 De-Reverb を非破壊追加".into(),

            adr_character: "Actor A".into(),
            adr_line_text: "ここが勝負のシーンだ、行くぞ！".into(),
            adr_start_sec: 3.0,
            adr_duration_sec: 2.0,
            adr_preroll_sec: 3.0,
            adr_subtitle_layer: 2,
            adr_status: "ADR 3-2-1 映像ストリーマ＆同期ビープ付きアフレコ収録キューを管理".into(),

            spatial_status:
                "ITU-R BS.775 5.1ch サラウンド / FOA Ambisonics (AmbiX W,Y,Z,X) マスタリングバス"
                    .into(),
        }
    }
}

impl V80PanelsState {
    // =========================================================================
    // #49: 4 点平面トラッカー ＆ エッジ吸着キーフレームロトブラシ
    // =========================================================================
    pub fn ui_planar_roto_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
        render_graph: &mut RenderGraph,
    ) -> bool {
        let mut dirty = false;

        ui.heading("🎯 4 点平面トラッカー（Planar Homography）＆ エッジ吸着ロトブラシ (#49)");
        ui.separator();

        // --- Section A: 4 点平面トラッカー ---
        ui.collapsing(
            RichText::new("📐 4 点平面ホモグラフィ追跡 → CornerPin ベイク (ZNCC + RANSAC)")
                .strong(),
            |ui| {
                // Interactive 4-corner quad preview canvas
                let canvas_size = Vec2::new(320.0, 180.0);
                let (rect, response) = ui.allocate_exact_size(canvas_size, Sense::drag());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 24, 34));
                painter.rect_stroke(
                    rect,
                    4.0,
                    Stroke::new(1.0, Color32::from_rgb(72, 84, 110)),
                    StrokeKind::Inside,
                );

                // Subtle 3x3 guide grid
                for k in 1..3 {
                    let frac = k as f32 / 3.0;
                    let gx = rect.left() + rect.width() * frac;
                    let gy = rect.top() + rect.height() * frac;
                    painter.line_segment(
                        [Pos2::new(gx, rect.top()), Pos2::new(gx, rect.bottom())],
                        Stroke::new(0.6, Color32::from_rgba_unmultiplied(120, 140, 180, 45)),
                    );
                    painter.line_segment(
                        [Pos2::new(rect.left(), gy), Pos2::new(rect.right(), gy)],
                        Stroke::new(0.6, Color32::from_rgba_unmultiplied(120, 140, 180, 45)),
                    );
                }

                let screen_pts: [Pos2; 4] = std::array::from_fn(|i| {
                    Pos2::new(
                        rect.left() + self.planar_quad[i][0] * rect.width(),
                        rect.top() + self.planar_quad[i][1] * rect.height(),
                    )
                });

                // Drag corner handles
                if response.drag_started()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let mut best = None;
                    let mut best_d = 18.0_f32;
                    for (idx, p) in screen_pts.iter().enumerate() {
                        let d = p.distance(pos);
                        if d < best_d {
                            best_d = d;
                            best = Some(idx);
                        }
                    }
                    self.planar_dragging_corner = best;
                }
                if response.dragged()
                    && let (Some(idx), Some(pos)) =
                        (self.planar_dragging_corner, response.interact_pointer_pos())
                {
                    self.planar_quad[idx][0] =
                        ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.02, 0.98);
                    self.planar_quad[idx][1] =
                        ((pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.02, 0.98);
                }
                if response.drag_stopped() {
                    self.planar_dragging_corner = None;
                }

                // Draw polygon edges & corner badges
                let quad_stroke = Stroke::new(2.0, Color32::from_rgb(56, 212, 245));
                for i in 0..4 {
                    painter.line_segment([screen_pts[i], screen_pts[(i + 1) % 4]], quad_stroke);
                }
                const LABELS: [&str; 4] = ["TL", "TR", "BR", "BL"];
                for i in 0..4 {
                    painter.circle_filled(screen_pts[i], 6.0, Color32::from_rgb(255, 214, 72));
                    painter.text(
                        screen_pts[i] + Vec2::new(8.0, -8.0),
                        egui::Align2::LEFT_TOP,
                        LABELS[i],
                        egui::FontId::monospace(10.0),
                        Color32::WHITE,
                    );
                }

                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("中央 25%-75%").clicked() {
                        self.planar_quad = PlanarTrackConfig::default().initial_quad;
                    }
                    if ui.small_button("斜めモニター 16:9").clicked() {
                        self.planar_quad =
                            [[0.22, 0.24], [0.76, 0.18], [0.80, 0.78], [0.20, 0.72]];
                    }
                    ui.add(
                        egui::DragValue::new(&mut self.planar_start_sec)
                            .prefix("開始: ")
                            .suffix("s")
                            .speed(0.1)
                            .range(0.0..=300.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.planar_end_sec)
                            .prefix("終了: ")
                            .suffix("s")
                            .speed(0.1)
                            .range(0.1..=300.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.planar_step_frames)
                            .prefix("間隔: ")
                            .suffix("f")
                            .range(1..=15),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.planar_feather_px, 0.0..=16.0)
                            .text("フェザー px"),
                    );
                });

                ui.horizontal_wrapped(|ui| {
                    if ui.button("🎯 4 点平面トラッキング解析のみ").clicked() {
                        let (proj, active_id) = {
                            let g = session.lock().unwrap();
                            (g.project.clone(), g.project.active_scene_id)
                        };
                        let cfg = PlanarTrackConfig {
                            start_time: RationalTime::from_seconds(self.planar_start_sec),
                            end_time: RationalTime::from_seconds(
                                self.planar_end_sec.max(self.planar_start_sec + 0.05),
                            ),
                            step_frames: self.planar_step_frames.max(1),
                            initial_quad: self.planar_quad,
                            ..PlanarTrackConfig::default()
                        };
                        let res = track_planar_quad_in_scene(render_graph, &proj, active_id, &cfg);
                        if let Some(last) = res.frames.last() {
                            self.planar_quad = last.quad;
                        }
                        self.planar_status = format!(
                            "✅ 解析完了: {} フレーム | 平均インライア {:.1} | 信頼度 {:.2}",
                            res.frames.len(),
                            res.mean_inliers,
                            res.mean_confidence
                        );
                    }

                    let can_bake = selected_item.is_some();
                    if ui
                        .add_enabled(
                            can_bake,
                            egui::Button::new("📌 選択クリップへ CornerPin ベイク (1-Undo)"),
                        )
                        .clicked()
                        && let Some(item_id) = selected_item
                    {
                        let (proj, active_id, item_start) = {
                            let g = session.lock().unwrap();
                            let start = g
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
                            (g.project.clone(), g.project.active_scene_id, start)
                        };
                        let cfg = PlanarTrackConfig {
                            start_time: RationalTime::from_seconds(self.planar_start_sec),
                            end_time: RationalTime::from_seconds(
                                self.planar_end_sec.max(self.planar_start_sec + 0.05),
                            ),
                            step_frames: self.planar_step_frames.max(1),
                            initial_quad: self.planar_quad,
                            ..PlanarTrackConfig::default()
                        };
                        let res = track_planar_quad_in_scene(render_graph, &proj, active_id, &cfg);
                        if let Some(cmd) = build_planar_track_corner_pin_command(
                            item_id,
                            item_start,
                            &res,
                            self.planar_feather_px,
                        ) {
                            let mut g = session.lock().unwrap();
                            let s = &mut *g;
                            match s.bus.execute(&mut s.project, cmd, CommandSource::Gui, None) {
                                Ok(_) => {
                                    self.planar_status = format!(
                                        "✅ CornerPin キーフレーム ({} フレーム分) をベイクしました",
                                        res.frames.len()
                                    );
                                    dirty = true;
                                }
                                Err(e) => self.planar_status = format!("❌ ベイク失敗: {e}"),
                            }
                        }
                    }
                });
                ui.weak(&self.planar_status);
            },
        );

        ui.separator();

        // --- Section B: エッジ吸着キーフレームロトブラシ ---
        ui.collapsing(
            RichText::new("✂ エッジ吸着キーフレームロトブラシ（Sobel + Optical Flow 伝播）")
                .strong(),
            |ui| {
                if let Some(item_id) = selected_item {
                    let existing_summary = {
                        let g = session.lock().unwrap();
                        g.project
                            .active_scene()
                            .and_then(|s| {
                                s.layers
                                    .iter()
                                    .flat_map(|l| l.items.iter())
                                    .find(|i| i.id == item_id)
                                    .map(|i| {
                                        format!(
                                            "選択中: {} | 有効: {} | キーフレーム数: {}",
                                            i.name,
                                            if i.roto_brush.enabled { "ON" } else { "OFF" },
                                            i.roto_brush.keyframes.len()
                                        )
                                    })
                            })
                            .unwrap_or_default()
                    };
                    ui.colored_label(Color32::LIGHT_BLUE, existing_summary);

                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Slider::new(&mut self.roto_cx, 0.05..=0.95).text("中心 X"));
                        ui.add(egui::Slider::new(&mut self.roto_cy, 0.05..=0.95).text("中心 Y"));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Slider::new(&mut self.roto_rx, 0.05..=0.48).text("半径 X"));
                        ui.add(egui::Slider::new(&mut self.roto_ry, 0.05..=0.48).text("半径 Y"));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Slider::new(&mut self.roto_vertices, 4..=24).text("頂点数"));
                        ui.add(
                            egui::Slider::new(&mut self.roto_snap_radius, 2..=32)
                                .text("吸着半径 px"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.roto_edge_weight, 0.1..=1.0)
                                .text("勾配重み"),
                        );
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::Slider::new(&mut self.roto_feather_px, 0.0..=24.0)
                                .text("フェザー px"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.roto_expansion_px, -30.0..=30.0)
                                .text("輪郭膨張 px"),
                        );
                        ui.checkbox(&mut self.roto_invert, "マスク反転");
                    });

                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .button("🧲 現在フレームへ Sobel エッジ吸着＆適用 (1-Undo)")
                            .clicked()
                        {
                            let (proj, active_id, playhead, item_start) = {
                                let g = session.lock().unwrap();
                                let start = g
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
                                (
                                    g.project.clone(),
                                    g.project.active_scene_id,
                                    g.playhead,
                                    start,
                                )
                            };
                            let seed = RotoBrushConfig::default_ellipse_vertices(
                                self.roto_cx,
                                self.roto_cy,
                                self.roto_rx,
                                self.roto_ry,
                                self.roto_vertices,
                            );
                            let snapped = if let Some(frame) = render_graph
                                .render_project_scene_scaled(
                                    &proj, active_id, playhead, 320, 180, true,
                                ) {
                                snap_roto_vertices_to_edges(
                                    &frame,
                                    &seed,
                                    self.roto_snap_radius,
                                    self.roto_edge_weight,
                                )
                            } else {
                                seed
                            };
                            let kf = RotoKeyframe {
                                time: (playhead - item_start).max(RationalTime::ZERO),
                                vertices: snapped,
                                feather_px: self.roto_feather_px,
                                expansion_px: self.roto_expansion_px,
                            };
                            let cfg = RotoBrushConfig {
                                enabled: true,
                                invert: self.roto_invert,
                                edge_snap_radius_px: self.roto_snap_radius,
                                edge_weight: self.roto_edge_weight,
                                keyframes: vec![kf],
                                ..RotoBrushConfig::default()
                            };
                            let mut g = session.lock().unwrap();
                            let s = &mut *g;
                            if s.bus
                                .execute(
                                    &mut s.project,
                                    EditCommand::SetRotoBrush {
                                        item_id,
                                        roto_brush: cfg,
                                    },
                                    CommandSource::Gui,
                                    None,
                                )
                                .is_ok()
                            {
                                self.roto_status =
                                    "✅ Sobel 輪郭吸着キーフレームを 1 点適用しました".into();
                                dirty = true;
                            }
                        }

                        if ui
                            .button("🎞 時間軸自動伝播 (Optical Flow + Sobel, 1-Undo)")
                            .clicked()
                        {
                            let (proj, active_id, item_start) = {
                                let g = session.lock().unwrap();
                                let start = g
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
                                (g.project.clone(), g.project.active_scene_id, start)
                            };
                            let seed = RotoBrushConfig::default_ellipse_vertices(
                                self.roto_cx,
                                self.roto_cy,
                                self.roto_rx,
                                self.roto_ry,
                                self.roto_vertices,
                            );
                            let mut kfs = propagate_roto_brush_in_scene(
                                render_graph,
                                &proj,
                                active_id,
                                item_start,
                                &seed,
                                RationalTime::from_seconds(self.planar_start_sec),
                                RationalTime::from_seconds(
                                    self.planar_end_sec.max(self.planar_start_sec + 0.05),
                                ),
                                self.planar_step_frames.max(1),
                                self.roto_snap_radius,
                                self.roto_edge_weight,
                                self.roto_feather_px,
                            );
                            for kf in &mut kfs {
                                kf.expansion_px = self.roto_expansion_px;
                            }
                            let count = kfs.len();
                            let cfg = RotoBrushConfig {
                                enabled: true,
                                invert: self.roto_invert,
                                edge_snap_radius_px: self.roto_snap_radius,
                                edge_weight: self.roto_edge_weight,
                                keyframes: kfs,
                                ..RotoBrushConfig::default()
                            };
                            let mut g = session.lock().unwrap();
                            let s = &mut *g;
                            if s.bus
                                .execute(
                                    &mut s.project,
                                    EditCommand::SetRotoBrush {
                                        item_id,
                                        roto_brush: cfg,
                                    },
                                    CommandSource::Gui,
                                    None,
                                )
                                .is_ok()
                            {
                                self.roto_status =
                                    format!("✅ 時間軸ロト伝播完了: {count} キーフレーム生成");
                                dirty = true;
                            }
                        }

                        if ui.small_button("🧹 ロト無効化").clicked() {
                            let mut g = session.lock().unwrap();
                            let s = &mut *g;
                            if s.bus
                                .execute(
                                    &mut s.project,
                                    EditCommand::SetRotoBrush {
                                        item_id,
                                        roto_brush: RotoBrushConfig::default(),
                                    },
                                    CommandSource::Gui,
                                    None,
                                )
                                .is_ok()
                            {
                                self.roto_status = "ロトブラシを無効化しました".into();
                                dirty = true;
                            }
                        }
                    });
                } else {
                    ui.weak("タイムライン上のクリップを選択するとロトブラシを適用できます。");
                }
                ui.weak(&self.roto_status);
            },
        );

        dirty
    }

    // =========================================================================
    // #50: SVG <path> モーフィング・Text-on-Path・グリッドワープ＆変位
    // =========================================================================
    pub fn ui_motion_foundry_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut dirty = false;

        ui.heading("✒ SVG パスモーフィング・Text-on-Path・4×4 グリッドワープ (#50)");
        ui.separator();

        // --- Section A: SVG <path> インポート＆モーフィング ---
        ui.collapsing(
            RichText::new("🔷 SVG <path> インポート＆等弧長パスモーフィング").strong(),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("シェイプ A プリセット:");
                    for &p in SvgShapePreset::ALL {
                        if ui.small_button(p.display_name()).clicked() {
                            self.svg_morph_cfg.path_a_d = p.svg_path_d().to_string();
                        }
                    }
                });
                ui.add(
                    egui::TextEdit::multiline(&mut self.svg_morph_cfg.path_a_d)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("SVG <path d=\"M...\"> または <svg> XML"),
                );

                ui.horizontal_wrapped(|ui| {
                    ui.label("シェイプ B プリセット:");
                    for &p in SvgShapePreset::ALL {
                        if ui.small_button(p.display_name()).clicked() {
                            self.svg_morph_cfg.path_b_d = p.svg_path_d().to_string();
                        }
                    }
                });
                ui.add(
                    egui::TextEdit::multiline(&mut self.svg_morph_cfg.path_b_d)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("変形先 SVG <path d=\"...\">"),
                );

                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.svg_morph_cfg.morph_progress, 0.0..=1.0)
                            .text("モーフ進行度"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.svg_morph_cfg.auto_morph_hz, 0.0..=3.0)
                            .text("自動往復 Hz"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.svg_morph_cfg.width, 64.0..=1200.0)
                            .text("幅 px"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.svg_morph_cfg.height, 64.0..=1200.0)
                            .text("高さ px"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.svg_morph_cfg.stroke_width, 0.0..=24.0)
                            .text("線幅 px"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("塗り色:");
                    ui.color_edit_button_rgb(&mut self.svg_fill_rgb);
                    ui.label("線色:");
                    ui.color_edit_button_rgb(&mut self.svg_stroke_rgb);
                    ui.add(
                        egui::DragValue::new(&mut self.svg_layer_index)
                            .prefix("レイヤー: ")
                            .range(0..=15),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.svg_duration_sec)
                            .prefix("尺: ")
                            .suffix("s")
                            .speed(0.2)
                            .range(0.5..=60.0),
                    );
                });

                self.svg_morph_cfg.fill = VectorFillMode::Solid(RgbaColor {
                    r: self.svg_fill_rgb[0],
                    g: self.svg_fill_rgb[1],
                    b: self.svg_fill_rgb[2],
                    a: 1.0,
                });
                self.svg_morph_cfg.stroke_color = RgbaColor {
                    r: self.svg_stroke_rgb[0],
                    g: self.svg_stroke_rgb[1],
                    b: self.svg_stroke_rgb[2],
                    a: 1.0,
                };

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button("➕ SVG モーフィングクリップを追加 (1-Undo)")
                        .clicked()
                    {
                        match parse_svg_path_to_vertices(&self.svg_morph_cfg.path_a_d) {
                            Ok((va, _)) => {
                                let mut g = session.lock().unwrap();
                                let playhead = g.playhead;
                                let (cx, cy) = g
                                    .project
                                    .active_scene()
                                    .map(|s| (s.width as f32 * 0.5, s.height as f32 * 0.5))
                                    .unwrap_or((640.0, 360.0));
                                let mut item = TimelineItem::new(
                                    "SVG Morph",
                                    playhead,
                                    RationalTime::from_seconds(self.svg_duration_sec as f64),
                                    ItemKind::SvgMorph {
                                        config: Box::new(self.svg_morph_cfg.clone()),
                                    },
                                );
                                item.transform.x = cx;
                                item.transform.y = cy;
                                let s = &mut *g;
                                match s.bus.execute(
                                    &mut s.project,
                                    EditCommand::AddObject {
                                        layer_index: self.svg_layer_index,
                                        item,
                                    },
                                    CommandSource::Gui,
                                    None,
                                ) {
                                    Ok(_) => {
                                        self.svg_status = format!(
                                            "✅ SVG モーフィング追加完了 (Path A: {} 頂点)",
                                            va.len()
                                        );
                                        dirty = true;
                                    }
                                    Err(e) => self.svg_status = format!("❌ 追加失敗: {e}"),
                                }
                            }
                            Err(e) => self.svg_status = format!("❌ SVG パス解析エラー: {e}"),
                        }
                    }

                    if let Some(item_id) = selected_item {
                        let is_svg = {
                            let g = session.lock().unwrap();
                            g.project
                                .active_scene()
                                .and_then(|s| {
                                    s.layers
                                        .iter()
                                        .flat_map(|l| l.items.iter())
                                        .find(|i| i.id == item_id)
                                        .map(|i| matches!(i.kind, ItemKind::SvgMorph { .. }))
                                })
                                .unwrap_or(false)
                        };
                        if is_svg
                            && ui
                                .button("🔄 選択中 SVG モーフィングへ適用 (1-Undo)")
                                .clicked()
                        {
                            let mut g = session.lock().unwrap();
                            let s = &mut *g;
                            if s.bus
                                .execute(
                                    &mut s.project,
                                    EditCommand::SetSvgMorph {
                                        item_id,
                                        config: Box::new(self.svg_morph_cfg.clone()),
                                    },
                                    CommandSource::Gui,
                                    None,
                                )
                                .is_ok()
                            {
                                self.svg_status = "✅ SVG モーフィングを更新しました".into();
                                dirty = true;
                            }
                        }
                    }
                });
                ui.weak(&self.svg_status);
            },
        );

        ui.separator();

        // --- Section B: Text-on-Path ---
        ui.collapsing(
            RichText::new("✍ Text-on-Path（SVG 曲線沿い接線回転タイポグラフィ）").strong(),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.text_path_cfg.enabled, "有効化");
                    ui.checkbox(&mut self.text_path_cfg.follow_tangent, "接線回転追従");
                    ui.checkbox(&mut self.text_path_cfg.show_guide, "パスガイド表示");
                    for &p in SvgShapePreset::ALL {
                        if ui.small_button(p.display_name()).clicked() {
                            self.text_path_cfg.svg_path_d = p.svg_path_d().to_string();
                        }
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.text_path_cfg.svg_path_d)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.text_path_cfg.path_offset, 0.0..=1.0)
                            .text("開始位置"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.text_path_cfg.animate_speed, -1.0..=1.0)
                            .text("移動速度/s"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.text_path_cfg.letter_spacing_px, -6.0..=24.0)
                            .text("文字間隔 px"),
                    );
                });
                let can_apply = selected_item.is_some();
                if ui
                    .add_enabled(
                        can_apply,
                        egui::Button::new("✍ 選択テキストクリップへ適用 (1-Undo)"),
                    )
                    .clicked()
                    && let Some(item_id) = selected_item
                {
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetTextOnPath {
                                item_id,
                                text_on_path: self.text_path_cfg.clone(),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.svg_status = "✅ Text-on-Path を選択クリップへ適用しました".into();
                        dirty = true;
                    }
                }
            },
        );

        ui.separator();

        // --- Section C: 4×4 双三次グリッドワープ＆輝度ディスプレイスメント ---
        ui.collapsing(
            RichText::new("🌊 ディスプレイスメント ＆ 4×4 双三次グリッドワープ").strong(),
            |ui| {
                self.warp_cfg.ensure_grid_size();
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.warp_cfg.enabled, "ワープ有効");
                    for &preset in WarpPreset::ALL {
                        if ui
                            .selectable_label(self.warp_cfg.preset == preset, preset.display_name())
                            .clicked()
                        {
                            self.warp_cfg.preset = preset;
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Slider::new(&mut self.warp_cfg.amount, -1.0..=1.0).text("強度"));
                    ui.add(
                        egui::Slider::new(&mut self.warp_cfg.frequency, 0.5..=8.0).text("周波数"),
                    );
                    ui.add(egui::Slider::new(&mut self.warp_cfg.speed, 0.0..=5.0).text("速度 Hz"));
                    ui.add(
                        egui::Slider::new(&mut self.warp_cfg.luma_displace_px, 0.0..=40.0)
                            .text("輝度変位 px"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("🎈 グリッド中央を膨らませる").clicked() {
                        self.warp_cfg.preset = WarpPreset::CustomGrid;
                        self.warp_cfg.ensure_grid_size();
                        self.warp_cfg.control_offsets[5] = [-0.08, -0.08];
                        self.warp_cfg.control_offsets[6] = [0.08, -0.08];
                        self.warp_cfg.control_offsets[9] = [-0.08, 0.08];
                        self.warp_cfg.control_offsets[10] = [0.08, 0.08];
                    }
                    if ui.small_button("↺ グリッド変位リセット").clicked() {
                        self.warp_cfg.control_offsets.fill([0.0, 0.0]);
                    }
                });
                let can_apply = selected_item.is_some();
                if ui
                    .add_enabled(
                        can_apply,
                        egui::Button::new("🌊 選択クリップへワープ＆変位を適用 (1-Undo)"),
                    )
                    .clicked()
                    && let Some(item_id) = selected_item
                {
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetWarpDisplace {
                                item_id,
                                warp_displace: self.warp_cfg.clone(),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.svg_status = "✅ グリッドワープ＆変位を適用しました".into();
                        dirty = true;
                    }
                }
            },
        );

        dirty
    }

    // =========================================================================
    // #51: シネマ光学 VFX（Rack Focus ボケ・ゴッドレイ・Brown-Conrady レンズ補正＆色収差）
    // =========================================================================
    pub fn ui_optical_vfx_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;

        ui.heading(
            "🎥 シネマ光学 VFX（Rack Focus ボケ・ゴッドレイ・Brown-Conrady レンズ補正＆色収差） (#51)",
        );
        ui.separator();

        let mut cfg = {
            let g = session.lock().unwrap();
            g.project
                .active_scene()
                .map(|s| s.optical_vfx.clone())
                .unwrap_or_default()
        };
        let mut changed = false;

        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(
                    &mut cfg.enabled,
                    RichText::new("シネマ光学 VFX 有効").strong(),
                )
                .changed()
            {
                changed = true;
            }
            for &preset in OpticalVfxPreset::ALL {
                if ui.small_button(preset.display_name()).clicked() {
                    cfg = OpticalVfxConfig::from_preset(preset);
                    self.optical_status = format!("✅ プリセット適用: {}", preset.display_name());
                    changed = true;
                }
            }
            if ui.small_button("↺ すべて無効化").clicked() {
                cfg = OpticalVfxConfig::default();
                self.optical_status = "シネマ光学 VFX をリセットしました".into();
                changed = true;
            }
        });
        ui.separator();

        // 1. Rack Focus 擬似深度ボケ
        ui.group(|ui| {
            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut cfg.bokeh_enabled,
                        RichText::new("📷 擬似深度ボケ (Rack Focus)").strong(),
                    )
                    .changed()
                {
                    if cfg.bokeh_enabled {
                        cfg.enabled = true;
                    }
                    changed = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                for &dm in PseudoDepthMode::ALL {
                    if ui
                        .selectable_label(cfg.depth_mode == dm, dm.display_name())
                        .clicked()
                    {
                        cfg.depth_mode = dm;
                        changed = true;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.focus_depth, 0.0..=1.0).text("合焦深度"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.focus_band, 0.02..=0.5).text("被写界深度幅"))
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(
                        egui::Slider::new(&mut cfg.bokeh_radius_px, 0.0..=20.0).text("ボケ半径 px"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut cfg.bokeh_highlight_boost, 0.0..=3.0)
                            .text("玉ボケ強調"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.bokeh_blades, 0..=9).text("絞り羽根 (0=円)"))
                    .changed();
            });
        });

        // 2. ゴッドレイ体積光散乱
        ui.group(|ui| {
            if ui
                .checkbox(
                    &mut cfg.god_rays_enabled,
                    RichText::new("☀ ゴッドレイ（放射体積光散乱）").strong(),
                )
                .changed()
            {
                if cfg.god_rays_enabled {
                    cfg.enabled = true;
                }
                changed = true;
            }
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.light_pos[0], 0.0..=1.0).text("光源 X"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.light_pos[1], 0.0..=1.0).text("光源 Y"))
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.god_rays_intensity, 0.0..=2.0).text("光束強度"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.god_rays_decay, 0.80..=0.99).text("減衰率"))
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut cfg.god_rays_threshold, 0.1..=0.95).text("輝度閾値"),
                    )
                    .changed();
            });
        });

        // 3. Brown-Conrady レンズ歪み補正 ＆ 物理色収差
        ui.group(|ui| {
            if ui
                .checkbox(
                    &mut cfg.lens_enabled,
                    RichText::new("🔭 Brown-Conrady レンズ歪み補正 ＆ 物理色収差").strong(),
                )
                .changed()
            {
                if cfg.lens_enabled {
                    cfg.enabled = true;
                }
                changed = true;
            }
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.k1, -0.5..=0.5).text("k1 (放射2次)"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.k2, -0.3..=0.3).text("k2 (放射4次)"))
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.p1, -0.1..=0.1).text("p1 (接線X)"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut cfg.p2, -0.1..=0.1).text("p2 (接線Y)"))
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(
                        egui::Slider::new(&mut cfg.chromatic_aberration, -0.04..=0.04)
                            .text("物理色収差 R/B"),
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut cfg.lens_zoom_compensate, "周辺黒枠ズーム補償")
                    .changed();
            });
        });

        if changed {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetOpticalVfx { optical_vfx: cfg },
                    CommandSource::Gui,
                    None,
                )
                .is_ok()
            {
                dirty = true;
            }
        }
        ui.weak(&self.optical_status);

        dirty
    }

    // =========================================================================
    // #52: オーディオ修復 DSP ＆ ADR 3-2-1 カウント収録 ＆ 5.1ch / FOA Ambisonics バス
    // =========================================================================
    pub fn ui_audio_restoration_adr_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut dirty = false;

        ui.heading("🎙 オーディオ修復・ADR カウント収録・5.1ch / FOA Ambisonics バス (#52)");
        ui.separator();

        // --- Section A: オーディオ修復 DSP ---
        ui.collapsing(
            RichText::new("🛠 オーディオ修復 DSP（ハム除去 / デクリック / 適応 De-Reverb）")
                .strong(),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.restore_target_clip, "選択クリップへ適用（OFF=マスター）");
                    if ui.small_button("50Hz (東日本)").clicked() {
                        self.hum_base_hz = 50.0;
                    }
                    if ui.small_button("60Hz (西日本)").clicked() {
                        self.hum_base_hz = 60.0;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Slider::new(&mut self.hum_base_hz, 45.0..=65.0).text("基本波 Hz"));
                    ui.add(egui::Slider::new(&mut self.hum_harmonics, 1..=12).text("高調波数"));
                    ui.add(egui::Slider::new(&mut self.hum_q, 6.0..=60.0).text("Q 値"));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.declick_sensitivity, 2.0..=8.0)
                            .text("デクリック感度"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.dereverb_strength, 0.0..=1.0)
                            .text("De-Reverb 強度"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.dereverb_decay_ms, 40.0..=400.0)
                            .text("残響減衰 ms"),
                    );
                });

                let target = if self.restore_target_clip {
                    selected_item
                } else {
                    None
                };

                ui.horizontal_wrapped(|ui| {
                    if ui.button("⚡ ハム除去を追加 (1-Undo)").clicked() {
                        let mut node = AudioEffectNode::with_defaults("hum_remove");
                        node.params.insert("base_hz".into(), self.hum_base_hz);
                        node.params
                            .insert("harmonics".into(), self.hum_harmonics as f32);
                        node.params.insert("q".into(), self.hum_q);
                        let mut g = session.lock().unwrap();
                        let s = &mut *g;
                        if s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::AddAudioEffect {
                                    item_id: target,
                                    effect: node,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                        {
                            self.restore_status = format!(
                                "✅ ハム除去 ({:.0}Hz × {}倍音) を追加しました",
                                self.hum_base_hz, self.hum_harmonics
                            );
                            dirty = true;
                        }
                    }

                    if ui.button("✨ デクリックを追加 (1-Undo)").clicked() {
                        let mut node = AudioEffectNode::with_defaults("declick");
                        node.params
                            .insert("sensitivity".into(), self.declick_sensitivity);
                        let mut g = session.lock().unwrap();
                        let s = &mut *g;
                        if s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::AddAudioEffect {
                                    item_id: target,
                                    effect: node,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                        {
                            self.restore_status = "✅ MAD+Hermite デクリックを追加しました".into();
                            dirty = true;
                        }
                    }

                    if ui.button("🔇 適応 De-Reverb を追加 (1-Undo)").clicked() {
                        let mut node = AudioEffectNode::with_defaults("dereverb");
                        node.params
                            .insert("strength".into(), self.dereverb_strength);
                        node.params
                            .insert("decay_ms".into(), self.dereverb_decay_ms);
                        let mut g = session.lock().unwrap();
                        let s = &mut *g;
                        if s.bus
                            .execute(
                                &mut s.project,
                                EditCommand::AddAudioEffect {
                                    item_id: target,
                                    effect: node,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                        {
                            self.restore_status = "✅ 適応 De-Reverb を追加しました".into();
                            dirty = true;
                        }
                    }

                    if ui.small_button("🧪 修復 DSP セルフテスト").clicked() {
                        let sr = 48_000u32;
                        let frames = (sr as usize) / 2;
                        let mut pcm = vec![0.0_f32; frames * 2];
                        for i in 0..frames {
                            let t = i as f32 / sr as f32;
                            let hum = 0.28 * (std::f32::consts::TAU * self.hum_base_hz * t).sin()
                                + 0.14
                                    * (std::f32::consts::TAU * self.hum_base_hz * 2.0 * t).sin();
                            let v = 0.25 * (std::f32::consts::TAU * 440.0 * t).sin();
                            pcm[i * 2] = hum + v;
                            pcm[i * 2 + 1] = hum + v;
                        }
                        pcm[400 * 2] = 0.95;
                        pcm[1400 * 2 + 1] = -0.95;
                        let (n, before_db, after_db) = apply_hum_removal(
                            &mut pcm,
                            2,
                            sr,
                            self.hum_base_hz,
                            self.hum_harmonics,
                            self.hum_q,
                            1.0,
                        );
                        let rep = apply_declick(&mut pcm, 2, self.declick_sensitivity, 8);
                        apply_adaptive_dereverb(
                            &mut pcm,
                            2,
                            sr,
                            self.dereverb_strength,
                            self.dereverb_decay_ms,
                            -24.0,
                        );
                        self.restore_status = format!(
                            "🧪 検証OK: ハム {n}段ノッチ ({before_db:.1} → {after_db:.1} dB) | クリック {}箇所修復",
                            rep.clicks_repaired
                        );
                    }
                });
                ui.weak(&self.restore_status);
            },
        );

        ui.separator();

        // --- Section B: ADR 3-2-1 カウント収録 ＆ キュー管理 ---
        ui.collapsing(
            RichText::new("🎬 ADR 3-2-1 カウント収録（映像ストリーマ＆同期ビープ）").strong(),
            |ui| {
                let mut adr = {
                    let g = session.lock().unwrap();
                    g.project
                        .active_scene()
                        .map(|s| s.adr_session.clone())
                        .unwrap_or_default()
                };
                let mut adr_changed = false;

                ui.horizontal_wrapped(|ui| {
                    adr_changed |= ui
                        .checkbox(&mut adr.enabled, "ADR セッション有効")
                        .changed();
                    adr_changed |= ui
                        .checkbox(
                            &mut adr.show_countdown_streamer,
                            "プレビュー映像ストリーマ表示",
                        )
                        .changed();
                    adr_changed |= ui
                        .checkbox(&mut adr.beep_enabled, "3-2-1 ビープ有効")
                        .changed();
                });

                ui.horizontal_wrapped(|ui| {
                    ui.label("話者:");
                    ui.add(egui::TextEdit::singleline(&mut self.adr_character).desired_width(90.0));
                    ui.label("台詞:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.adr_line_text).desired_width(220.0),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.adr_start_sec)
                            .prefix("開始: ")
                            .suffix("s")
                            .speed(0.1)
                            .range(0.0..=600.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.adr_duration_sec)
                            .prefix("尺: ")
                            .suffix("s")
                            .speed(0.1)
                            .range(0.2..=60.0),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.adr_preroll_sec)
                            .prefix("プリロール: ")
                            .suffix("s")
                            .speed(0.5)
                            .range(1.0..=5.0),
                    );
                    if ui.button("➕ ADR キュー追加 (1-Undo)").clicked() {
                        adr.enabled = true;
                        let next_num = adr.cues.iter().map(|c| c.cue_number).max().unwrap_or(0) + 1;
                        let mut cue = AdrCue::new(
                            next_num,
                            self.adr_character.clone(),
                            self.adr_line_text.clone(),
                            RationalTime::from_seconds(self.adr_start_sec),
                            RationalTime::from_seconds(self.adr_duration_sec),
                        );
                        cue.preroll_sec = self.adr_preroll_sec;
                        adr.cues.push(cue);
                        adr_changed = true;
                        self.adr_status = format!("✅ ADR Cue #{next_num} を登録しました");
                    }
                });

                let mut remove_idx = None;
                for (idx, cue) in adr.cues.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.monospace(format!(
                            "#{:02} [{:.1}s+{:.1}s] {}: {}",
                            cue.cue_number,
                            cue.start_time.as_seconds(),
                            cue.duration.as_seconds(),
                            cue.character,
                            cue.line_text
                        ));
                        if ui.small_button("🗑").clicked() {
                            remove_idx = Some(idx);
                        }
                    });
                }
                if let Some(idx) = remove_idx {
                    adr.cues.remove(idx);
                    adr_changed = true;
                }

                if adr_changed {
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetAdrSession {
                                adr_session: adr.clone(),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        dirty = true;
                    }
                }

                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.adr_subtitle_layer)
                            .prefix("配置レイヤー: ")
                            .range(0..=15),
                    );
                    if ui
                        .add_enabled(
                            !adr.cues.is_empty(),
                            egui::Button::new("📌 タイムラインへキュー配置 (1-Undo)"),
                        )
                        .clicked()
                        && let Some(cmd) =
                            build_adr_cue_timeline_commands(&adr.cues, self.adr_subtitle_layer)
                    {
                        let mut g = session.lock().unwrap();
                        let s = &mut *g;
                        if s.bus
                            .execute(&mut s.project, cmd, CommandSource::Gui, None)
                            .is_ok()
                        {
                            self.adr_status = format!(
                                "✅ {} 件の ADR キューを字幕＆マーカーとして配置しました",
                                adr.cues.len()
                            );
                            dirty = true;
                        }
                    }

                    if ui.button("🔔 3-2-1 同期ビープ WAV 書き出し").clicked() {
                        let sr = 48_000u32;
                        let pcm = synthesize_adr_preroll_beeps(sr, self.adr_preroll_sec);
                        let out = {
                            let g = session.lock().unwrap();
                            g.folder
                                .as_ref()
                                .map(|f| f.exports_dir().join("adr_countdown_beeps.wav"))
                                .unwrap_or_else(|| {
                                    std::env::temp_dir().join("ve_adr_countdown_beeps.wav")
                                })
                        };
                        match write_multichannel_wav_file(&out, &pcm, 2, sr) {
                            Ok(bytes) => {
                                self.adr_status = format!(
                                    "✅ ADR ビープ WAV 出力完了 ({bytes} bytes) → {}",
                                    out.display()
                                );
                            }
                            Err(e) => self.adr_status = format!("❌ 出力失敗: {e}"),
                        }
                    }
                });
                ui.weak(&self.adr_status);
            },
        );

        ui.separator();

        // --- Section C: 5.1ch サラウンド / FOA Ambisonics 空間音声バス ---
        ui.collapsing(
            RichText::new("🌐 5.1ch サラウンド (ITU-R BS.775) ＆ FOA Ambisonics (AmbiX W,Y,Z,X) バス")
                .strong(),
            |ui| {
                let mut bus = {
                    let g = session.lock().unwrap();
                    g.project
                        .active_scene()
                        .map(|s| s.spatial_bus.clone())
                        .unwrap_or_default()
                };
                let mut bus_changed = false;

                ui.horizontal_wrapped(|ui| {
                    for &m in SpatialAudioBusMode::ALL {
                        if ui
                            .selectable_label(bus.mode == m, m.display_name())
                            .clicked()
                        {
                            bus.mode = m;
                            bus_changed = true;
                        }
                    }
                });

                // 360° Spatial Radar Visualizer
                ui.horizontal(|ui| {
                    let radar_size = Vec2::splat(132.0);
                    let (rect, _) = ui.allocate_exact_size(radar_size, Sense::hover());
                    let p = ui.painter_at(rect);
                    let center = rect.center();
                    let radius = rect.width() * 0.44;
                    p.circle_filled(center, radius + 4.0, Color32::from_rgb(18, 22, 32));
                    p.circle_stroke(
                        center,
                        radius,
                        Stroke::new(1.0, Color32::from_rgb(72, 110, 152)),
                    );
                    p.circle_stroke(
                        center,
                        radius * 0.5,
                        Stroke::new(0.6, Color32::from_rgba_unmultiplied(72, 110, 152, 90)),
                    );

                    // ITU 5.1 speaker icons: L(-30), R(+30), C(0), Ls(-110), Rs(+110)
                    for &(deg, lbl) in &[
                        (-30.0_f32, "L"),
                        (0.0, "C"),
                        (30.0, "R"),
                        (-110.0, "Ls"),
                        (110.0, "Rs"),
                    ] {
                        let rad = deg.to_radians();
                        let sp = Pos2::new(
                            center.x + rad.sin() * radius,
                            center.y - rad.cos() * radius,
                        );
                        p.circle_filled(sp, 3.0, Color32::from_rgb(120, 180, 230));
                        p.text(
                            sp + Vec2::new(0.0, -7.0),
                            egui::Align2::CENTER_CENTER,
                            lbl,
                            egui::FontId::monospace(9.0),
                            Color32::LIGHT_GRAY,
                        );
                    }

                    // Active sound source dot
                    let az = bus.azimuth_deg.to_radians();
                    let elev_scale = (1.0 - (bus.elevation_deg.abs() / 110.0)).clamp(0.25, 1.0);
                    let src = Pos2::new(
                        center.x + az.sin() * radius * 0.85 * elev_scale,
                        center.y - az.cos() * radius * 0.85 * elev_scale,
                    );
                    p.line_segment(
                        [center, src],
                        Stroke::new(1.5, Color32::from_rgb(255, 186, 66)),
                    );
                    p.circle_filled(src, 5.5, Color32::from_rgb(255, 120, 66));

                    ui.vertical(|ui| {
                        bus_changed |= ui
                            .add(
                                egui::Slider::new(&mut bus.azimuth_deg, -180.0..=180.0)
                                    .text("方位角°"),
                            )
                            .changed();
                        bus_changed |= ui
                            .add(
                                egui::Slider::new(&mut bus.elevation_deg, -90.0..=90.0)
                                    .text("仰角° (Z)"),
                            )
                            .changed();
                        bus_changed |= ui
                            .add(
                                egui::Slider::new(&mut bus.room_width, 0.0..=1.0).text("音場広がり"),
                            )
                            .changed();
                    });
                });

                ui.horizontal_wrapped(|ui| {
                    bus_changed |= ui
                        .add(
                            egui::Slider::new(&mut bus.lfe_crossover_hz, 40.0..=160.0)
                                .text("LFE LPF Hz"),
                        )
                        .changed();
                    bus_changed |= ui
                        .add(
                            egui::Slider::new(&mut bus.center_dialogue_boost, 0.0..=1.0)
                                .text("センター台詞寄与"),
                        )
                        .changed();
                });
                ui.horizontal_wrapped(|ui| {
                    bus_changed |= ui
                        .add(
                            egui::Slider::new(&mut bus.head_yaw_deg, -180.0..=180.0)
                                .text("FOA 頭部ヨー°"),
                        )
                        .changed();
                    bus_changed |= ui
                        .add(
                            egui::Slider::new(&mut bus.head_pitch_deg, -90.0..=90.0)
                                .text("FOA 頭部ピッチ°"),
                        )
                        .changed();
                });

                if bus_changed {
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetSpatialBus {
                                spatial_bus: bus.clone(),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        dirty = true;
                    }
                }

                if ui
                    .button(format!(
                        "💾 空間マスター WAV 書き出し ({}ch)",
                        bus.mode.channel_count()
                    ))
                    .clicked()
                {
                    let sr = 48_000u32;
                    let frames = sr as usize;
                    let mut stereo = vec![0.0_f32; frames * 2];
                    for i in 0..frames {
                        let t = i as f32 / sr as f32;
                        let sig = 0.35 * (std::f32::consts::TAU * 330.0 * t).sin()
                            + 0.18 * (std::f32::consts::TAU * 70.0 * t).sin();
                        stereo[i * 2] = sig;
                        stereo[i * 2 + 1] = sig * 0.85;
                    }
                    let out = {
                        let g = session.lock().unwrap();
                        let fname = match bus.mode {
                            SpatialAudioBusMode::Surround51 => "master_51_surround.wav",
                            SpatialAudioBusMode::AmbisonicsFoaAmbiX => "master_foa_ambix.wav",
                            SpatialAudioBusMode::Stereo => "master_stereo.wav",
                        };
                        g.folder
                            .as_ref()
                            .map(|f| f.exports_dir().join(fname))
                            .unwrap_or_else(|| std::env::temp_dir().join(fname))
                    };
                    let res = match bus.mode {
                        SpatialAudioBusMode::Surround51 => {
                            let s51 = encode_stereo_to_51_surround(&stereo, sr, &bus);
                            let rms = s51.channel_rms_db();
                            let _dm = downmix_51_to_stereo_itu(&s51);
                            write_multichannel_wav_file(&out, &s51.samples, 6, sr).map(|bytes| {
                                format!(
                                    "✅ 5.1ch WAV 出力 ({bytes}B) | L:{:.1} R:{:.1} C:{:.1} LFE:{:.1} Ls:{:.1} Rs:{:.1} dB",
                                    rms[0], rms[1], rms[2], rms[3], rms[4], rms[5]
                                )
                            })
                        }
                        SpatialAudioBusMode::AmbisonicsFoaAmbiX => {
                            let mut foa = encode_stereo_to_foa_ambix(&stereo, sr, &bus);
                            rotate_foa_ambix(&mut foa, bus.head_yaw_deg, bus.head_pitch_deg);
                            let rms = foa.channel_rms_db();
                            let _bin = decode_foa_ambix_to_binaural_stereo(&foa);
                            write_multichannel_wav_file(&out, &foa.samples, 4, sr).map(|bytes| {
                                format!(
                                    "✅ FOA AmbiX 4ch WAV 出力 ({bytes}B) | W:{:.1} Y:{:.1} Z:{:.1} X:{:.1} dB",
                                    rms[0], rms[1], rms[2], rms[3]
                                )
                            })
                        }
                        SpatialAudioBusMode::Stereo => {
                            write_multichannel_wav_file(&out, &stereo, 2, sr)
                                .map(|bytes| format!("✅ ステレオ 2ch WAV 出力 ({bytes}B)"))
                        }
                    };
                    self.spatial_status = match res {
                        Ok(msg) => format!("{msg} → {}", out.display()),
                        Err(e) => format!("❌ 空間 WAV 出力失敗: {e}"),
                    };
                }
                ui.weak(&self.spatial_status);
            },
        );

        dirty
    }
}
