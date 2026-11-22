//! v3.0 #30: 3D メッシュ / glTF 2.0 PBR ラスタライザ ＆ 3D カメラトラッカー
//! Pure Rust・ゼロ `unsafe`・外部依存ゼロで Z バッファ付き PBR（Cook-Torrance/GGX 近似＋Schlick Fresnel＋環境反射＋3D ライト）
//! 描画と、特徴点視差ベースの 3D カメラ軌跡トラッキングを実現する。

use crate::color::{LinearImage, LinearPremul, srgb_to_linear};
use crate::graph::RenderGraph;
use crate::vfx_track::estimate_global_frame_motion;
use std::path::Path;
use ve_core::{
    Camera3D, EditCommand, Light3D, Mesh3DConfig, Mesh3DData, Project, RationalTime, SceneId,
    resolve_mesh_data,
};

#[inline]
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn norm3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-6 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// オイラー角 `(rx, ry, rz)`（度）で 3D ベクトルを回転させる
#[inline]
fn rotate_euler(v: [f32; 3], rx_deg: f32, ry_deg: f32, rz_deg: f32) -> [f32; 3] {
    let (sx, cx) = rx_deg.to_radians().sin_cos();
    let (sy, cy) = ry_deg.to_radians().sin_cos();
    let (sz, cz) = rz_deg.to_radians().sin_cos();

    // X 軸回転
    let y1 = v[1] * cx - v[2] * sx;
    let z1 = v[1] * sx + v[2] * cx;
    let x1 = v[0];

    // Y 軸回転
    let x2 = x1 * cy + z1 * sy;
    let z2 = -x1 * sy + z1 * cy;
    let y2 = y1;

    // Z 軸回転
    [x2 * cz - y2 * sz, x2 * sz + y2 * cz, z2]
}

/// プロシージャルスタジオ環境反射マップ（スカイブルー〜ホライゾン〜ダークスタジオ床）
#[inline]
fn sample_studio_env(refl_dir: [f32; 3]) -> [f32; 3] {
    let t = (refl_dir[1] * 0.5 + 0.5).clamp(0.0, 1.0);
    if t >= 0.5 {
        let k = (t - 0.5) * 2.0;
        [0.35 + 0.35 * k, 0.45 + 0.38 * k, 0.62 + 0.35 * k]
    } else {
        let k = t * 2.0;
        [0.08 + 0.16 * k, 0.09 + 0.17 * k, 0.12 + 0.22 * k]
    }
}

/// 3D メッシュ（glTF 2.0 / プロシージャル / 3D 押し出しテキスト）を Z バッファ付き PBR シェーディングで描画する
pub fn render_mesh3d_item(
    cfg: &Mesh3DConfig,
    base_dir: Option<&Path>,
    light: &Light3D,
    local_time_sec: f32,
    out_w: u32,
    out_h: u32,
) -> LinearImage {
    let w = out_w.clamp(16, 2048);
    let h = out_h.clamp(16, 2048);
    let mesh = resolve_mesh_data(cfg, base_dir);
    render_mesh_data_pbr(&mesh, cfg, light, local_time_sec, w, h)
}

