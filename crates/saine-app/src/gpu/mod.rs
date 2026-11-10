//! wgpu renderer for the timeline surface (the only module that touches wgpu).
//!
//! Pattern from ADR 0001 / `ui-spike`: we render into our own offscreen
//! `Rgba8Unorm` texture on the same `Device`/`Queue` Slint renders with, from
//! the `BeforeRendering` notifier, and hand the texture to Slint once per
//! resize via `slint::Image::try_from(wgpu::Texture)`.
//!
//! The renderer is data-driven: `timeline.rs` builds a [`DrawList`] of
//! instances in physical pixels from the project + view; this module only
//! uploads and draws it. Waveforms are drawn per pixel column from a packed
//! min/max peak pyramid texture built from `Engine::peaks` (see `peaks.rs`).

use bytemuck::{Pod, Zeroable};
use slint::wgpu_30::wgpu;
use wgpu::util::DeviceExt;

pub const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const PEAK_TEX_WIDTH: u32 = 4096;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    target_size: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct RectInst {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct WaveInst {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    /// x: asset frame (engine rate) at rect.x, y: frames per px, z: end frame (exclusive)
    pub params: [f32; 4],
    /// x: base texel, y: level-0 length, z: level count
    pub src: [u32; 4],
}

/// Instances for one frame, all in physical pixels.
#[derive(Default)]
pub struct DrawList {
    /// Lane backgrounds, grid, clip bodies/headers (under the waveforms).
    pub base: Vec<RectInst>,
    /// Waveforms (scissored to `lanes_clip`).
    pub waves: Vec<WaveInst>,
    /// Over the waveforms, scissored to `lanes_clip` (automation lines over track lanes).
    pub top: Vec<RectInst>,
    /// Ruler, master lane, separators, playhead (over everything).
    pub over: Vec<RectInst>,
    /// Vertical extent (y0, y1) in px where track content may draw.
    pub lanes_clip: (u32, u32),
    pub clear: [f64; 3],
}

impl DrawList {
    pub fn clear_all(&mut self) {
        self.base.clear();
        self.waves.clear();
        self.top.clear();
        self.over.clear();
    }
}

struct Target {
    view: wgpu::TextureView,
    size: (u32, u32),
    globals: wgpu::Buffer,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    rect_pipeline: wgpu::RenderPipeline,
    wave_pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    peaks_view: wgpu::TextureView,
    peaks_generation: u64,
    target: Option<Target>,
    bind_group: Option<wgpu::BindGroup>,
    base_buf: wgpu::Buffer,
    base_cap: usize,
    over_buf: wgpu::Buffer,
    over_cap: usize,
    top_buf: wgpu::Buffer,
    top_cap: usize,
    wave_buf: wgpu::Buffer,
    wave_cap: usize,
    pub last_counts: (usize, usize, usize),
    /// Frames submitted but maybe not finished, oldest first (see [`Renderer::end_frame`]).
    in_flight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    /// Frames whose [`Renderer::end_frame`] had to wait, and the total wait.
    pub throttled: (u64, std::time::Duration),
}

/// Frames that waited in [`Renderer::end_frame`], over all renderers (the self-test reports it).
pub static THROTTLED_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Frames the GPU may lag behind the CPU before [`Renderer::end_frame`] waits.
pub const MAX_FRAMES_IN_FLIGHT: usize = 2;

fn vbuf(device: &wgpu::Device, label: &str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("timeline shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("timeline globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("timeline layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let target = [Some(wgpu::ColorTargetState { format: TEX_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })];
        let rect_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
        let wave_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x4];
        let pipeline = |label: &str, vs: &str, fs: &str, stride: usize, attrs: &[wgpu::VertexAttribute]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: attrs })],
                },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &target }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let rect_pipeline = pipeline("rects", "vs_rect", "fs_rect", std::mem::size_of::<RectInst>(), &rect_attrs);
        let wave_pipeline = pipeline("waves", "vs_wave", "fs_wave", std::mem::size_of::<WaveInst>(), &wave_attrs);
        let peaks_view = Self::make_peaks(device, queue, &[]);
        Renderer {
            device: device.clone(),
            queue: queue.clone(),
            rect_pipeline,
            wave_pipeline,
            bgl,
            peaks_view,
            peaks_generation: 0,
            target: None,
            bind_group: None,
            base_buf: vbuf(device, "base", 4096 * 48),
            base_cap: 4096,
            over_buf: vbuf(device, "over", 1024 * 48),
            over_cap: 1024,
            top_buf: vbuf(device, "top", 1024 * 48),
            top_cap: 1024,
            wave_buf: vbuf(device, "waves", 1024 * 64),
            wave_cap: 1024,
            last_counts: (0, 0, 0),
            in_flight: Default::default(),
            throttled: (0, std::time::Duration::ZERO),
        }
    }

    fn make_peaks(device: &wgpu::Device, queue: &wgpu::Queue, data: &[[f32; 2]]) -> wgpu::TextureView {
        let rows = (data.len() as u32).div_ceil(PEAK_TEX_WIDTH).max(1);
        let mut padded = data.to_vec();
        padded.resize((rows * PEAK_TEX_WIDTH) as usize, [0.0, 0.0]);
        let tex = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("peaks"),
                size: wgpu::Extent3d { width: PEAK_TEX_WIDTH, height: rows, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&padded),
        );
        tex.create_view(&Default::default())
    }

    /// Re-upload the packed peak pyramids when they changed (new import).
    pub fn sync_peaks(&mut self, generation: u64, data: &[[f32; 2]]) {
        if generation != self.peaks_generation {
            self.peaks_view = Self::make_peaks(&self.device, &self.queue, data);
            self.peaks_generation = generation;
            self.bind_group = None;
        }
    }

    /// Ensure the offscreen target has the given physical size. Returns the new
    /// texture (to wrap as `slint::Image`) when it was (re)created.
    pub fn ensure_target(&mut self, size: (u32, u32)) -> Option<wgpu::Texture> {
        let size = (size.0.max(1), size.1.max(1));
        if self.target.as_ref().map(|t| t.size) == Some(size) {
            return None;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("timeline"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            // Both usages are required by `slint::Image::try_from(wgpu::Texture)`.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let globals = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals { target_size: [size.0 as f32, size.1 as f32], _pad: [0.0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        self.target = Some(Target { view, size, globals });
        self.bind_group = None;
        Some(texture)
    }

    /// Call once per frame after Slint has rendered: keeps the GPU at most
    /// [`MAX_FRAMES_IN_FLIGHT`] frames behind. A presenting swapchain normally
    /// throttles the CPU, but an occluded window (a locked CI desktop) presents
    /// nothing and so never blocks; with a slow software adapter (WARP) every
    /// frame's command buffers and bind groups then pile up until the device
    /// runs out of memory. On a real GPU the frame waited for is long done, so
    /// this returns at once.
    pub fn end_frame(&mut self) {
        // An empty submit marks the end of everything queued so far (Slint's
        // frame included; the queue runs submissions in order).
        self.in_flight.push_back(self.queue.submit(std::iter::empty()));
        while self.in_flight.len() > MAX_FRAMES_IN_FLIGHT {
            let Some(idx) = self.in_flight.pop_front() else { break };
            let t0 = std::time::Instant::now();
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: Some(std::time::Duration::from_secs(2)) });
            let waited = t0.elapsed();
            if waited >= std::time::Duration::from_micros(500) {
                self.throttled.0 += 1;
                self.throttled.1 += waited;
                THROTTLED_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    pub fn target_size(&self) -> (u32, u32) {
        self.target.as_ref().map(|t| t.size).unwrap_or((1, 1))
    }

    fn upload<T: Pod>(device: &wgpu::Device, queue: &wgpu::Queue, buf: &mut wgpu::Buffer, cap: &mut usize, data: &[T], label: &str) {
        if data.len() > *cap {
            *cap = data.len().next_power_of_two();
            *buf = vbuf(device, label, (*cap * std::mem::size_of::<T>()) as u64);
        }
        if !data.is_empty() {
            queue.write_buffer(buf, 0, bytemuck::cast_slice(data));
        }
    }

    /// Upload the draw list and submit one command buffer on Slint's queue.
    pub fn render(&mut self, dl: &DrawList) {
        let Some(tl) = self.target.as_ref() else { return };
        if self.bind_group.is_none() {
            self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("timeline"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: tl.globals.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.peaks_view) },
                ],
            }));
        }
        Self::upload(&self.device, &self.queue, &mut self.base_buf, &mut self.base_cap, &dl.base, "base");
        Self::upload(&self.device, &self.queue, &mut self.over_buf, &mut self.over_cap, &dl.over, "over");
        Self::upload(&self.device, &self.queue, &mut self.top_buf, &mut self.top_cap, &dl.top, "top");
        Self::upload(&self.device, &self.queue, &mut self.wave_buf, &mut self.wave_cap, &dl.waves, "waves");
        self.last_counts = (dl.base.len(), dl.waves.len(), dl.over.len());
        let (tw, th) = tl.size;
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("timeline") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("timeline"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tl.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: dl.clear[0], g: dl.clear[1], b: dl.clear[2], a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
            let (y0, y1) = (dl.lanes_clip.0.min(th), dl.lanes_clip.1.min(th));
            pass.set_pipeline(&self.rect_pipeline);
            if !dl.base.is_empty() && y1 > y0 {
                pass.set_scissor_rect(0, y0, tw, y1 - y0);
                pass.set_vertex_buffer(0, self.base_buf.slice(..));
                pass.draw(0..6, 0..dl.base.len() as u32);
            }
            if !dl.waves.is_empty() && y1 > y0 {
                pass.set_pipeline(&self.wave_pipeline);
                pass.set_vertex_buffer(0, self.wave_buf.slice(..));
                pass.draw(0..6, 0..dl.waves.len() as u32);
            }
            if !dl.top.is_empty() && y1 > y0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_scissor_rect(0, y0, tw, y1 - y0);
                pass.set_vertex_buffer(0, self.top_buf.slice(..));
                pass.draw(0..6, 0..dl.top.len() as u32);
            }
            pass.set_scissor_rect(0, 0, tw, th);
            if !dl.over.is_empty() {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_vertex_buffer(0, self.over_buf.slice(..));
                pass.draw(0..6, 0..dl.over.len() as u32);
            }
        }
        self.queue.submit([enc.finish()]);
    }
}

