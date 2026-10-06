//! (0.23 B3) The best-of-N scorer (`motion_render::score`) on the benchmark
//! stories, compiled the way the product path compiles (`--art auto --variety
//! auto --speech <fixture>`), plus synthetic failures for the hard-check and
//! shipped-with-failure paths.
//!
//! The tests run at a 540-wide canvas so the pixel stage stays quick in a debug
//! build; `bench_matrix` (ignored) runs the full matrix at the product canvas
//! and prints the per-check hard-failure table and the per-candidate cost:
//!
//! `cargo test --release -p motion-render --test score bench_matrix -- --ignored --nocapture`
//!
//! Frames are rendered in memory by the pixel check and never written.

use std::path::PathBuf;
use std::time::Instant;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::direction::{candidate_seed, take_seed};
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile_candidate, compile_with_report, AssetLibrary, CompileError, CompileOptions,
    CompileWarning, FontSet,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{LayerKind, MotionProject};
use motion_core::speech::{repair, SpeechMap};
use motion_core::style::StyleProfile;
use motion_render::score::{
    best_of, pixel_hard, structural, BestOfInput, HARD_COMPILE, W_BALANCE, W_DEAD_AIR, W_FOCAL,
};
use motion_render::FontMeasure;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read_repo(path: &str) -> String {
    let full = repo().join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

/// The CLI's `--variety auto` seed (FNV-1a over the title and statements).
fn story_seed(intent: &CreativeIntent) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |s: &str| {
        for b in s.bytes().chain([0u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    eat(&intent.title);
    for b in &intent.beats {
        eat(&b.statement);
    }
    h
}

/// The eight benchmark stories (`scripts/variety_bench.sh`).
const STORIES: [(&str, &str); 8] = [
    (
        "sleep_review",
        "docs/plans/sprint_0_23/stories/sleep_review.intent.json",
    ),
    (
        "money_review",
        "docs/plans/sprint_0_23/stories/money_review.intent.json",
    ),
    (
        "collection-accumulate",
        "examples/public/collection-accumulate.intent.json",
    ),
    ("state-change", "examples/public/state-change.intent.json"),
    (
        "derived-metric",
        "examples/public/derived-metric.intent.json",
    ),
    ("layers", "examples/public/layers.intent.json"),
    ("ai_learns", "examples/topics/ai_learns.intent.json"),
    ("space", "examples/cinematic/space.intent.json"),
];

const TONES: [&str; 9] = [
    "auto",
    "editorial",
    "technical",
    "playful",
    "street",
    "documentary",
    "hype",
    "studio",
    "cinematic",
];

struct Story {
    intent: CreativeIntent,
    speech: SpeechMap,
}

fn story(name: &str) -> Story {
    let (name, path) = STORIES
        .iter()
        .find(|(n, _)| *n == name)
        .copied()
        .unwrap_or_else(|| panic!("no story {name}"));
    let intent = CreativeIntent::from_json(&read_repo(path)).expect("intent");
    let map = SpeechMap::from_json(&read_repo(&format!(
        "golden/fixtures/variety/{name}.speech.json"
    )))
    .expect("speech fixture");
    let spoken: Vec<String> = intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect();
    let speech = repair(&map, &spoken).0;
    Story { intent, speech }
}

fn tone_style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!("{{\"tone\": \"{tone}\"}}")).expect("style")
}

/// The product path's compile options for a story and take.
fn options(s: &Story, take: u64, variety: bool, canvas: Option<(u32, u32)>) -> CompileOptions {
    CompileOptions {
        art: Some(ArtMode::Auto),
        variety: variety.then(|| story_seed(&s.intent)),
        take,
        speech: Some(s.speech.clone()),
        canvas,
        ..CompileOptions::default()
    }
}

type Compiled = Result<(MotionProject, Vec<CompileWarning>), CompileError>;

/// Candidate `k` of a story, with its own font measure (the measure is not
/// shareable across threads), exactly as `compile --candidates` does.
fn compile_k(s: &Story, tone: &str, opts: &CompileOptions, k: u8) -> Compiled {
    let assets = repo().join("assets");
    let style = tone_style(tone);
    let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure = FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path))
        .map_err(|e| CompileError::Invalid(format!("fonts: {e}")))?;
    compile_candidate(
        &s.intent,
        &style,
        None,
        &AssetLibrary::new(&assets),
        &measure,
        &AssetManifest::empty(),
        None,
        opts,
        k,
    )
}

