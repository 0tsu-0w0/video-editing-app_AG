use crate::color::{LinearImage, LinearPremul};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use std::collections::{BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use ve_core::{ColorPalette, RgbaColor, TextAnimator, TextStyle};

/// AviUtl2 インライン制御文字（`<#ff0000>`, `<@強調赤>`, `<s64>`, `<fFont>`, `<p10,-5>`, `<b1>`, `</>`）で分割された装飾スパン
#[derive(Debug, Clone, PartialEq)]
pub struct StyledSpan {
    pub text: String,
    pub font_family: String,
    pub font_size: f32,
    pub color: RgbaColor,
    pub bold: bool,
    pub offset_x: f32,
    pub offset_y: f32,
}

/// AviUtl2 準拠のインラインテキスト制御文字をパースし、装飾スパン列へ展開する
pub fn parse_inline_text_spans(
    input: &str,
    base_font: &str,
    base_size: f32,
    base_color: RgbaColor,
    palette: &ColorPalette,
) -> Vec<StyledSpan> {
    let mut spans: Vec<StyledSpan> = Vec::new();
    let mut cur_font = base_font.to_string();
    let mut cur_size = base_size;
    let mut cur_color = base_color;
    let mut cur_bold = false;
    let mut cur_ox = 0.0f32;
    let mut cur_oy = 0.0f32;
    let mut buf = String::new();

    let flush = |spans: &mut Vec<StyledSpan>,
                 buf: &mut String,
                 font: &str,
                 size: f32,
                 color: RgbaColor,
                 bold: bool,
                 ox: f32,
                 oy: f32| {
        if !buf.is_empty() {
            spans.push(StyledSpan {
                text: std::mem::take(buf),
                font_family: font.to_string(),
                font_size: size,
                color,
                bold,
                offset_x: ox,
                offset_y: oy,
            });
        }
    };

    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'<' {
            buf.push('<');
            i += 2;
            continue;
        }
        if bytes[i] == b'<'
            && let Some(rel_end) = input[i + 1..].find('>')
        {
            let tag = &input[i + 1..i + 1 + rel_end];
            let mut recognized = true;

            if tag == "/" {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_font = base_font.to_string();
                cur_size = base_size;
                cur_color = base_color;
                cur_bold = false;
                cur_ox = 0.0;
                cur_oy = 0.0;
            } else if tag == "#" || tag == "@" {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_color = base_color;
            } else if let Some(hex) = tag.strip_prefix('#') {
                if let Some(parsed) = RgbaColor::from_hex(hex) {
                    flush(
                        &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox,
                        cur_oy,
                    );
                    cur_color = parsed;
                } else {
                    recognized = false;
                }
            } else if let Some(name) = tag.strip_prefix('@') {
                if let Some(parsed) = palette.find_by_name(name.trim()) {
                    flush(
                        &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox,
                        cur_oy,
                    );
                    cur_color = parsed;
                } else {
                    recognized = false;
                }
            } else if tag == "s" {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_size = base_size;
            } else if let Some(sz_str) = tag.strip_prefix('s') {
                if let Ok(sz) = sz_str.trim().parse::<f32>()
                    && sz.is_finite()
                {
                    flush(
                        &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox,
                        cur_oy,
                    );
                    cur_size = sz.clamp(6.0, 400.0);
                } else {
                    recognized = false;
                }
            } else if tag == "f" {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_font = base_font.to_string();
            } else if let Some(font_name) = tag.strip_prefix('f') {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_font = font_name.trim().to_string();
            } else if tag == "p" {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_ox = 0.0;
                cur_oy = 0.0;
            } else if let Some(pos_str) = tag.strip_prefix('p') {
                let mut parts = pos_str.split(',');
                if let (Some(xs), Some(ys)) = (parts.next(), parts.next())
                    && let (Ok(ox), Ok(oy)) = (xs.trim().parse::<f32>(), ys.trim().parse::<f32>())
                    && ox.is_finite()
                    && oy.is_finite()
                {
                    flush(
                        &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox,
                        cur_oy,
                    );
                    cur_ox = ox.clamp(-500.0, 500.0);
                    cur_oy = oy.clamp(-500.0, 500.0);
                } else {
                    recognized = false;
                }
            } else if tag == "b1" || tag == "b" {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_bold = true;
            } else if tag == "b0" || tag == "/b" {
                flush(
                    &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
                );
                cur_bold = false;
            } else {
                recognized = false;
            }

            if recognized {
                i += rel_end + 2;
                continue;
            }
        }

        let ch = input[i..].chars().next().unwrap();
        buf.push(ch);
        i += ch.len_utf8();
    }

    flush(
        &mut spans, &mut buf, &cur_font, cur_size, cur_color, cur_bold, cur_ox, cur_oy,
    );
    spans
}

