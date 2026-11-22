use crate::color::LinearImage;
use crate::hot_shader::ShaderHotReloader;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::pin;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
use wgpu;

/// 共通 GPU エフェクト Uniform（48 bytes: 先頭 16 bytes が既存 4-float シェーダーと完全互換）
///
/// WGSL 側の宣言例:
/// ```wgsl
/// struct GpuEffectUniform {
///     p0: f32, p1: f32, p2: f32, p3: f32,
///     p4: f32, p5: f32, p6: f32, p7: f32,
///     resolution: vec2<f32>,
///     time_sec: f32,
///     _pad: f32,
/// };
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuEffectUniform {
    pub params: [f32; 8],
    pub resolution: [f32; 2],
    pub time_sec: f32,
    pub _pad: f32,
}

impl GpuEffectUniform {
    pub const BYTE_SIZE: usize = 48;

    pub fn to_le_bytes(self) -> [u8; Self::BYTE_SIZE] {
        let mut out = [0u8; Self::BYTE_SIZE];
        let vals = [
            self.params[0],
            self.params[1],
            self.params[2],
            self.params[3],
            self.params[4],
            self.params[5],
            self.params[6],
            self.params[7],
            self.resolution[0],
            self.resolution[1],
            self.time_sec,
            self._pad,
        ];
        for (i, v) in vals.into_iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// `wgpu` Ping-Pong パイプライン上で実行する 1 ポストプロセス／エフェクトパス
#[derive(Debug, Clone, PartialEq)]
pub struct GpuPostProcessPass {
    /// `"builtin:color_grade"`, `"builtin:blur"`, `"builtin:glow"`, `"builtin:clip_rect"`,
    /// または `ShaderHotReloader` に登録されたカスタム WGSL ID（例: `"custom_chromatic_aberration"`）
    pub shader_id: String,
    pub params: [f32; 8],
}

impl GpuPostProcessPass {
    pub fn custom(shader_id: impl Into<String>, params: [f32; 8]) -> Self {
        Self {
            shader_id: shader_id.into(),
            params,
        }
    }

    pub fn from_builtin_effect(effect_type: &str, params: &BTreeMap<String, f32>) -> Option<Self> {
        let mut p = [0.0f32; 8];
        let shader_id = match effect_type {
            "color_grade" => {
                p[0] = params.get("exposure").copied().unwrap_or(0.0);
                p[1] = params.get("contrast").copied().unwrap_or(1.0);
                p[2] = params.get("saturation").copied().unwrap_or(1.0);
                "builtin:color_grade"
            }
            "blur" => {
                p[0] = params
                    .get("radius")
                    .copied()
                    .unwrap_or(2.0)
                    .clamp(0.0, 16.0);
                "builtin:blur"
            }
            "glow" => {
                p[0] = params
                    .get("radius")
                    .copied()
                    .unwrap_or(3.0)
                    .clamp(1.0, 16.0);
                p[1] = params.get("threshold").copied().unwrap_or(0.5);
                p[2] = params.get("intensity").copied().unwrap_or(1.0).max(0.0);
                "builtin:glow"
            }
            "clip_rect" => {
                p[0] = params.get("left").copied().unwrap_or(0.0).clamp(0.0, 1.0);
                p[1] = params.get("right").copied().unwrap_or(0.0).clamp(0.0, 1.0);
                p[2] = params.get("top").copied().unwrap_or(0.0).clamp(0.0, 1.0);
                p[3] = params.get("bottom").copied().unwrap_or(0.0).clamp(0.0, 1.0);
                "builtin:clip_rect"
            }
            _ => return None,
        };
        Some(Self {
            shader_id: shader_id.to_string(),
            params: p,
        })
    }
}

/// `wgpu` ベースの Linear sRGB (`Rgba16Float`) Ping-Pong GPU エフェクト＆プレビューパイプライン
pub struct WgpuPreviewPipeline {
    pub width: u32,
    pub height: u32,
    pub ping_texture: wgpu::Texture,
    pub ping_view: wgpu::TextureView,
    pub pong_texture: wgpu::Texture,
    pub pong_view: wgpu::TextureView,
    pub output_texture: wgpu::Texture,
    pub output_view: wgpu::TextureView,
    composite_pipeline: wgpu::RenderPipeline,
    composite_bg_ping: wgpu::BindGroup,
    composite_bg_pong: wgpu::BindGroup,
    effect_pl: wgpu::PipelineLayout,
    effect_bg_ping: wgpu::BindGroup,
    effect_bg_pong: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    effects_module: wgpu::ShaderModule,
    builtin_color_grade: wgpu::RenderPipeline,
    builtin_blur_9tap: wgpu::RenderPipeline,
    builtin_glow: wgpu::RenderPipeline,
    builtin_clip_rect: wgpu::RenderPipeline,
    /// カスタム WGSL ID -> `(revision, compiled RenderPipeline)`
    custom_pipelines: HashMap<String, (u64, wgpu::RenderPipeline)>,
    staging_f16: Vec<u8>,
}

impl WgpuPreviewPipeline {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let tex_desc = |label| wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let ping_texture = device.create_texture(&tex_desc("ve_linear_f16_ping"));
        let ping_view = ping_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let pong_texture = device.create_texture(&tex_desc("ve_linear_f16_pong"));
        let pong_view = pong_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ve_preview_srgb_out"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_composite_wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
        });
        let effects_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_effects_wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/effects.wgsl").into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ve_linear_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Composite bind group layout (binding 0: tex, binding 1: sampler)
        let composite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_composite_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let make_comp_bg = |label, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &composite_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let composite_bg_ping = make_comp_bg("ve_composite_bg_ping", &ping_view);
        let composite_bg_pong = make_comp_bg("ve_composite_bg_pong", &pong_view);

