//! Image treatments (0.5): `treatment::apply` operations and the CPU renderer's
//! per-layer treated images. Spec: docs/IMAGE_TREATMENTS.md.
//!
//! Visual goldens (`duotone_asset`, `paper_cutout_asset`) live in
//! `golden/treatments/` with FNV-1a hashes keyed `"{name}@{os}-{arch}"`;
//! `MOTION_UPDATE_GOLDEN=1` rewrites the PNGs and this platform's hashes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use motion_core::scene::{
    Color, ContactShadow, Duotone, ImageTreatment, PaperEdge, Sticker, TintSpec, TreatmentPreset,
};
use motion_core::{evaluate_frame, MotionProject};
use motion_render::treatment::apply;
use motion_render::{CpuRenderer, Renderer};
use resvg::tiny_skia::Pixmap;

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Build a pixmap from straight-alpha RGBA.
fn pixmap(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    let data = pm.data_mut();
    for y in 0..h {
        for x in 0..w {
            let [r, g, b, a] = f(x, y);
            let i = ((y * w + x) * 4) as usize;
            let pre = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
            data[i] = pre(r);
            data[i + 1] = pre(g);
            data[i + 2] = pre(b);
            data[i + 3] = a;
        }
    }
    pm
}

fn alpha_at(pm: &Pixmap, x: u32, y: u32) -> u8 {
    pm.pixel(x, y).expect("in range").alpha()
}

/// Opaque-or-transparent disc centred in a `size x size` image.
fn disc(size: u32, radius: f32, rgb: [u8; 3]) -> Pixmap {
    let c = size as f32 / 2.0;
    pixmap(size, size, |x, y| {
        let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
        if d <= radius {
            [rgb[0], rgb[1], rgb[2], 255]
        } else {
            [0, 0, 0, 0]
        }
    })
}

fn natural() -> ImageTreatment {
    ImageTreatment::natural()
}

fn colourful() -> Pixmap {
    pixmap(64, 48, |x, y| {
        [
            (x * 4) as u8,
            (y * 5) as u8,
            (255 - x * 3) as u8,
            if (x + y) % 5 == 0 { 120 } else { 255 },
        ]
    })
}

// ---------------------------------------------------------------------------
// apply(): colour operations
// ---------------------------------------------------------------------------

#[test]
fn natural_returns_identical_pixels_and_no_pad() {
    let img = colourful();
    let (out, pad) = apply(&img, &natural(), 1.0);
    assert_eq!(pad, 0);
    assert_eq!((out.width(), out.height()), (img.width(), img.height()));
    assert_eq!(out.data(), img.data());

    // Preset / seed alone do not change pixels.
    let mut t = natural();
    t.preset = TreatmentPreset::PaperCutout;
    t.seed = 99;
    let (out, pad) = apply(&img, &t, 2.5);
    assert_eq!(pad, 0);
    assert_eq!(out.data(), img.data());
}

#[test]
fn desaturate_one_makes_opaque_pixels_grey() {
    let img = pixmap(32, 32, |x, y| [(x * 8) as u8, (y * 8) as u8, 200, 255]);
    let mut t = natural();
    t.desaturate = 1.0;
    let (out, pad) = apply(&img, &t, 1.0);
    assert_eq!(pad, 0);
    assert_ne!(out.data(), img.data());
    for px in out.pixels() {
        assert_eq!((px.red(), px.green()), (px.green(), px.blue()));
    }
}

