//! Saine Studio UI rendering spike.
//!
//! Slint shell (FemtoVG on wgpu 30) + custom wgpu-rendered timeline & meters,
//! sharing Slint's wgpu Device/Queue and composited as `slint::Image`s.
//!
//! Usage:
//!   ui-spike [--mode wgpu|path] [--tracks N] [--clips N] [--anim none|pan|zoom]
//!            [--frames N --exit] [--warmup N] [--device manual|slint]
//!            [--screenshot DIR/name] [--shot-frame N]

mod project;
mod renderer;

use project::{Clip, PeakPyramid, NUM_SOURCES, SAMPLES_PER_BEAT};
use renderer::{Renderer, View};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

slint::include_modules!();

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Wgpu,
    Path,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Anim {
    None,
    Pan,
    Zoom,
}

struct Args {
    mode: Mode,
    tracks: u32,
    clips: u32,
    anim: Anim,
    frames: Option<u64>,
    exit: bool,
    warmup: u64,
    manual_device: bool,
    screenshot: Option<String>,
    shot_frame: u64,
    path_points: usize,
}

fn parse_args() -> Args {
    let mut a = Args {
        mode: Mode::Wgpu,
        tracks: 32,
        clips: 16,
        anim: Anim::None,
        frames: None,
        exit: false,
        warmup: 30,
        manual_device: true,
        screenshot: None,
        shot_frame: 90,
        path_points: 128,
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let next = |i: &mut usize| -> String {
        *i += 1;
        v.get(*i).cloned().unwrap_or_else(|| panic!("missing value for {}", v[*i - 1]))
    };
    while i < v.len() {
        match v[i].as_str() {
            "--mode" => {
                a.mode = match next(&mut i).as_str() {
                    "path" => Mode::Path,
                    _ => Mode::Wgpu,
                }
            }
            "--tracks" => a.tracks = next(&mut i).parse().unwrap(),
            "--clips" => a.clips = next(&mut i).parse().unwrap(),
            "--anim" => {
                a.anim = match next(&mut i).as_str() {
                    "pan" => Anim::Pan,
                    "zoom" => Anim::Zoom,
                    _ => Anim::None,
                }
            }
            "--frames" => a.frames = Some(next(&mut i).parse().unwrap()),
            "--exit" => a.exit = true,
            "--warmup" => a.warmup = next(&mut i).parse().unwrap(),
            "--device" => a.manual_device = next(&mut i) != "slint",
            "--screenshot" => a.screenshot = Some(next(&mut i)),
            "--path-points" => a.path_points = next(&mut i).parse().unwrap(),
            "--shot-frame" => a.shot_frame = next(&mut i).parse().unwrap(),
            other => panic!("unknown arg {other}"),
        }
        i += 1;
    }
    a
}

#[derive(Default)]
struct Series(Vec<f64>);
impl Series {
    fn summary(&self) -> String {
        if self.0.is_empty() {
            return "null".into();
        }
        let mut s = self.0.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
        let avg = s.iter().sum::<f64>() / s.len() as f64;
        format!(
            "{{\"avg\":{:.3},\"p50\":{:.3},\"p95\":{:.3},\"p99\":{:.3},\"max\":{:.3}}}",
            avg,
            pct(0.5),
            pct(0.95),
            pct(0.99),
            s[s.len() - 1]
        )
    }
    fn avg(&self) -> f64 {
        if self.0.is_empty() { 0.0 } else { self.0.iter().sum::<f64>() / self.0.len() as f64 }
    }
}

struct State {
    args: Args,
    clips: Vec<Clip>,
    renderer: Option<Renderer>,
    adapter_info: String,
    // view
    view_start: f32,
    px_per_beat: f32,
    scroll_y: f32,
    playhead: f32,
    pinch_ppb0: f32,
    // meters
    levels: Vec<f32>,
    peaks: Vec<f32>,
    rng: u32,
    // timing
    t0: Instant,
    last_anim: Option<Instant>,
    last_before: Option<Instant>,
    slint_start: Option<Instant>,
    frame: u64,
    interval: Series,
    ours: Series,
    slint: Series,
    window_stats_from: Instant,
    tex_sizes: ((u32, u32), (u32, u32)),
    last_ruler_key: (i64, i64, i64),
}

const TRACK_HEIGHT: f32 = 28.0;
const RULER_HEIGHT: f32 = 24.0;

impl State {
    fn bar_step(&self) -> i64 {
        // Label every n bars so labels are >= 48 logical px apart.
        let mut n = 1i64;
        while (n as f32) * 4.0 * self.px_per_beat < 48.0 {
            n *= 2;
        }
        n
    }

    fn clamp_view(&mut self, timeline_h: f32) {
        self.px_per_beat = self.px_per_beat.clamp(0.25, 4000.0);
        self.view_start = self.view_start.max(-4.0);
        let content = RULER_HEIGHT + self.args.tracks as f32 * TRACK_HEIGHT;
        self.scroll_y = self.scroll_y.clamp(0.0, (content - timeline_h).max(0.0));
    }

    fn animate(&mut self, now: Instant) {
        let dt = self.last_anim.map(|l| (now - l).as_secs_f32()).unwrap_or(0.0).min(0.1);
        self.last_anim = Some(now);
        let t = (now - self.t0).as_secs_f32();
        // Playhead: 120 BPM = 2 beats/s, loops over 96 beats.
        self.playhead = (self.playhead + dt * 2.0) % 96.0;
        match self.args.anim {
            Anim::None => {}
            Anim::Pan => {
                self.view_start = 20.0 * (1.0 - (t * 0.5).cos());
            }
            Anim::Zoom => {
                self.px_per_beat = 24.0 * (2.0f32).powf(3.0 * (t * 0.7).sin());
                self.view_start = 8.0 + 16.0 * (1.0 - (t * 0.3).cos());
            }
        }
        // Meters: fake signal with attack/release and peak hold.
        for i in 0..self.levels.len() {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let r = self.rng as f32 / u32::MAX as f32;
            let base = 0.45 + 0.35 * (t * (1.3 + i as f32 * 0.07) + i as f32).sin();
            let target = (base + 0.25 * r).clamp(0.0, 1.0);
            let l = &mut self.levels[i];
            *l = if target > *l { target } else { *l + (target - *l) * (dt * 6.0).min(1.0) };
            let p = &mut self.peaks[i];
            *p = if *l > *p { *l } else { (*p - dt * 0.15).max(*l) };
        }
    }
}

fn make_wgpu_config(manual: bool) -> (slint::wgpu_30::WGPUConfiguration, String) {
    use slint::wgpu_30::{wgpu, WGPUConfiguration, WGPUSettings};
    if manual {
        // We own instance/adapter/device; Slint renders with them. This is how the
        // app would guarantee the limits/features it needs (Slint's Automatic
        // default asks for downlevel_webgl2 limits).
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .expect("no wgpu adapter");
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("saine-ui"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .expect("request_device");
        (
            WGPUConfiguration::Manual { instance, adapter, device, queue },
            format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type),
        )
    } else {
        let mut s = WGPUSettings::default();
        // Default is downlevel_webgl2 limits (+ resolution); fine for this spike's
        // usage (textures only, no storage buffers).
        s.power_preference = wgpu::PowerPreference::HighPerformance;
        (WGPUConfiguration::Automatic(s), "slint-automatic".into())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();
    let t_build = Instant::now();
    let peaks = Rc::new(PeakPyramid::build());
    let clips = project::build_clips(args.tracks, args.clips);
    eprintln!(
        "built {} sources x {} samples -> {} peak texels, {} clips in {:.0} ms",
        NUM_SOURCES,
        project::SOURCE_SAMPLES,
        peaks.data.len(),
        clips.len(),
        t_build.elapsed().as_secs_f64() * 1e3
    );

    let (cfg, adapter_info) = make_wgpu_config(args.manual_device);
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .require_wgpu_30(cfg)
        .select()?;

    let app = AppWindow::new()?;
    let mode = args.mode;
    app.set_path_mode(mode == Mode::Path);
    app.set_mode_text(
        format!(
            "{} mode  |  {} tracks x {} clips",
            if mode == Mode::Wgpu { "wgpu" } else { "Slint Path" },
            args.tracks,
            args.clips
        )
        .into(),
    );
    app.set_track_height(TRACK_HEIGHT);
    app.set_ruler_height(RULER_HEIGHT);

    let tracks: Vec<TrackHeader> = (0..args.tracks)
        .map(|t| {
            let [r, g, b] = project::TRACK_COLORS[t as usize % project::TRACK_COLORS.len()];
            TrackHeader {
                name: format!("Track {:02}", t + 1).into(),
                color: slint::Color::from_rgb_f32(r, g, b),
            }
        })
        .collect();
    app.set_tracks(ModelRc::new(VecModel::from(tracks)));

    let meter_model = Rc::new(VecModel::from(vec![0.0f32; 64]));
    let ruler_model = Rc::new(VecModel::<RulerLabel>::default());
    app.set_ruler_labels(ModelRc::from(ruler_model.clone()));
    if mode == Mode::Path {
        let t = Instant::now();
        let pclips: Vec<PClip> = clips
            .iter()
            .map(|c| {
                let len = (c.len_beats * SAMPLES_PER_BEAT) as usize;
                PClip {
                    track: c.track as i32,
                    start: c.start_beat,
                    len: c.len_beats,
                    color: slint::Color::from_rgb_f32(c.color[0], c.color[1], c.color[2]),
                    commands: peaks
                        .svg_envelope(c.source as usize, c.source_offset as usize, len, args.path_points)
                        .into(),
                }
            })
            .collect();
        app.set_path_points(args.path_points as f32);
        app.set_pclips(ModelRc::new(VecModel::from(pclips)));
        app.set_meter_levels(ModelRc::from(meter_model.clone()));
        eprintln!("path mode: built {} SVG paths in {:.0} ms", clips.len(), t.elapsed().as_secs_f64() * 1e3);
    }

    let now = Instant::now();
    let state = Rc::new(RefCell::new(State {
        args,
        clips,
        renderer: None,
        adapter_info,
        view_start: 0.0,
        px_per_beat: 24.0,
        scroll_y: 0.0,
        playhead: 0.0,
        pinch_ppb0: 24.0,
        levels: vec![0.0; 64],
        peaks: vec![0.0; 64],
        rng: 0x9e37_79b9,
        t0: now,
        last_anim: None,
        last_before: None,
        slint_start: None,
        frame: 0,
        interval: Series::default(),
        ours: Series::default(),
        slint: Series::default(),
        window_stats_from: now,
        tex_sizes: ((0, 0), (0, 0)),
        last_ruler_key: (i64::MIN, 0, 0),
    }));

    // ---------------- input routing: Slint callbacks -> view state ----------------
    {
        let st = state.clone();
        let aw = app.as_weak();
        app.on_timeline_scroll(move |dx, dy, x, zoom| {
            let mut s = st.borrow_mut();
            if zoom {
                let anchor = s.view_start + x / s.px_per_beat;
                s.px_per_beat *= (dy * 0.01).exp();
                s.px_per_beat = s.px_per_beat.clamp(0.25, 4000.0);
                s.view_start = anchor - x / s.px_per_beat;
            } else {
                s.view_start -= dx / s.px_per_beat;
                s.scroll_y -= dy;
            }
            let h = aw.upgrade().map(|a| a.get_timeline_height()).unwrap_or(600.0);
            s.clamp_view(h);
            if let Some(a) = aw.upgrade() {
                a.window().request_redraw();
            }
        });
    }
    {
        let st = state.clone();
        app.on_timeline_click(move |x, _y| {
            let mut s = st.borrow_mut();
            s.playhead = (s.view_start + x / s.px_per_beat).max(0.0);
        });
    }
    {
        let st = state.clone();
        app.on_pinch_started(move || {
            let mut s = st.borrow_mut();
            s.pinch_ppb0 = s.px_per_beat;
        });
    }
    {
        let st = state.clone();
        app.on_pinch_updated(move |scale, cx| {
            let mut s = st.borrow_mut();
            let anchor = s.view_start + cx / s.px_per_beat;
            s.px_per_beat = (s.pinch_ppb0 * scale).clamp(0.25, 4000.0);
            s.view_start = anchor - cx / s.px_per_beat;
        });
    }

    // ---------------- rendering notifier: the integration point ----------------
    let aw = app.as_weak();
    let st = state.clone();
    let mm = meter_model.clone();
    let rm = ruler_model.clone();
    app.window().set_rendering_notifier(move |rstate, api| {
        let Some(app) = aw.upgrade() else { return };
        match rstate {
            slint::RenderingState::RenderingSetup => {
                if let slint::GraphicsAPI::WGPU30 { device, queue, .. } = api {
                    let mut s = st.borrow_mut();
                    if s.args.mode == Mode::Wgpu {
                        // Same Device/Queue Slint renders with.
                        s.renderer = Some(Renderer::new(device, queue, &peaks));
                    }
                    eprintln!("RenderingSetup: GraphicsAPI::WGPU30, adapter={}", s.adapter_info);
                } else {
                    eprintln!("RenderingSetup: unexpected graphics API {api:?}");
                }
            }
            slint::RenderingState::BeforeRendering => {
                let t_start = Instant::now();
                let mut guard = st.borrow_mut();
                let s = &mut *guard;
                if let Some(last) = s.last_before {
                    if s.frame >= s.args.warmup {
                        s.interval.0.push((t_start - last).as_secs_f64() * 1e3);
                    }
                }
                s.last_before = Some(t_start);
                s.animate(t_start);
                let tl_h = app.get_timeline_height();
                s.clamp_view(tl_h);

                // Push view state to Slint (track headers + ruler labels + path mode use it).
                app.set_view_start(s.view_start);
                app.set_px_per_beat(s.px_per_beat);
                app.set_scroll_y(s.scroll_y);
                app.set_playhead_beat(s.playhead);
                let step = s.bar_step();
                let key = ((s.view_start * 64.0) as i64, (s.px_per_beat * 64.0) as i64, (tl_h) as i64);
                if key != s.last_ruler_key {
                    s.last_ruler_key = key;
                    let tl_w = app.get_timeline_width();
                    let first_bar = ((s.view_start / 4.0).floor() as i64).div_euclid(step) * step;
                    let mut labels = Vec::new();
                    let mut b = first_bar;
                    loop {
                        let x = (b as f32 * 4.0 - s.view_start) * s.px_per_beat;
                        if x > tl_w {
                            break;
                        }
                        if x > -40.0 && b >= 0 {
                            labels.push(RulerLabel { x, text: SharedString::from(format!("{}", b + 1)) });
                        }
                        b += step;
                    }
                    rm.set_vec(labels);
                }

                match s.args.mode {
                    Mode::Wgpu => {
                        let scale = app.window().scale_factor();
                        let tl = (
                            (app.get_timeline_width() * scale).round() as u32,
                            (app.get_timeline_height() * scale).round() as u32,
                        );
                        let mt = (
                            (app.get_meters_width() * scale).round() as u32,
                            (app.get_meters_height() * scale).round() as u32,
                        );
                        s.tex_sizes = (tl, mt);
                        let view = View {
                            view_start: s.view_start,
                            px_per_beat: s.px_per_beat,
                            scroll_y: s.scroll_y,
                            track_height: TRACK_HEIGHT,
                            ruler_height: RULER_HEIGHT,
                            playhead: s.playhead,
                            scale,
                        };
                        let r = s.renderer.as_mut().expect("renderer");
                        let (new_tl, new_mt) = r.ensure_targets(tl, mt);
                        if let Some(t) = new_tl {
                            app.set_timeline_image(slint::Image::try_from(t).expect("import timeline texture"));
                        }
                        if let Some(t) = new_mt {
                            app.set_meter_image(slint::Image::try_from(t).expect("import meter texture"));
                        }
                        r.render(&s.clips, s.args.tracks, &view, &s.levels, &s.peaks);
                    }
                    Mode::Path => {
                        for (i, &l) in s.levels.iter().enumerate() {
                            if mm.row_data(i) != Some(l) {
                                mm.set_row_data(i, l);
                            }
                        }
                    }
                }
                let t_end = Instant::now();
                if s.frame >= s.args.warmup {
                    s.ours.0.push((t_end - t_start).as_secs_f64() * 1e3);
                }
                s.slint_start = Some(t_end);
            }
            slint::RenderingState::AfterRendering => {
                let now = Instant::now();
                let mut guard = st.borrow_mut();
                let s = &mut *guard;
                if let Some(t) = s.slint_start.take() {
                    if s.frame >= s.args.warmup {
                        s.slint.0.push((now - t).as_secs_f64() * 1e3);
                    }
                }
                s.frame += 1;

                // Periodic stdout stats (interactive).
                if now - s.window_stats_from >= Duration::from_secs(3) && !s.interval.0.is_empty() {
                    let line = format!(
                        "frame {:6.2} ms ({:5.1} fps) | ours {:5.3} ms | slint {:5.3} ms",
                        s.interval.avg(),
                        1000.0 / s.interval.avg(),
                        s.ours.avg(),
                        s.slint.avg()
                    );
                    println!("[{:?}] {line}", s.args.mode);
                    app.set_stats_text(line.into());
                    s.window_stats_from = now;
                    if s.args.frames.is_none() {
                        s.interval.0.clear();
                        s.ours.0.clear();
                        s.slint.0.clear();
                    }
                }

                if s.args.screenshot.is_some() && s.frame == s.args.shot_frame {
                    let path = s.args.screenshot.clone().unwrap();
                    let aw2 = app.as_weak();
                    // Not re-entrant inside the notifier: do it from the event loop.
                    slint::Timer::single_shot(Duration::from_millis(0), move || {
                        if let Some(app) = aw2.upgrade() {
                            take_screenshots(&app, &path);
                        }
                    });
                }

                if let Some(n) = s.args.frames {
                    if s.frame == n + s.args.warmup {
                        print_json(s, &app);
                        if s.args.exit {
                            let _ = slint::quit_event_loop();
                            return;
                        }
                    }
                }
                drop(guard);
                // Continuous animation: ask for the next frame (paced by vsync / AutoVsync).
                app.window().request_redraw();
            }
            slint::RenderingState::RenderingTeardown => {
                st.borrow_mut().renderer = None;
            }
            _ => {}
        }
    })?;

    app.window().set_position(slint::LogicalPosition::new(20.0, 40.0));
    app.run()?;
    Ok(())
}

fn print_json(s: &State, app: &AppWindow) {
    let w = app.window();
    let scale = w.scale_factor();
    let sz = w.size();
    let rc = s.renderer.as_ref().map(|r| r.last_counts).unwrap_or((0, 0, 0));
    println!(
        "{{\"path_points\":{},\"mode\":\"{:?}\",\"anim\":\"{:?}\",\"tracks\":{},\"clips_per_track\":{},\"frames\":{},\"warmup\":{},\
\"adapter\":\"{}\",\"scale_factor\":{},\"window_physical\":[{},{}],\"timeline_tex\":[{},{}],\"meter_tex\":[{},{}],\
\"instances\":{{\"rects\":{},\"waves\":{},\"meter_rects\":{}}},\
\"frame_interval_ms\":{},\"our_cpu_ms\":{},\"slint_render_cpu_ms\":{}}}",
        s.args.path_points,
        s.args.mode,
        s.args.anim,
        s.args.tracks,
        s.args.clips,
        s.interval.0.len(),
        s.args.warmup,
        s.adapter_info,
        scale,
        sz.width,
        sz.height,
        s.tex_sizes.0 .0,
        s.tex_sizes.0 .1,
        s.tex_sizes.1 .0,
        s.tex_sizes.1 .1,
        rc.0,
        rc.1,
        rc.2,
        s.interval.summary(),
        s.ours.summary(),
        s.slint.summary()
    );
}

fn take_screenshots(app: &AppWindow, base: &str) {
    let w = app.window();
    // 1) Slint's own snapshot (re-renders the scene into an offscreen wgpu texture).
    match w.take_snapshot() {
        Ok(buf) => {
            let path = format!("{base}-snapshot.png");
            let f = std::fs::File::create(&path).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(f), buf.width(), buf.height());
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(buf.as_bytes()).unwrap();
            eprintln!("wrote {path}");
        }
        Err(e) => eprintln!("take_snapshot failed: {e}"),
    }
    // 2) Real on-screen pixels via screencapture (window frame incl. title bar).
    let scale = w.scale_factor();
    let pos = w.position().to_logical(scale);
    let size = w.size().to_logical(scale);
    let title_bar = 28.0;
    let rect = format!("{},{},{},{}", pos.x as i32, pos.y as i32, size.width as i32, (size.height + title_bar) as i32);
    let path = format!("{base}-screen.png");
    let status = std::process::Command::new("screencapture").args(["-x", &format!("-R{rect}"), &path]).status();
    eprintln!("screencapture -R{rect} {path}: {status:?}");
}
