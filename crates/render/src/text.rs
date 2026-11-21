use crate::color::{LinearImage, LinearPremul, srgb_to_linear};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use ve_core::RgbaColor;

/// `cosmic-text` + `swash` による動画内テキスト描画エンジン（日本語・マルチライン対応）
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
        let mut image = LinearImage::new(max_width, max_height, LinearPremul::TRANSPARENT);
        if text.is_empty() || max_width == 0 || max_height == 0 {
            return image;
        }

        let metrics = Metrics::new(font_size.max(8.0), (font_size * 1.35).max(10.0));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(max_width as f32), Some(max_height as f32));

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
                    let py = y + dy;
                    if py < 0 || py >= max_height as i32 {
                        continue;
                    }
                    for dx in 0..w as i32 {
                        let px = x + dx;
                        if px < 0 || px >= max_width as i32 {
                            continue;
                        }
                        let idx = (py as u32 * max_width + px as u32) as usize;
                        image.pixels[idx] = src.over(image.pixels[idx]);
                    }
                }
            },
        );

        image
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
        // システムにフォントが1つ以上あればグリフピクセルが描画される
        if renderer.loaded_font_count() > 0 {
            assert!(img.pixels.iter().any(|p| p.a > 0.0));
        }
    }
}
