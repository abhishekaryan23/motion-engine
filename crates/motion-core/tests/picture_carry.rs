//! (0.23 W8a) Picture continuity: a library picture the story carries
//! (`continuity: carry_primary` / `carry_secondary`) is one shared element
//! across the handoff, lands in its slot in the next beat, or settles into the
//! anchor chip when the next beat does not show it. Structural assertions on
//! the compiled scene; no pixels. A throwaway family of four square cutouts is
//! written under `target/`.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning, TextMeasure,
    WARN_CARRY_IGNORED, WARN_RELATIONSHIP_DROPPED,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionProject, Scene, SharedElement};
use motion_core::style::StyleProfile;
use serde_json::{json, Value};

/// A 1x1 transparent PNG (the engine never decodes it at compile time).
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F,
    0x00, 0x05, 0xFE, 0x02, 0xFE, 0xA7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

const FAMILY: &str = "carry_fixture";
const OBJECTS: [&str; 4] = ["apple", "pear", "plum", "cherry"];

/// `library/carry_fixture/`: four object cutouts (`apple`, `pear`, `plum`,
/// `cherry`), each a 512 x 512 transparent picture.
fn fixture_root(test: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/picture_carry_tests")
        .join(test);
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("library").join(FAMILY);
    std::fs::create_dir_all(&dir).expect("family dir");
    let mut catalog = Vec::new();
    let mut manifest = Vec::new();
    for id in OBJECTS {
        let file = format!("{id}.png");
        std::fs::write(dir.join(&file), PNG).expect("png");
        catalog.push(json!({
            "id": id, "file": file, "role": "object", "subject": id, "tags": [id, "fruit"],
            "alpha": true, "cutout_bbox": null, "open_edge": false,
            "source": { "vendor": "test" }, "qa": "PASS"
        }));
        manifest.push(json!({
            "id": format!("library.{id}"), "path": file,
            "width": 512, "height": 512, "alpha": true,
            "serves": [format!("library.{id}")]
        }));
    }
    std::fs::write(
        dir.join("catalog.json"),
        json!({ "version": "0.1", "family": FAMILY, "assets": catalog }).to_string(),
    )
    .expect("catalog.json");
    std::fs::write(
        dir.join("manifest.json"),
        json!({ "version": "0.2", "assets": manifest }).to_string(),
    )
    .expect("manifest.json");
    root
}

/// Compile `beats` the way the product path does (art direction, variety) in
/// the look `tone` names (`editorial` and `technical` are flat looks with no /
/// geometric handoffs, `playful` wipes with kinetic panels).
fn compile(
    test: &str,
    beats: Vec<Value>,
    tone: &str,
    art: Option<ArtMode>,
) -> (MotionProject, Vec<CompileWarning>) {
    compile_on(test, beats, tone, art, None)
}

/// [`compile`] on a canvas of `(width, height)` pixels (the legacy vertical
/// canvas when `None`).
fn compile_on(
    test: &str,
    beats: Vec<Value>,
    tone: &str,
    art: Option<ArtMode>,
    canvas: Option<(u32, u32)>,
) -> (MotionProject, Vec<CompileWarning>) {
    compile_with(test, beats, tone, art, canvas, Some(7))
}

/// The default compile (no variety, no direction seed): what `compile` does
/// without `--variety`.
fn compile_default(
    test: &str,
    beats: Vec<Value>,
    tone: &str,
    art: Option<ArtMode>,
) -> (MotionProject, Vec<CompileWarning>) {
    compile_with(test, beats, tone, art, None, None)
}

