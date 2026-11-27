//! Render Graph エフェクトチェーン・トランジション・ブラー・回転・グループ変形

use super::*;

pub(super) fn apply_group_transform(src: &LinearImage, tf: Transform2D) -> LinearImage {
    let mut dst = LinearImage::new(src.width, src.height, LinearPremul::TRANSPARENT);
    let ox = tf.x.round() as i32;
    let oy = tf.y.round() as i32;
    let sx = tf.scale_x.max(0.01);
    let sy = tf.scale_y.max(0.01);
    let opacity = tf.opacity.clamp(0.0, 1.0);

    if (sx - 1.0).abs() < 1e-4 && (sy - 1.0).abs() < 1e-4 {
        dst.blend_over(src, ox, oy, opacity);
        return dst;
    }

    let cx = src.width as f32 * 0.5;
    let cy = src.height as f32 * 0.5;
    for y in 0..dst.height {
        for x in 0..dst.width {
            let src_x = ((x as f32 - tf.x - cx) / sx + cx).round() as i32;
            let src_y = ((y as f32 - tf.y - cy) / sy + cy).round() as i32;
            if src_x >= 0 && src_x < src.width as i32 && src_y >= 0 && src_y < src.height as i32 {
                let mut p = src.pixels[(src_y as u32 * src.width + src_x as u32) as usize];
                p.r *= opacity;
                p.g *= opacity;
                p.b *= opacity;
                p.a *= opacity;
                dst.pixels[(y * dst.width + x) as usize] = p;
            }
        }
    }
    dst
}

pub(super) fn apply_effect_chain(
    image: &mut LinearImage,
    chain: &[EffectNode],
    time_sec: f64,
    plugins: &PluginRegistry,
) {
    for node in chain {
        if node.enabled {
            apply_named_effect(image, &node.effect_type, &node.params, time_sec, plugins);
        }
    }
}

