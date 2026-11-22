use egui::{Color32, Pos2, Rect, Ui, Vec2};
use std::collections::BTreeMap;
use uuid::Uuid;
use ve_audio::{AudioGraph, AudioMeterSnapshot, linear_to_db, measure_integrated_lufs};
use ve_core::{
    AudioEffectNode, AudioReactiveBand, BezierMask, CommandSource, CubicBezier, EditCommand,
    EffectNode, EffectPreset, FilterTargetScope, Interpolation, ItemId, ItemKind, Keyframe,
    MaskMode, MaskVertex, MotionPreset, MotionTrackConfig, RationalTime, RgbaColor, TextStyle,
    TextStylePreset, TimelineItem, TransitionKind, build_motion_preset_commands,
};
use ve_mcp::EditorSession;
use ve_plugin_api::{PluginManifest, PluginRegistry, WasmPluginManager, emit_sample_wasm_plugins};

/// インスペクタ拡張機能の状態（エフェクトクリップボード・プリセット・カラーパレット・制御文字補完・LUFS・トラッカー）
pub struct InspectorExtrasState {
    pub copied_effects: Vec<EffectNode>,
    pub new_palette_name: String,
    pub new_palette_rgb: [f32; 3],
    pub new_text_preset_name: String,
    pub new_effect_preset_name: String,
    pub preset_status: String,
    pub show_tag_completion: bool,
    pub measured_lufs: Option<f32>,
    // v0.7 #8: モーショントラッカー設定
    pub track_anchor_x: f32,
    pub track_anchor_y: f32,
    pub track_duration_sec: f64,
    pub track_step_frames: u32,
    pub track_patch_radius: u32,
    pub track_search_radius: u32,
    pub track_status: String,
    // v0.7 #9: WASM プラグイン操作メッセージ
    pub wasm_status: String,
}

impl Default for InspectorExtrasState {
    fn default() -> Self {
        Self {
            copied_effects: Vec::new(),
            new_palette_name: "カスタム色".to_string(),
            new_palette_rgb: [1.0, 0.45, 0.15],
            new_text_preset_name: "マイ字幕スタイル".to_string(),
            new_effect_preset_name: "マイエフェクト".to_string(),
            preset_status: String::new(),
            show_tag_completion: false,
            measured_lufs: None,
            track_anchor_x: 640.0,
            track_anchor_y: 360.0,
            track_duration_sec: 2.0,
            track_step_frames: 2,
            track_patch_radius: 12,
            track_search_radius: 28,
            track_status: "待機中".to_string(),
            wasm_status: "plugins/*.wasm 監視中".to_string(),
        }
    }
}

/// リアルタイム ステレオ VU / ピーク / LUFS メーターバー（タイムライン再生バー常駐用）
pub fn draw_audio_meter_bar(ui: &mut Ui, snap: AudioMeterSnapshot) {
    let w = 136.0;
    let h = 20.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(20, 22, 28));

    let bar_x0 = rect.left() + 4.0;
    let bar_w = rect.width() - 56.0;
    let draw_ch = |y: f32, ch_h: f32, peak: f32, rms: f32| {
        let bg = Rect::from_min_size(Pos2::new(bar_x0, y), Vec2::new(bar_w, ch_h));
        painter.rect_filled(bg, 1.5, Color32::from_rgb(35, 38, 46));
        let rms_frac = ((linear_to_db(rms) + 60.0) / 60.0).clamp(0.0, 1.0);
        let peak_frac = ((linear_to_db(peak) + 60.0) / 60.0).clamp(0.0, 1.0);
        let fill_color = if peak >= 0.995 {
            Color32::from_rgb(240, 56, 56)
        } else if peak >= 0.7 {
            Color32::from_rgb(236, 198, 52)
        } else {
            Color32::from_rgb(64, 208, 112)
        };
        if rms_frac > 0.01 {
            let fg = Rect::from_min_size(Pos2::new(bar_x0, y), Vec2::new(bar_w * rms_frac, ch_h));
            painter.rect_filled(fg, 1.5, fill_color);
        }
        if peak_frac > 0.01 {
            let px = bar_x0 + bar_w * peak_frac;
            painter.line_segment(
                [Pos2::new(px, y), Pos2::new(px, y + ch_h)],
                egui::Stroke::new(2.0, Color32::WHITE),
            );
        }
    };

    draw_ch(rect.top() + 3.0, 6.0, snap.peak_l, snap.rms_l);
    draw_ch(rect.top() + 11.0, 6.0, snap.peak_r, snap.rms_r);

    let lufs_txt = if snap.momentary_lufs <= -69.0 {
        "-∞".to_string()
    } else {
        format!("{:.1}LU", snap.momentary_lufs)
    };
    let txt_color = if snap.clipped {
        Color32::from_rgb(255, 85, 85)
    } else {
        Color32::from_gray(210)
    };
    painter.text(
        Pos2::new(rect.right() - 4.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        lufs_txt,
        egui::FontId::monospace(9.5),
        txt_color,
    );
}

/// A-1: AviUtl2 名前付きカラーパレット（複数選択オブジェクト一括色変更対応）
pub fn show_color_palette_ui(
    ui: &mut Ui,
    extras: &mut InspectorExtrasState,
    session: &mut EditorSession,
) {
    ui.collapsing(
        "🎨 カラーパレット（一括色変更 / <@名前>参照）",
        |ui| {
            let palette = session.project.color_palette.clone();
            let selected_ids = session.selected_items.clone();
            let mut apply_color: Option<RgbaColor> = None;
            let mut remove_name: Option<String> = None;

            ui.horizontal_wrapped(|ui| {
                for nc in &palette.colors {
                    let c32 = rgba_to_c32(nc.color);
                    let btn =
                        egui::Button::new(egui::RichText::new(&nc.name).color(contrast_text(c32)))
                            .fill(c32)
                            .small();
                    let resp = ui.add(btn).on_hover_text(format!(
                        "{} (クリックで選択中{}件へ適用)",
                        nc.color.to_hex_rgb(),
                        selected_ids.len()
                    ));
                    if resp.clicked() {
                        apply_color = Some(nc.color);
                    }
                    if resp.secondary_clicked() {
                        remove_name = Some(nc.name.clone());
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut extras.new_palette_name)
                        .desired_width(95.0)
                        .hint_text("色名"),
                );
                ui.color_edit_button_rgb(&mut extras.new_palette_rgb);
                if ui.small_button("＋ パレット追加").clicked()
                    && !extras.new_palette_name.trim().is_empty()
                {
                    let mut next = palette.clone();
                    next.add_or_update(
                        extras.new_palette_name.trim(),
                        RgbaColor {
                            r: extras.new_palette_rgb[0],
                            g: extras.new_palette_rgb[1],
                            b: extras.new_palette_rgb[2],
                            a: 1.0,
                        },
                    );
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetColorPalette { palette: next },
                        CommandSource::Gui,
                        None,
                    );
                }
            });

            if let Some(color) = apply_color
                && !selected_ids.is_empty()
            {
                let cmds: Vec<EditCommand> = selected_ids
                    .into_iter()
                    .map(|item_id| EditCommand::SetItemColor { item_id, color })
                    .collect();
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::Batch {
                        label: "apply_palette_color".into(),
                        commands: cmds,
                    },
                    CommandSource::Gui,
                    None,
                );
            }

            if let Some(name) = remove_name {
                let mut next = palette;
                if next.remove(&name) {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetColorPalette { palette: next },
                        CommandSource::Gui,
                        None,
                    );
                }
            }
        },
    );
}