fn compile_with(
    test: &str,
    beats: Vec<Value>,
    tone: &str,
    art: Option<ArtMode>,
    canvas: Option<(u32, u32)>,
    variety: Option<u64>,
) -> (MotionProject, Vec<CompileWarning>) {
    let root = fixture_root(test);
    let intent: CreativeIntent = serde_json::from_value(json!({
        "version": "0.2", "title": "picture_carry", "format": "vertical", "beats": beats
    }))
    .expect("intent");
    let style: StyleProfile = serde_json::from_value(json!({ "tone": tone })).expect("style");
    let library = AssetLibrary::new(&root).with_families(vec![FAMILY.to_string()]);
    let opts = CompileOptions {
        art: Some(art.unwrap_or(ArtMode::Auto)),
        canvas,
        variety,
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_report(
        &intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn object(asset: &str, meaning: &str, value: Option<&str>) -> Value {
    let mut v = json!({ "kind": "object", "asset": asset, "meaning": meaning });
    if let Some(value) = value {
        v["value"] = json!(value);
    }
    v
}

/// An emphasize beat of one picture (a hero object).
fn hero(asset: &str, carry: Option<&str>) -> Value {
    let mut beat = json!({
        "purpose": "emphasize", "statement": "One fruit", "keyword": "fruit",
        "primary": object(asset, "the fruit", Some("$5")), "energy": "building"
    });
    if let Some(c) = carry {
        beat["continuity"] = json!(c);
    }
    beat
}

/// A compare beat of two pictures (a stat pair).
fn pair(a: &str, b: &str, relationship: Option<&str>, carry: Option<&str>) -> Value {
    let mut beat = json!({
        "purpose": "compare", "statement": "Fruit against fruit", "keyword": "fruit",
        "primary": object(a, "this one", Some("$5")),
        "secondary": object(b, "that one", Some("$9")),
        "energy": "building"
    });
    if let Some(r) = relationship {
        beat["relationship"] = json!(r);
    }
    if let Some(c) = carry {
        beat["continuity"] = json!(c);
    }
    beat
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes.iter().find(|s| s.id == id).expect("scene")
}

fn all_layers<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            all_layers(children, out);
        }
    }
}

/// Image layers of a scene (group children included) that draw `file`.
fn images_of<'a>(p: &'a MotionProject, scene_id: &str, file: &str) -> Vec<&'a Layer> {
    let mut layers = Vec::new();
    all_layers(&scene(p, scene_id).layers, &mut layers);
    layers
        .into_iter()
        .filter(|l| match &l.kind {
            LayerKind::Image { asset, .. } => p
                .assets
                .iter()
                .any(|a| a.id == *asset && a.path.ends_with(file)),
            _ => false,
        })
        .collect()
}

fn shared<'a>(p: &'a MotionProject, id: &str) -> &'a SharedElement {
    p.shared
        .iter()
        .find(|e| e.id == id)
        .unwrap_or_else(|| panic!("no shared element {id}: {:?}", ids(p)))
}

fn apple(p: &MotionProject) -> &SharedElement {
    shared(p, "shared.pic.apple")
}

fn ids(p: &MotionProject) -> Vec<&str> {
    p.shared.iter().map(|e| e.id.as_str()).collect()
}

fn keys_in(e: &SharedElement, scene: &str) -> usize {
    e.track.iter().filter(|k| k.scene == scene).count()
}

fn with_code<'a>(w: &'a [CompileWarning], code: &str) -> Vec<&'a CompileWarning> {
    w.iter().filter(|w| w.code == code).collect()
}

// ---------------------------------------------------------------------------
// Carry-out, carry-in
// ---------------------------------------------------------------------------

#[test]
fn a_carried_picture_is_one_shared_element_across_the_handoff() {
    let (p, w) = compile(
        "one_element",
        vec![hero("apple", Some("carry_primary")), hero("apple", None)],
        "editorial",
        None,
    );
    assert_eq!(ids(&p), ["shared.pic.apple"], "{w:?}");
    let apple = apple(&p);
    // Drawn by the beat that starts it, then by the next beat: keys in both.
    assert!(keys_in(apple, "beat_1") >= 2, "enters and holds in beat 1");
    assert!(
        keys_in(apple, "beat_2") >= 1,
        "moves into its slot in beat 2"
    );
    // No second copy: neither beat has a scene layer of the apple.
    assert!(images_of(&p, "beat_1", "apple.png").is_empty());
    assert!(images_of(&p, "beat_2", "apple.png").is_empty());
    // The shared layer draws the apple's file.
    assert!(matches!(&apple.layer.kind, LayerKind::Image { asset, .. }
        if p.assets.iter().any(|a| a.id == *asset && a.path.ends_with("apple.png"))));
    assert!(with_code(&w, WARN_CARRY_IGNORED).is_empty(), "{w:?}");
}

