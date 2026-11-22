//! v3.0 #30: 3D メッシュ / glTF 2.0（`.gltf` / `.glb`）シーン合成 ＆ 3D 立体押し出しテキスト
//! Pure Rust・ゼロ `unsafe`・外部依存ゼロで標準 glTF 2.0 / GLB バイナリ解析とプロシージャル 3D メッシュ生成を行う。

use crate::encoding::base64_decode;
use crate::project::RgbaColor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::f32::consts::PI;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mesh3DPreset {
    #[default]
    Cube,
    Sphere,
    Torus,
    ExtrudedText,
    GltfFile,
}

impl Mesh3DPreset {
    pub const ALL: &'static [Self] = &[
        Self::Cube,
        Self::Sphere,
        Self::Torus,
        Self::ExtrudedText,
        Self::GltfFile,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Cube => "🧊 3D キューブ (PBR)",
            Self::Sphere => "🔮 3D 球体 (PBR)",
            Self::Torus => "🍩 3D トーラス (PBR)",
            Self::ExtrudedText => "🔤 3D 立体押し出しテキスト",
            Self::GltfFile => "📦 glTF 2.0 / GLB モデル",
        }
    }
}

/// タイムライン上の 3D メッシュアイテム設定（`ItemKind::Mesh3D`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh3DConfig {
    #[serde(default)]
    pub preset: Mesh3DPreset,
    /// `GltfFile` 時に参照する `.gltf` または `.glb` ファイルパス
    #[serde(default)]
    pub gltf_path: Option<String>,
    /// `ExtrudedText` 時の立体化テキスト文字列
    #[serde(default = "default_3d_text")]
    pub text_content: String,
    /// 3D 押し出し奥行き (0.05..=2.0)
    #[serde(default = "default_extrude_depth")]
    pub extrude_depth: f32,
    /// PBR アルベド基本色
    #[serde(default = "default_mesh_color")]
    pub base_color: RgbaColor,
    /// PBR メタリック度 (0.0..=1.0)
    #[serde(default = "default_metallic")]
    pub metallic: f32,
    /// PBR ラフネス粗さ (0.05..=1.0)
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    /// 環境マップ反射強度 (0.0..=1.0)
    #[serde(default = "default_env_refl")]
    pub env_reflection: f32,
    /// ワイヤーフレーム重ね描画フラグ
    #[serde(default)]
    pub wireframe: bool,
    /// キャンバス上のレンダリング枠サイズ (px)
    #[serde(default = "default_mesh_w")]
    pub width: f32,
    #[serde(default = "default_mesh_h")]
    pub height: f32,
    /// 初期オイラー回転角 (度)
    #[serde(default = "default_rot_x")]
    pub rot_x_deg: f32,
    #[serde(default = "default_rot_y")]
    pub rot_y_deg: f32,
    #[serde(default)]
    pub rot_z_deg: f32,
    /// 時間経過に伴う自動 Y 軸スピン速度 (度/秒)
    #[serde(default = "default_spin")]
    pub auto_spin_deg_per_sec: f32,
}

fn default_3d_text() -> String {
    "3D STUDIO".into()
}
fn default_extrude_depth() -> f32 {
    0.32
}
fn default_mesh_color() -> RgbaColor {
    RgbaColor {
        r: 0.28,
        g: 0.72,
        b: 0.98,
        a: 1.0,
    }
}
fn default_metallic() -> f32 {
    0.45
}
fn default_roughness() -> f32 {
    0.28
}
fn default_env_refl() -> f32 {
    0.55
}
fn default_mesh_w() -> f32 {
    420.0
}
fn default_mesh_h() -> f32 {
    420.0
}
fn default_rot_x() -> f32 {
    -18.0
}
fn default_rot_y() -> f32 {
    24.0
}
fn default_spin() -> f32 {
    35.0
}

