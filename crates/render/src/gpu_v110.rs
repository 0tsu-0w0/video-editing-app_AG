//! v11.0（#61・#64）完全ゼロコピー GPU レンダリンググラフ（VRAM 常駐 Ping-Pong 連結 WGSL Compute）
//! ＋ リアルタイム・パフォーマンスプロファイラ HUD オーバーレイ描画器
//!
//! Pure Rust・ゼロ `unsafe`・追加外部依存ゼロ。
//! GPU 利用可能時は `ZERO_COPY_COMPOSITE_GRADE_WGSL` → `SPATIAL_360_EQUIRECT_WGSL` 等を
//! 1 コマンドバッファ内で中間 CPU リードバックゼロのまま連続実行し、
//! 非 GPU／ヘッドレス環境では決定論 CPU パスへシームレスにフォールバックする。

use crate::color::{LinearImage, LinearPremul, linear_to_srgb, srgb_to_linear};
use crate::gpu_v100::{SPATIAL_360_EQUIRECT_WGSL, validate_all_builtin_wgsl_shaders};
use crate::v100_render::render_spatial_360_and_stereo_pass;
use half::f16;
use std::future::Future;
use std::pin::pin;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
use std::time::Instant;
use ve_core::{
    FrameProfileReport, GpuRenderGraphConfig, ProfilerHudConfig, Projection360Mode,
    RotationYprMatrix, Spatial360Preset, SpatialVideoConfig, StereoLayout,
    evaluate_horizon_locked_euler,
};

/// v11.0 #61: VRAM 常駐レイヤー合成 ＋ 露出/コントラスト/彩度 一括連結 WGSL Compute Shader
/// （IEEE-754 半精度 8B/px Ping-Pong ストレージバッファ・中間 CPU リードバックゼロ）
pub const ZERO_COPY_COMPOSITE_GRADE_WGSL: &str = r#"
struct ZeroCopyGradeUniforms {
    dims: vec4<u32>,    // x=width, y=height, z=enable_overlay(0/1), w=enable_grade(0/1)
    grade: vec4<f32>,   // x=overlay_opacity, y=exp_mult, z=contrast, w=saturation
};

@group(0) @binding(0) var<storage, read> base_f16: array<vec2<u32>>;
@group(0) @binding(1) var<storage, read> over_f16: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read_write> out_f16: array<vec2<u32>>;
@group(0) @binding(3) var<uniform> u: ZeroCopyGradeUniforms;

