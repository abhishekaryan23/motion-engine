//! Exploration knob (0.9 Phase 2): level 0 is canonical, exploration only
//! widens curated taste sets, guardrails fall back deterministically.

use motion_core::assets::AssetManifest;
use motion_core::audio::{
    beat_scenes, plan_audio, plan_audio_explored, AudioPlan, SfxFamily, SfxLibrary, SfxSound,
    SFX_LIBRARY_VERSION,
};
use motion_core::compiler::explore::{
    apply_exploration, contrast_ratio, finalize_record, guardrail_loop, guardrail_report,
    guardrail_report_with, ExploreDimension, ExploreRecord, Guardrails,
};
use motion_core::compiler::{
    compile_with_music, compile_with_options, resolve_taste, story_key_of, ApproxMeasure,
    AssetLibrary, CompileOptions, Palette,
};
use motion_core::intent::Energy;
use motion_core::scene::{Color, Layer, LayerKind, MotionProject};
use motion_core::{CreativeIntent, StyleProfile};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const STYLES: [(&str, &str); 4] = [
    (
        "editorial_demo",
        include_str!("../../../examples/editorial_demo.style.json"),
    ),
    (
        "warm_editorial",
        include_str!("../../../examples/taste/warm_editorial.style.json"),
    ),
    (
        "dark_technical",
        include_str!("../../../examples/taste/dark_technical.style.json"),
    ),
    (
        "playful_print",
        include_str!("../../../examples/taste/playful_print.style.json"),
    ),
];
const INTENTS: [(&str, &str); 5] = [
    (
        "editorial_demo",
        include_str!("../../../examples/editorial_demo.intent.json"),
    ),
    (
        "motion_language_demo",
        include_str!("../../../examples/motion_language_demo.intent.json"),
    ),
    (
        "derived_metric",
        include_str!("../../../examples/public/derived-metric.intent.json"),
    ),
    (
        "collection",
        include_str!("../../../examples/public/collection-accumulate.intent.json"),
    ),
    (
        "state_change",
        include_str!("../../../examples/public/state-change.intent.json"),
    ),
];

fn library() -> AssetLibrary {
    AssetLibrary::new(ASSETS)
}

fn compile(intent: &str, style: &str, opts: &CompileOptions) -> MotionProject {
    let intent = CreativeIntent::from_json(intent).expect("intent");
    let style: StyleProfile = serde_json::from_str(style).expect("style");
    compile_with_options(
        &intent,
        &style,
        None,
        &library(),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        opts,
    )
    .expect("compile")
}

fn opts(explore: u8, seed: u64) -> CompileOptions {
    CompileOptions {
        explore,
        seed,
        ..CompileOptions::default()
    }
}

fn bytes(p: &MotionProject) -> String {
    serde_json::to_string_pretty(p).expect("json")
}

fn texts(layers: &[Layer], out: &mut Vec<String>) {
    for l in layers {
        match &l.kind {
            LayerKind::Text(t) => out.push(t.text.clone()),
            LayerKind::Group { children } => texts(children, out),
            _ => {}
        }
    }
}

/// Every word the piece shows (line breaks between layers are layout, not
/// content: scale contrast may split or join lines).
fn all_texts(p: &MotionProject) -> Vec<String> {
    let mut out = Vec::new();
    for s in &p.scenes {
        texts(&s.layers, &mut out);
    }
    out.iter()
        .flat_map(|t| t.split_whitespace().map(str::to_string))
        .collect()
}

/// Decorative furniture words the density / transition choices may add,
/// remove or retime: zero-padded index numerals, "NO."/"SEC" labels,
/// separators and the technical start-time stamp ("T+4.1"). Authored words
/// and numbers never match.
fn is_furniture(w: &str) -> bool {
    w.starts_with("T+")
        || matches!(w, "NO." | "SEC" | "·" | "—")
        || (w.len() == 2 && w.starts_with('0') && w.chars().all(|c| c.is_ascii_digit()))
}

/// Words that differ between two projects (multiset difference, both ways).
fn word_diff(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: Vec<&String> = b.iter().collect();
    for w in a {
        match rest.iter().position(|x| *x == w) {
            Some(i) => {
                rest.remove(i);
            }
            None => out.push(w.clone()),
        }
    }
    out.extend(rest.into_iter().cloned());
    out
}

// ---- level 0 ---------------------------------------------------------------