#[test]
fn colour_ops_never_change_alpha() {
    let img = colourful();
    let mut t = natural();
    t.desaturate = 0.4;
    t.brightness = 0.1;
    t.contrast = 1.3;
    t.duotone = Some(Duotone {
        shadow: Color::rgb(20, 20, 40),
        highlight: Color::rgb(240, 230, 200),
        amount: 0.7,
    });
    t.tint = Some(TintSpec {
        color: Color::rgb(230, 220, 200),
        amount: 0.2,
    });
    t.grain = 0.3;
    t.seed = 5;
    let (out, pad) = apply(&img, &t, 1.0);
    assert_eq!(pad, 0);
    for (a, b) in img.pixels().iter().zip(out.pixels()) {
        assert_eq!(a.alpha(), b.alpha());
        // Premultiplied invariant.
        assert!(b.red() <= b.alpha() && b.green() <= b.alpha() && b.blue() <= b.alpha());
    }
    assert_ne!(out.data(), img.data());
}

#[test]
fn duotone_maps_black_and_white_to_shadow_and_highlight() {
    let img = pixmap(256, 1, |x, _| [x as u8, x as u8, x as u8, 255]);
    let (shadow, highlight) = (Color::rgb(30, 10, 60), Color::rgb(240, 220, 150));
    let mut t = natural();
    t.duotone = Some(Duotone {
        shadow,
        highlight,
        amount: 1.0,
    });
    let (out, _) = apply(&img, &t, 1.0);
    let close = |p: resvg::tiny_skia::PremultipliedColorU8, c: Color| {
        (i32::from(p.red()) - i32::from(c.r)).abs() <= 2
            && (i32::from(p.green()) - i32::from(c.g)).abs() <= 2
            && (i32::from(p.blue()) - i32::from(c.b)).abs() <= 2
    };
    assert!(close(out.pixel(0, 0).expect("px"), shadow));
    assert!(close(out.pixel(255, 0).expect("px"), highlight));
    // Monotone in between (luma of the mapped colour rises with the input).
    let mid = out.pixel(128, 0).expect("px");
    assert!(mid.red() > 30 && mid.red() < 240);
}

#[test]
fn grain_is_seeded_deterministic_and_mean_neutral() {
    let img = pixmap(128, 128, |_, _| [128, 128, 128, 255]);
    let with_seed = |seed| {
        let mut t = natural();
        t.grain = 0.5;
        t.seed = seed;
        apply(&img, &t, 1.0).0
    };
    let a = with_seed(1);
    let b = with_seed(1);
    let c = with_seed(2);
    assert_eq!(a.data(), b.data());
    assert_ne!(a.data(), c.data());
    assert_ne!(a.data(), img.data());
    let mean = a.pixels().iter().map(|p| f64::from(p.red())).sum::<f64>() / (128.0 * 128.0);
    assert!((mean - 128.0).abs() < 1.0, "mean {mean}");
    // The same noise on R, G and B (monochrome grain).
    assert!(a
        .pixels()
        .iter()
        .all(|p| p.red() == p.green() && p.green() == p.blue()));
}

// ---------------------------------------------------------------------------
// apply(): silhouette operations
// ---------------------------------------------------------------------------

#[test]
fn paper_edge_grows_the_silhouette_and_keeps_the_image() {
    let (size, radius, width, ppu) = (128u32, 40.0f32, 6.0f32, 1.5f32);
    let img = disc(size, radius, [30, 90, 200]);
    let mut t = natural();
    t.edge = Some(PaperEdge {
        color: Color::rgb(250, 240, 220),
        width,
    });
    let (out, pad) = apply(&img, &t, ppu);
    assert_eq!(pad, (width * ppu).ceil() as u32 + 2);
    assert_eq!(out.width(), size + 2 * pad);
    assert_eq!(out.height(), size + 2 * pad);

    // Along the horizontal centre line the outermost solid pixel grew by ~ width * ppu.
    let cy = pad + size / 2;
    let solid = |pm: &Pixmap, y: u32| {
        (0..pm.width())
            .filter(|&x| alpha_at(pm, x, y) >= 128)
            .max()
            .expect("solid pixel")
    };
    let before = solid(&img, size / 2) as f32;
    let after = solid(&out, cy) as f32 - pad as f32;
    let grown = after - before;
    assert!(
        (grown - width * ppu).abs() <= 1.5,
        "grew {grown}, expected ~{}",
        width * ppu
    );

    // Original opaque pixels are unchanged, offset by pad.
    for y in 0..size {
        for x in 0..size {
            if alpha_at(&img, x, y) == 255 {
                assert_eq!(out.pixel(x + pad, y + pad), img.pixel(x, y), "({x},{y})");
            }
        }
    }
    // The ring just outside the disc is the edge colour.
    let ring = out
        .pixel(pad + size / 2 + radius as u32 + 3, cy)
        .expect("px");
    assert_eq!(
        (ring.red(), ring.green(), ring.blue(), ring.alpha()),
        (250, 240, 220, 255)
    );
    // Far corner is still empty.
    assert_eq!(alpha_at(&out, 0, 0), 0);
}