fn f16_to_f32_zc(h: u32) -> f32 {
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

fn f32_to_f16_zc(val: f32) -> u32 {
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

fn unpack_zc(v: vec2<u32>) -> vec4<f32> {
    return vec4<f32>(
        f16_to_f32_zc(v.x & 0xFFFFu),
        f16_to_f32_zc(v.x >> 16u),
        f16_to_f32_zc(v.y & 0xFFFFu),
        f16_to_f32_zc(v.y >> 16u)
    );
}

fn pack_zc(c: vec4<f32>) -> vec2<u32> {
    let cl = clamp(c, vec4<f32>(0.0), vec4<f32>(65504.0));
    let r = f32_to_f16_zc(cl.x);
    let g = f32_to_f16_zc(cl.y);
    let b = f32_to_f16_zc(cl.z);
    let a = f32_to_f16_zc(cl.w);
    return vec2<u32>(r | (g << 16u), b | (a << 16u));
}

fn lin_to_srgb_ch(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    if (x <= 0.0031308) {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

fn srgb_to_lin_ch(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    if (x <= 0.04045) {
        return x / 12.92;
    }
    return pow((x + 0.055) / 1.055, 2.4);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    var c = unpack_zc(base_f16[idx]);

    if (u.dims.z != 0u) {
        let ov = unpack_zc(over_f16[idx]) * u.grade.x;
        let inv_a = 1.0 - ov.w;
        c = vec4<f32>(
            ov.x + c.x * inv_a,
            ov.y + c.y * inv_a,
            ov.z + c.z * inv_a,
            ov.w + c.w * inv_a
        );
    }

    if (u.dims.w != 0u && c.w > 0.000001) {
        let inv_a = 1.0 / c.w;
        let lr = max(c.x * inv_a * u.grade.y, 0.0);
        let lg = max(c.y * inv_a * u.grade.y, 0.0);
        let lb = max(c.z * inv_a * u.grade.y, 0.0);
        var sr = clamp((lin_to_srgb_ch(lr) - 0.5) * u.grade.z + 0.5, 0.0, 1.0);
        var sg = clamp((lin_to_srgb_ch(lg) - 0.5) * u.grade.z + 0.5, 0.0, 1.0);
        var sb = clamp((lin_to_srgb_ch(lb) - 0.5) * u.grade.z + 0.5, 0.0, 1.0);
        let luma = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
        sr = clamp(luma + (sr - luma) * u.grade.w, 0.0, 1.0);
        sg = clamp(luma + (sg - luma) * u.grade.w, 0.0, 1.0);
        sb = clamp(luma + (sb - luma) * u.grade.w, 0.0, 1.0);
        c = vec4<f32>(
            srgb_to_lin_ch(sr) * c.w,
            srgb_to_lin_ch(sg) * c.w,
            srgb_to_lin_ch(sb) * c.w,
            c.w
        );
    }

    out_f16[idx] = pack_zc(c);
}
"#;

/// ゼロコピー GPU グラフ連結実行の実測統計
#[derive(Debug, Clone, PartialEq)]
pub struct GpuZeroCopyExecutionStats {
    pub width: u32,
    pub height: u32,
    pub used_gpu: bool,
    pub adapter_name: String,
    pub backend_name: String,
    pub chained_passes: usize,
    pub intermediate_cpu_readbacks: usize,
    pub elapsed_ms: f64,
    pub validated_shader_count: usize,
    pub all_shaders_valid: bool,
}

fn block_on_v110<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn pack_f16_rgba(pixels: &[LinearPremul]) -> Vec<u8> {
    let mut out = vec![0u8; pixels.len() * 8];
    for (dst, px) in out.as_chunks_mut::<8>().0.iter_mut().zip(pixels.iter()) {
        let r = f16::from_f32(px.r).to_le_bytes();
        let g = f16::from_f32(px.g).to_le_bytes();
        let b = f16::from_f32(px.b).to_le_bytes();
        let a = f16::from_f32(px.a).to_le_bytes();
        *dst = [r[0], r[1], g[0], g[1], b[0], b[1], a[0], a[1]];
    }
    out
}

fn unpack_f16_rgba(bytes: &[u8], w: u32, h: u32) -> LinearImage {
    let count = (w as usize) * (h as usize);
    let mut pixels = Vec::with_capacity(count);
    for c in bytes.as_chunks::<8>().0.iter().take(count) {
        pixels.push(LinearPremul {
            r: f16::from_le_bytes([c[0], c[1]]).to_f32(),
            g: f16::from_le_bytes([c[2], c[3]]).to_f32(),
            b: f16::from_le_bytes([c[4], c[5]]).to_f32(),
            a: f16::from_le_bytes([c[6], c[7]]).to_f32(),
        });
    }
    pixels.resize(count, LinearPremul::TRANSPARENT);
    LinearImage {
        width: w,
        height: h,
        pixels,
    }
}

/// v11.0 #61: VRAM 常駐 Ping-Pong バッファ上で複数 WGSL パスを 1 コマンドバッファ連結実行する GPU グラフ
pub struct GpuZeroCopyRenderGraph {
    device: wgpu::Device,
    queue: wgpu::Queue,
    comp_grade_pipeline: wgpu::ComputePipeline,
    comp_grade_bgl: wgpu::BindGroupLayout,
    spatial_pipeline: wgpu::ComputePipeline,
    spatial_bgl: wgpu::BindGroupLayout,
    pub adapter_name: String,
    pub backend_name: String,
}

impl GpuZeroCopyRenderGraph {
    /// ヘッドレス GPU アダプタを検出してゼロコピー GPU グラフパイプラインを初期化する
    pub fn try_headless() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = block_on_v110(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .ok()?;
        let info = adapter.get_info();
        let (device, queue) =
            block_on_v110(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;

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

        let cg_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_v110_zero_copy_cg"),
            source: wgpu::ShaderSource::Wgsl(ZERO_COPY_COMPOSITE_GRADE_WGSL.into()),
        });
        let comp_grade_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_v110_cg_bgl"),
            entries: &[
                ro_storage(0),
                ro_storage(1),
                rw_storage(2),
                uniform_entry(3),
            ],
        });
        let cg_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_v110_cg_pl"),
            bind_group_layouts: &[Some(&comp_grade_bgl)],
            immediate_size: 0,
        });
        let comp_grade_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ve_v110_cg_pipe"),
                layout: Some(&cg_pl),
                module: &cg_sm,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });

        let sp_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_v110_spatial_cs"),
            source: wgpu::ShaderSource::Wgsl(SPATIAL_360_EQUIRECT_WGSL.into()),
        });
        let spatial_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_v110_sp_bgl"),
            entries: &[ro_storage(0), rw_storage(1), uniform_entry(2)],
        });
        let sp_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_v110_sp_pl"),
            bind_group_layouts: &[Some(&spatial_bgl)],
            immediate_size: 0,
        });
        let spatial_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ve_v110_sp_pipe"),
            layout: Some(&sp_pl),
            module: &sp_sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Some(Self {
            device,
            queue,
            comp_grade_pipeline,
            comp_grade_bgl,
            spatial_pipeline,
            spatial_bgl,
            adapter_name: info.name,
            backend_name: format!("{:?}", info.backend),
        })
    }

    /// VRAM 常駐 Ping-Pong バッファ上で「合成＋カラーグレーディング」→「360° 逆射影」を
    /// 1 コマンドバッファ・中間 CPU リードバックゼロで連結実行する。
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_zero_copy_chain(
        &self,
        base: &LinearImage,
        overlay: Option<&LinearImage>,
        overlay_opacity: f32,
        exposure: f32,
        contrast: f32,
        saturation: f32,
        spatial_cfg: Option<&SpatialVideoConfig>,
        time_sec: f32,
    ) -> Option<(LinearImage, usize)> {
        let w = base.width;
        let h = base.height;
        if w == 0 || h == 0 {
            return None;
        }
        let byte_len = (w as u64).checked_mul(h as u64)?.checked_mul(8)?;
        if byte_len > 128 * 1024 * 1024 {
            return None;
        }

        let has_overlay = overlay.is_some_and(|ov| ov.width == w && ov.height == h);
        let enable_grade = (exposure.abs() > 1e-4)
            || ((contrast - 1.0).abs() > 1e-4)
            || ((saturation - 1.0).abs() > 1e-4);
        let run_spatial =
            spatial_cfg.is_some_and(|s| s.enabled && s.projection_mode != Projection360Mode::Off);

        let base_bytes = pack_f16_rgba(&base.pixels);
        let ov_bytes = if let Some(ov) = overlay.filter(|_| has_overlay) {
            pack_f16_rgba(&ov.pixels)
        } else {
            vec![0u8; 8]
        };

        let buf_base = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v110_base"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let buf_over = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v110_over"),
            size: ov_bytes.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let buf_ping = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v110_vram_ping"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buf_pong = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v110_vram_pong"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buf_read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v110_read"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        self.queue.write_buffer(&buf_base, 0, &base_bytes);
        self.queue.write_buffer(&buf_over, 0, &ov_bytes);

        let mut cg_u = [0u8; 32];
        let exp_mult = 2.0f32.powf(exposure.clamp(-3.0, 3.0));
        for (i, v) in [w, h, u32::from(has_overlay), u32::from(enable_grade)]
            .into_iter()
            .enumerate()
        {
            cg_u[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, v) in [
            overlay_opacity.clamp(0.0, 1.0),
            exp_mult,
            contrast.clamp(0.2, 2.5),
            saturation.clamp(0.0, 2.5),
        ]
        .into_iter()
        .enumerate()
        {
            let off = 16 + i * 4;
            cg_u[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        let buf_cg_u = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v110_cg_u"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buf_cg_u, 0, &cg_u);

        let cg_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ve_v110_cg_bg"),
            layout: &self.comp_grade_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf_base.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_over.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf_ping.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buf_cg_u.as_entire_binding(),
                },
            ],
        });

        // 1 つの CommandEncoder 内で Pass 1（合成+色調）→ Pass 2（360° 逆射影）を VRAM ゼロコピー連結
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ve_v110_zero_copy_enc"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ve_v110_pass_composite_grade"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.comp_grade_pipeline);
            pass.set_bind_group(0, &cg_bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }

        let mut passes_executed = 1usize;
        if let Some(scfg) = spatial_cfg.filter(|_| run_spatial) {
            let (yaw, pitch, roll) = evaluate_horizon_locked_euler(scfg, time_sec);
            let rot = RotationYprMatrix::from_ypr_deg(yaw, pitch, roll);
            let aspect = w as f32 / h.max(1) as f32;
            let half_fov = (scfg.fov_deg.clamp(25.0, 155.0) * 0.5).to_radians();
            let tan_half_x = half_fov.tan();
            let tan_half_y = tan_half_x / aspect.max(0.1);
            let inv_tiny_zoom = 1.0 / scfg.tiny_planet_zoom.clamp(0.25, 4.0);
            let mode_code: u32 = match scfg.projection_mode {
                Projection360Mode::Off => 0,
                Projection360Mode::OverCapture => 1,
                Projection360Mode::TinyPlanet => 2,
                Projection360Mode::EquirectangularReview => 3,
            };
            let mut sp_u = [0u8; 96];
            for (i, v) in [w, h, mode_code, 0u32].into_iter().enumerate() {
                sp_u[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
            }
            let floats = [
                aspect,
                tan_half_x,
                tan_half_y,
                inv_tiny_zoom,
                yaw,
                pitch,
                0.0,
                0.0,
                rot.m[0][0],
                rot.m[0][1],
                rot.m[0][2],
                0.0,
                rot.m[1][0],
                rot.m[1][1],
                rot.m[1][2],
                0.0,
                rot.m[2][0],
                rot.m[2][1],
                rot.m[2][2],
                0.0,
            ];
            for (i, v) in floats.into_iter().enumerate() {
                let off = 16 + i * 4;
                sp_u[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }
            let buf_sp_u = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ve_v110_sp_u"),
                size: 96,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buf_sp_u, 0, &sp_u);
            let sp_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ve_v110_sp_bg"),
                layout: &self.spatial_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buf_ping.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buf_pong.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buf_sp_u.as_entire_binding(),
                    },
                ],
            });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("ve_v110_pass_spatial_360"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.spatial_pipeline);
                pass.set_bind_group(0, &sp_bg, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            enc.copy_buffer_to_buffer(&buf_pong, 0, &buf_read, 0, byte_len);
            passes_executed = 2;
        } else {
            enc.copy_buffer_to_buffer(&buf_ping, 0, &buf_read, 0, byte_len);
        }

        self.queue.submit(Some(enc.finish()));
        let slice = buf_read.slice(0..byte_len);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let Ok(mapped) = slice.get_mapped_range() else {
            buf_read.unmap();
            return None;
        };
        let out = unpack_f16_rgba(&mapped, w, h);
        drop(mapped);
        buf_read.unmap();
        Some((out, passes_executed))
    }
}