/// The directory the scene's `asset_root` ("assets") is relative to.
fn base() -> &'static std::path::Path {
    static BASE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BASE.get_or_init(|| repo().canonicalize().expect("workspace root"))
}

fn input<'a>(s: &'a Story, opts: &CompileOptions, n: u8) -> BestOfInput<'a> {
    BestOfInput {
        candidates: n,
        take_seed: take_seed(opts.variety.unwrap_or(0), opts.take),
        speech: Some(&s.speech),
        base_dir: base(),
        asset_root: "assets",
    }
}

const SMALL: Option<(u32, u32)> = Some((540, 960));

// ---------------------------------------------------------------------------
// Criterion 1: candidate 0 is the plain compile
// ---------------------------------------------------------------------------

#[test]
fn candidate_zero_is_compile_with_report_byte_for_byte() {
    for (name, tone) in [
        ("sleep_review", "playful"),
        ("money_review", "cinematic"),
        ("derived-metric", "editorial"),
    ] {
        let s = story(name);
        for variety in [true, false] {
            let opts = options(&s, 0, variety, SMALL);
            let style = tone_style(tone);
            let assets = repo().join("assets");
            let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
                .faces
                .iter()
                .map(|f| assets.join(f.path))
                .collect();
            let measure =
                FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("fonts");
            let plain = compile_with_report(
                &s.intent,
                &style,
                None,
                &AssetLibrary::new(&assets),
                &measure,
                &AssetManifest::empty(),
                None,
                &opts,
            )
            .expect("plain compile");
            let k0 = compile_k(&s, tone, &opts, 0).expect("candidate 0");
            assert_eq!(
                plain.0.to_json_pretty(),
                k0.0.to_json_pretty(),
                "{name} {tone} variety={variety}: candidate 0 differs"
            );
            assert_eq!(plain.1, k0.1, "{name} {tone}: warnings differ");
        }
    }
}

#[test]
fn other_candidates_differ_only_under_variety() {
    let s = story("sleep_review");
    // Without a variety seed there is no direction seed: every k is the same.
    let opts = options(&s, 0, false, SMALL);
    let a = compile_k(&s, "playful", &opts, 0).expect("k0").0;
    let b = compile_k(&s, "playful", &opts, 3).expect("k3").0;
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
    // With one, some candidate of some story differs from candidate 0.
    let mut differs = false;
    for name in ["sleep_review", "money_review", "ai_learns"] {
        let s = story(name);
        let opts = options(&s, 0, true, SMALL);
        let base = compile_k(&s, "playful", &opts, 0).expect("k0").0;
        for k in 1..4 {
            let other = compile_k(&s, "playful", &opts, k).expect("k").0;
            differs |= base.to_json_pretty() != other.to_json_pretty();
        }
    }
    assert!(differs, "no candidate of three stories differs from k = 0");
}

// ---------------------------------------------------------------------------
// Criterion 2: determinism
// ---------------------------------------------------------------------------

#[test]
fn best_of_four_is_deterministic_across_runs() {
    let s = story("sleep_review");
    let opts = options(&s, 0, true, SMALL);
    let run =
        || best_of(&input(&s, &opts, 4), |k| compile_k(&s, "playful", &opts, k)).expect("best_of");
    let first = run();
    for _ in 0..2 {
        let again = run();
        assert_eq!(first.chosen, again.chosen);
        assert_eq!(first.scores, again.scores);
        assert_eq!(
            first.project.to_json_pretty(),
            again.project.to_json_pretty(),
            "the chosen scene differs between runs"
        );
    }
    // The record carries every candidate and the choice.
    let d = first.project.project.direction.as_ref().expect("direction");
    assert_eq!(d.chosen, first.chosen);
    assert_eq!(d.candidates.len(), 4);
    assert!(d.shipped_with_failure.is_none());
    let seeds: Vec<u64> = d.candidates.iter().map(|c| c.seed).collect();
    let expected: Vec<u64> = (0..4)
        .map(|k| candidate_seed(take_seed(opts.variety.unwrap_or(0), 0), k))
        .collect();
    assert_eq!(seeds, expected);
    assert!(d.candidates.iter().all(|c| c.parts.len() == 5));
    // The structural top 2 were judged on pixels.
    assert!(d.candidates.iter().filter(|c| c.pixel).count() >= 1);
    // The chosen one is clean with the highest soft score among the clean.
    let chosen = &d.candidates[usize::from(d.chosen)];
    assert!(chosen.hard.is_empty(), "{chosen:?}");
    for c in d.candidates.iter().filter(|c| c.hard.is_empty()) {
        assert!(c.soft <= chosen.soft + 1e-9, "{c:?} beats {chosen:?}");
    }
}

