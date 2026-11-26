//! v10.0 #58: 決定論 2D Navier-Stokes 流体（煙・炎・インク）＆ PBD クロス物理 ＋ 3D シャドウマップ / HDRI IBL
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで以下を決定論的に実行する:
//! - Jos Stam 安定法（Stable Fluids）固定タイムステップ 2D オイラー格子流体ソルバー
//!   （移流・暗黙拡散・渦度閉じ込め・圧力ポアソン投影）
//! - Position-Based Dynamics（PBD）構造・せん断・曲げ距離拘束付き 2D/3D クロスシミュレーション
//! - 3D シャドウマップ（PCF ソフトシャドウ）＆ Equirectangular プロシージャル HDRI 拡散/鏡面 IBL 設定＆サンプラー

use crate::project::RgbaColor;
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

/// 2D Navier-Stokes 流体プリセット
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FluidPreset {
    #[default]
    CinemaSmoke,
    BonfireFlame,
    SumiInk,
    NeonPlasma,
}

impl FluidPreset {
    pub const ALL: &'static [Self] = &[
        Self::CinemaSmoke,
        Self::BonfireFlame,
        Self::SumiInk,
        Self::NeonPlasma,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::CinemaSmoke => "💨 シネマスモーク (Smoke Plume)",
            Self::BonfireFlame => "🔥 炎・ファイアプルーム (Flame)",
            Self::SumiInk => "🖋 墨・インク拡散 (Sumi Ink)",
            Self::NeonPlasma => "⚡ ネオンプラズマ渦 (Neon Plasma)",
        }
    }
}

/// 決定論 2D Navier-Stokes 流体アイテム設定（`ItemKind::FluidSim`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FluidSimConfig {
    #[serde(default)]
    pub preset: FluidPreset,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_fluid_grid")]
    pub grid_w: u32,
    #[serde(default = "default_fluid_grid")]
    pub grid_h: u32,
    #[serde(default = "default_fluid_px")]
    pub width: f32,
    #[serde(default = "default_fluid_px")]
    pub height: f32,
    #[serde(default = "default_viscosity")]
    pub viscosity: f32,
    #[serde(default = "default_diffusion")]
    pub diffusion: f32,
    /// 渦度閉じ込め係数（0.0..=12.0、渦ディテールを維持）
    #[serde(default = "default_vorticity")]
    pub vorticity_confinement: f32,
    /// 浮力強度（上向き熱上昇流）
    #[serde(default = "default_buoyancy")]
    pub buoyancy: f32,
    /// 発生源 X（0.0..=1.0）
    #[serde(default = "default_half")]
    pub emitter_x: f32,
    /// 発生源 Y（0.0..=1.0、下が 1.0）
    #[serde(default = "default_emitter_y")]
    pub emitter_y: f32,
    /// 発生源半径（0.02..=0.25）
    #[serde(default = "default_emitter_r")]
    pub emitter_radius: f32,
    #[serde(default = "default_smoke_primary")]
    pub primary_color: RgbaColor,
    #[serde(default = "default_smoke_secondary")]
    pub secondary_color: RgbaColor,
    #[serde(default = "default_one")]
    pub time_scale: f32,
}

fn default_seed() -> u64 {
    2026_1058
}
fn default_fluid_grid() -> u32 {
    44
}
fn default_fluid_px() -> f32 {
    512.0
}
fn default_viscosity() -> f32 {
    0.0004
}
fn default_diffusion() -> f32 {
    0.00025
}
fn default_vorticity() -> f32 {
    3.6
}
fn default_buoyancy() -> f32 {
    1.8
}
fn default_half() -> f32 {
    0.5
}
fn default_emitter_y() -> f32 {
    0.82
}
fn default_emitter_r() -> f32 {
    0.09
}
fn default_one() -> f32 {
    1.0
}
fn default_smoke_primary() -> RgbaColor {
    RgbaColor {
        r: 0.86,
        g: 0.90,
        b: 0.96,
        a: 0.92,
    }
}
fn default_smoke_secondary() -> RgbaColor {
    RgbaColor {
        r: 0.30,
        g: 0.36,
        b: 0.48,
        a: 0.85,
    }
}

impl Default for FluidSimConfig {
    fn default() -> Self {
        Self::from_preset(FluidPreset::CinemaSmoke)
    }
}