#[test]
fn carry_in_lands_in_the_slot_the_next_beat_gives_the_picture() {
    // The same two beats without the carry: beat 2 draws its apple locally.
    let (plain, _) = compile(
        "slot_plain",
        vec![hero("apple", None), hero("apple", None)],
        "editorial",
        None,
    );
    let local = images_of(&plain, "beat_2", "apple.png");
    assert_eq!(local.len(), 1, "the local picture of beat 2");
    let local = local[0];

    let (p, _) = compile(
        "slot_carried",
        vec![hero("apple", Some("carry_primary")), hero("apple", None)],
        "editorial",
        None,
    );
    let e = apple(&p);
    // The track key beat 2 adds moves the element to the local slot: same
    // anchor point, and the scale that gives the local size.
    let arrive = e
        .track
        .iter()
        .find(|k| k.scene == "beat_2" && k.state.scale.is_some())
        .expect("a key that moves the picture into its slot");
    let (x, y) = (arrive.state.x.expect("x"), arrive.state.y.expect("y"));
    assert!((x - local.x).abs() < 0.5, "x {x} vs {}", local.x);
    assert!((y - local.y).abs() < 0.5, "y {y} vs {}", local.y);
    let scale = arrive.state.scale.expect("scale");
    assert!(
        (e.layer.width * scale - local.width).abs() < 0.5,
        "{} x {scale} vs {}",
        e.layer.width,
        local.width
    );
    assert_eq!(arrive.state.opacity, Some(1.0));
}

#[test]
fn the_second_picture_of_a_pair_lands_in_the_slot_of_the_next_pair() {
    // The pear is the pair's second picture in beat 1 (carried on) and its
    // first picture in beat 2.
    let story = |carry: Option<&str>| {
        vec![
            pair("apple", "pear", Some("grow"), carry),
            pair("pear", "plum", Some("grow"), None),
        ]
    };
    let (plain, _) = compile("pair_slot_plain", story(None), "editorial", None);
    let local = images_of(&plain, "beat_2", "pear.png");
    assert_eq!(local.len(), 1, "the pear of beat 2 drawn locally");
    let local = local[0];

    let (p, w) = compile(
        "pair_slot_carried",
        story(Some("carry_secondary")),
        "editorial",
        None,
    );
    let e = shared(&p, "shared.pic.pear");
    let arrive = e
        .track
        .iter()
        .find(|k| k.scene == "beat_2" && k.state.scale.is_some())
        .expect("a key that moves the pear into its slot");
    let (x, y) = (arrive.state.x.expect("x"), arrive.state.y.expect("y"));
    assert!((x - local.x).abs() < 0.5, "x {x} vs {}", local.x);
    assert!((y - local.y).abs() < 0.5, "y {y} vs {}", local.y);
    let scale = arrive.state.scale.expect("scale");
    assert!(
        (e.layer.width * scale - local.width).abs() < 0.5,
        "{} x {scale} vs {}",
        e.layer.width,
        local.width
    );
    // It is on screen from beat 1's READ: the picture is already readable
    // (opacity 0.5 on its ease-out entrance) by then, though the second
    // picture of a pair would wait for the narrator or for EVOLVE.
    let read = scene(&p, "beat_1").lifecycle.expect("lifecycle").read;
    let (k0, k1) = (&e.track[0], &e.track[1]);
    let progress = ((read - k0.at) / (k1.at - k0.at)).clamp(0.0, 1.0);
    let opacity = 1.0 - (1.0 - progress).powi(3);
    assert!(opacity >= 0.5, "opacity {opacity} at READ {read}: {k0:?}");
    assert!(images_of(&p, "beat_1", "pear.png").is_empty());
    assert!(images_of(&p, "beat_2", "pear.png").is_empty());
    assert!(with_code(&w, WARN_CARRY_IGNORED).is_empty(), "{w:?}");
}

