//! v11.0（#61〜#64）Zero-Copy GPU Engine, Real-Time 8K & Native Studio Performance コアモデル＆アルゴリズム
//!
//! - #61: 完全ゼロコピー GPU レンダリンググラフ設定＆VRAM 依存 DAG プランナー
//! - #62: 適応解像度プロキシ自動切替（EMA ヒステリシス制御）＆ 複雑度ベーススマート事前レンダリングスケジューラ
//! - #63: 大規模プロジェクト超軽量化（高速バイナリスナップショット `project.fastbin` + FIPS 180-4 SHA-256 検証・構造共有レイヤー差分・メモリ監査＆コンパクション）
//! - #64: リアルタイム・パフォーマンスプロファイラ HUD データモデル・ローリング統計（p95/p99）・自動ボトルネック診断＆1-Undo 最適化ビルダー

use crate::command::EditCommand;
use crate::folder::ProjectFolder;
use crate::project::{AssetId, ItemId, ItemKind, Project, Scene, SceneId};
use crate::registry::sha256_hex;
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;

// ============================================================================
// #61: 完全ゼロコピー GPU レンダリンググラフ（VRAM 依存 DAG プランナー）
// ============================================================================

/// v11.0 #61: シーン単位のゼロコピー GPU レンダリンググラフ設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuRenderGraphConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// レイヤー合成・色調補正・360° 逆射影等を VRAM バッファ常駐のまま 1 コマンドバッファで連結実行する
    #[serde(default = "default_true")]
    pub prefer_zero_copy_vram: bool,
    /// GPU テクスチャ／ストレージバッファプールの上限（MiB）
    #[serde(default = "default_max_vram_mib")]
    pub max_vram_textures_mib: u32,
    /// 8K（7680×4320）等の高解像度時にストリップ分割タイル実行を許可する
    #[serde(default = "default_true")]
    pub allow_8k_tiling: bool,
    /// ポストプロセス（3D LUT / ACEScg / 光学 VFX / 360°）を GPU 上で連続連結する
    #[serde(default = "default_true")]
    pub chain_post_fx_on_gpu: bool,
    /// プレビュー HUD 上に `GPU DIRECT` バッジを表示する
    #[serde(default = "default_true")]
    pub show_gpu_direct_badge: bool,
}

fn default_true() -> bool {
    true
}

fn default_max_vram_mib() -> u32 {
    1024
}

impl Default for GpuRenderGraphConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prefer_zero_copy_vram: true,
            max_vram_textures_mib: 1024,
            allow_8k_tiling: true,
            chain_post_fx_on_gpu: true,
            show_gpu_direct_badge: true,
        }
    }
}

/// GPU レンダリンググラフ上のパス種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuGraphPassKind {
    LayerComposite,
    ColorGradeLut,
    AcesColorMgmt,
    OpticalVfx,
    CinemaFinishing,
    Spatial360,
    FluidAdvection,
}

impl GpuGraphPassKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::LayerComposite => "Layer Composite (WGSL f16)",
            Self::ColorGradeLut => "3-Way Color Grade & 3D LUT",
            Self::AcesColorMgmt => "ACEScg / OCIO Color Mgmt",
            Self::OpticalVfx => "Cinema Optical VFX",
            Self::CinemaFinishing => "35mm Cinema Finishing",
            Self::Spatial360 => "360° OverCapture / Tiny Planet",
            Self::FluidAdvection => "2D Navier-Stokes Fluid Advect",
        }
    }

    pub fn is_gpu_wgsl_accelerated(self) -> bool {
        matches!(
            self,
            Self::LayerComposite
                | Self::ColorGradeLut
                | Self::AcesColorMgmt
                | Self::Spatial360
                | Self::FluidAdvection
        )
    }
}

/// GPU レンダリンググラフ DAG の単一パスノード
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuGraphPassNode {
    pub pass_id: usize,
    pub kind: GpuGraphPassKind,
    pub label: String,
    pub estimated_vram_bytes: u64,
    pub zero_copy_eligible: bool,
    pub depends_on: Vec<usize>,
}

/// シーン全体から静的に構築されるゼロコピー GPU 実行計画（トポロジカル DAG）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuGraphExecutionPlan {
    pub width: u32,
    pub height: u32,
    pub vram_budget_bytes: u64,
    pub nodes: Vec<GpuGraphPassNode>,
    pub zero_copy_chain_len: usize,
    pub total_estimated_vram_bytes: u64,
    pub cpu_readbacks_saved: usize,
    pub estimated_pcie_bytes_saved_per_frame: u64,
    pub requires_8k_tiling: bool,
}

