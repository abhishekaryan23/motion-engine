//! Post effects (0.15) on small hand-built pixmaps: every effect is an exact
//! no-op at strength 0, deterministic, and does what its doc comment says.

use motion_core::scene::PostKind;
use motion_core::timeline::ResolvedPost;
use motion_render::postfx::apply_post;
use resvg::tiny_skia::Pixmap;

const N: u32 = 64;

fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    for px in pm.data_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    pm
}

fn set(pm: &mut Pixmap, x: u32, y: u32, rgb: [u8; 3]) {
    let w = pm.width();
    let i = ((y * w + x) * 4) as usize;
    pm.data_mut()[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
}

fn rgb(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let p = pm.pixel(x, y).expect("pixel in range");
    [p.red(), p.green(), p.blue()]
}

fn alpha(pm: &Pixmap, x: u32, y: u32) -> u8 {
    pm.pixel(x, y).expect("pixel in range").alpha()
}

/// Deterministic busy image (hash noise, opaque).
fn noisy(w: u32, h: u32) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    for (i, px) in pm.data_mut().chunks_exact_mut(4).enumerate() {
        let v = motion_core::noise::hash(99, i as u32);
        px.copy_from_slice(&[v as u8, (v >> 8) as u8, (v >> 16) as u8, 255]);
    }
    pm
}

/// Red/G/B ramps: R = 4x (so the row is a recognisable permutation under wrap).
fn ramp(w: u32, h: u32) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    for y in 0..h {
        for x in 0..w {
            set(&mut pm, x, y, [(x * 4) as u8, (x * 4) as u8, (y * 3) as u8]);
        }
    }
    pm
}

fn run(pm: &Pixmap, kind: &PostKind, strength: f32, time: f64) -> Pixmap {
    let mut out = pm.clone();
    apply_post(
        &mut out,
        &[ResolvedPost {
            kind,
            strength,
            time,
        }],
    );
    out
}

fn all_kinds() -> Vec<PostKind> {
    vec![
        PostKind::Bloom {
            threshold: 0.3,
            radius: 12.0,
        },
        PostKind::ChromaticAberration {
            shift: 4.0,
            angle_deg: 30.0,
        },
        PostKind::Glitch {
            bands: 8,
            max_shift: 20.0,
            seed: 7,
        },
        PostKind::Rays {
            center: [32.0, 32.0],
            length: 0.6,
        },
        PostKind::DirectionalBlur {
            angle_deg: 20.0,
            length: 12.0,
        },
        PostKind::Grain {
            amount: 0.8,
            seed: 3,
        },
        PostKind::Vignette { amount: 0.9 },
    ]
}

#[test]
fn strength_zero_is_an_exact_identity_for_every_effect() {
    let src = noisy(N, N);
    for kind in all_kinds() {
        for strength in [0.0, -1.0, f32::NAN] {
            let out = run(&src, &kind, strength, 1.234);
            assert_eq!(out.data(), src.data(), "{kind:?} at strength {strength}");
        }
        // A tiny positive strength is not skipped but must stay valid.
        let out = run(&src, &kind, 1e-4, 1.234);
        assert_eq!(out.width(), N);
    }
}

#[test]
fn every_effect_changes_a_noisy_frame_at_full_strength_and_keeps_it_valid() {
    let src = noisy(N, N);
    for kind in all_kinds() {
        let out = run(&src, &kind, 1.0, 0.7);
        assert_ne!(out.data(), src.data(), "{kind:?} did nothing");
        for px in out.data().chunks_exact(4) {
            assert_eq!(px[3], 255, "{kind:?} touched alpha");
        }
    }
}

#[test]
fn effects_are_deterministic() {
    let src = noisy(N, N);
    for kind in all_kinds() {
        let a = run(&src, &kind, 0.8, 2.5);
        let b = run(&src, &kind, 0.8, 2.5);
        assert_eq!(a.data(), b.data(), "{kind:?}");
    }
}

