//! Timing probe for the post effects and layer warps at 1080x1920.
//!
//! Ignored by default (timings are machine dependent). Run in release:
//! `cargo test --release -p motion-render --test postfx_timing -- --ignored --nocapture`
//! (add `RAYON_NUM_THREADS=1` for single-thread numbers).

use std::time::Instant;

use motion_core::scene::PostKind;
use motion_core::timeline::ResolvedPost;
use motion_render::blur::gaussian_blur;
use motion_render::postfx::apply_post;
use motion_render::warp::warp_over;
use resvg::tiny_skia::Pixmap;

const W: u32 = 1080;
const H: u32 = 1920;

/// A busy, opaque frame: gradient, bright blocks and hash noise.
fn frame() -> Pixmap {
    let mut pm = Pixmap::new(W, H).expect("pixmap");
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let n = motion_core::noise::hash(5, y * W + x) as u8 / 16;
            let block = ((x / 90 + y / 90) % 5 == 0) as u8 * 200;
            let d = pm.data_mut();
            d[i] = (x * 255 / W) as u8 / 2 + block / 2 + n;
            d[i + 1] = (y * 255 / H) as u8 / 2 + block / 2 + n;
            d[i + 2] = 60 + block / 2 + n;
            d[i + 3] = 255;
        }
    }
    pm
}

fn median_ms(mut f: impl FnMut()) -> f64 {
    f(); // warm up
    let mut t: Vec<f64> = (0..5)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[2]
}

#[test]
#[ignore]
fn timings_1080x1920() {
    let base = frame();
    let threads = rayon::current_num_threads();
    println!("threads: {threads}");
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
    for (name, kind) in &kinds {
        let ms = median_ms(|| {
            let mut pm = base.clone();
            apply_post(
                &mut pm,
                &[ResolvedPost {
                    kind,
                    strength: 1.0,
                    time: 0.3,
                }],
            );
        });
        println!("{name:<24} {ms:7.1} ms");
    }
    let clone_ms = median_ms(|| {
        let _ = base.clone();
    });
    println!("(frame clone alone      {clone_ms:7.1} ms, included above)");
    for sigma in [4.0f32, 12.0] {
        let ms = median_ms(|| {
            let mut pm = base.clone();
            gaussian_blur(&mut pm, sigma);
        });
        println!("layer blur sigma={sigma:<5}    {ms:7.1} ms (full frame)");
    }
    // A tilted, full-canvas layer warped onto the canvas.
    let h = [0.9, 0.05, 40.0, 0.02, 0.8, 80.0, 0.0001, 0.0002, 1.0];
    let m: [f64; 9] = h.map(|v: f64| v);
    let mut dst = base.clone();
    let ms = median_ms(|| {
        warp_over(&mut dst, &base, &m);
    });
    println!("warp full-canvas quad   {ms:7.1} ms");
}
