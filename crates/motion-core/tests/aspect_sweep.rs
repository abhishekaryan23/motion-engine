//! (0.9 Phase 3) Aspect sweep: every checked-in story compiles at every canvas
//! aspect from 9:21 to 21:9, validates, passes layout QA (`layout_report`: text
//! in the safe area, nothing clipped, legible type, subject images on canvas)
//! and no motion outlives its scene. Assertions are structural; no pixels.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::{CreativeIntent, Format};
use motion_core::layout_report;
use motion_core::scene::{Motion, MotionProject, Scene};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;

/// Aspect ratios (W / H) swept; the short side is always 1080.
const ASPECTS: &[f32] = &[0.43, 0.5625, 0.75, 0.8, 1.0, 1.2, 1.333, 1.778, 2.37];

const STYLES: &[&str] = &[
    "examples/taste/warm_editorial.style.json",
    "examples/taste/dark_technical.style.json",
    "examples/taste/playful_print.style.json",
];

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Short side 1080, long side rounded to an even number of pixels.
fn canvas_for(aspect: f32) -> (u32, u32) {
    let even = |v: f32| ((v / 2.0).round() as u32) * 2;
    if aspect >= 1.0 {
        (even(1080.0 * aspect), 1080)
    } else {
        (1080, even(1080.0 / aspect))
    }
}

/// A checked-in story and the asset families its compile enables.
struct Case {
    rel: &'static str,
    families: &'static [&'static str],
}

const fn case(rel: &'static str) -> Case {
    Case { rel, families: &[] }
}

const CORE: &[Case] = &[
    case("examples/editorial_demo.intent.json"),
    Case {
        rel: "examples/icon_demo/icon_demo.intent.json",
        families: &["sketch_icons", "woodcut_kitchen"],
    },
    Case {
        rel: "examples/library_09/classical.intent.json",
        families: &["classical_greyscale"],
    },
    case("golden/collection_accumulate.intent.json"),
    case("golden/derived_metric.intent.json"),
    case("golden/dual_state_change.intent.json"),
    case("golden/editorial_collage.intent.json"),
    case("golden/multiplane.intent.json"),
    case("golden/read_phase_evolution.intent.json"),
    case("golden/scene_lifecycle.intent.json"),
    case("golden/semantic_compile.intent.json"),
    case("golden/sequential_stack.intent.json"),
    case("golden/shared_motion_target.intent.json"),
    case("golden/split_contrast.intent.json"),
    case("golden/state_change.intent.json"),
];