#[test]
fn posts_apply_in_list_order() {
    let src = noisy(N, N);
    let (a, b) = (
        PostKind::Vignette { amount: 0.7 },
        PostKind::Grain {
            amount: 0.5,
            seed: 1,
        },
    );
    let mut both = src.clone();
    apply_post(
        &mut both,
        &[
            ResolvedPost {
                kind: &a,
                strength: 0.9,
                time: 0.1,
            },
            ResolvedPost {
                kind: &b,
                strength: 0.6,
                time: 0.1,
            },
        ],
    );
    let step = run(&run(&src, &a, 0.9, 0.1), &b, 0.6, 0.1);
    assert_eq!(both.data(), step.data());
    // Empty list: untouched.
    let mut none = src.clone();
    apply_post(&mut none, &[]);
    assert_eq!(none.data(), src.data());
}

#[test]
fn degenerate_canvases_and_parameters_do_not_panic() {
    let kinds = [
        PostKind::Bloom {
            threshold: f32::NAN,
            radius: f32::INFINITY,
        },
        PostKind::Bloom {
            threshold: 0.0,
            radius: 500.0,
        },
        PostKind::ChromaticAberration {
            shift: 1000.0,
            angle_deg: 33.0,
        },
        PostKind::Glitch {
            bands: 64,
            max_shift: 400.0,
            seed: 0,
        },
        PostKind::Rays {
            center: [-50.0, 9999.0],
            length: 1.0,
        },
        PostKind::DirectionalBlur {
            angle_deg: 123.0,
            length: 500.0,
        },
        PostKind::DirectionalBlur {
            angle_deg: f32::NAN,
            length: 5.0,
        },
        PostKind::Grain {
            amount: 1.0,
            seed: u32::MAX,
        },
        PostKind::Vignette { amount: 1.0 },
    ];
    for (w, h) in [(1, 1), (2, 1), (1, 3), (3, 3), (17, 5)] {
        for kind in &kinds {
            let out = run(&noisy(w, h), kind, 1.0, -3.7);
            assert_eq!((out.width(), out.height()), (w, h));
        }
    }
}

// --- chromatic aberration ---------------------------------------------------

/// White left half, black right half (edge between x = 31 and 32).
fn vertical_edge() -> Pixmap {
    let mut pm = solid(N, N, [0, 0, 0]);
    for y in 0..N {
        for x in 0..N / 2 {
            set(&mut pm, x, y, [255, 255, 255]);
        }
    }
    pm
}

#[test]
fn chromatic_aberration_offsets_red_and_blue_opposite_ways() {
    let src = vertical_edge();
    let kind = PostKind::ChromaticAberration {
        shift: 4.0,
        angle_deg: 0.0,
    };
    let out = run(&src, &kind, 1.0, 0.0);
    // Red is sampled 4 px to the right, so its white region ends 4 px early;
    // blue is sampled 4 px to the left, so it ends 4 px late.
    assert_eq!(rgb(&out, 10, 7), [255, 255, 255]);
    assert_eq!(rgb(&out, 60, 7), [0, 0, 0]);
    assert_eq!(rgb(&out, 29, 7), [0, 255, 255], "cyan on the white side");
    assert_eq!(rgb(&out, 33, 7), [0, 0, 255], "blue on the black side");
    let count = |ch: usize| {
        (0..N)
            .filter(|&x| out.data()[((7 * N + x) * 4) as usize + ch] > 127)
            .count()
    };
    assert_eq!((count(0), count(1), count(2)), (28, 32, 36));
    // Green and alpha are untouched.
    for y in 0..N {
        for x in 0..N {
            assert_eq!(rgb(&out, x, y)[1], rgb(&src, x, y)[1]);
            assert_eq!(alpha(&out, x, y), 255);
        }
    }
}