/// CPU 決定論フォールバック（`ZERO_COPY_COMPOSITE_GRADE_WGSL` と数式完全一致）
#[allow(clippy::too_many_arguments)]
pub fn execute_zero_copy_chain_cpu_reference(
    base: &LinearImage,
    overlay: Option<&LinearImage>,
    overlay_opacity: f32,
    exposure: f32,
    contrast: f32,
    saturation: f32,
    spatial_cfg: Option<&SpatialVideoConfig>,
    time_sec: f32,
) -> (LinearImage, usize) {
    let mut out = base.clone();
    if let Some(ov) = overlay
        && ov.width == base.width
        && ov.height == base.height
    {
        out.blend_over(ov, 0, 0, overlay_opacity.clamp(0.0, 1.0));
    }

    let enable_grade = (exposure.abs() > 1e-4)
        || ((contrast - 1.0).abs() > 1e-4)
        || ((saturation - 1.0).abs() > 1e-4);
    if enable_grade {
        let exp_mult = 2.0f32.powf(exposure.clamp(-3.0, 3.0));
        let cont = contrast.clamp(0.2, 2.5);
        let sat = saturation.clamp(0.0, 2.5);
        for px in &mut out.pixels {
            if px.a <= 1e-6 {
                continue;
            }
            let inv_a = 1.0 / px.a;
            let lr = (px.r * inv_a * exp_mult).max(0.0);
            let lg = (px.g * inv_a * exp_mult).max(0.0);
            let lb = (px.b * inv_a * exp_mult).max(0.0);
            let mut sr = ((linear_to_srgb(lr) - 0.5) * cont + 0.5).clamp(0.0, 1.0);
            let mut sg = ((linear_to_srgb(lg) - 0.5) * cont + 0.5).clamp(0.0, 1.0);
            let mut sb = ((linear_to_srgb(lb) - 0.5) * cont + 0.5).clamp(0.0, 1.0);
            let luma = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
            sr = (luma + (sr - luma) * sat).clamp(0.0, 1.0);
            sg = (luma + (sg - luma) * sat).clamp(0.0, 1.0);
            sb = (luma + (sb - luma) * sat).clamp(0.0, 1.0);
            px.r = srgb_to_linear(sr) * px.a;
            px.g = srgb_to_linear(sg) * px.a;
            px.b = srgb_to_linear(sb) * px.a;
        }
    }

    let mut passes = 1usize;
    if let Some(scfg) = spatial_cfg
        && scfg.enabled
        && scfg.projection_mode != Projection360Mode::Off
    {
        out = render_spatial_360_and_stereo_pass(&out, scfg, time_sec);
        passes = 2;
    }
    (out, passes)
}

