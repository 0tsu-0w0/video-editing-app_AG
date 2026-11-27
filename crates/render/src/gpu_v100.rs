//! v10.0.0 GPU / WGSL コンピュート高速化パス（360° 全天球逆射影 ＆ 2D Navier-Stokes 半ラグランジュ移流）
//! ＋ 全組み込み WGSL シェーダー一括 Naga 検証スイート
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで `wgpu` コンピュートシェーダーを実行し、
//! GPU 非搭載／ヘッドレス環境では決定論 CPU パス（`v100_render`）へシームレスにフォールバックする。

use crate::color::{LinearImage, LinearPremul};
use crate::gpu_v30::{COLOR_GRADE_COMPUTE_WGSL, VIDEO_SCOPES_COMPUTE_WGSL};
use crate::gpu_v70::{FAST_ACES_COLOR_MGMT_WGSL, FAST_BLEND_MASK_WGSL};
use crate::graph::EFFECTS_WGSL_SOURCE;
use crate::hot_shader::validate_wgsl;
use crate::pipeline::COMPOSITE_COMPUTE_WGSL_SOURCE;
use crate::v100_render::render_spatial_360_and_stereo_pass;
use half::f16;
use std::future::Future;
use std::pin::pin;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
use std::time::Instant;
use ve_core::{
    Projection360Mode, RotationYprMatrix, Spatial360Preset, SpatialVideoConfig, StereoLayout,
    evaluate_horizon_locked_euler,
};

/// v10.0 #57: 360° 全天球逆射影（OverCapture / Tiny Planet / Equirectangular）WGSL Compute Shader
/// （IEEE-754 半精度 8B/px ストレージバッファ対応・4K/8K 処理向け）
pub const SPATIAL_360_EQUIRECT_WGSL: &str = r#"
struct Spatial360Uniforms {
    dims: vec4<u32>,      // x=width, y=height, z=mode(0=off,1=overcapture,2=tinyplanet,3=review), w=reserved
    cam: vec4<f32>,       // x=aspect, y=tan_half_x, z=tan_half_y, w=inv_tiny_zoom
    angles: vec4<f32>,    // x=yaw_deg, y=pitch_deg, z=0, w=0
    row0: vec4<f32>,      // RotationYprMatrix row 0
    row1: vec4<f32>,      // RotationYprMatrix row 1
    row2: vec4<f32>,      // RotationYprMatrix row 2
};

@group(0) @binding(0) var<storage, read> src_f16: array<vec2<u32>>;
@group(0) @binding(1) var<storage, read_write> out_f16: array<vec2<u32>>;
@group(0) @binding(2) var<uniform> u: Spatial360Uniforms;

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
        f16_to_f32(v.x >> 16u),
        f16_to_f32(v.y & 0xFFFFu),
        f16_to_f32(v.y >> 16u)
    );
}

fn pack_rgba16(c: vec4<f32>) -> vec2<u32> {
    let clamped = clamp(c, vec4<f32>(0.0), vec4<f32>(65504.0));
    let r = f32_to_f16(clamped.x);
    let g = f32_to_f16(clamped.y);
    let b = f32_to_f16(clamped.z);
    let a = f32_to_f16(clamped.w);
    return vec2<u32>(r | (g << 16u), b | (a << 16u));
}

