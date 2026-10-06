//! (0.23 W4) `text_local_contrast` on synthetic projects: text over a field
//! of its own colour, display vs body limits, glyphs of neighbouring text left
//! out of the ground, words judged one by one. Only the repository fonts are
//! read; every frame stays in memory.

use std::path::Path;

use motion_core::checks::{TEXT_CONTRAST_BODY, TEXT_CONTRAST_DISPLAY};
use motion_core::layout_qa::LayoutCheck;
use motion_core::MotionProject;
use motion_render::contrast_qa::{
    contrast_ratio, needed_ratio, ring_ground, text_local_contrast, Grid, RING_FAR_PX, RING_NEAR_PX,
};
use motion_render::speech_qa::CheckStatus;

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// A one-beat project (`beat_1`, READ at 0.5 s) on a `width` x `height` canvas.
fn project(width: u32, height: u32, background: &str, layers: &[String]) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "contrast", "duration_seconds": 2.0 }},
        "canvas": {{ "width": {width}, "height": {height}, "fps": 30, "background": "{background}" }},
        "theme": {{ "fonts": {{ "number": "font.anton", "body": "font.fira" }} }},
        "assets": [
            {{ "id": "font.anton", "type": "font", "path": "fonts/Anton-Regular.ttf" }},
            {{ "id": "font.fira", "type": "font", "path": "fonts/FiraSans-Regular.ttf" }}
        ],
        "asset_root": "assets",
        "scenes": [
            {{ "id": "beat_1", "start_seconds": 0.0, "duration_seconds": 2.0,
               "layers": [{}],
               "lifecycle": {{ "enter": 0.1, "settle": 0.2, "read": 0.5, "evolve": 1.2,
                               "anticipate": 1.6, "bridge": 2.0 }} }}
        ]
    }}"##,
        layers.join(",")
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

fn rect(id: &str, x: u32, y: u32, w: u32, h: u32, fill: &str) -> String {
    format!(
        r##"{{ "id": "{id}", "type": "rectangle", "x": {x}, "y": {y}, "width": {w}, "height": {h},
              "fill": "{fill}" }}"##
    )
}

fn text(id: &str, x: u32, y: u32, w: u32, size: u32, color: &str, body: &str) -> String {
    format!(
        r##"{{ "id": "{id}", "type": "text", "x": {x}, "y": {y}, "width": {w}, "height": {},
              "text": "{body}", "font_role": "number", "font_size": {size}, "color": "{color}" }}"##,
        size + size / 2
    )
}

fn run(p: &MotionProject) -> motion_render::contrast_qa::ContrastReport {
    text_local_contrast(p, repo_root()).expect("contrast check")
}

// ---------------------------------------------------------------------------
// Text over a field of its own colour
// ---------------------------------------------------------------------------

#[test]
fn black_text_on_a_black_rectangle_fails() {
    let p = project(
        540,
        960,
        "#FFFFFF",
        &[
            rect("b1.field", 20, 180, 500, 240, "#000000"),
            text("b1.head", 40, 220, 460, 100, "#000000", "YOUR"),
        ],
    );
    let r = run(&p);
    assert_eq!(r.checked, 1, "{:?}", r.skipped);
    assert_eq!(r.findings.len(), 1, "{:?}", r.measured);
    let f = &r.findings[0];
    assert_eq!((f.scene.as_str(), f.layer.as_str()), ("beat_1", "b1.head"));
    assert_eq!(f.text, "YOUR");
    assert!(f.ratio < 1.1, "{}", f.ratio);
    assert_eq!(f.need, TEXT_CONTRAST_DISPLAY);
    assert_eq!(f.ink, [0, 0, 0]);
    assert!(f.ink_share > 0.9, "{}", f.ink_share);
    // The wiring: a layout finding and a speech check.
    let l = r.layout_findings();
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].check, LayoutCheck::TextLocalContrast);
    assert_eq!(
        (l[0].scene.as_str(), l[0].layer.as_str()),
        ("beat_1", "b1.head")
    );
    assert!(l[0].detail.contains("#000000"), "{}", l[0].detail);
    let c = r.check();
    assert_eq!(c.name, "text_local_contrast");
    assert_eq!(c.status, CheckStatus::Fail);
    assert!(c.detail.contains("b1.head"), "{}", c.detail);
}

