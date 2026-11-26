//! v10.0 #59: BVH / 動画シルエット モーションキャプチャ → 2D / 3D リグ自動リターゲット（足接地ロック付）
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで以下を実現する:
//! - 標準 `.bvh`（Biovision Hierarchy: `HIERARCHY` / `ROOT` / `JOINT` / `OFFSET` / `CHANNELS` / `End Site` / `MOTION`）パーサ＆3D 順運動学（FK）
//! - 動画フレーム／シルエット RGBA8 からの 7 関節 2D 骨格姿勢推定（`extract_silhouette_pose_from_rgba8`）
//! - 体格差を吸収するボーン長比率スケーリング＋足滑り防止 Foot Skate Locking IK リターゲット
//! - 2D パペットリグ（`ItemKind::RiggedPuppet`）および 3D メッシュ（`ItemKind::Mesh3D`）への 1-Undo キーフレームベイク

use crate::animation::{Interpolation, Keyframe};
use crate::command::EditCommand;
use crate::project::ItemId;
use crate::rigging::Rig2DConfig;
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};

/// BVH チャンネル種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BvhChannel {
    Xposition,
    Yposition,
    Zposition,
    Zrotation,
    Xrotation,
    Yrotation,
}

/// BVH スケルトン関節定義
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BvhJoint {
    pub name: String,
    pub parent_idx: Option<usize>,
    pub offset: [f32; 3],
    pub channels: Vec<BvhChannel>,
    pub channel_start: usize,
    pub is_end_site: bool,
}

/// 解析済み `.bvh` モーションキャプチャクリップ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BvhClip {
    pub joints: Vec<BvhJoint>,
    pub total_channels: usize,
    pub frame_time_sec: f32,
    pub frames: Vec<Vec<f32>>,
}

/// 組み込みサンプル BVH モーションプリセット（オフライン即テスト用）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MocapPreset {
    #[default]
    WalkCycle,
    WaveAndJump,
    HeroLanding,
}

impl MocapPreset {
    pub const ALL: &'static [Self] = &[Self::WalkCycle, Self::WaveAndJump, Self::HeroLanding];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::WalkCycle => "🚶 歩行サイクル (Walk Cycle + Foot Lock)",
            Self::WaveAndJump => "🙌 手振り＆ジャンプ (Wave & Jump)",
            Self::HeroLanding => "🦸 ヒーロー着地 (Hero Landing)",
        }
    }
}

