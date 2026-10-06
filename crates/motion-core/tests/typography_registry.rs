//! (0.9 Phase 1) Emotion typography registry: validator, legacy FontSet
//! identity, emotion table, exploration levels, availability fallback and the
//! compile integration.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use motion_core::assets::AssetManifest;
use motion_core::compiler::taste::{
    resolve_with, ResolvedStyleProfile, ResolvedTemperature, ResolvedTone, TemperamentKind,
    TypographyPairing,
};
use motion_core::compiler::typography::{
    option_faces, option_fontset, option_fontset_in, resolve_emotion, resolve_typography,
    role_name, validate_registry, validate_registry_str, Emotion, TypographyChoice,
    TypographyRequest,
};
use motion_core::compiler::{
    compile_full, compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, FontSet,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::FontRole;
use motion_core::style::{StyleProfile, Temperament, Temperature, Tone};
use serde_json::{json, Value};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn assets() -> PathBuf {
    repo().join("assets")
}

fn registry_json() -> Value {
    let text = std::fs::read_to_string(assets().join("fonts/registry.json")).expect("registry");
    serde_json::from_str(&text).expect("registry parses")
}

fn demo_intent() -> CreativeIntent {
    let text = std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json"))
        .expect("intent");
    CreativeIntent::from_json(&text).expect("intent parses")
}

fn taste_of(style: &StyleProfile) -> ResolvedStyleProfile {
    resolve_with(style, None, 0)
}

fn editorial_cool() -> ResolvedStyleProfile {
    let t = taste_of(&StyleProfile {
        tone: Tone::Editorial,
        temperature: Temperature::Cool,
        temperament: Temperament::Balanced,
        ..StyleProfile::default()
    });
    assert_eq!(resolve_emotion(&t), Emotion::Trust);
    t
}

fn req(level: u8, seed: u64, emotion: Option<Emotion>) -> TypographyRequest {
    TypographyRequest {
        exploration: level,
        seed,
        story_key: 42,
        emotion,
    }
}

// ---------------------------------------------------------------------------
// Validator
// ---------------------------------------------------------------------------

#[test]
fn registry_validates_and_pending_faces_are_unavailable() {
    let report = validate_registry(&assets());
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert_eq!(report.options.len(), 33);
    assert_eq!(report.faces.len(), 55);
    for o in &report.options {
        let faces = option_faces(&o.id).expect("complete option");
        let pending = faces.values().any(|f| {
            // Necto Mono is substituted by IBM Plex Mono while absent.
            f.asset_id.starts_with("font.sprat_") || f.asset_id.starts_with("font.mazius_")
        });
        assert_eq!(
            o.available, !pending,
            "{}: available={} problems={:?}",
            o.id, o.available, o.problems
        );
    }
}

fn mutated(f: impl FnOnce(&mut Value)) -> Vec<String> {
    let mut v = registry_json();
    f(&mut v);
    validate_registry_str(&v.to_string(), &assets()).errors
}

#[test]
fn validator_flags_broken_registries() {
    let e = mutated(|v| {
        v["emotions"]["trust"][0]["roles"]
            .as_object_mut()
            .unwrap()
            .remove("mono");
    });
    assert!(
        e.iter()
            .any(|m| m.contains("trust.1") && m.contains("missing role mono")),
        "{e:?}"
    );

    let e = mutated(|v| v["emotions"]["trust"][0]["roles"]["body"] = json!("font.nope"));
    assert!(
        e.iter().any(|m| m.contains("unknown face font.nope")),
        "{e:?}"
    );

    let e = mutated(|v| {
        v["legacy_pairings"]["poster_bold"]
            .as_object_mut()
            .unwrap()
            .remove("number");
    });
    assert!(
        e.iter()
            .any(|m| m.contains("poster_bold") && m.contains("missing role number")),
        "{e:?}"
    );

    let e = mutated(|v| v["adjacency"][0] = json!(["trust", "bogus"]));
    assert!(
        e.iter()
            .any(|m| m.contains("adjacency") && m.contains("bogus")),
        "{e:?}"
    );

    let e = mutated(|v| v["tone_emotions"]["classic"] = json!(["energy", "bogus"]));
    assert!(
        e.iter()
            .any(|m| m.contains("tone_emotions.classic") && m.contains("bogus")),
        "{e:?}"
    );

    // Body face without digits.
    let e = mutated(|v| v["emotions"]["trust"][0]["roles"]["body"] = json!("font.frantically"));
    assert!(
        e.iter()
            .any(|m| m.contains("trust.1") && m.contains("digit")),
        "{e:?}"
    );
    // Number face without digits.
    let e = mutated(|v| v["emotions"]["trust"][0]["roles"]["number"] = json!("font.frantically"));
    assert!(
        e.iter()
            .any(|m| m.contains("trust.1") && m.contains("digit")),
        "{e:?}"
    );

    assert!(mutated(|_| {}).is_empty());
}

// ---------------------------------------------------------------------------
// Legacy FontSet identity
// ---------------------------------------------------------------------------

fn ids(set: &FontSet) -> Vec<&'static str> {
    set.faces.iter().map(|f| f.asset_id).collect()
}

