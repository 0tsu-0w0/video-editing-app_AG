//! v7.0 #47: GPU-Resident 4K/8K 合成フレームプール ＆ WGSL ブレンド/マスク/ACES 色管理高速パス
//! （`pack2x16float` 半精度 8B/px ストレージ＋ストライプ分割で 8K 解像度も標準 `wgpu` 制限内で動作・決定論 CPU フォールバック維持）
//! ＋ v7.0 #48: A/B スプリット比較ビュー合成器 (`compose_ab_split_linear` / `render_ab_compare_scene`)

use crate::color::{LinearImage, LinearPremul};
use crate::hot_shader::validate_wgsl;
use crate::sprite::scale_linear_image;
use half::f16;
use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
use std::time::Instant;
use ve_core::{
    AbCompareConfig, AbCompareMode, AbReferenceSource, BlendMode, ColorManagementConfig,
    ColorSpace, Project, RationalTime, SceneId, ToneMapper, TrackMatteMode,
    transform_color_pipeline_rgb,
};

/// 8K UHD (`7680×4320`) でも `wgpu` 標準 `max_storage_buffer_binding_size` (128 MiB) 内に余裕で収まる 1 ストライプ上限 (64 MiB)
const MAX_GPU_STRIPE_BYTES: usize = 64 * 1024 * 1024;

/// プール管理するフレームサーフェス表現
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoolSurfaceFormat {
    /// `Rgba16Float` (8 bytes / pixel) — 4K で約 63.3 MiB, 8K で約 253.1 MiB
    Rgba16Float,
    /// `LinearPremul` `f32x4` (16 bytes / pixel)
    LinearF32x4,
}

impl PoolSurfaceFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba16Float => 8,
            Self::LinearF32x4 => 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FramePoolBucketKey {
    pub width: u32,
    pub height: u32,
    pub format: PoolSurfaceFormat,
}

impl FramePoolBucketKey {
    pub fn byte_len(self) -> Option<u64> {
        let px = (self.width as u64).checked_mul(self.height as u64)?;
        px.checked_mul(self.format.bytes_per_pixel() as u64)
    }
}

/// フレームプールの稼働統計メトリクス
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FramePoolMetrics {
    pub allocations_created: u64,
    pub pool_reuses: u64,
    pub evictions: u64,
    pub active_leased: usize,
    pub idle_cached: usize,
    pub resident_bytes: u64,
    pub peak_resident_bytes: u64,
    pub budget_bytes: u64,
    pub gpu_resident_buffers: usize,
}

impl FramePoolMetrics {
    pub fn hit_rate(&self) -> f64 {
        let total = self.allocations_created + self.pool_reuses;
        if total == 0 {
            0.0
        } else {
            (self.pool_reuses as f64) / (total as f64)
        }
    }
}

#[derive(Debug)]
struct PooledCanvasEntry {
    key: FramePoolBucketKey,
    last_used_tick: u64,
    bytes: u64,
    payload: PooledPayload,
}

#[derive(Debug)]
enum PooledPayload {
    LinearPixels(Vec<LinearPremul>),
    Rgba16Bytes(Vec<u8>),
}

/// 4K / 8K マルチレイヤー合成時の再確保を排除する VRAM/ホスト常駐フレームプール（LRU Eviction 対応）
#[derive(Debug)]
pub struct ResidentFramePool {
    budget_bytes: u64,
    resident_bytes: u64,
    peak_resident_bytes: u64,
    allocations_created: u64,
    pool_reuses: u64,
    evictions: u64,
    active_leased: usize,
    tick: u64,
    idle: Vec<PooledCanvasEntry>,
}

impl Default for ResidentFramePool {
    fn default() -> Self {
        // 既定予算: 512 MiB（4K Rgba16Float 約 8 枚 or LinearF32x4 約 4 枚、8K も保持可能）
        Self::with_budget_mib(512)
    }
}

impl ResidentFramePool {
    pub fn with_budget_mib(budget_mib: u64) -> Self {
        Self {
            budget_bytes: budget_mib.max(32) * 1024 * 1024,
            resident_bytes: 0,
            peak_resident_bytes: 0,
            allocations_created: 0,
            pool_reuses: 0,
            evictions: 0,
            active_leased: 0,
            tick: 0,
            idle: Vec::new(),
        }
    }

    pub fn set_budget_mib(&mut self, budget_mib: u64) {
        self.budget_bytes = budget_mib.max(32) * 1024 * 1024;
        self.evict_until_within_budget(0);
    }

    pub fn metrics(&self, gpu_resident_buffers: usize) -> FramePoolMetrics {
        FramePoolMetrics {
            allocations_created: self.allocations_created,
            pool_reuses: self.pool_reuses,
            evictions: self.evictions,
            active_leased: self.active_leased,
            idle_cached: self.idle.len(),
            resident_bytes: self.resident_bytes,
            peak_resident_bytes: self.peak_resident_bytes,
            budget_bytes: self.budget_bytes,
            gpu_resident_buffers,
        }
    }

    pub fn clear(&mut self) {
        self.evictions += self.idle.len() as u64;
        for entry in self.idle.drain(..) {
            self.resident_bytes = self.resident_bytes.saturating_sub(entry.bytes);
        }
    }

