use crate::command::EditCommand;
use crate::project::{
    ItemKind, Project, RgbaColor, Scene, SceneId, ShapeKind, TextAnimator, TextStyle, TimelineItem,
    Transform2D,
};
use crate::time::{FrameRate, RationalTime};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// SRT / ASS 共通の中間字幕エントリ
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleEntry {
    pub start: RationalTime,
    pub end: RationalTime,
    pub text: String,
}

/// SRT テキストをパースして字幕エントリ列へ変換する
pub fn parse_srt_subtitles(srt: &str) -> Vec<SubtitleEntry> {
    let normalized = srt.replace("\r\n", "\n");
    let mut entries = Vec::new();
    for block in normalized.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let Some(arrow_idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let parts: Vec<&str> = lines[arrow_idx].split("-->").collect();
        if parts.len() != 2 {
            continue;
        }
        if let (Some(start), Some(end)) =
            (parse_srt_ts(parts[0].trim()), parse_srt_ts(parts[1].trim()))
            && end > start
        {
            let text = lines[arrow_idx + 1..].join("\n");
            if !text.is_empty() {
                entries.push(SubtitleEntry { start, end, text });
            }
        }
    }
    entries.sort_by_key(|e| e.start);
    entries
}

fn parse_srt_ts(s: &str) -> Option<RationalTime> {
    let token = s.split_whitespace().next()?;
    let cleaned = token.replace(',', ".");
    let parts: Vec<&str> = cleaned.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    if !h.is_finite() || !m.is_finite() || !sec.is_finite() {
        return None;
    }
    Some(RationalTime::from_seconds(
        (h * 3600.0 + m * 60.0 + sec).max(0.0),
    ))
}

fn format_srt_ts(t: RationalTime) -> String {
    let total_ms = (t.as_seconds().max(0.0) * 1000.0).round() as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// シーン内の全テキストオブジェクトを標準 SRT 形式文字列へ書き出す（YouTube 字幕直結）
pub fn export_srt_subtitles(scene: &Scene) -> String {
    let mut rows: Vec<(RationalTime, RationalTime, String)> = Vec::new();
    for layer in &scene.layers {
        if !layer.visible {
            continue;
        }
        for item in &layer.items {
            if let ItemKind::Text { content, .. } = &item.kind {
                let clean = strip_inline_control_tags(content);
                if !clean.trim().is_empty() {
                    rows.push((item.start, item.end(), clean));
                }
            }
        }
    }
    rows.sort_by_key(|(s, _, _)| *s);

    let mut out = String::new();
    for (idx, (start, end, text)) in rows.into_iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            idx + 1,
            format_srt_ts(start),
            format_srt_ts(end),
            text
        ));
    }
    out
}

/// ASS (SubStation Alpha v4+) 字幕ファイルをパースする
pub fn parse_ass_subtitles(ass: &str) -> Vec<SubtitleEntry> {
    let mut entries = Vec::new();
    for raw in ass.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("Dialogue:") else {
            continue;
        };
        // Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
        let mut fields = rest.splitn(10, ',');
        let _layer = fields.next();
        let Some(start_s) = fields.next() else {
            continue;
        };
        let Some(end_s) = fields.next() else {
            continue;
        };
        for _ in 0..6 {
            if fields.next().is_none() {
                break;
            }
        }
        let Some(raw_text) = fields.next() else {
            continue;
        };
        if let (Some(start), Some(end)) = (parse_ass_ts(start_s.trim()), parse_ass_ts(end_s.trim()))
            && end > start
        {
            let text = strip_ass_override_tags(raw_text)
                .replace("\\N", "\n")
                .replace("\\n", "\n");
            if !text.trim().is_empty() {
                entries.push(SubtitleEntry { start, end, text });
            }
        }
    }
    entries.sort_by_key(|e| e.start);
    entries
}

fn parse_ass_ts(s: &str) -> Option<RationalTime> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    Some(RationalTime::from_seconds(
        (h * 3600.0 + m * 60.0 + sec).max(0.0),
    ))
}