impl FluidSimConfig {
    pub fn from_preset(preset: FluidPreset) -> Self {
        match preset {
            FluidPreset::CinemaSmoke => Self {
                preset,
                seed: default_seed(),
                grid_w: 44,
                grid_h: 44,
                width: 512.0,
                height: 512.0,
                viscosity: 0.0004,
                diffusion: 0.0003,
                vorticity_confinement: 3.6,
                buoyancy: 1.8,
                emitter_x: 0.5,
                emitter_y: 0.82,
                emitter_radius: 0.09,
                primary_color: default_smoke_primary(),
                secondary_color: default_smoke_secondary(),
                time_scale: 1.0,
            },
            FluidPreset::BonfireFlame => Self {
                preset,
                seed: default_seed() + 7,
                grid_w: 44,
                grid_h: 44,
                width: 512.0,
                height: 512.0,
                viscosity: 0.0002,
                diffusion: 0.00018,
                vorticity_confinement: 5.2,
                buoyancy: 3.4,
                emitter_x: 0.5,
                emitter_y: 0.85,
                emitter_radius: 0.10,
                primary_color: RgbaColor {
                    r: 1.0,
                    g: 0.88,
                    b: 0.30,
                    a: 1.0,
                },
                secondary_color: RgbaColor {
                    r: 0.96,
                    g: 0.24,
                    b: 0.05,
                    a: 0.95,
                },
                time_scale: 1.25,
            },
            FluidPreset::SumiInk => Self {
                preset,
                seed: default_seed() + 19,
                grid_w: 44,
                grid_h: 44,
                width: 512.0,
                height: 512.0,
                viscosity: 0.0012,
                diffusion: 0.00045,
                vorticity_confinement: 2.8,
                buoyancy: -0.6,
                emitter_x: 0.5,
                emitter_y: 0.28,
                emitter_radius: 0.11,
                primary_color: RgbaColor {
                    r: 0.08,
                    g: 0.10,
                    b: 0.16,
                    a: 0.96,
                },
                secondary_color: RgbaColor {
                    r: 0.24,
                    g: 0.34,
                    b: 0.52,
                    a: 0.85,
                },
                time_scale: 0.85,
            },
            FluidPreset::NeonPlasma => Self {
                preset,
                seed: default_seed() + 31,
                grid_w: 44,
                grid_h: 44,
                width: 512.0,
                height: 512.0,
                viscosity: 0.00015,
                diffusion: 0.0002,
                vorticity_confinement: 6.4,
                buoyancy: 1.2,
                emitter_x: 0.5,
                emitter_y: 0.55,
                emitter_radius: 0.10,
                primary_color: RgbaColor {
                    r: 0.16,
                    g: 0.95,
                    b: 0.98,
                    a: 1.0,
                },
                secondary_color: RgbaColor {
                    r: 0.92,
                    g: 0.18,
                    b: 0.86,
                    a: 0.95,
                },
                time_scale: 1.35,
            },
        }
    }
}

/// 決定論 2D 流体シミュレーション結果フィールド
#[derive(Debug, Clone)]
pub struct FluidField2D {
    pub width: usize,
    pub height: usize,
    pub density: Vec<f32>,
    pub temperature: Vec<f32>,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
    pub steps_simulated: usize,
    pub max_divergence_after_project: f32,
}

#[inline]
fn hash_f32(seed: u64, step: u64, salt: u64) -> f32 {
    let mut x = seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(step.wrapping_mul(0xbf58_476d_1ce4_e5b9))
        ^ salt.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    ((x >> 40) as f32) / ((1u64 << 24) as f32)
}

fn set_bnd(w: usize, h: usize, b: i32, x: &mut [f32]) {
    let idx = |i: usize, j: usize| j * w + i;
    for i in 1..(w - 1) {
        x[idx(i, 0)] = if b == 2 { -x[idx(i, 1)] } else { x[idx(i, 1)] };
        x[idx(i, h - 1)] = if b == 2 {
            -x[idx(i, h - 2)]
        } else {
            x[idx(i, h - 2)]
        };
    }
    for j in 1..(h - 1) {
        x[idx(0, j)] = if b == 1 { -x[idx(1, j)] } else { x[idx(1, j)] };
        x[idx(w - 1, j)] = if b == 1 {
            -x[idx(w - 2, j)]
        } else {
            x[idx(w - 2, j)]
        };
    }
    x[idx(0, 0)] = 0.5 * (x[idx(1, 0)] + x[idx(0, 1)]);
    x[idx(0, h - 1)] = 0.5 * (x[idx(1, h - 1)] + x[idx(0, h - 2)]);
    x[idx(w - 1, 0)] = 0.5 * (x[idx(w - 2, 0)] + x[idx(w - 1, 1)]);
    x[idx(w - 1, h - 1)] = 0.5 * (x[idx(w - 2, h - 1)] + x[idx(w - 1, h - 2)]);
}

#[allow(clippy::too_many_arguments)]
fn lin_solve(
    w: usize,
    h: usize,
    b: i32,
    x: &mut [f32],
    x0: &[f32],
    a: f32,
    c_recip: f32,
    iters: usize,
) {
    for _ in 0..iters {
        for j in 1..(h - 1) {
            let row = j * w;
            for i in 1..(w - 1) {
                let id = row + i;
                x[id] = (x0[id] + a * (x[id - 1] + x[id + 1] + x[id - w] + x[id + w])) * c_recip;
            }
        }
        set_bnd(w, h, b, x);
    }
}

fn diffuse(w: usize, h: usize, b: i32, x: &mut [f32], x0: &[f32], diff: f32, dt: f32) {
    let a = dt * diff * ((w - 2) * (h - 2)) as f32;
    lin_solve(w, h, b, x, x0, a, 1.0 / (1.0 + 4.0 * a), 8);
}