    fn evict_until_within_budget(&mut self, incoming_bytes: u64) {
        while !self.idle.is_empty()
            && self.resident_bytes.saturating_add(incoming_bytes) > self.budget_bytes
        {
            if let Some((oldest_idx, _)) = self
                .idle
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_used_tick)
            {
                let removed = self.idle.swap_remove(oldest_idx);
                self.resident_bytes = self.resident_bytes.saturating_sub(removed.bytes);
                self.evictions += 1;
            } else {
                break;
            }
        }
    }

    /// 指定解像度の `LinearImage` キャンバスをプールから取得（無ければ新規確保）し、`fill` で初期化する
    pub fn acquire_linear_canvas(
        &mut self,
        width: u32,
        height: u32,
        fill: LinearPremul,
    ) -> LinearImage {
        let w = width.max(1);
        let h = height.max(1);
        let key = FramePoolBucketKey {
            width: w,
            height: h,
            format: PoolSurfaceFormat::LinearF32x4,
        };
        let count = (w as usize).saturating_mul(h as usize);
        let bytes = key.byte_len().unwrap_or(0);

        self.tick = self.tick.wrapping_add(1);
        if let Some(pos) = self
            .idle
            .iter()
            .rposition(|e| e.key == key && matches!(e.payload, PooledPayload::LinearPixels(_)))
        {
            let entry = self.idle.swap_remove(pos);
            self.pool_reuses += 1;
            self.active_leased += 1;
            if let PooledPayload::LinearPixels(mut pixels) = entry.payload {
                pixels.resize(count, fill);
                pixels.fill(fill);
                return LinearImage {
                    width: w,
                    height: h,
                    pixels,
                };
            }
        }

        self.evict_until_within_budget(bytes);
        self.allocations_created += 1;
        self.active_leased += 1;
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        self.peak_resident_bytes = self.peak_resident_bytes.max(self.resident_bytes);
        LinearImage::new(w, h, fill)
    }

    /// 使用済み `LinearImage` をプールへ返却して次フレームでゼロアロケーション再利用できるようにする
    pub fn release_linear_canvas(&mut self, img: LinearImage) {
        self.active_leased = self.active_leased.saturating_sub(1);
        if img.width == 0 || img.height == 0 {
            return;
        }
        let key = FramePoolBucketKey {
            width: img.width,
            height: img.height,
            format: PoolSurfaceFormat::LinearF32x4,
        };
        let bytes = key.byte_len().unwrap_or(0);
        if bytes > self.budget_bytes {
            self.resident_bytes = self.resident_bytes.saturating_sub(bytes);
            self.evictions += 1;
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        self.idle.push(PooledCanvasEntry {
            key,
            last_used_tick: self.tick,
            bytes,
            payload: PooledPayload::LinearPixels(img.pixels),
        });
        self.evict_until_within_budget(0);
    }

    /// `Rgba16Float` パック済みバイトバッファ（`width * height * 8` bytes）をプールから取得する
    pub fn acquire_rgba16f_buffer(&mut self, width: u32, height: u32) -> Vec<u8> {
        let w = width.max(1);
        let h = height.max(1);
        let key = FramePoolBucketKey {
            width: w,
            height: h,
            format: PoolSurfaceFormat::Rgba16Float,
        };
        let needed = (w as usize).saturating_mul(h as usize).saturating_mul(8);
        let bytes = needed as u64;

        self.tick = self.tick.wrapping_add(1);
        if let Some(pos) = self
            .idle
            .iter()
            .rposition(|e| e.key == key && matches!(e.payload, PooledPayload::Rgba16Bytes(_)))
        {
            let entry = self.idle.swap_remove(pos);
            self.pool_reuses += 1;
            self.active_leased += 1;
            if let PooledPayload::Rgba16Bytes(mut buf) = entry.payload {
                buf.resize(needed, 0);
                return buf;
            }
        }

        self.evict_until_within_budget(bytes);
        self.allocations_created += 1;
        self.active_leased += 1;
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        self.peak_resident_bytes = self.peak_resident_bytes.max(self.resident_bytes);
        vec![0u8; needed]
    }

    /// 使用済み `Rgba16Float` バイトバッファをプールへ返却する
    pub fn release_rgba16f_buffer(&mut self, width: u32, height: u32, buf: Vec<u8>) {
        self.active_leased = self.active_leased.saturating_sub(1);
        if width == 0 || height == 0 {
            return;
        }
        let key = FramePoolBucketKey {
            width,
            height,
            format: PoolSurfaceFormat::Rgba16Float,
        };
        let bytes = buf.len() as u64;
        if bytes > self.budget_bytes {
            self.resident_bytes = self.resident_bytes.saturating_sub(bytes);
            self.evictions += 1;
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        self.idle.push(PooledCanvasEntry {
            key,
            last_used_tick: self.tick,
            bytes,
            payload: PooledPayload::Rgba16Bytes(buf),
        });
        self.evict_until_within_budget(0);
    }
}

// ============================================================================
// WGSL Fast-Path Shaders (`Rgba16Float` packed `vec2<u32>` via pack2x16float)
// ============================================================================

/// v7.0 #47: ブレンドモード + トラックマット + ソフト矩形/楕円フェザーマスク 1 パス統合 WGSL Compute Shader
pub const FAST_BLEND_MASK_WGSL: &str = r#"
struct FastBlendMaskUniform {
    width: u32,
    stripe_rows: u32,
    y_offset: u32,
    full_height: u32,
    blend_mode: u32,
    matte_mode: u32,
    mask_shape: u32,
    invert_mask: u32,
    opacity: f32,
    mask_feather: f32,
    mask_cx: f32,
    mask_cy: f32,
    mask_hw: f32,
    mask_hh: f32,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<storage, read> dst_f16: array<vec2<u32>>;
@group(0) @binding(1) var<storage, read> src_f16: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> matte_f16: array<vec2<u32>>;
@group(0) @binding(3) var<storage, read_write> out_f16: array<vec2<u32>>;
@group(0) @binding(4) var<uniform> u: FastBlendMaskUniform;

fn f16_to_f32(h: u32) -> f32 {
    let sign = (h & 0x8000u) << 16u;
    let exp = (h >> 10u) & 0x1Fu;
    let mant = h & 0x03FFu;
    if (exp == 0u) {
        if (mant == 0u) {
            return bitcast<f32>(sign);
        }
        let mag = (f32(mant) / 1024.0) * 0.00006103515625;
        if (sign != 0u) {
            return -mag;
        }
        return mag;
    }
    if (exp == 31u) {
        return bitcast<f32>(sign | 0x7F800000u | (mant << 13u));
    }
    return bitcast<f32>(sign | ((exp + 112u) << 23u) | (mant << 13u));
}

fn f32_to_f16(val: f32) -> u32 {
    let b = bitcast<u32>(val);
    let sign = (b >> 16u) & 0x8000u;
    let exp32 = i32((b >> 23u) & 0xFFu) - 127;
    let mant32 = b & 0x007FFFFFu;
    if (exp32 <= -15) {
        return sign;
    }
    if (exp32 >= 16) {
        return sign | 0x7BFFu;
    }
    let exp16 = u32(exp32 + 15);
    let mant16 = (mant32 + 0x00001000u) >> 13u;
    return sign | ((exp16 << 10u) + mant16);
}

fn unpack_rgba16(v: vec2<u32>) -> vec4<f32> {
    return vec4<f32>(
        f16_to_f32(v.x & 0xFFFFu),
        f16_to_f32((v.x >> 16u) & 0xFFFFu),
        f16_to_f32(v.y & 0xFFFFu),
        f16_to_f32((v.y >> 16u) & 0xFFFFu)
    );
}

fn pack_rgba16(c: vec4<f32>) -> vec2<u32> {
    let r = f32_to_f16(c.x);
    let g = f32_to_f16(c.y);
    let b = f32_to_f16(c.z);
    let a = f32_to_f16(c.w);
    return vec2<u32>(r | (g << 16u), b | (a << 16u));
}

fn eval_shape_mask(uv: vec2<f32>) -> f32 {
    if (u.mask_shape == 0u) {
        return 1.0;
    }
    let f = max(u.mask_feather, 0.0005);
    var dist = 0.0;
    if (u.mask_shape == 1u) {
        let d = abs(uv - vec2<f32>(u.mask_cx, u.mask_cy)) - vec2<f32>(max(u.mask_hw, 0.001), max(u.mask_hh, 0.001));
        dist = max(d.x, d.y);
    } else {
        let q = (uv - vec2<f32>(u.mask_cx, u.mask_cy)) / vec2<f32>(max(u.mask_hw, 0.001), max(u.mask_hh, 0.001));
        dist = length(q) - 1.0;
    }
    var m = 1.0 - smoothstep(-f, f, dist);
    if (u.invert_mask != 0u) {
        m = 1.0 - m;
    }
    return clamp(m, 0.0, 1.0);
}

fn eval_track_matte(m_px: vec4<f32>) -> f32 {
    if (u.matte_mode == 0u) {
        return 1.0;
    }
    if (u.matte_mode == 1u) {
        return clamp(m_px.w, 0.0, 1.0);
    }
    if (u.matte_mode == 2u) {
        return clamp(1.0 - m_px.w, 0.0, 1.0);
    }
    let luma = clamp(0.2126 * m_px.x + 0.7152 * m_px.y + 0.0722 * m_px.z, 0.0, 1.0);
    if (u.matte_mode == 3u) {
        return luma;
    }
    return clamp(1.0 - luma, 0.0, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn cs_fast_blend_mask(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.width || gid.y >= u.stripe_rows) {
        return;
    }
    let idx = gid.y * u.width + gid.x;
    let dst = unpack_rgba16(dst_f16[idx]);
    let raw_src = unpack_rgba16(src_f16[idx]);
    let uv = vec2<f32>(
        (f32(gid.x) + 0.5) / f32(max(u.width, 1u)),
        (f32(u.y_offset + gid.y) + 0.5) / f32(max(u.full_height, 1u))
    );

    let shape_w = eval_shape_mask(uv);
    let matte_w = eval_track_matte(unpack_rgba16(matte_f16[idx]));
    let weight = clamp(u.opacity, 0.0, 1.0) * shape_w * matte_w;
    let src = raw_src * weight;

    var res = dst;
    if (u.blend_mode == 0u) {
        res = src + dst * (1.0 - src.w);
    } else if (u.blend_mode == 1u) {
        res = vec4<f32>(
            src.xyz + dst.xyz,
            clamp(src.w + dst.w * (1.0 - src.w), 0.0, 1.0)
        );
    } else if (u.blend_mode == 2u) {
        let inv_sa = 1.0 - src.w;
        let inv_da = 1.0 - dst.w;
        res = vec4<f32>(
            src.xyz * dst.xyz + src.xyz * inv_da + dst.xyz * inv_sa,
            clamp(src.w + dst.w * inv_sa, 0.0, 1.0)
        );
    } else {
        res = vec4<f32>(
            src.xyz + dst.xyz - src.xyz * dst.xyz,
            clamp(src.w + dst.w * (1.0 - src.w), 0.0, 1.0)
        );
    }
    out_f16[idx] = pack_rgba16(res);
}
"#;

/// v7.0 #47: カメラ Log IDT → ACEScg (AP1) → ガマット圧縮 → AgX / ACES Filmic トーンマップ → ODT 高速 WGSL Compute Shader
pub const FAST_ACES_COLOR_MGMT_WGSL: &str = r#"
struct FastColorMgmtUniform {
    width: u32,
    stripe_rows: u32,
    input_idt: u32,
    working_space: u32,
    output_odt: u32,
    tone_mapper: u32,
    gamut_compress: u32,
    _pad0: u32,
    exposure_mult: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
};

@group(0) @binding(0) var<storage, read> in_f16: array<vec2<u32>>;
@group(0) @binding(1) var<storage, read_write> out_f16: array<vec2<u32>>;
@group(0) @binding(2) var<uniform> u: FastColorMgmtUniform;

fn f16_to_f32_cm(h: u32) -> f32 {
    let sign = (h & 0x8000u) << 16u;
    let exp = (h >> 10u) & 0x1Fu;
    let mant = h & 0x03FFu;
    if (exp == 0u) {
        if (mant == 0u) {
            return bitcast<f32>(sign);
        }
        let mag = (f32(mant) / 1024.0) * 0.00006103515625;
        if (sign != 0u) {
            return -mag;
        }
        return mag;
    }
    if (exp == 31u) {
        return bitcast<f32>(sign | 0x7F800000u | (mant << 13u));
    }
    return bitcast<f32>(sign | ((exp + 112u) << 23u) | (mant << 13u));
}

fn f32_to_f16_cm(val: f32) -> u32 {
    let b = bitcast<u32>(val);
    let sign = (b >> 16u) & 0x8000u;
    let exp32 = i32((b >> 23u) & 0xFFu) - 127;
    let mant32 = b & 0x007FFFFFu;
    if (exp32 <= -15) {
        return sign;
    }
    if (exp32 >= 16) {
        return sign | 0x7BFFu;
    }
    let exp16 = u32(exp32 + 15);
    let mant16 = (mant32 + 0x00001000u) >> 13u;
    return sign | ((exp16 << 10u) + mant16);
}

fn s_log3_to_lin_ch(x: f32) -> f32 {
    if (x >= 171.2102946929 / 1023.0) {
        return pow(10.0, (x * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01;
    }
    return (x * 1023.0 - 95.0) * 0.01125000 / (171.2102946929 - 95.0);
}

fn v_log_to_lin_ch(x: f32) -> f32 {
    if (x < 0.181) {
        return (x - 0.125) / 5.6;
    }
    return pow(10.0, (x - 0.410597) / 0.241514) - 0.00873;
}

fn log_c3_to_lin_ch(x: f32) -> f32 {
    if (x > 0.1496582) {
        return (pow(10.0, (x - 0.385537) / 0.247190) - 0.052272) / 5.555556;
    }
    return (x - 0.092809) / 5.367655;
}

fn srgb_to_acescg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.6131324 * c.x + 0.3395380 * c.y + 0.0474167 * c.z,
        0.0701244 * c.x + 0.9163940 * c.y + 0.0134515 * c.z,
        0.0205877 * c.x + 0.1095746 * c.y + 0.8697854 * c.z
    );
}

fn acescg_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
         1.7048587 * c.x - 0.6217160 * c.y - 0.0832994 * c.z,
        -0.1300768 * c.x + 1.1407358 * c.y - 0.0105599 * c.z,
        -0.0239641 * c.x - 0.1289755 * c.y + 1.1530140 * c.z
    );
}

fn p3_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
         1.2249401 * c.x - 0.2249404 * c.y,
        -0.0420569 * c.x + 1.0420570 * c.y,
        -0.0196376 * c.x - 0.0786360 * c.y + 1.0982736 * c.z
    );
}

