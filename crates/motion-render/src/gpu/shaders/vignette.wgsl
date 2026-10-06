struct P {
    w: u32,
    h: u32,
    k: f32,
    cx: f32,
    cy: f32,
    inv_max: f32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let i = gid.y * p.w + gid.x;
    let px = src[i];
    let dx = f32(gid.x) + 0.5 - p.cx;
    let dy = f32(gid.y) + 0.5 - p.cy;
    let r = sqrt(dx * dx + dy * dy) * p.inv_max;
    let t = clamp((r - 0.45) / 0.55, 0.0, 1.0);
    let f = 1.0 - p.k * t * t * (3.0 - 2.0 * t);
    if (f < 1.0) {
        let c = unpack4(px);
        dst[i] = pack4(to_u8(c.x * f), to_u8(c.y * f), to_u8(c.z * f), px >> 24u);
    } else {
        dst[i] = px;
    }
}
