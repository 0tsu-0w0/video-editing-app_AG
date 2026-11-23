//! v5.0 #37 & #39: 2D スケルトンパペットリグ描画エンジン（アンチエイリアスカプセル＆ボーン/IK オーバーレイ）
//! およびカラーヒストグラム χ² 距離ベースの自動シーンカット検出（Scene Edit Detection）

use crate::color::{LinearImage, LinearPremul, linear_to_srgb};
use crate::graph::RenderGraph;
use ve_core::{
    DetectedSceneCut, Project, RationalTime, RgbaColor, Rig2DConfig, SceneId, SolvedBonePose,
    evaluate_rig_poses,
};

#[inline]
fn blend_over(dst: &mut LinearPremul, src: LinearPremul) {
    let inv_a = 1.0 - src.a;
    dst.r = src.r + dst.r * inv_a;
    dst.g = src.g + dst.g * inv_a;
    dst.b = src.b + dst.b * inv_a;
    dst.a = src.a + dst.a * inv_a;
}

fn draw_capsule(img: &mut LinearImage, p0: [f32; 2], p1: [f32; 2], radius: f32, color: RgbaColor) {
    let r = radius.max(1.0);
    let min_x = ((p0[0].min(p1[0]) - r - 2.0).floor() as i32).max(0);
    let max_x = ((p0[0].max(p1[0]) + r + 2.0).ceil() as i32).min(img.width as i32 - 1);
    let min_y = ((p0[1].min(p1[1]) - r - 2.0).floor() as i32).max(0);
    let max_y = ((p0[1].max(p1[1]) + r + 2.0).ceil() as i32).min(img.height as i32 - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }

    let vx = p1[0] - p0[0];
    let vy = p1[1] - p0[1];
    let len_sq = (vx * vx + vy * vy).max(1e-5);
    let w = img.width as usize;

    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        let row = (y as usize) * w;
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let t = (((px - p0[0]) * vx + (py - p0[1]) * vy) / len_sq).clamp(0.0, 1.0);
            let cx = p0[0] + vx * t;
            let cy = p0[1] + vy * t;
            let d = (px - cx).hypot(py - cy);
            let cov = (r + 0.75 - d).clamp(0.0, 1.5) / 1.5;
            if cov > 0.001 {
                // さりげない立体リムシェーディング（左上光沢＋輪郭暗部）
                let edge_shade = 0.82 + 0.22 * (1.0 - (d / r).min(1.0));
                let rim_light = if px < cx { 1.06 } else { 0.94 };
                let shade = (edge_shade * rim_light).clamp(0.65, 1.12);
                let src = LinearPremul::from_srgb_straight(RgbaColor {
                    r: (color.r * shade).clamp(0.0, 1.0),
                    g: (color.g * shade).clamp(0.0, 1.0),
                    b: (color.b * shade).clamp(0.0, 1.0),
                    a: color.a * cov,
                });
                blend_over(&mut img.pixels[row + x as usize], src);
            }
        }
    }
}

fn draw_circle(img: &mut LinearImage, center: [f32; 2], radius: f32, color: RgbaColor) {
    draw_capsule(img, center, center, radius, color);
}