/// A-2: AviUtl2 インラインテキスト制御文字の入力補助 (`Ctrl+Space` 補完ポップアップ対応)
pub fn show_inline_tag_helper(
    ui: &mut Ui,
    extras: &mut InspectorExtrasState,
    session: &EditorSession,
    content: &mut String,
) {
    let ctrl_space = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Space));
    if ctrl_space {
        extras.show_tag_completion = !extras.show_tag_completion;
    }

    ui.horizontal_wrapped(|ui| {
        ui.weak("制御文字:");
        for (label, snippet) in [
            ("赤", "<#ff3333>"),
            ("黄", "<#ffe033>"),
            ("大字", "<s68>"),
            ("太字", "<b1>"),
            ("解除", "</>"),
        ] {
            if ui.small_button(label).clicked() {
                content.push_str(snippet);
            }
        }
        ui.toggle_value(&mut extras.show_tag_completion, "✨ 補完 (Ctrl+Space)");
    });

    if extras.show_tag_completion {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.weak("カラーパレットタグ (<@名前>):");
            ui.horizontal_wrapped(|ui| {
                for nc in &session.project.color_palette.colors {
                    let c32 = rgba_to_c32(nc.color);
                    let btn = egui::Button::new(
                        egui::RichText::new(format!("<@{}>", nc.name)).color(contrast_text(c32)),
                    )
                    .fill(c32)
                    .small();
                    if ui.add(btn).clicked() {
                        content.push_str(&format!("<@{}>", nc.name));
                    }
                }
            });
            if !session.project.favorite_fonts.is_empty() {
                ui.weak("お気に入りフォントタグ (<f名前>):");
                ui.horizontal_wrapped(|ui| {
                    for f in &session.project.favorite_fonts {
                        if ui.small_button(format!("<f{f}>")).clicked() {
                            content.push_str(&format!("<f{f}>"));
                        }
                    }
                });
            }
        });
    }
}

/// 字幕リッチ装飾スタイル（縁取り・ドロップシャドウ・座布団背景・スタイルプリセット管理）
#[allow(clippy::too_many_arguments)]
pub fn show_text_style_ui(
    ui: &mut Ui,
    extras: &mut InspectorExtrasState,
    session: &mut EditorSession,
    item_id: ItemId,
    font_family: &str,
    font_size: f32,
    text_color: RgbaColor,
    mut style: TextStyle,
) {
    let prev = style;
    let mut apply_preset: Option<TextStylePreset> = None;
    let mut delete_saved_name: Option<String> = None;

    ui.collapsing(
        "✨ 字幕装飾＆スタイルプリセット（縁取り・シャドウ・座布団）",
        |ui| {
            // 1. 組み込み＋プロジェクト保存済み字幕スタイルプリセット (presets/text_styles/*.json)
            ui.weak("スタイルプリセット（1クリック適用 / 保存済みは右クリック削除）:");
            let saved_presets = session
                .folder
                .as_ref()
                .map(|f| f.load_text_style_presets())
                .unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                for preset in TextStylePreset::default_presets() {
                    if ui
                        .small_button(format!("★ {}", preset.name))
                        .on_hover_text("標準スタイルを適用 (Undo可)")
                        .clicked()
                    {
                        apply_preset = Some(preset);
                    }
                }
                for preset in saved_presets {
                    let resp = ui
                        .small_button(format!("💾 {}", preset.name))
                        .on_hover_text("保存済みスタイルを適用（右クリックで削除）");
                    if resp.clicked() {
                        apply_preset = Some(preset.clone());
                    }
                    if resp.secondary_clicked() {
                        delete_saved_name = Some(preset.name);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut extras.new_text_preset_name)
                        .desired_width(120.0)
                        .hint_text("プリセット名"),
                );
                if ui.small_button("💾 現在スタイルを保存").clicked() {
                    let name = extras.new_text_preset_name.trim().to_string();
                    if name.is_empty() {
                        extras.preset_status = "プリセット名を入力してください".into();
                    } else if let Some(folder) = session.folder.as_ref() {
                        let preset = TextStylePreset {
                            name: name.clone(),
                            font_family: font_family.to_string(),
                            font_size,
                            color: text_color,
                            text_style: style,
                        };
                        match folder.save_text_style_preset(&preset) {
                            Ok(_) => {
                                extras.preset_status =
                                    format!("字幕スタイル「{name}」を presets/ へ保存しました");
                            }
                            Err(e) => {
                                extras.preset_status = format!("プリセット保存エラー: {e}");
                            }
                        }
                    } else {
                        extras.preset_status =
                            "先に上部バーからプロジェクトフォルダを作成/保存してください".into();
                    }
                }
            });
            if !extras.preset_status.is_empty() {
                ui.weak(&extras.preset_status);
            }
            ui.separator();

            // 2. 縁取り
            ui.add(egui::Slider::new(&mut style.outline_width, 0.0..=12.0).text("縁取り幅(px)"));
            if style.outline_width > 0.0 {
                let mut rgb = [
                    style.outline_color.r,
                    style.outline_color.g,
                    style.outline_color.b,
                ];
                ui.horizontal(|ui| {
                    ui.label("縁取り色:");
                    if ui.color_edit_button_rgb(&mut rgb).changed() {
                        style.outline_color.r = rgb[0];
                        style.outline_color.g = rgb[1];
                        style.outline_color.b = rgb[2];
                    }
                });
            }

            // 3. ドロップシャドウ（オフセット・ぼかし・影色RGBA）
            ui.horizontal(|ui| {
                ui.label("影オフセット X/Y:");
                ui.add(egui::DragValue::new(&mut style.shadow_offset[0]).speed(0.5));
                ui.add(egui::DragValue::new(&mut style.shadow_offset[1]).speed(0.5));
                ui.label("影色:");
                let mut srgba = [
                    style.shadow_color.r,
                    style.shadow_color.g,
                    style.shadow_color.b,
                    style.shadow_color.a,
                ];
                if ui.color_edit_button_rgba_unmultiplied(&mut srgba).changed() {
                    style.shadow_color = RgbaColor {
                        r: srgba[0],
                        g: srgba[1],
                        b: srgba[2],
                        a: srgba[3],
                    };
                }
            });
            ui.add(egui::Slider::new(&mut style.shadow_blur, 0.0..=10.0).text("影ぼかし"));

            // 4. 座布団背景（色RGBA・余白・角丸）
            ui.checkbox(&mut style.box_enabled, "座布団背景を有効化");
            if style.box_enabled {
                ui.horizontal_wrapped(|ui| {
                    ui.label("座布団色:");
                    let mut brgba = [
                        style.box_color.r,
                        style.box_color.g,
                        style.box_color.b,
                        style.box_color.a,
                    ];
                    if ui.color_edit_button_rgba_unmultiplied(&mut brgba).changed() {
                        style.box_color = RgbaColor {
                            r: brgba[0],
                            g: brgba[1],
                            b: brgba[2],
                            a: brgba[3],
                        };
                    }
                    ui.label("余白 X/Y:");
                    ui.add(egui::DragValue::new(&mut style.box_padding[0]).range(0.0..=80.0));
                    ui.add(egui::DragValue::new(&mut style.box_padding[1]).range(0.0..=60.0));
                    ui.label("角丸:");
                    ui.add(egui::DragValue::new(&mut style.box_radius).range(0.0..=40.0));
                });
            }
        },
    );

    if let Some(name) = delete_saved_name
        && let Some(folder) = session.folder.as_ref()
        && folder.delete_text_style_preset(&name).is_ok()
    {
        extras.preset_status = format!("字幕スタイル「{name}」を削除しました");
    }

    if let Some(preset) = apply_preset {
        let batch = EditCommand::Batch {
            label: format!("apply_text_style:{}", preset.name),
            commands: vec![
                EditCommand::SetTextFont {
                    item_id,
                    font_family: preset.font_family,
                    font_size: preset.font_size,
                },
                EditCommand::SetItemColor {
                    item_id,
                    color: preset.color,
                },
                EditCommand::SetTextStyle {
                    item_id,
                    style: preset.text_style,
                },
            ],
        };
        if session
            .bus
            .execute(&mut session.project, batch, CommandSource::Gui, None)
            .is_ok()
        {
            extras.preset_status = format!("字幕スタイル「{}」を適用しました", preset.name);
        }
    } else if style != prev {
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::SetTextStyle { item_id, style },
            CommandSource::Gui,
            Some("text_style_edit"),
        );
    }
}

