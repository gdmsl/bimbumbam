//! wgpu plumbing: device acquisition, the single render pipeline, and the
//! per-frame `render` entry point.
//!
//! We render with premultiplied-alpha blending against a 1×1 white "atlas".
//! Every quad samples the same texel — color comes from the per-vertex
//! `color` attribute already premultiplied by the CPU side. Glyphon's text
//! renderer adds its own pass into the same encoder.

use anyhow::{Context, Result, anyhow};
use bytemuck::{Pod, Zeroable};
use glyphon::{Resolution, TextArea};
use std::ptr::NonNull;
use wayland_client::{Connection, Proxy, protocol::wl_surface};
use wgpu::util::DeviceExt;

use crate::render::{DrawBatch, MAX_INDICES, MAX_VERTICES, Vertex};
use crate::text::TextSystem;

/// We always render to a premultiplied sRGB BGRA target. This is universally
/// available on Wayland + Vulkan / GL combinations we care about, and pinning
/// it lets the pipeline and the glyphon atlas share a known format. We verify
/// the surface advertises it in [`Gpu::configure_surface`] and otherwise fail
/// loudly.
pub const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

#[repr(C, align(16))]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    screen_size: [f32; 2],
    scale: f32,
    _pad0: f32,
    offset: [f32; 2],
    _pad1: [f32; 2],
}

const SHADER: &str = r#"
struct Uniforms {
    screen_size: vec2<f32>,
    scale: f32,
    _pad0: f32,
    offset: vec2<f32>,
    _pad1: vec2<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_samp: sampler;

struct VIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
};

struct VOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    let scaled_pos = in.pos * uniforms.scale + uniforms.offset;
    let ndc = scaled_pos / uniforms.screen_size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = in.color;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas_tex, atlas_samp, in.uv).a;
    return in.color * coverage;
}
"#;

pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// Static bind group — every binding (uniform buffer, white texture,
    /// sampler) is invariant for the lifetime of `Gpu`, so we build it once.
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    /// Held alive only so the [`bind_group`](Self::bind_group) sampler binding remains valid.
    _sampler: wgpu::Sampler,
    /// Held alive only so the bind group's texture view remains valid.
    _white_texture: wgpu::Texture,
    /// Held alive only so the bind group's texture view remains valid.
    _white_view: wgpu::TextureView,
}

pub struct RenderParams<'a, 'b> {
    pub surface: &'a wgpu::Surface<'static>,
    pub batch: &'a DrawBatch,
    pub text_sys: &'a mut TextSystem,
    pub text_areas: &'a [TextArea<'b>],
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub offset: (f32, f32),
    pub clear_color: [f64; 3],
}

pub struct CaptureParams<'a, 'b> {
    pub batch: &'a DrawBatch,
    pub text_sys: &'a mut TextSystem,
    pub text_areas: &'a [TextArea<'b>],
    pub width: u32,
    pub height: u32,
    pub clear_color: [f64; 3],
}

