use motion_bridge::*;

const BRIEF: &str = "Coffee prices, 60 s
Coffee costs more every year.
Arabica rose 38 52 61 74 90 cents a pound.
Farmers see little of it.
Roasters and shops keep 85% of the price.
Drink slower.";

const PROBE: &str =
    "Make a dynamic 15 s showreel that proves you are a great motion designer. Go all out.";

fn clock() -> Clock {
    Clock::steady(126.0, 400)
}

#[test]
fn direction_never_reaches_the_screen() {
    let c = parse_brief(PROBE);
    assert!(c.sections.is_empty() && c.title.is_none(), "{c:?}");
    assert_eq!(c.duration, Some(15.0));
    let (c, plan, p) = make_from_brief(PROBE, None, None, None, &clock(), 0).unwrap();
    assert!(prompt_echo(&c, &p).is_empty(), "{:?}", prompt_echo(&c, &p));
    let kinds: std::collections::BTreeSet<&str> =
        p.components.iter().map(|c| c.body.kind_name()).collect();
    assert!(kinds.len() >= 5, "{kinds:?}");
    assert!(
        (14.0..=17.0).contains(&p.meta.duration),
        "{}",
        p.meta.duration
    );
    assert_eq!(plan.final_downbeat() % 4, 0);
}

#[test]
fn headlines_are_claims_not_brief_words() {
    for (name, _) in RECIPES {
        let (c, _, p) = make_from_brief(BRIEF, Some(name), None, None, &clock(), 0).unwrap();
        assert!(
            prompt_echo(&c, &p).is_empty(),
            "{name}: {:?}",
            prompt_echo(&c, &p)
        );
    }
    assert_eq!(claim(&[38.0, 52.0, 61.0, 74.0, 90.0], &[]), "UP 2.4×");
    assert_eq!(claim(&[0.0, 85.0], &["85%".into()]), "NEARLY 9 IN 10");
    assert_eq!(claim(&[0.0, 7.0], &["7%".into()]), "ONLY 7%");
}

#[test]
fn quoted_words_are_content() {
    let c = parse_brief("Make a 15 s reel that says \"Slow coffee wins\"");
    assert_eq!(c.sections.len(), 1);
    assert!(!c.direction_words.contains(&"coffee".to_string()));
}

#[test]
fn every_recipe_builds_on_the_grid() {
    for (name, _) in RECIPES {
        let (_, plan, p) = make_from_brief(BRIEF, Some(name), None, None, &clock(), 0).expect(name);
        // Sparse content makes a shorter video than asked, never padding.
        assert!(
            (10.0..=62.0).contains(&p.meta.duration),
            "{name} {}",
            p.meta.duration
        );
        assert_eq!(plan.final_downbeat() % 4, 0);
        assert_eq!(p.meta.name, "coffee_prices");
    }
}

#[test]
fn deterministic() {
    let a = make_from_brief(BRIEF, None, None, None, &clock(), 0)
        .unwrap()
        .2
        .to_json();
    let b = make_from_brief(BRIEF, None, None, None, &clock(), 0)
        .unwrap()
        .2
        .to_json();
    assert_eq!(a, b);
}

#[test]
fn intent_example_bridges() {
    let text = std::fs::read_to_string("../../examples/editorial_demo.intent.json").unwrap();
    let i = motion_core::CreativeIntent::from_json(&text).unwrap();
    for (name, _) in RECIPES {
        make_from_intent(&i, Some(name), None, Some(30.0), &clock(), 0).expect(name);
    }
}

#[test]
fn unknown_recipe_is_named() {
    assert!(recipe("nope").unwrap_err().contains("showreel"));
}

#[test]
fn content_seconds_are_data_not_length() {
    let c = parse_brief("Wallet app, 15 s\nPayments settle in 2 seconds");
    assert_eq!(c.duration, Some(15.0));
    assert!(matches!(c.sections.as_slice(), [Section::Stat { .. }]));
}
