//! v0.8 #13: `.cube` 3D LUT（四面体補間）・3-Way カラーグレーディング・リアルタイムビデオスコープ解析

use crate::color::{LinearImage, LinearPremul, linear_to_srgb, srgb_to_linear};
use std::fs;
use std::path::{Path, PathBuf};
use ve_core::SceneColorGrade;

pub const WAVEFORM_COLS: usize = 128;
pub const WAVEFORM_BINS: usize = 64;
pub const PARADE_COLS: usize = 64;
pub const PARADE_BINS: usize = 64;
pub const VECTORSCOPE_SIZE: usize = 96;
pub const HIST_BINS: usize = 64;

/// Adobe / DaVinci Resolve 標準 `.cube` 3D LUT（R 最速・四面体補間対応）
#[derive(Debug, Clone, PartialEq)]
pub struct CubeLut3D {
    pub title: String,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `size^3` 要素。インデックスは `b * size * size + g * size + r`
    pub table: Vec<[f32; 3]>,
}

impl CubeLut3D {
    /// `.cube` テキストをパースして検証済み `CubeLut3D` を構築する
    pub fn parse_str(text: &str) -> Result<Self, String> {
        let mut title = "Untitled LUT".to_string();
        let mut size: Option<usize> = None;
        let mut domain_min = [0.0f32; 3];
        let mut domain_max = [1.0f32; 3];
        let mut table: Vec<[f32; 3]> = Vec::new();

        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("TITLE") {
                let t = rest.trim().trim_matches('"').trim();
                if !t.is_empty() {
                    title = t.to_string();
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("LUT_3D_SIZE") {
                let n: usize = rest
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid LUT_3D_SIZE at line {}", line_no + 1))?;
                if !(2..=65).contains(&n) {
                    return Err(format!("LUT_3D_SIZE out of supported range 2..=65: {n}"));
                }
                size = Some(n);
                table.reserve(n * n * n);
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MIN") {
                domain_min = parse_rgb_triplet(rest, line_no + 1)?;
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MAX") {
                domain_max = parse_rgb_triplet(rest, line_no + 1)?;
                continue;
            }
            if line.starts_with("LUT_1D_SIZE") {
                return Err("1D .cube LUT is not supported; use LUT_3D_SIZE".into());
            }
            // データ行: R G B
            let rgb = parse_rgb_triplet(line, line_no + 1)?;
            table.push(rgb);
        }

        let n = size.ok_or_else(|| "missing LUT_3D_SIZE in .cube file".to_string())?;
        let expected = n * n * n;
        if table.len() != expected {
            return Err(format!(
                "expected {expected} LUT entries for size {n}, found {}",
                table.len()
            ));
        }
        Ok(Self {
            title,
            size: n,
            domain_min,
            domain_max,
            table,
        })
    }

    pub fn load_file(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("failed to read LUT: {e}"))?;
        Self::parse_str(&text)
    }

    /// 標準 `.cube` フォーマットテキストへ書き出す
    pub fn to_cube_string(&self) -> String {
        let mut out = String::with_capacity(self.table.len() * 24 + 128);
        out.push_str(&format!("TITLE \"{}\"\n", self.title.replace('"', "")));
        out.push_str(&format!("LUT_3D_SIZE {}\n", self.size));
        out.push_str(&format!(
            "DOMAIN_MIN {:.6} {:.6} {:.6}\n",
            self.domain_min[0], self.domain_min[1], self.domain_min[2]
        ));
        out.push_str(&format!(
            "DOMAIN_MAX {:.6} {:.6} {:.6}\n",
            self.domain_max[0], self.domain_max[1], self.domain_max[2]
        ));
        for &[r, g, b] in &self.table {
            out.push_str(&format!("{r:.6} {g:.6} {b:.6}\n"));
        }
        out
    }