/// 標準 `.bvh`（HIERARCHY + MOTION）テキストを解析する
pub fn parse_bvh(text: &str) -> Result<BvhClip, String> {
    let mut joints: Vec<BvhJoint> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut total_channels: usize = 0;

    let mut in_motion = false;
    let mut expected_frames: Option<usize> = None;
    let mut frame_time_sec: f32 = 1.0 / 30.0;
    let mut frames: Vec<Vec<f32>> = Vec::new();
    let mut pending_joint: Option<usize> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if !in_motion {
            if line.eq_ignore_ascii_case("HIERARCHY") {
                continue;
            }
            if line.eq_ignore_ascii_case("MOTION") {
                in_motion = true;
                continue;
            }
            if line == "{" {
                if let Some(jidx) = pending_joint.take() {
                    stack.push(jidx);
                }
                continue;
            }
            if line == "}" {
                stack.pop();
                continue;
            }

            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.is_empty() {
                continue;
            }
            match tokens[0] {
                "ROOT" | "JOINT" => {
                    let name = tokens.get(1).copied().unwrap_or("Joint").to_string();
                    let parent_idx = stack.last().copied();
                    let idx = joints.len();
                    joints.push(BvhJoint {
                        name,
                        parent_idx,
                        offset: [0.0; 3],
                        channels: Vec::new(),
                        channel_start: total_channels,
                        is_end_site: false,
                    });
                    pending_joint = Some(idx);
                }
                "End" if tokens.get(1).copied() == Some("Site") => {
                    let parent_idx = stack.last().copied();
                    let parent_name = parent_idx
                        .and_then(|p| joints.get(p))
                        .map(|j| j.name.as_str())
                        .unwrap_or("Root");
                    let idx = joints.len();
                    joints.push(BvhJoint {
                        name: format!("{parent_name}_End"),
                        parent_idx,
                        offset: [0.0; 3],
                        channels: Vec::new(),
                        channel_start: total_channels,
                        is_end_site: true,
                    });
                    pending_joint = Some(idx);
                }
                "OFFSET" if tokens.len() >= 4 => {
                    if let Some(&cur) = stack.last() {
                        let ox = tokens[1].parse::<f32>().unwrap_or(0.0);
                        let oy = tokens[2].parse::<f32>().unwrap_or(0.0);
                        let oz = tokens[3].parse::<f32>().unwrap_or(0.0);
                        joints[cur].offset = [ox, oy, oz];
                    }
                }
                "CHANNELS" if tokens.len() >= 2 => {
                    if let Some(&cur) = stack.last() {
                        let count = tokens[1].parse::<usize>().unwrap_or(0);
                        joints[cur].channel_start = total_channels;
                        for &tok in tokens.iter().skip(2).take(count) {
                            let ch = match tok {
                                "Xposition" => Some(BvhChannel::Xposition),
                                "Yposition" => Some(BvhChannel::Yposition),
                                "Zposition" => Some(BvhChannel::Zposition),
                                "Zrotation" => Some(BvhChannel::Zrotation),
                                "Xrotation" => Some(BvhChannel::Xrotation),
                                "Yrotation" => Some(BvhChannel::Yrotation),
                                _ => None,
                            };
                            if let Some(c) = ch {
                                joints[cur].channels.push(c);
                                total_channels += 1;
                            }
                        }
                    }
                }
                _ => {}
            }
        } else {
            if let Some(rest) = line.strip_prefix("Frames:") {
                expected_frames = rest.trim().parse::<usize>().ok();
                continue;
            }
            if let Some(rest) = line.strip_prefix("Frame Time:") {
                if let Ok(ft) = rest.trim().parse::<f32>()
                    && ft > 1e-5
                {
                    frame_time_sec = ft;
                }
                continue;
            }
            let vals: Vec<f32> = line
                .split_whitespace()
                .filter_map(|t| t.parse::<f32>().ok())
                .collect();
            if vals.len() >= total_channels && total_channels > 0 {
                frames.push(vals[..total_channels].to_vec());
            }
        }
    }

    if joints.is_empty() {
        return Err("BVH に有効な HIERARCHY (ROOT/JOINT) が見つかりません".into());
    }
    if frames.is_empty() {
        return Err("BVH に有効な MOTION フレームが見つかりません".into());
    }
    if let Some(exp) = expected_frames
        && frames.len() > exp
    {
        frames.truncate(exp);
    }

    Ok(BvhClip {
        joints,
        total_channels,
        frame_time_sec,
        frames,
    })
}

