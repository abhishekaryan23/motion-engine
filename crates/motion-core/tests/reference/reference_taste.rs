//! ReferencePrinciples -> TasteDirector (0.7): precedence, gap filling,
//! material and image-treatment propagation, determinism.
//!
//! Principles are built by hand (the public profile is covered by
//! `reference_profile.rs`). Assertions are ordinal / structural, never exact
//! design values, except the curated accent colors the director documents as
//! its choice set.

use std::path::PathBuf;

use motion_core::assets::{AssetManifest, AssetStyleProfile};
use motion_core::compiler::taste::{
    resolve_with, BackgroundGrammar, CompositionRhythm, ImageTreatmentBias, LayerHints,
    MaterialFinish, PaletteFamily, PaletteTrajectory, ReferencePrinciples, ResolvedPolarity,
    ResolvedStyleProfile, ResolvedTemperature, ResolvedTone, TemperamentKind, TransitionFamily,
    TypographyPairing,
};
use motion_core::compiler::{
    compile_full, plan_assets, plan_assets_with_reference, resolve_taste, ApproxMeasure,
    AssetLibrary,
};
use motion_core::intent::CreativeIntent;
use motion_core::reference::oklab::hue_of;
use motion_core::scene::{
    Color, Layer, LayerKind, Material, MotionProject, Scene, TreatmentPreset,
};
use motion_core::style::{
    AccentRole, MaterialStyle, Polarity, StyleProfile, Temperament, Temperature, TextureStyle,
    Tone, TypographyStyle,
};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn library() -> AssetLibrary {
    AssetLibrary::new(repo().join("assets"))
}

fn demo_intent() -> CreativeIntent {
    CreativeIntent::from_json(&read("examples/editorial_demo.intent.json")).expect("intent")
}

/// Two beats that ask for a person image (beat 1) and an evidence image (beat 2).
fn image_intent() -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2",
        "title": "image_story",
        "format": "vertical",
        "beats": [
            {
                "purpose": "emphasize",
                "statement": "Nobody warned the new hires about the queue",
                "primary": { "kind": "phrase", "value": "new hires", "meaning": "a worker" },
                "secondary": { "kind": "phrase", "value": "on their first morning." },
                "energy": "building"
            },
            {
                "purpose": "reveal",
                "statement": "The receipt shows what changed",
                "primary": { "kind": "object", "asset": "shopping_basket", "meaning": "grocery receipt" },
                "secondary": { "kind": "number", "value": "6900", "meaning": "this week" },
                "energy": "building",
                "keyword": "changed"
            }
        ]
    }))
    .expect("intent parses")
}

fn manifest() -> AssetManifest {
    serde_json::from_str(&read("assets/test_manifest.json")).expect("manifest parses")
}

fn example_styles() -> Vec<(&'static str, StyleProfile)> {
    let parse = |rel: &str| StyleProfile::from_json(&read(rel)).expect("style parses");
    vec![
        ("default", StyleProfile::default()),
        (
            "warm_editorial",
            parse("examples/taste/warm_editorial.style.json"),
        ),
        (
            "dark_technical",
            parse("examples/taste/dark_technical.style.json"),
        ),
        (
            "playful_print",
            parse("examples/taste/playful_print.style.json"),
        ),
        (
            "editorial_demo",
            parse("examples/editorial_demo.style.json"),
        ),
    ]
}

fn compile_json(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&ReferencePrinciples>,
    manifest: &AssetManifest,
) -> String {
    let p = compile_project(intent, style, reference, manifest);
    serde_json::to_string(&p).expect("project serializes")
}

fn compile_project(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&ReferencePrinciples>,
    manifest: &AssetManifest,
) -> MotionProject {
    compile_full(
        intent,
        style,
        reference,
        &library(),
        &ApproxMeasure,
        manifest,
    )
    .expect("compiles")
}

fn walk<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            walk(children, out);
        }
    }
}

fn all_layers(s: &Scene) -> Vec<&Layer> {
    let mut out = Vec::new();
    walk(&s.layers, &mut out);
    out
}

/// Preset of the delivered image layer for `asset` (any scene).
fn image_preset(p: &MotionProject, asset: &str) -> TreatmentPreset {
    for s in &p.scenes {
        for l in all_layers(s) {
            if let LayerKind::Image {
                asset: a,
                treatment: Some(t),
                ..
            } = &l.kind
            {
                if a == asset {
                    return t.preset;
                }
            }
        }
    }
    panic!("no treated image layer for {asset}");
}

fn backdrop_textures(p: &MotionProject) -> Vec<Material> {
    let s = p
        .scenes
        .iter()
        .find(|s| s.id == "backdrop")
        .expect("backdrop scene");
    all_layers(s)
        .into_iter()
        .filter_map(|l| match &l.kind {
            LayerKind::Texture(t) => Some(t.material),
            _ => None,
        })
        .collect()
}

fn editorial() -> StyleProfile {
    StyleProfile {
        tone: Tone::Editorial,
        ..Default::default()
    }
}

fn technical_refs() -> ReferencePrinciples {
    ReferencePrinciples {
        tone: Some(ResolvedTone::Technical),
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        background: Some(BackgroundGrammar::TechnicalGrid),
        typography: Some(TypographyPairing::SerifSans),
        temperament: Some(TemperamentKind::Energetic),
        ..Default::default()
    }
}