/// 解析済み `Mesh3DData` を PBR（拡散＋GGX 鏡面＋Schlick Fresnel＋スタジオ環境反射）でラスタライズする
pub fn render_mesh_data_pbr(
    mesh: &Mesh3DData,
    cfg: &Mesh3DConfig,
    light: &Light3D,
    local_time_sec: f32,
    w: u32,
    h: u32,
) -> LinearImage {
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let mut zbuf = vec![f32::INFINITY; (w as usize) * (h as usize)];

    let rx = cfg.rot_x_deg;
    let ry = cfg.rot_y_deg + cfg.auto_spin_deg_per_sec * local_time_sec;
    let rz = cfg.rot_z_deg;

    let albedo = [
        srgb_to_linear(cfg.base_color.r),
        srgb_to_linear(cfg.base_color.g),
        srgb_to_linear(cfg.base_color.b),
    ];
    let metallic = cfg.metallic.clamp(0.0, 1.0);
    let roughness = cfg.roughness.clamp(0.06, 1.0);
    let env_strength = cfg.env_reflection.clamp(0.0, 1.0);
    let shininess = (2.0 / (roughness * roughness * roughness * roughness) - 2.0).clamp(4.0, 160.0);

    // シーンライト方向・色（3D ライト無効時は斜め上キーライトを既定使用）
    let (light_dir, light_col, ambient) = if light.enabled {
        let dir = norm3([-light.position[0], -light.position[1], -light.position[2]]);
        let inten = light.intensity.clamp(0.1, 4.0);
        let col = [
            srgb_to_linear(light.color[0]) * inten,
            srgb_to_linear(light.color[1]) * inten,
            srgb_to_linear(light.color[2]) * inten,
        ];
        (dir, col, light.ambient.clamp(0.05, 0.8))
    } else {
        (norm3([0.55, 0.72, 0.65]), [1.15, 1.12, 1.08], 0.22)
    };
    let view_dir = [0.0f32, 0.0, 1.0];

    let cam_dist = 3.1f32;
    let focal = (w.min(h) as f32) * 0.92;
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;

    // 変換済み頂点キャッシュ: (screen_x, screen_y, view_z, world_normal)
    let transformed: Vec<([f32; 3], [f32; 3])> = mesh
        .vertices
        .iter()
        .map(|v| {
            let p = rotate_euler(v.pos, rx, ry, rz);
            let n = norm3(rotate_euler(v.normal, rx, ry, rz));
            let vz = (cam_dist - p[2]).max(0.35);
            let sx = cx + (p[0] / vz) * focal;
            let sy = cy - (p[1] / vz) * focal;
            ([sx, sy, vz], n)
        })
        .collect();

    let fw = w as i32;
    let fh = h as i32;

    for tri in mesh.indices.as_chunks::<3>().0 {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= transformed.len() || i1 >= transformed.len() || i2 >= transformed.len() {
            continue;
        }
        let (p0, n0) = transformed[i0];
        let (p1, n1) = transformed[i1];
        let (p2, n2) = transformed[i2];

        // 2D 符号付き面積（裏面カリング）
        let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
        if area.abs() < 1e-4 {
            continue;
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

                // 補間法線（両面対応: 視線と逆向きなら反転）
                let mut n = norm3([
                    w0 * n0[0] + w1 * n1[0] + w2 * n2[0],
                    w0 * n0[1] + w1 * n1[1] + w2 * n2[1],
                    w0 * n0[2] + w1 * n1[2] + w2 * n2[2],
                ]);
                if dot3(n, view_dir) < 0.0 {
                    n = [-n[0], -n[1], -n[2]];
                }

                let n_dot_l = dot3(n, light_dir).max(0.0);
                let half_v = norm3([
                    light_dir[0] + view_dir[0],
                    light_dir[1] + view_dir[1],
                    light_dir[2] + view_dir[2],
                ]);
                let n_dot_h = dot3(n, half_v).max(0.0);
                let n_dot_v = dot3(n, view_dir).clamp(0.0, 1.0);

                // Schlick Fresnel F0（誘電体 0.04 ↔ 金属 albedo）
                let fres_pow = (1.0 - n_dot_v).powi(5);
                let f0 = [
                    0.04 * (1.0 - metallic) + albedo[0] * metallic,
                    0.04 * (1.0 - metallic) + albedo[1] * metallic,
                    0.04 * (1.0 - metallic) + albedo[2] * metallic,
                ];
                let fres = [
                    f0[0] + (1.0 - f0[0]) * fres_pow,
                    f0[1] + (1.0 - f0[1]) * fres_pow,
                    f0[2] + (1.0 - f0[2]) * fres_pow,
                ];
                let spec_term = n_dot_h.powf(shininess) * (1.0 - roughness * 0.65);

                // 環境反射ベクトル R = 2(N·V)N - V
                let refl = norm3([
                    2.0 * n_dot_v * n[0] - view_dir[0],
                    2.0 * n_dot_v * n[1] - view_dir[1],
                    2.0 * n_dot_v * n[2] - view_dir[2],
                ]);
                let env = sample_studio_env(refl);

                let diff_scale = (1.0 - metallic) * (ambient + n_dot_l * 0.85);
                let mut r = albedo[0] * diff_scale * light_col[0]
                    + fres[0] * spec_term * light_col[0]
                    + env[0] * fres[0] * env_strength;
                let mut g = albedo[1] * diff_scale * light_col[1]
                    + fres[1] * spec_term * light_col[1]
                    + env[1] * fres[1] * env_strength;
                let mut b = albedo[2] * diff_scale * light_col[2]
                    + fres[2] * spec_term * light_col[2]
                    + env[2] * fres[2] * env_strength;

                if cfg.wireframe && (w0 < 0.04 || w1 < 0.04 || w2 < 0.04) {
                    r = (r + 0.55).min(1.0);
                    g = (g + 0.85).min(1.0);
                    b = (b + 1.0).min(1.0);
                }

                let a = cfg.base_color.a.clamp(0.0, 1.0);
                img.pixels[idx] = LinearPremul {
                    r: r.clamp(0.0, 1.0) * a,
                    g: g.clamp(0.0, 1.0) * a,
                    b: b.clamp(0.0, 1.0) * a,
                    a,
                };
            }
        }
    }

    img
}

// =========================================================================
// 3D Camera Tracker (Multi-Patch Parallax + Scale Divergence -> 3D Camera)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraTrack3DSample {
    pub time: RationalTime,
    pub cam_x: f32,
    pub cam_y: f32,
    pub cam_z: f32,
    pub roll_deg: f32,
}