/// A-4: トランジション設定 UI
pub fn show_transition_ui(
    ui: &mut Ui,
    session: &mut EditorSession,
    item_id: ItemId,
    mut transition: TransitionKind,
    mut softness: f32,
) {
    let prev = (transition, softness);
    ui.separator();
    ui.strong("🎞 シーンチェンジ設定");
    egui::ComboBox::from_label("トランジション種別")
        .selected_text(transition.display_name())
        .show_ui(ui, |ui| {
            for kind in TransitionKind::ALL {
                ui.selectable_value(&mut transition, kind, kind.display_name());
            }
        });
    ui.add(egui::Slider::new(&mut softness, 0.01..=0.45).text("境界ぼかし"));
    if (transition, softness) != prev {
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::SetTransition {
                item_id,
                transition,
                softness,
            },
            CommandSource::Gui,
            Some("transition_edit"),
        );
    }
}

/// v0.7.1: AviUtl2方式フィルタオブジェクト専用インスペクタ（種別・適用レイヤー範囲・パラメータ編集）
pub fn show_filter_object_ui(
    ui: &mut Ui,
    session: &mut EditorSession,
    item_id: ItemId,
    mut effect_type: String,
    mut scope: FilterTargetScope,
    mut params: BTreeMap<String, f32>,
    plugin_manifests: &[PluginManifest],
) {
    let prev_type = effect_type.clone();
    let prev_scope = scope;
    let prev_params = params.clone();

    ui.separator();
    ui.strong("🧪 フィルタオブジェクト設定 (AviUtl2方式)");

    type BuiltinFilterDef = (&'static str, &'static str, &'static [(&'static str, f32)]);
    let builtins: &[BuiltinFilterDef] = &[
        ("明るさ", "brightness", &[("gain", 1.25)]),
        ("ブラー", "blur", &[("radius", 4.0)]),
        ("グロー", "glow", &[("intensity", 1.2), ("threshold", 0.5)]),
        (
            "色調補正",
            "color_grade",
            &[("exposure", 0.2), ("contrast", 1.1), ("saturation", 1.15)],
        ),
        ("モザイク", "mosaic", &[("block_size", 14.0)]),
        (
            "振動",
            "shake",
            &[("amp_x", 12.0), ("amp_y", 8.0), ("freq", 22.0)],
        ),
        (
            "角丸クリップ",
            "rounded_clip",
            &[("radius", 32.0), ("margin", 0.0)],
        ),
        ("クリップ", "clip_rect", &[("left", 0.1), ("right", 0.1)]),
    ];

    let mut type_changed = false;
    egui::ComboBox::from_label("フィルタ種別")
        .selected_text(&effect_type)
        .show_ui(ui, |ui| {
            for (label, id, defaults) in builtins {
                if ui
                    .selectable_label(effect_type == *id, format!("{label} ({id})"))
                    .clicked()
                {
                    effect_type = (*id).to_string();
                    params = defaults
                        .iter()
                        .map(|(k, v)| ((*k).to_string(), *v))
                        .collect();
                    type_changed = true;
                }
            }
            for m in plugin_manifests {
                if ui
                    .selectable_label(effect_type == m.id, format!("🧩 {} ({})", m.name, m.id))
                    .clicked()
                {
                    effect_type = m.id.clone();
                    params = m
                        .params
                        .iter()
                        .map(|p| (p.key.clone(), p.default_value))
                        .collect();
                    type_changed = true;
                }
            }
        });

    let mut scope_mode = match scope {
        FilterTargetScope::ImmediateLayer => 0usize,
        FilterTargetScope::LayerRange { .. } => 1,
        FilterTargetScope::AllRemaining => 2,
    };
    let mut range_count = match scope {
        FilterTargetScope::LayerRange { count } => count.max(1),
        _ => 2,
    };
    ui.horizontal_wrapped(|ui| {
        ui.label("適用範囲:");
        ui.selectable_value(&mut scope_mode, 0, "直上1レイヤー");
        ui.selectable_value(&mut scope_mode, 1, "指定レイヤー数");
        ui.selectable_value(&mut scope_mode, 2, "以降の全レイヤー");
    });
    if scope_mode == 1 {
        ui.horizontal(|ui| {
            ui.label("対象レイヤー数:");
            ui.add(egui::DragValue::new(&mut range_count).range(1..=16));
        });
    }
    scope = match scope_mode {
        0 => FilterTargetScope::ImmediateLayer,
        1 => FilterTargetScope::LayerRange {
            count: range_count.clamp(1, 16),
        },
        _ => FilterTargetScope::AllRemaining,
    };

    if !params.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (k, v) in &mut params {
                ui.label(format!("{k}:"));
                ui.add(egui::DragValue::new(v).speed(0.05));
            }
        });
    }

    if type_changed || effect_type != prev_type || scope != prev_scope || params != prev_params {
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::SetFilterObject {
                item_id,
                effect_type,
                scope,
                params,
            },
            CommandSource::Gui,
            Some("filter_object_edit"),
        );
    }
}

