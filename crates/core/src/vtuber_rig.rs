//! v9.0 #54: VTuber 2D 三角メッシュ変形（ブレンドシェイプ）・Verlet 物理揺れ・UDP OSC / VMC 顔トラッキング連携
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで以下を実現する:
//! - OSC 1.0 バイナリパケット（メッセージ＆`#bundle`）の完全エンコード／デコード
//! - VMC プロトコル（`/VMC/Ext/Blend/Val`, `/VMC/Ext/Bone/Pos` クォータニオン→オイラー変換）のリアルタイム受信
//! - 2D 三角グリッドメッシュ生成・ブレンドシェイプ変形・距離拘束付き Verlet 物理チェイン（髪・衣装揺れ）
//! - 顔トラッキング録画フレームの 1-Undo キーフレーム自動焼き込み

use crate::animation::{Interpolation, Keyframe};
use crate::command::EditCommand;
use crate::project::{AssetId, ItemId, RgbaColor};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use std::net::UdpSocket;

// ============================================================================
// 1. Pure-Rust OSC 1.0 バイナリパケット コーデック（#54 VMC & #55 ハードウェア共通）
// ============================================================================

/// OSC 1.0 引数値
#[derive(Debug, Clone, PartialEq)]
pub enum OscArg {
    Int(i32),
    Float(f32),
    String(String),
    Bool(bool),
}

