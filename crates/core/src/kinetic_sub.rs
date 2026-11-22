use crate::command::EditCommand;
use crate::project::{ItemId, ItemKind, RgbaColor, Scene};
use crate::text_edit::strip_control_tags;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

// =========================================================================
// 1. Dynamic Kinetic Subtitles (Word-by-Word Karaoke Highlight)
// =========================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KineticHighlightMode {
    /// 発話中ワードをアクティブ色＋ポップ拡大で強調（Shorts / TikTok 定番）
    #[default]
    WordPopColor,
    /// 左から右へ既読ワードを順番に塗り潰すカラオケスイープ
    KaraokeSweep,
    /// 未到達ワードを控えめにし、発話中ワードを強調表示
    ProgressiveReveal,
    /// 強調キーワードと現在ワードの両方をハイライト
    KeywordPulse,
}

impl KineticHighlightMode {
    pub const ALL: &'static [Self] = &[
        Self::WordPopColor,
        Self::KaraokeSweep,
        Self::ProgressiveReveal,
        Self::KeywordPulse,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::WordPopColor => "🎤 ワードポップ (Word Pop)",
            Self::KaraokeSweep => "🎶 カラオケスイープ (Karaoke)",
            Self::ProgressiveReveal => "⚡ 段階ハイライト (Progressive)",
            Self::KeywordPulse => "🔥 強調語パルス (Keyword Pulse)",
        }
    }
}

/// v2.1 #28: ワード単位キネティック字幕設定（`TimelineItem::kinetic_subtitle` に保持）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KineticSubtitleConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: KineticHighlightMode,
    /// 現在発話中ワードの色（既定: 鮮やかなイエロー）
    #[serde(default = "default_active_color")]
    pub active_color: RgbaColor,
    /// 通常・未到達ワードの色（既定: 白）
    #[serde(default = "default_inactive_color")]
    pub inactive_color: RgbaColor,
    /// 強調キーワードに自動適用する色（既定: コーラルオレンジ）
    #[serde(default = "default_emphasis_color")]
    pub emphasis_color: RgbaColor,
    /// アクティブワードのフォント拡大倍率 (1.0..=1.8, 既定 1.18)
    #[serde(default = "default_pop_scale")]
    pub pop_scale: f32,
    /// 自動強調するキーワード一覧（例: "重要", "ポイント", "結論", "無料", "速報"）
    #[serde(default = "default_emphasis_keywords")]
    pub auto_emphasize_keywords: Vec<String>,
}

fn default_active_color() -> RgbaColor {
    RgbaColor {
        r: 1.0,
        g: 0.90,
        b: 0.18,
        a: 1.0,
    }
}

fn default_inactive_color() -> RgbaColor {
    RgbaColor {
        r: 0.95,
        g: 0.96,
        b: 1.0,
        a: 1.0,
    }
}

fn default_emphasis_color() -> RgbaColor {
    RgbaColor {
        r: 1.0,
        g: 0.36,
        b: 0.32,
        a: 1.0,
    }
}

fn default_pop_scale() -> f32 {
    1.18
}

fn default_emphasis_keywords() -> Vec<String> {
    vec![
        "重要".into(),
        "ポイント".into(),
        "結論".into(),
        "注意".into(),
        "速報".into(),
        "無料".into(),
        "NEW".into(),
        "TOP".into(),
    ]
}

impl Default for KineticSubtitleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: KineticHighlightMode::WordPopColor,
            active_color: default_active_color(),
            inactive_color: default_inactive_color(),
            emphasis_color: default_emphasis_color(),
            pop_scale: default_pop_scale(),
            auto_emphasize_keywords: default_emphasis_keywords(),
        }
    }
}