fn accent(r: &ResolvedStyleProfile) -> Color {
    r.palette.colors.accent
}

// ---------------------------------------------------------------------------
// Empty principles behave like no reference
// ---------------------------------------------------------------------------

#[test]
fn empty_principles_resolve_exactly_like_no_reference() {
    let empty = ReferencePrinciples::default();
    assert!(empty.is_empty());
    for (name, style) in example_styles() {
        for key in [0u64, 1, 0xDEAD_BEEF, u64::MAX] {
            assert_eq!(
                resolve_with(&style, Some(&empty), key),
                resolve_with(&style, None, key),
                "{name} key {key}"
            );
        }
    }
    // Default style with an empty reference stays the classic look.
    assert_eq!(
        resolve_with(&StyleProfile::default(), Some(&empty), 0).tone,
        ResolvedTone::Classic
    );
}

#[test]
fn empty_principles_compile_byte_identically_to_no_reference() {
    let intent = demo_intent();
    let style = StyleProfile::default();
    let m = AssetManifest::empty();
    let with_empty = compile_json(&intent, &style, Some(&ReferencePrinciples::default()), &m);
    let without = compile_json(&intent, &style, None, &m);
    assert_eq!(with_empty, without);
    // Also for a taste-driven style.
    for (name, style) in example_styles() {
        let a = compile_json(&intent, &style, Some(&ReferencePrinciples::default()), &m);
        let b = compile_json(&intent, &style, None, &m);
        assert_eq!(a, b, "{name}");
    }
}

// ---------------------------------------------------------------------------
// Tone
// ---------------------------------------------------------------------------

#[test]
fn a_non_empty_reference_without_tone_resolves_editorial_never_classic() {
    let style = StyleProfile::default();
    let cases = [
        ReferencePrinciples {
            density: Some(motion_core::compiler::taste::DensityLevel::Dense),
            ..Default::default()
        },
        ReferencePrinciples {
            accent_hue: Some(20.0),
            ..Default::default()
        },
        ReferencePrinciples {
            layers: LayerHints {
                background: Some(motion_core::compiler::taste::Activity::Active),
                ..Default::default()
            },
            ..Default::default()
        },
        ReferencePrinciples {
            polarity: Some(ResolvedPolarity::Dark),
            ..Default::default()
        },
    ];
    for r in &cases {
        assert!(!r.is_empty());
        let tone = resolve_with(&style, Some(r), 0).tone;
        assert_eq!(tone, ResolvedTone::Editorial, "{r:?}");
    }
}

#[test]
fn a_reference_tone_is_used_when_the_style_tone_is_auto() {
    for tone in [
        ResolvedTone::Editorial,
        ResolvedTone::Technical,
        ResolvedTone::Playful,
    ] {
        let r = ReferencePrinciples {
            tone: Some(tone),
            ..Default::default()
        };
        assert_eq!(
            resolve_with(&StyleProfile::default(), Some(&r), 0).tone,
            tone
        );
    }
}

// ---------------------------------------------------------------------------
// Reference fills `auto`
// ---------------------------------------------------------------------------

#[test]
fn reference_fills_auto_dimensions() {
    let r = ReferencePrinciples {
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        background: Some(BackgroundGrammar::TechnicalGrid),
        temperament: Some(TemperamentKind::Precise),
        rhythm: Some(CompositionRhythm::HighFrequency),
        transition: Some(TransitionFamily::Hard),
        ..Default::default()
    };
    let out = resolve_with(&StyleProfile::default(), Some(&r), 0);
    assert_eq!(out.tone, ResolvedTone::Editorial);
    assert_eq!(out.palette.family, PaletteFamily::DarkCool);
    assert_eq!(out.palette.polarity, ResolvedPolarity::Dark);
    assert_eq!(out.palette.temperature, ResolvedTemperature::Cool);
    assert_eq!(out.background, BackgroundGrammar::TechnicalGrid);
    assert_eq!(out.motion.kind, TemperamentKind::Precise);
    assert_eq!(out.rhythm, CompositionRhythm::HighFrequency);
    assert_eq!(out.transition.family, TransitionFamily::Hard);
    // Each field reaches the style the compiler reads, not just the profile.
    assert_eq!(out.effective.tone, Tone::Editorial);
}

#[test]
fn high_frequency_rhythm_is_only_reachable_through_a_reference() {
    let tones = [Tone::Auto, Tone::Editorial, Tone::Technical, Tone::Playful];
    let temperaments = [
        Temperament::Auto,
        Temperament::Restrained,
        Temperament::Balanced,
        Temperament::Energetic,
    ];
    for tone in tones {
        for temperament in temperaments {
            for polarity in [Polarity::Auto, Polarity::Dark] {
                let style = StyleProfile {
                    tone,
                    temperament,
                    polarity,
                    ..Default::default()
                };
                assert_ne!(
                    resolve_with(&style, None, 0).rhythm,
                    CompositionRhythm::HighFrequency,
                    "{style:?}"
                );
            }
        }
    }
    let r = ReferencePrinciples {
        rhythm: Some(CompositionRhythm::HighFrequency),
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&editorial(), Some(&r), 0).rhythm,
        CompositionRhythm::HighFrequency
    );
}

