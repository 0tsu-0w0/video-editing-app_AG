use crate::color::{LinearImage, LinearPremul, linear_to_srgb};
use crate::graph::RenderGraph;
use std::path::{Path, PathBuf};
use ve_core::{
    CubicBezier, EditCommand, Interpolation, ItemKind, Keyframe, Project, RationalTime, Scene,
    SceneId,
};

// =========================================================================
// 0. Offline-Only Local Inference Backend Status (Zero Cloud Upload)
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalVisionBackend {
    /// 外部ランタイム不要・完全オフラインの Pure-Rust 内蔵コンピュータビジョンエンジン
    PureRustBuiltIn,
    /// オプショナルなローカル ONNX Runtime CLI 連携（クラウド送信ゼロ・未導入時は自動で PureRustBuiltIn へフォールバック）
    LocalOnnxCli {
        executable: PathBuf,
        model_path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVisionStatus {
    pub active_backend: LocalVisionBackend,
    pub onnx_models_found: Vec<String>,
    pub local_only_guaranteed: bool,
    pub summary: String,
}

/// プロジェクトの `models/` を検査し、利用可能なローカル推論バックエンド構成を返す（ネットワーク通信ゼロ）
pub fn detect_local_vision_status(models_dir: Option<&Path>) -> LocalVisionStatus {
    let mut onnx_models = Vec::new();
    let mut first_model: Option<PathBuf> = None;
    if let Some(dir) = models_dir
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("onnx"))
            {
                if first_model.is_none() {
                    first_model = Some(p.clone());
                }
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    onnx_models.push(name.to_string());
                }
            }
        }
    }
    onnx_models.sort();

    let onnx_bin = std::env::var_os("VE_LOCAL_ONNX_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file());

    let (active_backend, summary) = match (onnx_bin, first_model) {
        (Some(exec), Some(model)) => (
            LocalVisionBackend::LocalOnnxCli {
                executable: exec.clone(),
                model_path: model.clone(),
            },
            format!(
                "ローカル ONNX ランタイム検出 ({}, モデル {} 件) — 完全オフライン",
                exec.display(),
                onnx_models.len()
            ),
        ),
        _ => (
            LocalVisionBackend::PureRustBuiltIn,
            format!(
                "Pure-Rust 内蔵ローカル AI ビジョン稼働中 (models/*.onnx: {} 件) — クラウド送信ゼロ",
                onnx_models.len()
            ),
        ),
    };

    LocalVisionStatus {
        active_backend,
        onnx_models_found: onnx_models,
        local_only_guaranteed: true,
        summary,
    }
}

// =========================================================================
// 1. Green-Screen-Free Foreground Segmentation (Auto Rotobrush)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentationConfig {
    /// 背景除去の感度 (0.1..=1.0, 既定 0.55)
    pub sensitivity: f32,
    /// エッジのフェザー半径 px (0.0..=6.0, 既定 1.5)
    pub edge_feather_px: f32,
    /// 中央被写体バイアス (0.0..=1.0, 既定 0.45)
    pub fg_bias: f32,
    /// 人物肌色保護ブースト (0.0..=1.0, 既定 0.65)
    pub skin_boost: f32,
    /// エッジ保存バイラテラル精製反復回数 (1..=4, 既定 2)
    pub refine_iters: u32,
    /// 背景色エッジ汚染の除去強度 (0.0..=1.0, 既定 0.35)
    pub despill_strength: f32,
    /// マット反転（背景だけ残す場合 true）
    pub invert: bool,
}

impl Default for SegmentationConfig {
    fn default() -> Self {
        Self {
            sensitivity: 0.55,
            edge_feather_px: 1.5,
            fg_bias: 0.45,
            skin_boost: 0.65,
            refine_iters: 2,
            despill_strength: 0.35,
            invert: false,
        }
    }
}