fn srgb_to_p3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.8224620 * c.x + 0.1775380 * c.y,
        0.0331942 * c.x + 0.9668058 * c.y,
        0.0170826 * c.x + 0.0723974 * c.y + 0.9105200 * c.z
    );
}

fn rec2020_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
         1.6604910 * c.x - 0.5876411 * c.y - 0.0728499 * c.z,
        -0.1245505 * c.x + 1.1328999 * c.y - 0.0083494 * c.z,
        -0.0181508 * c.x - 0.1005789 * c.y + 1.1187297 * c.z
    );
}

fn srgb_to_rec2020(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.6274039 * c.x + 0.3292830 * c.y + 0.0433131 * c.z,
        0.0690973 * c.x + 0.9195404 * c.y + 0.0113623 * c.z,
        0.0163914 * c.x + 0.0880133 * c.y + 0.8955953 * c.z
    );
}

fn aces_gamut_comp(rgb: vec3<f32>) -> vec3<f32> {
    let achr = max(max(rgb.x, rgb.y), rgb.z);
    if (achr <= 1e-5) {
        return max(rgb, vec3<f32>(0.0));
    }
    let d = (achr - rgb) / achr;
    let thr = 0.82;
    var cd = d;
    for (var ch = 0; ch < 3; ch = ch + 1) {
        let v = d[ch];
        if (v > thr) {
            let t = (v - thr) / max(1.35 - thr, 1e-4);
            cd[ch] = thr + (1.0 - thr) * (t / (1.0 + t));
        }
    }
    return max(vec3<f32>(achr) - cd * achr, vec3<f32>(0.0));
}