#[allow(clippy::too_many_arguments)]
fn advect(
    w: usize,
    h: usize,
    b: i32,
    d: &mut [f32],
    d0: &[f32],
    u: &[f32],
    v: &[f32],
    dt: f32,
    dissipation: f32,
) {
    let dtx = dt * (w - 2) as f32;
    let dty = dt * (h - 2) as f32;
    let max_x = (w - 2) as f32 + 0.5;
    let max_y = (h - 2) as f32 + 0.5;

    for j in 1..(h - 1) {
        for i in 1..(w - 1) {
            let id = j * w + i;
            let x = (i as f32 - dtx * u[id]).clamp(0.5, max_x);
            let y = (j as f32 - dty * v[id]).clamp(0.5, max_y);

            let i0 = x.floor() as usize;
            let i1 = (i0 + 1).min(w - 1);
            let j0 = y.floor() as usize;
            let j1 = (j0 + 1).min(h - 1);

            let s1 = x - i0 as f32;
            let s0 = 1.0 - s1;
            let t1 = y - j0 as f32;
            let t0 = 1.0 - t1;

            let val = s0 * (t0 * d0[j0 * w + i0] + t1 * d0[j1 * w + i0])
                + s1 * (t0 * d0[j0 * w + i1] + t1 * d0[j1 * w + i1]);
            d[id] = val * dissipation;
        }
    }
    set_bnd(w, h, b, d);
}

fn project(
    w: usize,
    h: usize,
    u: &mut [f32],
    v: &mut [f32],
    p: &mut [f32],
    div: &mut [f32],
) -> f32 {
    let hx = 1.0 / (w - 2) as f32;
    let hy = 1.0 / (h - 2) as f32;
    for j in 1..(h - 1) {
        let row = j * w;
        for i in 1..(w - 1) {
            let id = row + i;
            div[id] = -0.5 * (hx * (u[id + 1] - u[id - 1]) + hy * (v[id + w] - v[id - w]));
            p[id] = 0.0;
        }
    }
    set_bnd(w, h, 0, div);
    set_bnd(w, h, 0, p);
    lin_solve(w, h, 0, p, div, 1.0, 0.25, 14);

    let inv_hx = 0.5 / hx;
    let inv_hy = 0.5 / hy;
    for j in 1..(h - 1) {
        let row = j * w;
        for i in 1..(w - 1) {
            let id = row + i;
            u[id] -= inv_hx * (p[id + 1] - p[id - 1]);
            v[id] -= inv_hy * (p[id + w] - p[id - w]);
        }
    }
    set_bnd(w, h, 1, u);
    set_bnd(w, h, 2, v);

    let mut max_div = 0.0f32;
    for j in 1..(h - 1) {
        let row = j * w;
        for i in 1..(w - 1) {
            let id = row + i;
            let d = (0.5 * (hx * (u[id + 1] - u[id - 1]) + hy * (v[id + w] - v[id - w]))).abs();
            max_div = max_div.max(d);
        }
    }
    max_div
}