fn format_ass_ts(t: RationalTime) -> String {
    let total_cs = (t.as_seconds().max(0.0) * 100.0).round() as u64;
    let h = total_cs / 360_000;
    let m = (total_cs % 360_000) / 6_000;
    let s = (total_cs % 6_000) / 100;
    let cs = total_cs % 100;
    format!("{h}:{m:02}:{s:02}.{cs:02}")
}

fn rgba_to_ass_color(c: RgbaColor) -> String {
    let r = (c.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (c.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (c.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    // ASS alpha is inverted: 00 = opaque, FF = transparent
    let a = ((1.0 - c.a.clamp(0.0, 1.0)) * 255.0).round() as u8;
    format!("&H{a:02X}{b:02X}{g:02X}{r:02X}")
}

/// シーン内テキストをスタイル付き ASS (v4+) 字幕ファイルへ書き出す
pub fn export_ass_subtitles(scene: &Scene) -> String {
    let mut font_name = "Sans".to_string();
    let mut font_size = 46u32;
    let mut primary = RgbaColor::WHITE;
    let mut outline = RgbaColor::BLACK;
    let mut outline_w = 2.0f32;
    let mut rows: Vec<(RationalTime, RationalTime, String)> = Vec::new();

    for layer in &scene.layers {
        if !layer.visible {
            continue;
        }
        for item in &layer.items {
            if let ItemKind::Text {
                content,
                font_family,
                font_size: fs,
                color,
                ..
            } = &item.kind
            {
                if rows.is_empty() {
                    if !font_family.is_empty() {
                        font_name = font_family.clone();
                    }
                    font_size = (*fs).round().clamp(12.0, 200.0) as u32;
                    primary = *color;
                    outline = item.text_style.outline_color;
                    outline_w = item.text_style.outline_width.max(1.0);
                }
                let clean = strip_inline_control_tags(content).replace('\n', "\\N");
                if !clean.trim().is_empty() {
                    rows.push((item.start, item.end(), clean));
                }
            }
        }
    }
    rows.sort_by_key(|(s, _, _)| *s);

    let mut out = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {}\nPlayResY: {}\nScaledBorderAndShadow: yes\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,{},{},{},&H000000FF,{},&H64000000,0,0,0,0,100,100,0,0,1,{:.1},1,2,40,40,40,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        scene.width,
        scene.height,
        font_name,
        font_size,
        rgba_to_ass_color(primary),
        rgba_to_ass_color(outline),
        outline_w
    );
    for (start, end, text) in rows {
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
            format_ass_ts(start),
            format_ass_ts(end),
            text
        ));
    }
    out
}

/// 字幕エントリ列をタイムライン配置コマンド（1 ステップ Undo）へ変換する
pub fn subtitle_entries_to_commands(
    entries: &[SubtitleEntry],
    layer_index: usize,
    scene_w: u32,
    scene_h: u32,
) -> Option<EditCommand> {
    if entries.is_empty() {
        return None;
    }
    let commands: Vec<EditCommand> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let dur = (e.end - e.start).max(RationalTime::from_seconds(0.1));
            let mut item = TimelineItem::new(
                format!("字幕 {}", i + 1),
                e.start,
                dur,
                ItemKind::Text {
                    content: e.text.clone(),
                    font_family: "sans-serif".to_string(),
                    font_size: 44.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            );
            item.transform = Transform2D {
                x: (scene_w as f32 * 0.10).round(),
                y: (scene_h as f32 * 0.78).round(),
                ..Default::default()
            };
            item.text_style = TextStyle {
                outline_width: 2.5,
                shadow_offset: [2.0, 2.0],
                ..Default::default()
            };
            EditCommand::AddObject { layer_index, item }
        })
        .collect();
    Some(EditCommand::Batch {
        label: "import_subtitles_file".into(),
        commands,
    })
}