#[test]
fn hard_transition_is_only_reachable_through_a_reference() {
    for tone in [Tone::Auto, Tone::Editorial, Tone::Technical, Tone::Playful] {
        for temperament in [
            Temperament::Auto,
            Temperament::Restrained,
            Temperament::Balanced,
            Temperament::Energetic,
        ] {
            let style = StyleProfile {
                tone,
                temperament,
                ..Default::default()
            };
            assert_ne!(
                resolve_with(&style, None, 0).transition.family,
                TransitionFamily::Hard
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Precedence: explicit style fields always win
// ---------------------------------------------------------------------------

#[test]
fn explicit_polarity_beats_the_reference() {
    let r = ReferencePrinciples {
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        ..Default::default()
    };
    let style = StyleProfile {
        polarity: Polarity::Light,
        ..Default::default()
    };
    let out = resolve_with(&style, Some(&r), 0);
    assert_eq!(out.palette.polarity, ResolvedPolarity::Light);
    assert_eq!(out.palette.family, PaletteFamily::CoolPaper);
    // The reverse: explicit dark beats a light reference.
    let r = ReferencePrinciples {
        polarity: Some(ResolvedPolarity::Light),
        ..Default::default()
    };
    let style = StyleProfile {
        polarity: Polarity::Dark,
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&style, Some(&r), 0).palette.polarity,
        ResolvedPolarity::Dark
    );
}

#[test]
fn explicit_temperature_beats_the_reference() {
    let r = ReferencePrinciples {
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        ..Default::default()
    };
    let style = StyleProfile {
        temperature: Temperature::Warm,
        ..Default::default()
    };
    let out = resolve_with(&style, Some(&r), 0);
    assert_eq!(out.palette.temperature, ResolvedTemperature::Warm);
    assert_eq!(out.palette.family, PaletteFamily::DarkWarm);
}

#[test]
fn explicit_tone_beats_the_reference() {
    let r = ReferencePrinciples {
        tone: Some(ResolvedTone::Technical),
        ..Default::default()
    };
    let style = StyleProfile {
        tone: Tone::Playful,
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&style, Some(&r), 0).tone,
        ResolvedTone::Playful
    );
}

#[test]
fn explicit_temperament_beats_the_reference() {
    let r = ReferencePrinciples {
        temperament: Some(TemperamentKind::Energetic),
        ..Default::default()
    };
    let style = StyleProfile {
        temperament: Temperament::Restrained,
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&style, Some(&r), 0).motion.kind,
        TemperamentKind::Restrained
    );
    // Without the explicit field the reference applies.
    assert_eq!(
        resolve_with(&editorial(), Some(&r), 0).motion.kind,
        TemperamentKind::Energetic
    );
}

#[test]
fn explicit_density_beats_the_reference() {
    use motion_core::compiler::taste::DensityLevel;
    use motion_core::style::Density;
    let r = ReferencePrinciples {
        density: Some(DensityLevel::Dense),
        ..Default::default()
    };
    let style = StyleProfile {
        density: Density::Sparse,
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&style, Some(&r), 0).density.level,
        DensityLevel::Sparse
    );
    assert_eq!(
        resolve_with(&editorial(), Some(&r), 0).density.level,
        DensityLevel::Dense
    );
}

#[test]
fn explicit_legacy_typography_beats_the_reference() {
    let r = ReferencePrinciples {
        tone: Some(ResolvedTone::Editorial),
        typography: Some(TypographyPairing::SerifSans),
        ..Default::default()
    };
    // Control: the reference pairing applies when nothing is explicit.
    assert_eq!(
        resolve_with(&StyleProfile::default(), Some(&r), 0).typography,
        TypographyPairing::SerifSans
    );
    let style = StyleProfile {
        typography_style: TypographyStyle::CondensedMono,
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&style, Some(&r), 0).typography,
        TypographyPairing::CondensedMono
    );
}

#[test]
fn brand_accent_wins_while_the_rest_of_the_reference_system_is_kept() {
    // A dark technical reference with an electric-blue accent (~230 degrees)
    // meets a style that pins the brand accent to acid yellow-green.
    let r = ReferencePrinciples {
        tone: Some(ResolvedTone::Technical),
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        background: Some(BackgroundGrammar::TechnicalGrid),
        typography: Some(TypographyPairing::CondensedMono),
        accent_hue: Some(230.0),
        ..Default::default()
    };
    // Control: without a brand accent the reference hue picks the cyan option.
    let free = resolve_with(&StyleProfile::default(), Some(&r), 0);
    assert_eq!(accent(&free), Color::rgb(0x3C, 0xC8, 0xF0));

    let style = StyleProfile {
        accent_role: AccentRole::Acid,
        ..Default::default()
    };
    let out = resolve_with(&style, Some(&r), 0);
    assert_eq!(accent(&out), Color::rgb(0xC9, 0xE4, 0x2B), "brand accent");
    assert_eq!(out.palette.polarity, ResolvedPolarity::Dark);
    assert_eq!(out.palette.family, PaletteFamily::DarkCool);
    assert_eq!(out.background, BackgroundGrammar::TechnicalGrid);
    assert_eq!(out.typography, TypographyPairing::CondensedMono);
    assert_eq!(out.tone, ResolvedTone::Technical);
}

