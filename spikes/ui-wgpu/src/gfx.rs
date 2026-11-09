//! Minimal wgpu renderer for the spike: one instanced-quad pipeline + one RGBA
//! glyph atlas. Mask glyphs are stored as white + alpha and tinted in the
//! shader; colour glyphs (emoji) are stored as RGBA and drawn untinted.

use std::collections::HashMap;
use std::sync::Arc;

use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};
use swash::FontRef;
use wgpu::util::DeviceExt;

pub const ATLAS: u32 = 2048;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Inst {
    pub rect: [f32; 4], // x, y, w, h in physical pixels
    pub uv: [f32; 4],   // u0, v0, u1, v1
    pub color: [f32; 4],
    pub flags: u32,
    pub _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

#[derive(Clone, Copy)]
pub struct GlyphEntry {
    pub uv: [f32; 4],
    pub left: i32,
    pub top: i32,
    pub w: u32,
    pub h: u32,
    pub color: bool,
}

pub struct Gfx {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    atlas: wgpu::Texture,
    inst_buf: wgpu::Buffer,
    inst_cap: usize,
    // glyph cache
    cache: HashMap<(u64, u16, u32), Option<GlyphEntry>>,
    scale_cx: ScaleContext,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    pub atlas_full: bool,
    pub adapter_info: String,
}

impl Gfx {
    pub async fn new(window: Arc<winit::window::Window>) -> Gfx {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window).unwrap();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("no adapter");
        let info = adapter.get_info();
        let adapter_info = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("rove-spike"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                ..Default::default()
            })
            .await
            .expect("no device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: ATLAS,
                height: ATLAS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // reserve a 2x2 opaque white block at (0,0) used by solid-colour quads
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8; 16],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
        );

        let view = atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals {
                viewport: [config.width as f32, config.height as f32],
                _pad: [0.0; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quads"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Inst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let inst_cap = 8192;
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("inst"),
            size: (inst_cap * std::mem::size_of::<Inst>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Gfx {
            device,
            queue,
            surface,
            config,
            pipeline,
            bind_group,
            globals,
            atlas,
            inst_buf,
            inst_cap,
            cache: HashMap::new(),
            scale_cx: ScaleContext::new(),
            shelf_x: 2,
            shelf_y: 0,
            shelf_h: 2,
            atlas_full: false,
            adapter_info,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&self.device, &self.config);
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                viewport: [self.config.width as f32, self.config.height as f32],
                _pad: [0.0; 2],
            }),
        );
    }

    /// Rasterize (or look up) one glyph. `size_px` is in physical pixels.
    pub fn glyph(
        &mut self,
        font_data: &[u8],
        font_index: u32,
        font_id: u64,
        glyph_id: u16,
        size_px: f32,
    ) -> Option<GlyphEntry> {
        let key = (font_id, glyph_id, size_px.to_bits());
        if let Some(e) = self.cache.get(&key) {
            return *e;
        }
        let entry = self.rasterize(font_data, font_index, glyph_id, size_px);
        self.cache.insert(key, entry);
        entry
    }

    fn rasterize(
        &mut self,
        font_data: &[u8],
        font_index: u32,
        glyph_id: u16,
        size_px: f32,
    ) -> Option<GlyphEntry> {
        let font = FontRef::from_index(font_data, font_index as usize)?;
        let mut scaler = self
            .scale_cx
            .builder(font)
            .size(size_px)
            .hint(true)
            .build();
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .offset(Vector::new(0.0, 0.0))
        .render(&mut scaler, glyph_id)?;

        let w = image.placement.width;
        let h = image.placement.height;
        if w == 0 || h == 0 {
            return None;
        }
        let color = image.content == swash::scale::image::Content::Color;
        let rgba: Vec<u8> = if color {
            image.data.clone()
        } else {
            let mut v = Vec::with_capacity((w * h * 4) as usize);
            for a in image.data.iter() {
                v.extend_from_slice(&[255, 255, 255, *a]);
            }
            v
        };

        // shelf allocation
        if self.shelf_x + w + 1 > ATLAS {
            self.shelf_x = 0;
            self.shelf_y += self.shelf_h + 1;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + 1 > ATLAS {
            self.atlas_full = true;
            return None;
        }
        let (x, y) = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + 1;
        self.shelf_h = self.shelf_h.max(h);

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        Some(GlyphEntry {
            uv: [
                x as f32 / ATLAS as f32,
                y as f32 / ATLAS as f32,
                (x + w) as f32 / ATLAS as f32,
                (y + h) as f32 / ATLAS as f32,
            ],
            left: image.placement.left,
            top: image.placement.top,
            w,
            h,
            color,
        })
    }

    pub fn render(&mut self, insts: &[Inst], bg: wgpu::Color) -> Result<(), &'static str> {
        if insts.len() > self.inst_cap {
            self.inst_cap = insts.len().next_power_of_two();
            self.inst_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("inst"),
                size: (self.inst_cap * std::mem::size_of::<Inst>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !insts.is_empty() {
            self.queue
                .write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(insts));
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            other => return Err(match other {
                wgpu::CurrentSurfaceTexture::Timeout => "timeout",
                wgpu::CurrentSurfaceTexture::Occluded => "occluded",
                wgpu::CurrentSurfaceTexture::Outdated => "outdated",
                wgpu::CurrentSurfaceTexture::Lost => "lost",
                _ => "validation",
            }),
        };
        let view = frame.texture.create_view(&Default::default());
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(bg),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !insts.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.inst_buf.slice(..));
                pass.draw(0..4, 0..insts.len() as u32);
            }
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
        Ok(())
    }
}

const SHADER: &str = r#"
struct Globals { viewport: vec2<f32>, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_smp: sampler;

struct VsIn {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) flags: u32,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) flags: u32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, in: VsIn) -> VsOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0));
    let c = corners[vi];
    let p = in.rect.xy + c * in.rect.zw;
    var out: VsOut;
    out.pos = vec4<f32>(
        p.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - p.y / globals.viewport.y * 2.0,
        0.0, 1.0);
    out.uv = in.uv.xy + c * (in.uv.zw - in.uv.xy);
    out.color = in.color;
    out.flags = in.flags;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(atlas_tex, atlas_smp, in.uv);
    if ((in.flags & 1u) != 0u) {
        return t;
    }
    return vec4<f32>(in.color.rgb, in.color.a * t.a);
}
"#;