/// AviUtl `.exo` ファイルをパースし、基本オブジェクト（テキスト・図形・座標・色・透明度・拡大率）を
/// `(layer_index, TimelineItem)` 列へ変換する。
/// AviUtl 固有の UTF-16LE 16進エンコード `text=` フィールドもネイティブ復号する。
pub fn parse_aviutl_exo(exo: &str, fallback_fps: FrameRate) -> Vec<(usize, TimelineItem)> {
    let mut fps = fallback_fps;
    let mut canvas_w = 1280.0_f32;
    let mut canvas_h = 720.0_f32;

    struct ExoObj {
        start_f: i64,
        end_f: i64,
        layer: usize,
        kind_name: String,
        text_hex: String,
        font: String,
        size: f32,
        color: RgbaColor,
        x: f32,
        y: f32,
        scale_pct: f32,
        alpha_pct: f32,
        rotation_deg: f32,
        shape_size: f32,
        file_name: String,
    }

    impl Default for ExoObj {
        fn default() -> Self {
            Self {
                start_f: 0,
                end_f: 60,
                layer: 0,
                kind_name: String::new(),
                text_hex: String::new(),
                font: String::new(),
                size: 42.0,
                color: RgbaColor::WHITE,
                x: 0.0,
                y: 0.0,
                scale_pct: 100.0,
                alpha_pct: 0.0,
                rotation_deg: 0.0,
                shape_size: 160.0,
                file_name: String::new(),
            }
        }
    }

    let mut objs: std::collections::BTreeMap<u32, ExoObj> = std::collections::BTreeMap::new();
    let mut cur_section = String::new();
    let mut rate_num: u32 = 0;
    let mut rate_scale: u32 = 1;

    for raw in exo.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            cur_section = line[1..line.len() - 1].to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();

        if cur_section == "exedit" {
            match key {
                "width" => {
                    if let Ok(w) = val.parse::<f32>() {
                        canvas_w = w.max(16.0);
                    }
                }
                "height" => {
                    if let Ok(h) = val.parse::<f32>() {
                        canvas_h = h.max(16.0);
                    }
                }
                "rate" => {
                    if let Ok(r) = val.parse::<u32>() {
                        rate_num = r;
                    }
                }
                "scale" => {
                    if let Ok(s) = val.parse::<u32>() {
                        rate_scale = s.max(1);
                    }
                }
                _ => {}
            }
            if rate_num > 0 {
                fps = FrameRate::new(rate_num, rate_scale);
            }
            continue;
        }

        // `[N]` or `[N.0]` / `[N.1]`
        let (obj_num_str, sub_idx) = match cur_section.split_once('.') {
            Some((a, b)) => (a, Some(b)),
            None => (cur_section.as_str(), None),
        };
        let Ok(obj_id) = obj_num_str.parse::<u32>() else {
            continue;
        };
        let entry = objs.entry(obj_id).or_default();

        match sub_idx {
            None => match key {
                "start" => {
                    entry.start_f = val.parse::<i64>().unwrap_or(1).saturating_sub(1).max(0);
                }
                "end" => {
                    entry.end_f = val.parse::<i64>().unwrap_or(60).max(1);
                }
                "layer" => {
                    entry.layer = val.parse::<usize>().unwrap_or(1).saturating_sub(1);
                }
                _ => {}
            },
            Some(_) => match key {
                "_name" => {
                    if entry.kind_name.is_empty()
                        || matches!(
                            val,
                            "テキスト"
                                | "Text"
                                | "図形"
                                | "Shape"
                                | "画像ファイル"
                                | "動画ファイル"
                                | "音声ファイル"
                        )
                    {
                        entry.kind_name = val.to_string();
                    }
                }
                "text" => entry.text_hex = val.to_string(),
                "font" => entry.font = val.to_string(),
                "サイズ" | "size" => {
                    if let Ok(sz) = parse_exo_first_f32(val) {
                        entry.size = sz.clamp(8.0, 300.0);
                        entry.shape_size = sz.clamp(16.0, 2000.0);
                    }
                }
                "color" => {
                    if let Some(c) = parse_hex_rgb(val) {
                        entry.color = c;
                    }
                }
                "X" => {
                    if let Ok(x) = parse_exo_first_f32(val) {
                        entry.x = x;
                    }
                }
                "Y" => {
                    if let Ok(y) = parse_exo_first_f32(val) {
                        entry.y = y;
                    }
                }
                "拡大率" => {
                    if let Ok(s) = parse_exo_first_f32(val) {
                        entry.scale_pct = s.clamp(1.0, 2000.0);
                    }
                }
                "透明度" => {
                    if let Ok(a) = parse_exo_first_f32(val) {
                        entry.alpha_pct = a.clamp(0.0, 100.0);
                    }
                }
                "回転" => {
                    if let Ok(r) = parse_exo_first_f32(val) {
                        entry.rotation_deg = r;
                    }
                }
                "file" => entry.file_name = val.to_string(),
                _ => {}
            },
        }
    }

    let mut out = Vec::new();
    for (idx, obj) in objs {
        if obj.end_f <= obj.start_f {
            continue;
        }
        let start = RationalTime::from_frame(obj.start_f, fps);
        let dur = RationalTime::from_frame((obj.end_f - obj.start_f).max(1), fps);
        // AviUtl の X/Y は画面中央 (0, 0) 基準 → 左上基準へ変換
        let tf = Transform2D {
            x: canvas_w * 0.5 + obj.x - obj.shape_size * 0.5,
            y: canvas_h * 0.5 + obj.y - obj.size * 0.5,
            scale_x: obj.scale_pct / 100.0,
            scale_y: obj.scale_pct / 100.0,
            rotation_deg: obj.rotation_deg,
            opacity: 1.0 - (obj.alpha_pct / 100.0),
        };

        let item = match obj.kind_name.as_str() {
            "テキスト" | "Text" => {
                let decoded = decode_exo_utf16le_hex(&obj.text_hex);
                let text = if decoded.is_empty() {
                    format!("EXO テキスト {idx}")
                } else {
                    decoded
                };
                let mut it = TimelineItem::new(
                    format!("EXO: {}", text.lines().next().unwrap_or("Text")),
                    start,
                    dur,
                    ItemKind::Text {
                        content: text,
                        font_family: if obj.font.is_empty() {
                            "sans-serif".to_string()
                        } else {
                            obj.font
                        },
                        font_size: obj.size,
                        color: obj.color,
                        animator: TextAnimator::default(),
                    },
                );
                it.transform = tf;
                Some(it)
            }
            "図形" | "Shape" => {
                let mut it = TimelineItem::new(
                    format!("EXO 図形 {idx}"),
                    start,
                    dur,
                    ItemKind::Shape {
                        shape: ShapeKind::Rectangle,
                        width: obj.shape_size,
                        height: obj.shape_size,
                        color: obj.color,
                    },
                );
                it.transform = tf;
                Some(it)
            }
            _ if !obj.file_name.is_empty() => {
                // ファイル参照オブジェクトは配置マーカー用テキストとして安全に取り込む（パス参照は再リンク可）
                let mut it = TimelineItem::new(
                    format!("EXO素材: {}", obj.file_name),
                    start,
                    dur,
                    ItemKind::Shape {
                        shape: ShapeKind::RoundedRectangle,
                        width: 220.0,
                        height: 80.0,
                        color: RgbaColor {
                            r: 0.25,
                            g: 0.45,
                            b: 0.72,
                            a: 0.85,
                        },
                    },
                );
                it.transform = tf;
                Some(it)
            }
            _ => None,
        };
        if let Some(it) = item {
            out.push((obj.layer, it));
        }
    }
    out
}

