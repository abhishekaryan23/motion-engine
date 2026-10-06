//! (0.22) Every value and picture the intent gives shows in the genre looks,
//! keyword stamps land only on words the narrator says, and a picture the
//! narration never names is reported (`unnamed_picture`). In-repo pictures
//! only: catalog objects (`piggy_bank`, `rocket`, `robot`) and the library's
//! own `shopping_basket`.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions,
    CompileWarning, WARN_UNNAMED_PICTURE,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_report;
use motion_core::scene::{Layer, LayerKind, MotionProject, Scene};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use motion_core::validate::validate;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn library() -> AssetLibrary {
    AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
        "clay_props_3d".to_string(),
    ])
}

fn style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!(
        r#"{{"tone":"{tone}","polarity":"dark","temperature":"warm","temperament":"balanced",
        "density":"dense","accent_role":"signal_red","texture_style":"heavy_print",
        "depth":"layered","camera_style":"drift","motion_language":"parallax"}}"#
    ))
    .expect("style")
}

/// A neutral (classic) style: no genre look.
fn classic_style() -> StyleProfile {
    serde_json::from_str(
        r#"{"material":"paper","typography_style":"grotesk_serif","depth":"layered",
        "camera_style":"slow_push","motion_language":"auto","texture_style":"subtle_print",
        "accent_role":"cobalt","seed":1}"#,
    )
    .expect("style")
}

fn options(
    look: Option<Look>,
    canvas: Option<(u32, u32)>,
    speech: Option<SpeechMap>,
) -> CompileOptions {
    CompileOptions {
        art: look.map(ArtMode::Force),
        canvas,
        speech,
        ..CompileOptions::default()
    }
}

fn compile(
    intent: &str,
    style: &StyleProfile,
    opts: &CompileOptions,
) -> (MotionProject, Vec<CompileWarning>) {
    let intent = CreativeIntent::from_json(intent).expect("intent");
    compile_with_report(
        &intent,
        style,
        None,
        &library(),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        opts,
    )
    .expect("compile")
}

/// A voice-over that says each beat's narration (else statement), words
/// 0.32 s apart and 0.5 s between beats.
fn speech_for(intent: &str) -> SpeechMap {
    let intent = CreativeIntent::from_json(intent).expect("intent");
    let (mut words, mut sentences) = (Vec::new(), Vec::new());
    let mut t = 0.3;
    for (i, beat) in intent.beats.iter().enumerate() {
        let line = beat
            .narration
            .clone()
            .unwrap_or_else(|| beat.statement.clone());
        let start = t;
        for w in motion_core::speech::statement_tokens(&line) {
            words.push(SpeechWord {
                text: w,
                start: (t * 1000.0_f64).round() / 1000.0,
                end: ((t + 0.27) * 1000.0_f64).round() / 1000.0,
                confidence: 1.0,
            });
            t += 0.32;
        }
        sentences.push(SpeechSentence {
            beat: i,
            start,
            end: t,
        });
        t += 0.5;
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration: t,
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words,
        sentences,
    }
}

fn beat_scenes(p: &MotionProject) -> Vec<&Scene> {
    p.scenes.iter().filter(|s| s.lifecycle.is_some()).collect()
}

fn all_layers<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            all_layers(children, out);
        }
    }
}

fn flat(s: &Scene) -> Vec<&Layer> {
    let mut v = Vec::new();
    all_layers(&s.layers, &mut v);
    v
}

/// The layer ids of a beat without the scene prefix (`b2.photo.0` -> `photo.0`).
fn local(id: &str) -> &str {
    id.split_once('.').map_or(id, |(_, rest)| rest)
}

/// Every text shown in a beat (layer text, upper-cased for comparison).
fn texts(s: &Scene) -> Vec<String> {
    flat(s)
        .into_iter()
        .filter_map(|l| match &l.kind {
            LayerKind::Text(t) => Some(t.text.replace('\n', " ").to_uppercase()),
            _ => None,
        })
        .collect()
}

/// Picture layers (images and SVGs) of a beat whose local id starts with `under`.
fn pictures_under<'a>(s: &'a Scene, under: &str) -> Vec<&'a Layer> {
    flat(s)
        .into_iter()
        .filter(|l| {
            matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. })
                && local(&l.id).starts_with(under)
        })
        .collect()
}