/// グリーンバック不要のローカル人物・被写体切り抜き（自動ロトブラシ）をインプレース適用する。
/// 外周（上・左・右）から背景色クラスタを自動推定し、中心被写体事前分布＋肌色保護＋エッジ保存バイラテラル精製で滑らかなアルファマットを生成する。
pub fn apply_ai_segmentation(image: &mut LinearImage, cfg: &SegmentationConfig) {
    let w = image.width as usize;
    let h = image.height as usize;
    if w < 4 || h < 4 {
        return;
    }

    // 1. 直線色空間から sRGB 近似へ戻して知覚均等に近い opponent 特徴 (L, a, b) を抽出
    let mut feat = vec![[0.0f32; 3]; w * h];
    let mut srgb_rgb = vec![[0.0f32; 3]; w * h];
    for (i, px) in image.pixels.iter().enumerate() {
        if px.a <= 1e-5 {
            continue;
        }
        let inv_a = 1.0 / px.a;
        let r = linear_to_srgb((px.r * inv_a).clamp(0.0, 1.0));
        let g = linear_to_srgb((px.g * inv_a).clamp(0.0, 1.0));
        let b = linear_to_srgb((px.b * inv_a).clamp(0.0, 1.0));
        srgb_rgb[i] = [r, g, b];
        let l = 0.299 * r + 0.587 * g + 0.114 * b;
        let opp_a = (r - g) * 0.85;
        let opp_b = (0.5 * (r + g) - b) * 0.85;
        feat[i] = [l, opp_a, opp_b];
    }

    // 2. 上端・左端・右端の帯域から背景プロトタイプ色（最大 12 代表点）を収集
    let border_x = (w / 14).clamp(2, 16);
    let border_y = (h / 14).clamp(2, 16);
    let mut bg_samples: Vec<[f32; 3]> = Vec::new();
    let step_x = (w / 24).max(1);
    let step_y = (h / 24).max(1);

    for y in (0..border_y).step_by(step_y.clamp(1, 2)) {
        for x in (0..w).step_by(step_x) {
            if image.pixels[y * w + x].a > 0.2 {
                bg_samples.push(feat[y * w + x]);
            }
        }
    }
    for y in (border_y..h.saturating_sub(border_y)).step_by(step_y) {
        for x in 0..border_x.min(w) {
            if image.pixels[y * w + x].a > 0.2 {
                bg_samples.push(feat[y * w + x]);
            }
            let rx = w - 1 - x;
            if image.pixels[y * w + rx].a > 0.2 {
                bg_samples.push(feat[y * w + rx]);
            }
        }
    }

    if bg_samples.is_empty() {
        return;
    }

    // Farthest-point sampling で最大 10 個の多様な背景色プロトタイプを抽出（単色～グラデーション背景両対応）
    let mut prototypes: Vec<[f32; 3]> = vec![bg_samples[0]];
    let max_protos = 10.min(bg_samples.len());
    while prototypes.len() < max_protos {
        let mut best_idx = None;
        let mut best_dist = 0.015_f32; // これ以上離れていなければクラスタ追加不要
        for (idx, s) in bg_samples.iter().enumerate() {
            let min_d = prototypes
                .iter()
                .map(|p| color_dist_sq(*s, *p))
                .fold(f32::INFINITY, f32::min);
            if min_d > best_dist {
                best_dist = min_d;
                best_idx = Some(idx);
            }
        }
        if let Some(i) = best_idx {
            prototypes.push(bg_samples[i]);
        } else {
            break;
        }
    }

    // 代表背景色平均（デスピル用）
    let mut mean_bg_rgb = [0.0f32; 3];
    let mut bg_cnt = 0.0f32;
    for y in 0..border_y {
        for x in (0..w).step_by(step_x) {
            let idx = y * w + x;
            if image.pixels[idx].a > 0.2 {
                mean_bg_rgb[0] += srgb_rgb[idx][0];
                mean_bg_rgb[1] += srgb_rgb[idx][1];
                mean_bg_rgb[2] += srgb_rgb[idx][2];
                bg_cnt += 1.0;
            }
        }
    }
    if bg_cnt > 0.0 {
        mean_bg_rgb[0] /= bg_cnt;
        mean_bg_rgb[1] /= bg_cnt;
        mean_bg_rgb[2] /= bg_cnt;
    }

    // 3. 初期前景確率マットを算出
    let sens = cfg.sensitivity.clamp(0.05, 1.0);
    // sensitivity が高いほど小さな色差でも背景と区別・逆に低くすると広い範囲を残す
    let low_thresh = (0.06 + (1.0 - sens) * 0.14).clamp(0.04, 0.28);
    let high_thresh = (low_thresh + 0.14).clamp(0.14, 0.55);
    let fg_bias = cfg.fg_bias.clamp(0.0, 1.0);
    let skin_boost = cfg.skin_boost.clamp(0.0, 1.0);

    let mut alpha = vec![0.0f32; w * h];
    for y in 0..h {
        let ny = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let idx = y * w + x;
            let orig_a = image.pixels[idx].a;
            if orig_a <= 1e-5 {
                alpha[idx] = 0.0;
                continue;
            }
            let nx = (x as f32 + 0.5) / w as f32;
            let f = feat[idx];
            let min_dist = prototypes
                .iter()
                .map(|p| color_dist_sq(f, *p))
                .fold(f32::INFINITY, f32::min)
                .sqrt();

            // 人物構図を想定したソフトな楕円中心事前分布（下中央〜中央やや上）
            let dx = (nx - 0.5) / 0.42;
            let dy = (ny - 0.56) / 0.52;
            let center_prior = (1.0 - (dx * dx + dy * dy)).clamp(0.0, 1.0);

            let skin_prob = estimate_skin_probability(srgb_rgb[idx]);
            // 背景との距離に中心バイアスと肌色ブーストを加味（ただし背景色完全一致の周囲は強く抑制）
            let effective_dist = min_dist
                + if min_dist > low_thresh * 0.55 {
                    center_prior * fg_bias * 0.12 + skin_prob * skin_boost * 0.18
                } else {
                    0.0
                };

            let t = ((effective_dist - low_thresh) / (high_thresh - low_thresh).max(1e-4))
                .clamp(0.0, 1.0);
            // Smoothstep で滑らかなソフトエッジ遷移
            let mut m = t * t * (3.0 - 2.0 * t);
            if cfg.invert {
                m = 1.0 - m;
            }
            alpha[idx] = m * orig_a;
        }
    }

    // 4. エッジ保存ジョイントバイラテラル精製（被写体輪郭へマットを吸着）
    let iters = cfg.refine_iters.clamp(1, 4);
    let radius = (cfg.edge_feather_px.round() as i32).clamp(1, 5);
    let sigma_color_sq = 0.025_f32;
    let mut scratch = vec![0.0f32; w * h];

    for _ in 0..iters {
        for y in 0..h {
            for x in 0..w {
                let c_idx = y * w + x;
                let cf = feat[c_idx];
                let mut sum_w = 0.0f32;
                let mut sum_a = 0.0f32;
                for ky in -radius..=radius {
                    let sy = (y as i32 + ky).clamp(0, h as i32 - 1) as usize;
                    for kx in -radius..=radius {
                        let sx = (x as i32 + kx).clamp(0, w as i32 - 1) as usize;
                        let n_idx = sy * w + sx;
                        let dc = color_dist_sq(cf, feat[n_idx]);
                        let ds = (kx * kx + ky * ky) as f32 / (radius * radius + 1) as f32;
                        let weight = (-dc / sigma_color_sq - ds * 0.5).exp();
                        sum_w += weight;
                        sum_a += alpha[n_idx] * weight;
                    }
                }
                scratch[c_idx] = if sum_w > 1e-6 {
                    (sum_a / sum_w).clamp(0.0, 1.0)
                } else {
                    alpha[c_idx]
                };
            }
        }
        std::mem::swap(&mut alpha, &mut scratch);
    }

    // 5. マット適用＆エッジ背景色デスピル
    let despill = cfg.despill_strength.clamp(0.0, 1.0);
    for (i, px) in image.pixels.iter_mut().enumerate() {
        let old_a = px.a;
        if old_a <= 1e-6 {
            continue;
        }
        let new_a = alpha[i].clamp(0.0, 1.0);
        if new_a <= 1e-4 {
            *px = LinearPremul::TRANSPARENT;
            continue;
        }
        let ratio = new_a / old_a;
        let mut r = px.r * ratio;
        let mut g = px.g * ratio;
        let mut b = px.b * ratio;

        // 半透明エッジ部（0 < a < 0.9）の背景色汚染を軽減
        if despill > 0.01 && new_a < 0.9 {
            let edge_w = (1.0 - new_a) * despill * 0.45;
            let bg_l = 0.2126 * mean_bg_rgb[0] + 0.7152 * mean_bg_rgb[1] + 0.0722 * mean_bg_rgb[2];
            r = (r - (mean_bg_rgb[0] - bg_l).max(0.0) * edge_w * new_a).max(0.0);
            g = (g - (mean_bg_rgb[1] - bg_l).max(0.0) * edge_w * new_a).max(0.0);
            b = (b - (mean_bg_rgb[2] - bg_l).max(0.0) * edge_w * new_a).max(0.0);
        }

        px.r = r;
        px.g = g;
        px.b = b;
        px.a = new_a;
    }
}