/// シーンのアクティブなエフェクト・レイヤー・ポストプロセス構成から
/// VRAM 常駐ゼロコピー GPU レンダリンググラフ DAG 実行計画を生成する。
pub fn build_gpu_render_graph_plan(
    scene: &Scene,
    target_w: u32,
    target_h: u32,
) -> GpuGraphExecutionPlan {
    let w = target_w.max(1);
    let h = target_h.max(1);
    let surface_f16_bytes = (w as u64).saturating_mul(h as u64).saturating_mul(8);
    let cfg = &scene.gpu_graph;
    let vram_budget_bytes = u64::from(cfg.max_vram_textures_mib.clamp(64, 16_384)) * 1024 * 1024;
    let requires_8k_tiling = cfg.allow_8k_tiling && (w > 3840 || h > 2160);

    let mut nodes: Vec<GpuGraphPassNode> = Vec::new();
    let mut prev_id: Option<usize> = None;

    let has_fluid = scene.layers.iter().any(|l| {
        l.visible
            && l.items
                .iter()
                .any(|it| matches!(it.kind, ItemKind::FluidSim { .. }))
    });
    if has_fluid {
        let id = nodes.len();
        nodes.push(GpuGraphPassNode {
            pass_id: id,
            kind: GpuGraphPassKind::FluidAdvection,
            label: GpuGraphPassKind::FluidAdvection.label().to_string(),
            estimated_vram_bytes: 128 * 128 * 4 * 4,
            zero_copy_eligible: cfg.enabled && cfg.prefer_zero_copy_vram,
            depends_on: Vec::new(),
        });
        prev_id = Some(id);
    }

    // ベース合成パス（Ping-Pong 2 面 = 2 * surface_f16_bytes）
    let comp_id = nodes.len();
    nodes.push(GpuGraphPassNode {
        pass_id: comp_id,
        kind: GpuGraphPassKind::LayerComposite,
        label: format!(
            "{} ({} layers)",
            GpuGraphPassKind::LayerComposite.label(),
            scene.layers.iter().filter(|l| l.visible).count()
        ),
        estimated_vram_bytes: surface_f16_bytes.saturating_mul(2),
        zero_copy_eligible: cfg.enabled && cfg.prefer_zero_copy_vram,
        depends_on: prev_id.into_iter().collect(),
    });
    prev_id = Some(comp_id);

    let mut push_post = |kind: GpuGraphPassKind, extra_bytes: u64| {
        let id = nodes.len();
        let eligible = cfg.enabled
            && cfg.prefer_zero_copy_vram
            && cfg.chain_post_fx_on_gpu
            && kind.is_gpu_wgsl_accelerated();
        nodes.push(GpuGraphPassNode {
            pass_id: id,
            kind,
            label: kind.label().to_string(),
            estimated_vram_bytes: extra_bytes,
            zero_copy_eligible: eligible,
            depends_on: prev_id.into_iter().collect(),
        });
        prev_id = Some(id);
    };

    if scene.color_grade.enabled {
        push_post(GpuGraphPassKind::ColorGradeLut, 33 * 33 * 33 * 16);
    }
    if scene.color_management.enabled {
        push_post(GpuGraphPassKind::AcesColorMgmt, 256);
    }
    if scene.optical_vfx.is_active() {
        push_post(GpuGraphPassKind::OpticalVfx, surface_f16_bytes);
    }
    if scene.cinema_finishing.enabled {
        push_post(GpuGraphPassKind::CinemaFinishing, surface_f16_bytes);
    }
    if scene.spatial_video.enabled {
        push_post(GpuGraphPassKind::Spatial360, surface_f16_bytes);
    }

    let zero_copy_chain_len = nodes.iter().filter(|n| n.zero_copy_eligible).count();
    let cpu_readbacks_saved = zero_copy_chain_len.saturating_sub(1);
    let total_estimated_vram_bytes = nodes
        .iter()
        .map(|n| n.estimated_vram_bytes)
        .fold(0u64, u64::saturating_add);
    // 各中間リードバック＆再アップロード（往復 2×surface_f16_bytes）を削減
    let estimated_pcie_bytes_saved_per_frame =
        (cpu_readbacks_saved as u64).saturating_mul(surface_f16_bytes.saturating_mul(2));

    GpuGraphExecutionPlan {
        width: w,
        height: h,
        vram_budget_bytes,
        nodes,
        zero_copy_chain_len,
        total_estimated_vram_bytes,
        cpu_readbacks_saved,
        estimated_pcie_bytes_saved_per_frame,
        requires_8k_tiling,
    }
}

// ============================================================================
// #62: 適応解像度プロキシ自動切替 ＆ スマート事前レンダリングスケジューラ
// ============================================================================

/// v11.0 #62: 再生負荷に応じた適応解像度スケーリング＆スマート事前レンダリング設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptivePlaybackConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 目標再生フレームレート（例: 60.0 fps → 予算 16.67ms）
    #[serde(default = "default_target_fps")]
    pub target_fps: f32,
    /// 最小縮小倍率（1 = 等倍）
    #[serde(default = "default_one_u32")]
    pub min_scale_divisor: u32,
    /// 最大縮小倍率（2 = 1/2, 4 = 1/4）
    #[serde(default = "default_four_u32")]
    pub max_scale_divisor: u32,
    /// フレーム時間が `予算 * drop_threshold_ratio` を超過し続けたら解像度を段階的に下げる
    #[serde(default = "default_drop_ratio")]
    pub drop_threshold_ratio: f32,
    /// フレーム時間が `予算 * recover_threshold_ratio` 以下で安定したら解像度を段階的に戻す
    #[serde(default = "default_recover_ratio")]
    pub recover_threshold_ratio: f32,
    /// 再生停止時に即座にフル解像度（1/1）へ復帰させる
    #[serde(default = "default_true")]
    pub snap_full_res_on_pause: bool,
    /// アイドル時に重いクリップ区間から優先して事前レンダリングする
    #[serde(default = "default_true")]
    pub smart_prerender_enabled: bool,
    /// スマート事前レンダリングの最大キューフレーム数
    #[serde(default = "default_prerender_budget")]
    pub prerender_budget_frames: usize,
}

fn default_target_fps() -> f32 {
    60.0
}
fn default_one_u32() -> u32 {
    1
}
fn default_four_u32() -> u32 {
    4
}
fn default_drop_ratio() -> f32 {
    1.15
}
fn default_recover_ratio() -> f32 {
    0.65
}
fn default_prerender_budget() -> usize {
    120
}

impl Default for AdaptivePlaybackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_fps: 60.0,
            min_scale_divisor: 1,
            max_scale_divisor: 4,
            drop_threshold_ratio: 1.15,
            recover_threshold_ratio: 0.65,
            snap_full_res_on_pause: true,
            smart_prerender_enabled: true,
            prerender_budget_frames: 120,
        }
    }
}

/// 適応解像度コントローラのランタイム状態（指数移動平均＋ヒステリシス）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveScaleState {
    pub current_divisor: u32,
    pub ema_frame_ms: f32,
    pub consecutive_slow: u32,
    pub consecutive_fast: u32,
    pub total_downscales: u64,
    pub total_upscales: u64,
}

impl Default for AdaptiveScaleState {
    fn default() -> Self {
        Self {
            current_divisor: 1,
            ema_frame_ms: 0.0,
            consecutive_slow: 0,
            consecutive_fast: 0,
            total_downscales: 0,
            total_upscales: 0,
        }
    }
}