#[test]
fn a_picture_the_next_beat_does_not_show_settles_into_the_anchor_chip() {
    let (p, w) = compile(
        "anchor_chip",
        vec![
            hero("apple", Some("carry_primary")),
            hero("pear", None),
            hero("plum", None),
        ],
        "editorial",
        None,
    );
    let e = apple(&p);
    // It stays into beat 2 as a small chip and leaves with the beat.
    let chip = e
        .track
        .iter()
        .find(|k| k.scene == "beat_2" && k.role.as_deref() == Some("anchor"))
        .expect("settles into the anchor chip");
    assert!(
        chip.state.scale.is_some_and(|s| s < 0.6),
        "{:?}",
        chip.state
    );
    let exit = e.track.last().expect("last key");
    assert_eq!(exit.scene, "beat_2");
    assert_eq!(exit.state.opacity, Some(0.0), "leaves with the beat");
    // Beat 2 shows its own picture: the pear, one copy, and not the apple.
    assert!(images_of(&p, "beat_2", "apple.png").is_empty());
    assert_eq!(images_of(&p, "beat_2", "pear.png").len(), 1);
    assert!(with_code(&w, WARN_CARRY_IGNORED).is_empty(), "{w:?}");
}

#[test]
fn the_last_beat_carries_nothing_and_says_so() {
    let (p, w) = compile(
        "last_beat",
        vec![hero("pear", None), hero("apple", Some("carry_primary"))],
        "editorial",
        None,
    );
    assert!(p.shared.is_empty(), "{:?}", ids(&p));
    assert_eq!(
        images_of(&p, "beat_2", "apple.png").len(),
        1,
        "drawn locally"
    );
    let hits = with_code(&w, WARN_CARRY_IGNORED);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert!(hits[0].message.contains("last beat"), "{}", hits[0].message);
}

#[test]
fn both_pictures_of_a_pair_are_carried() {
    // Beat 1: the apple, carried on. Beat 2: the apple (carried in, first
    // picture) against the pear, and the pear carried on (carry_secondary).
    // Beat 3: the pear (carried in, first picture) against the plum.
    let (p, w) = compile(
        "pair_both",
        vec![
            hero("apple", Some("carry_primary")),
            pair("apple", "pear", Some("grow"), Some("carry_secondary")),
            pair("pear", "plum", Some("accumulate"), None),
        ],
        "editorial",
        None,
    );
    assert_eq!(ids(&p), ["shared.pic.apple", "shared.pic.pear"], "{w:?}");
    let (apple, pear) = (apple(&p), shared(&p, "shared.pic.pear"));
    assert!(keys_in(apple, "beat_1") >= 1 && keys_in(apple, "beat_2") >= 1);
    assert_eq!(keys_in(apple, "beat_3"), 0, "the apple leaves with beat 2");
    assert!(keys_in(pear, "beat_2") >= 2 && keys_in(pear, "beat_3") >= 1);
    // Every picture of the pair is a shared element: no local copy of either
    // in beat 2, none of the pear in beat 3; the plum is beat 3's own.
    assert!(images_of(&p, "beat_2", "apple.png").is_empty());
    assert!(images_of(&p, "beat_2", "pear.png").is_empty());
    assert!(images_of(&p, "beat_3", "pear.png").is_empty());
    assert_eq!(images_of(&p, "beat_3", "plum.png").len(), 1);
    // The relationship is still drawn between them (an arrow, then a plus).
    assert!(with_code(&w, WARN_RELATIONSHIP_DROPPED).is_empty(), "{w:?}");
    assert!(with_code(&w, WARN_CARRY_IGNORED).is_empty(), "{w:?}");
}