        let composite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_composite_pl"),
            bind_group_layouts: &[Some(&composite_bgl)],
            immediate_size: 0,
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ve_composite_pipeline"),
            layout: Some(&composite_pl),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_to_srgb"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Effect Ping-Pong bind group layout (binding 0: tex, binding 1: sampler, binding 2: uniform)
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_gpu_effect_uniform"),
            size: GpuEffectUniform::BYTE_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let effect_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_effect_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let make_fx_bg = |label, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &effect_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let effect_bg_ping = make_fx_bg("ve_effect_bg_ping", &ping_view);
        let effect_bg_pong = make_fx_bg("ve_effect_bg_pong", &pong_view);

        let effect_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_effect_pl"),
            bind_group_layouts: &[Some(&effect_bgl)],
            immediate_size: 0,
        });

        let builtin_color_grade = build_f16_effect_pipeline(
            device,
            &effect_pl,
            &effects_module,
            &effects_module,
            "fs_color_grade",
            "ve_fx_color_grade",
        );
        let builtin_blur_9tap = build_f16_effect_pipeline(
            device,
            &effect_pl,
            &effects_module,
            &effects_module,
            "fs_blur_9tap",
            "ve_fx_blur_9tap",
        );
        let builtin_glow = build_f16_effect_pipeline(
            device,
            &effect_pl,
            &effects_module,
            &effects_module,
            "fs_glow",
            "ve_fx_glow",
        );
        let builtin_clip_rect = build_f16_effect_pipeline(
            device,
            &effect_pl,
            &effects_module,
            &effects_module,
            "fs_clip_rect",
            "ve_fx_clip_rect",
        );

        Self {
            width,
            height,
            ping_texture,
            ping_view,
            pong_texture,
            pong_view,
            output_texture,
            output_view,
            composite_pipeline,
            composite_bg_ping,
            composite_bg_pong,
            effect_pl,
            effect_bg_ping,
            effect_bg_pong,
            uniform_buffer,
            effects_module,
            builtin_color_grade,
            builtin_blur_9tap,
            builtin_glow,
            builtin_clip_rect,
            custom_pipelines: HashMap::new(),
            staging_f16: Vec::with_capacity((width as usize) * (height as usize) * 8),
        }
    }

    pub fn compiled_custom_shader_count(&self) -> usize {
        self.custom_pipelines.len()
    }

    /// `ShaderHotReloader` の検証済み最新 WGSL を監視し、リビジョン更新があったカスタムシェーダーの
    /// `wgpu::RenderPipeline` を自動再構築する（構文エラー時は直前の正常パイプラインを保持）。
    pub fn sync_hot_shaders(
        &mut self,
        device: &wgpu::Device,
        reloader: &ShaderHotReloader,
    ) -> usize {
        let mut rebuilt = 0usize;
        for entry in reloader.list() {
            let Some(frag_ep) = entry.fragment_entry_point.as_deref() else {
                continue;
            };
            let already_current = self
                .custom_pipelines
                .get(&entry.id)
                .is_some_and(|(rev, _)| *rev == entry.revision);
            if already_current {
                continue;
            }
            if let Some(frag_mod) = reloader.build_wgpu_module(device, &entry.id) {
                let pipe = build_f16_effect_pipeline(
                    device,
                    &self.effect_pl,
                    &self.effects_module,
                    &frag_mod,
                    frag_ep,
                    &format!("ve_custom_{}", entry.id),
                );
                self.custom_pipelines
                    .insert(entry.id.clone(), (entry.revision, pipe));
                rebuilt += 1;
            }
        }
        rebuilt
    }

    /// Linear f16 バッファを GPU へアップロードし、ポストプロセスなしで sRGB プレビューへ描画する
    pub fn upload_and_present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &LinearImage,
    ) {
        self.upload_process_and_present(device, queue, image, &[], 0.0);
    }

    /// Linear f16 バッファを GPU へアップロードし、Ping-Pong テクスチャ上で組み込み／カスタム WGSL パスを
    /// 連続実行したのち sRGB プレビューテクスチャへ出力する。
    pub fn upload_process_and_present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &LinearImage,
        passes: &[GpuPostProcessPass],
        time_sec: f32,
    ) {
        let Some(final_is_ping) =
            self.upload_and_run_ping_pong(device, queue, image, passes, time_sec)
        else {
            return;
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ve_preview_present_encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ve_preview_present_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(
                0,
                if final_is_ping {
                    &self.composite_bg_ping
                } else {
                    &self.composite_bg_pong
                },
                &[],
            );
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }

    /// Ping-Pong パスを実行し、最終結果が `ping_texture` にあるなら `Some(true)`、`pong_texture` なら `Some(false)` を返す
    fn upload_and_run_ping_pong(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &LinearImage,
        passes: &[GpuPostProcessPass],
        time_sec: f32,
    ) -> Option<bool> {
        if image.width != self.width || image.height != self.height {
            return None;
        }
        image.write_rgba16f_bytes(&mut self.staging_f16);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ping_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.staging_f16,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 8),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        let mut cur_is_ping = true;
        for pass_cfg in passes {
            if pass_cfg.shader_id == "builtin:blur" {
                let radius = pass_cfg.params[0].clamp(0.0, 16.0);
                if radius > 0.05 {
                    let h_u = GpuEffectUniform {
                        params: [radius, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        resolution: [self.width as f32, self.height as f32],
                        time_sec,
                        _pad: 0.0,
                    };
                    self.run_single_fx_pass(device, queue, "builtin:blur", h_u, cur_is_ping);
                    cur_is_ping = !cur_is_ping;

                    let v_u = GpuEffectUniform {
                        params: [0.0, radius, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        resolution: [self.width as f32, self.height as f32],
                        time_sec,
                        _pad: 0.0,
                    };
                    self.run_single_fx_pass(device, queue, "builtin:blur", v_u, cur_is_ping);
                    cur_is_ping = !cur_is_ping;
                }
            } else {
                let uniform = GpuEffectUniform {
                    params: pass_cfg.params,
                    resolution: [self.width as f32, self.height as f32],
                    time_sec,
                    _pad: 0.0,
                };
                if self.run_single_fx_pass(device, queue, &pass_cfg.shader_id, uniform, cur_is_ping)
                {
                    cur_is_ping = !cur_is_ping;
                }
            }
        }

        Some(cur_is_ping)
    }

    fn run_single_fx_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shader_id: &str,
        uniform: GpuEffectUniform,
        src_is_ping: bool,
    ) -> bool {
        let pipeline = match shader_id {
            "builtin:color_grade" => &self.builtin_color_grade,
            "builtin:blur" => &self.builtin_blur_9tap,
            "builtin:glow" => &self.builtin_glow,
            "builtin:clip_rect" => &self.builtin_clip_rect,
            custom_id => match self.custom_pipelines.get(custom_id) {
                Some((_, p)) => p,
                None => return false,
            },
        };

        queue.write_buffer(&self.uniform_buffer, 0, &uniform.to_le_bytes());
        let (src_bg, dst_view) = if src_is_ping {
            (&self.effect_bg_ping, &self.pong_view)
        } else {
            (&self.effect_bg_pong, &self.ping_view)
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ve_gpu_fx_encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ve_gpu_fx_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, src_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
        true
    }
}

/// オフスクリーン書き出し用ヘッドレス GPU オフローダ（GPU 非搭載環境では `try_new` が `None` を返し CPU フォールバック）
pub struct HeadlessGpuOffloader {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: WgpuPreviewPipeline,
    readback_buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

impl HeadlessGpuOffloader {
    pub fn try_new(width: u32, height: u32) -> Option<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let instance = wgpu::Instance::default();
        let adapter = block_on_simple(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = block_on_simple(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ve_headless_gpu_device"),
            ..Default::default()
        }))
        .ok()?;

        let pipeline = WgpuPreviewPipeline::new(&device, width, height);
        let unpadded = width * 8;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded.div_ceil(align) * align;
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_headless_readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Some(Self {
            device,
            queue,
            pipeline,
            readback_buffer,
            padded_bytes_per_row,
        })
    }

    pub fn apply_passes_readback(
        &mut self,
        image: &LinearImage,
        reloader: &ShaderHotReloader,
        passes: &[GpuPostProcessPass],
        time_sec: f32,
    ) -> Option<LinearImage> {
        if passes.is_empty()
            || image.width != self.pipeline.width
            || image.height != self.pipeline.height
        {
            return None;
        }
        self.pipeline.sync_hot_shaders(&self.device, reloader);
        let final_is_ping = self.pipeline.upload_and_run_ping_pong(
            &self.device,
            &self.queue,
            image,
            passes,
            time_sec,
        )?;
        let src_tex = if final_is_ping {
            &self.pipeline.ping_texture
        } else {
            &self.pipeline.pong_texture
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ve_headless_readback_encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: src_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.pipeline.height),
                },
            },
            wgpu::Extent3d {
                width: self.pipeline.width,
                height: self.pipeline.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;

        let mapped = slice.get_mapped_range().ok()?;
        let unpadded = (self.pipeline.width * 8) as usize;
        let padded = self.padded_bytes_per_row as usize;
        let mut tight = Vec::with_capacity(unpadded * self.pipeline.height as usize);
        for row in 0..self.pipeline.height as usize {
            tight.extend_from_slice(&mapped[row * padded..row * padded + unpadded]);
        }
        drop(mapped);
        self.readback_buffer.unmap();
        LinearImage::from_rgba16f_bytes(self.pipeline.width, self.pipeline.height, &tight)
    }
}