/// 組み込みプリセットから解析可能な標準 `.bvh` テキストを生成する（オフライン即座テスト・デモ用）
pub fn builtin_sample_bvh(preset: MocapPreset) -> String {
    let mut s = String::from(
        "HIERARCHY\n\
ROOT Hips\n\
{\n\
  OFFSET 0.0 90.0 0.0\n\
  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation\n\
  JOINT Spine\n\
  {\n\
    OFFSET 0.0 35.0 0.0\n\
    CHANNELS 3 Zrotation Xrotation Yrotation\n\
    JOINT Head\n\
    {\n\
      OFFSET 0.0 32.0 0.0\n\
      CHANNELS 3 Zrotation Xrotation Yrotation\n\
      End Site\n\
      {\n\
        OFFSET 0.0 18.0 0.0\n\
      }\n\
    }\n\
    JOINT LeftArm\n\
    {\n\
      OFFSET -18.0 24.0 0.0\n\
      CHANNELS 3 Zrotation Xrotation Yrotation\n\
      JOINT LeftHand\n\
      {\n\
        OFFSET -28.0 -22.0 0.0\n\
        CHANNELS 3 Zrotation Xrotation Yrotation\n\
        End Site\n\
        {\n\
          OFFSET -14.0 -12.0 0.0\n\
        }\n\
      }\n\
    }\n\
    JOINT RightArm\n\
    {\n\
      OFFSET 18.0 24.0 0.0\n\
      CHANNELS 3 Zrotation Xrotation Yrotation\n\
      JOINT RightHand\n\
      {\n\
        OFFSET 28.0 -22.0 0.0\n\
        CHANNELS 3 Zrotation Xrotation Yrotation\n\
        End Site\n\
        {\n\
          OFFSET 14.0 -12.0 0.0\n\
        }\n\
      }\n\
    }\n\
  }\n\
  JOINT LeftUpLeg\n\
  {\n\
    OFFSET -12.0 -5.0 0.0\n\
    CHANNELS 3 Zrotation Xrotation Yrotation\n\
    JOINT LeftFoot\n\
    {\n\
      OFFSET 0.0 -85.0 0.0\n\
      CHANNELS 3 Zrotation Xrotation Yrotation\n\
      End Site\n\
      {\n\
        OFFSET 0.0 -6.0 10.0\n\
      }\n\
    }\n\
  }\n\
  JOINT RightUpLeg\n\
  {\n\
    OFFSET 12.0 -5.0 0.0\n\
    CHANNELS 3 Zrotation Xrotation Yrotation\n\
    JOINT RightFoot\n\
    {\n\
      OFFSET 0.0 -85.0 0.0\n\
      CHANNELS 3 Zrotation Xrotation Yrotation\n\
      End Site\n\
      {\n\
        OFFSET 0.0 -6.0 10.0\n\
      }\n\
    }\n\
  }\n\
}\n\
MOTION\n\
Frames: 30\n\
Frame Time: 0.033333\n",
    );

    // 10 animated joints: Hips(6), Spine(3), Head(3), LeftArm(3), LeftHand(3), RightArm(3), RightHand(3), LeftUpLeg(3), LeftFoot(3), RightUpLeg(3), RightFoot(3) = 36 channels
    for f in 0..30 {
        let t = f as f32 / 29.0;
        let phase = t * std::f32::consts::TAU;
        let (hx, hy, hz, spine_z, head_z, la_z, lh_z, ra_z, rh_z, ll_z, lf_z, rl_z, rf_z) =
            match preset {
                MocapPreset::WalkCycle => {
                    let s = phase.sin();
                    let c = phase.cos();
                    let bob = (phase * 2.0).sin().abs() * 4.5;
                    // 左足は前半 0..0.45 で接地、右足は後半 0.5..0.95 で接地（Foot Skate Lock が明確に効く）
                    let l_leg = if t < 0.45 { 0.0 } else { s * 22.0 };
                    let r_leg = if t >= 0.5 { 0.0 } else { -s * 22.0 };
                    (
                        t * 48.0 - 24.0,
                        90.0 + bob,
                        0.0,
                        s * 6.0,
                        c * 4.0,
                        -s * 18.0,
                        -s * 10.0,
                        s * 18.0,
                        s * 10.0,
                        l_leg,
                        -l_leg * 0.4,
                        r_leg,
                        -r_leg * 0.4,
                    )
                }
                MocapPreset::WaveAndJump => {
                    let jump = if (0.35..=0.72).contains(&t) {
                        ((t - 0.35) / 0.37 * std::f32::consts::PI).sin() * 34.0
                    } else {
                        0.0
                    };
                    let wave = (phase * 3.0).sin() * 28.0;
                    (
                        0.0,
                        90.0 + jump,
                        0.0,
                        (phase).sin() * 8.0,
                        (phase * 2.0).sin() * 6.0,
                        -15.0,
                        -10.0,
                        75.0 + wave * 0.4,
                        wave,
                        jump * 0.35,
                        -jump * 0.2,
                        -jump * 0.35,
                        jump * 0.2,
                    )
                }
                MocapPreset::HeroLanding => {
                    let drop = if t < 0.35 {
                        (1.0 - t / 0.35).powi(2) * 48.0
                    } else {
                        0.0
                    };
                    let crouch = if (0.35..0.75).contains(&t) {
                        -22.0 * (1.0 - ((t - 0.55).abs() / 0.2).min(1.0))
                    } else {
                        0.0
                    };
                    (
                        0.0,
                        90.0 + drop + crouch,
                        0.0,
                        crouch * 0.7,
                        -crouch * 0.5,
                        -28.0 + crouch * 0.5,
                        -12.0,
                        34.0 - crouch * 0.6,
                        20.0,
                        crouch * 0.8,
                        -crouch * 0.5,
                        -crouch * 0.8,
                        crouch * 0.5,
                    )
                }
            };

        let line = format!(
            "{hx:.3} {hy:.3} {hz:.3} 0.000 0.000 0.000 \
             {spine_z:.3} 0.000 0.000 \
             {head_z:.3} 0.000 0.000 \
             {la_z:.3} 0.000 0.000 \
             {lh_z:.3} 0.000 0.000 \
             {ra_z:.3} 0.000 0.000 \
             {rh_z:.3} 0.000 0.000 \
             {ll_z:.3} 0.000 0.000 \
             {lf_z:.3} 0.000 0.000 \
             {rl_z:.3} 0.000 0.000 \
             {rf_z:.3} 0.000 0.000\n"
        );
        s.push_str(&line);
    }
    s
}

#[inline]
fn mat3_mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

