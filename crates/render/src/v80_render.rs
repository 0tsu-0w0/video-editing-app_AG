//! v8.0 #49〜#52 レンダリング＆コンピュータビジョン実装:
//! - #49: 4 点平面ホモグラフィトラッカー (`track_planar_quad_in_scene`, RANSAC) ＆
//!   Sobel エッジ吸着キーフレームロトブラシ (`snap_roto_vertices_to_edges`, `propagate_roto_brush_in_scene`)
//! - #50: SVG `<path>` 等弧長パスモーフィング描画 (`render_svg_morph`)・
//!   Text-on-Path 曲線接線追従タイポグラフィ (`render_text_on_path`)・
//!   双三次グリッドワープ＆輝度ディスプレイスメント (`apply_warp_and_displacement`)
//! - #51: シネマ光学 VFX (`apply_cinema_optical_vfx`: 擬似深度ボケ Rack Focus・ゴッドレイ・Brown-Conrady レンズ歪み＆物理色収差)
//! - #52: ADR 3-2-1 映像カウントダウンストリーマ描画 (`draw_adr_countdown_streamer`)
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロ。

use crate::color::{LinearImage, LinearPremul};
use crate::graph::RenderGraph;
use crate::text::TextRenderer;
use std::f32::consts::{PI, TAU};
use ve_core::{
    AdrCountdownState, ColorPalette, MaskVertex, OpticalVfxConfig, PlanarTrackConfig,
    PlanarTrackFrame, PlanarTrackResult, Project, PseudoDepthMode, RationalTime, RgbaColor,
    RotoKeyframe, SceneId, SvgMorphConfig, TextAnimator, TextOnPathConfig, TextStyle,
    VectorFillMode, WarpDisplaceConfig, WarpPreset, flatten_bezier_vertices,
    morph_paths_equal_arclength, parse_svg_path_to_vertices, sample_polyline_at_fraction,
};

// ============================================================================
// 共通サンプリング / 輝度ユーティリティ
// ============================================================================

#[inline]
fn pixel_luma(p: LinearPremul) -> f32 {
    0.2126 * p.r + 0.7152 * p.g + 0.0722 * p.b
}

#[inline]
fn sample_bilinear(img: &LinearImage, fx: f32, fy: f32) -> LinearPremul {
    if img.width == 0 || img.height == 0 {
        return LinearPremul::TRANSPARENT;
    }
    let max_x = (img.width - 1) as f32;
    let max_y = (img.height - 1) as f32;
    let cx = fx.clamp(0.0, max_x);
    let cy = fy.clamp(0.0, max_y);
    let x0 = cx.floor() as u32;
    let y0 = cy.floor() as u32;
    let x1 = (x0 + 1).min(img.width - 1);
    let y1 = (y0 + 1).min(img.height - 1);
    let tx = cx - x0 as f32;
    let ty = cy - y0 as f32;

    let w = img.width as usize;
    let p00 = img.pixels[y0 as usize * w + x0 as usize];
    let p10 = img.pixels[y0 as usize * w + x1 as usize];
    let p01 = img.pixels[y1 as usize * w + x0 as usize];
    let p11 = img.pixels[y1 as usize * w + x1 as usize];

    let w00 = (1.0 - tx) * (1.0 - ty);
    let w10 = tx * (1.0 - ty);
    let w01 = (1.0 - tx) * ty;
    let w11 = tx * ty;
    LinearPremul {
        r: p00.r * w00 + p10.r * w10 + p01.r * w01 + p11.r * w11,
        g: p00.g * w00 + p10.g * w10 + p01.g * w01 + p11.g * w11,
        b: p00.b * w00 + p10.b * w10 + p01.b * w01 + p11.b * w11,
        a: p00.a * w00 + p10.a * w10 + p01.a * w01 + p11.a * w11,
    }
}

fn image_luma_plane(img: &LinearImage) -> Vec<f32> {
    img.pixels.iter().map(|&p| pixel_luma(p)).collect()
}

// ============================================================================
// #49: 4 点平面ホモグラフィトラッカー（ZNCC + RANSAC）＆ Sobel ロトブラシ
// ============================================================================