pub(crate) fn apply_named_effect(
    image: &mut LinearImage,
    effect_type: &str,
    params: &BTreeMap<String, f32>,
    time_sec: f64,
    plugins: &PluginRegistry,
) {
    match effect_type {
        "ai_segment" => {
            let cfg = crate::ai_vision::SegmentationConfig {
                sensitivity: params.get("sensitivity").copied().unwrap_or(0.55),
                edge_feather_px: params.get("edge_feather_px").copied().unwrap_or(1.5),
                fg_bias: params.get("fg_bias").copied().unwrap_or(0.45),
                skin_boost: params.get("skin_boost").copied().unwrap_or(0.65),
                refine_iters: params.get("refine_iters").copied().unwrap_or(2.0).round() as u32,
                despill_strength: params.get("despill_strength").copied().unwrap_or(0.35),
                invert: params.get("invert").copied().unwrap_or(0.0) >= 0.5,
            };
            crate::ai_vision::apply_ai_segmentation(image, &cfg);
        }
        "ai_super_res" => {
            let sharpen = params.get("sharpen").copied().unwrap_or(0.55);
            let denoise = params.get("denoise").copied().unwrap_or(0.2);
            crate::ai_vision::apply_ai_super_res_detail(image, sharpen, denoise);
        }
        "brightness" => {
            let gain = params.get("gain").copied().unwrap_or(1.0).max(0.0);
            for px in &mut image.pixels {
                px.r *= gain;
                px.g *= gain;
                px.b *= gain;
            }
        }
        "invert" => {
            for px in &mut image.pixels {
                if px.a > 0.0 {
                    px.r = (px.a - px.r).max(0.0);
                    px.g = (px.a - px.g).max(0.0);
                    px.b = (px.a - px.b).max(0.0);
                }
            }
        }
        "color_grade" => {
            let exposure = params.get("exposure").copied().unwrap_or(0.0);
            let contrast = params.get("contrast").copied().unwrap_or(1.0).max(0.0);
            let saturation = params.get("saturation").copied().unwrap_or(1.0).max(0.0);
            let exp_mul = 2.0_f32.powf(exposure);
            for px in &mut image.pixels {
                if px.a <= 1e-6 {
                    continue;
                }
                let mut r = (px.r / px.a) * exp_mul;
                let mut g = (px.g / px.a) * exp_mul;
                let mut b = (px.b / px.a) * exp_mul;
                r = (r - 0.5) * contrast + 0.5;
                g = (g - 0.5) * contrast + 0.5;
                b = (b - 0.5) * contrast + 0.5;
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                r = (luma + (r - luma) * saturation).max(0.0);
                g = (luma + (g - luma) * saturation).max(0.0);
                b = (luma + (b - luma) * saturation).max(0.0);
                px.r = r * px.a;
                px.g = g * px.a;
                px.b = b * px.a;
            }
        }
        "blur" => {
            let radius = params.get("radius").copied().unwrap_or(2.0).round() as i32;
            if radius > 0 {
                box_blur_separable(image, radius.clamp(1, 16));
            }
        }
        "glow" => {
            let radius = params.get("radius").copied().unwrap_or(3.0).round() as i32;
            let threshold = params.get("threshold").copied().unwrap_or(0.5);
            let intensity = params.get("intensity").copied().unwrap_or(1.0).max(0.0);
            let mut bright = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            for (dst, src) in bright.pixels.iter_mut().zip(image.pixels.iter()) {
                let luma = src.luma();
                if luma > threshold {
                    let excess = (luma - threshold).max(0.0);
                    dst.r = src.r * excess;
                    dst.g = src.g * excess;
                    dst.b = src.b * excess;
                    dst.a = src.a;
                }
            }
            box_blur_separable(&mut bright, radius.clamp(1, 16));
            for (dst, b) in image.pixels.iter_mut().zip(bright.pixels.iter()) {
                dst.r += b.r * intensity;
                dst.g += b.g * intensity;
                dst.b += b.b * intensity;
            }
        }
        "clip_rect" => {
            let left = params.get("left").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let right = params.get("right").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let top = params.get("top").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let bottom = params.get("bottom").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let w = image.width.max(1) as f32;
            let h = image.height.max(1) as f32;
            for y in 0..image.height {
                let v = (y as f32 + 0.5) / h;
                for x in 0..image.width {
                    let u = (x as f32 + 0.5) / w;
                    if u < left || u > (1.0 - right) || v < top || v > (1.0 - bottom) {
                        image.pixels[(y * image.width + x) as usize] = LinearPremul::TRANSPARENT;
                    }
                }
            }
        }
        "chroma_key" => {
            // グリーンバック / ブルーバック / 任意色クロマキー＋スピル抑制
            let kr = params.get("key_r").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let kg = params.get("key_g").copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let kb = params.get("key_b").copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let tolerance = params
                .get("tolerance")
                .copied()
                .unwrap_or(0.35)
                .clamp(0.01, 1.5);
            let softness = params
                .get("softness")
                .copied()
                .unwrap_or(0.15)
                .clamp(0.01, 1.0);
            let spill = params
                .get("spill_suppress")
                .copied()
                .unwrap_or(0.4)
                .clamp(0.0, 1.0);
            let key_lin = LinearPremul::from_srgb_straight(RgbaColor {
                r: kr,
                g: kg,
                b: kb,
                a: 1.0,
            });
            for px in &mut image.pixels {
                if px.a <= 1e-6 {
                    continue;
                }
                let inv_a = 1.0 / px.a;
                let r = px.r * inv_a;
                let mut g = px.g * inv_a;
                let b = px.b * inv_a;
                let dr = r - key_lin.r;
                let dg = g - key_lin.g;
                let db = b - key_lin.b;
                let dist = (dr * dr + dg * dg + db * db).sqrt();
                let mask = ((dist - tolerance) / softness).clamp(0.0, 1.0);
                if spill > 0.0 && kg >= kr && kg >= kb {
                    let max_rb = r.max(b);
                    if g > max_rb {
                        g -= (g - max_rb) * spill;
                    }
                }
                let new_a = px.a * mask;
                px.r = r * new_a;
                px.g = g * new_a;
                px.b = b * new_a;
                px.a = new_a;
            }
        }
        "luma_key" => {
            // 黒背景・白背景エフェクト素材透過用のルミナンスキー
            let threshold = params
                .get("threshold")
                .copied()
                .unwrap_or(0.12)
                .clamp(0.0, 1.0);
            let softness = params
                .get("softness")
                .copied()
                .unwrap_or(0.08)
                .clamp(0.005, 1.0);
            let invert = params.get("invert").copied().unwrap_or(0.0) >= 0.5;
            for px in &mut image.pixels {
                if px.a <= 1e-6 {
                    continue;
                }
                let luma = px.luma() / px.a.max(1e-5);
                let mut mask = ((luma - threshold) / softness).clamp(0.0, 1.0);
                if invert {
                    mask = 1.0 - mask;
                }
                px.r *= mask;
                px.g *= mask;
                px.b *= mask;
                px.a *= mask;
            }
        }
        "mosaic" => {
            // ブロック平均モザイク / ピクセレート（個人情報隠し・ネタ演出）
            let block = params
                .get("block_size")
                .copied()
                .unwrap_or(12.0)
                .round()
                .clamp(2.0, 128.0) as u32;
            let w = image.width;
            let h = image.height;
            for by in (0..h).step_by(block as usize) {
                let y_end = (by + block).min(h);
                for bx in (0..w).step_by(block as usize) {
                    let x_end = (bx + block).min(w);
                    let mut sum = [0.0f64; 4];
                    let mut cnt = 0.0f64;
                    for y in by..y_end {
                        for x in bx..x_end {
                            let p = image.pixels[(y * w + x) as usize];
                            sum[0] += f64::from(p.r);
                            sum[1] += f64::from(p.g);
                            sum[2] += f64::from(p.b);
                            sum[3] += f64::from(p.a);
                            cnt += 1.0;
                        }
                    }
                    if cnt > 0.0 {
                        let inv = 1.0 / cnt;
                        let avg = LinearPremul {
                            r: (sum[0] * inv) as f32,
                            g: (sum[1] * inv) as f32,
                            b: (sum[2] * inv) as f32,
                            a: (sum[3] * inv) as f32,
                        };
                        for y in by..y_end {
                            for x in bx..x_end {
                                image.pixels[(y * w + x) as usize] = avg;
                            }
                        }
                    }
                }
            }
        }
        "rounded_clip" => {
            // 角丸クリッピング（ワイプ枠・小窓演出用）
            let w = image.width as f32;
            let h = image.height as f32;
            let margin = params
                .get("margin")
                .copied()
                .unwrap_or(0.0)
                .clamp(0.0, w.min(h) * 0.45);
            let max_r = ((w - margin * 2.0).min(h - margin * 2.0) * 0.5).max(0.0);
            let radius = params
                .get("radius")
                .copied()
                .unwrap_or(28.0)
                .clamp(0.0, max_r);
            let cx = w * 0.5;
            let cy = h * 0.5;
            let hx = (w * 0.5 - margin - radius).max(0.0);
            let hy = (h * 0.5 - margin - radius).max(0.0);
            for y in 0..image.height {
                let fy = y as f32 + 0.5;
                for x in 0..image.width {
                    let fx = x as f32 + 0.5;
                    let qx = (fx - cx).abs() - hx;
                    let qy = (fy - cy).abs() - hy;
                    let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                    let cov = (0.5 - dist).clamp(0.0, 1.0);
                    if cov < 1.0 {
                        let p = &mut image.pixels[(y * image.width + x) as usize];
                        p.r *= cov;
                        p.g *= cov;
                        p.b *= cov;
                        p.a *= cov;
                    }
                }
            }
        }
        "diagonal_clip" => {
            // AviUtl 定番「斜めクリッピング」（画面分割・スラッシュ演出）
            let angle_rad = params
                .get("angle_deg")
                .copied()
                .unwrap_or(30.0)
                .to_radians();
            let center_x = params
                .get("center_x")
                .copied()
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let center_y = params
                .get("center_y")
                .copied()
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let half_width = params.get("width").copied().unwrap_or(0.0).max(0.0) * 0.5;
            let feather = params.get("feather").copied().unwrap_or(2.0).max(0.5);
            let nx = angle_rad.cos();
            let ny = angle_rad.sin();
            let cx = image.width as f32 * center_x;
            let cy = image.height as f32 * center_y;
            for y in 0..image.height {
                let dy = y as f32 + 0.5 - cy;
                for x in 0..image.width {
                    let dx = x as f32 + 0.5 - cx;
                    let signed = dx * nx + dy * ny;
                    let cov = if half_width > 0.0 {
                        // バンド幅指定時は中心帯を残す
                        let d = half_width - signed.abs();
                        (d / feather + 0.5).clamp(0.0, 1.0)
                    } else {
                        // 片側クリップ
                        (signed / feather + 0.5).clamp(0.0, 1.0)
                    };
                    if cov < 1.0 {
                        let p = &mut image.pixels[(y * image.width + x) as usize];
                        p.r *= cov;
                        p.g *= cov;
                        p.b *= cov;
                        p.a *= cov;
                    }
                }
            }
        }
        "shake" => {
            // AviUtl 定番「振動」（手ブレ・ツッコミ衝撃シェイク）
            let amp_x = params
                .get("amp_x")
                .copied()
                .unwrap_or(14.0)
                .clamp(0.0, 200.0);
            let amp_y = params
                .get("amp_y")
                .copied()
                .unwrap_or(10.0)
                .clamp(0.0, 200.0);
            let freq = params
                .get("freq")
                .copied()
                .unwrap_or(24.0)
                .clamp(0.5, 120.0);
            let seed = params.get("seed").copied().unwrap_or(1.0);
            let t = time_sec as f32 * freq;
            let ox = ((t * 1.7 + seed * 3.1).sin() * 0.65 + (t * 3.3 + seed).cos() * 0.35) * amp_x;
            let oy = ((t * 2.1 + seed * 5.7).cos() * 0.65 + (t * 4.1 + seed).sin() * 0.35) * amp_y;
            let dx = ox.round() as i32;
            let dy = oy.round() as i32;
            if dx != 0 || dy != 0 {
                let mut shifted =
                    LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
                shifted.blend_over(image, dx, dy, 1.0);
                *image = shifted;
            }
        }
        "outline" => {
            // 任意画像・立ち絵・字幕へ適用できる縁取りフィルタ
            let width = params.get("width").copied().unwrap_or(3.0).clamp(1.0, 14.0);
            let oc = LinearPremul::from_srgb_straight(RgbaColor {
                r: params.get("r").copied().unwrap_or(0.0),
                g: params.get("g").copied().unwrap_or(0.0),
                b: params.get("b").copied().unwrap_or(0.0),
                a: params.get("a").copied().unwrap_or(1.0),
            });
            let rad = width.ceil() as i32;
            let rad_sq = width * width + 0.5;
            let mut offsets = Vec::new();
            for dy in -rad..=rad {
                for dx in -rad..=rad {
                    if (dx * dx + dy * dy) as f32 <= rad_sq {
                        offsets.push((dx, dy));
                    }
                }
            }
            let w = image.width as i32;
            let h = image.height as i32;
            let mut out = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            for y in 0..h {
                for x in 0..w {
                    let mut max_a = 0.0f32;
                    for &(dx, dy) in &offsets {
                        let sx = x + dx;
                        let sy = y + dy;
                        if sx >= 0 && sx < w && sy >= 0 && sy < h {
                            let a = image.pixels[(sy as u32 * image.width + sx as u32) as usize].a;
                            if a > max_a {
                                max_a = a;
                                if max_a >= 0.999 {
                                    break;
                                }
                            }
                        }
                    }
                    let idx = (y as u32 * image.width + x as u32) as usize;
                    let border = LinearPremul {
                        r: oc.r * max_a,
                        g: oc.g * max_a,
                        b: oc.b * max_a,
                        a: oc.a * max_a,
                    };
                    out.pixels[idx] = image.pixels[idx].over(border);
                }
            }
            *image = out;
        }
        "drop_shadow" => {
            // 任意画像・立ち絵・字幕へ適用できるドロップシャドウフィルタ
            let ox = params.get("offset_x").copied().unwrap_or(6.0).round() as i32;
            let oy = params.get("offset_y").copied().unwrap_or(6.0).round() as i32;
            let blur = params
                .get("blur")
                .copied()
                .unwrap_or(4.0)
                .round()
                .clamp(0.0, 14.0) as i32;
            let opacity = params
                .get("opacity")
                .copied()
                .unwrap_or(0.6)
                .clamp(0.0, 1.0);
            let mut shadow = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            let w = image.width as i32;
            let h = image.height as i32;
            for y in 0..h {
                let sy = y - oy;
                if sy < 0 || sy >= h {
                    continue;
                }
                for x in 0..w {
                    let sx = x - ox;
                    if sx < 0 || sx >= w {
                        continue;
                    }
                    let a =
                        image.pixels[(sy as u32 * image.width + sx as u32) as usize].a * opacity;
                    shadow.pixels[(y as u32 * image.width + x as u32) as usize] = LinearPremul {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a,
                    };
                }
            }
            if blur > 0 {
                box_blur_separable(&mut shadow, blur);
            }
            for (dst, &src) in shadow.pixels.iter_mut().zip(image.pixels.iter()) {
                *dst = src.over(*dst);
            }
            *image = shadow;
        }
        other => {
            // Phase 4: プラグイン API 経由のエフェクトディスパッチ
            if plugins.get_effect(other).is_some() {
                let mut raw: Vec<[f32; 4]> =
                    image.pixels.iter().map(|p| [p.r, p.g, p.b, p.a]).collect();
                if plugins
                    .apply_effect(other, &mut raw, image.width, image.height, time_sec, params)
                    .is_ok()
                {
                    for (dst, src) in image.pixels.iter_mut().zip(raw) {
                        dst.r = src[0];
                        dst.g = src[1];
                        dst.b = src[2];
                        dst.a = src[3];
                    }
                }
            }
        }
    }
}