impl OscArg {
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Self::Float(v) => Some(v),
            Self::Int(v) => Some(v as f32),
            Self::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
            Self::String(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// デコード済み OSC メッセージ
#[derive(Debug, Clone, PartialEq)]
pub struct OscMessage {
    pub address: String,
    pub args: Vec<OscArg>,
}

fn push_osc_padded_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn read_osc_padded_str(data: &[u8], cursor: &mut usize) -> Option<String> {
    if *cursor >= data.len() {
        return None;
    }
    let rel_nul = data[*cursor..].iter().position(|&b| b == 0)?;
    let s = std::str::from_utf8(&data[*cursor..*cursor + rel_nul])
        .ok()?
        .to_string();
    let next = *cursor + rel_nul + 1;
    let aligned = (next + 3) & !3;
    *cursor = aligned.min(data.len());
    Some(s)
}

/// OSC 1.0 メッセージを 4 バイト境界パディング付きバイナリへエンコードする
pub fn encode_osc_message(address: &str, args: &[OscArg]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    push_osc_padded_str(&mut out, address);
    let mut tags = String::with_capacity(args.len() + 1);
    tags.push(',');
    for a in args {
        match a {
            OscArg::Int(_) => tags.push('i'),
            OscArg::Float(_) => tags.push('f'),
            OscArg::String(_) => tags.push('s'),
            OscArg::Bool(true) => tags.push('T'),
            OscArg::Bool(false) => tags.push('F'),
        }
    }
    push_osc_padded_str(&mut out, &tags);
    for a in args {
        match a {
            OscArg::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::String(s) => push_osc_padded_str(&mut out, s),
            OscArg::Bool(_) => {}
        }
    }
    out
}

/// 単一 OSC メッセージまたは `#bundle` バイナリパケットを全メッセージ列へデコードする
pub fn decode_osc_packet(data: &[u8]) -> Vec<OscMessage> {
    if data.starts_with(b"#bundle\0") && data.len() >= 16 {
        let mut out = Vec::new();
        let mut pos = 16usize; // "#bundle\0" (8) + timetag (8)
        while pos + 4 <= data.len() {
            let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                as usize;
            pos += 4;
            if len == 0 || pos + len > data.len() {
                break;
            }
            out.extend(decode_osc_packet(&data[pos..pos + len]));
            pos += len;
        }
        return out;
    }

    let mut cur = 0usize;
    let Some(address) = read_osc_padded_str(data, &mut cur) else {
        return Vec::new();
    };
    if !address.starts_with('/') {
        return Vec::new();
    }
    let Some(tags) = read_osc_padded_str(data, &mut cur) else {
        return Vec::new();
    };
    let mut args = Vec::new();
    for ch in tags.chars().skip(1) {
        match ch {
            'i' => {
                if cur + 4 > data.len() {
                    break;
                }
                let v =
                    i32::from_be_bytes([data[cur], data[cur + 1], data[cur + 2], data[cur + 3]]);
                cur += 4;
                args.push(OscArg::Int(v));
            }
            'f' => {
                if cur + 4 > data.len() {
                    break;
                }
                let v =
                    f32::from_be_bytes([data[cur], data[cur + 1], data[cur + 2], data[cur + 3]]);
                cur += 4;
                if v.is_finite() {
                    args.push(OscArg::Float(v));
                }
            }
            's' => {
                if let Some(s) = read_osc_padded_str(data, &mut cur) {
                    args.push(OscArg::String(s));
                } else {
                    break;
                }
            }
            'T' => args.push(OscArg::Bool(true)),
            'F' => args.push(OscArg::Bool(false)),
            _ => {}
        }
    }
    vec![OscMessage { address, args }]
}

// ============================================================================
// 2. VTuber 2D 三角メッシュ・ブレンドシェイプ・Verlet 物理データモデル
// ============================================================================

/// 組み込み VTuber 2D アバター外観プリセット（PNG 素材なしでも即時利用可能）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VtuberAvatarPreset {
    #[default]
    AnimeCyan,
    CyberFoxViolet,
    MascotCatGold,
    MechaIdolWhite,
}

impl VtuberAvatarPreset {
    pub const ALL: &'static [Self] = &[
        Self::AnimeCyan,
        Self::CyberFoxViolet,
        Self::MascotCatGold,
        Self::MechaIdolWhite,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::AnimeCyan => "💙 シアン・アニメアイドル",
            Self::CyberFoxViolet => "🦊 サイバー九尾フォックス",
            Self::MascotCatGold => "🐱 ゴールデンマスコット猫",
            Self::MechaIdolWhite => "🤖 メカヴァーチャルユニット",
        }
    }

    pub fn palette(self) -> (RgbaColor, RgbaColor, RgbaColor) {
        // (primary/outfit, hair, eye)
        match self {
            Self::AnimeCyan => (
                RgbaColor {
                    r: 0.16,
                    g: 0.68,
                    b: 0.96,
                    a: 1.0,
                },
                RgbaColor {
                    r: 0.22,
                    g: 0.84,
                    b: 0.98,
                    a: 1.0,
                },
                RgbaColor {
                    r: 0.12,
                    g: 0.45,
                    b: 0.95,
                    a: 1.0,
                },
            ),
            Self::CyberFoxViolet => (
                RgbaColor {
                    r: 0.68,
                    g: 0.26,
                    b: 0.96,
                    a: 1.0,
                },
                RgbaColor {
                    r: 0.96,
                    g: 0.36,
                    b: 0.68,
                    a: 1.0,
                },
                RgbaColor {
                    r: 1.0,
                    g: 0.82,
                    b: 0.24,
                    a: 1.0,
                },
            ),
            Self::MascotCatGold => (
                RgbaColor {
                    r: 0.95,
                    g: 0.62,
                    b: 0.18,
                    a: 1.0,
                },
                RgbaColor {
                    r: 0.98,
                    g: 0.82,
                    b: 0.42,
                    a: 1.0,
                },
                RgbaColor {
                    r: 0.24,
                    g: 0.78,
                    b: 0.48,
                    a: 1.0,
                },
            ),
            Self::MechaIdolWhite => (
                RgbaColor {
                    r: 0.86,
                    g: 0.92,
                    b: 0.98,
                    a: 1.0,
                },
                RgbaColor {
                    r: 0.68,
                    g: 0.78,
                    b: 0.90,
                    a: 1.0,
                },
                RgbaColor {
                    r: 0.18,
                    g: 0.98,
                    b: 0.82,
                    a: 1.0,
                },
            ),
        }
    }
}

