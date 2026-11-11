# ADR 0001: UI rendering architecture (Slint shell + custom wgpu surfaces)

- Status: Accepted (implemented in `crates/dstudio-app/src/gpu/`; the spike that backed it was removed from the
  tree after it landed and is in the history at commit `148d66c`, folder `ui-spike/`)
- Date: 2026-11-08
- Spike environment: macOS 27.0.1, Apple M4 (Metal), built-in Retina panel at 60 Hz, scale factor 2.0, rustc 1.99.1

## Context

Device Studio needs a declarative UI toolkit for its chrome (panels, inspectors, dialogs, text) and
very fast custom drawing for the dense surfaces: an arrangement timeline with hundreds of clips
and waveforms, and meter bridges animating at display rate. We picked Slint as the main candidate,
so the open question was how Slint and custom GPU rendering fit together, and whether it holds
up under real numbers rather than in a demo.

## Options considered

| | Option | Result |
|---|---|---|
| a | `unstable-wgpu-30` + `BackendSelector::require_wgpu_30` + `Window::set_rendering_notifier`, drawing under/over Slint into the frame | **Not possible as described.** The notifier's `GraphicsAPI::WGPU30 { instance, device, queue }` gives no surface texture, view or encoder, so you cannot draw into Slint's frame directly. The notifier is still how you get the shared device and when to render (see b). |
| b | Render into our own `wgpu::Texture` on Slint's device/queue and show it with `slint::Image::try_from(wgpu::Texture)` in an `Image` element | **Works. Recommended.** Zero-copy import, correct z-order with Slint content on both sides, about 0.13–0.17 ms CPU per frame for the full timeline plus meters. |
| c | Pure Slint: waveforms as `Path` elements, meters as `Rectangle`s | Works at low fidelity (128 points/clip at 60 fps, 5–6 ms CPU). **Falls off a cliff** at the fidelity a DAW needs (1024 points/clip: 47 fps avg and 52 ms p99 spikes while zooming; 2048 points: 32 fps). |
| d | Separate native child surface (NSView/HWND/wl_subsurface) | Not built. (b) works, and (d) would bring per-platform compositing, z-order, input-routing and resize-sync problems that (b) avoids. Keep it only as the fallback if (b) ever regresses. |

## Exact versions (as pinned in the spike's `Cargo.toml` at the time)

- `slint = "=1.18.1"` with `default-features = false` and features `std, backend-winit, renderer-femtovg-wgpu, unstable-wgpu-30, compat-1-2, accessibility`
- `slint-build = "=1.18.1"` (build-dependency)
- `wgpu = "=30.0.1"` (the same single crate instance Slint uses; `cargo tree -d` shows no duplicate wgpu)
- Resolved transitively: `femtovg 0.27.0`, `winit 0.30.13`, `naga 30.0.1`, `i-slint-renderer-femtovg 1.18.1`, `i-slint-backend-winit 1.18.1`
- Small crates: `bytemuck =1.24.0`, `png =0.18.1`, `pollster =0.4.0`

Slint 1.18.1 has both `unstable-wgpu-29` and `unstable-wgpu-30`, and re-exports the matching crate
as `slint::wgpu_30::wgpu`. **Our `wgpu` version must exactly match the major version Slint's feature names.**

## APIs that worked (verified in the spike)

1. **Backend selection with our own device** (`WGPUConfiguration::Manual`):
   ```rust
   let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
   let adapter  = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
       power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))?;
   let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
       required_limits: adapter.limits(), ..Default::default() }))?;
   slint::BackendSelector::new()
       .backend_name("winit".into())
       .renderer_name("femtovg-wgpu".into())
       .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Manual { instance, adapter, device, queue })
       .select()?;
   ```
   `WGPUConfiguration::Automatic(WGPUSettings)` also works (`--device slint`). Its default
   `device_required_limits` is `Limits::downlevel_webgl2_defaults()`, raised only to fit the
   window resolution: no storage buffers, and 2D textures capped near the swapchain size. That is
   too tight for a DAW renderer (storage buffers for peaks, large atlases). **Use `Manual`, or set
   `WGPUSettings::device_required_limits` explicitly.**
2. **Rendering notifier** (`Window::set_rendering_notifier`):
   - `RenderingSetup` gives `GraphicsAPI::WGPU30 { device, queue, .. }`. We build pipelines and the peak texture here.
   - `BeforeRendering` fires after Slint acquires the swapchain texture and clears it, before it
     records the UI. We update the view state, record our two render passes, and call `queue.submit`.
     Submitting on the same queue is enough ordering; no fences are needed.
   - `AfterRendering` fires before present. We call `window.request_redraw()` here for continuous animation.
   - Setting Slint properties (view state, models, image) from inside `BeforeRendering` works and takes effect in the same frame.