#[test]
fn legacy_font_sets_are_unchanged() {
    // Hard-coded from theme.rs as of 0.8 (role faces + always-loaded extras
    // [+ plex_mono for the 0.8 sets], sorted by asset id, deduplicated).
    let expected: [(TypographyPairing, &[&str]); 7] = [
        (
            TypographyPairing::GroteskSerif,
            &[
                "font.anton",
                "font.archivo_black",
                "font.dm_serif",
                "font.dm_serif_italic",
                "font.fira_sans",
                "font.fira_sans_semibold",
                "font.space_mono",
                "font.space_mono_bold",
            ],
        ),
        (
            TypographyPairing::SerifSans,
            &[
                "font.anton",
                "font.dm_serif",
                "font.dm_serif_italic",
                "font.fira_sans",
                "font.fira_sans_semibold",
                "font.space_mono",
                "font.space_mono_bold",
            ],
        ),
        (
            TypographyPairing::PosterBold,
            &[
                "font.archivo_black",
                "font.dm_serif",
                "font.dm_serif_italic",
                "font.fira_sans_semibold",
                "font.space_mono_bold",
            ],
        ),
        (
            TypographyPairing::HumanistSerif,
            &[
                "font.barlow_condensed_semibold",
                "font.dm_serif",
                "font.dm_serif_italic",
                "font.fira_sans_semibold",
                "font.lato",
                "font.plex_mono",
                "font.space_mono_bold",
            ],
        ),
        (
            TypographyPairing::PrecisionGrotesk,
            &[
                "font.barlow_condensed_bold",
                "font.barlow_condensed_semibold",
                "font.barlow_medium",
                "font.dm_serif",
                "font.dm_serif_italic",
                "font.fira_sans_semibold",
                "font.plex_mono",
                "font.plex_mono_medium",
                "font.plex_serif_italic",
                "font.space_mono_bold",
            ],
        ),
        (
            TypographyPairing::FriendlyGeometric,
            &[
                "font.dm_serif",
                "font.dm_serif_italic",
                "font.fira_sans_semibold",
                "font.plex_mono",
                "font.plex_mono_semibold",
                "font.poppins_black",
                "font.poppins_bold",
                "font.poppins_extrabold",
                "font.poppins_medium",
                "font.space_mono_bold",
                "font.zilla_slab_semibold_italic",
            ],
        ),
        (
            TypographyPairing::CondensedMono,
            &[
                "font.anton",
                "font.bebas_neue",
                "font.dm_serif",
                "font.dm_serif_italic",
                "font.fira_sans_semibold",
                "font.space_mono",
                "font.space_mono_bold",
            ],
        ),
    ];
    for (pairing, want) in expected {
        let set = FontSet::for_pairing(pairing);
        let mut sorted = want.to_vec();
        sorted.sort_unstable();
        assert_eq!(ids(&set), sorted, "{pairing:?}");
        assert_eq!(set.roles.len(), 6);
    }
}

#[test]
fn legacy_pairings_match_the_registry_data() {
    let reg = registry_json();
    let faces = reg["faces"].as_array().unwrap();
    let names = [
        (TypographyPairing::GroteskSerif, "grotesk_serif"),
        (TypographyPairing::SerifSans, "serif_sans"),
        (TypographyPairing::PosterBold, "poster_bold"),
        (TypographyPairing::HumanistSerif, "humanist_serif"),
        (TypographyPairing::PrecisionGrotesk, "precision_grotesk"),
        (TypographyPairing::FriendlyGeometric, "friendly_geometric"),
        (TypographyPairing::CondensedMono, "condensed_mono"),
    ];
    for (pairing, name) in names {
        let set = FontSet::for_pairing(pairing);
        for role in FontRole::ALL {
            let want_id = reg["legacy_pairings"][name][role_name(role)]
                .as_str()
                .unwrap();
            let face = set.role(role);
            assert_eq!(face.asset_id, want_id, "{name} {role:?}");
            let rf = faces.iter().find(|f| f["id"] == want_id).unwrap();
            assert_eq!(face.path, rf["file"].as_str().unwrap());
            assert_eq!(u64::from(face.weight), rf["weight"].as_u64().unwrap());
            assert_eq!(face.italic, rf["italic"].as_bool().unwrap());
        }
    }
}

