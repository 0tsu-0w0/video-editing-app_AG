//! v2.1 #26: 自動手ぶれ補正（Auto Video Stabilizer）＆ 平面 4 点コーナーピン合成（Projective Homography VFX）
//! Pure Rust・ゼロ `unsafe`・外部依存ゼロで射影変換 $3\times 3$ ホモグラフィ合成とマルチパッチカメラ運動推定を実現する。

use crate::color::{LinearImage, LinearPremul};
use crate::graph::RenderGraph;
use ve_core::{
    CornerPinConfig, CubicBezier, EditCommand, Interpolation, ItemId, Keyframe, Project,
    RationalTime, Scene, SceneId,
};

// =========================================================================
// 1. Planar 4-Point Corner Pin Composite (Exact 3x3 Projective Homography)
// =========================================================================

/// 単位正方形 `(0,0),(1,0),(1,1),(0,1)` → 任意 4 隅 `(tl, tr, br, bl)` の射影変換 $3\times 3$ 行列 `H` を
/// 8×8 部分ピボット付きガウス消去法で厳密に解く。
pub fn solve_unit_square_to_quad_homography(
    tl: [f32; 2],
    tr: [f32; 2],
    br: [f32; 2],
    bl: [f32; 2],
) -> Option<[f64; 9]> {
    let src = [[0.0f64, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let dst = [
        [f64::from(tl[0]), f64::from(tl[1])],
        [f64::from(tr[0]), f64::from(tr[1])],
        [f64::from(br[0]), f64::from(br[1])],
        [f64::from(bl[0]), f64::from(bl[1])],
    ];

    let mut a = [[0.0f64; 9]; 8];
    for (i, (&[x, y], &[u, v])) in src.iter().zip(dst.iter()).enumerate() {
        a[i * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[i * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }

    // 部分ピボット付きガウス・ジョルダン消去法
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
        let pivot_row = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            if r == col {
                continue;
            }
            let factor = row[col];
            if factor.abs() < 1e-15 {
                continue;
            }
            for (dst_cell, &piv_cell) in row.iter_mut().zip(pivot_row.iter()).skip(col) {
                *dst_cell -= factor * piv_cell;
            }
        }
    }

    Some([
        a[0][8], a[1][8], a[2][8], a[3][8], a[4][8], a[5][8], a[6][8], a[7][8], 1.0,
    ])
}

/// $3\times 3$ 行列の逆行列を余因子展開で算出する
pub fn invert_3x3(m: [f64; 9]) -> Option<[f64; 9]> {
    let c00 = m[4] * m[8] - m[5] * m[7];
    let c01 = -(m[3] * m[8] - m[5] * m[6]);
    let c02 = m[3] * m[7] - m[4] * m[6];

    let c10 = -(m[1] * m[8] - m[2] * m[7]);
    let c11 = m[0] * m[8] - m[2] * m[6];
    let c12 = -(m[0] * m[7] - m[1] * m[6]);

    let c20 = m[1] * m[5] - m[2] * m[4];
    let c21 = -(m[0] * m[5] - m[2] * m[3]);
    let c22 = m[0] * m[4] - m[1] * m[3];

    let det = m[0] * c00 + m[1] * c01 + m[2] * c02;
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        c00 * inv,
        c10 * inv,
        c20 * inv,
        c01 * inv,
        c11 * inv,
        c21 * inv,
        c02 * inv,
        c12 * inv,
        c22 * inv,
    ])
}