impl AdaptiveScaleState {
    /// 実測フレーム描画時間（ms）と再生状態から次フレームの適応縮小倍率（1 / 2 / 4）を決定する
    pub fn record_frame_time(
        &mut self,
        frame_ms: f32,
        is_playing: bool,
        cfg: &AdaptivePlaybackConfig,
    ) -> u32 {
        let min_div = cfg.min_scale_divisor.clamp(1, 4);
        let max_div = cfg.max_scale_divisor.clamp(min_div, 4);
        if !cfg.enabled {
            self.current_divisor = min_div;
            return self.current_divisor;
        }
        if !is_playing && cfg.snap_full_res_on_pause {
            self.current_divisor = min_div;
            self.consecutive_slow = 0;
            self.consecutive_fast = 0;
            return self.current_divisor;
        }

        let clamped_ms = frame_ms.clamp(0.0, 2000.0);
        if self.ema_frame_ms <= 0.0 {
            self.ema_frame_ms = clamped_ms;
        } else {
            self.ema_frame_ms = self.ema_frame_ms * 0.7 + clamped_ms * 0.3;
        }

        let budget_ms = 1000.0 / cfg.target_fps.clamp(15.0, 240.0);
        let drop_ms = budget_ms * cfg.drop_threshold_ratio.clamp(1.0, 3.0);
        let recover_ms = budget_ms * cfg.recover_threshold_ratio.clamp(0.2, 0.95);

        if self.ema_frame_ms > drop_ms {
            self.consecutive_slow += 1;
            self.consecutive_fast = 0;
            if self.consecutive_slow >= 3 && self.current_divisor < max_div {
                self.current_divisor = (self.current_divisor * 2).min(max_div);
                self.consecutive_slow = 0;
                self.total_downscales += 1;
            }
        } else if self.ema_frame_ms < recover_ms {
            self.consecutive_fast += 1;
            self.consecutive_slow = 0;
            if self.consecutive_fast >= 6 && self.current_divisor > min_div {
                self.current_divisor = (self.current_divisor / 2).max(min_div);
                self.consecutive_fast = 0;
                self.total_upscales += 1;
            }
        } else {
            self.consecutive_slow = 0;
            self.consecutive_fast = 0;
        }

        self.current_divisor = self.current_divisor.clamp(min_div, max_div);
        self.current_divisor
    }
}

/// タイムライン上の高負荷（複雑度ホットスポット）区間
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplexityHotspotSegment {
    pub start_frame: i64,
    pub end_frame: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub complexity_score: f32,
    pub reasons: Vec<String>,
}

/// スマートバックグラウンド事前レンダリング計画
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartPrerenderPlan {
    pub total_frames: i64,
    pub hotspots: Vec<ComplexityHotspotSegment>,
    pub prioritized_frames: Vec<i64>,
}

/// シーンの 1 秒刻み複雑度（流体・クロス・3D シャドウ/IBL・光学 VFX・360°・モーションブラー・エフェクト段数・アクティブレイヤー数）
/// を解析し、高負荷区間から優先的に事前レンダリングするスケジュールを生成する。
pub fn plan_smart_prerender_schedule(
    scene: &Scene,
    cfg: &AdaptivePlaybackConfig,
) -> SmartPrerenderPlan {
    let fps = scene.fps.as_f64().max(1.0);
    let dur_sec = scene.total_duration().as_seconds().max(0.0);
    let total_frames = (dur_sec * fps).ceil() as i64;
    if total_frames <= 0 || !cfg.smart_prerender_enabled {
        return SmartPrerenderPlan {
            total_frames: total_frames.max(0),
            hotspots: Vec::new(),
            prioritized_frames: Vec::new(),
        };
    }

    let bucket_count = (dur_sec.ceil() as usize).clamp(1, 3600);
    let mut hotspots: Vec<ComplexityHotspotSegment> = Vec::new();

    for b in 0..bucket_count {
        let s0 = b as f64;
        let s1 = (s0 + 1.0).min(dur_sec.max(0.1));
        let mid_t = RationalTime::from_seconds((s0 + s1) * 0.5);

        let mut score = 0.0f32;
        let mut reasons: Vec<String> = Vec::new();
        let mut active_items = 0usize;

        for layer in &scene.layers {
            if !layer.visible {
                continue;
            }
            for item in &layer.items {
                if !item.contains(mid_t) {
                    continue;
                }
                active_items += 1;
                score += 1.0;
                let fx_cnt = item.effect_chain.iter().filter(|e| e.enabled).count();
                if fx_cnt > 0 {
                    score += fx_cnt as f32 * 1.4;
                }
                if !item.masks.is_empty() || item.roto_brush.enabled {
                    score += 2.2;
                    push_unique(&mut reasons, "Mask/Roto");
                }
                if item.warp_displace.enabled || item.corner_pin.enabled {
                    score += 2.5;
                    push_unique(&mut reasons, "Warp/CornerPin");
                }
                match &item.kind {
                    ItemKind::FluidSim { .. } => {
                        score += 6.0;
                        push_unique(&mut reasons, "2D Fluid Sim");
                    }
                    ItemKind::ClothSim { .. } => {
                        score += 4.5;
                        push_unique(&mut reasons, "PBD Cloth Sim");
                    }
                    ItemKind::Mesh3D { .. } => {
                        score += if scene.lighting_3d_adv.shadow_map_enabled
                            || scene.lighting_3d_adv.hdri_ibl_enabled
                        {
                            5.5
                        } else {
                            3.5
                        };
                        push_unique(&mut reasons, "3D PBR/Shadow");
                    }
                    ItemKind::VtuberAvatar { .. } => {
                        score += 3.5;
                        push_unique(&mut reasons, "VTuber Mesh");
                    }
                    ItemKind::ParticleEmitter { .. } => {
                        score += 2.5;
                        push_unique(&mut reasons, "Particles");
                    }
                    ItemKind::Precomp { .. } => {
                        score += 3.0;
                        push_unique(&mut reasons, "Precomp");
                    }
                    _ => {}
                }
            }
        }

        if active_items == 0 {
            continue;
        }
        if scene.motion_blur.enabled && scene.motion_blur.shutter_angle_deg > 0.5 {
            score += scene.motion_blur.preview_samples.max(1) as f32 * 1.5;
            push_unique(&mut reasons, "MotionBlur");
        }
        if scene.optical_vfx.is_active() {
            score += 4.5;
            push_unique(&mut reasons, "OpticalVFX");
        }
        if scene.spatial_video.enabled {
            score += 3.5;
            push_unique(&mut reasons, "360°/Spatial3D");
        }
        if scene.color_management.enabled {
            score += 1.5;
            push_unique(&mut reasons, "ACEScg");
        }
        if scene.cinema_finishing.enabled {
            score += 2.0;
            push_unique(&mut reasons, "35mm Finishing");
        }

        if score >= 3.5 {
            let start_frame = (s0 * fps).floor() as i64;
            let end_frame = ((s1 * fps).ceil() as i64)
                .min(total_frames)
                .max(start_frame + 1);
            hotspots.push(ComplexityHotspotSegment {
                start_frame,
                end_frame,
                start_sec: s0,
                end_sec: s1,
                complexity_score: (score * 10.0).round() * 0.1,
                reasons,
            });
        }
    }

    // 負荷が高い区間から優先ソート（同スコアならタイムライン前方優先）
    hotspots.sort_by(|a, b| {
        b.complexity_score
            .total_cmp(&a.complexity_score)
            .then_with(|| a.start_frame.cmp(&b.start_frame))
    });

    let budget = cfg.prerender_budget_frames.clamp(1, 2000);
    let mut prioritized_frames = Vec::with_capacity(budget);
    for seg in &hotspots {
        let len = (seg.end_frame - seg.start_frame).max(1);
        let stride = (len / 12).max(1);
        let mut f = seg.start_frame;
        while f < seg.end_frame && prioritized_frames.len() < budget {
            prioritized_frames.push(f);
            f += stride;
        }
        if prioritized_frames.len() >= budget {
            break;
        }
    }

    SmartPrerenderPlan {
        total_frames,
        hotspots,
        prioritized_frames,
    }
}

