//! (0.24 M1) Anti-slop QA: one positive and one negative fixture per check,
//! plus the critique of the compiled editorial demo.

use std::path::PathBuf;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary};
use motion_core::scene::*;
use motion_core::slop_qa::SlopCheck;
use motion_core::{critique, slop_report, CreativeIntent, Easing, SlopRules, StyleProfile};

fn rules() -> SlopRules {
    SlopRules::bundled()
}

fn base_layer(id: &str, kind: LayerKind) -> Layer {
    Layer {
        id: id.to_string(),
        x: 100.0,
        y: 400.0,
        width: 600.0,
        height: 120.0,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_degrees: 0.0,
        anchor_x: 0.0,
        anchor_y: 0.0,
        opacity: 1.0,
        z_index: 0,
        visible: true,
        clip: None,
        mask: None,
        depth: None,
        z: None,
        tilt: None,
        layout: None,
        kind,
    }
}

fn text_in(id: &str, text: &str, role: FontRole, color: Color) -> Layer {
    base_layer(
        id,
        LayerKind::Text(TextStyle {
            text: text.to_string(),
            font_role: role,
            font_size: 64.0,
            font_weight: 400,
            italic: false,
            color,
            align: TextAlign::Left,
            line_height: 1.1,
            letter_spacing: 0.0,
            max_width: None,
            uppercase: false,
            ink: None,
            fill: None,
            outline: None,
        }),
    )
}

fn text(id: &str, t: &str) -> Layer {
    text_in(id, t, FontRole::Display, Color::rgb(0x22, 0x22, 0x22))
}

fn rect(id: &str, fill: Color, w: f32, h: f32) -> Layer {
    let mut l = base_layer(id, LayerKind::Rectangle { fill, stroke: None });
    l.width = w;
    l.height = h;
    l
}

fn fade(target: &str, start: f64, easing: Easing) -> Motion {
    Motion {
        id: None,
        target: target.to_string(),
        start,
        duration: 0.3,
        easing,
        spring: None,
        op: MotionOp::Fade { from: 0.0, to: 1.0 },
    }
}

fn rise(target: &str, start: f64, easing: Easing) -> Motion {
    Motion {
        op: MotionOp::Move {
            from: [0.0, 40.0],
            to: [0.0, 0.0],
            via: None,
        },
        ..fade(target, start, easing)
    }
}

fn scene(id: &str, layers: Vec<Layer>, motions: Vec<Motion>) -> Scene {
    Scene {
        id: id.to_string(),
        start_seconds: 0.0,
        duration_seconds: 4.0,
        layers,
        motions,
        camera: None,
        lifecycle: Some(Lifecycle {
            enter: 0.3,
            settle: 0.9,
            read: 1.0,
            evolve: 3.0,
            anticipate: 3.5,
            bridge: 4.0,
        }),
        post: vec![],
    }
}

fn project(scenes: Vec<Scene>) -> MotionProject {
    MotionProject {
        version: SCENE_VERSION.to_string(),
        project: ProjectMeta {
            name: "t".to_string(),
            duration_seconds: None,
            exploration: None,
            speech: None,
            art: None,
            direction: None,
        },
        canvas: Canvas {
            width: 1080,
            height: 1920,
            fps: 30,
            background: Color::rgb(0xF2, 0xED, 0xE4),
        },
        theme: Theme::default(),
        assets: vec![],
        asset_root: None,
        scenes,
        shared: vec![],
        envelopes: vec![],
    }
}

/// Two distinct, staggered, well-held beats: no finding of any kind.
fn clean() -> MotionProject {
    project(vec![
        scene(
            "b0",
            vec![text("b0.head", "Rates fell"), text("b0.sub", "again")],
            vec![
                fade("b0.head", 0.1, Easing::OutCubic),
                rise("b0.head", 0.1, Easing::OutQuint),
                rise("b0.sub", 0.5, Easing::EditorialSpring),
            ],
        ),
        scene(
            "b1",
            vec![text("b1.stat", "42"), text("b1.label", "percent")],
            vec![
                fade("b1.stat", 0.1, Easing::OutCubic),
                rise("b1.stat", 0.1, Easing::OutQuint),
                rise("b1.label", 0.5, Easing::EditorialSpring),
            ],
        ),
    ])
}