/// v0.7.1: AviUtl2方式グループ制御オブジェクト専用インスペクタ
pub fn show_group_control_ui(
    ui: &mut Ui,
    session: &mut EditorSession,
    item_id: ItemId,
    mut target_layer_count: usize,
) {
    let prev = target_layer_count;
    ui.separator();
    ui.strong("🗂 グループ制御設定 (AviUtl2方式)");
    ui.weak("上の「Transform2D（X/Y/拡大率/回転/不透明度）」で対象レイヤー群を一括制御します。");
    ui.horizontal_wrapped(|ui| {
        ui.label("対象レイヤー数:");
        ui.add(egui::DragValue::new(&mut target_layer_count).range(1..=16));
        for n in [1usize, 2, 3, 5] {
            if ui.small_button(format!("{n}層")).clicked() {
                target_layer_count = n;
            }
        }
    });
    if target_layer_count != prev {
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::SetGroupControl {
                item_id,
                target_layer_count,
            },
            CommandSource::Gui,
            Some("group_control_edit"),
        );
    }
}

/// v0.7.1: オーディオスペクトラム ビジュアライザー専用インスペクタ
#[allow(clippy::too_many_arguments)]
pub fn show_audio_spectrum_ui(
    ui: &mut Ui,
    session: &mut EditorSession,
    item_id: ItemId,
    mut linked_audio_layer: usize,
    mut width: f32,
    mut height: f32,
    mut bar_count: usize,
    mut color: RgbaColor,
    mut peak_color: RgbaColor,
    mut sensitivity: f32,
) {
    let prev = (
        linked_audio_layer,
        width,
        height,
        bar_count,
        color,
        peak_color,
        sensitivity,
    );
    ui.separator();
    ui.strong("📊 オーディオスペクトラム設定");

    let layer_names: Vec<String> = session
        .project
        .active_scene()
        .map(|s| {
            s.layers
                .iter()
                .enumerate()
                .map(|(i, l)| format!("L{i}: {}", l.name))
                .collect()
        })
        .unwrap_or_default();
    let cur_layer = layer_names
        .get(linked_audio_layer)
        .cloned()
        .unwrap_or_else(|| format!("L{linked_audio_layer}"));
    egui::ComboBox::from_label("連動音声レイヤー")
        .selected_text(cur_layer)
        .show_ui(ui, |ui| {
            for (i, name) in layer_names.iter().enumerate() {
                ui.selectable_value(&mut linked_audio_layer, i, name);
            }
        });

    ui.horizontal_wrapped(|ui| {
        ui.label("幅:");
        ui.add(
            egui::DragValue::new(&mut width)
                .speed(4.0)
                .range(64.0..=3840.0),
        );
        ui.label("高さ:");
        ui.add(
            egui::DragValue::new(&mut height)
                .speed(4.0)
                .range(32.0..=2160.0),
        );
        ui.label("バー本数:");
        ui.add(egui::DragValue::new(&mut bar_count).range(4..=64));
    });
    ui.add(egui::Slider::new(&mut sensitivity, 0.2..=6.0).text("反応感度"));

    ui.horizontal_wrapped(|ui| {
        ui.label("基本色:");
        let mut c_arr = [color.r, color.g, color.b, color.a];
        if ui.color_edit_button_rgba_unmultiplied(&mut c_arr).changed() {
            color = RgbaColor {
                r: c_arr[0],
                g: c_arr[1],
                b: c_arr[2],
                a: c_arr[3],
            };
        }
        ui.label("ピーク色:");
        let mut p_arr = [peak_color.r, peak_color.g, peak_color.b, peak_color.a];
        if ui.color_edit_button_rgba_unmultiplied(&mut p_arr).changed() {
            peak_color = RgbaColor {
                r: p_arr[0],
                g: p_arr[1],
                b: p_arr[2],
                a: p_arr[3],
            };
        }
    });

    if (
        linked_audio_layer,
        width,
        height,
        bar_count,
        color,
        peak_color,
        sensitivity,
    ) != prev
    {
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::SetAudioSpectrum {
                item_id,
                linked_audio_layer,
                width,
                height,
                bar_count,
                color,
                peak_color,
                sensitivity,
            },
            CommandSource::Gui,
            Some("audio_spectrum_edit"),
        );
    }
}

/// A-5: 再生速度変更・逆再生・ループ再生・時間制御（タイムリマップ）UI
pub fn show_playback_speed_ui(ui: &mut Ui, session: &mut EditorSession, item: &TimelineItem) {
    if !matches!(item.kind, ItemKind::Video { .. } | ItemKind::Audio { .. }) {
        return;
    }
    ui.separator();
    ui.strong("⏩ 再生速度・ループ・時間制御 (AviUtl2)");
    let mut speed = item.playback.speed;
    let mut loop_pb = item.playback.loop_playback;
    ui.add(egui::Slider::new(&mut speed, -4.0..=4.0).text("再生倍率(負=逆再生)"));
    ui.horizontal_wrapped(|ui| {
        for (label, val) in [
            ("0.5x", 0.5f32),
            ("1.0x", 1.0),
            ("1.5x", 1.5),
            ("2.0x", 2.0),
            ("逆再生(-1x)", -1.0),
        ] {
            if ui.small_button(label).clicked() {
                speed = val;
            }
        }
        ui.checkbox(&mut loop_pb, "🔁 ループ再生");
    });
    if (speed - item.playback.speed).abs() > 1e-4 || loop_pb != item.playback.loop_playback {
        let _ = session.bus.execute(
            &mut session.project,
            EditCommand::SetPlaybackSpeed {
                item_id: item.id,
                speed,
                loop_playback: loop_pb,
            },
            CommandSource::Gui,
            Some("playback_speed_edit"),
        );
    }

    // 時間制御（タイムリマップ）キーフレームを現在位置へ打つクイックボタン
    let has_remap = item
        .keyframes
        .get("time_remap")
        .is_some_and(|v| !v.is_empty());
    ui.horizontal(|ui| {
        if ui.small_button("⏱ 時間制御KFを現在位置へ追加").clicked() {
            let local = (session.playhead - item.start).max(RationalTime::ZERO);
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetKeyframe {
                    item_id: item.id,
                    property: "time_remap".into(),
                    keyframe: Keyframe {
                        time: local,
                        value: local.as_seconds() as f32,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                    },
                },
                CommandSource::Gui,
                None,
            );
        }
        if has_remap {
            ui.weak("※グラフエディタで緩急カーブ編集可");
        }
    });
}