fn has_text(s: &Scene, want: &str) -> bool {
    texts(s).iter().any(|t| t.contains(&want.to_uppercase()))
}

/// Ids (local) of a beat's layers that end with `suffix`.
fn has_layer(s: &Scene, suffix: &str) -> bool {
    flat(s).iter().any(|l| local(&l.id) == suffix)
}

// ---------------------------------------------------------------------------
// The documentary pair
// ---------------------------------------------------------------------------

/// Two pictured subjects with values compared, then an object compared with a
/// number. `shopping_basket` is a library picture by name, `piggy_bank` a
/// catalog object.
const PAIR: &str = r#"{
  "version": "0.2", "title": "pair", "format": "vertical",
  "beats": [
    {"purpose": "compare", "statement": "Piggy bank versus basket",
     "narration": "The piggy bank holds 58 dollars. The shopping basket costs 127 dollars.",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$58", "meaning": "savings"},
     "secondary": {"kind": "object", "asset": "shopping_basket", "value": "$127", "meaning": "groceries"},
     "relationship": "separate", "energy": "impact", "keyword": "price"},
    {"purpose": "contrast", "statement": "Salary against groceries",
     "narration": "Your salary is flat. Groceries rose 38 percent.",
     "primary": {"kind": "number", "value": "₹50,000", "meaning": "salary"},
     "secondary": {"kind": "object", "asset": "shopping_basket", "value": "+38%", "meaning": "groceries"},
     "energy": "impact", "keyword": "groceries"}
  ]
}"#;

#[test]
fn the_documentary_pair_shows_both_pictures_and_both_values() {
    let (p, _) = compile(
        PAIR,
        &style("documentary"),
        &options(Some(Look::Dossier), None, None),
    );
    validate(&p, None).expect("validate");
    let scenes = beat_scenes(&p);
    // Beat 1: two documents, each a picture with its figure card.
    let s = scenes[0];
    assert!(
        !pictures_under(s, "photo.0").is_empty(),
        "first picture missing"
    );
    assert!(
        !pictures_under(s, "photo.1").is_empty(),
        "second picture missing"
    );
    for (card, value, label) in [
        ("figure.0", "$58", "SAVINGS"),
        ("figure.1", "$127", "GROCERIES"),
    ] {
        let v = flat(s)
            .into_iter()
            .find(|l| local(&l.id) == format!("{card}.value"))
            .unwrap_or_else(|| panic!("{card}: no value"));
        assert!(
            matches!(&v.kind, LayerKind::Text(t) if t.text == value),
            "{card}"
        );
        assert!(has_layer(s, &format!("{card}.label")), "{card}: no label");
        assert!(has_layer(s, &format!("{card}.underline")), "{card}");
        assert!(has_text(s, label), "{card}: label {label}");
    }
    // The comparison is what the beat is about: the clipping, stamped.
    let focal = p
        .project
        .art
        .as_ref()
        .and_then(|a| a.focal.get(&s.id))
        .expect("focal");
    assert_eq!(local(focal), "clip");
    assert!(
        has_text(s, "PRICE"),
        "no voice-over: the keyword is stamped"
    );
    // Beat 2: a figure (the salary) against a picture with its figure.
    let s = scenes[1];
    assert!(!pictures_under(s, "photo.1").is_empty());
    assert!(pictures_under(s, "photo.0").is_empty());
    assert!(
        has_text(s, "₹50,000") && has_text(s, "+38%"),
        "{:?}",
        texts(s)
    );
}

