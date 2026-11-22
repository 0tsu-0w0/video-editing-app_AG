//! v4.0 #35 & #36: オフライン・プロシージャル背景ジェネレーター描画エンジン、
//! 高クリック率ヒーローサムネイル自動スコア抽出、および放送コンプライアンス映像 QC メトリクス計測

use crate::color::{LinearImage, LinearPremul, linear_to_srgb};
use crate::graph::RenderGraph;
use crate::sprite::scale_linear_image;
use std::f32::consts::TAU;
use ve_core::{GenerativeBgConfig, GenerativeBgPreset, Project, RationalTime, RgbaColor, SceneId};

#[inline]
fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

#[inline]
fn lerp_color(a: RgbaColor, b: RgbaColor, t: f32) -> [f32; 3] {
    let k = t.clamp(0.0, 1.0);
    [
        lerp_f32(a.r, b.r, k),
        lerp_f32(a.g, b.g, k),
        lerp_f32(a.b, b.b, k),
    ]
}

#[inline]
fn hash2d(ix: i32, iy: i32, seed: u64) -> f32 {
    let mut h = (ix as i64 as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((iy as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
        ^ seed;
    h = (h ^ (h >> 30)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 27;
    ((h >> 40) as f32) / ((1u32 << 24) as f32)
}

#[inline]
fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let fx = x - ix as f32;
    let fy = y - iy as f32;
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash2d(ix, iy, seed);
    let b = hash2d(ix + 1, iy, seed);
    let c = hash2d(ix, iy + 1, seed);
    let d = hash2d(ix + 1, iy + 1, seed);
    let ab = a + (b - a) * ux;
    let cd = c + (d - c) * ux;
    ab + (cd - ab) * uy
}

fn fbm_noise(mut x: f32, mut y: f32, octaves: u32, seed: u64) -> f32 {
    let mut sum = 0.0f32;
    let mut amp = 0.5f32;
    let mut norm = 0.0f32;
    let n = octaves.clamp(1, 6);
    for o in 0..n {
        sum += amp * value_noise(x, y, seed.wrapping_add(u64::from(o) * 1013));
        norm += amp;
        x = x * 2.03 + 13.7;
        y = y * 2.01 - 7.3;
        amp *= 0.5;
    }
    if norm > 1e-6 { sum / norm } else { 0.0 }
}

/// 6 種のプロシージャル・アニメーション背景（オーロラ / シネマ玉ボケ / サイバー 3D グリッド /
/// リキッドマーブル / スターフィールドワープ / アブストラクト波）をオフライン決定論レンダリングする
pub fn render_generative_bg(
    cfg: &GenerativeBgConfig,
    local_time_sec: f32,
    out_w: u32,
    out_h: u32,
) -> LinearImage {
    let w = out_w.clamp(16, 4096);
    let h = out_h.clamp(16, 4096);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let t = local_time_sec * cfg.speed.clamp(0.0, 10.0);
    let sc = cfg.scale.clamp(0.1, 10.0);
    let octaves = cfg.complexity.clamp(1, 6);
    let c1 = cfg.primary_color;
    let c2 = cfg.secondary_color;
    let c3 = cfg.accent_color;
    let inv_w = 1.0 / (w as f32);
    let inv_h = 1.0 / (h as f32);
    let aspect = (w as f32) / (h as f32).max(1.0);

    // CinemaBokeh / StarfieldWarp 用に決定論的プリミティブ配列をフレーム前に事前計算
    let bokeh_count = (14 + cfg.complexity.clamp(1, 6) * 4) as usize;
    let bokeh_orbs: Vec<(f32, f32, f32, f32, bool)> =
        if cfg.preset == GenerativeBgPreset::CinemaBokeh {
            (0..bokeh_count)
                .map(|i| {
                    let bx = hash2d(i as i32, 1, cfg.seed);
                    let by = hash2d(i as i32, 2, cfg.seed);
                    let rad = (0.04 + hash2d(i as i32, 3, cfg.seed) * 0.14) / sc.sqrt();
                    let spd = 0.05 + hash2d(i as i32, 4, cfg.seed) * 0.14;
                    let phase = hash2d(i as i32, 5, cfg.seed) * TAU;
                    let cx = (bx + (t * spd + phase).sin() * 0.08).rem_euclid(1.0);
                    let cy = (by - t * spd * 0.35).rem_euclid(1.0);
                    let bright = 0.35 + 0.65 * (0.5 + 0.5 * (t * 1.4 + phase).sin());
                    (cx, cy, rad, bright, i.is_multiple_of(2))
                })
                .collect()
        } else {
            Vec::new()
        };

    let star_count = (48 + cfg.complexity.clamp(1, 6) * 20) as usize;
    let stars: Vec<(f32, f32, f32, bool)> = if cfg.preset == GenerativeBgPreset::StarfieldWarp {
        (0..star_count)
            .map(|i| {
                let ang = hash2d(i as i32, 11, cfg.seed) * TAU;
                let base_r = 0.08 + hash2d(i as i32, 12, cfg.seed) * 0.9;
                let z = (hash2d(i as i32, 13, cfg.seed) - t * 0.45 * sc).rem_euclid(1.0) + 0.08;
                let proj = (base_r / z) * 0.42;
                let sx = 0.5 + ang.cos() * proj / aspect;
                let sy = 0.5 + ang.sin() * proj;
                let size = (0.004 + (1.0 - z).max(0.0) * 0.012).clamp(0.003, 0.025);
                (sx, sy, size, i.is_multiple_of(3))
            })
            .collect()
    } else {
        Vec::new()
    };

    for y in 0..h {
        let v = (y as f32 + 0.5) * inv_h;
        let row_offset = (y * w) as usize;
        for x in 0..w {
            let u = (x as f32 + 0.5) * inv_w;
            let srgb = match cfg.preset {
                GenerativeBgPreset::AuroraBorealis => {
                    let sky = lerp_color(
                        c1,
                        RgbaColor {
                            r: c1.r * 0.45,
                            g: c1.g * 0.65,
                            b: (c1.b * 1.25).min(1.0),
                            a: 1.0,
                        },
                        1.0 - v,
                    );
                    let wave1 = (u * 3.8 * sc + t * 0.65 + (v * 2.4).sin()).sin() * 0.16;
                    let n = fbm_noise(
                        u * 4.5 * sc + t * 0.25,
                        v * 2.2 - t * 0.12,
                        octaves,
                        cfg.seed,
                    );
                    let ribbon1 = (1.0 - ((v - (0.36 + wave1 + (n - 0.5) * 0.22)).abs() * 4.6))
                        .clamp(0.0, 1.0)
                        .powf(1.8);
                    let wave2 = (u * 5.2 * sc - t * 0.45 + 1.4).cos() * 0.14;
                    let ribbon2 = (1.0 - ((v - (0.52 + wave2 + (n - 0.5) * 0.18)).abs() * 5.2))
                        .clamp(0.0, 1.0)
                        .powf(2.0);
                    [
                        (sky[0] + c2.r * ribbon1 * 0.95 + c3.r * ribbon2 * 0.80).clamp(0.0, 1.0),
                        (sky[1] + c2.g * ribbon1 * 0.95 + c3.g * ribbon2 * 0.80).clamp(0.0, 1.0),
                        (sky[2] + c2.b * ribbon1 * 0.95 + c3.b * ribbon2 * 0.80).clamp(0.0, 1.0),
                    ]
                }
                GenerativeBgPreset::CinemaBokeh => {
                    let mut rgb = lerp_color(c1, c3, v * 0.25);
                    for &(cx, cy, rad, bright, use_sec) in &bokeh_orbs {
                        let dx = (u - cx) * aspect;
                        let dy = v - cy;
                        let dist = dx.hypot(dy);
                        if dist < rad {
                            let edge = ((rad - dist) / (rad * 0.28)).clamp(0.0, 1.0);
                            let rim = 0.72 + 0.28 * (dist / rad).powf(2.0);
                            let contrib = edge * rim * bright * 0.48;
                            let orb_c = if use_sec { c2 } else { c3 };
                            rgb[0] = (rgb[0] + orb_c.r * contrib).min(1.0);
                            rgb[1] = (rgb[1] + orb_c.g * contrib).min(1.0);
                            rgb[2] = (rgb[2] + orb_c.b * contrib).min(1.0);
                        }
                    }
                    rgb
                }
                GenerativeBgPreset::CyberGrid3D => {
                    let horizon = 0.46;
                    if v < horizon {
                        // サイバー夕景空＋太陽グロー
                        let sky_t = (v / horizon).clamp(0.0, 1.0);
                        let mut sky = lerp_color(c1, c2, sky_t * 0.55);
                        let dx = (u - 0.5) * aspect;
                        let dy = v - (horizon - 0.14);
                        let sun_d = dx.hypot(dy);
                        if sun_d < 0.20 {
                            let stripes = if v > horizon - 0.14 && ((v * 52.0).sin() > 0.35) {
                                0.35
                            } else {
                                1.0
                            };
                            let sun_glow = ((0.20 - sun_d) / 0.04).clamp(0.0, 1.0) * stripes;
                            sky[0] = lerp_f32(sky[0], c2.r.max(0.95), sun_glow);
                            sky[1] = lerp_f32(sky[1], c3.g * 0.8 + 0.2, sun_glow * 0.65);
                            sky[2] = lerp_f32(sky[2], c2.b, sun_glow);
                        }
                        sky
                    } else {
                        // パースペクティブ 3D グリッド床
                        let depth = ((v - horizon) / (1.0 - horizon)).max(0.015);
                        let gx = ((u - 0.5) / depth) * 4.5 * sc;
                        let gz = (1.15 / depth) * sc + t * 2.2;
                        let lx = (gx.fract().abs() - 0.5).abs();
                        let lz = (gz.fract().abs() - 0.5).abs();
                        let line_x = ((0.46 - lx) / 0.05).clamp(0.0, 1.0);
                        let line_z = ((0.46 - lz) / 0.05).clamp(0.0, 1.0);
                        let grid = (1.0 - line_x.min(line_z)).clamp(0.0, 1.0) * depth.sqrt();
                        let glow = ((0.06 - (v - horizon).abs()) / 0.06).clamp(0.0, 1.0);
                        [
                            (c1.r + c3.r * grid * 0.95 + c2.r * glow * 0.85).clamp(0.0, 1.0),
                            (c1.g + c3.g * grid * 0.95 + c2.g * glow * 0.85).clamp(0.0, 1.0),
                            (c1.b + c3.b * grid * 0.95 + c2.b * glow * 0.85).clamp(0.0, 1.0),
                        ]
                    }
                }
                GenerativeBgPreset::LiquidMarble => {
                    let px = u * 3.4 * sc;
                    let py = v * 3.4 * sc;
                    let wx = fbm_noise(px + t * 0.28, py - t * 0.18, octaves, cfg.seed);
                    let wy = fbm_noise(
                        px - t * 0.22 + 5.2,
                        py + t * 0.25 + 1.3,
                        octaves,
                        cfg.seed ^ 0x55AA,
                    );
                    let swirl =
                        ((px + wx * 3.2 + wy * 2.1 + t * 0.35).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
                    let m1 = lerp_color(c1, c2, swirl);
                    let vein = (wy * 1.4 - 0.2).clamp(0.0, 1.0);
                    [
                        lerp_f32(m1[0], c3.r, vein * 0.65),
                        lerp_f32(m1[1], c3.g, vein * 0.65),
                        lerp_f32(m1[2], c3.b, vein * 0.65),
                    ]
                }
                GenerativeBgPreset::StarfieldWarp => {
                    let neb = fbm_noise(u * 2.5 * sc + t * 0.08, v * 2.5 * sc, octaves, cfg.seed);
                    let mut rgb = [
                        (c1.r + c3.r * neb * 0.26).clamp(0.0, 1.0),
                        (c1.g + c3.g * neb * 0.22).clamp(0.0, 1.0),
                        (c1.b + c3.b * neb * 0.34).clamp(0.0, 1.0),
                    ];
                    for &(sx, sy, size, accent) in &stars {
                        let d = ((u - sx) * aspect).hypot(v - sy);
                        if d < size * 2.2 {
                            let core = (1.0 - d / (size * 2.2)).clamp(0.0, 1.0).powf(2.2);
                            let sc_col = if accent { c3 } else { c2 };
                            rgb[0] = (rgb[0] + sc_col.r * core).min(1.0);
                            rgb[1] = (rgb[1] + sc_col.g * core).min(1.0);
                            rgb[2] = (rgb[2] + sc_col.b * core).min(1.0);
                        }
                    }
                    rgb
                }
                GenerativeBgPreset::AbstractWaves => {
                    let mut rgb = lerp_color(c1, c2, v * 0.85);
                    let bands = cfg.complexity.clamp(2, 6);
                    for b in 0..bands {
                        let bf = (b as f32 + 1.0) / (bands as f32 + 1.0);
                        let wave_y = 0.20
                            + bf * 0.62
                            + (u * (3.2 + bf * 2.5) * sc + t * (0.9 + bf * 0.6) + bf * 4.0).sin()
                                * 0.075;
                        let mask = ((v - wave_y) * 18.0 + 0.5).clamp(0.0, 1.0);
                        let layer_col = lerp_color(c2, c3, bf);
                        rgb[0] = lerp_f32(rgb[0], layer_col[0], mask * 0.42);
                        rgb[1] = lerp_f32(rgb[1], layer_col[1], mask * 0.42);
                        rgb[2] = lerp_f32(rgb[2], layer_col[2], mask * 0.42);
                    }
                    rgb
                }
            };
            img.pixels[row_offset + x as usize] = LinearPremul::from_srgb_straight(RgbaColor {
                r: srgb[0],
                g: srgb[1],
                b: srgb[2],
                a: 1.0,
            });
        }
    }
    img
}

/// `LinearImage` を PNG バイト列へエンコードする（ヒーローサムネイル書き出し＆HTML5 レビュー用）
pub fn encode_linear_image_png_bytes(img: &LinearImage) -> Result<Vec<u8>, String> {
    let rgba = img.to_rgba8_srgb();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width.max(1), img.height.max(1));
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| format!("failed to write png header: {e}"))?;
        writer
            .write_image_data(&rgba)
            .map_err(|e| format!("failed to encode png pixels: {e}"))?;
    }
    Ok(out)
}

/// v4.0 #36: フレームのサムネイル魅力度スコア（0..100）を算出する
/// シャープネス（エッジ勾配）・鮮やかさ（彩度広がり）・輝度コントラスト・適度な明るさから総合評価
pub fn score_frame_thumbnail_attractiveness(img: &LinearImage) -> f32 {
    let w = img.width as usize;
    let h = img.height as usize;
    if w < 4 || h < 4 || img.pixels.len() < w * h {
        return 0.0;
    }

    let step_x = (w / 64).max(1);
    let step_y = (h / 36).max(1);
    let mut luma_sum = 0.0f64;
    let mut luma_sq_sum = 0.0f64;
    let mut sat_sum = 0.0f64;
    let mut edge_sum = 0.0f64;
    let mut count = 0usize;

    let sample_luma_sat = |idx: usize| -> (f32, f32) {
        let p = img.pixels[idx];
        let r = linear_to_srgb(p.r.max(0.0));
        let g = linear_to_srgb(p.g.max(0.0));
        let b = linear_to_srgb(p.b.max(0.0));
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        (luma, max_c - min_c)
    };

    let mut y = 1;
    while y + 1 < h {
        let mut x = 1;
        while x + 1 < w {
            let idx = y * w + x;
            let (l, sat) = sample_luma_sat(idx);
            let (lr, _) = sample_luma_sat(idx + 1);
            let (ld, _) = sample_luma_sat(idx + w);
            let grad = (lr - l).abs() + (ld - l).abs();

            luma_sum += f64::from(l);
            luma_sq_sum += f64::from(l * l);
            sat_sum += f64::from(sat);
            edge_sum += f64::from(grad);
            count += 1;
            x += step_x;
        }
        y += step_y;
    }
    if count == 0 {
        return 0.0;
    }

    let n = count as f64;
    let mean_luma = (luma_sum / n) as f32;
    let variance = (luma_sq_sum / n - (luma_sum / n).powi(2)).max(0.0) as f32;
    let contrast = variance.sqrt();
    let mean_sat = (sat_sum / n) as f32;
    let mean_edge = (edge_sum / n) as f32;

    // 暗すぎる／真っ白すぎるフレームは減点（適正平均輝度 0.25..0.75）
    let exposure_quality = 1.0 - ((mean_luma - 0.48).abs() * 2.1).clamp(0.0, 0.92);
    let sharpness_score = (mean_edge * 220.0).clamp(0.0, 40.0);
    let color_score = (mean_sat * 65.0).clamp(0.0, 30.0);
    let contrast_score = (contrast * 110.0).clamp(0.0, 30.0);

    ((sharpness_score + color_score + contrast_score) * exposure_quality).clamp(0.0, 100.0)
}

/// タイムライン全体から最もクリック率・視認性が高いヒーローサムネイル候補フレームを自動抽出し、`1280x720` に整えて返す
pub fn select_best_thumbnail_frame(
    graph: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    sample_count: usize,
) -> Option<(f64, f32, LinearImage)> {
    let scene = project.scene(scene_id)?;
    let dur = scene.total_duration().as_seconds().max(0.5);
    let n = sample_count.clamp(3, 32);

    let mut best_t = 0.0f64;
    let mut best_score = -1.0f32;
    for i in 0..n {
        let frac = (i as f64 + 0.5) / (n as f64);
        let t_sec = dur * frac;
        let t = RationalTime::from_seconds(t_sec);
        if let Some(img) = graph.render_project_scene_scaled(project, scene_id, t, 320, 180, false)
        {
            let score = score_frame_thumbnail_attractiveness(&img);
            if score > best_score {
                best_score = score;
                best_t = t_sec;
            }
        }
    }

    let hero_raw =
        graph.render_project_scene(project, scene_id, RationalTime::from_seconds(best_t), false)?;
    let hero = if hero_raw.width == 1280 && hero_raw.height == 720 {
        hero_raw
    } else {
        scale_linear_image(&hero_raw, 1280, 720)
    };
    Some((best_t, best_score.max(0.0), hero))
}

/// v4.0 #36: シーン全区間をサンプリングして放送 QC 用映像メトリクス
/// `(peak_luma_ire, clipped_highlight_pct, crushed_shadow_pct, max_frame_luma_jump_ire)` を計測する
pub fn measure_scene_video_qc_metrics(
    graph: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    sample_count: usize,
) -> (f32, f32, f32, f32) {
    let Some(scene) = project.scene(scene_id) else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    let dur = scene.total_duration().as_seconds().max(0.5);
    let n = sample_count.clamp(4, 36);

    let mut peak_luma_ire = 0.0f32;
    let mut clipped_pixels = 0usize;
    let mut crushed_pixels = 0usize;
    let mut total_pixels = 0usize;
    let mut prev_avg_ire: Option<f32> = None;
    let mut max_jump_ire = 0.0f32;

    for i in 0..n {
        let t = RationalTime::from_seconds(dur * (i as f64) / ((n - 1).max(1) as f64));
        if let Some(img) = graph.render_project_scene_scaled(project, scene_id, t, 240, 135, false)
        {
            let mut sum_ire = 0.0f64;
            for px in &img.pixels {
                let r = linear_to_srgb(px.r.max(0.0));
                let g = linear_to_srgb(px.g.max(0.0));
                let b = linear_to_srgb(px.b.max(0.0));
                let ire = (0.2126 * r + 0.7152 * g + 0.0722 * b) * 100.0;
                peak_luma_ire = peak_luma_ire.max(ire);
                if ire >= 99.0 {
                    clipped_pixels += 1;
                }
                if ire <= 1.0 {
                    crushed_pixels += 1;
                }
                sum_ire += f64::from(ire);
            }
            let frame_px = img.pixels.len().max(1);
            let avg_ire = (sum_ire / (frame_px as f64)) as f32;
            if let Some(prev) = prev_avg_ire {
                max_jump_ire = max_jump_ire.max((avg_ire - prev).abs());
            }
            prev_avg_ire = Some(avg_ire);
            total_pixels += frame_px;
        }
    }

    let denom = total_pixels.max(1) as f32;
    let clipped_pct = (clipped_pixels as f32 / denom) * 100.0;
    let crushed_pct = (crushed_pixels as f32 / denom) * 100.0;
    (peak_luma_ire, clipped_pct, crushed_pct, max_jump_ire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_generative_bg_presets_render_deterministically_and_animate() {
        for &preset in GenerativeBgPreset::ALL {
            let cfg = GenerativeBgPreset::for_preset(preset);
            let f0_a = render_generative_bg(&cfg, 0.25, 64, 36).to_rgba8_srgb();
            let f0_b = render_generative_bg(&cfg, 0.25, 64, 36).to_rgba8_srgb();
            assert_eq!(f0_a, f0_b, "preset {:?} must be deterministic", preset);

            let f1 = render_generative_bg(&cfg, 1.85, 64, 36).to_rgba8_srgb();
            let changed = f0_a.iter().zip(f1.iter()).filter(|(a, b)| a != b).count();
            assert!(
                changed > 40,
                "preset {:?} must visibly animate over time: changed={changed}",
                preset
            );
        }
    }

    #[test]
    fn thumbnail_scorer_prefers_vivid_sharp_frame_over_flat_gray_and_encodes_png() {
        let flat = LinearImage::new(
            64,
            36,
            LinearPremul::from_srgb_straight(RgbaColor {
                r: 0.45,
                g: 0.45,
                b: 0.45,
                a: 1.0,
            }),
        );
        let vivid = render_generative_bg(
            &GenerativeBgPreset::for_preset(GenerativeBgPreset::CyberGrid3D),
            0.5,
            64,
            36,
        );
        let s_flat = score_frame_thumbnail_attractiveness(&flat);
        let s_vivid = score_frame_thumbnail_attractiveness(&vivid);
        assert!(
            s_vivid > s_flat + 10.0,
            "vivid cyber grid ({s_vivid}) should score much higher than flat gray ({s_flat})"
        );

        let png = encode_linear_image_png_bytes(&vivid).expect("png encode should succeed");
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }
}