/// 任意 4 対応点 `src -> dst` の射影変換 3×3 ホモグラフィ行列をガウス・ジョルダン消去法で厳密に解く
pub fn solve_quad_to_quad_homography(src: [[f64; 2]; 4], dst: [[f64; 2]; 4]) -> Option<[f64; 9]> {
    let mut a = [[0.0_f64; 9]; 8];
    for (i, (&[x, y], &[u, v])) in src.iter().zip(dst.iter()).enumerate() {
        a[i * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[i * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for col in 0..8 {
        let mut pivot = col;
        let mut max_abs = a[col][col].abs();
        for (r, row) in a.iter().enumerate().skip(col + 1) {
            if row[col].abs() > max_abs {
                max_abs = row[col].abs();
                pivot = r;
            }
        }
        if max_abs < 1e-10 {
            return None;
        }
        a.swap(col, pivot);
        let div = a[col][col];
        for v in a[col].iter_mut().skip(col) {
            *v /= div;
        }
        let piv_row = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            if r == col {
                continue;
            }
            let factor = row[col];
            if factor.abs() < 1e-15 {
                continue;
            }
            for (cell, &pv) in row.iter_mut().zip(piv_row.iter()).skip(col) {
                *cell -= factor * pv;
            }
        }
    }
    Some([
        a[0][8], a[1][8], a[2][8], a[3][8], a[4][8], a[5][8], a[6][8], a[7][8], 1.0,
    ])
}

#[inline]
pub fn project_homography(h: &[f64; 9], p: [f64; 2]) -> Option<[f64; 2]> {
    let denom = h[6] * p[0] + h[7] * p[1] + h[8];
    if denom.abs() < 1e-9 {
        return None;
    }
    Some([
        (h[0] * p[0] + h[1] * p[1] + h[2]) / denom,
        (h[3] * p[0] + h[4] * p[1] + h[5]) / denom,
    ])
}

fn multiply_3x3(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut c = [0.0_f64; 9];
    for r in 0..3 {
        for col in 0..3 {
            c[r * 3 + col] =
                a[r * 3] * b[col] + a[r * 3 + 1] * b[3 + col] + a[r * 3 + 2] * b[6 + col];
        }
    }
    if c[8].abs() > 1e-9 {
        let inv = 1.0 / c[8];
        for v in &mut c {
            *v *= inv;
        }
    }
    c
}

/// 決定論 RANSAC により対応点列 `matches` から外れ値に頑健な 3×3 ホモグラフィを推定する
pub fn estimate_homography_ransac(
    matches: &[([f64; 2], [f64; 2])],
    iterations: u32,
    inlier_thresh_px: f64,
    seed: u64,
) -> Option<([f64; 9], usize)> {
    let n = matches.len();
    if n < 4 {
        return None;
    }
    let thresh_sq = inlier_thresh_px.max(0.5) * inlier_thresh_px.max(0.5);
    let mut rng = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut next_usize = |bound: usize| -> usize {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((rng >> 33) as usize) % bound
    };

    let mut best_h: Option<[f64; 9]> = None;
    let mut best_inliers = 0usize;
    let mut best_err_sum = f64::INFINITY;
    let iters = iterations.clamp(8, 256);

    for _ in 0..iters {
        let mut idx = [0usize; 4];
        let mut ok = true;
        for k in 0..4 {
            let mut tries = 0;
            loop {
                let c = next_usize(n);
                if !idx[..k].contains(&c) {
                    idx[k] = c;
                    break;
                }
                tries += 1;
                if tries > 16 {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
        }
        if !ok {
            continue;
        }
        let src = [
            matches[idx[0]].0,
            matches[idx[1]].0,
            matches[idx[2]].0,
            matches[idx[3]].0,
        ];
        let dst = [
            matches[idx[0]].1,
            matches[idx[1]].1,
            matches[idx[2]].1,
            matches[idx[3]].1,
        ];
        let Some(h) = solve_quad_to_quad_homography(src, dst) else {
            continue;
        };
        let mut inliers = 0usize;
        let mut err_sum = 0.0_f64;
        for &(p, q) in matches {
            if let Some(proj) = project_homography(&h, p) {
                let d2 = (proj[0] - q[0]).powi(2) + (proj[1] - q[1]).powi(2);
                if d2 <= thresh_sq {
                    inliers += 1;
                    err_sum += d2;
                }
            }
        }
        if inliers > best_inliers || (inliers == best_inliers && err_sum < best_err_sum) {
            best_inliers = inliers;
            best_err_sum = err_sum;
            best_h = Some(h);
        }
    }

    best_h.map(|h| (h, best_inliers))
}

/// ZNCC 相関＋サブピクセル放物線補間で 1 パッチのフレーム間移動量を求める
#[allow(clippy::too_many_arguments)]
fn track_patch_zncc(
    prev: &[f32],
    curr: &[f32],
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    patch_r: i32,
    search_r: i32,
) -> Option<([f64; 2], f32)> {
    let px = cx.round() as i32;
    let py = cy.round() as i32;
    let pr = patch_r.clamp(3, 14);
    let sr = search_r.clamp(4, 32);
    if px - pr < 1 || px + pr >= w as i32 - 1 || py - pr < 1 || py + pr >= h as i32 - 1 {
        return None;
    }

    // テンプレート統計量（平均・標準偏差）
    let side = (2 * pr + 1) as usize;
    let count = (side * side) as f32;
    let mut tmpl = Vec::with_capacity(side * side);
    let mut sum_t = 0.0_f32;
    for dy in -pr..=pr {
        let row = (py + dy) as usize * w;
        for dx in -pr..=pr {
            let v = prev[row + (px + dx) as usize];
            tmpl.push(v);
            sum_t += v;
        }
    }
    let mean_t = sum_t / count;
    let mut var_t = 0.0_f32;
    for v in &mut tmpl {
        *v -= mean_t;
        var_t += *v * *v;
    }
    if var_t < 1e-4 {
        // テクスチャ無しのフラット領域は追跡不可
        return None;
    }
    let inv_std_t = 1.0 / var_t.sqrt();

    let eval_zncc = |tx: i32, ty: i32| -> f32 {
        if tx - pr < 0 || tx + pr >= w as i32 || ty - pr < 0 || ty + pr >= h as i32 {
            return -1.0;
        }
        let mut sum_c = 0.0_f32;
        let mut sum_c2 = 0.0_f32;
        let mut dot = 0.0_f32;
        let mut ti = 0usize;
        for dy in -pr..=pr {
            let row = (ty + dy) as usize * w;
            for dx in -pr..=pr {
                let c = curr[row + (tx + dx) as usize];
                sum_c += c;
                sum_c2 += c * c;
                dot += tmpl[ti] * c;
                ti += 1;
            }
        }
        let var_c = (sum_c2 - (sum_c * sum_c) / count).max(0.0);
        if var_c < 1e-4 {
            return 0.0;
        }
        (dot * inv_std_t / var_c.sqrt()).clamp(-1.0, 1.0)
    };

    // 粗探索 (stride 2) -> 精密 3x3 探索
    let mut best_dx = 0i32;
    let mut best_dy = 0i32;
    let mut best_corr = -1.0_f32;
    let step = if sr >= 10 { 2 } else { 1 };
    let mut dy = -sr;
    while dy <= sr {
        let mut dx = -sr;
        while dx <= sr {
            let corr = eval_zncc(px + dx, py + dy);
            if corr > best_corr {
                best_corr = corr;
                best_dx = dx;
                best_dy = dy;
            }
            dx += step;
        }
        dy += step;
    }
    if step > 1 {
        let (cdx, cdy) = (best_dx, best_dy);
        for oy in -2..=2 {
            for ox in -2..=2 {
                let dx = (cdx + ox).clamp(-sr, sr);
                let dy = (cdy + oy).clamp(-sr, sr);
                let corr = eval_zncc(px + dx, py + dy);
                if corr > best_corr {
                    best_corr = corr;
                    best_dx = dx;
                    best_dy = dy;
                }
            }
        }
    }

    // 3 点放物線サブピクセル補間
    let c_l = eval_zncc(px + best_dx - 1, py + best_dy);
    let c_r = eval_zncc(px + best_dx + 1, py + best_dy);
    let c_u = eval_zncc(px + best_dx, py + best_dy - 1);
    let c_d = eval_zncc(px + best_dx, py + best_dy + 1);
    let denom_x = 2.0 * (c_l - 2.0 * best_corr + c_r);
    let sub_x = if denom_x.abs() > 1e-4 {
        ((c_l - c_r) / denom_x).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let denom_y = 2.0 * (c_u - 2.0 * best_corr + c_d);
    let sub_y = if denom_y.abs() > 1e-4 {
        ((c_u - c_d) / denom_y).clamp(-0.5, 0.5)
    } else {
        0.0
    };

    Some((
        [
            f64::from(cx) + f64::from(best_dx) + f64::from(sub_x),
            f64::from(cy) + f64::from(best_dy) + f64::from(sub_y),
        ],
        best_corr,
    ))
}

fn bilinear_quad_point(q: &[[f32; 2]; 4], u: f32, v: f32) -> [f32; 2] {
    let top_x = q[0][0] + (q[1][0] - q[0][0]) * u;
    let top_y = q[0][1] + (q[1][1] - q[0][1]) * u;
    let bot_x = q[3][0] + (q[2][0] - q[3][0]) * u;
    let bot_y = q[3][1] + (q[2][1] - q[3][1]) * u;
    [top_x + (bot_x - top_x) * v, top_y + (bot_y - top_y) * v]
}

/// シーン上の指定時間区間について 4 点平面ホモグラフィ追跡（ZNCC マルチパッチ＋RANSAC）を実行する
pub fn track_planar_quad_in_scene(
    rg: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    cfg: &PlanarTrackConfig,
) -> PlanarTrackResult {
    let Some(scene) = project.scene(scene_id) else {
        return PlanarTrackResult::default();
    };
    let fps = scene.fps;
    let step = cfg.step_frames.max(1) as i64;
    let start_f = cfg.start_time.to_frame_floor(fps).max(0);
    let end_f = cfg.end_time.to_frame_floor(fps).max(start_f);
    let max_samples = 240usize;

    // 追跡解像度（最大辺 320px で高速・高精度バランス）
    let aspect = (scene.width.max(1) as f32) / (scene.height.max(1) as f32);
    let (tw, th) = if aspect >= 1.0 {
        (320u32, (320.0 / aspect).round().max(32.0) as u32)
    } else {
        ((320.0 * aspect).round().max(32.0) as u32, 320u32)
    };
    let fw = tw as f64;
    let fh = th as f64;

    let mut cur_quad = cfg.initial_quad;
    let mut cum_h = [1.0_f64, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let mut frames = Vec::new();
    let mut prev_luma: Option<Vec<f32>> = None;

    let mut f = start_f;
    while f <= end_f && frames.len() < max_samples {
        let t = RationalTime::from_frame(f, fps);
        let Some(img) = rg.render_project_scene_scaled(project, scene_id, t, tw, th, false) else {
            break;
        };
        let cur_luma = image_luma_plane(&img);

        if let Some(ref prev) = prev_luma {
            // 平面内部の 4×4 グリッドパッチを ZNCC 追跡
            let mut matches: Vec<([f64; 2], [f64; 2])> = Vec::with_capacity(16);
            let mut corr_sum = 0.0_f32;
            for gy in 0..4 {
                let v = (gy as f32 + 0.5) / 4.0;
                for gx in 0..4 {
                    let u = (gx as f32 + 0.5) / 4.0;
                    let norm_p = bilinear_quad_point(&cur_quad, u, v);
                    let px = norm_p[0] * tw as f32;
                    let py = norm_p[1] * th as f32;
                    if let Some((dst_p, corr)) = track_patch_zncc(
                        prev,
                        &cur_luma,
                        tw as usize,
                        th as usize,
                        px,
                        py,
                        cfg.patch_radius as i32,
                        cfg.search_radius as i32,
                    ) && corr >= 0.30
                    {
                        matches.push(([f64::from(px), f64::from(py)], dst_p));
                        corr_sum += corr;
                    }
                }
            }

            let mut inliers = 0usize;
            let conf = if !matches.is_empty() {
                (corr_sum / matches.len() as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };

            if let Some((h_step, inl)) = estimate_homography_ransac(
                &matches,
                cfg.ransac_iterations,
                f64::from(cfg.ransac_inlier_px),
                (f as u64).wrapping_add(0x80_49),
            ) {
                inliers = inl;
                let mut next_quad = cur_quad;
                let mut valid_quad = true;
                for (i, c) in cur_quad.iter().enumerate() {
                    let p_px = [f64::from(c[0]) * fw, f64::from(c[1]) * fh];
                    if let Some(q_px) = project_homography(&h_step, p_px) {
                        let nx = (q_px[0] / fw) as f32;
                        let ny = (q_px[1] / fh) as f32;
                        if nx.is_finite()
                            && ny.is_finite()
                            && (-0.5..=1.5).contains(&nx)
                            && (-0.5..=1.5).contains(&ny)
                        {
                            next_quad[i] = [nx, ny];
                        } else {
                            valid_quad = false;
                            break;
                        }
                    } else {
                        valid_quad = false;
                        break;
                    }
                }
                if valid_quad {
                    cur_quad = next_quad;
                    cum_h = multiply_3x3(&h_step, &cum_h);
                }
            } else if !matches.is_empty() {
                // 4 点未満のフォールバック: 平均並進シフト
                let dx = matches.iter().map(|(a, b)| b[0] - a[0]).sum::<f64>()
                    / matches.len() as f64
                    / fw;
                let dy = matches.iter().map(|(a, b)| b[1] - a[1]).sum::<f64>()
                    / matches.len() as f64
                    / fh;
                for c in &mut cur_quad {
                    c[0] = (c[0] + dx as f32).clamp(-0.5, 1.5);
                    c[1] = (c[1] + dy as f32).clamp(-0.5, 1.5);
                }
                inliers = matches.len();
            }

            frames.push(PlanarTrackFrame {
                time: t,
                quad: cur_quad,
                homography: cum_h,
                inlier_count: inliers,
                confidence: conf,
            });
        } else {
            frames.push(PlanarTrackFrame {
                time: t,
                quad: cur_quad,
                homography: cum_h,
                inlier_count: 16,
                confidence: 1.0,
            });
        }

        prev_luma = Some(cur_luma);
        if f == end_f {
            break;
        }
        f = (f + step).min(end_f);
    }

    let n = frames.len().max(1) as f32;
    let mean_confidence = frames.iter().map(|fr| fr.confidence).sum::<f32>() / n;
    let mean_inliers = frames.iter().map(|fr| fr.inlier_count as f32).sum::<f32>() / n;
    PlanarTrackResult {
        frames,
        mean_confidence,
        mean_inliers,
    }
}

/// Sobel 3×3 輝度勾配強度マップを算出する（ロトブラシエッジ吸着＆顕著性深度用）
pub fn compute_sobel_edge_magnitude(img: &LinearImage) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    if w < 3 || h < 3 {
        return vec![0.0; w * h];
    }
    let luma = image_luma_plane(img);
    let mut mag = vec![0.0_f32; w * h];
    for y in 1..(h - 1) {
        let r0 = (y - 1) * w;
        let r1 = y * w;
        let r2 = (y + 1) * w;
        for x in 1..(w - 1) {
            let gx = -luma[r0 + x - 1] + luma[r0 + x + 1] - 2.0 * luma[r1 + x - 1]
                + 2.0 * luma[r1 + x + 1]
                - luma[r2 + x - 1]
                + luma[r2 + x + 1];
            let gy = -luma[r0 + x - 1] - 2.0 * luma[r0 + x] - luma[r0 + x + 1]
                + luma[r2 + x - 1]
                + 2.0 * luma[r2 + x]
                + luma[r2 + x + 1];
            mag[r1 + x] = gx.hypot(gy);
        }
    }
    mag
}

/// ロトブラシ頂点列を画像の強い Sobel 輪郭エッジへ自動吸着させ、滑らかな 3 次ベジェハンドルを再計算する
pub fn snap_roto_vertices_to_edges(
    img: &LinearImage,
    vertices: &[MaskVertex],
    search_radius_px: u32,
    edge_weight: f32,
) -> Vec<MaskVertex> {
    if vertices.is_empty() || img.width < 4 || img.height < 4 {
        return vertices.to_vec();
    }
    let w = img.width as i32;
    let h = img.height as i32;
    let mag = compute_sobel_edge_magnitude(img);
    let max_mag = mag.iter().copied().fold(0.0_f32, f32::max).max(1e-4);
    let r = (search_radius_px as i32).clamp(1, 32);
    let ew = edge_weight.clamp(0.1, 0.98);

    let mut snapped_pts: Vec<[f32; 2]> = Vec::with_capacity(vertices.len());
    for v in vertices {
        let bx = (v.x * (w - 1) as f32).round() as i32;
        let by = (v.y * (h - 1) as f32).round() as i32;
        let mut best_x = bx.clamp(1, w - 2);
        let mut best_y = by.clamp(1, h - 2);
        let mut best_score = f32::NEG_INFINITY;

        for dy in -r..=r {
            let ny = by + dy;
            if ny < 1 || ny >= h - 1 {
                continue;
            }
            let row = ny as usize * w as usize;
            for dx in -r..=r {
                let nx = bx + dx;
                if nx < 1 || nx >= w - 1 {
                    continue;
                }
                let dist_norm = ((dx * dx + dy * dy) as f32).sqrt() / (r as f32 + 1e-4);
                if dist_norm > 1.05 {
                    continue;
                }
                let edge_norm = (mag[row + nx as usize] / max_mag).clamp(0.0, 1.0);
                let score = ew * edge_norm - (1.0 - ew) * dist_norm * dist_norm;
                if score > best_score {
                    best_score = score;
                    best_x = nx;
                    best_y = ny;
                }
            }
        }
        snapped_pts.push([
            (best_x as f32 / (w - 1) as f32).clamp(0.005, 0.995),
            (best_y as f32 / (h - 1) as f32).clamp(0.005, 0.995),
        ]);
    }

    // 閉曲線 Catmull-Rom 接線から滑らかな 3 次ベジェハンドルを自動生成
    let n = snapped_pts.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = snapped_pts[(i + n - 1) % n];
        let curr = snapped_pts[i];
        let next = snapped_pts[(i + 1) % n];
        let tx = (next[0] - prev[0]) / 6.0;
        let ty = (next[1] - prev[1]) / 6.0;
        out.push(MaskVertex {
            x: curr[0],
            y: curr[1],
            in_handle: [-tx, -ty],
            out_handle: [tx, ty],
        });
    }
    out
}

/// 初期ロト輪郭をオプティカルフロー（ZNCC 局所追跡）＋Sobel エッジ吸着で時間軸へ自動伝播し
/// キーフレームロト列を生成する
#[allow(clippy::too_many_arguments)]
pub fn propagate_roto_brush_in_scene(
    rg: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    item_start: RationalTime,
    base_vertices: &[MaskVertex],
    start_time: RationalTime,
    end_time: RationalTime,
    step_frames: u32,
    snap_radius_px: u32,
    edge_weight: f32,
    feather_px: f32,
) -> Vec<RotoKeyframe> {
    let Some(scene) = project.scene(scene_id) else {
        return Vec::new();
    };
    let fps = scene.fps;
    let step = step_frames.max(1) as i64;
    let start_f = start_time.to_frame_floor(fps).max(0);
    let end_f = end_time.to_frame_floor(fps).max(start_f);
    let aspect = (scene.width.max(1) as f32) / (scene.height.max(1) as f32);
    let (tw, th) = if aspect >= 1.0 {
        (256u32, (256.0 / aspect).round().max(32.0) as u32)
    } else {
        ((256.0 * aspect).round().max(32.0) as u32, 256u32)
    };

    let mut cur_verts = base_vertices.to_vec();
    let mut prev_luma: Option<Vec<f32>> = None;
    let mut out = Vec::new();
    let mut f = start_f;

    while f <= end_f && out.len() < 120 {
        let t = RationalTime::from_frame(f, fps);
        let Some(img) = rg.render_project_scene_scaled(project, scene_id, t, tw, th, false) else {
            break;
        };
        let cur_luma = image_luma_plane(&img);

        if let Some(ref prev) = prev_luma {
            for v in &mut cur_verts {
                let px = v.x * (tw - 1) as f32;
                let py = v.y * (th - 1) as f32;
                if let Some((dst, corr)) =
                    track_patch_zncc(prev, &cur_luma, tw as usize, th as usize, px, py, 6, 14)
                    && corr >= 0.25
                {
                    v.x = (dst[0] as f32 / (tw - 1) as f32).clamp(0.01, 0.99);
                    v.y = (dst[1] as f32 / (th - 1) as f32).clamp(0.01, 0.99);
                }
            }
        }

        cur_verts = snap_roto_vertices_to_edges(&img, &cur_verts, snap_radius_px, edge_weight);
        out.push(RotoKeyframe {
            time: (t - item_start).max(RationalTime::ZERO),
            vertices: cur_verts.clone(),
            feather_px: feather_px.max(0.0),
            expansion_px: 0.0,
        });

        prev_luma = Some(cur_luma);
        if f == end_f {
            break;
        }
        f = (f + step).min(end_f);
    }
    out
}

// ============================================================================
// #50: SVG `<path>` 等弧長パスモーフィング・Text-on-Path・グリッドワープ＆変位
// ============================================================================

/// `SvgMorphConfig` から等弧長モーフィング済みベクター画像を決定論的に描画する
pub fn render_svg_morph(
    cfg: &SvgMorphConfig,
    morph_override: Option<f32>,
    local_sec: f32,
    out_w: u32,
    out_h: u32,
) -> LinearImage {
    let w = out_w.max(8);
    let h = out_h.max(8);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);

    let Ok((va, closed_a)) = parse_svg_path_to_vertices(&cfg.path_a_d) else {
        return img;
    };
    let effective_t = if let Some(ov) = morph_override {
        ov.clamp(0.0, 1.0)
    } else if cfg.auto_morph_hz > 1e-4 {
        let phase = (local_sec * cfg.auto_morph_hz).rem_euclid(1.0);
        // 滑らかなピンポン往復 (0 -> 1 -> 0)
        0.5 - 0.5 * (phase * TAU).cos()
    } else {
        cfg.morph_progress.clamp(0.0, 1.0)
    };

    let (poly_norm, is_closed) = if !cfg.path_b_d.trim().is_empty()
        && let Ok((vb, closed_b)) = parse_svg_path_to_vertices(&cfg.path_b_d)
    {
        (
            morph_paths_equal_arclength(
                &va,
                closed_a,
                &vb,
                closed_b,
                effective_t,
                cfg.sample_count,
            ),
            closed_a || closed_b,
        )
    } else {
        (flatten_bezier_vertices(&va, closed_a, 12), closed_a)
    };

    if poly_norm.len() < 2 {
        return img;
    }

    let pts: Vec<[f32; 2]> = poly_norm
        .iter()
        .map(|p| [p[0] * (w as f32 - 1.0), p[1] * (h as f32 - 1.0)])
        .collect();

    // 1. 閉パスの塗りつぶし（Even-Odd スキャンライン）
    if is_closed && !matches!(cfg.fill, VectorFillMode::None) {
        let (grad_dx, grad_dy) = match cfg.fill {
            VectorFillMode::LinearGradient { angle_deg, .. } => {
                let rad = angle_deg.to_radians();
                (rad.cos(), rad.sin())
            }
            _ => (1.0, 0.0),
        };
        let mut xs = Vec::with_capacity(32);
        for y in 0..h {
            let fy = y as f32 + 0.5;
            xs.clear();
            for seg in pts.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                if (a[1] <= fy && b[1] > fy) || (b[1] <= fy && a[1] > fy) {
                    let t = (fy - a[1]) / (b[1] - a[1]);
                    xs.push(a[0] + t * (b[0] - a[0]));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            let row = (y * w) as usize;
            for pair in xs.as_chunks::<2>().0 {
                let x0 = (pair[0].floor() as i32).clamp(0, w as i32 - 1) as u32;
                let x1 = (pair[1].ceil() as i32).clamp(0, w as i32 - 1) as u32;
                for x in x0..=x1 {
                    let col = match cfg.fill {
                        VectorFillMode::None => RgbaColor::TRANSPARENT,
                        VectorFillMode::Solid(c) => c,
                        VectorFillMode::LinearGradient {
                            start_color,
                            end_color,
                            ..
                        } => {
                            let nx = (x as f32 / w as f32) - 0.5;
                            let ny = (y as f32 / h as f32) - 0.5;
                            let t = (nx * grad_dx + ny * grad_dy + 0.5).clamp(0.0, 1.0);
                            RgbaColor {
                                r: start_color.r + (end_color.r - start_color.r) * t,
                                g: start_color.g + (end_color.g - start_color.g) * t,
                                b: start_color.b + (end_color.b - start_color.b) * t,
                                a: start_color.a + (end_color.a - start_color.a) * t,
                            }
                        }
                    };
                    img.pixels[row + x as usize] = LinearPremul::from_srgb_straight(col);
                }
            }
        }
    }

    // 2. トリムパス対応アンチエイリアスストローク描画
    if cfg.stroke_width > 0.25 && cfg.stroke_color.a > 0.001 {
        let mut cum = Vec::with_capacity(pts.len());
        cum.push(0.0_f32);
        let mut total_len = 0.0_f32;
        for seg in pts.windows(2) {
            total_len += (seg[1][0] - seg[0][0]).hypot(seg[1][1] - seg[0][1]);
            cum.push(total_len);
        }
        let t_lo = cfg.trim_start.min(cfg.trim_end).clamp(0.0, 1.0) * total_len;
        let t_hi = cfg.trim_start.max(cfg.trim_end).clamp(0.0, 1.0) * total_len;
        let half_w = (cfg.stroke_width * 0.5).max(0.6);
        let sc = LinearPremul::from_srgb_straight(cfg.stroke_color);

        for (i, seg) in pts.windows(2).enumerate() {
            let d0 = cum[i];
            let d1 = cum[i + 1];
            if d1 < t_lo || d0 > t_hi {
                continue;
            }
            draw_aa_capsule(&mut img, seg[0], seg[1], half_w, sc);
        }
    }

    img
}

fn draw_aa_capsule(
    img: &mut LinearImage,
    a: [f32; 2],
    b: [f32; 2],
    radius: f32,
    col: LinearPremul,
) {
    let min_x = ((a[0].min(b[0]) - radius - 1.5).floor() as i32).clamp(0, img.width as i32 - 1);
    let max_x = ((a[0].max(b[0]) + radius + 1.5).ceil() as i32).clamp(0, img.width as i32 - 1);
    let min_y = ((a[1].min(b[1]) - radius - 1.5).floor() as i32).clamp(0, img.height as i32 - 1);
    let max_y = ((a[1].max(b[1]) + radius + 1.5).ceil() as i32).clamp(0, img.height as i32 - 1);

    let abx = b[0] - a[0];
    let aby = b[1] - a[1];
    let ab_len2 = (abx * abx + aby * aby).max(1e-6);
    let w = img.width as usize;

    for y in min_y..=max_y {
        let fy = y as f32 + 0.5;
        let row = y as usize * w;
        for x in min_x..=max_x {
            let fx = x as f32 + 0.5;
            let t = (((fx - a[0]) * abx + (fy - a[1]) * aby) / ab_len2).clamp(0.0, 1.0);
            let cx = a[0] + abx * t;
            let cy = a[1] + aby * t;
            let dist = (fx - cx).hypot(fy - cy);
            let cov = (radius + 0.75 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                let src = LinearPremul {
                    r: col.r * cov,
                    g: col.g * cov,
                    b: col.b * cov,
                    a: col.a * cov,
                };
                img.pixels[row + x as usize] = src.over(img.pixels[row + x as usize]);
            }
        }
    }
}

/// テキストを SVG `<path>` 曲線に沿って配置し、各文字を曲線接線角度へ回転追従させて描画する
#[allow(clippy::too_many_arguments)]
pub fn render_text_on_path(
    content: &str,
    font_family: &str,
    font_size: f32,
    color: RgbaColor,
    cfg: &TextOnPathConfig,
    text_style: &TextStyle,
    palette: &ColorPalette,
    text_renderer: &mut TextRenderer,
    canvas_w: u32,
    canvas_h: u32,
    local_sec: f32,
) -> LinearImage {
    let w = canvas_w.max(32);
    let h = canvas_h.max(32);
    let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);

    let Ok((verts, closed)) = parse_svg_path_to_vertices(&cfg.svg_path_d) else {
        return out;
    };
    let poly_norm = flatten_bezier_vertices(&verts, closed, 16);
    if poly_norm.len() < 2 {
        return out;
    }
    let poly_px: Vec<[f32; 2]> = poly_norm
        .iter()
        .map(|p| [p[0] * (w as f32 - 1.0), p[1] * (h as f32 - 1.0)])
        .collect();
    let total_len: f32 = poly_px
        .windows(2)
        .map(|seg| (seg[1][0] - seg[0][0]).hypot(seg[1][1] - seg[0][1]))
        .sum::<f32>()
        .max(1.0);

    // ガイド曲線表示（任意）
    if cfg.show_guide {
        let gcol = LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.25,
            g: 0.85,
            b: 1.0,
            a: 0.45,
        });
        for seg in poly_px.windows(2) {
            draw_aa_capsule(&mut out, seg[0], seg[1], 1.2, gcol);
        }
    }

    let chars: Vec<char> = content
        .chars()
        .filter(|c| *c != '\r' && *c != '\n')
        .collect();
    if chars.is_empty() {
        return out;
    }

    let no_anim = TextAnimator::default();
    let mut cursor_dist =
        ((cfg.path_offset + cfg.animate_speed * local_sec).rem_euclid(1.0)) * total_len;
    let glyph_box = (font_size * 2.5).round().clamp(32.0, 512.0) as u32;

    for ch in chars {
        if ch.is_whitespace() {
            cursor_dist += font_size * 0.45 + cfg.letter_spacing_px;
            continue;
        }
        let s = ch.to_string();
        let glyph = text_renderer.render_text_styled(
            &s,
            font_family,
            font_size,
            color,
            &no_anim,
            text_style,
            palette,
            local_sec,
            glyph_box,
            glyph_box,
        );
        // 描画済みピクセルの水平実測幅から送り量を求める
        let (g_min_x, g_max_x, g_min_y, g_max_y) = glyph_alpha_bounds(&glyph);
        let adv_w = if g_max_x >= g_min_x {
            (g_max_x - g_min_x + 1) as f32
        } else {
            font_size * 0.6
        };
        let center_dist = cursor_dist + adv_w * 0.5;
        let frac = if closed {
            (center_dist / total_len).rem_euclid(1.0)
        } else {
            center_dist / total_len
        };
        if !closed && !(0.0..=1.0).contains(&frac) {
            cursor_dist += adv_w + cfg.letter_spacing_px;
            continue;
        }

        let (pos, angle) = sample_polyline_at_fraction(&poly_px, frac);
        let rot = if cfg.follow_tangent { angle } else { 0.0 };
        let cos_a = rot.cos();
        let sin_a = rot.sin();
        // 曲線法線（上方向）オフセット
        let nx = -angle.sin();
        let ny = angle.cos();
        let dst_cx = pos[0] + nx * cfg.vertical_offset_px;
        let dst_cy = pos[1] + ny * cfg.vertical_offset_px;

        let src_cx = 0.5 * (g_min_x + g_max_x) as f32;
        let src_cy = 0.5 * (g_min_y + g_max_y) as f32;
        let half_diag =
            0.5 * ((g_max_x - g_min_x + 2) as f32).hypot((g_max_y - g_min_y + 2) as f32);

        let min_ox = ((dst_cx - half_diag - 1.0).floor() as i32).clamp(0, w as i32 - 1);
        let max_ox = ((dst_cx + half_diag + 1.0).ceil() as i32).clamp(0, w as i32 - 1);
        let min_oy = ((dst_cy - half_diag - 1.0).floor() as i32).clamp(0, h as i32 - 1);
        let max_oy = ((dst_cy + half_diag + 1.0).ceil() as i32).clamp(0, h as i32 - 1);

        for oy in min_oy..=max_oy {
            let dy = oy as f32 + 0.5 - dst_cy;
            let row = oy as usize * w as usize;
            for ox in min_ox..=max_ox {
                let dx = ox as f32 + 0.5 - dst_cx;
                // 逆回転サンプリング
                let sx = src_cx + dx * cos_a + dy * sin_a;
                let sy = src_cy - dx * sin_a + dy * cos_a;
                if sx >= 0.0
                    && sx < (glyph.width as f32 - 1.0)
                    && sy >= 0.0
                    && sy < (glyph.height as f32 - 1.0)
                {
                    let sp = sample_bilinear(&glyph, sx, sy);
                    if sp.a > 1e-4 {
                        out.pixels[row + ox as usize] = sp.over(out.pixels[row + ox as usize]);
                    }
                }
            }
        }

        cursor_dist += adv_w + cfg.letter_spacing_px;
    }

    out
}

fn glyph_alpha_bounds(img: &LinearImage) -> (u32, u32, u32, u32) {
    let mut min_x = img.width;
    let mut max_x = 0u32;
    let mut min_y = img.height;
    let mut max_y = 0u32;
    let w = img.width as usize;
    for y in 0..img.height {
        let row = y as usize * w;
        for x in 0..img.width {
            if img.pixels[row + x as usize].a > 0.02 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x {
        (0, 0, 0, 0)
    } else {
        (min_x, max_x, min_y, max_y)
    }
}

/// 4 点 Catmull-Rom 補間（双三次グリッドワープ基底）
#[inline]
fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

fn sample_bicubic_grid_offset(
    offsets: &[[f32; 2]],
    cols: usize,
    rows: usize,
    u: f32,
    v: f32,
) -> [f32; 2] {
    if offsets.len() < cols * rows || cols < 2 || rows < 2 {
        return [0.0, 0.0];
    }
    let gx = u.clamp(0.0, 1.0) * (cols - 1) as f32;
    let gy = v.clamp(0.0, 1.0) * (rows - 1) as f32;
    let ix = (gx.floor() as isize).clamp(0, cols as isize - 2);
    let iy = (gy.floor() as isize).clamp(0, rows as isize - 2);
    let tx = gx - ix as f32;
    let ty = gy - iy as f32;

    let get = |c: isize, r: isize| -> [f32; 2] {
        let cc = c.clamp(0, cols as isize - 1) as usize;
        let rr = r.clamp(0, rows as isize - 1) as usize;
        offsets[rr * cols + cc]
    };

    let mut row_vals_u = [0.0_f32; 4];
    let mut row_vals_v = [0.0_f32; 4];
    for kr in 0..4 {
        let r = iy + kr as isize - 1;
        let p0 = get(ix - 1, r);
        let p1 = get(ix, r);
        let p2 = get(ix + 1, r);
        let p3 = get(ix + 2, r);
        row_vals_u[kr] = catmull_rom(p0[0], p1[0], p2[0], p3[0], tx);
        row_vals_v[kr] = catmull_rom(p0[1], p1[1], p2[1], p3[1], tx);
    }
    [
        catmull_rom(
            row_vals_u[0],
            row_vals_u[1],
            row_vals_u[2],
            row_vals_u[3],
            ty,
        ),
        catmull_rom(
            row_vals_v[0],
            row_vals_v[1],
            row_vals_v[2],
            row_vals_v[3],
            ty,
        ),
    ]
}

/// クリップ画像に双三次グリッドワープ／プリセットワープ＋自己輝度マップディスプレイスメントを適用する
pub fn apply_warp_and_displacement(
    src: &LinearImage,
    cfg: &WarpDisplaceConfig,
    time_sec: f32,
) -> LinearImage {
    if !cfg.enabled || src.width < 2 || src.height < 2 {
        return src.clone();
    }
    let w = src.width;
    let h = src.height;
    let fw = (w - 1) as f32;
    let fh = (h - 1) as f32;
    let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);

    let luma = if cfg.luma_displace_px.abs() > 0.05 {
        Some(image_luma_plane(src))
    } else {
        None
    };

    let cols = cfg.grid_cols.clamp(2, 8);
    let rows = cfg.grid_rows.clamp(2, 8);
    let amt = cfg.amount.clamp(-1.0, 1.0);
    let freq = cfg.frequency.clamp(0.2, 16.0);
    let phase = time_sec * cfg.speed * TAU;

    for y in 0..h {
        let v = y as f32 / fh;
        let row = (y * w) as usize;
        for x in 0..w {
            let u = x as f32 / fw;
            let (mut su, mut sv) = match cfg.preset {
                WarpPreset::CustomGrid => {
                    let off = sample_bicubic_grid_offset(&cfg.control_offsets, cols, rows, u, v);
                    (u - off[0], v - off[1])
                }
                WarpPreset::FlagWave => {
                    let wave = (u * freq * TAU - phase).sin() * amt * 0.08 * u;
                    let wave_x = (v * freq * PI - phase * 0.7).cos() * amt * 0.025 * u;
                    (u - wave_x, v - wave)
                }
                WarpPreset::BulgeLens => {
                    let dx = u - 0.5;
                    let dy = v - 0.5;
                    let r = dx.hypot(dy);
                    let r_max = 0.5;
                    if r < r_max && r > 1e-5 {
                        let norm_r = r / r_max;
                        let factor = norm_r.powf(1.0 + amt * 0.85) / norm_r;
                        (0.5 + dx * factor, 0.5 + dy * factor)
                    } else {
                        (u, v)
                    }
                }
                WarpPreset::TwirlVortex => {
                    let dx = u - 0.5;
                    let dy = v - 0.5;
                    let r = dx.hypot(dy);
                    let falloff = (1.0 - (r / 0.55).min(1.0)).powi(2);
                    let theta = amt * PI * 1.4 * falloff;
                    let (ct, st) = (theta.cos(), theta.sin());
                    (0.5 + dx * ct - dy * st, 0.5 + dx * st + dy * ct)
                }
                WarpPreset::PerspectiveArch => {
                    let arch = ((u - 0.5) * PI).cos() * amt * 0.16;
                    (u, v + arch)
                }
                WarpPreset::RippleWater => {
                    let dx = u - 0.5;
                    let dy = v - 0.5;
                    let r = dx.hypot(dy).max(1e-4);
                    let rip = (r * freq * TAU - phase).sin() * amt * 0.035 * (-r * 2.5).exp();
                    (u - (dx / r) * rip, v - (dy / r) * rip)
                }
            };

            // 自己輝度マップによる X/Y 変位
            if let Some(ref lum) = luma {
                let d_luma = (lum[row + x as usize] - 0.5) * cfg.luma_displace_px;
                su += d_luma / fw.max(1.0);
                sv += d_luma / fh.max(1.0);
            }

            if (0.0..=1.0).contains(&su) && (0.0..=1.0).contains(&sv) {
                out.pixels[row + x as usize] = sample_bilinear(src, su * fw, sv * fh);
            }
        }
    }

    out
}

// ============================================================================
// #51: シネマ光学 VFX（擬似深度ボケ Rack Focus・ゴッドレイ・Brown-Conrady レンズ歪み＆色収差）
// ============================================================================

/// シーン全体へシネマ光学 VFX（擬似深度ボケ・ゴッドレイ・Brown-Conrady レンズ歪み＆物理色収差）を適用する
pub fn apply_cinema_optical_vfx(img: &mut LinearImage, cfg: &OpticalVfxConfig) {
    if !cfg.is_active() || img.width < 4 || img.height < 4 {
        return;
    }
    if cfg.bokeh_enabled && cfg.bokeh_radius_px > 0.3 {
        apply_pseudo_depth_bokeh(img, cfg);
    }
    if cfg.god_rays_enabled && cfg.god_rays_intensity > 0.005 {
        apply_volumetric_god_rays(img, cfg);
    }
    if cfg.lens_enabled
        && (cfg.k1.abs() > 1e-4
            || cfg.k2.abs() > 1e-4
            || cfg.p1.abs() > 1e-4
            || cfg.p2.abs() > 1e-4
            || cfg.chromatic_aberration.abs() > 1e-4)
    {
        apply_brown_conrady_and_ca(img, cfg);
    }
}

fn build_pseudo_depth_map(img: &LinearImage, mode: PseudoDepthMode) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut depth = vec![0.5_f32; w * h];
    match mode {
        PseudoDepthMode::VerticalTiltShift => {
            let denom = (h - 1).max(1) as f32;
            for y in 0..h {
                // 下端(手前)=0.0, 上端(奥)=1.0
                let d = 1.0 - (y as f32 / denom);
                depth[y * w..(y + 1) * w].fill(d);
            }
        }
        PseudoDepthMode::RadialCenter => {
            let cx = w as f32 * 0.5;
            let cy = h as f32 * 0.5;
            let max_r = cx.hypot(cy).max(1.0);
            for y in 0..h {
                let dy = y as f32 - cy;
                let row = y * w;
                for x in 0..w {
                    let dx = x as f32 - cx;
                    depth[row + x] = (dx.hypot(dy) / max_r).clamp(0.0, 1.0);
                }
            }
        }
        PseudoDepthMode::LuminanceSaliency => {
            let edges = compute_sobel_edge_magnitude(img);
            let max_e = edges.iter().copied().fold(0.0_f32, f32::max).max(1e-4);
            for i in 0..(w * h) {
                let lum = pixel_luma(img.pixels[i]).clamp(0.0, 1.0);
                let edge_n = (edges[i] / max_e).clamp(0.0, 1.0);
                let saliency = (0.55 * edge_n + 0.45 * lum).clamp(0.0, 1.0);
                // 顕著な被写体を手前 (0.0)、背景を奥 (1.0)
                depth[i] = 1.0 - saliency;
            }
        }
    }
    depth
}

/// 擬似深度マップに基づく可変錯乱円ディスク／多角形絞り玉ボケ（Rack Focus）
fn apply_pseudo_depth_bokeh(img: &mut LinearImage, cfg: &OpticalVfxConfig) {
    let w = img.width as usize;
    let h = img.height as usize;
    let depth = build_pseudo_depth_map(img, cfg.depth_mode);
    let src = img.clone();

    // 黄金角フィボナッチ分布による 24 点ディスク／多角形絞りカーネル
    let tap_count = 24usize;
    let golden_angle = 2.399_963_2_f32;
    let blades = cfg.bokeh_blades;
    let mut taps = Vec::with_capacity(tap_count);
    for i in 0..tap_count {
        let r_base = ((i as f32 + 0.5) / tap_count as f32).sqrt();
        let theta = i as f32 * golden_angle;
        let poly_scale = if blades >= 5 {
            let sector = TAU / blades as f32;
            let a = (theta % sector) - sector * 0.5;
            (sector * 0.5).cos() / a.cos().max(0.3)
        } else {
            1.0
        };
        let r = r_base * poly_scale;
        taps.push([r * theta.cos(), r * theta.sin()]);
    }

    let focus = cfg.focus_depth.clamp(0.0, 1.0);
    let band = cfg.focus_band.clamp(0.02, 0.6);
    let max_r = cfg.bokeh_radius_px.clamp(0.5, 20.0);
    let boost = cfg.bokeh_highlight_boost.clamp(0.0, 3.0);

    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let d = depth[row + x];
            let coc = ((d - focus).abs() - band).max(0.0) / (1.0 - band).max(0.1);
            let radius = (coc * max_r).clamp(0.0, max_r);
            if radius < 0.35 {
                continue;
            }
            let mut acc_r = 0.0_f32;
            let mut acc_g = 0.0_f32;
            let mut acc_b = 0.0_f32;
            let mut acc_a = 0.0_f32;
            let mut w_sum = 0.0_f32;
            for &[ox, oy] in &taps {
                let sx = (x as f32 + ox * radius).clamp(0.0, (w - 1) as f32);
                let sy = (y as f32 + oy * radius).clamp(0.0, (h - 1) as f32);
                let sp = sample_bilinear(&src, sx, sy);
                let l = pixel_luma(sp).clamp(0.0, 4.0);
                // 高輝度ハイライトを強く重み付けして映画的な玉ボケを生成
                let weight = 1.0 + l * l * boost;
                acc_r += sp.r * weight;
                acc_g += sp.g * weight;
                acc_b += sp.b * weight;
                acc_a += sp.a * weight;
                w_sum += weight;
            }
            if w_sum > 1e-6 {
                let inv = 1.0 / w_sum;
                img.pixels[row + x] = LinearPremul {
                    r: acc_r * inv,
                    g: acc_g * inv,
                    b: acc_b * inv,
                    a: acc_a * inv,
                };
            }
        }
    }
}