#[test]
fn the_asset_in_a_collection_next_beat_settles_into_the_anchor_chip() {
    // The collection builder draws its own items; the carried picture is not
    // one of them. It stays on screen as the chip and leaves with the beat.
    let collection = json!({
        "purpose": "reveal", "statement": "Three fruits", "keyword": "fruit",
        "primary": {
            "kind": "collection", "meaning": "the bowl",
            "items": [
                { "kind": "object", "asset": "apple", "meaning": "apple", "value": "$25,000" },
                { "kind": "object", "asset": "pear", "meaning": "pear", "value": "$75,000" },
                { "kind": "object", "asset": "plum", "meaning": "plum", "value": "$185,000" }
            ]
        },
        "energy": "building"
    });
    let (p, w) = compile(
        "collection_next",
        vec![hero("apple", Some("carry_primary")), collection],
        "editorial",
        None,
    );
    let e = apple(&p);
    assert!(
        e.track
            .iter()
            .any(|k| k.scene == "beat_2" && k.role.as_deref() == Some("anchor")),
        "{:?}",
        e.track
    );
    assert!(with_code(&w, WARN_CARRY_IGNORED).is_empty(), "{w:?}");
}

#[test]
fn a_picture_is_never_carried_into_or_out_of_an_accent_flood() {
    // A beat that enters with an accent flood (energy impact under a look whose
    // handoffs flood) stacks its stage above every shared picture.
    let mut impact = hero("apple", None);
    impact["energy"] = json!("impact");
    let (p, w) = compile(
        "accent",
        vec![hero("apple", Some("carry_primary")), impact],
        "playful",
        None,
    );
    assert!(p.shared.is_empty(), "{:?}", ids(&p));
    let hits = with_code(&w, WARN_CARRY_IGNORED);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert!(
        hits[0].message.contains("accent flood"),
        "{}",
        hits[0].message
    );
}

// ---------------------------------------------------------------------------
// Layering across the handoff
// ---------------------------------------------------------------------------

#[test]
fn a_wipe_look_draws_the_carried_picture_above_the_wipes() {
    // The playful look hands off with panel wipes at z 60 / 61: the carried
    // picture is above them, so it is never hidden behind a wipe.
    let beats = vec![
        hero("pear", None),
        pair("apple", "pear", Some("grow"), Some("carry_primary")),
        pair("apple", "plum", Some("grow"), None),
    ];
    let (p, _) = compile("wipes", beats.clone(), "playful", None);
    let e = apple(&p);
    assert!(e.layer.z_index > 61, "z {}", e.layer.z_index);
    // The look really wipes into the beat that receives the picture.
    let mut layers = Vec::new();
    all_layers(&scene(&p, "beat_3").layers, &mut layers);
    assert!(
        layers
            .iter()
            .any(|l| l.id.ends_with(".wipe_a") || l.id.ends_with(".wipe")),
        "beat 3 enters behind a wipe"
    );
    // Without wipes the picture sits in the stage's layering: below the front
    // group that keeps labels and figures above it.
    let (flat, _) = compile("no_wipes", beats, "editorial", None);
    let e = apple(&flat);
    assert!(e.layer.z_index < 60, "z {}", e.layer.z_index);
    assert!(
        scene(&flat, "beat_2")
            .layers
            .iter()
            .any(|l| l.id.ends_with("stage_front")),
        "the stage's front group stays above the picture"
    );
}

// ---------------------------------------------------------------------------
// carry_ignored: only when the look really cannot carry, and it says why
// ---------------------------------------------------------------------------

