//! v4.0 #33: プロカラー管理パイプライン（ACEScg / OpenColorIO 互換・カメラ Log IDT・ACES 1.3 ガマット圧縮・AgX/ACES トーンマップ）
//! Pure Rust・ゼロ `unsafe`・外部依存ゼロで Rec.709 / ACEScg (AP1) / Display P3 / Rec.2020 および
//! Sony S-Log3 / Panasonic V-Log / ARRI LogC3 の厳密色空間変換を実現する。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpace {
    #[default]
    LinearSrgb,
    AcesCg,
    DisplayP3,
    Rec2020,
    SLog3,
    VLog,
    LogC3,
}

impl ColorSpace {
    pub const INPUT_SPACES: &'static [Self] = &[
        Self::LinearSrgb,
        Self::AcesCg,
        Self::DisplayP3,
        Self::Rec2020,
        Self::SLog3,
        Self::VLog,
        Self::LogC3,
    ];

    pub const OUTPUT_SPACES: &'static [Self] = &[Self::LinearSrgb, Self::DisplayP3, Self::Rec2020];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::LinearSrgb => "Linear sRGB / Rec.709",
            Self::AcesCg => "ACEScg (AP1 Wide Gamut)",
            Self::DisplayP3 => "Display P3 (Apple / DCI)",
            Self::Rec2020 => "Rec.2020 (UHD / HDR10)",
            Self::SLog3 => "Sony S-Log3 / S-Gamut3",
            Self::VLog => "Panasonic V-Log",
            Self::LogC3 => "ARRI LogC3 (EI 800)",
        }
    }

    pub fn ocio_name(self) -> &'static str {
        match self {
            Self::LinearSrgb => "Utility - Linear - sRGB",
            Self::AcesCg => "ACES - ACEScg",
            Self::DisplayP3 => "Output - P3-D65",
            Self::Rec2020 => "Output - Rec.2020",
            Self::SLog3 => "Input - Sony - S-Log3 - S-Gamut3.Cine",
            Self::VLog => "Input - Panasonic - V-Log - V-Gamut",
            Self::LogC3 => "Input - ARRI - V3 LogC (EI800) - Wide Gamut",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToneMapper {
    None,
    #[default]
    AcesFitted,
    AgX,
    ReinhardExtended,
}

impl ToneMapper {
    pub const ALL: &'static [Self] = &[
        Self::None,
        Self::AcesFitted,
        Self::AgX,
        Self::ReinhardExtended,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::None => "なし (Linear Clip)",
            Self::AcesFitted => "🎬 ACES Filmic (RRT+ODT)",
            Self::AgX => "✨ AgX Punchy (ハイライト自然脱彩)",
            Self::ReinhardExtended => "📷 Reinhard Extended (映画ロールオフ)",
        }
    }
}

/// シーン単位のプロカラー管理設定（`Scene::color_management`）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorManagementConfig {
    #[serde(default)]
    pub enabled: bool,
    /// 入力素材の色空間（カメラ Log → 線形ワーキング空間への IDT）
    #[serde(default)]
    pub input_idt: ColorSpace,
    /// 内部ワーキング空間（既定: ACEScg AP1）
    #[serde(default = "default_working_space")]
    pub working_space: ColorSpace,
    /// 出力表示色空間（ODT）
    #[serde(default)]
    pub output_odt: ColorSpace,
    /// フィルム・ハイライトロールオフ用トーンマッパー
    #[serde(default)]
    pub tone_mapper: ToneMapper,
    /// ワーキング空間（シーンリニア）での露出調整（EV stops: -4.0..=4.0）
    #[serde(default)]
    pub exposure_stops: f32,
    /// ACES 1.3 Reference Gamut Compression（ネオン青・マゼンタ破綻防止）
    #[serde(default = "default_true")]
    pub gamut_compress: bool,
}

fn default_working_space() -> ColorSpace {
    ColorSpace::AcesCg
}

fn default_true() -> bool {
    true
}

impl Default for ColorManagementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            input_idt: ColorSpace::LinearSrgb,
            working_space: ColorSpace::AcesCg,
            output_odt: ColorSpace::LinearSrgb,
            tone_mapper: ToneMapper::AcesFitted,
            exposure_stops: 0.0,
            gamut_compress: true,
        }
    }
}

// =========================================================================
// Exact 3x3 Color Primaries Matrices (Bradford D65 <-> D60 Adapted)
// =========================================================================