/// A-6 + B-7: 映像エフェクト追加・ドラッグ/ボタン並び替え・単体/一括コピペ・パラメータ編集・モーションプリセット
pub fn show_video_effects_and_presets_ui(
    ui: &mut Ui,
    extras: &mut InspectorExtrasState,
    session: &mut EditorSession,
    item: &TimelineItem,
) {
    ui.separator();
    ui.strong("✨ 映像エフェクト追加（標準＋実務定番）");
    type VideoEffectPreset = (&'static str, &'static str, &'static [(&'static str, f32)]);
    let presets: &[VideoEffectPreset] = &[
        ("ブラー", "blur", &[("radius", 3.0)]),
        ("グロー", "glow", &[("intensity", 1.2), ("threshold", 0.5)]),
        (
            "色調補正",
            "color_grade",
            &[("exposure", 0.25), ("contrast", 1.05), ("saturation", 1.1)],
        ),
        ("クリップ", "clip_rect", &[("left", 0.1), ("right", 0.1)]),
        (
            "クロマキー",
            "chroma_key",
            &[
                ("key_r", 0.0),
                ("key_g", 1.0),
                ("key_b", 0.0),
                ("tolerance", 0.35),
                ("softness", 0.15),
                ("spill_suppress", 0.4),
            ],
        ),
        (
            "ルミナンスキー",
            "luma_key",
            &[("threshold", 0.12), ("softness", 0.08), ("invert", 0.0)],
        ),
        ("モザイク", "mosaic", &[("block_size", 14.0)]),
        (
            "角丸クリップ",
            "rounded_clip",
            &[("radius", 32.0), ("margin", 0.0)],
        ),
        (
            "斜めクリップ",
            "diagonal_clip",
            &[("angle_deg", 30.0), ("width", 0.0), ("feather", 2.0)],
        ),
        (
            "振動",
            "shake",
            &[("amp_x", 14.0), ("amp_y", 10.0), ("freq", 24.0)],
        ),
        ("縁取り", "outline", &[("width", 3.0)]),
        (
            "影(シャドウ)",
            "drop_shadow",
            &[
                ("offset_x", 6.0),
                ("offset_y", 6.0),
                ("blur", 4.0),
                ("opacity", 0.6),
            ],
        ),
        (
            "AI自動切り抜き",
            "ai_segment",
            &[
                ("sensitivity", 0.55),
                ("edge_feather_px", 1.5),
                ("skin_boost", 0.65),
                ("despill_strength", 0.35),
            ],
        ),
        (
            "AI超解像復元",
            "ai_super_res",
            &[("sharpen", 0.55), ("denoise", 0.2)],
        ),
    ];

    ui.horizontal_wrapped(|ui| {
        for (label, eff_type, defaults) in presets {
            if ui.small_button(*label).clicked() {
                let mut params = BTreeMap::new();
                for (k, v) in *defaults {
                    params.insert((*k).to_string(), *v);
                }
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::AddEffect {
                        item_id: item.id,
                        effect: EffectNode {
                            id: Uuid::new_v4(),
                            effect_type: (*eff_type).to_string(),
                            enabled: true,
                            params,
                        },
                    },
                    CommandSource::Gui,
                    None,
                );
            }
        }
    });

    // フィルタコピペバー（A-6）
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !item.effect_chain.is_empty(),
                egui::Button::new("📋 全フィルタコピー").small(),
            )
            .clicked()
        {
            extras.copied_effects = item.effect_chain.clone();
        }
        if ui
            .add_enabled(
                !extras.copied_effects.is_empty(),
                egui::Button::new(format!("📥 貼付 ({}件)", extras.copied_effects.len())).small(),
            )
            .clicked()
        {
            let cloned: Vec<EffectNode> = extras
                .copied_effects
                .iter()
                .cloned()
                .map(|mut e| {
                    e.id = Uuid::new_v4();
                    e
                })
                .collect();
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::PasteEffects {
                    item_id: item.id,
                    effects: cloned,
                },
                CommandSource::Gui,
                None,
            );
        }
    });

    // v0.7.1: エフェクトチェーンプリセット管理 (presets/effects/*.json ＋ 標準チェーン)
    ui.collapsing(
        "📦 エフェクトチェーンプリセット (保存・1クリック適用)",
        |ui| {
            let mut apply_fx: Option<EffectPreset> = None;
            let mut delete_fx_name: Option<String> = None;
            let saved_fx = session
                .folder
                .as_ref()
                .map(|f| f.load_effect_presets())
                .unwrap_or_default();

            ui.horizontal_wrapped(|ui| {
                for preset in EffectPreset::default_presets() {
                    if ui
                        .small_button(format!("★ {}", preset.name))
                        .on_hover_text("標準エフェクトチェーンを適用 (Undo可)")
                        .clicked()
                    {
                        apply_fx = Some(preset);
                    }
                }
                for preset in saved_fx {
                    let resp = ui
                        .small_button(format!("💾 {}", preset.name))
                        .on_hover_text("保存済みエフェクトチェーンを適用（右クリックで削除）");
                    if resp.clicked() {
                        apply_fx = Some(preset.clone());
                    }
                    if resp.secondary_clicked() {
                        delete_fx_name = Some(preset.name);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut extras.new_effect_preset_name)
                        .desired_width(120.0)
                        .hint_text("チェーン名"),
                );
                if ui
                    .add_enabled(
                        !item.effect_chain.is_empty(),
                        egui::Button::new("💾 現チェーンを保存").small(),
                    )
                    .clicked()
                {
                    let name = extras.new_effect_preset_name.trim().to_string();
                    if name.is_empty() {
                        extras.preset_status = "プリセット名を入力してください".into();
                    } else if let Some(folder) = session.folder.as_ref() {
                        let preset = EffectPreset {
                            name: name.clone(),
                            effects: item.effect_chain.clone(),
                        };
                        match folder.save_effect_preset(&preset) {
                            Ok(_) => {
                                extras.preset_status = format!(
                                    "エフェクトプリセット「{name}」を presets/ へ保存しました"
                                );
                            }
                            Err(e) => {
                                extras.preset_status = format!("プリセット保存エラー: {e}");
                            }
                        }
                    } else {
                        extras.preset_status =
                            "先に上部バーからプロジェクトフォルダを作成/保存してください".into();
                    }
                }
            });
            if !extras.preset_status.is_empty() {
                ui.weak(&extras.preset_status);
            }

            if let Some(name) = delete_fx_name
                && let Some(folder) = session.folder.as_ref()
                && folder.delete_effect_preset(&name).is_ok()
            {
                extras.preset_status = format!("エフェクトプリセット「{name}」を削除しました");
            }
            if let Some(preset) = apply_fx {
                let effects = preset.instantiate();
                if session
                    .bus
                    .execute(
                        &mut session.project,
                        EditCommand::PasteEffects {
                            item_id: item.id,
                            effects,
                        },
                        CommandSource::Gui,
                        None,
                    )
                    .is_ok()
                {
                    extras.preset_status = format!("エフェクト「{}」を適用しました", preset.name);
                }
            }
        },
    );

    // 適用済みエフェクトチェーン（↑↓並び替え・ON/OFF・パラメータ編集・単体コピー・削除）
    let chain_len = item.effect_chain.len();
    for (idx, eff) in item.effect_chain.iter().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut enabled = eff.enabled;
                if ui.checkbox(&mut enabled, "").changed() {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::ToggleEffect {
                            item_id: item.id,
                            effect_id: eff.id,
                            enabled,
                        },
                        CommandSource::Gui,
                        None,
                    );
                }
                ui.strong(&eff.effect_type);
                if ui
                    .add_enabled(idx > 0, egui::Button::new("↑").small())
                    .clicked()
                {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::ReorderEffect {
                            item_id: item.id,
                            from_index: idx,
                            to_index: idx - 1,
                        },
                        CommandSource::Gui,
                        None,
                    );
                }
                if ui
                    .add_enabled(idx + 1 < chain_len, egui::Button::new("↓").small())
                    .clicked()
                {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::ReorderEffect {
                            item_id: item.id,
                            from_index: idx,
                            to_index: idx + 1,
                        },
                        CommandSource::Gui,
                        None,
                    );
                }
                if ui
                    .small_button("📋")
                    .on_hover_text("このフィルタをコピー")
                    .clicked()
                {
                    extras.copied_effects = vec![eff.clone()];
                }
                if ui.small_button("✖").clicked() {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::RemoveEffect {
                            item_id: item.id,
                            effect_id: eff.id,
                        },
                        CommandSource::Gui,
                        None,
                    );
                }
            });

            if !eff.params.is_empty() {
                let mut updated = eff.params.clone();
                let mut changed = false;
                ui.horizontal_wrapped(|ui| {
                    for (k, v) in &mut updated {
                        ui.label(format!("{k}:"));
                        if ui.add(egui::DragValue::new(v).speed(0.05)).changed() {
                            changed = true;
                        }
                    }
                });
                if changed {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::UpdateEffectParams {
                            item_id: item.id,
                            effect_id: eff.id,
                            params: updated,
                        },
                        CommandSource::Gui,
                        Some("effect_param_edit"),
                    );
                }
            }
        });
    }

    // B-7: 登場・退場イージング / 演出プリセット（ワンクリック適用）
    ui.horizontal_wrapped(|ui| {
        ui.weak("🎬 演出プリセット:");
        for preset in MotionPreset::ALL {
            if ui.small_button(preset.display_name()).clicked() {
                let cmd = build_motion_preset_commands(item, preset);
                let _ = session
                    .bus
                    .execute(&mut session.project, cmd, CommandSource::Gui, None);
            }
        }
    });
}