#[test]
fn a_headline_half_over_a_field_of_its_colour_fails() {
    // The black block covers the left half of "YOUR": its ring is about half
    // black. Dropping every ink-coloured pixel as "a neighbour's glyph" would
    // read the paper half only and pass it.
    let p = project(
        540,
        960,
        "#FFFFFF",
        &[
            rect("b1.field", 0, 150, 160, 300, "#000000"),
            text("b1.head", 40, 220, 460, 100, "#000000", "YOUR"),
        ],
    );
    let r = run(&p);
    assert_eq!(r.measured.len(), 1, "{:?}", r.skipped);
    let m = &r.measured[0];
    assert!(m.ink_share > 0.4 && m.ink_share < 0.8, "{}", m.ink_share);
    assert_eq!(
        r.findings.len(),
        1,
        "ratio {} ink share {}",
        m.ratio,
        m.ink_share
    );
    assert!(r.findings[0].ratio < 1.5, "{}", r.findings[0].ratio);
}

#[test]
fn black_text_on_white_passes() {
    let p = project(
        540,
        960,
        "#FFFFFF",
        &[
            rect("b1.field", 20, 180, 500, 240, "#FFFFFF"),
            text("b1.head", 40, 220, 460, 100, "#000000", "YOUR"),
        ],
    );
    let r = run(&p);
    assert_eq!(r.checked, 1);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    assert!(r.min_margin.expect("measured") > 10.0);
    let c = r.check();
    assert_eq!(c.status, CheckStatus::Pass);
    assert!(r.layout_findings().is_empty());
}

#[test]
fn a_canvas_wider_than_the_analysis_scale_is_judged_the_same() {
    // 1080 px wide: the READ frame is rendered at full size and box-filtered
    // to 540 px before the ring is read; the verdict does not change.
    let layers = |field: &str| {
        [
            rect("b1.field", 40, 360, 1000, 480, field),
            text("b1.head", 80, 440, 920, 200, "#000000", "YOUR"),
        ]
    };
    let dark = run(&project(1080, 1920, "#FFFFFF", &layers("#000000")));
    assert_eq!(dark.findings.len(), 1, "{:?}", dark.measured);
    assert!(dark.findings[0].ratio < 1.1);
    assert_eq!(dark.findings[0].size_px, 200.0);
    let light = run(&project(1080, 1920, "#FFFFFF", &layers("#FFFFFF")));
    assert!(light.findings.is_empty(), "{:?}", light.findings);
    assert_eq!(light.checked, 1);
}

// ---------------------------------------------------------------------------
// Display vs body limits
// ---------------------------------------------------------------------------

