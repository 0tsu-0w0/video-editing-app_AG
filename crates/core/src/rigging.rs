//! v5.0 #37: 2D スケルトンボーン＆パペットリギングエンジン（階層 FK・解析的 2-Bone IK・二次バネ揺れ）
//! Pure Rust・ゼロ `unsafe`・外部依存ゼロで、立ち絵／キャラクターリグの関節階層と
//! 2 ボーン逆運動学（Law of Cosines）、髪・リボンの二次追従スウェイを決定論評価する。

use crate::project::RgbaColor;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigMotionPreset {
    #[default]
    IdleBreathe,
    WaveHello,
    NodTalk,
    DanceBounce,
}

impl RigMotionPreset {
    pub const ALL: &'static [Self] = &[
        Self::IdleBreathe,
        Self::WaveHello,
        Self::NodTalk,
        Self::DanceBounce,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::IdleBreathe => "😌 アイドル呼吸 (Idle Breathe)",
            Self::WaveHello => "👋 手振り挨拶 (Wave Hello + IK)",
            Self::NodTalk => "🗣 うなずき解説 (Nod & Talk)",
            Self::DanceBounce => "🕺 リズムバウンス (Dance Bounce)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneNode {
    pub id: String,
    pub parent_idx: Option<usize>,
    /// 親ボーン先端（またはルート時キャンバス中央）からのローカルオフセット
    pub local_offset: [f32; 2],
    /// ボーン長（px）
    pub length: f32,
    /// 基準角度（度、0°=+X 右、-90°=上、+90°=下）
    pub rest_angle_deg: f32,
    /// ユーザー手動オフセット角度（度）
    #[serde(default)]
    pub angle_offset_deg: f32,
    /// 描画時の太さ（px）
    pub thickness: f32,
    /// 二次物理揺れ係数（0.0=剛体, 0.3..1.0=髪・リボン等の遅れ追従揺れ）
    #[serde(default)]
    pub spring_sway: f32,
    pub color: RgbaColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IkTarget {
    #[serde(default)]
    pub enabled: bool,
    pub upper_bone_idx: usize,
    pub lower_bone_idx: usize,
    /// キャンバス正規化座標（中心=0.0,0.0、幅高さ基準のピクセル相対値）
    pub target_pos: [f32; 2],
    /// 肘・膝の曲げ方向（true=+角方向, false=-角方向）
    #[serde(default = "default_true")]
    pub bend_positive: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rig2DConfig {
    #[serde(default)]
    pub preset: RigMotionPreset,
    #[serde(default = "default_rig_w")]
    pub width: f32,
    #[serde(default = "default_rig_h")]
    pub height: f32,
    #[serde(default = "default_one")]
    pub motion_speed: f32,
    #[serde(default = "default_one")]
    pub motion_amp: f32,
    #[serde(default = "default_true")]
    pub show_bones_overlay: bool,
    pub bones: Vec<BoneNode>,
    pub right_arm_ik: IkTarget,
    pub left_arm_ik: IkTarget,
    #[serde(default = "default_skin")]
    pub skin_color: RgbaColor,
    #[serde(default = "default_outfit")]
    pub outfit_color: RgbaColor,
    #[serde(default = "default_accent")]
    pub accent_color: RgbaColor,
}

fn default_rig_w() -> f32 {
    360.0
}
fn default_rig_h() -> f32 {
    480.0
}
fn default_one() -> f32 {
    1.0
}
fn default_skin() -> RgbaColor {
    RgbaColor {
        r: 0.99,
        g: 0.85,
        b: 0.76,
        a: 1.0,
    }
}
fn default_outfit() -> RgbaColor {
    RgbaColor {
        r: 0.22,
        g: 0.48,
        b: 0.88,
        a: 1.0,
    }
}
fn default_accent() -> RgbaColor {
    RgbaColor {
        r: 0.98,
        g: 0.36,
        b: 0.56,
        a: 1.0,
    }
}

impl Default for Rig2DConfig {
    fn default() -> Self {
        Self::default_humanoid()
    }
}

impl Rig2DConfig {
    /// 標準 10 ボーン・ヒューマノイドパペットリグを構築する（体幹・頭・二次揺れリボン・両腕 2-Bone IK・両脚）
    pub fn default_humanoid() -> Self {
        let skin = default_skin();
        let outfit = default_outfit();
        let accent = default_accent();
        let dark = RgbaColor {
            r: 0.16,
            g: 0.20,
            b: 0.30,
            a: 1.0,
        };
        let bones = vec![
            // 0: pelvis -> torso
            BoneNode {
                id: "torso".into(),
                parent_idx: None,
                local_offset: [0.0, 62.0],
                length: 105.0,
                rest_angle_deg: -90.0,
                angle_offset_deg: 0.0,
                thickness: 52.0,
                spring_sway: 0.0,
                color: outfit,
            },
            // 1: head
            BoneNode {
                id: "head".into(),
                parent_idx: Some(0),
                local_offset: [0.0, -8.0],
                length: 68.0,
                rest_angle_deg: -90.0,
                angle_offset_deg: 0.0,
                thickness: 76.0,
                spring_sway: 0.0,
                color: skin,
            },
            // 2: hair_ribbon (secondary spring sway)
            BoneNode {
                id: "hair_ribbon".into(),
                parent_idx: Some(1),
                local_offset: [16.0, 12.0],
                length: 56.0,
                rest_angle_deg: 72.0,
                angle_offset_deg: 0.0,
                thickness: 20.0,
                spring_sway: 0.85,
                color: accent,
            },
            // 3: left_upper_arm
            BoneNode {
                id: "l_upper_arm".into(),
                parent_idx: Some(0),
                local_offset: [-26.0, 14.0],
                length: 62.0,
                rest_angle_deg: 118.0,
                angle_offset_deg: 0.0,
                thickness: 22.0,
                spring_sway: 0.0,
                color: outfit,
            },
            // 4: left_forearm
            BoneNode {
                id: "l_forearm".into(),
                parent_idx: Some(3),
                local_offset: [0.0, 0.0],
                length: 58.0,
                rest_angle_deg: 102.0,
                angle_offset_deg: 0.0,
                thickness: 18.0,
                spring_sway: 0.0,
                color: skin,
            },
            // 5: right_upper_arm
            BoneNode {
                id: "r_upper_arm".into(),
                parent_idx: Some(0),
                local_offset: [26.0, 14.0],
                length: 62.0,
                rest_angle_deg: 62.0,
                angle_offset_deg: 0.0,
                thickness: 22.0,
                spring_sway: 0.0,
                color: outfit,
            },
            // 6: right_forearm
            BoneNode {
                id: "r_forearm".into(),
                parent_idx: Some(5),
                local_offset: [0.0, 0.0],
                length: 58.0,
                rest_angle_deg: 46.0,
                angle_offset_deg: 0.0,
                thickness: 18.0,
                spring_sway: 0.0,
                color: skin,
            },
            // 7: left_leg
            BoneNode {
                id: "l_leg".into(),
                parent_idx: None,
                local_offset: [-18.0, 66.0],
                length: 104.0,
                rest_angle_deg: 95.0,
                angle_offset_deg: 0.0,
                thickness: 24.0,
                spring_sway: 0.0,
                color: dark,
            },
            // 8: right_leg
            BoneNode {
                id: "r_leg".into(),
                parent_idx: None,
                local_offset: [18.0, 66.0],
                length: 104.0,
                rest_angle_deg: 85.0,
                angle_offset_deg: 0.0,
                thickness: 24.0,
                spring_sway: 0.0,
                color: dark,
            },
            // 9: scarf_tail (secondary spring sway)
            BoneNode {
                id: "scarf_tail".into(),
                parent_idx: Some(0),
                local_offset: [-14.0, 8.0],
                length: 64.0,
                rest_angle_deg: 112.0,
                angle_offset_deg: 0.0,
                thickness: 16.0,
                spring_sway: 0.95,
                color: accent,
            },
        ];

        Self {
            preset: RigMotionPreset::WaveHello,
            width: default_rig_w(),
            height: default_rig_h(),
            motion_speed: 1.0,
            motion_amp: 1.0,
            show_bones_overlay: true,
            bones,
            right_arm_ik: IkTarget {
                enabled: true,
                upper_bone_idx: 5,
                lower_bone_idx: 6,
                target_pos: [92.0, -96.0],
                bend_positive: false,
            },
            left_arm_ik: IkTarget {
                enabled: false,
                upper_bone_idx: 3,
                lower_bone_idx: 4,
                target_pos: [-86.0, 12.0],
                bend_positive: true,
            },
            skin_color: skin,
            outfit_color: outfit,
            accent_color: accent,
        }
    }
}

/// 評価済みのワールド空間ボーン姿勢（中心原点 px 座標）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolvedBonePose {
    pub id: String,
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub world_angle_rad: f32,
    pub thickness: f32,
    pub color: RgbaColor,
}

/// 解析的 2-Bone IK ソルバー（余弦定理 / Law of Cosines）。
/// 始点 `origin`、ボーン長 `l1`, `l2`、目標座標 `target` から `(upper_angle_rad, lower_angle_rad)` を厳密算出する。
pub fn solve_two_bone_ik(
    origin: [f32; 2],
    l1: f32,
    l2: f32,
    target: [f32; 2],
    bend_positive: bool,
) -> (f32, f32) {
    let a = l1.max(1e-3);
    let b = l2.max(1e-3);
    let dx = target[0] - origin[0];
    let dy = target[1] - origin[1];
    let base_angle = dy.atan2(dx);
    let raw_dist = dx.hypot(dy);
    let min_d = (a - b).abs() + 1e-3;
    let max_d = (a + b) - 1e-3;
    let d = raw_dist.clamp(min_d, max_d);

    let cos_shoulder = ((a * a + d * d - b * b) / (2.0 * a * d)).clamp(-1.0, 1.0);
    let cos_elbow = ((a * a + b * b - d * d) / (2.0 * a * b)).clamp(-1.0, 1.0);
    let alpha = cos_shoulder.acos();
    let beta = std::f32::consts::PI - cos_elbow.acos();

    let sign = if bend_positive { 1.0 } else { -1.0 };
    let world_a1 = base_angle - sign * alpha;
    let world_a2 = world_a1 + sign * beta;
    (world_a1, world_a2)
}

/// 指定時刻 `local_time_sec` における全ボーンの FK + プロシージャル揺れ + 二次バネ + 2-Bone IK 姿勢を算出する
pub fn evaluate_rig_poses(cfg: &Rig2DConfig, local_time_sec: f32) -> Vec<SolvedBonePose> {
    let n = cfg.bones.len();
    if n == 0 {
        return Vec::new();
    }
    let t = local_time_sec * cfg.motion_speed.clamp(0.0, 6.0);
    let amp = cfg.motion_amp.clamp(0.0, 3.0);

    // モーションプリセットによる体幹上下・角度揺れ
    let (body_bob_y, torso_sway_deg, head_nod_deg, arm_wave_phase) = match cfg.preset {
        RigMotionPreset::IdleBreathe => (
            (t * TAU * 0.45).sin() * 4.5 * amp,
            (t * TAU * 0.25).sin() * 2.5 * amp,
            (t * TAU * 0.45 - 0.5).sin() * 4.0 * amp,
            0.0,
        ),
        RigMotionPreset::WaveHello => (
            (t * TAU * 0.9).sin() * 6.0 * amp,
            (t * TAU * 0.45).sin() * 5.0 * amp,
            (t * TAU * 0.9 - 0.4).sin() * 7.5 * amp,
            (t * TAU * 1.4).sin(),
        ),
        RigMotionPreset::NodTalk => (
            (t * TAU * 1.1).sin().abs() * -5.0 * amp,
            (t * TAU * 0.55).sin() * 3.5 * amp,
            (t * TAU * 1.6).sin() * 14.0 * amp,
            (t * TAU * 0.8).sin() * 0.4,
        ),
        RigMotionPreset::DanceBounce => (
            (t * TAU * 2.0).sin().abs() * -16.0 * amp,
            (t * TAU * 1.0).sin() * 11.0 * amp,
            (t * TAU * 2.0 - 0.6).sin() * 10.0 * amp,
            (t * TAU * 2.0).sin(),
        ),
    };

    // 1. Forward Kinematics パス
    let mut poses: Vec<SolvedBonePose> = Vec::with_capacity(n);
    for (idx, bone) in cfg.bones.iter().enumerate() {
        let (parent_end, parent_delta_rad) = if let Some(p_idx) = bone.parent_idx
            && p_idx < poses.len()
        {
            let p_bone = &cfg.bones[p_idx];
            let p_pose = &poses[p_idx];
            let delta = p_pose.world_angle_rad - p_bone.rest_angle_deg.to_radians();
            (p_pose.end, delta)
        } else {
            ([0.0, body_bob_y], 0.0)
        };

        let cos_p = parent_delta_rad.cos();
        let sin_p = parent_delta_rad.sin();
        let ox = bone.local_offset[0] * cos_p - bone.local_offset[1] * sin_p;
        let oy = bone.local_offset[0] * sin_p + bone.local_offset[1] * cos_p;
        let start = [parent_end[0] + ox, parent_end[1] + oy];

        let mut proc_deg = 0.0f32;
        if idx == 0 {
            proc_deg += torso_sway_deg;
        } else if bone.id == "head" {
            proc_deg += head_nod_deg;
        } else if bone.id == "l_leg" && cfg.preset == RigMotionPreset::DanceBounce {
            proc_deg += (t * TAU).sin() * 14.0 * amp;
        } else if bone.id == "r_leg" && cfg.preset == RigMotionPreset::DanceBounce {
            proc_deg -= (t * TAU).sin() * 14.0 * amp;
        }

        // 二次物理バネ遅れ揺れ（位相遅れ＋高調波）
        if bone.spring_sway > 0.01 {
            let phase_lag = 0.75 + (idx as f32) * 0.25;
            let sway = ((t * TAU * 1.15 - phase_lag).sin() * 14.0
                + (t * TAU * 2.3 - phase_lag).cos() * 4.5)
                * bone.spring_sway.clamp(0.0, 2.0)
                * amp;
            proc_deg += sway;
        }

        let world_angle_rad = (bone.rest_angle_deg + bone.angle_offset_deg + proc_deg).to_radians()
            + parent_delta_rad;
        let end = [
            start[0] + world_angle_rad.cos() * bone.length,
            start[1] + world_angle_rad.sin() * bone.length,
        ];
        poses.push(SolvedBonePose {
            id: bone.id.clone(),
            start,
            end,
            world_angle_rad,
            thickness: bone.thickness,
            color: bone.color,
        });
    }

    // 2. 2-Bone Analytical IK オーバーライド（右腕 / 左腕）
    let mut apply_arm_ik = |ik: IkTarget, wave_offset: [f32; 2]| {
        if !ik.enabled || ik.upper_bone_idx >= poses.len() || ik.lower_bone_idx >= poses.len() {
            return;
        }
        let origin = poses[ik.upper_bone_idx].start;
        let l1 = cfg.bones[ik.upper_bone_idx].length;
        let l2 = cfg.bones[ik.lower_bone_idx].length;
        let target = [
            ik.target_pos[0] + wave_offset[0],
            ik.target_pos[1] + wave_offset[1],
        ];
        let (a1, a2) = solve_two_bone_ik(origin, l1, l2, target, ik.bend_positive);
        let elbow = [origin[0] + a1.cos() * l1, origin[1] + a1.sin() * l1];
        let wrist = [elbow[0] + a2.cos() * l2, elbow[1] + a2.sin() * l2];

        poses[ik.upper_bone_idx].world_angle_rad = a1;
        poses[ik.upper_bone_idx].end = elbow;
        poses[ik.lower_bone_idx].start = elbow;
        poses[ik.lower_bone_idx].world_angle_rad = a2;
        poses[ik.lower_bone_idx].end = wrist;
    };

    let r_wave = [
        arm_wave_phase * 28.0 * amp,
        (arm_wave_phase.abs() * -14.0) * amp + body_bob_y * 0.5,
    ];
    let l_wave = [-arm_wave_phase * 22.0 * amp, body_bob_y * 0.5];
    apply_arm_ik(cfg.right_arm_ik, r_wave);
    apply_arm_ik(cfg.left_arm_ik, l_wave);

    poses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytical_two_bone_ik_reaches_reachable_target_precisely() {
        let origin = [10.0, -20.0];
        let l1 = 62.0;
        let l2 = 58.0;
        let target = [85.0, -75.0];
        let (a1, a2) = solve_two_bone_ik(origin, l1, l2, target, false);
        let elbow = [origin[0] + a1.cos() * l1, origin[1] + a1.sin() * l1];
        let end = [elbow[0] + a2.cos() * l2, elbow[1] + a2.sin() * l2];
        let err = (end[0] - target[0]).hypot(end[1] - target[1]);
        assert!(err < 1e-2, "2-Bone IK end effector error too large: {err}");
    }

    #[test]
    fn rig_poses_animate_deterministically_with_secondary_sway_and_ik() {
        let cfg = Rig2DConfig::default_humanoid();
        let p0a = evaluate_rig_poses(&cfg, 0.0);
        let p0b = evaluate_rig_poses(&cfg, 0.0);
        assert_eq!(p0a, p0b);
        assert_eq!(p0a.len(), 10);

        let p1 = evaluate_rig_poses(&cfg, 0.18);
        let wrist_move = (p0a[6].end[0] - p1[6].end[0]).hypot(p0a[6].end[1] - p1[6].end[1]);
        let ribbon_move = (p0a[2].end[0] - p1[2].end[0]).hypot(p0a[2].end[1] - p1[2].end[1]);
        assert!(wrist_move > 15.0, "IK waving arm should move: {wrist_move}");
        assert!(
            ribbon_move > 2.0,
            "spring hair ribbon should sway: {ribbon_move}"
        );
    }
}