/// B-3: 音声 DSP エフェクトラック（クリップ単位 / シーンマスター共通）＋ ラウドネス正規化 (LUFS)
pub fn show_audio_dsp_and_lufs_ui(
    ui: &mut Ui,
    extras: &mut InspectorExtrasState,
    session: &mut EditorSession,
    selected_item: Option<&TimelineItem>,
) {
    ui.collapsing(
        "🎚 音声DSPエフェクト & ラウドネス(LUFS) (Utsbox相当)",
        |ui| {
            // 1. 選択クリップ用 DSP（Audio / Video 選択時）
            if let Some(item) = selected_item
                && matches!(item.kind, ItemKind::Audio { .. } | ItemKind::Video { .. })
            {
                ui.strong(format!("クリップ音声DSP: {}", item.name));
                show_dsp_chain_editor(ui, session, Some(item.id), &item.audio_effects);
                ui.separator();
            }

            // 2. シーンマスター用 DSP
            let (master_fx, target_lufs) = session
                .project
                .active_scene()
                .map(|s| (s.master_audio_effects.clone(), s.target_lufs))
                .unwrap_or_default();
            ui.strong("シーンマスター音声DSP（全体出力）");
            show_dsp_chain_editor(ui, session, None, &master_fx);

            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label("書き出しラウドネス正規化:");
                let mut choice = target_lufs;
                if ui.selectable_label(choice.is_none(), "OFF").clicked() {
                    choice = None;
                }
                if ui
                    .selectable_label(choice == Some(-14.0), "YouTube (-14 LUFS)")
                    .clicked()
                {
                    choice = Some(-14.0);
                }
                if ui
                    .selectable_label(choice == Some(-16.0), "Podcast (-16 LUFS)")
                    .clicked()
                {
                    choice = Some(-16.0);
                }
                if choice != target_lufs {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetTargetLufs {
                            target_lufs: choice,
                        },
                        CommandSource::Gui,
                        None,
                    );
                }
            });

            ui.horizontal(|ui| {
                if ui.small_button("📊 現在の全体LUFSを計測").clicked() {
                    let scene_id = session.project.active_scene_id;
                    let sr = session.project.settings.audio_sample_rate.max(8_000);
                    let dur = session
                        .project
                        .active_scene()
                        .map(|s| s.total_duration())
                        .unwrap_or(RationalTime::from_seconds(1.0))
                        .min(RationalTime::from_seconds(60.0));
                    let graph = AudioGraph::from_project_scene(
                        &session.project,
                        scene_id,
                        session.folder.as_ref(),
                        sr,
                    );
                    let pcm = graph.render_range(RationalTime::ZERO, dur);
                    extras.measured_lufs = Some(measure_integrated_lufs(&pcm, sr));
                }
                if let Some(lufs) = extras.measured_lufs {
                    ui.monospace(format!("Integrated: {lufs:.1} LUFS"));
                }
            });

            // v0.6 #4: BGM 自動ダッキング（サイドチェイン減衰）
            ui.separator();
            ui.strong("🦆 BGM 自動ダッキング（サイドチェイン音量減衰）");
            if let Some(scene) = session.project.active_scene() {
                let mut ducking = scene.audio_ducking;
                let prev_ducking = ducking;
                let layer_names: Vec<String> = scene
                    .layers
                    .iter()
                    .enumerate()
                    .map(|(i, l)| format!("L{i}: {}", l.name))
                    .collect();

                ui.checkbox(
                    &mut ducking.enabled,
                    "発話レイヤーの声に合わせて BGM を自動で下げる",
                );
                if ducking.enabled {
                    let voice_lbl = layer_names
                        .get(ducking.voice_layer_index)
                        .cloned()
                        .unwrap_or_else(|| format!("L{}", ducking.voice_layer_index));
                    egui::ComboBox::from_id_salt("duck_voice_layer")
                        .selected_text(voice_lbl)
                        .show_ui(ui, |ui| {
                            for (i, name) in layer_names.iter().enumerate() {
                                ui.selectable_value(&mut ducking.voice_layer_index, i, name);
                            }
                        })
                        .response
                        .on_hover_text("サイドチェイン入力（発話・声レイヤー）");
                    let bgm_lbl = layer_names
                        .get(ducking.bgm_layer_index)
                        .cloned()
                        .unwrap_or_else(|| format!("L{}", ducking.bgm_layer_index));
                    egui::ComboBox::from_id_salt("duck_bgm_layer")
                        .selected_text(bgm_lbl)
                        .show_ui(ui, |ui| {
                            for (i, name) in layer_names.iter().enumerate() {
                                ui.selectable_value(&mut ducking.bgm_layer_index, i, name);
                            }
                        })
                        .response
                        .on_hover_text("自動減衰させる対象（BGM レイヤー）");

                    ui.add(
                        egui::Slider::new(&mut ducking.reduction_db, -30.0..=-1.0)
                            .text("減衰量 (dB)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut ducking.threshold_rms, 0.005..=0.25)
                            .text("発話検出しきい値"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Attack(s):");
                        ui.add(
                            egui::DragValue::new(&mut ducking.attack_sec)
                                .speed(0.005)
                                .range(0.005..=0.5),
                        );
                        ui.label("Release(s):");
                        ui.add(
                            egui::DragValue::new(&mut ducking.release_sec)
                                .speed(0.01)
                                .range(0.02..=2.0),
                        );
                    });
                }
                if ducking != prev_ducking {
                    let _ = session.bus.execute(
                        &mut session.project,
                        EditCommand::SetAudioDucking { ducking },
                        CommandSource::Gui,
                        Some("audio_ducking_edit"),
                    );
                }
            }
        },
    );
}