/// VTuber 表情・頭部姿勢ブレンドシェイプパラメータ
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct VtuberBlendShapes {
    /// 左目閉じ (`0.0`=開, `1.0`=閉)
    #[serde(default)]
    pub blink_l: f32,
    /// 右目閉じ (`0.0`=開, `1.0`=閉)
    #[serde(default)]
    pub blink_r: f32,
    /// 口の開き (`0.0..=1.0`)
    #[serde(default)]
    pub mouth_open: f32,
    /// 笑顔・口角 (`0.0..=1.0`)
    #[serde(default)]
    pub mouth_smile: f32,
    /// 眉の上下 (`-1.0..=1.0`)
    #[serde(default)]
    pub brow_y: f32,
    /// 顔左右向きヨー角（度: `-35..=35`）
    #[serde(default)]
    pub head_yaw_deg: f32,
    /// 顔上下うなずきピッチ角（度: `-25..=25`）
    #[serde(default)]
    pub head_pitch_deg: f32,
    /// 顔傾きロール角（度: `-25..=25`）
    #[serde(default)]
    pub head_roll_deg: f32,
    /// 視線左右 (`-1.0..=1.0`)
    #[serde(default)]
    pub eye_look_x: f32,
    /// 視線上下 (`-1.0..=1.0`)
    #[serde(default)]
    pub eye_look_y: f32,
}

fn default_true() -> bool {
    true
}
fn default_verlet_gravity() -> f32 {
    0.85
}
fn default_verlet_stiffness() -> f32 {
    0.72
}
fn default_verlet_damping() -> f32 {
    0.88
}
fn default_verlet_nodes() -> usize {
    6
}

/// 髪・衣装の距離拘束付き Verlet 物理揺れ設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerletChainConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_verlet_gravity")]
    pub gravity: f32,
    #[serde(default = "default_verlet_stiffness")]
    pub stiffness: f32,
    #[serde(default = "default_verlet_damping")]
    pub damping: f32,
    #[serde(default)]
    pub wind_amp: f32,
    #[serde(default)]
    pub wind_hz: f32,
    #[serde(default = "default_verlet_nodes")]
    pub node_count: usize,
}

impl Default for VerletChainConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gravity: default_verlet_gravity(),
            stiffness: default_verlet_stiffness(),
            damping: default_verlet_damping(),
            wind_amp: 0.14,
            wind_hz: 1.2,
            node_count: default_verlet_nodes(),
        }
    }
}

fn default_vt_dim() -> f32 {
    460.0
}
fn default_grid_n() -> usize {
    8
}
fn default_vmc_port() -> u16 {
    39539
}

/// `ItemKind::VtuberAvatar` 設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VtuberMeshConfig {
    #[serde(default = "default_vt_dim")]
    pub width: f32,
    #[serde(default = "default_vt_dim")]
    pub height: f32,
    #[serde(default)]
    pub preset: VtuberAvatarPreset,
    /// カスタム立ち絵 PNG/画像アセット ID（`None` なら高品質プロシージャル描画）
    #[serde(default)]
    pub asset_id: Option<AssetId>,
    #[serde(default = "default_grid_n")]
    pub grid_cols: usize,
    #[serde(default = "default_grid_n")]
    pub grid_rows: usize,
    #[serde(default)]
    pub blend_shapes: VtuberBlendShapes,
    #[serde(default)]
    pub physics: VerletChainConfig,
    /// タイムライン再生中の自動まばたき・呼吸アイドルモーション
    #[serde(default = "default_true")]
    pub auto_idle_animate: bool,
    /// VMC プロトコル UDP 受信ポート（既定 39539）
    #[serde(default = "default_vmc_port")]
    pub vmc_udp_port: u16,
    /// 三角メッシュワイヤーフレームをデバッグ重畳表示するか
    #[serde(default)]
    pub show_wireframe: bool,
}

impl Default for VtuberMeshConfig {
    fn default() -> Self {
        Self {
            width: default_vt_dim(),
            height: default_vt_dim(),
            preset: VtuberAvatarPreset::AnimeCyan,
            asset_id: None,
            grid_cols: default_grid_n(),
            grid_rows: default_grid_n(),
            blend_shapes: VtuberBlendShapes::default(),
            physics: VerletChainConfig::default(),
            auto_idle_animate: true,
            vmc_udp_port: default_vmc_port(),
            show_wireframe: false,
        }
    }
}