fn tone_map_aces_fitted(x: vec3<f32>) -> vec3<f32> {
    let a = x * (x + 0.0245786) - 0.000090537;
    let b = x * (0.983729 * x + 0.4329510) + 0.238081;
    return clamp(a / b, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn tone_map_agx(x: vec3<f32>) -> vec3<f32> {
    let v = max(x, vec3<f32>(1e-6));
    let lg = clamp((log2(v) + 10.0) / 16.5, vec3<f32>(0.0), vec3<f32>(1.0));
    let s = lg * lg * (3.0 - 2.0 * lg);
    let luma = 0.2126 * s.x + 0.7152 * s.y + 0.0722 * s.z;
    let desat = mix(s, vec3<f32>(luma), s * s * 0.28);
    return clamp(pow(desat, vec3<f32>(2.2)), vec3<f32>(0.0), vec3<f32>(1.0));
}

@compute @workgroup_size(8, 8, 1)
fn cs_fast_aces_color_mgmt(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.width || gid.y >= u.stripe_rows) {
        return;
    }
    let idx = gid.y * u.width + gid.x;
    let raw_pk = in_f16[idx];
    let r_in = f16_to_f32_cm(raw_pk.x & 0xFFFFu);
    let g_in = f16_to_f32_cm((raw_pk.x >> 16u) & 0xFFFFu);
    let b_in = f16_to_f32_cm(raw_pk.y & 0xFFFFu);
    let a = f16_to_f32_cm((raw_pk.y >> 16u) & 0xFFFFu);
    if (a <= 1e-6) {
        out_f16[idx] = raw_pk;
        return;
    }
    var rgb = vec3<f32>(r_in, g_in, b_in) / a;

    // 1. IDT -> Linear sRGB
    if (u.input_idt == 1u) {
        rgb = acescg_to_srgb(rgb);
    } else if (u.input_idt == 2u) {
        rgb = p3_to_srgb(rgb);
    } else if (u.input_idt == 3u) {
        rgb = rec2020_to_srgb(rgb);
    } else if (u.input_idt == 4u) {
        rgb = vec3<f32>(s_log3_to_lin_ch(rgb.x), s_log3_to_lin_ch(rgb.y), s_log3_to_lin_ch(rgb.z));
    } else if (u.input_idt == 5u) {
        rgb = vec3<f32>(v_log_to_lin_ch(rgb.x), v_log_to_lin_ch(rgb.y), v_log_to_lin_ch(rgb.z));
    } else if (u.input_idt == 6u) {
        rgb = vec3<f32>(log_c3_to_lin_ch(rgb.x), log_c3_to_lin_ch(rgb.y), log_c3_to_lin_ch(rgb.z));
    }

    // 2. Working Space + Exposure + Gamut Compression
    var work = rgb;
    if (u.working_space == 1u) {
        work = srgb_to_acescg(rgb);
    }
    work = max(work * u.exposure_mult, vec3<f32>(0.0));
    if (u.gamut_compress != 0u) {
        work = aces_gamut_comp(work);
    }

    var lin_srgb = work;
    if (u.working_space == 1u) {
        lin_srgb = acescg_to_srgb(work);
    }
    lin_srgb = max(lin_srgb, vec3<f32>(0.0));

    // 3. Tone Mapper
    if (u.tone_mapper == 0u) {
        lin_srgb = clamp(lin_srgb, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (u.tone_mapper == 1u) {
        lin_srgb = tone_map_aces_fitted(lin_srgb);
    } else if (u.tone_mapper == 2u) {
        lin_srgb = tone_map_agx(lin_srgb);
    } else {
        let white_sq = 16.0;
        lin_srgb = clamp((lin_srgb * (1.0 + lin_srgb / white_sq)) / (1.0 + lin_srgb), vec3<f32>(0.0), vec3<f32>(1.0));
    }

    // 4. ODT
    var odt = lin_srgb;
    if (u.output_odt == 2u) {
        odt = srgb_to_p3(lin_srgb);
    } else if (u.output_odt == 3u) {
        odt = srgb_to_rec2020(lin_srgb);
    }
    odt = clamp(odt, vec3<f32>(0.0), vec3<f32>(1.0)) * a;
    let ro = f32_to_f16_cm(odt.x);
    let go = f32_to_f16_cm(odt.y);
    let bo = f32_to_f16_cm(odt.z);
    let ao = f32_to_f16_cm(a);
    out_f16[idx] = vec2<u32>(ro | (go << 16u), bo | (ao << 16u));
}
"#;

/// 高速ブレンド＋マスク合成パス向けの幾何マスク形状
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FastMaskShape {
    #[default]
    None,
    SoftRect,
    SoftEllipse,
}

/// ブレンド＋トラックマット＋フェザーマスク高速パスの設定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FastBlendMaskParams {
    pub blend_mode: BlendMode,
    pub opacity: f32,
    pub matte_mode: Option<TrackMatteMode>,
    pub mask_shape: FastMaskShape,
    pub mask_center: [f32; 2],
    pub mask_half_size: [f32; 2],
    pub mask_feather: f32,
    pub invert_mask: bool,
}

impl Default for FastBlendMaskParams {
    fn default() -> Self {
        Self {
            blend_mode: BlendMode::Normal,
            opacity: 1.0,
            matte_mode: None,
            mask_shape: FastMaskShape::None,
            mask_center: [0.5, 0.5],
            mask_half_size: [0.35, 0.35],
            mask_feather: 0.05,
            invert_mask: false,
        }
    }
}

#[derive(Debug)]
struct GpuStripeBufferSet {
    byte_len: u64,
    buf_a: wgpu::Buffer,
    buf_b: wgpu::Buffer,
    buf_c: wgpu::Buffer,
    buf_out: wgpu::Buffer,
    buf_read: wgpu::Buffer,
}

/// GPU-Resident バッファ再利用キャッシュを備える v7.0 WGSL 高速コンピュート実行器
pub struct GpuV70FastPipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    blend_pipeline: wgpu::ComputePipeline,
    blend_bgl: wgpu::BindGroupLayout,
    blend_uniform: wgpu::Buffer,
    aces_pipeline: wgpu::ComputePipeline,
    aces_bgl: wgpu::BindGroupLayout,
    aces_uniform: wgpu::Buffer,
    stripe_cache: HashMap<u64, GpuStripeBufferSet>,
    pub adapter_name: String,
    pub backend_name: String,
    pub gpu_buffer_reuses: u64,
    pub gpu_buffer_allocs: u64,
}

impl GpuV70FastPipeline {
    pub fn try_headless() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = block_on_v70(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let info = adapter.get_info();
        let (device, queue) = block_on_v70(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ve_v70_fast_gpu_device"),
            ..Default::default()
        }))
        .ok()?;

        let blend_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_v70_fast_blend_mask_cs"),
            source: wgpu::ShaderSource::Wgsl(FAST_BLEND_MASK_WGSL.into()),
        });
        let ro_storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let rw_storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let blend_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_v70_blend_bgl"),
            entries: &[
                ro_storage(0),
                ro_storage(1),
                ro_storage(2),
                rw_storage(3),
                uniform_entry(4),
            ],
        });
        let blend_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_v70_blend_pl"),
            bind_group_layouts: &[Some(&blend_bgl)],
            immediate_size: 0,
        });
        let blend_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ve_v70_blend_pipeline"),
            layout: Some(&blend_pl),
            module: &blend_shader,
            entry_point: Some("cs_fast_blend_mask"),
            compilation_options: Default::default(),
            cache: None,
        });
        let blend_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v70_blend_uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let aces_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_v70_fast_aces_cs"),
            source: wgpu::ShaderSource::Wgsl(FAST_ACES_COLOR_MGMT_WGSL.into()),
        });
        let aces_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_v70_aces_bgl"),
            entries: &[ro_storage(0), rw_storage(1), uniform_entry(2)],
        });
        let aces_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_v70_aces_pl"),
            bind_group_layouts: &[Some(&aces_bgl)],
            immediate_size: 0,
        });
        let aces_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ve_v70_aces_pipeline"),
            layout: Some(&aces_pl),
            module: &aces_shader,
            entry_point: Some("cs_fast_aces_color_mgmt"),
            compilation_options: Default::default(),
            cache: None,
        });
        let aces_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v70_aces_uniform"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Some(Self {
            device,
            queue,
            blend_pipeline,
            blend_bgl,
            blend_uniform,
            aces_pipeline,
            aces_bgl,
            aces_uniform,
            stripe_cache: HashMap::new(),
            adapter_name: info.name,
            backend_name: format!("{:?}", info.backend),
            gpu_buffer_reuses: 0,
            gpu_buffer_allocs: 0,
        })
    }

    pub fn resident_buffer_count(&self) -> usize {
        self.stripe_cache.len() * 5
    }

    pub fn clear_gpu_buffers(&mut self) {
        self.stripe_cache.clear();
    }

    fn ensure_stripe_buffers(&mut self, byte_len: u64) -> &GpuStripeBufferSet {
        let aligned = byte_len.max(16).div_ceil(16) * 16;
        if self.stripe_cache.contains_key(&aligned) {
            self.gpu_buffer_reuses += 1;
        } else {
            if self.stripe_cache.len() >= 6 {
                self.stripe_cache.clear();
            }
            let mk_storage = |label, rw: bool| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: aligned,
                    usage: if rw {
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
                    } else {
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST
                    },
                    mapped_at_creation: false,
                })
            };
            let set = GpuStripeBufferSet {
                byte_len: aligned,
                buf_a: mk_storage("ve_v70_pool_a", false),
                buf_b: mk_storage("ve_v70_pool_b", false),
                buf_c: mk_storage("ve_v70_pool_c", false),
                buf_out: mk_storage("ve_v70_pool_out", true),
                buf_read: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ve_v70_pool_read"),
                    size: aligned,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
            };
            self.gpu_buffer_allocs += 1;
            self.stripe_cache.insert(aligned, set);
        }
        self.stripe_cache.get(&aligned).unwrap()
    }

    /// 4K / 8K 対応ストライプ分割 GPU ブレンド＋トラックマット＋ソフトマスク合成
    pub fn dispatch_blend_mask(
        &mut self,
        dst: &LinearImage,
        src: &LinearImage,
        matte: Option<&LinearImage>,
        params: &FastBlendMaskParams,
        host_pool: &mut ResidentFramePool,
    ) -> Option<LinearImage> {
        if dst.width != src.width || dst.height != src.height || dst.width == 0 || dst.height == 0 {
            return None;
        }
        if let Some(m) = matte
            && (m.width != dst.width || m.height != dst.height)
        {
            return None;
        }
        let w = dst.width;
        let h = dst.height;
        let row_bytes = (w as usize) * 8;
        let max_rows_per_stripe = (MAX_GPU_STRIPE_BYTES / row_bytes.max(8)).max(1) as u32;
        let stripe_rows_cap = max_rows_per_stripe.min(h);
        let stripe_cap_bytes = (stripe_rows_cap as u64) * (row_bytes as u64);

        let mut out_img = host_pool.acquire_linear_canvas(w, h, LinearPremul::TRANSPARENT);
        let mut pack_a = Vec::with_capacity(stripe_cap_bytes as usize);
        let mut pack_b = Vec::with_capacity(stripe_cap_bytes as usize);
        let mut pack_c = Vec::with_capacity(stripe_cap_bytes as usize);

        let blend_u32 = match params.blend_mode {
            BlendMode::Normal => 0u32,
            BlendMode::Add => 1u32,
            BlendMode::Multiply => 2u32,
            BlendMode::Screen => 3u32,
        };
        let matte_u32 = match params.matte_mode {
            None => 0u32,
            Some(TrackMatteMode::Alpha) => 1u32,
            Some(TrackMatteMode::AlphaInverted) => 2u32,
            Some(TrackMatteMode::Luma) => 3u32,
            Some(TrackMatteMode::LumaInverted) => 4u32,
        };
        let shape_u32 = match params.mask_shape {
            FastMaskShape::None => 0u32,
            FastMaskShape::SoftRect => 1u32,
            FastMaskShape::SoftEllipse => 2u32,
        };

        let mut y0 = 0u32;
        while y0 < h {
            let rows = (h - y0).min(stripe_rows_cap);
            let px_start = (y0 as usize) * (w as usize);
            let px_len = (rows as usize) * (w as usize);
            let stripe_bytes = (px_len * 8) as u64;

            pack_slice_rgba16f(&dst.pixels[px_start..px_start + px_len], &mut pack_a);
            pack_slice_rgba16f(&src.pixels[px_start..px_start + px_len], &mut pack_b);
            if let Some(m) = matte {
                pack_slice_rgba16f(&m.pixels[px_start..px_start + px_len], &mut pack_c);
            } else {
                pack_c.clear();
                pack_c.resize(px_len * 8, 0);
            }

            let mut u_bytes = [0u8; 64];
            let u_u32 = [
                w,
                rows,
                y0,
                h,
                blend_u32,
                matte_u32,
                shape_u32,
                u32::from(params.invert_mask),
            ];
            for (i, v) in u_u32.into_iter().enumerate() {
                u_bytes[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
            }
            let u_f32 = [
                params.opacity.clamp(0.0, 1.0),
                params.mask_feather.max(0.0005),
                params.mask_center[0],
                params.mask_center[1],
                params.mask_half_size[0].max(0.001),
                params.mask_half_size[1].max(0.001),
                0.0f32,
                0.0f32,
            ];
            for (i, v) in u_f32.into_iter().enumerate() {
                let off = 32 + i * 4;
                u_bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }

            self.ensure_stripe_buffers(stripe_cap_bytes);
            let set = self
                .stripe_cache
                .get(&(stripe_cap_bytes.max(16).div_ceil(16) * 16))?;
            self.queue.write_buffer(&set.buf_a, 0, &pack_a);
            self.queue.write_buffer(&set.buf_b, 0, &pack_b);
            self.queue.write_buffer(&set.buf_c, 0, &pack_c);
            self.queue.write_buffer(&self.blend_uniform, 0, &u_bytes);

            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ve_v70_blend_bg"),
                layout: &self.blend_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: set.buf_a.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: set.buf_b.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: set.buf_c.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: set.buf_out.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.blend_uniform.as_entire_binding(),
                    },
                ],
            });

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ve_v70_blend_enc"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("ve_v70_blend_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blend_pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), rows.div_ceil(8), 1);
            }
            encoder.copy_buffer_to_buffer(&set.buf_out, 0, &set.buf_read, 0, stripe_bytes);
            self.queue.submit(Some(encoder.finish()));

            let slice = set.buf_read.slice(0..stripe_bytes);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            rx.recv().ok()?.ok()?;
            let mapped = slice.get_mapped_range().ok()?;
            unpack_slice_rgba16f(&mapped, &mut out_img.pixels[px_start..px_start + px_len]);
            drop(mapped);
            set.buf_read.unmap();

            y0 += rows;
        }

        let _ = set_byte_len_touch(&self.stripe_cache);
        Some(out_img)
    }

    /// 4K / 8K 対応ストライプ分割 GPU ACEScg / OpenColorIO カラー管理パス
    pub fn dispatch_aces_color_mgmt(
        &mut self,
        img: &mut LinearImage,
        cfg: &ColorManagementConfig,
    ) -> bool {
        if !cfg.enabled || img.width == 0 || img.height == 0 {
            return true;
        }
        let w = img.width;
        let h = img.height;
        let row_bytes = (w as usize) * 8;
        let max_rows_per_stripe = (MAX_GPU_STRIPE_BYTES / row_bytes.max(8)).max(1) as u32;
        let stripe_rows_cap = max_rows_per_stripe.min(h);
        let stripe_cap_bytes = (stripe_rows_cap as u64) * (row_bytes as u64);
        let mut pack_in = Vec::with_capacity(stripe_cap_bytes as usize);

        let cs_code = |cs: ColorSpace| -> u32 {
            match cs {
                ColorSpace::LinearSrgb => 0,
                ColorSpace::AcesCg => 1,
                ColorSpace::DisplayP3 => 2,
                ColorSpace::Rec2020 => 3,
                ColorSpace::SLog3 => 4,
                ColorSpace::VLog => 5,
                ColorSpace::LogC3 => 6,
            }
        };
        let tm_code = match cfg.tone_mapper {
            ToneMapper::None => 0u32,
            ToneMapper::AcesFitted => 1u32,
            ToneMapper::AgX => 2u32,
            ToneMapper::ReinhardExtended => 3u32,
        };
        let exp_mult = 2.0f32.powf(cfg.exposure_stops.clamp(-6.0, 6.0));

        let mut y0 = 0u32;
        while y0 < h {
            let rows = (h - y0).min(stripe_rows_cap);
            let px_start = (y0 as usize) * (w as usize);
            let px_len = (rows as usize) * (w as usize);
            let stripe_bytes = (px_len * 8) as u64;

            pack_slice_rgba16f(&img.pixels[px_start..px_start + px_len], &mut pack_in);

            let mut u_bytes = [0u8; 48];
            let u_u32 = [
                w,
                rows,
                cs_code(cfg.input_idt),
                u32::from(cfg.working_space == ColorSpace::AcesCg),
                cs_code(cfg.output_odt),
                tm_code,
                u32::from(cfg.gamut_compress),
                0u32,
            ];
            for (i, v) in u_u32.into_iter().enumerate() {
                u_bytes[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
            }
            u_bytes[32..36].copy_from_slice(&exp_mult.to_le_bytes());

            self.ensure_stripe_buffers(stripe_cap_bytes);
            let Some(set) = self
                .stripe_cache
                .get(&(stripe_cap_bytes.max(16).div_ceil(16) * 16))
            else {
                return false;
            };
            self.queue.write_buffer(&set.buf_a, 0, &pack_in);
            self.queue.write_buffer(&self.aces_uniform, 0, &u_bytes);

            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ve_v70_aces_bg"),
                layout: &self.aces_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: set.buf_a.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: set.buf_out.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.aces_uniform.as_entire_binding(),
                    },
                ],
            });

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ve_v70_aces_enc"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("ve_v70_aces_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.aces_pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), rows.div_ceil(8), 1);
            }
            encoder.copy_buffer_to_buffer(&set.buf_out, 0, &set.buf_read, 0, stripe_bytes);
            self.queue.submit(Some(encoder.finish()));

            let slice = set.buf_read.slice(0..stripe_bytes);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            if rx.recv().ok().and_then(Result::ok).is_none() {
                return false;
            }
            let Ok(mapped) = slice.get_mapped_range() else {
                return false;
            };
            unpack_slice_rgba16f(&mapped, &mut img.pixels[px_start..px_start + px_len]);
            drop(mapped);
            set.buf_read.unmap();

            y0 += rows;
        }
        true
    }
}

