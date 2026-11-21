//! v0.9 (#14〜#16) 専用 UI パネル群:
//! - #14 Pure-Rust エクスプレッションエディタ（リアルタイム評価・エラー表示・プリセット・Bake）
//! - #15 モーショングラフィックス テンプレートギャラリー＆公開パラメータ編集＆プロジェクト雛形
//! - #16 パーティクルエミッター / ベクターシェイプ（トリムパス・グラデーション・破線）/ 3D ライト＆モーションブラー UI

use egui::{Color32, Ui};
use std::collections::BTreeMap;
use ve_core::{
    CommandSource, CubicBezier, EXPRESSION_PROPERTIES, EditCommand, ExposedParamKind,
    Interpolation, Keyframe, Light3DType, ParticleConfig, ParticlePreset, Project, ProjectTemplate,
    RationalTime, RgbaColor, TemplateBinding, TemplateParamValue, TimelineItem, VectorFillMode,
    VectorShapeConfig, build_bake_expression_command, build_instantiate_template_command,
    builtin_motion_templates, builtin_project_templates, compile_expression,
    evaluate_item_property, expression_presets, extract_motion_template_from_scene,
};
use ve_mcp::EditorSession;

pub struct V09PanelsState {
    // #14 エクスプレッションエディタ状態
    pub expr_property: String,
    pub expr_draft: String,
    pub expr_synced_item: Option<(uuid::Uuid, String)>,
    pub expr_bake_step: u32,
    pub expr_status: String,

    // #15 テンプレート状態
    pub tpl_save_name: String,
    pub tpl_save_category: String,
    pub tpl_save_intro: f64,
    pub tpl_save_outro: f64,
    pub proj_tpl_save_name: String,
    pub tpl_status: String,
}

impl Default for V09PanelsState {
    fn default() -> Self {
        Self {
            expr_property: "x".to_string(),
            expr_draft: String::new(),
            expr_synced_item: None,
            expr_bake_step: 3,
            expr_status: String::new(),
            tpl_save_name: "マイテンプレート".to_string(),
            tpl_save_category: "カスタム".to_string(),
            tpl_save_intro: 0.45,
            tpl_save_outro: 0.45,
            proj_tpl_save_name: "マイプロジェクト雛形".to_string(),
            tpl_status: String::new(),
        }
    }
}

/// テンプレート操作によってアプリ全体の状態（キャッシュやプロジェクト差し替え）を更新する必要があるか返す
#[derive(Debug, Default)]
pub struct V09PanelAction {
    pub dirty: bool,
    pub replaced_project: Option<Project>,
}

impl V09PanelsState {
    /// #14: 選択アイテムのエクスプレッション（式アニメーション）編集 UI
    pub fn show_expression_editor_ui(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        layer_idx: usize,
        item: &TimelineItem,
    ) -> bool {
        let mut dirty = false;
        let active_count = item.expressions.len();
        let header = if active_count > 0 {
            format!("✨ エクスプレッション (式アニメーション: {active_count} 件適用中 / v0.9)")
        } else {
            "✨ エクスプレッション (式アニメーション / v0.9)".to_string()
        };

        egui::CollapsingHeader::new(header)
            .id_salt("v09_expression_editor")
            .default_open(active_count > 0)
            .show(ui, |ui| {
                // 適用中式バッジ一覧（ワンクリックで選択・削除）
                if !item.expressions.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.small("適用中:");
                        for (prop, expr) in &item.expressions {
                            let label = format!("fx:{prop}");
                            if ui
                                .small_button(label)
                                .on_hover_text(expr)
                                .clicked()
                            {
                                self.expr_property = prop.clone();
                                self.expr_draft = expr.clone();
                                self.expr_synced_item = Some((item.id, prop.clone()));
                            }
                        }
                    });
                }

                // 選択アイテム／プロパティが変わったら下書きを同期
                let sync_key = (item.id, self.expr_property.clone());
                if self.expr_synced_item.as_ref() != Some(&sync_key) {
                    self.expr_draft = item
                        .expressions
                        .get(&self.expr_property)
                        .cloned()
                        .unwrap_or_default();
                    self.expr_synced_item = Some(sync_key);
                }