/// 2D 三角メッシュの 1 頂点（正規化座標 `0..=1`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mesh2DVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
}

/// 2D 三角メッシュの三角形インデックス
pub type Mesh2DTriangle = [usize; 3];

/// `(cols × rows)` の規則三角グリッドメッシュを生成する
pub fn build_vtuber_grid_mesh(
    cols: usize,
    rows: usize,
) -> (Vec<Mesh2DVertex>, Vec<Mesh2DTriangle>) {
    let c = cols.clamp(3, 18);
    let r = rows.clamp(3, 18);
    let mut verts = Vec::with_capacity((c + 1) * (r + 1));
    for iy in 0..=r {
        let v = iy as f32 / r as f32;
        for ix in 0..=c {
            let u = ix as f32 / c as f32;
            verts.push(Mesh2DVertex {
                pos: [u, v],
                uv: [u, v],
            });
        }
    }
    let stride = c + 1;
    let mut tris = Vec::with_capacity(c * r * 2);
    for iy in 0..r {
        for ix in 0..c {
            let tl = iy * stride + ix;
            let tr = tl + 1;
            let bl = (iy + 1) * stride + ix;
            let br = bl + 1;
            // 中心対称になるよう象限ごとに分割対角線を反転
            if (ix < c / 2) ^ (iy < r / 2) {
                tris.push([tl, tr, br]);
                tris.push([tl, br, bl]);
            } else {
                tris.push([tl, tr, bl]);
                tris.push([tr, br, bl]);
            }
        }
    }
    (verts, tris)
}

/// 距離拘束付き決定論 Verlet 物理チェインを 60Hz でシミュレーションし、各ノードの正規化オフセット `[dx, dy]` を返す
pub fn simulate_verlet_hair_chain(
    cfg: &VerletChainConfig,
    head_yaw_deg: f32,
    head_roll_deg: f32,
    time_sec: f32,
    side_sign: f32,
) -> Vec<[f32; 2]> {
    let n = cfg.node_count.clamp(2, 12);
    if !cfg.enabled {
        return vec![[0.0, 0.0]; n];
    }

    let seg_len = 0.065_f32;
    let mut pos: Vec<[f32; 2]> = (0..n).map(|i| [0.0, i as f32 * seg_len]).collect();
    let mut prev = pos.clone();

    // 決定論的 24 ステップ固定 60Hz シミュレーション（任意シーク時刻で完全一致）
    let dt = 1.0_f32 / 60.0;
    let steps = 24usize;
    let damp = cfg.damping.clamp(0.5, 0.99);
    let grav = cfg.gravity.clamp(0.1, 3.0) * 0.45;
    let stiff = cfg.stiffness.clamp(0.1, 1.0);

    for step in 0..steps {
        let t = (time_sec - ((steps - 1 - step) as f32) * dt).max(0.0);
        let sway_drive = (head_yaw_deg / 35.0) * 0.08
            + (head_roll_deg / 25.0) * 0.06
            + cfg.wind_amp
                * (std::f32::consts::TAU * cfg.wind_hz.max(0.1) * t + side_sign * 0.7).sin()
                * 0.08;
        // 根元アンカー位置
        pos[0] = [sway_drive * 0.35, 0.0];
        prev[0] = pos[0];

        // Verlet 積分
        for (i, (p, pv)) in pos.iter_mut().zip(prev.iter_mut()).enumerate().skip(1) {
            let cur = *p;
            let vx = (cur[0] - pv[0]) * damp;
            let vy = (cur[1] - pv[1]) * damp;
            *pv = cur;
            let frac = i as f32 / (n - 1) as f32;
            let wind_x = sway_drive * frac * 0.42;
            let restore_x = -cur[0] * stiff * 0.08;
            p[0] += vx + (wind_x + restore_x) * dt * 60.0 * 0.08;
            p[1] += vy + grav * dt * dt * 22.0;
        }

        // 距離拘束のヤコビ反復投影（4 イテレーション）
        for _ in 0..4 {
            for i in 1..n {
                let dx = pos[i][0] - pos[i - 1][0];
                let dy = pos[i][1] - pos[i - 1][1];
                let dist = dx.hypot(dy).max(1e-5);
                let diff = (dist - seg_len) / dist;
                if i == 1 {
                    pos[i][0] -= dx * diff;
                    pos[i][1] -= dy * diff;
                } else {
                    pos[i - 1][0] += dx * diff * 0.25;
                    pos[i - 1][1] += dy * diff * 0.25;
                    pos[i][0] -= dx * diff * 0.75;
                    pos[i][1] -= dy * diff * 0.75;
                }
            }
        }
    }

    pos.iter()
        .enumerate()
        .map(|(i, p)| [p[0], p[1] - i as f32 * seg_len])
        .collect()
}