fn set_byte_len_touch(map: &HashMap<u64, GpuStripeBufferSet>) -> u64 {
    map.values().map(|s| s.byte_len).sum()
}

fn pack_slice_rgba16f(src: &[LinearPremul], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(src.len() * 8);
    for p in src {
        out.extend_from_slice(&f16::from_f32(p.r).to_le_bytes());
        out.extend_from_slice(&f16::from_f32(p.g).to_le_bytes());
        out.extend_from_slice(&f16::from_f32(p.b).to_le_bytes());
        out.extend_from_slice(&f16::from_f32(p.a).to_le_bytes());
    }
}

fn unpack_slice_rgba16f(bytes: &[u8], dst: &mut [LinearPremul]) {
    for (slot, chunk) in dst.iter_mut().zip(bytes.as_chunks::<8>().0) {
        *slot = LinearPremul {
            r: f16::from_le_bytes([chunk[0], chunk[1]]).to_f32(),
            g: f16::from_le_bytes([chunk[2], chunk[3]]).to_f32(),
            b: f16::from_le_bytes([chunk[4], chunk[5]]).to_f32(),
            a: f16::from_le_bytes([chunk[6], chunk[7]]).to_f32(),
        };
    }
}

fn smoothstep_f32(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 決定論的 CPU フォールバック: ブレンド＋トラックマット＋ソフト矩形/楕円マスク合成
pub fn execute_fast_blend_mask_cpu(
    dst: &LinearImage,
    src: &LinearImage,
    matte: Option<&LinearImage>,
    params: &FastBlendMaskParams,
    host_pool: &mut ResidentFramePool,
) -> LinearImage {
    let w = dst.width.max(1);
    let h = dst.height.max(1);
    let mut out = host_pool.acquire_linear_canvas(w, h, LinearPremul::TRANSPARENT);
    if src.width != w || src.height != h {
        out.pixels.copy_from_slice(&dst.pixels);
        return out;
    }

    let op = params.opacity.clamp(0.0, 1.0);
    let f = params.mask_feather.max(0.0005);
    let hw = params.mask_half_size[0].max(0.001);
    let hh = params.mask_half_size[1].max(0.001);

    for y in 0..h {
        let uv_y = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let uv_x = (x as f32 + 0.5) / w as f32;

            let shape_w = match params.mask_shape {
                FastMaskShape::None => 1.0,
                FastMaskShape::SoftRect => {
                    let dx = (uv_x - params.mask_center[0]).abs() - hw;
                    let dy = (uv_y - params.mask_center[1]).abs() - hh;
                    let dist = dx.max(dy);
                    let m = 1.0 - smoothstep_f32(-f, f, dist);
                    if params.invert_mask { 1.0 - m } else { m }
                }
                FastMaskShape::SoftEllipse => {
                    let qx = (uv_x - params.mask_center[0]) / hw;
                    let qy = (uv_y - params.mask_center[1]) / hh;
                    let dist = (qx * qx + qy * qy).sqrt() - 1.0;
                    let m = 1.0 - smoothstep_f32(-f, f, dist);
                    if params.invert_mask { 1.0 - m } else { m }
                }
            };

            let matte_w = match (params.matte_mode, matte) {
                (Some(mode), Some(m_img)) if m_img.pixels.len() > idx => {
                    let mp = m_img.pixels[idx];
                    match mode {
                        TrackMatteMode::Alpha => mp.a.clamp(0.0, 1.0),
                        TrackMatteMode::AlphaInverted => (1.0 - mp.a).clamp(0.0, 1.0),
                        TrackMatteMode::Luma => mp.luma(),
                        TrackMatteMode::LumaInverted => (1.0 - mp.luma()).clamp(0.0, 1.0),
                    }
                }
                _ => 1.0,
            };

            let weight = (op * shape_w * matte_w).clamp(0.0, 1.0);
            let sp = src.pixels[idx];
            let weighted_src = LinearPremul {
                r: sp.r * weight,
                g: sp.g * weight,
                b: sp.b * weight,
                a: sp.a * weight,
            };
            out.pixels[idx] = weighted_src.blend(dst.pixels[idx], params.blend_mode);
        }
    }
    out
}