fn push_unique(list: &mut Vec<String>, label: &str) {
    if !list.iter().any(|s| s == label) {
        list.push(label.to_string());
    }
}

// ============================================================================
// #63: 大規模プロジェクト超軽量化（FastBin スナップショット・構造共有差分・メモリ監査）
// ============================================================================

const FASTBIN_MAGIC: [u8; 8] = *b"VEFAST11";
const FASTBIN_VERSION: u32 = 1;

/// Pure-Rust RLE 軽量圧縮（ゼロ／同一バイト連続を 3 バイトへ圧縮し、非反復列はそのまま転送）
fn rle_compress(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 2 + 16);
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        let mut run = 1usize;
        while i + run < input.len() && input[i + run] == b && run < 130 {
            run += 1;
        }
        if run >= 4 {
            // 制御バイト: 0x80 | (run - 4)  [run: 4..=130]
            out.push(0x80 | ((run - 4) as u8));
            out.push(b);
            i += run;
        } else {
            let lit_start = i;
            i += run;
            while i < input.len() && (i - lit_start) < 127 {
                let nb = input[i];
                let mut next_run = 1usize;
                while i + next_run < input.len() && input[i + next_run] == nb && next_run < 4 {
                    next_run += 1;
                }
                if next_run >= 4 {
                    break;
                }
                i += 1;
            }
            let lit_len = i - lit_start;
            out.push(lit_len as u8); // 0x01..=0x7F
            out.extend_from_slice(&input[lit_start..i]);
        }
    }
    out
}

fn rle_decompress(input: &[u8], expected_len: usize) -> Result<Vec<u8>, String> {
    if expected_len > 256 * 1024 * 1024 {
        return Err("fastbin payload exceeds 256 MiB safety bound".into());
    }
    let mut out = Vec::with_capacity(expected_len);
    let mut i = 0;
    while i < input.len() {
        let ctrl = input[i];
        i += 1;
        if ctrl & 0x80 != 0 {
            let run = (ctrl & 0x7F) as usize + 4;
            let Some(&val) = input.get(i) else {
                return Err("truncated fastbin RLE run".into());
            };
            i += 1;
            if out.len().saturating_add(run) > expected_len {
                return Err("fastbin RLE overflow".into());
            }
            out.resize(out.len() + run, val);
        } else {
            let lit_len = ctrl as usize;
            if lit_len == 0 || i.saturating_add(lit_len) > input.len() {
                return Err("invalid fastbin literal block".into());
            }
            if out.len().saturating_add(lit_len) > expected_len {
                return Err("fastbin literal overflow".into());
            }
            out.extend_from_slice(&input[i..i + lit_len]);
            i += lit_len;
        }
    }
    if out.len() != expected_len {
        return Err(format!(
            "fastbin decompressed length mismatch: expected {expected_len}, got {}",
            out.len()
        ));
    }
    Ok(out)
}

