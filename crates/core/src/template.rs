//! v0.9 #15: モーショングラフィックス テンプレート（`.mogrt` 相当）＆ プロジェクトテンプレート
//!
//! Precomp シーン内の公開パラメータ（テキスト・色・フォントサイズ・数値・アセット）と
//! レスポンシブ時間伸縮（イントロ／アウトロのアニメーション尺を保護し中間のみ伸縮）を提供する。

use crate::animation::{CubicBezier, Interpolation, Keyframe};
use crate::command::EditCommand;
use crate::project::{
    AssetId, ItemKind, Layer, Project, ProjectSettings, RgbaColor, Scene, ScenePreset, ShapeKind,
    TextAnimator, TextStyle, TimelineItem, VectorFillMode, VectorShapeConfig,
};
use crate::time::{FrameRate, RationalTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// テンプレート公開パラメータの種別と対象アイテム指定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExposedParamKind {
    Text {
        item_name: String,
        default: String,
    },
    Color {
        item_name: String,
        default: RgbaColor,
    },
    FontSize {
        item_name: String,
        default: f32,
        min: f32,
        max: f32,
    },
    Number {
        item_name: String,
        /// `"x"`, `"y"`, `"scale_x"`, `"scale_y"`, `"rotation_deg"`, `"opacity"`, `"speed"`
        property: String,
        default: f32,
        min: f32,
        max: f32,
    },
    Asset {
        item_name: String,
        default_asset_id: Option<AssetId>,
    },
}

/// テンプレート編集パネルに露出させる 1 パラメータ定義
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExposedParam {
    pub id: String,
    pub label: String,
    pub kind: ExposedParamKind,
}

/// 公開パラメータの実行時値
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TemplateParamValue {
    Text(String),
    Color(RgbaColor),
    Number(f32),
    Asset(Option<AssetId>),
}

impl ExposedParam {
    pub fn default_value(&self) -> TemplateParamValue {
        match &self.kind {
            ExposedParamKind::Text { default, .. } => TemplateParamValue::Text(default.clone()),
            ExposedParamKind::Color { default, .. } => TemplateParamValue::Color(*default),
            ExposedParamKind::FontSize { default, .. } => TemplateParamValue::Number(*default),
            ExposedParamKind::Number { default, .. } => TemplateParamValue::Number(*default),
            ExposedParamKind::Asset {
                default_asset_id, ..
            } => TemplateParamValue::Asset(*default_asset_id),
        }
    }
}

/// レスポンシブ時間伸縮（Intro / Outro 保護領域）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResponsiveTimeRegion {
    /// イントロ保護尺（秒）：この区間内のキーフレーム・開始時刻は伸縮しない
    pub intro_sec: f64,
    /// アウトロ保護尺（秒）：末尾からこの区間内のキーフレーム・終了時刻は尺変更に追従スライドする
    pub outro_sec: f64,
    /// テンプレート基準尺（秒）
    pub default_duration_sec: f64,
}

impl Default for ResponsiveTimeRegion {
    fn default() -> Self {
        Self {
            intro_sec: 0.45,
            outro_sec: 0.40,
            default_duration_sec: 4.0,
        }
    }
}

/// 再利用可能なモーショングラフィックス テンプレート（`templates/*.json` に永続化）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionTemplate {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    #[serde(default)]
    pub responsive_time: ResponsiveTimeRegion,
    #[serde(default)]
    pub exposed_params: Vec<ExposedParam>,
    pub scene: Scene,
}

/// タイムラインへ配置されたテンプレートインスタンス（Precomp アイテムに紐づけ）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateBinding {
    pub template_id: String,
    pub template_name: String,
    pub responsive_time: ResponsiveTimeRegion,
    /// 現在の内部 Precomp 基準尺（再伸縮時の参照元）
    pub current_duration_sec: f64,
    pub exposed_params: Vec<ExposedParam>,
    pub values: BTreeMap<String, TemplateParamValue>,
}

/// プロジェクト一式の雛形（`templates/projects/*.json` に永続化）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub project: Project,
}