#[test]
fn display_text_needs_three_and_small_text_four_and_a_half() {
    // Mid grey on white: between the two limits.
    let ratio = contrast_ratio([0x8A; 3], [0xFF; 3]);
    assert!(
        ratio > TEXT_CONTRAST_DISPLAY && ratio < TEXT_CONTRAST_BODY,
        "{ratio}"
    );
    // 540 px canvas: u = 0.5, so display text starts at 12 px.
    assert_eq!(needed_ratio(80.0, 0.5), TEXT_CONTRAST_DISPLAY);
    assert_eq!(needed_ratio(12.0, 0.5), TEXT_CONTRAST_DISPLAY);
    assert_eq!(needed_ratio(11.0, 0.5), TEXT_CONTRAST_BODY);
    assert_eq!(needed_ratio(24.0, 1.0), TEXT_CONTRAST_DISPLAY);
    assert_eq!(needed_ratio(23.0, 1.0), TEXT_CONTRAST_BODY);

    let p = project(
        540,
        960,
        "#FFFFFF",
        &[
            text("b1.big", 40, 200, 460, 80, "#8A8A8A", "BIG"),
            text("b1.small", 40, 600, 460, 10, "#8A8A8A", "small print"),
        ],
    );
    let r = run(&p);
    assert_eq!(r.checked, 2, "{:?}", r.skipped);
    assert_eq!(r.findings.len(), 1, "{:?}", r.measured);
    let f = &r.findings[0];
    assert_eq!(f.layer, "b1.small");
    assert_eq!(f.need, TEXT_CONTRAST_BODY);
    assert!((f.ratio - ratio).abs() < 0.05, "{} vs {ratio}", f.ratio);
    let big = r
        .measured
        .iter()
        .find(|m| m.layer == "b1.big")
        .expect("big measured");
    assert_eq!(big.need, TEXT_CONTRAST_DISPLAY);
    assert!(big.ratio >= big.need);
}

// ---------------------------------------------------------------------------
// What is judged
// ---------------------------------------------------------------------------

#[test]
fn decorative_layers_are_not_judged() {
    // A black ghost word on a black field is atmosphere by id convention.
    let p = project(
        540,
        960,
        "#FFFFFF",
        &[
            rect("b1.field", 20, 180, 500, 240, "#000000"),
            text("b1.ghost", 40, 220, 460, 100, "#000000", "GHOST"),
            text("b1.head", 40, 600, 460, 80, "#000000", "SEEN"),
        ],
    );
    let r = run(&p);
    assert_eq!(r.checked, 1);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    assert_eq!(r.measured[0].layer, "b1.head");
}

#[test]
fn a_line_whose_first_word_hides_under_a_field_fails_on_that_word() {
    // "XX" over the black block, "YY" far to the right on white: the ring
    // around the whole line is mostly white, the ring around "XX" is black.
    let gap = " ".repeat(14);
    let body = format!("XX{gap}YY");
    let p = project(
        540,
        960,
        "#FFFFFF",
        &[
            rect("b1.field", 0, 180, 190, 240, "#000000"),
            text("b1.head", 40, 220, 480, 80, "#000000", &body),
        ],
    );
    let r = run(&p);
    assert_eq!(r.measured.len(), 1, "{:?}", r.skipped);
    assert!(
        r.measured[0].ratio >= TEXT_CONTRAST_DISPLAY,
        "the line as a whole passes: {}",
        r.measured[0].ratio
    );
    assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
    let f = &r.findings[0];
    assert_eq!(f.text, "XX");
    assert_eq!(f.of.as_deref(), Some(body.as_str()));
    assert!(f.ratio < 1.5, "{}", f.ratio);
    assert!(f.detail().contains("word of"), "{}", f.detail());
}

// ---------------------------------------------------------------------------
// The ground
// ---------------------------------------------------------------------------

/// A 100 x 100 white grid; the ink box is `[40, 40, 60, 60]`, so the ring is
/// x/y in 28..36 and 64..72 around it.
fn white() -> Grid {
    Grid::filled(100, 100, [255, 255, 255])
}

const BOX: [f32; 4] = [40.0, 40.0, 60.0, 60.0];