fn build_f16_effect_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    vs_module: &wgpu::ShaderModule,
    fs_module: &wgpu::ShaderModule,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: vs_module,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: fs_module,
            entry_point: Some(fs_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn block_on_simple<F: Future>(fut: F) -> F::Output {
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

/// v1.0 #18: Linear Premultiplied レイヤー合成用 WGSL コンピュートシェーダーソース
pub const COMPOSITE_COMPUTE_WGSL_SOURCE: &str = include_str!("shaders/composite_compute.wgsl");

/// v1.0 #18: `wgpu` コンピュートシェーダー（`@workgroup_size(8, 8)`）で
/// Linear Premultiplied Alpha 合成（Normal / Add / Multiply / Screen）を並列実行するコンポジタ。
pub struct WgpuComputeCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
}

impl WgpuComputeCompositor {
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ve_composite_compute_wgsl"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_COMPUTE_WGSL_SOURCE.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ve_compute_comp_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ve_compute_comp_pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ve_compute_comp_pipeline"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("cs_composite"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            device,
            queue,
            pipeline,
            bgl,
        }
    }

    /// ヘッドレス GPU アダプタで `WgpuComputeCompositor` を初期化する（GPU 非搭載環境では `None`）
    pub fn try_headless() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = block_on_simple(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = block_on_simple(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ve_compute_compositor_device"),
            ..Default::default()
        }))
        .ok()?;
        Some(Self::from_device(device, queue))
    }

    /// 同解像度の 2 枚の `LinearImage` を GPU コンピュートシェーダーで合成する
    pub fn composite_linear_images(
        &self,
        dst: &LinearImage,
        src: &LinearImage,
        blend_mode: ve_core::BlendMode,
        opacity: f32,
    ) -> Option<LinearImage> {
        if dst.width != src.width || dst.height != src.height || dst.width == 0 || dst.height == 0 {
            return None;
        }
        let pixel_count = (dst.width as usize) * (dst.height as usize);
        let byte_len = (pixel_count * 16) as u64;

        let pack_pixels = |img: &LinearImage| -> Vec<u8> {
            let mut bytes = Vec::with_capacity(pixel_count * 16);
            for p in &img.pixels {
                bytes.extend_from_slice(&p.r.to_le_bytes());
                bytes.extend_from_slice(&p.g.to_le_bytes());
                bytes.extend_from_slice(&p.b.to_le_bytes());
                bytes.extend_from_slice(&p.a.to_le_bytes());
            }
            bytes
        };

        let dst_bytes = pack_pixels(dst);
        let src_bytes = pack_pixels(src);

        let dst_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_comp_dst"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let src_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_comp_src"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_comp_out"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_comp_read"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mode_u32: u32 = match blend_mode {
            ve_core::BlendMode::Normal => 0,
            ve_core::BlendMode::Add => 1,
            ve_core::BlendMode::Multiply => 2,
            ve_core::BlendMode::Screen => 3,
        };
        let mut uniform_bytes = [0u8; 16];
        uniform_bytes[0..4].copy_from_slice(&dst.width.to_le_bytes());
        uniform_bytes[4..8].copy_from_slice(&dst.height.to_le_bytes());
        uniform_bytes[8..12].copy_from_slice(&mode_u32.to_le_bytes());
        uniform_bytes[12..16].copy_from_slice(&opacity.clamp(0.0, 1.0).to_le_bytes());

        let uniform_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ve_comp_uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.queue.write_buffer(&dst_buf, 0, &dst_bytes);
        self.queue.write_buffer(&src_buf, 0, &src_bytes);
        self.queue.write_buffer(&uniform_buf, 0, &uniform_bytes);

        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ve_comp_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: dst_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: src_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ve_compute_comp_encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ve_compute_comp_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(dst.width.div_ceil(8), dst.height.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, byte_len);
        self.queue.submit(Some(encoder.finish()));

        let slice = read_buf.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;

        let mapped = slice.get_mapped_range().ok()?;
        let mut pixels = Vec::with_capacity(pixel_count);
        for chunk in mapped.as_chunks::<16>().0 {
            pixels.push(crate::color::LinearPremul {
                r: f32::from_le_bytes(chunk[0..4].try_into().ok()?),
                g: f32::from_le_bytes(chunk[4..8].try_into().ok()?),
                b: f32::from_le_bytes(chunk[8..12].try_into().ok()?),
                a: f32::from_le_bytes(chunk[12..16].try_into().ok()?),
            });
        }
        drop(mapped);
        read_buf.unmap();
        Some(LinearImage {
            width: dst.width,
            height: dst.height,
            pixels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_uniform_serializes_48_le_bytes_with_p0_p3_first() {
        let u = GpuEffectUniform {
            params: [1.5, 2.0, -0.5, 4.0, 5.0, 6.0, 7.0, 8.0],
            resolution: [1920.0, 1080.0],
            time_sec: 12.25,
            _pad: 0.0,
        };
        let bytes = u.to_le_bytes();
        assert_eq!(bytes.len(), 48);
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 1.5);
        assert_eq!(
            f32::from_le_bytes(bytes[32..36].try_into().unwrap()),
            1920.0
        );
        assert_eq!(f32::from_le_bytes(bytes[40..44].try_into().unwrap()), 12.25);
    }

    #[test]
    fn headless_gpu_offloader_gracefully_handles_gpu_or_cpu_fallback() {
        // GPU が利用可能な環境では実際に Rgba16Float Ping-Pong + リードバックを検証し、
        // GPU 非搭載ヘッドレスコンテナではパニックせず None (CPUフォールバック) になることを確認
        if let Some(mut gpu) = HeadlessGpuOffloader::try_new(16, 16) {
            let reloader = ShaderHotReloader::new();
            let img = LinearImage::new(
                16,
                16,
                crate::color::LinearPremul {
                    r: 0.25,
                    g: 0.25,
                    b: 0.25,
                    a: 1.0,
                },
            );
            let mut params = BTreeMap::new();
            params.insert("exposure".into(), 1.0); // 2^1 = 2x brighter -> ~0.5
            let pass = GpuPostProcessPass::from_builtin_effect("color_grade", &params).unwrap();
            let out = gpu
                .apply_passes_readback(&img, &reloader, &[pass], 0.0)
                .expect("gpu readback");
            assert!(
                (out.pixels[0].r - 0.5).abs() < 0.05,
                "got {}",
                out.pixels[0].r
            );
        }
    }

    #[test]
    fn v10_compute_compositor_shader_validates_and_runs_or_falls_back() {
        // WGSL コンピュートシェーダー自体が naga により全環境で構文・型検証をパスすることを確認
        crate::hot_shader::validate_wgsl(COMPOSITE_COMPUTE_WGSL_SOURCE)
            .expect("composite_compute.wgsl must pass naga validation");

        if let Some(comp) = WgpuComputeCompositor::try_headless() {
            let dst = LinearImage::new(
                8,
                8,
                crate::color::LinearPremul {
                    r: 0.2,
                    g: 0.2,
                    b: 0.2,
                    a: 1.0,
                },
            );
            let src = LinearImage::new(
                8,
                8,
                crate::color::LinearPremul {
                    r: 0.4,
                    g: 0.0,
                    b: 0.0,
                    a: 0.5,
                },
            );
            let out = comp
                .composite_linear_images(&dst, &src, ve_core::BlendMode::Add, 1.0)
                .expect("compute composite");
            assert!((out.pixels[0].r - 0.6).abs() < 1e-3);
        }
    }
}