/// アイドル呼吸・まばたき込みの実効 `VtuberBlendShapes` を評価する
pub fn evaluate_vtuber_shapes(cfg: &VtuberMeshConfig, time_sec: f32) -> VtuberBlendShapes {
    let mut s = cfg.blend_shapes;
    if cfg.auto_idle_animate {
        // 3.6 秒周期の自然な自動まばたき
        let phase = time_sec.rem_euclid(3.6);
        let auto_blink = if phase > 3.42 {
            let u = ((phase - 3.42) / 0.18).clamp(0.0, 1.0);
            (u * std::f32::consts::PI).sin()
        } else {
            0.0
        };
        s.blink_l = s.blink_l.max(auto_blink).clamp(0.0, 1.0);
        s.blink_r = s.blink_r.max(auto_blink).clamp(0.0, 1.0);
        // 微細な呼吸頭揺れ
        s.head_pitch_deg += (time_sec * 1.4).sin() * 1.4;
        s.head_roll_deg += (time_sec * 0.9).cos() * 1.2;
    }
    s.blink_l = s.blink_l.clamp(0.0, 1.0);
    s.blink_r = s.blink_r.clamp(0.0, 1.0);
    s.mouth_open = s.mouth_open.clamp(0.0, 1.0);
    s.mouth_smile = s.mouth_smile.clamp(0.0, 1.0);
    s.brow_y = s.brow_y.clamp(-1.0, 1.0);
    s.head_yaw_deg = s.head_yaw_deg.clamp(-35.0, 35.0);
    s.head_pitch_deg = s.head_pitch_deg.clamp(-25.0, 25.0);
    s.head_roll_deg = s.head_roll_deg.clamp(-25.0, 25.0);
    s.eye_look_x = s.eye_look_x.clamp(-1.0, 1.0);
    s.eye_look_y = s.eye_look_y.clamp(-1.0, 1.0);
    s
}