#[test]
fn chromatic_aberration_scales_with_strength_follows_angle_and_sign() {
    let src = vertical_edge();
    let kind = PostKind::ChromaticAberration {
        shift: 4.0,
        angle_deg: 0.0,
    };
    // Half strength = 2 px.
    let half = run(&src, &kind, 0.5, 0.0);
    let reds = (0..N).filter(|&x| rgb(&half, x, 3)[0] > 127).count();
    let blues = (0..N).filter(|&x| rgb(&half, x, 3)[2] > 127).count();
    assert_eq!((reds, blues), (30, 34));

    // A negative shift swaps the channels' directions.
    let neg = PostKind::ChromaticAberration {
        shift: -4.0,
        angle_deg: 0.0,
    };
    let out = run(&src, &neg, 1.0, 0.0);
    assert_eq!(rgb(&out, 29, 7), [255, 255, 0], "yellow: blue lost first");
    assert_eq!(rgb(&out, 33, 7), [255, 0, 0], "red on the black side");

    // 90 degrees shifts along y: a vertical edge does not move at all, a
    // horizontal one does.
    let down = PostKind::ChromaticAberration {
        shift: 4.0,
        angle_deg: 90.0,
    };
    let v = run(&src, &down, 1.0, 0.0);
    assert_eq!(v.data(), src.data(), "vertical edge unaffected by y shift");
    let mut horiz = solid(N, N, [0, 0, 0]);
    for y in 0..N / 2 {
        for x in 0..N {
            set(&mut horiz, x, y, [255, 255, 255]);
        }
    }
    let h = run(&horiz, &down, 1.0, 0.0);
    let reds = (0..N).filter(|&y| rgb(&h, 5, y)[0] > 127).count();
    let blues = (0..N).filter(|&y| rgb(&h, 5, y)[2] > 127).count();
    assert_eq!((reds, blues), (28, 36));
}

#[test]
fn chromatic_aberration_interpolates_fractional_shifts() {
    let src = vertical_edge();
    let kind = PostKind::ChromaticAberration {
        shift: 0.5,
        angle_deg: 0.0,
    };
    let out = run(&src, &kind, 1.0, 0.0);
    // Red at x = 31 samples halfway between white (31) and black (32).
    let r = rgb(&out, 31, 0)[0];
    assert!((126..=129).contains(&r), "red {r}");
    assert_eq!(rgb(&out, 20, 0)[0], 255);
}

// --- bloom ------------------------------------------------------------------

fn block(bg: [u8; 3], fg: [u8; 3], half: u32) -> Pixmap {
    let mut pm = solid(N, N, bg);
    for y in N / 2 - half..N / 2 + half {
        for x in N / 2 - half..N / 2 + half {
            set(&mut pm, x, y, fg);
        }
    }
    pm
}

#[test]
fn bloom_brightens_the_surroundings_of_pixels_above_the_threshold_only() {
    let src = block([40, 40, 40], [255, 255, 255], 3);
    let bright = run(
        &src,
        &PostKind::Bloom {
            threshold: 0.6,
            radius: 8.0,
        },
        1.0,
        0.0,
    );
    // Neighbours of the bright block gain light; far corners stay.
    let near = rgb(&bright, N / 2 + 6, N / 2);
    assert!(near[0] > 40 + 8, "glow next to the block: {near:?}");
    assert_eq!(rgb(&bright, 0, 0), [40, 40, 40]);
    assert!(bright.data()[3] == 255);
    // Glow falls off with distance.
    let farther = rgb(&bright, N / 2 + 12, N / 2);
    assert!(farther[0] < near[0], "{farther:?} vs {near:?}");

    // The same block under a threshold above its luma produces nothing.
    let dim = block([40, 40, 40], [150, 150, 150], 3);
    let none = run(
        &dim,
        &PostKind::Bloom {
            threshold: 0.7,
            radius: 8.0,
        },
        1.0,
        0.0,
    );
    assert_eq!(none.data(), dim.data(), "below threshold: no bloom");
    // ... but a lower threshold blooms it.
    let some = run(
        &dim,
        &PostKind::Bloom {
            threshold: 0.3,
            radius: 8.0,
        },
        1.0,
        0.0,
    );
    assert!(rgb(&some, N / 2 + 6, N / 2)[0] > 40);
}