/// 平面 4 点コーナーピン合成（逆ホモグラフィ射影変換＋バイリニア補間＋エッジアンチエイリアス）
pub fn apply_corner_pin(src: &LinearImage, cfg: &CornerPinConfig) -> LinearImage {
    if !cfg.enabled || src.width < 2 || src.height < 2 {
        return src.clone();
    }
    let Some(h_fwd) = solve_unit_square_to_quad_homography(cfg.tl, cfg.tr, cfg.br, cfg.bl) else {
        return src.clone();
    };
    let Some(h_inv) = invert_3x3(h_fwd) else {
        return src.clone();
    };

    let w = src.width;
    let h = src.height;
    let fw = w as f64;
    let fh = h as f64;
    let feather_u = (f64::from(cfg.feather_px.clamp(0.0, 16.0)) / fw.max(1.0)).max(1e-4);
    let feather_v = (f64::from(cfg.feather_px.clamp(0.0, 16.0)) / fh.max(1.0)).max(1e-4);

    let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    for y in 0..h {
        let ny = (f64::from(y) + 0.5) / fh;
        let row = (y * w) as usize;
        for x in 0..w {
            let nx = (f64::from(x) + 0.5) / fw;
            let denom = h_inv[6] * nx + h_inv[7] * ny + h_inv[8];
            if denom.abs() < 1e-9 {
                continue;
            }
            let u = (h_inv[0] * nx + h_inv[1] * ny + h_inv[2]) / denom;
            let v = (h_inv[3] * nx + h_inv[4] * ny + h_inv[5]) / denom;
            if !(-feather_u..=1.0 + feather_u).contains(&u)
                || !(-feather_v..=1.0 + feather_v).contains(&v)
            {
                continue;
            }

            let sx = (u.clamp(0.0, 1.0) * (fw - 1.0)) as f32;
            let sy = (v.clamp(0.0, 1.0) * (fh - 1.0)) as f32;
            let mut px = sample_bilinear(src, sx, sy);

            // 四辺エッジの滑らかなアンチエイリアスフェザー
            let edge_u = (u / feather_u).min((1.0 - u) / feather_u).clamp(0.0, 1.0);
            let edge_v = (v / feather_v).min((1.0 - v) / feather_v).clamp(0.0, 1.0);
            let edge_cov = (edge_u * edge_v) as f32;
            px.r *= edge_cov;
            px.g *= edge_cov;
            px.b *= edge_cov;
            px.a *= edge_cov;
            out.pixels[row + x as usize] = px;
        }
    }
    out
}

fn sample_bilinear(img: &LinearImage, x: f32, y: f32) -> LinearPremul {
    let cx = x.clamp(0.0, (img.width - 1) as f32);
    let cy = y.clamp(0.0, (img.height - 1) as f32);
    let x0 = cx.floor() as u32;
    let y0 = cy.floor() as u32;
    let x1 = (x0 + 1).min(img.width - 1);
    let y1 = (y0 + 1).min(img.height - 1);
    let fx = cx - x0 as f32;
    let fy = cy - y0 as f32;
    let w = img.width;
    let p00 = img.pixels[(y0 * w + x0) as usize];
    let p10 = img.pixels[(y0 * w + x1) as usize];
    let p01 = img.pixels[(y1 * w + x0) as usize];
    let p11 = img.pixels[(y1 * w + x1) as usize];
    let lerp = |a: LinearPremul, b: LinearPremul, t: f32| {
        let u = 1.0 - t;
        LinearPremul {
            r: a.r * u + b.r * t,
            g: a.g * u + b.g * t,
            b: a.b * u + b.b * t,
            a: a.a * u + b.a * t,
        }
    };
    lerp(lerp(p00, p10, fx), lerp(p01, p11, fx), fy)
}

// =========================================================================
// 2. Auto 2D Video Stabilizer (Multi-Patch Camera Motion + Zero-Phase Smoothing)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabilizeConfig {
    /// 解析サンプリング間隔（秒、既定 0.2s）
    pub sample_interval_sec: f64,
    /// 手ぶれ平滑化強度（0.1..=0.95、大きいほど三脚固定寄り）
    pub smoothness: f32,
    /// 最大補正シフト量（シーン幅に対する比率、既定 0.08 = 8%）
    pub max_shift_ratio: f32,
    /// 回転ブレ補正も有効化
    pub stabilize_rotation: bool,
    /// エッジの黒帯を隠す自動クロップ拡大を有効化
    pub auto_crop_zoom: bool,
}

