//! v10.0 #57〜#58: 360° 全天球投影＆ステレオスコピック空間 3D パス・2D Navier-Stokes 流体・PBD クロスラスタライザ
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで以下を決定論的に描画する:
//! - #57 Equirectangular ⇔ OverCapture / Tiny Planet 逆射影、水平ロックガイド、SBS / TAB / Red-Cyan アナグリフ空間 3D 合成
//! - #58 Jos Stam 2D Navier-Stokes 流体フィールドのバイリニア黒体／スモーク／墨／ネオンシェーディング
//! - #58 PBD クロス 3D メッシュの Z バッファ付き法線シェーディング＆織りストライプ＆ワイヤーフレーム描画

use crate::color::{LinearImage, LinearPremul, srgb_to_linear};
use ve_core::{
    ClothSimConfig, FluidPreset, FluidSimConfig, Projection360Mode, SpatialVideoConfig,
    StereoLayout, evaluate_horizon_locked_euler, project_screen_to_equirect_uv,
    simulate_cloth_pbd_at_time, simulate_fluid_2d_at_time,
};

#[inline]
fn sample_bilinear_wrap(img: &LinearImage, u: f32, v: f32) -> LinearPremul {
    if img.width == 0 || img.height == 0 {
        return LinearPremul::TRANSPARENT;
    }
    let w = img.width as usize;
    let h = img.height as usize;
    let fx = u.rem_euclid(1.0) * (w as f32 - 1.0).max(0.0);
    let fy = v.clamp(0.0, 1.0) * (h as f32 - 1.0).max(0.0);
    let x0 = fx.floor() as usize % w;
    let x1 = (x0 + 1) % w;
    let y0 = (fy.floor() as usize).min(h - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = fx - fx.floor();
    let ty = fy - fy.floor();

    let p00 = img.pixels[y0 * w + x0];
    let p10 = img.pixels[y0 * w + x1];
    let p01 = img.pixels[y1 * w + x0];
    let p11 = img.pixels[y1 * w + x1];

    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let top_r = lerp(p00.r, p10.r, tx);
    let top_g = lerp(p00.g, p10.g, tx);
    let top_b = lerp(p00.b, p10.b, tx);
    let top_a = lerp(p00.a, p10.a, tx);

    let bot_r = lerp(p01.r, p11.r, tx);
    let bot_g = lerp(p01.g, p11.g, tx);
    let bot_b = lerp(p01.b, p11.b, tx);
    let bot_a = lerp(p01.a, p11.a, tx);

    LinearPremul {
        r: lerp(top_r, bot_r, ty),
        g: lerp(top_g, bot_g, ty),
        b: lerp(top_b, bot_b, ty),
        a: lerp(top_a, bot_a, ty),
    }
}

#[inline]
fn sample_clamped(img: &LinearImage, fx: f32, fy: f32) -> LinearPremul {
    if img.width == 0 || img.height == 0 {
        return LinearPremul::TRANSPARENT;
    }
    let w = img.width as usize;
    let h = img.height as usize;
    let x = fx.clamp(0.0, (w - 1) as f32);
    let y = fy.clamp(0.0, (h - 1) as f32);
    let x0 = x.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y0 = y.floor() as usize;
    let y1 = (y0 + 1).min(h - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let p00 = img.pixels[y0 * w + x0];
    let p10 = img.pixels[y0 * w + x1];
    let p01 = img.pixels[y1 * w + x0];
    let p11 = img.pixels[y1 * w + x1];
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    LinearPremul {
        r: mix(mix(p00.r, p10.r, tx), mix(p01.r, p11.r, tx), ty),
        g: mix(mix(p00.g, p10.g, tx), mix(p01.g, p11.g, tx), ty),
        b: mix(mix(p00.b, p10.b, tx), mix(p01.b, p11.b, tx), ty),
        a: mix(mix(p00.a, p10.a, tx), mix(p01.a, p11.a, tx), ty),
    }
}

#[inline]
fn pixel_depth_proxy(p: LinearPremul) -> f32 {
    let luma = (0.2126 * p.r + 0.7152 * p.g + 0.0722 * p.b).clamp(0.0, 1.0);
    // 明るく前景に近い領域ほど正視差（飛び出し側）、暗部は奥側へマップ
    (luma * 2.0 - 1.0) * p.a.clamp(0.0, 1.0)
}

/// v10.0 #57: 360° 全天球投影（OverCapture / Tiny Planet / 水平ロック）＆ ステレオスコピック空間 3D パスを適用する
pub fn render_spatial_360_and_stereo_pass(
    src: &LinearImage,
    cfg: &SpatialVideoConfig,
    time_sec: f32,
) -> LinearImage {
    if !cfg.enabled || src.width == 0 || src.height == 0 {
        return src.clone();
    }
    let w = src.width;
    let h = src.height;
    let aspect = w as f32 / h.max(1) as f32;

    // ステップ 1: 360° Equirectangular 逆射影パス
    let projected = if cfg.projection_mode != Projection360Mode::Off {
        let (yaw, pitch, roll) = evaluate_horizon_locked_euler(cfg, time_sec);
        let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
        for y in 0..h {
            let ny = 1.0 - ((y as f32 + 0.5) / h as f32) * 2.0;
            let row = (y * w) as usize;
            for x in 0..w {
                let nx = ((x as f32 + 0.5) / w as f32) * 2.0 - 1.0;
                let (u, v) = project_screen_to_equirect_uv(
                    nx,
                    ny,
                    aspect,
                    cfg.projection_mode,
                    yaw,
                    pitch,
                    roll,
                    cfg.fov_deg,
                    cfg.tiny_planet_zoom,
                );
                out.pixels[row + x as usize] = sample_bilinear_wrap(src, u, v);
            }
        }
        out
    } else {
        src.clone()
    };

    // ステップ 2: ステレオスコピック空間 3D パス（SBS / TAB / Red-Cyan アナグリフ / Mono L・R）
    let mut final_img = if cfg.stereo_layout != StereoLayout::Off {
        let ipd = cfg.ipd_parallax_px.clamp(0.0, 64.0);
        let conv = cfg.convergence_offset_px.clamp(-48.0, 48.0);
        let sample_eye = |x: f32, y: f32, eye_sign: f32| -> LinearPremul {
            let center_px = sample_clamped(&projected, x, y);
            let depth = pixel_depth_proxy(center_px);
            let shift = eye_sign * (depth * ipd * 0.5 + conv * 0.5);
            sample_clamped(&projected, x + shift, y)
        };

        let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
        match cfg.stereo_layout {
            StereoLayout::Off => projected,
            StereoLayout::SideBySide => {
                let half_w = (w / 2).max(1);
                for y in 0..h {
                    let row = (y * w) as usize;
                    for x in 0..w {
                        let (src_x, eye) = if x < half_w {
                            (x as f32 * 2.0, -1.0)
                        } else {
                            ((x - half_w) as f32 * 2.0, 1.0)
                        };
                        out.pixels[row + x as usize] = sample_eye(src_x, y as f32, eye);
                    }
                }
                out
            }
            StereoLayout::TopAndBottom => {
                let half_h = (h / 2).max(1);
                for y in 0..h {
                    let row = (y * w) as usize;
                    let (src_y, eye) = if y < half_h {
                        (y as f32 * 2.0, -1.0)
                    } else {
                        ((y - half_h) as f32 * 2.0, 1.0)
                    };
                    for x in 0..w {
                        out.pixels[row + x as usize] = sample_eye(x as f32, src_y, eye);
                    }
                }
                out
            }
            StereoLayout::AnaglyphRedCyan => {
                for y in 0..h {
                    let row = (y * w) as usize;
                    for x in 0..w {
                        let left = sample_eye(x as f32, y as f32, -1.0);
                        let right = sample_eye(x as f32, y as f32, 1.0);
                        out.pixels[row + x as usize] = LinearPremul {
                            r: left.r,
                            g: right.g,
                            b: right.b,
                            a: left.a.max(right.a),
                        };
                    }
                }
                out
            }
            StereoLayout::MonoLeft => {
                for y in 0..h {
                    let row = (y * w) as usize;
                    for x in 0..w {
                        out.pixels[row + x as usize] = sample_eye(x as f32, y as f32, -1.0);
                    }
                }
                out
            }
            StereoLayout::MonoRight => {
                for y in 0..h {
                    let row = (y * w) as usize;
                    for x in 0..w {
                        out.pixels[row + x as usize] = sample_eye(x as f32, y as f32, 1.0);
                    }
                }
                out
            }
        }
    } else {
        projected
    };

    // ステップ 3: 水平線ガイド＆中央レティクル描画（任意）
    if cfg.show_horizon_guides {
        let mid_y = h / 2;
        let mid_x = w / 2;
        let guide = LinearPremul {
            r: 0.12,
            g: 0.92,
            b: 0.78,
            a: 0.82,
        };
        for x in (0..w).step_by(4) {
            for dx in 0..2 {
                let px = (x + dx).min(w - 1);
                final_img.pixels[(mid_y * w + px) as usize] = guide;
            }
        }
        let cross = (w.min(h) / 18).clamp(6, 28);
        for d in 0..=cross {
            let y0 = mid_y.saturating_sub(d);
            let y1 = (mid_y + d).min(h - 1);
            final_img.pixels[(y0 * w + mid_x) as usize] = guide;
            final_img.pixels[(y1 * w + mid_x) as usize] = guide;
        }
    }

    final_img
}

/// v10.0 #58: 決定論 2D Navier-Stokes 流体フィールドをリニア Premultiplied RGBA へレンダリングする
pub fn render_fluid_sim_item(
    cfg: &FluidSimConfig,
    time_sec: f32,
    out_w: u32,
    out_h: u32,
) -> LinearImage {
    let w = out_w.clamp(16, 2048);
    let h = out_h.clamp(16, 2048);
    let field = simulate_fluid_2d_at_time(cfg, time_sec);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);

    let p_col = [
        srgb_to_linear(cfg.primary_color.r),
        srgb_to_linear(cfg.primary_color.g),
        srgb_to_linear(cfg.primary_color.b),
    ];
    let s_col = [
        srgb_to_linear(cfg.secondary_color.r),
        srgb_to_linear(cfg.secondary_color.g),
        srgb_to_linear(cfg.secondary_color.b),
    ];

    let gw = field.width;
    let gh = field.height;

    let sample_grid = |grid: &[f32], u: f32, v: f32| -> f32 {
        let fx = (u * (gw - 1) as f32).clamp(0.0, (gw - 1) as f32);
        let fy = (v * (gh - 1) as f32).clamp(0.0, (gh - 1) as f32);
        let x0 = fx.floor() as usize;
        let x1 = (x0 + 1).min(gw - 1);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(gh - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let top = grid[y0 * gw + x0] * (1.0 - tx) + grid[y0 * gw + x1] * tx;
        let bot = grid[y1 * gw + x0] * (1.0 - tx) + grid[y1 * gw + x1] * tx;
        top * (1.0 - ty) + bot * ty
    };

    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let row = (y * w) as usize;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let dens = sample_grid(&field.density, u, v).max(0.0);
            let temp = sample_grid(&field.temperature, u, v).max(0.0);
            if dens < 0.008 && temp < 0.008 {
                continue;
            }

            let alpha = match cfg.preset {
                FluidPreset::BonfireFlame | FluidPreset::NeonPlasma => {
                    (dens * 0.72 + temp * 0.55).clamp(0.0, 1.0)
                }
                FluidPreset::SumiInk => (dens * 1.15).clamp(0.0, 0.98),
                FluidPreset::CinemaSmoke => (dens * 0.85).clamp(0.0, 0.94),
            };
            let hot = (temp * 0.75).clamp(0.0, 1.0);
            let mut r = s_col[0] * (1.0 - hot) + p_col[0] * hot;
            let mut g = s_col[1] * (1.0 - hot) + p_col[1] * hot;
            let mut b = s_col[2] * (1.0 - hot) + p_col[2] * hot;

            if matches!(cfg.preset, FluidPreset::BonfireFlame) && hot > 0.65 {
                let core = ((hot - 0.65) / 0.35).clamp(0.0, 1.0);
                r = (r + core * 0.45).min(1.0);
                g = (g + core * 0.38).min(1.0);
                b = (b + core * 0.25).min(1.0);
            }

            img.pixels[row + x as usize] = LinearPremul {
                r: r.clamp(0.0, 1.0) * alpha,
                g: g.clamp(0.0, 1.0) * alpha,
                b: b.clamp(0.0, 1.0) * alpha,
                a: alpha,
            };
        }
    }

    img
}