#[inline]
fn color_dist_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dl = a[0] - b[0];
    let da = a[1] - b[1];
    let db = a[2] - b[2];
    dl * dl * 0.75 + da * da * 1.4 + db * db * 1.4
}

/// YCbCr 空間による肌色・顔領域確率 (0.0..=1.0)
#[inline]
fn estimate_skin_probability(rgb: [f32; 3]) -> f32 {
    let r = rgb[0] * 255.0;
    let g = rgb[1] * 255.0;
    let b = rgb[2] * 255.0;
    let cb = 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b;
    let cr = 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b;
    let d_cb = (cb - 105.0) / 22.0;
    let d_cr = (cr - 152.0) / 20.0;
    let dist = d_cb * d_cb + d_cr * d_cr;
    if dist < 2.2 && r > 45.0 && r > g * 0.92 {
        (-dist * 0.7).exp()
    } else {
        0.0
    }
}

// =========================================================================
// 2. Subject & Face-Aware Intelligent Auto-Reframe (16:9 -> 9:16 / 1:1)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoReframeConfig {
    pub target_width: u32,
    pub target_height: u32,
    /// サンプリング間隔（秒、既定 0.4s）
    pub sample_interval_sec: f64,
    /// カメラ追従スムージング強度 (0.0..=0.95, 既定 0.72)
    pub smoothing_strength: f32,
    /// 顔・肌色検出ウェイト
    pub face_skin_weight: f32,
    /// フレーム間モーションウェイト
    pub motion_weight: f32,
    /// エッジ顕著性ウェイト
    pub edge_saliency_weight: f32,
}