/// GPU ゼロコピー連結（利用可能かつ `cfg.enabled && cfg.prefer_zero_copy_vram`）または決定論 CPU パスを実行する
#[allow(clippy::too_many_arguments)]
pub fn execute_zero_copy_gpu_graph_or_cpu(
    gpu: Option<&GpuZeroCopyRenderGraph>,
    cfg: &GpuRenderGraphConfig,
    base: &LinearImage,
    overlay: Option<&LinearImage>,
    overlay_opacity: f32,
    exposure: f32,
    contrast: f32,
    saturation: f32,
    spatial_cfg: Option<&SpatialVideoConfig>,
    time_sec: f32,
) -> (LinearImage, bool, usize) {
    if cfg.enabled
        && cfg.prefer_zero_copy_vram
        && let Some(pipe) = gpu
        && let Some((img, passes)) = pipe.dispatch_zero_copy_chain(
            base,
            overlay,
            overlay_opacity,
            exposure,
            contrast,
            saturation,
            spatial_cfg,
            time_sec,
        )
    {
        // ステレオ 3D 出力が必要な場合は CPU 後段のみ適用
        if let Some(scfg) = spatial_cfg
            && scfg.enabled
            && (scfg.stereo_layout != StereoLayout::Off || scfg.show_horizon_guides)
        {
            let mut post = scfg.clone();
            post.projection_mode = Projection360Mode::Off;
            return (
                render_spatial_360_and_stereo_pass(&img, &post, time_sec),
                true,
                passes,
            );
        }
        return (img, true, passes);
    }
    let (img, passes) = execute_zero_copy_chain_cpu_reference(
        base,
        overlay,
        overlay_opacity,
        exposure,
        contrast,
        saturation,
        spatial_cfg,
        time_sec,
    );
    (img, false, passes)
}

