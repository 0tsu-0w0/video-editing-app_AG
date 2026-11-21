use crate::command::EditCommand;
use crate::project::{ItemKind, RgbaColor, TextAnimator, TextStyle, TimelineItem, Transform2D};
use crate::time::RationalTime;
use crate::timeline_ops::wrap_subtitle_japanese;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 台本（CSV / Markdown）の1発話行
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptLine {
    pub speaker: String,
    pub text: String,
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub image_path: Option<String>,
}

/// 台本→タイムライン自動組み立て設定
#[derive(Debug, Clone)]
pub struct ScriptBuildConfig {
    pub start_time: RationalTime,
    /// TTS 音声尺が無い場合の 1 秒あたり読み上げ文字数（既定 6.5 文字/秒）
    pub chars_per_sec: f64,
    /// 1セリフの最小表示秒数（既定 1.2s）
    pub min_line_sec: f64,
    /// セリフ間のポーズ（間）秒数（既定 0.25s）
    pub pause_gap_sec: f64,
    /// 1行あたりの自動改行文字数目安（既定 20文字）
    pub max_chars_per_line: usize,
    pub subtitle_layer: usize,
    pub font_size: f32,
    pub show_speaker_prefix: bool,
    pub speaker_colors: BTreeMap<String, RgbaColor>,
}

impl Default for ScriptBuildConfig {
    fn default() -> Self {
        Self {
            start_time: RationalTime::ZERO,
            chars_per_sec: 6.5,
            min_line_sec: 1.2,
            pause_gap_sec: 0.25,
            max_chars_per_line: 20,
            subtitle_layer: 1,
            font_size: 44.0,
            show_speaker_prefix: false,
            speaker_colors: BTreeMap::new(),
        }
    }
}

const DEFAULT_SPEAKER_PALETTE: &[RgbaColor] = &[
    // 1人目: 白＋赤縁取りイメージ（文字色: クリーム白）
    RgbaColor {
        r: 1.0,
        g: 0.96,
        b: 0.92,
        a: 1.0,
    },
    // 2人目: 明るいイエロー
    RgbaColor {
        r: 1.0,
        g: 0.92,
        b: 0.42,
        a: 1.0,
    },
    // 3人目: ずんだグリーン
    RgbaColor {
        r: 0.68,
        g: 0.98,
        b: 0.62,
        a: 1.0,
    },
    // 4人目: 解説シアン
    RgbaColor {
        r: 0.52,
        g: 0.90,
        b: 1.0,
        a: 1.0,
    },
    // 5人目: 桃ピンク
    RgbaColor {
        r: 1.0,
        g: 0.72,
        b: 0.86,
        a: 1.0,
    },
];

/// CSV 形式（`話者,セリフ,表情,背景画像`）の台本をパースする（ダブルクォート囲み対応）
pub fn parse_script_csv(csv: &str) -> Vec<ScriptLine> {
    let mut lines = Vec::new();
    for (idx, raw_line) in csv.lines().enumerate() {
        let trimmed = raw_line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols = split_csv_row(trimmed);
        if cols.len() < 2 {
            continue;
        }
        let speaker = cols[0].trim();
        let text = cols[1].trim();
        // 先頭ヘッダー行を自動スキップ
        if idx == 0
            && (speaker.eq_ignore_ascii_case("speaker") || speaker == "話者" || speaker == "キャラ")
        {
            continue;
        }
        if text.is_empty() {
            continue;
        }
        let expression = cols
            .get(2)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let image_path = cols
            .get(3)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        lines.push(ScriptLine {
            speaker: if speaker.is_empty() {
                "ナレーター".to_string()
            } else {
                speaker.to_string()
            },
            text: text.to_string(),
            expression,
            image_path,
        });
    }
    lines
}