                ui.horizontal(|ui| {
                    ui.label("対象プロパティ:");
                    egui::ComboBox::from_id_salt("v09_expr_prop_combo")
                        .selected_text(&self.expr_property)
                        .show_ui(ui, |ui| {
                            for &p in EXPRESSION_PROPERTIES {
                                let has_fx = item.expressions.contains_key(p);
                                let text = if has_fx {
                                    format!("fx {p}")
                                } else {
                                    p.to_string()
                                };
                                if ui
                                    .selectable_value(&mut self.expr_property, p.to_string(), text)
                                    .changed()
                                {
                                    self.expr_draft = item
                                        .expressions
                                        .get(p)
                                        .cloned()
                                        .unwrap_or_default();
                                    self.expr_synced_item = Some((item.id, p.to_string()));
                                }
                            }
                        });

                    // プリセットスニペット挿入メニュー
                    egui::ComboBox::from_id_salt("v09_expr_preset_combo")
                        .selected_text("📖 スニペット挿入")
                        .show_ui(ui, |ui| {
                            for (name, rec_prop, code) in expression_presets() {
                                if ui
                                    .selectable_label(false, format!("{name} ({rec_prop})"))
                                    .on_hover_text(code)
                                    .clicked()
                                {
                                    self.expr_property = rec_prop.to_string();
                                    self.expr_draft = code.to_string();
                                    self.expr_synced_item = Some((item.id, rec_prop.to_string()));
                                }
                            }
                        });
                });

                ui.add(
                    egui::TextEdit::multiline(&mut self.expr_draft)
                        .code_editor()
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("例: value + wiggle(2.5, 18)  /  loopOut(\"pingpong\")  /  layer(\"L0\").x + 100"),
                );

