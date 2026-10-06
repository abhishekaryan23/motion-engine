// Bloom final pass: add the merged half-resolution glow (bilinear) back onto
// the frame, `min`ed with alpha so the pixmap stays valid premultiplied.

struct P {
    w: u32,
    h: u32,
    mw: u32,
    mh: u32,
    kx: f32,
    ky: f32,
    gain: f32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> glow: array<vec4<f32>>;

fn bilinear(u: f32, v: f32) -> vec3<f32> {
    let fx = u - 0.5;
    let fy = v - 0.5;
    let x0f = floor(fx);
    let y0f = floor(fy);
    let wx = fx - x0f;
    let wy = fy - y0f;
    let mx = i32(p.mw) - 1;
    let my = i32(p.mh) - 1;
    let x0 = u32(clamp(i32(x0f), 0, mx));
    let x1 = u32(clamp(i32(x0f + 1.0), 0, mx));
    let y0 = u32(clamp(i32(y0f), 0, my));
    let y1 = u32(clamp(i32(y0f + 1.0), 0, my));
    let a = glow[y0 * p.mw + x0].xyz;
    let b = glow[y0 * p.mw + x1].xyz;
    let c = glow[y1 * p.mw + x0].xyz;
    let d = glow[y1 * p.mw + x1].xyz;
    let top = a * (1.0 - wx) + b * wx;
    let bot = c * (1.0 - wx) + d * wx;
    return top * (1.0 - wy) + bot * wy;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let i = gid.y * p.w + gid.x;
    let px = src[i];
    let a = px >> 24u;
    let g = bilinear((f32(gid.x) + 0.5) * p.kx, (f32(gid.y) + 0.5) * p.ky);
    let add = vec3<u32>(g * p.gain + vec3<f32>(0.5));
    let c = vec3<u32>(px & 255u, (px >> 8u) & 255u, (px >> 16u) & 255u);
    let o = min(c + add, vec3<u32>(a));
    dst[i] = pack4(o.x, o.y, o.z, a);
}