fn sample_bilinear_wrap(u_coord: f32, v_coord: f32, w: u32, h: u32) -> vec4<f32> {
    let uf = u_coord - floor(u_coord);
    let vf = clamp(v_coord, 0.0, 1.0);
    let fx = uf * f32(max(w, 1u) - 1u);
    let fy = vf * f32(max(h, 1u) - 1u);

    let x0 = u32(floor(fx)) % w;
    let x1 = (x0 + 1u) % w;
    let y0 = min(u32(floor(fy)), h - 1u);
    let y1 = min(y0 + 1u, h - 1u);
    let tx = fract(fx);
    let ty = fract(fy);

    let p00 = unpack_rgba16(src_f16[y0 * w + x0]);
    let p10 = unpack_rgba16(src_f16[y0 * w + x1]);
    let p01 = unpack_rgba16(src_f16[y1 * w + x0]);
    let p11 = unpack_rgba16(src_f16[y1 * w + x1]);

    let top = mix(p00, p10, tx);
    let bot = mix(p01, p11, tx);
    return mix(top, bot, ty);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    let mode = u.dims.z;
    if (mode == 0u) {
        out_f16[idx] = src_f16[idx];
        return;
    }

    let nx = ((f32(gid.x) + 0.5) / f32(w)) * 2.0 - 1.0;
    let ny = 1.0 - ((f32(gid.y) + 0.5) / f32(h)) * 2.0;

    var eu: f32 = 0.5;
    var ev: f32 = 0.5;

    if (mode == 3u) {
        let raw_u = (nx * 0.5 + 0.5) + u.angles.x / 360.0;
        eu = raw_u - floor(raw_u);
        ev = clamp(0.5 - ny * 0.5 - u.angles.y / 180.0, 0.0, 1.0);
    } else {
        var ray_cam = vec3<f32>(0.0, 0.0, 1.0);
        if (mode == 1u) {
            ray_cam = vec3<f32>(nx * u.cam.y, ny * u.cam.z, 1.0);
        } else {
            let px = nx * u.cam.x * u.cam.w;
            let py = ny * u.cam.w;
            let r2 = px * px + py * py;
            let inv_d = 1.0 / (1.0 + r2);
            ray_cam = vec3<f32>((2.0 * px) * inv_d, (2.0 * py) * inv_d, (1.0 - r2) * inv_d);
        }
        let rw = vec3<f32>(
            dot(u.row0.xyz, ray_cam),
            dot(u.row1.xyz, ray_cam),
            dot(u.row2.xyz, ray_cam)
        );
        let dir = normalize(rw);
        let lon = atan2(dir.x, dir.z);
        let lat = asin(clamp(dir.y, -1.0, 1.0));
        let raw_u = lon / 6.28318530718 + 0.5;
        eu = raw_u - floor(raw_u);
        ev = clamp(0.5 - lat / 3.14159265359, 0.0, 1.0);
    }

    let c = sample_bilinear_wrap(eu, ev, w, h);
    out_f16[idx] = pack_rgba16(c);
}
"#;

/// v10.0 #58: Jos Stam 2D Navier-Stokes 半ラグランジュ移流＋減衰 WGSL Compute Shader
pub const FLUID_2D_ADVECTION_WGSL: &str = r#"
struct FluidAdvectUniforms {
    dims: vec4<u32>,    // x=grid_w, y=grid_h, z=0, w=0
    params: vec4<f32>,  // x=dt, y=dissipation, z=0, w=0
};

@group(0) @binding(0) var<storage, read> vel_u: array<f32>;
@group(0) @binding(1) var<storage, read> vel_v: array<f32>;
@group(0) @binding(2) var<storage, read> src_field: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst_field: array<f32>;
@group(0) @binding(4) var<uniform> u: FluidAdvectUniforms;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    if (gid.x == 0u || gid.y == 0u || gid.x + 1u >= w || gid.y + 1u >= h) {
        dst_field[idx] = src_field[idx] * u.params.y;
        return;
    }

    let dtx = u.params.x * f32(w - 2u);
    let dty = u.params.x * f32(h - 2u);
    let max_x = f32(w - 2u) + 0.5;
    let max_y = f32(h - 2u) + 0.5;

    let x = clamp(f32(gid.x) - dtx * vel_u[idx], 0.5, max_x);
    let y = clamp(f32(gid.y) - dty * vel_v[idx], 0.5, max_y);

    let i0 = u32(floor(x));
    let i1 = min(i0 + 1u, w - 1u);
    let j0 = u32(floor(y));
    let j1 = min(j0 + 1u, h - 1u);

    let s1 = x - f32(i0);
    let s0 = 1.0 - s1;
    let t1 = y - f32(j0);
    let t0 = 1.0 - t1;

    let v00 = src_field[j0 * w + i0];
    let v10 = src_field[j0 * w + i1];
    let v01 = src_field[j1 * w + i0];
    let v11 = src_field[j1 * w + i1];

    let val = s0 * (t0 * v00 + t1 * v01) + s1 * (t0 * v10 + t1 * v11);
    dst_field[idx] = val * u.params.y;
}
"#;

/// 全世代（v0.2〜v10.0）組み込み WGSL シェーダーの Naga 検証結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgslValidationSummary {
    pub name: &'static str,
    pub milestone: &'static str,
    pub valid: bool,
    pub error: Option<String>,
}