#[test]
fn shadow_falls_below_right_and_pad_follows_the_formula() {
    let (size, radius, ppu) = (128u32, 30.0f32, 1.5f32);
    let img = disc(size, radius, [30, 90, 200]);
    let mut t = natural();
    t.edge = Some(PaperEdge {
        color: Color::rgb(255, 255, 255),
        width: 4.0,
    });
    t.shadow = Some(ContactShadow {
        color: Color::rgb(10, 10, 10).with_alpha(0xC0),
        offset: [3.0, 4.0],
        blur: 8.0,
    });
    let (out, pad) = apply(&img, &t, ppu);
    // max(edge 4, |(3,4)| + blur 8 = 13) * 1.5 = 19.5 -> 20, + 2.
    assert_eq!(pad, 22);
    assert_eq!(out.width(), size + 2 * pad);

    let c = (pad + size / 2) as f32;
    let d = (radius + 4.0 * ppu + 4.0) / 2f32.sqrt();
    let below_right = alpha_at(&out, (c + d) as u32, (c + d) as u32);
    let above_left = alpha_at(&out, (c - d) as u32, (c - d) as u32);
    assert!(below_right > 0, "shadow missing below-right");
    assert!(
        below_right > above_left,
        "shadow ({below_right}) should be heavier below-right than above-left ({above_left})"
    );
    // The shadow colour is dark: the pixel is not the white edge.
    let p = out.pixel((c + d) as u32, (c + d) as u32).expect("px");
    assert!(p.red() < 60, "shadow pixel {p:?}");
    // The image stays on top, unchanged.
    assert_eq!(
        out.pixel(pad + size / 2, pad + size / 2),
        img.pixel(size / 2, size / 2)
    );
}

#[test]
fn opaque_image_with_shadow_gets_a_drop_shadow_around_its_rectangle() {
    // `treatment_for` never sets shadow for opaque images, but `apply` handles
    // it uniformly: the alpha silhouette is the whole rectangle.
    let img = pixmap(32, 32, |_, _| [200, 50, 50, 255]);
    let mut t = natural();
    t.shadow = Some(ContactShadow {
        color: Color::rgb(0, 0, 0).with_alpha(0xFF),
        offset: [4.0, 4.0],
        blur: 4.0,
    });
    let (out, pad) = apply(&img, &t, 1.0);
    assert_eq!(
        pad,
        (8.0f32.hypot(0.0).max(4.0 * 2f32.sqrt() + 4.0)).ceil() as u32 + 2
    );
    // Image pixels unchanged, shifted by pad.
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(out.pixel(x + pad, y + pad), img.pixel(x, y));
        }
    }
    // Shadow peeks out right of / below the rectangle, not above / left of it.
    assert!(alpha_at(&out, pad + 34, pad + 20) > 0);
    assert!(alpha_at(&out, pad + 20, pad + 34) > 0);
    assert_eq!(alpha_at(&out, 0, 0), 0);
}

// ---------------------------------------------------------------------------
// (0.10 Q) Sticker: alpha grown by width_px, filled, soft 10 % shadow
// ---------------------------------------------------------------------------