#[test]
fn the_documentary_pair_is_laid_out_cleanly_on_every_canvas() {
    for canvas in [(1080, 1920), (1080, 1080), (1920, 1080), (1080, 1350)] {
        for speech in [None, Some(speech_for(PAIR))] {
            let with_voice = speech.is_some();
            let (p, _) = compile(
                PAIR,
                &style("documentary"),
                &options(Some(Look::Dossier), Some(canvas), speech),
            );
            validate(&p, None).expect("validate");
            let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
            let report = layout_report(&p, &frame);
            assert!(
                report.passed(),
                "layout QA {canvas:?} voice {with_voice}\n{}",
                report.to_text()
            );
            // Once everything has landed, no two cards of the pair overlap
            // and every card is on the canvas.
            for s in beat_scenes(&p) {
                let life = s.lifecycle.expect("lifecycle");
                let at = life.anticipate - 0.05;
                let n = ((s.start_seconds + at) * 30.0).round() as u32;
                let cards = card_boxes(&p, n, s);
                let (w, h) = (p.canvas.width as f32, p.canvas.height as f32);
                for (id, b) in &cards {
                    assert!(
                        b[0] >= -2.0 && b[1] >= -2.0 && b[2] <= w + 2.0 && b[3] <= h + 2.0,
                        "{canvas:?} {}: {id} off canvas {b:?}",
                        s.id
                    );
                }
                for (i, (a, ba)) in cards.iter().enumerate() {
                    for (c, bc) in &cards[i + 1..] {
                        let overlap = ba[0] < bc[2] - 1.0
                            && bc[0] < ba[2] - 1.0
                            && ba[1] < bc[3] - 1.0
                            && bc[1] < ba[3] - 1.0;
                        assert!(
                            !overlap,
                            "{canvas:?} voice {with_voice} {}: {a} {ba:?} overlaps {c} {bc:?}",
                            s.id
                        );
                    }
                }
            }
        }
    }
}

/// Resolved boxes `[x0, y0, x1, y1]` of the pair's card groups (`photo.N`,
/// `figure.N`, `clip`) at frame `n`.
fn card_boxes(p: &MotionProject, n: u32, s: &Scene) -> Vec<(String, [f32; 4])> {
    fn walk(layers: &[ResolvedLayer], prefix: &str, out: &mut Vec<(String, [f32; 4])>) {
        for l in layers {
            let name = l.id.strip_prefix(prefix).unwrap_or("");
            let card = name == "clip"
                || ((name.starts_with("photo.") || name.starts_with("figure."))
                    && name.split('.').count() == 2);
            if card {
                let t = l.transform;
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for (cx, cy) in [
                    (0.0, 0.0),
                    (l.width, 0.0),
                    (0.0, l.height),
                    (l.width, l.height),
                ] {
                    let x = t.a * cx + t.c * cy + t.e;
                    let y = t.b * cx + t.d * cy + t.f;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                out.push((name.to_string(), [x0, y0, x1, y1]));
            }
            walk(&l.children, prefix, out);
        }
    }
    let f = evaluate_frame(p, n).expect("frame");
    let mut out = Vec::new();
    let prefix = format!("{}.", s.id.replace("beat_", "b"));
    walk(&f.layers, &prefix, &mut out);
    out
}

// ---------------------------------------------------------------------------
// One object with a value: a stat card in every look
// ---------------------------------------------------------------------------

const SINGLE: &str = r#"{
  "version": "0.2", "title": "single", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Savings grow",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$58", "meaning": "savings"},
     "energy": "impact", "keyword": "grow"}
  ]
}"#;

#[test]
fn a_single_object_value_shows_in_every_genre_look_and_the_classic_one() {
    let cases: [(&str, Option<Look>, StyleProfile); 5] = [
        ("dossier", Some(Look::Dossier), style("documentary")),
        ("hype", Some(Look::HypeSlam), style("hype")),
        ("street", Some(Look::StreetCollage), style("street")),
        ("studio", Some(Look::StudioPop), style("studio")),
        ("classic", None, classic_style()),
    ];
    for (name, look, st) in cases {
        let (p, _) = compile(SINGLE, &st, &options(look, None, None));
        validate(&p, None).expect("validate");
        let s = beat_scenes(&p)[0];
        assert!(has_text(s, "$58"), "{name}: value missing {:?}", texts(s));
        assert!(
            flat(s)
                .iter()
                .any(|l| matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. })),
            "{name}: picture missing"
        );
    }
    // Hype: the figure slams above its picture (the same cut).
    let (p, _) = compile(
        SINGLE,
        &style("hype"),
        &options(Some(Look::HypeSlam), None, None),
    );
    let s = beat_scenes(&p)[0];
    let cut_of_value = flat(s)
        .into_iter()
        .find(|l| matches!(&l.kind, LayerKind::Text(t) if t.text.contains("58")))
        .map(|l| l.id.split('.').take(3).collect::<Vec<_>>().join("."))
        .expect("value cut");
    assert!(
        flat(s)
            .iter()
            .any(|l| l.id.starts_with(&format!("{cut_of_value}.pic"))),
        "the picture is on the value's cut {cut_of_value}"
    );
}