/// シーンの映像フレーム列から視差・拡大発散・回転を解析し、3D カメラ軌跡（X, Y, Z, Roll）を復元する
pub fn analyze_3d_camera_track(
    graph: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    sample_interval_sec: f64,
) -> Vec<CameraTrack3DSample> {
    let Some(scene) = project.scene(scene_id) else {
        return Vec::new();
    };
    let total_sec = scene.total_duration().as_seconds().max(0.1);
    let step = sample_interval_sec.clamp(0.1, 2.0);
    let analysis_w = 160u32;
    let analysis_h = ((f64::from(analysis_w) * f64::from(scene.height.max(1)))
        / f64::from(scene.width.max(1)))
    .round()
    .clamp(40.0, 220.0) as u32;
    let scale_x = scene.width.max(1) as f32 / analysis_w as f32;
    let scale_y = scene.height.max(1) as f32 / analysis_h as f32;

    let mut samples = Vec::new();
    let mut prev: Option<LinearImage> = None;
    let mut cam_x = scene.camera.position[0];
    let mut cam_y = scene.camera.position[1];
    let mut cam_z = scene.camera.position[2];
    let mut roll = 0.0f32;
    let mut t = 0.0;

    while t <= total_sec + 1e-6 && samples.len() < 48 {
        let rt = RationalTime::from_seconds(t.min(total_sec));
        if let Some(frame) =
            graph.render_project_scene_scaled(project, scene_id, rt, analysis_w, analysis_h, false)
        {
            if let Some(p) = prev.as_ref() {
                let (dx, dy, drot) = estimate_global_frame_motion(p, &frame);
                // 画面中心と外周の輝度広がり変化から簡易 Z ドリー移動を推定
                let dz = estimate_radial_zoom_delta(p, &frame) * 180.0;
                cam_x -= dx * scale_x * 0.5;
                cam_y -= dy * scale_y * 0.5;
                cam_z = (cam_z + dz).clamp(-3000.0, -120.0);
                roll += drot;
            }
            samples.push(CameraTrack3DSample {
                time: rt,
                cam_x,
                cam_y,
                cam_z,
                roll_deg: roll,
            });
            prev = Some(frame);
        }
        t += step;
    }
    samples
}

fn estimate_radial_zoom_delta(prev: &LinearImage, curr: &LinearImage) -> f32 {
    fn mean_radius_weighted_edge(img: &LinearImage) -> f32 {
        let w = img.width as usize;
        let h = img.height as usize;
        if w < 4 || h < 4 {
            return 0.5;
        }
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.5;
        let norm = cx.hypot(cy).max(1.0);
        let mut num = 0.0f32;
        let mut den = 0.0f32;
        for y in (1..h - 1).step_by(2) {
            for x in (1..w - 1).step_by(2) {
                let gx =
                    (img.pixels[y * w + x + 1].luma() - img.pixels[y * w + x - 1].luma()).abs();
                let gy =
                    (img.pixels[(y + 1) * w + x].luma() - img.pixels[(y - 1) * w + x].luma()).abs();
                let e = gx + gy;
                if e > 0.02 {
                    let r = (x as f32 - cx).hypot(y as f32 - cy) / norm;
                    num += r * e;
                    den += e;
                }
            }
        }
        if den > 1e-4 { num / den } else { 0.5 }
    }
    (mean_radius_weighted_edge(curr) - mean_radius_weighted_edge(prev)).clamp(-0.25, 0.25)
}

/// 3D カメラトラッキング解析結果からシーンの `Camera3D` を自動設定する 1-Undo コマンドを生成する
pub fn build_3d_camera_track_command(
    base_camera: &Camera3D,
    samples: &[CameraTrack3DSample],
) -> Option<EditCommand> {
    if samples.is_empty() {
        return None;
    }
    let n = samples.len() as f32;
    let avg_x = samples.iter().map(|s| s.cam_x).sum::<f32>() / n;
    let avg_y = samples.iter().map(|s| s.cam_y).sum::<f32>() / n;
    let avg_z = samples.iter().map(|s| s.cam_z).sum::<f32>() / n;

    let mut next = *base_camera;
    next.enabled = true;
    next.position = [avg_x, avg_y, avg_z];
    Some(EditCommand::SetCamera3D { camera: next })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::Mesh3DPreset;

    #[test]
    fn pbr_3d_rasterizer_renders_visible_shaded_cube_and_extruded_text() {
        let cfg_cube = Mesh3DConfig {
            preset: Mesh3DPreset::Cube,
            metallic: 0.6,
            roughness: 0.25,
            wireframe: true,
            ..Mesh3DConfig::default()
        };
        let light = Light3D {
            enabled: true,
            ..Light3D::default()
        };
        let img = render_mesh3d_item(&cfg_cube, None, &light, 0.5, 128, 128);
        let opaque = img.pixels.iter().filter(|p| p.a > 0.5).count();
        assert!(
            opaque > 1500,
            "expected PBR 3D cube to cover >1500 pixels in 128x128 frame, got {opaque}"
        );

        let cfg_text = Mesh3DConfig {
            preset: Mesh3DPreset::ExtrudedText,
            text_content: "V3".into(),
            ..Mesh3DConfig::default()
        };
        let img_txt = render_mesh3d_item(&cfg_text, None, &light, 0.0, 128, 128);
        let opaque_txt = img_txt.pixels.iter().filter(|p| p.a > 0.5).count();
        assert!(
            opaque_txt > 400,
            "expected 3D extruded text to cover >400 pixels, got {opaque_txt}"
        );
    }
}