/// 指定時刻 `time_sec` における決定論 2D Navier-Stokes 流体フィールドを Jos Stam 安定法で計算する
pub fn simulate_fluid_2d_at_time(cfg: &FluidSimConfig, time_sec: f32) -> FluidField2D {
    let w = cfg.grid_w.clamp(16, 96) as usize;
    let h = cfg.grid_h.clamp(16, 96) as usize;
    let n = w * h;

    let mut u = vec![0.0f32; n];
    let mut v = vec![0.0f32; n];
    let mut u0 = vec![0.0f32; n];
    let mut v0 = vec![0.0f32; n];
    let mut dens = vec![0.0f32; n];
    let mut dens0 = vec![0.0f32; n];
    let mut temp = vec![0.0f32; n];
    let mut temp0 = vec![0.0f32; n];
    let mut curl = vec![0.0f32; n];

    let dt = 1.0 / 30.0;
    let eff_t = (time_sec.max(0.0) * cfg.time_scale.clamp(0.1, 4.0)).min(4.0);
    // プレビュー応答性を保ちつつ滑らかに進化する固定ステップ数（最低 6 ステップで初期プルーム形成）
    let steps = ((eff_t * 30.0).round() as usize).clamp(6, 72);

    let ex = cfg.emitter_x.clamp(0.1, 0.9) * (w as f32);
    let ey = cfg.emitter_y.clamp(0.1, 0.9) * (h as f32);
    let er = (cfg.emitter_radius.clamp(0.03, 0.28) * (w.min(h) as f32)).max(2.0);
    let er2 = er * er;
    let mut max_div = 0.0f32;

    for step in 0..steps {
        let phase = step as f32 * 0.22;
        let sway = (phase + hash_f32(cfg.seed, step as u64, 1) * 0.6).sin();
        let jet_vx = sway * 0.95;
        let jet_vy = if cfg.buoyancy >= 0.0 {
            -(1.15 + 0.35 * cfg.buoyancy.abs())
        } else {
            1.15 + 0.35 * cfg.buoyancy.abs()
        };

        // 発生源から密度・温度・初速を注入
        for j in 1..(h - 1) {
            let dy = j as f32 - ey;
            for i in 1..(w - 1) {
                let dx = i as f32 - ex;
                let d2 = dx * dx + dy * dy;
                if d2 < er2 {
                    let falloff = 1.0 - (d2 / er2).sqrt();
                    let id = j * w + i;
                    dens[id] = (dens[id] + 0.62 * falloff).min(2.2);
                    temp[id] = (temp[id] + 0.75 * falloff).min(2.2);
                    let swirl_x = -dy / er * 0.45 * sway;
                    let swirl_y = dx / er * 0.22;
                    u[id] += (jet_vx + swirl_x) * falloff * 0.55;
                    v[id] += (jet_vy + swirl_y) * falloff * 0.55;
                }
            }
        }

        // 熱浮力 + 渦度閉じ込め（Vorticity Confinement）
        for j in 1..(h - 1) {
            let row = j * w;
            for i in 1..(w - 1) {
                let id = row + i;
                v[id] -= cfg.buoyancy * temp[id] * dt * 1.65;
                let dvdx = 0.5 * (v[id + 1] - v[id - 1]);
                let dudy = 0.5 * (u[id + w] - u[id - w]);
                curl[id] = dvdx - dudy;
            }
        }
        let vort = cfg.vorticity_confinement.clamp(0.0, 12.0);
        if vort > 1e-4 {
            for j in 2..(h - 2) {
                let row = j * w;
                for i in 2..(w - 2) {
                    let id = row + i;
                    let dw_dx = 0.5 * (curl[id + 1].abs() - curl[id - 1].abs());
                    let dw_dy = 0.5 * (curl[id + w].abs() - curl[id - w].abs());
                    let len = dw_dx.hypot(dw_dy) + 1e-5;
                    let nx = dw_dx / len;
                    let ny = dw_dy / len;
                    let c = curl[id];
                    u[id] += ny * c * vort * dt * 0.45;
                    v[id] -= nx * c * vort * dt * 0.45;
                }
            }
        }

        // 速度ステップ: 拡散 → 投影 → 移流 → 投影
        u0.copy_from_slice(&u);
        v0.copy_from_slice(&v);
        diffuse(w, h, 1, &mut u, &u0, cfg.viscosity.max(0.0), dt);
        diffuse(w, h, 2, &mut v, &v0, cfg.viscosity.max(0.0), dt);
        project(w, h, &mut u, &mut v, &mut u0, &mut v0);

        u0.copy_from_slice(&u);
        v0.copy_from_slice(&v);
        advect(w, h, 1, &mut u, &u0, &u0, &v0, dt, 0.992);
        advect(w, h, 2, &mut v, &v0, &u0, &v0, dt, 0.992);
        max_div = project(w, h, &mut u, &mut v, &mut u0, &mut v0);

        // スカラー（密度・温度）ステップ: 拡散 → 移流
        dens0.copy_from_slice(&dens);
        diffuse(w, h, 0, &mut dens, &dens0, cfg.diffusion.max(0.0), dt);
        dens0.copy_from_slice(&dens);
        advect(w, h, 0, &mut dens, &dens0, &u, &v, dt, 0.978);

        temp0.copy_from_slice(&temp);
        diffuse(w, h, 0, &mut temp, &temp0, cfg.diffusion.max(0.0) * 1.4, dt);
        temp0.copy_from_slice(&temp);
        advect(w, h, 0, &mut temp, &temp0, &u, &v, dt, 0.955);
    }

    FluidField2D {
        width: w,
        height: h,
        density: dens,
        temperature: temp,
        u,
        v,
        steps_simulated: steps,
        max_divergence_after_project: max_div,
    }
}

// =========================================================================
// PBD（Position-Based Dynamics）クロス物理シミュレーション
// =========================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClothPreset {
    #[default]
    SilkBanner,
    HeroCape,
    CurtainDrape,
    CyberFlag,
}

impl ClothPreset {
    pub const ALL: &'static [Self] = &[
        Self::SilkBanner,
        Self::HeroCape,
        Self::CurtainDrape,
        Self::CyberFlag,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::SilkBanner => "🎀 シルクバナー (Silk Banner)",
            Self::HeroCape => "🦸 ヒーローマント (Hero Cape)",
            Self::CurtainDrape => "🎭 シアターカーテン (Curtain Drape)",
            Self::CyberFlag => "🚩 サイバーフラッグ (Cyber Flag)",
        }
    }
}

/// PBD クロスシミュレーションアイテム設定（`ItemKind::ClothSim`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothSimConfig {
    #[serde(default)]
    pub preset: ClothPreset,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_cloth_cols")]
    pub cols: u32,
    #[serde(default = "default_cloth_rows")]
    pub rows: u32,
    #[serde(default = "default_cloth_w")]
    pub width: f32,
    #[serde(default = "default_cloth_h")]
    pub height: f32,
    #[serde(default = "default_wind")]
    pub wind_strength: f32,
    #[serde(default = "default_turb")]
    pub wind_turbulence: f32,
    #[serde(default = "default_gravity")]
    pub gravity: f32,
    #[serde(default = "default_stiff")]
    pub stiffness: f32,
    #[serde(default = "default_damping")]
    pub damping: f32,
    #[serde(default = "default_true")]
    pub pin_top_edge: bool,
    #[serde(default)]
    pub pin_left_edge: bool,
    #[serde(default = "default_cloth_color")]
    pub cloth_color: RgbaColor,
    #[serde(default = "default_cloth_accent")]
    pub accent_color: RgbaColor,
    #[serde(default)]
    pub show_wireframe: bool,
}