/// レスポンシブ時間マッピング: イントロ `[0..intro]`・アウトロ `[orig-outro..orig]` を保護し中間のみ伸縮する
pub fn map_responsive_time(
    t: f64,
    orig_dur: f64,
    new_dur: f64,
    intro_sec: f64,
    outro_sec: f64,
) -> f64 {
    let orig = orig_dur.max(0.05);
    let target = new_dur.max(0.05);
    if (orig - target).abs() < 1e-6 {
        return t.clamp(0.0, target);
    }

    let protected_sum = (intro_sec.max(0.0) + outro_sec.max(0.0)).max(0.0);
    // 新尺が保護領域合計より短い場合は全体を比例スケーリングして崩れを防ぐ
    if protected_sum >= orig - 1e-4 || protected_sum >= target - 1e-4 {
        return (t * (target / orig)).clamp(0.0, target);
    }

    let intro = intro_sec.max(0.0);
    let outro = outro_sec.max(0.0);
    let orig_outro_start = orig - outro;
    let new_outro_start = target - outro;

    if t <= intro {
        t.max(0.0)
    } else if t >= orig_outro_start {
        (new_outro_start + (t - orig_outro_start)).clamp(0.0, target)
    } else {
        let orig_mid = (orig_outro_start - intro).max(1e-6);
        let new_mid = (new_outro_start - intro).max(1e-6);
        let u = (t - intro) / orig_mid;
        (intro + u * new_mid).clamp(0.0, target)
    }
}

/// シーン内の全アイテムおよび全キーフレームにレスポンシブ時間伸縮を適用する
pub fn apply_responsive_time_stretch(
    scene: &mut Scene,
    orig_dur_sec: f64,
    new_dur_sec: f64,
    intro_sec: f64,
    outro_sec: f64,
) {
    for layer in &mut scene.layers {
        for item in &mut layer.items {
            let old_start = item.start.as_seconds();
            let old_end = item.end().as_seconds();
            let new_start =
                map_responsive_time(old_start, orig_dur_sec, new_dur_sec, intro_sec, outro_sec);
            let new_end =
                map_responsive_time(old_end, orig_dur_sec, new_dur_sec, intro_sec, outro_sec);
            let new_dur = (new_end - new_start).max(0.05);

            for kfs in item.keyframes.values_mut() {
                for kf in kfs.iter_mut() {
                    let old_global = old_start + kf.time.as_seconds();
                    let new_global = map_responsive_time(
                        old_global,
                        orig_dur_sec,
                        new_dur_sec,
                        intro_sec,
                        outro_sec,
                    );
                    let new_local = (new_global - new_start).clamp(0.0, new_dur);
                    kf.time = RationalTime::from_seconds(new_local);
                }
                kfs.sort_by_key(|k| k.time);
            }

            item.start = RationalTime::from_seconds(new_start);
            item.duration = RationalTime::from_seconds(new_dur);
        }
    }
}

