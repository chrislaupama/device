// Timeline shaders: instanced (rounded) rects and per-pixel-column waveforms.
// All instance coordinates are physical pixels of the target texture.

struct Globals {
    target_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
// Packed min/max peak pyramids for every loaded asset (Rg32Float, 4096 wide).
@group(0) @binding(1) var peaks: texture_2d<f32>;

const PEAK_TEX_WIDTH: u32 = 4096u;

fn quad_corner(vi: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    return c[vi];
}

fn to_clip(p: vec2<f32>) -> vec4<f32> {
    let ndc = p / g.target_size * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    return vec4(ndc, 0.0, 1.0);
}

// ---------------------------------------------------------------- rects
struct RectIn {
    @location(0) rect: vec4<f32>,   // x, y, w, h
    @location(1) color: vec4<f32>,  // straight alpha
    @location(2) params: vec4<f32>, // x: corner radius (px)
};
struct RectOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) size: vec2<f32>,
    @location(3) @interpolate(flat) radius: f32,
};

@vertex
fn vs_rect(@builtin(vertex_index) vi: u32, inst: RectIn) -> RectOut {
    var o: RectOut;
    let c = quad_corner(vi);
    o.pos = to_clip(inst.rect.xy + c * inst.rect.zw);
    o.color = inst.color;
    o.local = c * inst.rect.zw;
    o.size = inst.rect.zw;
    o.radius = inst.params.x;
    return o;
}

@fragment
fn fs_rect(i: RectOut) -> @location(0) vec4<f32> {
    var a = i.color.a;
    if (i.radius > 0.0) {
        let half = i.size * 0.5;
        let q = abs(i.local - half) - (half - vec2(i.radius));
        let d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - i.radius;
        a = a * clamp(0.5 - d, 0.0, 1.0);
    }
    return vec4(i.color.rgb, a);
}

// ---------------------------------------------------------------- waveforms
struct WaveIn {
    @location(0) rect: vec4<f32>,   // waveform region, px
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>, // x: asset frame at rect.x, y: frames per px, z: last frame (exclusive)
    @location(3) src: vec4<u32>,    // x: base texel of the asset's pyramid, y: level-0 length, z: level count
};
struct WaveOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) params: vec4<f32>,
    @location(3) @interpolate(flat) src: vec4<u32>,
};

@vertex
fn vs_wave(@builtin(vertex_index) vi: u32, inst: WaveIn) -> WaveOut {
    var o: WaveOut;
    o.pos = to_clip(inst.rect.xy + quad_corner(vi) * inst.rect.zw);
    o.color = inst.color;
    o.rect = inst.rect;
    o.params = inst.params;
    o.src = inst.src;
    return o;
}

fn load_peak(index: u32) -> vec2<f32> {
    return textureLoad(peaks, vec2<i32>(i32(index % PEAK_TEX_WIDTH), i32(index / PEAK_TEX_WIDTH)), 0).xy;
}

@fragment
fn fs_wave(i: WaveOut) -> @location(0) vec4<f32> {
    let col = floor(i.pos.x) - i.rect.x;
    let fpp = max(i.params.y, 1.0);
    let s0 = max(i.params.x + col * fpp, 0.0);
    let s1 = min(s0 + fpp, i.params.z);
    if (s0 >= i.params.z) { discard; }
    // Level 0 holds one min/max pair per 32 frames; level l per 32 << l frames.
    let lvl = u32(clamp(floor(log2(fpp / 32.0)), 0.0, f32(i.src.z - 1u)));
    var off = 0u;
    var len = i.src.y;
    for (var l = 0u; l < lvl; l++) {
        off += len;
        len = (len + 1u) / 2u;
    }
    let shift = 5u + lvl;
    let a = u32(s0) >> shift;
    if (a >= len) { discard; }
    let b = min(min(u32(max(s1 - 1.0, s0)) >> shift, len - 1u), a + 3u);
    let base = i.src.x + off;
    var mn = 1.0;
    var mx = -1.0;
    for (var k = a; k <= b; k++) {
        let p = load_peak(base + k);
        mn = min(mn, p.x);
        mx = max(mx, p.y);
    }
    let mid = i.rect.y + i.rect.w * 0.5;
    let half = i.rect.w * 0.5 * 0.92;
    let top = mid - clamp(mx, -1.0, 1.0) * half - 0.5;
    let bot = mid - clamp(mn, -1.0, 1.0) * half + 0.5;
    let y = i.pos.y;
    let cov = clamp(min(y - top, bot - y) + 0.5, 0.0, 1.0);
    if (cov <= 0.0) { discard; }
    return vec4(i.color.rgb, i.color.a * cov);
}