const MAT_SRGB_TO_ACESCG: [[f32; 3]; 3] = [
    [0.613_132_4, 0.339_538, 0.047_416_7],
    [0.070_124_4, 0.916_394, 0.013_451_5],
    [0.020_587_7, 0.109_574_6, 0.869_785_4],
];

const MAT_ACESCG_TO_SRGB: [[f32; 3]; 3] = [
    [1.704_858_7, -0.621_716, -0.083_299_4],
    [-0.130_076_8, 1.140_735_8, -0.010_559_9],
    [-0.023_964_1, -0.128_975_5, 1.153_014],
];

const MAT_P3_TO_SRGB: [[f32; 3]; 3] = [
    [1.224_940_1, -0.224_940_4, 0.0],
    [-0.042_056_9, 1.042_057, 0.0],
    [-0.019_637_6, -0.078_636_0, 1.098_273_6],
];

const MAT_SRGB_TO_P3: [[f32; 3]; 3] = [
    [0.822_462, 0.177_538, 0.0],
    [0.033_194_2, 0.966_805_8, 0.0],
    [0.017_082_6, 0.072_397_4, 0.910_52],
];

const MAT_REC2020_TO_SRGB: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];

const MAT_SRGB_TO_REC2020: [[f32; 3]; 3] = [
    [0.627_403_9, 0.329_283, 0.043_313_1],
    [0.069_097_3, 0.919_540_4, 0.011_362_3],
    [0.016_391_4, 0.088_013_3, 0.895_595_3],
];

#[inline]
fn mul_mat3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Sony S-Log3 正規化値 `[0, 1]` → シーンリニア反射率変換（Sony 公式仕様）
#[inline]
pub fn s_log3_to_linear(v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0);
    if x >= 171.210_3 / 1023.0 {
        10.0_f32.powf((x * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
    } else {
        (x * 1023.0 - 95.0) * 0.011_25 / (171.210_3 - 95.0)
    }
    .max(0.0)
}

/// Panasonic V-Log 正規化値 `[0, 1]` → シーンリニア反射率変換
#[inline]
pub fn v_log_to_linear(v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0);
    if x < 0.181 {
        ((x - 0.125) / 5.6).max(0.0)
    } else {
        (10.0_f32.powf((x - 0.598_206) / 0.241_514) - 0.008_73).max(0.0)
    }
}

/// ARRI LogC3 (EI 800) 正規化値 `[0, 1]` → シーンリニア変換
#[inline]
pub fn log_c3_to_linear(v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0);
    if x > 0.149_658_2 {
        ((10.0_f32.powf((x - 0.385_537) / 0.247_189_6) - 0.052_272) / 5.555_556).max(0.0)
    } else {
        (((x - 0.092_809) / 5.367_655 - 0.052_272) / 5.555_556).max(0.0)
    }
}