/// ワークスペース内の全 9 本の組み込み WGSL シェーダー（v0.2〜v11.0）を `naga` で一括静的検証する
pub fn validate_all_builtin_wgsl_shaders() -> Vec<WgslValidationSummary> {
    let shaders: &[(&'static str, &'static str, &'static str)] = &[
        ("effects.wgsl", "v0.2", EFFECTS_WGSL_SOURCE),
        (
            "composite_compute.wgsl",
            "v1.0",
            COMPOSITE_COMPUTE_WGSL_SOURCE,
        ),
        ("color_grade_compute.wgsl", "v3.0", COLOR_GRADE_COMPUTE_WGSL),
        (
            "video_scopes_compute.wgsl",
            "v3.0",
            VIDEO_SCOPES_COMPUTE_WGSL,
        ),
        ("fast_blend_mask.wgsl", "v7.0", FAST_BLEND_MASK_WGSL),
        (
            "fast_aces_color_mgmt.wgsl",
            "v7.0",
            FAST_ACES_COLOR_MGMT_WGSL,
        ),
        (
            "spatial_360_equirect.wgsl",
            "v10.0",
            SPATIAL_360_EQUIRECT_WGSL,
        ),
        ("fluid_2d_advection.wgsl", "v10.0", FLUID_2D_ADVECTION_WGSL),
        (
            "zero_copy_composite_grade.wgsl",
            "v11.0",
            crate::gpu_v110::ZERO_COPY_COMPOSITE_GRADE_WGSL,
        ),
    ];

    shaders
        .iter()
        .map(|&(name, milestone, src)| match validate_wgsl(src) {
            Ok(_) => WgslValidationSummary {
                name,
                milestone,
                valid: true,
                error: None,
            },
            Err(e) => WgslValidationSummary {
                name,
                milestone,
                valid: false,
                error: Some(e),
            },
        })
        .collect()
}

/// v10.0 GPU / CPU 360° ＆ 流体 WGSL パイプラインのベンチマーク結果
#[derive(Debug, Clone, PartialEq)]
pub struct GpuV100BenchmarkReport {
    pub width: u32,
    pub height: u32,
    pub used_gpu: bool,
    pub adapter_name: String,
    pub backend_name: String,
    pub spatial_360_ms: f64,
    pub validated_shader_count: usize,
    pub all_shaders_valid: bool,
}

fn block_on_v100<F: Future>(fut: F) -> F::Output {
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

fn pack_linear_rgba16f(src: &[LinearPremul]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 8);
    for p in src {
        out.extend_from_slice(&f16::from_f32(p.r).to_le_bytes());
        out.extend_from_slice(&f16::from_f32(p.g).to_le_bytes());
        out.extend_from_slice(&f16::from_f32(p.b).to_le_bytes());
        out.extend_from_slice(&f16::from_f32(p.a).to_le_bytes());
    }
    out
}

fn unpack_linear_rgba16f(bytes: &[u8], w: u32, h: u32) -> LinearImage {
    let mut img = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    for (slot, chunk) in img.pixels.iter_mut().zip(bytes.as_chunks::<8>().0) {
        *slot = LinearPremul {
            r: f16::from_le_bytes([chunk[0], chunk[1]]).to_f32(),
            g: f16::from_le_bytes([chunk[2], chunk[3]]).to_f32(),
            b: f16::from_le_bytes([chunk[4], chunk[5]]).to_f32(),
            a: f16::from_le_bytes([chunk[6], chunk[7]]).to_f32(),
        };
    }
    img
}

/// v10.0 360° 全天球 ＆ 2D 流体 WGSL Compute Pipeline（ヘッドレス実 GPU 実行器）
pub struct GpuV100ComputePipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    spatial_pipeline: wgpu::ComputePipeline,
    spatial_bgl: wgpu::BindGroupLayout,
    fluid_pipeline: wgpu::ComputePipeline,
    fluid_bgl: wgpu::BindGroupLayout,
    pub adapter_name: String,
    pub backend_name: String,
}

