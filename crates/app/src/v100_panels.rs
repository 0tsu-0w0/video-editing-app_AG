//! v10.0 #57〜#60 Spatial Computing, Physics Fluids & Autonomous Cinema OS スタジオパネル
//!
//! 既存の 6 スタジオ Hub 内サブタブとして機能ゼロ欠落・トップバー肥大化ゼロで統合される:
//! - VFX & Motion Hub:
//!   - `ui_spatial_360_tab` (#57 360° OverCapture / Tiny Planet / 水平ロック ＆ 空間 3D)
//!   - `ui_fluid_cloth_ibl_tab` (#58 2D Navier-Stokes 流体・PBD クロス ＆ 3D シャドウ/HDRI IBL)
//!   - `ui_mocap_retarget_tab` (#59 BVH / シルエット モーションキャプチャ ＆ 足接地ロックリターゲット)
//! - AI & Collab Hub:
//!   - `ui_cinema_producer_tab` (#60 自律 AI シネマプロデューサー：台本→絵コンテ・視聴維持率予測・A/B 自動編集)
//! - Deliver Hub:
//!   - `ui_render_farm_tab` (#60 LAN 分散レンダーファーム：HMAC-SHA256 分割＆ローカルコーディネータ＆concat 結合)

use eframe::egui::{self, Color32, Pos2, RichText, Stroke, Ui, Vec2};
use std::io::{Read, Write};
use std::net::TcpStream;
use ve_core::{
    ClothPreset, ClothSimConfig, CommandSource, EditCommand, EditVariantStyle, FluidPreset,
    FluidSimConfig, HdriEnvPreset, ItemId, ItemKind, MocapPreset, Projection360Mode, RationalTime,
    RenderFarmCoordinatorServer, RenderFarmWorkerNode, RetargetConfig, RetentionDiagnostics,
    Rig2DConfig, Spatial360Preset, SpatialVideoConfig, StereoLayout, StoryboardPlan, TimelineItem,
    analyze_viewer_retention, build_ab_edit_variant_command, build_farm_concat_manifest,
    build_mocap_retarget_bake_commands, build_storyboard_animatic_commands, builtin_sample_bvh,
    compass_heading_label, evaluate_horizon_locked_euler, extract_silhouette_pose_from_rgba8,
    parse_bvh, parse_script_to_storyboard, plan_render_farm_chunks, retarget_bvh_to_rig2d,
    retarget_silhouette_sequence_to_rig2d, verify_chunk_hmac,
};
use ve_mcp::SharedEditorSession;
use ve_render::RenderGraph;

/// v10.0 #57〜#60 Hub 内サブパネル状態
pub struct V100PanelsState {
    // --- #57 360° / 空間 3D ---
    pub spatial_status: String,

    // --- #58 流体・クロス・3D シャドウ/HDRI IBL ---
    pub fluid_cfg: FluidSimConfig,
    pub cloth_cfg: ClothSimConfig,
    pub physics_layer: usize,
    pub physics_duration_sec: f32,
    pub vfx10_status: String,

    // --- #59 BVH / シルエット モキャップ ---
    pub mocap_preset: MocapPreset,
    pub bvh_input: String,
    pub retarget_cfg: RetargetConfig,
    pub target_is_3d: bool,
    pub mocap_status: String,

    // --- #60 自律 AI シネマプロデューサー ---
    pub script_input: String,
    pub last_storyboard: Option<StoryboardPlan>,
    pub last_retention: Option<RetentionDiagnostics>,
    pub producer_status: String,

    // --- #60 LAN 分散レンダーファーム ---
    pub new_worker_id: String,
    pub new_worker_host: String,
    pub new_worker_threads: u32,
    pub farm_coordinator: Option<RenderFarmCoordinatorServer>,
    pub farm_manifest_preview: String,
    pub farm_status: String,
}

