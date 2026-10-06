struct P {
    w: u32,
    h: u32,
    cx: f32,
    cy: f32,
    step: f32,
    inv_n: f32,
    strength: f32,
    samples: u32,
    // Always 0 (see `opaque`).
    zero: u32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

// Identity the shader compiler cannot see through (XOR with a runtime zero).
// Metal compiles with fast-math, which would otherwise fuse the step product
// into the accumulation (`fma(c - p, step, q)`) or rewrite `q += v` into
// `p + k * v`; either changes the last bit of a ray position and flips the
// nearest texel where a position is an exact integer. The CPU's rounding
// (a rounded step, then repeated f32 adds) is the reference, so keep it.
fn opaque(v: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(v) ^ p.zero);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let i = gid.y * p.w + gid.x;
    let own = src[i];
    let maxx = i32(p.w) - 1;
    let maxy = i32(p.h) - 1;
    let px = f32(gid.x) + 0.5;
    let py = f32(gid.y) + 0.5;
    let vx = opaque((p.cx - px) * p.step);
    let vy = opaque((p.cy - py) * p.step);
    var qx = px;
    var qy = py;
    var acc = vec3<u32>(0u, 0u, 0u);
    // Same accumulation as the CPU (`q += v`), nearest texel.
    for (var k = 0u; k < p.samples; k = k + 1u) {
        let ix = u32(clamp(i32(qx), 0, maxx));
        let iy = u32(clamp(i32(qy), 0, maxy));
        let t = src[iy * p.w + ix];
        acc = acc + vec3<u32>(t & 255u, (t >> 8u) & 255u, (t >> 16u) & 255u);
        qx = opaque(qx + vx);
        qy = opaque(qy + vy);
    }
    let a = own >> 24u;
    let d = unpack4(own);
    let r = vec3<f32>(acc) * p.inv_n * p.strength;
    let o = d.xyz + r * (vec3<f32>(255.0) - d.xyz) / 255.0;
    dst[i] = pack4(min(to_u8(o.x), a), min(to_u8(o.y), a), min(to_u8(o.z), a), a);
}