/// GPU が利用可能な場合は WGSL 高速パスを実行し、非搭載環境では決定論的 CPU パスへ透過フォールバックする
pub fn execute_v70_blend_mask_or_cpu(
    gpu: Option<&mut GpuV70FastPipeline>,
    dst: &LinearImage,
    src: &LinearImage,
    matte: Option<&LinearImage>,
    params: &FastBlendMaskParams,
    host_pool: &mut ResidentFramePool,
) -> (LinearImage, bool) {
    if let Some(pipe) = gpu
        && let Some(res) = pipe.dispatch_blend_mask(dst, src, matte, params, host_pool)
    {
        return (res, true);
    }
    (
        execute_fast_blend_mask_cpu(dst, src, matte, params, host_pool),
        false,
    )
}

/// GPU が利用可能な場合は WGSL ACEScg カラー管理を実行し、非搭載環境では決定論的 CPU パスへフォールバックする
pub fn execute_v70_aces_or_cpu(
    gpu: Option<&mut GpuV70FastPipeline>,
    img: &mut LinearImage,
    cfg: &ColorManagementConfig,
) -> bool {
    if !cfg.enabled {
        return false;
    }
    if let Some(pipe) = gpu
        && pipe.dispatch_aces_color_mgmt(img, cfg)
    {
        return true;
    }
    for px in &mut img.pixels {
        if px.a <= 1e-6 {
            continue;
        }
        let inv_a = 1.0 / px.a;
        let out = transform_color_pipeline_rgb([px.r * inv_a, px.g * inv_a, px.b * inv_a], cfg);
        px.r = out[0] * px.a;
        px.g = out[1] * px.a;
        px.b = out[2] * px.a;
    }
    false
}

/// v7.0 #47: GPU-Resident 4K/8K フレームプール＆シェーダー検証ステータス
#[derive(Debug, Clone)]
pub struct GpuV70Status {
    pub gpu_available: bool,
    pub adapter_name: String,
    pub backend_name: String,
    pub blend_mask_shader_valid: bool,
    pub aces_color_shader_valid: bool,
    pub max_stripe_bytes: usize,
}