impl Default for StabilizeConfig {
    fn default() -> Self {
        Self {
            sample_interval_sec: 0.2,
            smoothness: 0.78,
            max_shift_ratio: 0.08,
            stabilize_rotation: true,
            auto_crop_zoom: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraMotionSample {
    pub time: RationalTime,
    /// 累積生カメラ位置 X (シーン px)
    pub raw_x: f32,
    /// 累積生カメラ位置 Y (シーン px)
    pub raw_y: f32,
    /// 累積生回転角 (度)
    pub raw_rot_deg: f32,
    /// ブレ打ち消し補正量 X (シーン px)
    pub comp_x: f32,
    /// ブレ打ち消し補正量 Y (シーン px)
    pub comp_y: f32,
    /// ブレ打ち消し回転補正 (度)
    pub comp_rot_deg: f32,
}

/// 2 フレーム間のグローバル並進 `(dx, dy)` および回転 `d_rot_deg` を 4 象限マルチパッチ SAD マッチングで推定する
pub fn estimate_global_frame_motion(prev: &LinearImage, curr: &LinearImage) -> (f32, f32, f32) {
    let w = prev.width.min(curr.width) as i32;
    let h = prev.height.min(curr.height) as i32;
    if w < 32 || h < 24 {
        return (0.0, 0.0, 0.0);
    }
    let l_prev: Vec<f32> = prev.pixels.iter().map(|p| p.luma()).collect();
    let l_curr: Vec<f32> = curr.pixels.iter().map(|p| p.luma()).collect();
    let pw = prev.width as usize;
    let cw = curr.width as usize;

    // 4 象限パッチ中心（背景・構造物の動きをロバストに捉える）
    let anchors = [
        (w / 4, h / 4),
        ((w * 3) / 4, h / 4),
        (w / 4, (h * 3) / 4),
        ((w * 3) / 4, (h * 3) / 4),
    ];
    let patch_r = (w.min(h) / 10).clamp(4, 12);
    let search_r = (w.min(h) / 9).clamp(6, 16);

    let mut shifts = Vec::with_capacity(4);
    for &(ax, ay) in &anchors {
        let mut best_dx = 0i32;
        let mut best_dy = 0i32;
        let mut best_sad = f32::INFINITY;

        for dy in -search_r..=search_r {
            for dx in -search_r..=search_r {
                let mut sad = 0.0f32;
                let mut cnt = 0u32;
                for oy in -patch_r..=patch_r {
                    let y0 = (ay + oy).clamp(0, h - 1) as usize;
                    let y1 = (ay + oy + dy).clamp(0, h - 1) as usize;
                    for ox in -patch_r..=patch_r {
                        let x0 = (ax + ox).clamp(0, w - 1) as usize;
                        let x1 = (ax + ox + dx).clamp(0, w - 1) as usize;
                        sad += (l_prev[y0 * pw + x0] - l_curr[y1 * cw + x1]).abs();
                        cnt += 1;
                    }
                }
                let avg = sad / cnt.max(1) as f32 + (dx.abs() + dy.abs()) as f32 * 0.0002;
                if avg < best_sad {
                    best_sad = avg;
                    best_dx = dx;
                    best_dy = dy;
                }
            }
        }
        shifts.push((ax as f32, ay as f32, best_dx as f32, best_dy as f32));
    }

    // 平均並進（中央値寄りトリム）
    let mean_dx: f32 = shifts.iter().map(|s| s.2).sum::<f32>() / shifts.len() as f32;
    let mean_dy: f32 = shifts.iter().map(|s| s.3).sum::<f32>() / shifts.len() as f32;

    // 回転角推定: 各パッチの中心からのモーメント角変化
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    let mut num = 0.0f32;
    let mut den = 0.0f32;
    for &(ax, ay, dx, dy) in &shifts {
        let rx = ax - cx;
        let ry = ay - cy;
        let rdx = dx - mean_dx;
        let rdy = dy - mean_dy;
        // 2D 外積 r × dr = |r|^2 * dtheta
        num += rx * rdy - ry * rdx;
        den += rx * rx + ry * ry;
    }
    let d_rot_deg = if den > 1e-3 {
        (num / den).to_degrees().clamp(-12.0, 12.0)
    } else {
        0.0
    };

    (mean_dx, mean_dy, d_rot_deg)
}

/// シーン全体の手ぶれ軌跡を解析し、ゼロ位相平滑化によるブレ打ち消し補正サンプル列を算出する
pub fn analyze_camera_shake(
    graph: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    cfg: &StabilizeConfig,
) -> Vec<CameraMotionSample> {
    let Some(scene) = project.scene(scene_id) else {
        return Vec::new();
    };
    let total_sec = scene.total_duration().as_seconds().max(0.1);
    let max_samples = 60usize;
    let step = (total_sec / max_samples as f64).max(cfg.sample_interval_sec.clamp(0.05, 2.0));

    let analysis_w = 160u32;
    let analysis_h = ((f64::from(analysis_w) * f64::from(scene.height.max(1)))
        / f64::from(scene.width.max(1)))
    .round()
    .clamp(36.0, 240.0) as u32;
    let scale_x = scene.width.max(1) as f32 / analysis_w as f32;
    let scale_y = scene.height.max(1) as f32 / analysis_h as f32;

    let mut samples = Vec::new();
    let mut prev_frame: Option<LinearImage> = None;
    let mut acc_x = 0.0f32;
    let mut acc_y = 0.0f32;
    let mut acc_rot = 0.0f32;
    let mut t = 0.0;

    while t <= total_sec + 1e-6 && samples.len() < max_samples {
        let rt = RationalTime::from_seconds(t.min(total_sec));
        if let Some(frame) =
            graph.render_project_scene_scaled(project, scene_id, rt, analysis_w, analysis_h, false)
        {
            if let Some(prev) = prev_frame.as_ref() {
                let (dx, dy, drot) = estimate_global_frame_motion(prev, &frame);
                acc_x += dx * scale_x;
                acc_y += dy * scale_y;
                acc_rot += drot;
            }
            samples.push(CameraMotionSample {
                time: rt,
                raw_x: acc_x,
                raw_y: acc_y,
                raw_rot_deg: acc_rot,
                comp_x: 0.0,
                comp_y: 0.0,
                comp_rot_deg: 0.0,
            });
            prev_frame = Some(frame);
        }
        t += step;
    }

    compute_shake_compensation(&mut samples, scene.width, scene.height, cfg);
    samples
}

/// 生軌跡へゼロ位相（前後両方向）平滑化を適用し、高周波の手ぶれ成分だけを打ち消す補正量を計算する
pub fn compute_shake_compensation(
    samples: &mut [CameraMotionSample],
    scene_w: u32,
    scene_h: u32,
    cfg: &StabilizeConfig,
) {
    let n = samples.len();
    if n < 2 {
        return;
    }
    let alpha = (1.0 - cfg.smoothness.clamp(0.1, 0.95)).clamp(0.05, 0.9);
    let mut sx: Vec<f32> = samples.iter().map(|s| s.raw_x).collect();
    let mut sy: Vec<f32> = samples.iter().map(|s| s.raw_y).collect();
    let mut sr: Vec<f32> = samples.iter().map(|s| s.raw_rot_deg).collect();

    // Forward pass
    for i in 1..n {
        sx[i] = sx[i - 1] + (sx[i] - sx[i - 1]) * alpha;
        sy[i] = sy[i - 1] + (sy[i] - sy[i - 1]) * alpha;
        sr[i] = sr[i - 1] + (sr[i] - sr[i - 1]) * alpha;
    }
    // Backward pass（ゼロ位相遅れ）
    for i in (0..n - 1).rev() {
        sx[i] = sx[i + 1] + (sx[i] - sx[i + 1]) * alpha;
        sy[i] = sy[i + 1] + (sy[i] - sy[i + 1]) * alpha;
        sr[i] = sr[i + 1] + (sr[i] - sr[i + 1]) * alpha;
    }

    let max_x = scene_w.max(1) as f32 * cfg.max_shift_ratio.clamp(0.01, 0.25);
    let max_y = scene_h.max(1) as f32 * cfg.max_shift_ratio.clamp(0.01, 0.25);

    for (idx, sm) in samples.iter_mut().enumerate() {
        sm.comp_x = (sx[idx] - sm.raw_x).clamp(-max_x, max_x);
        sm.comp_y = (sy[idx] - sm.raw_y).clamp(-max_y, max_y);
        sm.comp_rot_deg = if cfg.stabilize_rotation {
            (sr[idx] - sm.raw_rot_deg).clamp(-8.0, 8.0)
        } else {
            0.0
        };
    }
}

/// 手ぶれ補正サンプル列から、対象クリップ用の 1-Undo キーフレーム＆自動クロップズームコマンド列を生成する
pub fn build_stabilize_commands(
    scene: &Scene,
    item_id: ItemId,
    samples: &[CameraMotionSample],
    cfg: &StabilizeConfig,
) -> Vec<EditCommand> {
    let Some(item) = scene
        .layers
        .iter()
        .flat_map(|l| l.items.iter())
        .find(|i| i.id == item_id)
    else {
        return Vec::new();
    };
    if samples.is_empty() {
        return Vec::new();
    }

    let mut max_dx = 0.0f32;
    let mut max_dy = 0.0f32;
    let mut max_rot = 0.0f32;
    for s in samples {
        max_dx = max_dx.max(s.comp_x.abs());
        max_dy = max_dy.max(s.comp_y.abs());
        max_rot = max_rot.max(s.comp_rot_deg.abs());
    }

    let mut cmds = Vec::new();
    if cfg.auto_crop_zoom {
        let zoom_x = 1.0 + (2.0 * max_dx) / scene.width.max(1) as f32;
        let zoom_y = 1.0 + (2.0 * max_dy) / scene.height.max(1) as f32;
        let rot_pad = 1.0 + max_rot.to_radians().sin().abs() * 0.6;
        let zoom = zoom_x.max(zoom_y).max(rot_pad).clamp(1.0, 1.45);
        let mut next_tf = item.transform;
        next_tf.scale_x = (item.transform.scale_x * zoom).clamp(0.1, 8.0);
        next_tf.scale_y = (item.transform.scale_y * zoom).clamp(0.1, 8.0);
        cmds.push(EditCommand::SetTransform {
            item_id,
            transform: next_tf,
        });
    }

    for s in samples {
        if s.time < item.start || s.time > item.end() {
            continue;
        }
        let local_t = s.time - item.start;
        cmds.push(EditCommand::SetKeyframe {
            item_id,
            property: "x".into(),
            keyframe: Keyframe {
                time: local_t,
                value: item.transform.x + s.comp_x,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
            },
        });
        cmds.push(EditCommand::SetKeyframe {
            item_id,
            property: "y".into(),
            keyframe: Keyframe {
                time: local_t,
                value: item.transform.y + s.comp_y,
                interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
            },
        });
        if cfg.stabilize_rotation {
            cmds.push(EditCommand::SetKeyframe {
                item_id,
                property: "rotation_deg".into(),
                keyframe: Keyframe {
                    time: local_t,
                    value: item.transform.rotation_deg + s.comp_rot_deg,
                    interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                },
            });
        }
    }

    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::RgbaColor;

    #[test]
    fn corner_pin_homography_warps_image_into_trapezoid_accurately() {
        let white = LinearPremul::from_srgb_straight(RgbaColor::WHITE);
        let src = LinearImage::new(64, 64, white);
        // 上辺を中央へ狭めた台形（遠近法モニタ投影）
        let cfg = CornerPinConfig {
            enabled: true,
            tl: [0.25, 0.15],
            tr: [0.75, 0.15],
            br: [0.95, 0.90],
            bl: [0.05, 0.90],
            feather_px: 1.0,
        };
        let warped = apply_corner_pin(&src, &cfg);
        // 左上隅 (2, 2) は台形の外なので透明、中央 (32, 32) は台形の内側なので不透明白
        assert!(warped.pixels[2 * 64 + 2].a < 0.05);
        assert!(warped.pixels[32 * 64 + 32].a > 0.95);
        assert!(warped.pixels[32 * 64 + 32].luma() > 0.9);
    }

    #[test]
    fn global_motion_and_stabilizer_cancel_high_frequency_shake() {
        let black = LinearPremul::from_srgb_straight(RgbaColor::BLACK);
        let white = LinearPremul::from_srgb_straight(RgbaColor::WHITE);
        let mut a = LinearImage::new(80, 60, black);
        let mut b = LinearImage::new(80, 60, black);
        // 4象限すべてに特徴ブロックを描画し、Frame B を (+5, -3) だけカメラブレさせる
        let boxes = [(16, 12), (56, 12), (16, 42), (56, 42)];
        for &(bx, by) in &boxes {
            for oy in -3..=3i32 {
                for ox in -3..=3i32 {
                    a.pixels[((by + oy) * 80 + (bx + ox)) as usize] = white;
                    b.pixels[((by + oy - 3) * 80 + (bx + ox + 5)) as usize] = white;
                }
            }
        }
        let (dx, dy, _rot) = estimate_global_frame_motion(&a, &b);
        assert!((dx - 5.0).abs() <= 1.2, "expected dx≈5, got {dx}");
        assert!((dy - (-3.0)).abs() <= 1.2, "expected dy≈-3, got {dy}");

        // ジグザグ手ぶれ軌跡の平滑化補償チェック
        let mut samples: Vec<CameraMotionSample> = (0..10)
            .map(|i| CameraMotionSample {
                time: RationalTime::from_seconds(i as f64 * 0.2),
                raw_x: if i % 2 == 0 { 12.0 } else { -12.0 },
                raw_y: 0.0,
                raw_rot_deg: 0.0,
                comp_x: 0.0,
                comp_y: 0.0,
                comp_rot_deg: 0.0,
            })
            .collect();
        compute_shake_compensation(&mut samples, 1280, 720, &StabilizeConfig::default());
        // 偶数フレーム (+12) には負の打ち消し補正、奇数フレーム (-12) には正の打ち消し補正が入る
        assert!(samples[2].comp_x < -4.0);
        assert!(samples[3].comp_x > 4.0);
    }
}
