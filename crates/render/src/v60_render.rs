//! v6.0 #41 & #42: データ駆動インフォグラフィック・チャート描画 ＆ 非破壊 VFX ペイント / インペイントパス
//! Pure Rust・ゼロ `unsafe` で以下を決定論的に描画する:
//! - #41 `render_data_chart`: 棒グラフ・折れ線＆エリア・ドーナツ・KPI ロールアップカウンター（Cubic Ease-Out 立ち上がり）
//! - #42 `apply_vfx_paint`: グローブラシ（Write-On 対応）・クローンスタンプ・ワイヤー消し（境界補間インペイント）・プライバシーぼかし

use crate::color::{LinearImage, LinearPremul, srgb_to_linear};
use std::f32::consts::{FRAC_PI_2, TAU};
use ve_core::{ChartKind, DataChartConfig, PaintToolKind, RgbaColor, VfxPaintConfig};

const SERIES_PALETTE: [[f32; 3]; 6] = [
    [0.20, 0.78, 0.98], // Cyan
    [0.98, 0.42, 0.56], // Coral Rose
    [0.34, 0.92, 0.58], // Mint Emerald
    [1.00, 0.76, 0.24], // Amber Gold
    [0.68, 0.48, 0.98], // Violet
    [0.98, 0.58, 0.26], // Sunset Orange
];

#[inline]
fn rgba_to_linear_premul(c: RgbaColor, alpha_mul: f32) -> LinearPremul {
    let a = (c.a * alpha_mul).clamp(0.0, 1.0);
    LinearPremul {
        r: srgb_to_linear(c.r.clamp(0.0, 1.0)) * a,
        g: srgb_to_linear(c.g.clamp(0.0, 1.0)) * a,
        b: srgb_to_linear(c.b.clamp(0.0, 1.0)) * a,
        a,
    }
}

#[inline]
fn rgb_to_linear_premul(rgb: [f32; 3], a: f32) -> LinearPremul {
    let a = a.clamp(0.0, 1.0);
    LinearPremul {
        r: srgb_to_linear(rgb[0].clamp(0.0, 1.0)) * a,
        g: srgb_to_linear(rgb[1].clamp(0.0, 1.0)) * a,
        b: srgb_to_linear(rgb[2].clamp(0.0, 1.0)) * a,
        a,
    }
}

#[inline]
fn blend_over(dst: &mut LinearPremul, src: LinearPremul) {
    let inv = (1.0 - src.a).max(0.0);
    dst.r = src.r + dst.r * inv;
    dst.g = src.g + dst.g * inv;
    dst.b = src.b + dst.b * inv;
    dst.a = src.a + dst.a * inv;
}

#[inline]
fn blend_add(dst: &mut LinearPremul, r: f32, g: f32, b: f32, a: f32) {
    dst.r = (dst.r + r).min(2.0);
    dst.g = (dst.g + g).min(2.0);
    dst.b = (dst.b + b).min(2.0);
    dst.a = (dst.a + a * (1.0 - dst.a)).clamp(0.0, 1.0);
}

#[inline]
fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

#[inline]
fn point_series_rgb(cfg: &DataChartConfig, idx: usize) -> [f32; 3] {
    if let Some(pt) = cfg.points.get(idx)
        && let Some(c) = pt.color
    {
        return [c.r, c.g, c.b];
    }
    if idx == 0 {
        [cfg.accent_color.r, cfg.accent_color.g, cfg.accent_color.b]
    } else {
        SERIES_PALETTE[idx % SERIES_PALETTE.len()]
    }
}

/// v6.0 #41: データ駆動モーショングラフィックス＆インフォグラフィック・チャートを描画する
pub fn render_data_chart(
    cfg: &DataChartConfig,
    local_time_sec: f32,
    out_w: u32,
    out_h: u32,
) -> LinearImage {
    let w = out_w.clamp(64, 4096);
    let h = out_h.clamp(48, 4096);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let wf = w as f32;
    let hf = h as f32;

    let progress_raw = (local_time_sec.max(0.0) / cfg.animate_in_sec.max(0.05)).clamp(0.0, 1.0);
    let anim = ease_out_cubic(progress_raw);

    // 1. 角丸カード背景＆上部アクセント枠
    let radius = (wf.min(hf) * 0.045).clamp(4.0, 26.0);
    let bg_px = rgba_to_linear_premul(cfg.bg_color, 1.0);
    let border_px = rgba_to_linear_premul(cfg.accent_color, 0.35);
    let accent_top_px = rgba_to_linear_premul(cfg.accent_color, 0.9);
    for y in 0..h {
        let py = y as f32 + 0.5;
        let row = (y as usize) * (w as usize);
        for x in 0..w {
            let px = x as f32 + 0.5;
            let sdf = rounded_rect_sdf(
                px,
                py,
                wf * 0.5,
                hf * 0.5,
                wf * 0.5 - 2.0,
                hf * 0.5 - 2.0,
                radius,
            );
            if sdf < 1.0 {
                let cov = (0.5 - sdf).clamp(0.0, 1.0);
                let mut c = LinearPremul {
                    r: bg_px.r * cov,
                    g: bg_px.g * cov,
                    b: bg_px.b * cov,
                    a: bg_px.a * cov,
                };
                if sdf > -2.0 {
                    let bcov = (1.0 - (sdf + 1.0).abs()).clamp(0.0, 1.0) * cov;
                    let bpx = LinearPremul {
                        r: border_px.r * bcov,
                        g: border_px.g * bcov,
                        b: border_px.b * bcov,
                        a: border_px.a * bcov,
                    };
                    blend_over(&mut c, bpx);
                }
                if py < 6.0 && sdf < -1.0 {
                    blend_over(&mut c, accent_top_px);
                }
                img.pixels[row + x as usize] = c;
            }
        }
    }

    // 2. タイトルテキスト
    let text_scale = ((h / 150) as usize).clamp(1, 4);
    let pad_x = (wf * 0.06).max(8.0);
    let pad_top = (hf * 0.07).max(8.0);
    let title_color = rgb_to_linear_premul([0.96, 0.98, 1.0], 0.98);
    if !cfg.title.trim().is_empty() {
        draw_bitmap_text(
            &mut img,
            &cfg.title.to_uppercase(),
            pad_x as i32,
            pad_top as i32,
            text_scale,
            title_color,
        );
    }

    let plot_x0 = pad_x;
    let plot_x1 = (wf - pad_x).max(plot_x0 + 16.0);
    let plot_y0 = (pad_top + (9 * text_scale) as f32 + hf * 0.05).min(hf * 0.35);
    let plot_y1 = (hf - (pad_top + (9 * text_scale) as f32)).max(plot_y0 + 16.0);

    // 3. 種別ごとのチャート描画
    match cfg.kind {
        ChartKind::BarChart => {
            draw_bar_chart(
                &mut img, cfg, anim, plot_x0, plot_y0, plot_x1, plot_y1, text_scale,
            );
        }
        ChartKind::LineChart => {
            draw_line_chart(
                &mut img, cfg, anim, plot_x0, plot_y0, plot_x1, plot_y1, text_scale,
            );
        }
        ChartKind::DonutChart => {
            draw_donut_chart(
                &mut img, cfg, anim, plot_x0, plot_y0, plot_x1, plot_y1, text_scale,
            );
        }
        ChartKind::KpiCounter => {
            draw_kpi_counter(
                &mut img, cfg, anim, plot_x0, plot_y0, plot_x1, plot_y1, text_scale,
            );
        }
    }

    img
}

