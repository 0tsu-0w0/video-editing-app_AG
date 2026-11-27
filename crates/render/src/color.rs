use half::f16;
use std::sync::LazyLock;
use ve_core::{BlendMode, RgbaColor, TrackMatteMode};

const LINEAR_LUT_MAX: usize = 8192;

static SRGB_U8_TO_LINEAR_LUT: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut table = [0.0f32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = srgb_to_linear(i as f32 / 255.0);
    }
    table
});

static LINEAR_TO_SRGB_U8_LUT: LazyLock<[u8; LINEAR_LUT_MAX + 1]> = LazyLock::new(|| {
    let mut table = [0u8; LINEAR_LUT_MAX + 1];
    for (i, slot) in table.iter_mut().enumerate() {
        let lin = i as f32 / LINEAR_LUT_MAX as f32;
        *slot = (linear_to_srgb(lin) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    table
});

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

/// 8-bit sRGB 値を `LazyLock` LUT 経由で高速に Linear 値へ変換する
#[inline]
pub fn srgb_u8_to_linear(v: u8) -> f32 {
    SRGB_U8_TO_LINEAR_LUT[v as usize]
}

/// Linear 値 (0.0..=1.0) を `LazyLock` 8193段 LUT 経由で高速に 8-bit sRGB へ変換する
#[inline]
pub fn linear_to_srgb_u8(c: f32) -> u8 {
    let idx = (c.clamp(0.0, 1.0) * LINEAR_LUT_MAX as f32).round() as usize;
    LINEAR_TO_SRGB_U8_LUT[idx.min(LINEAR_LUT_MAX)]
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

    #[inline]
    pub fn from_rgba8_srgb(r: u8, g: u8, b: u8, a: u8) -> Self {
        if a == 0 {
            return Self::TRANSPARENT;
        }
        let af = f32::from(a) * (1.0 / 255.0);
        Self {
            r: srgb_u8_to_linear(r) * af,
            g: srgb_u8_to_linear(g) * af,
            b: srgb_u8_to_linear(b) * af,
            a: af,
        }
    }

    /// Rec.709 リニア輝度
    #[inline]
    pub fn luma(self) -> f32 {
        (0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b).clamp(0.0, 1.0)
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

    /// ブレンドモード付き合成（Linear Premultiplied Alpha）
    #[inline]
    pub fn blend(self, dst: Self, mode: BlendMode) -> Self {
        match mode {
            BlendMode::Normal => self.over(dst),
            BlendMode::Add => Self {
                r: self.r + dst.r,
                g: self.g + dst.g,
                b: self.b + dst.b,
                a: (self.a + dst.a * (1.0 - self.a)).clamp(0.0, 1.0),
            },
            BlendMode::Multiply => {
                let inv_sa = 1.0 - self.a;
                let inv_da = 1.0 - dst.a;
                Self {
                    r: self.r * dst.r + self.r * inv_da + dst.r * inv_sa,
                    g: self.g * dst.g + self.g * inv_da + dst.g * inv_sa,
                    b: self.b * dst.b + self.b * inv_da + dst.b * inv_sa,
                    a: (self.a + dst.a * inv_sa).clamp(0.0, 1.0),
                }
            }
            BlendMode::Screen => Self {
                r: self.r + dst.r - self.r * dst.r,
                g: self.g + dst.g - self.g * dst.g,
                b: self.b + dst.b - self.b * dst.b,
                a: (self.a + dst.a * (1.0 - self.a)).clamp(0.0, 1.0),
            },
        }
    }

    /// 出力用（sRGB Straight RGBA8）へ高速LUT変換
    #[inline]
    pub fn to_rgba8_srgb(self) -> [u8; 4] {
        let a = self.a.clamp(0.0, 1.0);
        if a <= 1e-6 {
            return [0, 0, 0, 0];
        }
        let inv_a = 1.0 / a;
        [
            linear_to_srgb_u8(self.r * inv_a),
            linear_to_srgb_u8(self.g * inv_a),
            linear_to_srgb_u8(self.b * inv_a),
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
            pixels: vec![fill; (width as usize).saturating_mul(height as usize)],
        }
    }

    pub fn from_rgba8_srgb(width: u32, height: u32, data: &[u8]) -> Self {
        let count = (width as usize).saturating_mul(height as usize);
        let mut pixels = Vec::with_capacity(count);
        for px in data.as_chunks::<4>().0.iter().take(count) {
            pixels.push(LinearPremul::from_rgba8_srgb(px[0], px[1], px[2], px[3]));
        }
        pixels.resize(count, LinearPremul::TRANSPARENT);
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn to_rgba8_srgb(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.pixels.len() * 4];
        for (dst, px) in out
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.pixels.iter())
        {
            *dst = px.to_rgba8_srgb();
        }
        out
    }

    /// 呼び出し側の再利用バッファへ `wgpu::TextureFormat::Rgba16Float` バイト列を書き込む（毎フレームゼロ確保）
    pub fn write_rgba16f_bytes(&self, out: &mut Vec<u8>) {
        let needed = self.pixels.len() * 8;
        out.resize(needed, 0);
        for (dst, px) in out
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip(self.pixels.iter())
        {
            let r = f16::from_f32(px.r).to_le_bytes();
            let g = f16::from_f32(px.g).to_le_bytes();
            let b = f16::from_f32(px.b).to_le_bytes();
            let a = f16::from_f32(px.a).to_le_bytes();
            *dst = [r[0], r[1], g[0], g[1], b[0], b[1], a[0], a[1]];
        }
    }

    /// `wgpu::TextureFormat::Rgba16Float` 転送・キャッシュ保存用バイト列へ変換
    pub fn to_rgba16f_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 8);
        self.write_rgba16f_bytes(&mut out);
        out
    }

    /// キャッシュされた `Rgba16Float` バイト列から復元する
    pub fn from_rgba16f_bytes(width: u32, height: u32, bytes: &[u8]) -> Option<Self> {
        let count = (width as usize).checked_mul(height as usize)?;
        let expected = count.checked_mul(8)?;
        if bytes.len() != expected {
            return None;
        }
        let mut pixels = Vec::with_capacity(count);
        for chunk in bytes.as_chunks::<8>().0 {
            let r = f16::from_le_bytes([chunk[0], chunk[1]]).to_f32();
            let g = f16::from_le_bytes([chunk[2], chunk[3]]).to_f32();
            let b = f16::from_le_bytes([chunk[4], chunk[5]]).to_f32();
            let a = f16::from_le_bytes([chunk[6], chunk[7]]).to_f32();
            pixels.push(LinearPremul { r, g, b, a });
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn blend_over(&mut self, src: &LinearImage, offset_x: i32, offset_y: i32, opacity: f32) {
        self.blend_with_mode(src, offset_x, offset_y, opacity, BlendMode::Normal);
    }

    pub fn blend_with_mode(
        &mut self,
        src: &LinearImage,
        offset_x: i32,
        offset_y: i32,
        opacity: f32,
        mode: BlendMode,
    ) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.0
            || src.width == 0
            || src.height == 0
            || self.width == 0
            || self.height == 0
        {
            return;
        }

        // ループ外で描画交差領域を事前クリッピングし、内部ループの境界分岐を排除
        let dst_x0 = offset_x.max(0) as i64;
        let dst_y0 = offset_y.max(0) as i64;
        let dst_x1 = (offset_x as i64 + src.width as i64).min(self.width as i64);
        let dst_y1 = (offset_y as i64 + src.height as i64).min(self.height as i64);
        if dst_x0 >= dst_x1 || dst_y0 >= dst_y1 {
            return;
        }

        let copy_w = (dst_x1 - dst_x0) as usize;
        let copy_h = (dst_y1 - dst_y0) as usize;
        let src_x0 = (dst_x0 - offset_x as i64) as usize;
        let src_y0 = (dst_y0 - offset_y as i64) as usize;
        let dst_x0_u = dst_x0 as usize;
        let dst_y0_u = dst_y0 as usize;
        let src_stride = src.width as usize;
        let dst_stride = self.width as usize;
        let full_opacity = (opacity - 1.0).abs() <= 1e-6;

        // モード・不透明度分岐をループ外へホイストし、最頻出の Normal 合成をタイト化
        match (mode, full_opacity) {
            (BlendMode::Normal, true) => {
                for row in 0..copy_h {
                    let src_row_start = (src_y0 + row) * src_stride + src_x0;
                    let dst_row_start = (dst_y0_u + row) * dst_stride + dst_x0_u;
                    let src_slice = &src.pixels[src_row_start..src_row_start + copy_w];
                    let dst_slice = &mut self.pixels[dst_row_start..dst_row_start + copy_w];
                    for (dst_px, &sp) in dst_slice.iter_mut().zip(src_slice.iter()) {
                        if sp.a <= 1e-6 && sp.r == 0.0 && sp.g == 0.0 && sp.b == 0.0 {
                            continue;
                        }
                        if sp.a >= 1.0 - 1e-6 {
                            *dst_px = sp;
                        } else {
                            *dst_px = sp.over(*dst_px);
                        }
                    }
                }
            }
            (BlendMode::Normal, false) => {
                for row in 0..copy_h {
                    let src_row_start = (src_y0 + row) * src_stride + src_x0;
                    let dst_row_start = (dst_y0_u + row) * dst_stride + dst_x0_u;
                    let src_slice = &src.pixels[src_row_start..src_row_start + copy_w];
                    let dst_slice = &mut self.pixels[dst_row_start..dst_row_start + copy_w];
                    for (dst_px, &raw_sp) in dst_slice.iter_mut().zip(src_slice.iter()) {
                        if raw_sp.a <= 1e-6 && raw_sp.r == 0.0 && raw_sp.g == 0.0 && raw_sp.b == 0.0
                        {
                            continue;
                        }
                        let sp = LinearPremul {
                            r: raw_sp.r * opacity,
                            g: raw_sp.g * opacity,
                            b: raw_sp.b * opacity,
                            a: raw_sp.a * opacity,
                        };
                        *dst_px = sp.over(*dst_px);
                    }
                }
            }
            (_, true) => {
                for row in 0..copy_h {
                    let src_row_start = (src_y0 + row) * src_stride + src_x0;
                    let dst_row_start = (dst_y0_u + row) * dst_stride + dst_x0_u;
                    let src_slice = &src.pixels[src_row_start..src_row_start + copy_w];
                    let dst_slice = &mut self.pixels[dst_row_start..dst_row_start + copy_w];
                    for (dst_px, &sp) in dst_slice.iter_mut().zip(src_slice.iter()) {
                        if sp.a <= 1e-6 && sp.r == 0.0 && sp.g == 0.0 && sp.b == 0.0 {
                            continue;
                        }
                        *dst_px = sp.blend(*dst_px, mode);
                    }
                }
            }
            (_, false) => {
                for row in 0..copy_h {
                    let src_row_start = (src_y0 + row) * src_stride + src_x0;
                    let dst_row_start = (dst_y0_u + row) * dst_stride + dst_x0_u;
                    let src_slice = &src.pixels[src_row_start..src_row_start + copy_w];
                    let dst_slice = &mut self.pixels[dst_row_start..dst_row_start + copy_w];
                    for (dst_px, &raw_sp) in dst_slice.iter_mut().zip(src_slice.iter()) {
                        if raw_sp.a <= 1e-6 && raw_sp.r == 0.0 && raw_sp.g == 0.0 && raw_sp.b == 0.0
                        {
                            continue;
                        }
                        let sp = LinearPremul {
                            r: raw_sp.r * opacity,
                            g: raw_sp.g * opacity,
                            b: raw_sp.b * opacity,
                            a: raw_sp.a * opacity,
                        };
                        *dst_px = sp.blend(*dst_px, mode);
                    }
                }
            }
        }
    }

    /// トラックマット適用（アルファ／ルミナンスおよび反転・モード分岐をループ外へホイスト）
    pub fn apply_track_matte(&mut self, matte: &LinearImage, mode: TrackMatteMode) {
        let w = self.width.min(matte.width) as usize;
        let h = self.height.min(matte.height) as usize;
        let dst_stride = self.width as usize;
        let matte_stride = matte.width as usize;
        for y in 0..h {
            let dst_row = &mut self.pixels[y * dst_stride..y * dst_stride + w];
            let matte_row = &matte.pixels[y * matte_stride..y * matte_stride + w];
            match mode {
                TrackMatteMode::Alpha => {
                    for (dst, &mp) in dst_row.iter_mut().zip(matte_row.iter()) {
                        let f = mp.a.clamp(0.0, 1.0);
                        dst.r *= f;
                        dst.g *= f;
                        dst.b *= f;
                        dst.a *= f;
                    }
                }
                TrackMatteMode::AlphaInverted => {
                    for (dst, &mp) in dst_row.iter_mut().zip(matte_row.iter()) {
                        let f = (1.0 - mp.a).clamp(0.0, 1.0);
                        dst.r *= f;
                        dst.g *= f;
                        dst.b *= f;
                        dst.a *= f;
                    }
                }
                TrackMatteMode::Luma => {
                    for (dst, &mp) in dst_row.iter_mut().zip(matte_row.iter()) {
                        let f = mp.luma();
                        dst.r *= f;
                        dst.g *= f;
                        dst.b *= f;
                        dst.a *= f;
                    }
                }
                TrackMatteMode::LumaInverted => {
                    for (dst, &mp) in dst_row.iter_mut().zip(matte_row.iter()) {
                        let f = (1.0 - mp.luma()).clamp(0.0, 1.0);
                        dst.r *= f;
                        dst.g *= f;
                        dst.b *= f;
                        dst.a *= f;
                    }
                }
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

    #[test]
    fn srgb_lut_roundtrip_is_exact_for_all_u8_values() {
        for v in 0u8..=255 {
            let lin = srgb_u8_to_linear(v);
            let exact = srgb_to_linear(f32::from(v) / 255.0);
            assert!((lin - exact).abs() < 1e-7);
            let back = linear_to_srgb_u8(lin);
            assert_eq!(back, v, "roundtrip mismatch at sRGB={v}");
        }
    }

    #[test]
    fn rgba16f_cache_roundtrip() {
        let img = LinearImage::new(
            4,
            4,
            LinearPremul {
                r: 0.25,
                g: 0.5,
                b: 0.75,
                a: 1.0,
            },
        );
        let bytes = img.to_rgba16f_bytes();
        let restored = LinearImage::from_rgba16f_bytes(4, 4, &bytes).unwrap();
        assert!((restored.pixels[0].r - 0.25).abs() < 1e-3);
        assert!((restored.pixels[0].g - 0.5).abs() < 1e-3);
    }
}
