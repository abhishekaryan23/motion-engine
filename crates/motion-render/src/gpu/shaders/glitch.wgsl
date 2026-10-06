// Glitch: two entry points sharing one layout (both buffers read_write).
//
// `copy` duplicates the frame (src -> dst). `rows` then tears bands of dst in
// place, one invocation per row, applying the bands covering that row in band
// order exactly like the CPU (each band reads the row as the previous band left
// it). `src` doubles as the per-row scratch copy, which is safe because it is
// not needed again. Band positions, heights, magnitudes and signs come from the
// same lowbias32 hash and the same f32 formulas as postfx::glitch.

struct P {
    w: u32,
    h: u32,
    bands: u32,
    key: u32,
    reach: f32,
}
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

@compute @workgroup_size(16, 16)
fn copy(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let i = gid.y * p.w + gid.x;
    dst[i] = src[i];
}

// f32::round for x >= 0 (half away from zero; WGSL `round` is half to even).
fn round_away(x: f32) -> f32 {
    let f = floor(x);
    return select(f, f + 1.0, x - f >= 0.5);
}

fn wrap(v: i32, w: i32) -> u32 {
    return u32(((v % w) + w) % w);
}

@compute @workgroup_size(64)
fn rows(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.x;
    if (y >= p.h) {
        return;
    }
    let wi = i32(p.w);
    let base_row = y * p.w;
    for (var i = 0u; i < p.bands; i = i + 1u) {
        let y0 = min(u32(hash01(p.key, i * 4u) * f32(p.h)), p.h - 1u);
        let band_h = max(u32((0.01 + 0.07 * hash01(p.key, i * 4u + 1u)) * f32(p.h)), 1u);
        let y1 = min(y0 + band_h, p.h);
        if (y < y0 || y >= y1) {
            continue;
        }
        let mag = round_away((0.25 + 0.75 * hash01(p.key, i * 4u + 2u)) * p.reach);
        if (mag == 0.0) {
            continue;
        }
        let neg = (hash(p.key, i * 4u + 3u) & 1u) != 0u;
        let off = select(i32(mag), -i32(mag), neg);
        let split = max(i32(round_away(f32(abs(off)) * 0.12)), 1);
        for (var x = 0u; x < p.w; x = x + 1u) {
            src[base_row + x] = dst[base_row + x];
        }
        for (var x = 0u; x < p.w; x = x + 1u) {
            let base = i32(x) - off;
            let pc = src[base_row + wrap(base, wi)];
            let pr = src[base_row + wrap(base - split, wi)];
            let pb = src[base_row + wrap(base + split, wi)];
            let a = pc >> 24u;
            dst[base_row + x] = pack4(
                min(pr & 255u, a),
                (pc >> 8u) & 255u,
                min((pb >> 16u) & 255u, a),
                a,
            );
        }
    }
}