/// Wrap a target texture for Slint (zero-copy import, ADR 0001).
pub fn to_slint_image(tex: wgpu::Texture) -> Result<slint::Image, String> {
    slint::Image::try_from(tex).map_err(|e| format!("{e:?}"))
}

/// Create our own instance/adapter/device and hand them to Slint
/// (`WGPUConfiguration::Manual`), so we control the limits (Slint's automatic
/// default is downlevel WebGL2 limits). Returns the config and an adapter description.
pub fn make_wgpu_config() -> Result<(slint::wgpu_30::WGPUConfiguration, String), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .map_err(|e| format!("no wgpu adapter: {e}"))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("saine-ui"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .map_err(|e| format!("request_device: {e}"))?;
    // Slint keeps this queue in its thread-local context, which is dropped by the main thread's
    // TLS destructors at exit, and dropping a queue takes the device's snatch lock. In debug
    // builds wgpu-core traces that lock in a thread local of its own, and TLS destructors run in
    // reverse order of first use: touched only later (first frame), the trace slot is gone before
    // Slint's context drops the queue, and the process aborts. One empty submit here (it takes
    // the snatch lock) touches the trace slot before Slint's context exists, so it outlives it.
    queue.submit([]);
    Ok((
        slint::wgpu_30::WGPUConfiguration::Manual { instance, adapter, device, queue },
        format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type),
    ))
}

/// Run `f` with the device/queue if the notifier's graphics API is wgpu 30.
pub fn with_wgpu<R>(api: &slint::GraphicsAPI<'_>, f: impl FnOnce(&wgpu::Device, &wgpu::Queue) -> R) -> Option<R> {
    match api {
        slint::GraphicsAPI::WGPU30 { device, queue, .. } => Some(f(device, queue)),
        _ => None,
    }
}