impl GpuV100ComputePipeline {
    pub fn try_headless() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = block_on_v100(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let info = adapter.get_info();
        let (device, queue) = block_on_v100(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ve_v100_compute_device"),
            ..Default::default()
        }))
        .ok()?;

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

        let spatial_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_v100_spatial_360_cs"),
            source: wgpu::ShaderSource::Wgsl(SPATIAL_360_EQUIRECT_WGSL.into()),
        });
        let spatial_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_v100_spatial_bgl"),
            entries: &[ro_storage(0), rw_storage(1), uniform_entry(2)],
        });
        let spatial_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_v100_spatial_pl"),
            bind_group_layouts: &[Some(&spatial_bgl)],
            immediate_size: 0,
        });
        let spatial_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ve_v100_spatial_pipeline"),
            layout: Some(&spatial_pl),
            module: &spatial_sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let fluid_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_v100_fluid_advect_cs"),
            source: wgpu::ShaderSource::Wgsl(FLUID_2D_ADVECTION_WGSL.into()),
        });
        let fluid_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_v100_fluid_bgl"),
            entries: &[
                ro_storage(0),
                ro_storage(1),
                ro_storage(2),
                rw_storage(3),
                uniform_entry(4),
            ],
        });
        let fluid_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_v100_fluid_pl"),
            bind_group_layouts: &[Some(&fluid_bgl)],
            immediate_size: 0,
        });
        let fluid_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ve_v100_fluid_pipeline"),
            layout: Some(&fluid_pl),
            module: &fluid_sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Some(Self {
            device,
            queue,
            spatial_pipeline,
            spatial_bgl,
            fluid_pipeline,
            fluid_bgl,
            adapter_name: info.name,
            backend_name: format!("{:?}", info.backend),
        })
    }

    /// GPU Compute Shader で 360° Equirectangular 逆射影を実行する
    pub fn dispatch_spatial_360(
        &self,
        src: &LinearImage,
        cfg: &SpatialVideoConfig,
        time_sec: f32,
    ) -> Option<LinearImage> {
        let w = src.width;
        let h = src.height;
        if w == 0 || h == 0 {
            return None;
        }
        let byte_len = (w as u64).checked_mul(h as u64)?.checked_mul(8)?;
        if byte_len > 120 * 1024 * 1024 {
            return None;
        }

        let (yaw, pitch, roll) = evaluate_horizon_locked_euler(cfg, time_sec);
        let rot = RotationYprMatrix::from_ypr_deg(yaw, pitch, roll);
        let aspect = w as f32 / h.max(1) as f32;
        let half_fov = (cfg.fov_deg.clamp(25.0, 155.0) * 0.5).to_radians();
        let tan_half_x = half_fov.tan();
        let tan_half_y = tan_half_x / aspect.max(0.1);
        let inv_tiny_zoom = 1.0 / cfg.tiny_planet_zoom.clamp(0.25, 4.0);
        let mode_code: u32 = match cfg.projection_mode {
            Projection360Mode::Off => 0,
            Projection360Mode::OverCapture => 1,
            Projection360Mode::TinyPlanet => 2,
            Projection360Mode::EquirectangularReview => 3,
        };

        let mut u_bytes = [0u8; 96];
        for (i, v) in [w, h, mode_code, 0u32].into_iter().enumerate() {
            u_bytes[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
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
            u_bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }

        let packed_in = pack_linear_rgba16f(&src.pixels);
        let buf_in = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v100_sp_in"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let buf_out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v100_sp_out"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buf_read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v100_sp_read"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let buf_u = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v100_sp_u"),
            size: u_bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buf_in, 0, &packed_in);
        self.queue.write_buffer(&buf_u, 0, &u_bytes);

        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ve_v100_sp_bg"),
            layout: &self.spatial_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf_in.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_out.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf_u.as_entire_binding(),
                },
            ],
        });

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ve_v100_sp_enc"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ve_v100_sp_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.spatial_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(&buf_out, 0, &buf_read, 0, byte_len);
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
        let out = unpack_linear_rgba16f(&mapped, w, h);
        drop(mapped);
        buf_read.unmap();
        Some(out)
    }

    /// GPU Compute Shader で 2D 流体半ラグランジュ移流の 1 ステップを実行する
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_fluid_advect_step(
        &self,
        w: u32,
        h: u32,
        u: &[f32],
        v: &[f32],
        src_field: &[f32],
        dt: f32,
        dissipation: f32,
    ) -> Option<Vec<f32>> {
        let count = (w as usize).checked_mul(h as usize)?;
        if count == 0 || u.len() < count || v.len() < count || src_field.len() < count {
            return None;
        }
        let byte_len = (count * 4) as u64;
        let to_bytes = |s: &[f32]| -> Vec<u8> {
            let mut b = Vec::with_capacity(count * 4);
            for &x in &s[..count] {
                b.extend_from_slice(&x.to_le_bytes());
            }
            b
        };
        let mut u_bytes = [0u8; 32];
        u_bytes[0..4].copy_from_slice(&w.to_le_bytes());
        u_bytes[4..8].copy_from_slice(&h.to_le_bytes());
        u_bytes[16..20].copy_from_slice(&dt.to_le_bytes());
        u_bytes[20..24].copy_from_slice(&dissipation.to_le_bytes());

        let mk_ro = |lbl: &str, data: &[u8]| {
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(lbl),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buf, 0, data);
            buf
        };
        let bu = mk_ro("ve_v100_fl_u", &to_bytes(u));
        let bv = mk_ro("ve_v100_fl_v", &to_bytes(v));
        let bsrc = mk_ro("ve_v100_fl_src", &to_bytes(src_field));
        let bdst = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v100_fl_dst"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bread = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v100_fl_read"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bunif = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_v100_fl_unif"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&bunif, 0, &u_bytes);

        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ve_v100_fl_bg"),
            layout: &self.fluid_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bu.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bv.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: bsrc.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: bdst.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: bunif.as_entire_binding(),
                },
            ],
        });

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ve_v100_fl_enc"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ve_v100_fl_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.fluid_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(&bdst, 0, &bread, 0, byte_len);
        self.queue.submit(Some(enc.finish()));

        let slice = bread.slice(0..byte_len);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let Ok(mapped) = slice.get_mapped_range() else {
            bread.unmap();
            return None;
        };
        let mut out = Vec::with_capacity(count);
        for c in mapped.as_chunks::<4>().0 {
            out.push(f32::from_le_bytes(*c));
        }
        drop(mapped);
        bread.unmap();
        Some(out)
    }
}