impl Default for AutoReframeConfig {
    fn default() -> Self {
        Self {
            target_width: 1080,
            target_height: 1920,
            sample_interval_sec: 0.4,
            smoothing_strength: 0.72,
            face_skin_weight: 2.4,
            motion_weight: 1.8,
            edge_saliency_weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReframeSample {
    pub time: RationalTime,
    /// 正規化注視点 X (0.0 左端 〜 1.0 右端)
    pub focus_x_norm: f32,
    /// 正規化注視点 Y (0.0 上端 〜 1.0 下端)
    pub focus_y_norm: f32,
    pub confidence: f32,
}

/// 1 フレーム（＋任意の前フレーム）から被写体・顔・動きの注視重心 `(focus_x, focus_y, confidence)` を推定する
pub fn detect_subject_focus_in_frame(
    frame: &LinearImage,
    prev_luma: Option<&[f32]>,
    cfg: &AutoReframeConfig,
) -> (f32, f32, f32, Vec<f32>) {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if w < 4 || h < 4 {
        return (0.5, 0.5, 0.0, Vec::new());
    }
    let mut luma = vec![0.0f32; w * h];
    let mut skin = vec![0.0f32; w * h];
    for (i, px) in frame.pixels.iter().enumerate() {
        let a = px.a.clamp(0.0, 1.0);
        if a <= 1e-4 {
            continue;
        }
        let inv = 1.0 / a;
        let r = linear_to_srgb((px.r * inv).clamp(0.0, 1.0));
        let g = linear_to_srgb((px.g * inv).clamp(0.0, 1.0));
        let b = linear_to_srgb((px.b * inv).clamp(0.0, 1.0));
        luma[i] = (0.299 * r + 0.587 * g + 0.114 * b) * a;
        skin[i] = estimate_skin_probability([r, g, b]) * a;
    }

    let mut sum_w = 0.0f64;
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut peak_score = 0.0f32;

    for y in 1..h - 1 {
        let ny = (y as f32 + 0.5) / h as f32;
        // 人物顔は上部 15%〜65% に出やすいため緩やかな縦優先ウェイト
        let portrait_y_prior = 1.0 + 0.35 * (-((ny - 0.38) / 0.28).powi(2)).exp();
        for x in 1..w - 1 {
            let idx = y * w + x;
            let gx = (luma[idx + 1] - luma[idx - 1]).abs();
            let gy = (luma[idx + w] - luma[idx - w]).abs();
            let edge = (gx + gy).min(1.0);

            let motion = prev_luma
                .and_then(|p| p.get(idx).copied())
                .map(|pl| (luma[idx] - pl).abs())
                .unwrap_or(0.0);

            let face_score = skin[idx] * portrait_y_prior * cfg.face_skin_weight;
            let motion_score = motion * cfg.motion_weight;
            let edge_score = edge * cfg.edge_saliency_weight;
            // 背景一様領域のノイズを弾くため閾値以上のスコアのみ採用
            let total = (face_score + motion_score + edge_score).max(0.0);
            if total > 0.08 {
                let nx = (x as f64 + 0.5) / w as f64;
                let w_f64 = f64::from(total * total);
                sum_w += w_f64;
                sum_x += nx * w_f64;
                sum_y += f64::from(ny) * w_f64;
                peak_score = peak_score.max(total);
            }
        }
    }

    if sum_w <= 1e-6 {
        return (0.5, 0.5, 0.0, luma);
    }

    let fx = (sum_x / sum_w).clamp(0.1, 0.9) as f32;
    let fy = (sum_y / sum_w).clamp(0.1, 0.9) as f32;
    let conf = (peak_score / 2.5).clamp(0.0, 1.0);
    (fx, fy, conf, luma)
}

/// シーン全体を走査し、被写体・顔・動きに追従する滑らかなリフレーム軌跡 (`Vec<ReframeSample>`) を解析する
pub fn analyze_scene_auto_reframe(
    graph: &mut RenderGraph,
    project: &Project,
    scene_id: SceneId,
    cfg: &AutoReframeConfig,
) -> Vec<ReframeSample> {
    let Some(scene) = project.scene(scene_id) else {
        return Vec::new();
    };
    let total_sec = scene.total_duration().as_seconds().max(0.1);
    let interval = cfg.sample_interval_sec.clamp(0.1, 5.0);
    let max_samples = 48usize;
    let step = (total_sec / max_samples as f64).max(interval);

    let analysis_w = 160u32;
    let analysis_h = ((analysis_w as f64 * scene.height.max(1) as f64) / scene.width.max(1) as f64)
        .round()
        .clamp(32.0, 240.0) as u32;

    let mut samples = Vec::new();
    let mut prev_luma: Option<Vec<f32>> = None;
    let mut t = 0.0;
    while t <= total_sec + 1e-6 && samples.len() < max_samples {
        let rt = RationalTime::from_seconds(t.min(total_sec));
        if let Some(frame) =
            graph.render_project_scene_scaled(project, scene_id, rt, analysis_w, analysis_h, false)
        {
            let (fx, fy, conf, luma) =
                detect_subject_focus_in_frame(&frame, prev_luma.as_deref(), cfg);
            samples.push(ReframeSample {
                time: rt,
                focus_x_norm: fx,
                focus_y_norm: fy,
                confidence: conf,
            });
            prev_luma = Some(luma);
        }
        t += step;
    }

    if samples.len() >= 2 {
        smooth_reframe_trajectory(&mut samples, cfg.smoothing_strength);
    }
    samples
}

/// 前向き＋後ろ向き指数平滑化（ゼロ位相遅れ）＋マイクロジッター抑制デッドバンド
pub fn smooth_reframe_trajectory(samples: &mut [ReframeSample], smoothing: f32) {
    let alpha = (1.0 - smoothing.clamp(0.0, 0.95)).clamp(0.05, 1.0);
    let n = samples.len();
    if n < 2 {
        return;
    }
    // Forward pass
    for i in 1..n {
        let prev_x = samples[i - 1].focus_x_norm;
        let prev_y = samples[i - 1].focus_y_norm;
        let dx = samples[i].focus_x_norm - prev_x;
        let dy = samples[i].focus_y_norm - prev_y;
        // 2% 未満の微小揺れはデッドバンドで固定
        let eff_dx = if dx.abs() < 0.02 { 0.0 } else { dx };
        let eff_dy = if dy.abs() < 0.02 { 0.0 } else { dy };
        samples[i].focus_x_norm = prev_x + eff_dx * alpha;
        samples[i].focus_y_norm = prev_y + eff_dy * alpha;
    }
    // Backward pass（遅れを打ち消す）
    for i in (0..n - 1).rev() {
        let next_x = samples[i + 1].focus_x_norm;
        let next_y = samples[i + 1].focus_y_norm;
        samples[i].focus_x_norm = samples[i].focus_x_norm * 0.5 + next_x * 0.5;
        samples[i].focus_y_norm = samples[i].focus_y_norm * 0.5 + next_y * 0.5;
    }
}

/// 解析した被写体追尾軌跡から、1 ステップ Undo 可能な自動リフレーム（解像度変更＋滑らかパンキーフレーム）コマンド列を生成する
pub fn build_auto_reframe_commands(
    scene: &Scene,
    cfg: &AutoReframeConfig,
    samples: &[ReframeSample],
) -> Vec<EditCommand> {
    let tw = cfg.target_width.clamp(64, 4096);
    let th = cfg.target_height.clamp(64, 4096);
    let mut cmds = vec![EditCommand::SetSceneSettings {
        width: tw,
        height: th,
        fps: scene.fps,
        rescale_items: true,
    }];

    if samples.is_empty() {
        return cmds;
    }

    let old_w = scene.width.max(1) as f32;
    let old_h = scene.height.max(1) as f32;
    let sx = tw as f32 / old_w;
    let sy = th as f32 / old_h;

    for layer in &scene.layers {
        for item in &layer.items {
            // 音声・フィルタ・グループ制御はスキップし、映像・画像・シェイプ・立ち絵に追従パンを適用
            if matches!(
                item.kind,
                ItemKind::Audio { .. }
                    | ItemKind::FilterObject { .. }
                    | ItemKind::Transition { .. }
                    | ItemKind::GroupControl { .. }
                    | ItemKind::Text { .. }
            ) {
                continue;
            }
            let base_x = item.transform.x * sx;
            let base_y = item.transform.y * sy;
            for sample in samples {
                if sample.time < item.start || sample.time > item.end() {
                    continue;
                }
                let local_t = sample.time - item.start;
                // 被写体中心が target_width の中央 (0.5) に来るように水平パン補正
                let pan_x = (0.5 - sample.focus_x_norm) * tw as f32 * 0.85;
                let pan_y = (0.5 - sample.focus_y_norm) * th as f32 * 0.25;
                cmds.push(EditCommand::SetKeyframe {
                    item_id: item.id,
                    property: "x".to_string(),
                    keyframe: Keyframe {
                        time: local_t,
                        value: base_x + pan_x,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                    },
                });
                cmds.push(EditCommand::SetKeyframe {
                    item_id: item.id,
                    property: "y".to_string(),
                    keyframe: Keyframe {
                        time: local_t,
                        value: base_y + pan_y,
                        interpolation: Interpolation::Bezier(CubicBezier::EASE_IN_OUT),
                    },
                });
            }
        }
    }

    cmds
}

// =========================================================================
// 3. Local Super-Resolution (2x / 4x Lanczos-3 + Edge-Directed Detail)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuperResolutionConfig {
    /// 拡大倍率（2 または 4）
    pub scale_factor: u32,
    /// エッジ方向ディテール強調量 (0.0..=1.5, 既定 0.45)
    pub sharpen_amount: f32,
    /// 平坦部のノイズ抑制量 (0.0..=1.0, 既定 0.2)
    pub denoise_strength: f32,
}

impl Default for SuperResolutionConfig {
    fn default() -> Self {
        Self {
            scale_factor: 2,
            sharpen_amount: 0.45,
            denoise_strength: 0.2,
        }
    }
}

#[inline]
fn lanczos3(x: f32) -> f32 {
    let ax = x.abs();
    if ax < 1e-5 {
        return 1.0;
    }
    if ax >= 3.0 {
        return 0.0;
    }
    let pi_x = std::f32::consts::PI * ax;
    let pi_x3 = pi_x / 3.0;
    (pi_x.sin() / pi_x) * (pi_x3.sin() / pi_x3)
}

/// ローカル超解像: 2 パス分離 Lanczos-3 アップサンプリング ＋ Sobel エッジ誘導ディテール復元＋平坦部デノイズ
pub fn upscale_super_resolution(src: &LinearImage, cfg: &SuperResolutionConfig) -> LinearImage {
    let scale = if cfg.scale_factor >= 4 { 4 } else { 2 };
    let dst_w = (src.width.saturating_mul(scale)).clamp(1, 4096);
    let dst_h = (src.height.saturating_mul(scale)).clamp(1, 4096);
    if src.width == 0 || src.height == 0 {
        return LinearImage::new(dst_w, dst_h, LinearPremul::TRANSPARENT);
    }

    // Pass 1: Horizontal Lanczos-3
    let mut temp = LinearImage::new(dst_w, src.height, LinearPremul::TRANSPARENT);
    let ratio_x = src.width as f32 / dst_w as f32;
    for y in 0..src.height {
        let src_row = (y * src.width) as usize;
        let dst_row = (y * dst_w) as usize;
        for dx in 0..dst_w {
            let center = (dx as f32 + 0.5) * ratio_x - 0.5;
            let i0 = (center.floor() as i32) - 2;
            let i1 = (center.floor() as i32) + 3;
            let mut acc = LinearPremul::TRANSPARENT;
            let mut w_sum = 0.0f32;
            for sx in i0..=i1 {
                let w = lanczos3(center - sx as f32);
                if w.abs() < 1e-6 {
                    continue;
                }
                let cx = sx.clamp(0, src.width as i32 - 1) as usize;
                let p = src.pixels[src_row + cx];
                acc.r += p.r * w;
                acc.g += p.g * w;
                acc.b += p.b * w;
                acc.a += p.a * w;
                w_sum += w;
            }
            if w_sum.abs() > 1e-6 {
                let inv = 1.0 / w_sum;
                temp.pixels[dst_row + dx as usize] = LinearPremul {
                    r: (acc.r * inv).max(0.0),
                    g: (acc.g * inv).max(0.0),
                    b: (acc.b * inv).max(0.0),
                    a: (acc.a * inv).clamp(0.0, 1.0),
                };
            }
        }
    }

    // Pass 2: Vertical Lanczos-3
    let mut out = LinearImage::new(dst_w, dst_h, LinearPremul::TRANSPARENT);
    let ratio_y = src.height as f32 / dst_h as f32;
    for dy in 0..dst_h {
        let center = (dy as f32 + 0.5) * ratio_y - 0.5;
        let j0 = (center.floor() as i32) - 2;
        let j1 = (center.floor() as i32) + 3;
        let dst_row = (dy * dst_w) as usize;
        for dx in 0..dst_w as usize {
            let mut acc = LinearPremul::TRANSPARENT;
            let mut w_sum = 0.0f32;
            for sy in j0..=j1 {
                let w = lanczos3(center - sy as f32);
                if w.abs() < 1e-6 {
                    continue;
                }
                let cy = sy.clamp(0, src.height as i32 - 1) as usize;
                let p = temp.pixels[cy * dst_w as usize + dx];
                acc.r += p.r * w;
                acc.g += p.g * w;
                acc.b += p.b * w;
                acc.a += p.a * w;
                w_sum += w;
            }
            if w_sum.abs() > 1e-6 {
                let inv = 1.0 / w_sum;
                out.pixels[dst_row + dx] = LinearPremul {
                    r: (acc.r * inv).max(0.0),
                    g: (acc.g * inv).max(0.0),
                    b: (acc.b * inv).max(0.0),
                    a: (acc.a * inv).clamp(0.0, 1.0),
                };
            }
        }
    }

    apply_ai_super_res_detail(&mut out, cfg.sharpen_amount, cfg.denoise_strength);
    out
}

/// インプレースのエッジ誘導ディテール復元＋平坦部デノイズ（クリップエフェクト `"ai_super_res"` からも共用）
pub fn apply_ai_super_res_detail(
    image: &mut LinearImage,
    sharpen_amount: f32,
    denoise_strength: f32,
) {
    let w = image.width as usize;
    let h = image.height as usize;
    if w < 3 || h < 3 {
        return;
    }
    let sharp = sharpen_amount.clamp(0.0, 2.0);
    let denoise = denoise_strength.clamp(0.0, 1.0);
    if sharp < 1e-3 && denoise < 1e-3 {
        return;
    }

    let orig = image.pixels.clone();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let idx = y * w + x;
            let c = orig[idx];
            if c.a <= 1e-5 {
                continue;
            }
            let l = orig[idx - 1];
            let r = orig[idx + 1];
            let u = orig[idx - w];
            let d = orig[idx + w];

            let blur_r = (l.r + r.r + u.r + d.r) * 0.25;
            let blur_g = (l.g + r.g + u.g + d.g) * 0.25;
            let blur_b = (l.b + r.b + u.b + d.b) * 0.25;

            let grad = ((r.luma() - l.luma()).abs() + (d.luma() - u.luma()).abs()).min(1.0);
            // エッジ領域はハロー抑止付きアンシャープ復元、平坦部は軽いデノイズ
            let edge_gate = ((grad - 0.025) * 8.0).clamp(0.0, 1.0);
            let flat_gate = 1.0 - edge_gate;

            let detail_r = (c.r - blur_r).clamp(-0.25, 0.25);
            let detail_g = (c.g - blur_g).clamp(-0.25, 0.25);
            let detail_b = (c.b - blur_b).clamp(-0.25, 0.25);

            let mut nr = c.r + detail_r * sharp * edge_gate;
            let mut ng = c.g + detail_g * sharp * edge_gate;
            let mut nb = c.b + detail_b * sharp * edge_gate;

            if denoise > 1e-3 && flat_gate > 0.05 {
                let mix = denoise * flat_gate * 0.5;
                nr = nr * (1.0 - mix) + blur_r * mix;
                ng = ng * (1.0 - mix) + blur_g * mix;
                nb = nb * (1.0 - mix) + blur_b * mix;
            }

            image.pixels[idx] = LinearPremul {
                r: nr.max(0.0),
                g: ng.max(0.0),
                b: nb.max(0.0),
                a: c.a,
            };
        }
    }
}