fn default_cloth_cols() -> u32 {
    14
}
fn default_cloth_rows() -> u32 {
    10
}
fn default_cloth_w() -> f32 {
    480.0
}
fn default_cloth_h() -> f32 {
    340.0
}
fn default_wind() -> f32 {
    1.6
}
fn default_turb() -> f32 {
    0.85
}
fn default_gravity() -> f32 {
    1.1
}
fn default_stiff() -> f32 {
    0.86
}
fn default_damping() -> f32 {
    0.975
}
fn default_true() -> bool {
    true
}
fn default_cloth_color() -> RgbaColor {
    RgbaColor {
        r: 0.86,
        g: 0.16,
        b: 0.24,
        a: 1.0,
    }
}
fn default_cloth_accent() -> RgbaColor {
    RgbaColor {
        r: 0.98,
        g: 0.82,
        b: 0.32,
        a: 1.0,
    }
}

impl Default for ClothSimConfig {
    fn default() -> Self {
        Self::from_preset(ClothPreset::SilkBanner)
    }
}

impl ClothSimConfig {
    pub fn from_preset(preset: ClothPreset) -> Self {
        match preset {
            ClothPreset::SilkBanner => Self {
                preset,
                seed: 1058,
                cols: 14,
                rows: 10,
                width: 480.0,
                height: 340.0,
                wind_strength: 1.6,
                wind_turbulence: 0.85,
                gravity: 1.05,
                stiffness: 0.86,
                damping: 0.975,
                pin_top_edge: true,
                pin_left_edge: false,
                cloth_color: default_cloth_color(),
                accent_color: default_cloth_accent(),
                show_wireframe: false,
            },
            ClothPreset::HeroCape => Self {
                preset,
                seed: 2058,
                cols: 12,
                rows: 12,
                width: 380.0,
                height: 460.0,
                wind_strength: 2.3,
                wind_turbulence: 1.15,
                gravity: 1.35,
                stiffness: 0.82,
                damping: 0.972,
                pin_top_edge: true,
                pin_left_edge: false,
                cloth_color: RgbaColor {
                    r: 0.72,
                    g: 0.08,
                    b: 0.14,
                    a: 1.0,
                },
                accent_color: RgbaColor {
                    r: 0.96,
                    g: 0.76,
                    b: 0.22,
                    a: 1.0,
                },
                show_wireframe: false,
            },
            ClothPreset::CurtainDrape => Self {
                preset,
                seed: 3058,
                cols: 16,
                rows: 12,
                width: 560.0,
                height: 420.0,
                wind_strength: 0.75,
                wind_turbulence: 0.45,
                gravity: 1.5,
                stiffness: 0.90,
                damping: 0.98,
                pin_top_edge: true,
                pin_left_edge: false,
                cloth_color: RgbaColor {
                    r: 0.42,
                    g: 0.07,
                    b: 0.18,
                    a: 1.0,
                },
                accent_color: RgbaColor {
                    r: 0.94,
                    g: 0.78,
                    b: 0.40,
                    a: 1.0,
                },
                show_wireframe: false,
            },
            ClothPreset::CyberFlag => Self {
                preset,
                seed: 4058,
                cols: 16,
                rows: 10,
                width: 520.0,
                height: 320.0,
                wind_strength: 3.1,
                wind_turbulence: 1.35,
                gravity: 0.65,
                stiffness: 0.88,
                damping: 0.97,
                pin_top_edge: false,
                pin_left_edge: true,
                cloth_color: RgbaColor {
                    r: 0.12,
                    g: 0.26,
                    b: 0.84,
                    a: 1.0,
                },
                accent_color: RgbaColor {
                    r: 0.16,
                    g: 0.94,
                    b: 0.96,
                    a: 1.0,
                },
                show_wireframe: false,
            },
        }
    }
}

/// PBD クロスシミュレーションの出力メッシュ状態
#[derive(Debug, Clone)]
pub struct ClothMeshState {
    pub cols: usize,
    pub rows: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub pinned: Vec<bool>,
}

