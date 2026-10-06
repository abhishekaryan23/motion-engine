//! (0.14) Deterministic noise shared by the timeline (shakes, glyph order)
//! and renderers (glitch, grain): integer hashing, no global state.

/// 32-bit integer hash (lowbias32) of `seed` and `i`.
pub fn hash(seed: u32, i: u32) -> u32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ i;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// Hash mapped to [0, 1).
pub fn hash01(seed: u32, i: u32) -> f64 {
    hash(seed, i) as f64 / 4_294_967_296.0
}

/// Smooth 1D value noise in [-1, 1] at `x` (lattice every 1.0), C1-continuous
/// (smoothstep between hashed lattice values).
pub fn smooth(seed: u32, x: f64) -> f64 {
    let i = x.floor();
    let f = x - i;
    let i = i as i64 as u32;
    let a = hash01(seed, i) * 2.0 - 1.0;
    let b = hash01(seed, i.wrapping_add(1)) * 2.0 - 1.0;
    let u = f * f * (3.0 - 2.0 * f);
    a + (b - a) * u
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_noise_is_bounded_continuous_and_seeded() {
        let mut prev = smooth(7, 0.0);
        for k in 1..2000 {
            let v = smooth(7, k as f64 * 0.01);
            assert!((-1.0..=1.0).contains(&v));
            assert!((v - prev).abs() < 0.1);
            prev = v;
        }
        assert_ne!(smooth(1, 3.3), smooth(2, 3.3));
        assert_eq!(smooth(5, 1.25), smooth(5, 1.25));
    }
}