/// v0.6 #4: オーディオリアクティブ（音声エネルギー連動アニメーション）設定 UI
pub fn show_audio_reactive_ui(ui: &mut Ui, session: &mut EditorSession, item: &TimelineItem) {
    if matches!(
        item.kind,
        ItemKind::Audio { .. }
            | ItemKind::FilterObject { .. }
            | ItemKind::Transition { .. }
            | ItemKind::GroupControl { .. }
    ) {
        return;
    }
    ui.collapsing(
        "🎵 オーディオリアクティブ（音連動バウンス＆拡大）",
        |ui| {
            let mut cfg = item.audio_reactive;
            let prev = cfg;
            let layer_names: Vec<String> = session
                .project
                .active_scene()
                .map(|s| {
                    s.layers
                        .iter()
                        .enumerate()
                        .map(|(i, l)| format!("L{i}: {}", l.name))
                        .collect()
                })
                .unwrap_or_default();

            ui.checkbox(&mut cfg.enabled, "音声に合わせて自動アニメーション");
            if cfg.enabled {
                let cur_layer = layer_names
                    .get(cfg.linked_audio_layer)
                    .cloned()
                    .unwrap_or_else(|| format!("L{}", cfg.linked_audio_layer));
                egui::ComboBox::from_label("連動音声レイヤー")
                    .selected_text(cur_layer)
                    .show_ui(ui, |ui| {
                        for (i, name) in layer_names.iter().enumerate() {
                            ui.selectable_value(&mut cfg.linked_audio_layer, i, name);
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("帯域:");
                    for (band, label) in [
                        (AudioReactiveBand::Rms, "全体RMS"),
                        (AudioReactiveBand::Low, "低音(キック)"),
                        (AudioReactiveBand::Mid, "中音(声)"),
                        (AudioReactiveBand::High, "高音(アタック)"),
                    ] {
                        ui.selectable_value(&mut cfg.band, band, label);
                    }
                });
                ui.add(egui::Slider::new(&mut cfg.sensitivity, 0.2..=6.0).text("反応感度"));
                ui.add(egui::Slider::new(&mut cfg.scale_amount, -0.5..=1.0).text("拡大率ブースト"));
                ui.add(
                    egui::Slider::new(&mut cfg.y_bounce_px, -160.0..=160.0)
                        .text("Yバウンス(px, 負=上へ跳ねる)"),
                );
                ui.add(egui::Slider::new(&mut cfg.opacity_amount, -0.8..=0.8).text("不透明度連動"));
            }
            if cfg != prev {
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::SetAudioReactive {
                        item_id: item.id,
                        reactive: cfg,
                    },
                    CommandSource::Gui,
                    Some("audio_reactive_edit"),
                );
            }
        },
    );
}

/// v0.7 #8: 自由ベジェマスク編集 UI（プリセット追加・加算/減算・反転・フェザー・頂点細分化）
pub fn show_bezier_masks_ui(ui: &mut Ui, session: &mut EditorSession, item: &TimelineItem) {
    if matches!(
        item.kind,
        ItemKind::Audio { .. }
            | ItemKind::FilterObject { .. }
            | ItemKind::Transition { .. }
            | ItemKind::GroupControl { .. }
    ) {
        return;
    }
    ui.collapsing(
        format!(
            "✂ 自由ベジェマスク ({}件) — プレビュー上で頂点ドラッグ可",
            item.masks.len()
        ),
        |ui| {
            let mut masks = item.masks.clone();
            let prev = masks.clone();

            ui.horizontal_wrapped(|ui| {
                if ui.small_button("＋ 楕円マスク").clicked() {
                    masks.push(BezierMask::ellipse());
                }
                if ui.small_button("＋ 矩形マスク").clicked() {
                    masks.push(BezierMask::rectangle());
                }
                if ui.small_button("＋ ひし形マスク").clicked() {
                    masks.push(BezierMask::diamond());
                }
            });

            let mut delete_idx: Option<usize> = None;
            for (idx, m) in masks.iter_mut().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut m.enabled, format!("#{} {}", idx + 1, m.name));
                        ui.selectable_value(&mut m.mode, MaskMode::Add, "加算");
                        ui.selectable_value(&mut m.mode, MaskMode::Subtract, "減算");
                        ui.checkbox(&mut m.inverted, "反転");
                        if ui
                            .small_button("＋頂点")
                            .on_hover_text("辺の中点に頂点を追加")
                            .clicked()
                            && m.vertices.len() >= 3
                            && m.vertices.len() < 24
                        {
                            let v0 = m.vertices[0];
                            let v1 = m.vertices[1];
                            m.vertices.insert(
                                1,
                                MaskVertex::corner((v0.x + v1.x) * 0.5, (v0.y + v1.y) * 0.5),
                            );
                        }
                        if ui.small_button("✖").clicked() {
                            delete_idx = Some(idx);
                        }
                    });
                    ui.add(egui::Slider::new(&mut m.feather_px, 0.0..=64.0).text("フェザー(px)"));
                    ui.add(egui::Slider::new(&mut m.opacity, 0.0..=1.0).text("不透明度"));
                });
            }
            if let Some(di) = delete_idx {
                masks.remove(di);
            }

            if masks != prev {
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::SetMasks {
                        item_id: item.id,
                        masks,
                    },
                    CommandSource::Gui,
                    Some("bezier_mask_ui_edit"),
                );
            }
        },
    );
}