/// 決定論 PBD（Position-Based Dynamics）クロスソルバーで指定時刻の布頂点＆法線を算出する
pub fn simulate_cloth_pbd_at_time(cfg: &ClothSimConfig, time_sec: f32) -> ClothMeshState {
    let cols = cfg.cols.clamp(4, 28) as usize;
    let rows = cfg.rows.clamp(4, 28) as usize;
    let n = cols * rows;

    let mut pos = Vec::with_capacity(n);
    let mut prev = Vec::with_capacity(n);
    let mut uvs = Vec::with_capacity(n);
    let mut pinned = vec![false; n];

    let dx = 1.6 / (cols - 1) as f32;
    let dy = 1.2 / (rows - 1) as f32;

    for r in 0..rows {
        let v = r as f32 / (rows - 1) as f32;
        for c in 0..cols {
            let u = c as f32 / (cols - 1) as f32;
            let x = -0.8 + c as f32 * dx;
            let y = 0.6 - r as f32 * dy;
            pos.push([x, y, 0.0]);
            prev.push([x, y, 0.0]);
            uvs.push([u, v]);

            let id = r * cols + c;
            if cfg.pin_top_edge && r == 0 && (c == 0 || c == cols - 1 || c % 3 == 0) {
                pinned[id] = true;
            }
            if cfg.pin_left_edge && c == 0 {
                pinned[id] = true;
            }
        }
    }
    // 少なくとも 2 点は固定して落下消失を防ぐ
    if !pinned.iter().any(|&p| p) {
        pinned[0] = true;
        pinned[cols - 1] = true;
    }

    // 構造・せん断・曲げ距離拘束リスト構築
    let mut constraints: Vec<(usize, usize, f32, f32)> = Vec::new();
    let diag = dx.hypot(dy);
    let stiff = cfg.stiffness.clamp(0.2, 1.0);
    for r in 0..rows {
        for c in 0..cols {
            let a = r * cols + c;
            if c + 1 < cols {
                constraints.push((a, a + 1, dx, stiff));
            }
            if r + 1 < rows {
                constraints.push((a, a + cols, dy, stiff));
            }
            if c + 1 < cols && r + 1 < rows {
                constraints.push((a, a + cols + 1, diag, stiff * 0.75));
                constraints.push((a + 1, a + cols, diag, stiff * 0.75));
            }
            if c + 2 < cols {
                constraints.push((a, a + 2, dx * 2.0, stiff * 0.45));
            }
            if r + 2 < rows {
                constraints.push((a, a + cols * 2, dy * 2.0, stiff * 0.45));
            }
        }
    }

    let dt = 1.0 / 60.0;
    let dt2 = dt * dt;
    let damp = cfg.damping.clamp(0.85, 0.999);
    let steps = ((time_sec.clamp(0.0, 4.0) * 60.0).round() as usize).clamp(8, 120);
    let seed_phase = hash_f32(cfg.seed, 1, 7) * TAU;

    for step in 0..steps {
        let t = step as f32 * dt;
        for (i, ((p, pv), &is_pin)) in pos
            .iter_mut()
            .zip(prev.iter_mut())
            .zip(pinned.iter())
            .enumerate()
        {
            if is_pin {
                continue;
            }
            let cur = *p;
            let vx = (cur[0] - pv[0]) * damp;
            let vy = (cur[1] - pv[1]) * damp;
            let vz = (cur[2] - pv[2]) * damp;
            *pv = cur;

            let uv = uvs[i];
            let gust = (t * 3.8 + uv[0] * 5.0 + uv[1] * 2.5 + seed_phase).sin();
            let ripple = (t * 6.4 - uv[0] * 7.2 + uv[1] * 4.1).cos();
            let fx = cfg.wind_strength * (0.45 + 0.35 * gust);
            let fy = -cfg.gravity * 1.45;
            let fz = cfg.wind_strength * (0.65 * gust + cfg.wind_turbulence * 0.48 * ripple);

            p[0] = cur[0] + vx + fx * dt2;
            p[1] = cur[1] + vy + fy * dt2;
            p[2] = (cur[2] + vz + fz * dt2).clamp(-0.75, 0.75);
        }

        // PBD 距離拘束プロジェクション（5 イテレーション）
        for _ in 0..5 {
            for &(a, b, rest, k) in &constraints {
                let pa = pos[a];
                let pb = pos[b];
                let d0 = pb[0] - pa[0];
                let d1 = pb[1] - pa[1];
                let d2 = pb[2] - pa[2];
                let dist = (d0 * d0 + d1 * d1 + d2 * d2).sqrt().max(1e-5);
                let corr = (dist - rest) / dist * k;
                let pin_a = pinned[a];
                let pin_b = pinned[b];
                match (pin_a, pin_b) {
                    (true, true) => {}
                    (true, false) => {
                        pos[b][0] -= d0 * corr;
                        pos[b][1] -= d1 * corr;
                        pos[b][2] -= d2 * corr;
                    }
                    (false, true) => {
                        pos[a][0] += d0 * corr;
                        pos[a][1] += d1 * corr;
                        pos[a][2] += d2 * corr;
                    }
                    (false, false) => {
                        let h = 0.5 * corr;
                        pos[a][0] += d0 * h;
                        pos[a][1] += d1 * h;
                        pos[a][2] += d2 * h;
                        pos[b][0] -= d0 * h;
                        pos[b][1] -= d1 * h;
                        pos[b][2] -= d2 * h;
                    }
                }
            }
        }
    }

    // 頂点法線算出（シェーディング用）
    let mut normals = vec![[0.0f32, 0.0, 1.0]; n];
    for r in 0..rows {
        for c in 0..cols {
            let id = r * cols + c;
            let p_l = pos[r * cols + c.saturating_sub(1)];
            let p_r = pos[r * cols + (c + 1).min(cols - 1)];
            let p_u = pos[r.saturating_sub(1) * cols + c];
            let p_d = pos[(r + 1).min(rows - 1) * cols + c];
            let tx = [p_r[0] - p_l[0], p_r[1] - p_l[1], p_r[2] - p_l[2]];
            let ty = [p_u[0] - p_d[0], p_u[1] - p_d[1], p_u[2] - p_d[2]];
            let nx = tx[1] * ty[2] - tx[2] * ty[1];
            let ny = tx[2] * ty[0] - tx[0] * ty[2];
            let nz = tx[0] * ty[1] - tx[1] * ty[0];
            let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            normals[id] = [nx / len, ny / len, nz / len];
        }
    }

    ClothMeshState {
        cols,
        rows,
        positions: pos,
        normals,
        uvs,
        pinned,
    }
}