pub(super) fn resolve_scope_layers(owner_layer_idx: usize, scope: FilterTargetScope) -> Vec<usize> {
    match scope {
        FilterTargetScope::ImmediateLayer => {
            if owner_layer_idx > 0 {
                vec![owner_layer_idx - 1]
            } else {
                vec![]
            }
        }
        FilterTargetScope::LayerRange { count } => {
            let start = owner_layer_idx.saturating_sub(count);
            (start..owner_layer_idx).collect()
        }
        FilterTargetScope::AllRemaining => (0..owner_layer_idx).collect(),
    }
}

/// AviUtl2 風シーンチェンジ（トランジションオブジェクト）を対象レイヤーへ適用する
pub(super) fn apply_transition(
    image: &mut LinearImage,
    transition: TransitionKind,
    progress: f32,
    softness: f32,
) {
    let p = progress.clamp(0.0, 1.0);
    let soft = softness.clamp(0.005, 0.5);
    let w = image.width.max(1) as f32;
    let h = image.height.max(1) as f32;

    match transition {
        TransitionKind::CrossFade => {
            for px in &mut image.pixels {
                px.r *= p;
                px.g *= p;
                px.b *= p;
                px.a *= p;
            }
        }
        TransitionKind::WipeLeft
        | TransitionKind::WipeRight
        | TransitionKind::WipeUp
        | TransitionKind::WipeDown => {
            let edge = p * (1.0 + soft) - soft * 0.5;
            for y in 0..image.height {
                let v = (y as f32 + 0.5) / h;
                for x in 0..image.width {
                    let u = (x as f32 + 0.5) / w;
                    let coord = match transition {
                        TransitionKind::WipeLeft => u,
                        TransitionKind::WipeRight => 1.0 - u,
                        TransitionKind::WipeUp => v,
                        TransitionKind::WipeDown => 1.0 - v,
                        _ => u,
                    };
                    let mask = ((edge - coord) / soft + 0.5).clamp(0.0, 1.0);
                    if mask < 1.0 {
                        let px = &mut image.pixels[(y * image.width + x) as usize];
                        px.r *= mask;
                        px.g *= mask;
                        px.b *= mask;
                        px.a *= mask;
                    }
                }
            }
        }
        TransitionKind::SlideLeft | TransitionKind::SlideRight => {
            let dir = if matches!(transition, TransitionKind::SlideLeft) {
                -1.0
            } else {
                1.0
            };
            let shift_x = (dir * (1.0 - p) * w).round() as i32;
            let mut moved = LinearImage::new(image.width, image.height, LinearPremul::TRANSPARENT);
            moved.blend_over(image, shift_x, 0, 1.0);
            *image = moved;
        }
        TransitionKind::Flash => {
            // 中央 (p=0.5) でピークに達する発光トランジション
            let bell = (1.0 - (p * 2.0 - 1.0).abs()).clamp(0.0, 1.0);
            let boost = bell * bell * 2.5;
            for px in &mut image.pixels {
                if px.a > 0.0 {
                    px.r += boost * px.a;
                    px.g += boost * px.a;
                    px.b += boost * px.a;
                }
            }
        }
        TransitionKind::RandomLines => {
            // 決定論的ハッシュによる水平スキャンライン出現（AviUtl2「ランダムライン」相当）
            let bands = 32u32;
            let band_h = (image.height / bands).max(1);
            for y in 0..image.height {
                let band_idx = y / band_h;
                let hash = ((band_idx.wrapping_mul(2_654_435_761)) >> 16) as f32 / 65535.0;
                if hash > p {
                    let row_start = (y * image.width) as usize;
                    image.pixels[row_start..row_start + image.width as usize]
                        .fill(LinearPremul::TRANSPARENT);
                }
            }
        }
    }
}

