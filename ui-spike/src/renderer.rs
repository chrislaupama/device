//! Custom wgpu renderer for the timeline and the meter bridge.
//!
//! Renders into two offscreen `Rgba8Unorm` textures using the *same*
//! `wgpu::Device`/`wgpu::Queue` as Slint's FemtoVG-wgpu renderer. The textures
//! are handed to Slint once (per resize) as `slint::Image` via
//! `Image::try_from(wgpu::Texture)`; afterwards we just re-render into them
//! each frame from the `BeforeRendering` notifier, which runs on the same queue
//! right before Slint records its own frame, so no extra synchronisation is needed.

use crate::project::{Clip, PeakPyramid, PEAK_TEX_WIDTH, SAMPLES_PER_BEAT};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    target_size: [f32; 2],
    level_count: u32,
    _pad: u32,
    level_offsets: [[u32; 4]; 8],
    level_lens: [[u32; 4]; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RectInst {
    pub rect: [f32; 4],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct WaveInst {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub params: [f32; 4],
    pub src: [u32; 4],
}

/// View state (all lengths in logical px; scaled by `scale` when drawing).
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub view_start: f32, // beats
    pub px_per_beat: f32,
    pub scroll_y: f32,
    pub track_height: f32,
    pub ruler_height: f32,
    pub playhead: f32,
    pub scale: f32,
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
    #[allow(dead_code)] // kept alive alongside the bind group
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    rect_pipeline: wgpu::RenderPipeline,
    wave_pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    peaks_view: wgpu::TextureView,
    level_offsets: [[u32; 4]; 8],
    level_lens: [[u32; 4]; 8],
    level_count: u32,
    source_stride: u32,
    timeline: Option<Target>,
    meters: Option<Target>,
    rect_buf: wgpu::Buffer,
    rect_cap: usize,
    wave_buf: wgpu::Buffer,
    wave_cap: usize,
    meter_buf: wgpu::Buffer,
    meter_cap: usize,
    // scratch
    rects: Vec<RectInst>,
    waves: Vec<WaveInst>,
    meter_rects: Vec<RectInst>,
    n_base: usize,
    pub last_counts: (usize, usize, usize),
}

fn rgba(hex: u32, a: f32) -> [f32; 4] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        a,
    ]
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, peaks: &PeakPyramid) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spike shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });

        // Peaks texture: Rg32Float, 4096 wide, rows packed linearly.
        let texels = peaks.data.len() as u32;
        let rows = texels.div_ceil(PEAK_TEX_WIDTH);
        let mut padded = peaks.data.clone();
        padded.resize((rows * PEAK_TEX_WIDTH) as usize, [0.0, 0.0]);
        let peaks_tex = device.create_texture_with_data(
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
        let peaks_view = peaks_tex.create_view(&Default::default());

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spike layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let target = [Some(wgpu::ColorTargetState {
            format: TEX_FORMAT,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let rect_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rects"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_rect"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RectInst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &rect_attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_rect"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let wave_attrs =
            wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x4];
        let wave_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("waves"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_wave"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WaveInst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wave_attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_wave"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let mut level_offsets = [[0u32; 4]; 8];
        let mut level_lens = [[0u32; 4]; 8];
        let level_count = peaks.level_offsets.len().min(32) as u32;
        for l in 0..level_count as usize {
            level_offsets[l / 4][l % 4] = peaks.level_offsets[l];
            level_lens[l / 4][l % 4] = peaks.level_lens[l];
        }

        let mk = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Renderer {
            device: device.clone(),
            queue: queue.clone(),
            rect_pipeline,
            wave_pipeline,
            bgl,
            peaks_view,
            level_offsets,
            level_lens,
            level_count,
            source_stride: peaks.source_stride,
            timeline: None,
            meters: None,
            rect_buf: mk("rects", 64 * 1024 * 32),
            rect_cap: 64 * 1024,
            wave_buf: mk("waves", 16 * 1024 * 64),
            wave_cap: 16 * 1024,
            meter_buf: mk("meters", 4096 * 32),
            meter_cap: 4096,
            rects: Vec::new(),
            waves: Vec::new(),
            meter_rects: Vec::new(),
            n_base: 0,
            last_counts: (0, 0, 0),
        }
    }

    fn make_target(&self, w: u32, h: u32, label: &str) -> Target {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            // Both usages are required by `slint::Image::try_from(wgpu::Texture)`.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let globals = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.peaks_view),
                },
            ],
        });
        let g = Globals {
            target_size: [w as f32, h as f32],
            level_count: self.level_count,
            _pad: 0,
            level_offsets: self.level_offsets,
            level_lens: self.level_lens,
        };
        self.queue.write_buffer(&globals, 0, bytemuck::bytes_of(&g));
        Target { texture, view, size: (w, h), globals, bind_group }
    }

    /// Ensure the offscreen targets have the given physical sizes. Returns new
    /// textures (to be wrapped as `slint::Image`) when (re)created.
    pub fn ensure_targets(
        &mut self,
        tl: (u32, u32),
        mt: (u32, u32),
    ) -> (Option<wgpu::Texture>, Option<wgpu::Texture>) {
        let tl = (tl.0.max(1), tl.1.max(1));
        let mt = (mt.0.max(1), mt.1.max(1));
        let mut out = (None, None);
        if self.timeline.as_ref().map(|t| t.size) != Some(tl) {
            let t = self.make_target(tl.0, tl.1, "timeline");
            out.0 = Some(t.texture.clone());
            self.timeline = Some(t);
        }
        if self.meters.as_ref().map(|t| t.size) != Some(mt) {
            let t = self.make_target(mt.0, mt.1, "meters");
            out.1 = Some(t.texture.clone());
            self.meters = Some(t);
        }
        out
    }

    fn build_timeline(&mut self, clips: &[Clip], tracks: u32, v: &View) {
        let s = v.scale;
        let (w, h) = self.timeline.as_ref().unwrap().size;
        let (w, h) = (w as f32, h as f32);
        let ruler = (v.ruler_height * s).round();
        let th = v.track_height * s;
        let ppb = v.px_per_beat * s;
        let x_of = |beat: f32| (beat - v.view_start) * ppb;
        self.rects.clear();
        self.waves.clear();

        // Track lanes
        for t in 0..tracks {
            let y = ruler + t as f32 * th - v.scroll_y * s;
            if y + th < ruler || y > h {
                continue;
            }
            let c = if t % 2 == 0 { rgba(0x16171b, 1.0) } else { rgba(0x141518, 1.0) };
            self.rects.push(RectInst { rect: [0.0, y, w, th - s], color: c });
        }
        // Grid: choose a beat step so lines are >= 6 logical px apart.
        let mut step = 1.0f32;
        while step * v.px_per_beat < 6.0 {
            step *= 2.0;
        }
        let first = (v.view_start / step).floor() as i64;
        let last = ((v.view_start + w / ppb) / step).ceil() as i64;
        for i in first..=last {
            let beat = i as f32 * step;
            let x = x_of(beat).round();
            let bar = (beat as i64) % 4 == 0;
            let c = if bar { rgba(0x2a2c33, 1.0) } else { rgba(0x1d1f24, 1.0) };
            self.rects.push(RectInst { rect: [x, ruler, s.max(1.0), h - ruler], color: c });
        }
        // Clips
        let header = (4.0 * s).round();
        for c in clips {
            let x0 = x_of(c.start_beat);
            let cw = c.len_beats * ppb;
            let y = ruler + c.track as f32 * th - v.scroll_y * s + s;
            let ch = th - 3.0 * s;
            if x0 + cw < 0.0 || x0 > w || y + ch < ruler || y > h {
                continue;
            }
            let [r, g, b] = c.color;
            self.rects.push(RectInst { rect: [x0, y, cw, ch], color: [r * 0.25, g * 0.25, b * 0.25, 1.0] });
            self.rects.push(RectInst { rect: [x0, y, cw, header], color: [r, g, b, 0.85] });
            // Waveform: clamp the quad to the visible area so fragment work is bounded.
            let vx0 = x0.max(0.0);
            let vx1 = (x0 + cw).min(w);
            let spp = SAMPLES_PER_BEAT / ppb;
            let sample_at_vx0 = c.source_offset + (vx0 - x0) * spp;
            self.waves.push(WaveInst {
                rect: [vx0.floor(), y + header, (vx1 - vx0).ceil(), ch - header],
                color: [r.min(1.0) * 0.9 + 0.1, g * 0.9 + 0.1, b * 0.9 + 0.1, 1.0],
                params: [sample_at_vx0, spp, 0.0, 0.0],
                src: [c.source * self.source_stride, 0, 0, 0],
            });
        }
        // Ruler + ticks + playhead (drawn last, on top).
        let mut over = Vec::new();
        over.push(RectInst { rect: [0.0, 0.0, w, ruler], color: rgba(0x1a1b1f, 1.0) });
        for i in first..=last {
            let beat = i as f32 * step;
            let bar = (beat as i64) % 4 == 0;
            let x = x_of(beat).round();
            let th = if bar { ruler * 0.6 } else { ruler * 0.25 };
            over.push(RectInst { rect: [x, ruler - th, s.max(1.0), th], color: rgba(0x50535c, 1.0) });
        }
        let px = x_of(v.playhead).round();
        over.push(RectInst { rect: [px - s, 0.0, 2.0 * s, h], color: rgba(0x2f6bff, 1.0) });
        over.push(RectInst { rect: [px - 5.0 * s, 0.0, 10.0 * s, 6.0 * s], color: rgba(0x2f6bff, 1.0) });
        self.n_base = self.rects.len();
        self.rects.extend(over);
    }

    fn build_meters(&mut self, levels: &[f32], peaks: &[f32]) {
        let (w, h) = self.meters.as_ref().unwrap().size;
        let (w, h) = (w as f32, h as f32);
        let n = levels.len() as f32;
        let mw = w / n;
        let gap = (mw * 0.18).max(2.0);
        self.meter_rects.clear();
        let zones = [(0.0, 0.6, 0x2fbf71u32), (0.6, 0.85, 0xe3c53a), (0.85, 1.0, 0xe5484d)];
        for (i, (&lvl, &pk)) in levels.iter().zip(peaks).enumerate() {
            let x = (i as f32 * mw).round();
            let bw = (mw - gap).round().max(1.0);
            self.meter_rects.push(RectInst { rect: [x, 0.0, bw, h], color: rgba(0x101114, 1.0) });
            for &(a, b, col) in &zones {
                let top = lvl.min(b);
                if top <= a {
                    break;
                }
                let y0 = h * (1.0 - top);
                let y1 = h * (1.0 - a);
                self.meter_rects.push(RectInst { rect: [x, y0, bw, y1 - y0], color: rgba(col, 1.0) });
            }
            let py = (h * (1.0 - pk)).round();
            self.meter_rects.push(RectInst { rect: [x, py, bw, 2.0], color: rgba(0xe6e6e6, 0.9) });
        }
    }

    fn upload<T: Pod>(device: &wgpu::Device, queue: &wgpu::Queue, buf: &mut wgpu::Buffer, cap: &mut usize, data: &[T], label: &str) {
        if data.len() > *cap {
            *cap = data.len().next_power_of_two();
            *buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (*cap * std::mem::size_of::<T>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !data.is_empty() {
            queue.write_buffer(buf, 0, bytemuck::cast_slice(data));
        }
    }

    /// Build instance data and submit one command buffer rendering both textures.
    pub fn render(&mut self, clips: &[Clip], tracks: u32, view: &View, levels: &[f32], peaks: &[f32]) {
        self.build_timeline(clips, tracks, view);
        self.build_meters(levels, peaks);
        // The rect buffer holds timeline rects; waves separate; meters separate.
        Self::upload(&self.device, &self.queue, &mut self.rect_buf, &mut self.rect_cap, &self.rects, "rects");
        Self::upload(&self.device, &self.queue, &mut self.wave_buf, &mut self.wave_cap, &self.waves, "waves");
        Self::upload(&self.device, &self.queue, &mut self.meter_buf, &mut self.meter_cap, &self.meter_rects, "meters");
        self.last_counts = (self.rects.len(), self.waves.len(), self.meter_rects.len());

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("spike") });
        let tl = self.timeline.as_ref().unwrap();
        let (tw, th) = tl.size;
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("timeline"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tl.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0706, g: 0.0745, b: 0.0863, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &tl.bind_group, &[]);
            // Rects are [lanes, grid, clip bodies | ruler, playhead]: base rects, then waves, then overlay.
            let n_base = self.n_base;
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_vertex_buffer(0, self.rect_buf.slice(..));
            pass.draw(0..6, 0..n_base as u32);
            if !self.waves.is_empty() {
                let ruler = (view.ruler_height * view.scale).round() as u32;
                pass.set_scissor_rect(0, ruler.min(th), tw, th - ruler.min(th));
                pass.set_pipeline(&self.wave_pipeline);
                pass.set_vertex_buffer(0, self.wave_buf.slice(..));
                pass.draw(0..6, 0..self.waves.len() as u32);
                pass.set_scissor_rect(0, 0, tw, th);
            }
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_vertex_buffer(0, self.rect_buf.slice(..));
            pass.draw(0..6, n_base as u32..self.rects.len() as u32);
        }
        let mt = self.meters.as_ref().unwrap();
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("meters"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &mt.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.102, g: 0.106, b: 0.122, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &mt.bind_group, &[]);
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_vertex_buffer(0, self.meter_buf.slice(..));
            pass.draw(0..6, 0..self.meter_rects.len() as u32);
        }
        self.queue.submit([enc.finish()]);
    }
}

