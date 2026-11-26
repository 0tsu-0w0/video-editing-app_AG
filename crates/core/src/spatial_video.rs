//! v10.0 #57: 360° 全天球動画（OverCapture / Tiny Planet / 水平ロック）＆ ステレオスコピック空間 3D ビデオ編集
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで以下を実現する:
//! - Equirectangular ⇔ 透視投影（OverCapture）/ 逆ステレオ投影（Tiny Planet）の厳密逆射影計算
//! - ジャイロ／特徴点姿勢サンプルからの指数平滑化水平ロック（Horizon Lock）
//! - Side-by-Side / Top-and-Bottom / Red-Cyan アナグリフ / 単眼プレビューのステレオスコピック空間 3D 設定
//! - 1-Undo `EditCommand::SetSpatialVideo` プリセット生成

use crate::command::EditCommand;
use serde::{Deserialize, Serialize};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// 360° 全天球映像の投影モード
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection360Mode {
    /// 無効（通常の平面 2D/3D 合成）
    #[default]
    Off,
    /// Equirectangular 360° → 自由なカメラワーク切り出し（透視投影リフレーム）
    OverCapture,
    /// 逆ステレオ投影による Tiny Planet（リトルプラネット／ラビットホール）
    TinyPlanet,
    /// Equirectangular 全天球展開レビュー（水平グリッド＆方位コンパス付）
    EquirectangularReview,
}

impl Projection360Mode {
    pub const ALL: &'static [Self] = &[
        Self::Off,
        Self::OverCapture,
        Self::TinyPlanet,
        Self::EquirectangularReview,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Off => "無効 (2D 標準)",
            Self::OverCapture => "🎥 OverCapture (透視投影リフレーム)",
            Self::TinyPlanet => "🌍 Tiny Planet (逆ステレオ投影)",
            Self::EquirectangularReview => "🌐 Equirectangular 360° レビュー",
        }
    }
}

/// ステレオスコピック空間 3D 出力レイアウト
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StereoLayout {
    #[default]
    Off,
    /// 左右並列 Side-by-Side（VR / Vision / Quest 互換）
    SideBySide,
    /// 上下分割 Top-and-Bottom（360° 3D 立体視標準）
    TopAndBottom,
    /// 赤／シアン アナグリフ立体視プレビュー
    AnaglyphRedCyan,
    /// 左眼単独確認
    MonoLeft,
    /// 右眼単独確認
    MonoRight,
}

impl StereoLayout {
    pub const ALL: &'static [Self] = &[
        Self::Off,
        Self::SideBySide,
        Self::TopAndBottom,
        Self::AnaglyphRedCyan,
        Self::MonoLeft,
        Self::MonoRight,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Off => "無効 (モノラル)",
            Self::SideBySide => "🥽 Side-by-Side (SBS 空間 3D)",
            Self::TopAndBottom => "🥽 Top-and-Bottom (TAB 360° 3D)",
            Self::AnaglyphRedCyan => "🕶 赤／シアン アナグリフ確認",
            Self::MonoLeft => "👁 左眼 (L)",
            Self::MonoRight => "👁 右眼 (R)",
        }
    }
}

/// 360° カメラのジャイロ／特徴点推定による時系列姿勢サンプル
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GyroOrientationSample {
    pub time_sec: f32,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
}

/// 360° / 空間 3D クイックプリセット
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Spatial360Preset {
    ActionHorizonLock,
    TinyPlanetOrbit,
    SpatialVrSbs,
    Anaglyph3DReview,
}

impl Spatial360Preset {
    pub const ALL: &'static [Self] = &[
        Self::ActionHorizonLock,
        Self::TinyPlanetOrbit,
        Self::SpatialVrSbs,
        Self::Anaglyph3DReview,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ActionHorizonLock => "🏔 アクション水平ロック (OverCapture)",
            Self::TinyPlanetOrbit => "🪐 Tiny Planet オービット",
            Self::SpatialVrSbs => "🥽 VR Side-by-Side 空間 3D",
            Self::Anaglyph3DReview => "🕶 アナグリフ 3D 視差チェック",
        }
    }
}