#[test]
fn explicit_style_fields_win_over_a_full_technical_reference() {
    let style = StyleProfile {
        polarity: Polarity::Light,
        temperature: Temperature::Warm,
        tone: Tone::Editorial,
        temperament: Temperament::Restrained,
        ..Default::default()
    };
    let out = resolve_with(&style, Some(&technical_refs()), 0);
    assert_eq!(out.tone, ResolvedTone::Editorial);
    assert_eq!(out.palette.family, PaletteFamily::WarmPaper);
    assert_eq!(out.motion.kind, TemperamentKind::Restrained);
    // Unset dimensions still come from the reference.
    assert_eq!(out.background, BackgroundGrammar::TechnicalGrid);
    assert_eq!(out.typography, TypographyPairing::SerifSans);
}

// ---------------------------------------------------------------------------
// Accent hint
// ---------------------------------------------------------------------------

fn dark_cool_with_hue(hue: f32) -> ReferencePrinciples {
    ReferencePrinciples {
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        accent_hue: Some(hue),
        ..Default::default()
    }
}

#[test]
fn accent_hue_picks_the_nearest_curated_accent_of_the_family() {
    let cyan = hue_of(0x3CC8F0);
    let yellow_green = hue_of(0xB8F23A);
    assert!(
        (cyan - yellow_green).abs() > 30.0,
        "options must be distinct"
    );
    let style = StyleProfile::default();
    for key in [0u64, 1, 2, 3, 99, 0xFFFF_FFFF] {
        for seed in [0u64, 7, 12345] {
            let style = StyleProfile {
                seed,
                ..style.clone()
            };
            let a = resolve_with(&style, Some(&dark_cool_with_hue(cyan + 5.0)), key);
            assert_eq!(a.palette.family, PaletteFamily::DarkCool);
            assert_eq!(accent(&a), Color::rgb(0x3C, 0xC8, 0xF0), "key {key}");
            let b = resolve_with(&style, Some(&dark_cool_with_hue(yellow_green - 5.0)), key);
            assert_eq!(accent(&b), Color::rgb(0xB8, 0xF2, 0x3A), "key {key}");
        }
    }
}

#[test]
fn accent_hue_wraps_around_the_color_circle() {
    // 355 degrees and 5 degrees are 10 apart; both must pick the same warm option.
    let style = StyleProfile {
        tone: Tone::Editorial,
        ..Default::default()
    };
    let mk = |h: f32| ReferencePrinciples {
        accent_hue: Some(h),
        ..Default::default()
    };
    let a = resolve_with(&style, Some(&mk(355.0)), 0);
    let b = resolve_with(&style, Some(&mk(5.0)), 0);
    assert_eq!(accent(&a), accent(&b));
}

#[test]
fn accent_hue_never_changes_the_palette_family() {
    let style = editorial();
    let base = resolve_with(&style, None, 0);
    for hue in [0.0f32, 60.0, 130.0, 230.0, 300.0] {
        let r = ReferencePrinciples {
            accent_hue: Some(hue),
            ..Default::default()
        };
        let out = resolve_with(&style, Some(&r), 0);
        assert_eq!(out.palette.family, base.palette.family, "hue {hue}");
        assert_eq!(out.background, base.background);
    }
}

// ---------------------------------------------------------------------------
// Color fields
// ---------------------------------------------------------------------------

#[test]
fn color_fields_with_dark_polarity_pick_print_dark() {
    let r = ReferencePrinciples {
        color_fields: Some(true),
        ..Default::default()
    };
    let style = StyleProfile {
        polarity: Polarity::Dark,
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&style, Some(&r), 0).palette.family,
        PaletteFamily::PrintDark
    );
    let style = StyleProfile {
        polarity: Polarity::Light,
        ..Default::default()
    };
    assert_eq!(
        resolve_with(&style, Some(&r), 0).palette.family,
        PaletteFamily::PrintBright
    );
}

#[test]
fn no_color_fields_under_playful_gives_a_paper_family_but_keeps_print_fields_background() {
    let playful = StyleProfile {
        tone: Tone::Playful,
        ..Default::default()
    };
    let r = ReferencePrinciples {
        color_fields: Some(false),
        ..Default::default()
    };
    let out = resolve_with(&playful, Some(&r), 0);
    assert!(
        matches!(
            out.palette.family,
            PaletteFamily::WarmPaper | PaletteFamily::CoolPaper
        ),
        "{:?}",
        out.palette.family
    );
    // Only an explicit reference background changes the tone's own grammar.
    assert_eq!(out.background, BackgroundGrammar::PrintFields);
    assert_eq!(out.palette.trajectory, PaletteTrajectory::FieldCycle);

    let r = ReferencePrinciples {
        color_fields: Some(false),
        background: Some(BackgroundGrammar::CleanFlat),
        ..Default::default()
    };
    let out = resolve_with(&playful, Some(&r), 0);
    assert_eq!(out.background, BackgroundGrammar::CleanFlat);
    assert_eq!(out.palette.trajectory, PaletteTrajectory::Steady);

    // Control: without the reference playful uses print colors.
    assert_eq!(
        resolve_with(&playful, None, 0).palette.family,
        PaletteFamily::PrintBright
    );
}

#[test]
fn color_fields_true_under_editorial_uses_print_colors() {
    let r = ReferencePrinciples {
        color_fields: Some(true),
        ..Default::default()
    };
    let out = resolve_with(&editorial(), Some(&r), 0);
    assert_eq!(out.palette.family, PaletteFamily::PrintBright);
    assert!(!out.palette.colors.fields.is_empty());
}