impl Default for V100PanelsState {
    fn default() -> Self {
        Self {
            spatial_status: "Equirectangular 360° OverCapture / Tiny Planet / ジャイロ水平ロック / SBS・TAB・アナグリフ 3D".into(),
            fluid_cfg: FluidSimConfig::from_preset(FluidPreset::BonfireFlame),
            cloth_cfg: ClothSimConfig::from_preset(ClothPreset::CyberFlag),
            physics_layer: 1,
            physics_duration_sec: 5.0,
            vfx10_status: "Jos Stam 決定論 2D 流体・PBD クロス物理・3D PCF シャドウマップ＆HDRI IBL".into(),
            mocap_preset: MocapPreset::WalkCycle,
            bvh_input: String::new(),
            retarget_cfg: RetargetConfig::default(),
            target_is_3d: false,
            mocap_status: "Pure-Rust .bvh 解析 / 動画シルエット抽出 → ボーン比率スケーリング＋足接地ロック".into(),
            script_input: "# Next-Gen Cinema OS v10.0\n## Hook\n[Close Up] 全 100 MCP ツールでローカル映像制作を自律自動化！\n## Demo\n[Wide] 360° 全天球・流体物理・BVH リターゲット・LAN 分散レンダーを完全オフラインで完結。\n## Outro\n[Dynamic Insert] 今すぐ新しいクリエイティブ体験を始めましょう！\n".into(),
            last_storyboard: None,
            last_retention: None,
            producer_status: "台本→絵コンテアニマティック構築・秒別視聴維持率予測・A/B 自動編集バリアント生成".into(),
            new_worker_id: "node-lan-workstation04".into(),
            new_worker_host: "192.168.1.24".into(),
            new_worker_threads: 16,
            farm_coordinator: None,
            farm_manifest_preview: String::new(),
            farm_status: "Bearer + RFC 2104 HMAC-SHA256 署名付き LAN フレームチャンク分散＆無劣化結合".into(),
        }
    }
}

impl V100PanelsState {
    // =========================================================================
    // #57: 360° 全天球動画（OverCapture / Tiny Planet / 水平ロック）＆ 空間 3D
    // =========================================================================
    pub fn ui_spatial_360_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        ui.heading("🌐 360° 全天球動画（OverCapture / Tiny Planet / 水平ロック）＆ 空間 3D (#57)");
        ui.label(RichText::new("Equirectangular 逆射影カメラワーク切り出し・ジャイロ水平ロック・VR SBS / TAB / アナグリフ立体視").weak());
        ui.add_space(4.0);

        let (mut cfg, playhead_sec) = {
            let g = session.lock().unwrap();
            let c = g
                .project
                .active_scene()
                .map(|s| s.spatial_video.clone())
                .unwrap_or_default();
            (c, g.playhead.as_seconds() as f32)
        };
        let before = cfg.clone();