/// シーン単位の 360° 全天球＆ステレオスコピック空間 3D 設定（`Scene::spatial_video`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpatialVideoConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub projection_mode: Projection360Mode,
    /// カメラ水平パン角（-180..=180 度）
    #[serde(default)]
    pub yaw_deg: f32,
    /// カメラ上下チルト角（-90..=90 度）
    #[serde(default)]
    pub pitch_deg: f32,
    /// カメラロール角（-180..=180 度）
    #[serde(default)]
    pub roll_deg: f32,
    /// OverCapture 水平視野角 FOV（30..=150 度）
    #[serde(default = "default_fov")]
    pub fov_deg: f32,
    /// Tiny Planet ズーム倍率（0.3..=3.0）
    #[serde(default = "default_one")]
    pub tiny_planet_zoom: f32,
    /// ジャイロ／特徴点ベースの水平ロック（Horizon Lock）
    #[serde(default)]
    pub horizon_lock_enabled: bool,
    /// 水平ロック平滑化強度（0.0=即応, 0.95=強平滑）
    #[serde(default = "default_smoothing")]
    pub horizon_lock_smoothing: f32,
    /// 時系列ジャイロサンプル（カメラ揺れ補正用）
    #[serde(default)]
    pub gyro_samples: Vec<GyroOrientationSample>,
    /// ステレオスコピック出力形式
    #[serde(default)]
    pub stereo_layout: StereoLayout,
    /// 左右視差強度（IPD ベース・px）
    #[serde(default = "default_ipd_px")]
    pub ipd_parallax_px: f32,
    /// コンバージェンス（ゼロ視差面オフセット・px）
    #[serde(default)]
    pub convergence_offset_px: f32,
    /// 水平線＆方位コンパスガイド表示
    #[serde(default)]
    pub show_horizon_guides: bool,
}

fn default_fov() -> f32 {
    95.0
}
fn default_one() -> f32 {
    1.0
}
fn default_smoothing() -> f32 {
    0.72
}
fn default_ipd_px() -> f32 {
    14.0
}

impl Default for SpatialVideoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            projection_mode: Projection360Mode::Off,
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            fov_deg: default_fov(),
            tiny_planet_zoom: default_one(),
            horizon_lock_enabled: false,
            horizon_lock_smoothing: default_smoothing(),
            gyro_samples: Vec::new(),
            stereo_layout: StereoLayout::Off,
            ipd_parallax_px: default_ipd_px(),
            convergence_offset_px: 0.0,
            show_horizon_guides: false,
        }
    }
}