fn sticker(color: Color, width_px: f32) -> ImageTreatment {
    let mut t = natural();
    t.sticker = Some(Sticker { color, width_px });
    t
}

#[test]
fn sticker_outlines_a_dark_cutout_in_paper_with_a_soft_shadow() {
    // A dark disc on a transparent ground: it would vanish on a dark page.
    let (size, radius, ppu) = (160u32, 40.0f32, 1.0f32);
    let img = disc(size, radius, [20, 18, 16]);
    let paper = Color::rgb(0xEC, 0xE3, 0xD2);
    let width = 8.0f32;
    let (out, pad) = apply(&img, &sticker(paper, width), ppu);

    // Pad covers the dilation and the shadow (offset 4 + blur 12 = 16 > 8).
    assert_eq!(pad, 16 + 2);
    assert_eq!(out.width(), size + 2 * pad);

    let c = pad + size / 2;
    // The subject itself is untouched, on top.
    assert_eq!(
        out.pixel(c, c),
        img.pixel(size / 2, size / 2),
        "interior unchanged"
    );
    // The ring 4 px outside the disc (inside the 8 px band) is solid paper.
    let ring = out.pixel(c + radius as u32 + 4, c).expect("px");
    assert_eq!(
        (ring.red(), ring.green(), ring.blue(), ring.alpha()),
        (paper.r, paper.g, paper.b, 255),
        "sticker band"
    );
    // The band ends near `width` px from the disc: 4 px past it is not paper.
    let past = out.pixel(c + radius as u32 + 8 + 4, c).expect("px");
    assert!(
        past.alpha() < 128,
        "outside the band the shadow is faint, got alpha {}",
        past.alpha()
    );
    // The shadow is a soft 10 % black: it never exceeds ~12 % opacity.
    let max_shadow = (0..out.height())
        .flat_map(|y| (0..out.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let (dx, dy) = (x as f32 - c as f32, y as f32 - c as f32);
            dx.hypot(dy) > radius + width + 1.5
        })
        .map(|(x, y)| alpha_at(&out, x, y))
        .max()
        .expect("pixels");
    assert!(
        (1..=32).contains(&max_shadow),
        "a faint shadow past the band: max alpha {max_shadow}"
    );
    // Far corners are empty.
    assert_eq!(alpha_at(&out, 0, 0), 0);
    assert_eq!(alpha_at(&out, out.width() - 1, out.height() - 1), 0);
}

#[test]
fn sticker_scales_with_px_per_unit_and_is_deterministic() {
    let img = disc(96, 24.0, [10, 10, 10]);
    let t = sticker(Color::rgb(255, 255, 255), 6.0);
    let (a, pa) = apply(&img, &t, 1.5);
    let (b, pb) = apply(&img, &t, 1.5);
    assert_eq!(pa, pb);
    assert_eq!(a.data(), b.data());
    // 6 px at 1.5 px/unit = 9 px band; shadow reach 1.5*(3 + 9) = 18 -> pad 20.
    assert_eq!(pa, 20);
    let c = pa + 48;
    let in_band = a.pixel(c + 24 + 7, c).expect("px");
    assert_eq!(
        (in_band.red(), in_band.alpha()),
        (255, 255),
        "7 px past the disc is inside the 9 px band"
    );
}

#[test]
fn sticker_replaces_edge_and_shadow() {
    let img = disc(96, 24.0, [10, 10, 10]);
    let mut t = sticker(Color::rgb(255, 255, 255), 6.0);
    // A paper-cutout style edge and shadow that the sticker must override.
    t.edge = Some(PaperEdge {
        color: Color::rgb(200, 0, 0),
        width: 20.0,
    });
    t.shadow = Some(ContactShadow {
        color: Color::rgb(0, 0, 0).with_alpha(0xFF),
        offset: [30.0, 30.0],
        blur: 30.0,
    });
    let (with_style, p1) = apply(&img, &t, 1.0);
    let (plain, p2) = apply(&img, &sticker(Color::rgb(255, 255, 255), 6.0), 1.0);
    assert_eq!(p1, p2);
    assert_eq!(with_style.data(), plain.data());
}