// ---------------------------------------------------------------------------
// Material propagation
// ---------------------------------------------------------------------------

#[test]
fn clean_flat_or_screen_material_flattens_the_effective_style() {
    let base = resolve_with(&editorial(), None, 0);
    assert_eq!(base.material, MaterialFinish::UncoatedPaper);
    assert_eq!(base.effective.material, MaterialStyle::Paper);
    assert_eq!(base.effective.texture_style, TextureStyle::SubtlePrint);

    for finish in [MaterialFinish::CleanFlat, MaterialFinish::Screen] {
        let r = ReferencePrinciples {
            material: Some(finish),
            ..Default::default()
        };
        let out = resolve_with(&editorial(), Some(&r), 0);
        assert_eq!(out.material, finish);
        assert_eq!(out.effective.material, MaterialStyle::Flat, "{finish:?}");
        assert_eq!(
            out.effective.texture_style,
            TextureStyle::None,
            "{finish:?}"
        );
    }
}

#[test]
fn coated_print_material_keeps_a_light_grain() {
    let r = ReferencePrinciples {
        material: Some(MaterialFinish::CoatedPrint),
        ..Default::default()
    };
    let out = resolve_with(&editorial(), Some(&r), 0);
    assert_eq!(out.effective.material, MaterialStyle::Flat);
    assert_eq!(out.effective.texture_style, TextureStyle::SubtlePrint);
}

#[test]
fn uncoated_paper_under_technical_light_becomes_paper_with_print() {
    let r = ReferencePrinciples {
        material: Some(MaterialFinish::UncoatedPaper),
        ..Default::default()
    };
    let style = StyleProfile {
        tone: Tone::Technical,
        polarity: Polarity::Light,
        ..Default::default()
    };
    let out = resolve_with(&style, Some(&r), 0);
    assert_eq!(out.effective.material, MaterialStyle::Paper);
    assert_eq!(out.effective.texture_style, TextureStyle::SubtlePrint);
    // Control: technical's own material is a flat screen.
    let control = resolve_with(&style, None, 0);
    assert_eq!(control.effective.material, MaterialStyle::Flat);
    assert_eq!(control.effective.texture_style, TextureStyle::None);
}

#[test]
fn dark_polarity_still_forces_a_flat_ground() {
    let r = ReferencePrinciples {
        material: Some(MaterialFinish::UncoatedPaper),
        ..Default::default()
    };
    // Technical is dark by default.
    let tech = StyleProfile {
        tone: Tone::Technical,
        ..Default::default()
    };
    let out = resolve_with(&tech, Some(&r), 0);
    assert_eq!(out.palette.polarity, ResolvedPolarity::Dark);
    assert_eq!(out.effective.material, MaterialStyle::Flat);
    // Explicit dark under editorial as well.
    let dark_editorial = StyleProfile {
        tone: Tone::Editorial,
        polarity: Polarity::Dark,
        ..Default::default()
    };
    let out = resolve_with(&dark_editorial, Some(&r), 0);
    assert_eq!(out.effective.material, MaterialStyle::Flat);
}

#[test]
fn explicit_style_material_and_texture_win_over_the_reference_material() {
    let r = ReferencePrinciples {
        material: Some(MaterialFinish::UncoatedPaper),
        ..Default::default()
    };
    let style = StyleProfile {
        tone: Tone::Technical,
        polarity: Polarity::Light,
        material: MaterialStyle::Flat,
        texture_style: TextureStyle::HeavyPrint,
        ..Default::default()
    };
    let out = resolve_with(&style, Some(&r), 0);
    assert_eq!(out.effective.texture_style, TextureStyle::HeavyPrint);
    // `material: paper` is the schema default, so only a non-default value is explicit.
    assert_eq!(out.effective.material, MaterialStyle::Flat);
}

#[test]
fn reference_material_reaches_the_compiled_backdrop() {
    let intent = demo_intent();
    let m = AssetManifest::empty();
    let base = compile_project(&intent, &editorial(), None, &m);
    let flat_ref = ReferencePrinciples {
        material: Some(MaterialFinish::CleanFlat),
        ..Default::default()
    };
    let flat = compile_project(&intent, &editorial(), Some(&flat_ref), &m);

    let base_tex = backdrop_textures(&base);
    let flat_tex = backdrop_textures(&flat);
    assert!(
        base_tex.contains(&Material::Paper),
        "editorial default has a paper ground: {base_tex:?}"
    );
    assert!(
        !flat_tex.contains(&Material::Paper),
        "reference clean_flat must remove the paper ground: {flat_tex:?}"
    );
    assert!(flat_tex.contains(&Material::Flat), "{flat_tex:?}");
    assert!(
        !flat_tex.contains(&Material::Grain),
        "clean flat carries no film grain: {flat_tex:?}"
    );
    assert_ne!(
        serde_json::to_string(&base).unwrap(),
        serde_json::to_string(&flat).unwrap()
    );

    // A paper reference under a technical/light style brings the paper back.
    let paper_ref = ReferencePrinciples {
        material: Some(MaterialFinish::UncoatedPaper),
        background: Some(BackgroundGrammar::PaperField),
        ..Default::default()
    };
    let style = StyleProfile {
        tone: Tone::Technical,
        polarity: Polarity::Light,
        ..Default::default()
    };
    let with_paper = compile_project(&intent, &style, Some(&paper_ref), &m);
    assert!(backdrop_textures(&with_paper).contains(&Material::Paper));
    let without = compile_project(&intent, &style, None, &m);
    assert!(!backdrop_textures(&without).contains(&Material::Paper));
}