/// O(W * H) スライディングウィンドウ分離ボックスブラー（半径 R に依存しない一定計算量＋行優先メモリアクセス）
pub(crate) fn box_blur_separable(image: &mut LinearImage, radius: i32) {
    let w = image.width as i32;
    let h = image.height as i32;
    if w == 0 || h == 0 || radius <= 0 {
        return;
    }
    let wu = w as usize;
    let hu = h as usize;
    let mut temp = vec![LinearPremul::TRANSPARENT; image.pixels.len()];
    let inv_denom = 1.0_f64 / f64::from(radius * 2 + 1);

    // 水平パス（行ごとにスライディングウィンドウ加算・中央領域は clamp 分岐ゼロ）
    let r_u = radius as usize;
    let inner_start = (r_u + 1).min(wu);
    let inner_end = wu.saturating_sub(r_u).max(inner_start);
    for y in 0..hu {
        let src_row = &image.pixels[y * wu..(y + 1) * wu];
        let dst_row = &mut temp[y * wu..(y + 1) * wu];

        let mut acc = [0.0f64; 4];
        for dx in -radius..=radius {
            let sx = dx.clamp(0, w - 1) as usize;
            let p = src_row[sx];
            acc[0] += f64::from(p.r);
            acc[1] += f64::from(p.g);
            acc[2] += f64::from(p.b);
            acc[3] += f64::from(p.a);
        }
        dst_row[0] = LinearPremul {
            r: (acc[0] * inv_denom).max(0.0) as f32,
            g: (acc[1] * inv_denom).max(0.0) as f32,
            b: (acc[2] * inv_denom).max(0.0) as f32,
            a: (acc[3] * inv_denom).max(0.0) as f32,
        };

        // 左端境界
        for (x, dst_px) in dst_row.iter_mut().enumerate().take(inner_start).skip(1) {
            let sub_x = (x as i32 - 1 - radius).clamp(0, w - 1) as usize;
            let add_x = (x as i32 + radius).clamp(0, w - 1) as usize;
            let p_sub = src_row[sub_x];
            let p_add = src_row[add_x];
            acc[0] += f64::from(p_add.r) - f64::from(p_sub.r);
            acc[1] += f64::from(p_add.g) - f64::from(p_sub.g);
            acc[2] += f64::from(p_add.b) - f64::from(p_sub.b);
            acc[3] += f64::from(p_add.a) - f64::from(p_sub.a);
            *dst_px = LinearPremul {
                r: (acc[0] * inv_denom).max(0.0) as f32,
                g: (acc[1] * inv_denom).max(0.0) as f32,
                b: (acc[2] * inv_denom).max(0.0) as f32,
                a: (acc[3] * inv_denom).max(0.0) as f32,
            };
        }
        // 中央領域（境界クランプ不要の連続走査）
        for x in inner_start..inner_end {
            let p_sub = src_row[x - 1 - r_u];
            let p_add = src_row[x + r_u];
            acc[0] += f64::from(p_add.r) - f64::from(p_sub.r);
            acc[1] += f64::from(p_add.g) - f64::from(p_sub.g);
            acc[2] += f64::from(p_add.b) - f64::from(p_sub.b);
            acc[3] += f64::from(p_add.a) - f64::from(p_sub.a);
            dst_row[x] = LinearPremul {
                r: (acc[0] * inv_denom).max(0.0) as f32,
                g: (acc[1] * inv_denom).max(0.0) as f32,
                b: (acc[2] * inv_denom).max(0.0) as f32,
                a: (acc[3] * inv_denom).max(0.0) as f32,
            };
        }
        // 右端境界
        for (x, dst_px) in dst_row.iter_mut().enumerate().skip(inner_end) {
            let sub_x = (x as i32 - 1 - radius).clamp(0, w - 1) as usize;
            let add_x = (x as i32 + radius).clamp(0, w - 1) as usize;
            let p_sub = src_row[sub_x];
            let p_add = src_row[add_x];
            acc[0] += f64::from(p_add.r) - f64::from(p_sub.r);
            acc[1] += f64::from(p_add.g) - f64::from(p_sub.g);
            acc[2] += f64::from(p_add.b) - f64::from(p_sub.b);
            acc[3] += f64::from(p_add.a) - f64::from(p_sub.a);
            *dst_px = LinearPremul {
                r: (acc[0] * inv_denom).max(0.0) as f32,
                g: (acc[1] * inv_denom).max(0.0) as f32,
                b: (acc[2] * inv_denom).max(0.0) as f32,
                a: (acc[3] * inv_denom).max(0.0) as f32,
            };
        }
    }

    // 垂直パス（幅 W の行アキュムレータで常に連続ストライド1のキャッシュフレンドリー走査）
    let mut col_acc = vec![[0.0f64; 4]; wu];
    for dy in -radius..=radius {
        let sy = dy.clamp(0, h - 1) as usize;
        let row = &temp[sy * wu..(sy + 1) * wu];
        for (acc, &p) in col_acc.iter_mut().zip(row.iter()) {
            acc[0] += f64::from(p.r);
            acc[1] += f64::from(p.g);
            acc[2] += f64::from(p.b);
            acc[3] += f64::from(p.a);
        }
    }
    for (dst, acc) in image.pixels[..wu].iter_mut().zip(col_acc.iter()) {
        *dst = LinearPremul {
            r: (acc[0] * inv_denom).max(0.0) as f32,
            g: (acc[1] * inv_denom).max(0.0) as f32,
            b: (acc[2] * inv_denom).max(0.0) as f32,
            a: (acc[3] * inv_denom).max(0.0) as f32,
        };
    }

    for y in 1..h {
        let sub_y = (y - 1 - radius).clamp(0, h - 1) as usize;
        let add_y = (y + radius).clamp(0, h - 1) as usize;
        let sub_row = &temp[sub_y * wu..(sub_y + 1) * wu];
        let add_row = &temp[add_y * wu..(add_y + 1) * wu];
        let dst_row = &mut image.pixels[(y as usize) * wu..(y as usize + 1) * wu];
        for ((acc, (&p_add, &p_sub)), dst) in col_acc
            .iter_mut()
            .zip(add_row.iter().zip(sub_row.iter()))
            .zip(dst_row.iter_mut())
        {
            acc[0] += f64::from(p_add.r) - f64::from(p_sub.r);
            acc[1] += f64::from(p_add.g) - f64::from(p_sub.g);
            acc[2] += f64::from(p_add.b) - f64::from(p_sub.b);
            acc[3] += f64::from(p_add.a) - f64::from(p_sub.a);
            *dst = LinearPremul {
                r: (acc[0] * inv_denom).max(0.0) as f32,
                g: (acc[1] * inv_denom).max(0.0) as f32,
                b: (acc[2] * inv_denom).max(0.0) as f32,
                a: (acc[3] * inv_denom).max(0.0) as f32,
            };
        }
    }
}

