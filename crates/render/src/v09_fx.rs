//! v0.9 #16: 高度な 2D/3D 合成レンダラー
//!
//! - 決定論的シード 2D/3D パーティクルエミッター（紙吹雪・雪・火花・光粒・集中線・スピードライン）
//! - 自由ベジェベクターシェイプ（単色/線形グラデーション塗り・破線ストローク・トリムパスアニメーション）
//! - 3D カメラ空間のライティング＆ドロップシャドウキャッチャー補助関数

use crate::color::{LinearImage, LinearPremul};
use ve_core::{
    MaskVertex, ParticleConfig, ParticlePreset, RgbaColor, VectorFillMode, VectorShapeConfig,
};

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn rand01(seed: u64, salt: u64) -> f32 {
    let h = splitmix64(seed ^ salt.wrapping_mul(0x517C_C1B7_2722_0A95));
    ((h >> 40) as f32) * (1.0 / ((1u32 << 24) as f32))
}

fn lerp_color(a: RgbaColor, b: RgbaColor, t: f32) -> RgbaColor {
    let u = t.clamp(0.0, 1.0);
    RgbaColor {
        r: a.r + (b.r - a.r) * u,
        g: a.g + (b.g - a.g) * u,
        b: a.b + (b.b - a.b) * u,
        a: a.a + (b.a - a.a) * u,
    }
}

fn blend_pixel_over(img: &mut LinearImage, x: i32, y: i32, src: LinearPremul) {
    if x < 0 || y < 0 || x >= img.width as i32 || y >= img.height as i32 || src.a <= 1e-5 {
        return;
    }
    let idx = (y as u32 * img.width + x as u32) as usize;
    img.pixels[idx] = src.over(img.pixels[idx]);
}

fn draw_soft_disc(
    img: &mut LinearImage,
    cx: f32,
    cy: f32,
    radius: f32,
    color: RgbaColor,
    softness: f32,
) {
    if radius <= 0.3 || color.a <= 1e-4 {
        return;
    }
    let min_x = (cx - radius - 1.0).floor().max(0.0) as i32;
    let max_x = (cx + radius + 1.0).ceil().min((img.width - 1) as f32) as i32;
    let min_y = (cy - radius - 1.0).floor().max(0.0) as i32;
    let max_y = (cy + radius + 1.0).ceil().min((img.height - 1) as f32) as i32;
    let base = LinearPremul::from_srgb_straight(color);
    let edge = (radius * softness.clamp(0.05, 1.0)).max(1.0);

    for py in min_y..=max_y {
        let dy = py as f32 + 0.5 - cy;
        for px in min_x..=max_x {
            let dx = px as f32 + 0.5 - cx;
            let dist = dx.hypot(dy);
            if dist <= radius {
                let cov = ((radius - dist) / edge).clamp(0.0, 1.0);
                let cov = cov * cov * (3.0 - 2.0 * cov);
                blend_pixel_over(
                    img,
                    px,
                    py,
                    LinearPremul {
                        r: base.r * cov,
                        g: base.g * cov,
                        b: base.b * cov,
                        a: base.a * cov,
                    },
                );
            }
        }
    }
}

fn draw_rotated_rect(
    img: &mut LinearImage,
    cx: f32,
    cy: f32,
    half_w: f32,
    half_h: f32,
    angle_rad: f32,
    color: RgbaColor,
) {
    if half_w <= 0.3 || half_h <= 0.3 || color.a <= 1e-4 {
        return;
    }
    let reach = half_w.hypot(half_h) + 1.5;
    let min_x = (cx - reach).floor().max(0.0) as i32;
    let max_x = (cx + reach).ceil().min((img.width - 1) as f32) as i32;
    let min_y = (cy - reach).floor().max(0.0) as i32;
    let max_y = (cy + reach).ceil().min((img.height - 1) as f32) as i32;
    let (sin_a, cos_a) = angle_rad.sin_cos();
    let base = LinearPremul::from_srgb_straight(color);

    for py in min_y..=max_y {
        let dy = py as f32 + 0.5 - cy;
        for px in min_x..=max_x {
            let dx = px as f32 + 0.5 - cx;
            let lx = dx * cos_a + dy * sin_a;
            let ly = -dx * sin_a + dy * cos_a;
            let dist_x = lx.abs() - half_w;
            let dist_y = ly.abs() - half_h;
            let sdf = dist_x.max(dist_y);
            let cov = (0.5 - sdf).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_pixel_over(
                    img,
                    px,
                    py,
                    LinearPremul {
                        r: base.r * cov,
                        g: base.g * cov,
                        b: base.b * cov,
                        a: base.a * cov,
                    },
                );
            }
        }
    }
}