fn has(p: &MotionProject, c: SlopCheck) -> bool {
    slop_report(p, &rules())
        .findings
        .iter()
        .any(|f| f.check == c)
}

fn set_text(p: &mut MotionProject, id: &str, new: &str) {
    for s in &mut p.scenes {
        for l in &mut s.layers {
            if l.id == id {
                if let LayerKind::Text(t) = &mut l.kind {
                    t.text = new.to_string();
                }
            }
        }
    }
}

#[test]
fn clean_project_has_no_findings() {
    let r = slop_report(&clean(), &rules());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn bundled_rules_parse() {
    let r = rules();
    assert!(r.stock_phrases.len() >= 20);
    assert!(r.stock_phrases.iter().all(|p| *p == p.to_lowercase()));
}

#[test]
fn stock_phrase() {
    let mut p = clean();
    assert!(!has(&p, SlopCheck::StockPhrase));
    set_text(&mut p, "b0.head", "Unlock the Power of savings");
    assert!(has(&p, SlopCheck::StockPhrase));
}

#[test]
fn emoji_bullet() {
    let mut p = clean();
    assert!(!has(&p, SlopCheck::EmojiBullet));
    set_text(&mut p, "b0.sub", "line one\n\u{1F680} launch");
    assert!(has(&p, SlopCheck::EmojiBullet));
}

#[test]
fn ai_palette_purple_blue_pair() {
    let mut p = clean();
    assert!(!has(&p, SlopCheck::AiPalette));
    p.scenes[0]
        .layers
        .push(rect("b0.card", Color::rgb(0x7C, 0x3A, 0xED), 400.0, 200.0));
    assert!(!has(&p, SlopCheck::AiPalette));
    p.scenes[0]
        .layers
        .push(rect("b0.card2", Color::rgb(0x3B, 0x4F, 0xE0), 400.0, 200.0));
    assert!(has(&p, SlopCheck::AiPalette));
}

#[test]
fn ai_palette_neon_on_dark() {
    let mut p = clean();
    p.canvas.background = Color::rgb(0x0B, 0x0D, 0x12);
    assert!(!has(&p, SlopCheck::AiPalette));
    p.scenes[0].layers[0] = text_in(
        "b0.head",
        "Rates fell",
        FontRole::Display,
        Color::rgb(0x00, 0xF0, 0xFF),
    );
    assert!(has(&p, SlopCheck::AiPalette));
}

#[test]
fn ai_palette_pure_ground() {
    let mut p = clean();
    p.canvas.background = Color::rgb(0xFF, 0xFF, 0xFF);
    assert!(has(&p, SlopCheck::AiPalette));
    p.canvas.background = Color::rgb(0xF5, 0xF5, 0xF0);
    assert!(!has(&p, SlopCheck::AiPalette));
}

#[test]
fn uniform_entrance_same_frame() {
    let mut p = clean();
    for s in &mut p.scenes {
        s.layers.push(text("x.third", "three"));
    }
    p.scenes[0].layers[2].id = "b0.third".to_string();
    p.scenes[1].layers[2].id = "b1.third".to_string();
    // Staggered third layer: fine.
    p.scenes[0]
        .motions
        .push(rise("b0.third", 0.8, Easing::InCubic));
    assert!(!has(&p, SlopCheck::UniformEntrance));
    // Three layers entering together.
    p.scenes[1].motions = vec![
        fade("b1.stat", 0.2, Easing::OutCubic),
        rise("b1.label", 0.2, Easing::EditorialSpring),
        rise("b1.third", 0.2, Easing::InCubic),
    ];
    assert!(has(&p, SlopCheck::UniformEntrance));
}

#[test]
fn uniform_entrance_one_curve() {
    let mut p = clean();
    for s in &mut p.scenes {
        s.layers.push(text("x.third", "three"));
    }
    p.scenes[0].layers[2].id = "b0.third".to_string();
    p.scenes[1].layers[2].id = "b1.third".to_string();
    p.scenes[0].motions = vec![
        fade("b0.head", 0.1, Easing::OutCubic),
        fade("b0.sub", 0.4, Easing::OutCubic),
        fade("b0.third", 0.7, Easing::OutCubic),
    ];
    assert!(has(&p, SlopCheck::UniformEntrance));
    p.scenes[0].motions[2] = rise("b0.third", 0.7, Easing::OutCubic);
    assert!(!has(&p, SlopCheck::UniformEntrance));
}

#[test]
fn no_hold() {
    let mut p = clean();
    assert!(!has(&p, SlopCheck::NoHold));
    // A slow entrance that is fully in only 0.1 s before the beat ends.
    p.scenes[0].motions[1].start = 0.8;
    p.scenes[0].motions[1].duration = 3.1;
    assert!(has(&p, SlopCheck::NoHold));
}

#[test]
fn fx_stack() {
    let mut p = clean();
    let fx = |kind| PostEffect {
        start: 0.0,
        duration: 0.0,
        easing: Easing::Linear,
        from: 0.0,
        to: 0.5,
        kind,
    };
    p.scenes[0].post = vec![
        fx(PostKind::Grain {
            amount: 0.3,
            seed: 1,
        }),
        fx(PostKind::Vignette { amount: 0.3 }),
        fx(PostKind::Bloom {
            threshold: 0.8,
            radius: 8.0,
        }),
    ];
    assert!(!has(&p, SlopCheck::FxStack));
    p.scenes[0].post.push(fx(PostKind::ChromaticAberration {
        shift: 2.0,
        angle_deg: 0.0,
    }));
    assert!(has(&p, SlopCheck::FxStack));
}

#[test]
fn repeated_grammar() {
    let mut p = clean();
    assert!(!has(&p, SlopCheck::RepeatedGrammar));
    // Same layer names and kinds in consecutive beats.
    p.scenes[1].layers = vec![text("b1.head", "Other"), text("b1.sub", "words")];
    p.scenes[1].motions = vec![
        fade("b1.head", 0.1, Easing::OutCubic),
        rise("b1.sub", 0.5, Easing::EditorialSpring),
    ];
    assert!(has(&p, SlopCheck::RepeatedGrammar));
}

#[test]
fn showy_transitions() {
    let mut p = clean();
    assert!(!has(&p, SlopCheck::ShowyTransitions));
    for s in &mut p.scenes {
        let id = format!("{}.wipe", s.id);
        s.layers
            .push(rect(&id, Color::rgb(0xC0, 0x39, 0x2B), 1080.0, 1920.0));
    }
    assert!(has(&p, SlopCheck::ShowyTransitions));
}

#[test]
fn family_sprawl() {
    let mut p = clean();
    assert!(!has(&p, SlopCheck::FamilySprawl));
    let ink = Color::rgb(0x22, 0x22, 0x22);
    p.scenes[0]
        .layers
        .push(text_in("b0.a", "a", FontRole::SerifEmotional, ink));
    p.scenes[0]
        .layers
        .push(text_in("b0.b", "b", FontRole::Mono, ink));
    p.scenes[0]
        .layers
        .push(text_in("b0.c", "c", FontRole::Number, ink));
    // Same family regardless of role when the assets share a stem.
    assert!(has(&p, SlopCheck::FamilySprawl));
}

#[test]
fn unmeasurable_with_one_beat() {
    let mut p = clean();
    p.scenes.truncate(1);
    let r = slop_report(&p, &rules());
    assert_eq!(r.not_measurable.len(), 2, "{:?}", r.not_measurable);
}

#[test]
fn critique_axes_and_audio_na() {
    let c = critique(&clean(), &rules());
    assert_eq!(c.axes.len(), 8);
    assert_eq!(c.axes[7].score, None);
    assert_eq!(c.total, 40.0, "{}", c.to_text());
    assert!(c.to_text().lines().count() <= 14);
    let mut p = clean();
    set_text(&mut p, "b0.head", "A game-changer");
    let c = critique(&p, &rules());
    assert!(c.total < 40.0);
}

#[test]
fn critique_of_editorial_demo_is_deterministic() {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let intent = CreativeIntent::from_json(
        &std::fs::read_to_string(root.join("examples/editorial_demo.intent.json")).expect("intent"),
    )
    .expect("intent parses");
    let style = StyleProfile::from_json(
        &std::fs::read_to_string(root.join("examples/editorial_demo.style.json")).expect("style"),
    )
    .expect("style parses");
    let run = || {
        let project = compile(
            &intent,
            &style,
            &AssetLibrary::new(root.join("assets")),
            &ApproxMeasure,
        )
        .expect("compiles");
        serde_json::to_string(&critique(&project, &rules())).expect("json")
    };
    assert_eq!(run(), run());
}

/// A layer moved to `(y, h)` with font size `size` when it is text.
fn placed(mut l: Layer, y: f32, h: f32, size: f32) -> Layer {
    l.y = y;
    l.height = h;
    if let LayerKind::Text(t) = &mut l.kind {
        t.font_size = size;
    }
    l
}

#[test]
fn eyebrow_label_positive() {
    // "SALARY" in small mono caps at the top-left with a 7 u rule under it.
    let mut p = clean();
    p.scenes[0]
        .layers
        .push(placed(text("b0.kicker", "SALARY"), 201.0, 36.0, 30.0));
    p.scenes[0].layers.push(placed(
        rect("b0.kicker_rule", Color::rgb(0xCC, 0x99, 0), 96.0, 7.0),
        251.0,
        7.0,
        0.0,
    ));
    assert!(has(&p, SlopCheck::EyebrowLabel));
}

#[test]
fn eyebrow_label_negative() {
    // The label alone is fine.
    let mut p = clean();
    p.scenes[0]
        .layers
        .push(placed(text("b0.kicker", "SALARY"), 201.0, 36.0, 30.0));
    assert!(!has(&p, SlopCheck::EyebrowLabel));
    // A label far from the rule, a long line, a headline-size text and a
    // label below the top band with a rule are not eyebrows.
    let mut p = clean();
    p.scenes[0]
        .layers
        .push(placed(text("b0.kicker", "SALARY"), 201.0, 36.0, 30.0));
    p.scenes[0].layers.push(placed(
        rect("b0.r", Color::rgb(0xCC, 0x99, 0), 96.0, 7.0),
        900.0,
        7.0,
        0.0,
    ));
    p.scenes[0].layers.push(placed(
        text("b0.long", "a long line of five words"),
        201.0,
        36.0,
        30.0,
    ));
    p.scenes[0]
        .layers
        .push(placed(text("b0.big", "FLAT"), 100.0, 130.0, 130.0));
    p.scenes[0]
        .layers
        .push(placed(text("b0.low", "SALARY"), 1200.0, 36.0, 30.0));
    p.scenes[0].layers.push(placed(
        rect("b0.r2", Color::rgb(0xCC, 0x99, 0), 96.0, 7.0),
        1250.0,
        7.0,
        0.0,
    ));
    assert!(!has(&p, SlopCheck::EyebrowLabel));
}

// --- (0.24 M9) slop_accent_overuse -------------------------------------------

const GOLD: Color = Color::rgb(0xF4, 0xB9, 0x3F);

/// `n` beats of one headline each, with the accent in the palette.
fn beats_with(n: usize, extra: impl Fn(usize) -> Vec<Layer>) -> MotionProject {
    let scenes = (0..n)
        .map(|i| {
            let mut layers = vec![text(&format!("b{i}.head"), "Word")];
            layers.extend(extra(i));
            scene(&format!("b{i}"), layers, vec![])
        })
        .collect();
    let mut p = project(scenes);
    p.theme.palette.insert("accent".to_string(), GOLD);
    p.theme
        .palette
        .insert("field_1".to_string(), Color::rgb(0x0D, 0xAC, 0x83));
    p
}

fn field(id: &str, fill: Color) -> Layer {
    let mut l = rect(id, fill, 1080.0, 1920.0);
    l.x = 0.0;
    l.y = 0.0;
    l
}

/// A gold box right behind the beat's headline.
fn block(id: &str) -> Layer {
    let mut l = rect(id, GOLD, 700.0, 140.0);
    l.x = 80.0;
    l.y = 380.0;
    l.z_index = -1;
    l
}

fn accent_findings(p: &MotionProject) -> Vec<String> {
    slop_report(p, &rules())
        .findings
        .into_iter()
        .filter(|f| f.check == SlopCheck::AccentOveruse)
        .map(|f| f.detail)
        .collect()
}

#[test]
fn accent_overuse_counts_colour_fields() {
    let one = beats_with(6, |i| {
        if i == 5 {
            vec![field(&format!("b{i}.field"), GOLD)]
        } else {
            vec![]
        }
    });
    assert!(accent_findings(&one).is_empty());
    let two = beats_with(6, |i| {
        if i >= 4 {
            vec![field(&format!("b{i}.field"), GOLD)]
        } else {
            vec![]
        }
    });
    let found = accent_findings(&two);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("full-frame colour fields"), "{found:?}");
}

