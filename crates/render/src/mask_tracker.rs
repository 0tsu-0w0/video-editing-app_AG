use crate::color::LinearImage;
use crate::graph::RenderGraph;
use ve_core::{
    BezierMask, MaskMode, MotionTrackConfig, Project, RationalTime, SceneId, TrackPointSample,
};

/// v0.7 #8: アイテム画像へ自由ベジェマスク列（加算・減算・反転・フェザー）を適用する
pub fn apply_bezier_masks(img: &mut LinearImage, masks: &[BezierMask]) {
    let w = img.width as usize;
    let h = img.height as usize;
    if w == 0 || h == 0 {
        return;
    }

    let active: Vec<&BezierMask> = masks
        .iter()
        .filter(|m| m.enabled && m.vertices.len() >= 3 && m.opacity > 0.0)
        .collect();
    if active.is_empty() {
        return;
    }

    let has_add = active.iter().any(|m| m.mode == MaskMode::Add);
    let mut composite = vec![if has_add { 0.0f32 } else { 1.0f32 }; w * h];

    // 1. Add マスクの和集合
    for mask in active.iter().filter(|m| m.mode == MaskMode::Add) {
        let mut buf = rasterize_single_mask(mask, w, h);
        if mask.feather_px > 0.5 {
            feather_alpha_buffer(&mut buf, w, h, mask.feather_px);
        }
        let op = mask.opacity.clamp(0.0, 1.0);
        for (dst, &m) in composite.iter_mut().zip(buf.iter()) {
            *dst = dst.max(m * op);
        }
    }

    // 2. Subtract マスクの減算
    for mask in active.iter().filter(|m| m.mode == MaskMode::Subtract) {
        let mut buf = rasterize_single_mask(mask, w, h);
        if mask.feather_px > 0.5 {
            feather_alpha_buffer(&mut buf, w, h, mask.feather_px);
        }
        let op = mask.opacity.clamp(0.0, 1.0);
        for (dst, &m) in composite.iter_mut().zip(buf.iter()) {
            *dst = (*dst * (1.0 - m * op)).clamp(0.0, 1.0);
        }
    }

    // Premultiplied Linear RGBA へマスクアルファを乗算
    for (px, &alpha) in img.pixels.iter_mut().zip(composite.iter()) {
        px.r *= alpha;
        px.g *= alpha;
        px.b *= alpha;
        px.a *= alpha;
    }
}