#[test]
fn degenerate_sticker_is_ignored() {
    let img = colourful();
    for width in [0.0f32, -3.0, f32::NAN, f32::INFINITY] {
        let (out, pad) = apply(&img, &sticker(Color::rgb(255, 255, 255), width), 1.0);
        assert_eq!(pad, 0, "width {width}");
        assert_eq!(out.data(), img.data(), "width {width}");
    }
}

#[test]
fn sticker_treatment_round_trips_through_json_and_old_scenes_parse() {
    let t = sticker(Color::rgb(0xEC, 0xE3, 0xD2), 8.0);
    let json = serde_json::to_string(&t).expect("serialize");
    assert!(json.contains("\"sticker\""), "{json}");
    assert!(json.contains("#ECE3D2"), "{json}");
    let back: ImageTreatment = serde_json::from_str(&json).expect("parse");
    assert_eq!(back, t);
    // Treatments without the field (every pre-0.10 scene) parse and do not
    // serialize one.
    let old = serde_json::to_string(&natural()).expect("serialize");
    assert!(!old.contains("sticker"), "{old}");
    let parsed: ImageTreatment = serde_json::from_str(&old).expect("parse");
    assert!(parsed.sticker.is_none());
}

#[test]
fn edge_and_shadow_are_deterministic() {
    let img = disc(64, 20.0, [1, 2, 3]);
    let mut t = natural();
    t.grain = 0.2;
    t.seed = 3;
    t.edge = Some(PaperEdge {
        color: Color::rgb(255, 255, 255),
        width: 3.0,
    });
    t.shadow = Some(ContactShadow {
        color: Color::rgb(0, 0, 0).with_alpha(0x60),
        offset: [2.0, 5.0],
        blur: 6.0,
    });
    let (a, pa) = apply(&img, &t, 1.3);
    let (b, pb) = apply(&img, &t, 1.3);
    assert_eq!(pa, pb);
    assert_eq!(a.data(), b.data());
}