#[test]
fn a_neutral_full_frame_surface_is_not_a_colour_field() {
    // The canvas ground and card stock are not colour moves; a wipe's second
    // bar belongs to its first.
    let p = beats_with(3, |i| {
        vec![
            field(&format!("b{i}.ground"), Color::rgb(0xF2, 0xED, 0xE4)),
            field(&format!("b{i}.wipe_a"), GOLD),
            field(&format!("b{i}.wipe_b"), Color::rgb(0x0D, 0xAC, 0x83)),
        ]
    });
    // Three wipes (one per beat) are three fields, not six.
    let found = accent_findings(&p);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].starts_with("3 full-frame colour fields"),
        "{found:?}"
    );
}

#[test]
fn accent_overuse_allows_one_block_per_three_beats() {
    let ok = beats_with(9, |i| {
        if i % 3 == 0 {
            vec![block(&format!("b{i}.band"))]
        } else {
            vec![]
        }
    });
    assert!(accent_findings(&ok).is_empty());
    let busy = beats_with(9, |i| {
        if i % 2 == 0 {
            vec![block(&format!("b{i}.band"))]
        } else {
            vec![]
        }
    });
    let found = accent_findings(&busy);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("accent blocks behind words"), "{found:?}");
    // A gold rectangle with no words on it is a rule or a swatch, not a label.
    let bare = beats_with(9, |i| {
        let mut l = block(&format!("b{i}.swatch"));
        l.y = 1500.0;
        vec![l]
    });
    assert!(accent_findings(&bare).is_empty());
}