fn rasterize_single_mask(mask: &BezierMask, w: usize, h: usize) -> Vec<f32> {
    let poly_norm = mask.flatten_polygon(16);
    if poly_norm.len() < 3 {
        return vec![if mask.inverted { 1.0 } else { 0.0 }; w * h];
    }
    let wf = w as f32;
    let hf = h as f32;
    let poly: Vec<[f32; 2]> = poly_norm
        .into_iter()
        .map(|[nx, ny]| [nx * wf, ny * hf])
        .collect();

    let n = poly.len();
    let mut out = vec![if mask.inverted { 1.0f32 } else { 0.0f32 }; w * h];
    let fill_val = if mask.inverted { 0.0f32 } else { 1.0f32 };
    let mut xs: Vec<f32> = Vec::with_capacity(16);

    for y in 0..h {
        let fy = y as f32 + 0.5;
        xs.clear();
        for i in 0..n {
            let [x0, y0] = poly[i];
            let [x1, y1] = poly[(i + 1) % n];
            if (y0 <= fy && y1 > fy) || (y1 <= fy && y0 > fy) {
                let t = (fy - y0) / (y1 - y0);
                xs.push(x0 + t * (x1 - x0));
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let row = y * w;
        for pair in xs.as_chunks::<2>().0 {
            let x_start = pair[0].ceil().max(0.0) as usize;
            let x_end = (pair[1].ceil().max(0.0) as usize).min(w);
            for x in x_start..x_end {
                out[row + x] = fill_val;
            }
        }
    }
    out
}

/// O(W×H) スライディングウィンドウ分離ボックスブラー 2パスによる滑らかなマスクフェザー
fn feather_alpha_buffer(buf: &mut [f32], w: usize, h: usize, feather_px: f32) {
    let r = feather_px.round().clamp(1.0, 96.0) as usize;
    let mut tmp = vec![0.0f32; w * h];
    for _pass in 0..2 {
        // Horizontal
        let diam = (2 * r + 1) as f32;
        for y in 0..h {
            let row = y * w;
            let mut sum = 0.0f32;
            for dx in -(r as isize)..=(r as isize) {
                let sx = dx.clamp(0, (w - 1) as isize) as usize;
                sum += buf[row + sx];
            }
            for x in 0..w {
                tmp[row + x] = sum / diam;
                let sub_x = (x as isize - r as isize).clamp(0, (w - 1) as isize) as usize;
                let add_x = (x as isize + r as isize + 1).clamp(0, (w - 1) as isize) as usize;
                sum += buf[row + add_x] - buf[row + sub_x];
            }
        }
        // Vertical
        for x in 0..w {
            let mut sum = 0.0f32;
            for dy in -(r as isize)..=(r as isize) {
                let sy = dy.clamp(0, (h - 1) as isize) as usize;
                sum += tmp[sy * w + x];
            }
            for y in 0..h {
                buf[y * w + x] = (sum / diam).clamp(0.0, 1.0);
                let sub_y = (y as isize - r as isize).clamp(0, (h - 1) as isize) as usize;
                let add_y = (y as isize + r as isize + 1).clamp(0, (h - 1) as isize) as usize;
                sum += tmp[add_y * w + x] - tmp[sub_y * w + x];
            }
        }
    }
}

/// v0.7 #8: Pure-Rust 1ポイント自動モーショントラッカー（テンプレートマッチング＋サブピクセル補間）
pub fn track_point_across_frames(
    frames: &[(RationalTime, LinearImage)],
    cfg: &MotionTrackConfig,
) -> Vec<TrackPointSample> {
    let Some((t0, img0)) = frames.first() else {
        return Vec::new();
    };
    let w = img0.width as i32;
    let h = img0.height as i32;
    if w < 8 || h < 8 {
        return Vec::new();
    }

    let r = (cfg.patch_radius as i32).clamp(3, 28);
    let sr = (cfg.search_radius as i32).clamp(4, 96);
    let mut cur_x = cfg.anchor_x.clamp(r as f32, (w - 1 - r) as f32);
    let mut cur_y = cfg.anchor_y.clamp(r as f32, (h - 1 - r) as f32);

    let luma0 = image_to_luma(img0);
    let template = extract_patch(&luma0, w, h, cur_x.round() as i32, cur_y.round() as i32, r);
    let patch_len = template.len().max(1) as f32;

    let mut out = Vec::with_capacity(frames.len());
    out.push(TrackPointSample {
        time: *t0,
        x: cur_x,
        y: cur_y,
        confidence: 1.0,
    });

    for (t, img) in frames.iter().skip(1) {
        if img.width as i32 != w || img.height as i32 != h {
            continue;
        }
        let luma = image_to_luma(img);
        let cx0 = cur_x.round() as i32;
        let cy0 = cur_y.round() as i32;

        // 1. 粗探索（2px ステップ）
        let mut best_x = cx0;
        let mut best_y = cy0;
        let mut best_err = f32::MAX;
        let min_x = (cx0 - sr).max(r);
        let max_x = (cx0 + sr).min(w - 1 - r);
        let min_y = (cy0 - sr).max(r);
        let max_y = (cy0 + sr).min(h - 1 - r);

        let mut y = min_y;
        while y <= max_y {
            let mut x = min_x;
            while x <= max_x {
                let err = patch_sad(&luma, w, &template, x, y, r);
                if err < best_err {
                    best_err = err;
                    best_x = x;
                    best_y = y;
                }
                x += 2;
            }
            y += 2;
        }

        // 2. 精密探索（1px ステップ、粗最良点の周辺 ±2px）
        let fine_x0 = (best_x - 2).max(min_x);
        let fine_x1 = (best_x + 2).min(max_x);
        let fine_y0 = (best_y - 2).max(min_y);
        let fine_y1 = (best_y + 2).min(max_y);
        for fy in fine_y0..=fine_y1 {
            for fx in fine_x0..=fine_x1 {
                let err = patch_sad(&luma, w, &template, fx, fy, r);
                if err < best_err {
                    best_err = err;
                    best_x = fx;
                    best_y = fy;
                }
            }
        }

        // 3. 放物線フィッティングによるサブピクセル精密化
        let mut sub_x = best_x as f32;
        let mut sub_y = best_y as f32;
        if best_x > min_x && best_x < max_x {
            let el = patch_sad(&luma, w, &template, best_x - 1, best_y, r);
            let er = patch_sad(&luma, w, &template, best_x + 1, best_y, r);
            let denom = 2.0 * (el - 2.0 * best_err + er);
            if denom.abs() > 1e-5 {
                sub_x += ((el - er) / denom).clamp(-0.5, 0.5);
            }
        }
        if best_y > min_y && best_y < max_y {
            let eu = patch_sad(&luma, w, &template, best_x, best_y - 1, r);
            let ed = patch_sad(&luma, w, &template, best_x, best_y + 1, r);
            let denom = 2.0 * (eu - 2.0 * best_err + ed);
            if denom.abs() > 1e-5 {
                sub_y += ((eu - ed) / denom).clamp(-0.5, 0.5);
            }
        }

        let mean_abs_err = best_err / patch_len;
        let confidence = (1.0 - mean_abs_err * 2.5).clamp(0.0, 1.0);
        cur_x = sub_x;
        cur_y = sub_y;
        out.push(TrackPointSample {
            time: *t,
            x: cur_x,
            y: cur_y,
            confidence,
        });
    }

    out
}

/// シーンの指定区間をレンダリングしながら自動モーショントラッキングを実行し、
/// シーン実解像度座標の `TrackPointSample` 列を返す。
pub fn track_point_in_scene(
    rg: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    cfg: &MotionTrackConfig,
) -> Vec<TrackPointSample> {
    let Some(scene) = project.scene(scene_id) else {
        return Vec::new();
    };
    let fps = scene.fps;
    let start_f = cfg.start_time.to_frame_floor(fps).max(0);
    let end_f = cfg.end_time.to_frame_floor(fps).max(start_f);
    let step = i64::from(cfg.step_frames.max(1));

    // 高解像度シーンは最大幅 480px に縮小して高速追跡し、結果を元座標へ戻す
    let scale = if scene.width > 480 {
        480.0 / scene.width as f32
    } else {
        1.0
    };
    let tw = ((scene.width as f32) * scale).round().max(16.0) as u32;
    let th = ((scene.height as f32) * scale).round().max(16.0) as u32;
    let sx = tw as f32 / scene.width.max(1) as f32;
    let sy = th as f32 / scene.height.max(1) as f32;

    let mut frames = Vec::new();
    let mut f = start_f;
    let mut safety = 0usize;
    while f <= end_f && safety < 240 {
        safety += 1;
        let t = RationalTime::from_frame(f, fps);
        if let Some(img) = rg.render_project_scene_scaled(project, scene_id, t, tw, th, false) {
            frames.push((t, img));
        }
        if f == end_f {
            break;
        }
        f = (f + step).min(end_f);
    }

    let scaled_cfg = MotionTrackConfig {
        start_time: cfg.start_time,
        end_time: cfg.end_time,
        step_frames: cfg.step_frames,
        anchor_x: cfg.anchor_x * sx,
        anchor_y: cfg.anchor_y * sy,
        patch_radius: ((cfg.patch_radius as f32) * sx).round().clamp(4.0, 24.0) as u32,
        search_radius: ((cfg.search_radius as f32) * sx).round().clamp(6.0, 72.0) as u32,
    };

    track_point_across_frames(&frames, &scaled_cfg)
        .into_iter()
        .map(|s| TrackPointSample {
            time: s.time,
            x: s.x / sx,
            y: s.y / sy,
            confidence: s.confidence,
        })
        .collect()
}

fn image_to_luma(img: &LinearImage) -> Vec<f32> {
    img.pixels
        .iter()
        .map(|p| 0.2126 * p.r + 0.7152 * p.g + 0.0722 * p.b)
        .collect()
}

fn extract_patch(luma: &[f32], w: i32, h: i32, cx: i32, cy: i32, r: i32) -> Vec<f32> {
    let side = (2 * r + 1) as usize;
    let mut out = Vec::with_capacity(side * side);
    for dy in -r..=r {
        let y = (cy + dy).clamp(0, h - 1);
        let row = (y * w) as usize;
        for dx in -r..=r {
            let x = (cx + dx).clamp(0, w - 1) as usize;
            out.push(luma[row + x]);
        }
    }
    out
}

fn patch_sad(luma: &[f32], w: i32, template: &[f32], cx: i32, cy: i32, r: i32) -> f32 {
    let mut err = 0.0f32;
    let mut idx = 0usize;
    for dy in -r..=r {
        let row = ((cy + dy) * w) as usize;
        for dx in -r..=r {
            let x = (cx + dx) as usize;
            err += (luma[row + x] - template[idx]).abs();
            idx += 1;
        }
    }
    err
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::LinearPremul;
    use ve_core::{
        FrameRate, Interpolation, ItemKind, Keyframe, ProjectSettings, RgbaColor, ShapeKind,
        TimelineItem,
    };

    #[test]
    fn bezier_mask_clips_ellipse_with_feather_and_subtraction() {
        let mut img =
            LinearImage::new(100, 100, LinearPremul::from_srgb_straight(RgbaColor::WHITE));
        let mut outer = BezierMask::ellipse();
        outer.feather_px = 3.0;
        let mut hole = BezierMask::rectangle();
        hole.mode = MaskMode::Subtract;
        hole.feather_px = 0.0;
        hole.vertices = vec![
            ve_core::MaskVertex::corner(0.42, 0.42),
            ve_core::MaskVertex::corner(0.58, 0.42),
            ve_core::MaskVertex::corner(0.58, 0.58),
            ve_core::MaskVertex::corner(0.42, 0.58),
        ];

        apply_bezier_masks(&mut img, &[outer, hole]);

        // Corner (2,2) outside ellipse -> transparent
        assert!(img.pixels[2 * 100 + 2].a < 0.02);
        // Ring inside ellipse outside center hole (30,50) -> opaque
        assert!(
            img.pixels[50 * 100 + 30].a > 0.9,
            "ring alpha={}",
            img.pixels[50 * 100 + 30].a
        );
        // Center hole (50,50) subtracted -> transparent
        assert!(
            img.pixels[50 * 100 + 50].a < 0.05,
            "hole alpha={}",
            img.pixels[50 * 100 + 50].a
        );
    }

    #[test]
    fn motion_tracker_follows_moving_target_accurately_within_1_5px() {
        let mut proj = Project::new(
            "TrackTest",
            ProjectSettings {
                width: 200,
                height: 120,
                fps: FrameRate::FPS_30,
                ..Default::default()
            },
        );
        // A 24x24 bright white square moves linearly from (30, 40) at t=0s to (110, 70) at t=0.4s
        let mut target = TimelineItem::new(
            "MovingTarget",
            RationalTime::ZERO,
            RationalTime::from_seconds(1.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 24.0,
                height: 24.0,
                color: RgbaColor::WHITE,
            },
        );
        target.keyframes.insert(
            "x".into(),
            vec![
                Keyframe {
                    time: RationalTime::ZERO,
                    value: 30.0,
                    interpolation: Interpolation::Linear,
                },
                Keyframe {
                    time: RationalTime::from_seconds(0.4),
                    value: 110.0,
                    interpolation: Interpolation::Linear,
                },
            ],
        );
        target.keyframes.insert(
            "y".into(),
            vec![
                Keyframe {
                    time: RationalTime::ZERO,
                    value: 40.0,
                    interpolation: Interpolation::Linear,
                },
                Keyframe {
                    time: RationalTime::from_seconds(0.4),
                    value: 70.0,
                    interpolation: Interpolation::Linear,
                },
            ],
        );
        proj.active_scene_mut().unwrap().layers[0]
            .items
            .push(target);

        let mut rg = RenderGraph::new();
        let cfg = MotionTrackConfig {
            start_time: RationalTime::ZERO,
            end_time: RationalTime::from_seconds(0.4),
            step_frames: 2,
            // Center of 24x24 box starting at (30,40) is (42, 52)
            anchor_x: 42.0,
            anchor_y: 52.0,
            patch_radius: 10,
            search_radius: 26,
        };
        let samples = track_point_in_scene(&mut rg, &proj, proj.active_scene_id, &cfg);
        assert!(samples.len() >= 6, "samples={}", samples.len());
        let last = samples.last().unwrap();
        // True center at t=0.4s is (110+12, 70+12) = (122, 82)
        assert!(
            (last.x - 122.0).abs() < 2.0 && (last.y - 82.0).abs() < 2.0,
            "expected ~(122, 82), got ({:.2}, {:.2}), conf={:.2}",
            last.x,
            last.y,
            last.confidence
        );
    }
}
