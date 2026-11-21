use crate::color::{LinearImage, LinearPremul};
use std::fs::File;
use std::path::Path;
use ve_core::{MouthShape, RgbaColor};

/// PNG の最大許容解像度（8K）・メモリ上限（64 MiB）による展開爆弾対策
const MAX_PNG_DIMENSION: u32 = 8192;
const MAX_PNG_DECODE_BYTES: usize = 64 * 1024 * 1024;

/// PNG ファイルを読み込み Linear sRGB + Premultiplied Alpha バッファへ変換する
pub fn load_png_linear(path: &Path) -> Option<LinearImage> {
    let file = File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_limits(png::Limits {
        bytes: MAX_PNG_DECODE_BYTES,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let out_size = reader.output_buffer_size()?;
    if out_size > MAX_PNG_DECODE_BYTES {
        return None;
    }
    let mut buf = vec![0u8; out_size];
    let info = reader.next_frame(&mut buf).ok()?;
    let bytes = &buf[..info.buffer_size()];
    let w = info.width;
    let h = info.height;
    if w == 0 || h == 0 || w > MAX_PNG_DIMENSION || h > MAX_PNG_DIMENSION {
        return None;
    }

    if info.color_type == png::ColorType::Rgba {
        return Some(LinearImage::from_rgba8_srgb(w, h, bytes));
    }

    let cap = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    let mut rgba = Vec::with_capacity(cap);
    match info.color_type {
        png::ColorType::Rgb => {
            for px in bytes.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for px in bytes.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
        }
        png::ColorType::Grayscale => {
            for &v in bytes {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        _ => return None,
    }
    Some(LinearImage::from_rgba8_srgb(w, h, &rgba))
}

/// ニアレスト／バイリニア簡易リサイズ（PNG立ち絵・3D透視スケール兼用）
pub fn scale_linear_image(src: &LinearImage, dst_w: u32, dst_h: u32) -> LinearImage {
    let dst_w = dst_w.max(1);
    let dst_h = dst_h.max(1);
    if src.width == dst_w && src.height == dst_h {
        return src.clone();
    }
    let mut dst = LinearImage::new(dst_w, dst_h, LinearPremul::TRANSPARENT);
    let sx_ratio = src.width as f32 / dst_w as f32;
    let sy_ratio = src.height as f32 / dst_h as f32;
    let max_sx = (src.width.saturating_sub(1)) as f32;
    let max_sy = (src.height.saturating_sub(1)) as f32;

    // X方向の参照インデックスを事前計算し、行ループ内での浮動小数点丸めを排除
    let sx_map: Vec<usize> = (0..dst_w)
        .map(|x| {
            ((x as f32 + 0.5) * sx_ratio - 0.5)
                .round()
                .clamp(0.0, max_sx) as usize
        })
        .collect();
    let src_stride = src.width as usize;
    let dst_stride = dst_w as usize;

    for y in 0..dst_h {
        let sy = ((y as f32 + 0.5) * sy_ratio - 0.5)
            .round()
            .clamp(0.0, max_sy) as usize;
        let src_row = &src.pixels[sy * src_stride..(sy + 1) * src_stride];
        let dst_row = &mut dst.pixels[(y as usize) * dst_stride..(y as usize + 1) * dst_stride];
        for (dst_px, &sx) in dst_row.iter_mut().zip(sx_map.iter()) {
            *dst_px = src_row[sx];
        }
    }
    dst
}

/// PNG 未設定時でも口パク連動を即確認できるプロシージャル立ち絵アバター
pub fn render_placeholder_character(width: u32, height: u32, mouth: MouthShape) -> LinearImage {
    let w = width.max(16);
    let h = height.max(16);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);

    let skin = LinearPremul::from_srgb_straight(RgbaColor {
        r: 0.98,
        g: 0.87,
        b: 0.78,
        a: 1.0,
    });
    let hair = LinearPremul::from_srgb_straight(RgbaColor {
        r: 0.20,
        g: 0.28,
        b: 0.45,
        a: 1.0,
    });
    let eye = LinearPremul::from_srgb_straight(RgbaColor {
        r: 0.12,
        g: 0.16,
        b: 0.25,
        a: 1.0,
    });
    let mouth_color = LinearPremul::from_srgb_straight(RgbaColor {
        r: 0.82,
        g: 0.26,
        b: 0.32,
        a: 1.0,
    });

    let cx = w as f32 * 0.5;
    let head_cy = h as f32 * 0.44;
    let rx = w as f32 * 0.34;
    let ry = h as f32 * 0.34;

    // 頭部・髪
    for y in 0..h {
        for x in 0..w {
            let nx = (x as f32 - cx) / rx;
            let ny = (y as f32 - head_cy) / ry;
            let d2 = nx * nx + ny * ny;
            if d2 <= 1.0 {
                img.pixels[(y * w + x) as usize] = if ny < -0.35 { hair } else { skin };
            } else if y as f32 > head_cy + ry * 0.85 && (x as f32 - cx).abs() < rx * 0.85 {
                // 肩シルエット
                img.pixels[(y * w + x) as usize] = hair;
            }
        }
    }

    // 両目
    let eye_y = head_cy - ry * 0.08;
    let eye_r = (rx * 0.12).max(2.0);
    for ex in [cx - rx * 0.36, cx + rx * 0.36] {
        fill_ellipse(&mut img, ex, eye_y, eye_r, eye_r * 1.15, eye);
    }

    // 口形状（Closed / HalfOpen / Open / A / I / U / E / O）
    let my = head_cy + ry * 0.38;
    match mouth {
        MouthShape::Closed => {
            fill_ellipse(
                &mut img,
                cx,
                my,
                rx * 0.22,
                (ry * 0.03).max(1.0),
                mouth_color,
            );
        }
        MouthShape::HalfOpen | MouthShape::E => {
            fill_ellipse(
                &mut img,
                cx,
                my,
                rx * 0.24,
                (ry * 0.10).max(2.0),
                mouth_color,
            );
        }
        MouthShape::Open | MouthShape::A => {
            fill_ellipse(
                &mut img,
                cx,
                my,
                rx * 0.26,
                (ry * 0.20).max(3.0),
                mouth_color,
            );
        }
        MouthShape::I => {
            fill_ellipse(
                &mut img,
                cx,
                my,
                rx * 0.32,
                (ry * 0.08).max(2.0),
                mouth_color,
            );
        }
        MouthShape::U => {
            fill_ellipse(
                &mut img,
                cx,
                my,
                rx * 0.12,
                (ry * 0.13).max(2.0),
                mouth_color,
            );
        }
        MouthShape::O => {
            fill_ellipse(
                &mut img,
                cx,
                my,
                rx * 0.18,
                (ry * 0.19).max(3.0),
                mouth_color,
            );
        }
    }

    img
}

fn fill_ellipse(img: &mut LinearImage, cx: f32, cy: f32, rx: f32, ry: f32, color: LinearPremul) {
    let x0 = (cx - rx).floor().max(0.0) as u32;
    let x1 = (cx + rx).ceil().min(img.width as f32) as u32;
    let y0 = (cy - ry).floor().max(0.0) as u32;
    let y1 = (cy + ry).ceil().min(img.height as f32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let nx = (x as f32 + 0.5 - cx) / rx.max(0.5);
            let ny = (y as f32 + 0.5 - cy) / ry.max(0.5);
            if nx * nx + ny * ny <= 1.0 {
                img.pixels[(y * img.width + x) as usize] = color;
            }
        }
    }
}
