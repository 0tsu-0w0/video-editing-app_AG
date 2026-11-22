use crate::command::EditCommand;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecipeError {
    #[error("Recipe parse error: {0}")]
    Parse(String),
    #[error("Recipe JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// マルチエージェント AI 編集ディレクターにおける専門ロール
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// 企画・構成・台本設計
    #[default]
    Planner,
    /// 素材選定・タイムライン配置
    MediaCurator,
    /// 粗編集・カット・整列・同期
    RoughCutEditor,
    /// テロップ装飾・演出・BGM/SE 調整
    MotionAndAudioDesigner,
    /// `render_frame_preview` による視覚自己検査・修正提案
    VisualInspector,
}

impl AgentRole {
    pub fn display_label(self) -> &'static str {
        match self {
            Self::Planner => "📝 Planner (企画・台本)",
            Self::MediaCurator => "🗂 MediaCurator (素材選定)",
            Self::RoughCutEditor => "✂ RoughCutEditor (粗編集)",
            Self::MotionAndAudioDesigner => "✨ Motion&Audio (演出・音声)",
            Self::VisualInspector => "👁 VisualInspector (視覚自己検査)",
        }
    }
}

/// ステップ実行後に `render_frame_preview` 相当で自動検証する視覚品質基準
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualCheckExpectation {
    /// 検査するタイムライン時刻（秒）
    pub time_sec: f64,
    /// 許容する最小平均輝度（0.0..=1.0、真っ暗フレーム検知用）
    #[serde(default = "default_min_luma")]
    pub min_mean_luma: f32,
    /// 許容する最大平均輝度（0.0..=1.0、白飛びフレーム検知用）
    #[serde(default = "default_max_luma")]
    pub max_mean_luma: f32,
    /// 非透明／非黒カバレッジの最低比率（0.0..=1.0）
    #[serde(default = "default_min_coverage")]
    pub min_non_black_ratio: f32,
}

fn default_min_luma() -> f32 {
    0.01
}
fn default_max_luma() -> f32 {
    0.99
}
fn default_min_coverage() -> f32 {
    0.01
}

/// 編集レシピを構成する 1 ステップ（任意 MCP ツール呼び出し＋承認ゲート＋視覚自己検査）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeStep {
    pub id: String,
    #[serde(default)]
    pub role: AgentRole,
    pub title: String,
    pub tool_name: String,
    #[serde(default = "default_empty_object")]
    pub args: Value,
    /// true の場合、このステップ実行前に人間の承認ゲートで一時停止する
    #[serde(default)]
    pub requires_human_approval: bool,
    /// 設定されている場合、実行後にフレームをレンダリングして視覚自己検査を実行する
    #[serde(default)]
    pub visual_check: Option<VisualCheckExpectation>,
}

fn default_empty_object() -> Value {
    Value::Object(Map::new())
}

/// v2.0 #23: 再現・共有可能な AI 編集ディレクターレシピ（YAML / JSON 両対応）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditRecipe {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_recipe_version")]
    pub version: String,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    #[serde(default)]
    pub steps: Vec<RecipeStep>,
}

fn default_recipe_version() -> String {
    "2.0".to_string()
}

/// 人間承認ゲートの現在状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalGateStatus {
    Idle,
    Running,
    AwaitingApproval,
    Completed,
    Rejected,
    Failed,
}

/// 視覚自己検査の結果メトリクス
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualCheckReport {
    pub time_sec: f64,
    pub mean_luma: f32,
    pub contrast_std: f32,
    pub non_black_ratio: f32,
    pub passed: bool,
    pub advice: String,
}

/// レシピ実行時の 1 ステップ実行ログ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeStepLog {
    pub step_index: usize,
    pub step_id: String,
    pub role: AgentRole,
    pub title: String,
    pub tool_name: String,
    pub succeeded: bool,
    pub output_summary: String,
    pub undo_depth_before: usize,
    pub visual_report: Option<VisualCheckReport>,
}

