//! GPU post effects vs the CPU reference (`postfx::apply_post`).
//!
//! Run with `cargo test -p motion-render --features gpu --test gpu_parity`.
//! Every test skips (prints and returns) when no GPU adapter is available.
//! The ignored probe prints per-effect timings at 1080x1920:
//! `cargo test --release -p motion-render --features gpu --test gpu_parity -- --ignored --nocapture`.
#![cfg(feature = "gpu")]

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use motion_core::scene::PostKind;
use motion_core::timeline::ResolvedPost;
use motion_render::gpu::GpuPost;
use motion_render::postfx::apply_post;
use resvg::tiny_skia::Pixmap;

/// Per-channel tolerance of one effect: max and mean absolute difference.
const MAX_DIFF: u8 = 3;
const MEAN_DIFF: f64 = 0.5;

/// One shared device for all tests (also exercises resource reuse across
/// canvas sizes). `None` when there is no adapter.
fn gpu() -> Option<MutexGuard<'static, GpuPost>> {
    static GPU: OnceLock<Option<Mutex<GpuPost>>> = OnceLock::new();
    let slot = GPU.get_or_init(|| match GpuPost::new() {
        Ok(g) => {
            println!("gpu adapter: {}", g.adapter_name());
            Some(Mutex::new(g))
        }
        Err(e) => {
            println!("gpu unavailable ({e}); skipping GPU parity tests");
            None
        }
    });
    slot.as_ref()
        .map(|m| m.lock().unwrap_or_else(|p| p.into_inner()))
}

// ---------------------------------------------------------------------------
// Test images
// ---------------------------------------------------------------------------

fn put(pm: &mut Pixmap, x: u32, y: u32, rgba: [u8; 4]) {
    let w = pm.width();
    let i = ((y * w + x) * 4) as usize;
    pm.data_mut()[i..i + 4].copy_from_slice(&rgba);
}

fn pixmap(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    for y in 0..h {
        for x in 0..w {
            put(&mut pm, x, y, f(x, y));
        }
    }
    pm
}

/// Smooth colour gradients, opaque.
fn gradient(w: u32, h: u32) -> Pixmap {
    pixmap(w, h, |x, y| {
        [
            (x * 255 / (w - 1).max(1)) as u8,
            (y * 255 / (h - 1).max(1)) as u8,
            ((x + y) * 255 / (w + h - 2).max(1)) as u8,
            255,
        ]
    })
}

/// Dark ground with hard-edged saturated blocks, a bright disc and stripes.
fn shapes(w: u32, h: u32) -> Pixmap {
    pixmap(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let (dx, dy) = (fx - 0.62, fy - 0.35);
        if fx > 0.1 && fx < 0.4 && fy > 0.1 && fy < 0.3 {
            [255, 255, 255, 255]
        } else if fx > 0.15 && fx < 0.5 && fy > 0.55 && fy < 0.8 {
            [230, 40, 30, 255]
        } else if dx * dx + dy * dy < 0.012 {
            [250, 230, 60, 255]
        } else if fy > 0.88 && (x / 8) % 2 == 0 {
            [40, 200, 90, 255]
        } else if fx > 0.8 && fy > 0.5 && fy < 0.75 {
            [30, 60, 240, 255]
        } else {
            [12, 14, 28, 255]
        }
    })
}