#[test]
fn the_soft_score_has_its_five_documented_parts() {
    let s = story("money_review");
    let opts = options(&s, 0, true, SMALL);
    let (p, w) = compile_k(&s, "documentary", &opts, 0).expect("compile");
    let st = structural(&p, &w, Some(&s.speech));
    let names: Vec<&str> = st.parts.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["balance", "dead_air", "density", "focal", "variety"]
    );
    let sum: f64 = st.parts.values().sum();
    assert!((sum - st.soft).abs() < 1e-5, "{sum} vs {}", st.soft);
    assert!(st.parts["balance"] <= 0.0 && st.parts["balance"] >= -W_BALANCE * 20.0);
    assert!(st.parts["dead_air"] <= 0.0 && st.parts["dead_air"] >= -W_DEAD_AIR * 100.0);
    assert!(st.parts["density"] <= 0.0 && st.parts["variety"] <= 0.0);
    assert!((0.0..=W_FOCAL).contains(&st.parts["focal"]));
    // No negative zeros in the record.
    assert!(st.parts.values().all(|v| v.is_sign_positive() || *v != 0.0));
}

// ---------------------------------------------------------------------------
// Hard checks and the choice, with synthetic failures
// ---------------------------------------------------------------------------

/// Delay every content motion of beat 1 so the video opens on nothing: the
/// project then fails `dead_air`.
fn make_dead_air(mut p: MotionProject) -> MotionProject {
    const DELAY: f64 = 2.5;
    for scene in p.scenes.iter_mut().filter(|s| s.id == "beat_1") {
        let duration = scene.duration_seconds;
        for m in &mut scene.motions {
            // The arrival only, and never past the end of the scene (the
            // project must still validate).
            if m.start < 1.5 && m.start + m.duration + DELAY <= duration {
                m.start += DELAY;
            }
        }
    }
    p
}

/// Paint every beat-1 headline text in the canvas background colour: the
/// text is invisible, which only the pixel check sees.
fn make_invisible_text(mut p: MotionProject) -> MotionProject {
    fn walk(layers: &mut [motion_core::scene::Layer], c: motion_core::scene::Color) {
        for l in layers {
            match &mut l.kind {
                LayerKind::Text(t) => t.color = c,
                LayerKind::Group { children } => walk(children, c),
                _ => {}
            }
        }
    }
    let bg = p.canvas.background;
    for scene in p.scenes.iter_mut().filter(|s| s.id.starts_with("beat_")) {
        walk(&mut scene.layers, bg);
    }
    p
}

#[test]
fn a_failing_candidate_is_dropped_while_a_clean_one_exists() {
    let s = story("sleep_review");
    let opts = options(&s, 0, true, SMALL);
    let got = best_of(&input(&s, &opts, 4), |k| {
        let (p, w) = compile_k(&s, "playful", &opts, k)?;
        // Candidates 0 and 2 open on dead air; 1 and 3 are untouched.
        Ok(if k % 2 == 0 {
            (make_dead_air(p), w)
        } else {
            (p, w)
        })
    })
    .expect("best_of");
    assert!(got.chosen == 1 || got.chosen == 3, "chose {}", got.chosen);
    assert!(got.shipped_with_failure.is_none());
    assert_eq!(got.dropped, 2);
    for k in [0usize, 2] {
        assert!(
            got.scores[k].hard.contains(&"dead_air".to_string()),
            "{:?}",
            got.scores[k]
        );
        assert!(
            !got.scores[k].pixel,
            "failing candidates skip the pixel stage"
        );
    }
    assert!(got.scores[usize::from(got.chosen)].hard.is_empty());
    assert!(got.summary().starts_with("candidates: 4, chose "));
    assert!(got.summary().ends_with("2 hard failures dropped)"));
}

