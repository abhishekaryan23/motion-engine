struct P {
    w: u32,
    h: u32,
    rix: i32,
    riy: i32,
    rfx: f32,
    rfy: f32,
    bix: i32,
    biy: i32,
    bfx: f32,
    bfy: f32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

fn chan(px: u32, ch: u32) -> f32 {
    return f32((px >> (8u * ch)) & 255u);
}

// Bilinear sample of one channel at (x + ix + fx, y + iy + fy), edge clamped.
fn sample_ch(x: i32, y: i32, ix: i32, iy: i32, fx: f32, fy: f32, ch: u32) -> f32 {
    let mx = i32(p.w) - 1;
    let my = i32(p.h) - 1;
    let x0 = u32(clamp(x + ix, 0, mx));
    let x1 = u32(clamp(x + ix + 1, 0, mx));
    let y0 = u32(clamp(y + iy, 0, my));
    let y1 = u32(clamp(y + iy + 1, 0, my));
    let top = chan(src[y0 * p.w + x0], ch) * (1.0 - fx) + chan(src[y0 * p.w + x1], ch) * fx;
    let bot = chan(src[y1 * p.w + x0], ch) * (1.0 - fx) + chan(src[y1 * p.w + x1], ch) * fx;
    return top * (1.0 - fy) + bot * fy;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let i = gid.y * p.w + gid.x;
    let own = src[i];
    let a = own >> 24u;
    let x = i32(gid.x);
    let y = i32(gid.y);
    let r = min(to_u8(sample_ch(x, y, p.rix, p.riy, p.rfx, p.rfy, 0u)), a);
    let b = min(to_u8(sample_ch(x, y, p.bix, p.biy, p.bfx, p.bfy, 2u)), a);
    dst[i] = pack4(r, (own >> 8u) & 255u, b, a);
}