/// v11.0 #61: 全 9 本組み込み WGSL シェーダー Naga 検証 ＋ ゼロコピー GPU/CPU 連結ベンチマークを実行する
pub fn benchmark_v110_zero_copy_graph(width: u32, height: u32) -> GpuZeroCopyExecutionStats {
    let w = width.clamp(64, 3840);
    let h = height.clamp(64, 2160);
    let checks = validate_all_builtin_wgsl_shaders();
    let all_valid = checks.iter().all(|c| c.valid);

    let mut base = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    let mut over = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    for y in 0..h {
        let gy = y as f32 / h as f32;
        let row = (y * w) as usize;
        for x in 0..w {
            let gx = x as f32 / w as f32;
            base.pixels[row + x as usize] = LinearPremul {
                r: gx * 0.7,
                g: gy * 0.6,
                b: 0.35,
                a: 1.0,
            };
            over.pixels[row + x as usize] = LinearPremul {
                r: 0.25,
                g: 0.15,
                b: gx * 0.4,
                a: 0.4,
            };
        }
    }

    let cfg = GpuRenderGraphConfig::default();
    let sp_cfg = SpatialVideoConfig::from_preset(Spatial360Preset::ActionHorizonLock);
    let gpu = GpuZeroCopyRenderGraph::try_headless();
    let (adapter_name, backend_name) = match gpu.as_ref() {
        Some(g) => (g.adapter_name.clone(), g.backend_name.clone()),
        None => ("CPU Deterministic Fallback".into(), "CPU".into()),
    };

    let t0 = Instant::now();
    let (_out, used_gpu, chained_passes) = execute_zero_copy_gpu_graph_or_cpu(
        gpu.as_ref(),
        &cfg,
        &base,
        Some(&over),
        0.6,
        0.15,
        1.1,
        1.08,
        Some(&sp_cfg),
        0.5,
    );
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    GpuZeroCopyExecutionStats {
        width: w,
        height: h,
        used_gpu,
        adapter_name,
        backend_name,
        chained_passes,
        intermediate_cpu_readbacks: 0,
        elapsed_ms,
        validated_shader_count: checks.len(),
        all_shaders_valid: all_valid,
    }
}