3. **Texture import**: `slint::Image::try_from(texture)` requires format `Rgba8Unorm` or
   `Rgba8UnormSrgb` and usage `RENDER_ATTACHMENT | TEXTURE_BINDING`. FemtoVG wraps it with
   `create_image_from_native_texture`, so there is **no copy**. We set the image property **only when
   the texture is (re)created on resize**. Every other frame we just re-render into the same texture.
4. **Compositing order**: Slint content above and below the `Image` composites correctly. Ruler bar
   numbers are Slint `Text` drawn *over* the wgpu texture, and track headers are Slint elements kept
   in sync with the wgpu vertical scroll.
5. **`Window::take_snapshot()`** works with femtovg-wgpu and includes imported textures
   (`*-snapshot.png`). That is useful for headless visual regression tests.
6. **Input**: `TouchArea.scroll-event(PointerScrollEvent)` provides `delta-x`, `delta-y`, `modifiers`
   and `mouse-x`. `ScaleRotateGestureHandler` (Slint 1.18) provides trackpad pinch on macOS
   (`scale`, `center`). `clicked`/`pressed-x` handle seeking. These are forwarded to Rust callbacks
   that mutate the view state the renderer reads.

## Measured numbers

Each run renders 600 frames after a 30-frame warmup, launched from the GUI session with
`--frames 600 --exit`. Window 1440×801 logical (2880×1602 physical; macOS clamped the height),
timeline texture 2478×1218 px, meter texture 2848×236 px. The raw JSON was in the spike's `bench/*.jsonl`.

- **frame interval**: time between consecutive `BeforeRendering` calls. It is vsync-paced (`PresentMode::AutoVsync`, which Slint forces), so 16.67 ms means a solid 60 fps on this display.
- **ours**: CPU time inside our `BeforeRendering` work: animation, culling, instance building, buffer upload, encoding and submit.
- **slint**: CPU time from the end of our callback to `AfterRendering`. That is FemtoVG recording and submitting the UI, including any `Path` tessellation.

| Mode | Scene | anim | frame interval avg / p99 (ms) | ours avg / p99 (ms) | slint avg / p99 (ms) |
|---|---|---|---|---|---|
| wgpu (b) | 32×16 clips | none | 16.67 / 17.31 | 0.125 / 0.22 | 2.50 / 3.51 |
| wgpu (b) | 32×16 | pan | 16.67 / 18.27 | 0.157 / 0.50 | 2.68 / 3.83 |
| wgpu (b) | 32×16 | zoom | 16.67 / 17.79 | 0.157 / 0.38 | 2.46 / 3.35 |
| wgpu (b) | **128×64 (8192 clips)** | zoom | 16.67 / 17.45 | 0.167 / 0.36 | 2.57 / 3.85 |
| wgpu (b), `Automatic` device | 32×16 | zoom | 16.67 / 17.09 | 0.121 / 0.19 | 2.05 / 3.18 |
| Path (c), 128 pts/clip | 32×16 | none | 16.67 / 17.08 | 0.006 | 5.27 / 5.78 |
| Path (c), 128 pts | 32×16 | pan | 16.67 / 17.12 | 0.044 | 5.50 / 6.13 |
| Path (c), 128 pts | 32×16 | zoom | 16.67 / 17.09 | 0.056 | 6.38 / 10.49 |
| Path (c), 128 pts | 8×8 | zoom | 16.67 / 17.03 | 0.024 | 4.17 / 5.16 |
| Path (c), 512 pts | 32×16 | zoom | **18.83 / 29.64** (53 fps) | 0.057 | 11.11 / 26.89 |
| Path (c), 1024 pts | 32×16 | none | **19.92 / 32.80** (50 fps) | 0.008 | 17.64 / 20.79 |
| Path (c), 1024 pts | 32×16 | pan | **21.91 / 32.55** (46 fps) | 0.055 | 17.50 / 19.08 |
| Path (c), 1024 pts | 32×16 | zoom | **21.17 / 52.27** (47 fps) | 0.055 | 14.77 / 51.42 |
| Path (c), 2048 pts | 32×16 | zoom | **31.00 / 107.3** (32 fps) | 0.060 | 27.90 / 106.2 |

Reading the numbers:

- Option (b) costs a **flat ~0.15 ms of CPU** per frame. That does not grow with clip count:
  8192 clips is the same as 512, because culling is O(n) but trivial and the fragment work is
  bounded by visible pixels. The waveform shader reads a min/max peak pyramid (32 samples/peak at
  level 0, 17 levels, 2M samples × 8 sources) and fetches at most 4 texels per pixel column at any zoom.
- The ~2.5 ms "slint" share in wgpu mode is FemtoVG drawing the shell: about 60 text items, panels,
  and the two textured quads.
- Option (c) re-tessellates and fills every visible `Path` on every frame **even when nothing moves**
  (1024 pts, `anim none`: 17.6 ms CPU). Cost scales with points × visible clips. At 128 points
  the waveform is too coarse for editing. At 1024–2048 points, about one point per pixel column
  of a wide clip on a HiDPI display, it misses 60 fps with large spikes. Each zoom level would also
  need a fresh path from the pyramid (the spike builds paths once, which is the best case for (c)).
  **This is why custom rendering is needed.** Pure Slint is fine for chrome, and meters alone (64
  `Rectangle`s) would be fine too.
- Path-mode gotcha: `Path` defaults to aspect-preserving fit, so a 512×2 viewbox draws as a flat
  line. You must set `fit: fill`. We found this through the screenshots.

## Limitations and risks

- **Unstable feature flags.** `unstable-wgpu-30` ties us to one wgpu major. Slint documents that the
  module "may be removed or changed in future minor releases", so each Slint upgrade may require a
  lockstep wgpu bump of our renderer. Mitigations: keep all wgpu code in one crate behind a narrow
  interface, pin `=` versions, and upgrade both together. Slint 1.18 already carries both 29 and 30,
  which allows staggered migration.
- **No direct draw into Slint's frame** (option a as stated). Every custom surface costs one
  offscreen texture: memory of width × height × 4 bytes (about 12 MB for a 2478×1218 timeline) plus
  one extra sampled-quad blit. This is negligible on Apple Silicon and should be checked on low-end iGPUs.
- **Texture formats.** Only `Rgba8Unorm`/`Rgba8UnormSrgb` can be imported: no HDR/float and no MSAA
  target. If we need anti-aliasing beyond analytic coverage (the spike uses analytic 1 px coverage
  in the waveform shader), resolve MSAA into the Rgba8 texture ourselves.
- **Colour space.** We write sRGB-encoded values into `Rgba8Unorm`, and FemtoVG composites them
  as-is. Colours matched the Slint chrome by eye. Verify once colour management or wide-gamut displays matter.
- **HiDPI.** Read the element's logical size (an `out property` bound to the element's
  width/height) × `window.scale_factor()`, size the texture in physical pixels, and scale all
  layout constants by the same factor. 1:1 sampling depends on the `Image` element sitting at
  integer physical coordinates. FemtoVG's internal canvas uses `ceil(scale)`. Verified only at
  scale 2.0. Fractional scales (1.25/1.5 on Windows/Linux) are **untested** and could blur by up to one pixel.
- **Frame pacing.** Slint forces `PresentMode::AutoVsync` (FIFO). Continuous animation comes from
  `request_redraw()` in `AfterRendering`. The main app should request redraws only while transport,
  meters or animations are live, to save power. The display tested was 60 Hz. ProMotion/120 Hz and
  variable refresh are untested.
- **Meter latency.** Meter values should come from the audio thread through a lock-free snapshot
  read in `BeforeRendering`. The spike synthesises them on the UI thread.
- **Input routing.** Everything goes through Slint (`TouchArea`, `ScaleRotateGestureHandler`). This
  keeps Slint content and wgpu content under one hit-testing system, but hit-testing inside the
  timeline (clip under cursor, drag handles) must be done in Rust against our own geometry. The
  callbacks are wired and compile. **They were not exercised with physical trackpad or mouse input
  in this session**, because the benchmarks used scripted pan/zoom animation.
- **Platform backends.** Verified on Metal (macOS) only. Per Slint's source, DX12 and Metal are
  excluded on platforms where they don't apply and GL is avoided by default, so we get Vulkan on
  Linux and DX12 or Vulkan on Windows. With `Manual`, `InstanceDescriptor::display` is unused on
  Vulkan/Metal/DX12 but **required for GLES, especially on Wayland**, so either keep GL disabled or
  pass winit's display handle. Wayland and X11 both run through winit 0.30. `Window::set_position`,
  which the spike uses for screenshots, does not work on Wayland.
- **GPU time is not measured.** Frame intervals stayed at vsync in every wgpu run, so GPU time fits
  in the frame, but there is no timestamp-query breakdown yet. Add `TIMESTAMP_QUERY` profiling in
  the real renderer.