fn split_csv_row(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Markdown / プレーンテキスト対話形式の台本をパースする。
/// 対応例:
/// - `**霊夢** [smile]: こんにちは！`
/// - `魔理沙（驚き）：今日は解説動画を作るぜ！`
/// - `![bg](slide1.png)`（直後のセリフへ背景画像パスを紐付け）
pub fn parse_script_markdown(md: &str) -> Vec<ScriptLine> {
    let mut out = Vec::new();
    let mut pending_image: Option<String> = None;

    for raw in md.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') || line.starts_with("---") {
            continue;
        }
        // Markdown 画像記法 `![alt](path)` を次のセリフへ関連付け
        if let Some(rest) = line.strip_prefix("![")
            && let Some(mid) = rest.find("](")
            && let Some(end) = rest[mid + 2..].find(')')
        {
            let path = rest[mid + 2..mid + 2 + end].trim();
            if !path.is_empty() {
                pending_image = Some(path.to_string());
            }
            continue;
        }

        // 先頭の箇条書き記号 `- ` / `* ` を除去
        let stripped = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line)
            .trim();

        // 半角 `:` または全角 `：` で話者とセリフを分離
        let colon_pos = stripped
            .char_indices()
            .find(|&(_, ch)| ch == ':' || ch == '：')
            .map(|(i, ch)| (i, ch.len_utf8()));

        let Some((pos, clen)) = colon_pos else {
            continue;
        };
        let raw_speaker = stripped[..pos].trim().replace("**", "");
        let text = stripped[pos + clen..].trim();
        if text.is_empty() {
            continue;
        }
        let (speaker, expression) = extract_speaker_and_expression(&raw_speaker);
        out.push(ScriptLine {
            speaker: if speaker.is_empty() {
                "ナレーター".to_string()
            } else {
                speaker
            },
            text: text.to_string(),
            expression,
            image_path: pending_image.take(),
        });
    }
    out
}

fn extract_speaker_and_expression(raw: &str) -> (String, Option<String>) {
    // `[smile]` or `(smile)` or `（笑顔）` を表情タグとして分離
    for (open, close) in [('[', ']'), ('(', ')'), ('（', '）')] {
        if let Some(s) = raw.find(open)
            && let Some(e) = raw[s..].find(close)
        {
            let sp = raw[..s].trim().to_string();
            let expr = raw[s + open.len_utf8()..s + e].trim().to_string();
            return (sp, (!expr.is_empty()).then_some(expr));
        }
    }
    (raw.trim().to_string(), None)
}

/// 入力文字列が CSV か Markdown かを自動判定してパースする
pub fn parse_script_auto(input: &str) -> Vec<ScriptLine> {
    let first_non_empty = input
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let looks_csv = first_non_empty.contains(',')
        && !first_non_empty.contains(':')
        && !first_non_empty.contains('：');
    if looks_csv {
        let rows = parse_script_csv(input);
        if !rows.is_empty() {
            return rows;
        }
    }
    let md = parse_script_markdown(input);
    if !md.is_empty() {
        md
    } else {
        parse_script_csv(input)
    }
}