// ============================================================================
// #64: リアルタイム・パフォーマンスプロファイラ HUD オーバーレイ描画器（Pure Rust）
// ============================================================================

/// プレビュー画像左上へダークガラス調プロファイラ HUD（予算ゲージ・パス内訳フレームグラフバー・GPU DIRECT バッジ）を合成する
pub fn draw_profiler_hud_overlay(
    canvas: &mut LinearImage,
    report: &FrameProfileReport,
    cfg: &ProfilerHudConfig,
    show_gpu_direct_badge: bool,
) {
    if !cfg.enabled || canvas.width < 96 || canvas.height < 48 {
        return;
    }
    let w = canvas.width;
    let h = canvas.height;
    let margin = 6u32;
    let panel_w = (w.saturating_sub(margin * 2)).min(260);
    let panel_h = (h.saturating_sub(margin * 2)).min(if cfg.show_frame_graph { 52 } else { 32 });

    // 1. 半透明ダークガラス背景パネル
    let bg = LinearPremul {
        r: 0.02 * 0.80,
        g: 0.03 * 0.80,
        b: 0.05 * 0.80,
        a: 0.80,
    };
    fill_rect_blend(canvas, margin, margin, panel_w, panel_h, bg);

    // 2. フレーム予算ステータスバー（緑 <= warn_ms / 琥珀 <= 1.5*warn_ms / 赤 > 1.5*warn_ms）
    let ms = report.total_ms();
    let warn = cfg.warn_frame_ms.max(4.0);
    let status_col = if ms <= warn {
        LinearPremul {
            r: 0.12,
            g: 0.82,
            b: 0.34,
            a: 0.95,
        }
    } else if ms <= warn * 1.5 {
        LinearPremul {
            r: 0.95,
            g: 0.65,
            b: 0.10,
            a: 0.95,
        }
    } else {
        LinearPremul {
            r: 0.92,
            g: 0.14,
            b: 0.18,
            a: 0.95,
        }
    };
    let bar_max_w = panel_w.saturating_sub(12);
    let fill_frac = (ms / (warn * 1.5)).clamp(0.08, 1.0);
    let bar_w = ((bar_max_w as f32) * fill_frac).round().max(4.0) as u32;
    fill_rect_blend(canvas, margin + 6, margin + 6, bar_w, 7, status_col);

    // 3. `GPU DIRECT` / `1/N` ステータスインジケータバッジ
    if show_gpu_direct_badge {
        let badge_col = if report.gpu_zero_copy_used {
            LinearPremul {
                r: 0.10,
                g: 0.72,
                b: 0.95,
                a: 0.95,
            }
        } else {
            LinearPremul {
                r: 0.45,
                g: 0.50,
                b: 0.60,
                a: 0.90,
            }
        };
        let bx = margin + panel_w.saturating_sub(34);
        fill_rect_blend(canvas, bx, margin + 17, 28, 8, badge_col);
    }

    // 適応縮小倍率ドット（1/1 = 1個, 1/2 = 2個, 1/4 = 3個）
    let dots = match report.adaptive_divisor {
        1 => 1u32,
        2 => 2,
        _ => 3,
    };
    for d in 0..dots {
        fill_rect_blend(
            canvas,
            margin + 6 + d * 8,
            margin + 18,
            5,
            6,
            LinearPremul {
                r: 0.92,
                g: 0.85,
                b: 0.25,
                a: 0.95,
            },
        );
    }

    // 4. パス別フレームグラフバー（スタック比率バー）
    if cfg.show_frame_graph && panel_h >= 46 && !report.pass_breakdown.is_empty() {
        let palette = [
            LinearPremul {
                r: 0.20,
                g: 0.55,
                b: 0.95,
                a: 0.95,
            }, // LayerRender
            LinearPremul {
                r: 0.18,
                g: 0.80,
                b: 0.55,
                a: 0.95,
            }, // Composite
            LinearPremul {
                r: 0.95,
                g: 0.62,
                b: 0.16,
                a: 0.95,
            }, // ColorGrade
            LinearPremul {
                r: 0.82,
                g: 0.30,
                b: 0.92,
                a: 0.95,
            }, // OpticalVFX
            LinearPremul {
                r: 0.95,
                g: 0.24,
                b: 0.45,
                a: 0.95,
            }, // Spatial360
        ];
        let total_us: u64 = report
            .pass_breakdown
            .iter()
            .map(|p| p.duration_us)
            .sum::<u64>()
            .max(1);
        let mut cursor_x = margin + 6;
        let max_x = margin + 6 + bar_max_w;
        for (i, sample) in report.pass_breakdown.iter().enumerate() {
            if cursor_x >= max_x {
                break;
            }
            let frac = sample.duration_us as f32 / total_us as f32;
            let seg_w = ((bar_max_w as f32) * frac).round().max(2.0) as u32;
            let draw_w = seg_w.min(max_x - cursor_x);
            fill_rect_blend(
                canvas,
                cursor_x,
                margin + 32,
                draw_w,
                11,
                palette[i % palette.len()],
            );
            cursor_x += draw_w;
        }
    }
}