/// v0.7 #8: 自動モーショントラッキング設定 UI（押下時に追跡リクエストを返す）
pub fn show_motion_tracker_ui(
    ui: &mut Ui,
    extras: &mut InspectorExtrasState,
    session: &EditorSession,
    item: &TimelineItem,
) -> Option<MotionTrackConfig> {
    if matches!(
        item.kind,
        ItemKind::Audio { .. }
            | ItemKind::FilterObject { .. }
            | ItemKind::Transition { .. }
            | ItemKind::GroupControl { .. }
    ) {
        return None;
    }
    let mut trigger: Option<MotionTrackConfig> = None;
    ui.collapsing(
        "🎯 自動モーショントラッキング（顔モザイク・テロップ追従）",
        |ui| {
            ui.weak("指定座標の模様を追跡し、このオブジェクトの X/Y キーフレームへ焼き込みます。");
            ui.horizontal_wrapped(|ui| {
                ui.label("追跡起点:");
                ui.add(egui::DragValue::new(&mut extras.track_anchor_x).prefix("X:"));
                ui.add(egui::DragValue::new(&mut extras.track_anchor_y).prefix("Y:"));
                if ui.small_button("現在位置をセット").clicked() {
                    let tf = item.eval_transform(session.playhead);
                    extras.track_anchor_x = tf.x + 60.0;
                    extras.track_anchor_y = tf.y + 40.0;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("追跡尺:");
                ui.add(
                    egui::DragValue::new(&mut extras.track_duration_sec)
                        .speed(0.1)
                        .range(0.2..=20.0)
                        .suffix("s"),
                );
                ui.label("間隔:");
                ui.add(
                    egui::DragValue::new(&mut extras.track_step_frames)
                        .range(1..=10)
                        .suffix("F"),
                );
                ui.label("探索半径:");
                ui.add(
                    egui::DragValue::new(&mut extras.track_search_radius)
                        .range(8..=80)
                        .suffix("px"),
                );
            });
            if ui
                .button("🎯 自動追跡を実行 → X/Y キーフレーム焼き込み (Undo可)")
                .clicked()
            {
                let start = session.playhead;
                let end = start + RationalTime::from_seconds(extras.track_duration_sec);
                trigger = Some(MotionTrackConfig {
                    start_time: start,
                    end_time: end,
                    step_frames: extras.track_step_frames,
                    anchor_x: extras.track_anchor_x,
                    anchor_y: extras.track_anchor_y,
                    patch_radius: extras.track_patch_radius,
                    search_radius: extras.track_search_radius,
                });
            }
            ui.weak(format!("トラッカー状態: {}", extras.track_status));
        },
    );
    trigger
}

/// v0.7 #9: WASM 動的プラグイン管理 UI（`plugins/*.wasm` ホットリロード＆適用）
pub fn show_wasm_plugins_ui(
    ui: &mut Ui,
    extras: &mut InspectorExtrasState,
    session: &mut EditorSession,
    selected_item_id: Option<ItemId>,
    wasm_mgr: &mut WasmPluginManager,
    registry: &mut PluginRegistry,
) -> bool {
    let mut dirty = false;
    ui.collapsing("🧩 WASM 動的プラグイン (plugins/*.wasm サンドボックス)", |ui| {
        if let Some(dir) = wasm_mgr.plugin_dir() {
            ui.weak(format!("監視フォルダ: {}", dir.display()));
        }
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("🔄 今すぐ再読込").clicked() {
                let events = wasm_mgr.poll_and_sync(registry);
                extras.wasm_status = if events.is_empty() {
                    "変更なし".into()
                } else {
                    dirty = true;
                    events.join(" / ")
                };
            }
            if ui.small_button("📦 サンプル .wasm 3種を生成＆読込").clicked()
                && let Some(dir) = wasm_mgr.plugin_dir().map(std::path::Path::to_path_buf)
            {
                match emit_sample_wasm_plugins(&dir) {
                    Ok(_) => {
                        let events = wasm_mgr.poll_and_sync(registry);
                        extras.wasm_status = format!(
                            "サンプルWASM読込完了 ({}件)",
                            events.len()
                        );
                        dirty = true;
                    }
                    Err(e) => extras.wasm_status = format!("生成エラー: {e}"),
                }
            }
        });

        let statuses = wasm_mgr.list_status();
        if statuses.is_empty() {
            ui.weak("plugins/*.wasm が未配置です。「サンプル .wasm 3種を生成＆読込」ですぐ試せます。");
        } else {
            for st in statuses {
                ui.horizontal_wrapped(|ui| {
                    if let Some(err) = &st.last_error {
                        ui.colored_label(Color32::from_rgb(245, 110, 110), format!("⚠ {}", st.path.display()));
                        ui.weak(err);
                    } else {
                        ui.strong(&st.name);
                        ui.weak(format!("({})", st.id));
                        if let Some(item_id) = selected_item_id
                            && ui.small_button("＋ 選択アイテムへ適用").clicked()
                        {
                            let mut params = BTreeMap::new();
                            params.insert("strength".into(), 1.0);
                            if session
                                .bus
                                .execute(
                                    &mut session.project,
                                    EditCommand::AddEffect {
                                        item_id,
                                        effect: EffectNode {
                                            id: Uuid::new_v4(),
                                            effect_type: st.id.clone(),
                                            enabled: true,
                                            params,
                                        },
                                    },
                                    CommandSource::Gui,
                                    None,
                                )
                                .is_ok()
                            {
                                dirty = true;
                            }
                        }
                    }
                });
            }
        }
        ui.weak(format!("WASM状態: {}", extras.wasm_status));
    });
    dirty
}

fn show_dsp_chain_editor(
    ui: &mut Ui,
    session: &mut EditorSession,
    item_id: Option<ItemId>,
    chain: &[AudioEffectNode],
) {
    ui.horizontal_wrapped(|ui| {
        for (label, eff_type) in [
            ("ローカット", "low_cut"),
            ("ハイカット", "high_cut"),
            ("3バンドEQ", "eq_3band"),
            ("コンプ", "compressor"),
            ("リミッター", "limiter"),
            ("ディレイ", "delay"),
            ("リバーブ", "reverb"),
            ("ラジオ風", "radio_voice"),
        ] {
            if ui.small_button(format!("＋{label}")).clicked() {
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::AddAudioEffect {
                        item_id,
                        effect: AudioEffectNode::with_defaults(eff_type),
                    },
                    CommandSource::Gui,
                    None,
                );
            }
        }
    });

    for eff in chain {
        ui.horizontal_wrapped(|ui| {
            let mut enabled = eff.enabled;
            let mut params = eff.params.clone();
            let mut changed = ui.checkbox(&mut enabled, &eff.effect_type).changed();
            for (k, v) in &mut params {
                ui.label(format!("{k}:"));
                if ui.add(egui::DragValue::new(v).speed(0.5)).changed() {
                    changed = true;
                }
            }
            if changed {
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::UpdateAudioEffect {
                        item_id,
                        effect_id: eff.id,
                        enabled,
                        params,
                    },
                    CommandSource::Gui,
                    Some("audio_dsp_edit"),
                );
            }
            if ui.small_button("✖").clicked() {
                let _ = session.bus.execute(
                    &mut session.project,
                    EditCommand::RemoveAudioEffect {
                        item_id,
                        effect_id: eff.id,
                    },
                    CommandSource::Gui,
                    None,
                );
            }
        });
    }
}

fn rgba_to_c32(c: RgbaColor) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    )
}

fn contrast_text(bg: Color32) -> Color32 {
    let luma = 0.299 * f32::from(bg.r()) + 0.587 * f32::from(bg.g()) + 0.114 * f32::from(bg.b());
    if luma > 145.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}