// =========================================================================
// 4. Optical Flow Frame Interpolation (Smooth Slow Motion)
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpticalFlowConfig {
    /// ブロックサイズ px (4..=16, 既定 8)
    pub block_size: usize,
    /// 最大探索半径 px (4..=24, 既定 10)
    pub search_radius: i32,
    /// 前後フロー整合性に基づくオクルージョン保護を有効化
    pub occlusion_aware: bool,
}

impl Default for OpticalFlowConfig {
    fn default() -> Self {
        Self {
            block_size: 8,
            search_radius: 10,
            occlusion_aware: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MotionVector {
    pub dx: f32,
    pub dy: f32,
}

/// ブロックマッチング SAD ＋ 3×3 空間正則化による密ブロックオプティカルフロー推定（A -> B）
pub fn estimate_optical_flow(
    frame_a: &LinearImage,
    frame_b: &LinearImage,
    cfg: &OpticalFlowConfig,
) -> (usize, usize, Vec<MotionVector>) {
    let w = frame_a.width.min(frame_b.width) as usize;
    let h = frame_a.height.min(frame_b.height) as usize;
    let bs = cfg.block_size.clamp(4, 16);
    let gw = w.div_ceil(bs).max(1);
    let gh = h.div_ceil(bs).max(1);
    if w < 4 || h < 4 {
        return (gw, gh, vec![MotionVector::default(); gw * gh]);
    }

    let luma_a: Vec<f32> = frame_a.pixels.iter().map(|p| p.luma()).collect();
    let luma_b: Vec<f32> = frame_b.pixels.iter().map(|p| p.luma()).collect();
    let wa = frame_a.width as usize;
    let wb = frame_b.width as usize;

    let radius = cfg.search_radius.clamp(2, 24);
    let mut raw_flow = vec![MotionVector::default(); gw * gh];

    for by in 0..gh {
        let y0 = by * bs;
        let y1 = (y0 + bs).min(h);
        for bx in 0..gw {
            let x0 = bx * bs;
            let x1 = (x0 + bs).min(w);

            // ステップ 2 の粗探索 → 周囲 ±1 精密探索で高速化
            let mut best_dx = 0i32;
            let mut best_dy = 0i32;
            let mut best_cost = f32::INFINITY;

            let eval_sad = |dx: i32, dy: i32| -> f32 {
                let mut sum = 0.0f32;
                let mut cnt = 0u32;
                for y in y0..y1 {
                    let ny = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                    for x in x0..x1 {
                        let nx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                        sum += (luma_a[y * wa + x] - luma_b[ny * wb + nx]).abs();
                        cnt += 1;
                    }
                }
                // 平坦部での誤移動を抑える微小ゼロ移動バイアス
                (sum / cnt.max(1) as f32) + (dx.abs() + dy.abs()) as f32 * 0.0005
            };

            let coarse_step = if radius >= 6 { 2 } else { 1 };
            let mut dy = -radius;
            while dy <= radius {
                let mut dx = -radius;
                while dx <= radius {
                    let c = eval_sad(dx, dy);
                    if c < best_cost {
                        best_cost = c;
                        best_dx = dx;
                        best_dy = dy;
                    }
                    dx += coarse_step;
                }
                dy += coarse_step;
            }

            if coarse_step > 1 {
                let cx = best_dx;
                let cy = best_dy;
                for oy in -1..=1 {
                    for ox in -1..=1 {
                        let ndx = (cx + ox).clamp(-radius, radius);
                        let ndy = (cy + oy).clamp(-radius, radius);
                        let c = eval_sad(ndx, ndy);
                        if c < best_cost {
                            best_cost = c;
                            best_dx = ndx;
                            best_dy = ndy;
                        }
                    }
                }
            }

            // サブピクセル放物線補間
            let c_l = eval_sad((best_dx - 1).clamp(-radius, radius), best_dy);
            let c_r = eval_sad((best_dx + 1).clamp(-radius, radius), best_dy);
            let sub_x = parabolic_offset(c_l, best_cost, c_r);
            let c_u = eval_sad(best_dx, (best_dy - 1).clamp(-radius, radius));
            let c_d = eval_sad(best_dx, (best_dy + 1).clamp(-radius, radius));
            let sub_y = parabolic_offset(c_u, best_cost, c_d);

            raw_flow[by * gw + bx] = MotionVector {
                dx: best_dx as f32 + sub_x,
                dy: best_dy as f32 + sub_y,
            };
        }
    }

    // 3×3 空間スムージング（孤立外れ値フローを抑制）
    let mut smooth = vec![MotionVector::default(); gw * gh];
    for by in 0..gh {
        for bx in 0..gw {
            let mut sx = 0.0f32;
            let mut sy = 0.0f32;
            let mut sw = 0.0f32;
            for oy in -1..=1i32 {
                let ny = (by as i32 + oy).clamp(0, gh as i32 - 1) as usize;
                for ox in -1..=1i32 {
                    let nx = (bx as i32 + ox).clamp(0, gw as i32 - 1) as usize;
                    let wgt = if ox == 0 && oy == 0 { 2.0 } else { 1.0 };
                    let mv = raw_flow[ny * gw + nx];
                    sx += mv.dx * wgt;
                    sy += mv.dy * wgt;
                    sw += wgt;
                }
            }
            smooth[by * gw + bx] = MotionVector {
                dx: sx / sw,
                dy: sy / sw,
            };
        }
    }

    (gw, gh, smooth)
}

#[inline]
fn parabolic_offset(c_prev: f32, c_mid: f32, c_next: f32) -> f32 {
    let denom = 2.0 * (c_prev - 2.0 * c_mid + c_next);
    if denom.abs() < 1e-5 {
        0.0
    } else {
        ((c_prev - c_next) / denom).clamp(-0.5, 0.5)
    }
}

/// 2 フレーム間 (`frame_a`, `frame_b`) の任意時刻 `t ∈ [0.0, 1.0]` を双方向オプティカルフロー補償で補間合成する
pub fn interpolate_frame_optical_flow(
    frame_a: &LinearImage,
    frame_b: &LinearImage,
    t: f32,
    cfg: &OpticalFlowConfig,
) -> LinearImage {
    let t = t.clamp(0.0, 1.0);
    if t <= 1e-4 {
        return frame_a.clone();
    }
    if t >= 1.0 - 1e-4 {
        return frame_b.clone();
    }
    let w = frame_a.width.min(frame_b.width);
    let h = frame_a.height.min(frame_b.height);
    if w < 4 || h < 4 {
        return frame_a.clone();
    }

    let bs = cfg.block_size.clamp(4, 16) as f32;
    let (gw, gh, flow_ab) = estimate_optical_flow(frame_a, frame_b, cfg);
    let (_, _, flow_ba) = estimate_optical_flow(frame_b, frame_a, cfg);

    let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    for y in 0..h {
        let by = ((y as f32 + 0.5) / bs - 0.5).clamp(0.0, (gh - 1) as f32);
        let row = (y * w) as usize;
        for x in 0..w {
            let bx = ((x as f32 + 0.5) / bs - 0.5).clamp(0.0, (gw - 1) as f32);
            let f_ab = sample_flow_bilinear(&flow_ab, gw, gh, bx, by);
            let f_ba = sample_flow_bilinear(&flow_ba, gw, gh, bx, by);

            // 中間時刻 t のピクセル (x, y) から A 側へ -t*f_ab, B 側へ -(1-t)*f_ba を逆ワープ参照
            let ax = x as f32 - t * f_ab.dx;
            let ay = y as f32 - t * f_ab.dy;
            let b_x = x as f32 - (1.0 - t) * f_ba.dx;
            let b_y = y as f32 - (1.0 - t) * f_ba.dy;

            let pa = sample_linear_bilinear(frame_a, ax, ay);
            let pb = sample_linear_bilinear(frame_b, b_x, b_y);

            // オクルージョン検出: A->B と B->A が矛盾する場合は輝度差の小さい側へ自然にフェード
            let mut w_b = t;
            if cfg.occlusion_aware {
                let fb_err = (f_ab.dx + f_ba.dx).hypot(f_ab.dy + f_ba.dy);
                if fb_err > 4.0 {
                    // 矛盾時は直線クロスフェード寄りへ緩和してゴーストを防止
                    let raw_a = frame_a.pixels[(y * frame_a.width + x) as usize];
                    let raw_b = frame_b.pixels[(y * frame_b.width + x) as usize];
                    let mix = ((fb_err - 4.0) / 8.0).clamp(0.0, 0.7);
                    let warped = lerp_premul(pa, pb, w_b);
                    let blended = lerp_premul(raw_a, raw_b, t);
                    out.pixels[row + x as usize] = lerp_premul(warped, blended, mix);
                    continue;
                }
                let _ = &mut w_b;
            }
            out.pixels[row + x as usize] = lerp_premul(pa, pb, w_b);
        }
    }
    out
}

fn sample_flow_bilinear(
    flow: &[MotionVector],
    gw: usize,
    gh: usize,
    x: f32,
    y: f32,
) -> MotionVector {
    let x0 = (x.floor() as usize).min(gw - 1);
    let y0 = (y.floor() as usize).min(gh - 1);
    let x1 = (x0 + 1).min(gw - 1);
    let y1 = (y0 + 1).min(gh - 1);
    let fx = (x - x0 as f32).clamp(0.0, 1.0);
    let fy = (y - y0 as f32).clamp(0.0, 1.0);

    let f00 = flow[y0 * gw + x0];
    let f10 = flow[y0 * gw + x1];
    let f01 = flow[y1 * gw + x0];
    let f11 = flow[y1 * gw + x1];

    let top_x = f00.dx + (f10.dx - f00.dx) * fx;
    let top_y = f00.dy + (f10.dy - f00.dy) * fx;
    let bot_x = f01.dx + (f11.dx - f01.dx) * fx;
    let bot_y = f01.dy + (f11.dy - f01.dy) * fx;
    MotionVector {
        dx: top_x + (bot_x - top_x) * fy,
        dy: top_y + (bot_y - top_y) * fy,
    }
}

fn sample_linear_bilinear(img: &LinearImage, x: f32, y: f32) -> LinearPremul {
    let cx = x.clamp(0.0, (img.width - 1) as f32);
    let cy = y.clamp(0.0, (img.height - 1) as f32);
    let x0 = cx.floor() as u32;
    let y0 = cy.floor() as u32;
    let x1 = (x0 + 1).min(img.width - 1);
    let y1 = (y0 + 1).min(img.height - 1);
    let fx = cx - x0 as f32;
    let fy = cy - y0 as f32;

    let w = img.width;
    let p00 = img.pixels[(y0 * w + x0) as usize];
    let p10 = img.pixels[(y0 * w + x1) as usize];
    let p01 = img.pixels[(y1 * w + x0) as usize];
    let p11 = img.pixels[(y1 * w + x1) as usize];

    let top = lerp_premul(p00, p10, fx);
    let bot = lerp_premul(p01, p11, fx);
    lerp_premul(top, bot, fy)
}

#[inline]
fn lerp_premul(a: LinearPremul, b: LinearPremul, t: f32) -> LinearPremul {
    let u = 1.0 - t;
    LinearPremul {
        r: a.r * u + b.r * t,
        g: a.g * u + b.g * t,
        b: a.b * u + b.b * t,
        a: a.a * u + b.a * t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::{ProjectSettings, RgbaColor, ShapeKind, TimelineItem};

    #[test]
    fn ai_segmentation_separates_non_green_foreground_subject_from_background() {
        // 濃い青系背景の中央に暖かい肌色・赤系の被写体を配置（グリーンバック不使用）
        let bg = LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.12,
            g: 0.18,
            b: 0.36,
            a: 1.0,
        });
        let subject = LinearPremul::from_srgb_straight(RgbaColor {
            r: 0.90,
            g: 0.68,
            b: 0.56,
            a: 1.0,
        });
        let mut img = LinearImage::new(64, 64, bg);
        for y in 18..48 {
            for x in 20..44 {
                img.pixels[(y * 64 + x) as usize] = subject;
            }
        }

        apply_ai_segmentation(&mut img, &SegmentationConfig::default());

        let corner_alpha = img.pixels[2 * 64 + 2].a;
        let center_alpha = img.pixels[32 * 64 + 32].a;
        assert!(
            corner_alpha < 0.15,
            "background corner should be removed, got alpha={corner_alpha}"
        );
        assert!(
            center_alpha > 0.85,
            "foreground subject center should be preserved, got alpha={center_alpha}"
        );
    }

    #[test]
    fn auto_reframe_tracks_moving_subject_and_builds_smooth_keyframes() {
        let mut project = Project::new("ReframeTest", ProjectSettings::default());
        let scene_id = project.active_scene_id;
        // 右側 (x=78% 付近) に明るい肌色の被写体を配置
        let mut item = TimelineItem::new(
            "Subject",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 220.0,
                height: 220.0,
                color: RgbaColor {
                    r: 0.92,
                    g: 0.72,
                    b: 0.58,
                    a: 1.0,
                },
            },
        );
        item.transform.x = 1400.0;
        item.transform.y = 320.0;
        project.scenes[0].layers[0].items.push(item);

        let mut graph = RenderGraph::new();
        let cfg = AutoReframeConfig::default();
        let samples = analyze_scene_auto_reframe(&mut graph, &project, scene_id, &cfg);
        assert!(!samples.is_empty());
        assert!(
            samples[0].focus_x_norm > 0.62,
            "subject on right side should pull focus right: {}",
            samples[0].focus_x_norm
        );

        let cmds = build_auto_reframe_commands(&project.scenes[0], &cfg, &samples);
        assert!(cmds.len() >= 3);
    }

    #[test]
    fn super_resolution_upscales_2x_and_preserves_edges() {
        let mut src = LinearImage::new(16, 12, LinearPremul::from_srgb_straight(RgbaColor::BLACK));
        for y in 4..8 {
            for x in 4..12 {
                src.pixels[(y * 16 + x) as usize] =
                    LinearPremul::from_srgb_straight(RgbaColor::WHITE);
            }
        }
        let up = upscale_super_resolution(&src, &SuperResolutionConfig::default());
        assert_eq!(up.width, 32);
        assert_eq!(up.height, 24);
        assert!(up.pixels[12 * 32 + 16].luma() > 0.8);
        assert!(up.pixels[33].luma() < 0.1);
    }

    #[test]
    fn optical_flow_interpolates_translating_object_to_midpoint() {
        let black = LinearPremul::from_srgb_straight(RgbaColor::BLACK);
        let white = LinearPremul::from_srgb_straight(RgbaColor::WHITE);
        let mut frame_a = LinearImage::new(64, 48, black);
        let mut frame_b = LinearImage::new(64, 48, black);
        // Frame A: x in [12..24], Frame B: x in [20..32] (+8px shift)
        for y in 16..32 {
            for x in 12..24 {
                frame_a.pixels[(y * 64 + x) as usize] = white;
            }
            for x in 20..32 {
                frame_b.pixels[(y * 64 + x) as usize] = white;
            }
        }

        let mid =
            interpolate_frame_optical_flow(&frame_a, &frame_b, 0.5, &OpticalFlowConfig::default());
        // 中間フレームでは x=22（元の20..24の重なり＋移動先16..28の中央）が明るく、遠くの x=4 は暗い
        assert!(mid.pixels[24 * 64 + 22].luma() > 0.65);
        assert!(mid.pixels[24 * 64 + 4].luma() < 0.05);
    }
}