fn parse_exo_first_f32(val: &str) -> Result<f32, std::num::ParseFloatError> {
    // AviUtl の中間点付きパラメータ例: `100.0,200.0,1` の先頭値を抽出
    val.split(',').next().unwrap_or(val).trim().parse::<f32>()
}

fn parse_hex_rgb(val: &str) -> Option<RgbaColor> {
    let hex = val.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(RgbaColor {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    })
}

/// AviUtl `.exo` の UTF-16LE 16進文字列（4096文字、終端 `0000` 埋め）を UTF-8 文字列へ復号する
pub fn decode_exo_utf16le_hex(hex: &str) -> String {
    let bytes = hex.trim().as_bytes();
    let mut units = Vec::new();
    for chunk in bytes.as_chunks::<4>().0 {
        let Ok(s) = std::str::from_utf8(chunk) else {
            break;
        };
        let (Ok(lo), Ok(hi)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
        ) else {
            break;
        };
        let unit = u16::from_le_bytes([lo, hi]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    String::from_utf16_lossy(&units).replace("\r\n", "\n")
}

/// OpenTimelineIO (`.otio` JSON v1) 形式でアクティブシーンを書き出す
pub fn export_otio_json(project: &Project, scene_id: SceneId) -> Option<String> {
    let scene = project.scene(scene_id)?;
    let rate = scene.fps.as_f64();

    let tracks: Vec<serde_json::Value> = scene
        .layers
        .iter()
        .map(|layer| {
            let is_audio_only = !layer.items.is_empty()
                && layer
                    .items
                    .iter()
                    .all(|i| matches!(i.kind, ItemKind::Audio { .. }));
            let mut items_sorted = layer.items.clone();
            items_sorted.sort_by_key(|i| i.start);
            let mut children = Vec::new();
            let mut cursor_f = 0i64;
            for item in items_sorted {
                let start_f = item.start.to_frame_round(scene.fps).max(0);
                let dur_f = item.duration.to_frame_round(scene.fps).max(1);
                if start_f > cursor_f {
                    children.push(json!({
                        "OTIO_SCHEMA": "Gap.1",
                        "source_range": {
                            "OTIO_SCHEMA": "TimeRange.1",
                            "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": 0.0 },
                            "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": (start_f - cursor_f) as f64 }
                        }
                    }));
                }
                let (media_url, src_start_f) = match &item.kind {
                    ItemKind::Video {
                        asset_id,
                        source_offset,
                        ..
                    }
                    | ItemKind::Audio {
                        asset_id,
                        source_offset,
                        ..
                    } => {
                        let url = project
                            .asset(*asset_id)
                            .and_then(|a| {
                                a.relative_path
                                    .as_ref()
                                    .or(a.fallback_absolute_path.as_ref())
                            })
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        (url, source_offset.to_frame_round(scene.fps).max(0))
                    }
                    ItemKind::Image { asset_id } => {
                        let url = project
                            .asset(*asset_id)
                            .and_then(|a| {
                                a.relative_path
                                    .as_ref()
                                    .or(a.fallback_absolute_path.as_ref())
                            })
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        (url, 0)
                    }
                    _ => (String::new(), 0),
                };
                children.push(json!({
                    "OTIO_SCHEMA": "Clip.2",
                    "name": item.name,
                    "media_reference": if media_url.is_empty() {
                        json!({ "OTIO_SCHEMA": "MissingReference.1" })
                    } else {
                        json!({ "OTIO_SCHEMA": "ExternalReference.1", "target_url": media_url })
                    },
                    "source_range": {
                        "OTIO_SCHEMA": "TimeRange.1",
                        "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": src_start_f as f64 },
                        "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": dur_f as f64 }
                    }
                }));
                cursor_f = start_f + dur_f;
            }
            json!({
                "OTIO_SCHEMA": "Track.1",
                "name": layer.name,
                "kind": if is_audio_only { "Audio" } else { "Video" },
                "children": children
            })
        })
        .collect();

    let markers: Vec<serde_json::Value> = scene
        .markers
        .iter()
        .map(|m| {
            let mf = m.time.to_frame_round(scene.fps).max(0) as f64;
            json!({
                "OTIO_SCHEMA": "Marker.2",
                "name": m.label,
                "marked_range": {
                    "OTIO_SCHEMA": "TimeRange.1",
                    "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": mf },
                    "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": 0.0 }
                }
            })
        })
        .collect();

    let doc = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": format!("{} - {}", project.name, scene.name),
        "global_start_time": {
            "OTIO_SCHEMA": "RationalTime.1",
            "rate": rate,
            "value": 0.0
        },
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "markers": markers,
            "children": tracks
        }
    });
    serde_json::to_string_pretty(&doc).ok()
}