- **Software fallback (not tested).** On machines without a hardware wgpu adapter (VMs, remote
  desktops), Slint's own source probes for GPU adapters and prefers a software renderer, which would
  not satisfy `require_wgpu_30`. The app needs a degraded mode, for example CPU-rasterised waveform
  images fed through `SharedPixelBuffer` on `renderer-software`, and should treat a failed
  `select()` as the trigger for it.
- **Not evaluated:** the Skia renderer (`renderer-skia` + wgpu 30 is exposed as `SkiaWGPU30Renderer`),
  and Slint's own GPU profiling. FemtoVG-wgpu was enough.

## Decision

Use **Slint 1.18 (FemtoVG on wgpu 30) for the whole window**, and render performance-critical
surfaces (arrangement timeline, waveform editors, meter bridges, later spectrum/scope views) with
**our own wgpu pipelines into offscreen textures shared with Slint as `slint::Image`** (option b).

The integration pattern:

```rust
// 1. Own the device so we control limits/features; give it to Slint.
slint::BackendSelector::new().renderer_name("femtovg-wgpu".into())
    .require_wgpu_30(WGPUConfiguration::Manual { instance, adapter, device, queue }).select()?;

// 2. Drive custom surfaces from the rendering notifier.
app.window().set_rendering_notifier(move |state, api| match state {
    RenderingState::RenderingSetup => if let GraphicsAPI::WGPU30 { device, queue, .. } = api {
        renderer = Some(Renderer::new(device, queue));          // pipelines, peak pyramid textures
    },
    RenderingState::BeforeRendering => {
        let px = (app.get_timeline_width() * scale, app.get_timeline_height() * scale);
        if let Some(tex) = renderer.ensure_target(px) {          // only on resize
            app.set_timeline_image(slint::Image::try_from(tex)?); // Rgba8Unorm, RENDER_ATTACHMENT|TEXTURE_BINDING
        }
        renderer.render(&view_state);                            // encode + queue.submit on Slint's queue
    }
    RenderingState::AfterRendering => if animating { app.window().request_redraw() },
    _ => {}
});
```

```slint
timeline := Rectangle {
    Image { source: root.timeline-image; width: parent.width; height: parent.height; image-fit: fill; }
    // Slint overlays (labels, selection marquee, drag ghosts) can go above the texture.
    TouchArea { scroll-event(e) => { root.timeline-scroll(e.delta-x/1px, e.delta-y/1px, self.mouse-x/1px, e.modifiers.control); accept } }
    ScaleRotateGestureHandler { updated => { root.pinch-updated(self.scale, self.center.x/1px); } }
}
```

Rules for the main app:

- One wgpu crate (`crates/ui-render` or similar) owns every pipeline and has the only direct
  `wgpu` dependency, re-exported from `slint::wgpu_30::wgpu` so versions cannot drift.
- Waveforms come from a precomputed min/max peak pyramid (built off-thread when audio is imported)
  and are drawn per pixel column in the fragment shader. The CPU emits one instance per visible clip.
- Rendering is redraw-on-demand. Continuous redraws run only while playing or metering.
- Keep a software fallback path and a `take_snapshot()`-based visual test harness.

If a future Slint release drops or breaks wgpu texture import, the fallback in order of preference
is: (1) stay on the last working Slint/wgpu pair; (2) render with our own wgpu into a CPU-readable
buffer and import it through `SharedPixelBuffer`, which costs a GPU→CPU→GPU copy (fine for meters,
marginal for a full-screen timeline); (3) a native child surface (option d).

## How to reproduce

Check out the spike from the history (`git checkout 148d66c -- ui-spike`), then:

```sh
cd ui-spike
cargo build --release
./target/release/ui-spike                                   # interactive: wheel = scroll, Cmd/Ctrl/Alt+wheel or pinch = zoom, click = seek
./target/release/ui-spike --frames 600 --exit --anim zoom   # prints JSON stats and exits
./target/release/ui-spike --mode path --path-points 1024 --frames 600 --exit --anim zoom
./target/release/ui-spike --tracks 128 --clips 64 --frames 600 --exit --anim zoom
./target/release/ui-spike --frames 200 --exit --screenshot screenshots/name --shot-frame 120
```

Its screenshots (on-screen `screencapture` plus Slint `take_snapshot`) are in `ui-spike/screenshots/`:
`wgpu-static-*`, `wgpu-zoomed-in-*`, `wgpu-zoomed-out-*` and `path-512pts-*`.