/// パース済み台本行から話者別カラー・座布団装飾付き字幕＆間（ポーズ）調整済みの
/// 1 ステップ Undo 可能 `EditCommand::Batch` を生成する。
/// `durations_override` に TTS 実尺（秒）を渡した場合はその尺を優先使用する。
pub fn build_script_timeline_commands(
    lines: &[ScriptLine],
    durations_override: Option<&[f64]>,
    cfg: &ScriptBuildConfig,
    scene_w: u32,
    scene_h: u32,
) -> Option<(usize, RationalTime, EditCommand)> {
    if lines.is_empty() {
        return None;
    }

    let mut speaker_color_map = cfg.speaker_colors.clone();
    let mut next_palette_idx = 0usize;
    let mut cursor = cfg.start_time.max(RationalTime::ZERO);
    let mut commands = Vec::new();
    let cps = cfg.chars_per_sec.max(1.0);
    let min_dur = cfg.min_line_sec.max(0.2);
    let gap = RationalTime::from_seconds(cfg.pause_gap_sec.max(0.0));

    for (idx, line) in lines.iter().enumerate() {
        let color = *speaker_color_map
            .entry(line.speaker.clone())
            .or_insert_with(|| {
                let c = DEFAULT_SPEAKER_PALETTE[next_palette_idx % DEFAULT_SPEAKER_PALETTE.len()];
                next_palette_idx += 1;
                c
            });

        let dur_sec = durations_override
            .and_then(|d| d.get(idx).copied())
            .filter(|d| d.is_finite() && *d > 0.1)
            .unwrap_or_else(|| {
                let char_cnt = line.text.chars().count().max(1) as f64;
                (char_cnt / cps).max(min_dur)
            });
        let dur = RationalTime::from_seconds(dur_sec);

        let wrapped = wrap_subtitle_japanese(&line.text, cfg.max_chars_per_line.max(8));
        let content = if cfg.show_speaker_prefix {
            format!("【{}】\n{wrapped}", line.speaker)
        } else {
            wrapped
        };
        let expr_suffix = line
            .expression
            .as_deref()
            .map(|e| format!(" [{e}]"))
            .unwrap_or_default();

        let mut item = TimelineItem::new(
            format!("{}{expr_suffix}", line.speaker),
            cursor,
            dur,
            ItemKind::Text {
                content,
                font_family: "sans-serif".to_string(),
                font_size: cfg.font_size.clamp(12.0, 160.0),
                color,
                animator: TextAnimator::default(),
            },
        );
        item.transform = Transform2D {
            x: (scene_w as f32 * 0.10).round(),
            y: (scene_h as f32 * 0.74).round(),
            ..Default::default()
        };
        item.text_style = TextStyle {
            outline_width: 3.0,
            outline_color: RgbaColor {
                r: 0.05,
                g: 0.05,
                b: 0.08,
                a: 1.0,
            },
            shadow_offset: [2.0, 2.0],
            box_enabled: true,
            box_color: RgbaColor {
                r: 0.04,
                g: 0.06,
                b: 0.10,
                a: 0.68,
            },
            ..Default::default()
        };

        commands.push(EditCommand::AddObject {
            layer_index: cfg.subtitle_layer,
            item,
        });
        cursor = cursor + dur + gap;
    }

    let count = lines.len();
    Some((
        count,
        cursor,
        EditCommand::Batch {
            label: "import_script_timeline".into(),
            commands,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::{Project, ProjectSettings};

    #[test]
    fn parses_csv_and_markdown_scripts_and_builds_undoable_timeline() {
        let csv = "\
話者,セリフ,表情,背景画像
霊夢,\"こんにちは、今日は動画編集を解説します！\",smile,bg1.png
魔理沙,これは便利だぜ！,surprise,
";
        let csv_lines = parse_script_auto(csv);
        assert_eq!(csv_lines.len(), 2);
        assert_eq!(csv_lines[0].speaker, "霊夢");
        assert_eq!(csv_lines[0].expression.as_deref(), Some("smile"));
        assert_eq!(csv_lines[0].image_path.as_deref(), Some("bg1.png"));

        let md = "\
![slide](intro.png)
**ずんだもん** [happy]: ずんだもんなのだ！
めたん（通常）：解説を始めるわよ。
";
        let md_lines = parse_script_auto(md);
        assert_eq!(md_lines.len(), 2);
        assert_eq!(md_lines[0].speaker, "ずんだもん");
        assert_eq!(md_lines[0].expression.as_deref(), Some("happy"));
        assert_eq!(md_lines[0].image_path.as_deref(), Some("intro.png"));
        assert_eq!(md_lines[1].speaker, "めたん");
        assert_eq!(md_lines[1].expression.as_deref(), Some("通常"));

        let mut proj = Project::new("ScriptTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let cfg = ScriptBuildConfig::default();
        let (cnt, end_t, cmd) =
            build_script_timeline_commands(&md_lines, None, &cfg, 1920, 1080).unwrap();
        assert_eq!(cnt, 2);
        assert!(end_t.as_seconds() > 2.4);

        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        assert_eq!(proj.active_scene().unwrap().layers[1].items.len(), 2);
        // 話者ごとに自動で別色が割り当てられる
        let c0 = match &proj.active_scene().unwrap().layers[1].items[0].kind {
            ItemKind::Text { color, .. } => *color,
            _ => panic!("expected text"),
        };
        let c1 = match &proj.active_scene().unwrap().layers[1].items[1].kind {
            ItemKind::Text { color, .. } => *color,
            _ => panic!("expected text"),
        };
        assert_ne!(c0, c1);

        // 1 ステップ Undo で全行撤回可能
        assert!(bus.undo(&mut proj));
        assert!(proj.active_scene().unwrap().layers[1].items.is_empty());
    }
}