/// 放射状オクルージョンサンプリングによる体積光ゴッドレイ（Volumetric God Rays）
fn apply_volumetric_god_rays(img: &mut LinearImage, cfg: &OpticalVfxConfig) {
    let w = img.width as usize;
    let h = img.height as usize;
    let src = img.clone();
    let samples = cfg.god_rays_samples.clamp(8, 64);
    let lx = cfg.light_pos[0] * (w - 1) as f32;
    let ly = cfg.light_pos[1] * (h - 1) as f32;
    let decay = cfg.god_rays_decay.clamp(0.80, 0.99);
    let thresh = cfg.god_rays_threshold.clamp(0.1, 2.0);
    let intensity = cfg.god_rays_intensity.clamp(0.0, 2.0);
    let tint = LinearPremul::from_srgb_straight(cfg.god_rays_tint);
    let inv_s = 1.0 / samples as f32;

    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let mut sx = x as f32;
            let mut sy = y as f32;
            let dx = (lx - sx) * inv_s * 0.85;
            let dy = (ly - sy) * inv_s * 0.85;
            let mut illum = 1.0_f32;
            let mut ray_r = 0.0_f32;
            let mut ray_g = 0.0_f32;
            let mut ray_b = 0.0_f32;
            for _ in 0..samples {
                sx += dx;
                sy += dy;
                if sx < 0.0 || sx >= w as f32 || sy < 0.0 || sy >= h as f32 {
                    break;
                }
                let sp = sample_bilinear(&src, sx, sy);
                let l = pixel_luma(sp);
                if l > thresh {
                    let excess = ((l - thresh) / thresh.max(0.1)).min(2.5) * illum;
                    ray_r += sp.r * excess;
                    ray_g += sp.g * excess;
                    ray_b += sp.b * excess;
                }
                illum *= decay;
            }
            let scale = intensity * inv_s * 2.2;
            let dst = &mut img.pixels[row + x];
            dst.r = (dst.r + ray_r * tint.r * scale).clamp(0.0, 8.0);
            dst.g = (dst.g + ray_g * tint.g * scale).clamp(0.0, 8.0);
            dst.b = (dst.b + ray_b * tint.b * scale).clamp(0.0, 8.0);
        }
    }
}