/// 制御タグを取り除いたプレーン文字列を返す
pub fn strip_inline_tags(input: &str, palette: &ColorPalette) -> String {
    parse_inline_text_spans(input, "sans-serif", 32.0, RgbaColor::WHITE, palette)
        .into_iter()
        .map(|s| s.text)
        .collect()
}

const MAX_TEXT_CACHE_ENTRIES: usize = 64;
const MAX_TEXT_CACHE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextCacheKey {
    hash_a: u64,
    hash_b: u64,
    width: u32,
    height: u32,
}

/// 透過領域を切り落とした省メモリなテキスト描画キャッシュエントリ
#[derive(Debug, Clone)]
struct CachedTextBbox {
    ox: u32,
    oy: u32,
    sub_w: u32,
    sub_h: u32,
    pixels: Vec<LinearPremul>,
    last_used_tick: u64,
}

impl CachedTextBbox {
    fn from_image(img: &LinearImage, tick: u64) -> Self {
        let Some((min_x, min_y, max_x, max_y)) = alpha_bounds(img) else {
            return Self {
                ox: 0,
                oy: 0,
                sub_w: 0,
                sub_h: 0,
                pixels: Vec::new(),
                last_used_tick: tick,
            };
        };
        let sub_w = max_x - min_x + 1;
        let sub_h = max_y - min_y + 1;
        let w = img.width as usize;
        let mut pixels = Vec::with_capacity((sub_w as usize) * (sub_h as usize));
        for y in min_y..=max_y {
            let row_start = (y as usize) * w + (min_x as usize);
            pixels.extend_from_slice(&img.pixels[row_start..row_start + sub_w as usize]);
        }
        Self {
            ox: min_x,
            oy: min_y,
            sub_w,
            sub_h,
            pixels,
            last_used_tick: tick,
        }
    }

    fn byte_len(&self) -> usize {
        self.pixels.len() * std::mem::size_of::<LinearPremul>()
    }