/// v10.0 #58: PBD クロスシミュレーションメッシュを Z バッファ付き法線シェーディングで描画する
pub fn render_cloth_sim_item(
    cfg: &ClothSimConfig,
    time_sec: f32,
    out_w: u32,
    out_h: u32,
) -> LinearImage {
    let w = out_w.clamp(16, 2048);
    let h = out_h.clamp(16, 2048);
    let state = simulate_cloth_pbd_at_time(cfg, time_sec);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let mut zbuf = vec![f32::INFINITY; (w as usize) * (h as usize)];

    let base_rgb = [
        srgb_to_linear(cfg.cloth_color.r),
        srgb_to_linear(cfg.cloth_color.g),
        srgb_to_linear(cfg.cloth_color.b),
    ];
    let acc_rgb = [
        srgb_to_linear(cfg.accent_color.r),
        srgb_to_linear(cfg.accent_color.g),
        srgb_to_linear(cfg.accent_color.b),
    ];

    let cam_dist = 2.65f32;
    let focal = (w.min(h) as f32) * 0.96;
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.46;
    let light_dir = [0.42f32, 0.58, 0.70];

    let proj: Vec<[f32; 3]> = state
        .positions
        .iter()
        .map(|p| {
            let vz = (cam_dist - p[2]).max(0.4);
            [cx + (p[0] / vz) * focal, cy - (p[1] / vz) * focal, vz]
        })
        .collect();

    let fw = w as i32;
    let fh = h as i32;

    let mut draw_tri = |i0: usize, i1: usize, i2: usize| {
        let p0 = proj[i0];
        let p1 = proj[i1];
        let p2 = proj[i2];
        let n0 = state.normals[i0];
        let n1 = state.normals[i1];
        let n2 = state.normals[i2];
        let uv0 = state.uvs[i0];
        let uv1 = state.uvs[i1];
        let uv2 = state.uvs[i2];

        let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
        if area.abs() < 1e-4 {
            return;
        }
        let inv_area = 1.0 / area;
        let min_x = (p0[0].min(p1[0]).min(p2[0]).floor() as i32).clamp(0, fw - 1);
        let max_x = (p0[0].max(p1[0]).max(p2[0]).ceil() as i32).clamp(0, fw - 1);
        let min_y = (p0[1].min(p1[1]).min(p2[1]).floor() as i32).clamp(0, fh - 1);
        let max_y = (p0[1].max(p1[1]).max(p2[1]).ceil() as i32).clamp(0, fh - 1);

        for py in min_y..=max_y {
            let fy = py as f32 + 0.5;
            let row = (py as u32 * w) as usize;
            for px in min_x..=max_x {
                let fx = px as f32 + 0.5;
                let w0 = ((p1[0] - fx) * (p2[1] - fy) - (p2[0] - fx) * (p1[1] - fy)) * inv_area;
                let w1 = ((p2[0] - fx) * (p0[1] - fy) - (p0[0] - fx) * (p2[1] - fy)) * inv_area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let depth = w0 * p0[2] + w1 * p1[2] + w2 * p2[2];
                let idx = row + px as usize;
                if depth >= zbuf[idx] {
                    continue;
                }
                zbuf[idx] = depth;

                let nx = w0 * n0[0] + w1 * n1[0] + w2 * n2[0];
                let ny = w0 * n0[1] + w1 * n1[1] + w2 * n2[1];
                let nz = w0 * n0[2] + w1 * n1[2] + w2 * n2[2];
                let nl = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-5);
                let mut n = [nx / nl, ny / nl, nz / nl];
                if n[2] < 0.0 {
                    n = [-n[0], -n[1], -n[2]];
                }

                let u = w0 * uv0[0] + w1 * uv1[0] + w2 * uv2[0];
                let v = w0 * uv0[1] + w1 * uv1[1] + w2 * uv2[1];

                // 縁取り・ストライプ織りアクセント
                let border = u < 0.06 || u > 0.94 || v < 0.07 || v > 0.93;
                let stripe =
                    ((v - 0.5).abs() < 0.07) || ((u * 12.0).sin().abs() > 0.92 && v > 0.78);
                let col = if border || stripe { acc_rgb } else { base_rgb };

                let ndotl = (n[0] * light_dir[0] + n[1] * light_dir[1] + n[2] * light_dir[2])
                    .clamp(0.0, 1.0);
                // シルク風リムハイライト（布の折り目に柔らかな光沢）
                let sheen = (1.0 - n[2].abs()).powi(2) * 0.24;
                let shade = 0.26 + ndotl * 0.78;
                let mut r = col[0] * shade + sheen;
                let mut g = col[1] * shade + sheen;
                let mut b = col[2] * shade + sheen;

                if cfg.show_wireframe && (w0 < 0.05 || w1 < 0.05 || w2 < 0.05) {
                    r = (r + 0.5).min(1.0);
                    g = (g + 0.85).min(1.0);
                    b = (b + 0.95).min(1.0);
                }

                let a = cfg.cloth_color.a.clamp(0.0, 1.0);
                img.pixels[idx] = LinearPremul {
                    r: r.clamp(0.0, 1.0) * a,
                    g: g.clamp(0.0, 1.0) * a,
                    b: b.clamp(0.0, 1.0) * a,
                    a,
                };
            }
        }
    };

    for r in 0..(state.rows - 1) {
        for c in 0..(state.cols - 1) {
            let a = r * state.cols + c;
            let b = a + 1;
            let d = a + state.cols;
            let e = d + 1;
            draw_tri(a, b, e);
            draw_tri(a, e, d);
        }
    }

    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::{ClothPreset, Spatial360Preset};

    #[test]
    fn spatial_360_overcapture_tiny_planet_and_anaglyph_render_deterministically() {
        let mut equirect = LinearImage::new(96, 48, LinearPremul::TRANSPARENT);
        for y in 0..48 {
            for x in 0..96 {
                equirect.pixels[(y * 96 + x) as usize] = LinearPremul {
                    r: x as f32 / 96.0,
                    g: y as f32 / 48.0,
                    b: 0.45,
                    a: 1.0,
                };
            }
        }

        for preset in Spatial360Preset::ALL {
            let cfg = SpatialVideoConfig::from_preset(*preset);
            let a = render_spatial_360_and_stereo_pass(&equirect, &cfg, 0.75);
            let b = render_spatial_360_and_stereo_pass(&equirect, &cfg, 0.75);
            assert_eq!(a.pixels, b.pixels);
            assert!(a.pixels.iter().any(|p| p.a > 0.5));
        }
    }

    #[test]
    fn fluid_and_cloth_renderers_produce_non_empty_deterministic_images() {
        for &fp in FluidPreset::ALL {
            let fcfg = FluidSimConfig::from_preset(fp);
            let i1 = render_fluid_sim_item(&fcfg, 1.0, 96, 96);
            let i2 = render_fluid_sim_item(&fcfg, 1.0, 96, 96);
            assert_eq!(i1.pixels, i2.pixels);
            assert!(i1.pixels.iter().filter(|p| p.a > 0.1).count() > 80);
        }

        for &cp in ClothPreset::ALL {
            let ccfg = ClothSimConfig::from_preset(cp);
            let c1 = render_cloth_sim_item(&ccfg, 1.0, 96, 72);
            let c2 = render_cloth_sim_item(&ccfg, 1.0, 96, 72);
            assert_eq!(c1.pixels, c2.pixels);
            assert!(c1.pixels.iter().filter(|p| p.a > 0.5).count() > 200);
        }
    }
}