/// 2D 三角グリッドメッシュへブレンドシェイプ変形（頭部擬似 3D ヨー/ピッチ/ロール＋目パチ＋口開き＋Verlet 髪揺れ）を適用する
pub fn deform_vtuber_mesh(
    cfg: &VtuberMeshConfig,
    shapes: &VtuberBlendShapes,
    time_sec: f32,
) -> (Vec<Mesh2DVertex>, Vec<Mesh2DTriangle>) {
    let (mut verts, tris) = build_vtuber_grid_mesh(cfg.grid_cols, cfg.grid_rows);
    let hair_l = simulate_verlet_hair_chain(
        &cfg.physics,
        shapes.head_yaw_deg,
        shapes.head_roll_deg,
        time_sec,
        -1.0,
    );
    let hair_r = simulate_verlet_hair_chain(
        &cfg.physics,
        shapes.head_yaw_deg,
        shapes.head_roll_deg,
        time_sec,
        1.0,
    );

    let yaw = shapes.head_yaw_deg / 35.0;
    let pitch = shapes.head_pitch_deg / 25.0;
    let roll = shapes.head_roll_deg.to_radians();
    let (cos_r, sin_r) = (roll.cos(), roll.sin());

    for v in &mut verts {
        let u = v.uv[0];
        let w = v.uv[1];
        let mut x = u;
        let mut y = w;

        // 1. 擬似 3D 顔立体パース変形（頭部が上部 0.08..0.78 領域）
        let head_mask = ((0.82 - w) / 0.25).clamp(0.0, 1.0);
        let dx_c = u - 0.5;
        let dy_c = w - 0.44;
        // 顔中央ほど大きく横移動し、左右端は奥へ圧縮（Live2D 擬似 3D 回転の要）
        let bulge = (1.0 - (dx_c * dx_c * 3.2 + dy_c * dy_c * 2.5)).clamp(0.0, 1.0);
        x += yaw * (0.045 + 0.07 * bulge) * head_mask;
        y += pitch * (0.035 + 0.055 * bulge) * head_mask;

        // 2. 左目・右目のまばたき縦圧縮
        let dist_eye_l = ((u - 0.38) / 0.11).hypot((w - 0.43) / 0.08);
        if dist_eye_l < 1.0 {
            let k = (1.0 - dist_eye_l) * shapes.blink_l;
            y += (0.43 - w) * k * 0.65;
        }
        let dist_eye_r = ((u - 0.62) / 0.11).hypot((w - 0.43) / 0.08);
        if dist_eye_r < 1.0 {
            let k = (1.0 - dist_eye_r) * shapes.blink_r;
            y += (0.43 - w) * k * 0.65;
        }

        // 3. 口領域の開閉＆笑顔スマイル変形
        let dist_mouth = ((u - 0.5) / 0.12).hypot((w - 0.60) / 0.09);
        if dist_mouth < 1.0 {
            let k = 1.0 - dist_mouth;
            if w > 0.60 {
                y += k * shapes.mouth_open * 0.055;
            }
            // 口角引き上げ
            y -= k * shapes.mouth_smile * (u - 0.5).abs() * 0.14;
        }

        // 4. 左右サイド髪・ロングヘアへの Verlet 物理揺れ反映
        let side_hair = ((u - 0.5).abs() - 0.18).max(0.0) / 0.32;
        if side_hair > 0.0 && w > 0.22 {
            let chain = if u < 0.5 { &hair_l } else { &hair_r };
            let idx_f =
                ((w - 0.22) / 0.78).clamp(0.0, 1.0) * (chain.len().saturating_sub(1) as f32);
            let i0 = (idx_f.floor() as usize).min(chain.len() - 1);
            let i1 = (i0 + 1).min(chain.len() - 1);
            let frac = idx_f - i0 as f32;
            let ox = chain[i0][0] + (chain[i1][0] - chain[i0][0]) * frac;
            let oy = chain[i0][1] + (chain[i1][1] - chain[i0][1]) * frac;
            let weight = side_hair.clamp(0.0, 1.0);
            x += ox * weight * 1.4;
            y += oy * weight * 1.1;
        }

        // 5. 頭部ロール回転（首の付け根 (0.5, 0.72) を支点）
        let rx = x - 0.5;
        let ry = y - 0.72;
        let rot_x = 0.5 + (rx * cos_r - ry * sin_r) * head_mask + rx * (1.0 - head_mask);
        let rot_y = 0.72 + (rx * sin_r + ry * cos_r) * head_mask + ry * (1.0 - head_mask);

        v.pos = [rot_x.clamp(0.0, 1.0), rot_y.clamp(0.0, 1.0)];
    }

    (verts, tris)
}

// ============================================================================
// 3. VMC プロトコル (OSC-over-UDP) 顔トラッキング変換 ＆ 受信機 ＆ キーフレームベイク
// ============================================================================

