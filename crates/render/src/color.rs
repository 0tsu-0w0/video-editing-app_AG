use half::f16;
use ve_core::RgbaColor;

#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear sRGB + Premultiplied Alpha ピクセル
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LinearPremul {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl LinearPremul {
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub fn from_srgb_straight(color: RgbaColor) -> Self {
        let a = color.a.clamp(0.0, 1.0);
        Self {
            r: srgb_to_linear(color.r) * a,
            g: srgb_to_linear(color.g) * a,
            b: srgb_to_linear(color.b) * a,
            a,
        }
    }

    pub fn from_rgba8_srgb(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::from_srgb_straight(RgbaColor {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: f32::from(a) / 255.0,
        })
    }

    /// Porter-Duff SrcOver（Linear Premultiplied Alpha）
    #[inline]
    pub fn over(self, dst: Self) -> Self {
        let inv_a = 1.0 - self.a;
        Self {
            r: self.r + dst.r * inv_a,
            g: self.g + dst.g * inv_a,
            b: self.b + dst.b * inv_a,
            a: self.a + dst.a * inv_a,
        }
    }

    /// 出力用（sRGB Straight RGBA8）へ変換
    pub fn to_rgba8_srgb(self) -> [u8; 4] {
        let a = self.a.clamp(0.0, 1.0);
        let (r_lin, g_lin, b_lin) = if a > 1e-6 {
            (self.r / a, self.g / a, self.b / a)
        } else {
            (0.0, 0.0, 0.0)
        };
        [
            (linear_to_srgb(r_lin) * 255.0).round().clamp(0.0, 255.0) as u8,
            (linear_to_srgb(g_lin) * 255.0).round().clamp(0.0, 255.0) as u8,
            (linear_to_srgb(b_lin) * 255.0).round().clamp(0.0, 255.0) as u8,
            (a * 255.0).round().clamp(0.0, 255.0) as u8,
        ]
    }
}

/// Linear sRGB + Premultiplied Alpha バッファ
#[derive(Debug, Clone)]
pub struct LinearImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<LinearPremul>,
}

impl LinearImage {
    pub fn new(width: u32, height: u32, fill: LinearPremul) -> Self {
        Self {
            width,
            height,
            pixels: vec![fill; (width * height) as usize],
        }
    }

    pub fn from_rgba8_srgb(width: u32, height: u32, data: &[u8]) -> Self {
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for px in data.as_chunks::<4>().0 {
            pixels.push(LinearPremul::from_rgba8_srgb(px[0], px[1], px[2], px[3]));
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn to_rgba8_srgb(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for px in &self.pixels {
            out.extend_from_slice(&px.to_rgba8_srgb());
        }
        out
    }

    /// `wgpu::TextureFormat::Rgba16Float` 転送用バイト列へ変換
    pub fn to_rgba16f_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 8);
        for px in &self.pixels {
            for c in [px.r, px.g, px.b, px.a] {
                out.extend_from_slice(&f16::from_f32(c).to_le_bytes());
            }
        }
        out
    }

    pub fn blend_over(&mut self, src: &LinearImage, offset_x: i32, offset_y: i32, opacity: f32) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        for sy in 0..src.height {
            let dy = sy as i32 + offset_y;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            for sx in 0..src.width {
                let dx = sx as i32 + offset_x;
                if dx < 0 || dx >= self.width as i32 {
                    continue;
                }
                let mut sp = src.pixels[(sy * src.width + sx) as usize];
                sp.r *= opacity;
                sp.g *= opacity;
                sp.b *= opacity;
                sp.a *= opacity;

                let dst_idx = (dy as u32 * self.width + dx as u32) as usize;
                self.pixels[dst_idx] = sp.over(self.pixels[dst_idx]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_premultiplied_over_avoids_dark_fringing() {
        let white_half = LinearPremul::from_srgb_straight(RgbaColor {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.5,
        });
        let black = LinearPremul::from_srgb_straight(RgbaColor::BLACK);
        let blended = white_half.over(black);
        // Linear 空間の 0.5 は sRGB で約 0.735（188/255）になる
        let rgba8 = blended.to_rgba8_srgb();
        assert!((rgba8[0] as i32 - 188).abs() <= 2, "got {:?}", rgba8);
        assert_eq!(rgba8[3], 255);
    }
}
