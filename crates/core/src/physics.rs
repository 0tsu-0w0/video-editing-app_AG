//! v4.0 #34: 2D/3D リジッドボディ物理演算シミュレーションエンジン
//! Pure Rust・ゼロ `unsafe`・決定論的固定タイムステップ（60Hz）で重力・床/壁バウンス・
//! 剛体同士の弾性衝突インパルス・衝突トルク回転をシミュレートし、1-Undo キーフレームへ自動ベイクする。

use crate::animation::{CubicBezier, Interpolation, Keyframe};
use crate::command::EditCommand;
use crate::project::{ItemId, Scene};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsPreset {
    #[default]
    BouncingTitle,
    DominoCascade,
    ZeroGravityFloat,
    OrbitalVortex,
}

impl PhysicsPreset {
    pub const ALL: &'static [Self] = &[
        Self::BouncingTitle,
        Self::DominoCascade,
        Self::ZeroGravityFloat,
        Self::OrbitalVortex,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::BouncingTitle => "🏀 バウンス落下登場 (Bouncing Drop)",
            Self::DominoCascade => "💥 ドミノ連鎖衝突 (Domino Impulse)",
            Self::ZeroGravityFloat => "🪐 無重力フローティング衝突 (Zero-G)",
            Self::OrbitalVortex => "🌀 軌道重力ボテックス (Orbital Attract)",
        }
    }

    pub fn default_config(self) -> PhysicsConfig {
        match self {
            Self::BouncingTitle => PhysicsConfig {
                preset: self,
                gravity_y: 1450.0,
                restitution: 0.68,
                friction: 0.92,
                air_drag: 0.04,
                initial_vx: 65.0,
                initial_vy: -180.0,
                initial_spin_deg: 28.0,
                ..PhysicsConfig::default()
            },
            Self::DominoCascade => PhysicsConfig {
                preset: self,
                gravity_y: 980.0,
                restitution: 0.85,
                friction: 0.95,
                air_drag: 0.02,
                initial_vx: 460.0,
                initial_vy: -40.0,
                initial_spin_deg: 95.0,
                ..PhysicsConfig::default()
            },
            Self::ZeroGravityFloat => PhysicsConfig {
                preset: self,
                gravity_y: 0.0,
                restitution: 0.94,
                friction: 0.99,
                air_drag: 0.01,
                initial_vx: 190.0,
                initial_vy: 135.0,
                initial_spin_deg: 42.0,
                ..PhysicsConfig::default()
            },
            Self::OrbitalVortex => PhysicsConfig {
                preset: self,
                gravity_y: 0.0,
                restitution: 0.78,
                friction: 0.96,
                air_drag: 0.02,
                initial_vx: 260.0,
                initial_vy: -140.0,
                initial_spin_deg: 55.0,
                central_attract: 680.0,
                ..PhysicsConfig::default()
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsConfig {
    #[serde(default)]
    pub preset: PhysicsPreset,
    /// 下向き重力加速度 (px/s^2)
    pub gravity_y: f32,
    /// 反発係数 (0.0..=1.0)
    pub restitution: f32,
    /// 床・壁接触時の接線減衰係数 (0.5..=1.0)
    pub friction: f32,
    /// 空気抵抗 (0.0..=0.5)
    pub air_drag: f32,
    /// 初期水平速度 (px/s)
    pub initial_vx: f32,
    /// 初期垂直速度 (px/s)
    pub initial_vy: f32,
    /// 初期角速度 (deg/s)
    pub initial_spin_deg: f32,
    /// 画面中央への引力（OrbitalVortex 用）
    #[serde(default)]
    pub central_attract: f32,
    /// シミュレーション時間（秒）
    pub duration_sec: f64,
    /// キーフレーム焼き込みステップ（フレーム間隔、既定 3 = 20fps キー）
    pub bake_step_frames: u32,
    /// 床 Y 位置比率（シーン高さ比 0.2..=0.98）
    pub floor_ratio: f32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            preset: PhysicsPreset::BouncingTitle,
            gravity_y: 1450.0,
            restitution: 0.68,
            friction: 0.92,
            air_drag: 0.04,
            initial_vx: 65.0,
            initial_vy: -180.0,
            initial_spin_deg: 28.0,
            central_attract: 0.0,
            duration_sec: 3.0,
            bake_step_frames: 3,
            floor_ratio: 0.84,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsTrajectorySample {
    pub time_sec: f64,
    pub x: f32,
    pub y: f32,
    pub rotation_deg: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsBodyTrajectory {
    pub item_id: ItemId,
    pub item_name: String,
    pub samples: Vec<PhysicsTrajectorySample>,
    pub bounce_count: u32,
}

#[derive(Debug, Clone)]
struct SimBody {
    item_id: ItemId,
    name: String,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    rot_deg: f32,
    omega_deg: f32,
    radius: f32,
    mass: f32,
    bounces: u32,
}

/// シーン上の指定クリップ群に対して決定論的 60Hz リジッドボディ物理シミュレーションを実行する
pub fn simulate_scene_physics(
    scene: &Scene,
    item_ids: &[ItemId],
    cfg: &PhysicsConfig,
) -> Vec<PhysicsBodyTrajectory> {
    let w = scene.width.max(64) as f32;
    let h = scene.height.max(64) as f32;
    let floor_y = h * cfg.floor_ratio.clamp(0.2, 0.98);
    let wall_pad = 36.0f32;

    let mut bodies: Vec<SimBody> = Vec::new();
    for (idx, &id) in item_ids.iter().enumerate() {
        if let Some(item) = scene
            .layers
            .iter()
            .flat_map(|l| l.items.iter())
            .find(|i| i.id == id)
        {
            // DominoCascade では先頭オブジェクトだけ強く押し出し、後続は連鎖衝突で動き出す
            let impulse_scale = if cfg.preset == PhysicsPreset::DominoCascade && idx > 0 {
                0.05
            } else {
                1.0
            };
            let phase = (idx as f32) * 0.7;
            bodies.push(SimBody {
                item_id: item.id,
                name: item.name.clone(),
                x: item.transform.x,
                y: item.transform.y,
                vx: cfg.initial_vx * impulse_scale * if idx % 2 == 0 { 1.0 } else { -0.75 },
                vy: cfg.initial_vy * impulse_scale + phase.sin() * 25.0,
                rot_deg: item.transform.rotation_deg,
                omega_deg: cfg.initial_spin_deg * impulse_scale,
                radius: 56.0 * item.transform.scale_x.clamp(0.3, 3.0),
                mass: 1.0,
                bounces: 0,
            });
        }
    }

    if bodies.is_empty() {
        return Vec::new();
    }

    let dur = cfg.duration_sec.clamp(0.2, 30.0);
    let dt = 1.0f32 / 60.0;
    let total_steps = (dur * 60.0).round() as usize;
    let sample_stride = cfg.bake_step_frames.clamp(1, 30) as usize;
    let rest = cfg.restitution.clamp(0.0, 1.0);
    let fric = cfg.friction.clamp(0.5, 1.0);
    let drag = (1.0 - cfg.air_drag.clamp(0.0, 0.5) * dt).clamp(0.8, 1.0);
    let cx = w * 0.5;
    let cy = h * 0.45;

    let mut trajectories: Vec<Vec<PhysicsTrajectorySample>> =
        vec![Vec::with_capacity(total_steps / sample_stride + 2); bodies.len()];

    for step in 0..=total_steps {
        let t_sec = step as f64 / 60.0;
        if step % sample_stride == 0 || step == total_steps {
            for (b, tr) in bodies.iter().zip(trajectories.iter_mut()) {
                tr.push(PhysicsTrajectorySample {
                    time_sec: t_sec,
                    x: b.x,
                    y: b.y,
                    rotation_deg: b.rot_deg,
                });
            }
        }
        if step == total_steps {
            break;
        }

        // 1. 外力積分（重力＋中心引力＋空気抵抗）
        for b in &mut bodies {
            b.vy += cfg.gravity_y * dt;
            if cfg.central_attract.abs() > 1e-3 {
                let dx = cx - b.x;
                let dy = cy - b.y;
                let dist = dx.hypot(dy).max(40.0);
                let acc = cfg.central_attract / (dist / 220.0).max(0.5);
                b.vx += (dx / dist) * acc * dt;
                b.vy += (dy / dist) * acc * dt;
            }
            b.vx *= drag;
            b.vy *= drag;
            b.omega_deg *= drag;

            b.x += b.vx * dt;
            b.y += b.vy * dt;
            b.rot_deg += b.omega_deg * dt;

            // 床・天井・左右壁との衝突＆摩擦回転
            if b.y + b.radius > floor_y {
                b.y = floor_y - b.radius;
                if b.vy > 24.0 {
                    b.bounces += 1;
                    b.omega_deg += b.vx * 0.14;
                }
                b.vy = -b.vy * rest;
                b.vx *= fric;
                if b.vy.abs() < 18.0 && cfg.gravity_y > 10.0 {
                    b.vy = 0.0;
                    b.omega_deg *= 0.85;
                }
            }
            if b.y < wall_pad {
                b.y = wall_pad;
                b.vy = b.vy.abs() * rest;
            }
            if b.x < wall_pad {
                b.x = wall_pad;
                b.vx = b.vx.abs() * rest;
                b.bounces += 1;
            } else if b.x + b.radius > w - wall_pad {
                b.x = w - wall_pad - b.radius;
                b.vx = -b.vx.abs() * rest;
                b.bounces += 1;
            }
        }

        // 2. 剛体同士の 2D 弾性衝突（インパルス解決＋めり込み補正）
        let n = bodies.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = bodies[j].x - bodies[i].x;
                let dy = bodies[j].y - bodies[i].y;
                let dist = dx.hypot(dy);
                let min_dist = (bodies[i].radius + bodies[j].radius) * 0.85;
                if dist < min_dist && dist > 1e-4 {
                    let nx = dx / dist;
                    let ny = dy / dist;
                    let overlap = (min_dist - dist) * 0.5;
                    bodies[i].x -= nx * overlap;
                    bodies[i].y -= ny * overlap;
                    bodies[j].x += nx * overlap;
                    bodies[j].y += ny * overlap;

                    let rvx = bodies[j].vx - bodies[i].vx;
                    let rvy = bodies[j].vy - bodies[i].vy;
                    let vel_along_n = rvx * nx + rvy * ny;
                    if vel_along_n < 0.0 {
                        let inv_m_sum = 1.0 / bodies[i].mass + 1.0 / bodies[j].mass;
                        let impulse = -(1.0 + rest) * vel_along_n / inv_m_sum;
                        bodies[i].vx -= (impulse / bodies[i].mass) * nx;
                        bodies[i].vy -= (impulse / bodies[i].mass) * ny;
                        bodies[j].vx += (impulse / bodies[j].mass) * nx;
                        bodies[j].vy += (impulse / bodies[j].mass) * ny;
                        bodies[i].omega_deg -= impulse * 0.08;
                        bodies[j].omega_deg += impulse * 0.08;
                        bodies[i].bounces += 1;
                        bodies[j].bounces += 1;
                    }
                }
            }
        }
    }

    bodies
        .into_iter()
        .zip(trajectories)
        .map(|(b, samples)| PhysicsBodyTrajectory {
            item_id: b.item_id,
            item_name: b.name,
            samples,
            bounce_count: b.bounces,
        })
        .collect()
}

/// 物理演算軌跡を `x`・`y`・`rotation_deg` の 1-Undo ベジェキーフレーム焼き込みコマンド列へ変換する
pub fn build_physics_bake_commands(
    scene: &Scene,
    item_ids: &[ItemId],
    cfg: &PhysicsConfig,
) -> (Vec<PhysicsBodyTrajectory>, Vec<EditCommand>) {
    let trajectories = simulate_scene_physics(scene, item_ids, cfg);
    let mut cmds = Vec::new();
    for tr in &trajectories {
        for sm in &tr.samples {
            let t = RationalTime::from_seconds(sm.time_sec);
            cmds.push(EditCommand::SetKeyframe {
                item_id: tr.item_id,
                property: "x".into(),
                keyframe: Keyframe {
                    time: t,
                    value: sm.x,
                    interpolation: Interpolation::Bezier(CubicBezier::LINEAR),
                },
            });
            cmds.push(EditCommand::SetKeyframe {
                item_id: tr.item_id,
                property: "y".into(),
                keyframe: Keyframe {
                    time: t,
                    value: sm.y,
                    interpolation: Interpolation::Bezier(CubicBezier::LINEAR),
                },
            });
            cmds.push(EditCommand::SetKeyframe {
                item_id: tr.item_id,
                property: "rotation_deg".into(),
                keyframe: Keyframe {
                    time: t,
                    value: sm.rotation_deg,
                    interpolation: Interpolation::Bezier(CubicBezier::LINEAR),
                },
            });
        }
    }
    (trajectories, cmds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ItemKind, RgbaColor, ShapeKind, TimelineItem};
    use crate::time::FrameRate;

    #[test]
    fn bouncing_title_drops_hits_floor_bounces_and_bakes_keyframes() {
        let mut scene = Scene::new("PhysTest", 1280, 720, FrameRate::FPS_60);
        let mut a = TimelineItem::new(
            "TitleA",
            RationalTime::ZERO,
            RationalTime::from_seconds(4.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 120.0,
                height: 80.0,
                color: RgbaColor::WHITE,
            },
        );
        a.transform.x = 240.0;
        a.transform.y = 80.0;
        let id_a = a.id;

        let mut b = TimelineItem::new(
            "TitleB",
            RationalTime::ZERO,
            RationalTime::from_seconds(4.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 120.0,
                height: 120.0,
                color: RgbaColor::WHITE,
            },
        );
        b.transform.x = 340.0;
        b.transform.y = 100.0;
        let id_b = b.id;
        scene.layers[0].items.extend([a, b]);

        let cfg = PhysicsPreset::BouncingTitle.default_config();
        let (trajs, cmds) = build_physics_bake_commands(&scene, &[id_a, id_b], &cfg);
        assert_eq!(trajs.len(), 2);
        assert!(trajs[0].bounce_count >= 1, "body must bounce at least once");
        assert!(cmds.len() >= 120);
        // 初期位置 y=80 から床付近へ落下していること
        assert!(trajs[0].samples.last().unwrap().y > 400.0);
    }
}