#[allow(clippy::too_many_arguments)]
fn draw_grid_lines(img: &mut LinearImage, x0: f32, y0: f32, x1: f32, y1: f32) {
    let grid_px = rgb_to_linear_premul([0.72, 0.80, 0.92], 0.16);
    for step in 0..=4 {
        let t = step as f32 / 4.0;
        let y = (y0 + (y1 - y0) * t).round() as i32;
        if y < 0 || y >= img.height as i32 {
            continue;
        }
        let sx0 = (x0.round() as i32).clamp(0, img.width as i32 - 1) as usize;
        let sx1 = (x1.round() as i32).clamp(0, img.width as i32 - 1) as usize;
        let row = (y as usize) * (img.width as usize);
        for x in sx0..=sx1 {
            blend_over(&mut img.pixels[row + x], grid_px);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_bar_chart(
    img: &mut LinearImage,
    cfg: &DataChartConfig,
    anim: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    text_scale: usize,
) {
    if cfg.show_grid {
        draw_grid_lines(img, x0, y0, x1, y1);
    }
    if cfg.points.is_empty() {
        return;
    }
    let max_val = cfg
        .points
        .iter()
        .map(|p| p.value.abs())
        .fold(1e-6_f64, f64::max) as f32;
    let n = cfg.points.len();
    let slot_w = (x1 - x0) / (n as f32);
    let bar_w = (slot_w * 0.58).max(4.0);
    let label_col = rgb_to_linear_premul([0.84, 0.90, 0.98], 0.92);
    let val_col = rgb_to_linear_premul([1.0, 1.0, 1.0], 0.96);

    for (idx, pt) in cfg.points.iter().enumerate() {
        // 棒ごとに微妙な時間差スタガー演出
        let stagger = if n > 1 {
            (idx as f32 / (n as f32 - 1.0)) * 0.25
        } else {
            0.0
        };
        let local_a =
            ease_out_cubic(((anim - stagger * 0.5) / (1.0 - stagger * 0.5)).clamp(0.0, 1.0));
        let norm_h = ((pt.value.max(0.0) as f32) / max_val).clamp(0.0, 1.0) * local_a;
        let bar_h = ((y1 - y0) * norm_h).max(1.0);
        let bx0 = x0 + (idx as f32 + 0.5) * slot_w - bar_w * 0.5;
        let bx1 = bx0 + bar_w;
        let by0 = y1 - bar_h;
        let rgb = point_series_rgb(cfg, idx);

        let ix0 = (bx0.floor() as i32).clamp(0, img.width as i32 - 1) as u32;
        let ix1 = (bx1.ceil() as i32).clamp(0, img.width as i32 - 1) as u32;
        let iy0 = (by0.floor() as i32).clamp(0, img.height as i32 - 1) as u32;
        let iy1 = (y1.ceil() as i32).clamp(0, img.height as i32 - 1) as u32;

        for y in iy0..=iy1 {
            let v_grad = 1.0 - ((y as f32 - by0) / bar_h.max(1.0)).clamp(0.0, 1.0) * 0.32;
            let px = rgb_to_linear_premul(
                [
                    (rgb[0] * v_grad).min(1.0),
                    (rgb[1] * v_grad).min(1.0),
                    (rgb[2] * v_grad).min(1.0),
                ],
                0.96,
            );
            let row = (y as usize) * (img.width as usize);
            for x in ix0..=ix1 {
                blend_over(&mut img.pixels[row + x as usize], px);
            }
        }

        let lbl = pt.label.to_uppercase();
        let lbl_w = (lbl.chars().count().min(6) * 6 * text_scale) as f32;
        let lx = (bx0 + bar_w * 0.5 - lbl_w * 0.5) as i32;
        draw_bitmap_text(img, &lbl, lx, (y1 + 4.0) as i32, text_scale, label_col);

        if cfg.show_values {
            let cur_val = pt.value * f64::from(local_a);
            let vtxt = format!("{:.0}{}", cur_val.round(), cfg.unit_suffix);
            let vw = (vtxt.chars().count().min(8) * 6 * text_scale) as f32;
            let vx = (bx0 + bar_w * 0.5 - vw * 0.5) as i32;
            let vy = (by0 - (8 * text_scale) as f32).max(4.0) as i32;
            draw_bitmap_text(img, &vtxt, vx, vy, text_scale, val_col);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line_chart(
    img: &mut LinearImage,
    cfg: &DataChartConfig,
    anim: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    text_scale: usize,
) {
    if cfg.show_grid {
        draw_grid_lines(img, x0, y0, x1, y1);
    }
    if cfg.points.is_empty() {
        return;
    }
    let max_val = cfg
        .points
        .iter()
        .map(|p| p.value.abs())
        .fold(1e-6_f64, f64::max) as f32;
    let n = cfg.points.len();
    let coords: Vec<[f32; 2]> = cfg
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let tx = if n > 1 {
                i as f32 / (n as f32 - 1.0)
            } else {
                0.5
            };
            let px = x0 + (x1 - x0) * tx;
            let ny = ((p.value.max(0.0) as f32) / max_val).clamp(0.0, 1.0);
            let py = y1 - (y1 - y0) * ny;
            [px, py]
        })
        .collect();

    let reveal_x = x0 + (x1 - x0) * anim;
    let accent = [cfg.accent_color.r, cfg.accent_color.g, cfg.accent_color.b];
    let line_width = ((img.height as f32) * 0.012).clamp(2.0, 7.0);

    // 1. ライン下の半透明グラデーションエリア
    if coords.len() >= 2 {
        let sx0 = (x0.floor() as i32).clamp(0, img.width as i32 - 1) as u32;
        let sx1 = (reveal_x.ceil() as i32).clamp(0, img.width as i32 - 1) as u32;
        for x in sx0..=sx1 {
            let xf = x as f32 + 0.5;
            if let Some(ly) = sample_polyline_y(&coords, xf) {
                let iy0 = (ly.ceil() as i32).clamp(0, img.height as i32 - 1) as u32;
                let iy1 = (y1.floor() as i32).clamp(0, img.height as i32 - 1) as u32;
                for y in iy0..=iy1 {
                    let fade =
                        (1.0 - ((y as f32 - ly) / (y1 - ly).max(1.0)).clamp(0.0, 1.0)) * 0.32;
                    let fill_px = rgb_to_linear_premul(accent, fade);
                    blend_over(
                        &mut img.pixels[(y as usize) * (img.width as usize) + x as usize],
                        fill_px,
                    );
                }
            }
        }
    }

    // 2. アンチエイリアス折れ線ストローク
    for pair in coords.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if a[0] > reveal_x {
            break;
        }
        let end_pt = if b[0] > reveal_x && (b[0] - a[0]).abs() > 1e-4 {
            let t = ((reveal_x - a[0]) / (b[0] - a[0])).clamp(0.0, 1.0);
            [reveal_x, a[1] + (b[1] - a[1]) * t]
        } else {
            b
        };
        draw_aa_segment(img, a, end_pt, line_width, accent);
    }

    // 3. データ点ノード＆ラベル
    let dot_r = (line_width * 1.45).clamp(3.0, 10.0);
    let label_col = rgb_to_linear_premul([0.84, 0.90, 0.98], 0.92);
    let val_col = rgb_to_linear_premul([1.0, 1.0, 1.0], 0.96);
    for (idx, (&pt_xy, pt)) in coords.iter().zip(cfg.points.iter()).enumerate() {
        if pt_xy[0] <= reveal_x + 1.0 {
            draw_aa_disc(img, pt_xy[0], pt_xy[1], dot_r, [1.0, 1.0, 1.0], 1.0);
            draw_aa_disc(img, pt_xy[0], pt_xy[1], dot_r * 0.62, accent, 1.0);
            if cfg.show_values {
                let vtxt = format!("{:.0}{}", pt.value.round(), cfg.unit_suffix);
                draw_bitmap_text(
                    img,
                    &vtxt,
                    (pt_xy[0] - (vtxt.len() * 3 * text_scale) as f32) as i32,
                    (pt_xy[1] - dot_r - (8 * text_scale) as f32).max(4.0) as i32,
                    text_scale,
                    val_col,
                );
            }
        }
        if idx == 0 || idx + 1 == n || idx.is_multiple_of((n / 4).max(1)) {
            let lbl = pt.label.to_uppercase();
            draw_bitmap_text(
                img,
                &lbl,
                (pt_xy[0] - (lbl.len() * 3 * text_scale) as f32) as i32,
                (y1 + 4.0) as i32,
                text_scale,
                label_col,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_donut_chart(
    img: &mut LinearImage,
    cfg: &DataChartConfig,
    anim: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    text_scale: usize,
) {
    if cfg.points.is_empty() {
        return;
    }
    let total: f64 = cfg.points.iter().map(|p| p.value.max(0.0)).sum();
    if total <= 1e-6 {
        return;
    }
    let cx = x0 + (x1 - x0) * 0.34;
    let cy = (y0 + y1) * 0.5;
    let outer_r = ((y1 - y0).min((x1 - x0) * 0.58) * 0.45).max(12.0);
    let inner_r = outer_r * 0.56;
    let max_sweep = TAU * anim;

    let mut cumulative = Vec::with_capacity(cfg.points.len());
    let mut acc = 0.0f32;
    for (idx, pt) in cfg.points.iter().enumerate() {
        let frac = (pt.value.max(0.0) / total) as f32;
        let start = acc;
        acc += frac * TAU;
        cumulative.push((start, acc, point_series_rgb(cfg, idx)));
    }

    let ix0 = ((cx - outer_r - 2.0).floor() as i32).clamp(0, img.width as i32 - 1) as u32;
    let ix1 = ((cx + outer_r + 2.0).ceil() as i32).clamp(0, img.width as i32 - 1) as u32;
    let iy0 = ((cy - outer_r - 2.0).floor() as i32).clamp(0, img.height as i32 - 1) as u32;
    let iy1 = ((cy + outer_r + 2.0).ceil() as i32).clamp(0, img.height as i32 - 1) as u32;

    for y in iy0..=iy1 {
        let dy = y as f32 + 0.5 - cy;
        let row = (y as usize) * (img.width as usize);
        for x in ix0..=ix1 {
            let dx = x as f32 + 0.5 - cx;
            let dist = (dx * dx + dy * dy).sqrt();
            let ring_cov = ((outer_r - dist + 0.75).clamp(0.0, 1.0))
                .min((dist - inner_r + 0.75).clamp(0.0, 1.0));
            if ring_cov <= 0.001 {
                continue;
            }
            // 上端 12 時方向を 0 として時計回りに角度算出
            let mut angle = dy.atan2(dx) + FRAC_PI_2;
            if angle < 0.0 {
                angle += TAU;
            }
            if angle > max_sweep {
                continue;
            }
            if let Some(&(_, _, rgb)) = cumulative
                .iter()
                .find(|&&(s, e, _)| angle >= s && angle <= e + 1e-4)
            {
                let px = rgb_to_linear_premul(rgb, ring_cov * 0.98);
                blend_over(&mut img.pixels[row + x as usize], px);
            }
        }
    }

    // 中央合計値
    if cfg.show_values {
        let center_txt = format!(
            "{:.0}{}",
            (total * f64::from(anim)).round(),
            cfg.unit_suffix
        );
        let cw = (center_txt.len() * 6 * text_scale) as f32;
        draw_bitmap_text(
            img,
            &center_txt,
            (cx - cw * 0.5) as i32,
            (cy - (3.5 * text_scale as f32)) as i32,
            text_scale,
            rgb_to_linear_premul([1.0, 1.0, 1.0], 0.98),
        );
    }

    // 右側凡例リスト
    let leg_x = (cx + outer_r + (x1 - x0) * 0.07) as i32;
    let row_h = ((12 * text_scale) as f32).max(16.0);
    let leg_y0 = (cy - (cfg.points.len().min(6) as f32 * row_h) * 0.5).max(y0);
    let lbl_col = rgb_to_linear_premul([0.92, 0.96, 1.0], 0.94);
    for (idx, pt) in cfg.points.iter().take(6).enumerate() {
        let ly = (leg_y0 + idx as f32 * row_h) as i32;
        let rgb = point_series_rgb(cfg, idx);
        draw_aa_disc(
            img,
            (leg_x + 4 * text_scale as i32) as f32,
            (ly + 3 * text_scale as i32) as f32,
            (3.5 * text_scale as f32).max(3.0),
            rgb,
            1.0,
        );
        let pct = (pt.value.max(0.0) / total * 100.0).round();
        let txt = format!("{} {:.0}%", pt.label.to_uppercase(), pct);
        draw_bitmap_text(
            img,
            &txt,
            leg_x + 11 * text_scale as i32,
            ly,
            text_scale,
            lbl_col,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_kpi_counter(
    img: &mut LinearImage,
    cfg: &DataChartConfig,
    anim: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    text_scale: usize,
) {
    let target = cfg.points.first().map(|p| p.value).unwrap_or(100.0);
    let label = cfg
        .points
        .first()
        .map(|p| p.label.to_uppercase())
        .unwrap_or_else(|| "TOTAL".into());
    let cur = target * f64::from(anim);
    let num_txt = format!("{:.0}{}", cur.round(), cfg.unit_suffix);
    let accent = [cfg.accent_color.r, cfg.accent_color.g, cfg.accent_color.b];

    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.48;
    // 背景ソフトラジアルグロー
    let glow_r = ((x1 - x0).min(y1 - y0) * 0.55).max(20.0);
    let gx0 = ((cx - glow_r).floor() as i32).clamp(0, img.width as i32 - 1) as u32;
    let gx1 = ((cx + glow_r).ceil() as i32).clamp(0, img.width as i32 - 1) as u32;
    let gy0 = ((cy - glow_r).floor() as i32).clamp(0, img.height as i32 - 1) as u32;
    let gy1 = ((cy + glow_r).ceil() as i32).clamp(0, img.height as i32 - 1) as u32;
    for y in gy0..=gy1 {
        let dy = y as f32 + 0.5 - cy;
        let row = (y as usize) * (img.width as usize);
        for x in gx0..=gx1 {
            let dx = x as f32 + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt() / glow_r;
            if d < 1.0 {
                let g = (1.0 - d) * (1.0 - d) * 0.22;
                blend_add(
                    &mut img.pixels[row + x as usize],
                    srgb_to_linear(accent[0]) * g,
                    srgb_to_linear(accent[1]) * g,
                    srgb_to_linear(accent[2]) * g,
                    g,
                );
            }
        }
    }

    // 巨大 KPI 数値ロールアップ
    let big_scale = (text_scale * 3).clamp(2, 10);
    let num_w = (num_txt.len() * 6 * big_scale) as f32;
    let num_h = (7 * big_scale) as f32;
    draw_bitmap_text(
        img,
        &num_txt,
        (cx - num_w * 0.5) as i32,
        (cy - num_h * 0.5) as i32,
        big_scale,
        rgb_to_linear_premul(accent, 1.0),
    );

    // サブキャプション＆下部進捗バー
    let lw = (label.len() * 6 * text_scale) as f32;
    draw_bitmap_text(
        img,
        &label,
        (cx - lw * 0.5) as i32,
        (cy + num_h * 0.55 + 6.0) as i32,
        text_scale,
        rgb_to_linear_premul([0.86, 0.90, 0.98], 0.9),
    );

    let bar_y0 = (y1 - 10.0).max(y0) as u32;
    let bar_y1 = (y1 - 3.0).min((img.height - 1) as f32) as u32;
    let bar_x0 = x0 as u32;
    let bar_x1 = (x0 + (x1 - x0) * anim).min((img.width - 1) as f32) as u32;
    let bar_px = rgb_to_linear_premul(accent, 0.95);
    for y in bar_y0..=bar_y1 {
        let row = (y as usize) * (img.width as usize);
        for x in bar_x0..=bar_x1 {
            blend_over(&mut img.pixels[row + x as usize], bar_px);
        }
    }
}

/// v6.0 #42: クリップ画像に対して非破壊 VFX ペイント（グローブラシ / クローンスタンプ / ワイヤー消し / 秘匿ぼかし）を適用する
pub fn apply_vfx_paint(img: &mut LinearImage, cfg: &VfxPaintConfig, local_progress: f32) {
    if !cfg.enabled || cfg.strokes.is_empty() || img.width < 2 || img.height < 2 {
        return;
    }
    let wf = img.width as f32;
    let hf = img.height as f32;
    let w = img.width as usize;
    let h = img.height as usize;

    for stroke in &cfg.strokes {
        if stroke.points.is_empty() || stroke.opacity <= 0.001 {
            continue;
        }
        let eff_write_on = if cfg.animate_write_on && stroke.tool == PaintToolKind::BrushGlow {
            (local_progress.clamp(0.0, 1.0) * stroke.write_on_progress).clamp(0.0, 1.0)
        } else {
            stroke.write_on_progress.clamp(0.0, 1.0)
        };
        if eff_write_on <= 0.001 {
            continue;
        }

        let px_pts = build_active_polyline(&stroke.points, wf, hf, eff_write_on);
        if px_pts.is_empty() {
            continue;
        }
        let radius = stroke.radius_px.clamp(1.0, 256.0);
        let hardness = stroke.hardness.clamp(0.05, 1.0);
        let opacity = stroke.opacity.clamp(0.0, 1.0);
        let glow_pad = if stroke.tool == PaintToolKind::BrushGlow {
            radius * 2.2
        } else {
            radius + 3.0
        };

        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for &[px, py] in &px_pts {
            min_x = min_x.min(px);
            max_x = max_x.max(px);
            min_y = min_y.min(py);
            max_y = max_y.max(py);
        }
        let bx0 = ((min_x - glow_pad).floor() as i32).clamp(0, img.width as i32 - 1) as usize;
        let bx1 = ((max_x + glow_pad).ceil() as i32).clamp(0, img.width as i32 - 1) as usize;
        let by0 = ((min_y - glow_pad).floor() as i32).clamp(0, img.height as i32 - 1) as usize;
        let by1 = ((max_y + glow_pad).ceil() as i32).clamp(0, img.height as i32 - 1) as usize;

        match stroke.tool {
            PaintToolKind::BrushGlow => {
                let lin_r = srgb_to_linear(stroke.color.r.clamp(0.0, 1.0));
                let lin_g = srgb_to_linear(stroke.color.g.clamp(0.0, 1.0));
                let lin_b = srgb_to_linear(stroke.color.b.clamp(0.0, 1.0));
                let inner_r = radius * hardness;
                let outer_glow = radius * 2.0;

                for y in by0..=by1 {
                    let py = y as f32 + 0.5;
                    let row = y * w;
                    for x in bx0..=bx1 {
                        let px = x as f32 + 0.5;
                        let (d, _) = dist_and_dir_to_polyline(px, py, &px_pts);
                        if d >= outer_glow {
                            continue;
                        }
                        if d <= radius {
                            let core_t = if d <= inner_r {
                                1.0
                            } else {
                                (1.0 - (d - inner_r) / (radius - inner_r).max(0.5)).clamp(0.0, 1.0)
                            };
                            let a = core_t * opacity * stroke.color.a.clamp(0.0, 1.0);
                            blend_over(
                                &mut img.pixels[row + x],
                                LinearPremul {
                                    r: lin_r * a,
                                    g: lin_g * a,
                                    b: lin_b * a,
                                    a,
                                },
                            );
                        } else {
                            let gt = 1.0
                                - ((d - radius) / (outer_glow - radius).max(1.0)).clamp(0.0, 1.0);
                            let g = gt * gt * 0.42 * opacity;
                            blend_add(
                                &mut img.pixels[row + x],
                                lin_r * g,
                                lin_g * g,
                                lin_b * g,
                                g * 0.4,
                            );
                        }
                    }
                }
            }
            PaintToolKind::CloneStamp => {
                let src_snap = img.clone();
                let off_x = stroke.clone_offset[0] * wf;
                let off_y = stroke.clone_offset[1] * hf;
                let inner_r = radius * hardness;

                for y in by0..=by1 {
                    let py = y as f32 + 0.5;
                    let row = y * w;
                    for x in bx0..=bx1 {
                        let px = x as f32 + 0.5;
                        let (d, _) = dist_and_dir_to_polyline(px, py, &px_pts);
                        if d >= radius {
                            continue;
                        }
                        let w_mask = if d <= inner_r {
                            1.0
                        } else {
                            (1.0 - (d - inner_r) / (radius - inner_r).max(0.5)).clamp(0.0, 1.0)
                        } * opacity;
                        let sampled = sample_bilinear(&src_snap, px + off_x, py + off_y);
                        let dst = &mut img.pixels[row + x];
                        dst.r = dst.r * (1.0 - w_mask) + sampled.r * w_mask;
                        dst.g = dst.g * (1.0 - w_mask) + sampled.g * w_mask;
                        dst.b = dst.b * (1.0 - w_mask) + sampled.b * w_mask;
                        dst.a = dst.a * (1.0 - w_mask) + sampled.a * w_mask;
                    }
                }
            }
            PaintToolKind::WireEraseInpaint => {
                // 境界リング法線インペインティング: ストローク外のクリーン背景ピクセルを法線両側＆周囲8方向から距離逆重み補間
                let src_snap = img.clone();
                let inner_r = radius * hardness;
                let ring_dist = radius + 2.5;
                let compass: [[f32; 2]; 8] = [
                    [1.0, 0.0],
                    [-1.0, 0.0],
                    [0.0, 1.0],
                    [0.0, -1.0],
                    [0.707, 0.707],
                    [-0.707, 0.707],
                    [0.707, -0.707],
                    [-0.707, -0.707],
                ];

                for y in by0..=by1 {
                    let py = y as f32 + 0.5;
                    let row = y * w;
                    for x in bx0..=bx1 {
                        let px = x as f32 + 0.5;
                        let (d, [tx, ty]) = dist_and_dir_to_polyline(px, py, &px_pts);
                        if d >= radius {
                            continue;
                        }
                        let w_mask = if d <= inner_r {
                            1.0
                        } else {
                            (1.0 - (d - inner_r) / (radius - inner_r).max(0.5)).clamp(0.0, 1.0)
                        } * opacity;

                        // まずストローク法線（±perp）優先でクリーン境界を取得
                        let nx = -ty;
                        let ny = tx;
                        let mut acc_r = 0.0f32;
                        let mut acc_g = 0.0f32;
                        let mut acc_b = 0.0f32;
                        let mut acc_a = 0.0f32;
                        let mut w_sum = 0.0f32;

                        for &side in &[-1.0f32, 1.0f32] {
                            let sx = (px + nx * ring_dist * side).clamp(0.5, wf - 0.5);
                            let sy = (py + ny * ring_dist * side).clamp(0.5, hf - 0.5);
                            let (sd, _) = dist_and_dir_to_polyline(sx, sy, &px_pts);
                            if sd >= radius + 0.75 {
                                let c = sample_bilinear(&src_snap, sx, sy);
                                let wt = 2.0 / (1.0 + (sx - px).hypot(sy - py));
                                acc_r += c.r * wt;
                                acc_g += c.g * wt;
                                acc_b += c.b * wt;
                                acc_a += c.a * wt;
                                w_sum += wt;
                            }
                        }

                        // 端部や交差部向けに 8 方向フォールバックも補助加算
                        for &[cx, cy] in &compass {
                            let sx = (px + cx * (ring_dist + 1.5)).clamp(0.5, wf - 0.5);
                            let sy = (py + cy * (ring_dist + 1.5)).clamp(0.5, hf - 0.5);
                            let (sd, _) = dist_and_dir_to_polyline(sx, sy, &px_pts);
                            if sd >= radius + 0.75 {
                                let c = sample_bilinear(&src_snap, sx, sy);
                                let wt = 0.65 / (1.0 + (sx - px).hypot(sy - py));
                                acc_r += c.r * wt;
                                acc_g += c.g * wt;
                                acc_b += c.b * wt;
                                acc_a += c.a * wt;
                                w_sum += wt;
                            }
                        }

                        if w_sum > 1e-5 {
                            let inv = 1.0 / w_sum;
                            let dst = &mut img.pixels[row + x];
                            dst.r = dst.r * (1.0 - w_mask) + (acc_r * inv) * w_mask;
                            dst.g = dst.g * (1.0 - w_mask) + (acc_g * inv) * w_mask;
                            dst.b = dst.b * (1.0 - w_mask) + (acc_b * inv) * w_mask;
                            dst.a = dst.a * (1.0 - w_mask) + (acc_a * inv) * w_mask;
                        }
                    }
                }
            }
            PaintToolKind::BlurRedact => {
                // プライバシー秘匿モザイクブロック平均
                let src_snap = img.clone();
                let block = ((radius * 0.45).round() as usize).clamp(4, 24);
                let inner_r = radius * hardness;

                for y in by0..=by1 {
                    let py = y as f32 + 0.5;
                    let row = y * w;
                    let cy0 = (y / block) * block;
                    let cy1 = (cy0 + block).min(h);
                    for x in bx0..=bx1 {
                        let px = x as f32 + 0.5;
                        let (d, _) = dist_and_dir_to_polyline(px, py, &px_pts);
                        if d >= radius {
                            continue;
                        }
                        let w_mask = if d <= inner_r {
                            1.0
                        } else {
                            (1.0 - (d - inner_r) / (radius - inner_r).max(0.5)).clamp(0.0, 1.0)
                        } * opacity;
                        let cx0 = (x / block) * block;
                        let cx1 = (cx0 + block).min(w);
                        let mut sr = 0.0f32;
                        let mut sg = 0.0f32;
                        let mut sb = 0.0f32;
                        let mut sa = 0.0f32;
                        let mut cnt = 0.0f32;
                        for my in cy0..cy1 {
                            let mrow = my * w;
                            for mx in cx0..cx1 {
                                let sp = src_snap.pixels[mrow + mx];
                                sr += sp.r;
                                sg += sp.g;
                                sb += sp.b;
                                sa += sp.a;
                                cnt += 1.0;
                            }
                        }
                        let inv = 1.0 / cnt.max(1.0);
                        let dst = &mut img.pixels[row + x];
                        dst.r = dst.r * (1.0 - w_mask) + (sr * inv) * w_mask;
                        dst.g = dst.g * (1.0 - w_mask) + (sg * inv) * w_mask;
                        dst.b = dst.b * (1.0 - w_mask) + (sb * inv) * w_mask;
                        dst.a = dst.a * (1.0 - w_mask) + (sa * inv) * w_mask;
                    }
                }
            }
        }
    }
}

fn build_active_polyline(norm_pts: &[[f32; 2]], wf: f32, hf: f32, write_on: f32) -> Vec<[f32; 2]> {
    let full: Vec<[f32; 2]> = norm_pts
        .iter()
        .map(|&[nx, ny]| [nx.clamp(0.0, 1.0) * wf, ny.clamp(0.0, 1.0) * hf])
        .collect();
    if full.len() <= 1 || write_on >= 0.999 {
        return full;
    }
    let total_len: f32 = full
        .windows(2)
        .map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]))
        .sum();
    if total_len <= 1e-4 {
        return vec![full[0]];
    }
    let target = total_len * write_on.clamp(0.0, 1.0);
    let mut out = vec![full[0]];
    let mut acc = 0.0f32;
    for seg in full.windows(2) {
        let len = (seg[1][0] - seg[0][0]).hypot(seg[1][1] - seg[0][1]);
        if acc + len >= target {
            let t = if len > 1e-4 {
                ((target - acc) / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            out.push([
                seg[0][0] + (seg[1][0] - seg[0][0]) * t,
                seg[0][1] + (seg[1][1] - seg[0][1]) * t,
            ]);
            break;
        }
        out.push(seg[1]);
        acc += len;
    }
    out
}

/// 点からポリラインまでの最短距離と最寄りセグメントの単位接線ベクトルを返す
fn dist_and_dir_to_polyline(px: f32, py: f32, pts: &[[f32; 2]]) -> (f32, [f32; 2]) {
    if pts.len() == 1 {
        let d = (px - pts[0][0]).hypot(py - pts[0][1]);
        return (d, [1.0, 0.0]);
    }
    let mut best_d = f32::INFINITY;
    let mut best_dir = [1.0f32, 0.0f32];
    for seg in pts.windows(2) {
        let vx = seg[1][0] - seg[0][0];
        let vy = seg[1][1] - seg[0][1];
        let len_sq = vx * vx + vy * vy;
        let t = if len_sq > 1e-6 {
            (((px - seg[0][0]) * vx + (py - seg[0][1]) * vy) / len_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let cx = seg[0][0] + vx * t;
        let cy = seg[0][1] + vy * t;
        let d = (px - cx).hypot(py - cy);
        if d < best_d {
            best_d = d;
            let len = len_sq.sqrt();
            if len > 1e-4 {
                best_dir = [vx / len, vy / len];
            }
        }
    }
    (best_d, best_dir)
}

fn sample_bilinear(img: &LinearImage, fx: f32, fy: f32) -> LinearPremul {
    let max_x = (img.width as f32 - 1.0).max(0.0);
    let max_y = (img.height as f32 - 1.0).max(0.0);
    let x = (fx - 0.5).clamp(0.0, max_x);
    let y = (fy - 0.5).clamp(0.0, max_y);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(img.width as usize - 1);
    let y1 = (y0 + 1).min(img.height as usize - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let w = img.width as usize;

    let c00 = img.pixels[y0 * w + x0];
    let c10 = img.pixels[y0 * w + x1];
    let c01 = img.pixels[y1 * w + x0];
    let c11 = img.pixels[y1 * w + x1];

    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let r0 = lerp(c00.r, c10.r, tx);
    let r1 = lerp(c01.r, c11.r, tx);
    let g0 = lerp(c00.g, c10.g, tx);
    let g1 = lerp(c01.g, c11.g, tx);
    let b0 = lerp(c00.b, c10.b, tx);
    let b1 = lerp(c01.b, c11.b, tx);
    let a0 = lerp(c00.a, c10.a, tx);
    let a1 = lerp(c01.a, c11.a, tx);

    LinearPremul {
        r: lerp(r0, r1, ty),
        g: lerp(g0, g1, ty),
        b: lerp(b0, b1, ty),
        a: lerp(a0, a1, ty),
    }
}

fn rounded_rect_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r).max(0.0);
    let qy = (py - cy).abs() - (hy - r).max(0.0);
    let out = qx.max(0.0).hypot(qy.max(0.0));
    let inside = qx.max(qy).min(0.0);
    out + inside - r
}

fn sample_polyline_y(coords: &[[f32; 2]], x: f32) -> Option<f32> {
    for pair in coords.windows(2) {
        if x >= pair[0][0] && x <= pair[1][0] {
            let span = (pair[1][0] - pair[0][0]).max(1e-4);
            let t = ((x - pair[0][0]) / span).clamp(0.0, 1.0);
            return Some(pair[0][1] + (pair[1][1] - pair[0][1]) * t);
        }
    }
    None
}

fn draw_aa_segment(img: &mut LinearImage, a: [f32; 2], b: [f32; 2], thickness: f32, rgb: [f32; 3]) {
    let half = thickness * 0.5;
    let pad = half + 2.0;
    let x0 = ((a[0].min(b[0]) - pad).floor() as i32).clamp(0, img.width as i32 - 1) as u32;
    let x1 = ((a[0].max(b[0]) + pad).ceil() as i32).clamp(0, img.width as i32 - 1) as u32;
    let y0 = ((a[1].min(b[1]) - pad).floor() as i32).clamp(0, img.height as i32 - 1) as u32;
    let y1 = ((a[1].max(b[1]) + pad).ceil() as i32).clamp(0, img.height as i32 - 1) as u32;
    let vx = b[0] - a[0];
    let vy = b[1] - a[1];
    let len_sq = (vx * vx + vy * vy).max(1e-5);

    for y in y0..=y1 {
        let py = y as f32 + 0.5;
        let row = (y as usize) * (img.width as usize);
        for x in x0..=x1 {
            let px = x as f32 + 0.5;
            let t = (((px - a[0]) * vx + (py - a[1]) * vy) / len_sq).clamp(0.0, 1.0);
            let d = (px - (a[0] + vx * t)).hypot(py - (a[1] + vy * t));
            let cov = (half + 0.75 - d).clamp(0.0, 1.0);
            if cov > 0.001 {
                blend_over(
                    &mut img.pixels[row + x as usize],
                    rgb_to_linear_premul(rgb, cov),
                );
            }
        }
    }
}

fn draw_aa_disc(img: &mut LinearImage, cx: f32, cy: f32, r: f32, rgb: [f32; 3], alpha: f32) {
    let pad = r + 1.5;
    let x0 = ((cx - pad).floor() as i32).clamp(0, img.width as i32 - 1) as u32;
    let x1 = ((cx + pad).ceil() as i32).clamp(0, img.width as i32 - 1) as u32;
    let y0 = ((cy - pad).floor() as i32).clamp(0, img.height as i32 - 1) as u32;
    let y1 = ((cy + pad).ceil() as i32).clamp(0, img.height as i32 - 1) as u32;
    for y in y0..=y1 {
        let dy = y as f32 + 0.5 - cy;
        let row = (y as usize) * (img.width as usize);
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let cov = (r + 0.7 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0) * alpha;
            if cov > 0.001 {
                blend_over(
                    &mut img.pixels[row + x as usize],
                    rgb_to_linear_premul(rgb, cov),
                );
            }
        }
    }
}

fn draw_bitmap_text(
    img: &mut LinearImage,
    text: &str,
    ox: i32,
    oy: i32,
    scale: usize,
    color: LinearPremul,
) {
    let w = img.width as i32;
    let h = img.height as i32;
    let step = (6 * scale) as i32;
    for (ci, ch) in text.chars().take(36).enumerate() {
        let gx = ox + (ci as i32) * step;
        let bits = glyph_5x7(ch.to_ascii_uppercase());
        for (ry, &row_bits) in bits.iter().enumerate() {
            for rx in 0..5usize {
                if (row_bits >> (4 - rx)) & 1 == 1 {
                    for sy in 0..scale {
                        let py = oy + (ry * scale + sy) as i32;
                        if py < 0 || py >= h {
                            continue;
                        }
                        for sx in 0..scale {
                            let px = gx + (rx * scale + sx) as i32;
                            if px >= 0 && px < w {
                                blend_over(
                                    &mut img.pixels
                                        [(py as usize) * (img.width as usize) + px as usize],
                                    color,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn glyph_5x7(ch: char) -> [u8; 7] {
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
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x02, 0x04, 0x04, 0x08, 0x10, 0x00],
        '$' => [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::PaintStroke;

    #[test]
    fn data_chart_renders_all_four_kinds_deterministically_and_animates() {
        for &kind in ChartKind::ALL {
            let cfg = DataChartConfig::for_kind(kind);
            let early = render_data_chart(&cfg, 0.15, 240, 140);
            let full_a = render_data_chart(&cfg, 1.5, 240, 140);
            let full_b = render_data_chart(&cfg, 1.5, 240, 140);
            assert_eq!(
                full_a.pixels, full_b.pixels,
                "chart {kind:?} must be deterministic"
            );
            assert_ne!(
                early.pixels, full_a.pixels,
                "chart {kind:?} should animate between early and full time"
            );
            let visible_px = full_a.pixels.iter().filter(|p| p.a > 0.5).count();
            assert!(
                visible_px > 20_000,
                "chart {kind:?} should render visible graphic pixels, got {visible_px}"
            );
        }
    }

    #[test]
    fn vfx_paint_wire_erase_brush_clone_and_redact_transform_pixels() {
        // 1. ワイヤー消しテスト: 均一な青背景に描かれた白い水平ワイヤーを WireEraseInpaint で除去
        let bg = LinearPremul {
            r: 0.08,
            g: 0.20,
            b: 0.60,
            a: 1.0,
        };
        let mut wire_img = LinearImage::new(96, 64, bg);
        for x in 10..86 {
            wire_img.pixels[32 * 96 + x] = LinearPremul {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            };
        }
        let wire_cfg = VfxPaintConfig {
            enabled: true,
            animate_write_on: false,
            strokes: vec![PaintStroke::wire_erase(
                vec![[0.10, 0.50], [0.90, 0.50]],
                4.0,
            )],
        };
        apply_vfx_paint(&mut wire_img, &wire_cfg, 1.0);
        let erased_px = wire_img.pixels[32 * 96 + 48];
        assert!(
            (erased_px.r - bg.r).abs() < 0.08 && (erased_px.b - bg.b).abs() < 0.08,
            "wire should be cleanly inpainted back to blue background, got {erased_px:?}"
        );

        // 2. グローブラシ + Write-On アニメーション
        let mut brush_img = LinearImage::new(96, 64, LinearPremul::TRANSPARENT);
        let brush_cfg = VfxPaintConfig {
            enabled: true,
            animate_write_on: true,
            strokes: vec![PaintStroke::brush_glow(
                vec![[0.15, 0.5], [0.85, 0.5]],
                6.0,
                RgbaColor {
                    r: 1.0,
                    g: 0.2,
                    b: 0.4,
                    a: 1.0,
                },
            )],
        };
        let mut half_img = brush_img.clone();
        apply_vfx_paint(&mut half_img, &brush_cfg, 0.4);
        apply_vfx_paint(&mut brush_img, &brush_cfg, 1.0);
        assert!(
            half_img.pixels[32 * 96 + 75].a < 0.05,
            "write-on 40% shouldn't reach right end yet"
        );
        assert!(
            brush_img.pixels[32 * 96 + 75].a > 0.7,
            "write-on 100% should paint right end"
        );

        // 3. クローンスタンプ: 右側の赤い領域を中央へ複製
        let mut clone_img = LinearImage::new(96, 64, bg);
        for y in 20..44 {
            for x in 60..84 {
                clone_img.pixels[y * 96 + x] = LinearPremul {
                    r: 0.9,
                    g: 0.1,
                    b: 0.1,
                    a: 1.0,
                };
            }
        }
        let clone_cfg = VfxPaintConfig {
            enabled: true,
            animate_write_on: false,
            strokes: vec![PaintStroke::clone_stamp(
                vec![[0.40, 0.50]],
                8.0,
                [0.35, 0.0],
            )],
        };
        apply_vfx_paint(&mut clone_img, &clone_cfg, 1.0);
        let cloned_px = clone_img.pixels[32 * 96 + 38];
        assert!(
            cloned_px.r > 0.7 && cloned_px.b < 0.2,
            "clone stamp should transfer red patch, got {cloned_px:?}"
        );

        // 4. プライバシーぼかし / モザイク秘匿: 1px チェッカーボードを局所平均化
        let mut redact_img = LinearImage::new(96, 64, bg);
        for y in 0usize..64 {
            for x in 0usize..96 {
                let v = if (x + y).is_multiple_of(2) { 1.0 } else { 0.0 };
                redact_img.pixels[y * 96 + x] = LinearPremul {
                    r: v,
                    g: v,
                    b: v,
                    a: 1.0,
                };
            }
        }
        let redact_cfg = VfxPaintConfig {
            enabled: true,
            animate_write_on: false,
            strokes: vec![PaintStroke::blur_redact(vec![[0.5, 0.5]], 12.0)],
        };
        apply_vfx_paint(&mut redact_img, &redact_cfg, 1.0);
        let mid_px = redact_img.pixels[32 * 96 + 48];
        assert!(
            (0.35..0.65).contains(&mid_px.r),
            "mosaic redaction should average checkerboard near 0.5, got {mid_px:?}"
        );
    }
}