#[test]
fn option_font_sets_carry_fallback_and_always_loaded() {
    let set = option_fontset("trust.3").expect("trust.3");
    let got = ids(&set);
    let mut sorted = got.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(got, sorted, "sorted + deduped");
    for must in [
        "font.plex_mono",
        "font.dm_serif",
        "font.dm_serif_italic",
        "font.fira_sans_semibold",
        "font.space_mono_bold",
        "font.instrument_serif",
        "font.satoshi_regular",
    ] {
        assert!(got.contains(&must), "{must} in {got:?}");
    }
    assert_eq!(set.roles.len(), 6);
    assert_eq!(
        set.role(FontRole::Display).asset_id,
        "font.instrument_serif"
    );
}

// ---------------------------------------------------------------------------
// resolve_emotion table
// ---------------------------------------------------------------------------

fn emotion_for(tone: ResolvedTone, kind: TemperamentKind, temp: ResolvedTemperature) -> Emotion {
    let mut t = taste_of(&StyleProfile::default());
    t.tone = tone;
    t.motion.kind = kind;
    t.palette.temperature = temp;
    resolve_emotion(&t)
}

#[test]
fn emotion_table() {
    use Emotion::*;
    use ResolvedTemperature::{Cool, Warm};
    use ResolvedTone::*;
    use TemperamentKind as K;
    for temp in [Warm, Cool] {
        let warm = temp == Warm;
        // editorial
        assert_eq!(emotion_for(Editorial, K::Restrained, temp), Calm);
        assert_eq!(
            emotion_for(Editorial, K::Editorial, temp),
            if warm { Warmth } else { Trust }
        );
        assert_eq!(
            emotion_for(Editorial, K::Precise, temp),
            if warm { Warmth } else { Trust }
        );
        assert_eq!(emotion_for(Editorial, K::Energetic, temp), Drama);
        // technical
        assert_eq!(emotion_for(Technical, K::Restrained, temp), Calm);
        assert_eq!(emotion_for(Technical, K::Editorial, temp), Precision);
        assert_eq!(emotion_for(Technical, K::Precise, temp), Precision);
        assert_eq!(emotion_for(Technical, K::Energetic, temp), Energy);
        // playful
        assert_eq!(emotion_for(Playful, K::Restrained, temp), Warmth);
        assert_eq!(emotion_for(Playful, K::Editorial, temp), Joy);
        assert_eq!(emotion_for(Playful, K::Precise, temp), PlayfulRetro);
        assert_eq!(emotion_for(Playful, K::Energetic, temp), Energy);
        // classic (reporting only)
        assert_eq!(emotion_for(Classic, K::Energetic, temp), Energy);
        assert_eq!(emotion_for(Classic, K::Restrained, temp), Drama);
        assert_eq!(emotion_for(Classic, K::Editorial, temp), Drama);
        assert_eq!(emotion_for(Classic, K::Precise, temp), Drama);
    }
}

// ---------------------------------------------------------------------------
// resolve_typography
// ---------------------------------------------------------------------------

fn pick(t: &ResolvedStyleProfile, r: &TypographyRequest) -> (TypographyChoice, FontSet) {
    resolve_typography(t, r, &assets())
}

fn emotion_of_option(id: &str) -> &str {
    id.split('.').next().unwrap()
}

#[test]
fn level_zero_is_the_legacy_pairing() {
    let t = editorial_cool();
    let (choice, set) = pick(&t, &req(0, 5, None));
    assert_eq!(choice.option, None);
    assert_eq!(choice.legacy, Some(TypographyPairing::HumanistSerif));
    assert_eq!(choice.emotion, Emotion::Trust);
    assert!(choice.fallback_from.is_empty());
    assert_eq!(
        ids(&set),
        ids(&FontSet::for_pairing(TypographyPairing::HumanistSerif))
    );

    let classic = taste_of(&StyleProfile::default());
    assert_eq!(classic.tone, ResolvedTone::Classic);
    let (choice, _) = pick(&classic, &req(0, 5, None));
    assert_eq!(choice.legacy, Some(TypographyPairing::GroteskSerif));
    assert_eq!(choice.option, None);
}