fn strip_inline_control_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' && in_tag {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    out
}

fn strip_ass_override_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_brace = false;
    for ch in s.chars() {
        if ch == '{' {
            in_brace = true;
        } else if ch == '}' && in_brace {
            in_brace = false;
        } else if !in_brace {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectSettings;

    #[test]
    fn srt_and_ass_roundtrip_preserve_timestamps_and_text() {
        let mut proj = Project::new("SubExport", ProjectSettings::default());
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .push(TimelineItem::new(
                "Sub1",
                RationalTime::from_seconds(1.5),
                RationalTime::from_seconds(2.25),
                ItemKind::Text {
                    content: "<#FF0000>赤い字幕</>\n2行目".into(),
                    font_family: "sans-serif".into(),
                    font_size: 48.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
        let scene = proj.active_scene().unwrap();

        let srt = export_srt_subtitles(scene);
        assert!(srt.contains("00:00:01,500 --> 00:00:03,750"));
        let parsed_srt = parse_srt_subtitles(&srt);
        assert_eq!(parsed_srt.len(), 1);
        assert_eq!(parsed_srt[0].text, "赤い字幕\n2行目");

        let ass = export_ass_subtitles(scene);
        assert!(ass.contains("[V4+ Styles]"));
        assert!(ass.contains("Dialogue: 0,0:00:01.50,0:00:03.75"));
        let parsed_ass = parse_ass_subtitles(&ass);
        assert_eq!(parsed_ass.len(), 1);
        assert_eq!(parsed_ass[0].text, "赤い字幕\n2行目");
    }

    #[test]
    fn parses_aviutl_exo_utf16le_hex_japanese_text_and_exports_otio() {
        // "あいう" in UTF-16LE hex: あ(3042)->4230, い(3044)->4430, う(3046)->4630 + 0000
        let exo = "\
[exedit]
width=1920
height=1080
rate=60
scale=1
[0]
start=1
end=121
layer=2
[0.0]
_name=テキスト
サイズ=56
color=ffcc00
font=Meiryo
text=4230443046300000
[0.1]
_name=標準描画
X=100.0
Y=-50.0
拡大率=120.0
透明度=10.0
回転=15.0
";
        let items = parse_aviutl_exo(exo, FrameRate::FPS_60);
        assert_eq!(items.len(), 1);
        let (layer_idx, item) = &items[0];
        assert_eq!(*layer_idx, 1);
        assert!((item.duration.as_seconds() - 2.0).abs() < 0.02);
        assert!((item.transform.scale_x - 1.2).abs() < 0.01);
        assert!((item.transform.rotation_deg - 15.0).abs() < 0.01);
        match &item.kind {
            ItemKind::Text { content, .. } => assert_eq!(content, "あいう"),
            other => panic!("expected Text, got {other:?}"),
        }

        // OTIO JSON export check
        let mut proj = Project::new("OtioDemo", ProjectSettings::default());
        proj.active_scene_mut().unwrap().layers[1]
            .items
            .push(item.clone());
        let otio_str = export_otio_json(&proj, proj.active_scene_id).unwrap();
        let val: serde_json::Value = serde_json::from_str(&otio_str).unwrap();
        assert_eq!(val["OTIO_SCHEMA"], "Timeline.1");
        assert_eq!(val["tracks"]["OTIO_SCHEMA"], "Stack.1");
        assert_eq!(
            val["tracks"]["children"][1]["children"][0]["OTIO_SCHEMA"],
            "Clip.2"
        );
    }
}
