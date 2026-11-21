//! v1.0 #20: 公式拡張エコシステム（WASM・WGSL・プリセット・テンプレート導入ブラウザ＋SHA-256 署名検証）
//!
//! Pure-Rust・ゼロ `unsafe`・ゼロ追加依存の FIPS 180-4 SHA-256 ハッシュ検証と、
//! サンドボックス権限明示付きの安全な拡張導入マネージャを提供する。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 拡張パッケージ種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    WasmPlugin,
    WgslShader,
    MotionTemplate,
    CubeLut,
    TextStylePreset,
}

impl ExtensionKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::WasmPlugin => "WASM プラグイン",
            Self::WgslShader => "WGSL カスタムシェーダー",
            Self::MotionTemplate => "モーショングラフィックス テンプレート",
            Self::CubeLut => ".cube 3D LUT",
            Self::TextStylePreset => "字幕スタイルプリセット",
        }
    }

    pub fn target_subdir(self) -> &'static str {
        match self {
            Self::WasmPlugin => "plugins",
            Self::WgslShader => "shaders",
            Self::MotionTemplate => "templates",
            Self::CubeLut => "luts",
            Self::TextStylePreset => "presets/text_styles",
        }
    }
}

/// サンドボックス権限の明示ラベル
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPermission {
    PixelTransformOnly,
    GpuFragmentPass,
    TimelineJsonOnly,
    NoFilesystemAccess,
    NoNetworkAccess,
    FuelBoundedExecution,
}

impl SandboxPermission {
    pub fn label(self) -> &'static str {
        match self {
            Self::PixelTransformOnly => "ピクセル変換のみ",
            Self::GpuFragmentPass => "GPU フラグメントパス (naga 静的検証済)",
            Self::TimelineJsonOnly => "タイムライン JSON のみ（実行コードなし）",
            Self::NoFilesystemAccess => "ファイルシステムアクセス不可",
            Self::NoNetworkAccess => "ネットワーク通信不可",
            Self::FuelBoundedExecution => "命令フューエル上限保護あり",
        }
    }
}

/// 公式拡張インデックスの 1 エントリ
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub license: String,
    pub kind: ExtensionKind,
    pub filename: String,
    pub description: String,
    /// ペイロードの期待 SHA-256（小文字64桁 hex）
    pub sha256: String,
    pub permissions: Vec<SandboxPermission>,
}