// =========================================================================
// 3D シャドウマップ ＋ Equirectangular HDRI IBL 環境照明
// =========================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdriEnvPreset {
    #[default]
    StudioSoftbox,
    GoldenSunset,
    CyberNeonNight,
    OvercastDaylight,
}

impl HdriEnvPreset {
    pub const ALL: &'static [Self] = &[
        Self::StudioSoftbox,
        Self::GoldenSunset,
        Self::CyberNeonNight,
        Self::OvercastDaylight,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::StudioSoftbox => "💡 スタジオソフトボックス HDRI",
            Self::GoldenSunset => "🌅 ゴールデンアワー夕景 HDRI",
            Self::CyberNeonNight => "🌃 サイバーネオン夜景 HDRI",
            Self::OvercastDaylight => "☁️ 曇天ソフトデイライト HDRI",
        }
    }
}

/// シーン単位の 3D ソフトシャドウマップ ＆ HDRI IBL 環境照明設定（`Scene::lighting_3d_adv`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lighting3DAdvConfig {
    #[serde(default)]
    pub shadow_map_enabled: bool,
    /// デプスシャドウマップ解像度（64..=512）
    #[serde(default = "default_shadow_res")]
    pub shadow_map_size: u32,
    /// シャドウアクネ防止デプスバイアス
    #[serde(default = "default_shadow_bias")]
    pub shadow_bias: f32,
    /// PCF フィルタ半径（1=3×3 ソフトシャドウ, 2=5×5）
    #[serde(default = "default_pcf")]
    pub shadow_softness_pcf: u32,
    /// 影の濃さ（0.0..=1.0）
    #[serde(default = "default_shadow_strength")]
    pub shadow_strength: f32,
    /// Equirectangular HDRI IBL 有効化
    #[serde(default)]
    pub hdri_ibl_enabled: bool,
    #[serde(default)]
    pub hdri_preset: HdriEnvPreset,
    /// HDRI 環境マップ Y 軸回転角（0..=360 度）
    #[serde(default)]
    pub hdri_rotation_deg: f32,
    /// HDRI IBL 露出倍率（0.1..=4.0）
    #[serde(default = "default_one")]
    pub hdri_exposure: f32,
    /// メッシュ下部に影を受けるスタジオ床プレーンを追加
    #[serde(default = "default_true")]
    pub ground_catcher_enabled: bool,
}

fn default_shadow_res() -> u32 {
    192
}
fn default_shadow_bias() -> f32 {
    0.025
}
fn default_pcf() -> u32 {
    1
}
fn default_shadow_strength() -> f32 {
    0.72
}

impl Default for Lighting3DAdvConfig {
    fn default() -> Self {
        Self {
            shadow_map_enabled: false,
            shadow_map_size: default_shadow_res(),
            shadow_bias: default_shadow_bias(),
            shadow_softness_pcf: default_pcf(),
            shadow_strength: default_shadow_strength(),
            hdri_ibl_enabled: false,
            hdri_preset: HdriEnvPreset::StudioSoftbox,
            hdri_rotation_deg: 0.0,
            hdri_exposure: 1.0,
            ground_catcher_enabled: true,
        }
    }
}