    #[inline]
    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.table[b * self.size * self.size + g * self.size + r]
    }

    /// 四面体補間（Tetrahedral Interpolation）で sRGB `[0..1]` 入力を変換する
    pub fn sample_tetrahedral(&self, r: f32, g: f32, b: f32) -> [f32; 3] {
        let norm = |v: f32, lo: f32, hi: f32| -> f32 {
            let span = (hi - lo).max(1e-6);
            ((v - lo) / span).clamp(0.0, 1.0)
        };
        let scale = (self.size - 1) as f32;
        let rx = norm(r, self.domain_min[0], self.domain_max[0]) * scale;
        let gy = norm(g, self.domain_min[1], self.domain_max[1]) * scale;
        let bz = norm(b, self.domain_min[2], self.domain_max[2]) * scale;

        let r0 = (rx.floor() as usize).min(self.size - 1);
        let g0 = (gy.floor() as usize).min(self.size - 1);
        let b0 = (bz.floor() as usize).min(self.size - 1);
        let r1 = (r0 + 1).min(self.size - 1);
        let g1 = (g0 + 1).min(self.size - 1);
        let b1 = (b0 + 1).min(self.size - 1);

        let dr = rx - r0 as f32;
        let dg = gy - g0 as f32;
        let db = bz - b0 as f32;

        let c000 = self.at(r0, g0, b0);
        let c111 = self.at(r1, g1, b1);

        let (c1, c2, c3, f1, f2, f3) = if dr >= dg {
            if dg >= db {
                // dr >= dg >= db
                (
                    self.at(r1, g0, b0),
                    self.at(r1, g1, b0),
                    c111,
                    dr - dg,
                    dg - db,
                    db,
                )
            } else if dr >= db {
                // dr >= db > dg
                (
                    self.at(r1, g0, b0),
                    self.at(r1, g0, b1),
                    c111,
                    dr - db,
                    db - dg,
                    dg,
                )
            } else {
                // db > dr >= dg
                (
                    self.at(r0, g0, b1),
                    self.at(r1, g0, b1),
                    c111,
                    db - dr,
                    dr - dg,
                    dg,
                )
            }
        } else if db >= dg {
            // db >= dg > dr
            (
                self.at(r0, g0, b1),
                self.at(r0, g1, b1),
                c111,
                db - dg,
                dg - dr,
                dr,
            )
        } else if db >= dr {
            // dg > db >= dr
            (
                self.at(r0, g1, b0),
                self.at(r0, g1, b1),
                c111,
                dg - db,
                db - dr,
                dr,
            )
        } else {
            // dg > dr > db
            (
                self.at(r0, g1, b0),
                self.at(r1, g1, b0),
                c111,
                dg - dr,
                dr - db,
                db,
            )
        };

        let w0 = 1.0 - (f1 + f2 + f3);
        [
            (w0 * c000[0] + f1 * c1[0] + f2 * c2[0] + f3 * c3[0]).clamp(0.0, 1.0),
            (w0 * c000[1] + f1 * c1[1] + f2 * c2[1] + f3 * c3[1]).clamp(0.0, 1.0),
            (w0 * c000[2] + f1 * c1[2] + f2 * c2[2] + f3 * c3[2]).clamp(0.0, 1.0),
        ]
    }

    /// 組み込みシネマルック 3D LUT（17x17x17）をプロシージャル生成する
    pub fn builtin_preset(id: &str) -> Option<Self> {
        let size = 17usize;
        let key = id.strip_prefix("builtin:").unwrap_or(id);
        type RgbMapper = fn(f32, f32, f32) -> [f32; 3];
        let (title, map_fn): (&str, RgbMapper) = match key {
            "teal_orange" => ("Teal & Orange Cinema", |r, g, b| {
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                // シャドウをティール寄り、ハイライトをウォームオレンジ寄りへシフト＋S字コントラスト
                let s = |v: f32| {
                    let c = (v - 0.5) * 1.14 + 0.5;
                    c.clamp(0.0, 1.0)
                };
                let warm = (luma - 0.35).clamp(0.0, 1.0);
                let cool = (0.55 - luma).clamp(0.0, 1.0);
                [
                    s(r + warm * 0.14 - cool * 0.08),
                    s(g + warm * 0.03 + cool * 0.03),
                    s(b - warm * 0.10 + cool * 0.14),
                ]
            }),
            "warm_vintage" => ("Warm Vintage Film", |r, g, b| {
                // わずかに黒を浮かせたフィルムフェード＋暖色ハイライト
                let lift = 0.035;
                let nr = lift + r * 0.96 + 0.04 * (1.0 - b);
                let ng = lift + g * 0.93 + 0.015 * r;
                let nb = lift * 0.8 + b * 0.86;
                [nr.clamp(0.0, 1.0), ng.clamp(0.0, 1.0), nb.clamp(0.0, 1.0)]
            }),
            "cool_cyber" => ("Cool Cyber Night", |r, g, b| {
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let boost = (luma - 0.5).max(0.0);
                let nr = (r * 0.90 + boost * 0.18).clamp(0.0, 1.0);
                let ng = (g * 0.98 + (1.0 - luma) * 0.05).clamp(0.0, 1.0);
                let nb = (b * 1.12 + 0.06).clamp(0.0, 1.0);
                [nr, ng, nb]
            }),
            _ => return None,
        };

        let mut table = Vec::with_capacity(size * size * size);
        let denom = (size - 1) as f32;
        for bi in 0..size {
            let b = bi as f32 / denom;
            for gi in 0..size {
                let g = gi as f32 / denom;
                for ri in 0..size {
                    let r = ri as f32 / denom;
                    table.push(map_fn(r, g, b));
                }
            }
        }
        Some(Self {
            title: title.to_string(),
            size,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        })
    }
}