/// 進行中のマルチエージェント編集レシピ実行状態
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeRunState {
    pub recipe_name: String,
    pub current_step_index: usize,
    pub total_steps: usize,
    pub status: ApprovalGateStatus,
    pub pending_gate_step_id: Option<String>,
    pub pending_gate_title: Option<String>,
    pub last_message: String,
    pub logs: Vec<RecipeStepLog>,
}

impl RecipeRunState {
    pub fn start(recipe: &EditRecipe) -> Self {
        Self {
            recipe_name: recipe.name.clone(),
            current_step_index: 0,
            total_steps: recipe.steps.len(),
            status: if recipe.steps.is_empty() {
                ApprovalGateStatus::Completed
            } else {
                ApprovalGateStatus::Running
            },
            pending_gate_step_id: None,
            pending_gate_title: None,
            last_message: format!(
                "レシピ「{}」を開始しました（全 {} ステップ）",
                recipe.name,
                recipe.steps.len()
            ),
            logs: Vec::new(),
        }
    }
}

/// `args` 内の `{{var_name}}` プレースホルダを `variables` で再帰的に置換する
pub fn resolve_template_vars(val: &Value, vars: &BTreeMap<String, String>) -> Value {
    match val {
        Value::String(s) => {
            let mut out = s.clone();
            for (k, v) in vars {
                let token = format!("{{{{{k}}}}}");
                if out.contains(&token) {
                    out = out.replace(&token, v);
                }
            }
            Value::String(out)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| resolve_template_vars(v, vars)).collect())
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(k.clone(), resolve_template_vars(v, vars));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// 編集レシピを整形 JSON 文字列へシリアライズする
pub fn serialize_recipe_json(recipe: &EditRecipe) -> Result<String, RecipeError> {
    Ok(serde_json::to_string_pretty(recipe)?)
}

/// 外部依存ゼロの Pure-Rust YAML シリアライザ（人間が読み書き・Git 共有しやすい形式）
pub fn serialize_recipe_yaml(recipe: &EditRecipe) -> Result<String, RecipeError> {
    let val = serde_json::to_value(recipe)?;
    let mut out = String::new();
    out.push_str("# Video Editing App v2.0 AI Director Recipe\n");
    write_yaml_value(&val, 0, &mut out);
    Ok(out)
}

fn write_yaml_value(val: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match val {
        Value::Null => out.push_str("null\n"),
        Value::Bool(b) => {
            out.push_str(if *b { "true\n" } else { "false\n" });
        }
        Value::Number(n) => {
            out.push_str(&n.to_string());
            out.push('\n');
        }
        Value::String(s) => {
            out.push_str(&format_yaml_scalar(s));
            out.push('\n');
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                out.push_str("[]\n");
                return;
            }
            for item in arr {
                out.push_str(&pad);
                match item {
                    Value::Object(map) if !map.is_empty() => {
                        let mut iter = map.iter();
                        if let Some((first_k, first_v)) = iter.next() {
                            out.push_str("- ");
                            write_yaml_kv(first_k, first_v, indent + 2, out);
                            for (k, v) in iter {
                                out.push_str(&" ".repeat(indent + 2));
                                write_yaml_kv(k, v, indent + 2, out);
                            }
                        }
                    }
                    Value::Array(inner) if !inner.is_empty() => {
                        out.push_str("-\n");
                        write_yaml_value(item, indent + 2, out);
                    }
                    _ => {
                        out.push_str("- ");
                        write_yaml_value(item, indent + 2, out);
                    }
                }
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}\n");
                return;
            }
            for (k, v) in map {
                out.push_str(&pad);
                write_yaml_kv(k, v, indent, out);
            }
        }
    }
}

fn write_yaml_kv(key: &str, val: &Value, indent: usize, out: &mut String) {
    out.push_str(key);
    out.push(':');
    match val {
        Value::Object(m) if !m.is_empty() => {
            out.push('\n');
            write_yaml_value(val, indent + 2, out);
        }
        Value::Array(a) if !a.is_empty() => {
            out.push('\n');
            write_yaml_value(val, indent + 2, out);
        }
        _ => {
            out.push(' ');
            write_yaml_value(val, indent + 2, out);
        }
    }
}