#[inline]
fn linear_to_srgb_approx(x: f32) -> f32 {
    let c = x.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// ACES 1.3 Reference Gamut Compression（ネオン LED・高彩度青の負値・色相破綻を滑らかに抑圧）
#[inline]
pub fn aces_gamut_compress(rgb: [f32; 3]) -> [f32; 3] {
    let ach = rgb[0].max(rgb[1]).max(rgb[2]);
    if ach <= 1e-6 {
        return rgb;
    }
    let compress_ch = |ch: f32, lim: f32, thr: f32| -> f32 {
        let dist = (ach - ch) / ach;
        if dist <= thr {
            ch
        } else {
            let s = (lim - thr) / ((1.0 - thr).max(1e-4));
            let compressed = thr + s * (1.0 - (-((dist - thr) / s.max(1e-4))).exp());
            ach - compressed * ach
        }
    };
    [
        compress_ch(rgb[0], 1.147, 0.815),
        compress_ch(rgb[1], 1.264, 0.803),
        compress_ch(rgb[2], 1.312, 0.880),
    ]
}

/// 選択トーンマッパーをシーンリニア RGB へ適用し、ディスプレイリニア `[0, 1]` へ写像する
#[inline]
pub fn apply_tone_mapper_rgb(rgb: [f32; 3], mapper: ToneMapper) -> [f32; 3] {
    match mapper {
        ToneMapper::None => [
            rgb[0].clamp(0.0, 1.0),
            rgb[1].clamp(0.0, 1.0),
            rgb[2].clamp(0.0, 1.0),
        ],
        ToneMapper::AcesFitted => {
            // Krzysztof Narkowicz / Stephen Hill 近似 ACES Filmic 曲線
            let aces = |x: f32| {
                let v = x.max(0.0);
                let num = v * (2.51 * v + 0.03);
                let den = v * (2.43 * v + 0.59) + 0.14;
                (num / den.max(1e-5)).clamp(0.0, 1.0)
            };
            [aces(rgb[0]), aces(rgb[1]), aces(rgb[2])]
        }
        ToneMapper::AgX => {
            // AgX インセット行列＋ハイライト自然脱彩シグモイド近似
            let inset: [[f32; 3]; 3] = [
                [0.842_479, 0.078_434, 0.079_224],
                [0.042_328, 0.878_469, 0.079_166],
                [0.042_376, 0.078_434, 0.879_143],
            ];
            let outset: [[f32; 3]; 3] = [
                [1.196_879, -0.098_021, -0.099_030],
                [-0.052_897, 1.151_903, -0.098_961],
                [-0.052_972, -0.098_043, 1.151_074],
            ];
            let v = mul_mat3(&inset, [rgb[0].max(0.0), rgb[1].max(0.0), rgb[2].max(0.0)]);
            let curve = |x: f32| {
                let lg = ((x + 1e-6).log2() + 12.473_93) / 16.5;
                let t = lg.clamp(0.0, 1.0);
                // S-curve ロールオフ
                let t2 = t * t;
                (3.0 * t2 - 2.0 * t2 * t).powf(2.2)
            };
            let mapped = mul_mat3(&outset, [curve(v[0]), curve(v[1]), curve(v[2])]);
            [
                mapped[0].clamp(0.0, 1.0),
                mapped[1].clamp(0.0, 1.0),
                mapped[2].clamp(0.0, 1.0),
            ]
        }
        ToneMapper::ReinhardExtended => {
            let l_white_sq = 4.0 * 4.0;
            let rh = |x: f32| {
                let v = x.max(0.0);
                ((v * (1.0 + v / l_white_sq)) / (1.0 + v)).clamp(0.0, 1.0)
            };
            [rh(rgb[0]), rh(rgb[1]), rh(rgb[2])]
        }
    }
}

/// 1 ピクセルの線形 sRGB 値へ完全 ACEScg / OCIO カラー管理パイプライン（IDT → Working → Exposure → Gamut Compress → Tone Map → ODT）を適用する
pub fn transform_color_pipeline_rgb(linear_rgb: [f32; 3], cfg: &ColorManagementConfig) -> [f32; 3] {
    if !cfg.enabled {
        return linear_rgb;
    }

    // 1. Input Device Transform (IDT): 入力空間 → 線形 Rec.709 基準へ統一
    let mut rec709_lin = match cfg.input_idt {
        ColorSpace::LinearSrgb => linear_rgb,
        ColorSpace::AcesCg => mul_mat3(&MAT_ACESCG_TO_SRGB, linear_rgb),
        ColorSpace::DisplayP3 => mul_mat3(&MAT_P3_TO_SRGB, linear_rgb),
        ColorSpace::Rec2020 => mul_mat3(&MAT_REC2020_TO_SRGB, linear_rgb),
        ColorSpace::SLog3 => [
            s_log3_to_linear(linear_to_srgb_approx(linear_rgb[0])),
            s_log3_to_linear(linear_to_srgb_approx(linear_rgb[1])),
            s_log3_to_linear(linear_to_srgb_approx(linear_rgb[2])),
        ],
        ColorSpace::VLog => [
            v_log_to_linear(linear_to_srgb_approx(linear_rgb[0])),
            v_log_to_linear(linear_to_srgb_approx(linear_rgb[1])),
            v_log_to_linear(linear_to_srgb_approx(linear_rgb[2])),
        ],
        ColorSpace::LogC3 => [
            log_c3_to_linear(linear_to_srgb_approx(linear_rgb[0])),
            log_c3_to_linear(linear_to_srgb_approx(linear_rgb[1])),
            log_c3_to_linear(linear_to_srgb_approx(linear_rgb[2])),
        ],
    };

    // 2. ワーキング空間変換（ACEScg AP1 推奨）
    let mut work = match cfg.working_space {
        ColorSpace::AcesCg => mul_mat3(&MAT_SRGB_TO_ACESCG, rec709_lin),
        ColorSpace::DisplayP3 => mul_mat3(&MAT_SRGB_TO_P3, rec709_lin),
        ColorSpace::Rec2020 => mul_mat3(&MAT_SRGB_TO_REC2020, rec709_lin),
        _ => rec709_lin,
    };

    // 3. シーンリニア露出調整（EV stops）
    let exp = 2.0_f32.powf(cfg.exposure_stops.clamp(-4.0, 4.0));
    work = [work[0] * exp, work[1] * exp, work[2] * exp];

    // 4. ACES 1.3 ガマット圧縮
    if cfg.gamut_compress {
        work = aces_gamut_compress(work);
    }

    // 5. トーンマッピング（ハイライトロールオフ）
    let toned = apply_tone_mapper_rgb(work, cfg.tone_mapper);

    // 6. ワーキング空間 → 基準 Rec.709 → Output Device Transform (ODT)
    rec709_lin = match cfg.working_space {
        ColorSpace::AcesCg => mul_mat3(&MAT_ACESCG_TO_SRGB, toned),
        ColorSpace::DisplayP3 => mul_mat3(&MAT_P3_TO_SRGB, toned),
        ColorSpace::Rec2020 => mul_mat3(&MAT_REC2020_TO_SRGB, toned),
        _ => toned,
    };
    let out = match cfg.output_odt {
        ColorSpace::DisplayP3 => mul_mat3(&MAT_SRGB_TO_P3, rec709_lin),
        ColorSpace::Rec2020 => mul_mat3(&MAT_SRGB_TO_REC2020, rec709_lin),
        _ => rec709_lin,
    };
    [
        out[0].clamp(0.0, 1.0),
        out[1].clamp(0.0, 1.0),
        out[2].clamp(0.0, 1.0),
    ]
}

/// OpenColorIO v2 互換の `.ocio` 設定 YAML を生成する（他ソフト・VFX スタジオ連携用）
pub fn export_ocio_config_yaml(cfg: &ColorManagementConfig) -> String {
    format!(
        "ocio_profile_version: 2\n\
         name: NextGenVideoEditor_ACEScg_v4\n\
         description: Exported by Next-Gen Hybrid Video Editor v4.0 (Zero-Unsafe Pure Rust)\n\
         roles:\n\
           reference: {}\n\
           scene_linear: {}\n\
           compositing_linear: {}\n\
           default: {}\n\
         displays:\n\
           StudioDisplay:\n\
             - !<View> {{name: {}, colorspace: {}}}\n\
         active_displays: [StudioDisplay]\n\
         active_views: [{}]\n\
         gamut_compression: {}\n\
         exposure_stops: {:.3}\n",
        cfg.working_space.ocio_name(),
        cfg.working_space.ocio_name(),
        cfg.working_space.ocio_name(),
        cfg.input_idt.ocio_name(),
        cfg.tone_mapper.display_name(),
        cfg.output_odt.ocio_name(),
        cfg.tone_mapper.display_name(),
        cfg.gamut_compress,
        cfg.exposure_stops
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acescg_matrices_are_inverses_and_log_curves_are_monotone() {
        let orig = [0.62, 0.34, 0.18];
        let ap1 = mul_mat3(&MAT_SRGB_TO_ACESCG, orig);
        let back = mul_mat3(&MAT_ACESCG_TO_SRGB, ap1);
        for (a, b) in orig.into_iter().zip(back) {
            assert!(
                (a - b).abs() < 1e-3,
                "ACEScg roundtrip drift: {orig:?} -> {back:?}"
            );
        }

        // Camera Log EOTFs are strictly increasing in [0.2, 0.9]
        assert!(s_log3_to_linear(0.75) > s_log3_to_linear(0.41));
        assert!(v_log_to_linear(0.75) > v_log_to_linear(0.42));
        assert!(log_c3_to_linear(0.75) > log_c3_to_linear(0.39));
    }

    #[test]
    fn aces_and_agx_tone_mappers_roll_off_hdr_highlights_smoothly_and_export_ocio() {
        let hdr = [4.5, 2.2, 0.8];
        let cfg_aces = ColorManagementConfig {
            enabled: true,
            tone_mapper: ToneMapper::AcesFitted,
            ..ColorManagementConfig::default()
        };
        let out_aces = transform_color_pipeline_rgb(hdr, &cfg_aces);
        assert!(out_aces[0] > 0.7 && out_aces[0] <= 1.0);
        assert!(out_aces[0] > out_aces[1] && out_aces[1] > out_aces[2]);

        let cfg_agx = ColorManagementConfig {
            enabled: true,
            tone_mapper: ToneMapper::AgX,
            ..ColorManagementConfig::default()
        };
        let out_agx = transform_color_pipeline_rgb(hdr, &cfg_agx);
        assert!(out_agx[0] > 0.4 && out_agx[0] <= 1.0);

        let ocio = export_ocio_config_yaml(&cfg_aces);
        assert!(ocio.contains("ocio_profile_version: 2"));
        assert!(ocio.contains("ACES - ACEScg"));
    }
}