#[test]
fn the_pixel_check_drops_invisible_text() {
    let s = story("sleep_review");
    let opts = options(&s, 0, true, SMALL);
    // Candidate 0 would win on k alone (all scores tie in structure), but its
    // text is invisible; the pixel stage must move on to a clean one.
    let got = best_of(&input(&s, &opts, 4), |k| {
        let (p, w) = compile_k(&s, "playful", &opts, k)?;
        Ok(if k == 0 {
            (make_invisible_text(p), w)
        } else {
            (p, w)
        })
    })
    .expect("best_of");
    assert_ne!(got.chosen, 0);
    assert!(got.scores[0].pixel, "{:?}", got.scores[0]);
    assert!(
        got.scores[0]
            .hard
            .contains(&"text_local_contrast".to_string()),
        "{:?}",
        got.scores[0]
    );
    assert!(got.shipped_with_failure.is_none());
}

#[test]
fn every_candidate_failing_dead_air_ships_the_best_with_the_field_set() {
    let s = story("sleep_review");
    let opts = options(&s, 0, true, SMALL);
    let got = best_of(&input(&s, &opts, 3), |k| {
        let (p, w) = compile_k(&s, "playful", &opts, k)?;
        Ok((make_dead_air(p), w))
    })
    .expect("best_of");
    assert_eq!(
        got.shipped_with_failure.as_deref(),
        Some("every candidate failed: dead_air")
    );
    let d = got.project.project.direction.as_ref().expect("direction");
    assert_eq!(
        d.shipped_with_failure.as_deref(),
        Some("every candidate failed: dead_air")
    );
    assert_eq!(d.chosen, got.chosen);
    assert_eq!(d.candidates.len(), 3);
    // The best of a failing set: the highest soft score (dead-air seconds
    // cost the same here), the lowest k on a tie.
    let best = d
        .candidates
        .iter()
        .min_by(|a, b| b.soft.total_cmp(&a.soft).then(a.k.cmp(&b.k)))
        .expect("candidates");
    assert_eq!(best.k, got.chosen);
}

#[test]
fn a_candidate_that_does_not_compile_is_a_hard_failure() {
    let s = story("sleep_review");
    let opts = options(&s, 0, true, SMALL);
    let got = best_of(&input(&s, &opts, 3), |k| {
        if k == 0 {
            Err(CompileError::Invalid("synthetic".into()))
        } else {
            compile_k(&s, "playful", &opts, k)
        }
    })
    .expect("best_of");
    assert_ne!(got.chosen, 0);
    assert_eq!(got.scores[0].hard, [HARD_COMPILE]);
    // When none compiles the error of candidate 0 is returned.
    let err = best_of(&input(&s, &opts, 2), |_| {
        Err::<(MotionProject, Vec<CompileWarning>), _>(CompileError::NoBeats)
    })
    .expect_err("nothing compiled");
    assert!(matches!(err, CompileError::NoBeats), "{err}");
}

#[test]
fn an_uncompiled_candidate_never_ships_while_another_compiled() {
    let s = story("sleep_review");
    let opts = options(&s, 0, true, SMALL);
    // k = 1 does not compile; k = 0 and 2 compile but open on dead air. The
    // uncompiled candidate must not win the all-failing fallback.
    let got = best_of(&input(&s, &opts, 3), |k| {
        if k == 1 {
            return Err(CompileError::Invalid("synthetic".into()));
        }
        let (p, w) = compile_k(&s, "playful", &opts, k)?;
        Ok((make_dead_air(p), w))
    })
    .expect("a compiled candidate exists, so best_of must not fail");
    assert_ne!(got.chosen, 1);
    assert_eq!(
        got.shipped_with_failure.as_deref(),
        Some("every candidate failed: dead_air")
    );
    assert_eq!(got.scores[1].hard, [HARD_COMPILE]);
    assert!(got.summary().contains("every candidate failed"));
}

#[test]
fn pixel_hard_is_clean_on_a_normal_candidate() {
    let s = story("sleep_review");
    let opts = options(&s, 0, true, SMALL);
    let (mut p, _) = compile_k(&s, "playful", &opts, 0).expect("compile");
    p.asset_root = Some("assets".into());
    assert!(pixel_hard(&p, base()).is_empty());
    let p = make_invisible_text(p);
    assert_eq!(pixel_hard(&p, base()), ["text_local_contrast"]);
}