#[test]
fn level_zero_is_byte_identical_to_canonical() {
    for (iname, intent_json) in INTENTS {
        for (sname, style_json) in STYLES {
            let intent = CreativeIntent::from_json(intent_json).expect("intent");
            let style: StyleProfile = serde_json::from_str(style_json).expect("style");
            let canonical = compile_with_music(
                &intent,
                &style,
                None,
                &library(),
                &ApproxMeasure,
                &AssetManifest::empty(),
                None,
            )
            .expect("compile");
            for seed in [0u64, 1, 0xDEAD_BEEF] {
                let p = compile(intent_json, style_json, &opts(0, seed));
                assert_eq!(
                    bytes(&p),
                    bytes(&canonical),
                    "{iname} / {sname} seed {seed}"
                );
                assert!(p.project.exploration.is_none());
            }
        }
    }
}

// ---- determinism + allowed dimensions -----------------------------------

#[test]
fn same_seed_and_level_is_identical() {
    for (_, intent) in INTENTS {
        for (_, style) in STYLES {
            for level in 1..=3u8 {
                let a = compile(intent, style, &opts(level, 5));
                let b = compile(intent, style, &opts(level, 5));
                assert_eq!(bytes(&a), bytes(&b));
            }
        }
    }
}

#[test]
fn text_content_never_changes_across_seeds_and_levels() {
    for (iname, intent) in INTENTS {
        for (sname, style) in STYLES {
            let base = all_texts(&compile(intent, style, &opts(0, 0)));
            for level in 1..=3u8 {
                for seed in 0..10u64 {
                    let p = compile(intent, style, &opts(level, seed));
                    let diff = word_diff(&all_texts(&p), &base);
                    assert!(
                        diff.iter().all(|w| is_furniture(w)),
                        "{iname} / {sname}: level {level} seed {seed} changed content words: {diff:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn record_lists_only_dimensions_up_to_the_level() {
    for (_, style) in STYLES {
        for level in 1..=3u8 {
            for seed in 0..10u64 {
                let p = compile(INTENTS[0].1, style, &opts(level, seed));
                let rec = p.project.exploration.expect("record at level > 0");
                assert_eq!(rec.level, level);
                assert_eq!(rec.seed, seed);
                let expected: Vec<ExploreDimension> = ExploreDimension::ALL
                    .into_iter()
                    .filter(|d| d.level() <= level)
                    .collect();
                let got: Vec<ExploreDimension> = rec.choices.iter().map(|c| c.dimension).collect();
                assert_eq!(got, expected);
                for c in &rec.choices {
                    assert!(c.dimension.level() <= level);
                }
            }
        }
    }
}

#[test]
fn seeds_change_the_design_at_higher_levels() {
    // A non-classic style must actually vary across seeds at L2 and L3.
    let style = STYLES[2].1;
    for level in [2u8, 3] {
        let outputs: std::collections::BTreeSet<String> = (0..10u64)
            .map(|s| bytes(&compile(INTENTS[0].1, style, &opts(level, s))))
            .collect();
        assert!(outputs.len() > 1, "level {level} produced one design");
    }
}

#[test]
fn apply_exploration_level_zero_is_identity_and_classic_taste_is_fixed() {
    let style: StyleProfile = serde_json::from_str(STYLES[0].1).expect("style");
    let intent = CreativeIntent::from_json(INTENTS[0].1).expect("intent");
    let taste = resolve_taste(&intent, &style, None);
    let (t0, r0) = apply_exploration(&taste, 0, 3, story_key_of(&intent));
    assert_eq!(t0, taste);
    assert!(r0.is_none());
    // Classic: the taste never varies, at any level.
    for level in 1..=3 {
        let (t, r) = apply_exploration(&taste, level, 9, story_key_of(&intent));
        assert_eq!(t, taste);
        assert!(r.is_some());
    }
}

#[test]
fn explored_dimensions_stay_in_curated_sets() {
    use motion_core::compiler::explore::{
        background_neighbours, palette_neighbours, AMPLITUDE_ORDER, DENSITY_ORDER, RHYTHM_ORDER,
        SCALE_ORDER, TRANSITION_ORDER,
    };
    for (_, style_json) in &STYLES[1..] {
        let style: StyleProfile = serde_json::from_str(style_json).expect("style");
        let intent = CreativeIntent::from_json(INTENTS[0].1).expect("intent");
        let taste = resolve_taste(&intent, &style, None);
        for level in 1..=3u8 {
            for seed in 0..25u64 {
                let (t, _) = apply_exploration(&taste, level, seed, story_key_of(&intent));
                assert_eq!(t.tone, taste.tone);
                if level < 2 {
                    assert_eq!(t.background, taste.background);
                    assert_eq!(t.transition, taste.transition);
                    assert_eq!(t.scale, taste.scale);
                    assert_eq!(t.motion, taste.motion);
                } else {
                    assert!(
                        t.background == taste.background
                            || background_neighbours(taste.background).contains(&t.background)
                    );
                    assert!(
                        (idx(&TRANSITION_ORDER, t.transition.family)
                            - idx(&TRANSITION_ORDER, taste.transition.family))
                        .abs()
                            <= 1
                    );
                    let reach = if level >= 3 { 2 } else { 1 };
                    assert!(
                        (idx(&SCALE_ORDER, t.scale) - idx(&SCALE_ORDER, taste.scale)).abs()
                            <= reach
                    );
                    assert!(
                        (idx(&AMPLITUDE_ORDER, t.motion.amplitude)
                            - idx(&AMPLITUDE_ORDER, taste.motion.amplitude))
                        .abs()
                            <= 1
                    );
                }
                if level < 3 {
                    assert_eq!(t.palette.family, taste.palette.family);
                    assert_eq!(t.rhythm, taste.rhythm);
                    assert_eq!(t.density.level, taste.density.level);
                } else {
                    assert!(
                        t.palette.family == taste.palette.family
                            || palette_neighbours(taste.palette.family).contains(&t.palette.family)
                    );
                    assert!(
                        (idx(&RHYTHM_ORDER, t.rhythm) - idx(&RHYTHM_ORDER, taste.rhythm)).abs()
                            <= 1
                    );
                    assert!(
                        (idx(&DENSITY_ORDER, t.density.level)
                            - idx(&DENSITY_ORDER, taste.density.level))
                        .abs()
                            <= 1
                    );
                }
                // Every explored palette still reads (curated sets only).
                let c = &t.palette.colors;
                assert!(contrast_ratio(c.ink, c.paper) >= 4.5);
                assert!(contrast_ratio(c.on_accent, c.accent) >= 3.0);
            }
        }
    }
}

// ---- guardrails -----------------------------------------------------------

fn palette_of(style_json: &str) -> Palette {
    let style: StyleProfile = serde_json::from_str(style_json).expect("style");
    let intent = CreativeIntent::from_json(INTENTS[0].1).expect("intent");
    resolve_taste(&intent, &style, None).palette.colors
}

#[test]
fn canonical_projects_pass_relative_guardrails() {
    for (_, style_json) in STYLES {
        let palette = palette_of(style_json);
        let p = compile(INTENTS[0].1, style_json, &opts(0, 0));
        let g = Guardrails::relative_to(&p, &palette);
        let report = guardrail_report_with(&p, &palette, &g);
        assert!(report.is_empty(), "canonical project fails: {report:?}");
        // The absolute contract only ever objects to known canonical quirks
        // (accent button contrast, the oversized ghost word), never to
        // validation or ink contrast.
        for r in guardrail_report(&p, &palette) {
            assert!(
                r.starts_with("contrast on_accent")
                    || r.starts_with("layout:")
                    || r.starts_with("min font size"),
                "{r}"
            );
        }
    }
}

#[test]
fn guardrail_report_flags_low_contrast_palettes() {
    let p = compile(INTENTS[0].1, STYLES[1].1, &opts(0, 0));
    let mut palette = palette_of(STYLES[1].1);
    palette.ink = Color::rgb(0x80, 0x80, 0x80);
    palette.paper = Color::rgb(0x88, 0x88, 0x88);
    let report = guardrail_report(&p, &palette);
    assert!(report.iter().any(|r| r.starts_with("contrast ink/paper")));
    palette.ink = Color::rgb(0, 0, 0);
    palette.paper = Color::rgb(255, 255, 255);
    palette.accent = Color::rgb(0xFF, 0xFF, 0x00);
    palette.on_accent = Color::rgb(0xFF, 0xFF, 0x80);
    let report = guardrail_report(&p, &palette);
    assert!(
        report
            .iter()
            .any(|r| r.starts_with("contrast on_accent/accent")),
        "{report:?}"
    );
    assert!(!report.iter().any(|r| r.starts_with("contrast ink")));
    // A relative baseline built from the bad palette accepts it (no regression).
    let g = Guardrails::relative_to(&p, &palette);
    assert!(!guardrail_report_with(&p, &palette, &g)
        .iter()
        .any(|r| r.starts_with("contrast")));
}

fn visit_text(layers: &mut [Layer], f: &mut dyn FnMut(&mut Layer)) {
    for l in layers {
        if matches!(l.kind, LayerKind::Text(_)) {
            f(l);
        }
        if let LayerKind::Group { children } = &mut l.kind {
            visit_text(children, f);
        }
    }
}

#[test]
fn guardrail_report_flags_small_type_and_boxes_outside_the_canvas() {
    let mut p = compile(INTENTS[0].1, STYLES[1].1, &opts(0, 0));
    let palette = palette_of(STYLES[1].1);
    let g = Guardrails::relative_to(&p, &palette);
    let canvas_w = p.canvas.width as f32;
    // Shrink one text layer below 20 px and push another off the canvas.
    let mut shrunk = None;
    let mut moved = None;
    for scene in &mut p.scenes {
        visit_text(&mut scene.layers, &mut |l| {
            if shrunk.is_none() && !l.id.ends_with(".ghost") {
                if let LayerKind::Text(t) = &mut l.kind {
                    t.font_size = 12.0;
                    shrunk = Some(l.id.clone());
                    return;
                }
            }
            if moved.is_none() && l.layout.is_none() && l.width > 0.0 && !l.id.ends_with(".ghost") {
                l.x = canvas_w + 500.0;
                moved = Some(l.id.clone());
            }
        });
    }
    let (shrunk, moved) = (shrunk.expect("shrunk"), moved.expect("moved"));
    let report = guardrail_report_with(&p, &palette, &g);
    assert!(
        report
            .iter()
            .any(|r| r.starts_with("min font size") && r.contains(&shrunk)),
        "{report:?}"
    );
    assert!(
        report
            .iter()
            .any(|r| r.starts_with("layout:") && r.contains(&moved)),
        "{report:?}"
    );
}

#[test]
fn guardrail_loop_falls_back_one_level_at_a_time() {
    // Levels 3 and 2 fail; 1 passes.
    let mut seen = Vec::new();
    let out = guardrail_loop::<u8, ()>(
        3,
        |l| {
            seen.push(l);
            Ok(l)
        },
        |l| {
            if *l >= 2 {
                vec![format!("fails at {l}"), "second".to_string()]
            } else {
                Vec::new()
            }
        },
    )
    .expect("loop");
    assert_eq!(seen, vec![3, 2, 1]);
    assert_eq!(out.level, 1);
    assert_eq!(
        out.fallbacks,
        vec![(3, "fails at 3".to_string()), (2, "fails at 2".to_string())]
    );
    // Everything fails: terminates at level 0, which is never checked.
    let out = guardrail_loop::<u8, ()>(3, Ok, |_| vec!["always".to_string()]).expect("loop");
    assert_eq!(out.level, 0);
    assert_eq!(out.fallbacks.len(), 3);
    // Compile errors propagate.
    assert!(guardrail_loop::<u8, &str>(2, |_| Err("boom"), |_| Vec::new()).is_err());
}

#[test]
fn finalize_record_marks_dropped_dimensions() {
    let style: StyleProfile = serde_json::from_str(STYLES[2].1).expect("style");
    let intent = CreativeIntent::from_json(INTENTS[0].1).expect("intent");
    let taste = resolve_taste(&intent, &style, None);
    let key = story_key_of(&intent);
    let (_, first) = apply_exploration(&taste, 3, 4, key);
    let (_, mid) = apply_exploration(&taste, 2, 4, key);
    let first: ExploreRecord = first.expect("record");
    let fb = vec![(3u8, "contrast ink/paper 3.00:1 < 4.5:1".to_string())];
    let rec = finalize_record(first.clone(), 2, mid, "grotesk_serif", &fb);
    assert_eq!(rec.level, 3);
    for c in &rec.choices {
        assert_eq!(c.level, 2);
        if c.dimension.level() == 3 {
            assert_eq!(c.value, c.canonical);
            assert_eq!(
                c.fallback.as_deref(),
                Some("guardrail: contrast ink/paper 3.00:1 < 4.5:1")
            );
        } else {
            assert!(c.fallback.is_none());
        }
    }
    // Back to level 0: everything is canonical, typography is what was used.
    let rec0 = finalize_record(
        first,
        0,
        None,
        "grotesk_serif",
        &[(3, "a".into()), (2, "b".into()), (1, "c".into())],
    );
    for c in &rec0.choices {
        assert_eq!(c.level, 0);
        assert!(c.fallback.is_some());
        if c.dimension == ExploreDimension::Typography {
            assert_eq!(c.value, "grotesk_serif");
        } else {
            assert_eq!(c.value, c.canonical);
        }
    }
    assert_eq!(
        rec0.choices
            .iter()
            .find(|c| c.dimension == ExploreDimension::Density)
            .and_then(|c| c.fallback.clone()),
        Some("guardrail: a".to_string())
    );
}

#[test]
fn compiled_explorations_pass_their_own_guardrails() {
    // A compiled exploration satisfies every (relative) check at the level it
    // finally resolved at; fallbacks are recorded with a guardrail reason.
    for (_, style_json) in STYLES {
        let style: StyleProfile = serde_json::from_str(style_json).expect("style");
        let intent = CreativeIntent::from_json(INTENTS[0].1).expect("intent");
        let taste = resolve_taste(&intent, &style, None);
        let canonical = compile(INTENTS[0].1, style_json, &opts(0, 0));
        let g = Guardrails::relative_to(&canonical, &taste.palette.colors);
        for level in 1..=3u8 {
            for seed in 0..6u64 {
                let p = compile(INTENTS[0].1, style_json, &opts(level, seed));
                let rec = p.project.exploration.clone().expect("record");
                let final_level = rec.choices[0].level;
                assert!(final_level <= level);
                let palette = if final_level == 0 {
                    taste.palette.colors.clone()
                } else {
                    apply_exploration(&taste, final_level, seed, story_key_of(&intent))
                        .0
                        .palette
                        .colors
                };
                let report = guardrail_report_with(&p, &palette, &g);
                assert!(report.is_empty(), "level {level} seed {seed}: {report:?}");
                for c in &rec.choices {
                    assert_eq!(c.level, final_level);
                    if let Some(f) = &c.fallback {
                        assert!(f.starts_with("guardrail: "));
                        assert!(c.dimension.level() > final_level);
                    } else {
                        assert!(c.dimension.level() <= final_level);
                    }
                }
            }
        }
    }
}

// ---- audio ------------------------------------------------------------------

fn audio_fixture() -> (
    MotionProject,
    motion_core::compiler::taste::ResolvedStyleProfile,
) {
    let intent = CreativeIntent::from_json(INTENTS[0].1).expect("intent");
    let style: StyleProfile = serde_json::from_str(STYLES[2].1).expect("style");
    let p = compile(INTENTS[0].1, STYLES[2].1, &opts(0, 0));
    (p, resolve_taste(&intent, &style, None))
}

fn sfx_library() -> SfxLibrary {
    let mut sounds = Vec::new();
    for f in SfxFamily::ALL {
        for (k, peak_db) in [-6.0, -9.0, -12.0, -7.5].into_iter().enumerate() {
            let id = format!("{}_{k}", f.as_str());
            sounds.push(SfxSound {
                path: format!("sounds/{}/{id}.wav", f.as_str()),
                id,
                family: f,
                duration: 1.0,
                onset: 0.4,
                peak: 0.5,
                audible_end: 0.9,
                peak_db,
                lufs: None,
                sha256: "0".repeat(64),
                tags: vec![],
            });
        }
    }
    sounds.sort_by(|a, b| a.id.cmp(&b.id));
    SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds,
    }
}

fn shape(plan: &AudioPlan) -> Vec<(String, String, String, u64)> {
    plan.cues
        .iter()
        .map(|c| {
            (
                c.scene.clone(),
                c.kind.as_str().to_string(),
                c.family.as_str().to_string(),
                (c.time * 1000.0).round() as u64,
            )
        })
        .collect()
}

#[test]
fn plan_audio_explored_none_equals_plan_audio_and_seeds_only_change_sounds() {
    let (p, taste) = audio_fixture();
    let lib = sfx_library();
    let energy = vec![Energy::Building; beat_scenes(&p).len()];
    let base = plan_audio(&p, &taste, &energy, &lib, None);
    let none = plan_audio_explored(&p, &taste, &energy, &lib, None, None);
    assert_eq!(base.to_json_pretty(), none.to_json_pretty());
    let mut any_sound_changed = false;
    for seed in 0..12u64 {
        let ex = plan_audio_explored(&p, &taste, &energy, &lib, None, Some(seed));
        assert_eq!(
            shape(&ex),
            shape(&base),
            "seed {seed}: families/times moved"
        );
        any_sound_changed |= ex
            .cues
            .iter()
            .zip(&base.cues)
            .any(|(a, b)| a.sound_id != b.sound_id);
        // Deterministic.
        let again = plan_audio_explored(&p, &taste, &energy, &lib, None, Some(seed));
        assert_eq!(ex.to_json_pretty(), again.to_json_pretty());
    }
    assert!(any_sound_changed, "no seed changed any sound");
}

#[test]
fn compile_path_falls_back_through_the_guardrails() {
    // Scale contrast can shrink furniture labels below 20 px: those seeds must
    // come back one level lower, recorded, and equal the lower-level compile
    // everywhere except the recorded fallback metadata.
    let mut fallbacks = 0;
    for (_, intent) in INTENTS {
        for (_, style) in &STYLES[1..3] {
            for level in 2..=3u8 {
                for seed in 0..10u64 {
                    let p = compile(intent, style, &opts(level, seed));
                    let rec = p.project.exploration.clone().expect("record");
                    let final_level = rec.choices[0].level;
                    if final_level == level {
                        continue;
                    }
                    fallbacks += 1;
                    assert!(rec.choices.iter().any(|c| c.fallback.is_some()));
                    let lower = compile(intent, style, &opts(final_level, seed));
                    let mut a = p.clone();
                    let mut b = lower;
                    a.project.exploration = None;
                    b.project.exploration = None;
                    assert_eq!(bytes(&a), bytes(&b), "fallback differs from L{final_level}");
                }
            }
        }
    }
    assert!(fallbacks > 0, "no guardrail fallback was exercised");
}

/// Two catalog families holding equally good matches for "kettle".
fn tie_library(test: &str) -> AssetLibrary {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/exploration_tests")
        .join(test);
    let _ = std::fs::remove_dir_all(&root);
    for family in ["alpha", "beta", "gamma"] {
        let dir = root.join("library").join(family);
        std::fs::create_dir_all(&dir).expect("dir");
        let id = format!("{family}_kettle");
        let catalog = serde_json::json!({ "version": "0.1", "family": family, "assets": [{
            "id": id, "file": "k.png", "role": "object", "subject": "kettle",
            "tags": ["kettle"], "qa": "PASS" }] });
        let manifest = serde_json::json!({ "version": "0.2", "assets": [{
            "id": format!("library.{id}"), "path": "k.png", "width": 64, "height": 64,
            "alpha": true, "serves": [format!("library.{id}")] }] });
        std::fs::write(dir.join("catalog.json"), catalog.to_string()).expect("catalog");
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).expect("manifest");
    }
    AssetLibrary::new(root).with_families(vec!["alpha".into(), "beta".into(), "gamma".into()])
}

#[test]
fn asset_family_ties_follow_order_until_explored() {
    use motion_core::compiler::catalog::{request_words, MatchKind};
    let words = request_words(&["kettle"]);
    let lib = tie_library("ties");
    // Canonical and levels below 2: family order.
    let first = lib.catalog_match(&words, MatchKind::Object).expect("match");
    assert_eq!(first.family, "alpha");
    for level in [0u8, 1] {
        let l = lib.clone().with_exploration(level, 99);
        assert_eq!(
            l.catalog_match(&words, MatchKind::Object),
            Some(first.clone())
        );
    }
    // Level >= 2: a deterministic pick among the tied families, all reachable.
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..40u64 {
        let l = lib.clone().with_exploration(2, seed);
        let a = l.catalog_match(&words, MatchKind::Object).expect("match");
        let b = l.catalog_match(&words, MatchKind::Object).expect("match");
        assert_eq!(a, b);
        seen.insert(a.family);
    }
    assert_eq!(seen.len(), 3, "families reached: {seen:?}");
}

fn idx<T: PartialEq + Copy>(order: &[T], v: T) -> i32 {
    order.iter().position(|x| *x == v).expect("in order") as i32
}