        ui.horizontal_wrapped(|ui| {
            ui.label("プリセット:");
            for &preset in Spatial360Preset::ALL {
                if ui.small_button(preset.display_name()).clicked() {
                    cfg = SpatialVideoConfig::from_preset(preset);
                    self.spatial_status =
                        format!("✅ プリセット '{}' を適用しました", preset.display_name());
                }
            }
        });

        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut cfg.enabled, "360° / 空間 3D パスを有効化");
                ui.checkbox(&mut cfg.horizon_lock_enabled, "🧭 ジャイロ水平ロック (Horizon Lock)");
                ui.checkbox(&mut cfg.show_horizon_guides, "水平線＆方位ガイド");
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("投影モード:");
                for &m in Projection360Mode::ALL {
                    ui.selectable_value(&mut cfg.projection_mode, m, m.display_name());
                }
            });

            egui::Grid::new("v100_spatial_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Yaw (水平パン):");
                    ui.add(egui::Slider::new(&mut cfg.yaw_deg, -180.0..=180.0).suffix("°"));
                    ui.end_row();
                    ui.label("Pitch (上下チルト):");
                    ui.add(egui::Slider::new(&mut cfg.pitch_deg, -90.0..=90.0).suffix("°"));
                    ui.end_row();
                    ui.label("Roll (傾き):");
                    ui.add(egui::Slider::new(&mut cfg.roll_deg, -180.0..=180.0).suffix("°"));
                    ui.end_row();
                    ui.label("OverCapture 視野角 FOV:");
                    ui.add(egui::Slider::new(&mut cfg.fov_deg, 30.0..=150.0).suffix("°"));
                    ui.end_row();
                    ui.label("Tiny Planet ズーム:");
                    ui.add(egui::Slider::new(&mut cfg.tiny_planet_zoom, 0.3..=3.0).suffix("x"));
                    ui.end_row();
                    ui.label("水平ロック平滑化:");
                    ui.add(egui::Slider::new(&mut cfg.horizon_lock_smoothing, 0.0..=0.95));
                    ui.end_row();
                });

            let (ey, ep, er) = evaluate_horizon_locked_euler(&cfg, playhead_sec);
            ui.monospace(format!(
                "実効カメラ姿勢 (t={playhead_sec:.2}s): 方位 {} | Yaw={ey:+.1}° Pitch={ep:+.1}° Roll={er:+.1}°",
                compass_heading_label(ey)
            ));
        });

        ui.group(|ui| {
            ui.label(RichText::new("🥽 ステレオスコピック空間 3D 出力").strong());
            ui.horizontal_wrapped(|ui| {
                for &sl in StereoLayout::ALL {
                    ui.selectable_value(&mut cfg.stereo_layout, sl, sl.display_name());
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("IPD 視差:");
                ui.add(egui::Slider::new(&mut cfg.ipd_parallax_px, 0.0..=48.0).suffix(" px"));
                ui.label("コンバージェンス:");
                ui.add(
                    egui::Slider::new(&mut cfg.convergence_offset_px, -32.0..=32.0).suffix(" px"),
                );
            });
        });

        if cfg != before {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetSpatialVideo { spatial_video: cfg },
                    CommandSource::Gui,
                    Some("spatial_360_ui"),
                )
                .is_ok()
            {
                dirty = true;
            }
        }
        ui.weak(&self.spatial_status);
        dirty
    }

    // =========================================================================
    // #58: 決定論 2D 流体・PBD クロス物理 ＆ 3D シャドウマップ / HDRI IBL
    // =========================================================================
    pub fn ui_fluid_cloth_ibl_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
    ) -> bool {
        let mut dirty = false;
        ui.heading("🔥 決定論 2D Navier-Stokes 流体・PBD クロス物理 ＆ 3D シャドウ/HDRI IBL (#58)");
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.label("配置レイヤー:");
            ui.add(egui::DragValue::new(&mut self.physics_layer).range(0..=3));
            ui.label("クリップ尺:");
            ui.add(
                egui::DragValue::new(&mut self.physics_duration_sec)
                    .range(0.5..=60.0)
                    .suffix(" s"),
            );
        });

        // --- 2D Navier-Stokes 流体 ---
        ui.group(|ui| {
            ui.label(
                RichText::new("💨 Jos Stam 決定論 2D Navier-Stokes 流体（煙・炎・墨・プラズマ）")
                    .strong(),
            );
            ui.horizontal_wrapped(|ui| {
                for &p in FluidPreset::ALL {
                    if ui
                        .selectable_label(self.fluid_cfg.preset == p, p.display_name())
                        .clicked()
                    {
                        self.fluid_cfg = FluidSimConfig::from_preset(p);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("渦度閉じ込め:");
                ui.add(egui::Slider::new(
                    &mut self.fluid_cfg.vorticity_confinement,
                    0.0..=10.0,
                ));
                ui.label("熱浮力:");
                ui.add(egui::Slider::new(&mut self.fluid_cfg.buoyancy, -3.0..=6.0));
                ui.label("格子:");
                ui.add(egui::DragValue::new(&mut self.fluid_cfg.grid_w).range(20..=80));
                self.fluid_cfg.grid_h = self.fluid_cfg.grid_w;
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("🔥 2D 流体アイテムをタイムラインに追加 (1-Undo)")
                    .clicked()
                {
                    let mut g = session.lock().unwrap();
                    let (w, h, start) = (
                        g.project.active_scene().map(|s| s.width).unwrap_or(1920) as f32,
                        g.project.active_scene().map(|s| s.height).unwrap_or(1080) as f32,
                        g.playhead,
                    );
                    let mut item = TimelineItem::new(
                        format!("Fluid_{:?}", self.fluid_cfg.preset),
                        start,
                        RationalTime::from_seconds(self.physics_duration_sec as f64),
                        ItemKind::FluidSim {
                            config: Box::new(self.fluid_cfg.clone()),
                        },
                    );
                    item.transform.x = w * 0.5;
                    item.transform.y = h * 0.5;
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject {
                                layer_index: self.physics_layer,
                                item,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.vfx10_status = "✅ 決定論 2D 流体アイテムを追加しました".into();
                        dirty = true;
                    }
                }
                if let Some(sel_id) = selected_item
                    && ui.small_button("選択クリップへ流体設定を反映").clicked()
                {
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetFluidSim {
                                item_id: sel_id,
                                config: Box::new(self.fluid_cfg.clone()),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.vfx10_status = "✅ 選択アイテムの流体設定を更新しました".into();
                        dirty = true;
                    }
                }
            });
        });

        // --- PBD クロス物理 ---
        ui.group(|ui| {
            ui.label(
                RichText::new("🚩 Position-Based Dynamics (PBD) クロス物理シミュレーション")
                    .strong(),
            );
            ui.horizontal_wrapped(|ui| {
                for &p in ClothPreset::ALL {
                    if ui
                        .selectable_label(self.cloth_cfg.preset == p, p.display_name())
                        .clicked()
                    {
                        self.cloth_cfg = ClothSimConfig::from_preset(p);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("風力:");
                ui.add(egui::Slider::new(
                    &mut self.cloth_cfg.wind_strength,
                    0.0..=5.0,
                ));
                ui.label("乱流:");
                ui.add(egui::Slider::new(
                    &mut self.cloth_cfg.wind_turbulence,
                    0.0..=3.0,
                ));
                ui.checkbox(&mut self.cloth_cfg.show_wireframe, "ワイヤーフレーム");
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("🚩 クロス物理アイテムをタイムラインに追加 (1-Undo)")
                    .clicked()
                {
                    let mut g = session.lock().unwrap();
                    let (w, h, start) = (
                        g.project.active_scene().map(|s| s.width).unwrap_or(1920) as f32,
                        g.project.active_scene().map(|s| s.height).unwrap_or(1080) as f32,
                        g.playhead,
                    );
                    let mut item = TimelineItem::new(
                        format!("Cloth_{:?}", self.cloth_cfg.preset),
                        start,
                        RationalTime::from_seconds(self.physics_duration_sec as f64),
                        ItemKind::ClothSim {
                            config: Box::new(self.cloth_cfg.clone()),
                        },
                    );
                    item.transform.x = w * 0.5;
                    item.transform.y = h * 0.5;
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::AddObject {
                                layer_index: self.physics_layer,
                                item,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.vfx10_status = "✅ PBD クロス物理アイテムを追加しました".into();
                        dirty = true;
                    }
                }
                if let Some(sel_id) = selected_item
                    && ui.small_button("選択クリップへクロス設定を反映").clicked()
                {
                    let mut g = session.lock().unwrap();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::SetClothSim {
                                item_id: sel_id,
                                config: Box::new(self.cloth_cfg.clone()),
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.vfx10_status = "✅ 選択アイテムのクロス設定を更新しました".into();
                        dirty = true;
                    }
                }
            });
        });

        // --- 3D シャドウマップ & HDRI IBL ---
        ui.group(|ui| {
            ui.label(
                RichText::new(
                    "☀ 3D ソフトシャドウマップ (PCF) ＆ Equirectangular HDRI IBL 環境照明",
                )
                .strong(),
            );
            let mut adv = {
                let g = session.lock().unwrap();
                g.project
                    .active_scene()
                    .map(|s| s.lighting_3d_adv.clone())
                    .unwrap_or_default()
            };
            let before = adv.clone();

            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut adv.shadow_map_enabled, "デプスシャドウマップ有効");
                ui.checkbox(&mut adv.ground_catcher_enabled, "床シャドウキャッチャー");
                ui.checkbox(&mut adv.hdri_ibl_enabled, "HDRI 拡散/鏡面 IBL 有効");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("HDRI 環境:");
                for &p in HdriEnvPreset::ALL {
                    ui.selectable_value(&mut adv.hdri_preset, p, p.display_name());
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("影強度:");
                ui.add(egui::Slider::new(&mut adv.shadow_strength, 0.0..=1.0));
                ui.label("HDRI 回転:");
                ui.add(egui::Slider::new(&mut adv.hdri_rotation_deg, 0.0..=360.0).suffix("°"));
                ui.label("IBL 露出:");
                ui.add(egui::Slider::new(&mut adv.hdri_exposure, 0.2..=3.0));
            });

            if adv != before {
                let mut g = session.lock().unwrap();
                let s = &mut *g;
                if s.bus
                    .execute(
                        &mut s.project,
                        EditCommand::SetLighting3DAdv {
                            lighting_3d_adv: adv,
                        },
                        CommandSource::Gui,
                        Some("lighting_3d_adv_ui"),
                    )
                    .is_ok()
                {
                    dirty = true;
                }
            }
        });

        ui.weak(&self.vfx10_status);
        dirty
    }

    // =========================================================================
    // #59: BVH / 動画シルエット モーションキャプチャ → リターゲット（足接地ロック）
    // =========================================================================
    pub fn ui_mocap_retarget_tab(
        &mut self,
        ui: &mut Ui,
        session: &SharedEditorSession,
        selected_item: Option<ItemId>,
        render_graph: &mut RenderGraph,
    ) -> bool {
        let mut dirty = false;
        ui.heading(
            "🏃 BVH / 動画シルエット モーションキャプチャ ＆ 足接地ロックリターゲット (#59)",
        );
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.label("内蔵 BVH モーション:");
            for &p in MocapPreset::ALL {
                ui.selectable_value(&mut self.mocap_preset, p, p.display_name());
            }
            if ui.small_button("📥 内蔵 BVH をエディタへ読込").clicked() {
                self.bvh_input = builtin_sample_bvh(self.mocap_preset);
                self.mocap_status = format!(
                    "✅ 内蔵 BVH '{}' を読み込みました",
                    self.mocap_preset.display_name()
                );
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.retarget_cfg.scale_by_bone_ratio,
                "体格差吸収ボーン長比率スケーリング",
            );
            ui.checkbox(
                &mut self.retarget_cfg.foot_lock_enabled,
                "🦶 足滑り防止 Foot Skate Locking IK",
            );
            ui.checkbox(&mut self.target_is_3d, "3D メッシュターゲットモード");
        });

        ui.collapsing(
            "📄 カスタム .bvh テキスト入力（任意・空欄時は上記プリセットを使用）",
            |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.bvh_input)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .code_editor(),
                );
            },
        );

        ui.horizontal_wrapped(|ui| {
            if ui
                .button("🏃 BVH 解析＆リターゲットを選択アイテムへベイク (1-Undo)")
                .clicked()
            {
                let src = if self.bvh_input.trim().is_empty() {
                    builtin_sample_bvh(self.mocap_preset)
                } else {
                    self.bvh_input.clone()
                };
                match parse_bvh(&src) {
                    Ok(clip) => {
                        let rig = Rig2DConfig::default_humanoid();
                        let poses = retarget_bvh_to_rig2d(&clip, &rig, &self.retarget_cfg);
                        let locked = poses
                            .iter()
                            .filter(|p| p.left_foot_locked || p.right_foot_locked)
                            .count();
                        if let Some(target_id) = selected_item {
                            let mut g = session.lock().unwrap();
                            let (bx, by) = g
                                .project
                                .active_scene()
                                .and_then(|sc| {
                                    sc.layers
                                        .iter()
                                        .flat_map(|l| l.items.iter())
                                        .find(|it| it.id == target_id)
                                        .map(|it| (it.transform.x, it.transform.y))
                                })
                                .unwrap_or((640.0, 360.0));
                            let cmds = build_mocap_retarget_bake_commands(
                                target_id,
                                bx,
                                by,
                                &poses,
                                self.target_is_3d,
                            );
                            let cmd_cnt = cmds.len();
                            let s = &mut *g;
                            if s.bus
                                .execute(
                                    &mut s.project,
                                    EditCommand::Batch {
                                        label: "BVH Mocap Retarget Bake".into(),
                                        commands: cmds,
                                    },
                                    CommandSource::Gui,
                                    None,
                                )
                                .is_ok()
                            {
                                self.mocap_status = format!(
                                    "✅ BVH {} 関節 / {} フレームをベイク完了（足接地ロック {} フレーム / {} キーフレーム）",
                                    clip.joints.len(),
                                    clip.frames.len(),
                                    locked,
                                    cmd_cnt
                                );
                                dirty = true;
                            }
                        } else {
                            self.mocap_status = format!(
                                "📊 BVH 解析成功: {} 関節 / {} フレーム / 足接地ロック {} フレーム（タイムラインでアイテム選択後にベイク可能）",
                                clip.joints.len(),
                                clip.frames.len(),
                                locked
                            );
                        }
                    }
                    Err(e) => self.mocap_status = format!("❌ BVH 解析エラー: {e}"),
                }
            }

            if ui
                .button("🎥 現在シーンからシルエット 2D 姿勢抽出 (4 フレーム)")
                .clicked()
            {
                let g = session.lock().unwrap();
                let sc_id = g.project.active_scene_id;
                let base_t = g.playhead.as_seconds();
                let mut sils = Vec::new();
                for i in 0..4 {
                    let t_sec = base_t + i as f64 * 0.2;
                    if let Some(img) = render_graph.render_project_scene_scaled(
                        &g.project,
                        sc_id,
                        RationalTime::from_seconds(t_sec),
                        128,
                        72,
                        false,
                    ) {
                        let rgba = img.to_rgba8_srgb();
                        sils.push(extract_silhouette_pose_from_rgba8(
                            &rgba,
                            img.width,
                            img.height,
                            0.15,
                            t_sec as f32,
                        ));
                    }
                }
                let rig = Rig2DConfig::default_humanoid();
                let poses = retarget_silhouette_sequence_to_rig2d(&sils, &rig, &self.retarget_cfg);
                let avg_c = sils.iter().map(|s| s.confidence).sum::<f32>() / sils.len().max(1) as f32;
                self.mocap_status = format!(
                    "✅ シルエット抽出完了: {} フレーム / 平均信頼度 {:.0}% / リターゲット {} ポーズ",
                    sils.len(),
                    avg_c * 100.0,
                    poses.len()
                );
            }
        });

        ui.weak(&self.mocap_status);
        dirty
    }

    // =========================================================================
    // #60 (AI Hub): 自律 AI シネマプロデューサー（絵コンテ・視聴維持率予測・A/B 自動編集）
    // =========================================================================
    pub fn ui_cinema_producer_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        ui.heading(
            "🎬 自律 AI シネマプロデューサー（台本→絵コンテ・視聴維持率予測・A/B 自動編集）(#60)",
        );
        ui.label(RichText::new("全 100 MCP ツール完全体系化 — オフライン完結で企画構成・アニマティック構築・維持率診断・A/B 最適化を実行").weak());
        ui.add_space(4.0);

        ui.group(|ui| {
            ui.label(RichText::new("📝 台本 → プロシージャル絵コンテ＆アニマティック").strong());
            ui.add(
                egui::TextEdit::multiline(&mut self.script_input)
                    .desired_rows(5)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("📋 台本を絵コンテ構造へ解析").clicked() {
                    let fps = session.lock().unwrap().project.settings.fps;
                    let plan = parse_script_to_storyboard(&self.script_input, fps);
                    self.producer_status = format!(
                        "✅ 絵コンテ解析完了: {} ショット / 合計尺 {:.1}s",
                        plan.beats.len(),
                        plan.total_duration_sec
                    );
                    self.last_storyboard = Some(plan);
                }
                if ui
                    .button("🎬 絵コンテからプレイアブル・アニマティック自動構築 (1-Undo)")
                    .clicked()
                {
                    let mut g = session.lock().unwrap();
                    let fps = g.project.settings.fps;
                    let (w, h) = g
                        .project
                        .active_scene()
                        .map(|s| (s.width, s.height))
                        .unwrap_or((1920, 1080));
                    let plan = parse_script_to_storyboard(&self.script_input, fps);
                    let cmds = build_storyboard_animatic_commands(&plan, w, h);
                    let cnt = cmds.len();
                    let s = &mut *g;
                    if s.bus
                        .execute(
                            &mut s.project,
                            EditCommand::Batch {
                                label: "Autonomous Storyboard Animatic".into(),
                                commands: cmds,
                            },
                            CommandSource::Gui,
                            None,
                        )
                        .is_ok()
                    {
                        self.producer_status = format!(
                            "✅ {} ショット ({:.1}s / {} コマンド) を 1-Undo でタイムライン構築しました",
                            plan.beats.len(),
                            plan.total_duration_sec,
                            cnt
                        );
                        self.last_storyboard = Some(plan);
                        dirty = true;
                    }
                }
            });
            if let Some(ref sb) = self.last_storyboard {
                for beat in sb.beats.iter().take(6) {
                    ui.monospace(format!(
                        "S{:02} [{:.1}s+{:.1}s] {:20} | {}",
                        beat.shot_index,
                        beat.start_sec,
                        beat.duration_sec,
                        beat.framing.label(),
                        beat.dialogue
                    ));
                }
            }
        });

        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("📈 視聴維持率カーブ予測 ＆ A/B 自動編集バリアント").strong());
                if ui.button("🔍 視聴維持率を診断").clicked() {
                    let g = session.lock().unwrap();
                    let diag = analyze_viewer_retention(&g.project, None);
                    self.producer_status = format!(
                        "📈 予測平均維持率 {:.1}% / 冒頭フック {:.0} 点 / 完走率 {:.1}% / ASL {:.2}s",
                        diag.predicted_avg_retention_pct,
                        diag.hook_score,
                        diag.predicted_completion_pct,
                        diag.average_shot_length_sec
                    );
                    self.last_retention = Some(diag);
                }
            });

            if let Some(ref diag) = self.last_retention {
                // 視聴維持率カーブのビジュアルグラフ描画
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width().clamp(220.0, 560.0), 86.0),
                    egui::Sense::hover(),
                );
                let p = ui.painter();
                p.rect_filled(rect, 4.0, Color32::from_rgb(18, 22, 30));
                if diag.curve.len() >= 2 {
                    let max_i = (diag.curve.len() - 1) as f32;
                    let pts: Vec<Pos2> = diag
                        .curve
                        .iter()
                        .enumerate()
                        .map(|(i, pt)| {
                            let x = rect.left() + (i as f32 / max_i) * rect.width();
                            let y = rect.bottom() - (pt.retention_pct / 100.0).clamp(0.0, 1.0) * rect.height();
                            Pos2::new(x, y)
                        })
                        .collect();
                    p.add(egui::Shape::line(
                        pts,
                        Stroke::new(2.0, Color32::from_rgb(64, 226, 178)),
                    ));
                }
                for rec in &diag.recommendations {
                    ui.label(format!("💡 {rec}"));
                }
            }

            ui.horizontal_wrapped(|ui| {
                ui.label("A/B バリアント自動生成:");
                for &style in EditVariantStyle::ALL {
                    if ui.small_button(style.display_name()).clicked() {
                        let mut g = session.lock().unwrap();
                        if let Ok((cmd, summary)) =
                            build_ab_edit_variant_command(&g.project, style, true)
                        {
                            let s = &mut *g;
                            if s.bus
                                .execute(&mut s.project, cmd, CommandSource::Gui, None)
                                .is_ok()
                            {
                                self.producer_status = format!(
                                    "✅ A/B シーン '{}' を生成しました（維持率 {:.1}% → {:.1}% / Δ{:+.1}%）",
                                    summary.new_scene_name,
                                    summary.baseline_retention_pct,
                                    summary.variant_retention_pct,
                                    summary.retention_delta_pct
                                );
                                dirty = true;
                            }
                        }
                    }
                }
            });
        });

        ui.weak(&self.producer_status);
        dirty
    }

    // =========================================================================
    // #60 (Deliver Hub): LAN 分散レンダーファーム
    // =========================================================================
    pub fn ui_render_farm_tab(&mut self, ui: &mut Ui, session: &SharedEditorSession) -> bool {
        let mut dirty = false;
        ui.heading("🖥 LAN 分散レンダーファーム（HMAC-SHA256 分割レンダリング＆自動結合）(#60)");
        ui.add_space(4.0);

        // 稼働中コーディネータを非ブロッキングポーリング
        if let Some(ref mut srv) = self.farm_coordinator {
            let n = srv.poll_once();
            if n > 0 {
                self.farm_status = format!(
                    "📡 LAN コーディネータ :{} 稼働中 (累計 {} リクエスト処理)",
                    srv.port, srv.requests_served
                );
            }
        }

        let mut cfg = {
            let g = session.lock().unwrap();
            g.project.render_farm.clone()
        };
        let mut changed = false;

        ui.horizontal_wrapped(|ui| {
            ui.label("総フレーム数:");
            if ui
                .add(egui::DragValue::new(&mut cfg.total_frames).range(30..=500_000))
                .changed()
            {
                changed = true;
            }
            if ui
                .button("📐 HMAC-SHA256 チャンク分割計画を再計算")
                .clicked()
            {
                cfg.enabled = true;
                cfg.chunks = plan_render_farm_chunks(&cfg);
                let ok = cfg
                    .chunks
                    .iter()
                    .all(|c| verify_chunk_hmac(&cfg.hmac_secret, c));
                self.farm_manifest_preview = build_farm_concat_manifest(&cfg.chunks);
                self.farm_status = format!(
                    "✅ {} チャンク分割完了（HMAC-SHA256 全検証: {}）",
                    cfg.chunks.len(),
                    if ok { "OK" } else { "NG" }
                );
                changed = true;
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("ノード追加:");
            ui.add(egui::TextEdit::singleline(&mut self.new_worker_id).desired_width(140.0));
            ui.add(egui::TextEdit::singleline(&mut self.new_worker_host).desired_width(110.0));
            ui.add(egui::DragValue::new(&mut self.new_worker_threads).suffix(" thr"));
            if ui.small_button("➕ ワーカー追加").clicked() {
                cfg.workers.push(RenderFarmWorkerNode {
                    worker_id: self.new_worker_id.clone(),
                    host: self.new_worker_host.clone(),
                    port: 19481,
                    cpu_threads: self.new_worker_threads,
                    weight: (self.new_worker_threads as f32 / 12.0).clamp(0.25, 4.0),
                    healthy: true,
                });
                cfg.chunks = plan_render_farm_chunks(&cfg);
                changed = true;
            }
        });

        ui.horizontal_wrapped(|ui| {
            let running = self.farm_coordinator.is_some();
            if ui
                .button(if running {
                    "⏹ コーディネータ停止"
                } else {
                    "🚀 ローカル LAN コーディネータ起動"
                })
                .clicked()
            {
                if running {
                    self.farm_coordinator = None;
                    self.farm_status = "コーディネータを停止しました".into();
                } else {
                    match RenderFarmCoordinatorServer::bind_localhost(&cfg) {
                        Ok(srv) => {
                            self.farm_status = format!(
                                "✅ LAN コーディネータ稼働中 on 127.0.0.1:{} ({} チャンク待機)",
                                srv.port,
                                srv.chunks.len()
                            );
                            self.farm_coordinator = Some(srv);
                        }
                        Err(e) => self.farm_status = format!("❌ 起動失敗: {e}"),
                    }
                }
            }

            if ui.small_button("🧪 ループバックワーカー検証 (Bearer + HMAC)").clicked() {
                let mut test_cfg = cfg.clone();
                test_cfg.coordinator_port = 0;
                test_cfg.chunks = plan_render_farm_chunks(&test_cfg);
                if let Ok(mut srv) = RenderFarmCoordinatorServer::bind_localhost(&test_cfg) {
                    let port = srv.port;
                    let tok = test_cfg.bearer_token.clone();
                    let th = std::thread::spawn(move || {
                        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
                            let req = format!(
                                "POST /api/v1/claim HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {tok}\r\nContent-Length: 0\r\n\r\n"
                            );
                            let _ = s.write_all(req.as_bytes());
                            let mut resp = String::new();
                            let _ = s.read_to_string(&mut resp);
                            return resp.contains("200 OK");
                        }
                        false
                    });
                    for _ in 0..20 {
                        if srv.poll_once() > 0 {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    let ok = th.join().unwrap_or(false);
                    self.farm_status = if ok {
                        format!("✅ ループバックワーカー認証＆チャンク取得成功 (port :{port})")
                    } else {
                        "❌ ループバック検証失敗".into()
                    };
                }
            }
        });

        ui.group(|ui| {
            ui.label(
                RichText::new(format!(
                    "登録ワーカー ({} 台) / 割当チャンク ({} 件)",
                    cfg.workers.len(),
                    cfg.chunks.len()
                ))
                .strong(),
            );
            for ch in &cfg.chunks {
                let sig_short: String = ch.hmac_signature.chars().take(12).collect();
                ui.monospace(format!(
                    "Chunk #{:02} [{}..{}) → {:18} | {:?} | HMAC:{sig_short}...",
                    ch.chunk_id, ch.start_frame, ch.end_frame, ch.worker_id, ch.status
                ));
            }
            if !self.farm_manifest_preview.is_empty() {
                ui.collapsing(
                    "📄 FFmpeg concat 無劣化結合マニフェスト",
                    |ui| {
                        ui.monospace(&self.farm_manifest_preview);
                    },
                );
            }
        });

        if changed {
            let mut g = session.lock().unwrap();
            let s = &mut *g;
            if s.bus
                .execute(
                    &mut s.project,
                    EditCommand::SetRenderFarm { render_farm: cfg },
                    CommandSource::Gui,
                    None,
                )
                .is_ok()
            {
                dirty = true;
            }
        }

        ui.weak(&self.farm_status);
        dirty
    }
}