#[test]
fn bloom_scales_with_strength_and_clamps() {
    let src = block([40, 40, 40], [255, 255, 255], 3);
    let kind = PostKind::Bloom {
        threshold: 0.5,
        radius: 8.0,
    };
    let glow = |s: f32| rgb(&run(&src, &kind, s, 0.0), N / 2 + 5, N / 2)[0] as i32 - 40;
    let (g1, g2) = (glow(1.0), glow(0.5));
    assert!(g1 > 8, "{g1}");
    assert!(
        (g2 * 2 - g1).abs() <= 2,
        "half strength ~ half the glow: {g1} {g2}"
    );
    // Already-white pixels cannot overflow.
    let white = solid(N, N, [255, 255, 255]);
    let out = run(&white, &kind, 1.0, 0.0);
    assert_eq!(out.data(), white.data());
}

#[test]
fn bloom_radius_widens_the_glow() {
    let src = block([0, 0, 0], [255, 255, 255], 2);
    let reach = |radius: f32| {
        let out = run(
            &src,
            &PostKind::Bloom {
                threshold: 0.5,
                radius,
            },
            1.0,
            0.0,
        );
        (N / 2 + 2..N)
            .filter(|&x| rgb(&out, x, N / 2)[0] > 0)
            .count()
    };
    assert!(
        reach(24.0) > reach(4.0) + 4,
        "{} vs {}",
        reach(24.0),
        reach(4.0)
    );
}

// --- glitch -----------------------------------------------------------------

#[test]
fn glitch_is_deterministic_and_varies_with_seed_and_time_bucket() {
    let src = ramp(N, N);
    let kind = |seed| PostKind::Glitch {
        bands: 6,
        max_shift: 20.0,
        seed,
    };
    let a = run(&src, &kind(1), 1.0, 0.0);
    let again = run(&src, &kind(1), 1.0, 0.0);
    assert_eq!(a.data(), again.data());
    assert_ne!(a.data(), src.data());
    assert_ne!(a.data(), run(&src, &kind(2), 1.0, 0.0).data(), "seed");
    // 12 Hz buckets: t = 0.0 and 0.5 are different buckets...
    assert_ne!(a.data(), run(&src, &kind(1), 1.0, 0.5).data(), "time");
    // ... t = 0.0 and 0.05 share one.
    assert_eq!(a.data(), run(&src, &kind(1), 1.0, 0.05).data(), "bucket");
}

#[test]
fn glitch_displaces_whole_bands_by_at_most_max_shift_times_strength() {
    let src = ramp(N, N);
    let kind = PostKind::Glitch {
        bands: 5,
        max_shift: 20.0,
        seed: 11,
    };
    for strength in [1.0f32, 0.5] {
        let out = run(&src, &kind, strength, 0.3);
        let limit = (20.0 * strength).round() as i32;
        let (mut moved, mut still) = (0, 0);
        for y in 0..N {
            // Green is never split, so G(x) = 4 * wrap(x - off).
            let g0 = rgb(&out, 0, y)[1] as i32;
            let off = (-(g0 / 4)).rem_euclid(N as i32);
            let off = if off > N as i32 / 2 {
                off - N as i32
            } else {
                off
            };
            let row_same = (0..N).all(|x| rgb(&out, x, y) == rgb(&src, x, y));
            if row_same {
                still += 1;
                continue;
            }
            moved += 1;
            assert!(off.abs() <= limit, "row {y}: shift {off} > {limit}");
            for x in 0..N {
                let expect = 4 * ((x as i32 - off).rem_euclid(N as i32)) as u32;
                assert_eq!(rgb(&out, x, y)[1] as u32, expect, "row {y} x {x}");
                assert_eq!(alpha(&out, x, y), 255);
            }
        }
        assert!(moved > 0 && still > 0, "moved {moved} still {still}");
    }
}