/// 公開パラメータ値を Precomp シーン内の該当アイテムへ反映する
pub fn apply_template_values_to_scene(
    scene: &mut Scene,
    params: &[ExposedParam],
    values: &BTreeMap<String, TemplateParamValue>,
) {
    for param in params {
        let val = values
            .get(&param.id)
            .cloned()
            .unwrap_or_else(|| param.default_value());
        match (&param.kind, val) {
            (ExposedParamKind::Text { item_name, .. }, TemplateParamValue::Text(text)) => {
                for item in find_items_by_name_mut(scene, item_name) {
                    if let ItemKind::Text { content, .. } = &mut item.kind {
                        *content = text.clone();
                    }
                }
            }
            (ExposedParamKind::Color { item_name, .. }, TemplateParamValue::Color(col)) => {
                for item in find_items_by_name_mut(scene, item_name) {
                    match &mut item.kind {
                        ItemKind::Text { color, .. } | ItemKind::Shape { color, .. } => {
                            *color = col;
                        }
                        ItemKind::VectorShape { config } => {
                            config.stroke_color = col;
                            if let VectorFillMode::Solid(c) = &mut config.fill {
                                *c = col;
                            }
                        }
                        ItemKind::ParticleEmitter { config } => {
                            config.color_primary = col;
                        }
                        _ => {}
                    }
                }
            }
            (
                ExposedParamKind::FontSize {
                    item_name,
                    min,
                    max,
                    ..
                },
                TemplateParamValue::Number(n),
            ) => {
                let clamped = n.clamp(*min, *max);
                for item in find_items_by_name_mut(scene, item_name) {
                    if let ItemKind::Text { font_size, .. } = &mut item.kind {
                        *font_size = clamped;
                    }
                }
            }
            (
                ExposedParamKind::Number {
                    item_name,
                    property,
                    min,
                    max,
                    ..
                },
                TemplateParamValue::Number(n),
            ) => {
                let clamped = n.clamp(*min, *max);
                for item in find_items_by_name_mut(scene, item_name) {
                    match property.as_str() {
                        "x" => item.transform.x = clamped,
                        "y" => item.transform.y = clamped,
                        "scale_x" => item.transform.scale_x = clamped,
                        "scale_y" => item.transform.scale_y = clamped,
                        "rotation_deg" => item.transform.rotation_deg = clamped,
                        "opacity" => item.transform.opacity = clamped.clamp(0.0, 1.0),
                        "speed" => item.playback.speed = clamped.clamp(0.1, 8.0),
                        _ => {}
                    }
                }
            }
            (ExposedParamKind::Asset { item_name, .. }, TemplateParamValue::Asset(Some(aid))) => {
                for item in find_items_by_name_mut(scene, item_name) {
                    match &mut item.kind {
                        ItemKind::Image { asset_id }
                        | ItemKind::Video { asset_id, .. }
                        | ItemKind::Audio { asset_id, .. } => {
                            *asset_id = aid;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn find_items_by_name_mut<'a>(scene: &'a mut Scene, name: &str) -> Vec<&'a mut TimelineItem> {
    scene
        .layers
        .iter_mut()
        .flat_map(|l| l.items.iter_mut())
        .filter(|i| i.name == name)
        .collect()
}

/// 任意のシーンからテキスト・図形・ベクターを自動検出し、再利用可能な `MotionTemplate` を生成する
pub fn extract_motion_template_from_scene(
    scene: &Scene,
    name: impl Into<String>,
    category: impl Into<String>,
    intro_sec: f64,
    outro_sec: f64,
) -> MotionTemplate {
    let name_s = name.into();
    let max_end_sec = scene
        .layers
        .iter()
        .flat_map(|l| l.items.iter())
        .map(|i| i.end().as_seconds())
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut exposed = Vec::new();
    let mut text_idx = 0usize;
    let mut shape_idx = 0usize;

    for layer in &scene.layers {
        for item in &layer.items {
            match &item.kind {
                ItemKind::Text {
                    content,
                    font_size,
                    color,
                    ..
                } => {
                    text_idx += 1;
                    exposed.push(ExposedParam {
                        id: format!("text_{text_idx}"),
                        label: format!("テキスト: {}", item.name),
                        kind: ExposedParamKind::Text {
                            item_name: item.name.clone(),
                            default: content.clone(),
                        },
                    });
                    exposed.push(ExposedParam {
                        id: format!("text_color_{text_idx}"),
                        label: format!("文字色: {}", item.name),
                        kind: ExposedParamKind::Color {
                            item_name: item.name.clone(),
                            default: *color,
                        },
                    });
                    exposed.push(ExposedParam {
                        id: format!("font_size_{text_idx}"),
                        label: format!("文字サイズ: {}", item.name),
                        kind: ExposedParamKind::FontSize {
                            item_name: item.name.clone(),
                            default: *font_size,
                            min: 14.0,
                            max: 180.0,
                        },
                    });
                }
                ItemKind::Shape { color, .. } => {
                    shape_idx += 1;
                    exposed.push(ExposedParam {
                        id: format!("shape_color_{shape_idx}"),
                        label: format!("図形色: {}", item.name),
                        kind: ExposedParamKind::Color {
                            item_name: item.name.clone(),
                            default: *color,
                        },
                    });
                }
                ItemKind::VectorShape { config } => {
                    shape_idx += 1;
                    exposed.push(ExposedParam {
                        id: format!("vector_color_{shape_idx}"),
                        label: format!("ライン色: {}", item.name),
                        kind: ExposedParamKind::Color {
                            item_name: item.name.clone(),
                            default: config.stroke_color,
                        },
                    });
                }
                _ => {}
            }
        }
    }

    let id_slug: String = name_s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let id = format!(
        "tpl_{}_{}",
        id_slug.trim_matches('_'),
        &Uuid::new_v4().simple().to_string()[..6]
    );

    MotionTemplate {
        id,
        name: name_s,
        category: category.into(),
        description: "シーンから自動抽出されたカスタムテンプレート".into(),
        responsive_time: ResponsiveTimeRegion {
            intro_sec: intro_sec.clamp(0.0, max_end_sec * 0.45),
            outro_sec: outro_sec.clamp(0.0, max_end_sec * 0.45),
            default_duration_sec: max_end_sec,
        },
        exposed_params: exposed,
        scene: scene.clone(),
    }
}

/// テンプレートを新規 Precomp シーン＋アクティブシーン上の Precomp アイテムとして 1 ステップ Undo で配置するコマンドを生成
pub fn build_instantiate_template_command(
    template: &MotionTemplate,
    layer_index: usize,
    start: RationalTime,
    duration_sec: Option<f64>,
    overrides: &BTreeMap<String, TemplateParamValue>,
) -> EditCommand {
    let target_dur = duration_sec
        .unwrap_or(template.responsive_time.default_duration_sec)
        .max(0.3);

    // 独立した ID で Precomp シーンを複製
    let mut inst_scene = template.scene.clone();
    inst_scene.id = Uuid::new_v4();
    inst_scene.name = format!("[TPL] {}", template.name);
    for layer in &mut inst_scene.layers {
        layer.id = Uuid::new_v4();
        for item in &mut layer.items {
            item.id = Uuid::new_v4();
        }
    }

    let mut merged_values = BTreeMap::new();
    for p in &template.exposed_params {
        merged_values.insert(
            p.id.clone(),
            overrides
                .get(&p.id)
                .cloned()
                .unwrap_or_else(|| p.default_value()),
        );
    }

    apply_template_values_to_scene(&mut inst_scene, &template.exposed_params, &merged_values);
    apply_responsive_time_stretch(
        &mut inst_scene,
        template.responsive_time.default_duration_sec,
        target_dur,
        template.responsive_time.intro_sec,
        template.responsive_time.outro_sec,
    );

    let precomp_scene_id = inst_scene.id;
    let mut precomp_item = TimelineItem::new(
        format!("🧩 {}", template.name),
        start,
        RationalTime::from_seconds(target_dur),
        ItemKind::Precomp {
            scene_id: precomp_scene_id,
        },
    );
    precomp_item.template_binding = Some(TemplateBinding {
        template_id: template.id.clone(),
        template_name: template.name.clone(),
        responsive_time: template.responsive_time,
        current_duration_sec: target_dur,
        exposed_params: template.exposed_params.clone(),
        values: merged_values,
    });

    EditCommand::Batch {
        label: format!("instantiate_template:{}", template.id),
        commands: vec![
            EditCommand::CreateScene {
                scene: inst_scene,
                switch_to: false,
            },
            EditCommand::AddObject {
                layer_index,
                item: precomp_item,
            },
        ],
    }
}

/// 組み込みモーショングラフィックス テンプレート（即配置可能な標準 3 種）
pub fn builtin_motion_templates() -> Vec<MotionTemplate> {
    vec![
        make_lower_third_news_template(),
        make_kinetic_title_template(),
        make_subscribe_cta_template(),
    ]
}

fn make_lower_third_news_template() -> MotionTemplate {
    let dur = 4.0_f64;
    let mut scene = Scene::new("ニュース風ローワーサード", 1920, 1080, FrameRate::FPS_60);
    scene.layers.clear();

    let accent_color = RgbaColor {
        r: 0.14,
        g: 0.68,
        b: 0.98,
        a: 1.0,
    };
    let bg_color = RgbaColor {
        r: 0.08,
        g: 0.11,
        b: 0.17,
        a: 0.88,
    };

    let mut bg = TimelineItem::new(
        "帯背景",
        RationalTime::ZERO,
        RationalTime::from_seconds(dur),
        ItemKind::Shape {
            shape: ShapeKind::RoundedRectangle,
            width: 780.0,
            height: 126.0,
            color: bg_color,
        },
    );
    bg.transform.x = 96.0;
    bg.transform.y = 850.0;
    bg.keyframes.insert(
        "x".into(),
        vec![
            Keyframe {
                time: RationalTime::ZERO,
                value: -780.0,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
            },
            Keyframe {
                time: RationalTime::from_seconds(0.45),
                value: 96.0,
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: RationalTime::from_seconds(3.60),
                value: 96.0,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
            },
            Keyframe {
                time: RationalTime::from_seconds(4.0),
                value: -780.0,
                interpolation: Interpolation::Linear,
            },
        ],
    );

    let mut bar = TimelineItem::new(
        "アクセントバー",
        RationalTime::ZERO,
        RationalTime::from_seconds(dur),
        ItemKind::Shape {
            shape: ShapeKind::Rectangle,
            width: 14.0,
            height: 126.0,
            color: accent_color,
        },
    );
    bar.transform.x = 96.0;
    bar.transform.y = 850.0;
    bar.keyframes.insert(
        "opacity".into(),
        vec![
            Keyframe {
                time: RationalTime::ZERO,
                value: 0.0,
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: RationalTime::from_seconds(0.35),
                value: 1.0,
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: RationalTime::from_seconds(3.65),
                value: 1.0,
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: RationalTime::from_seconds(4.0),
                value: 0.0,
                interpolation: Interpolation::Linear,
            },
        ],
    );

    let mut title = TimelineItem::new(
        "メイン見出し",
        RationalTime::from_seconds(0.15),
        RationalTime::from_seconds(dur - 0.15),
        ItemKind::Text {
            content: "新機能 v0.9 リリース速報".into(),
            font_family: "Noto Sans JP".into(),
            font_size: 46.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator {
                enabled: true,
                start_offset_y: 18.0,
                delay_per_char_sec: 0.02,
                duration_per_char_sec: 0.22,
                ..TextAnimator::default()
            },
        },
    );
    title.transform.x = 132.0;
    title.transform.y = 864.0;

    let mut subtitle = TimelineItem::new(
        "サブ見出し",
        RationalTime::from_seconds(0.25),
        RationalTime::from_seconds(dur - 0.25),
        ItemKind::Text {
            content: "Pure-Rust エクスプレッション＆テンプレート搭載".into(),
            font_family: "Noto Sans JP".into(),
            font_size: 26.0,
            color: RgbaColor {
                r: 0.78,
                g: 0.88,
                b: 0.98,
                a: 1.0,
            },
            animator: TextAnimator::default(),
        },
    );
    subtitle.transform.x = 134.0;
    subtitle.transform.y = 924.0;

    scene.layers = vec![
        Layer {
            items: vec![bg],
            ..Layer::new("帯背景")
        },
        Layer {
            items: vec![bar],
            ..Layer::new("アクセント")
        },
        Layer {
            items: vec![title],
            ..Layer::new("メイン見出し")
        },
        Layer {
            items: vec![subtitle],
            ..Layer::new("サブ見出し")
        },
    ];

    MotionTemplate {
        id: "builtin_lower_third_news".into(),
        name: "ニュース風ローワーサード".into(),
        category: "見出し・テロップ".into(),
        description: "スライドイン＆アウト保護（レスポンシブ時間対応）の定番見出し帯".into(),
        responsive_time: ResponsiveTimeRegion {
            intro_sec: 0.50,
            outro_sec: 0.45,
            default_duration_sec: dur,
        },
        exposed_params: vec![
            ExposedParam {
                id: "title".into(),
                label: "メイン見出し".into(),
                kind: ExposedParamKind::Text {
                    item_name: "メイン見出し".into(),
                    default: "新機能 v0.9 リリース速報".into(),
                },
            },
            ExposedParam {
                id: "subtitle".into(),
                label: "サブ見出し".into(),
                kind: ExposedParamKind::Text {
                    item_name: "サブ見出し".into(),
                    default: "Pure-Rust エクスプレッション＆テンプレート搭載".into(),
                },
            },
            ExposedParam {
                id: "accent_color".into(),
                label: "アクセント色".into(),
                kind: ExposedParamKind::Color {
                    item_name: "アクセントバー".into(),
                    default: accent_color,
                },
            },
            ExposedParam {
                id: "bg_color".into(),
                label: "帯背景色".into(),
                kind: ExposedParamKind::Color {
                    item_name: "帯背景".into(),
                    default: bg_color,
                },
            },
            ExposedParam {
                id: "title_size".into(),
                label: "見出しサイズ".into(),
                kind: ExposedParamKind::FontSize {
                    item_name: "メイン見出し".into(),
                    default: 46.0,
                    min: 24.0,
                    max: 96.0,
                },
            },
        ],
        scene,
    }
}

fn make_kinetic_title_template() -> MotionTemplate {
    let dur = 4.5_f64;
    let mut scene = Scene::new("キネティック OP タイトル", 1920, 1080, FrameRate::FPS_60);
    scene.layers.clear();

    let line_color = RgbaColor {
        r: 1.0,
        g: 0.78,
        b: 0.24,
        a: 1.0,
    };

    let mut underline = TimelineItem::new(
        "手描きライン",
        RationalTime::ZERO,
        RationalTime::from_seconds(dur),
        ItemKind::VectorShape {
            config: VectorShapeConfig::underline_wave(line_color),
        },
    );
    underline.transform.x = 560.0;
    underline.transform.y = 560.0;
    underline.keyframes.insert(
        "trim_end".into(),
        vec![
            Keyframe {
                time: RationalTime::ZERO,
                value: 0.0,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
            },
            Keyframe {
                time: RationalTime::from_seconds(0.60),
                value: 1.0,
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: RationalTime::from_seconds(4.0),
                value: 1.0,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
            },
            Keyframe {
                time: RationalTime::from_seconds(4.5),
                value: 0.0,
                interpolation: Interpolation::Linear,
            },
        ],
    );

    let mut main_title = TimelineItem::new(
        "タイトル文字",
        RationalTime::from_seconds(0.1),
        RationalTime::from_seconds(dur - 0.1),
        ItemKind::Text {
            content: "CHAPTER 01 : OPENING".into(),
            font_family: "Noto Sans JP".into(),
            font_size: 84.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator {
                enabled: true,
                start_offset_y: 42.0,
                start_scale: 0.8,
                delay_per_char_sec: 0.03,
                duration_per_char_sec: 0.28,
                ..TextAnimator::default()
            },
        },
    );
    main_title.transform.x = 460.0;
    main_title.transform.y = 430.0;
    main_title.text_style.outline_width = 4.0;
    main_title.text_style.shadow_offset = [0.0, 6.0];
    main_title.text_style.shadow_blur = 8.0;

    scene.layers = vec![
        Layer {
            items: vec![underline],
            ..Layer::new("トリムライン")
        },
        Layer {
            items: vec![main_title],
            ..Layer::new("タイトル文字")
        },
    ];

    MotionTemplate {
        id: "builtin_kinetic_title".into(),
        name: "キネティック OP タイトル".into(),
        category: "OP / チャプター".into(),
        description: "トリムパス波線アニメーション＋文字ごとアニメーターの演出タイトル".into(),
        responsive_time: ResponsiveTimeRegion {
            intro_sec: 0.65,
            outro_sec: 0.55,
            default_duration_sec: dur,
        },
        exposed_params: vec![
            ExposedParam {
                id: "title".into(),
                label: "タイトル文言".into(),
                kind: ExposedParamKind::Text {
                    item_name: "タイトル文字".into(),
                    default: "CHAPTER 01 : OPENING".into(),
                },
            },
            ExposedParam {
                id: "text_color".into(),
                label: "テキスト色".into(),
                kind: ExposedParamKind::Color {
                    item_name: "タイトル文字".into(),
                    default: RgbaColor::WHITE,
                },
            },
            ExposedParam {
                id: "line_color".into(),
                label: "トリムライン色".into(),
                kind: ExposedParamKind::Color {
                    item_name: "手描きライン".into(),
                    default: line_color,
                },
            },
            ExposedParam {
                id: "font_size".into(),
                label: "文字サイズ".into(),
                kind: ExposedParamKind::FontSize {
                    item_name: "タイトル文字".into(),
                    default: 84.0,
                    min: 32.0,
                    max: 150.0,
                },
            },
        ],
        scene,
    }
}

fn make_subscribe_cta_template() -> MotionTemplate {
    let dur = 3.5_f64;
    let mut scene = Scene::new("告知 CTA カード", 1920, 1080, FrameRate::FPS_60);
    scene.layers.clear();

    let card_color = RgbaColor {
        r: 0.10,
        g: 0.12,
        b: 0.18,
        a: 0.92,
    };
    let cta_color = RgbaColor {
        r: 0.92,
        g: 0.16,
        b: 0.22,
        a: 1.0,
    };

    let mut card = TimelineItem::new(
        "カード背景",
        RationalTime::ZERO,
        RationalTime::from_seconds(dur),
        ItemKind::Shape {
            shape: ShapeKind::RoundedRectangle,
            width: 580.0,
            height: 140.0,
            color: card_color,
        },
    );
    card.transform.x = 1260.0;
    card.transform.y = 860.0;
    card.keyframes.insert(
        "y".into(),
        vec![
            Keyframe {
                time: RationalTime::ZERO,
                value: 1120.0,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_OUT),
            },
            Keyframe {
                time: RationalTime::from_seconds(0.4),
                value: 860.0,
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: RationalTime::from_seconds(3.1),
                value: 860.0,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
            },
            Keyframe {
                time: RationalTime::from_seconds(3.5),
                value: 1120.0,
                interpolation: Interpolation::Linear,
            },
        ],
    );

    let mut btn = TimelineItem::new(
        "CTAボタン",
        RationalTime::ZERO,
        RationalTime::from_seconds(dur),
        ItemKind::Shape {
            shape: ShapeKind::RoundedRectangle,
            width: 190.0,
            height: 64.0,
            color: cta_color,
        },
    );
    btn.transform.x = 1620.0;
    btn.transform.y = 898.0;

    let mut heading = TimelineItem::new(
        "告知見出し",
        RationalTime::from_seconds(0.1),
        RationalTime::from_seconds(dur - 0.1),
        ItemKind::Text {
            content: "高評価＆チャンネル登録！".into(),
            font_family: "Noto Sans JP".into(),
            font_size: 32.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    heading.transform.x = 1288.0;
    heading.transform.y = 885.0;
    heading.text_style = TextStyle::default();

    let mut badge = TimelineItem::new(
        "ボタンラベル",
        RationalTime::from_seconds(0.1),
        RationalTime::from_seconds(dur - 0.1),
        ItemKind::Text {
            content: "登録する".into(),
            font_family: "Noto Sans JP".into(),
            font_size: 28.0,
            color: RgbaColor::WHITE,
            animator: TextAnimator::default(),
        },
    );
    badge.transform.x = 1656.0;
    badge.transform.y = 910.0;

    scene.layers = vec![
        Layer {
            items: vec![card],
            ..Layer::new("カード")
        },
        Layer {
            items: vec![btn],
            ..Layer::new("ボタン")
        },
        Layer {
            items: vec![heading],
            ..Layer::new("告知見出し")
        },
        Layer {
            items: vec![badge],
            ..Layer::new("ボタン文字")
        },
    ];

    MotionTemplate {
        id: "builtin_subscribe_cta".into(),
        name: "告知・登録 CTA カード".into(),
        category: "告知 / SNS".into(),
        description: "右下からポップアップする YouTube チャンネル登録・告知カード".into(),
        responsive_time: ResponsiveTimeRegion {
            intro_sec: 0.45,
            outro_sec: 0.45,
            default_duration_sec: dur,
        },
        exposed_params: vec![
            ExposedParam {
                id: "heading".into(),
                label: "告知メッセージ".into(),
                kind: ExposedParamKind::Text {
                    item_name: "告知見出し".into(),
                    default: "高評価＆チャンネル登録！".into(),
                },
            },
            ExposedParam {
                id: "button_text".into(),
                label: "ボタン文字".into(),
                kind: ExposedParamKind::Text {
                    item_name: "ボタンラベル".into(),
                    default: "登録する".into(),
                },
            },
            ExposedParam {
                id: "button_color".into(),
                label: "ボタン色".into(),
                kind: ExposedParamKind::Color {
                    item_name: "CTAボタン".into(),
                    default: cta_color,
                },
            },
            ExposedParam {
                id: "card_color".into(),
                label: "カード背景色".into(),
                kind: ExposedParamKind::Color {
                    item_name: "カード背景".into(),
                    default: card_color,
                },
            },
        ],
        scene,
    }
}

/// 組み込みプロジェクトテンプレート（新規プロジェクト雛形 3 種）
pub fn builtin_project_templates() -> Vec<ProjectTemplate> {
    // 1. YouTube 横長 16:9
    let mut yt = Project::new("YouTube 16:9 テンプレート", ProjectSettings::default());
    if let Some(s) = yt.active_scene_mut() {
        s.name = "Main (16:9 YouTube)".into();
        s.target_lufs = Some(-14.0);
        s.audio_ducking.enabled = true;
        s.layers = vec![
            Layer::new("L0 映像・背景"),
            Layer::new("L1 テロップ・見出し"),
            Layer::new("L2 立ち絵・装飾"),
            Layer::new("L3 音声・ナレーション"),
            Layer::new("L4 BGM・SE"),
        ];
    }

    // 2. 縦型ショート 9:16
    let mut shorts = Project::new("縦型ショート 9:16 テンプレート", ProjectSettings::default());
    if let Some(s) = shorts.active_scene_mut() {
        let (w, h) = ScenePreset::Shorts9x16.dimensions();
        let fps = ScenePreset::Shorts9x16.default_fps();
        s.name = "Shorts (9:16 縦動画)".into();
        s.width = w;
        s.height = h;
        s.fps = fps;
        s.target_lufs = Some(-14.0);
        s.audio_ducking.enabled = true;
        s.layers = vec![
            Layer::new("L0 縦動画メイン"),
            Layer::new("L1 中央デカ字幕"),
            Layer::new("L2 演出・パーティクル"),
            Layer::new("L3 ボイス"),
            Layer::new("L4 BGM"),
        ];
    }

    // 3. MV / リリック動画
    let mut mv = Project::new("MV・リリック動画テンプレート", ProjectSettings::default());
    if let Some(s) = mv.active_scene_mut() {
        s.name = "MV Main (60fps + 3D/MB)".into();
        s.camera.enabled = true;
        s.camera.light.enabled = true;
        s.motion_blur.enabled = true;
        s.layers = vec![
            Layer::new("L0 背景ビジュアル"),
            Layer::new("L1 パーティクル・演出"),
            Layer::new("L2 リリック字幕"),
            Layer::new("L3 楽曲マスター"),
        ];
    }

    vec![
        ProjectTemplate {
            id: "proj_youtube_16x9".into(),
            name: "YouTube 解説・実況 (16:9 / 60fps)".into(),
            description: "映像・テロップ・立ち絵・音声・BGM（-14 LUFS & ダッキング有効）の標準構成"
                .into(),
            project: yt,
        },
        ProjectTemplate {
            id: "proj_shorts_9x16".into(),
            name: "YouTube Shorts / TikTok (9:16 / 60fps)".into(),
            description: "1080×1920 縦動画・セーフゾーン向け5レイヤー構成".into(),
            project: shorts,
        },
        ProjectTemplate {
            id: "proj_mv_lyric".into(),
            name: "MV / リリック動画 (3Dライト & モーションブラー)".into(),
            description: "3Dカメラ照明・シャッター角モーションブラー標準ONの演出向け構成".into(),
            project: mv,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};

    #[test]
    fn responsive_time_stretch_preserves_intro_and_outro_and_stretches_middle() {
        // 基準 4.0s (intro=0.5s, outro=0.4s) -> 8.0s に倍増
        let t_intro = map_responsive_time(0.25, 4.0, 8.0, 0.5, 0.4);
        assert!((t_intro - 0.25).abs() < 1e-6);

        let t_outro = map_responsive_time(3.8, 4.0, 8.0, 0.5, 0.4);
        // 元の終端から 0.2s 手前 -> 新尺 8.0s から 0.2s 手前 = 7.8s
        assert!((t_outro - 7.8).abs() < 1e-6);

        // 中間ど真ん中: orig mid = [0.5, 3.6] -> center 2.05; new mid = [0.5, 7.6] -> center 4.05
        let t_mid = map_responsive_time(2.05, 4.0, 8.0, 0.5, 0.4);
        assert!((t_mid - 4.05).abs() < 1e-6);
    }

    #[test]
    fn instantiate_and_update_motion_template_with_single_undo() {
        let templates = builtin_motion_templates();
        assert_eq!(templates.len(), 3);
        let tpl = &templates[0];

        let mut project = Project::new("tpl_test", ProjectSettings::default());
        let mut bus = CommandBus::default();
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "title".into(),
            TemplateParamValue::Text("カスタム速報タイトル".into()),
        );
        let cmd = build_instantiate_template_command(
            tpl,
            0,
            RationalTime::from_seconds(1.0),
            Some(7.0),
            &overrides,
        );
        bus.execute(&mut project, cmd, CommandSource::Gui, None)
            .unwrap();

        let item = &project.active_scene().unwrap().layers[0].items[0];
        assert!((item.duration.as_seconds() - 7.0).abs() < 1e-3);
        let ItemKind::Precomp { scene_id } = item.kind else {
            panic!("expected Precomp");
        };
        let nested = project.scene(scene_id).unwrap();
        let title_item = nested
            .layers
            .iter()
            .flat_map(|l| l.items.iter())
            .find(|i| i.name == "メイン見出し")
            .unwrap();
        if let ItemKind::Text { content, .. } = &title_item.kind {
            assert_eq!(content, "カスタム速報タイトル");
        } else {
            panic!("expected text");
        }

        // Undo 1 ステップで追加シーンごと巻き戻る
        assert!(bus.undo(&mut project));
        assert!(project.scene(scene_id).is_none());
        assert!(project.active_scene().unwrap().layers[0].items.is_empty());
    }
}