#[test]
fn glyphs_of_a_neighbouring_text_are_left_out_of_the_ground() {
    // Vertical black strokes, 2 px wide every 4 px, fill the whole left band
    // and the top band of the ring: half of those pixels are ink-coloured,
    // together 30 % of the ring.
    let mut g = white();
    let stroke = |x: u32| x % 4 < 2;
    for y in 28..72u32 {
        for x in 28..36u32 {
            if stroke(x) {
                g.put(x, y, [0, 0, 0]);
            }
        }
    }
    for y in 28..36u32 {
        for x in 36..72u32 {
            if stroke(x) {
                g.put(x, y, [0, 0, 0]);
            }
        }
    }
    let ground = ring_ground(&g, BOX, RING_NEAR_PX, RING_FAR_PX, [0, 0, 0], 20.0).expect("ring");
    assert_eq!(ground.color, [255, 255, 255]);
    assert!(
        ground.glyph_share > 0.1 && ground.glyph_share == ground.ink_share,
        "{ground:?}"
    );
    // The contrast is that of the paper, not of the neighbour's ink.
    assert!(contrast_ratio([0, 0, 0], ground.color) > 20.0);

    // The same ink-coloured pixels as a solid field are ground: the text sits
    // on its own colour.
    let mut field = white();
    for y in 10..90u32 {
        for x in 10..90u32 {
            field.put(x, y, [0, 0, 0]);
        }
    }
    let ground =
        ring_ground(&field, BOX, RING_NEAR_PX, RING_FAR_PX, [0, 0, 0], 20.0).expect("ring");
    assert_eq!(ground.color, [0, 0, 0]);
    assert_eq!(ground.glyph_share, 0.0);
    assert!(ground.ink_share > 0.99);
}

#[test]
fn dense_neighbouring_glyphs_do_not_darken_the_ground() {
    // Half of every ring pixel is a black stroke (2 px every 4 px); the rest is
    // a ground that ramps from dark grey to white. Counting the strokes the
    // median would sit in the dark end of the ramp.
    let mut g = Grid::filled(100, 100, [255, 255, 255]);
    for y in 20..80u32 {
        for x in 20..80u32 {
            let ramp = (40 + (y.saturating_sub(20)) * 215 / 60).min(255) as u8;
            let c = if x % 4 < 2 { [0, 0, 0] } else { [ramp; 3] };
            g.put(x, y, c);
        }
    }
    let ground = ring_ground(&g, BOX, RING_NEAR_PX, RING_FAR_PX, [0, 0, 0], 20.0).expect("ring");
    assert!(ground.glyph_share > 0.4, "{ground:?}");
    // The middle of the ramp, not its dark end.
    assert!((100..=200).contains(&ground.color[0]), "{ground:?}");
    assert!(contrast_ratio([0, 0, 0], ground.color) > 3.0);
}

#[test]
fn a_grainy_field_of_the_ink_colour_is_still_a_field() {
    // Black ground with +-6 grain: many pixels sit beyond the ink distance, the
    // window mean does not.
    let mut g = Grid::filled(100, 100, [255, 255, 255]);
    for y in 20..80u32 {
        for x in 20..80u32 {
            let n = ((x * 7 + y * 13) % 13) as u8; // 0..=12
            g.put(x, y, [n, n, n]);
        }
    }
    let ground = ring_ground(&g, BOX, RING_NEAR_PX, RING_FAR_PX, [3, 3, 3], 20.0).expect("ring");
    assert!(ground.color[0] <= 12, "{ground:?}");
    assert!(contrast_ratio([3, 3, 3], ground.color) < 1.3);
}

#[test]
fn a_ring_outside_the_canvas_has_no_ground() {
    let g = Grid::filled(20, 20, [255, 255, 255]);
    assert!(ring_ground(&g, [100.0, 100.0, 120.0, 120.0], 4, 12, [0, 0, 0], 20.0).is_none());
}

#[test]
fn median_is_per_channel_and_ignores_a_minority() {
    // A 20 % blob of another colour does not move the median.
    let mut g = Grid::filled(100, 100, [200, 100, 50]);
    for y in 28..72u32 {
        for x in 28..36u32 {
            g.put(x, y, [10, 240, 10]);
        }
    }
    let ground = ring_ground(&g, BOX, RING_NEAR_PX, RING_FAR_PX, [0, 0, 0], 20.0).expect("ring");
    assert_eq!(ground.color, [200, 100, 50]);
}