#[test]
fn hero_object_stamps_the_secondary_value_too() {
    let intent = r#"{
      "version": "0.2", "title": "two", "format": "vertical",
      "beats": [
        {"purpose": "emphasize", "statement": "Two prizes",
         "primary": {"kind": "object", "asset": "piggy_bank", "value": "$90", "meaning": "savings"},
         "secondary": {"kind": "object", "asset": "shopping_basket", "value": "$12", "meaning": "groceries"},
         "energy": "building"}
      ]
    }"#;
    for canvas in [(1080, 1920), (1080, 1080), (1920, 1080)] {
        let (p, _) = compile(intent, &classic_style(), &options(None, Some(canvas), None));
        validate(&p, None).expect("validate");
        let s = beat_scenes(&p)[0];
        let stamp = flat(s)
            .into_iter()
            .find(|l| local(&l.id) == "support_stamp")
            .unwrap_or_else(|| panic!("{canvas:?}: no secondary stamp"));
        assert!(matches!(&stamp.kind, LayerKind::Text(t) if t.text == "$12"));
        assert!(has_text(s, "$90"));
        let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
        let report = layout_report(&p, &frame);
        assert!(report.passed(), "{canvas:?}\n{}", report.to_text());
    }
}

// ---------------------------------------------------------------------------
// Stamps only on spoken words
// ---------------------------------------------------------------------------

/// Beat 1 stamps a keyword the narrator never says; beat 2 one he says
/// ("warning" as "warns").
const STAMPS: &str = r#"{
  "version": "0.2", "title": "stamps", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Fifty litres",
     "narration": "Fifty litres of fuel fill the same tank.",
     "primary": {"kind": "number", "value": "50 L", "meaning": "same tank"},
     "secondary": {"kind": "object", "asset": "piggy_bank", "meaning": "piggy bank"},
     "energy": "impact", "keyword": "pump"},
    {"purpose": "reveal", "statement": "A forecast",
     "narration": "The bank warns the piggy bank could lose four percent.",
     "primary": {"kind": "number", "value": "4%", "meaning": "forecast"},
     "secondary": {"kind": "object", "asset": "piggy_bank", "meaning": "piggy bank"},
     "energy": "impact", "keyword": "warning"}
  ]
}"#;

/// Whether beat `i` shows its keyword as a stamp / sticker / burst / slam.
fn stamped(p: &MotionProject, i: usize, word: &str) -> bool {
    let s = beat_scenes(p)[i];
    flat(s).into_iter().any(|l| {
        let id = local(&l.id);
        let loud = id.starts_with("stamp")
            || id.starts_with("keytag")
            || id.starts_with("burst")
            || (id.starts_with("cut.") && id.ends_with(".word"));
        loud && matches!(&l.kind, LayerKind::Text(t) if t.text.to_uppercase().contains(&word.to_uppercase()))
    })
}

#[test]
fn keywords_are_stamped_only_when_the_narrator_says_them() {
    let looks = [
        (Look::Dossier, style("documentary")),
        (Look::StreetCollage, style("street")),
        (Look::HypeSlam, style("hype")),
    ];
    for (look, st) in looks {
        // Without a voice-over: unchanged, both keywords stamped.
        let (p, _) = compile(STAMPS, &st, &options(Some(look), None, None));
        assert!(stamped(&p, 0, "pump"), "{look:?}: silent piece stamps PUMP");
        assert!(
            stamped(&p, 1, "warning"),
            "{look:?}: silent piece stamps WARNING"
        );
        // With one: "pump" is never said, "warning" is ("warns").
        let (p, _) = compile(
            STAMPS,
            &st,
            &options(Some(look), None, Some(speech_for(STAMPS))),
        );
        validate(&p, None).expect("validate");
        assert!(!stamped(&p, 0, "pump"), "{look:?}: PUMP is never said");
        assert!(
            stamped(&p, 1, "warning"),
            "{look:?}: WARNING is said (warns)"
        );
    }
}

// ---------------------------------------------------------------------------
// unnamed_picture
// ---------------------------------------------------------------------------

fn unnamed(w: &[CompileWarning]) -> Vec<&CompileWarning> {
    w.iter()
        .filter(|w| w.code == WARN_UNNAMED_PICTURE)
        .collect()
}