/// 分割された 1 ワード／フレーズの時間占有範囲（0.0..=1.0）
#[derive(Debug, Clone, PartialEq)]
pub struct KineticWordSpan {
    pub text: String,
    pub start_frac: f32,
    pub end_frac: f32,
    pub is_emphasized: bool,
    pub is_newline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharScript {
    Whitespace,
    Punct,
    AsciiWord,
    Hiragana,
    Katakana,
    KanjiOrOther,
}

fn classify_char(c: char) -> CharScript {
    if c.is_whitespace() {
        CharScript::Whitespace
    } else if matches!(
        c,
        '、' | '。'
            | '！'
            | '？'
            | '!'
            | '?'
            | ','
            | '.'
            | '・'
            | '「'
            | '」'
            | '（'
            | '）'
            | '('
            | ')'
            | ':'
            | '：'
    ) {
        CharScript::Punct
    } else if c.is_ascii_alphanumeric() || c == '\'' || c == '-' || c == '_' {
        CharScript::AsciiWord
    } else if ('\u{3040}'..='\u{309F}').contains(&c) {
        CharScript::Hiragana
    } else if ('\u{30A0}'..='\u{30FF}').contains(&c) || c == 'ー' {
        CharScript::Katakana
    } else {
        CharScript::KanjiOrOther
    }
}

/// 日本語（文字種境界・句読点）および英語（スペース境界）の字幕テキストを自然な読み上げチャンク列へ分割し、文字数比例のタイミングを割り当てる
pub fn tokenize_kinetic_words(
    raw_content: &str,
    emphasis_keywords: &[String],
) -> Vec<KineticWordSpan> {
    let clean = strip_control_tags(raw_content);
    let mut raw_tokens: Vec<(String, bool)> = Vec::new();

    for (line_idx, line) in clean.split('\n').enumerate() {
        if line_idx > 0 {
            raw_tokens.push(("\n".to_string(), true));
        }
        let mut cur = String::new();
        let mut prev_script: Option<CharScript> = None;

        for ch in line.chars() {
            let sc = classify_char(ch);
            match sc {
                CharScript::Whitespace => {
                    if !cur.is_empty() {
                        cur.push(ch);
                        raw_tokens.push((std::mem::take(&mut cur), false));
                    } else if let Some(last) = raw_tokens.last_mut()
                        && !last.1
                    {
                        last.0.push(ch);
                    }
                    prev_script = None;
                }
                CharScript::Punct => {
                    cur.push(ch);
                    raw_tokens.push((std::mem::take(&mut cur), false));
                    prev_script = None;
                }
                _ => {
                    // 日本語の文字種遷移（例: 漢字→カタカナ、ひらがな→漢字、ASCII→日本語）や長すぎるチャンクを自然に区切る
                    let should_split = match prev_script {
                        None => false,
                        Some(p) if p == sc => {
                            cur.chars().count() >= 7 && sc != CharScript::AsciiWord
                        }
                        // 漢字の直後に続く送り仮名・助詞（ひらがな）は同一文節にまとめ、ひらがな→漢字/カタカナで区切る
                        Some(CharScript::KanjiOrOther) if sc == CharScript::Hiragana => {
                            cur.chars().count() >= 6
                        }
                        Some(_) => !cur.is_empty(),
                    };
                    if should_split {
                        raw_tokens.push((std::mem::take(&mut cur), false));
                    }
                    cur.push(ch);
                    prev_script = Some(sc);
                }
            }
        }
        if !cur.is_empty() {
            raw_tokens.push((cur, false));
        }
    }

    let total_weight: f32 = raw_tokens
        .iter()
        .filter(|(_, nl)| !*nl)
        .map(|(t, _)| (t.trim().chars().count().max(1)) as f32)
        .sum::<f32>()
        .max(1.0);

    let mut out = Vec::with_capacity(raw_tokens.len());
    let mut acc = 0.0f32;
    for (text, is_newline) in raw_tokens {
        if is_newline {
            out.push(KineticWordSpan {
                text,
                start_frac: acc / total_weight,
                end_frac: acc / total_weight,
                is_emphasized: false,
                is_newline: true,
            });
            continue;
        }
        let w = (text.trim().chars().count().max(1)) as f32;
        let start_frac = acc / total_weight;
        acc += w;
        let end_frac = (acc / total_weight).min(1.0);
        let trimmed_lower = text.trim().to_ascii_lowercase();
        let is_emphasized = emphasis_keywords.iter().any(|kw| {
            let k = kw.trim();
            !k.is_empty() && (text.contains(k) || trimmed_lower.contains(&k.to_ascii_lowercase()))
        });
        out.push(KineticWordSpan {
            text,
            start_frac,
            end_frac,
            is_emphasized,
            is_newline: false,
        });
    }
    out
}

/// 指定進行度 `progress ∈ [0.0, 1.0]` に応じて、AviUtl2 制御文字（`<#RRGGBB>`, `<sNN>`）を動的に埋め込んだ字幕マークアップを生成する
pub fn generate_kinetic_inline_markup(
    raw_content: &str,
    progress: f32,
    base_font_size: f32,
    cfg: &KineticSubtitleConfig,
) -> String {
    if !cfg.enabled {
        return raw_content.to_string();
    }
    let spans = tokenize_kinetic_words(raw_content, &cfg.auto_emphasize_keywords);
    if spans.is_empty() {
        return raw_content.to_string();
    }
    let p = progress.clamp(0.0, 1.0);
    let active_hex = cfg.active_color.to_hex_rgb();
    let inactive_hex = cfg.inactive_color.to_hex_rgb();
    let emph_hex = cfg.emphasis_color.to_hex_rgb();
    let dim_hex = RgbaColor {
        r: cfg.inactive_color.r * 0.62,
        g: cfg.inactive_color.g * 0.62,
        b: cfg.inactive_color.b * 0.62,
        a: 1.0,
    }
    .to_hex_rgb();

    let base_sz = base_font_size.clamp(8.0, 400.0).round() as u32;
    let pop_sz = (base_font_size * cfg.pop_scale.clamp(1.0, 1.8))
        .clamp(8.0, 480.0)
        .round() as u32;

    let mut out = String::with_capacity(raw_content.len() * 2 + 64);
    for span in &spans {
        if span.is_newline {
            out.push('\n');
            continue;
        }
        let is_active = p >= span.start_frac && (p < span.end_frac || span.end_frac >= 0.999);
        let is_passed = p >= span.end_frac;

        let (hex, sz) = match cfg.mode {
            KineticHighlightMode::WordPopColor => {
                if is_active {
                    let c = if span.is_emphasized {
                        &emph_hex
                    } else {
                        &active_hex
                    };
                    (c.as_str(), pop_sz)
                } else if span.is_emphasized {
                    (emph_hex.as_str(), base_sz)
                } else {
                    (inactive_hex.as_str(), base_sz)
                }
            }
            KineticHighlightMode::KaraokeSweep => {
                if is_active || is_passed {
                    let c = if span.is_emphasized {
                        &emph_hex
                    } else {
                        &active_hex
                    };
                    (c.as_str(), if is_active { pop_sz } else { base_sz })
                } else {
                    (dim_hex.as_str(), base_sz)
                }
            }
            KineticHighlightMode::ProgressiveReveal => {
                if is_active {
                    (active_hex.as_str(), pop_sz)
                } else if is_passed {
                    (
                        if span.is_emphasized {
                            emph_hex.as_str()
                        } else {
                            inactive_hex.as_str()
                        },
                        base_sz,
                    )
                } else {
                    (dim_hex.as_str(), base_sz)
                }
            }
            KineticHighlightMode::KeywordPulse => {
                if span.is_emphasized {
                    (
                        emph_hex.as_str(),
                        if is_active {
                            pop_sz
                        } else {
                            (base_sz + pop_sz) / 2
                        },
                    )
                } else if is_active {
                    (active_hex.as_str(), pop_sz)
                } else {
                    (inactive_hex.as_str(), base_sz)
                }
            }
        };

        out.push_str(&format!("<{hex}><s{sz}>{}", span.text));
    }
    out
}

// =========================================================================
// 2. Multi-Language Subtitle Localization Table & 1-Undo Switcher
// =========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalizationEntry {
    pub item_id: ItemId,
    pub item_name: String,
    pub start_sec: f64,
    /// 言語コード（"ja", "en", "zh", "ko" 等）ごとの翻訳テキスト
    pub translations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalizationTable {
    pub locales: Vec<String>,
    pub entries: Vec<LocalizationEntry>,
}

/// シーン内の全テキスト／字幕アイテムから多言語ローカライズテーブルを抽出する
pub fn extract_localization_table(scene: &Scene, source_locale: &str) -> LocalizationTable {
    let locale = if source_locale.trim().is_empty() {
        "ja"
    } else {
        source_locale.trim()
    };
    let mut entries = Vec::new();
    for layer in &scene.layers {
        for item in &layer.items {
            if let ItemKind::Text { content, .. } = &item.kind {
                let mut map = BTreeMap::new();
                map.insert(locale.to_string(), content.clone());
                entries.push(LocalizationEntry {
                    item_id: item.id,
                    item_name: item.name.clone(),
                    start_sec: item.start.as_seconds(),
                    translations: map,
                });
            }
        }
    }
    entries.sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
    LocalizationTable {
        locales: vec![locale.to_string()],
        entries,
    }
}

/// 多言語テーブルをスプレッドシート互換 CSV（`item_id,start_sec,item_name,ja,en,...`）へ書き出す
pub fn export_localization_csv(table: &LocalizationTable) -> String {
    let mut locales = table.locales.clone();
    for e in &table.entries {
        for k in e.translations.keys() {
            if !locales.contains(k) {
                locales.push(k.clone());
            }
        }
    }
    if locales.is_empty() {
        locales.push("ja".into());
    }

    let mut out = format!("item_id,start_sec,item_name,{}\n", locales.join(","));
    for e in &table.entries {
        let mut row = vec![
            e.item_id.to_string(),
            format!("{:.3}", e.start_sec),
            escape_csv(&e.item_name),
        ];
        for loc in &locales {
            let text = e.translations.get(loc).map(String::as_str).unwrap_or("");
            row.push(escape_csv(text));
        }
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// CSV 形式の多言語翻訳テーブルを読み込む
pub fn import_localization_csv(csv_text: &str) -> LocalizationTable {
    let mut lines = csv_text.lines().filter(|l| !l.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return LocalizationTable::default();
    };
    let headers = parse_csv_line(header_line);
    if headers.len() < 4 {
        return LocalizationTable::default();
    }
    let locales: Vec<String> = headers[3..]
        .iter()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .collect();

    let mut entries = Vec::new();
    for line in lines {
        let cols = parse_csv_line(line);
        let Some(id) = cols.first().and_then(|s| Uuid::parse_str(s.trim()).ok()) else {
            continue;
        };
        let start_sec = cols
            .get(1)
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
        let item_name = cols.get(2).cloned().unwrap_or_default();
        let mut translations = BTreeMap::new();
        for (i, loc) in locales.iter().enumerate() {
            if let Some(val) = cols.get(3 + i)
                && !val.is_empty()
            {
                translations.insert(loc.clone(), val.clone());
            }
        }
        entries.push(LocalizationEntry {
            item_id: id,
            item_name,
            start_sec,
            translations,
        });
    }

    LocalizationTable { locales, entries }
}

/// 指定言語（例: `"en"`）の翻訳テキストをシーン内テキストアイテムへ一括反映する 1-Undo コマンド列を生成する
pub fn build_apply_locale_commands(
    scene: &Scene,
    table: &LocalizationTable,
    target_locale: &str,
) -> Vec<EditCommand> {
    let loc = target_locale.trim();
    let existing_ids: std::collections::BTreeSet<ItemId> = scene
        .layers
        .iter()
        .flat_map(|l| l.items.iter().map(|i| i.id))
        .collect();

    let mut cmds = Vec::new();
    for entry in &table.entries {
        if existing_ids.contains(&entry.item_id)
            && let Some(translated) = entry.translations.get(loc)
            && !translated.is_empty()
        {
            cmds.push(EditCommand::SetTextContent {
                item_id: entry.item_id,
                content: translated.clone(),
            });
        }
    }
    cmds
}

fn escape_csv(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_q = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_q = true;
        } else if c == ',' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{TextAnimator, TimelineItem};
    use crate::time::{FrameRate, RationalTime};

    #[test]
    fn kinetic_subtitle_tokenizes_japanese_and_english_and_highlights_active_word() {
        let cfg = KineticSubtitleConfig {
            enabled: true,
            mode: KineticHighlightMode::WordPopColor,
            ..Default::default()
        };
        let spans = tokenize_kinetic_words(
            "ここが重要なポイント！Hello World",
            &cfg.auto_emphasize_keywords,
        );
        assert!(
            spans.len() >= 4,
            "should split into multiple word spans: {spans:?}"
        );
        assert!(spans.iter().any(|s| s.is_emphasized));

        let markup_start = generate_kinetic_inline_markup("速報 テスト字幕", 0.1, 50.0, &cfg);
        let markup_end = generate_kinetic_inline_markup("速報 テスト字幕", 0.9, 50.0, &cfg);
        assert!(markup_start.contains("<#"));
        assert!(markup_start.contains("<s59>"));
        assert_ne!(markup_start, markup_end);
    }

    #[test]
    fn localization_table_csv_roundtrip_and_locale_switch_commands() {
        let mut scene = Scene::new("LocTest", 1280, 720, FrameRate::FPS_60);
        let item = TimelineItem::new(
            "Title",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Text {
                content: "こんにちは世界".into(),
                font_family: "sans-serif".into(),
                font_size: 48.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        let id = item.id;
        scene.layers[0].items.push(item);

        let mut table = extract_localization_table(&scene, "ja");
        table.locales.push("en".into());
        table.entries[0]
            .translations
            .insert("en".into(), "Hello, World!".into());

        let csv = export_localization_csv(&table);
        let reimported = import_localization_csv(&csv);
        assert_eq!(reimported.entries.len(), 1);
        assert_eq!(
            reimported.entries[0]
                .translations
                .get("en")
                .map(String::as_str),
            Some("Hello, World!")
        );

        let cmds = build_apply_locale_commands(&scene, &reimported, "en");
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            EditCommand::SetTextContent { item_id, content } if *item_id == id && content == "Hello, World!"
        ));
    }
}