/// Thin antialiased strokes (letter-like glyph skeletons) on a dark ground.
fn strokes(w: u32, h: u32) -> Pixmap {
    // Segments in unit coordinates: a few capital letters.
    type Seg = ((f32, f32), (f32, f32));
    let segs: &[Seg] = &[
        ((0.10, 0.20), (0.10, 0.50)),
        ((0.10, 0.20), (0.24, 0.20)),
        ((0.10, 0.35), (0.22, 0.35)),
        ((0.34, 0.20), (0.34, 0.50)),
        ((0.34, 0.50), (0.46, 0.50)),
        ((0.58, 0.20), (0.70, 0.50)),
        ((0.70, 0.20), (0.58, 0.50)),
        ((0.15, 0.62), (0.85, 0.62)),
        ((0.15, 0.70), (0.85, 0.86)),
        ((0.50, 0.55), (0.50, 0.95)),
    ];
    pixmap(w, h, |x, y| {
        let (px, py) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        let mut cover = 0.0f32;
        for &((ax, ay), (bx, by)) in segs {
            let (vx, vy) = (bx - ax, by - ay);
            let t = (((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
            let (qx, qy) = (ax + vx * t - px, ay + vy * t - py);
            let d = (qx * qx + qy * qy).sqrt() * w as f32; // px
            cover = cover.max((1.5 - d).clamp(0.0, 1.0));
        }
        let v = |c: f32, bg: f32| (bg + (c - bg) * cover) as u8;
        [v(245.0, 8.0), v(240.0, 10.0), v(225.0, 16.0), 255]
    })
}

/// Semi-transparent premultiplied content (colour <= alpha everywhere).
fn translucent(w: u32, h: u32) -> Pixmap {
    pixmap(w, h, |x, y| {
        let a = (40 + (x * 215 / (w - 1).max(1))) as u8;
        let c = |k: u32| ((x * k + y * 7) % 256 * u32::from(a) / 255) as u8;
        [c(3), c(5), c(2), a]
    })
}

/// Hash noise, opaque.
fn noisy(w: u32, h: u32) -> Pixmap {
    pixmap(w, h, |x, y| {
        let v = motion_core::noise::hash(99, y * w + x);
        [v as u8, (v >> 8) as u8, (v >> 16) as u8, 255]
    })
}

fn images(w: u32, h: u32) -> Vec<(&'static str, Pixmap)> {
    vec![
        ("gradient", gradient(w, h)),
        ("shapes", shapes(w, h)),
        ("strokes", strokes(w, h)),
        ("translucent", translucent(w, h)),
        ("noisy", noisy(w, h)),
    ]
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct Diff {
    max: u8,
    mean: f64,
}

fn diff(a: &Pixmap, b: &Pixmap) -> Diff {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let (mut max, mut sum) = (0u8, 0u64);
    for (x, y) in a.data().iter().zip(b.data()) {
        let d = x.abs_diff(*y);
        max = max.max(d);
        sum += u64::from(d);
    }
    Diff {
        max,
        mean: sum as f64 / a.data().len() as f64,
    }
}

fn post(kind: &PostKind, strength: f32, time: f64) -> ResolvedPost<'_> {
    ResolvedPost {
        kind,
        strength,
        time,
    }
}

fn cpu(pm: &Pixmap, posts: &[ResolvedPost<'_>]) -> Pixmap {
    let mut out = pm.clone();
    apply_post(&mut out, posts);
    out
}

fn on_gpu(g: &mut GpuPost, pm: &Pixmap, posts: &[ResolvedPost<'_>]) -> Pixmap {
    let mut out = pm.clone();
    g.apply(&mut out, posts).expect("gpu apply");
    out
}

/// Every effect (name, kind, strength, time) the parity suite covers.
fn cases() -> Vec<(String, PostKind, f32, f64)> {
    let mut v: Vec<(String, PostKind, f32, f64)> = Vec::new();
    let mut add = |name: &str, kind: PostKind, strength: f32, time: f64| {
        v.push((format!("{name} s={strength}"), kind, strength, time));
    };
    for s in [1.0, 0.6] {
        add(
            "bloom r=24 t=0.5",
            PostKind::Bloom {
                threshold: 0.5,
                radius: 24.0,
            },
            s,
            0.0,
        );
    }
    add(
        "bloom r=100 t=0.2",
        PostKind::Bloom {
            threshold: 0.2,
            radius: 100.0,
        },
        1.0,
        0.0,
    );
    add(
        "bloom r=6 t=0.7",
        PostKind::Bloom {
            threshold: 0.7,
            radius: 6.0,
        },
        1.0,
        0.0,
    );
    add(
        "bloom r=300 t=0",
        PostKind::Bloom {
            threshold: 0.0,
            radius: 300.0,
        },
        0.8,
        0.0,
    );
    for s in [1.0, 0.5] {
        add(
            "chromatic 6px @20deg",
            PostKind::ChromaticAberration {
                shift: 6.0,
                angle_deg: 20.0,
            },
            s,
            0.0,
        );
    }
    add(
        "chromatic 5px @0deg",
        PostKind::ChromaticAberration {
            shift: 5.0,
            angle_deg: 0.0,
        },
        1.0,
        0.0,
    );
    add(
        "chromatic -3.5px @90deg",
        PostKind::ChromaticAberration {
            shift: -3.5,
            angle_deg: 90.0,
        },
        1.0,
        0.0,
    );
    add(
        "chromatic 2px @225deg",
        PostKind::ChromaticAberration {
            shift: 2.0,
            angle_deg: 225.0,
        },
        1.0,
        0.0,
    );
    add(
        "chromatic 600px (past the canvas)",
        PostKind::ChromaticAberration {
            shift: 600.0,
            angle_deg: 33.0,
        },
        1.0,
        0.0,
    );
    for (bands, shift, seed, time) in [
        (12, 40.0, 1, 0.3),
        (6, 90.0, 7, 1.7),
        (40, 25.0, 3, 0.05),
        (3, 8.0, 99, 12.4),
    ] {
        add(
            &format!("glitch {bands} bands {shift}px seed {seed} t={time}"),
            PostKind::Glitch {
                bands,
                max_shift: shift,
                seed,
            },
            1.0,
            time,
        );
    }
    add(
        "glitch 12 bands 40px half strength",
        PostKind::Glitch {
            bands: 12,
            max_shift: 40.0,
            seed: 5,
        },
        0.5,
        0.9,
    );
    for s in [1.0, 0.4] {
        add(
            "rays centre (128,90) len 0.5",
            PostKind::Rays {
                center: [128.0, 90.0],
                length: 0.5,
            },
            s,
            0.0,
        );
    }
    add(
        "rays centre off-canvas len 0.9",
        PostKind::Rays {
            center: [300.0, -40.0],
            length: 0.9,
        },
        1.0,
        0.0,
    );
    for (angle, len) in [
        (0.0, 30.0),
        (90.0, 24.0),
        (30.0, 40.0),
        (135.0, 17.5),
        (200.0, 3.0),
        (60.0, 90.0),
    ] {
        add(
            &format!("dir blur {angle}deg {len}px"),
            PostKind::DirectionalBlur {
                angle_deg: angle,
                length: len,
            },
            1.0,
            0.0,
        );
    }
    add(
        "dir blur 30deg 40px half strength",
        PostKind::DirectionalBlur {
            angle_deg: 30.0,
            length: 40.0,
        },
        0.5,
        0.0,
    );
    for (amount, seed, time) in [(0.5, 1, 0.0), (1.0, 77, 2.3), (0.2, 4242, 10.01)] {
        add(
            &format!("grain {amount} seed {seed} t={time}"),
            PostKind::Grain { amount, seed },
            1.0,
            time,
        );
    }
    add(
        "grain 0.8 half strength",
        PostKind::Grain {
            amount: 0.8,
            seed: 3,
        },
        0.5,
        0.4,
    );
    for (amount, s) in [(0.8, 1.0), (1.0, 1.0), (0.5, 0.5)] {
        add(
            &format!("vignette {amount}"),
            PostKind::Vignette { amount },
            s,
            0.0,
        );
    }
    v
}

#[test]
fn every_effect_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let imgs = images(256, 256);
    let mut worst: Vec<(String, Diff)> = Vec::new();
    for (name, kind, strength, time) in cases() {
        let mut eff = Diff::default();
        let mut active = false;
        for (img_name, img) in &imgs {
            let posts = [post(&kind, strength, time)];
            let want = cpu(img, &posts);
            let got = on_gpu(&mut g, img, &posts);
            active |= want.data() != img.data();
            let d = diff(&want, &got);
            println!(
                "{name:<52} {img_name:<12} max {:>2}  mean {:.4}",
                d.max, d.mean
            );
            assert!(
                d.max <= MAX_DIFF && d.mean <= MEAN_DIFF,
                "{name} on {img_name}: max {} mean {:.4}",
                d.max,
                d.mean
            );
            eff.max = eff.max.max(d.max);
            eff.mean = eff.mean.max(d.mean);
        }
        // A case that changes nothing would pass trivially.
        assert!(active, "{name} left every test image unchanged");
        worst.push((name, eff));
    }
    println!("--- worst case per effect (over {} images) ---", imgs.len());
    for (name, d) in worst {
        println!("{name:<52} max {:>2}  mean {:.4}", d.max, d.mean);
    }
}

#[test]
fn glitch_and_grain_make_the_same_random_choices() {
    let Some(mut g) = gpu() else { return };
    // On a ramp every row/pixel carries a distinct value, so a different band
    // position, magnitude, sign or noise value would show as a large error.
    // Rounding aside (<= 3), the two backends must agree everywhere.
    let ramp = pixmap(256, 256, |x, y| [x as u8, (x * 3) as u8, y as u8, 255]);
    for seed in 0..24u32 {
        let time = seed as f64 * 0.37;
        let glitch = PostKind::Glitch {
            bands: 20,
            max_shift: 70.0,
            seed,
        };
        let grain = PostKind::Grain { amount: 1.0, seed };
        for kind in [&glitch, &grain] {
            let posts = [post(kind, 1.0, time)];
            let d = diff(&cpu(&ramp, &posts), &on_gpu(&mut g, &ramp, &posts));
            assert!(
                d.max <= 3,
                "{kind:?} seed {seed} t={time}: max {} mean {:.4}",
                d.max,
                d.mean
            );
        }
    }
    // The noise really is the shared hash: GPU grain differs between seeds and
    // between time buckets, and is identical within a bucket.
    let flat = pixmap(64, 64, |_, _| [128, 128, 128, 255]);
    let kind = PostKind::Grain {
        amount: 1.0,
        seed: 9,
    };
    let a = on_gpu(&mut g, &flat, &[post(&kind, 1.0, 0.0)]);
    let b = on_gpu(&mut g, &flat, &[post(&kind, 1.0, 0.03)]); // same 1/24 s bucket
    let c = on_gpu(&mut g, &flat, &[post(&kind, 1.0, 0.5)]);
    assert_eq!(a.data(), b.data());
    assert_ne!(a.data(), c.data());
}

#[test]
fn strength_zero_is_identity() {
    let Some(mut g) = gpu() else { return };
    let imgs = images(97, 61);
    for (name, kind, _, time) in cases() {
        for strength in [0.0f32, -0.5, f32::NAN] {
            for (img_name, img) in &imgs {
                let got = on_gpu(&mut g, img, &[post(&kind, strength, time)]);
                assert_eq!(
                    got.data(),
                    img.data(),
                    "{name} at strength {strength} changed {img_name}"
                );
            }
        }
    }
    // An empty list is a no-op too.
    let img = &imgs[0].1;
    assert_eq!(on_gpu(&mut g, img, &[]).data(), img.data());
}

#[test]
fn strength_is_clamped_to_one() {
    let Some(mut g) = gpu() else { return };
    let img = shapes(128, 96);
    let kind = PostKind::Bloom {
        threshold: 0.3,
        radius: 20.0,
    };
    let one = on_gpu(&mut g, &img, &[post(&kind, 1.0, 0.0)]);
    let two = on_gpu(&mut g, &img, &[post(&kind, 2.0, 0.0)]);
    assert_eq!(one.data(), two.data());
    assert_ne!(one.data(), img.data());
}

#[test]
fn odd_sizes_and_resource_reuse() {
    let Some(mut g) = gpu() else { return };
    // Sizes that are not multiples of the workgroup, tiny canvases (bloom
    // pyramid ends early) and size changes between frames (buffers rebuilt).
    let sizes = [
        (97, 61),
        (1, 1),
        (2, 3),
        (33, 17),
        (256, 256),
        (97, 61),
        (5, 300),
    ];
    let kinds = [
        PostKind::Bloom {
            threshold: 0.3,
            radius: 40.0,
        },
        PostKind::ChromaticAberration {
            shift: 4.0,
            angle_deg: 40.0,
        },
        PostKind::Glitch {
            bands: 9,
            max_shift: 30.0,
            seed: 2,
        },
        PostKind::Rays {
            center: [20.0, 30.0],
            length: 0.6,
        },
        PostKind::DirectionalBlur {
            angle_deg: 25.0,
            length: 12.0,
        },
        PostKind::Grain {
            amount: 0.7,
            seed: 6,
        },
        PostKind::Vignette { amount: 0.9 },
    ];
    for (w, h) in sizes {
        let img = shapes(w, h);
        for kind in &kinds {
            let posts = [post(kind, 1.0, 0.4)];
            let d = diff(&cpu(&img, &posts), &on_gpu(&mut g, &img, &posts));
            assert!(
                d.max <= MAX_DIFF && d.mean <= MEAN_DIFF,
                "{kind:?} at {w}x{h}: max {} mean {:.4}",
                d.max,
                d.mean
            );
        }
    }
}

#[test]
fn a_chain_in_list_order_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let kinds = [
        PostKind::Bloom {
            threshold: 0.45,
            radius: 32.0,
        },
        PostKind::ChromaticAberration {
            shift: 4.0,
            angle_deg: 15.0,
        },
        PostKind::Glitch {
            bands: 10,
            max_shift: 30.0,
            seed: 11,
        },
        PostKind::Rays {
            center: [128.0, 100.0],
            length: 0.4,
        },
        PostKind::DirectionalBlur {
            angle_deg: 10.0,
            length: 14.0,
        },
        PostKind::Grain {
            amount: 0.4,
            seed: 8,
        },
        PostKind::Vignette { amount: 0.7 },
    ];
    let posts: Vec<ResolvedPost<'_>> = kinds.iter().map(|k| post(k, 0.8, 0.77)).collect();
    // The u8 quantisation between effects is the same on both sides, so a
    // seven-effect chain stays within a few levels; it need not be bit-equal.
    for (name, img) in images(256, 256) {
        let d = diff(&cpu(&img, &posts), &on_gpu(&mut g, &img, &posts));
        println!(
            "chain of 7 on {name:<12} max {:>2} mean {:.4}",
            d.max, d.mean
        );
        assert!(d.max <= 6 && d.mean <= 0.6, "chain on {name}: {d:?}");
    }
    // Order matters on both sides: reversed order also matches the CPU.
    let rev: Vec<ResolvedPost<'_>> = kinds.iter().rev().map(|k| post(k, 0.8, 0.77)).collect();
    for (name, img) in images(256, 256) {
        let d = diff(&cpu(&img, &rev), &on_gpu(&mut g, &img, &rev));
        println!(
            "reversed chain on {name:<12} max {:>2} mean {:.4}",
            d.max, d.mean
        );
        assert!(
            d.max <= 6 && d.mean <= 0.6,
            "reversed chain on {name}: {d:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Renderer integration
// ---------------------------------------------------------------------------

fn repo_root() -> &'static std::path::Path {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// A 600x300 text frame on black with three persistent post effects.
fn post_project() -> motion_core::MotionProject {
    let json = r##"{
        "version": "0.2",
        "project": { "name": "gpu_post", "duration_seconds": 1.0 },
        "canvas": { "width": 600, "height": 300, "fps": 30, "background": "#05060A" },
        "theme": { "fonts": { "number": "font.anton", "body": "font.fira" } },
        "assets": [
            { "id": "font.anton", "type": "font", "path": "fonts/Anton-Regular.ttf" },
            { "id": "font.fira", "type": "font", "path": "fonts/FiraSans-Regular.ttf" }
        ],
        "asset_root": "assets",
        "scenes": [ { "id": "s", "start_seconds": 0.0, "duration_seconds": 1.0,
            "layers": [ { "id": "t", "type": "text", "text": "GLOW", "font_role": "number",
                "font_size": 150, "color": "#FFFFFF", "x": 20, "y": 10,
                "width": 560, "height": 280 } ],
            "post": [
                { "effect": "bloom", "threshold": 0.4, "radius": 30.0,
                  "start": 0.0, "duration": 0.0, "from": 0.9, "to": 0.9 },
                { "effect": "chromatic_aberration", "shift": 5.0, "angle_deg": 10.0,
                  "start": 0.0, "duration": 0.0, "from": 1.0, "to": 1.0 },
                { "effect": "grain", "amount": 0.4, "seed": 5,
                  "start": 0.0, "duration": 0.0, "from": 1.0, "to": 1.0 },
                { "effect": "vignette", "amount": 0.7,
                  "start": 0.0, "duration": 0.0, "from": 1.0, "to": 1.0 }
            ] } ]
    }"##;
    motion_core::MotionProject::from_json(json).expect("fixture parses")
}

#[test]
fn renderer_with_gpu_post_matches_the_cpu_renderer() {
    use motion_render::{CpuRenderer, Renderer};
    let Ok(gpu_post) = GpuPost::new() else {
        println!("gpu unavailable; skipping");
        return;
    };
    let p = post_project();
    let cpu_r = CpuRenderer::new(&p, repo_root()).expect("renderer");
    let gpu_r = CpuRenderer::new(&p, repo_root())
        .expect("renderer")
        .with_gpu_post(gpu_post);
    for frame in [0, 7, 21] {
        let f = motion_core::evaluate_frame(&p, frame).expect("evaluate");
        assert_eq!(f.post.len(), 4, "the scene's post effects are active");
        let a = cpu_r.render(&f).expect("cpu render");
        let b = gpu_r.render(&f).expect("gpu render");
        let d = diff(&a, &b);
        println!("frame {frame}: max {} mean {:.4}", d.max, d.mean);
        assert!(
            d.max <= MAX_DIFF && d.mean <= MEAN_DIFF,
            "frame {frame}: {d:?}"
        );
        // The post stage really ran on the frame (it is not the bare composite).
        let mut bare = f.clone();
        bare.post.clear();
        assert_ne!(cpu_r.render(&bare).expect("bare").data(), a.data());
        // Without post effects both renderers are byte-identical.
        assert_eq!(
            cpu_r.render(&bare).expect("bare").data(),
            gpu_r.render(&bare).expect("bare").data()
        );
    }
    assert_eq!(gpu_r.gpu_post_failure(), None);
}

#[test]
fn render_frames_with_gpu_post_matches_the_default_cpu_path() {
    use motion_render::export::{render_frames_with, FrameRange, RenderOptions};
    if GpuPost::new().is_err() {
        println!("gpu unavailable; skipping");
        return;
    }
    let p = post_project();
    let root = std::env::temp_dir().join(format!("motion_gpu_parity_{}", std::process::id()));
    let (cpu_dir, gpu_dir) = (root.join("cpu"), root.join("gpu"));
    let cpu_report = render_frames_with(
        &p,
        repo_root(),
        &cpu_dir,
        FrameRange::All,
        RenderOptions::default(),
    )
    .expect("cpu render");
    let gpu_report = render_frames_with(
        &p,
        repo_root(),
        &gpu_dir,
        FrameRange::All,
        RenderOptions { gpu_post: true },
    )
    .expect("gpu render");
    assert_eq!(cpu_report.gpu_notice, None, "default path is silent CPU");
    assert_eq!(gpu_report.gpu_notice, None, "the GPU worked throughout");
    assert_eq!(cpu_report.frames.len(), 30);
    assert_eq!(gpu_report.frames.len(), 30);
    let mut worst = Diff::default();
    for (a, b) in cpu_report.frames.iter().zip(&gpu_report.frames) {
        let d = diff(
            &Pixmap::load_png(a).expect("cpu png"),
            &Pixmap::load_png(b).expect("gpu png"),
        );
        worst.max = worst.max.max(d.max);
        worst.mean = worst.mean.max(d.mean);
    }
    println!("30 frames via render_frames_with: worst {worst:?}");
    assert!(
        worst.max <= MAX_DIFF && worst.mean <= MEAN_DIFF,
        "{worst:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Timing probe (ignored: machine dependent)
// ---------------------------------------------------------------------------

const W: u32 = 1080;
const H: u32 = 1920;

/// A busy, opaque frame: gradient, bright blocks and hash noise.
fn busy_frame() -> Pixmap {
    pixmap(W, H, |x, y| {
        let n = motion_core::noise::hash(5, y * W + x) as u8 / 16;
        let block = ((x / 90 + y / 90) % 5 == 0) as u8 * 200;
        [
            (x * 255 / W) as u8 / 2 + block / 2 + n,
            (y * 255 / H) as u8 / 2 + block / 2 + n,
            60 + block / 2 + n,
            255,
        ]
    })
}

fn median(mut f: impl FnMut()) -> f64 {
    f(); // warm up (pipeline compile, buffers)
    let mut t: Vec<f64> = (0..7)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[3]
}

#[test]
#[ignore]
fn timings_1080x1920_gpu_vs_cpu() {
    let Some(mut g) = gpu() else {
        println!("no GPU adapter");
        return;
    };
    let base = busy_frame();
    println!(
        "adapter: {}   cpu threads: {}",
        g.adapter_name(),
        rayon::current_num_threads()
    );
    let kinds: Vec<(&str, PostKind)> = vec![
        (
            "bloom r=24",
            PostKind::Bloom {
                threshold: 0.5,
                radius: 24.0,
            },
        ),
        (
            "bloom r=100",
            PostKind::Bloom {
                threshold: 0.5,
                radius: 100.0,
            },
        ),
        (
            "chromatic 6px @20deg",
            PostKind::ChromaticAberration {
                shift: 6.0,
                angle_deg: 20.0,
            },
        ),
        (
            "glitch 12 bands",
            PostKind::Glitch {
                bands: 12,
                max_shift: 60.0,
                seed: 1,
            },
        ),
        (
            "rays len 0.5",
            PostKind::Rays {
                center: [540.0, 700.0],
                length: 0.5,
            },
        ),
        (
            "dir blur 0deg 60px",
            PostKind::DirectionalBlur {
                angle_deg: 0.0,
                length: 60.0,
            },
        ),
        (
            "dir blur 30deg 60px",
            PostKind::DirectionalBlur {
                angle_deg: 30.0,
                length: 60.0,
            },
        ),
        (
            "dir blur 30deg 200px",
            PostKind::DirectionalBlur {
                angle_deg: 30.0,
                length: 200.0,
            },
        ),
        (
            "grain",
            PostKind::Grain {
                amount: 0.5,
                seed: 1,
            },
        ),
        ("vignette", PostKind::Vignette { amount: 0.8 }),
    ];

    let rt = median(|| {
        let mut pm = base.clone();
        g.roundtrip(&mut pm).expect("roundtrip");
    });
    let clone_ms = median(|| {
        std::hint::black_box(base.clone());
    });
    println!("(upload + readback with no effect: {rt:.1} ms; frame clone alone: {clone_ms:.1} ms; both included below)");
    println!(
        "{:<24} {:>9} {:>9} {:>8}   gpu split: {:>7} {:>8} {:>9}   {:>9}",
        "effect", "cpu ms", "gpu ms", "speedup", "setup", "compute", "readback", "max diff"
    );
    let mut all: Vec<ResolvedPost<'_>> = Vec::new();
    for (name, kind) in &kinds {
        let posts = [post(kind, 1.0, 0.3)];
        let cpu_ms = median(|| {
            let mut pm = base.clone();
            apply_post(&mut pm, &posts);
        });
        let gpu_ms = median(|| {
            let mut pm = base.clone();
            g.apply(&mut pm, &posts).expect("gpu apply");
        });
        let t = g.last_timings();
        let d = diff(&cpu(&base, &posts), &on_gpu(&mut g, &base, &posts));
        println!(
            "{name:<24} {cpu_ms:>9.1} {gpu_ms:>9.1} {:>7.1}x   gpu split: {:>7.1} {:>8.1} {:>9.1}   {:>9}",
            cpu_ms / gpu_ms,
            t.upload_ms,
            t.compute_ms,
            t.readback_ms,
            d.max
        );
    }
    // The whole stack in one go: what a busy frame actually pays.
    for (_, kind) in &kinds {
        all.push(post(kind, 1.0, 0.3));
    }
    let cpu_ms = median(|| {
        let mut pm = base.clone();
        apply_post(&mut pm, &all);
    });
    let gpu_ms = median(|| {
        let mut pm = base.clone();
        g.apply(&mut pm, &all).expect("gpu apply");
    });
    println!(
        "{:<24} {cpu_ms:>9.1} {gpu_ms:>9.1} {:>7.1}x",
        "all 10 effects in one frame",
        cpu_ms / gpu_ms
    );
}