#[test]
fn explicit_emotion_at_level_zero_is_option_one() {
    let t = editorial_cool();
    let (choice, set) = pick(&t, &req(0, 5, Some(Emotion::Trust)));
    assert_eq!(choice.option.as_deref(), Some("trust.1"));
    assert_eq!(choice.legacy, None);
    assert_eq!(set.role(FontRole::Display).asset_id, "font.dm_serif");
}

#[test]
fn same_request_same_choice() {
    let t = editorial_cool();
    for level in 0..=3 {
        for seed in 0..20 {
            let a = pick(&t, &req(level, seed, None)).0;
            let b = pick(&t, &req(level, seed, None)).0;
            assert_eq!(a, b);
        }
    }
}

#[test]
fn level_one_stays_within_the_resolved_emotion() {
    let t = editorial_cool();
    let mut seen = BTreeSet::new();
    for seed in 0..200 {
        let (c, _) = pick(&t, &req(1, seed, None));
        let o = c.option.expect("registry option");
        assert_eq!(emotion_of_option(&o), "trust", "{o}");
        assert_eq!(c.emotion, Emotion::Trust);
        seen.insert(o);
    }
    assert!(seen.len() >= 2, "level 1 varies: {seen:?}");
}

#[test]
fn level_two_reaches_adjacent_and_level_three_reaches_tone_listed() {
    let t = editorial_cool();
    let emotions_at = |level: u8| -> BTreeSet<Emotion> {
        (0..200)
            .map(|s| pick(&t, &req(level, s, None)).0.emotion)
            .collect()
    };
    let l1 = emotions_at(1);
    assert_eq!(l1, BTreeSet::from([Emotion::Trust]));
    let l2 = emotions_at(2);
    // trust is adjacent to calm and precision.
    assert!(
        l2.contains(&Emotion::Calm) || l2.contains(&Emotion::Precision),
        "{l2:?}"
    );
    for e in &l2 {
        assert!(
            [Emotion::Trust, Emotion::Calm, Emotion::Precision].contains(e),
            "{e:?} is not adjacent to trust"
        );
    }
    let l3 = emotions_at(3);
    // editorial lists warmth, handmade, luxury, drama beyond the adjacent ones.
    assert!(
        l3.iter().any(|e| [
            Emotion::Warmth,
            Emotion::Handmade,
            Emotion::Luxury,
            Emotion::Drama
        ]
        .contains(e)),
        "{l3:?}"
    );
    let allowed = [
        Emotion::Trust,
        Emotion::Calm,
        Emotion::Precision,
        Emotion::Warmth,
        Emotion::Handmade,
        Emotion::Luxury,
        Emotion::Drama,
    ];
    for e in &l3 {
        assert!(allowed.contains(e), "{e:?}");
    }
}

#[test]
fn legacy_locked_styles_stay_legacy() {
    let mut t = editorial_cool();
    t.typography = TypographyPairing::PosterBold; // as if from a reference / explicit style
    for level in 0..=3 {
        for seed in 0..50 {
            let (c, set) = pick(&t, &req(level, seed, None));
            assert_eq!(c.option, None);
            assert_eq!(c.legacy, Some(TypographyPairing::PosterBold));
            assert_eq!(
                ids(&set),
                ids(&FontSet::for_pairing(TypographyPairing::PosterBold))
            );
        }
    }
}

#[test]
fn classic_explores_legacy_plus_energy_and_drama() {
    let t = taste_of(&StyleProfile::default());
    let mut legacy = 0;
    let mut emotions = BTreeSet::new();
    for seed in 0..200 {
        let (c, _) = pick(&t, &req(1, seed, None));
        match &c.option {
            None => {
                legacy += 1;
                assert_eq!(c.legacy, Some(TypographyPairing::GroteskSerif));
            }
            Some(_) => {
                emotions.insert(c.emotion);
            }
        }
    }
    assert!(legacy > 0, "legacy GroteskSerif stays a candidate");
    assert!(
        emotions
            .iter()
            .all(|e| [Emotion::Energy, Emotion::Drama].contains(e)),
        "{emotions:?}"
    );
    assert!(!emotions.is_empty());
}

