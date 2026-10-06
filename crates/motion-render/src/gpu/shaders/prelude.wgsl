// Shared helpers, prepended to every post-effect shader.
//
// Pixels live in storage buffers as packed premultiplied RGBA8: one u32 per
// pixel, red in the low byte (the little-endian view of the pixmap bytes).
// Everything is integer-exact or plain f32 arithmetic mirroring postfx.rs; no
// hardware filtering is used (bilinear taps are evaluated by hand) so the
// weights are the CPU's weights.

// lowbias32, identical u32 arithmetic to motion_core::noise::hash.
fn hash(seed: u32, i: u32) -> u32 {
    var x = (seed * 0x9E3779B9u) ^ i;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

// hash mapped to [0, 1): the f32 nearest to hash / 2^32 (a power-of-two scale
// of the f32-rounded integer, which is what the CPU's f64 -> f32 cast yields).
fn hash01(seed: u32, i: u32) -> f32 {
    return f32(hash(seed, i)) * 2.3283064365386963e-10;
}

fn unpack4(p: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(p & 255u),
        f32((p >> 8u) & 255u),
        f32((p >> 16u) & 255u),
        f32(p >> 24u),
    );
}

fn pack4(r: u32, g: u32, b: u32, a: u32) -> u32 {
    return r | (g << 8u) | (b << 16u) | (a << 24u);
}

// (v + 0.5) clamped to 0..255 and truncated: the CPU's `to_u8`.
fn to_u8(v: f32) -> u32 {
    return u32(clamp(v + 0.5, 0.0, 255.0));
}