// ---------------------------------------------------------------------------
// Criteria 3 and 4: the bench matrix (ignored; run in release)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "bench: 8 stories x 9 tones x takes 0..=2 at N = 4; run with --release --nocapture"]
fn bench_matrix() {
    use std::collections::BTreeMap;
    let mut n1: BTreeMap<String, usize> = BTreeMap::new();
    let mut n4: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_candidate: BTreeMap<String, usize> = BTreeMap::new();
    let (mut candidates_total, mut candidates_failing, mut pixel_checked, mut rescued) =
        (0usize, 0usize, 0usize, 0usize);
    let (mut cases, mut changed, mut bad_ship, mut n1_bad, mut n4_bad) = (0, 0, 0, 0, 0);
    let (mut compile_ms, mut compile_ms_by_look): (Vec<f64>, BTreeMap<String, Vec<f64>>) =
        (Vec::new(), BTreeMap::new());
    let (mut t_structural, mut t_pixel, mut t_n4) = (0.0f64, 0.0f64, 0.0f64);
    for (name, _) in STORIES {
        let s = story(name);
        for tone in TONES {
            for take in 0..3u64 {
                let opts = options(&s, take, true, None);
                cases += 1;
                // N = 1: candidate 0 judged on everything (structural + pixel).
                let t = Instant::now();
                let one = compile_k(&s, tone, &opts, 0);
                let Ok((mut p0, w0)) = one else {
                    eprintln!("{name} {tone} take {take}: candidate 0 does not compile");
                    *n1.entry(HARD_COMPILE.into()).or_default() += 1;
                    *n4.entry(HARD_COMPILE.into()).or_default() += 0;
                    continue;
                };
                compile_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                let look = p0
                    .project
                    .art
                    .as_ref()
                    .map_or("none".to_string(), |a| a.look.clone());
                compile_ms_by_look
                    .entry(look)
                    .or_default()
                    .push(t.elapsed().as_secs_f64() * 1000.0);
                p0.asset_root = Some("assets".into());
                // Both stages always run on the N = 1 scene, so every check is
                // counted for it (best_of skips the pixel stage of a candidate
                // that already fails structurally).
                let t = Instant::now();
                let mut hard0 = structural(&p0, &w0, Some(&s.speech)).hard;
                t_structural += t.elapsed().as_secs_f64();
                let t = Instant::now();
                hard0.extend(pixel_hard(&p0, base()));
                t_pixel += t.elapsed().as_secs_f64();
                for h in &hard0 {
                    *n1.entry(h.clone()).or_default() += 1;
                }
                n1_bad += usize::from(!hard0.is_empty());
                // N = 4.
                let t = Instant::now();
                let got = best_of(&input(&s, &opts, 4), |k| compile_k(&s, tone, &opts, k))
                    .expect("best_of");
                t_n4 += t.elapsed().as_secs_f64();
                let shipped = &got.scores[usize::from(got.chosen)];
                // The shipped scene judged on everything, like the N = 1 one.
                let mut shipped_hard = shipped.hard.clone();
                if !shipped.pixel {
                    shipped_hard.extend(pixel_hard(&got.project, base()));
                }
                for h in &shipped_hard {
                    *n4.entry(h.clone()).or_default() += 1;
                }
                n4_bad += usize::from(!shipped_hard.is_empty());
                candidates_total += got.scores.len();
                for c in &got.scores {
                    candidates_failing += usize::from(!c.hard.is_empty());
                    pixel_checked += usize::from(c.pixel);
                    for h in &c.hard {
                        *per_candidate.entry(h.clone()).or_default() += 1;
                    }
                }
                rescued += usize::from(!hard0.is_empty() && shipped_hard.is_empty());
                changed += usize::from(got.chosen != 0);
                if !shipped_hard.is_empty() && got.scores.iter().any(|c| c.hard.is_empty()) {
                    bad_ship += 1;
                    eprintln!("{name} {tone} take {take}: shipped a failing candidate");
                }
                eprintln!(
                    "{name:22} {tone:12} take {take} chose {} soft {:6.2} hard {:?} | n1 hard {:?}",
                    got.chosen, shipped.soft, shipped_hard, hard0
                );
            }
        }
    }
    let mut v = compile_ms.clone();
    v.sort_by(f64::total_cmp);
    eprintln!("\ncases {cases}; chose k != 0 in {changed}");
    eprintln!("scenes with a hard failure: N=1 {n1_bad}, N=4 {n4_bad}");
    eprintln!("hard failures per check, N=1: {n1:?}");
    eprintln!("hard failures per check, N=4: {n4:?}");
    eprintln!(
        "candidates judged {candidates_total}, with a hard failure {candidates_failing} \
         ({per_candidate:?}), pixel-checked {pixel_checked}; scenes rescued (N=1 failed, N=4 clean) {rescued}"
    );
    eprintln!("hard-failing candidate shipped while a clean one existed: {bad_ship}");
    eprintln!(
        "candidate-0 compile ms: median {:.1}, max {:.1} (n = {})",
        v[v.len() / 2],
        v[v.len() - 1],
        v.len()
    );
    for (look, mut ms) in compile_ms_by_look {
        ms.sort_by(f64::total_cmp);
        eprintln!(
            "  {look:20} median {:6.1} ms  max {:6.1} ms  (n = {})",
            ms[ms.len() / 2],
            ms[ms.len() - 1],
            ms.len()
        );
    }
    eprintln!(
        "per candidate: structural stage {:.0} ms, pixel stage {:.0} ms (means over {cases} cases); \
         best_of N=4 total {t_n4:.1}s ({:.2}s per take)",
        t_structural / cases as f64 * 1000.0,
        t_pixel / cases as f64 * 1000.0,
        t_n4 / cases as f64
    );
    assert_eq!(bad_ship, 0);
    assert!(n4_bad <= n1_bad, "N=4 ships more failing scenes than N=1");
}