/// 2D スケルトンボーン＆2-Bone IK パペットリグを決定論レンダリングする
pub fn render_rigged_puppet(
    cfg: &Rig2DConfig,
    local_time_sec: f32,
    out_w: u32,
    out_h: u32,
) -> LinearImage {
    let w = out_w.clamp(32, 4096);
    let h = out_h.clamp(32, 4096);
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let poses = evaluate_rig_poses(cfg, local_time_sec);
    if poses.is_empty() {
        return img;
    }

    let sx = (w as f32) / cfg.width.max(32.0);
    let sy = (h as f32) / cfg.height.max(32.0);
    let sc = sx.min(sy);
    let cx = (w as f32) * 0.5;
    let cy = (h as f32) * 0.5;
    let map_pt = |p: [f32; 2]| -> [f32; 2] { [cx + p[0] * sc, cy + p[1] * sc] };

    // 背面 → 前面の描画順でパーツを合成
    let draw_order = [
        "scarf_tail",
        "l_leg",
        "r_leg",
        "l_upper_arm",
        "l_forearm",
        "torso",
        "r_upper_arm",
        "r_forearm",
        "hair_ribbon",
        "head",
    ];

    let mut drawn = vec![false; poses.len()];
    let mut draw_pose = |pose: &SolvedBonePose| {
        let p0 = map_pt(pose.start);
        let p1 = map_pt(pose.end);
        let rad = (pose.thickness * 0.5 * sc).max(2.0);
        if pose.id == "head" {
            let mid = [(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5];
            let head_r = (pose.thickness * 0.52 * sc).max(6.0);
            draw_circle(&mut img, mid, head_r, pose.color);
            // 目＆スマイル表情を頭の回転に追従させて描画
            let dir_x = (p1[0] - p0[0]) / ((p1[0] - p0[0]).hypot(p1[1] - p0[1]).max(1.0));
            let dir_y = (p1[1] - p0[1]) / ((p1[0] - p0[0]).hypot(p1[1] - p0[1]).max(1.0));
            let right_x = -dir_y;
            let right_y = dir_x;
            let eye_col = RgbaColor {
                r: 0.12,
                g: 0.14,
                b: 0.22,
                a: 1.0,
            };
            let blush = RgbaColor {
                r: 0.98,
                g: 0.52,
                b: 0.60,
                a: 0.55,
            };
            for side in [-1.0f32, 1.0f32] {
                let ex = mid[0] + right_x * head_r * 0.34 * side + dir_x * head_r * 0.05;
                let ey = mid[1] + right_y * head_r * 0.34 * side + dir_y * head_r * 0.05;
                draw_circle(&mut img, [ex, ey], (head_r * 0.12).max(1.5), eye_col);
                let bx = mid[0] + right_x * head_r * 0.48 * side - dir_x * head_r * 0.14;
                let by = mid[1] + right_y * head_r * 0.48 * side - dir_y * head_r * 0.14;
                draw_circle(&mut img, [bx, by], (head_r * 0.14).max(1.5), blush);
            }
            // 口
            let mx = mid[0] - dir_x * head_r * 0.28;
            let my = mid[1] - dir_y * head_r * 0.28;
            draw_capsule(
                &mut img,
                [mx - right_x * head_r * 0.16, my - right_y * head_r * 0.16],
                [mx + right_x * head_r * 0.16, my + right_y * head_r * 0.16],
                (head_r * 0.06).max(1.2),
                RgbaColor {
                    r: 0.85,
                    g: 0.30,
                    b: 0.38,
                    a: 1.0,
                },
            );
        } else {
            draw_capsule(&mut img, p0, p1, rad, pose.color);
        }
    };

    for name in draw_order {
        if let Some((idx, pose)) = poses.iter().enumerate().find(|(_, p)| p.id == name) {
            draw_pose(pose);
            drawn[idx] = true;
        }
    }
    for (idx, pose) in poses.iter().enumerate() {
        if !drawn[idx] {
            draw_pose(pose);
        }
    }

    // スケルトンボーン＆2-Bone IK ターゲットオーバーレイ
    if cfg.show_bones_overlay {
        let bone_col = RgbaColor {
            r: 0.15,
            g: 0.94,
            b: 0.98,
            a: 0.85,
        };
        let joint_col = RgbaColor {
            r: 1.0,
            g: 0.84,
            b: 0.20,
            a: 0.95,
        };
        for pose in &poses {
            let p0 = map_pt(pose.start);
            let p1 = map_pt(pose.end);
            draw_capsule(&mut img, p0, p1, (2.2 * sc).max(1.2), bone_col);
            draw_circle(&mut img, p0, (4.5 * sc).max(2.2), joint_col);
            draw_circle(&mut img, p1, (3.2 * sc).max(1.8), bone_col);
        }
        let ik_col = RgbaColor {
            r: 1.0,
            g: 0.20,
            b: 0.72,
            a: 0.95,
        };
        for ik in [cfg.right_arm_ik, cfg.left_arm_ik] {
            if ik.enabled && ik.lower_bone_idx < poses.len() {
                let end = map_pt(poses[ik.lower_bone_idx].end);
                draw_circle(&mut img, end, (7.0 * sc).max(3.5), ik_col);
            }
        }
    }

    img
}

/// 4×4×4 = 64 ビン正規化 RGB カラーヒストグラムと平均輝度 IRE を算出する
fn frame_color_histogram_and_luma(img: &LinearImage) -> ([f32; 64], f32) {
    let mut hist = [0.0f32; 64];
    let n = img.pixels.len().max(1);
    let mut luma_sum = 0.0f64;
    for px in &img.pixels {
        let r = linear_to_srgb(px.r.max(0.0));
        let g = linear_to_srgb(px.g.max(0.0));
        let b = linear_to_srgb(px.b.max(0.0));
        let ri = ((r * 3.999) as usize).min(3);
        let gi = ((g * 3.999) as usize).min(3);
        let bi = ((b * 3.999) as usize).min(3);
        hist[ri * 16 + gi * 4 + bi] += 1.0;
        luma_sum += f64::from((0.2126 * r + 0.7152 * g + 0.0722 * b) * 100.0);
    }
    let inv = 1.0 / (n as f32);
    for h in &mut hist {
        *h *= inv;
    }
    (hist, (luma_sum / (n as f64)) as f32)
}

/// 2 つの正規化ヒストグラム間の対称 χ² (Chi-Square) 距離 `[0.0, 2.0]` を算出する
pub fn compute_frame_histogram_chi_sq(a: &[f32; 64], b: &[f32; 64]) -> f32 {
    let mut sum = 0.0f32;
    for (&ha, &hb) in a.iter().zip(b.iter()) {
        let den = ha + hb;
        if den > 1e-6 {
            let diff = ha - hb;
            sum += (diff * diff) / den;
        }
    }
    sum
}

/// v5.0 #39: シーン全区間をサンプリングし、RGB ヒストグラム χ² 距離＋輝度変化から
/// ショット境界（ハードカット）を自動検出する（`sensitivity`: `0.1..=1.0`, 既定 `0.55`）
pub fn detect_scene_cuts_in_scene(
    graph: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    sample_interval_sec: f64,
    sensitivity: f32,
) -> Vec<DetectedSceneCut> {
    let Some(scene) = project.scene(scene_id) else {
        return Vec::new();
    };
    let dur = scene.total_duration().as_seconds();
    if dur <= 0.3 {
        return Vec::new();
    }
    let step = sample_interval_sec.clamp(0.08, 1.0);
    let max_samples = ((dur / step).floor() as usize + 1).clamp(3, 180);
    let sens = sensitivity.clamp(0.1, 1.0);
    // 感度が高いほどしきい値を下げる (chi_sq 0.20..0.75)
    let threshold = 0.80 - sens * 0.58;

    let mut prev: Option<([f32; 64], f32)> = None;
    let mut cuts: Vec<DetectedSceneCut> = Vec::new();
    let mut last_cut_t = -10.0f64;

    for i in 0..max_samples {
        let t_sec = (i as f64) * step;
        if t_sec >= dur {
            break;
        }
        let Some(img) = graph.render_project_scene_scaled(
            project,
            scene_id,
            RationalTime::from_seconds(t_sec),
            128,
            72,
            false,
        ) else {
            continue;
        };
        let cur = frame_color_histogram_and_luma(&img);
        if let Some((prev_h, prev_l)) = prev {
            let chi = compute_frame_histogram_chi_sq(&prev_h, &cur.0);
            let luma_delta = (cur.1 - prev_l).abs();
            let combined = chi + (luma_delta / 100.0) * 0.45;
            if combined >= threshold && (t_sec - last_cut_t) >= 0.35 {
                let conf = ((combined - threshold) / (1.4 - threshold).max(0.2) * 0.5 + 0.5)
                    .clamp(0.5, 1.0);
                cuts.push(DetectedSceneCut {
                    time_sec: t_sec,
                    confidence: conf,
                    hist_chi_sq: chi,
                    luma_delta_ire: luma_delta,
                });
                last_cut_t = t_sec;
            }
        }
        prev = Some(cur);
    }
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::{ItemKind, ProjectSettings, ShapeKind, TimelineItem};

    #[test]
    fn rigged_puppet_renders_deterministically_and_animates_arms_and_ribbons() {
        let cfg = Rig2DConfig::default_humanoid();
        let a = render_rigged_puppet(&cfg, 0.0, 120, 160).to_rgba8_srgb();
        let b = render_rigged_puppet(&cfg, 0.0, 120, 160).to_rgba8_srgb();
        assert_eq!(a, b);
        let non_empty = a.as_chunks::<4>().0.iter().filter(|p| p[3] > 20).count();
        assert!(
            non_empty > 1500,
            "puppet should cover visible area: {non_empty}"
        );

        let later = render_rigged_puppet(&cfg, 0.45, 120, 160).to_rgba8_srgb();
        let diff = a.iter().zip(later.iter()).filter(|(x, y)| x != y).count();
        assert!(diff > 500, "puppet should visibly animate: diff={diff}");
    }

    #[test]
    fn scene_cut_detector_finds_hard_shot_boundary_accurately() {
        let mut proj = Project::new("CutDetect", ProjectSettings::default());
        {
            let sc = proj.active_scene_mut().unwrap();
            sc.width = 160;
            sc.height = 90;
            // 0..1.5s: 明るい赤ショット / 1.5..3.0s: 深い青ショット
            sc.layers[0].items.push(TimelineItem::new(
                "Shot1",
                RationalTime::ZERO,
                RationalTime::from_seconds(1.5),
                ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: 160.0,
                    height: 90.0,
                    color: RgbaColor {
                        r: 0.92,
                        g: 0.14,
                        b: 0.12,
                        a: 1.0,
                    },
                },
            ));
            sc.layers[0].items.push(TimelineItem::new(
                "Shot2",
                RationalTime::from_seconds(1.5),
                RationalTime::from_seconds(1.5),
                ItemKind::Shape {
                    shape: ShapeKind::Rectangle,
                    width: 160.0,
                    height: 90.0,
                    color: RgbaColor {
                        r: 0.08,
                        g: 0.20,
                        b: 0.95,
                        a: 1.0,
                    },
                },
            ));
        }

        let mut rg = RenderGraph::new();
        let cuts = detect_scene_cuts_in_scene(&mut rg, &proj, proj.active_scene_id, 0.25, 0.6);
        assert_eq!(
            cuts.len(),
            1,
            "expected exactly 1 hard shot cut, got {cuts:?}"
        );
        assert!(
            (cuts[0].time_sec - 1.5).abs() <= 0.3,
            "cut time should be near 1.5s, got {}",
            cuts[0].time_sec
        );
    }
}
