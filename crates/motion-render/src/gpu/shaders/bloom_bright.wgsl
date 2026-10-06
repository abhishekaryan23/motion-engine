// Bloom pass 1: bright pass of the frame fused with the first dual-filter
// downsample (4x4 taps: centre 2x2 weight 5, ring weight 1, over 32).
// Output level: vec4 (rgb, 0) in 0..1.

struct P {
    w: u32,
    h: u32,
    dw: u32,
    dh: u32,
    threshold: f32,
    den: f32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;

fn fetch(x: i32, y: i32) -> vec3<f32> {
    let xi = u32(clamp(x, 0, i32(p.w) - 1));
    let yi = u32(clamp(y, 0, i32(p.h) - 1));
    let c = unpack4(src[yi * p.w + xi]);
    let r = c.x / 255.0;
    let g = c.y / 255.0;
    let b = c.z / 255.0;
    let k = clamp(((0.2126 * r + 0.7152 * g + 0.0722 * b) - p.threshold) / p.den, 0.0, 1.0);
    return vec3<f32>(r * k, g * k, b * k);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.dw || gid.y >= p.dh) {
        return;
    }
    let cx = 2 * i32(gid.x);
    let cy = 2 * i32(gid.y);
    var acc = vec3<f32>(0.0);
    for (var j = -1; j <= 2; j = j + 1) {
        for (var i = -1; i <= 2; i = i + 1) {
            let centre = i >= 0 && i <= 1 && j >= 0 && j <= 1;
            let wgt = select(1.0, 5.0, centre);
            acc = acc + fetch(cx + i, cy + j) * wgt;
        }
    }
    dst[gid.y * p.dw + gid.x] = vec4<f32>(acc / 32.0, 0.0);
}