/// 組み込み LUT 一覧 `(id, display_name)`
pub const BUILTIN_LUT_PRESETS: &[(&str, &str)] = &[
    ("builtin:teal_orange", "🎬 Teal & Orange"),
    ("builtin:warm_vintage", "🎞 Warm Vintage"),
    ("builtin:cool_cyber", "🌃 Cool Cyber"),
];

/// 指定ディレクトリ（例: `luts/`）へ標準 `.cube` サンプル 3 種を書き出す
pub fn emit_sample_cube_luts(dir: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("failed to create LUT dir: {e}"))?;
    let mut created = Vec::new();
    for (id, file_name) in [
        ("teal_orange", "teal_orange_cinema.cube"),
        ("warm_vintage", "warm_vintage_film.cube"),
        ("cool_cyber", "cool_cyber_night.cube"),
    ] {
        if let Some(lut) = CubeLut3D::builtin_preset(id) {
            let path = dir.join(file_name);
            fs::write(&path, lut.to_cube_string())
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
            created.push(path);
        }
    }
    Ok(created)
}

fn parse_rgb_triplet(s: &str, line_no: usize) -> Result<[f32; 3], String> {
    let mut parts = s.split_whitespace();
    let r: f32 = parts
        .next()
        .ok_or_else(|| format!("missing R at line {line_no}"))?
        .parse()
        .map_err(|_| format!("invalid R float at line {line_no}"))?;
    let g: f32 = parts
        .next()
        .ok_or_else(|| format!("missing G at line {line_no}"))?
        .parse()
        .map_err(|_| format!("invalid G float at line {line_no}"))?;
    let b: f32 = parts
        .next()
        .ok_or_else(|| format!("missing B at line {line_no}"))?
        .parse()
        .map_err(|_| format!("invalid B float at line {line_no}"))?;
    if !r.is_finite() || !g.is_finite() || !b.is_finite() {
        return Err(format!("non-finite LUT value at line {line_no}"));
    }
    Ok([r, g, b])
}

