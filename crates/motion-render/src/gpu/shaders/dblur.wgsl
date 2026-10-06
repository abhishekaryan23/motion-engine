struct P {
    w: u32,
    h: u32,
    first: u32,
    n: u32,
    norm: f32,
}
struct Tap {
    ox: i32,
    oy: i32,
    wgt: f32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> taps: array<Tap>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let mx = i32(p.w) - 1;
    let my = i32(p.h) - 1;
    let x = i32(gid.x);
    let y = i32(gid.y);
    var acc = vec4<f32>(0.0);
    // The CPU's tap list (offsets and bilinear weights), in the same order.
    for (var t = p.first; t < p.first + p.n; t = t + 1u) {
        let tap = taps[t];
        let rx = u32(clamp(x + tap.ox, 0, mx));
        let ry = u32(clamp(y + tap.oy, 0, my));
        acc = acc + tap.wgt * unpack4(src[ry * p.w + rx]);
    }
    let a = to_u8(acc.w * p.norm);
    dst[gid.y * p.w + gid.x] = pack4(
        min(to_u8(acc.x * p.norm), a),
        min(to_u8(acc.y * p.norm), a),
        min(to_u8(acc.z * p.norm), a),
        a,
    );
}
