use crate::color::{LinearImage, LinearPremul, srgb_to_linear};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use ve_core::{RgbaColor, TextAnimator};

/// `cosmic-text` + `swash` による動画内テキスト描画エンジン（日本語・マルチライン・文字ごとアニメーター対応）
pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
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
        }
    }

    pub fn loaded_font_count(&self) -> usize {
        self.font_system.db().len()
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
        self.render_text_animated(
            text,
            font_family,
            font_size,
            color,
            &TextAnimator::default(),
            0.0,
            max_width,
            max_height,
        )
    }

    /// 文字ごとアニメーター対応テキスト描画（1文字単位の位置・スケール・不透明度・出現ディレイ制御）
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
        let mut image = LinearImage::new(max_width, max_height, LinearPremul::TRANSPARENT);
        if text.is_empty() || max_width == 0 || max_height == 0 {
            return image;
        }

        if !animator.enabled {
            self.rasterize_into(
                &mut image,
                text,
                font_family,
                font_size,
                color,
                0,
                0,
                max_width,
                max_height,
            );
            return image;
        }

        // 文字ごとにディレイ付きイージングを適用して配置
        let line_height = (font_size * 1.35).max(10.0);
        let mut char_index: usize = 0;
        let mut cursor_y = 0.0f32;

        for line in text.split('\n') {
            let mut cursor_x = 0.0f32;
            for ch in line.chars() {
                let start_t = char_index as f32 * animator.delay_per_char_sec.max(0.0);
                let dur = animator.duration_per_char_sec.max(0.01);
                let raw_p = ((local_time_sec - start_t) / dur).clamp(0.0, 1.0);
                let p = animator.easing.evaluate(raw_p);

                let opacity =
                    (animator.start_opacity + (1.0 - animator.start_opacity) * p).clamp(0.0, 1.0);
                let scale = (animator.start_scale + (1.0 - animator.start_scale) * p).max(0.1);
                let ox = animator.start_offset_x * (1.0 - p);
                let oy = animator.start_offset_y * (1.0 - p);

                let advance = if ch.is_ascii() {
                    font_size * 0.62
                } else {
                    font_size * 1.05
                };

                if opacity > 0.005 && !ch.is_whitespace() {
                    let mut ch_color = color;
                    ch_color.a *= opacity;
                    let glyph_size = (font_size * scale).max(6.0);
                    let cell = (glyph_size * 2.0).ceil() as u32;
                    let ch_str = ch.to_string();
                    self.rasterize_into(
                        &mut image,
                        &ch_str,
                        font_family,
                        glyph_size,
                        ch_color,
                        (cursor_x + ox).round() as i32,
                        (cursor_y + oy).round() as i32,
                        cell.min(max_width),
                        cell.min(max_height),
                    );
                }

                cursor_x += advance;
                char_index += 1;
            }
            cursor_y += line_height;
        }

        image
    }

    #[allow(clippy::too_many_arguments)]
    fn rasterize_into(
        &mut self,
        image: &mut LinearImage,
        text: &str,
        font_family: &str,
        font_size: f32,
        color: RgbaColor,
        offset_x: i32,
        offset_y: i32,
        box_width: u32,
        box_height: u32,
    ) {
        let metrics = Metrics::new(font_size.max(8.0), (font_size * 1.35).max(10.0));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(box_width as f32), Some(box_height as f32));

        let family = if font_family.is_empty() || font_family.eq_ignore_ascii_case("sans-serif") {
            Family::SansSerif
        } else {
            Family::Name(font_family)
        };
        let attrs = Attrs::new().family(family);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

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
                let a = f32::from(glyph_color.a()) / 255.0;
                let src = LinearPremul {
                    r: srgb_to_linear(f32::from(glyph_color.r()) / 255.0) * a,
                    g: srgb_to_linear(f32::from(glyph_color.g()) / 255.0) * a,
                    b: srgb_to_linear(f32::from(glyph_color.b()) / 255.0) * a,
                    a,
                };
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(alpha_sum_late > alpha_sum_0);
        }
    }
}