pub fn detect_gpu_v70_status() -> GpuV70Status {
    let blend_ok = validate_wgsl(FAST_BLEND_MASK_WGSL).is_ok();
    let aces_ok = validate_wgsl(FAST_ACES_COLOR_MGMT_WGSL).is_ok();
    let instance = wgpu::Instance::default();
    let adapter_opt = block_on_v70(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .ok();

    if let Some(adapter) = adapter_opt {
        let info = adapter.get_info();
        GpuV70Status {
            gpu_available: true,
            adapter_name: info.name,
            backend_name: format!("{:?}", info.backend),
            blend_mask_shader_valid: blend_ok,
            aces_color_shader_valid: aces_ok,
            max_stripe_bytes: MAX_GPU_STRIPE_BYTES,
        }
    } else {
        GpuV70Status {
            gpu_available: false,
            adapter_name: "Deterministic CPU Fallback + Host Resident Pool".into(),
            backend_name: "Software".into(),
            blend_mask_shader_valid: blend_ok,
            aces_color_shader_valid: aces_ok,
            max_stripe_bytes: MAX_GPU_STRIPE_BYTES,
        }
    }
}

/// 4K / 8K フレームプール再利用＆高速合成パス実測レポート
#[derive(Debug, Clone)]
pub struct FramePoolBenchmarkReport {
    pub width: u32,
    pub height: u32,
    pub iterations: u32,
    pub executed_on_gpu: bool,
    pub backend_name: String,
    pub elapsed_ms: f64,
    pub estimated_fps: f64,
    pub pool_metrics: FramePoolMetrics,
}

/// 指定解像度（1080p / 4K `3840×2160` / 8K `7680×4320` 等）でフレームプール再利用と高速ブレンド/マスク/ACES パスをベンチマークする
pub fn benchmark_v70_frame_pool(
    width: u32,
    height: u32,
    iterations: u32,
    budget_mib: u64,
) -> FramePoolBenchmarkReport {
    let w = width.clamp(16, 8192);
    let h = height.clamp(16, 8192);
    let iters = iterations.clamp(2, 32);

    let mut host_pool = ResidentFramePool::with_budget_mib(budget_mib.max(256));
    let mut gpu = GpuV70FastPipeline::try_headless();
    let backend_name = gpu
        .as_ref()
        .map(|g| g.backend_name.clone())
        .unwrap_or_else(|| "Software (Deterministic CPU)".into());

    let dst = host_pool.acquire_linear_canvas(
        w,
        h,
        LinearPremul {
            r: 0.14,
            g: 0.22,
            b: 0.36,
            a: 1.0,
        },
    );
    let src = host_pool.acquire_linear_canvas(
        w,
        h,
        LinearPremul {
            r: 0.52,
            g: 0.28,
            b: 0.12,
            a: 0.85,
        },
    );

    let blend_params = FastBlendMaskParams {
        blend_mode: BlendMode::Screen,
        opacity: 0.9,
        matte_mode: None,
        mask_shape: FastMaskShape::SoftEllipse,
        mask_center: [0.5, 0.5],
        mask_half_size: [0.38, 0.38],
        mask_feather: 0.08,
        invert_mask: false,
    };
    let aces_cfg = ColorManagementConfig {
        enabled: true,
        input_idt: ColorSpace::LinearSrgb,
        working_space: ColorSpace::AcesCg,
        output_odt: ColorSpace::LinearSrgb,
        tone_mapper: ToneMapper::AgX,
        exposure_stops: 0.25,
        gamut_compress: true,
    };

    let t0 = Instant::now();
    let mut ran_gpu = false;
    for _ in 0..iters {
        let (mut blended, used_gpu) = execute_v70_blend_mask_or_cpu(
            gpu.as_mut(),
            &dst,
            &src,
            None,
            &blend_params,
            &mut host_pool,
        );
        ran_gpu |= used_gpu;
        let _ = execute_v70_aces_or_cpu(gpu.as_mut(), &mut blended, &aces_cfg);
        // プールへ返却して次イテレーションでゼロアロケーション再利用させる
        host_pool.release_linear_canvas(blended);
    }
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    host_pool.release_linear_canvas(src);
    host_pool.release_linear_canvas(dst);

    let gpu_bufs = gpu
        .as_ref()
        .map_or(0, GpuV70FastPipeline::resident_buffer_count);
    let pool_metrics = host_pool.metrics(gpu_bufs);
    let avg_ms = (elapsed_ms / f64::from(iters)).max(0.001);

    FramePoolBenchmarkReport {
        width: w,
        height: h,
        iterations: iters,
        executed_on_gpu: ran_gpu,
        backend_name,
        elapsed_ms,
        estimated_fps: 1000.0 / avg_ms,
        pool_metrics,
    }
}

// ============================================================================
// v7.0 #48: A/B スプリット比較ビュー合成器
// ============================================================================

/// 2 枚の `LinearImage` (A 側 / B 側) を `AbCompareConfig`（垂直・水平・斜めワイプ・差分強調）に従って合成する
pub fn compose_ab_split_linear(
    frame_a: &LinearImage,
    frame_b: &LinearImage,
    cfg: &AbCompareConfig,
) -> LinearImage {
    if cfg.mode == AbCompareMode::Off || frame_b.width == 0 || frame_b.height == 0 {
        return frame_b.clone();
    }
    let w = frame_b.width;
    let h = frame_b.height;
    let resized_a;
    let a_ref = if frame_a.width == w && frame_a.height == h {
        frame_a
    } else {
        resized_a = scale_linear_image(frame_a, w, h);
        &resized_a
    };

    let (lhs, rhs) = if cfg.swap_sides {
        (frame_b, a_ref)
    } else {
        (a_ref, frame_b)
    };

    let ratio = cfg.split_ratio.clamp(0.05, 0.95);
    let gain = cfg.diff_gain.clamp(1.0, 16.0);
    let divider_px = LinearPremul {
        r: 0.18,
        g: 0.88,
        b: 1.0,
        a: 1.0,
    };

    let mut out = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let split_x = ((wf * ratio).round() as i32).clamp(0, w as i32 - 1);
    let split_y = ((hf * ratio).round() as i32).clamp(0, h as i32 - 1);

    for y in 0..h {
        let ny = (y as f32 + 0.5) / hf;
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let nx = (x as f32 + 0.5) / wf;
            let pa = lhs.pixels[idx];
            let pb = rhs.pixels[idx];

            out.pixels[idx] = match cfg.mode {
                AbCompareMode::Off => pb,
                AbCompareMode::VerticalSplit => {
                    if cfg.show_divider && ((x as i32) - split_x).abs() <= 1 {
                        divider_px
                    } else if (x as i32) < split_x {
                        pa
                    } else {
                        pb
                    }
                }
                AbCompareMode::HorizontalSplit => {
                    if cfg.show_divider && ((y as i32) - split_y).abs() <= 1 {
                        divider_px
                    } else if (y as i32) < split_y {
                        pa
                    } else {
                        pb
                    }
                }
                AbCompareMode::DiagonalSplit => {
                    let proj = 0.5 * (nx + ny);
                    let dist = (proj - ratio).abs() * (wf.min(hf));
                    if cfg.show_divider && dist <= 1.2 {
                        divider_px
                    } else if proj < ratio {
                        pa
                    } else {
                        pb
                    }
                }
                AbCompareMode::DifferenceAmplified => LinearPremul {
                    r: ((pa.r - pb.r).abs() * gain).clamp(0.0, 1.0),
                    g: ((pa.g - pb.g).abs() * gain).clamp(0.0, 1.0),
                    b: ((pa.b - pb.b).abs() * gain).clamp(0.0, 1.0),
                    a: 1.0,
                },
            };
        }
    }

    if cfg.show_divider && cfg.mode != AbCompareMode::DifferenceAmplified && w >= 64 && h >= 32 {
        let left_char = if cfg.swap_sides { b'B' } else { b'A' };
        let right_char = if cfg.swap_sides { b'A' } else { b'B' };
        draw_corner_badge(&mut out, 8, 8, left_char);
        draw_corner_badge(
            &mut out,
            w.saturating_sub(20),
            h.saturating_sub(20),
            right_char,
        );
    }

    out
}