/// プロジェクトを FIPS 180-4 SHA-256 整合性検証付きコンパクト高速スナップショット（`VEFAST11`）へエンコードする
pub fn encode_project_fastbin(project: &Project) -> Result<Vec<u8>, String> {
    let raw = serde_json::to_vec(project).map_err(|e| format!("serialize error: {e}"))?;
    let digest_hex = sha256_hex(&raw);
    let compressed = rle_compress(&raw);

    let mut out = Vec::with_capacity(8 + 4 + 8 + 8 + 64 + compressed.len());
    out.extend_from_slice(&FASTBIN_MAGIC);
    out.extend_from_slice(&FASTBIN_VERSION.to_le_bytes());
    out.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    out.extend_from_slice(digest_hex.as_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// `VEFAST11` バイナリスナップショットを SHA-256 検証付きでデコードする（破損時は安全に `Err` を返し `project.json` フォールバック可能）
pub fn decode_project_fastbin(bytes: &[u8]) -> Result<Project, String> {
    const HEADER_LEN: usize = 8 + 4 + 8 + 8 + 64;
    if bytes.len() < HEADER_LEN {
        return Err("fastbin file too small".into());
    }
    if bytes[0..8] != FASTBIN_MAGIC {
        return Err("invalid fastbin magic header".into());
    }
    let ver = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if ver != FASTBIN_VERSION {
        return Err(format!("unsupported fastbin version: {ver}"));
    }
    let raw_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let comp_len = u64::from_le_bytes(bytes[20..28].try_into().unwrap()) as usize;
    let expected_sha = std::str::from_utf8(&bytes[28..92])
        .map_err(|_| "invalid fastbin sha256 header".to_string())?;
    let payload = &bytes[HEADER_LEN..];
    if payload.len() != comp_len {
        return Err("fastbin compressed payload length mismatch".into());
    }
    let raw = rle_decompress(payload, raw_len)?;
    let actual_sha = sha256_hex(&raw);
    if actual_sha != expected_sha {
        return Err("fastbin SHA-256 integrity verification failed".into());
    }
    serde_json::from_slice(&raw).map_err(|e| format!("fastbin JSON decode failed: {e}"))
}

/// プロジェクトフォルダの `cache/project.fastbin` パスを返す
pub fn fastbin_snapshot_path(folder: &ProjectFolder) -> PathBuf {
    folder.cache_dir().join("project.fastbin")
}

/// `cache/project.fastbin` へ SHA-256 検証付き高速スナップショットを書き出す
pub fn save_project_fastbin_to_folder(
    folder: &ProjectFolder,
    project: &Project,
) -> Result<(PathBuf, usize), String> {
    let bytes = encode_project_fastbin(project)?;
    let path = fastbin_snapshot_path(folder);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    fs::write(&path, &bytes).map_err(|e| format!("write fastbin failed: {e}"))?;
    Ok((path, bytes.len()))
}

/// `cache/project.fastbin` を検証付きで読み込む
pub fn load_project_fastbin_from_folder(folder: &ProjectFolder) -> Result<Project, String> {
    let path = fastbin_snapshot_path(folder);
    let bytes = fs::read(&path).map_err(|e| format!("read fastbin failed: {e}"))?;
    decode_project_fastbin(&bytes)
}

/// 2 シーン間のレイヤー単位構造共有差分サマリー（Copy-on-Write 省メモリ評価用）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralLayerDiff {
    pub total_layers: usize,
    pub unchanged_layers: usize,
    pub modified_layers: usize,
    pub shared_ratio_pct: u8,
}

pub fn compute_structural_layer_diff(before: &Scene, after: &Scene) -> StructuralLayerDiff {
    let total = before.layers.len().max(after.layers.len());
    let mut unchanged = 0usize;
    for i in 0..before.layers.len().min(after.layers.len()) {
        if before.layers[i] == after.layers[i] {
            unchanged += 1;
        }
    }
    let modified = total.saturating_sub(unchanged);
    let shared_ratio_pct = (unchanged * 100)
        .checked_div(total)
        .map_or(100, |v| v.min(100) as u8);
    StructuralLayerDiff {
        total_layers: total,
        unchanged_layers: unchanged,
        modified_layers: modified,
        shared_ratio_pct,
    }
}

/// 大規模プロジェクトのメモリ＆ディスク軽量化監査レポート
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectMemoryAudit {
    pub scene_count: usize,
    pub total_layers: usize,
    pub total_items: usize,
    pub total_keyframes: usize,
    pub asset_count: usize,
    pub unreferenced_asset_ids: Vec<AssetId>,
    pub json_bytes: usize,
    pub fastbin_bytes: usize,
    pub fastbin_ratio_pct: f32,
    pub undo_steps: usize,
    pub unique_undo_snapshots: usize,
    pub undo_memory_saved_pct: f32,
    pub disk_cache_bytes: u64,
}

fn dir_size_bytes(dir: &std::path::Path) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total = total.saturating_add(dir_size_bytes(&p));
        } else if let Ok(m) = entry.metadata() {
            total = total.saturating_add(m.len());
        }
    }
    total
}

/// プロジェクトの参照グラフ・JSON/FastBin サイズ・Undo 共有率・キャッシュ容量を監査する
pub fn audit_project_memory(
    project: &Project,
    folder: Option<&ProjectFolder>,
    undo_steps: usize,
    unique_undo_snapshots: usize,
) -> ProjectMemoryAudit {
    let mut total_layers = 0usize;
    let mut total_items = 0usize;
    let mut total_keyframes = 0usize;
    let mut referenced: HashSet<AssetId> = HashSet::new();

    for scene in &project.scenes {
        total_layers += scene.layers.len();
        for layer in &scene.layers {
            total_items += layer.items.len();
            for item in &layer.items {
                total_keyframes += item.keyframes.values().map(Vec::len).sum::<usize>();
                match &item.kind {
                    ItemKind::Video { asset_id, .. }
                    | ItemKind::Audio { asset_id, .. }
                    | ItemKind::Image { asset_id } => {
                        referenced.insert(*asset_id);
                    }
                    ItemKind::VtuberAvatar { config } => {
                        if let Some(aid) = config.asset_id {
                            referenced.insert(aid);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let unreferenced_asset_ids: Vec<AssetId> = project
        .assets
        .iter()
        .filter(|a| !referenced.contains(&a.id))
        .map(|a| a.id)
        .collect();

    let json_bytes = serde_json::to_vec(project).map(|v| v.len()).unwrap_or(0);
    let fastbin_bytes = encode_project_fastbin(project)
        .map(|v| v.len())
        .unwrap_or(json_bytes);
    let fastbin_ratio_pct = if json_bytes > 0 {
        (fastbin_bytes as f32 / json_bytes as f32) * 100.0
    } else {
        100.0
    };

    let unshared_snapshots = undo_steps.saturating_mul(2);
    let undo_memory_saved_pct = if unshared_snapshots > 0 {
        let saved = unshared_snapshots.saturating_sub(unique_undo_snapshots);
        (saved as f32 / unshared_snapshots as f32) * 100.0
    } else {
        0.0
    };

    let disk_cache_bytes = folder.map(|f| dir_size_bytes(&f.cache_dir())).unwrap_or(0);

    ProjectMemoryAudit {
        scene_count: project.scenes.len(),
        total_layers,
        total_items,
        total_keyframes,
        asset_count: project.assets.len(),
        unreferenced_asset_ids,
        json_bytes,
        fastbin_bytes,
        fastbin_ratio_pct,
        undo_steps,
        unique_undo_snapshots,
        undo_memory_saved_pct,
        disk_cache_bytes,
    }
}

/// コンパクション実行結果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCompactionReport {
    pub removed_unused_assets: usize,
    pub purged_cache_files: usize,
    pub purged_cache_bytes: u64,
    pub fastbin_saved_bytes: Option<usize>,
}

/// 未使用アセット整理用の 1-Undo `EditCommand::ReplaceProjectSnapshot` とキャッシュパージを実行する
pub fn compact_project_and_caches(
    project: &Project,
    folder: Option<&ProjectFolder>,
    remove_unused_assets: bool,
    purge_frame_cache: bool,
    write_fastbin: bool,
) -> (Option<EditCommand>, ProjectCompactionReport) {
    let audit = audit_project_memory(project, folder, 0, 0);
    let mut cmd_opt = None;
    let mut removed_unused_assets = 0usize;
    let mut next_proj = project.clone();

    if remove_unused_assets && !audit.unreferenced_asset_ids.is_empty() {
        let drop_set: HashSet<AssetId> = audit.unreferenced_asset_ids.iter().copied().collect();
        let before = next_proj.assets.len();
        next_proj.assets.retain(|a| !drop_set.contains(&a.id));
        removed_unused_assets = before.saturating_sub(next_proj.assets.len());
        if removed_unused_assets > 0 {
            cmd_opt = Some(EditCommand::ReplaceProjectSnapshot {
                project: Box::new(next_proj.clone()),
            });
        }
    }

    let mut purged_cache_files = 0usize;
    let mut purged_cache_bytes = 0u64;
    if purge_frame_cache
        && let Some(f) = folder
        && let Ok(rd) = fs::read_dir(f.cache_frames_dir())
    {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                if fs::remove_file(&p).is_ok() {
                    purged_cache_files += 1;
                    purged_cache_bytes = purged_cache_bytes.saturating_add(len);
                }
            }
        }
    }

    let fastbin_saved_bytes = if write_fastbin
        && let Some(f) = folder
        && let Ok((_, sz)) = save_project_fastbin_to_folder(f, &next_proj)
    {
        Some(sz)
    } else {
        None
    };

    (
        cmd_opt,
        ProjectCompactionReport {
            removed_unused_assets,
            purged_cache_files,
            purged_cache_bytes,
            fastbin_saved_bytes,
        },
    )
}

// ============================================================================
// #64: リアルタイム・パフォーマンスプロファイラ HUD ＆ 自動ボトルネック診断
// ============================================================================

/// v11.0 #64: プレビュー上のパフォーマンスプロファイラ HUD 設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfilerHudConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub show_frame_graph: bool,
    #[serde(default = "default_true")]
    pub show_cache_stats: bool,
    #[serde(default = "default_warn_ms")]
    pub warn_frame_ms: f32,
}

fn default_warn_ms() -> f32 {
    16.67
}

impl Default for ProfilerHudConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            show_frame_graph: true,
            show_cache_stats: true,
            warn_frame_ms: 16.67,
        }
    }
}