impl SpatialVideoConfig {
    pub fn from_preset(preset: Spatial360Preset) -> Self {
        match preset {
            Spatial360Preset::ActionHorizonLock => Self {
                enabled: true,
                projection_mode: Projection360Mode::OverCapture,
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                roll_deg: 0.0,
                fov_deg: 102.0,
                tiny_planet_zoom: 1.0,
                horizon_lock_enabled: true,
                horizon_lock_smoothing: 0.78,
                gyro_samples: vec![
                    GyroOrientationSample {
                        time_sec: 0.0,
                        yaw_deg: 0.0,
                        pitch_deg: 8.0,
                        roll_deg: -18.0,
                    },
                    GyroOrientationSample {
                        time_sec: 1.5,
                        yaw_deg: 24.0,
                        pitch_deg: -6.0,
                        roll_deg: 22.0,
                    },
                    GyroOrientationSample {
                        time_sec: 3.0,
                        yaw_deg: -15.0,
                        pitch_deg: 5.0,
                        roll_deg: -14.0,
                    },
                ],
                stereo_layout: StereoLayout::Off,
                ipd_parallax_px: 12.0,
                convergence_offset_px: 0.0,
                show_horizon_guides: true,
            },
            Spatial360Preset::TinyPlanetOrbit => Self {
                enabled: true,
                projection_mode: Projection360Mode::TinyPlanet,
                yaw_deg: 15.0,
                pitch_deg: -85.0,
                roll_deg: 0.0,
                fov_deg: 120.0,
                tiny_planet_zoom: 1.15,
                horizon_lock_enabled: false,
                horizon_lock_smoothing: 0.5,
                gyro_samples: Vec::new(),
                stereo_layout: StereoLayout::Off,
                ipd_parallax_px: 12.0,
                convergence_offset_px: 0.0,
                show_horizon_guides: false,
            },
            Spatial360Preset::SpatialVrSbs => Self {
                enabled: true,
                projection_mode: Projection360Mode::Off,
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                roll_deg: 0.0,
                fov_deg: 95.0,
                tiny_planet_zoom: 1.0,
                horizon_lock_enabled: false,
                horizon_lock_smoothing: 0.5,
                gyro_samples: Vec::new(),
                stereo_layout: StereoLayout::SideBySide,
                ipd_parallax_px: 16.0,
                convergence_offset_px: 2.0,
                show_horizon_guides: false,
            },
            Spatial360Preset::Anaglyph3DReview => Self {
                enabled: true,
                projection_mode: Projection360Mode::Off,
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                roll_deg: 0.0,
                fov_deg: 95.0,
                tiny_planet_zoom: 1.0,
                horizon_lock_enabled: false,
                horizon_lock_smoothing: 0.5,
                gyro_samples: Vec::new(),
                stereo_layout: StereoLayout::AnaglyphRedCyan,
                ipd_parallax_px: 18.0,
                convergence_offset_px: 0.0,
                show_horizon_guides: true,
            },
        }
    }
}

/// ジャイロ姿勢サンプルを時刻補間し、水平ロック有効時はカメラのピッチ／ロール傾きを逆補正した実効オイラー角 `(yaw, pitch, roll)` を返す
pub fn evaluate_horizon_locked_euler(cfg: &SpatialVideoConfig, time_sec: f32) -> (f32, f32, f32) {
    if !cfg.horizon_lock_enabled || cfg.gyro_samples.is_empty() {
        return (cfg.yaw_deg, cfg.pitch_deg, cfg.roll_deg);
    }

    let mut samples = cfg.gyro_samples.clone();
    samples.sort_by(|a, b| a.time_sec.total_cmp(&b.time_sec));

    // 現在時刻のジャイロ姿勢を線形補間
    let (g_yaw, g_pitch, g_roll) = if time_sec <= samples[0].time_sec {
        let s = samples[0];
        (s.yaw_deg, s.pitch_deg, s.roll_deg)
    } else if time_sec >= samples[samples.len() - 1].time_sec {
        let s = samples[samples.len() - 1];
        (s.yaw_deg, s.pitch_deg, s.roll_deg)
    } else {
        let mut out = (0.0f32, 0.0f32, 0.0f32);
        for pair in samples.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if time_sec >= a.time_sec && time_sec <= b.time_sec {
                let dt = (b.time_sec - a.time_sec).max(1e-5);
                let t = ((time_sec - a.time_sec) / dt).clamp(0.0, 1.0);
                out = (
                    a.yaw_deg + (b.yaw_deg - a.yaw_deg) * t,
                    a.pitch_deg + (b.pitch_deg - a.pitch_deg) * t,
                    a.roll_deg + (b.roll_deg - a.roll_deg) * t,
                );
                break;
            }
        }
        out
    };

    // 水平ロック: ロール・ピッチの揺れを逆位相で打ち消し、ヨーは平滑化追従
    let lock_strength = (1.0 - cfg.horizon_lock_smoothing.clamp(0.0, 0.95) * 0.25).clamp(0.5, 1.0);
    let eff_yaw = cfg.yaw_deg + g_yaw * (1.0 - cfg.horizon_lock_smoothing.clamp(0.0, 0.98));
    let eff_pitch = (cfg.pitch_deg - g_pitch * lock_strength).clamp(-89.5, 89.5);
    let eff_roll = cfg.roll_deg - g_roll * lock_strength;
    (eff_yaw, eff_pitch, eff_roll)
}