#[test]
fn large_cutout_with_edge_and_shadow_is_fast() {
    let (w, h) = (1024u32, 1365u32);
    let img = pixmap(w, h, |x, y| {
        let (dx, dy) = (x as f32 - 512.0, y as f32 - 680.0);
        if (dx / 380.0).powi(2) + (dy / 600.0).powi(2) <= 1.0 {
            [180, 140, 110, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    let mut t = natural();
    t.desaturate = 0.18;
    t.grain = 0.04;
    t.edge = Some(PaperEdge {
        color: Color::rgb(246, 241, 230),
        width: 7.0,
    });
    t.shadow = Some(ContactShadow {
        color: Color::rgb(23, 21, 19).with_alpha(0x38),
        offset: [5.0, 9.0],
        blur: 14.0,
    });
    let start = Instant::now();
    let (out, pad) = apply(&img, &t, 1.5);
    let ms = start.elapsed().as_millis();
    assert!(pad > 0 && out.width() == w + 2 * pad);
    eprintln!("apply 1024x1365 edge+shadow: {ms} ms");
    if !cfg!(debug_assertions) {
        assert!(ms < 150, "took {ms} ms");
    }
}

// ---------------------------------------------------------------------------
// CpuRenderer integration
// ---------------------------------------------------------------------------

const FIGURE_BOX: (f32, f32, f32, f32) = (50.0, 50.0, 200.0, 300.0);

fn figure_project(extra: &str, fit: &str) -> MotionProject {
    let (x, y, w, h) = FIGURE_BOX;
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "treatment", "duration_seconds": 1.0 }},
        "canvas": {{ "width": 300, "height": 400, "fps": 30, "background": "#FFFFFF" }},
        "assets": [ {{ "id": "fig", "type": "image", "path": "assets/test_images/figure.png" }} ],
        "scenes": [ {{ "id": "s", "start_seconds": 0.0, "duration_seconds": 1.0, "layers": [
            {{ "id": "img", "type": "image", "asset": "fig", "fit": "{fit}",
               "x": {x}, "y": {y}, "width": {w}, "height": {h}{extra} }}
        ] }} ]
    }}"##
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

fn treatment_json(t: &ImageTreatment) -> String {
    format!(
        r#", "treatment": {}"#,
        serde_json::to_string(t).expect("serialize treatment")
    )
}

fn render0(p: &MotionProject) -> Pixmap {
    let r = CpuRenderer::new(p, repo_root()).expect("renderer");
    let f = evaluate_frame(p, 0).expect("evaluate");
    r.render(&f).expect("render")
}

fn paper_cutout(u: f32) -> ImageTreatment {
    let ink = Color::rgb(0x17, 0x15, 0x13);
    ImageTreatment {
        preset: TreatmentPreset::PaperCutout,
        desaturate: 0.18,
        brightness: 0.01,
        contrast: 1.06,
        tint: Some(TintSpec {
            color: Color::rgb(0xEC, 0xE3, 0xD2),
            amount: 0.05,
        }),
        grain: 0.04,
        seed: 7,
        edge: Some(PaperEdge {
            color: Color::rgb(0xF6, 0xF1, 0xE6),
            width: 7.0 * u,
        }),
        shadow: Some(ContactShadow {
            color: ink.with_alpha(0x38),
            offset: [5.0 * u, 9.0 * u],
            blur: 14.0 * u,
        }),
        ..natural()
    }
}

#[test]
fn renderer_natural_treatment_is_identical_to_untreated() {
    let plain = render0(&figure_project("", "contain"));
    let treated = render0(&figure_project(&treatment_json(&natural()), "contain"));
    assert_eq!(plain.data(), treated.data());
}

#[test]
fn renderer_paper_cutout_draws_outside_the_silhouette_and_keeps_the_content_in_place() {
    let plain = render0(&figure_project("", "contain"));
    let treated = render0(&figure_project(
        &treatment_json(&paper_cutout(1.0)),
        "contain",
    ));
    assert_ne!(plain.data(), treated.data());

    // Edge / shadow: pixels that are pure background without treatment but drawn with it.
    let white = |pm: &Pixmap, x: u32, y: u32| {
        let p = pm.pixel(x, y).expect("px");
        (p.red(), p.green(), p.blue()) == (255, 255, 255)
    };
    let mut new_outside = 0;
    for y in 0..400 {
        for x in 0..300 {
            if white(&plain, x, y) && !white(&treated, x, y) {
                new_outside += 1;
            }
        }
    }
    assert!(new_outside > 500, "only {new_outside} new pixels");

    // The image content is not shifted or rescaled: the centre stays close.
    let (cx, cy) = (150, 200);
    let (a, b) = (
        plain.pixel(cx, cy).expect("px"),
        treated.pixel(cx, cy).expect("px"),
    );
    for (p, q) in [
        (a.red(), b.red()),
        (a.green(), b.green()),
        (a.blue(), b.blue()),
    ] {
        assert!(
            (i32::from(p) - i32::from(q)).abs() < 40,
            "centre {a:?} vs {b:?}"
        );
    }

    // Deterministic across fresh renderers.
    let again = render0(&figure_project(
        &treatment_json(&paper_cutout(1.0)),
        "contain",
    ));
    assert_eq!(treated.data(), again.data());
}

#[test]
fn renderer_cover_fit_still_clips_treated_images_to_the_box() {
    let t = paper_cutout(1.0);
    let treated = render0(&figure_project(&treatment_json(&t), "cover"));
    let (x, y, w, h) = FIGURE_BOX;
    let inside = |px: u32, py: u32| {
        (px as f32) >= x && (px as f32) < x + w && (py as f32) >= y && (py as f32) < y + h
    };
    for py in 0..400 {
        for px in 0..300 {
            if !inside(px, py) {
                let p = treated.pixel(px, py).expect("px");
                assert_eq!(
                    (p.red(), p.green(), p.blue()),
                    (255, 255, 255),
                    "({px},{py}) outside the cover box"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

fn golden_dir() -> PathBuf {
    repo_root().join("golden/treatments")
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn read_hashes(path: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.trim().trim_end_matches(',').split("\": \"");
            let key = parts.next()?.strip_prefix('"')?;
            let value = parts.next()?.strip_suffix('"')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// Render `figure.png` through `treatment` on a small paper-coloured canvas.
fn golden_frame(treatment: &ImageTreatment) -> Pixmap {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "treatment_golden", "duration_seconds": 1.0 }},
        "canvas": {{ "width": 240, "height": 320, "fps": 30, "background": "#ECE3D2" }},
        "assets": [ {{ "id": "fig", "type": "image", "path": "assets/test_images/figure.png" }} ],
        "scenes": [ {{ "id": "s", "start_seconds": 0.0, "duration_seconds": 1.0, "layers": [
            {{ "id": "img", "type": "image", "asset": "fig", "fit": "contain",
               "x": 20, "y": 20, "width": 200, "height": 280{} }}
        ] }} ]
    }}"##,
        treatment_json(treatment)
    );
    render0(&MotionProject::from_json(&json).expect("golden project"))
}

fn check_golden(name: &str, pm: &Pixmap) {
    let update = std::env::var("MOTION_UPDATE_GOLDEN").is_ok_and(|v| v == "1");
    let dir = golden_dir();
    let hashes_path = dir.join("hashes.json");
    let key = format!("{name}@{}", platform());
    let hash = format!("{:016x}", fnv1a(pm.data()));
    let mut hashes = read_hashes(&hashes_path);

    // Same input twice: same pixels.
    assert!(
        pm.data().iter().any(|&b| b != pm.data()[0]),
        "{name}: blank"
    );

    if update {
        std::fs::create_dir_all(&dir).expect("mkdir golden/treatments");
        pm.save_png(dir.join(format!("{name}.png"))).expect("png");
        hashes.insert(key, hash);
        let body: Vec<String> = hashes
            .iter()
            .map(|(k, v)| format!("  \"{k}\": \"{v}\""))
            .collect();
        std::fs::write(&hashes_path, format!("{{\n{}\n}}\n", body.join(",\n")))
            .expect("write hashes");
    } else {
        assert!(
            dir.join(format!("{name}.png")).exists(),
            "{name}.png missing; run with MOTION_UPDATE_GOLDEN=1"
        );
        if let Some(expected) = hashes.get(&key) {
            assert_eq!(
                expected,
                &hash,
                "{name}: hash changed on {}. If intentional, rerun with MOTION_UPDATE_GOLDEN=1 \
                 and review the PNG.",
                platform()
            );
        }
    }
}

#[test]
fn golden_duotone_asset() {
    let t = ImageTreatment {
        preset: TreatmentPreset::EditorialDuotone,
        contrast: 1.08,
        duotone: Some(Duotone {
            shadow: Color::rgb(0x17, 0x15, 0x13),
            highlight: Color::rgb(0xEC, 0xE3, 0xD2),
            amount: 0.9,
        }),
        grain: 0.05,
        seed: 11,
        ..natural()
    };
    let a = golden_frame(&t);
    let b = golden_frame(&t);
    assert_eq!(a.data(), b.data());
    check_golden("duotone_asset", &a);
}

#[test]
fn golden_paper_cutout_asset() {
    // u = canvas width / 1080 reference.
    let t = paper_cutout(240.0 / 1080.0);
    let a = golden_frame(&t);
    let b = golden_frame(&t);
    assert_eq!(a.data(), b.data());
    check_golden("paper_cutout_asset", &a);
}