/// 1 パス分の実測マイクロ秒データ
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassTimingSample {
    pub pass_name: String,
    pub duration_us: u64,
}

/// 1 フレーム分の詳細パフォーマンス計測レポート
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameProfileReport {
    pub width: u32,
    pub height: u32,
    pub total_us: u64,
    pub active_layer_count: usize,
    pub active_item_count: usize,
    pub gpu_zero_copy_used: bool,
    pub adaptive_divisor: u32,
    pub pass_breakdown: Vec<PassTimingSample>,
    pub text_cache_hits: u64,
    pub text_cache_misses: u64,
    pub text_cache_bytes: usize,
    pub frame_pool_reuses: u64,
    pub frame_pool_bytes: u64,
    pub lut_cache_entries: usize,
}

impl FrameProfileReport {
    pub fn total_ms(&self) -> f32 {
        self.total_us as f32 / 1000.0
    }
}

/// 直近 120 フレームのローリング統計（平均・p95・p99・推定 FPS・ドロップ率）
#[derive(Debug, Clone)]
pub struct RollingProfilerHistory {
    samples: VecDeque<FrameProfileReport>,
    capacity: usize,
}

impl Default for RollingProfilerHistory {
    fn default() -> Self {
        Self::new(120)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfilerSummaryStats {
    pub sample_count: usize,
    pub avg_ms: f32,
    pub p95_ms: f32,
    pub p99_ms: f32,
    pub fps_estimate: f32,
    pub over_budget_pct: f32,
}

impl RollingProfilerHistory {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(8);
        Self {
            samples: VecDeque::with_capacity(cap),
            capacity: cap,
        }
    }

    pub fn push(&mut self, report: FrameProfileReport) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(report);
    }

    pub fn latest(&self) -> Option<&FrameProfileReport> {
        self.samples.back()
    }