// --- rays -------------------------------------------------------------------

#[test]
fn rays_streak_a_bright_source_away_from_the_centre() {
    let mut src = solid(N, N, [0, 0, 0]);
    for y in 30..34 {
        for x in 38..44 {
            set(&mut src, x, y, [255, 255, 255]);
        }
    }
    let kind = PostKind::Rays {
        center: [32.0, 32.0],
        length: 0.8,
    };
    let out = run(&src, &kind, 1.0, 0.0);
    // Behind the source (further from the centre) its light is gathered in.
    let behind = rgb(&out, 52, 32)[0];
    assert!(behind > 20, "streak behind the source: {behind}");
    // Between the source and the centre, and off the ray, nothing.
    assert_eq!(rgb(&out, 34, 32), [0, 0, 0]);
    assert_eq!(rgb(&out, 52, 8), [0, 0, 0]);
    // The source itself never gets darker.
    assert!(rgb(&out, 40, 32)[0] >= 255 - 1);
    // Stronger strength streaks more.
    let weak = rgb(&run(&src, &kind, 0.25, 0.0), 52, 32)[0];
    assert!(weak < behind, "{weak} < {behind}");
}

// --- directional blur -------------------------------------------------------

fn vline() -> Pixmap {
    let mut pm = solid(N, N, [0, 0, 0]);
    for y in 0..N {
        set(&mut pm, 32, y, [255, 255, 255]);
    }
    pm
}

#[test]
fn directional_blur_smears_along_the_angle_only() {
    let src = vline();
    let h = run(
        &src,
        &PostKind::DirectionalBlur {
            angle_deg: 0.0,
            length: 9.0,
        },
        1.0,
        0.0,
    );
    // Horizontal smear: spreads out to +-4 px and every row is identical.
    assert!(rgb(&h, 30, 20)[0] > 0 && rgb(&h, 34, 20)[0] > 0);
    assert!(rgb(&h, 32, 20)[0] < 255);
    assert_eq!(rgb(&h, 20, 20), [0, 0, 0]);
    assert_eq!(rgb(&h, 45, 20), [0, 0, 0]);
    for y in 1..N {
        for x in 0..N {
            assert_eq!(rgb(&h, x, y), rgb(&h, x, 0), "row {y} x {x}");
        }
    }
    // Box blur conserves the line's energy (it is far from the borders).
    let sum = |pm: &Pixmap| (0..N).map(|x| rgb(pm, x, 9)[0] as i32).sum::<i32>();
    assert!((sum(&h) - 255).abs() <= 4, "{}", sum(&h));

    // Along the line's own direction nothing changes (a line stays a line).
    let v = run(
        &src,
        &PostKind::DirectionalBlur {
            angle_deg: 90.0,
            length: 9.0,
        },
        1.0,
        0.0,
    );
    assert_eq!(v.data(), src.data());
}

#[test]
fn directional_blur_follows_diagonals_and_length_scales_with_strength() {
    let mut dot = solid(N, N, [0, 0, 0]);
    set(&mut dot, 32, 32, [255, 255, 255]);
    let kind = PostKind::DirectionalBlur {
        angle_deg: 45.0,
        length: 16.0,
    };
    let out = run(&dot, &kind, 1.0, 0.0);
    assert!(
        rgb(&out, 35, 35)[0] > 0 && rgb(&out, 29, 29)[0] > 0,
        "diagonal"
    );
    assert_eq!(rgb(&out, 36, 28), [0, 0, 0], "anti-diagonal untouched");
    assert_eq!(rgb(&out, 32, 20), [0, 0, 0]);
    // Half strength = half the length: the reach shrinks.
    let half = run(&dot, &kind, 0.5, 0.0);
    let reach = |pm: &Pixmap| {
        (1..20)
            .take_while(|d| rgb(pm, 32 + d, 32 + d)[0] > 0)
            .count()
    };
    assert!(
        reach(&half) < reach(&out),
        "{} {}",
        reach(&half),
        reach(&out)
    );
    // Zero length: no-op.
    let zero = run(
        &dot,
        &PostKind::DirectionalBlur {
            angle_deg: 45.0,
            length: 0.0,
        },
        1.0,
        0.0,
    );
    assert_eq!(zero.data(), dot.data());
}