// ---------------------------------------------------------------------------
// Image-treatment propagation
// ---------------------------------------------------------------------------

fn bias_reference(bias: ImageTreatmentBias) -> ReferencePrinciples {
    ReferencePrinciples {
        image_treatment: Some(bias),
        ..Default::default()
    }
}

#[test]
fn image_treatment_bias_from_a_reference_restates_the_asset_style() {
    let style = editorial();
    let baseline = AssetStyleProfile::from_resolved(&resolve_with(&style, None, 0));
    let mut seen = vec![baseline.medium.clone()];
    for bias in [
        ImageTreatmentBias::Natural,
        ImageTreatmentBias::Monochrome,
        ImageTreatmentBias::Muted,
        ImageTreatmentBias::Duotone,
    ] {
        let resolved = resolve_with(&style, Some(&bias_reference(bias)), 0);
        assert_eq!(resolved.image_treatment, bias);
        let p = AssetStyleProfile::from_resolved(&resolved);
        assert_ne!(p, baseline, "{bias:?} must change the asset style");
        assert_ne!(p.medium, baseline.medium, "{bias:?} medium");
        assert!(
            !seen.contains(&p.medium),
            "{bias:?} medium duplicates another bias: {}",
            p.medium
        );
        seen.push(p.medium.clone());
    }
    // Contrast changes for the biases that restate it.
    for bias in [
        ImageTreatmentBias::Monochrome,
        ImageTreatmentBias::Muted,
        ImageTreatmentBias::Duotone,
    ] {
        let p =
            AssetStyleProfile::from_resolved(&resolve_with(&style, Some(&bias_reference(bias)), 0));
        assert_ne!(p.contrast, baseline.contrast, "{bias:?} contrast");
    }
    // Natural leaves contrast to the style (no restatement).
    let natural = AssetStyleProfile::from_resolved(&resolve_with(
        &style,
        Some(&bias_reference(ImageTreatmentBias::Natural)),
        0,
    ));
    assert_eq!(natural.contrast, baseline.contrast);
}

#[test]
fn a_bias_equal_to_the_tone_default_changes_nothing() {
    // Editorial's own bias is PaperCutout: naming it explicitly is a no-op.
    let style = editorial();
    let with = resolve_with(
        &style,
        Some(&bias_reference(ImageTreatmentBias::PaperCutout)),
        0,
    );
    let without = resolve_with(&style, None, 0);
    assert_eq!(
        AssetStyleProfile::from_resolved(&with),
        AssetStyleProfile::from_resolved(&without)
    );
    assert_eq!(with.image_treatment, without.image_treatment);
}

#[test]
fn asset_plan_style_follows_the_reference_bias() {
    let intent = image_intent();
    let style = editorial();
    let lib = library();
    let plain = plan_assets(&intent, &style, &lib).expect("plan");
    // No reference == plan_assets.
    let none = plan_assets_with_reference(&intent, &style, None, &lib).expect("plan");
    assert_eq!(plain.style, none.style);
    let empty =
        plan_assets_with_reference(&intent, &style, Some(&ReferencePrinciples::default()), &lib)
            .expect("plan");
    assert_eq!(plain.style, empty.style);

    for bias in [
        ImageTreatmentBias::Natural,
        ImageTreatmentBias::Monochrome,
        ImageTreatmentBias::Muted,
        ImageTreatmentBias::Duotone,
    ] {
        let with = plan_assets_with_reference(&intent, &style, Some(&bias_reference(bias)), &lib)
            .expect("plan");
        assert_ne!(with.style, plain.style, "{bias:?}");
        // The plan's requests themselves are style-independent.
        assert_eq!(with.requests.len(), plain.requests.len(), "{bias:?}");
    }
}

#[test]
fn asset_plan_style_follows_reference_material_and_palette() {
    let intent = image_intent();
    let lib = library();
    let plain = plan_assets(&intent, &editorial(), &lib).expect("plan");
    let dark_cool = ReferencePrinciples {
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        ..Default::default()
    };
    let with =
        plan_assets_with_reference(&intent, &editorial(), Some(&dark_cool), &lib).expect("plan");
    assert_ne!(with.style.palette_tendency, plain.style.palette_tendency);
}

#[test]
fn delivered_image_presets_follow_the_reference_bias() {
    let intent = image_intent();
    let m = manifest();
    let style = editorial();
    const HERO: &str = "asset.beat_1.hero_subject";
    const EVIDENCE: &str = "asset.beat_2.evidence_image";

    let presets = |bias: Option<ImageTreatmentBias>| {
        let r = bias.map(bias_reference);
        let p = compile_project(&intent, &style, r.as_ref(), &m);
        (image_preset(&p, HERO), image_preset(&p, EVIDENCE))
    };

    // Natural: images as they are.
    assert_eq!(
        presets(Some(ImageTreatmentBias::Natural)),
        (TreatmentPreset::Natural, TreatmentPreset::Natural)
    );
    // Muted: documentary look everywhere.
    assert_eq!(
        presets(Some(ImageTreatmentBias::Muted)),
        (
            TreatmentPreset::MutedDocumentary,
            TreatmentPreset::MutedDocumentary
        )
    );
    // Monochrome: the person is monochrome; evidence stays legible.
    let (hero, evidence) = presets(Some(ImageTreatmentBias::Monochrome));
    assert_eq!(hero, TreatmentPreset::EditorialMonochrome);
    assert_eq!(evidence, TreatmentPreset::MutedDocumentary);
    // Duotone: a person is never duotoned, nor is evidence.
    let (hero, evidence) = presets(Some(ImageTreatmentBias::Duotone));
    assert_ne!(
        hero,
        TreatmentPreset::EditorialDuotone,
        "people are never duotoned"
    );
    assert_ne!(evidence, TreatmentPreset::EditorialDuotone);
    assert_eq!(hero, TreatmentPreset::MutedDocumentary);

    // The bias visibly changes the compiled project.
    let none = presets(None);
    assert_ne!(none, presets(Some(ImageTreatmentBias::Natural)));
}