#[test]
fn carry_ignored_names_the_look_that_cannot_carry_pictures() {
    // Two stories: a hero carried into a hero, and the second picture of a pair
    // carried into a pair.
    let stories: [Vec<Value>; 2] = [
        vec![hero("pear", Some("carry_primary")), hero("apple", None)],
        vec![
            pair("pear", "plum", Some("grow"), Some("carry_secondary")),
            pair("plum", "apple", Some("grow"), None),
        ],
    ];
    let mut fired = std::collections::BTreeSet::new();
    for look in [
        Look::Cinematic3d,
        Look::Dossier,
        Look::StudioPop,
        Look::StreetCollage,
        Look::HypeSlam,
    ] {
        for (n, beats) in stories.iter().enumerate() {
            let (_, w) = compile(
                &format!("ignored_{}_{n}", look.name()),
                beats.clone(),
                "editorial",
                Some(ArtMode::Force(look)),
            );
            // A genre look may carry what its builder places (the text-style
            // carry); when nothing is carried the warning names the look.
            for hit in with_code(&w, WARN_CARRY_IGNORED) {
                assert!(
                    hit.message.contains(look.name())
                        && hit.message.contains("does not carry library pictures"),
                    "{}: {}",
                    look.name(),
                    hit.message
                );
                fired.insert(look.name());
            }
        }
    }
    // The cinematic look drops the hero carry, the dossier the pair's.
    assert!(fired.contains("cinematic_3d"), "{fired:?}");
    assert!(fired.contains("dossier"), "{fired:?}");
    // A flat look carries it: nothing is reported.
    for look in [
        Look::ClassicalNeon,
        Look::HalftoneCutout,
        Look::ClayPop,
        Look::OrnamentEditorial,
        Look::Journey,
    ] {
        let (p, w) = compile(
            &format!("carried_{}", look.name()),
            vec![hero("pear", Some("carry_primary")), hero("apple", None)],
            "editorial",
            Some(ArtMode::Force(look)),
        );
        assert_eq!(p.shared.len(), 1, "{}: {:?}", look.name(), ids(&p));
        assert!(
            with_code(&w, WARN_CARRY_IGNORED).is_empty(),
            "{}: {w:?}",
            look.name()
        );
    }
}

// ---------------------------------------------------------------------------
// relationship_dropped (compile level)
// ---------------------------------------------------------------------------

#[test]
fn the_flat_looks_draw_the_relationship_of_a_pair() {
    for (relationship, word) in [
        ("grow", "arrow"),
        ("replace", "arrow"),
        ("compress", "arrow"),
        ("accumulate", "badge"),
        ("separate", "badge"),
        ("carry", "link"),
    ] {
        let (p, w) = compile(
            &format!("rel_{relationship}"),
            vec![pair("apple", "pear", Some(relationship), None)],
            "editorial",
            None,
        );
        let mut layers = Vec::new();
        all_layers(&scene(&p, "beat_1").layers, &mut layers);
        assert!(
            layers.iter().any(|l| l.id.contains(word)),
            "{relationship}: a {word} layer"
        );
        assert!(
            with_code(&w, WARN_RELATIONSHIP_DROPPED).is_empty(),
            "{relationship}: {w:?}"
        );
    }
}

#[test]
fn a_look_that_draws_no_connector_reports_the_dropped_relationship() {
    // The dossier look stages a pair of pictures without a connector (W8b).
    let (_, w) = compile(
        "rel_dossier",
        vec![pair("apple", "pear", Some("grow"), None)],
        "editorial",
        Some(ArtMode::Force(Look::Dossier)),
    );
    let hits = with_code(&w, WARN_RELATIONSHIP_DROPPED);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert_eq!(hits[0].beat, Some(0));
    assert!(hits[0].message.contains("grow"), "{}", hits[0].message);
    // No relationship named: nothing to drop.
    let (_, w) = compile(
        "rel_none",
        vec![pair("apple", "pear", None, None)],
        "editorial",
        Some(ArtMode::Force(Look::Dossier)),
    );
    assert!(with_code(&w, WARN_RELATIONSHIP_DROPPED).is_empty(), "{w:?}");
}