// ---------------------------------------------------------------------------
// Availability fallback
// ---------------------------------------------------------------------------

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("motion_typography_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        copy_dir(&assets().join("fonts"), &root.join("fonts"));
        TempRoot(root)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_face_falls_back_deterministically() {
    let root = TempRoot::new("missing");
    let t = editorial_cool();
    let run = |seed: u64| resolve_typography(&t, &req(1, seed, Some(Emotion::Trust)), &root.0);

    // trust.2 needs Zodiak-Bold + (intentionally absent) NectoMono: unavailable.
    std::fs::remove_file(root.0.join("fonts/Zodiak-Bold.otf")).unwrap();
    let seed = (0..200)
        .find(|s| run(*s).0.fallback_from == ["trust.2"])
        .expect("a seed picks trust.2");
    let (choice, set) = run(seed);
    assert_eq!(choice.option.as_deref(), Some("trust.3"));
    assert_eq!(choice.emotion, Emotion::Trust);
    assert_eq!(
        set.role(FontRole::Display).asset_id,
        "font.instrument_serif"
    );
    assert_eq!(run(seed).0, choice, "repeat is identical");

    // Removing trust.3's display face: picked trust.3 wraps to trust.1.
    std::fs::remove_file(root.0.join("fonts/InstrumentSerif-Regular.ttf")).unwrap();
    let seed = (0..200)
        .find(|s| run(*s).0.fallback_from == ["trust.3"])
        .expect("a seed picks trust.3");
    let (choice, _) = run(seed);
    assert_eq!(choice.option.as_deref(), Some("trust.1"));
    assert_eq!(run(seed).0, choice);

    // Nothing usable: legacy pairing, every sibling recorded.
    let empty = TempRoot::new("empty");
    std::fs::remove_dir_all(empty.0.join("fonts")).unwrap();
    let (choice, set) = resolve_typography(&t, &req(1, 3, Some(Emotion::Trust)), &empty.0);
    assert_eq!(choice.option, None);
    assert_eq!(choice.legacy, Some(TypographyPairing::HumanistSerif));
    assert_eq!(choice.fallback_from.len(), 3);
    assert_eq!(
        ids(&set),
        ids(&FontSet::for_pairing(TypographyPairing::HumanistSerif))
    );
}

// ---------------------------------------------------------------------------
// Compile integration
// ---------------------------------------------------------------------------

fn library() -> AssetLibrary {
    AssetLibrary::new(assets())
}

#[test]
fn default_options_are_byte_identical_to_compile_full() {
    let intent = demo_intent();
    let style = StyleProfile::default();
    let a = compile_full(
        &intent,
        &style,
        None,
        &library(),
        &ApproxMeasure,
        &AssetManifest::empty(),
    )
    .unwrap();
    let b = compile_with_options(
        &intent,
        &style,
        None,
        &library(),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
    assert!(a.theme.typography.is_none());
}

#[test]
fn emotion_override_points_the_scene_at_the_option_faces() {
    let intent = demo_intent();
    let opts = CompileOptions {
        emotion: Some(Emotion::Energy),
        ..CompileOptions::default()
    };
    let project = compile_with_options(
        &intent,
        &StyleProfile::default(),
        None,
        &library(),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .unwrap();
    let choice = project
        .theme
        .typography
        .clone()
        .expect("typography recorded");
    assert_eq!(choice.emotion, Emotion::Energy);
    // energy.1 is available with Necto Mono substituted by IBM Plex Mono.
    let option = choice.option.clone().expect("registry option");
    assert_eq!(option, "energy.1");
    assert_eq!(
        choice.fallback_from,
        ["energy.1:font.necto_mono->font.plex_mono"]
    );
    let faces = option_fontset_in(&option, &library().root).unwrap().roles;
    assert_eq!(faces[&FontRole::Mono].asset_id, "font.plex_mono");
    for (role, face) in &faces {
        assert_eq!(project.theme.fonts[role], face.asset_id);
    }
    let set = option_fontset_in(&option, &library().root).unwrap();
    for face in &set.faces {
        let asset = project
            .assets
            .iter()
            .find(|a| a.id == face.asset_id)
            .unwrap_or_else(|| panic!("asset {}", face.asset_id));
        assert_eq!(asset.path, face.path);
    }
}