fn format_yaml_scalar(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.contains('\n')
        || s.contains('\r')
        || s.contains(": ")
        || s.contains('#')
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s.starts_with(['-', '[', '{', '"', '\'', '*', '&', '!', '?', '%', '@', '`'])
        || matches!(s, "true" | "false" | "null" | "~")
        || s.parse::<f64>().is_ok();
    if needs_quote {
        serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
    } else {
        s.to_string()
    }
}

/// JSON または YAML（Pure-Rust パーサ）のどちらからでも `EditRecipe` を読み込む
pub fn parse_recipe_json_or_yaml(input: &str) -> Result<EditRecipe, RecipeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(RecipeError::Parse("empty recipe text".into()));
    }
    if trimmed.starts_with('{') {
        return Ok(serde_json::from_str::<EditRecipe>(trimmed)?);
    }
    let lines = tokenize_yaml_lines(trimmed);
    let (val, _) = parse_yaml_block(&lines, 0, 0)?;
    Ok(serde_json::from_value::<EditRecipe>(val)?)
}

#[derive(Debug, Clone)]
struct YamlLine {
    indent: usize,
    text: String,
    line_no: usize,
}

fn tokenize_yaml_lines(src: &str) -> Vec<YamlLine> {
    let mut out = Vec::new();
    for (idx, raw) in src.lines().enumerate() {
        let stripped = strip_yaml_comment(raw);
        let trimmed = stripped.trim();
        if trimmed.is_empty() || trimmed == "---" || trimmed == "..." {
            continue;
        }
        let indent = stripped.bytes().take_while(|b| *b == b' ').count();
        out.push(YamlLine {
            indent,
            text: trimmed.to_string(),
            line_no: idx + 1,
        });
    }
    out
}