fn draw_capsule_segment(
    img: &mut LinearImage,
    p0: [f32; 2],
    p1: [f32; 2],
    radius: f32,
    color: RgbaColor,
) {
    if radius <= 0.1 || color.a <= 1e-4 {
        return;
    }
    let pad = radius + 1.5;
    let min_x = (p0[0].min(p1[0]) - pad).floor().max(0.0) as i32;
    let max_x = (p0[0].max(p1[0]) + pad).ceil().min((img.width - 1) as f32) as i32;
    let min_y = (p0[1].min(p1[1]) - pad).floor().max(0.0) as i32;
    let max_y = (p0[1].max(p1[1]) + pad).ceil().min((img.height - 1) as f32) as i32;
    let vx = p1[0] - p0[0];
    let vy = p1[1] - p0[1];
    let len_sq = vx * vx + vy * vy;
    let base = LinearPremul::from_srgb_straight(color);

    for py in min_y..=max_y {
        let fy = py as f32 + 0.5;
        for px in min_x..=max_x {
            let fx = px as f32 + 0.5;
            let t = if len_sq > 1e-6 {
                (((fx - p0[0]) * vx + (fy - p0[1]) * vy) / len_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let qx = p0[0] + vx * t;
            let qy = p0[1] + vy * t;
            let dist = (fx - qx).hypot(fy - qy);
            let cov = (radius + 0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_pixel_over(
                    img,
                    px,
                    py,
                    LinearPremul {
                        r: base.r * cov,
                        g: base.g * cov,
                        b: base.b * cov,
                        a: base.a * cov,
                    },
                );
            }
        }
    }
}

/// 決定論的シード 2D/3D パーティクルエミッターを指定時刻で直接評価・描画する。
/// シークしても過去フレーム状態に依存せず完全に同一ピクセルを再現する。
pub fn render_particle_emitter(
    config: &ParticleConfig,
    local_time_sec: f32,
    width: u32,
    height: u32,
) -> LinearImage {
    let w = width.max(1);
    let h = height.max(1);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let wf = w as f32;
    let hf = h as f32;
    let count = config.count.clamp(1, 600);
    let life = config.lifetime_sec.clamp(0.1, 30.0);
    let t = local_time_sec.max(0.0);
    let scale_ref = (hf / 1080.0).max(0.25);

    match config.preset {
        ParticlePreset::Confetti => {
            for i in 0..count {
                let s = config.seed.wrapping_add(u64::from(i) * 31);
                let phase = rand01(s, 1);
                let age = (t + phase * life).rem_euclid(life);
                let u = age / life;
                let depth = 1.0 - config.depth_3d * rand01(s, 2) * 0.65;

                let x0 = rand01(s, 3) * wf;
                let vx = (rand01(s, 4) - 0.5) * config.speed * 0.9 + config.wind_x;
                let vy0 = (0.35 + rand01(s, 5) * 0.75) * config.speed * scale_ref;
                let sway = ((age * (2.5 + rand01(s, 6) * 3.0))
                    + rand01(s, 7) * std::f32::consts::TAU)
                    .sin()
                    * (28.0 * scale_ref);

                let px = (x0 + vx * age * scale_ref + sway).rem_euclid(wf + 80.0) - 40.0;
                let py = -30.0
                    + (vy0 * age + 0.5 * config.gravity * scale_ref * age * age)
                        .rem_euclid(hf + 80.0);

                let spin = age * (3.0 + rand01(s, 8) * 6.0) + rand01(s, 9) * std::f32::consts::TAU;
                let tumble = (age * (4.0 + rand01(s, 10) * 5.0)).cos().abs().max(0.22);
                let sz = config.size_px * scale_ref * depth;
                let mut col =
                    lerp_color(config.color_primary, config.color_secondary, rand01(s, 11));
                // 3色目のアクセントを混在させて紙吹雪らしい華やかさを出す
                if rand01(s, 12) > 0.72 {
                    col = RgbaColor {
                        r: 1.0,
                        g: 0.88,
                        b: 0.25,
                        a: col.a,
                    };
                }
                col.a *= (1.0 - (u - 0.85).max(0.0) / 0.15).clamp(0.0, 1.0) * (0.45 + 0.55 * depth);
                draw_rotated_rect(&mut img, px, py, sz * 0.65, sz * 0.38 * tumble, spin, col);
            }
        }
        ParticlePreset::Snow => {
            for i in 0..count {
                let s = config.seed.wrapping_add(u64::from(i) * 43);
                let phase = rand01(s, 1);
                let age = (t + phase * life).rem_euclid(life);
                let depth = 1.0 - config.depth_3d * rand01(s, 2) * 0.7;

                let x0 = rand01(s, 3) * wf;
                let fall_speed = (0.55 + rand01(s, 4) * 0.9) * config.speed * scale_ref * depth;
                let sway_amp = (16.0 + rand01(s, 5) * 32.0) * scale_ref * depth;
                let sway_freq = 0.8 + rand01(s, 6) * 1.4;
                let px = (x0
                    + (config.wind_x * age * scale_ref)
                    + (age * sway_freq + rand01(s, 7) * std::f32::consts::TAU).sin() * sway_amp)
                    .rem_euclid(wf + 40.0)
                    - 20.0;
                let py = (rand01(s, 8) * hf + fall_speed * age + 0.2 * config.gravity * age)
                    .rem_euclid(hf + 40.0)
                    - 20.0;

                let r = (config.size_px * scale_ref * (0.45 + 0.75 * depth)).max(1.5);
                let mut col =
                    lerp_color(config.color_primary, config.color_secondary, rand01(s, 9));
                col.a *= (0.30 + 0.70 * depth).clamp(0.0, 1.0);
                draw_soft_disc(&mut img, px, py, r, col, 0.75);
            }
        }
        ParticlePreset::Sparks => {
            let origin_x = wf * 0.5;
            let origin_y = hf * 0.62;
            let spread_rad = config.spread_deg.clamp(5.0, 360.0).to_radians();
            for i in 0..count {
                let s = config.seed.wrapping_add(u64::from(i) * 59);
                let cycle = ((t / life) + rand01(s, 1)).floor() as u64;
                let age = (t + rand01(s, 1) * life).rem_euclid(life);
                let u = (age / life).clamp(0.0, 1.0);
                let sc = splitmix64(s ^ cycle.wrapping_mul(0x9E37_79B9));

                let angle = -std::f32::consts::FRAC_PI_2 + (rand01(sc, 2) - 0.5) * spread_rad;
                let spd = (0.35 + rand01(sc, 3) * 0.95) * config.speed * scale_ref;
                let vx = angle.cos() * spd + config.wind_x * scale_ref;
                let vy = angle.sin() * spd;
                let grav = config.gravity * scale_ref;

                let px = origin_x + vx * age;
                let py = origin_y + vy * age + 0.5 * grav * age * age;
                let dt_trail = 0.045;
                let prev_age = (age - dt_trail).max(0.0);
                let tx = origin_x + vx * prev_age;
                let ty = origin_y + vy * prev_age + 0.5 * grav * prev_age * prev_age;

                let mut col = lerp_color(config.color_primary, config.color_secondary, u);
                col.a *= (1.0 - u * u).clamp(0.0, 1.0);
                let rad = (config.size_px * scale_ref * 0.45 * (1.0 - 0.5 * u)).max(1.0);
                draw_capsule_segment(&mut img, [tx, ty], [px, py], rad, col);
            }
        }
        ParticlePreset::LightOrbs => {
            for i in 0..count {
                let s = config.seed.wrapping_add(u64::from(i) * 71);
                let depth = 1.0 - config.depth_3d * rand01(s, 1) * 0.72;
                let phase = rand01(s, 2) * life;
                let age = (t + phase).rem_euclid(life);
                let u = age / life;
                let pulse = (u * std::f32::consts::PI).sin().clamp(0.0, 1.0);

                let base_x = rand01(s, 3) * wf;
                let base_y = rand01(s, 4) * hf;
                let drift_x = (config.wind_x + (rand01(s, 5) - 0.5) * config.speed) * scale_ref;
                let drift_y = (config.gravity + (rand01(s, 6) - 0.5) * config.speed) * scale_ref;
                let px = (base_x + drift_x * age * depth).rem_euclid(wf);
                let py = (base_y + drift_y * age * depth).rem_euclid(hf);

                let rad = (config.size_px * scale_ref * (0.4 + 0.9 * depth)).max(3.0);
                let mut col =
                    lerp_color(config.color_primary, config.color_secondary, rand01(s, 7));
                col.a *= pulse * (0.3 + 0.7 * depth);
                draw_soft_disc(&mut img, px, py, rad, col, 0.95);
            }
        }
        ParticlePreset::SpeedLines => {
            // 漫画・アニメ風 集中線（フレームごとに軽快に明滅・伸縮）
            let cx = wf * 0.5;
            let cy = hf * 0.5;
            let outer_r = cx.hypot(cy) * 1.08;
            let step_tick = (t * config.speed.max(1.0)).floor() as u64;
            for i in 0..count {
                let base_angle = (i as f32 / count as f32) * std::f32::consts::TAU;
                let s = splitmix64(config.seed ^ (u64::from(i) * 97) ^ step_tick.wrapping_mul(131));
                let angle = base_angle
                    + (rand01(s, 1) - 0.5) * (std::f32::consts::TAU / count as f32) * 0.75;
                let inner_frac = 0.34 + rand01(s, 2) * 0.32;
                let inner_r = outer_r * inner_frac;
                let p0 = [cx + angle.cos() * inner_r, cy + angle.sin() * inner_r];
                let p1 = [cx + angle.cos() * outer_r, cy + angle.sin() * outer_r];
                let col = if rand01(s, 3) > 0.25 {
                    config.color_primary
                } else {
                    config.color_secondary
                };
                let thick = (config.size_px * scale_ref * (0.35 + rand01(s, 4) * 0.85)).max(1.0);
                draw_capsule_segment(&mut img, p0, p1, thick * 0.5, col);
            }
        }
        ParticlePreset::ActionLines => {
            // 横方向スピードライン（疾走感・強調演出）
            let spd = config.speed.max(120.0) * scale_ref;
            for i in 0..count {
                let s = config.seed.wrapping_add(u64::from(i) * 109);
                let depth = 1.0 - config.depth_3d * rand01(s, 1) * 0.55;
                let y = rand01(s, 2) * hf;
                let line_len = (160.0 + rand01(s, 3) * 340.0) * scale_ref * depth;
                let travel = wf + line_len * 2.0;
                let x_head = (rand01(s, 4) * travel + t * spd * (0.65 + 0.7 * depth))
                    .rem_euclid(travel)
                    - line_len;
                let mut col =
                    lerp_color(config.color_primary, config.color_secondary, rand01(s, 5));
                col.a *= 0.45 + 0.55 * depth;
                let rad = (config.size_px * scale_ref * 0.45 * depth).max(1.0);
                draw_capsule_segment(&mut img, [x_head - line_len, y], [x_head, y], rad, col);
            }
        }
    }

    img
}

/// 自由ベジェベクターシェイプのポリライン展開（閉パス・開パス両対応）
fn flatten_vector_path(
    vertices: &[MaskVertex],
    closed: bool,
    width: f32,
    height: f32,
    samples_per_segment: usize,
) -> Vec<[f32; 2]> {
    let n = vertices.len();
    if n < 2 {
        return Vec::new();
    }
    let steps = samples_per_segment.clamp(4, 48);
    let seg_count = if closed { n } else { n - 1 };
    let mut pts = Vec::with_capacity(seg_count * steps + 1);

    for i in 0..seg_count {
        let v0 = vertices[i];
        let v1 = vertices[(i + 1) % n];
        let p0 = [v0.x * width, v0.y * height];
        let p1 = [
            (v0.x + v0.out_handle[0]) * width,
            (v0.y + v0.out_handle[1]) * height,
        ];
        let p2 = [
            (v1.x + v1.in_handle[0]) * width,
            (v1.y + v1.in_handle[1]) * height,
        ];
        let p3 = [v1.x * width, v1.y * height];
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let u = 1.0 - t;
            let b0 = u * u * u;
            let b1 = 3.0 * u * u * t;
            let b2 = 3.0 * u * t * t;
            let b3 = t * t * t;
            pts.push([
                b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
                b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
            ]);
        }
    }
    let last = if closed { vertices[0] } else { vertices[n - 1] };
    pts.push([last.x * width, last.y * height]);
    pts
}

/// 自由ベジェベクターシェイプを描画（塗り・線形グラデーション・破線ストローク・トリムパス対応）
pub fn render_vector_shape(
    config: &VectorShapeConfig,
    trim_start: f32,
    trim_end: f32,
    trim_offset: f32,
    scale_x: f32,
    scale_y: f32,
) -> LinearImage {
    let sw = (config.width * scale_x).round().clamp(4.0, 8192.0) as u32;
    let sh = (config.height * scale_y).round().clamp(4.0, 8192.0) as u32;
    let mut img = LinearImage::new(sw, sh, LinearPremul::TRANSPARENT);

    let pts = flatten_vector_path(&config.vertices, config.closed, sw as f32, sh as f32, 20);
    if pts.len() < 2 {
        return img;
    }

    // Step 1: 塗りつぶし（閉パス時のみ：Solid / LinearGradient）
    if config.closed && !matches!(config.fill, VectorFillMode::None) && pts.len() >= 4 {
        let poly = &pts[..pts.len() - 1];
        let mut x_intersections = Vec::with_capacity(16);
        let grad_info = match config.fill {
            VectorFillMode::LinearGradient {
                start_color,
                end_color,
                angle_deg,
            } => {
                let rad = angle_deg.to_radians();
                Some((start_color, end_color, rad.cos(), rad.sin()))
            }
            _ => None,
        };
        let solid_premul = match config.fill {
            VectorFillMode::Solid(c) => Some(LinearPremul::from_srgb_straight(c)),
            _ => None,
        };

        let wf = sw as f32;
        let hf = sh as f32;
        for y in 0..sh {
            let fy = y as f32 + 0.5;
            x_intersections.clear();
            let pn = poly.len();
            for i in 0..pn {
                let a = poly[i];
                let b = poly[(i + 1) % pn];
                if (a[1] <= fy && b[1] > fy) || (b[1] <= fy && a[1] > fy) {
                    let frac = (fy - a[1]) / (b[1] - a[1]);
                    x_intersections.push(a[0] + frac * (b[0] - a[0]));
                }
            }
            x_intersections.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in x_intersections.as_chunks::<2>().0 {
                let x0 = pair[0].floor().max(0.0) as u32;
                let x1 = pair[1].ceil().min(wf) as u32;
                let row_off = (y * sw) as usize;
                for x in x0..x1 {
                    let px = if let Some(sp) = solid_premul {
                        sp
                    } else if let Some((sc, ec, cos_a, sin_a)) = grad_info {
                        let nx = (x as f32 + 0.5) / wf - 0.5;
                        let ny = fy / hf - 0.5;
                        let t = (nx * cos_a + ny * sin_a + 0.5).clamp(0.0, 1.0);
                        LinearPremul::from_srgb_straight(lerp_color(sc, ec, t))
                    } else {
                        continue;
                    };
                    img.pixels[row_off + x as usize] = px;
                }
            }
        }
    }

    // Step 2: ストローク（トリムパス＆破線対応）
    let uniform_scale = ((scale_x.abs() + scale_y.abs()) * 0.5).max(0.1);
    let stroke_w = (config.stroke_width * uniform_scale).max(0.0);
    if stroke_w <= 0.1 || config.stroke_color.a <= 1e-4 {
        return img;
    }

    // 累積弧長テーブル作成
    let mut cum_len = Vec::with_capacity(pts.len());
    cum_len.push(0.0_f32);
    let mut total_len = 0.0_f32;
    for win in pts.windows(2) {
        let d = (win[1][0] - win[0][0]).hypot(win[1][1] - win[0][1]);
        total_len += d;
        cum_len.push(total_len);
    }
    if total_len <= 1e-4 {
        return img;
    }

    let t_start = trim_start.clamp(0.0, 1.0).min(trim_end.clamp(0.0, 1.0));
    let t_end = trim_start.clamp(0.0, 1.0).max(trim_end.clamp(0.0, 1.0));
    if (t_end - t_start) <= 1e-4 {
        return img;
    }

    // 描画すべき正規化弧長区間 [a, b] のリスト（閉パスの trim_offset 周回ラップ対応）
    let mut intervals: Vec<(f32, f32)> = Vec::new();
    let off = trim_offset.rem_euclid(1.0);
    if config.closed && off.abs() > 1e-5 {
        let s0 = t_start + off;
        let s1 = t_end + off;
        if s1 <= 1.0 {
            intervals.push((s0, s1));
        } else if s0 >= 1.0 {
            intervals.push((s0 - 1.0, s1 - 1.0));
        } else {
            intervals.push((s0, 1.0));
            intervals.push((0.0, s1 - 1.0));
        }
    } else {
        let s0 = (t_start + trim_offset).clamp(0.0, 1.0);
        let s1 = (t_end + trim_offset).clamp(0.0, 1.0);
        if s1 > s0 + 1e-4 {
            intervals.push((s0, s1));
        }
    }

    let dash_len = (config.dash_length * uniform_scale).max(0.0);
    let dash_gap = (config.dash_gap * uniform_scale).max(0.0);
    let use_dash = dash_len > 0.5 && dash_gap > 0.5;
    let dash_cycle = dash_len + dash_gap;
    let radius = stroke_w * 0.5;

    for (iv_start, iv_end) in intervals {
        let d_start = iv_start * total_len;
        let d_end = iv_end * total_len;
        for seg_idx in 0..(pts.len() - 1) {
            let l0 = cum_len[seg_idx];
            let l1 = cum_len[seg_idx + 1];
            let seg_len = l1 - l0;
            if seg_len <= 1e-5 || l1 <= d_start || l0 >= d_end {
                continue;
            }
            let c0 = l0.max(d_start);
            let c1 = l1.min(d_end);
            let u0 = (c0 - l0) / seg_len;
            let u1 = (c1 - l0) / seg_len;
            let a = pts[seg_idx];
            let b = pts[seg_idx + 1];
            let pa = [a[0] + (b[0] - a[0]) * u0, a[1] + (b[1] - a[1]) * u0];
            let pb = [a[0] + (b[0] - a[0]) * u1, a[1] + (b[1] - a[1]) * u1];

            if use_dash {
                let mid_d = 0.5 * (c0 + c1);
                if mid_d.rem_euclid(dash_cycle) > dash_len {
                    continue;
                }
            }
            draw_capsule_segment(&mut img, pa, pb, radius, config.stroke_color);
        }
    }

    img
}

/// 3D カメラのランバート照明係数 `[r, g, b]` を LinearPremul 画像へ乗算適用する
pub fn apply_3d_lighting_tint(img: &mut LinearImage, tint: [f32; 3]) {
    if (tint[0] - 1.0).abs() < 1e-3 && (tint[1] - 1.0).abs() < 1e-3 && (tint[2] - 1.0).abs() < 1e-3
    {
        return;
    }
    for p in &mut img.pixels {
        if p.a > 0.0 {
            p.r = (p.r * tint[0]).clamp(0.0, 4.0);
            p.g = (p.g * tint[1]).clamp(0.0, 4.0);
            p.b = (p.b * tint[2]).clamp(0.0, 4.0);
        }
    }
}

/// 3D ライト用ドロップシャドウシルエットを生成する（アルファマスクから影画像を構築）
pub fn build_drop_shadow_image(
    src: &LinearImage,
    opacity: f32,
    softness_radius: i32,
) -> LinearImage {
    let op = opacity.clamp(0.0, 1.0);
    let mut shadow = LinearImage::new(src.width, src.height, LinearPremul::TRANSPARENT);
    if op <= 1e-4 {
        return shadow;
    }
    for (dst, s) in shadow.pixels.iter_mut().zip(src.pixels.iter()) {
        if s.a > 1e-4 {
            dst.a = (s.a * op).clamp(0.0, 1.0);
        }
    }
    if softness_radius > 0 {
        crate::graph::box_blur_separable(&mut shadow, softness_radius.clamp(1, 24));
    }
    shadow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_emitter_is_deterministic_across_seeks_and_renders_all_presets() {
        for &preset in ParticlePreset::ALL {
            let cfg = ParticleConfig::for_preset(preset);
            let frame_a = render_particle_emitter(&cfg, 1.25, 160, 90);
            let frame_b = render_particle_emitter(&cfg, 1.25, 160, 90);
            assert_eq!(
                frame_a.pixels, frame_b.pixels,
                "preset {preset:?} must be deterministic"
            );

            let non_transparent = frame_a.pixels.iter().filter(|p| p.a > 0.02).count();
            assert!(
                non_transparent > 15,
                "preset {preset:?} should render visible pixels, got {non_transparent}"
            );
        }
    }

    #[test]
    fn vector_shape_supports_gradient_fill_trim_paths_and_dashed_stroke() {
        // トリムパス開パス（塗りなし）
        let wave = VectorShapeConfig::underline_wave(RgbaColor::WHITE);
        let empty = render_vector_shape(&wave, 0.0, 0.0, 0.0, 0.25, 0.5);
        let half = render_vector_shape(&wave, 0.0, 0.5, 0.0, 0.25, 0.5);
        let full = render_vector_shape(&wave, 0.0, 1.0, 0.0, 0.25, 0.5);

        let count_lit = |img: &LinearImage| img.pixels.iter().filter(|p| p.a > 0.1).count();
        assert_eq!(count_lit(&empty), 0);
        assert!(count_lit(&half) > 20);
        assert!(count_lit(&full) > count_lit(&half));

        // グラデーション塗り五芒星＋破線比較
        let mut star = VectorShapeConfig::star();
        let filled = render_vector_shape(&star, 0.0, 1.0, 0.0, 0.3, 0.3);
        assert!(count_lit(&filled) > 500);

        star.fill = VectorFillMode::None;
        let solid_stroke = render_vector_shape(&star, 0.0, 1.0, 0.0, 0.3, 0.3);
        star.dash_length = 12.0;
        star.dash_gap = 12.0;
        let dashed_stroke = render_vector_shape(&star, 0.0, 1.0, 0.0, 0.3, 0.3);
        assert!(count_lit(&solid_stroke) > count_lit(&dashed_stroke));
    }

    #[test]
    fn lighting_tint_and_drop_shadow_modify_linear_image() {
        let mut img = LinearImage::new(32, 32, LinearPremul::from_srgb_straight(RgbaColor::WHITE));
        apply_3d_lighting_tint(&mut img, [0.5, 0.5, 0.5]);
        assert!((img.pixels[0].r - 0.5).abs() < 1e-3);

        let shadow = build_drop_shadow_image(&img, 0.5, 2);
        assert!(shadow.pixels[16 * 32 + 16].a > 0.3);
        assert!(shadow.pixels[16 * 32 + 16].r < 1e-4);
    }
}