#[test]
fn accent_overuse_counts_stamps() {
    let stamp = |i: usize| {
        let mut g = base_layer(
            &format!("b{i}.stamp"),
            LayerKind::Group { children: vec![] },
        );
        g.width = 200.0;
        g.height = 100.0;
        vec![g]
    };
    let ok = beats_with(6, |i| if i % 3 == 0 { stamp(i) } else { vec![] });
    assert!(accent_findings(&ok).is_empty());
    let busy = beats_with(6, stamp);
    let found = accent_findings(&busy);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("stamps"), "{found:?}");
}

// --- (0.24 M9) slop_skeleton_lines -------------------------------------------

fn bar(id: &str, y: f32, w: f32) -> Layer {
    let mut l = rect(id, Color::rgb(0x20, 0x20, 0x20), w, 26.0);
    l.x = 100.0;
    l.y = y;
    l
}

fn skeleton(p: &MotionProject) -> bool {
    slop_report(p, &rules())
        .findings
        .iter()
        .any(|f| f.check == SlopCheck::SkeletonLines)
}

#[test]
fn bars_stacked_like_text_lines_with_no_text_are_flagged() {
    let mut p = clean();
    for (i, w) in [500.0, 420.0, 300.0].into_iter().enumerate() {
        p.scenes[0]
            .layers
            .push(bar(&format!("b0.bar.{i}"), 900.0 + 45.0 * i as f32, w));
    }
    assert!(skeleton(&p));
}