/// Brown-Conrady 放射/接線レンズ歪み補正 ($k_1, k_2, p_1, p_2$) ＆ RGB 波長別物理色収差
fn apply_brown_conrady_and_ca(img: &mut LinearImage, cfg: &OpticalVfxConfig) {
    let w = img.width;
    let h = img.height;
    let src = img.clone();
    let cx = (w as f32 - 1.0) * 0.5;
    let cy = (h as f32 - 1.0) * 0.5;
    let inv_r = 1.0 / cx.max(cy).max(1.0);
    let k1 = cfg.k1;
    let k2 = cfg.k2;
    let p1 = cfg.p1;
    let p2 = cfg.p2;
    let ca = cfg.chromatic_aberration;

    // 樽型補正などで周辺が黒く欠けるのを自動ズームで補償
    let zoom = if cfg.lens_zoom_compensate {
        let corner_r2 = 1.0_f32;
        let corner_scale = (1.0 + k1 * corner_r2 + k2 * corner_r2 * corner_r2).max(0.4);
        if corner_scale < 1.0 {
            corner_scale
        } else {
            1.0
        }
    } else {
        1.0
    };

    let map_channel = |nx: f32, ny: f32, ch_scale: f32| -> (f32, f32) {
        let x = nx * zoom * ch_scale;
        let y = ny * zoom * ch_scale;
        let r2 = x * x + y * y;
        let r4 = r2 * r2;
        let radial = 1.0 + k1 * r2 + k2 * r4;
        let xd = x * radial + 2.0 * p1 * x * y + p2 * (r2 + 2.0 * x * x);
        let yd = y * radial + p1 * (r2 + 2.0 * y * y) + 2.0 * p2 * x * y;
        (cx + xd / inv_r, cy + yd / inv_r)
    };

    let fw = (w - 1) as f32;
    let fh = (h - 1) as f32;
    for y in 0..h {
        let ny = (y as f32 - cy) * inv_r;
        let row = (y * w) as usize;
        for x in 0..w {
            let nx = (x as f32 - cx) * inv_r;
            let (rx, ry) = map_channel(nx, ny, 1.0 + ca);
            let (gx, gy) = map_channel(nx, ny, 1.0);
            let (bx, by) = map_channel(nx, ny, 1.0 - ca);

            let pr = if (0.0..=fw).contains(&rx) && (0.0..=fh).contains(&ry) {
                sample_bilinear(&src, rx, ry)
            } else {
                LinearPremul::TRANSPARENT
            };
            let pg = if (0.0..=fw).contains(&gx) && (0.0..=fh).contains(&gy) {
                sample_bilinear(&src, gx, gy)
            } else {
                LinearPremul::TRANSPARENT
            };
            let pb = if (0.0..=fw).contains(&bx) && (0.0..=fh).contains(&by) {
                sample_bilinear(&src, bx, by)
            } else {
                LinearPremul::TRANSPARENT
            };

            img.pixels[row + x as usize] = LinearPremul {
                r: pr.r,
                g: pg.g,
                b: pb.b,
                a: pg.a.max(pr.a).max(pb.a),
            };
        }
    }
}