/// One layout finding, keyed so it can be compared across canvases.
type FindingKey = (String, String, &'static str);

/// A compile's structural failures plus its layout findings (key, detail).
type Checked = (Vec<String>, Vec<(FindingKey, String)>);

/// Compile one story at one canvas and check it (validate, canvas, no motion
/// outliving its scene). Returns the project's layout findings separately.
fn compile_checked(
    case: &Case,
    style_rel: &str,
    (w, h): (u32, u32),
    label: &str,
    measure: &dyn TextMeasure,
) -> Result<Checked, String> {
    let intent = CreativeIntent::from_json(&read(case.rel)).map_err(|e| format!("{label}: {e}"))?;
    let style: StyleProfile =
        serde_json::from_str(&read(style_rel)).map_err(|e| format!("{label}: {e}"))?;
    let library = AssetLibrary::new(repo().join("assets"))
        .with_families(case.families.iter().map(|s| s.to_string()).collect());
    let opts = CompileOptions {
        canvas: Some((w, h)),
        ..CompileOptions::default()
    };
    let project = compile_with_options(
        &intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .map_err(|e| format!("{label}: compile: {e}"))?;
    let mut fails = Vec::new();
    if (project.canvas.width, project.canvas.height) != (w, h) {
        fails.push(format!(
            "{label}: canvas is {}x{}",
            project.canvas.width, project.canvas.height
        ));
    }
    if let Err(e) = validate(&project, None) {
        fails.push(format!("{label}: validate: {e}"));
    }
    fails.extend(outliving_motion(&project, label));
    let frame = LayoutFrame::new(w, h).map_err(|e| format!("{label}: {e}"))?;
    let findings = layout_report(&project, &frame)
        .findings
        .into_iter()
        .map(|f| {
            let key = (f.scene, f.layer, f.check.name());
            (key, f.detail)
        })
        .collect();
    Ok((fails, findings))
}

/// The legacy canvas (1080x1920 / 1080x1080 / 1920x1080) of an aspect's class.
fn legacy_canvas_for(aspect: f32) -> (u32, u32) {
    let (w, h) = canvas_for(aspect);
    let class = LayoutFrame::new(w, h)
        .map(|f| f.class)
        .unwrap_or(Format::Square);
    LayoutFrame::legacy_size(class)
}

/// Compile one story at one canvas; the failures (empty = ok).
///
/// Layout findings are judged against the legacy canvas of the same aspect
/// class: the legacy canvases are frozen byte-for-byte (goldens), so a finding
/// that the frozen legacy compile already has (same scene, layer and check, in
/// any of the swept styles) is inherited, not an aspect regression.
/// Everything else must pass.
fn run(
    case: &Case,
    style_rel: &str,
    aspect: f32,
    measure: &dyn TextMeasure,
) -> Result<Vec<String>, String> {
    let (w, h) = canvas_for(aspect);
    let label = format!("{} x {} @ {aspect} ({w}x{h})", case.rel, style_rel);
    let (mut fails, findings) = compile_checked(case, style_rel, (w, h), &label, measure)?;
    let legacy = legacy_canvas_for(aspect);
    // Aspect-independent legacy problems show up in some styles only (the
    // sample times shift with the style), so the baseline is the union over
    // every style at the legacy canvas.
    let mut inherited: Vec<FindingKey> = Vec::new();
    for st in STYLES {
        if legacy == (w, h) && *st != style_rel {
            continue;
        }
        let lbl = format!("{} x {st} @ legacy {}x{}", case.rel, legacy.0, legacy.1);
        let (_, f) = if legacy == (w, h) {
            (Vec::new(), findings.clone())
        } else {
            compile_checked(case, st, legacy, &lbl, measure)?
        };
        inherited.extend(f.into_iter().map(|(k, _)| k));
    }
    for (key, detail) in &findings {
        if !inherited.contains(key) {
            fails.push(format!(
                "{label}: layout {} {} {}: {detail}",
                key.0, key.1, key.2
            ));
        }
    }
    Ok(fails)
}

/// No motion (scene, camera, shared track key) outlives its scene.
fn outliving_motion(p: &MotionProject, label: &str) -> Vec<String> {
    const EPS: f64 = 1e-6;
    let mut out = Vec::new();
    let all: Vec<(&Scene, &Motion)> = p
        .scenes
        .iter()
        .flat_map(|s| s.motions.iter().map(move |m| (s, m)))
        .collect();
    for (s, m) in all {
        if m.start < -EPS || m.start + m.duration > s.duration_seconds + EPS {
            out.push(format!(
                "{label}: motion on '{}' in scene '{}' spans {}..{} (scene {}s)",
                m.target,
                s.id,
                m.start,
                m.start + m.duration,
                s.duration_seconds
            ));
        }
    }
    for s in &p.scenes {
        for cm in s.camera.iter().flat_map(|c| &c.motions) {
            if cm.start + cm.duration > s.duration_seconds + EPS {
                out.push(format!("{label}: camera motion outlives scene '{}'", s.id));
            }
        }
    }
    for sh in &p.shared {
        for k in &sh.track {
            let outlives = p
                .scenes
                .iter()
                .find(|s| s.id == k.scene)
                .map(|sc| k.at > sc.duration_seconds + EPS)
                .unwrap_or(true);
            if outlives {
                out.push(format!(
                    "{label}: shared key at {} outlives scene '{}'",
                    k.at, k.scene
                ));
            }
        }
    }
    out
}

/// Sweep `cases` x STYLES x ASPECTS; returns (compiled count, failures).
fn sweep(cases: &[Case]) -> (usize, Vec<String>) {
    let mut count = 0;
    let mut failures = Vec::new();
    for c in cases {
        for style in STYLES {
            for &aspect in ASPECTS {
                count += 1;
                match run(c, style, aspect, &ApproxMeasure) {
                    Ok(f) => failures.extend(f),
                    Err(e) => failures.push(e),
                }
            }
        }
    }
    (count, failures)
}

fn assert_clean(cases: &[Case]) {
    let (count, failures) = sweep(cases);
    assert!(
        failures.is_empty(),
        "{} of {count} sweep cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn canvas_sizes_are_even_and_in_range() {
    for &a in ASPECTS {
        let (w, h) = canvas_for(a);
        assert!(w % 2 == 0 && h % 2 == 0);
        assert!(LayoutFrame::new(w, h).is_ok(), "{a}: {w}x{h}");
    }
}

#[test]
fn core_stories_fit_every_aspect() {
    assert_clean(CORE);
}

// ---------------------------------------------------------------------------
// Grammars whose builders are converted elsewhere. They run with the same
// sweep; each is its own test so a pending conversion is one `#[ignore]` line.
// ---------------------------------------------------------------------------

#[test]
fn hero_object_fits_every_aspect() {
    assert_clean(&[case("golden/hero_object.intent.json")]);
}

#[test]
fn evidence_stack_fits_every_aspect() {
    assert_clean(&[case("golden/evidence_stack.intent.json")]);
}

#[test]
fn type_image_fits_every_aspect() {
    assert_clean(&[case("golden/type_image.intent.json")]);
}

#[test]
fn kinetic_poster_fits_every_aspect() {
    assert_clean(&[case("golden/kinetic_poster.intent.json")]);
}

#[test]
fn data_story_fits_every_aspect() {
    assert_clean(&[case("golden/data_story.intent.json")]);
}

/// `derived.rs`: comparison row captions / bar tags are lifted to the
/// `min_type_px` floor on non-legacy canvases (the frozen legacy canvases keep
/// their sizes).
#[test]
fn derived_metric_compare_fits_every_aspect() {
    assert_clean(&[case("golden/derived_metric_compare.intent.json")]);
}

/// Informational: the layout findings the frozen legacy canvases already have
/// (the sweep treats these as inherited). Run with
/// `cargo test -p motion-core --test aspect_sweep -- --ignored --nocapture legacy`.
#[test]
#[ignore = "informational: prints the findings of the frozen legacy canvases"]
fn legacy_baseline_findings() {
    let extra = [
        case("golden/hero_object.intent.json"),
        case("golden/evidence_stack.intent.json"),
        case("golden/type_image.intent.json"),
        case("golden/kinetic_poster.intent.json"),
        case("golden/data_story.intent.json"),
        case("golden/derived_metric_compare.intent.json"),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for c in CORE.iter().chain(extra.iter()) {
        for style in STYLES {
            for size in [(1080, 1920), (1080, 1080), (1920, 1080)] {
                let label = format!("{} x {style} @ {}x{}", c.rel, size.0, size.1);
                let Ok((_, findings)) = compile_checked(c, style, size, &label, &ApproxMeasure)
                else {
                    continue;
                };
                for (key, detail) in findings {
                    if seen.insert(format!("{} {} {}", c.rel, key.1, key.2)) {
                        println!("LEGACY {} {} {} {}: {detail}", c.rel, key.0, key.1, key.2);
                    }
                }
            }
        }
    }
}