#[test]
fn two_bars_a_hairline_or_bars_over_text_are_not_skeletons() {
    // Two bars: a pair of rules, not a paragraph.
    let mut p = clean();
    for i in 0..2 {
        p.scenes[0]
            .layers
            .push(bar(&format!("b0.bar.{i}"), 900.0 + 45.0 * i as f32, 400.0));
    }
    assert!(!skeleton(&p));
    // Hairlines (2 px) are rules.
    let mut p = clean();
    for i in 0..4 {
        let mut l = bar(&format!("b0.rule.{i}"), 900.0 + 45.0 * i as f32, 400.0);
        l.height = 2.0;
        p.scenes[0].layers.push(l);
    }
    assert!(!skeleton(&p));
    // Bars with a text layer on them are chart rows or highlights.
    let mut p = clean();
    for i in 0..3 {
        p.scenes[0]
            .layers
            .push(bar(&format!("b0.bar.{i}"), 900.0 + 45.0 * i as f32, 400.0));
    }
    let mut label = text("b0.label", "rows");
    label.x = 120.0;
    label.y = 920.0;
    label.height = 60.0;
    p.scenes[0].layers.push(label);
    assert!(!skeleton(&p));
}

// --- (0.24 M9) the design read -----------------------------------------------

#[test]
fn critique_prints_the_design_read_and_notes_unsourced_figures() {
    let c = critique(&clean(), &rules());
    assert!(c
        .reading
        .starts_with("Reading this as: auto for vertical 1080x1920, dial ENERGY"));
    assert!(c.to_text().lines().count() <= 14);
    // "42" is a figure and nothing sources it: a note, never a finding.
    assert!(c
        .notes
        .iter()
        .any(|n| n.starts_with("note_unsourced_numbers")));
    assert_eq!(c.total, 40.0);
    let mut p = clean();
    set_text(&mut p, "b1.label", "SmartAsset, 2024");
    assert!(critique(&p, &rules()).notes.is_empty());
}