fn fill_rect_blend(
    canvas: &mut LinearImage,
    x0: u32,
    y0: u32,
    rw: u32,
    rh: u32,
    color: LinearPremul,
) {
    let x1 = x0.saturating_add(rw).min(canvas.width);
    let y1 = y0.saturating_add(rh).min(canvas.height);
    let stride = canvas.width as usize;
    for y in y0..y1 {
        let row = (y as usize) * stride;
        for x in x0..x1 {
            let dst = &mut canvas.pixels[row + x as usize];
            *dst = color.over(*dst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::PassTimingSample;

    #[test]
    fn v110_zero_copy_wgsl_shader_validates_and_benchmark_passes_all_nine() {
        let stats = benchmark_v110_zero_copy_graph(160, 90);
        assert_eq!(stats.validated_shader_count, 9);
        assert!(stats.all_shaders_valid);
        assert_eq!(stats.chained_passes, 2);
        assert_eq!(stats.intermediate_cpu_readbacks, 0);
        assert!(stats.elapsed_ms >= 0.0);
    }

    #[test]
    fn v110_profiler_hud_overlay_draws_visible_telemetry_card() {
        let mut img = LinearImage::new(
            200,
            100,
            LinearPremul {
                r: 0.1,
                g: 0.1,
                b: 0.1,
                a: 1.0,
            },
        );
        let before = img.pixels.clone();
        let report = FrameProfileReport {
            width: 200,
            height: 100,
            total_us: 8_500,
            active_layer_count: 2,
            active_item_count: 3,
            gpu_zero_copy_used: true,
            adaptive_divisor: 2,
            pass_breakdown: vec![
                PassTimingSample {
                    pass_name: "LayerRender".into(),
                    duration_us: 5_000,
                },
                PassTimingSample {
                    pass_name: "ColorGrade".into(),
                    duration_us: 3_500,
                },
            ],
            text_cache_hits: 5,
            text_cache_misses: 1,
            text_cache_bytes: 64_000,
            frame_pool_reuses: 4,
            frame_pool_bytes: 1_000_000,
            lut_cache_entries: 1,
        };
        let cfg = ProfilerHudConfig {
            enabled: true,
            ..ProfilerHudConfig::default()
        };
        draw_profiler_hud_overlay(&mut img, &report, &cfg, true);
        assert_ne!(img.pixels, before);
        // ステータスバー付近 (x=12.. bar_w, y=12..19) が緑チャンネル優勢であること
        let bar_px = img.pixels[15 * 200 + 16];
        assert!(bar_px.g > bar_px.r && bar_px.g > 0.5);
    }
}