#[test]
fn a_reference_bias_equal_to_the_tone_bias_keeps_the_legacy_preset_mapping() {
    let intent = image_intent();
    let m = manifest();
    let style = editorial();
    let a = compile_json(&intent, &style, None, &m);
    let b = compile_json(
        &intent,
        &style,
        Some(&bias_reference(ImageTreatmentBias::PaperCutout)),
        &m,
    );
    assert_eq!(a, b);
}

#[test]
fn non_person_roles_may_be_duotoned_only_via_the_environment_path() {
    // Direct check of the documented rule on the resolved bias: Duotone bias is
    // "reference only" (no style field reaches it).
    for (name, style) in example_styles() {
        assert_ne!(
            resolve_with(&style, None, 0).image_treatment,
            ImageTreatmentBias::Duotone,
            "{name}"
        );
        assert_ne!(
            resolve_with(&style, None, 0).image_treatment,
            ImageTreatmentBias::Natural,
            "{name}"
        );
        assert_ne!(
            resolve_with(&style, None, 0).image_treatment,
            ImageTreatmentBias::Muted,
            "{name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Determinism and integration
// ---------------------------------------------------------------------------

#[test]
fn compile_with_a_reference_is_deterministic() {
    let intent = demo_intent();
    let m = AssetManifest::empty();
    let style = StyleProfile::default();
    let r = ReferencePrinciples {
        tone: Some(ResolvedTone::Technical),
        polarity: Some(ResolvedPolarity::Dark),
        temperature: Some(ResolvedTemperature::Cool),
        color_fields: Some(false),
        accent_hue: Some(230.0),
        background: Some(BackgroundGrammar::TechnicalGrid),
        typography: Some(TypographyPairing::CondensedMono),
        material: Some(MaterialFinish::Screen),
        rhythm: Some(CompositionRhythm::HighFrequency),
        temperament: Some(TemperamentKind::Precise),
        transition: Some(TransitionFamily::Hard),
        layers: LayerHints {
            background: Some(motion_core::compiler::taste::Activity::Structured),
            ..Default::default()
        },
        ..Default::default()
    };
    let a = compile_json(&intent, &style, Some(&r), &m);
    let b = compile_json(&intent, &style, Some(&r), &m);
    assert_eq!(a, b);
    // ... and the reference genuinely changes the output.
    assert_ne!(a, compile_json(&intent, &style, None, &m));
}

#[test]
fn a_reference_compiled_project_validates() {
    let intent = demo_intent();
    let m = AssetManifest::empty();
    // Precise (not Energetic): see the ignored DEFECT test below.
    let r = ReferencePrinciples {
        temperament: Some(TemperamentKind::Precise),
        ..technical_refs()
    };
    let p = compile_project(&intent, &StyleProfile::default(), Some(&r), &m);
    if let Err(e) = motion_core::validate(&p, Some(&repo().join("assets"))) {
        panic!("reference-driven project fails validation: {e:?}");
    }
    let m = manifest();
    let p = compile_project(
        &image_intent(),
        &editorial(),
        Some(&bias_reference(ImageTreatmentBias::Natural)),
        &m,
    );
    if let Err(e) = motion_core::validate(&p, Some(&repo().join("assets"))) {
        panic!("reference-driven image project fails validation: {e:?}");
    }
}

/// DEFECT (pre-existing, not reference-specific): the technical tone with an
/// energetic temperament compiles `examples/editorial_demo.intent.json` into a
/// project whose `beat_2` scene has a motion ending 49 ms after the scene
/// (`motion ends after scene (ends at 4.132s, scene 'beat_2' lasts 4.083s)`).
/// It reproduces with a plain StyleProfile and no reference at all; a
/// reference only makes it reachable through `temperament: energetic` too.
#[test]
fn technical_energetic_project_validates() {
    let intent = demo_intent();
    let m = AssetManifest::empty();
    let style = StyleProfile {
        tone: Tone::Technical,
        temperament: Temperament::Energetic,
        ..Default::default()
    };
    let p = compile_project(&intent, &style, None, &m);
    if let Err(e) = motion_core::validate(&p, Some(&repo().join("assets"))) {
        panic!("plain style (no reference) fails validation: {e:?}");
    }
    // The same combination through a reference.
    let r = technical_refs();
    let p = compile_project(&intent, &StyleProfile::default(), Some(&r), &m);
    if let Err(e) = motion_core::validate(&p, Some(&repo().join("assets"))) {
        panic!("reference-driven project fails validation: {e:?}");
    }
}

/// Every single-dimension principle compiles into a valid project (the
/// director never produces a combination the compiler cannot express).
/// DEFECT: a reference rhythm of progressive / active / high_frequency under
/// the default (auto) style compiles editorial_demo into scenes containing a
/// motion that ends after its scene (by 41 ms up to 0.64 s), so `validate`
/// rejects the project. Measured: progressive -> beat_2 ends 4.449s vs 4.408s;
/// active -> beat_2 4.449s vs 4.083s; high_frequency -> beat_1 3.878s vs
/// 3.87s and beat_2 4.446s vs 3.805s.
#[test]
fn reference_rhythm_projects_validate() {
    let intent = demo_intent();
    let m = AssetManifest::empty();
    let assets = repo().join("assets");
    let mut failures = Vec::new();
    for rhythm in [
        CompositionRhythm::Progressive,
        CompositionRhythm::Active,
        CompositionRhythm::HighFrequency,
    ] {
        let r = ReferencePrinciples {
            rhythm: Some(rhythm),
            ..Default::default()
        };
        let p = compile_project(&intent, &StyleProfile::default(), Some(&r), &m);
        if let Err(e) = motion_core::validate(&p, Some(&assets)) {
            failures.push(format!("{rhythm:?}: {e}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_single_principle_compiles_and_validates() {
    use motion_core::compiler::taste::{Activity, DensityLevel, Presence, ScaleContrast};
    let intent = demo_intent();
    let m = AssetManifest::empty();
    let mut cases: Vec<(String, ReferencePrinciples)> = Vec::new();
    macro_rules! each {
        ($field:ident, [$($v:expr),*]) => {$(
            cases.push((
                format!("{}={:?}", stringify!($field), $v),
                ReferencePrinciples { $field: Some($v), ..Default::default() },
            ));
        )*};
    }
    each!(
        tone,
        [
            ResolvedTone::Editorial,
            ResolvedTone::Technical,
            ResolvedTone::Playful
        ]
    );
    each!(polarity, [ResolvedPolarity::Light, ResolvedPolarity::Dark]);
    each!(
        temperature,
        [ResolvedTemperature::Warm, ResolvedTemperature::Cool]
    );
    each!(color_fields, [true, false]);
    each!(accent_hue, [0.0f32, 130.0, 230.0]);
    each!(
        background,
        [
            BackgroundGrammar::PaperField,
            BackgroundGrammar::TechnicalGrid,
            BackgroundGrammar::PrintFields,
            BackgroundGrammar::CleanFlat
        ]
    );
    each!(
        typography,
        [
            TypographyPairing::GroteskSerif,
            TypographyPairing::SerifSans,
            TypographyPairing::CondensedMono,
            TypographyPairing::PosterBold
        ]
    );
    each!(
        material,
        [
            MaterialFinish::UncoatedPaper,
            MaterialFinish::CleanFlat,
            MaterialFinish::Screen,
            MaterialFinish::CoatedPrint
        ]
    );
    each!(
        density,
        [
            DensityLevel::Sparse,
            DensityLevel::Balanced,
            DensityLevel::Dense
        ]
    );
    each!(
        rhythm,
        [
            CompositionRhythm::SlowBreathing,
            CompositionRhythm::MeasuredEditorial
        ]
    );
    // Progressive / Active / HighFrequency: see the ignored DEFECT test below.
    each!(
        temperament,
        [
            TemperamentKind::Restrained,
            TemperamentKind::Editorial,
            TemperamentKind::Precise
        ]
    );
    each!(
        transition,
        [
            TransitionFamily::Subtle,
            TransitionFamily::Editorial,
            TransitionFamily::Geometric,
            TransitionFamily::Kinetic,
            TransitionFamily::Hard
        ]
    );
    each!(
        scale,
        [
            ScaleContrast::Subtle,
            ScaleContrast::Moderate,
            ScaleContrast::Large,
            ScaleContrast::Dramatic
        ]
    );
    cases.push((
        "layers".into(),
        ReferencePrinciples {
            layers: LayerHints {
                foreground: Some(Presence::Dominant),
                midground: Some(Activity::Active),
                background: Some(Activity::Still),
            },
            ..Default::default()
        },
    ));
    let assets = repo().join("assets");
    let mut failures = Vec::new();
    for (name, r) in &cases {
        let p = compile_project(&intent, &StyleProfile::default(), Some(r), &m);
        if let Err(e) = motion_core::validate(&p, Some(&assets)) {
            failures.push(format!("{name}: {e}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn resolve_taste_is_story_keyed_and_reference_aware() {
    let intent = demo_intent();
    let r = dark_cool_with_hue(hue_of(0x3CC8F0));
    let a = resolve_taste(&intent, &StyleProfile::default(), Some(&r));
    let b = resolve_taste(&intent, &StyleProfile::default(), Some(&r));
    assert_eq!(a, b);
    assert_eq!(a.palette.family, PaletteFamily::DarkCool);
    // The reference accent choice does not depend on the story.
    let mut other: Value = serde_json::to_value(&intent).unwrap();
    other["title"] = json!("a completely different title");
    let other: CreativeIntent = serde_json::from_value(other).unwrap();
    let c = resolve_taste(&other, &StyleProfile::default(), Some(&r));
    assert_eq!(accent(&a), accent(&c));
}