    fn expand_to_image(&self, full_w: u32, full_h: u32) -> LinearImage {
        let mut out = LinearImage::new(full_w, full_h, LinearPremul::TRANSPARENT);
        if self.sub_w == 0 || self.sub_h == 0 {
            return out;
        }
        let dst_w = full_w as usize;
        let sw = self.sub_w as usize;
        for row in 0..(self.sub_h as usize) {
            let src_start = row * sw;
            let dst_start = (self.oy as usize + row) * dst_w + (self.ox as usize);
            out.pixels[dst_start..dst_start + sw]
                .copy_from_slice(&self.pixels[src_start..src_start + sw]);
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_text_cache_key(
    text: &str,
    font_family: &str,
    font_size: f32,
    color: RgbaColor,
    style: &TextStyle,
    palette: &ColorPalette,
    animator_settled: bool,
    width: u32,
    height: u32,
) -> TextCacheKey {
    let mut h1 = DefaultHasher::new();
    let mut h2 = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |bytes: &[u8]| {
        bytes.hash(&mut h1);
        for &b in bytes {
            h2 ^= u64::from(b);
            h2 = h2.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    mix(text.as_bytes());
    mix(&[0xFF]);
    mix(font_family.as_bytes());
    mix(&font_size.to_bits().to_le_bytes());
    mix(&color.r.to_bits().to_le_bytes());
    mix(&color.g.to_bits().to_le_bytes());
    mix(&color.b.to_bits().to_le_bytes());
    mix(&color.a.to_bits().to_le_bytes());
    mix(&style.outline_width.to_bits().to_le_bytes());
    mix(&style.outline_color.r.to_bits().to_le_bytes());
    mix(&style.outline_color.g.to_bits().to_le_bytes());
    mix(&style.outline_color.b.to_bits().to_le_bytes());
    mix(&style.outline_color.a.to_bits().to_le_bytes());
    mix(&style.shadow_offset[0].to_bits().to_le_bytes());
    mix(&style.shadow_offset[1].to_bits().to_le_bytes());
    mix(&style.shadow_blur.to_bits().to_le_bytes());
    mix(&style.shadow_color.r.to_bits().to_le_bytes());
    mix(&style.shadow_color.g.to_bits().to_le_bytes());
    mix(&style.shadow_color.b.to_bits().to_le_bytes());
    mix(&style.shadow_color.a.to_bits().to_le_bytes());
    mix(&[u8::from(style.box_enabled), u8::from(animator_settled)]);
    mix(&style.box_color.r.to_bits().to_le_bytes());
    mix(&style.box_color.g.to_bits().to_le_bytes());
    mix(&style.box_color.b.to_bits().to_le_bytes());
    mix(&style.box_color.a.to_bits().to_le_bytes());
    mix(&style.box_padding[0].to_bits().to_le_bytes());
    mix(&style.box_padding[1].to_bits().to_le_bytes());
    mix(&style.box_radius.to_bits().to_le_bytes());
    if text.contains("<@") {
        for entry in &palette.colors {
            mix(entry.name.as_bytes());
            mix(&entry.color.r.to_bits().to_le_bytes());
            mix(&entry.color.g.to_bits().to_le_bytes());
            mix(&entry.color.b.to_bits().to_le_bytes());
            mix(&entry.color.a.to_bits().to_le_bytes());
        }
    }
    TextCacheKey {
        hash_a: h1.finish(),
        hash_b: h2,
        width,
        height,
    }
}

/// `cosmic-text` + `swash` による動画内テキスト描画エンジン
/// （日本語・マルチライン・AviUtl2インライン制御文字・縁取り/シャドウ/座布団・文字ごとアニメーター・省メモリ BBox LRU キャッシュ対応）
pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_cache: HashMap<TextCacheKey, CachedTextBbox>,
    text_cache_bytes: usize,
    text_cache_tick: u64,
    text_cache_hits: u64,
    text_cache_misses: u64,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            text_cache: HashMap::new(),
            text_cache_bytes: 0,
            text_cache_tick: 0,
            text_cache_hits: 0,
            text_cache_misses: 0,
        }
    }

    pub fn text_cache_hits(&self) -> u64 {
        self.text_cache_hits
    }

    pub fn text_cache_misses(&self) -> u64 {
        self.text_cache_misses
    }

    pub fn text_cache_bytes(&self) -> usize {
        self.text_cache_bytes
    }

    pub fn clear_text_cache(&mut self) {
        self.text_cache.clear();
        self.text_cache_bytes = 0;
    }

    fn insert_text_cache(&mut self, key: TextCacheKey, img: &LinearImage) {
        self.text_cache_tick = self.text_cache_tick.wrapping_add(1);
        let entry = CachedTextBbox::from_image(img, self.text_cache_tick);
        let entry_bytes = entry.byte_len();
        if entry_bytes > MAX_TEXT_CACHE_BYTES {
            return;
        }
        if let Some(prev) = self.text_cache.remove(&key) {
            self.text_cache_bytes = self.text_cache_bytes.saturating_sub(prev.byte_len());
        }
        while self.text_cache.len() >= MAX_TEXT_CACHE_ENTRIES
            || self.text_cache_bytes + entry_bytes > MAX_TEXT_CACHE_BYTES
        {
            let Some(oldest_k) = self
                .text_cache
                .iter()
                .min_by_key(|(_, v)| v.last_used_tick)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(removed) = self.text_cache.remove(&oldest_k) {
                self.text_cache_bytes = self.text_cache_bytes.saturating_sub(removed.byte_len());
            }
        }
        self.text_cache_bytes += entry_bytes;
        self.text_cache.insert(key, entry);
    }

    pub fn loaded_font_count(&self) -> usize {
        self.font_system.db().len()
    }

    /// OS にインストールされている利用可能なフォントファミリー名一覧をソート済み・重複排除で返す（フォント視覚化ピッカー用）
    pub fn list_system_fonts(&self) -> Vec<String> {
        let mut set = BTreeSet::new();
        for face in self.font_system.db().faces() {
            for (family_name, _) in &face.families {
                if !family_name.trim().is_empty() {
                    set.insert(family_name.clone());
                }
            }
        }
        set.into_iter().collect()
    }

    /// フォント視覚化ピッカー向けに指定フォントのサンプルプレビューを sRGB RGBA8 で描画する
    pub fn render_font_sample_rgba8(
        &mut self,
        font_family: &str,
        sample: &str,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let font_size = (height as f32 * 0.58).clamp(12.0, 32.0);
        let img = self.render_text(
            sample,
            font_family,
            font_size,
            RgbaColor::WHITE,
            width,
            height,
        );
        img.to_rgba8_srgb()
    }

    /// テキストを Linear sRGB + Premultiplied Alpha 画像へラスタライズする
    pub fn render_text(
        &mut self,
        text: &str,
        font_family: &str,
        font_size: f32,
        color: RgbaColor,
        max_width: u32,
        max_height: u32,
    ) -> LinearImage {
        self.render_text_styled(
            text,
            font_family,
            font_size,
            color,
            &TextAnimator::default(),
            &TextStyle::default(),
            &ColorPalette::default(),
            0.0,
            max_width,
            max_height,
        )
    }

    /// 文字ごとアニメーター対応テキスト描画（後方互換ラッパー）
    #[allow(clippy::too_many_arguments)]
    pub fn render_text_animated(
        &mut self,
        text: &str,
        font_family: &str,
        font_size: f32,
        color: RgbaColor,
        animator: &TextAnimator,
        local_time_sec: f32,
        max_width: u32,
        max_height: u32,
    ) -> LinearImage {
        self.render_text_styled(
            text,
            font_family,
            font_size,
            color,
            animator,
            &TextStyle::default(),
            &ColorPalette::default(),
            local_time_sec,
            max_width,
            max_height,
        )
    }

    /// インライン制御文字・縁取り/シャドウ/座布団・文字ごとアニメーターすべてに対応したフルテキスト描画
    #[allow(clippy::too_many_arguments)]
    pub fn render_text_styled(
        &mut self,
        text: &str,
        font_family: &str,
        font_size: f32,
        color: RgbaColor,
        animator: &TextAnimator,
        style: &TextStyle,
        palette: &ColorPalette,
        local_time_sec: f32,
        max_width: u32,
        max_height: u32,
    ) -> LinearImage {
        if text.is_empty() || max_width == 0 || max_height == 0 {
            return LinearImage::new(max_width, max_height, LinearPremul::TRANSPARENT);
        }

        // 静的テキストまたは文字ごとアニメーション完了後の定常状態は省メモリ BBox LRU キャッシュで O(1) 再利用
        let spans = parse_inline_text_spans(text, font_family, font_size, color, palette);
        let cache_key = if !animator.enabled {
            Some(compute_text_cache_key(
                text,
                font_family,
                font_size,
                color,
                style,
                palette,
                false,
                max_width,
                max_height,
            ))
        } else {
            let char_count: usize = spans
                .iter()
                .map(|s| s.text.chars().filter(|&c| c != '\n').count())
                .sum();
            let settled_sec = char_count.saturating_sub(1) as f32
                * animator.delay_per_char_sec.max(0.0)
                + animator.duration_per_char_sec.max(0.01);
            if local_time_sec >= settled_sec {
                Some(compute_text_cache_key(
                    text,
                    font_family,
                    font_size,
                    color,
                    style,
                    palette,
                    true,
                    max_width,
                    max_height,
                ))
            } else {
                None
            }
        };

        if let Some(key) = cache_key
            && let Some(cached) = self.text_cache.get_mut(&key)
        {
            self.text_cache_tick = self.text_cache_tick.wrapping_add(1);
            cached.last_used_tick = self.text_cache_tick;
            self.text_cache_hits += 1;
            return cached.expand_to_image(max_width, max_height);
        }
        if cache_key.is_some() {
            self.text_cache_misses += 1;
        }

        let mut glyphs = LinearImage::new(max_width, max_height, LinearPremul::TRANSPARENT);
        let pad_x = if style.box_enabled {
            style.box_padding[0].max(0.0).round() as i32
        } else {
            style.outline_width.ceil() as i32
        };
        let pad_y = if style.box_enabled {
            style.box_padding[1].max(0.0).round() as i32
        } else {
            style.outline_width.ceil() as i32
        };

        let has_rich_spans = spans.len() > 1
            || spans.first().is_some_and(|s| {
                s.bold
                    || s.offset_x.abs() > 1e-3
                    || s.offset_y.abs() > 1e-3
                    || (s.font_size - font_size).abs() > 1e-3
                    || s.color != color
                    || s.font_family != font_family
            });

        if !animator.enabled && !has_rich_spans {
            let plain = spans.first().map(|s| s.text.as_str()).unwrap_or(text);
            self.rasterize_into(
                &mut glyphs,
                plain,
                font_family,
                font_size,
                color,
                false,
                pad_x,
                pad_y,
                max_width.saturating_sub(pad_x.max(0) as u32),
                max_height.saturating_sub(pad_y.max(0) as u32),
            );
        } else if !animator.enabled {
            // インライン制御文字あり（静的描画）: 行・スパンごとに正確に配置
            let mut cursor_x = pad_x as f32;
            let mut cursor_y = pad_y as f32;
            let mut line_max_h = (font_size * 1.35).max(10.0);

            for span in &spans {
                let mut parts = span.text.split('\n').peekable();
                while let Some(seg) = parts.next() {
                    if !seg.is_empty() {
                        let span_h = (span.font_size * 1.35).max(10.0);
                        line_max_h = line_max_h.max(span_h);
                        let baseline_adj = (font_size - span.font_size) * 0.7;
                        let draw_x = (cursor_x + span.offset_x).round() as i32;
                        let draw_y = (cursor_y + baseline_adj + span.offset_y).round() as i32;
                        let avail_w = max_width.saturating_sub(draw_x.max(0) as u32).max(1);
                        let avail_h = max_height.saturating_sub(draw_y.max(0) as u32).max(1);
                        let adv = self.rasterize_into(
                            &mut glyphs,
                            seg,
                            &span.font_family,
                            span.font_size,
                            span.color,
                            span.bold,
                            draw_x,
                            draw_y,
                            avail_w,
                            avail_h,
                        );
                        cursor_x += adv;
                    }
                    if parts.peek().is_some() {
                        cursor_x = pad_x as f32;
                        cursor_y += line_max_h;
                        line_max_h = (font_size * 1.35).max(10.0);
                    }
                }
            }
        } else {
            // 文字ごとアニメーター＋インライン制御文字の両立
            let mut char_index: usize = 0;
            let mut cursor_x = pad_x as f32;
            let mut cursor_y = pad_y as f32;
            let mut line_max_h = (font_size * 1.35).max(10.0);

            for span in &spans {
                for ch in span.text.chars() {
                    if ch == '\n' {
                        cursor_x = pad_x as f32;
                        cursor_y += line_max_h;
                        line_max_h = (font_size * 1.35).max(10.0);
                        continue;
                    }
                    line_max_h = line_max_h.max((span.font_size * 1.35).max(10.0));
                    let start_t = char_index as f32 * animator.delay_per_char_sec.max(0.0);
                    let dur = animator.duration_per_char_sec.max(0.01);
                    let raw_p = ((local_time_sec - start_t) / dur).clamp(0.0, 1.0);
                    let p = animator.easing.evaluate(raw_p);

                    let opacity = (animator.start_opacity + (1.0 - animator.start_opacity) * p)
                        .clamp(0.0, 1.0);
                    let scale = (animator.start_scale + (1.0 - animator.start_scale) * p).max(0.1);
                    let ox = animator.start_offset_x * (1.0 - p) + span.offset_x;
                    let oy = animator.start_offset_y * (1.0 - p) + span.offset_y;

                    let advance = if ch.is_ascii() {
                        span.font_size * 0.62
                    } else {
                        span.font_size * 1.05
                    };

                    if opacity > 0.005 && !ch.is_whitespace() {
                        let mut ch_color = span.color;
                        ch_color.a *= opacity;
                        let glyph_size = (span.font_size * scale).max(6.0);
                        let cell = (glyph_size * 2.0).ceil() as u32;
                        let mut ch_buf = [0u8; 4];
                        let ch_str = ch.encode_utf8(&mut ch_buf);
                        self.rasterize_into(
                            &mut glyphs,
                            ch_str,
                            &span.font_family,
                            glyph_size,
                            ch_color,
                            span.bold,
                            (cursor_x + ox).round() as i32,
                            (cursor_y + oy).round() as i32,
                            cell.min(max_width),
                            cell.min(max_height),
                        );
                    }

                    cursor_x += advance;
                    char_index += 1;
                }
            }
        }

        // 縁取り・ドロップシャドウ・座布団背景の合成
        let needs_style = style.box_enabled
            || style.outline_width > 0.1
            || style.shadow_blur > 0.1
            || style.shadow_offset[0].abs() > 0.1
            || style.shadow_offset[1].abs() > 0.1;
        let result = if !needs_style {
            glyphs
        } else {
            apply_text_style(&glyphs, style)
        };
        if let Some(key) = cache_key {
            self.insert_text_cache(key, &result);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn rasterize_into(
        &mut self,
        image: &mut LinearImage,
        text: &str,
        font_family: &str,
        font_size: f32,
        color: RgbaColor,
        bold: bool,
        offset_x: i32,
        offset_y: i32,
        box_width: u32,
        box_height: u32,
    ) -> f32 {
        let metrics = Metrics::new(font_size.max(8.0), (font_size * 1.35).max(10.0));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(box_width as f32), Some(box_height as f32));

        let family = if font_family.is_empty() || font_family.eq_ignore_ascii_case("sans-serif") {
            Family::SansSerif
        } else {
            Family::Name(font_family)
        };
        let mut attrs = Attrs::new().family(family);
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut measured_w = 0.0f32;
        for run in buffer.layout_runs() {
            measured_w = measured_w.max(run.line_w);
        }

        let text_color = Color::rgba(
            (color.r.clamp(0.0, 1.0) * 255.0) as u8,
            (color.g.clamp(0.0, 1.0) * 255.0) as u8,
            (color.b.clamp(0.0, 1.0) * 255.0) as u8,
            (color.a.clamp(0.0, 1.0) * 255.0) as u8,
        );

        let max_w = image.width as i32;
        let max_h = image.height as i32;
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |x, y, w, h, glyph_color| {
                if glyph_color.a() == 0 {
                    return;
                }
                let src = LinearPremul::from_rgba8_srgb(
                    glyph_color.r(),
                    glyph_color.g(),
                    glyph_color.b(),
                    glyph_color.a(),
                );
                for dy in 0..h as i32 {
                    let py = y + dy + offset_y;
                    if py < 0 || py >= max_h {
                        continue;
                    }
                    for dx in 0..w as i32 {
                        let px = x + dx + offset_x;
                        if px < 0 || px >= max_w {
                            continue;
                        }
                        let idx = (py as u32 * image.width + px as u32) as usize;
                        image.pixels[idx] = src.over(image.pixels[idx]);
                    }
                }
            },
        );

        if measured_w > 0.5 {
            measured_w
        } else {
            text.chars()
                .map(|c| {
                    if c.is_ascii() {
                        font_size * 0.58
                    } else {
                        font_size * 1.0
                    }
                })
                .sum()
        }
    }
}

/// 縁取り（Linear Premultiplied 空間膨張）・ドロップシャドウ・座布団背景を合成する
fn apply_text_style(glyphs: &LinearImage, style: &TextStyle) -> LinearImage {
    let w = glyphs.width;
    let h = glyphs.height;
    let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let Some((min_x, min_y, max_x, max_y)) = alpha_bounds(glyphs) else {
        return out;
    };

    // 1. 座布団背景（文字のバウンディングボックスをパディング付き角丸矩形で描画）
    if style.box_enabled {
        let px = style.box_padding[0].max(0.0);
        let py = style.box_padding[1].max(0.0);
        let x0 = (min_x as f32 - px).max(0.0);
        let y0 = (min_y as f32 - py).max(0.0);
        let x1 = (max_x as f32 + 1.0 + px).min(w as f32);
        let y1 = (max_y as f32 + 1.0 + py).min(h as f32);
        let r = style
            .box_radius
            .max(0.0)
            .min((x1 - x0) * 0.5)
            .min((y1 - y0) * 0.5);
        let fill = LinearPremul::from_srgb_straight(style.box_color);
        let ix0 = x0.floor() as u32;
        let iy0 = y0.floor() as u32;
        let ix1 = (x1.ceil() as u32).min(w);
        let iy1 = (y1.ceil() as u32).min(h);
        for y in iy0..iy1 {
            let fy = y as f32 + 0.5;
            for x in ix0..ix1 {
                let fx = x as f32 + 0.5;
                let cov = rounded_rect_coverage(fx, fy, x0, y0, x1, y1, r);
                if cov > 0.0 {
                    let p = LinearPremul {
                        r: fill.r * cov,
                        g: fill.g * cov,
                        b: fill.b * cov,
                        a: fill.a * cov,
                    };
                    let idx = (y * w + x) as usize;
                    out.pixels[idx] = p.over(out.pixels[idx]);
                }
            }
        }
    }

    // 2. ドロップシャドウ
    if style.shadow_blur > 0.1
        || style.shadow_offset[0].abs() > 0.1
        || style.shadow_offset[1].abs() > 0.1
    {
        let ox = style.shadow_offset[0].round() as i32;
        let oy = style.shadow_offset[1].round() as i32;
        let blur_r = style.shadow_blur.round().clamp(0.0, 12.0) as i32;
        let mut mask = vec![0.0f32; (w * h) as usize];
        let src_y0 = min_y as i32;
        let src_y1 = max_y as i32;
        let src_x0 = min_x as i32;
        let src_x1 = max_x as i32;
        for sy in src_y0..=src_y1 {
            let y = sy + oy;
            if y < 0 || y >= h as i32 {
                continue;
            }
            for sx in src_x0..=src_x1 {
                let x = sx + ox;
                if x < 0 || x >= w as i32 {
                    continue;
                }
                mask[(y as u32 * w + x as u32) as usize] =
                    glyphs.pixels[(sy as u32 * w + sx as u32) as usize].a;
            }
        }
        if blur_r > 0 {
            mask = box_blur_alpha(&mask, w, h, blur_r);
        }
        let sc = LinearPremul::from_srgb_straight(style.shadow_color);
        for (dst, &m) in out.pixels.iter_mut().zip(mask.iter()) {
            if m > 1e-4 {
                let p = LinearPremul {
                    r: sc.r * m,
                    g: sc.g * m,
                    b: sc.b * m,
                    a: sc.a * m,
                };
                *dst = p.over(*dst);
            }
        }
    }

    // 3. 縁取り（円形カーネル膨張・グリフ外接矩形 + rad のみに限定して透過領域を完全スキップ）
    if style.outline_width > 0.1 {
        let rad = style.outline_width.ceil().clamp(1.0, 16.0) as i32;
        let rad_sq = style.outline_width * style.outline_width;
        let mut offsets = Vec::new();
        for dy in -rad..=rad {
            for dx in -rad..=rad {
                if (dx * dx + dy * dy) as f32 <= rad_sq + 0.5 {
                    offsets.push((dx, dy));
                }
            }
        }
        let oc = LinearPremul::from_srgb_straight(style.outline_color);
        let oy0 = (min_y as i32 - rad).max(0);
        let oy1 = (max_y as i32 + rad).min(h as i32 - 1);
        let ox0 = (min_x as i32 - rad).max(0);
        let ox1 = (max_x as i32 + rad).min(w as i32 - 1);
        for y in oy0..=oy1 {
            for x in ox0..=ox1 {
                let mut max_a = 0.0f32;
                for &(dx, dy) in &offsets {
                    let sx = x + dx;
                    let sy = y + dy;
                    if sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32 {
                        let a = glyphs.pixels[(sy as u32 * w + sx as u32) as usize].a;
                        if a > max_a {
                            max_a = a;
                            if max_a >= 0.999 {
                                break;
                            }
                        }
                    }
                }
                if max_a > 1e-4 {
                    let p = LinearPremul {
                        r: oc.r * max_a,
                        g: oc.g * max_a,
                        b: oc.b * max_a,
                        a: oc.a * max_a,
                    };
                    let idx = (y as u32 * w + x as u32) as usize;
                    out.pixels[idx] = p.over(out.pixels[idx]);
                }
            }
        }
    }

    // 4. 本体グリフを最前面へ合成
    for (dst, &src) in out.pixels.iter_mut().zip(glyphs.pixels.iter()) {
        if src.a > 0.0 {
            *dst = src.over(*dst);
        }
    }

    out
}

fn alpha_bounds(img: &LinearImage) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..img.height {
        for x in 0..img.width {
            if img.pixels[(y * img.width + x) as usize].a > 0.01 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    found.then_some((min_x, min_y, max_x, max_y))
}

fn rounded_rect_coverage(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, radius: f32) -> f32 {
    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.5;
    let hx = ((x1 - x0) * 0.5 - radius).max(0.0);
    let hy = ((y1 - y0) * 0.5 - radius).max(0.0);
    let qx = (px - cx).abs() - hx;
    let qy = (py - cy).abs() - hy;
    let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
    (0.5 - dist).clamp(0.0, 1.0)
}

fn box_blur_alpha(mask: &[f32], w: u32, h: u32, radius: i32) -> Vec<f32> {
    let wu = w as usize;
    let hu = h as usize;
    let mut temp = vec![0.0f32; mask.len()];
    let mut out = vec![0.0f32; mask.len()];
    let inv = 1.0 / (radius * 2 + 1) as f32;
    for y in 0..hu {
        for x in 0..w as i32 {
            let mut sum = 0.0f32;
            for dx in -radius..=radius {
                let sx = (x + dx).clamp(0, w as i32 - 1) as usize;
                sum += mask[y * wu + sx];
            }
            temp[y * wu + x as usize] = sum * inv;
        }
    }
    for y in 0..h as i32 {
        for x in 0..wu {
            let mut sum = 0.0f32;
            for dy in -radius..=radius {
                let sy = (y + dy).clamp(0, h as i32 - 1) as usize;
                sum += temp[sy * wu + x];
            }
            out[y as usize * wu + x] = sum * inv;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aviutl2_inline_control_tags_and_palette_refs() {
        let palette = ColorPalette::default_presets();
        let spans = parse_inline_text_spans(
            "通常<#ff0000>赤<s72>大</><@解説シアン>シアン\\<生>",
            "sans-serif",
            36.0,
            RgbaColor::WHITE,
            &palette,
        );
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].text, "通常");
        assert_eq!(spans[1].text, "赤");
        assert!((spans[1].color.r - 1.0).abs() < 1e-3 && spans[1].color.g < 1e-3);
        assert_eq!(spans[2].text, "大");
        assert!((spans[2].font_size - 72.0).abs() < 1e-3);
        assert_eq!(spans[3].text, "シアン<生>");
        assert_eq!(spans[3].color, palette.find_by_name("解説シアン").unwrap());
    }

    #[test]
    fn renders_text_to_linear_premultiplied_buffer() {
        let mut renderer = TextRenderer::new();
        let img = renderer.render_text(
            "Hello 次世代動画編集",
            "sans-serif",
            32.0,
            RgbaColor::WHITE,
            400,
            100,
        );
        assert_eq!(img.width, 400);
        assert_eq!(img.height, 100);
        if renderer.loaded_font_count() > 0 {
            assert!(img.pixels.iter().any(|p| p.a > 0.0));
        }
    }

    #[test]
    fn styled_text_with_box_and_outline_renders_larger_coverage() {
        let mut renderer = TextRenderer::new();
        if renderer.loaded_font_count() == 0 {
            return;
        }
        let plain = renderer.render_text("Test", "sans-serif", 32.0, RgbaColor::WHITE, 200, 90);
        let style = TextStyle {
            outline_width: 3.0,
            box_enabled: true,
            ..Default::default()
        };
        let styled = renderer.render_text_styled(
            "Test",
            "sans-serif",
            32.0,
            RgbaColor::WHITE,
            &TextAnimator::default(),
            &style,
            &ColorPalette::default(),
            0.0,
            200,
            90,
        );
        let plain_cov: f32 = plain.pixels.iter().map(|p| p.a).sum();
        let styled_cov: f32 = styled.pixels.iter().map(|p| p.a).sum();
        assert!(styled_cov > plain_cov * 1.5);
    }

    #[test]
    fn per_char_text_animator_staggers_appearance() {
        let mut renderer = TextRenderer::new();
        let animator = TextAnimator {
            enabled: true,
            start_opacity: 0.0,
            delay_per_char_sec: 0.5,
            duration_per_char_sec: 0.2,
            ..Default::default()
        };
        let t0 = renderer.render_text_animated(
            "ABC",
            "sans-serif",
            32.0,
            RgbaColor::WHITE,
            &animator,
            0.0,
            300,
            100,
        );
        let t_late = renderer.render_text_animated(
            "ABC",
            "sans-serif",
            32.0,
            RgbaColor::WHITE,
            &animator,
            2.0,
            300,
            100,
        );
        let alpha_sum_0: f32 = t0.pixels.iter().map(|p| p.a).sum();
        let alpha_sum_late: f32 = t_late.pixels.iter().map(|p| p.a).sum();
        assert!(alpha_sum_0 < 1e-3);
        if renderer.loaded_font_count() > 0 {
            assert!(alpha_sum_late > 1.0);
        }
    }
}