/// Captured frame: RGBA8 row-tightly-packed pixels plus dimensions.
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // The whole frame is transparent quads on a clear color — integrated
            // GPUs are plenty fast and consume less power.
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))
        .context(
            "no suitable GPU adapter found — install vulkan-loader and ensure your Vulkan ICD is \
             discoverable (on NixOS, /run/opengl-driver/share must be in XDG_DATA_DIRS)",
        )?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .context("failed to create wgpu device")?;

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bimbumbam.bgl"),
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bimbumbam.shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bimbumbam.pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bimbumbam.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 8,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 24,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: SURFACE_FORMAT,
                    blend: Some(wgpu::BlendState {
                        // Premultiplied alpha "over" compositing.
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bimbumbam.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bimbumbam.vbo"),
            size: (MAX_VERTICES * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bimbumbam.ibo"),
            size: (MAX_INDICES * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let white_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("bimbumbam.white"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255u8, 255, 255, 255],
        );
        let white_view = white_texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bimbumbam.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bimbumbam.bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            pipeline,
            bind_group,
            uniform_buffer,
            vertex_buffer,
            index_buffer,
            _sampler: sampler,
            _white_texture: white_texture,
            _white_view: white_view,
        })
    }

    pub fn create_surface(
        &self,
        conn: &Connection,
        wl_surface: &wl_surface::WlSurface,
    ) -> Result<wgpu::Surface<'static>> {
        let display_ptr = conn.backend().display_ptr();
        let surface_ptr = wl_surface.id().as_ptr();
        // SAFETY: `Connection` outlives the wgpu surface for the lifetime of `App`
        // and `WlSurface` is an owned proxy bumping the refcount of the underlying
        // wayland-client object. The surface is dropped before the connection
        // (see [`crate::wayland::App::run`]). Both pointers are non-null
        // for live proxies, but we still null-check to avoid UB on a hypothetical
        // future change.
        unsafe {
            use raw_window_handle::{
                RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
            };
            let display = NonNull::new(display_ptr.cast())
                .ok_or_else(|| anyhow!("wayland display pointer is null"))?;
            let surface = NonNull::new(surface_ptr.cast())
                .ok_or_else(|| anyhow!("wayland surface pointer is null"))?;
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                        display,
                    ))),
                    raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
                })
                .context("failed to create wgpu surface from wayland handle")
        }
    }

    pub fn configure_surface(
        &self,
        surface: &wgpu::Surface<'_>,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let caps = surface.get_capabilities(&self.adapter);
        if !caps.formats.contains(&SURFACE_FORMAT) {
            return Err(anyhow!(
                "GPU surface does not advertise BGRA8 sRGB format (got: {:?})",
                caps.formats
            ));
        }
        let alpha = if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            // Defensive fallback — every real Wayland compositor advertises at
            // least `Auto`, but a degraded ICD reporting an empty list would
            // otherwise panic before the user sees an error.
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        surface.configure(
            &self.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: SURFACE_FORMAT,
                width,
                height,
                present_mode,
                alpha_mode: alpha,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );
        Ok(())
    }

    /// Returns `true` on a successful submit, `false` if the surface is in a
    /// transient state (Lost/Outdated) and the caller should reconfigure it
    /// before the next frame.
    pub fn render(&self, params: RenderParams<'_, '_>) -> bool {
        let RenderParams {
            surface,
            batch,
            text_sys,
            text_areas,
            width,
            height,
            scale,
            offset,
            clear_color,
        } = params;

        let output = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                return false;
            }
            // Timeout / OutOfMemory — drop the frame; nothing useful we can do.
            _ => return true,
        };
        let view = output.texture.create_view(&Default::default());

        text_sys
            .viewport
            .update(&self.queue, Resolution { width, height });

        // Glyphon may fail to prepare a frame if its atlas is too small for the
        // current set of glyphs. Text is decorative for us so we drop the frame
        // and keep going, but log so a debug session can see it happen.
        if let Err(e) = text_sys.renderer.prepare(
            &self.device,
            &self.queue,
            &mut text_sys.font_system,
            &mut text_sys.atlas,
            &text_sys.viewport,
            text_areas.iter().cloned(),
            &mut text_sys.swash_cache,
        ) {
            tracing::warn!(error = %e, "glyphon prepare failed, skipping text this frame");
        }

        if !batch.vertices.is_empty() {
            let uniforms = Uniforms {
                screen_size: [width as f32, height as f32],
                scale,
                _pad0: 0.0,
                offset: [offset.0, offset.1],
                _pad1: [0.0, 0.0],
            };
            self.queue
                .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
            self.queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&batch.vertices),
            );
            self.queue
                .write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&batch.indices));
        }

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bimbumbam.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear_color[0],
                            g: clear_color[1],
                            b: clear_color[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !batch.vertices.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.indices.len() as u32, 0, 0..1);
            }
            if let Err(e) = text_sys
                .renderer
                .render(&text_sys.atlas, &text_sys.viewport, &mut pass)
            {
                tracing::warn!(error = %e, "glyphon render failed");
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        text_sys.atlas.trim();
        true
    }

    /// Render a single frame at scale=1, offset=(0,0) to an offscreen texture
    /// and read it back as RGBA8 bytes.
    ///
    /// Used by the screenshot pipeline. Synchronous: blocks the calling
    /// thread until the GPU has finished and the readback buffer maps. The
    /// caller is expected to hand the result off to a worker thread for
    /// PNG encoding.
    pub fn capture(&self, params: CaptureParams<'_, '_>) -> Result<CapturedFrame> {
        let CaptureParams {
            batch,
            text_sys,
            text_areas,
            width,
            height,
            clear_color,
        } = params;

        if width == 0 || height == 0 {
            return Err(anyhow!("cannot capture a zero-sized frame"));
        }

        // Offscreen colour target. RENDER_ATTACHMENT for the pass, COPY_SRC so
        // we can blit it into a buffer afterwards.
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bimbumbam.capture.tex"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SURFACE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());

        text_sys
            .viewport
            .update(&self.queue, Resolution { width, height });
        if let Err(e) = text_sys.renderer.prepare(
            &self.device,
            &self.queue,
            &mut text_sys.font_system,
            &mut text_sys.atlas,
            &text_sys.viewport,
            text_areas.iter().cloned(),
            &mut text_sys.swash_cache,
        ) {
            tracing::warn!(error = %e, "glyphon prepare failed during capture");
        }

        if !batch.vertices.is_empty() {
            let uniforms = Uniforms {
                screen_size: [width as f32, height as f32],
                scale: 1.0,
                _pad0: 0.0,
                offset: [0.0, 0.0],
                _pad1: [0.0, 0.0],
            };
            self.queue
                .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
            self.queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&batch.vertices),
            );
            self.queue
                .write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&batch.indices));
        }

        // Buffer rows must be aligned to COPY_BYTES_PER_ROW_ALIGNMENT (256).
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = u64::from(padded_bytes_per_row) * u64::from(height);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bimbumbam.capture.buf"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bimbumbam.capture.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear_color[0],
                            g: clear_color[1],
                            b: clear_color[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !batch.vertices.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.indices.len() as u32, 0, 0..1);
            }
            if let Err(e) = text_sys
                .renderer
                .render(&text_sys.atlas, &text_sys.viewport, &mut pass)
            {
                tracing::warn!(error = %e, "glyphon render failed during capture");
            }
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Block until the readback buffer is mapped.
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        rx.recv()
            .map_err(|_| anyhow!("capture readback channel closed unexpectedly"))?
            .map_err(|e| anyhow!("buffer map failed: {e:?}"))?;

        let data = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        // Strip row-padding and swap BGRA → RGBA. SURFACE_FORMAT is sRGB BGRA.
        for row in 0..height {
            let row_start = (row * padded_bytes_per_row) as usize;
            for x in 0..width as usize {
                let p = row_start + x * 4;
                rgba.extend_from_slice(&[data[p + 2], data[p + 1], data[p], data[p + 3]]);
            }
        }
        drop(data);
        buffer.unmap();

        Ok(CapturedFrame {
            width,
            height,
            rgba,
        })
    }
}