fn strip_yaml_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_double {
            if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
            continue;
        }
        match b {
            b'"' => in_double = true,
            b'\'' => in_single = true,
            b'#' if i == 0 || bytes[i - 1].is_ascii_whitespace() => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_yaml_block(
    lines: &[YamlLine],
    start: usize,
    min_indent: usize,
) -> Result<(Value, usize), RecipeError> {
    if start >= lines.len() {
        return Ok((Value::Null, start));
    }
    if lines[start].text == "-" || lines[start].text.starts_with("- ") {
        parse_yaml_seq(lines, start, lines[start].indent)
    } else {
        parse_yaml_map(lines, start, lines[start].indent.max(min_indent))
    }
}

fn parse_yaml_seq(
    lines: &[YamlLine],
    mut idx: usize,
    seq_indent: usize,
) -> Result<(Value, usize), RecipeError> {
    let mut items = Vec::new();
    while idx < lines.len() {
        let line = &lines[idx];
        if line.indent < seq_indent {
            break;
        }
        if line.indent != seq_indent || !(line.text == "-" || line.text.starts_with("- ")) {
            break;
        }
        let rest = line.text.strip_prefix('-').unwrap_or("").trim();
        if rest.is_empty() {
            idx += 1;
            if idx < lines.len() && lines[idx].indent > seq_indent {
                let (v, next) = parse_yaml_block(lines, idx, lines[idx].indent)?;
                items.push(v);
                idx = next;
            } else {
                items.push(Value::Null);
            }
        } else if let Some((k, v_str)) = split_yaml_key_val(rest) {
            // インラインマップ開始 `- key: val`
            let item_indent = seq_indent + 2;
            let mut map = Map::new();
            idx += 1;
            let val = if v_str.is_empty() {
                if idx < lines.len() && lines[idx].indent > item_indent {
                    let (nested, next) = parse_yaml_block(lines, idx, lines[idx].indent)?;
                    idx = next;
                    nested
                } else {
                    Value::Null
                }
            } else {
                parse_yaml_scalar(v_str)?
            };
            map.insert(k.to_string(), val);
            // 同一オブジェクトの続きのキーを読む
            if idx < lines.len() && lines[idx].indent >= item_indent {
                let (more, next) = parse_yaml_map(lines, idx, item_indent)?;
                if let Value::Object(more_map) = more {
                    for (mk, mv) in more_map {
                        map.insert(mk, mv);
                    }
                }
                idx = next;
            }
            items.push(Value::Object(map));
        } else {
            items.push(parse_yaml_scalar(rest)?);
            idx += 1;
        }
    }
    Ok((Value::Array(items), idx))
}

fn parse_yaml_map(
    lines: &[YamlLine],
    mut idx: usize,
    map_indent: usize,
) -> Result<(Value, usize), RecipeError> {
    let mut map = Map::new();
    while idx < lines.len() {
        let line = &lines[idx];
        if line.indent < map_indent {
            break;
        }
        if line.indent > map_indent {
            return Err(RecipeError::Parse(format!(
                "unexpected indentation at line {}",
                line.line_no
            )));
        }
        if line.text == "-" || line.text.starts_with("- ") {
            break;
        }
        let Some((key, val_str)) = split_yaml_key_val(&line.text) else {
            return Err(RecipeError::Parse(format!(
                "expected 'key: value' at line {}: {}",
                line.line_no, line.text
            )));
        };
        idx += 1;
        let value = if val_str.is_empty() {
            if idx < lines.len() && lines[idx].indent > map_indent {
                let (nested, next) = parse_yaml_block(lines, idx, lines[idx].indent)?;
                idx = next;
                nested
            } else {
                Value::Null
            }
        } else {
            parse_yaml_scalar(val_str)?
        };
        map.insert(key.to_string(), value);
    }
    Ok((Value::Object(map), idx))
}

fn split_yaml_key_val(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' if !in_single => in_double = !in_double,
            b'\'' if !in_double => in_single = !in_single,
            b':' if !in_single && !in_double => {
                let next_ok = i + 1 == bytes.len() || bytes[i + 1].is_ascii_whitespace();
                if next_ok {
                    let key = s[..i].trim().trim_matches('"').trim_matches('\'');
                    let val = s[i + 1..].trim();
                    if !key.is_empty() {
                        return Some((key, val));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_yaml_scalar(s: &str) -> Result<Value, RecipeError> {
    let t = s.trim();
    if t == "null" || t == "~" {
        return Ok(Value::Null);
    }
    if t == "true" {
        return Ok(Value::Bool(true));
    }
    if t == "false" {
        return Ok(Value::Bool(false));
    }
    if t == "[]" {
        return Ok(Value::Array(Vec::new()));
    }
    if t == "{}" {
        return Ok(Value::Object(Map::new()));
    }
    if ((t.starts_with('[') && t.ends_with(']')) || (t.starts_with('{') && t.ends_with('}')))
        && let Ok(v) = serde_json::from_str::<Value>(t)
    {
        return Ok(v);
    }
    if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
        return Ok(serde_json::from_str::<Value>(t)?);
    }
    if t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2 {
        return Ok(Value::String(t[1..t.len() - 1].replace("''", "'")));
    }
    if let Ok(i) = t.parse::<i64>() {
        return Ok(Value::Number(i.into()));
    }
    if let Ok(f) = t.parse::<f64>()
        && f.is_finite()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return Ok(Value::Number(n));
    }
    Ok(Value::String(t.to_string()))
}

/// 実行済み `EditCommand` 履歴から再利用可能な `EditRecipe` マクロを自動記録する
pub fn record_recipe_from_commands(
    name: impl Into<String>,
    description: impl Into<String>,
    commands: &[EditCommand],
) -> EditRecipe {
    let mut steps = Vec::new();
    for (idx, cmd) in commands.iter().enumerate() {
        let (role, tool_name, title, args) = match cmd {
            EditCommand::AddObject { layer_index, item } => {
                let (kind_str, extra) = match &item.kind {
                    crate::project::ItemKind::Text {
                        content, font_size, ..
                    } => (
                        "text",
                        serde_json::json!({
                            "layer_index": layer_index,
                            "name": item.name,
                            "kind": "text",
                            "content": content,
                            "font_size": font_size,
                            "start_sec": item.start.as_seconds(),
                            "duration_sec": item.duration.as_seconds(),
                        }),
                    ),
                    crate::project::ItemKind::Shape { width, height, .. } => (
                        "shape",
                        serde_json::json!({
                            "layer_index": layer_index,
                            "name": item.name,
                            "kind": "shape",
                            "width": width,
                            "height": height,
                            "start_sec": item.start.as_seconds(),
                            "duration_sec": item.duration.as_seconds(),
                        }),
                    ),
                    _ => continue,
                };
                (
                    AgentRole::MediaCurator,
                    "add_object",
                    format!("オブジェクト追加 ({kind_str}): {}", item.name),
                    extra,
                )
            }
            EditCommand::SetSceneColorGrade { grade } => (
                AgentRole::MotionAndAudioDesigner,
                "set_color_grade",
                "カラーグレーディング適用".to_string(),
                serde_json::json!({
                    "enabled": grade.enabled,
                    "exposure": grade.exposure,
                    "contrast": grade.contrast,
                    "saturation": grade.saturation,
                    "temperature": grade.temperature,
                    "tint": grade.tint,
                    "lut_path": grade.lut_path,
                    "lut_intensity": grade.lut_intensity,
                }),
            ),
            EditCommand::AddMarker { marker } => (
                AgentRole::RoughCutEditor,
                "manage_markers",
                format!("マーカー追加: {}", marker.label),
                serde_json::json!({
                    "action": "add",
                    "time_sec": marker.time.as_seconds(),
                    "label": marker.label,
                }),
            ),
            EditCommand::SetSceneSettings {
                width,
                height,
                fps,
                rescale_items,
            } => (
                AgentRole::Planner,
                "set_scene_settings",
                format!("シーン設定変更 ({width}x{height})"),
                serde_json::json!({
                    "width": width,
                    "height": height,
                    "fps": fps.as_f64(),
                    "rescale_items": rescale_items,
                }),
            ),
            EditCommand::SetAudioDucking { ducking } => (
                AgentRole::MotionAndAudioDesigner,
                "set_audio_ducking",
                "BGM 自動ダッキング設定".to_string(),
                serde_json::json!({
                    "enabled": ducking.enabled,
                    "voice_layer_index": ducking.voice_layer_index,
                    "bgm_layer_index": ducking.bgm_layer_index,
                    "reduction_db": ducking.reduction_db,
                }),
            ),
            _ => continue,
        };

        steps.push(RecipeStep {
            id: format!("recorded_{:02}", idx + 1),
            role,
            title,
            tool_name: tool_name.to_string(),
            args,
            requires_human_approval: false,
            visual_check: None,
        });
    }

    // 最後に視覚自己検査ステップを 1 つ添付
    steps.push(RecipeStep {
        id: "visual_verify_final".to_string(),
        role: AgentRole::VisualInspector,
        title: "最終フレーム視覚自己検査".to_string(),
        tool_name: "render_frame_preview".to_string(),
        args: serde_json::json!({ "seconds": 0.5, "scale": 0.25 }),
        requires_human_approval: false,
        visual_check: Some(VisualCheckExpectation {
            time_sec: 0.5,
            min_mean_luma: 0.01,
            max_mean_luma: 0.99,
            min_non_black_ratio: 0.01,
        }),
    });

    EditRecipe {
        name: name.into(),
        description: description.into(),
        version: "2.0".to_string(),
        variables: BTreeMap::new(),
        steps,
    }
}

/// 標準内蔵マルチエージェント編集ディレクターレシピ 3 種
pub fn builtin_edit_recipes() -> Vec<EditRecipe> {
    let mut explainer_vars = BTreeMap::new();
    explainer_vars.insert(
        "title".to_string(),
        "AI ディレクター自動構成タイトル".to_string(),
    );
    explainer_vars.insert(
        "subtitle".to_string(),
        "企画→粗編集→演出→視覚検査まで一気通貫".to_string(),
    );

    let youtube_explainer = EditRecipe {
        name: "YouTube 解説動画・標準ディレクターパイプライン".to_string(),
        description: "企画テロップ配置 → 構成マーカー付与 → シネマ色調＆BGMダッキング → 人間承認ゲート → 視覚自己検査".to_string(),
        version: "2.0".to_string(),
        variables: explainer_vars,
        steps: vec![
            RecipeStep {
                id: "plan_title".to_string(),
                role: AgentRole::Planner,
                title: "オープニングタイトルを配置".to_string(),
                tool_name: "add_object".to_string(),
                args: serde_json::json!({
                    "layer_index": 1,
                    "name": "OPタイトル",
                    "kind": "text",
                    "content": "{{title}}",
                    "font_size": 58.0,
                    "start_sec": 0.0,
                    "duration_sec": 4.0
                }),
                requires_human_approval: false,
                visual_check: None,
            },
            RecipeStep {
                id: "curate_sub".to_string(),
                role: AgentRole::MediaCurator,
                title: "導入テロップを配置".to_string(),
                tool_name: "add_object".to_string(),
                args: serde_json::json!({
                    "layer_index": 2,
                    "name": "導入サブタイトル",
                    "kind": "text",
                    "content": "{{subtitle}}",
                    "font_size": 36.0,
                    "start_sec": 0.5,
                    "duration_sec": 3.5
                }),
                requires_human_approval: false,
                visual_check: None,
            },
            RecipeStep {
                id: "rough_marker".to_string(),
                role: AgentRole::RoughCutEditor,
                title: "本編チャプターマーカーを追加（承認ゲート付き）".to_string(),
                tool_name: "manage_markers".to_string(),
                args: serde_json::json!({
                    "action": "add",
                    "time_sec": 0.0,
                    "label": "01 オープニング"
                }),
                requires_human_approval: true,
                visual_check: None,
            },
            RecipeStep {
                id: "style_grade".to_string(),
                role: AgentRole::MotionAndAudioDesigner,
                title: "クリーンカラーグレーディング＆BGMダッキング適用".to_string(),
                tool_name: "set_color_grade".to_string(),
                args: serde_json::json!({
                    "enabled": true,
                    "exposure": 0.08,
                    "contrast": 1.06,
                    "saturation": 1.08,
                    "lut_path": "builtin:teal_orange",
                    "lut_intensity": 0.35
                }),
                requires_human_approval: false,
                visual_check: Some(VisualCheckExpectation {
                    time_sec: 1.0,
                    min_mean_luma: 0.01,
                    max_mean_luma: 0.98,
                    min_non_black_ratio: 0.01,
                }),
            },
            RecipeStep {
                id: "inspect_preview".to_string(),
                role: AgentRole::VisualInspector,
                title: "render_frame_preview による最終視覚自己検査".to_string(),
                tool_name: "render_frame_preview".to_string(),
                args: serde_json::json!({ "seconds": 1.0, "scale": 0.25 }),
                requires_human_approval: false,
                visual_check: Some(VisualCheckExpectation {
                    time_sec: 1.0,
                    min_mean_luma: 0.01,
                    max_mean_luma: 0.98,
                    min_non_black_ratio: 0.01,
                }),
            },
        ],
    };

    let shorts_pipeline = EditRecipe {
        name: "ショート動画 (9:16) 自動リフレーム＆仕上げ".to_string(),
        description: "ローカル AI 被写体追尾で 9:16 自動リフレーム → 承認ゲート → 納品検証"
            .to_string(),
        version: "2.0".to_string(),
        variables: BTreeMap::new(),
        steps: vec![
            RecipeStep {
                id: "ai_reframe".to_string(),
                role: AgentRole::RoughCutEditor,
                title: "ローカル AI 被写体追尾 9:16 自動リフレーム".to_string(),
                tool_name: "run_local_ai_vision".to_string(),
                args: serde_json::json!({
                    "action": "auto_reframe",
                    "target_width": 1080,
                    "target_height": 1920,
                    "apply": true
                }),
                requires_human_approval: true,
                visual_check: Some(VisualCheckExpectation {
                    time_sec: 0.5,
                    min_mean_luma: 0.005,
                    max_mean_luma: 0.99,
                    min_non_black_ratio: 0.01,
                }),
            },
            RecipeStep {
                id: "delivery_check".to_string(),
                role: AgentRole::VisualInspector,
                title: "書き出し前品質チェックリスト検証".to_string(),
                tool_name: "validate_and_package_project".to_string(),
                args: serde_json::json!({ "action": "validate" }),
                requires_human_approval: false,
                visual_check: None,
            },
        ],
    };

    let node_comp_pipeline = EditRecipe {
        name: "ノード合成空間セットアップ＆品質検査".to_string(),
        description: "現レイヤーからノード合成グラフを生成し、視覚自己検査まで自動実行".to_string(),
        version: "2.0".to_string(),
        variables: BTreeMap::new(),
        steps: vec![
            RecipeStep {
                id: "build_nodes".to_string(),
                role: AgentRole::MotionAndAudioDesigner,
                title: "レイヤー構成からノード合成グラフを構築".to_string(),
                tool_name: "manage_node_graph".to_string(),
                args: serde_json::json!({ "action": "from_layers" }),
                requires_human_approval: true,
                visual_check: None,
            },
            RecipeStep {
                id: "verify_nodes".to_string(),
                role: AgentRole::VisualInspector,
                title: "ノード合成結果の視覚自己検査".to_string(),
                tool_name: "render_frame_preview".to_string(),
                args: serde_json::json!({ "seconds": 0.5, "scale": 0.25 }),
                requires_human_approval: false,
                visual_check: Some(VisualCheckExpectation {
                    time_sec: 0.5,
                    min_mean_luma: 0.005,
                    max_mean_luma: 0.99,
                    min_non_black_ratio: 0.01,
                }),
            },
        ],
    };

    vec![youtube_explainer, shorts_pipeline, node_comp_pipeline]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_yaml_and_json_roundtrip_preserves_steps_and_variables() {
        let original = builtin_edit_recipes().remove(0);

        let json = serialize_recipe_json(&original).unwrap();
        let from_json = parse_recipe_json_or_yaml(&json).unwrap();
        assert_eq!(original, from_json);

        let yaml = serialize_recipe_yaml(&original).unwrap();
        assert!(yaml.contains("steps:"));
        assert!(yaml.contains("requires_human_approval:"));
        let from_yaml = parse_recipe_json_or_yaml(&yaml).unwrap();
        assert_eq!(original.name, from_yaml.name);
        assert_eq!(original.steps.len(), from_yaml.steps.len());
        assert_eq!(original.variables, from_yaml.variables);
        assert_eq!(
            original.steps[2].requires_human_approval,
            from_yaml.steps[2].requires_human_approval
        );
        assert_eq!(
            original.steps[3].visual_check,
            from_yaml.steps[3].visual_check
        );
    }

    #[test]
    fn template_var_substitution_replaces_nested_strings() {
        let mut vars = BTreeMap::new();
        vars.insert("title".into(), "新機能発表".into());
        let args = serde_json::json!({
            "content": "【速報】{{title}}！",
            "nested": { "arr": ["{{title}}", 42] }
        });
        let resolved = resolve_template_vars(&args, &vars);
        assert_eq!(resolved["content"], "【速報】新機能発表！");
        assert_eq!(resolved["nested"]["arr"][0], "新機能発表");
        assert_eq!(resolved["nested"]["arr"][1], 42);
    }
}