    pub fn samples(&self) -> &VecDeque<FrameProfileReport> {
        &self.samples
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn summarize(&self, warn_ms: f32) -> ProfilerSummaryStats {
        let n = self.samples.len();
        if n == 0 {
            return ProfilerSummaryStats {
                sample_count: 0,
                avg_ms: 0.0,
                p95_ms: 0.0,
                p99_ms: 0.0,
                fps_estimate: 0.0,
                over_budget_pct: 0.0,
            };
        }
        let mut ms_vals: Vec<f32> = self
            .samples
            .iter()
            .map(FrameProfileReport::total_ms)
            .collect();
        let sum: f32 = ms_vals.iter().sum();
        let avg_ms = sum / n as f32;
        let over = ms_vals.iter().filter(|&&m| m > warn_ms).count();
        ms_vals.sort_by(f32::total_cmp);
        let p95_idx = ((n as f32 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1);
        let p99_idx = ((n as f32 * 0.99).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1);
        let fps_estimate = if avg_ms > 0.01 {
            1000.0 / avg_ms
        } else {
            999.0
        };
        ProfilerSummaryStats {
            sample_count: n,
            avg_ms,
            p95_ms: ms_vals[p95_idx],
            p99_ms: ms_vals[p99_idx],
            fps_estimate,
            over_budget_pct: (over as f32 / n as f32) * 100.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BottleneckSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BottleneckRecommendation {
    pub severity: BottleneckSeverity,
    pub layer_index: Option<usize>,
    pub item_id: Option<ItemId>,
    pub title: String,
    pub detail: String,
    pub estimated_speedup_pct: u8,
    pub suggested_action: String,
}

/// シーン構造と最新フレーム実測値からボトルネックを自動診断し、具体的な最適化提案列を返す
pub fn diagnose_timeline_bottlenecks(
    project: &Project,
    scene_id: SceneId,
    latest_profile: Option<&FrameProfileReport>,
) -> Vec<BottleneckRecommendation> {
    let Some(scene) = project.scene(scene_id) else {
        return Vec::new();
    };
    let mut recs: Vec<BottleneckRecommendation> = Vec::new();

    if !scene.gpu_graph.enabled || !scene.gpu_graph.prefer_zero_copy_vram {
        recs.push(BottleneckRecommendation {
            severity: BottleneckSeverity::Warning,
            layer_index: None,
            item_id: None,
            title: "ゼロコピー GPU レンダリンググラフが無効です".into(),
            detail:
                "VRAM 常駐テクスチャ連結を有効にすると 4K/8K 時の PCIe 往復転送を削減できます。"
                    .into(),
            estimated_speedup_pct: 45,
            suggested_action: "enable_gpu_zero_copy_graph".into(),
        });
    }

    if !project.adaptive_playback.enabled && (scene.width >= 1920 || scene.height >= 1080) {
        recs.push(BottleneckRecommendation {
            severity: BottleneckSeverity::Info,
            layer_index: None,
            item_id: None,
            title: "高解像度シーンで適応解像度再生が無効です".into(),
            detail: "再生中のみ 1/2〜1/4 へ自動スケールし停止時にフル解像度復帰することでフレーム落ちを防げます。".into(),
            estimated_speedup_pct: 50,
            suggested_action: "enable_adaptive_playback".into(),
        });
    }

    if scene.motion_blur.enabled && scene.motion_blur.preview_samples > 4 {
        recs.push(BottleneckRecommendation {
            severity: BottleneckSeverity::Warning,
            layer_index: None,
            item_id: None,
            title: format!(
                "プレビューモーションブラー分割数が高い ({}x)",
                scene.motion_blur.preview_samples
            ),
            detail: "プレビュー時のサブフレーム分割を 4 以下にすると書き出し品質を維持しつつリアルタイム性を向上できます。".into(),
            estimated_speedup_pct: 40,
            suggested_action: "cap_preview_motion_blur_samples".into(),
        });
    }

    for (l_idx, layer) in scene.layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        for item in &layer.items {
            let fx_cnt = item.effect_chain.iter().filter(|e| e.enabled).count();
            if fx_cnt >= 3 {
                recs.push(BottleneckRecommendation {
                    severity: BottleneckSeverity::Warning,
                    layer_index: Some(l_idx),
                    item_id: Some(item.id),
                    title: format!("重エフェクトチェーン: '{}' ({fx_cnt}段)", item.name),
                    detail: "Smart Precompose でハッシュキャッシュ化すると同一フレーム再描画コストを大幅に削減できます。".into(),
                    estimated_speedup_pct: 35,
                    suggested_action: "precompose_heavy_item".into(),
                });
            }
            if matches!(
                item.kind,
                ItemKind::FluidSim { .. } | ItemKind::ClothSim { .. } | ItemKind::Mesh3D { .. }
            ) {
                recs.push(BottleneckRecommendation {
                    severity: BottleneckSeverity::Info,
                    layer_index: Some(l_idx),
                    item_id: Some(item.id),
                    title: format!("高負荷シミュレーション/3D クリップ: '{}'", item.name),
                    detail: "スマート事前レンダリング (#62) または RAM 先読みキャッシュの対象に含めるのが最適です。".into(),
                    estimated_speedup_pct: 55,
                    suggested_action: "schedule_smart_prerender".into(),
                });
            }
        }
    }

    if let Some(prof) = latest_profile
        && prof.total_ms() > project.profiler_hud.warn_frame_ms.max(8.0)
    {
        let heaviest = prof
            .pass_breakdown
            .iter()
            .max_by_key(|p| p.duration_us)
            .map(|p| format!("{} ({:.2}ms)", p.pass_name, p.duration_us as f32 / 1000.0))
            .unwrap_or_else(|| "Composite".into());
        recs.push(BottleneckRecommendation {
            severity: BottleneckSeverity::Critical,
            layer_index: None,
            item_id: None,
            title: format!(
                "フレーム予算超過: {:.2}ms（最重パス: {heaviest}）",
                prof.total_ms()
            ),
            detail:
                "GPU ゼロコピー連結＋適応解像度再生＋スマート事前レンダリングの併用を推奨します。"
                    .into(),
            estimated_speedup_pct: 60,
            suggested_action: "auto_optimize_scene".into(),
        });
    }

    recs
}

/// 診断に基づき安全な非破壊パフォーマンス最適化（ゼロコピー GPU グラフ有効化・適応解像度再生有効化・プレビュー MB 4x 上限）を 1-Undo `Batch` で生成する
pub fn build_auto_optimize_scene_commands(
    project: &Project,
    scene_id: SceneId,
) -> Option<EditCommand> {
    let scene = project.scene(scene_id)?;
    let mut cmds: Vec<EditCommand> = Vec::new();

    if !scene.gpu_graph.enabled || !scene.gpu_graph.prefer_zero_copy_vram {
        let mut next = scene.gpu_graph.clone();
        next.enabled = true;
        next.prefer_zero_copy_vram = true;
        next.chain_post_fx_on_gpu = true;
        cmds.push(EditCommand::SetGpuRenderGraph { gpu_graph: next });
    }
    if !project.adaptive_playback.enabled || !project.adaptive_playback.smart_prerender_enabled {
        let mut next = project.adaptive_playback.clone();
        next.enabled = true;
        next.smart_prerender_enabled = true;
        cmds.push(EditCommand::SetAdaptivePlayback {
            adaptive_playback: next,
        });
    }
    if scene.motion_blur.enabled && scene.motion_blur.preview_samples > 4 {
        let mut next = scene.motion_blur;
        next.preview_samples = 4;
        cmds.push(EditCommand::SetMotionBlur { motion_blur: next });
    }

    if cmds.is_empty() {
        None
    } else {
        Some(EditCommand::Batch {
            label: "v11.0 自動パフォーマンス最適化".into(),
            commands: cmds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluid_cloth::{FluidPreset, FluidSimConfig};
    use crate::project::{ProjectSettings, RgbaColor, ShapeKind, TimelineItem};
    use crate::spatial_video::{Spatial360Preset, SpatialVideoConfig};
    use tempfile::tempdir;

    #[test]
    fn gpu_render_graph_plan_chains_passes_and_calculates_pcie_savings() {
        let mut proj = Project::new("GpuPlanTest", ProjectSettings::default());
        let sc = proj.active_scene_mut().unwrap();
        sc.color_grade.enabled = true;
        sc.color_management.enabled = true;
        sc.spatial_video = SpatialVideoConfig::from_preset(Spatial360Preset::ActionHorizonLock);
        sc.layers[0].items.push(TimelineItem::new(
            "Fluid",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::FluidSim {
                config: Box::new(FluidSimConfig::from_preset(FluidPreset::BonfireFlame)),
            },
        ));

        let plan = build_gpu_render_graph_plan(sc, 3840, 2160);
        assert!(plan.zero_copy_chain_len >= 5);
        assert_eq!(plan.cpu_readbacks_saved, plan.zero_copy_chain_len - 1);
        assert!(plan.estimated_pcie_bytes_saved_per_frame > 100 * 1024 * 1024);
        assert!(!plan.requires_8k_tiling);

        let plan_8k = build_gpu_render_graph_plan(sc, 7680, 4320);
        assert!(plan_8k.requires_8k_tiling);
    }

    #[test]
    fn adaptive_scale_hysteresis_and_smart_prerender_prioritize_hotspots() {
        let cfg = AdaptivePlaybackConfig::default();
        let mut state = AdaptiveScaleState::default();
        assert_eq!(state.current_divisor, 1);

        // 30ms フレームが続くと 1 -> 2 -> 4 へ段階的に縮小
        for _ in 0..8 {
            state.record_frame_time(32.0, true, &cfg);
        }
        assert!(state.current_divisor >= 2);
        assert!(state.total_downscales >= 1);

        // 停止時は即座に 1/1 フル解像度へ復帰
        assert_eq!(state.record_frame_time(32.0, false, &cfg), 1);

        // スマート事前レンダリング: 流体がある 2s〜4s 区間が矩形だけの 0s〜2s より優先される
        let mut proj = Project::new("HotspotTest", ProjectSettings::default());
        let sc = proj.active_scene_mut().unwrap();
        sc.layers[0].items.push(TimelineItem::new(
            "SimpleRect",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        ));
        sc.layers[1].items.push(TimelineItem::new(
            "HeavyFluid",
            RationalTime::from_seconds(2.0),
            RationalTime::from_seconds(2.0),
            ItemKind::FluidSim {
                config: Box::new(FluidSimConfig::from_preset(FluidPreset::BonfireFlame)),
            },
        ));
        let plan = plan_smart_prerender_schedule(sc, &cfg);
        assert!(!plan.hotspots.is_empty());
        assert!(plan.hotspots[0].start_sec >= 2.0);
        assert!(!plan.prioritized_frames.is_empty());
    }

    #[test]
    fn fastbin_snapshot_roundtrip_sha256_integrity_and_memory_audit_work() {
        let tmp = tempdir().unwrap();
        let mut proj = Project::new("FastbinTest", ProjectSettings::default());
        proj.active_scene_mut().unwrap().layers[0]
            .items
            .push(TimelineItem::new(
                "Clip",
                RationalTime::ZERO,
                RationalTime::from_seconds(3.0),
                ItemKind::Shape {
                    shape: ShapeKind::Ellipse,
                    width: 200.0,
                    height: 120.0,
                    color: RgbaColor::WHITE,
                },
            ));

        let mut encoded = encode_project_fastbin(&proj).unwrap();
        let decoded = decode_project_fastbin(&encoded).unwrap();
        assert_eq!(proj, decoded);

        // 1 バイト改ざんで SHA-256 検証エラーを返すこと
        let last = encoded.len() - 1;
        encoded[last] ^= 0x55;
        assert!(decode_project_fastbin(&encoded).is_err());

        // フォルダ保存＆監査
        let folder = ProjectFolder::init(tmp.path().join("fb_proj"), &proj).unwrap();
        let (path, sz) = save_project_fastbin_to_folder(&folder, &proj).unwrap();
        assert!(path.exists() && sz > 92);
        assert_eq!(load_project_fastbin_from_folder(&folder).unwrap(), proj);

        let audit = audit_project_memory(&proj, Some(&folder), 50, 51);
        assert!(audit.undo_memory_saved_pct >= 48.0);
        assert!(audit.fastbin_bytes > 0);
    }

    #[test]
    fn profiler_rolling_stats_and_bottleneck_diagnosis_produce_auto_optimize() {
        let mut proj = Project::new("ProfileDiag", ProjectSettings::default());
        proj.adaptive_playback.enabled = false;
        {
            let sc = proj.active_scene_mut().unwrap();
            sc.gpu_graph.enabled = false;
            sc.motion_blur.enabled = true;
            sc.motion_blur.preview_samples = 8;
        }
        let mut hist = RollingProfilerHistory::new(30);
        for us in [10_000u64, 15_000, 22_000, 28_000, 35_000] {
            hist.push(FrameProfileReport {
                width: 1920,
                height: 1080,
                total_us: us,
                active_layer_count: 3,
                active_item_count: 4,
                gpu_zero_copy_used: false,
                adaptive_divisor: 1,
                pass_breakdown: vec![PassTimingSample {
                    pass_name: "LayerRender".into(),
                    duration_us: us,
                }],
                text_cache_hits: 10,
                text_cache_misses: 2,
                text_cache_bytes: 120_000,
                frame_pool_reuses: 4,
                frame_pool_bytes: 16_000_000,
                lut_cache_entries: 1,
            });
        }
        let stats = hist.summarize(16.67);
        assert_eq!(stats.sample_count, 5);
        assert!(stats.p95_ms >= 28.0);
        assert!(stats.over_budget_pct > 50.0);

        let recs = diagnose_timeline_bottlenecks(&proj, proj.active_scene_id, hist.latest());
        assert!(recs.len() >= 3);
        let auto_cmd = build_auto_optimize_scene_commands(&proj, proj.active_scene_id);
        assert!(auto_cmd.is_some());
    }
}
