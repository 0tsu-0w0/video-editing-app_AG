use serde::{Deserialize, Serialize};

/// レイヤー／アイテムの 3D 空間配置（仕様書 2.2 / Phase 3）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Transform3D {
    pub enable_3d: bool,
    /// 奥行き座標（0 = 基準面、正値 = 奥、負値 = 手前）
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
}

/// シーンの 3D カメラ制御・被写界深度設定（仕様書 2.2）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera3D {
    pub enabled: bool,
    /// カメラ位置 [x, y, z]（既定: z = -1000 付近から原点を注視）
    pub position: [f32; 3],
    /// 注視点 [x, y, z]
    pub target: [f32; 3],
    /// 垂直視野角（度）
    pub fov_deg: f32,
    /// 被写界深度（ボケ表現）有効フラグ
    pub dof_enabled: bool,
    /// 合焦距離（カメラからの距離）
    pub focus_distance: f32,
    /// 絞り強度（ボケの広がり係数）
    pub aperture: f32,
}

impl Default for Camera3D {
    fn default() -> Self {
        Self {
            enabled: false,
            position: [0.0, 0.0, -1000.0],
            target: [0.0, 0.0, 0.0],
            fov_deg: 50.0,
            dof_enabled: false,
            focus_distance: 1000.0,
            aperture: 0.02,
        }
    }
}

/// 3D カメラによる透視投影結果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projected3D {
    /// キャンバス上の平行移動オフセット (px)
    pub screen_offset_x: f32,
    pub screen_offset_y: f32,
    /// 透視投影スケール（近いほど大きく、遠いほど小さい）
    pub perspective_scale: f32,
    /// カメラからの距離（Zソートおよび被写界深度ボケ計算用）
    pub camera_depth: f32,
    /// 被写界深度によるボケ半径（ピクセル、0 = 合焦）
    pub dof_blur_radius: i32,
}

impl Camera3D {
    /// ワールド座標 `(world_x, world_y, world_z)`（キャンバス中心原点）をカメラ空間へ投影する
    pub fn project(
        &self,
        world_x: f32,
        world_y: f32,
        world_z: f32,
        canvas_height: u32,
    ) -> Option<Projected3D> {
        let forward = normalize3([
            self.target[0] - self.position[0],
            self.target[1] - self.position[1],
            self.target[2] - self.position[2],
        ]);
        let world_up = [0.0, 1.0, 0.0];
        let right = normalize3(cross3(world_up, forward));
        let up = cross3(forward, right);

        let rel = [
            world_x - self.position[0],
            world_y - self.position[1],
            world_z - self.position[2],
        ];

        let cam_x = dot3(rel, right);
        let cam_y = dot3(rel, up);
        let cam_z = dot3(rel, forward);

        // カメラ背面はクリップ
        if cam_z <= 10.0 {
            return None;
        }

        let fov_rad = self.fov_deg.clamp(10.0, 140.0).to_radians();
        let focal_px = (canvas_height.max(1) as f32 * 0.5) / (fov_rad * 0.5).tan();
        let ref_depth = (self.target[2] - self.position[2]).abs().max(100.0);

        let perspective_scale = (ref_depth / cam_z).clamp(0.05, 8.0);
        let screen_offset_x = (cam_x / cam_z) * focal_px;
        let screen_offset_y = (cam_y / cam_z) * focal_px;

        let dof_blur_radius = if self.dof_enabled {
            let defocus = (cam_z - self.focus_distance).abs() * self.aperture.max(0.0);
            (defocus * 0.1).round().clamp(0.0, 14.0) as i32
        } else {
            0
        };

        Some(Projected3D {
            screen_offset_x,
            screen_offset_y,
            perspective_scale,
            camera_depth: cam_z,
            dof_blur_radius,
        })
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= 1e-6 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closer_object_projects_larger_and_out_of_focus_gets_dof_blur() {
        let cam = Camera3D {
            enabled: true,
            position: [0.0, 0.0, -1000.0],
            target: [0.0, 0.0, 0.0],
            fov_deg: 50.0,
            dof_enabled: true,
            focus_distance: 1000.0,
            aperture: 0.05,
        };
        let near = cam.project(0.0, 0.0, -500.0, 720).unwrap();
        let focus = cam.project(0.0, 0.0, 0.0, 720).unwrap();
        let far = cam.project(0.0, 0.0, 1000.0, 720).unwrap();

        assert!(near.perspective_scale > focus.perspective_scale);
        assert!(focus.perspective_scale > far.perspective_scale);
        assert_eq!(focus.dof_blur_radius, 0);
        assert!(near.dof_blur_radius > 0);
        assert!(far.dof_blur_radius > 0);
    }
}