// (0.24 V1) slop_boxy.

#[test]
fn two_cards_behind_text_or_two_decorative_rules_are_boxy() {
    let card = |id: &str, x: f32| {
        let mut l = rect(id, Color::rgb(0x30, 0x30, 0x34), 300.0, 200.0);
        l.x = x;
        l.y = 380.0;
        l
    };
    // The headline (100..700 x 400..520) sits on both cards.
    let mut p = clean();
    p.scenes[0].layers.push(card("b0.hero.card", 100.0));
    assert!(!has(&p, SlopCheck::Boxy), "one card is allowed");
    p.scenes[0].layers.push(card("b0.tag.card", 400.0));
    assert!(has(&p, SlopCheck::Boxy));

    let mut p = clean();
    p.scenes[0].layers.push(rect(
        "b0.folio_rule",
        Color::rgb(0x80, 0x80, 0x80),
        800.0,
        2.0,
    ));
    assert!(!has(&p, SlopCheck::Boxy));
    p.scenes[0].layers.push(rect(
        "b0.hero_rule",
        Color::rgb(0x80, 0x80, 0x80),
        400.0,
        3.0,
    ));
    assert!(has(&p, SlopCheck::Boxy));
    // A hidden anchor is not drawn: it does not count.
    for l in p.scenes[0]
        .layers
        .iter_mut()
        .filter(|l| l.id.ends_with("_rule"))
    {
        l.visible = false;
    }
    assert!(!has(&p, SlopCheck::Boxy));
}