// ---------------------------------------------------------------------------
// Determinism, and nothing changes without a carry request
// ---------------------------------------------------------------------------

#[test]
fn a_carry_compiles_to_the_same_scene_every_time() {
    let beats = vec![
        hero("apple", Some("carry_primary")),
        pair("apple", "pear", Some("replace"), Some("carry_secondary")),
        pair("pear", "plum", Some("grow"), None),
    ];
    let a = compile("det_a", beats.clone(), "playful", None).0;
    let b = compile("det_b", beats, "playful", None).0;
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
}

#[test]
fn a_carry_lands_inside_the_canvas_on_every_canvas_class() {
    // Legacy vertical, square, landscape and a tall canvas (the content is laid
    // out as on a 1920u canvas and shifted: the keys shift with it).
    let beats = vec![
        hero("apple", Some("carry_primary")),
        pair("apple", "pear", Some("grow"), Some("carry_secondary")),
        pair("pear", "plum", Some("accumulate"), None),
    ];
    for (w, h) in [(1080, 1920), (1080, 1080), (1920, 1080), (1080, 2400)] {
        for tone in ["editorial", "playful"] {
            let (p, warnings) = compile_on(
                &format!("canvas_{w}x{h}_{tone}"),
                beats.clone(),
                tone,
                None,
                Some((w, h)),
            );
            assert_eq!(
                ids(&p),
                ["shared.pic.apple", "shared.pic.pear"],
                "{w}x{h} {tone}: {warnings:?}"
            );
            for e in &p.shared {
                for k in &e.track {
                    for (x, limit) in [(k.state.x, w), (k.state.y, h)] {
                        if let Some(v) = x {
                            assert!(
                                v >= -0.1 * limit as f32 && v <= 1.1 * limit as f32,
                                "{w}x{h} {tone}: {} key {k:?}",
                                e.id
                            );
                        }
                    }
                }
            }
            assert!(
                with_code(&warnings, WARN_CARRY_IGNORED).is_empty(),
                "{w}x{h} {tone}: {warnings:?}"
            );
        }
    }
}

#[test]
fn the_default_compile_keeps_pictures_as_it_always_did_and_says_why() {
    // Without a direction seed (no `--variety`) nothing is carried: the same two
    // beats compile to the same scene with or without the carry request, and the
    // warning names the reason.
    let story = |carry: Option<&str>| vec![hero("apple", carry), hero("apple", None)];
    let (plain, _) = compile_default("default_plain", story(None), "editorial", None);
    let (asked, w) = compile_default(
        "default_asked",
        story(Some("carry_primary")),
        "editorial",
        None,
    );
    assert!(asked.shared.is_empty(), "{:?}", ids(&asked));
    // The carry request is the only difference between the intents, and the
    // scene does not show it.
    assert_eq!(plain.to_json_pretty(), asked.to_json_pretty());
    let hits = with_code(&w, WARN_CARRY_IGNORED);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert!(
        hits[0].message.contains("--variety"),
        "the reason names the product path: {}",
        hits[0].message
    );
    // With a seed the same story carries.
    let (seeded, w) = compile(
        "default_seeded",
        story(Some("carry_primary")),
        "editorial",
        None,
    );
    assert_eq!(ids(&seeded), ["shared.pic.apple"], "{w:?}");
}

#[test]
fn a_story_without_a_carry_request_has_no_shared_element() {
    let (p, w) = compile(
        "no_request",
        vec![
            hero("apple", None),
            pair("apple", "pear", Some("grow"), None),
        ],
        "editorial",
        None,
    );
    assert!(p.shared.is_empty(), "{:?}", ids(&p));
    assert!(with_code(&w, WARN_CARRY_IGNORED).is_empty(), "{w:?}");
}
