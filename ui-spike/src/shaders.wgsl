// Instanced rect + waveform shaders for the timeline / meter textures.
// All instance coordinates are in physical pixels of the target texture.

struct Globals {
    target_size: vec2<f32>,
    level_count: u32,
    _pad: u32,
    // level offsets (texel units, relative to a source's base), packed 4 per vec4.
    level_offsets: array<vec4<u32>, 8>,
    level_lens: array<vec4<u32>, 8>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var peaks: texture_2d<f32>;

fn quad_corner(vi: u32) -> vec2<f32> {
    // two triangles: (0,0) (1,0) (0,1) (0,1) (1,0) (1,1)
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
    @location(0) rect: vec4<f32>,  // x, y, w, h
    @location(1) color: vec4<f32>,
};
struct RectOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_rect(@builtin(vertex_index) vi: u32, inst: RectIn) -> RectOut {
    var o: RectOut;
    let p = inst.rect.xy + quad_corner(vi) * inst.rect.zw;
    o.pos = to_clip(p);
    o.color = inst.color;
    return o;
}

@fragment
fn fs_rect(i: RectOut) -> @location(0) vec4<f32> {
    return i.color;
}

// ---------------------------------------------------------------- waveforms
struct WaveIn {
    @location(0) rect: vec4<f32>,     // waveform region, px
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,   // x: sample at rect.x, y: samples per px, z: source base (as f32-bits u32 below)
    @location(3) src: vec4<u32>,      // x: source base texel index
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
    let p = inst.rect.xy + quad_corner(vi) * inst.rect.zw;
    o.pos = to_clip(p);
    o.color = inst.color;
    o.rect = inst.rect;
    o.params = inst.params;
    o.src = inst.src;
    return o;
}

fn level_offset(l: u32) -> u32 { return g.level_offsets[l / 4u][l % 4u]; }
fn level_len(l: u32) -> u32 { return g.level_lens[l / 4u][l % 4u]; }

fn load_peak(index: u32) -> vec2<f32> {
    let w = 4096u;
    return textureLoad(peaks, vec2<i32>(i32(index % w), i32(index / w)), 0).xy;
}

@fragment
fn fs_wave(i: WaveOut) -> @location(0) vec4<f32> {
    let col = floor(i.pos.x) - i.rect.x;
    let spp = max(i.params.y, 1.0);
    let s0 = max(i.params.x + col * spp, 0.0);
    let s1 = s0 + spp;
    // Pick the pyramid level whose texel covers <= spp samples (level 0 = 32 spp).
    let lvl = u32(clamp(floor(log2(spp / 32.0)), 0.0, f32(g.level_count - 1u)));
    let shift = 5u + lvl;
    let n = level_len(lvl);
    var a = u32(s0) >> shift;
    var b = u32(max(s1 - 1.0, s0)) >> shift;
    if (a >= n) { discard; }
    b = min(min(b, n - 1u), a + 3u);
    let base = i.src.x + level_offset(lvl);
    var mn = 1.0;
    var mx = -1.0;
    for (var k = a; k <= b; k++) {
        let p = load_peak(base + k);
        mn = min(mn, p.x);
        mx = max(mx, p.y);
    }
    let mid = i.rect.y + i.rect.w * 0.5;
    let half = i.rect.w * 0.5;
    let top = mid - mx * half - 0.5;
    let bot = mid - mn * half + 0.5;
    let y = i.pos.y;
    // 1px soft edge coverage.
    let cov = clamp(min(y - top, bot - y) + 0.5, 0.0, 1.0);
    if (cov <= 0.0) { discard; }
    return vec4(i.color.rgb, i.color.a * cov);
}