/// Calibration dump (ignored): the distributions the weights and the per-look
/// density maxima in `score.rs` were set from.
#[test]
#[ignore = "calibration dump; run with --release --nocapture"]
fn calibration_dump() {
    use motion_render::score::{phase_counts, read_measures};
    use std::collections::BTreeMap;
    let mut phases: BTreeMap<String, [Vec<usize>; 4]> = BTreeMap::new();
    let (mut focal, mut text): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    for (name, _) in STORIES {
        let s = story(name);
        for tone in TONES {
            let opts = options(&s, 0, true, None);
            for k in 0..4u8 {
                let Ok((p, _)) = compile_k(&s, tone, &opts, k) else {
                    continue;
                };
                let look = p
                    .project
                    .art
                    .as_ref()
                    .map_or("none".to_string(), |a| a.look.clone());
                for scene in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
                    if let Some(c) = phase_counts(scene) {
                        let row = phases.entry(look.clone()).or_default();
                        row[0].push(c.enter);
                        row[1].push(c.read);
                        row[2].push(c.evolve);
                        row[3].push(c.anticipate);
                    }
                }
                if let Ok(ms) = read_measures(&p) {
                    for m in ms {
                        focal.extend(m.focal_share);
                        text.push(m.text_share);
                    }
                }
            }
        }
    }
    let q = |v: &mut Vec<usize>, f: f64| {
        v.sort_unstable();
        v[((v.len() - 1) as f64 * f).round() as usize]
    };
    for (look, row) in phases.iter_mut() {
        let mut line = format!("{look:20} n={:3}", row[0].len());
        for (label, v) in ["enter", "read", "evolve", "anticipate"]
            .iter()
            .zip(row.iter_mut())
        {
            let zeros = v.iter().filter(|n| **n == 0).count();
            line += &format!(
                " | {label} p50/p90/max {}/{}/{} zero {zeros}",
                q(v, 0.5),
                q(v, 0.9),
                q(v, 1.0)
            );
        }
        eprintln!("{line}");
    }
    let pct = |v: &mut Vec<f64>, f: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * f).round() as usize]
    };
    for (label, v) in [("focal share", &mut focal), ("text share", &mut text)] {
        eprintln!(
            "{label}: n={} p05 {:.2} p25 {:.2} p50 {:.2} p75 {:.2} p95 {:.2} max {:.2}",
            v.len(),
            pct(v, 0.05),
            pct(v, 0.25),
            pct(v, 0.5),
            pct(v, 0.75),
            pct(v, 0.95),
            pct(v, 1.0)
        );
    }
}