/// Equirectangular プロシージャル HDRI 環境マップから方向 `dir`・ラフネス `roughness` に応じた IBL 放射輝度 `[R, G, B]` をサンプルする
pub fn sample_hdri_ibl(
    preset: HdriEnvPreset,
    dir: [f32; 3],
    roughness: f32,
    rotation_deg: f32,
    exposure: f32,
) -> [f32; 3] {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
        .sqrt()
        .max(1e-6);
    let dx = dir[0] / len;
    let dy = (dir[1] / len).clamp(-1.0, 1.0);
    let dz = dir[2] / len;
    let rot = rotation_deg.to_radians();
    let (sr, cr) = rot.sin_cos();
    let rx = dx * cr + dz * sr;
    let rz = -dx * sr + dz * cr;

    let lon = rx.atan2(rz); // -PI..=PI
    let lat = dy.asin(); // -PI/2..=PI/2
    let sharp = (1.0 - roughness.clamp(0.0, 1.0)).powi(2);
    let exp = exposure.clamp(0.05, 5.0);

    let (base, lobe) = match preset {
        HdriEnvPreset::StudioSoftbox => {
            let sky = if dy >= 0.0 {
                [0.24 + 0.34 * dy, 0.27 + 0.36 * dy, 0.33 + 0.40 * dy]
            } else {
                [0.09, 0.10, 0.12]
            };
            // 上部・左右 2 基のソフトボックス高輝度 HDR ローブ
            let key = ((lon - 0.85).cos().max(0.0) * (lat - 0.55).cos().max(0.0))
                .powf(4.0 + 18.0 * sharp);
            let fill = ((lon + 1.35).cos().max(0.0) * (lat - 0.35).cos().max(0.0))
                .powf(3.0 + 10.0 * sharp);
            (
                sky,
                [
                    key * 2.4 + fill * 0.9,
                    key * 2.35 + fill * 1.05,
                    key * 2.2 + fill * 1.35,
                ],
            )
        }
        HdriEnvPreset::GoldenSunset => {
            let horizon = (1.0 - dy.abs()).powf(2.4);
            let sky = if dy >= 0.0 {
                [
                    0.18 + 0.85 * horizon,
                    0.16 + 0.42 * horizon,
                    0.28 + 0.12 * horizon,
                ]
            } else {
                [0.10 + 0.18 * horizon, 0.07 + 0.10 * horizon, 0.08]
            };
            let sun = ((lon - 0.35).cos().max(0.0) * (lat - 0.14).cos().max(0.0))
                .powf(6.0 + 36.0 * sharp);
            (sky, [sun * 4.2, sun * 2.1, sun * 0.65])
        }
        HdriEnvPreset::CyberNeonNight => {
            let band = (1.0 - (dy + 0.05).abs()).powf(3.0);
            let magenta = (0.5 + 0.5 * (lon * 2.0 + 0.4).sin()).powf(2.0 + 8.0 * sharp) * band;
            let cyan = (0.5 + 0.5 * (lon * 2.0 - 2.2).sin()).powf(2.0 + 8.0 * sharp) * band;
            let sky = [0.04, 0.05, 0.11 + 0.08 * dy.max(0.0)];
            (
                sky,
                [
                    magenta * 2.8 + cyan * 0.2,
                    magenta * 0.25 + cyan * 2.4,
                    magenta * 2.4 + cyan * 2.9,
                ],
            )
        }
        HdriEnvPreset::OvercastDaylight => {
            let dome = 0.38 + 0.42 * (dy * 0.5 + 0.5);
            let sky = [dome * 0.94, dome * 0.98, dome * 1.04];
            let zenith = (dy.max(0.0)).powf(1.5 + 4.0 * sharp);
            (sky, [zenith * 0.85, zenith * 0.90, zenith * 0.96])
        }
    };

    let _ = PI;
    [
        (base[0] + lobe[0]) * exp,
        (base[1] + lobe[1]) * exp,
        (base[2] + lobe[2]) * exp,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navier_stokes_2d_fluid_is_deterministic_and_nearly_incompressible() {
        let cfg = FluidSimConfig::from_preset(FluidPreset::BonfireFlame);
        let f1 = simulate_fluid_2d_at_time(&cfg, 1.2);
        let f2 = simulate_fluid_2d_at_time(&cfg, 1.2);
        assert_eq!(f1.density, f2.density);
        assert_eq!(f1.temperature, f2.temperature);
        let total_density: f32 = f1.density.iter().sum();
        assert!(total_density > 5.0, "expected active fluid plume");
        assert!(
            f1.max_divergence_after_project < 0.25,
            "expected bounded divergence after Poisson projection, got {}",
            f1.max_divergence_after_project
        );
    }

    #[test]
    fn pbd_cloth_preserves_pinned_anchors_and_billows_under_wind() {
        let cfg = ClothSimConfig::from_preset(ClothPreset::CyberFlag);
        let c0 = simulate_cloth_pbd_at_time(&cfg, 0.1);
        let c1 = simulate_cloth_pbd_at_time(&cfg, 1.4);
        assert_eq!(c0.positions[0], c1.positions[0]);
        let max_z = c1
            .positions
            .iter()
            .map(|p| p[2].abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_z > 0.02,
            "expected cloth billowing along Z, got {max_z}"
        );
        assert!(
            c1.normals
                .iter()
                .all(|n| n[0].is_finite() && n[2].is_finite())
        );
    }

    #[test]
    fn hdri_ibl_sampler_varies_by_preset_and_direction() {
        let sunset_sun = sample_hdri_ibl(
            HdriEnvPreset::GoldenSunset,
            [0.34, 0.14, 0.93],
            0.1,
            0.0,
            1.0,
        );
        let neon = sample_hdri_ibl(
            HdriEnvPreset::CyberNeonNight,
            [0.8, 0.0, 0.6],
            0.2,
            0.0,
            1.0,
        );
        assert!(
            sunset_sun[0] > sunset_sun[2] * 1.5,
            "golden sunset should be warm"
        );
        assert!(
            neon[2] > 0.2,
            "cyber neon should emit strong blue/cyan/magenta"
        );
    }
}