impl Default for Mesh3DConfig {
    fn default() -> Self {
        Self {
            preset: Mesh3DPreset::Cube,
            gltf_path: None,
            text_content: default_3d_text(),
            extrude_depth: default_extrude_depth(),
            base_color: default_mesh_color(),
            metallic: default_metallic(),
            roughness: default_roughness(),
            env_reflection: default_env_refl(),
            wireframe: false,
            width: default_mesh_w(),
            height: default_mesh_h(),
            rot_x_deg: default_rot_x(),
            rot_y_deg: default_rot_y(),
            rot_z_deg: 0.0,
            auto_spin_deg_per_sec: default_spin(),
        }
    }
}

/// 3D 頂点データ（位置・法線・UV）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vertex3D {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// 三角形インデックス付き 3D メッシュデータ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh3DData {
    pub name: String,
    pub vertices: Vec<Vertex3D>,
    pub indices: Vec<u32>,
}

impl Mesh3DData {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// モデルを原点中心・最大半径 1.0 へ正規化（サイズ差の大きい glTF でも画角に確実に収まる）
    pub fn normalize_bounds(&mut self) {
        if self.vertices.is_empty() {
            return;
        }
        let mut min_p = [f32::INFINITY; 3];
        let mut max_p = [f32::NEG_INFINITY; 3];
        for v in &self.vertices {
            min_p[0] = min_p[0].min(v.pos[0]);
            min_p[1] = min_p[1].min(v.pos[1]);
            min_p[2] = min_p[2].min(v.pos[2]);
            max_p[0] = max_p[0].max(v.pos[0]);
            max_p[1] = max_p[1].max(v.pos[1]);
            max_p[2] = max_p[2].max(v.pos[2]);
        }
        let center = [
            0.5 * (min_p[0] + max_p[0]),
            0.5 * (min_p[1] + max_p[1]),
            0.5 * (min_p[2] + max_p[2]),
        ];
        let extent = (max_p[0] - min_p[0])
            .max(max_p[1] - min_p[1])
            .max(max_p[2] - min_p[2])
            .max(1e-5);
        let inv = 1.6 / extent;
        for v in &mut self.vertices {
            v.pos = [
                (v.pos[0] - center[0]) * inv,
                (v.pos[1] - center[1]) * inv,
                (v.pos[2] - center[2]) * inv,
            ];
        }
    }

    /// 法線が未定義の glTF メッシュ向けに頂点法線を三角形外積から自動再計算する
    pub fn recompute_normals(&mut self) {
        let mut acc = vec![[0.0f32; 3]; self.vertices.len()];
        for tri in self.indices.as_chunks::<3>().0 {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if i0 >= self.vertices.len() || i1 >= self.vertices.len() || i2 >= self.vertices.len() {
                continue;
            }
            let p0 = self.vertices[i0].pos;
            let p1 = self.vertices[i1].pos;
            let p2 = self.vertices[i2].pos;
            let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            for idx in [i0, i1, i2] {
                acc[idx][0] += n[0];
                acc[idx][1] += n[1];
                acc[idx][2] += n[2];
            }
        }
        for (v, n) in self.vertices.iter_mut().zip(acc) {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len > 1e-6 {
                v.normal = [n[0] / len, n[1] / len, n[2] / len];
            } else {
                v.normal = [0.0, 1.0, 0.0];
            }
        }
    }
}