/// Linear Premultiplied 画像を中心軸まわりに指定角度（度）でバイリニア回転する
pub(crate) fn rotate_linear_image(src: &LinearImage, angle_deg: f32) -> LinearImage {
    let rad = angle_deg.to_radians();
    let cos = rad.cos();
    let sin = rad.sin();
    let sw = src.width as f32;
    let sh = src.height as f32;
    let out_w = (sw * cos.abs() + sh * sin.abs()).ceil().clamp(1.0, 4096.0) as u32;
    let out_h = (sw * sin.abs() + sh * cos.abs()).ceil().clamp(1.0, 4096.0) as u32;

    let mut dst = LinearImage::new(out_w, out_h, LinearPremul::TRANSPARENT);
    let src_cx = sw * 0.5;
    let src_cy = sh * 0.5;
    let dst_cx = out_w as f32 * 0.5;
    let dst_cy = out_h as f32 * 0.5;
    let iw = src.width as i32;
    let ih = src.height as i32;
    let src_w_u = src.width as usize;

    let sample = |x: i32, y: i32| -> LinearPremul {
        if x < 0 || y < 0 || x >= iw || y >= ih {
            LinearPremul::TRANSPARENT
        } else {
            src.pixels[y as usize * src_w_u + x as usize]
        }
    };

    for y in 0..out_h {
        let dy = y as f32 + 0.5 - dst_cy;
        let row_sx_base = dy * sin + src_cx - 0.5;
        let row_sy_base = dy * cos + src_cy - 0.5;
        let dst_row = &mut dst.pixels[(y * out_w) as usize..((y + 1) * out_w) as usize];
        for (x, dst_px) in dst_row.iter_mut().enumerate() {
            let dx = x as f32 + 0.5 - dst_cx;
            // 逆回転でソース座標を算出
            let sx = dx * cos + row_sx_base;
            let sy = -dx * sin + row_sy_base;
            if sx < -1.0 || sy < -1.0 || sx >= sw || sy >= sh {
                continue;
            }
            let x0 = sx.floor() as i32;
            let y0 = sy.floor() as i32;
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let (p00, p10, p01, p11) = if x0 >= 0 && y0 >= 0 && x0 + 1 < iw && y0 + 1 < ih {
                let r0 = y0 as usize * src_w_u + x0 as usize;
                let r1 = r0 + src_w_u;
                (
                    src.pixels[r0],
                    src.pixels[r0 + 1],
                    src.pixels[r1],
                    src.pixels[r1 + 1],
                )
            } else {
                (
                    sample(x0, y0),
                    sample(x0 + 1, y0),
                    sample(x0, y0 + 1),
                    sample(x0 + 1, y0 + 1),
                )
            };
            let inv_fx = 1.0 - fx;
            let inv_fy = 1.0 - fy;
            let w00 = inv_fx * inv_fy;
            let w10 = fx * inv_fy;
            let w01 = inv_fx * fy;
            let w11 = fx * fy;
            *dst_px = LinearPremul {
                r: p00.r * w00 + p10.r * w10 + p01.r * w01 + p11.r * w11,
                g: p00.g * w00 + p10.g * w10 + p01.g * w01 + p11.g * w11,
                b: p00.b * w00 + p10.b * w10 + p01.b * w01 + p11.b * w11,
                a: p00.a * w00 + p10.a * w10 + p01.a * w01 + p11.a * w11,
            };
        }
    }
    dst
}
