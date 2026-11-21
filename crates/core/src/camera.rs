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

/// v0.9 #16: 3D カメラ空間のライト種別
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Light3DType {
    #[default]
    Directional,
    Point,
}

/// v0.9 #16: 3D カメラ空間のディレクショナル／ポイントライト＆ドロップシャドウキャッチャー設定
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Light3D {
    pub enabled: bool,
    pub light_type: Light3DType,
    /// ライト座標（Point 時）または入射方向ベクトル（Directional 時）
    pub position: [f32; 3],
    /// ライト色（リニア RGB 0.0..=1.0）
    pub color: [f32; 3],
    /// ライト強度（0.0..=3.0）
    pub intensity: f32,
    /// 環境光ベース（0.0..=1.0、暗部が完全な黒にならないよう維持）
    pub ambient: f32,
    /// 背景面へのドロップシャドウキャッチャー有効フラグ
    pub cast_shadow: bool,
    /// 影のぼかし半径（px）
    pub shadow_softness: f32,
    /// 影の不透明度（0.0..=1.0）
    pub shadow_opacity: f32,
}

impl Default for Light3D {
    fn default() -> Self {
        Self {
            enabled: false,
            light_type: Light3DType::Directional,
            position: [-350.0, -450.0, -700.0],
            color: [1.0, 0.98, 0.94],
            intensity: 1.15,
            ambient: 0.35,
            cast_shadow: true,
            shadow_softness: 8.0,
            shadow_opacity: 0.45,
        }
    }
}

/// シーンの 3D カメラ制御・被写界深度・ライティング設定（仕様書 2.2 / v0.9 #16）
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
    /// v0.9 #16: 3D ディレクショナル／ポイントライト＆シャドウキャッチャー
    #[serde(default)]
    pub light: Light3D,
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
            light: Light3D::default(),
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

    /// v0.9 #16: 3D 空間上のレイヤー面に対するランバート照明係数 `[r, g, b]` と
    /// ドロップシャドウ投影オフセット `[dx, dy]` (px) を算出する。
    pub fn compute_layer_lighting(
        &self,
        world_pos: [f32; 3],
        rotation_x_deg: f32,
        rotation_y_deg: f32,
    ) -> ([f32; 3], [f32; 2]) {
        if !self.light.enabled {
            return ([1.0, 1.0, 1.0], [0.0, 0.0]);
        }
        let rx = rotation_x_deg.to_radians();
        let ry = rotation_y_deg.to_radians();
        // 基準平面法線 (0, 0, -1) を X/Y 軸回転させた法線ベクトル
        let normal = normalize3([ry.sin() * rx.cos(), -rx.sin(), -(rx.cos() * ry.cos())]);

        let (light_dir, attenuation) = match self.light.light_type {
            Light3DType::Directional => {
                // ライト位置から原点へ向かう方向の逆＝表面からライトへの単位ベクトル
                let dir = normalize3([
                    self.light.position[0],
                    self.light.position[1],
                    self.light.position[2],
                ]);
                (dir, 1.0_f32)
            }
            Light3DType::Point => {
                let dx = self.light.position[0] - world_pos[0];
                let dy = self.light.position[1] - world_pos[1];
                let dz = self.light.position[2] - world_pos[2];
                let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);
                // 1000px 基準の穏やかな距離減衰
                let atten = (1.0 / (1.0 + (dist / 1200.0).powi(2))).clamp(0.15, 2.0);
                ([dx / dist, dy / dist, dz / dist], atten)
            }
        };

        // 両面照明（裏面でも暗くなりすぎないよう abs を緩やかに併用）
        let ndotl = dot3(normal, light_dir).abs().clamp(0.0, 1.0);
        let amb = self.light.ambient.clamp(0.0, 1.5);
        let diff = ndotl * self.light.intensity.clamp(0.0, 3.0) * attenuation;
        let tint = [
            (amb + diff * self.light.color[0].clamp(0.0, 2.0)).clamp(0.05, 3.0),
            (amb + diff * self.light.color[1].clamp(0.0, 2.0)).clamp(0.05, 3.0),
            (amb + diff * self.light.color[2].clamp(0.0, 2.0)).clamp(0.05, 3.0),
        ];

        // ライト反対方向へのドロップシャドウ投影オフセット（Z 浮き量に応じて広がる）
        let lift = (120.0 - world_pos[2]).clamp(24.0, 360.0) * 0.14;
        let shadow_dx = (-light_dir[0] * lift).clamp(-96.0, 96.0);
        let shadow_dy = (-light_dir[1] * lift).clamp(-96.0, 96.0);
        (tint, [shadow_dx, shadow_dy])
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
            light: Light3D::default(),
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

    #[test]
    fn directional_and_point_light_shade_tilted_layers_and_project_shadow() {
        let mut cam = Camera3D {
            enabled: true,
            ..Camera3D::default()
        };
        cam.light.enabled = true;
        cam.light.light_type = Light3DType::Directional;
        cam.light.position = [-400.0, -400.0, -800.0];

        let (front_tint, shadow_off) = cam.compute_layer_lighting([0.0, 0.0, 0.0], 0.0, 0.0);
        let (steep_tint, _) = cam.compute_layer_lighting([0.0, 0.0, 0.0], 0.0, 78.0);
        assert!(front_tint[0] > steep_tint[0]);
        // 左上ライトなので影は右下（+x, +y）へ落ちる
        assert!(shadow_off[0] > 0.0 && shadow_off[1] > 0.0);

        cam.light.light_type = Light3DType::Point;
        let (near_pt, _) = cam.compute_layer_lighting([-300.0, -300.0, -600.0], 0.0, 0.0);
        let (far_pt, _) = cam.compute_layer_lighting([1500.0, 1200.0, 900.0], 0.0, 0.0);
        assert!(near_pt[0] > far_pt[0]);
    }
}