// --- grain ------------------------------------------------------------------

#[test]
fn grain_is_seeded_per_frame_bounded_and_monochrome() {
    let src = solid(N, N, [128, 128, 128]);
    let kind = |seed| PostKind::Grain { amount: 0.5, seed };
    let a = run(&src, &kind(1), 1.0, 0.0);
    assert_eq!(a.data(), run(&src, &kind(1), 1.0, 0.0).data());
    assert_ne!(a.data(), run(&src, &kind(2), 1.0, 0.0).data(), "seed");
    assert_ne!(a.data(), run(&src, &kind(1), 1.0, 1.0).data(), "time");
    // Same 1/24 s bucket: same grain.
    assert_eq!(a.data(), run(&src, &kind(1), 1.0, 0.02).data(), "bucket");

    let bound = (0.5f32 * 0.25 * 255.0).ceil() as i32 + 1;
    let mut sum = 0i64;
    for y in 0..N {
        for x in 0..N {
            let p = rgb(&a, x, y);
            assert_eq!(p[0], p[1]);
            assert_eq!(p[1], p[2]);
            assert!((p[0] as i32 - 128).abs() <= bound, "{p:?}");
            assert_eq!(alpha(&a, x, y), 255);
            sum += p[0] as i64 - 128;
        }
    }
    assert!(
        sum.abs() < (N * N) as i64 * 2,
        "mean should stay near zero: {sum}"
    );
    // It actually varies, and scales with strength.
    let spread = |pm: &Pixmap| {
        let v: Vec<u8> = pm.data().chunks_exact(4).map(|p| p[0]).collect();
        *v.iter().max().expect("px") as i32 - *v.iter().min().expect("px") as i32
    };
    let full = spread(&a);
    let half = spread(&run(&src, &kind(1), 0.5, 0.0));
    assert!(full > 20 && half < full, "{full} {half}");
}

// --- vignette ---------------------------------------------------------------

#[test]
fn vignette_darkens_corners_but_not_the_centre() {
    let src = solid(N, N, [200, 180, 160]);
    let out = run(&src, &PostKind::Vignette { amount: 1.0 }, 1.0, 0.0);
    assert_eq!(rgb(&out, N / 2, N / 2), [200, 180, 160], "centre untouched");
    assert_eq!(
        rgb(&out, N / 2, 20),
        rgb(&src, N / 2, 20),
        "inside r < 0.45"
    );
    let corner = rgb(&out, 0, 0);
    assert!(corner[0] < 10, "corner nearly black: {corner:?}");
    // Monotone darkening toward the corner along the diagonal.
    let diag: Vec<u8> = (0..N / 2).map(|i| rgb(&out, i, i)[0]).collect();
    assert!(diag.windows(2).all(|w| w[0] <= w[1]), "{diag:?}");
    // Symmetric corners, alpha untouched.
    assert_eq!(rgb(&out, 0, 0), rgb(&out, N - 1, N - 1));
    assert_eq!(rgb(&out, N - 1, 0), rgb(&out, 0, N - 1));
    assert_eq!(alpha(&out, 0, 0), 255);
    // Amount and strength scale the darkening.
    let half = run(&src, &PostKind::Vignette { amount: 0.5 }, 1.0, 0.0);
    let half_s = run(&src, &PostKind::Vignette { amount: 1.0 }, 0.5, 0.0);
    assert!((rgb(&half, 0, 0)[0] as i32 - 100).abs() <= 3);
    assert_eq!(half.data(), half_s.data());
}
