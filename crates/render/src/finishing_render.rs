//! v5.0 #40: プロ・シネマフィニッシング描画パス
//! 35mm フィルムハレーション（高輝度赤橙散乱）・アナモルフィック水平レンズフレア・
//! オーガニック 35mm フィルムグレイン・周辺ビネット・シネマレターボックス・SMPTE バーンイン HUD

use crate::color::{LinearImage, LinearPremul, linear_to_srgb, srgb_to_linear};
use ve_core::{CinemaFinishingConfig, FrameRate, RationalTime, format_smpte_timecode};

#[inline]
fn hash_grain(x: u32, y: u32, frame: i64) -> f32 {
    let mut h = (u64::from(x))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(y).wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add((frame as u64).wrapping_mul(0x94D0_49BB_1331_11EB));
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    let u = ((h >> 40) as f32) / ((1u32 << 24) as f32);
    u * 2.0 - 1.0
}

/// シーン最終段へ 35mm ハレーション・アナモルフィックフレア・グレイン・ビネット・レターボックス・バーンインを適用する
pub fn apply_cinema_finishing(
    img: &mut LinearImage,
    cfg: &CinemaFinishingConfig,
    time: RationalTime,
    fps: FrameRate,
) {
    if !cfg.enabled || img.width < 4 || img.height < 4 {
        return;
    }
    let w = img.width as usize;
    let h = img.height as usize;

    // 1. 35mm フィルム・ハレーション（高輝度部からの赤橙光学散乱）
    if cfg.halation_strength > 0.01 {
        let thresh = cfg.halation_threshold.clamp(0.3, 1.0);
        let mut mask = vec![0.0f32; w * h];
        for (dst, px) in mask.iter_mut().zip(img.pixels.iter()) {
            let luma = 0.2126 * px.r + 0.7152 * px.g + 0.0722 * px.b;
            let excess = ((luma - thresh) / (1.0 - thresh).max(0.08)).clamp(0.0, 2.5);
            *dst = excess;
        }
        // 高速 2-Pass 分離ボックスブラー（半径は解像度比）
        let rad = ((w.min(h) / 48).clamp(2, 16)) as i32;
        let mut tmp = vec![0.0f32; w * h];
        let win = (rad * 2 + 1) as f32;
        for y in 0..h {
            let row = y * w;
            let mut acc = 0.0f32;
            for k in -rad..=rad {
                acc += mask[row + k.clamp(0, w as i32 - 1) as usize];
            }
            for x in 0..w {
                tmp[row + x] = acc / win;
                let left = (x as i32 - rad).clamp(0, w as i32 - 1) as usize;
                let right = (x as i32 + rad + 1).clamp(0, w as i32 - 1) as usize;
                acc += mask[row + right] - mask[row + left];
            }
        }
        let mut blur = vec![0.0f32; w * h];
        for x in 0..w {
            let mut acc = 0.0f32;
            for k in -rad..=rad {
                acc += tmp[(k.clamp(0, h as i32 - 1) as usize) * w + x];
            }
            for y in 0..h {
                blur[y * w + x] = acc / win;
                let top = (y as i32 - rad).clamp(0, h as i32 - 1) as usize;
                let bot = (y as i32 + rad + 1).clamp(0, h as i32 - 1) as usize;
                acc += tmp[bot * w + x] - tmp[top * w + x];
            }
        }
        let str_k = cfg.halation_strength.clamp(0.0, 1.5);
        for (px, &hal) in img.pixels.iter_mut().zip(blur.iter()) {
            if hal > 1e-4 {
                let g = hal * str_k;
                px.r = (px.r + g * 0.92).min(1.8);
                px.g = (px.g + g * 0.24).min(1.8);
                px.b = (px.b + g * 0.06).min(1.8);
            }
        }
    }

    // 2. アナモルフィック水平レンズフレア（走査線ごとの双方向 IIR 指数ストリーク）
    if cfg.anamorphic_flare > 0.01 {
        let flare_str = cfg.anamorphic_flare.clamp(0.0, 1.5);
        let decay = (-1.0 / (w as f32 * 0.14).max(6.0)).exp();
        let tint = cfg.anamorphic_tint;
        let mut streak = vec![0.0f32; w];
        for y in 0..h {
            let row = y * w;
            let mut run_l = 0.0f32;
            for (x, slot) in streak.iter_mut().enumerate() {
                let px = img.pixels[row + x];
                let peak = ((0.2126 * px.r + 0.7152 * px.g + 0.0722 * px.b) - 0.78).max(0.0);
                run_l = run_l * decay + peak * (1.0 - decay) * 3.2;
                *slot = run_l;
            }
            let mut run_r = 0.0f32;
            for (x, &left_val) in streak.iter().enumerate().rev() {
                let px = img.pixels[row + x];
                let peak = ((0.2126 * px.r + 0.7152 * px.g + 0.0722 * px.b) - 0.78).max(0.0);
                run_r = run_r * decay + peak * (1.0 - decay) * 3.2;
                let s = (left_val + run_r) * flare_str * 0.55;
                if s > 1e-4 {
                    let dst = &mut img.pixels[row + x];
                    dst.r = (dst.r + s * tint[0]).min(1.8);
                    dst.g = (dst.g + s * tint[1]).min(1.8);
                    dst.b = (dst.b + s * tint[2]).min(1.8);
                }
            }
        }
    }

    // 3. オーガニック 35mm フィルムグレイン ＆ 周辺ビネット
    let grain_amt = cfg.film_grain.clamp(0.0, 0.5);
    let vig_amt = cfg.vignette.clamp(0.0, 1.0);
    let frame_idx = (time.as_seconds() * fps.as_f64()).round() as i64;
    if grain_amt > 1e-4 || vig_amt > 1e-4 {
        let inv_w = 1.0 / (w as f32);
        let inv_h = 1.0 / (h as f32);
        for y in 0..h {
            let ny = (y as f32 + 0.5) * inv_h * 2.0 - 1.0;
            let row = y * w;
            for x in 0..w {
                let nx = (x as f32 + 0.5) * inv_w * 2.0 - 1.0;
                let px = &mut img.pixels[row + x];
                if vig_amt > 1e-4 {
                    let r2 = (nx * nx + ny * ny) * 0.5;
                    let vig = (1.0 - r2 * vig_amt * 0.85).clamp(0.18, 1.0);
                    px.r *= vig;
                    px.g *= vig;
                    px.b *= vig;
                }
                if grain_amt > 1e-4 {
                    let sr = linear_to_srgb(px.r.max(0.0));
                    let sg = linear_to_srgb(px.g.max(0.0));
                    let sb = linear_to_srgb(px.b.max(0.0));
                    let luma = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
                    let mid_weight = 1.0 - ((luma - 0.5).abs() * 1.6).clamp(0.0, 0.85);
                    let n =
                        hash_grain(x as u32, y as u32, frame_idx) * grain_amt * 0.35 * mid_weight;
                    px.r = srgb_to_linear((sr + n).clamp(0.0, 1.0)) * px.a;
                    px.g = srgb_to_linear((sg + n).clamp(0.0, 1.0)) * px.a;
                    px.b = srgb_to_linear((sb + n).clamp(0.0, 1.0)) * px.a;
                }
            }
        }
    }

    // 4. シネマスコープ・レターボックス / ピラーボックス
    if let Some(target_ratio) = cfg.letterbox.target_ratio() {
        let cur_ratio = (w as f32) / (h as f32).max(1.0);
        let black = LinearPremul {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        if target_ratio > cur_ratio + 0.02 {
            let active_h = ((w as f32) / target_ratio).round() as usize;
            let bar = h.saturating_sub(active_h) / 2;
            for y in 0..bar {
                for x in 0..w {
                    img.pixels[y * w + x] = black;
                    img.pixels[(h - 1 - y) * w + x] = black;
                }
            }
        } else if target_ratio < cur_ratio - 0.02 {
            let active_w = ((h as f32) * target_ratio).round() as usize;
            let bar = w.saturating_sub(active_w) / 2;
            for y in 0..h {
                let row = y * w;
                for x in 0..bar {
                    img.pixels[row + x] = black;
                    img.pixels[row + (w - 1 - x)] = black;
                }
            }
        }
    }

    // 5. クライアント納品確認用 SMPTE タイムコード / 透かしバーンイン HUD
    let wm = cfg.watermark_text.trim();
    if cfg.burn_in_timecode || !wm.is_empty() {
        let mut label = String::new();
        if cfg.burn_in_timecode {
            label.push_str(&format!("TC {}", format_smpte_timecode(time, fps)));
        }
        if !wm.is_empty() {
            if !label.is_empty() {
                label.push_str("  |  ");
            }
            label.push_str(&wm.to_uppercase());
        }
        draw_burn_in_pill(img, &label);
    }
}

fn draw_burn_in_pill(img: &mut LinearImage, text: &str) {
    let w = img.width as usize;
    let h = img.height as usize;
    let scale = (h / 240).clamp(1, 4);
    let glyph_w = 6 * scale;
    let glyph_h = 7 * scale;
    let pad_x = 8 * scale;
    let pad_y = 4 * scale;
    let chars: Vec<char> = text.chars().take(48).collect();
    let box_w = (chars.len() * glyph_w + pad_x * 2).min(w.saturating_sub(8));
    let box_h = (glyph_h + pad_y * 2).min(h.saturating_sub(8));
    let ox = (w.saturating_sub(box_w)) / 2;
    let oy = h.saturating_sub(box_h + (h / 24).max(4));

    // 半透明ダーク座布団
    for y in oy..(oy + box_h).min(h) {
        let row = y * w;
        for x in ox..(ox + box_w).min(w) {
            let p = &mut img.pixels[row + x];
            p.r *= 0.25;
            p.g *= 0.25;
            p.b *= 0.25;
        }
    }

    let white = LinearPremul {
        r: 0.95,
        g: 0.95,
        b: 0.95,
        a: 1.0,
    };
    for (ci, ch) in chars.iter().enumerate() {
        let gx0 = ox + pad_x + ci * glyph_w;
        if gx0 + 5 * scale >= ox + box_w {
            break;
        }
        let bits = bitmap_5x7(*ch);
        for (ry, &row_bits) in bits.iter().enumerate() {
            for rx in 0..5usize {
                if (row_bits >> (4 - rx)) & 1 == 1 {
                    for sy in 0..scale {
                        let py = oy + pad_y + ry * scale + sy;
                        if py >= h {
                            continue;
                        }
                        for sx in 0..scale {
                            let px = gx0 + rx * scale + sx;
                            if px < w {
                                img.pixels[py * w + px] = white;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn bitmap_5x7(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::LetterboxAspect;

    #[test]
    fn cinema_finishing_applies_halation_flare_letterbox_and_timecode_burnin() {
        let mut img = LinearImage::new(
            128,
            72,
            LinearPremul {
                r: 0.04,
                g: 0.04,
                b: 0.05,
                a: 1.0,
            },
        );
        // 中央に高輝度光源スポットを配置
        for y in 30..42 {
            for x in 58..70 {
                img.pixels[y * 128 + x] = LinearPremul {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                };
            }
        }

        let cfg = CinemaFinishingConfig {
            enabled: true,
            halation_strength: 0.85,
            halation_threshold: 0.6,
            anamorphic_flare: 0.9,
            anamorphic_tint: [0.2, 0.7, 1.0],
            film_grain: 0.08,
            vignette: 0.25,
            letterbox: LetterboxAspect::Cinema239,
            burn_in_timecode: true,
            watermark_text: "TEST".into(),
        };
        apply_cinema_finishing(
            &mut img,
            &cfg,
            RationalTime::from_seconds(1.5),
            FrameRate::FPS_30,
        );

        // レターボックス上端 (y=0) は完全黒
        let top = img.pixels[10];
        assert!(top.r < 1e-4 && top.g < 1e-4 && top.b < 1e-4);

        // 光源の水平横方向 (x=40, y=36) へアナモルフィック青フレア＆ハレーションが広がっていること
        let side = img.pixels[36 * 128 + 40];
        assert!(
            side.b > 0.08 && side.r > 0.05,
            "expected horizontal flare/halation bleed at (40,36), got {side:?}"
        );
    }
}