/// VMC プロトコル OSC メッセージ 1 件を `VtuberBlendShapes` へ反映する（変更ありで `true`）
pub fn apply_vmc_osc_message_to_blendshapes(
    msg: &OscMessage,
    shapes: &mut VtuberBlendShapes,
) -> bool {
    match msg.address.as_str() {
        "/VMC/Ext/Blend/Val" => {
            if msg.args.len() >= 2
                && let (Some(key), Some(val)) = (msg.args[0].as_str(), msg.args[1].as_f32())
            {
                let v = val.clamp(0.0, 1.0);
                match key.to_ascii_lowercase().as_str() {
                    "blink_l" | "eyeblinkleft" => shapes.blink_l = v,
                    "blink_r" | "eyeblinkright" => shapes.blink_r = v,
                    "blink" => {
                        shapes.blink_l = v;
                        shapes.blink_r = v;
                    }
                    "a" | "aa" | "jawopen" | "mouth_open" => shapes.mouth_open = v,
                    "joy" | "fun" | "smile" | "mouthsmile" => shapes.mouth_smile = v,
                    "sorrow" => shapes.brow_y = -v,
                    "surprised" => shapes.brow_y = v,
                    _ => return false,
                }
                return true;
            }
            false
        }
        "/VMC/Ext/Bone/Pos" => {
            // /VMC/Ext/Bone/Pos name px py pz qx qy qz qw
            if msg.args.len() >= 8
                && let Some(bone) = msg.args[0].as_str()
                && bone.eq_ignore_ascii_case("Head")
                && let (Some(qx), Some(qy), Some(qz), Some(qw)) = (
                    msg.args[4].as_f32(),
                    msg.args[5].as_f32(),
                    msg.args[6].as_f32(),
                    msg.args[7].as_f32(),
                )
            {
                // クォータニオン -> オイラー角 (Yaw/Pitch/Roll in degrees)
                let sinp = (2.0 * (qw * qx - qz * qy)).clamp(-1.0, 1.0);
                let pitch = sinp.asin().to_degrees();
                let siny_cosp = 2.0 * (qw * qy + qx * qz);
                let cosy_cosp = 1.0 - 2.0 * (qx * qx + qy * qy);
                let yaw = siny_cosp.atan2(cosy_cosp).to_degrees();
                let sinr_cosp = 2.0 * (qw * qz + qx * qy);
                let cosr_cosp = 1.0 - 2.0 * (qx * qx + qz * qz);
                let roll = sinr_cosp.atan2(cosr_cosp).to_degrees();

                shapes.head_yaw_deg = yaw.clamp(-35.0, 35.0);
                shapes.head_pitch_deg = pitch.clamp(-25.0, 25.0);
                shapes.head_roll_deg = roll.clamp(-25.0, 25.0);
                return true;
            }
            false
        }
        _ => false,
    }
}

/// VMC 録画トラックの 1 サンプル
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VmcTrackFrame {
    pub time: RationalTime,
    pub shapes: VtuberBlendShapes,
}

/// VMC 顔トラッキング録画列を 1-Undo キーフレーム（`vt_yaw`, `vt_pitch`, `vt_roll`, `vt_mouth`, `vt_blink_l`, `vt_blink_r`）へ一括ベイクする
pub fn build_vtuber_vmc_bake_command(
    item_id: ItemId,
    item_start: RationalTime,
    frames: &[VmcTrackFrame],
) -> Option<EditCommand> {
    if frames.is_empty() {
        return None;
    }
    let mut cmds = Vec::with_capacity(frames.len() * 6);
    for f in frames {
        let local_t = (f.time - item_start).max(RationalTime::ZERO);
        let pairs: [(&str, f32); 6] = [
            ("vt_yaw", f.shapes.head_yaw_deg),
            ("vt_pitch", f.shapes.head_pitch_deg),
            ("vt_roll", f.shapes.head_roll_deg),
            ("vt_mouth", f.shapes.mouth_open),
            ("vt_blink_l", f.shapes.blink_l),
            ("vt_blink_r", f.shapes.blink_r),
        ];
        for (prop, val) in pairs {
            cmds.push(EditCommand::SetKeyframe {
                item_id,
                property: prop.to_string(),
                keyframe: Keyframe {
                    time: local_t,
                    value: val,
                    interpolation: Interpolation::Linear,
                },
            });
        }
    }
    Some(EditCommand::Batch {
        label: format!("bake_vtuber_vmc_{}f", frames.len()),
        commands: cmds,
    })
}