// ============================================================================
// #52: ADR 3-2-1 映像カウントダウンストリーマ描画
// ============================================================================

/// ADR プリロール中・収録中の視覚ストリーマ＆3-2-1 巨大カウントダウン数字をプレビューフレームへ重ねる
pub fn draw_adr_countdown_streamer(img: &mut LinearImage, state: &AdrCountdownState) {
    let w = img.width;
    let h = img.height;
    if w < 32 || h < 32 {
        return;
    }

    if let Some((beat, progress)) = state.countdown_beat {
        // 映画スタジオ標準の左→右ワイプストリーマバー（黄色・半透明オーバーレイ）
        let bar_x = (progress.clamp(0.0, 1.0) * (w as f32)) as i32;
        let bar_half = ((w as f32 * 0.015).round() as i32).clamp(3, 18);
        let bar_col = LinearPremul::from_srgb_straight(RgbaColor {
            r: 1.0,
            g: 0.86,
            b: 0.12,
            a: 0.82,
        });
        for y in 0..h {
            let row = (y * w) as usize;
            let x0 = (bar_x - bar_half).clamp(0, w as i32 - 1) as u32;
            let x1 = (bar_x + bar_half).clamp(0, w as i32 - 1) as u32;
            for x in x0..=x1 {
                img.pixels[row + x as usize] = bar_col.over(img.pixels[row + x as usize]);
            }
        }

        // 中央のカウントダウン円形バッジ + 巨大数字 (3 / 2 / 1)
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.45;
        let rad = (w.min(h) as f32 * 0.14).clamp(16.0, 120.0);
        let bg = LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.06,
            g: 0.08,
            b: 0.12,
            a: 0.78,
        });
        let ring = LinearPremul::from_srgb_straight(RgbaColor {
            r: 1.0,
            g: 0.84,
            b: 0.16,
            a: 0.95,
        });
        let min_y = ((cy - rad - 3.0).floor() as i32).clamp(0, h as i32 - 1) as u32;
        let max_y = ((cy + rad + 3.0).ceil() as i32).clamp(0, h as i32 - 1) as u32;
        let min_x = ((cx - rad - 3.0).floor() as i32).clamp(0, w as i32 - 1) as u32;
        let max_x = ((cx + rad + 3.0).ceil() as i32).clamp(0, w as i32 - 1) as u32;
        for y in min_y..=max_y {
            let row = (y * w) as usize;
            for x in min_x..=max_x {
                let dist = (x as f32 - cx).hypot(y as f32 - cy);
                if dist <= rad {
                    let c = if (rad - dist) <= 3.0 { ring } else { bg };
                    img.pixels[row + x as usize] = c.over(img.pixels[row + x as usize]);
                }
            }
        }

        let digit_char = char::from_digit(beat.min(9), 10).unwrap_or('3');
        let scale = ((rad * 0.18).round() as u32).clamp(2, 16);
        let dw = 5 * scale;
        let dh = 7 * scale;
        draw_bitmap_digit_5x7(
            img,
            digit_char,
            (cx as u32).saturating_sub(dw / 2),
            (cy as u32).saturating_sub(dh / 2),
            scale,
            ring,
        );
    } else if state.recording_active {
        // 収録中レッドプログレスバー（上端）
        let bar_h = (h / 32).clamp(4, 16);
        let fill_w = ((state.record_progress.clamp(0.0, 1.0) * w as f32).round() as u32).min(w);
        let rec_col = LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.95,
            g: 0.16,
            b: 0.22,
            a: 0.9,
        });
        for y in 0..bar_h {
            let row = (y * w) as usize;
            for x in 0..fill_w {
                img.pixels[row + x as usize] = rec_col.over(img.pixels[row + x as usize]);
            }
        }
    }
}

