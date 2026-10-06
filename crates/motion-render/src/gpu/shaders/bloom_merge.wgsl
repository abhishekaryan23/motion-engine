// Bloom: dual-filter upsample of the merged coarser level (8 bilinear taps over
// 12), blended into the finer level in place:
//   fine' = (fine + count * up(coarse)) / (count + 1)
// `count` is the number of levels already merged into `coarse`. Each invocation
// reads and writes only its own `fine` texel, so the in-place update is safe.

struct P {
    cw: u32,
    ch: u32,
    dw: u32,
    dh: u32,
    kx: f32,
    ky: f32,
    count: f32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> coarse: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> fine: array<vec4<f32>>;

// Bilinear sample at (u, v) in corner coordinates (texel i spans [i, i + 1]),
// edge clamped.
fn bilinear(u: f32, v: f32) -> vec3<f32> {
    let fx = u - 0.5;
    let fy = v - 0.5;
    let x0f = floor(fx);
    let y0f = floor(fy);
    let wx = fx - x0f;
    let wy = fy - y0f;
    let mx = i32(p.cw) - 1;
    let my = i32(p.ch) - 1;
    let x0 = u32(clamp(i32(x0f), 0, mx));
    let x1 = u32(clamp(i32(x0f + 1.0), 0, mx));
    let y0 = u32(clamp(i32(y0f), 0, my));
    let y1 = u32(clamp(i32(y0f + 1.0), 0, my));
    let a = coarse[y0 * p.cw + x0].xyz;
    let b = coarse[y0 * p.cw + x1].xyz;
    let c = coarse[y1 * p.cw + x0].xyz;
    let d = coarse[y1 * p.cw + x1].xyz;
    let top = a * (1.0 - wx) + b * wx;
    let bot = c * (1.0 - wx) + d * wx;
    return top * (1.0 - wy) + bot * wy;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.dw || gid.y >= p.dh) {
        return;
    }
    let v = (f32(gid.y) + 0.5) * p.ky;
    let u = (f32(gid.x) + 0.5) * p.kx;
    var acc = vec3<f32>(0.0);
    acc = acc + bilinear(u - 1.0, v) * 1.0;
    acc = acc + bilinear(u + 1.0, v) * 1.0;
    acc = acc + bilinear(u, v - 1.0) * 1.0;
    acc = acc + bilinear(u, v + 1.0) * 1.0;
    acc = acc + bilinear(u - 0.5, v - 0.5) * 2.0;
    acc = acc + bilinear(u + 0.5, v - 0.5) * 2.0;
    acc = acc + bilinear(u - 0.5, v + 0.5) * 2.0;
    acc = acc + bilinear(u + 0.5, v + 0.5) * 2.0;
    let up = acc / 12.0;
    let i = gid.y * p.dw + gid.x;
    let f = fine[i].xyz;
    fine[i] = vec4<f32>((f + p.count * up) / (p.count + 1.0), 0.0);
}