/// ノンブロッキング ローカル UDP VMC レシーバ（`127.0.0.1:<port>`）
pub struct VmcUdpReceiver {
    socket: UdpSocket,
    pub bound_port: u16,
    pub packets_received: u64,
}

impl VmcUdpReceiver {
    /// `port == 0` で OS 割り当て空きポートにバインド可能（単体テスト・競合回避向け）
    pub fn bind_localhost(port: u16) -> Result<Self, String> {
        let sock = UdpSocket::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        let bound_port = sock.local_addr().map_err(|e| e.to_string())?.port();
        Ok(Self {
            socket: sock,
            bound_port,
            packets_received: 0,
        })
    }

    /// 受信キューに届いている全 VMC OSC パケットを非ブロッキングでドレインし `shapes` を更新する
    pub fn poll_into(&mut self, shapes: &mut VtuberBlendShapes) -> usize {
        let mut buf = [0u8; 4096];
        let mut applied = 0usize;
        for _ in 0..64 {
            match self.socket.recv_from(&mut buf) {
                Ok((len, _addr)) => {
                    self.packets_received += 1;
                    for msg in decode_osc_packet(&buf[..len]) {
                        if apply_vmc_osc_message_to_blendshapes(&msg, shapes) {
                            applied += 1;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_codec_and_vmc_udp_receiver_update_blendshapes_over_loopback() {
        let mut rx = VmcUdpReceiver::bind_localhost(0).unwrap();
        let tx = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let target = ("127.0.0.1", rx.bound_port);

        let pkt1 = encode_osc_message(
            "/VMC/Ext/Blend/Val",
            &[OscArg::String("A".into()), OscArg::Float(0.82)],
        );
        let pkt2 = encode_osc_message(
            "/VMC/Ext/Blend/Val",
            &[OscArg::String("Blink_L".into()), OscArg::Float(0.95)],
        );
        // 20 deg yaw quaternion around Y: qy = sin(10deg), qw = cos(10deg)
        let half = 10.0_f32.to_radians();
        let pkt3 = encode_osc_message(
            "/VMC/Ext/Bone/Pos",
            &[
                OscArg::String("Head".into()),
                OscArg::Float(0.0),
                OscArg::Float(0.0),
                OscArg::Float(0.0),
                OscArg::Float(0.0),
                OscArg::Float(half.sin()),
                OscArg::Float(0.0),
                OscArg::Float(half.cos()),
            ],
        );
        tx.send_to(&pkt1, target).unwrap();
        tx.send_to(&pkt2, target).unwrap();
        tx.send_to(&pkt3, target).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));

        let mut shapes = VtuberBlendShapes::default();
        let applied = rx.poll_into(&mut shapes);
        assert!(applied >= 3, "expected >=3 VMC messages, got {applied}");
        assert!((shapes.mouth_open - 0.82).abs() < 1e-3);
        assert!((shapes.blink_l - 0.95).abs() < 1e-3);
        assert!((shapes.head_yaw_deg - 20.0).abs() < 1.0);
    }

    #[test]
    fn verlet_hair_chain_and_mesh_deform_are_deterministic_and_finite() {
        let cfg = VtuberMeshConfig::default();
        let shapes = VtuberBlendShapes {
            head_yaw_deg: 24.0,
            mouth_open: 0.7,
            blink_l: 0.9,
            ..VtuberBlendShapes::default()
        };
        let (v1, t1) = deform_vtuber_mesh(&cfg, &shapes, 1.25);
        let (v2, t2) = deform_vtuber_mesh(&cfg, &shapes, 1.25);
        assert_eq!(v1, v2);
        assert_eq!(t1, t2);
        assert!(!t1.is_empty());
        assert!(
            v1.iter()
                .all(|v| v.pos[0].is_finite() && v.pos[1].is_finite())
        );
    }
}