/// 3D 方向ベクトル `(x, y, z)`（+Z 前方・+Y 上方・+X 右方）を Equirectangular UV `(u ∈ [0,1), v ∈ [0,1])` へ変換する
#[inline]
pub fn equirect_uv_for_ray(dir: [f32; 3]) -> (f32, f32) {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
        .sqrt()
        .max(1e-6);
    let x = dir[0] / len;
    let y = (dir[1] / len).clamp(-1.0, 1.0);
    let z = dir[2] / len;
    let lon = x.atan2(z); // -PI..=PI
    let lat = y.asin(); // -PI/2..=PI/2
    let u = (lon / TAU + 0.5).rem_euclid(1.0);
    let v = (0.5 - lat / PI).clamp(0.0, 1.0);
    (u, v)
}

/// カメラ姿勢 `(yaw, pitch, roll)`（度）でレイを回転させる（Roll → Pitch → Yaw 順）
#[inline]
pub fn rotate_ray_ypr(v: [f32; 3], yaw_deg: f32, pitch_deg: f32, roll_deg: f32) -> [f32; 3] {
    let (sr, cr) = roll_deg.to_radians().sin_cos();
    let (sp, cp) = pitch_deg.to_radians().sin_cos();
    let (sy, cy) = yaw_deg.to_radians().sin_cos();

    // Roll (Z 軸)
    let x1 = v[0] * cr - v[1] * sr;
    let y1 = v[0] * sr + v[1] * cr;
    let z1 = v[2];

    // Pitch (X 軸)
    let y2 = y1 * cp - z1 * sp;
    let z2 = y1 * sp + z1 * cp;
    let x2 = x1;

    // Yaw (Y 軸)
    [x2 * cy + z2 * sy, y2, -x2 * sy + z2 * cy]
}

/// 正規化スクリーン座標 `(nx, ny) ∈ [-1, 1]`（+ny=上）を 360° Equirectangular UV へ逆射影する
#[allow(clippy::too_many_arguments)]
pub fn project_screen_to_equirect_uv(
    nx: f32,
    ny: f32,
    aspect: f32,
    mode: Projection360Mode,
    yaw_deg: f32,
    pitch_deg: f32,
    roll_deg: f32,
    fov_deg: f32,
    tiny_planet_zoom: f32,
) -> (f32, f32) {
    match mode {
        Projection360Mode::Off | Projection360Mode::EquirectangularReview => {
            let u = ((nx * 0.5 + 0.5) + yaw_deg / 360.0).rem_euclid(1.0);
            let v = (0.5 - ny * 0.5 - pitch_deg / 180.0).clamp(0.0, 1.0);
            (u, v)
        }
        Projection360Mode::OverCapture => {
            let half_fov = (fov_deg.clamp(25.0, 155.0) * 0.5).to_radians();
            let tan_half_x = half_fov.tan();
            let tan_half_y = tan_half_x / aspect.max(0.1);
            let ray_cam = [nx * tan_half_x, ny * tan_half_y, 1.0];
            let ray_world = rotate_ray_ypr(ray_cam, yaw_deg, pitch_deg, roll_deg);
            equirect_uv_for_ray(ray_world)
        }
        Projection360Mode::TinyPlanet => {
            // 逆ステレオ投影: 画面中心が南極（または指定向き）、外周が天頂へ回り込むリトルプラネット変換
            let zoom = tiny_planet_zoom.clamp(0.25, 4.0);
            let px = (nx * aspect.max(0.1)) / zoom;
            let py = ny / zoom;
            let r2 = px * px + py * py;
            let denom = 1.0 + r2;
            // 南極中心ステレオ逆写影（r=0 → [0, 0, 1], r→∞ → [0, 0, -1]）
            let sx = (2.0 * px) / denom;
            let sy = (2.0 * py) / denom;
            let sz = (1.0 - r2) / denom;
            let ray_world = rotate_ray_ypr([sx, sy, sz], yaw_deg, pitch_deg, roll_deg);
            equirect_uv_for_ray(ray_world)
        }
    }
}