#[test]
fn a_picture_the_narration_never_names_is_reported() {
    // Wallet Atlas' fuel story, beat 1: the US flag on a line about fuel.
    let intent = r#"{
      "version": "0.2", "title": "fuel", "format": "vertical",
      "beats": [
        {"purpose": "emphasize", "statement": "Same tank. Five prices.",
         "narration": "Fifty litres of fuel. Same tank. Five countries. The price difference will surprise you.",
         "primary": {"kind": "number", "value": "50 L", "meaning": "same tank"},
         "secondary": {"kind": "object", "asset": "shopping_basket", "meaning": "United States"},
         "energy": "impact", "keyword": "fuel"},
        {"purpose": "compare", "statement": "US vs Germany",
         "narration": "In the United States that tank costs just $58. But in Germany the same fill-up runs $127.",
         "primary": {"kind": "object", "asset": "shopping_basket", "value": "$58", "meaning": "United States"},
         "secondary": {"kind": "object", "asset": "piggy_bank", "value": "$127", "meaning": "Germany"},
         "relationship": "separate", "energy": "impact", "keyword": "price"},
        {"purpose": "contrast", "statement": "Tax is the gap",
         "narration": "Germany adds over $1.15 of tax per litre. The US adds just $0.14.",
         "primary": {"kind": "object", "asset": "piggy_bank", "value": "$1.15 / L", "meaning": "Germany's fuel tax"},
         "secondary": {"kind": "object", "asset": "shopping_basket", "value": "$0.14 / L", "meaning": "US fuel tax"},
         "relationship": "compress", "energy": "impact", "keyword": "tax"}
      ]
    }"#;
    let check = |w: &[CompileWarning], voice: &str| {
        let hits = unnamed(w);
        assert_eq!(hits.len(), 1, "{voice}: {hits:?}");
        assert_eq!(hits[0].beat, Some(0));
        assert!(
            hits[0].message.contains("'United States'")
                && hits[0].message.contains("never mentioned"),
            "{}",
            hits[0].message
        );
    };
    // A plain compile (no voice-over) reports it: the narration text decides.
    let (_, w) = compile(intent, &style("documentary"), &options(None, None, None));
    check(&w, "no voice");
    // The same with a voice-over.
    let (_, w) = compile(
        intent,
        &style("documentary"),
        &options(Some(Look::Dossier), None, Some(speech_for(intent))),
    );
    check(&w, "voice");
}

#[test]
fn abbreviations_name_a_picture_only_in_capitals() {
    let story = |line: &str| {
        format!(
            r#"{{"version": "0.2", "title": "uk", "format": "vertical",
            "beats": [{{"purpose": "reveal", "statement": "A forecast",
             "narration": "{line}",
             "primary": {{"kind": "number", "value": "4%", "meaning": "forecast"}},
             "secondary": {{"kind": "object", "asset": "shopping_basket", "meaning": "US"}},
             "energy": "impact"}}]}}"#
        )
    };
    let (_, w) = compile(
        &story("The US could see four percent."),
        &classic_style(),
        &options(None, None, None),
    );
    assert!(unnamed(&w).is_empty(), "{w:?}");
    let (_, w) = compile(
        &story("Let us see four percent."),
        &classic_style(),
        &options(None, None, None),
    );
    assert_eq!(
        unnamed(&w).len(),
        1,
        "the pronoun 'us' names nothing: {w:?}"
    );
}

#[test]
fn genre_values_compile_deterministically() {
    for look in [Look::Dossier, Look::HypeSlam, Look::StreetCollage] {
        let a = serde_json::to_string(
            &compile(
                PAIR,
                &style("documentary"),
                &options(Some(look), None, None),
            )
            .0,
        )
        .expect("json");
        let b = serde_json::to_string(
            &compile(
                PAIR,
                &style("documentary"),
                &options(Some(look), None, None),
            )
            .0,
        )
        .expect("json");
        assert_eq!(a, b, "{look:?}");
    }
    // The pair with a voice-over too.
    let speech = speech_for(PAIR);
    let run = || {
        let intent = CreativeIntent::from_json(PAIR).expect("intent");
        serde_json::to_string(
            &compile_with_options(
                &intent,
                &style("documentary"),
                None,
                &library(),
                &ApproxMeasure,
                &AssetManifest::empty(),
                None,
                &options(Some(Look::Dossier), None, Some(speech.clone())),
            )
            .expect("compile"),
        )
        .expect("json")
    };
    assert_eq!(run(), run());
}