/// シーン全体へ非破壊カラーグレーディング（露出・色温度/ティント・3-Way Lift/Gamma/Gain・コントラスト・彩度・3D LUT）を適用する
pub fn apply_scene_color_grade(
    img: &mut LinearImage,
    grade: &SceneColorGrade,
    lut: Option<&CubeLut3D>,
) {
    if !grade.enabled {
        return;
    }

    let exp_mult = 2.0f32.powf(grade.exposure.clamp(-3.0, 3.0));
    let temp = grade.temperature.clamp(-1.0, 1.0);
    let tint = grade.tint.clamp(-1.0, 1.0);
    let wb_r = (1.0 + temp * 0.25 + tint * 0.12).max(0.1);
    let wb_g = (1.0 - tint * 0.18).max(0.1);
    let wb_b = (1.0 - temp * 0.25 + tint * 0.12).max(0.1);

    let contrast = grade.contrast.clamp(0.2, 2.5);
    let sat = grade.saturation.clamp(0.0, 2.5);
    let lift = grade.lift;
    let gamma_exp = [
        (1.0 / (1.0 + grade.gamma[0].clamp(-0.5, 0.5) * 1.4)).clamp(0.35, 3.0),
        (1.0 / (1.0 + grade.gamma[1].clamp(-0.5, 0.5) * 1.4)).clamp(0.35, 3.0),
        (1.0 / (1.0 + grade.gamma[2].clamp(-0.5, 0.5) * 1.4)).clamp(0.35, 3.0),
    ];
    let gain = grade.gain;
    let lut_mix = grade.lut_intensity.clamp(0.0, 1.0);

    for px in &mut img.pixels {
        let a = px.a;
        if a <= 1e-6 {
            continue;
        }
        let inv_a = 1.0 / a;
        // 1. 線形空間で露出・ホワイトバランス適用
        let lr = (px.r * inv_a * exp_mult * wb_r).max(0.0);
        let lg = (px.g * inv_a * exp_mult * wb_g).max(0.0);
        let lb = (px.b * inv_a * exp_mult * wb_b).max(0.0);

        // 2. sRGB グレーディング空間で 3-Way (Lift / Gamma / Gain)・コントラスト・彩度
        let mut sr = linear_to_srgb(lr);
        let mut sg = linear_to_srgb(lg);
        let mut sb = linear_to_srgb(lb);

        sr = (sr * gain[0] + lift[0] * (1.0 - sr))
            .clamp(0.0, 1.0)
            .powf(gamma_exp[0]);
        sg = (sg * gain[1] + lift[1] * (1.0 - sg))
            .clamp(0.0, 1.0)
            .powf(gamma_exp[1]);
        sb = (sb * gain[2] + lift[2] * (1.0 - sb))
            .clamp(0.0, 1.0)
            .powf(gamma_exp[2]);

        sr = ((sr - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
        sg = ((sg - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
        sb = ((sb - 0.5) * contrast + 0.5).clamp(0.0, 1.0);

        let luma = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
        sr = (luma + (sr - luma) * sat).clamp(0.0, 1.0);
        sg = (luma + (sg - luma) * sat).clamp(0.0, 1.0);
        sb = (luma + (sb - luma) * sat).clamp(0.0, 1.0);

        // 3. `.cube` 3D LUT（四面体補間）
        if let Some(lut3d) = lut
            && lut_mix > 0.001
        {
            let mapped = lut3d.sample_tetrahedral(sr, sg, sb);
            sr = sr + (mapped[0] - sr) * lut_mix;
            sg = sg + (mapped[1] - sg) * lut_mix;
            sb = sb + (mapped[2] - sb) * lut_mix;
        }

        // 4. 線形空間＋乗算済みアルファへ戻す
        *px = LinearPremul {
            r: srgb_to_linear(sr) * a,
            g: srgb_to_linear(sg) * a,
            b: srgb_to_linear(sb) * a,
            a,
        };
    }
}

/// リアルタイム色調スコープ解析結果（Waveform / RGB Parade / Vectorscope / Histogram）
#[derive(Debug, Clone)]
pub struct VideoScopeData {
    /// 輝度波形 (`WAVEFORM_COLS * WAVEFORM_BINS`, row-major: `bin * COLS + col`, bin 0 = 0 IRE, bin 63 = 100 IRE)
    pub waveform: Vec<u16>,
    pub waveform_max: u16,
    /// RGB パレード (`PARADE_COLS * PARADE_BINS` 各チャンネル)
    pub parade_r: Vec<u16>,
    pub parade_g: Vec<u16>,
    pub parade_b: Vec<u16>,
    pub parade_max: u16,
    /// ベクトルスコープ (`VECTORSCOPE_SIZE * VECTORSCOPE_SIZE`, x=Cb[-0.5..0.5], y=Cr[0.5..-0.5])
    pub vectorscope: Vec<u16>,
    pub vectorscope_max: u16,
    /// RGB + 輝度ヒストグラム (各 64 ビン)
    pub hist_r: [u32; HIST_BINS],
    pub hist_g: [u32; HIST_BINS],
    pub hist_b: [u32; HIST_BINS],
    pub hist_luma: [u32; HIST_BINS],
    pub hist_max: u32,
    /// 統計サマリ
    pub avg_luma_ire: f32,
    pub peak_luma_ire: f32,
    pub clipped_highlight_pct: f32,
    pub crushed_shadow_pct: f32,
}

impl Default for VideoScopeData {
    fn default() -> Self {
        Self {
            waveform: vec![0; WAVEFORM_COLS * WAVEFORM_BINS],
            waveform_max: 1,
            parade_r: vec![0; PARADE_COLS * PARADE_BINS],
            parade_g: vec![0; PARADE_COLS * PARADE_BINS],
            parade_b: vec![0; PARADE_COLS * PARADE_BINS],
            parade_max: 1,
            vectorscope: vec![0; VECTORSCOPE_SIZE * VECTORSCOPE_SIZE],
            vectorscope_max: 1,
            hist_r: [0; HIST_BINS],
            hist_g: [0; HIST_BINS],
            hist_b: [0; HIST_BINS],
            hist_luma: [0; HIST_BINS],
            hist_max: 1,
            avg_luma_ire: 0.0,
            peak_luma_ire: 0.0,
            clipped_highlight_pct: 0.0,
            crushed_shadow_pct: 0.0,
        }
    }
}

/// プレビュー画像（Linear Premultiplied）から 4 種のビデオスコープを高速算出する（< 0.5ms）
pub fn compute_video_scopes(img: &LinearImage) -> VideoScopeData {
    let mut data = VideoScopeData::default();
    let w = img.width as usize;
    let h = img.height as usize;
    if w == 0 || h == 0 || img.pixels.is_empty() {
        return data;
    }

    // 最大 ~24,000 サンプルになるよう均等ストライド間引き
    let total_px = w * h;
    let stride = ((total_px as f32 / 24_000.0).sqrt().floor() as usize).max(1);

    let mut sampled = 0u64;
    let mut luma_sum = 0.0f64;
    let mut peak_luma = 0.0f32;
    let mut clipped_cnt = 0u64;
    let mut crushed_cnt = 0u64;

    let mut y = 0usize;
    while y < h {
        let row = y * w;
        let mut x = 0usize;
        while x < w {
            let px = img.pixels[row + x];
            let (sr, sg, sb) = if px.a > 1e-6 {
                let inv_a = 1.0 / px.a;
                (
                    linear_to_srgb((px.r * inv_a).clamp(0.0, 1.0)),
                    linear_to_srgb((px.g * inv_a).clamp(0.0, 1.0)),
                    linear_to_srgb((px.b * inv_a).clamp(0.0, 1.0)),
                )
            } else {
                (0.0, 0.0, 0.0)
            };

            // Rec.709 YCbCr
            let luma = (0.2126 * sr + 0.7152 * sg + 0.0722 * sb).clamp(0.0, 1.0);
            let cb = (-0.1146 * sr - 0.3854 * sg + 0.5000 * sb).clamp(-0.5, 0.5);
            let cr = (0.5000 * sr - 0.4542 * sg - 0.0458 * sb).clamp(-0.5, 0.5);

            sampled += 1;
            luma_sum += f64::from(luma);
            if luma > peak_luma {
                peak_luma = luma;
            }
            if sr >= 0.99 || sg >= 0.99 || sb >= 0.99 {
                clipped_cnt += 1;
            }
            if luma <= 0.015 {
                crushed_cnt += 1;
            }

            // 1. Waveform
            let wf_col = (x * WAVEFORM_COLS / w).min(WAVEFORM_COLS - 1);
            let wf_bin =
                ((luma * (WAVEFORM_BINS - 1) as f32).round() as usize).min(WAVEFORM_BINS - 1);
            let wf_slot = &mut data.waveform[wf_bin * WAVEFORM_COLS + wf_col];
            *wf_slot = wf_slot.saturating_add(1);
            if *wf_slot > data.waveform_max {
                data.waveform_max = *wf_slot;
            }

            // 2. RGB Parade
            let p_col = (x * PARADE_COLS / w).min(PARADE_COLS - 1);
            let rb = ((sr * (PARADE_BINS - 1) as f32).round() as usize).min(PARADE_BINS - 1);
            let gb = ((sg * (PARADE_BINS - 1) as f32).round() as usize).min(PARADE_BINS - 1);
            let bb = ((sb * (PARADE_BINS - 1) as f32).round() as usize).min(PARADE_BINS - 1);
            let pr = &mut data.parade_r[rb * PARADE_COLS + p_col];
            *pr = pr.saturating_add(1);
            let pg = &mut data.parade_g[gb * PARADE_COLS + p_col];
            *pg = pg.saturating_add(1);
            let pb = &mut data.parade_b[bb * PARADE_COLS + p_col];
            *pb = pb.saturating_add(1);
            data.parade_max = data.parade_max.max(*pr).max(*pg).max(*pb);

            // 3. Vectorscope (Cb -> X, +Cr -> Up)
            let vx = (((cb + 0.5) * (VECTORSCOPE_SIZE - 1) as f32).round() as usize)
                .min(VECTORSCOPE_SIZE - 1);
            let vy = (((0.5 - cr) * (VECTORSCOPE_SIZE - 1) as f32).round() as usize)
                .min(VECTORSCOPE_SIZE - 1);
            let vs = &mut data.vectorscope[vy * VECTORSCOPE_SIZE + vx];
            *vs = vs.saturating_add(1);
            if *vs > data.vectorscope_max {
                data.vectorscope_max = *vs;
            }

            // 4. Histogram
            let hr = ((sr * (HIST_BINS - 1) as f32).round() as usize).min(HIST_BINS - 1);
            let hg = ((sg * (HIST_BINS - 1) as f32).round() as usize).min(HIST_BINS - 1);
            let hb = ((sb * (HIST_BINS - 1) as f32).round() as usize).min(HIST_BINS - 1);
            let hy = ((luma * (HIST_BINS - 1) as f32).round() as usize).min(HIST_BINS - 1);
            data.hist_r[hr] += 1;
            data.hist_g[hg] += 1;
            data.hist_b[hb] += 1;
            data.hist_luma[hy] += 1;
            data.hist_max = data
                .hist_max
                .max(data.hist_r[hr])
                .max(data.hist_g[hg])
                .max(data.hist_b[hb])
                .max(data.hist_luma[hy]);

            x += stride;
        }
        y += stride;
    }

    if sampled > 0 {
        let denom = sampled as f64;
        data.avg_luma_ire = ((luma_sum / denom) * 100.0) as f32;
        data.peak_luma_ire = peak_luma * 100.0;
        data.clipped_highlight_pct = (clipped_cnt as f64 / denom * 100.0) as f32;
        data.crushed_shadow_pct = (crushed_cnt as f64 / denom * 100.0) as f32;
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::RgbaColor;

    #[test]
    fn parses_cube_lut_and_tetrahedral_interpolates_accurately() {
        // 2x2x2 恒等変換 LUT の R/B スワップ版を作成して四面体補間精度を検証
        let cube = "\
# Comment line
TITLE \"Swap RB\"
LUT_3D_SIZE 2
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 1.0 1.0 1.0
0.0 0.0 0.0
0.0 0.0 1.0
0.0 1.0 0.0
0.0 1.0 1.0
1.0 0.0 0.0
1.0 0.0 1.0
1.0 1.0 0.0
1.0 1.0 1.0
";
        let lut = CubeLut3D::parse_str(cube).expect("parse 2x2x2 cube");
        assert_eq!(lut.title, "Swap RB");
        let out = lut.sample_tetrahedral(0.8, 0.3, 0.2);
        assert!((out[0] - 0.2).abs() < 1e-5, "out={out:?}");
        assert!((out[1] - 0.3).abs() < 1e-5, "out={out:?}");
        assert!((out[2] - 0.8).abs() < 1e-5, "out={out:?}");

        // 組み込みプリセットの書き出し→再読込ラウンドトリップ検証
        let tmp_dir = std::env::temp_dir().join(format!("ve_lut_test_{}", uuid::Uuid::new_v4()));
        let paths = emit_sample_cube_luts(&tmp_dir).unwrap();
        assert_eq!(paths.len(), 3);
        let reloaded = CubeLut3D::load_file(&paths[0]).unwrap();
        assert_eq!(reloaded.size, 17);
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn scene_color_grade_applies_lift_gamma_gain_and_lut() {
        let mut img = LinearImage::new(
            8,
            8,
            LinearPremul::from_srgb_straight(RgbaColor {
                r: 0.5,
                g: 0.5,
                b: 0.5,
                a: 1.0,
            }),
        );
        let lut = CubeLut3D::builtin_preset("teal_orange").unwrap();
        let grade = SceneColorGrade {
            enabled: true,
            temperature: 0.5,
            gain: [1.2, 1.0, 0.85],
            lut_intensity: 0.8,
            ..SceneColorGrade::default()
        };
        apply_scene_color_grade(&mut img, &grade, Some(&lut));
        let p = img.pixels[0];
        // 暖色＋赤 Gain で R > B に変化していること
        assert!(p.r > p.b, "expected warm shift r={} > b={}", p.r, p.b);
    }

    #[test]
    fn video_scopes_compute_waveform_parade_vectorscope_and_histogram() {
        let mut img = LinearImage::new(32, 32, LinearPremul::TRANSPARENT);
        for y in 0..32u32 {
            for x in 0..32u32 {
                let c = if x < 16 {
                    RgbaColor {
                        r: 0.9,
                        g: 0.1,
                        b: 0.1,
                        a: 1.0,
                    }
                } else {
                    RgbaColor {
                        r: 0.1,
                        g: 0.8,
                        b: 0.9,
                        a: 1.0,
                    }
                };
                img.pixels[(y * 32 + x) as usize] = LinearPremul::from_srgb_straight(c);
            }
        }
        let scopes = compute_video_scopes(&img);
        assert!(scopes.waveform_max > 0);
        assert!(scopes.parade_max > 0);
        assert!(scopes.vectorscope_max > 0);
        assert!(scopes.hist_max > 0);
        assert!(scopes.avg_luma_ire > 20.0 && scopes.avg_luma_ire < 80.0);
    }
}