/// 単位キューブ（6 面・24 頂点・12 三角形・面ごとの正確な法線）
pub fn build_cube_mesh() -> Mesh3DData {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        // +Z Front
        (
            [0.0, 0.0, 1.0],
            [
                [-0.7, -0.7, 0.7],
                [0.7, -0.7, 0.7],
                [0.7, 0.7, 0.7],
                [-0.7, 0.7, 0.7],
            ],
        ),
        // -Z Back
        (
            [0.0, 0.0, -1.0],
            [
                [0.7, -0.7, -0.7],
                [-0.7, -0.7, -0.7],
                [-0.7, 0.7, -0.7],
                [0.7, 0.7, -0.7],
            ],
        ),
        // +X Right
        (
            [1.0, 0.0, 0.0],
            [
                [0.7, -0.7, 0.7],
                [0.7, -0.7, -0.7],
                [0.7, 0.7, -0.7],
                [0.7, 0.7, 0.7],
            ],
        ),
        // -X Left
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.7, -0.7, -0.7],
                [-0.7, -0.7, 0.7],
                [-0.7, 0.7, 0.7],
                [-0.7, 0.7, -0.7],
            ],
        ),
        // +Y Top
        (
            [0.0, 1.0, 0.0],
            [
                [-0.7, 0.7, 0.7],
                [0.7, 0.7, 0.7],
                [0.7, 0.7, -0.7],
                [-0.7, 0.7, -0.7],
            ],
        ),
        // -Y Bottom
        (
            [0.0, -1.0, 0.0],
            [
                [-0.7, -0.7, -0.7],
                [0.7, -0.7, -0.7],
                [0.7, -0.7, 0.7],
                [-0.7, -0.7, 0.7],
            ],
        ),
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (normal, quad) in faces {
        let base = vertices.len() as u32;
        for (pos, uv) in quad.into_iter().zip(uvs) {
            vertices.push(Vertex3D { pos, normal, uv });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh3DData {
        name: "Cube".into(),
        vertices,
        indices,
    }
}

/// UV 球体メッシュ（滑らかな法線付き）
pub fn build_sphere_mesh(lat_segs: u32, lon_segs: u32) -> Mesh3DData {
    let lat = lat_segs.clamp(6, 48);
    let lon = lon_segs.clamp(8, 64);
    let r = 0.85f32;
    let mut vertices = Vec::with_capacity(((lat + 1) * (lon + 1)) as usize);
    let mut indices = Vec::with_capacity((lat * lon * 6) as usize);

    for iy in 0..=lat {
        let v = iy as f32 / lat as f32;
        let phi = v * PI;
        for ix in 0..=lon {
            let u = ix as f32 / lon as f32;
            let theta = u * 2.0 * PI;
            let nx = phi.sin() * theta.cos();
            let ny = phi.cos();
            let nz = phi.sin() * theta.sin();
            vertices.push(Vertex3D {
                pos: [nx * r, ny * r, nz * r],
                normal: [nx, ny, nz],
                uv: [u, v],
            });
        }
    }
    let stride = lon + 1;
    for iy in 0..lat {
        for ix in 0..lon {
            let a = iy * stride + ix;
            let b = a + stride;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    Mesh3DData {
        name: "Sphere".into(),
        vertices,
        indices,
    }
}

/// トーラス（ドーナツ型）メッシュ
pub fn build_torus_mesh(major_segs: u32, minor_segs: u32) -> Mesh3DData {
    let maj = major_segs.clamp(12, 64);
    let min = minor_segs.clamp(8, 36);
    let big_r = 0.62f32;
    let small_r = 0.26f32;
    let mut vertices = Vec::with_capacity(((maj + 1) * (min + 1)) as usize);
    let mut indices = Vec::with_capacity((maj * min * 6) as usize);

    for i in 0..=maj {
        let u = i as f32 / maj as f32;
        let a = u * 2.0 * PI;
        let (ca, sa) = (a.cos(), a.sin());
        for j in 0..=min {
            let v = j as f32 / min as f32;
            let b = v * 2.0 * PI;
            let (cb, sb) = (b.cos(), b.sin());
            let nx = cb * ca;
            let ny = sb;
            let nz = cb * sa;
            let rr = big_r + small_r * cb;
            vertices.push(Vertex3D {
                pos: [rr * ca, small_r * sb, rr * sa],
                normal: [nx, ny, nz],
                uv: [u, v],
            });
        }
    }
    let stride = min + 1;
    for i in 0..maj {
        for j in 0..min {
            let a = i * stride + j;
            let b = a + stride;
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    Mesh3DData {
        name: "Torus".into(),
        vertices,
        indices,
    }
}

/// 5x7 ドットグリフから 3D 押し出し立体テキストメッシュを構築する（文字ごとに前面・背面・側面を生成）
pub fn build_extruded_3d_text_mesh(text: &str, depth: f32) -> Mesh3DData {
    let clean: Vec<char> = text.trim().chars().take(12).collect();
    let chars = if clean.is_empty() {
        vec!['3', 'D']
    } else {
        clean
    };
    let half_d = (depth.clamp(0.05, 1.5)) * 0.5;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    let cell = 0.085f32;
    let char_adv = 6.2 * cell;
    let total_w = chars.len() as f32 * char_adv;
    let start_x = -total_w * 0.5;

    for (ci, ch) in chars.iter().enumerate() {
        let bitmap = glyph_5x7(*ch);
        let ox = start_x + ci as f32 * char_adv;
        for (row_idx, &row_bits) in bitmap.iter().enumerate() {
            for col in 0..5usize {
                if (row_bits >> (4 - col)) & 1 == 0 {
                    continue;
                }
                let x0 = ox + col as f32 * cell;
                let x1 = x0 + cell * 0.94;
                let y1 = (3.5 - row_idx as f32) * cell;
                let y0 = y1 - cell * 0.94;
                push_box(
                    &mut vertices,
                    &mut indices,
                    [x0, y0, -half_d],
                    [x1, y1, half_d],
                );
            }
        }
    }

    let mut mesh = Mesh3DData {
        name: format!("ExtrudedText({})", text.trim()),
        vertices,
        indices,
    };
    mesh.normalize_bounds();
    mesh
}

fn push_box(vertices: &mut Vec<Vertex3D>, indices: &mut Vec<u32>, min: [f32; 3], max: [f32; 3]) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 0.0, 1.0],
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        ),
        (
            [0.0, 0.0, -1.0],
            [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
        ),
        (
            [1.0, 0.0, 0.0],
            [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        ),
        (
            [0.0, 1.0, 0.0],
            [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]],
        ),
        (
            [0.0, -1.0, 0.0],
            [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        ),
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (normal, quad) in faces {
        let base = vertices.len() as u32;
        for (pos, uv) in quad.into_iter().zip(uvs) {
            vertices.push(Vertex3D { pos, normal, uv });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x0E, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        ' ' => [0x00; 7],
        // 非ASCII（日本語等）はスタイリッシュな立体フレームブロックで表現
        _ => [0x1F, 0x11, 0x15, 0x15, 0x15, 0x11, 0x1F],
    }
}

/// Pure-Rust glTF 2.0 (`.gltf` JSON または `.glb` バイナリコンテナ) パーサ
pub fn parse_gltf_or_glb(bytes: &[u8], base_dir: Option<&Path>) -> Result<Mesh3DData, String> {
    let (json_val, bin_chunk): (Value, Option<Vec<u8>>) =
        if bytes.len() >= 12 && &bytes[0..4] == b"glTF" {
            let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            if version != 2 {
                return Err(format!("unsupported glTF version: {version}"));
            }
            let mut cursor = 12usize;
            let mut json_bytes: Option<&[u8]> = None;
            let mut bin_bytes: Option<Vec<u8>> = None;
            while cursor + 8 <= bytes.len() {
                let chunk_len = u32::from_le_bytes([
                    bytes[cursor],
                    bytes[cursor + 1],
                    bytes[cursor + 2],
                    bytes[cursor + 3],
                ]) as usize;
                let chunk_type = &bytes[cursor + 4..cursor + 8];
                let data_start = cursor + 8;
                let data_end = data_start
                    .checked_add(chunk_len)
                    .filter(|&e| e <= bytes.len())
                    .ok_or_else(|| "truncated GLB chunk".to_string())?;
                if chunk_type == b"JSON" {
                    json_bytes = Some(&bytes[data_start..data_end]);
                } else if chunk_type == b"BIN\0" {
                    bin_bytes = Some(bytes[data_start..data_end].to_vec());
                }
                cursor = data_end;
            }
            let jb = json_bytes.ok_or_else(|| "GLB missing JSON chunk".to_string())?;
            let val: Value =
                serde_json::from_slice(jb).map_err(|e| format!("invalid GLB JSON: {e}"))?;
            (val, bin_bytes)
        } else {
            let val: Value =
                serde_json::from_slice(bytes).map_err(|e| format!("invalid glTF JSON: {e}"))?;
            (val, None)
        };

    // バッファ解決（GLB BIN / data URI Base64 / 外部 .bin）
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    if let Some(arr) = json_val.get("buffers").and_then(Value::as_array) {
        for (idx, b) in arr.iter().enumerate() {
            if let Some(uri) = b.get("uri").and_then(Value::as_str) {
                if let Some((_, b64)) = uri.split_once(";base64,") {
                    buffers.push(base64_decode(b64)?);
                } else if let Some(dir) = base_dir {
                    let safe_name = Path::new(uri)
                        .file_name()
                        .ok_or_else(|| "invalid buffer uri".to_string())?;
                    let data = std::fs::read(dir.join(safe_name))
                        .map_err(|e| format!("failed to read glTF buffer {uri}: {e}"))?;
                    buffers.push(data);
                } else {
                    return Err(format!(
                        "cannot resolve external glTF buffer without base_dir: {uri}"
                    ));
                }
            } else if idx == 0
                && let Some(bin) = bin_chunk.clone()
            {
                buffers.push(bin);
            } else {
                return Err(format!("missing buffer data for buffer[{idx}]"));
            }
        }
    }

    let buffer_views = json_val
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or_else(|| "glTF missing bufferViews".to_string())?;
    let accessors = json_val
        .get("accessors")
        .and_then(Value::as_array)
        .ok_or_else(|| "glTF missing accessors".to_string())?;
    let meshes = json_val
        .get("meshes")
        .and_then(Value::as_array)
        .ok_or_else(|| "glTF missing meshes".to_string())?;

    let slice_for_accessor = |acc_idx: usize| -> Result<(&[u8], usize, u64, usize), String> {
        let acc = accessors
            .get(acc_idx)
            .ok_or_else(|| format!("accessor {acc_idx} out of bounds"))?;
        let bv_idx =
            acc.get("bufferView")
                .and_then(Value::as_u64)
                .ok_or_else(|| "accessor missing bufferView".to_string())? as usize;
        let count = acc
            .get("count")
            .and_then(Value::as_u64)
            .ok_or_else(|| "accessor missing count".to_string())? as usize;
        let comp_type = acc
            .get("componentType")
            .and_then(Value::as_u64)
            .ok_or_else(|| "accessor missing componentType".to_string())?;
        let acc_off = acc.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let bv = buffer_views
            .get(bv_idx)
            .ok_or_else(|| format!("bufferView {bv_idx} out of bounds"))?;
        let buf_idx = bv.get("buffer").and_then(Value::as_u64).unwrap_or(0) as usize;
        let bv_off = bv.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let stride = bv.get("byteStride").and_then(Value::as_u64).unwrap_or(0) as usize;
        let buf = buffers
            .get(buf_idx)
            .ok_or_else(|| format!("buffer {buf_idx} out of bounds"))?;
        let start = bv_off
            .checked_add(acc_off)
            .filter(|&s| s <= buf.len())
            .ok_or_else(|| "accessor offset out of bounds".to_string())?;
        Ok((&buf[start..], count, comp_type, stride))
    };

    let mut all_vertices = Vec::new();
    let mut all_indices = Vec::new();
    let mesh_name = meshes
        .first()
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("glTF Mesh")
        .to_string();
    let mut had_normals = false;

    for mesh in meshes {
        let Some(prims) = mesh.get("primitives").and_then(Value::as_array) else {
            continue;
        };
        for prim in prims {
            let Some(attrs) = prim.get("attributes").and_then(Value::as_object) else {
                continue;
            };
            let Some(pos_idx) = attrs.get("POSITION").and_then(Value::as_u64) else {
                continue;
            };
            let (pos_bytes, v_count, pos_ct, pos_stride) = slice_for_accessor(pos_idx as usize)?;
            if pos_ct != 5126 {
                return Err("glTF POSITION must be f32 (5126)".into());
            }
            let p_step = if pos_stride > 0 { pos_stride } else { 12 };
            let base_v = all_vertices.len() as u32;

            let norm_info = attrs
                .get("NORMAL")
                .and_then(Value::as_u64)
                .and_then(|idx| slice_for_accessor(idx as usize).ok())
                .filter(|(_, cnt, ct, _)| *ct == 5126 && *cnt >= v_count);
            if norm_info.is_some() {
                had_normals = true;
            }

            for i in 0..v_count {
                let off = i * p_step;
                if off + 12 > pos_bytes.len() {
                    break;
                }
                let pos = [
                    f32::from_le_bytes([
                        pos_bytes[off],
                        pos_bytes[off + 1],
                        pos_bytes[off + 2],
                        pos_bytes[off + 3],
                    ]),
                    f32::from_le_bytes([
                        pos_bytes[off + 4],
                        pos_bytes[off + 5],
                        pos_bytes[off + 6],
                        pos_bytes[off + 7],
                    ]),
                    f32::from_le_bytes([
                        pos_bytes[off + 8],
                        pos_bytes[off + 9],
                        pos_bytes[off + 10],
                        pos_bytes[off + 11],
                    ]),
                ];
                let normal = if let Some((nb, _, _, ns)) = norm_info {
                    let n_step = if ns > 0 { ns } else { 12 };
                    let noff = i * n_step;
                    if noff + 12 <= nb.len() {
                        [
                            f32::from_le_bytes([
                                nb[noff],
                                nb[noff + 1],
                                nb[noff + 2],
                                nb[noff + 3],
                            ]),
                            f32::from_le_bytes([
                                nb[noff + 4],
                                nb[noff + 5],
                                nb[noff + 6],
                                nb[noff + 7],
                            ]),
                            f32::from_le_bytes([
                                nb[noff + 8],
                                nb[noff + 9],
                                nb[noff + 10],
                                nb[noff + 11],
                            ]),
                        ]
                    } else {
                        [0.0, 1.0, 0.0]
                    }
                } else {
                    [0.0, 1.0, 0.0]
                };
                all_vertices.push(Vertex3D {
                    pos,
                    normal,
                    uv: [0.0, 0.0],
                });
            }

            if let Some(idx_acc) = prim.get("indices").and_then(Value::as_u64) {
                let (ib, icnt, ict, _) = slice_for_accessor(idx_acc as usize)?;
                for i in 0..icnt {
                    let v_id = match ict {
                        5121 if i < ib.len() => u32::from(ib[i]),
                        5123 if i * 2 + 2 <= ib.len() => {
                            u32::from(u16::from_le_bytes([ib[i * 2], ib[i * 2 + 1]]))
                        }
                        5125 if i * 4 + 4 <= ib.len() => u32::from_le_bytes([
                            ib[i * 4],
                            ib[i * 4 + 1],
                            ib[i * 4 + 2],
                            ib[i * 4 + 3],
                        ]),
                        _ => break,
                    };
                    all_indices.push(base_v + v_id);
                }
            } else {
                for i in 0..(v_count as u32) {
                    all_indices.push(base_v + i);
                }
            }
        }
    }

    if all_vertices.is_empty() || all_indices.len() < 3 {
        return Err("glTF contained no renderable triangles".into());
    }
    let mut mesh = Mesh3DData {
        name: mesh_name,
        vertices: all_vertices,
        indices: all_indices,
    };
    if !had_normals {
        mesh.recompute_normals();
    }
    mesh.normalize_bounds();
    Ok(mesh)
}

/// 任意の `Mesh3DData` から標準準拠のバイナリ glTF 2.0 (`.glb`) バイト列を生成する（サンプル出力・検証用）
pub fn encode_mesh_to_glb(mesh: &Mesh3DData) -> Vec<u8> {
    let mut bin = Vec::new();
    for v in &mesh.vertices {
        for f in v.pos {
            bin.extend_from_slice(&f.to_le_bytes());
        }
    }
    let pos_len = bin.len();
    for v in &mesh.vertices {
        for f in v.normal {
            bin.extend_from_slice(&f.to_le_bytes());
        }
    }
    let norm_len = bin.len() - pos_len;
    let idx_offset = bin.len();
    for &idx in &mesh.indices {
        bin.extend_from_slice(&idx.to_le_bytes());
    }
    let idx_len = bin.len() - idx_offset;
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    let json_doc = serde_json::json!({
        "asset": { "version": "2.0", "generator": "ve-core v3.0" },
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": pos_len },
            { "buffer": 0, "byteOffset": pos_len, "byteLength": norm_len },
            { "buffer": 0, "byteOffset": idx_offset, "byteLength": idx_len }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": mesh.vertices.len(), "type": "VEC3" },
            { "bufferView": 1, "componentType": 5126, "count": mesh.vertices.len(), "type": "VEC3" },
            { "bufferView": 2, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }
        ],
        "meshes": [{
            "name": mesh.name,
            "primitives": [{
                "attributes": { "POSITION": 0, "NORMAL": 1 },
                "indices": 2
            }]
        }]
    });
    let mut json_bytes = serde_json::to_vec(&json_doc).unwrap_or_default();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }

    let total_len = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// `Mesh3DConfig` からレンダリング対象の `Mesh3DData` を解決する（ファイル読込失敗時は安全にキューブへフォールバック）
pub fn resolve_mesh_data(cfg: &Mesh3DConfig, base_dir: Option<&Path>) -> Mesh3DData {
    match cfg.preset {
        Mesh3DPreset::Cube => build_cube_mesh(),
        Mesh3DPreset::Sphere => build_sphere_mesh(16, 24),
        Mesh3DPreset::Torus => build_torus_mesh(28, 14),
        Mesh3DPreset::ExtrudedText => {
            build_extruded_3d_text_mesh(&cfg.text_content, cfg.extrude_depth)
        }
        Mesh3DPreset::GltfFile => {
            if let Some(p) = cfg.gltf_path.as_deref()
                && !p.trim().is_empty()
            {
                let path = Path::new(p);
                let resolved = if path.is_absolute() {
                    path.to_path_buf()
                } else if let Some(dir) = base_dir {
                    dir.join(path)
                } else {
                    path.to_path_buf()
                };
                if let Ok(bytes) = std::fs::read(&resolved)
                    && let Ok(parsed) = parse_gltf_or_glb(&bytes, resolved.parent())
                {
                    return parsed;
                }
            }
            build_cube_mesh()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glb_binary_roundtrip_and_procedural_3d_meshes_work() {
        let cube = build_cube_mesh();
        assert_eq!(cube.triangle_count(), 12);
        let glb = encode_mesh_to_glb(&cube);
        let parsed = parse_gltf_or_glb(&glb, None).expect("parse GLB");
        assert_eq!(parsed.name, "Cube");
        assert_eq!(parsed.triangle_count(), 12);
        assert_eq!(parsed.vertices.len(), cube.vertices.len());

        let sphere = build_sphere_mesh(12, 18);
        assert!(sphere.triangle_count() >= 200);

        let torus = build_torus_mesh(20, 10);
        assert!(torus.triangle_count() >= 200);

        let text3d = build_extruded_3d_text_mesh("V3.0!", 0.35);
        assert!(text3d.triangle_count() >= 100);
    }

    #[test]
    fn parses_gltf_json_with_embedded_base64_data_uri() {
        let mut raw = Vec::new();
        for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            raw.extend_from_slice(&f.to_le_bytes());
        }
        let b64 = crate::encoding::base64_encode(&raw);
        let gltf = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"uri": "data:application/octet-stream;base64,{b64}", "byteLength": 36}}],
                "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
                "accessors": [{{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}}],
                "meshes": [{{"name": "SingleTri", "primitives": [{{"attributes": {{"POSITION": 0}}}}]}}]
            }}"#
        );
        let parsed = parse_gltf_or_glb(gltf.as_bytes(), None).expect("parse data-URI glTF");
        assert_eq!(parsed.name, "SingleTri");
        assert_eq!(parsed.triangle_count(), 1);
    }
}