/// Pure-Rust・ゼロ `unsafe` FIPS 180-4 SHA-256 ダイジェスト計算（64文字小文字hex）
pub fn sha256_hex(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = input.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let o = i * 4;
            *word = u32::from_be_bytes([block[o], block[o + 1], block[o + 2], block[o + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut hex = String::with_capacity(64);
    for word in h {
        hex.push_str(&format!("{word:08x}"));
    }
    hex
}

/// 組み込み公式スターターカタログ（オフラインでも SHA-256 署名検証付きで即導入可能）
pub fn official_registry_catalog() -> Vec<(RegistryEntry, Vec<u8>)> {
    let wgsl_vignette =
        br#"// Official Verified Custom WGSL: Film Halation & Vignette (MIT OR Apache-2.0)
struct GpuEffectUniform {
    p0: f32, p1: f32, p2: f32, p3: f32,
    p4: f32, p5: f32, p6: f32, p7: f32,
    resolution: vec2<f32>,
    time_sec: f32,
    _pad: f32,
};
@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> u: GpuEffectUniform;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let col = textureSample(src_tex, src_samp, uv);
    let d = distance(uv, vec2<f32>(0.5, 0.5));
    let strength = clamp(u.p0 * 40.0, 0.0, 0.95);
    let vig = 1.0 - smoothstep(0.28, 0.82, d) * strength;
    return vec4<f32>(col.rgb * vig, col.a);
}
"#
        .to_vec();

    let preset_cinema_sub = serde_json::to_vec_pretty(&crate::preset::TextStylePreset {
        name: "公式シネマ字幕".into(),
        font_family: "Noto Sans JP".into(),
        font_size: 44.0,
        color: crate::project::RgbaColor::WHITE,
        text_style: crate::project::TextStyle {
            outline_width: 2.0,
            outline_color: crate::project::RgbaColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.85,
            },
            shadow_offset: [0.0, 4.0],
            shadow_blur: 6.0,
            shadow_color: crate::project::RgbaColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.75,
            },
            box_enabled: false,
            ..crate::project::TextStyle::default()
        },
    })
    .unwrap_or_default();

    let tpl_news = serde_json::to_vec_pretty(&crate::template::builtin_motion_templates()[0])
        .unwrap_or_default();

    vec![
        (
            RegistryEntry {
                id: "official_wgsl_film_vignette".into(),
                name: "Film Vignette WGSL シェーダー".into(),
                version: "1.0.0".into(),
                author: "video-editing-app Official".into(),
                license: "MIT OR Apache-2.0".into(),
                kind: ExtensionKind::WgslShader,
                filename: "official_film_vignette.wgsl".into(),
                description: "滑らかな周辺減光を加える naga 検証済みカスタム WGSL シェーダー"
                    .into(),
                sha256: sha256_hex(&wgsl_vignette),
                permissions: vec![
                    SandboxPermission::GpuFragmentPass,
                    SandboxPermission::NoFilesystemAccess,
                    SandboxPermission::NoNetworkAccess,
                ],
            },
            wgsl_vignette,
        ),
        (
            RegistryEntry {
                id: "official_preset_cinema_sub".into(),
                name: "公式シネマ字幕スタイル".into(),
                version: "1.0.0".into(),
                author: "video-editing-app Official".into(),
                license: "MIT OR Apache-2.0".into(),
                kind: ExtensionKind::TextStylePreset,
                filename: "official_cinema_subtitle.json".into(),
                description: "映画・Vlog 向けの上品な細縁＋ドロップシャドウ字幕プリセット".into(),
                sha256: sha256_hex(&preset_cinema_sub),
                permissions: vec![
                    SandboxPermission::TimelineJsonOnly,
                    SandboxPermission::NoFilesystemAccess,
                    SandboxPermission::NoNetworkAccess,
                ],
            },
            preset_cinema_sub,
        ),
        (
            RegistryEntry {
                id: "official_tpl_lower_third".into(),
                name: "ニュース風ローワーサード (.mogrt相当)".into(),
                version: "1.0.0".into(),
                author: "video-editing-app Official".into(),
                license: "MIT OR Apache-2.0".into(),
                kind: ExtensionKind::MotionTemplate,
                filename: "official_lower_third_news.json".into(),
                description: "レスポンシブ時間保護付きの公式見出しテンプレート".into(),
                sha256: sha256_hex(&tpl_news),
                permissions: vec![
                    SandboxPermission::TimelineJsonOnly,
                    SandboxPermission::NoFilesystemAccess,
                    SandboxPermission::NoNetworkAccess,
                ],
            },
            tpl_news,
        ),
    ]
}

/// SHA-256 ダイジェストを厳密検証し、安全なファイル名でプロジェクト配下へ配置する
pub fn verify_and_install_extension(
    entry: &RegistryEntry,
    payload: &[u8],
    project_root: &Path,
) -> Result<PathBuf, String> {
    let actual = sha256_hex(payload);
    if !actual.eq_ignore_ascii_case(entry.sha256.trim()) {
        return Err(format!(
            "SHA-256 署名検証失敗 ({}): expected {}, got {actual}",
            entry.id, entry.sha256
        ));
    }

    // ファイル名のパストラバーサル拒否
    if entry.filename.is_empty()
        || entry.filename.contains('/')
        || entry.filename.contains('\\')
        || entry.filename.contains("..")
    {
        return Err(format!("不正なファイル名です: {}", entry.filename));
    }

    let dest_dir = project_root.join(entry.kind.target_subdir());
    fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let dest_path = dest_dir.join(&entry.filename);
    fs::write(&dest_path, payload).map_err(|e| e.to_string())?;
    Ok(dest_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sha256_matches_nist_fips_180_4_test_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn registry_verifies_sha256_and_rejects_tampered_payload() {
        let tmp = tempdir().unwrap();
        let catalog = official_registry_catalog();
        assert!(catalog.len() >= 3);

        let (entry, payload) = &catalog[0];
        let installed = verify_and_install_extension(entry, payload, tmp.path()).unwrap();
        assert!(installed.exists());

        // 1 バイト改ざんされたペイロードは SHA-256 検証で必ず拒否される
        let mut tampered = payload.clone();
        tampered[0] ^= 0xFF;
        assert!(verify_and_install_extension(entry, &tampered, tmp.path()).is_err());
    }
}