fn draw_corner_badge(img: &mut LinearImage, x0: u32, y0: u32, ch: u8) {
    let glyph: [u8; 7] = match ch {
        b'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        _ => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
    };
    let bg = LinearPremul {
        r: 0.02,
        g: 0.03,
        b: 0.05,
        a: 0.85,
    };
    let fg = LinearPremul {
        r: 0.95,
        g: 0.98,
        b: 1.0,
        a: 1.0,
    };
    for dy in 0..11u32 {
        for dx in 0..9u32 {
            let px = x0 + dx;
            let py = y0 + dy;
            if px < img.width && py < img.height {
                img.pixels[(py * img.width + px) as usize] = bg;
            }
        }
    }
    for (row_i, bits) in glyph.iter().enumerate() {
        for col_i in 0..5u32 {
            if (bits >> (4 - col_i)) & 1 == 1 {
                let px = x0 + 2 + col_i;
                let py = y0 + 2 + row_i as u32;
                if px < img.width && py < img.height {
                    img.pixels[(py * img.width + px) as usize] = fg;
                }
            }
        }
    }
}

/// シーンの `AbCompareConfig` に基づいて A 側・B 側を描画し、スプリット比較合成フレームを返す
pub fn render_ab_compare_scene(
    graph: &mut crate::graph::RenderGraph,
    project: &Project,
    scene_id: SceneId,
    time: RationalTime,
    target_w: u32,
    target_h: u32,
    frozen_still_a: Option<&LinearImage>,
) -> Option<LinearImage> {
    let scene = project.scene(scene_id)?;
    let cfg = scene.ab_compare.clone();
    let frame_b =
        graph.render_project_scene_scaled(project, scene_id, time, target_w, target_h, false)?;
    if !cfg.is_active() {
        return Some(frame_b);
    }

    let frame_a = match cfg.reference {
        AbReferenceSource::UngradedBypass => {
            let mut bypassed = project.clone();
            if let Some(sc) = bypassed.scenes.iter_mut().find(|s| s.id == scene_id) {
                sc.color_grade.enabled = false;
                sc.color_management.enabled = false;
                sc.cinema_finishing.enabled = false;
                sc.optical_vfx.enabled = false;
            }
            graph
                .render_project_scene_scaled(&bypassed, scene_id, time, target_w, target_h, false)?
        }
        AbReferenceSource::FrozenStill => {
            if let Some(still) = frozen_still_a {
                still.clone()
            } else {
                let mut bypassed = project.clone();
                if let Some(sc) = bypassed.scenes.iter_mut().find(|s| s.id == scene_id) {
                    sc.color_grade.enabled = false;
                    sc.color_management.enabled = false;
                    sc.cinema_finishing.enabled = false;
                    sc.optical_vfx.enabled = false;
                }
                graph.render_project_scene_scaled(
                    &bypassed, scene_id, time, target_w, target_h, false,
                )?
            }
        }
        AbReferenceSource::ReferenceTimecode => {
            let ref_t = RationalTime::from_seconds(cfg.reference_sec.max(0.0));
            graph
                .render_project_scene_scaled(project, scene_id, ref_t, target_w, target_h, false)?
        }
        AbReferenceSource::AlternateScene => {
            let alt_id = cfg.reference_scene_id.unwrap_or(scene_id);
            graph.render_project_scene_scaled(project, alt_id, time, target_w, target_h, false)?
        }
    };

    Some(compose_ab_split_linear(&frame_a, &frame_b, &cfg))
}

fn block_on_v70<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(out) => return out,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v70_wgsl_fast_blend_mask_and_aces_shaders_pass_naga_validation() {
        validate_wgsl(FAST_BLEND_MASK_WGSL).expect("FAST_BLEND_MASK_WGSL must pass naga");
        validate_wgsl(FAST_ACES_COLOR_MGMT_WGSL).expect("FAST_ACES_COLOR_MGMT_WGSL must pass naga");
    }

    #[test]
    fn resident_frame_pool_reuses_4k_buffers_and_evicts_lru_on_budget_limit() {
        // 4K LinearF32x4 1枚 = 3840 * 2160 * 16 = 132.71 MiB
        // 予算 200 MiB なら 1 枚キャッシュ可能、別サイズ確保時は古いアイドルが LRU Evict される
        let mut pool = ResidentFramePool::with_budget_mib(200);
        let c1 = pool.acquire_linear_canvas(3840, 2160, LinearPremul::TRANSPARENT);
        assert_eq!(pool.metrics(0).allocations_created, 1);
        assert_eq!(pool.metrics(0).pool_reuses, 0);
        pool.release_linear_canvas(c1);

        // 同解像度 4K 再取得 -> ゼロアロケーションで再利用ヒット！
        let c2 = pool.acquire_linear_canvas(3840, 2160, LinearPremul::TRANSPARENT);
        assert_eq!(pool.metrics(0).allocations_created, 1);
        assert_eq!(pool.metrics(0).pool_reuses, 1);
        pool.release_linear_canvas(c2);

        // 別サイズ (DCI 4K 4096x2160 = 141.55 MiB) を要求すると 200 MiB 超過のため LRU Eviction が発生
        let c3 = pool.acquire_linear_canvas(4096, 2160, LinearPremul::TRANSPARENT);
        assert_eq!(pool.metrics(0).evictions, 1);
        assert!(pool.metrics(0).resident_bytes <= pool.metrics(0).budget_bytes);
        pool.release_linear_canvas(c3);
    }

    #[test]
    fn fast_blend_mask_and_ab_split_compare_are_deterministic() {
        let mut pool = ResidentFramePool::with_budget_mib(64);
        let a = LinearImage::new(
            64,
            36,
            LinearPremul {
                r: 0.1,
                g: 0.2,
                b: 0.8,
                a: 1.0,
            },
        );
        let b = LinearImage::new(
            64,
            36,
            LinearPremul {
                r: 0.9,
                g: 0.3,
                b: 0.1,
                a: 1.0,
            },
        );

        let mut gpu = GpuV70FastPipeline::try_headless();
        let params = FastBlendMaskParams {
            blend_mode: BlendMode::Add,
            opacity: 0.5,
            matte_mode: None,
            mask_shape: FastMaskShape::SoftRect,
            mask_center: [0.5, 0.5],
            mask_half_size: [0.3, 0.3],
            mask_feather: 0.05,
            invert_mask: false,
        };
        let (mixed, _) =
            execute_v70_blend_mask_or_cpu(gpu.as_mut(), &a, &b, None, &params, &mut pool);
        // 中央ピクセルではマスク内なので Add 加算 (0.1 + 0.9 * 0.5 = 0.55)、四隅ではマスク外なので 0.1 のまま
        let center = mixed.pixels[(18 * 64 + 32) as usize];
        let corner = mixed.pixels[0];
        assert!((center.r - 0.55).abs() < 0.02, "center.r={}", center.r);
        assert!((corner.r - 0.10).abs() < 0.02, "corner.r={}", corner.r);

        // A/B Vertical Split (50%)
        let ab_cfg = AbCompareConfig {
            mode: AbCompareMode::VerticalSplit,
            split_ratio: 0.5,
            show_divider: false,
            ..AbCompareConfig::default()
        };
        let ab = compose_ab_split_linear(&a, &b, &ab_cfg);
        assert!((ab.pixels[10].r - 0.1).abs() < 1e-4);
        assert!((ab.pixels[54].r - 0.9).abs() < 1e-4);
    }
}