#[inline]
fn mat3_vec(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn rot_x(deg: f32) -> [[f32; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rot_y(deg: f32) -> [[f32; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rot_z(deg: f32) -> [[f32; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// BVH 指定フレームの 3D ワールド関節座標 `(world_pos, joint_name)` を順運動学（FK）で評価する
pub fn evaluate_bvh_world_joints(clip: &BvhClip, frame_idx: usize) -> Vec<([f32; 3], String)> {
    let frame = match clip.frames.get(frame_idx) {
        Some(f) => f.as_slice(),
        None => return Vec::new(),
    };
    let n = clip.joints.len();
    let mut world_pos = vec![[0.0f32; 3]; n];
    let mut world_rot = vec![[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]; n];

    for (i, j) in clip.joints.iter().enumerate() {
        let mut local_pos = j.offset;
        let mut local_rot = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for (c_off, &ch) in j.channels.iter().enumerate() {
            let val = frame.get(j.channel_start + c_off).copied().unwrap_or(0.0);
            match ch {
                BvhChannel::Xposition => local_pos[0] = val,
                BvhChannel::Yposition => local_pos[1] = val,
                BvhChannel::Zposition => local_pos[2] = val,
                BvhChannel::Xrotation => local_rot = mat3_mul(local_rot, rot_x(val)),
                BvhChannel::Yrotation => local_rot = mat3_mul(local_rot, rot_y(val)),
                BvhChannel::Zrotation => local_rot = mat3_mul(local_rot, rot_z(val)),
            }
        }
        if let Some(pidx) = j.parent_idx {
            let rotated_off = mat3_vec(world_rot[pidx], local_pos);
            world_pos[i] = [
                world_pos[pidx][0] + rotated_off[0],
                world_pos[pidx][1] + rotated_off[1],
                world_pos[pidx][2] + rotated_off[2],
            ];
            world_rot[i] = mat3_mul(world_rot[pidx], local_rot);
        } else {
            world_pos[i] = local_pos;
            world_rot[i] = local_rot;
        }
    }

    clip.joints
        .iter()
        .enumerate()
        .map(|(i, j)| (world_pos[i], j.name.clone()))
        .collect()
}

// =========================================================================
// 動画シルエット 2D 骨格姿勢推定
// =========================================================================

/// 動画フレームシルエットから抽出された 7 関節 2D ポーズ（正規化座標 `[0, 1]`）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SilhouettePoseFrame {
    pub time_sec: f32,
    pub hip: [f32; 2],
    pub torso: [f32; 2],
    pub head: [f32; 2],
    pub left_hand: [f32; 2],
    pub right_hand: [f32; 2],
    pub left_foot: [f32; 2],
    pub right_foot: [f32; 2],
    pub confidence: f32,
}

/// RGBA8 フレームから前景シルエットを検出し、頭・胴・腰・左右手・左右足の 7 関節 2D 骨格を推定する
pub fn extract_silhouette_pose_from_rgba8(
    rgba: &[u8],
    width: u32,
    height: u32,
    luma_threshold: f32,
    time_sec: f32,
) -> SilhouettePoseFrame {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return SilhouettePoseFrame {
            time_sec,
            hip: [0.5, 0.56],
            torso: [0.5, 0.42],
            head: [0.5, 0.24],
            left_hand: [0.35, 0.48],
            right_hand: [0.65, 0.48],
            left_foot: [0.44, 0.86],
            right_foot: [0.56, 0.86],
            confidence: 0.0,
        };
    }

    let thr = luma_threshold.clamp(0.02, 0.95);
    let mut min_x = w;
    let mut max_x = 0usize;
    let mut min_y = h;
    let mut max_y = 0usize;
    let mut fg_count = 0usize;

    // 各行の前景 X 範囲と重心を収集
    let mut row_min_x = vec![w; h];
    let mut row_max_x = vec![0usize; h];
    let mut row_sum_x = vec![0.0f64; h];
    let mut row_cnt = vec![0usize; h];

    let pixels = rgba.as_chunks::<4>().0;
    for y in 0..h {
        let base = y * w;
        for x in 0..w {
            let p = pixels[base + x];
            let alpha = p[3] as f32 / 255.0;
            let luma = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
            if alpha > 0.2 && luma >= thr {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                row_min_x[y] = row_min_x[y].min(x);
                row_max_x[y] = row_max_x[y].max(x);
                row_sum_x[y] += x as f64;
                row_cnt[y] += 1;
                fg_count += 1;
            }
        }
    }

    if fg_count < 8 || max_y <= min_y || max_x <= min_x {
        return SilhouettePoseFrame {
            time_sec,
            hip: [0.5, 0.56],
            torso: [0.5, 0.42],
            head: [0.5, 0.24],
            left_hand: [0.35, 0.48],
            right_hand: [0.65, 0.48],
            left_foot: [0.44, 0.86],
            right_foot: [0.56, 0.86],
            confidence: 0.0,
        };
    }

    let fw = w as f32;
    let fh = h as f32;
    let bh = (max_y - min_y).max(1) as f32;
    let band_centroid = |f0: f32, f1: f32| -> [f32; 2] {
        let y0 = (min_y as f32 + bh * f0).round() as usize;
        let y1 = ((min_y as f32 + bh * f1).round() as usize).clamp(y0 + 1, h);
        let mut sx = 0.0f64;
        let mut sy = 0.0f64;
        let mut c = 0usize;
        for y in y0..y1 {
            if row_cnt[y] > 0 {
                sx += row_sum_x[y];
                sy += (y * row_cnt[y]) as f64;
                c += row_cnt[y];
            }
        }
        if c == 0 {
            [
                ((min_x + max_x) as f32 * 0.5) / fw,
                ((y0 + y1) as f32 * 0.5) / fh,
            ]
        } else {
            [(sx / c as f64) as f32 / fw, (sy / c as f64) as f32 / fh]
        }
    };

    let head = band_centroid(0.0, 0.18);
    let torso = band_centroid(0.22, 0.46);
    let hip = band_centroid(0.46, 0.62);

    // 腕帯（22%..60%）の左右最外郭から左右手を推定
    let arm_y0 = (min_y as f32 + bh * 0.22).round() as usize;
    let arm_y1 = ((min_y as f32 + bh * 0.62).round() as usize).clamp(arm_y0 + 1, h);
    let mut lx = w;
    let mut ly = (arm_y0 + arm_y1) / 2;
    let mut rx = 0usize;
    let mut ry = (arm_y0 + arm_y1) / 2;
    for y in arm_y0..arm_y1 {
        if row_cnt[y] > 0 {
            if row_min_x[y] < lx {
                lx = row_min_x[y];
                ly = y;
            }
            if row_max_x[y] > rx {
                rx = row_max_x[y];
                ry = y;
            }
        }
    }
    let left_hand = [lx as f32 / fw, ly as f32 / fh];
    let right_hand = [rx as f32 / fw, ry as f32 / fh];

    // 足帯（82%..100%）で腰 X より左・右の重心から左右足を推定
    let hip_px = (hip[0] * fw) as usize;
    let foot_y0 = (min_y as f32 + bh * 0.80).round() as usize;
    let mut l_sx = 0.0f64;
    let mut l_sy = 0.0f64;
    let mut l_c = 0usize;
    let mut r_sx = 0.0f64;
    let mut r_sy = 0.0f64;
    let mut r_c = 0usize;
    for y in foot_y0..=max_y.min(h - 1) {
        let base = y * w;
        for x in min_x..=max_x.min(w - 1) {
            let p = pixels[base + x];
            let alpha = p[3] as f32 / 255.0;
            let luma = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
            if alpha > 0.2 && luma >= thr {
                if x <= hip_px {
                    l_sx += x as f64;
                    l_sy += y as f64;
                    l_c += 1;
                } else {
                    r_sx += x as f64;
                    r_sy += y as f64;
                    r_c += 1;
                }
            }
        }
    }
    let left_foot = if l_c > 0 {
        [
            (l_sx / l_c as f64) as f32 / fw,
            (l_sy / l_c as f64) as f32 / fh,
        ]
    } else {
        [(hip[0] - 0.06).max(0.0), max_y as f32 / fh]
    };
    let right_foot = if r_c > 0 {
        [
            (r_sx / r_c as f64) as f32 / fw,
            (r_sy / r_c as f64) as f32 / fh,
        ]
    } else {
        [(hip[0] + 0.06).min(1.0), max_y as f32 / fh]
    };

    let coverage = fg_count as f32 / (w * h) as f32;
    let confidence = (coverage * 8.0).clamp(0.15, 0.99);

    SilhouettePoseFrame {
        time_sec,
        hip,
        torso,
        head,
        left_hand,
        right_hand,
        left_foot,
        right_foot,
        confidence,
    }
}

// =========================================================================
// リターゲット ＆ 足接地ロック（Foot Skate Locking）
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RetargetConfig {
    /// 体格差吸収ボーン長比率スケーリングを有効化
    #[serde(default = "default_true")]
    pub scale_by_bone_ratio: bool,
    /// 足滑り防止 Foot Skate Locking IK を有効化
    #[serde(default = "default_true")]
    pub foot_lock_enabled: bool,
    /// 足接地と判定する速度しきい値（px/フレーム）
    #[serde(default = "default_foot_vel")]
    pub foot_lock_velocity_thresh: f32,
    /// 足接地高さしきい値（下端からの許容マージン px）
    #[serde(default = "default_ground_margin")]
    pub ground_contact_margin_px: f32,
}

fn default_true() -> bool {
    true
}
fn default_foot_vel() -> f32 {
    8.5
}
fn default_ground_margin() -> f32 {
    14.0
}

impl Default for RetargetConfig {
    fn default() -> Self {
        Self {
            scale_by_bone_ratio: true,
            foot_lock_enabled: true,
            foot_lock_velocity_thresh: default_foot_vel(),
            ground_contact_margin_px: default_ground_margin(),
        }
    }
}

/// リターゲット済み 1 フレーム分のリグ姿勢
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RetargetedPoseFrame {
    pub time_sec: f32,
    pub hip_pos: [f32; 2],
    pub torso_angle_deg: f32,
    pub head_angle_deg: f32,
    pub left_hand_target: [f32; 2],
    pub right_hand_target: [f32; 2],
    pub left_foot_pos: [f32; 2],
    pub right_foot_pos: [f32; 2],
    pub left_foot_locked: bool,
    pub right_foot_locked: bool,
    pub root_rot_3d_deg: [f32; 3],
}

fn find_joint_pos(world: &[([f32; 3], String)], keywords: &[&str]) -> Option<[f32; 3]> {
    for (pos, name) in world {
        let lower = name.to_ascii_lowercase();
        if keywords.iter().any(|k| lower.contains(k)) {
            return Some(*pos);
        }
    }
    None
}

fn apply_foot_skate_lock(poses: &mut [RetargetedPoseFrame], cfg: &RetargetConfig) {
    if !cfg.foot_lock_enabled || poses.len() < 2 {
        return;
    }
    // 接地床レベル（2D 画面下向き +Y なので最大 Y 付近が床）
    let mut ground_y = f32::NEG_INFINITY;
    for p in poses.iter() {
        ground_y = ground_y.max(p.left_foot_pos[1]).max(p.right_foot_pos[1]);
    }
    let margin = cfg.ground_contact_margin_px.max(1.0);
    let vel_thr = cfg.foot_lock_velocity_thresh.max(0.5);

    let mut l_anchor: Option<[f32; 2]> = None;
    let mut r_anchor: Option<[f32; 2]> = None;

    for i in 0..poses.len() {
        let prev_l = if i > 0 {
            poses[i - 1].left_foot_pos
        } else {
            poses[i].left_foot_pos
        };
        let prev_r = if i > 0 {
            poses[i - 1].right_foot_pos
        } else {
            poses[i].right_foot_pos
        };

        let cur_l = poses[i].left_foot_pos;
        let cur_r = poses[i].right_foot_pos;
        let l_vel = (cur_l[0] - prev_l[0]).hypot(cur_l[1] - prev_l[1]);
        let r_vel = (cur_r[0] - prev_r[0]).hypot(cur_r[1] - prev_r[1]);

        let l_near_ground = (ground_y - cur_l[1]).abs() <= margin;
        let r_near_ground = (ground_y - cur_r[1]).abs() <= margin;

        if l_near_ground && l_vel <= vel_thr {
            let anchor = *l_anchor.get_or_insert([cur_l[0], ground_y]);
            let dx = anchor[0] - cur_l[0];
            poses[i].left_foot_pos = anchor;
            poses[i].left_foot_locked = true;
            // 腰の微補正で自然な接地感を維持
            poses[i].hip_pos[0] += dx * 0.25;
        } else {
            l_anchor = None;
            poses[i].left_foot_locked = false;
        }

        if r_near_ground && r_vel <= vel_thr {
            let anchor = *r_anchor.get_or_insert([cur_r[0], ground_y]);
            let dx = anchor[0] - cur_r[0];
            poses[i].right_foot_pos = anchor;
            poses[i].right_foot_locked = true;
            poses[i].hip_pos[0] += dx * 0.25;
        } else {
            r_anchor = None;
            poses[i].right_foot_locked = false;
        }
    }
}

/// BVH モーションキャプチャを `Rig2DConfig` の体格（ボーン長比率）へスケーリング＋足接地ロック付きで自動リターゲットする
pub fn retarget_bvh_to_rig2d(
    clip: &BvhClip,
    target_rig: &Rig2DConfig,
    cfg: &RetargetConfig,
) -> Vec<RetargetedPoseFrame> {
    if clip.frames.is_empty() {
        return Vec::new();
    }

    // ターゲットリグの体幹ボーン長（既定 105px）
    let target_torso_len = target_rig
        .bones
        .first()
        .map(|b| b.length.max(20.0))
        .unwrap_or(105.0);

    // フレーム 0 からソース BVH の体幹長を計測し、体格差吸収スケールを決定
    let w0 = evaluate_bvh_world_joints(clip, 0);
    let src_hips0 = find_joint_pos(&w0, &["hip", "pelvis", "root"]).unwrap_or([0.0, 90.0, 0.0]);
    let src_head0 = find_joint_pos(&w0, &["head"]).unwrap_or([0.0, 155.0, 0.0]);
    let src_torso_len = (src_head0[1] - src_hips0[1]).abs().max(15.0);
    let scale = if cfg.scale_by_bone_ratio {
        (target_torso_len / src_torso_len).clamp(0.25, 6.0)
    } else {
        1.0
    };

    let mut poses = Vec::with_capacity(clip.frames.len());
    for fi in 0..clip.frames.len() {
        let world = evaluate_bvh_world_joints(clip, fi);
        let hips = find_joint_pos(&world, &["hip", "pelvis", "root"]).unwrap_or(src_hips0);
        let spine = find_joint_pos(&world, &["spine", "chest"]).unwrap_or([
            hips[0],
            hips[1] + src_torso_len * 0.5,
            hips[2],
        ]);
        let head = find_joint_pos(&world, &["head"]).unwrap_or([
            hips[0],
            hips[1] + src_torso_len,
            hips[2],
        ]);
        let lhand = find_joint_pos(&world, &["lefthand", "lhand", "leftwrist"]).unwrap_or([
            hips[0] - 40.0,
            spine[1],
            hips[2],
        ]);
        let rhand = find_joint_pos(&world, &["righthand", "rhand", "rightwrist"]).unwrap_or([
            hips[0] + 40.0,
            spine[1],
            hips[2],
        ]);
        let lfoot = find_joint_pos(&world, &["leftfoot", "lfoot", "leftankle"]).unwrap_or([
            hips[0] - 12.0,
            0.0,
            hips[2],
        ]);
        let rfoot = find_joint_pos(&world, &["rightfoot", "rfoot", "rightankle"]).unwrap_or([
            hips[0] + 12.0,
            0.0,
            hips[2],
        ]);

        let to_2d = |p: [f32; 3]| -> [f32; 2] {
            [
                (p[0] - src_hips0[0]) * scale,
                -(p[1] - src_hips0[1]) * scale,
            ]
        };

        let hip_2d = to_2d(hips);
        let spine_2d = to_2d(spine);
        let head_2d = to_2d(head);
        let torso_angle_deg = (spine_2d[0] - hip_2d[0])
            .atan2(-(spine_2d[1] - hip_2d[1]).min(-1e-3))
            .to_degrees()
            .clamp(-45.0, 45.0);
        let head_angle_deg = (head_2d[0] - spine_2d[0])
            .atan2(-(head_2d[1] - spine_2d[1]).min(-1e-3))
            .to_degrees()
            .clamp(-45.0, 45.0);

        poses.push(RetargetedPoseFrame {
            time_sec: fi as f32 * clip.frame_time_sec,
            hip_pos: hip_2d,
            torso_angle_deg,
            head_angle_deg,
            left_hand_target: to_2d(lhand),
            right_hand_target: to_2d(rhand),
            left_foot_pos: to_2d(lfoot),
            right_foot_pos: to_2d(rfoot),
            left_foot_locked: false,
            right_foot_locked: false,
            root_rot_3d_deg: [
                torso_angle_deg * 0.4,
                ((rhand[2] - lhand[2]) * 0.8).clamp(-45.0, 45.0),
                torso_angle_deg,
            ],
        });
    }

    apply_foot_skate_lock(&mut poses, cfg);
    poses
}

/// 動画シルエット姿勢列を `Rig2DConfig` へリターゲット＋足接地ロックする
pub fn retarget_silhouette_sequence_to_rig2d(
    frames: &[SilhouettePoseFrame],
    target_rig: &Rig2DConfig,
    cfg: &RetargetConfig,
) -> Vec<RetargetedPoseFrame> {
    if frames.is_empty() {
        return Vec::new();
    }
    let w = target_rig.width.max(120.0);
    let h = target_rig.height.max(160.0);
    let origin = frames[0].hip;

    let mut poses: Vec<RetargetedPoseFrame> = frames
        .iter()
        .map(|f| {
            let map =
                |pt: [f32; 2]| -> [f32; 2] { [(pt[0] - origin[0]) * w, (pt[1] - origin[1]) * h] };
            let hip_2d = map(f.hip);
            let torso_2d = map(f.torso);
            let head_2d = map(f.head);
            let torso_angle_deg = (torso_2d[0] - hip_2d[0])
                .atan2(-(torso_2d[1] - hip_2d[1]).min(-1e-3))
                .to_degrees()
                .clamp(-45.0, 45.0);
            let head_angle_deg = (head_2d[0] - torso_2d[0])
                .atan2(-(head_2d[1] - torso_2d[1]).min(-1e-3))
                .to_degrees()
                .clamp(-45.0, 45.0);
            RetargetedPoseFrame {
                time_sec: f.time_sec,
                hip_pos: hip_2d,
                torso_angle_deg,
                head_angle_deg,
                left_hand_target: map(f.left_hand),
                right_hand_target: map(f.right_hand),
                left_foot_pos: map(f.left_foot),
                right_foot_pos: map(f.right_foot),
                left_foot_locked: false,
                right_foot_locked: false,
                root_rot_3d_deg: [torso_angle_deg * 0.3, 0.0, torso_angle_deg],
            }
        })
        .collect();

    apply_foot_skate_lock(&mut poses, cfg);
    poses
}

/// リターゲット結果を対象アイテム（2D パペットまたは 3D メッシュ）のキーフレーム群へ 1-Undo ベイクするコマンド列を構築する
pub fn build_mocap_retarget_bake_commands(
    item_id: ItemId,
    base_x: f32,
    base_y: f32,
    poses: &[RetargetedPoseFrame],
    target_is_3d: bool,
) -> Vec<EditCommand> {
    let mut cmds = Vec::new();
    // キーフレーム爆発を防ぐため最大 60 サンプルへ間引き
    let stride = (poses.len() / 60).max(1);
    for p in poses.iter().step_by(stride) {
        let t = RationalTime::from_seconds(p.time_sec as f64);
        cmds.push(EditCommand::SetKeyframe {
            item_id,
            property: "x".into(),
            keyframe: Keyframe {
                time: t,
                value: base_x + p.hip_pos[0],
                interpolation: Interpolation::Linear,
            },
        });
        cmds.push(EditCommand::SetKeyframe {
            item_id,
            property: "y".into(),
            keyframe: Keyframe {
                time: t,
                value: base_y + p.hip_pos[1],
                interpolation: Interpolation::Linear,
            },
        });
        cmds.push(EditCommand::SetKeyframe {
            item_id,
            property: "rotation_deg".into(),
            keyframe: Keyframe {
                time: t,
                value: if target_is_3d {
                    p.root_rot_3d_deg[2]
                } else {
                    p.torso_angle_deg
                },
                interpolation: Interpolation::Linear,
            },
        });
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bvh_parser_evaluates_fk_and_retargets_with_foot_skate_lock() {
        let bvh_text = builtin_sample_bvh(MocapPreset::WalkCycle);
        let clip = parse_bvh(&bvh_text).expect("valid builtin BVH");
        assert!(clip.joints.len() >= 11);
        assert_eq!(clip.frames.len(), 30);

        let rig = Rig2DConfig::default_humanoid();
        let cfg = RetargetConfig::default();
        let poses = retarget_bvh_to_rig2d(&clip, &rig, &cfg);
        assert_eq!(poses.len(), 30);

        let locked_count = poses
            .iter()
            .filter(|p| p.left_foot_locked || p.right_foot_locked)
            .count();
        assert!(
            locked_count >= 10,
            "expected foot skate lock active during stance frames, got {locked_count}"
        );

        // 接地ロック区間では足座標の横滑りが 0 である
        for w in poses.windows(2) {
            if w[0].left_foot_locked && w[1].left_foot_locked {
                assert!(
                    (w[1].left_foot_pos[0] - w[0].left_foot_pos[0]).abs() < 1e-4,
                    "locked left foot must not skate"
                );
            }
        }
    }

    #[test]
    fn silhouette_pose_extractor_recovers_vertical_humanoid_proportions() {
        let (w, h) = (64u32, 96u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let mut paint = |x0: u32, y0: u32, x1: u32, y1: u32| {
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i] = 240;
                    rgba[i + 1] = 240;
                    rgba[i + 2] = 240;
                    rgba[i + 3] = 255;
                }
            }
        };
        // 頭・胴・左右腕・左右脚シルエットを描画
        paint(28, 8, 36, 20);
        paint(24, 22, 40, 56);
        paint(10, 26, 54, 34);
        paint(24, 56, 30, 88);
        paint(34, 56, 40, 88);

        let pose = extract_silhouette_pose_from_rgba8(&rgba, w, h, 0.3, 0.5);
        assert!(pose.confidence > 0.3);
        assert!(pose.head[1] < pose.torso[1]);
        assert!(pose.torso[1] < pose.hip[1]);
        assert!(pose.hip[1] < pose.left_foot[1]);
        assert!(pose.left_hand[0] < pose.right_hand[0]);
        assert!(pose.left_foot[0] < pose.right_foot[0]);
    }
}