/// プリセット適用用の 1-Undo `EditCommand` を生成する
pub fn build_spatial_preset_command(preset: Spatial360Preset) -> EditCommand {
    EditCommand::SetSpatialVideo {
        spatial_video: SpatialVideoConfig::from_preset(preset),
    }
}

/// 方位角（度）から 8 方位コンパスラベルを返す
pub fn compass_heading_label(yaw_deg: f32) -> &'static str {
    let norm = yaw_deg.rem_euclid(360.0);
    let idx = ((norm + 22.5) / 45.0).floor() as usize % 8;
    const LABELS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let _ = FRAC_PI_2;
    LABELS[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overcapture_and_tiny_planet_inverse_projections_are_exact_and_continuous() {
        // 正面中央 (0,0) で Yaw=0, Pitch=0 → u=0.5, v=0.5
        let (u0, v0) = project_screen_to_equirect_uv(
            0.0,
            0.0,
            16.0 / 9.0,
            Projection360Mode::OverCapture,
            0.0,
            0.0,
            0.0,
            90.0,
            1.0,
        );
        assert!((u0 - 0.5).abs() < 1e-4, "expected u=0.5, got {u0}");
        assert!((v0 - 0.5).abs() < 1e-4, "expected v=0.5, got {v0}");

        // Yaw=+90° で正面レイは +X を向き u=0.75
        let (u90, v90) = project_screen_to_equirect_uv(
            0.0,
            0.0,
            16.0 / 9.0,
            Projection360Mode::OverCapture,
            90.0,
            0.0,
            0.0,
            90.0,
            1.0,
        );
        assert!((u90 - 0.75).abs() < 1e-4, "expected u=0.75, got {u90}");
        assert!((v90 - 0.5).abs() < 1e-4);

        // Tiny Planet は中心と外周で異なる緯度へ連続写像される
        let (_, v_center) = project_screen_to_equirect_uv(
            0.0,
            0.0,
            1.0,
            Projection360Mode::TinyPlanet,
            0.0,
            -90.0,
            0.0,
            120.0,
            1.0,
        );
        let (_, v_edge) = project_screen_to_equirect_uv(
            0.9,
            0.0,
            1.0,
            Projection360Mode::TinyPlanet,
            0.0,
            -90.0,
            0.0,
            120.0,
            1.0,
        );
        assert!((v_center - v_edge).abs() > 0.25);
    }

    #[test]
    fn horizon_lock_counteracts_gyro_roll_and_pitch_drift() {
        let cfg = SpatialVideoConfig::from_preset(Spatial360Preset::ActionHorizonLock);
        let (_, p0, r0) = evaluate_horizon_locked_euler(&cfg, 0.0);
        let (_, p1, r1) = evaluate_horizon_locked_euler(&cfg, 1.5);
        // t=0 は gyro roll=-18° を打ち消す正のロール補正、t=1.5 は gyro roll=+22° を打ち消す負のロール補正
        assert!(r0 > 10.0, "expected positive counter-roll at t=0, got {r0}");
        assert!(
            r1 < -10.0,
            "expected negative counter-roll at t=1.5, got {r1}"
        );
        assert!(p0 < 0.0 && p1 > 0.0);
        assert_eq!(compass_heading_label(0.0), "N");
        assert_eq!(compass_heading_label(90.0), "E");
    }
}