fn draw_bitmap_digit_5x7(
    img: &mut LinearImage,
    ch: char,
    ox: u32,
    oy: u32,
    scale: u32,
    col: LinearPremul,
) {
    let rows: [u8; 7] = match ch {
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        _ => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
    };
    let w = img.width;
    let h = img.height;
    for (gy, &bits) in rows.iter().enumerate() {
        for gx in 0..5u32 {
            if ((bits >> (4 - gx)) & 1) == 0 {
                continue;
            }
            for sy in 0..scale {
                let py = oy + gy as u32 * scale + sy;
                if py >= h {
                    continue;
                }
                let row = (py * w) as usize;
                for sx in 0..scale {
                    let px = ox + gx * scale + sx;
                    if px < w {
                        img.pixels[row + px as usize] = col.over(img.pixels[row + px as usize]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::{
        AdrCue, AdrSessionConfig, FrameRate, ItemKind, OpticalVfxPreset, ProjectSettings,
        ShapeKind, SvgShapePreset, TimelineItem, Transform2D,
    };

    #[test]
    fn planar_homography_ransac_recovers_projective_transform_with_outliers() {
        let src = [[10.0, 20.0], [110.0, 20.0], [110.0, 120.0], [10.0, 120.0]];
        let dst = [[22.0, 28.0], [126.0, 24.0], [130.0, 132.0], [16.0, 126.0]];
        let h_true = solve_quad_to_quad_homography(src, dst).expect("true H");
        let mut matches = Vec::new();
        for gy in 0..4 {
            for gx in 0..4 {
                let p = [15.0 + gx as f64 * 28.0, 25.0 + gy as f64 * 28.0];
                let q = project_homography(&h_true, p).unwrap();
                matches.push((p, q));
            }
        }
        // 外れ値 3 個を混入
        matches[1].1 = [300.0, -50.0];
        matches[6].1 = [-40.0, 220.0];
        matches[11].1 = [250.0, 260.0];

        let (h_est, inliers) =
            estimate_homography_ransac(&matches, 128, 1.5, 42).expect("ransac H");
        assert!(inliers >= 12, "inliers={inliers}");
        let test_pt = project_homography(&h_est, [60.0, 70.0]).unwrap();
        let ref_pt = project_homography(&h_true, [60.0, 70.0]).unwrap();
        assert!((test_pt[0] - ref_pt[0]).hypot(test_pt[1] - ref_pt[1]) < 1.5);
    }

    #[test]
    fn roto_brush_snaps_vertices_onto_high_contrast_subject_edge() {
        let mut img = LinearImage::new(96, 96, LinearPremul::from_srgb_straight(RgbaColor::BLACK));
        let white = LinearPremul::from_srgb_straight(RgbaColor::WHITE);
        for y in 24..72 {
            for x in 24..72 {
                img.pixels[y * 96 + x] = white;
            }
        }
        // 境界から少し内側／外側にずれた 4 頂点を配置 -> 正方形境界 (0.25, 0.75) へ吸着すること
        let coarse = vec![
            MaskVertex::corner(0.28, 0.28),
            MaskVertex::corner(0.72, 0.28),
            MaskVertex::corner(0.72, 0.72),
            MaskVertex::corner(0.28, 0.72),
        ];
        let snapped = snap_roto_vertices_to_edges(&img, &coarse, 8, 0.85);
        assert_eq!(snapped.len(), 4);
        assert!((snapped[0].x - 0.25).abs() < 0.04, "x0={}", snapped[0].x);
        assert!((snapped[1].x - 0.75).abs() < 0.04, "x1={}", snapped[1].x);
    }

    #[test]
    fn svg_morph_warp_and_cinema_optical_vfx_render_deterministically() {
        let cfg = SvgMorphConfig {
            path_a_d: SvgShapePreset::LightningBolt.svg_path_d().to_string(),
            path_b_d: SvgShapePreset::Heart.svg_path_d().to_string(),
            morph_progress: 0.5,
            auto_morph_hz: 0.0,
            ..Default::default()
        };
        let a = render_svg_morph(&cfg, None, 0.0, 128, 128);
        let b = render_svg_morph(&cfg, None, 0.0, 128, 128);
        assert_eq!(a.pixels, b.pixels);
        assert!(a.pixels.iter().any(|p| p.a > 0.5));

        let warp_cfg = WarpDisplaceConfig {
            enabled: true,
            preset: WarpPreset::FlagWave,
            amount: 0.5,
            ..Default::default()
        };
        let warped = apply_warp_and_displacement(&a, &warp_cfg, 0.3);
        assert_ne!(warped.pixels, a.pixels);

        let mut opt_img = a.clone();
        let opt = OpticalVfxConfig::from_preset(OpticalVfxPreset::VintagePrismLens);
        apply_cinema_optical_vfx(&mut opt_img, &opt);
        assert_ne!(opt_img.pixels, a.pixels);

        // ADR 映像カウントダウンストリーマの描画検証
        let mut adr = AdrSessionConfig {
            enabled: true,
            ..Default::default()
        };
        adr.cues.push(AdrCue::new(
            1,
            "Narrator",
            "テスト収録",
            RationalTime::from_seconds(3.0),
            RationalTime::from_seconds(2.0),
        ));
        let st = adr
            .evaluate_countdown_at(RationalTime::from_seconds(1.5))
            .expect("countdown state");
        let mut adr_img =
            LinearImage::new(160, 90, LinearPremul::from_srgb_straight(RgbaColor::BLACK));
        draw_adr_countdown_streamer(&mut adr_img, &st);
        assert!(adr_img.pixels.iter().any(|p| p.r > 0.5 && p.g > 0.4));
    }

    #[test]
    fn planar_tracker_tracks_moving_textured_quad_in_scene() {
        let mut proj = Project::new(
            "PlanarTrackTest",
            ProjectSettings {
                width: 320,
                height: 180,
                fps: FrameRate::FPS_30,
                ..Default::default()
            },
        );
        let scene_id = proj.active_scene_id;
        // 移動する高コントラスト白矩形を配置
        let mut card = TimelineItem::new(
            "Signboard",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 60.0,
                color: RgbaColor::WHITE,
            },
        );
        card.transform = Transform2D {
            x: 80.0,
            y: 60.0,
            ..Default::default()
        };
        card.keyframes.insert(
            "x".to_string(),
            vec![
                ve_core::Keyframe {
                    time: RationalTime::ZERO,
                    value: 80.0,
                    interpolation: ve_core::Interpolation::Linear,
                },
                ve_core::Keyframe {
                    time: RationalTime::from_seconds(0.5),
                    value: 120.0,
                    interpolation: ve_core::Interpolation::Linear,
                },
            ],
        );
        proj.active_scene_mut().unwrap().layers[0].items.push(card);

        let mut rg = RenderGraph::new();
        let cfg = PlanarTrackConfig {
            start_time: RationalTime::ZERO,
            end_time: RationalTime::from_seconds(0.5),
            step_frames: 3,
            initial_quad: [[0.22, 0.30], [0.58, 0.30], [0.58, 0.70], [0.22, 0.70]],
            patch_radius: 6,
            search_radius: 16,
            ransac_iterations: 48,
            ransac_inlier_px: 2.5,
        };
        let res = track_planar_quad_in_scene(&mut rg, &proj, scene_id, &cfg);
        assert!(res.frames.len() >= 4);
        let first_x = res.frames.first().unwrap().quad[0][0];
        let last_x = res.frames.last().unwrap().quad[0][0];
        assert!(
            last_x > first_x + 0.05,
            "tracked quad should shift right: first={first_x:.3} last={last_x:.3}"
        );
    }
}