                // リアルタイム構文検証＆現在値プレビュー
                let trimmed = self.expr_draft.trim().to_string();
                let syntax_ok = if trimmed.is_empty() {
                    ui.weak("式を入力して「適用」を押すとアニメーションが有効になります。");
                    true
                } else {
                    match compile_expression(&trimmed) {
                        Ok(_) => {
                            let preview_val = session.project.active_scene().map(|scene| {
                                let mut tmp = item.clone();
                                tmp.expressions
                                    .insert(self.expr_property.clone(), trimmed.to_string());
                                evaluate_item_property(
                                    scene,
                                    layer_idx,
                                    &tmp,
                                    &self.expr_property,
                                    session.playhead,
                                    [0.0; 4],
                                )
                            });
                            if let Some(v) = preview_val {
                                ui.colored_label(
                                    Color32::from_rgb(120, 230, 150),
                                    format!(
                                        "✔ 構文OK / t={:.2}s での評価値: {:.3}",
                                        session.playhead.as_seconds(),
                                        v
                                    ),
                                );
                            }
                            true
                        }
                        Err(err) => {
                            ui.colored_label(
                                Color32::from_rgb(255, 110, 110),
                                format!("✖ 構文エラー: {err}"),
                            );
                            false
                        }
                    }
                };

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(syntax_ok && !trimmed.is_empty(), egui::Button::new("✔ 式を適用"))
                        .clicked()
                    {
                        match session.bus.execute(
                            &mut session.project,
                            EditCommand::SetExpression {
                                item_id: item.id,
                                property: self.expr_property.clone(),
                                expression: Some(trimmed.to_string()),
                            },
                            CommandSource::Gui,
                            None,
                        ) {
                            Ok(_) => {
                                self.expr_status =
                                    format!("{} に式を適用しました", self.expr_property);
                                dirty = true;
                            }
                            Err(e) => self.expr_status = format!("適用失敗: {e}"),
                        }
                    }
                    if item.expressions.contains_key(&self.expr_property)
                        && ui.button("✖ 式を解除").clicked()
                        && session
                            .bus
                            .execute(
                                &mut session.project,
                                EditCommand::SetExpression {
                                    item_id: item.id,
                                    property: self.expr_property.clone(),
                                    expression: None,
                                },
                                CommandSource::Gui,
                                None,
                            )
                            .is_ok()
                    {
                        self.expr_draft.clear();
                        self.expr_status =
                            format!("{} の式を解除しました", self.expr_property);
                        dirty = true;
                    }
                    if item.expressions.contains_key(&self.expr_property) {
                        ui.add(
                            egui::DragValue::new(&mut self.expr_bake_step)
                                .range(1..=30)
                                .prefix("間隔:")
                                .suffix("f"),
                        );
                        if ui
                            .button("🔥 キーフレームへ焼き込み (Bake)")
                            .on_hover_text(
                                "式の軌跡を標準キーフレームへ変換し、1ステップ Undo で式を解除します",
                            )
                            .clicked()
                            && let Some(scene) = session.project.active_scene().cloned()
                        {
                            match build_bake_expression_command(
                                &scene,
                                layer_idx,
                                item,
                                &self.expr_property,
                                self.expr_bake_step,
                                |_| [0.0; 4],
                            ) {
                                Ok(cmd) => {
                                    if session
                                        .bus
                                        .execute(&mut session.project, cmd, CommandSource::Gui, None)
                                        .is_ok()
                                    {
                                        self.expr_draft.clear();
                                        self.expr_status = format!(
                                            "{} をキーフレームへ焼き込みました",
                                            self.expr_property
                                        );
                                        dirty = true;
                                    }
                                }
                                Err(e) => self.expr_status = format!("Bake 失敗: {e}"),
                            }
                        }
                    }
                });

                if !self.expr_status.is_empty() {
                    ui.weak(&self.expr_status);
                }
            });
        dirty
    }

    /// #15: 選択中のテンプレート Precomp アイテムの公開パラメータ専用編集パネル
    pub fn show_template_instance_ui(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        item_id: uuid::Uuid,
        binding: &TemplateBinding,
    ) -> bool {
        let mut dirty = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong(format!("🧩 テンプレート編集: {}", binding.template_name));
            ui.small(format!(
                "レスポンシブ保護: Intro {:.2}s / Outro {:.2}s（中間のみ自動伸縮）",
                binding.responsive_time.intro_sec, binding.responsive_time.outro_sec
            ));

            let mut values = binding.values.clone();
            let mut dur = binding.current_duration_sec;
            let mut changed = false;
            let mut dur_changed = false;

            ui.horizontal(|ui| {
                ui.label("テンプレート尺 (秒):");
                if ui
                    .add(
                        egui::Slider::new(&mut dur, 0.8..=30.0)
                            .fixed_decimals(2)
                            .suffix("s"),
                    )
                    .changed()
                {
                    dur_changed = true;
                }
            });

            for param in &binding.exposed_params {
                let current = values
                    .get(&param.id)
                    .cloned()
                    .unwrap_or_else(|| param.default_value());
                match (&param.kind, current) {
                    (ExposedParamKind::Text { .. }, TemplateParamValue::Text(mut s)) => {
                        ui.horizontal(|ui| {
                            ui.label(&param.label);
                            if ui.text_edit_singleline(&mut s).changed() {
                                values.insert(param.id.clone(), TemplateParamValue::Text(s));
                                changed = true;
                            }
                        });
                    }
                    (ExposedParamKind::Color { .. }, TemplateParamValue::Color(col)) => {
                        let mut rgba = [col.r, col.g, col.b, col.a];
                        ui.horizontal(|ui| {
                            ui.label(&param.label);
                            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                                values.insert(
                                    param.id.clone(),
                                    TemplateParamValue::Color(RgbaColor {
                                        r: rgba[0],
                                        g: rgba[1],
                                        b: rgba[2],
                                        a: rgba[3],
                                    }),
                                );
                                changed = true;
                            }
                        });
                    }
                    (
                        ExposedParamKind::FontSize { min, max, .. }
                        | ExposedParamKind::Number { min, max, .. },
                        TemplateParamValue::Number(mut n),
                    ) => {
                        ui.horizontal(|ui| {
                            ui.label(&param.label);
                            if ui.add(egui::Slider::new(&mut n, *min..=*max)).changed() {
                                values.insert(param.id.clone(), TemplateParamValue::Number(n));
                                changed = true;
                            }
                        });
                    }
                    _ => {}
                }
            }

            if (changed || dur_changed)
                && session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::UpdateTemplateInstance {
                            item_id,
                            values,
                            duration_sec: if dur_changed { Some(dur) } else { None },
                        },
                        CommandSource::Gui,
                        Some("template_instance_edit"),
                    )
                    .is_ok()
            {
                dirty = true;
            }
        });
        dirty
    }

    /// #15: モーショングラフィックス テンプレートギャラリー＆プロジェクト雛形 UI
    pub fn show_template_gallery_ui(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
    ) -> V09PanelAction {
        let mut action = V09PanelAction::default();
        egui::CollapsingHeader::new(
            "🧩 テンプレートギャラリー (.mogrt 相当) ＆ プロジェクト雛形 (v0.9)",
        )
        .id_salt("v09_template_gallery")
        .default_open(false)
        .show(ui, |ui| {
            ui.strong("モーショングラフィックス テンプレート");
            let mut templates = builtin_motion_templates();
            let builtin_count = templates.len();
            if let Some(folder) = session.folder.as_ref() {
                templates.extend(folder.load_motion_templates());
            }

            for (idx, tpl) in templates.iter().enumerate() {
                let is_custom = idx >= builtin_count;
                ui.horizontal_wrapped(|ui| {
                    let tag = if is_custom { "★" } else { "📦" };
                    ui.strong(format!("{tag} {}", tpl.name));
                    ui.small(format!(
                        "[{}] {:.1}s / 公開{}項目",
                        tpl.category,
                        tpl.responsive_time.default_duration_sec,
                        tpl.exposed_params.len()
                    ));
                    if ui
                        .small_button("＋ タイムラインへ配置")
                        .on_hover_text(&tpl.description)
                        .clicked()
                    {
                        let target_layer = session
                            .project
                            .active_scene()
                            .map(|s| 1.min(s.layers.len().saturating_sub(1)))
                            .unwrap_or(0);
                        let cmd = build_instantiate_template_command(
                            tpl,
                            target_layer,
                            session.playhead,
                            None,
                            &BTreeMap::new(),
                        );
                        if session
                            .bus
                            .execute(&mut session.project, cmd, CommandSource::Gui, None)
                            .is_ok()
                        {
                            if let Some(new_id) = session
                                .project
                                .active_scene()
                                .and_then(|s| s.layers.get(target_layer))
                                .and_then(|l| l.items.last())
                                .map(|i| i.id)
                            {
                                session.selected_items = vec![new_id];
                            }
                            self.tpl_status = format!("「{}」を配置しました", tpl.name);
                            action.dirty = true;
                        }
                    }
                    if is_custom
                        && ui
                            .small_button("🗑")
                            .on_hover_text("カスタムテンプレートを削除")
                            .clicked()
                        && let Some(folder) = session.folder.as_ref()
                    {
                        let _ = folder.delete_motion_template(&tpl.name);
                        self.tpl_status = format!("「{}」を削除しました", tpl.name);
                    }
                });
            }

            ui.separator();
            ui.small("現在のシーンをテンプレートとして保存（templates/*.json）:");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.tpl_save_name);
                ui.add(
                    egui::DragValue::new(&mut self.tpl_save_intro)
                        .range(0.0..=5.0)
                        .speed(0.05)
                        .prefix("Intro:")
                        .suffix("s"),
                );
                ui.add(
                    egui::DragValue::new(&mut self.tpl_save_outro)
                        .range(0.0..=5.0)
                        .speed(0.05)
                        .prefix("Outro:")
                        .suffix("s"),
                );
                if ui.button("💾 保存").clicked() {
                    if let (Some(folder), Some(scene)) =
                        (session.folder.as_ref(), session.project.active_scene())
                    {
                        let tpl = extract_motion_template_from_scene(
                            scene,
                            self.tpl_save_name.trim(),
                            self.tpl_save_category.trim(),
                            self.tpl_save_intro,
                            self.tpl_save_outro,
                        );
                        match folder.save_motion_template(&tpl) {
                            Ok(_) => {
                                self.tpl_status = format!(
                                    "テンプレート「{}」を保存しました（公開 {} 項目）",
                                    tpl.name,
                                    tpl.exposed_params.len()
                                );
                            }
                            Err(e) => self.tpl_status = format!("保存エラー: {e}"),
                        }
                    } else {
                        self.tpl_status =
                            "先にプロジェクトフォルダを開くと templates/ へ保存できます".into();
                    }
                }
            });

            ui.separator();
            ui.strong("📄 プロジェクトテンプレートから新規作成");
            let mut proj_templates = builtin_project_templates();
            if let Some(folder) = session.folder.as_ref() {
                proj_templates.extend(folder.load_project_templates());
            }
            for pt in &proj_templates {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&pt.name);
                    if ui
                        .small_button("この雛形で新規作成")
                        .on_hover_text(&pt.description)
                        .clicked()
                    {
                        action.replaced_project = Some(pt.project.clone());
                        action.dirty = true;
                        self.tpl_status = format!("プロジェクト雛形「{}」を適用しました", pt.name);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.proj_tpl_save_name);
                if ui.button("💾 現プロジェクトを雛形保存").clicked() {
                    if let Some(folder) = session.folder.as_ref() {
                        let pt = ProjectTemplate {
                            id: format!(
                                "proj_custom_{}",
                                &uuid::Uuid::new_v4().simple().to_string()[..6]
                            ),
                            name: self.proj_tpl_save_name.trim().to_string(),
                            description: "ユーザー保存プロジェクトテンプレート".into(),
                            project: session.project.clone(),
                        };
                        match folder.save_project_template(&pt) {
                            Ok(_) => {
                                self.tpl_status =
                                    format!("プロジェクト雛形「{}」を保存しました", pt.name);
                            }
                            Err(e) => self.tpl_status = format!("保存エラー: {e}"),
                        }
                    } else {
                        self.tpl_status =
                            "先にプロジェクトフォルダを開くと雛形を保存できます".into();
                    }
                }
            });

            if !self.tpl_status.is_empty() {
                ui.colored_label(Color32::LIGHT_GREEN, &self.tpl_status);
            }
        });
        action
    }

    /// #16: パーティクルエミッター専用インスペクタ
    pub fn show_particle_inspector_ui(
        ui: &mut Ui,
        session: &mut EditorSession,
        item_id: uuid::Uuid,
        mut cfg: ParticleConfig,
    ) -> bool {
        let prev = cfg;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("✨ 決定論的 2D/3D パーティクル設定 (v0.9)");
            egui::ComboBox::from_id_salt("v09_particle_preset")
                .selected_text(cfg.preset.display_name())
                .show_ui(ui, |ui| {
                    for &p in ParticlePreset::ALL {
                        if ui
                            .selectable_label(cfg.preset == p, p.display_name())
                            .clicked()
                        {
                            cfg = ParticleConfig::for_preset(p);
                        }
                    }
                });

            ui.horizontal(|ui| {
                ui.label("シード:");
                ui.add(egui::DragValue::new(&mut cfg.seed));
                if ui.small_button("🎲 ランダム").clicked() {
                    cfg.seed = cfg
                        .seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                }
            });
            ui.add(egui::Slider::new(&mut cfg.count, 1..=500).text("パーティクル数"));
            ui.add(egui::Slider::new(&mut cfg.speed, 0.0..=2500.0).text("速度 (px/s)"));
            ui.add(egui::Slider::new(&mut cfg.size_px, 1.0..=100.0).text("サイズ (px)"));
            ui.add(egui::Slider::new(&mut cfg.gravity, -400.0..=800.0).text("重力"));
            ui.add(egui::Slider::new(&mut cfg.wind_x, -400.0..=400.0).text("横風"));
            ui.add(egui::Slider::new(&mut cfg.lifetime_sec, 0.2..=12.0).text("寿命 (秒)"));
            ui.add(egui::Slider::new(&mut cfg.depth_3d, 0.0..=1.0).text("3D 奥行き視差"));

            ui.horizontal(|ui| {
                let mut c1 = [
                    cfg.color_primary.r,
                    cfg.color_primary.g,
                    cfg.color_primary.b,
                    cfg.color_primary.a,
                ];
                let mut c2 = [
                    cfg.color_secondary.r,
                    cfg.color_secondary.g,
                    cfg.color_secondary.b,
                    cfg.color_secondary.a,
                ];
                ui.label("主色:");
                if ui.color_edit_button_rgba_unmultiplied(&mut c1).changed() {
                    cfg.color_primary = RgbaColor {
                        r: c1[0],
                        g: c1[1],
                        b: c1[2],
                        a: c1[3],
                    };
                }
                ui.label("副色:");
                if ui.color_edit_button_rgba_unmultiplied(&mut c2).changed() {
                    cfg.color_secondary = RgbaColor {
                        r: c2[0],
                        g: c2[1],
                        b: c2[2],
                        a: c2[3],
                    };
                }
            });
        });

        if cfg != prev {
            return session
                .bus
                .execute(
                    &mut session.project,
                    EditCommand::SetParticleConfig {
                        item_id,
                        config: cfg,
                    },
                    CommandSource::Gui,
                    Some("particle_edit"),
                )
                .is_ok();
        }
        false
    }

    /// #16: 自由ベジェベクターシェイプ専用インスペクタ（グラデーション塗り・破線・トリムパス）
    pub fn show_vector_shape_inspector_ui(
        ui: &mut Ui,
        session: &mut EditorSession,
        item_id: uuid::Uuid,
        mut cfg: VectorShapeConfig,
    ) -> bool {
        let prev = cfg.clone();
        let mut dirty = false;

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong("✒ 自由ベジェ ベクターシェイプ (v0.9)");
            ui.horizontal_wrapped(|ui| {
                ui.small("形状:");
                if ui.small_button("★ 星").clicked() {
                    cfg = VectorShapeConfig::star();
                }
                if ui.small_button("➡ 矢印").clicked() {
                    cfg = VectorShapeConfig::arrow();
                }
                if ui.small_button("〰 ウェーブ線").clicked() {
                    cfg = VectorShapeConfig::underline_wave(cfg.stroke_color);
                }
                if ui.small_button("💬 吹き出し").clicked() {
                    cfg = VectorShapeConfig::speech_bubble();
                }
                if ui.small_button("⭕ 破線リング").clicked() {
                    cfg = VectorShapeConfig::circle_ring();
                }
            });

            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut cfg.width).range(16.0..=4096.0).prefix("W:"));
                ui.add(egui::DragValue::new(&mut cfg.height).range(16.0..=4096.0).prefix("H:"));
                ui.checkbox(&mut cfg.closed, "パスを閉じる");
            });

            // 塗りつぶしモード
            let fill_label = match cfg.fill {
                VectorFillMode::None => "なし",
                VectorFillMode::Solid(_) => "単色",
                VectorFillMode::LinearGradient { .. } => "線形グラデーション",
            };
            egui::ComboBox::from_label("塗り")
                .selected_text(fill_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(matches!(cfg.fill, VectorFillMode::None), "なし")
                        .clicked()
                    {
                        cfg.fill = VectorFillMode::None;
                    }
                    if ui
                        .selectable_label(matches!(cfg.fill, VectorFillMode::Solid(_)), "単色")
                        .clicked()
                    {
                        cfg.fill = VectorFillMode::Solid(RgbaColor {
                            r: 0.25,
                            g: 0.65,
                            b: 1.0,
                            a: 0.95,
                        });
                    }
                    if ui
                        .selectable_label(
                            matches!(cfg.fill, VectorFillMode::LinearGradient { .. }),
                            "線形グラデーション",
                        )
                        .clicked()
                    {
                        cfg.fill = VectorFillMode::default();
                    }
                });

            match &mut cfg.fill {
                VectorFillMode::None => {}
                VectorFillMode::Solid(col) => {
                    let mut c = [col.r, col.g, col.b, col.a];
                    ui.horizontal(|ui| {
                        ui.label("塗り色:");
                        if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                            *col = RgbaColor {
                                r: c[0],
                                g: c[1],
                                b: c[2],
                                a: c[3],
                            };
                        }
                    });
                }
                VectorFillMode::LinearGradient {
                    start_color,
                    end_color,
                    angle_deg,
                } => {
                    ui.horizontal(|ui| {
                        let mut c1 = [start_color.r, start_color.g, start_color.b, start_color.a];
                        let mut c2 = [end_color.r, end_color.g, end_color.b, end_color.a];
                        ui.label("開始色:");
                        if ui.color_edit_button_rgba_unmultiplied(&mut c1).changed() {
                            *start_color = RgbaColor {
                                r: c1[0],
                                g: c1[1],
                                b: c1[2],
                                a: c1[3],
                            };
                        }
                        ui.label("終了色:");
                        if ui.color_edit_button_rgba_unmultiplied(&mut c2).changed() {
                            *end_color = RgbaColor {
                                r: c2[0],
                                g: c2[1],
                                b: c2[2],
                                a: c2[3],
                            };
                        }
                    });
                    ui.add(egui::Slider::new(angle_deg, 0.0..=360.0).text("グラデ角度 (°)"));
                }
            }

            // ストローク・破線
            ui.horizontal(|ui| {
                let mut sc = [
                    cfg.stroke_color.r,
                    cfg.stroke_color.g,
                    cfg.stroke_color.b,
                    cfg.stroke_color.a,
                ];
                ui.label("線色:");
                if ui.color_edit_button_rgba_unmultiplied(&mut sc).changed() {
                    cfg.stroke_color = RgbaColor {
                        r: sc[0],
                        g: sc[1],
                        b: sc[2],
                        a: sc[3],
                    };
                }
                ui.add(egui::Slider::new(&mut cfg.stroke_width, 0.0..=64.0).text("線幅"));
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut cfg.dash_length)
                        .range(0.0..=200.0)
                        .prefix("破線長:"),
                );
                ui.add(
                    egui::DragValue::new(&mut cfg.dash_gap)
                        .range(0.0..=200.0)
                        .prefix("隙間:"),
                );
            });

            // トリムパス（手描き線演出）
            ui.separator();
            ui.small("トリムパス (Trim Paths — グラフ/式で trim_start / trim_end / trim_offset をアニメ可):");
            ui.add(egui::Slider::new(&mut cfg.trim_start, 0.0..=1.0).text("トリム開始"));
            ui.add(egui::Slider::new(&mut cfg.trim_end, 0.0..=1.0).text("トリム終了"));
            ui.add(egui::Slider::new(&mut cfg.trim_offset, -1.0..=1.0).text("トリムオフセット"));
            if ui
                .button("✨ 手描き登場アニメ (0→1 / 0.8s) を自動設定")
                .clicked()
            {
                let cmd = EditCommand::Batch {
                    label: "trim_path_reveal".into(),
                    commands: vec![
                        EditCommand::SetKeyframe {
                            item_id,
                            property: "trim_end".into(),
                            keyframe: Keyframe {
                                time: RationalTime::ZERO,
                                value: 0.0,
                                interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
                            },
                        },
                        EditCommand::SetKeyframe {
                            item_id,
                            property: "trim_end".into(),
                            keyframe: Keyframe {
                                time: RationalTime::from_seconds(0.8),
                                value: 1.0,
                                interpolation: Interpolation::Linear,
                            },
                        },
                    ],
                };
                if session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None)
                    .is_ok()
                {
                    dirty = true;
                }
            }
        });

        if cfg != prev
            && session
                .bus
                .execute(
                    &mut session.project,
                    EditCommand::SetVectorShape {
                        item_id,
                        config: cfg,
                    },
                    CommandSource::Gui,
                    Some("vector_shape_edit"),
                )
                .is_ok()
        {
            dirty = true;
        }
        dirty
    }

    /// #16: シーン全体の 3D ライト＆シャドウキャッチャー ＋ シャッター角度モーションブラー UI
    pub fn show_scene_lighting_and_motion_blur_ui(
        ui: &mut Ui,
        session: &mut EditorSession,
    ) -> bool {
        let mut dirty = false;
        let Some((mut cam, mut mb)) = session
            .project
            .active_scene()
            .map(|s| (s.camera, s.motion_blur))
        else {
            return false;
        };
        let prev_cam = cam;
        let prev_mb = mb;

        egui::CollapsingHeader::new("💡 3D ライト＆影 ＆ モーションブラー (v0.9)")
            .id_salt("v09_light_and_motion_blur")
            .default_open(cam.light.enabled || mb.enabled)
            .show(ui, |ui| {
                // シャッター角度モーションブラー
                ui.strong("🎞 シャッター角度モーションブラー");
                ui.checkbox(&mut mb.enabled, "モーションブラー有効（サブフレーム合成）");
                if mb.enabled {
                    ui.add(
                        egui::Slider::new(&mut mb.shutter_angle_deg, 0.0..=360.0)
                            .text("シャッター角度 (°)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut mb.shutter_phase_deg, -180.0..=180.0)
                            .text("シャッター位相 (°)"),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut mb.preview_samples)
                                .range(1..=8)
                                .prefix("プレビュー分割:"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut mb.export_samples)
                                .range(1..=16)
                                .prefix("書き出し分割:"),
                        );
                    });
                }

                ui.separator();

                // 3D ディレクショナル／ポイントライト＆ドロップシャドウキャッチャー
                ui.strong("💡 3D カメラライト ＆ シャドウキャッチャー");
                if ui
                    .checkbox(
                        &mut cam.light.enabled,
                        "3D ライト有効（3Dレイヤーに照明＆影を適用）",
                    )
                    .changed()
                    && cam.light.enabled
                {
                    cam.enabled = true;
                }
                if cam.light.enabled {
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut cam.light.light_type,
                            Light3DType::Directional,
                            "平行光 (Directional)",
                        );
                        ui.selectable_value(
                            &mut cam.light.light_type,
                            Light3DType::Point,
                            "点光源 (Point)",
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("光源位置/方向:");
                        ui.add(egui::DragValue::new(&mut cam.light.position[0]).prefix("X:"));
                        ui.add(egui::DragValue::new(&mut cam.light.position[1]).prefix("Y:"));
                        ui.add(egui::DragValue::new(&mut cam.light.position[2]).prefix("Z:"));
                    });
                    ui.add(egui::Slider::new(&mut cam.light.intensity, 0.0..=3.0).text("光量"));
                    ui.add(egui::Slider::new(&mut cam.light.ambient, 0.0..=1.2).text("環境光"));
                    ui.checkbox(
                        &mut cam.light.cast_shadow,
                        "ドロップシャドウキャッチャー有効",
                    );
                    if cam.light.cast_shadow {
                        ui.add(
                            egui::Slider::new(&mut cam.light.shadow_softness, 0.0..=24.0)
                                .text("影のぼかし (px)"),
                        );
                        ui.add(
                            egui::Slider::new(&mut cam.light.shadow_opacity, 0.0..=1.0)
                                .text("影の濃さ"),
                        );
                    }
                }
            });

        if mb != prev_mb
            && session
                .bus
                .execute(
                    &mut session.project,
                    EditCommand::SetMotionBlur { motion_blur: mb },
                    CommandSource::Gui,
                    Some("motion_blur_edit"),
                )
                .is_ok()
        {
            dirty = true;
        }
        if cam != prev_cam
            && session
                .bus
                .execute(
                    &mut session.project,
                    EditCommand::SetCamera3D { camera: cam },
                    CommandSource::Gui,
                    Some("light_3d_edit"),
                )
                .is_ok()
        {
            dirty = true;
        }
        dirty
    }
}
