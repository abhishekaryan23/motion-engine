struct P {
    w: u32,
    h: u32,
    key: u32,
    amp: f32,
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
    let a = px >> 24u;
    // hash(seed ^ time bucket, y * width + x), top 24 bits -> [0, 1).
    let u = f32(hash(p.key, i) >> 8u) * (1.0 / 16777216.0);
    let d = (u * 2.0 - 1.0) * p.amp * (f32(a) / 255.0);
    let c = unpack4(px);
    dst[i] = pack4(
        min(to_u8(c.x + d), a),
        min(to_u8(c.y + d), a),
        min(to_u8(c.z + d), a),
        a,
    );
}