/// GPU が利用可能な場合は `SPATIAL_360_EQUIRECT_WGSL` で逆射影を高速実行し、非搭載時は決定論 CPU パスへフォールバックする
pub fn execute_v100_spatial_360_or_cpu(
    gpu: Option<&GpuV100ComputePipeline>,
    src: &LinearImage,
    cfg: &SpatialVideoConfig,
    time_sec: f32,
) -> (LinearImage, bool) {
    if !cfg.enabled || src.width == 0 || src.height == 0 {
        return (src.clone(), false);
    }
    if let Some(pipe) = gpu
        && cfg.projection_mode != Projection360Mode::Off
        && let Some(projected) = pipe.dispatch_spatial_360(src, cfg, time_sec)
    {
        if cfg.stereo_layout == StereoLayout::Off && !cfg.show_horizon_guides {
            return (projected, true);
        }
        let mut post_cfg = cfg.clone();
        post_cfg.projection_mode = Projection360Mode::Off;
        return (
            render_spatial_360_and_stereo_pass(&projected, &post_cfg, time_sec),
            true,
        );
    }
    (
        render_spatial_360_and_stereo_pass(src, cfg, time_sec),
        false,
    )
}

/// v10.0 WGSL シェーダー全検証＋360° パス実測ベンチマークを実行する
pub fn benchmark_v100_gpu_or_cpu(width: u32, height: u32) -> GpuV100BenchmarkReport {
    let w = width.clamp(64, 1920);
    let h = height.clamp(64, 1080);
    let checks = validate_all_builtin_wgsl_shaders();
    let all_valid = checks.iter().all(|c| c.valid);

    let mut src = LinearImage::new(w, h, LinearPremul::TRANSPARENT);
    for y in 0..h {
        let gy = y as f32 / h as f32;
        let row = (y * w) as usize;
        for x in 0..w {
            src.pixels[row + x as usize] = LinearPremul {
                r: x as f32 / w as f32,
                g: gy,
                b: 0.42,
                a: 1.0,
            };
        }
    }

    let cfg = SpatialVideoConfig::from_preset(Spatial360Preset::ActionHorizonLock);
    let gpu = GpuV100ComputePipeline::try_headless();
    let (adapter_name, backend_name) = match gpu.as_ref() {
        Some(g) => (g.adapter_name.clone(), g.backend_name.clone()),
        None => ("CPU Deterministic Fallback".into(), "CPU".into()),
    };

    let t0 = Instant::now();
    let (_out, used_gpu) = execute_v100_spatial_360_or_cpu(gpu.as_ref(), &src, &cfg, 0.75);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;

    GpuV100BenchmarkReport {
        width: w,
        height: h,
        used_gpu,
        adapter_name,
        backend_name,
        spatial_360_ms: ms,
        validated_shader_count: checks.len(),
        all_shaders_valid: all_valid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_builtin_wgsl_shaders_from_v02_to_v100_pass_naga_validation() {
        let checks = validate_all_builtin_wgsl_shaders();
        assert_eq!(checks.len(), 9);
        for c in &checks {
            assert!(
                c.valid,
                "WGSL shader '{}' ({}) failed naga validation: {:?}",
                c.name, c.milestone, c.error
            );
        }
    }

    #[test]
    fn v100_spatial_360_gpu_or_cpu_fallback_and_benchmark_work_cleanly() {
        let rep = benchmark_v100_gpu_or_cpu(160, 90);
        assert_eq!(rep.validated_shader_count, 9);
        assert!(rep.all_shaders_valid);
        assert!(rep.spatial_360_ms >= 0.0 && rep.spatial_360_ms.is_finite());
    }
}
