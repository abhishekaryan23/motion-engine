//! Unit tests of the caption builder (every treatment, lanes, budgets).

use super::*;
use crate::scene::{FontRole, MotionOp};

fn width(v: &Voice, t: &str) -> f32 {
    // ApproxMeasure-like: 0.5 em per char, a little wider for mono.
    let em = if v.role == FontRole::Mono { 0.61 } else { 0.5 };
    t.chars().count() as f32 * em
}

fn sentence(text: &str, primary: Option<&str>, t0: f64, step: f64) -> SentenceIn {
    let words: Vec<WordIn> = text
        .split_whitespace()
        .enumerate()
        .map(|(i, w)| WordIn {
            index: i,
            text: w.trim_matches(|c: char| c == '"' || c == '.').to_string(),
            start: t0 + step * i as f64,
        })
        .collect();
    let last = words.last().map(|w| w.start).unwrap_or(t0);
    SentenceIn {
        words,
        statement: text.to_string(),
        primary_value: primary.map(str::to_string),
        end: last + 0.35,
        lane: Lane::Lower,
        backing: false,
    }
}

fn build(
    treatment: CaptionTreatment,
    frame: &LayoutFrame,
    s: &SentenceIn,
) -> (Vec<Layer>, Vec<Motion>) {
    let pal = Palette::default();
    let w = |v: &Voice, t: &str| width(v, t);
    let ink = |_: &Block| None;
    let env = CaptionEnv {
        frame,
        palette: &pal,
        treatment,
        width: &w,
        ink: &ink,
    };
    build_layers(&env, std::slice::from_ref(s), s.end + 1.0)
}

fn collect<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            collect(children, out);
        }
    }
}

fn flat(layers: &[Layer]) -> Vec<&Layer> {
    let mut v = Vec::new();
    collect(layers, &mut v);
    v
}

fn text_of(l: &Layer) -> Option<&crate::scene::TextStyle> {
    match &l.kind {
        LayerKind::Text(t) => Some(t),
        _ => None,
    }
}

const LINE: &str = "Thirty percent of 30% leaks from old pipes every single year and nobody ever notices it at all";

#[test]
fn treatment_table_unchanged() {
    assert_eq!(
        caption_treatment(Emotion::Calm),
        CaptionTreatment::SerifItalic
    );
    assert_eq!(
        caption_treatment(Emotion::Urgency),
        CaptionTreatment::LabelPill
    );
    assert_eq!(caption_treatment(Emotion::Joy), CaptionTreatment::Highlight);
    assert_eq!(
        caption_treatment(Emotion::Luxury),
        CaptionTreatment::SingleWord
    );
    assert_eq!(
        caption_treatment(Emotion::Handmade),
        CaptionTreatment::Handwritten
    );
}

#[test]
fn emphasis_numbers_primary_and_quotes() {
    let s = sentence(
        r#"Save ₹42,000 in 3 months for "tea" and lost water"#,
        Some("lost water"),
        0.0,
        0.3,
    );
    let flags = emphasis_flags(&s);
    let on: Vec<&str> = s
        .words
        .iter()
        .zip(&flags)
        .filter(|(_, f)| **f)
        .map(|(w, _)| w.text.as_str())
        .collect();
    // Budget of MAX_EMPHASIS: numbers win over quotes and the primary value.
    assert_eq!(on, ["₹42,000", "3"]);
}

fn emphasised(statement: &str, primary: Option<&str>) -> Vec<String> {
    let s = sentence(statement, primary, 0.0, 0.3);
    let flags = emphasis_flags(&s);
    s.words
        .iter()
        .zip(&flags)
        .filter(|(_, f)| **f)
        .map(|(w, _)| w.text.clone())
        .collect()
}

#[test]
fn emphasis_is_a_spotlight_not_a_highlighter() {
    // Owner bug: every word of "the water we keep" got a pill, twice.
    assert_eq!(
        emphasised(
            "The cheapest water is the water we keep",
            Some("the water we keep")
        ),
        ["water", "keep"]
    );
    // Quotes beat the primary value; function words never qualify.
    assert_eq!(
        emphasised(r#"They call it "quiet" saving"#, Some("the saving")),
        ["quiet", "saving"]
    );
    // Nothing to stress → nothing emphasised.
    assert!(emphasised("It is what it is", None).is_empty());
}

#[test]
fn pages_respect_line_and_char_budgets_and_timing() {
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let s = sentence(LINE, None, 1.0, 0.25);
    let (layers, motions) = build(CaptionTreatment::SerifItalic, &frame, &s);
    assert!(layers.len() >= 2, "{} page(s)", layers.len());
    let mut prev_end = 0.0;
    for page in &layers {
        let LayerKind::Group { children } = &page.kind else {
            panic!("page is a group")
        };
        let mut ys: Vec<i32> = children
            .iter()
            .filter(|c| text_of(c).is_some())
            .map(|c| (c.y + c.height / 2.0).round() as i32)
            .collect();
        ys.sort_unstable();
        ys.dedup();
        assert!(ys.len() <= MAX_LINES);
        for y in &ys {
            let n: usize = children
                .iter()
                .filter_map(|c| text_of(c).map(|t| (c, t)))
                .filter(|(c, _)| (c.y + c.height / 2.0).round() as i32 == *y)
                .map(|(_, t)| t.text.chars().count() + 1)
                .sum::<usize>()
                - 1;
            assert!(n <= MAX_CHARS_PER_LINE, "{n} chars");
        }
        // The page leaves no later than the next page's first word starts.
        let exit = motions
            .iter()
            .find(|m| m.target == page.id)
            .expect("exit fade");
        assert!(exit.duration <= EXIT_SECONDS + 1e-9);
        assert!(exit.start + exit.duration >= prev_end - 1e-9);
        prev_end = exit.start + exit.duration;
    }
    // Every word fades in exactly at its start, quickly.
    for (i, w) in s.words.iter().enumerate() {
        let m = motions
            .iter()
            .find(|m| m.target == word_layer_id(i) && matches!(m.op, MotionOp::Fade { .. }))
            .expect("word fade");
        assert!((m.start - w.start).abs() < 1e-3);
        assert!(m.duration <= APPEAR_SECONDS + 1e-9);
    }
}

#[test]
fn serif_italic_sets_emphasis_in_the_voice_face() {
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let s = sentence("Thirty percent leaks", Some("percent"), 0.0, 0.3);
    let (layers, _) = build(CaptionTreatment::SerifItalic, &frame, &s);
    let f = flat(&layers);
    let word = |i: usize| text_of(f.iter().find(|l| l.id == word_layer_id(i)).unwrap()).unwrap();
    assert_eq!(word(0).font_role, FontRole::Body);
    assert_eq!(word(1).font_role, FontRole::SerifEmotional);
    assert!(word(1).italic);
    assert!(f.iter().all(|l| !l.id.ends_with(".bg")));
    assert!(word(0).font_size >= frame.min_type_px * 1.9 - 0.01);
}

#[test]
fn label_pill_follows_the_spoken_word_with_contrasting_text() {
    let frame = LayoutFrame::new(1080, 1080).unwrap();
    let s = sentence("Costs 40% less today", None, 0.0, 0.3);
    let (layers, motions) = build(CaptionTreatment::LabelPill, &frame, &s);
    let f = flat(&layers);
    let pal = Palette::default();
    // Every word owns an active pill; its overlay text reads on the accent.
    for i in 0..4 {
        let bg = f
            .iter()
            .find(|l| l.id == format!("cap.w{i}.bg"))
            .expect("pill");
        let LayerKind::RoundedRectangle { fill, .. } = bg.kind else {
            panic!("rounded pill")
        };
        assert_eq!(fill, pal.accent);
        let on = text_of(f.iter().find(|l| l.id == format!("cap.w{i}.on")).unwrap()).unwrap();
        assert!(contrast_ratio(on.color, fill) >= 4.5);
        assert!(on.font_size >= frame.min_type_px);
    }
    // The keyword "40%" stays distinct after its turn (accent text, no pill).
    let kw = text_of(f.iter().find(|l| l.id == "cap.w1").unwrap()).unwrap();
    assert!(kw.text.starts_with("40%"));
    assert_eq!(kw.font_role, FontRole::Mono);
    // Pill of word 1: on at its onset (0.3), off by the next onset (0.6).
    let fades: Vec<&Motion> = motions.iter().filter(|m| m.target == "cap.w1.bg").collect();
    assert!(fades.iter().any(|m| (m.start - 0.3).abs() < 1e-6));
    assert!(fades
        .iter()
        .any(|m| matches!(m.op, MotionOp::Fade { to, .. } if to == 0.0)
            && (m.start + m.duration - 0.6).abs() < 1e-3));
}

#[test]
fn at_most_one_word_is_active_at_any_time() {
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let s = sentence(
        "The cheapest water is the water we keep",
        Some("the water we keep"),
        0.0,
        0.25,
    );
    for tr in [CaptionTreatment::LabelPill, CaptionTreatment::Highlight] {
        let (_, motions) = build(tr, &frame, &s);
        // Active window of each pill = [fade-in start, fade-out end].
        let mut windows: Vec<(f64, f64)> = Vec::new();
        for i in 0..s.words.len() {
            let id = format!("cap.w{i}.bg");
            let on = motions
                .iter()
                .find(|m| m.target == id && matches!(m.op, MotionOp::Fade { to, .. } if to == 1.0))
                .unwrap();
            let off = motions
                .iter()
                .find(|m| m.target == id && matches!(m.op, MotionOp::Fade { to, .. } if to == 0.0))
                .unwrap();
            windows.push((on.start, off.start + off.duration));
        }
        for t in (0..40).map(|k| k as f64 * 0.05) {
            let active = windows
                .iter()
                .filter(|(a, b)| t > *a + 1e-6 && t < *b - 1e-6)
                .count();
            assert!(active <= 1, "{tr:?}: {active} words active at {t:.2}s");
        }
    }
}

#[test]
fn pill_text_contrast_holds_for_every_accent() {
    for accent in [
        Color::rgb(0xE0, 0x33, 0x1F),
        Color::rgb(0x23, 0x3F, 0xD1),
        Color::rgb(0xC9, 0xE4, 0x2B),
        Color::rgb(0x80, 0x80, 0x80),
    ] {
        let pal = Palette {
            accent,
            ..Palette::default()
        };
        assert!(contrast_ratio(legible_on(accent, &pal), accent) >= 4.5);
    }
}

#[test]
fn highlight_field_sits_behind_the_active_word() {
    let frame = LayoutFrame::new(1920, 1080).unwrap();
    let s = sentence("It grew 12 percent", None, 0.0, 0.3);
    let (layers, motions) = build(CaptionTreatment::Highlight, &frame, &s);
    let f = flat(&layers);
    let bg = f.iter().find(|l| l.id == "cap.w2.bg").expect("field");
    assert!(matches!(bg.kind, LayerKind::Rectangle { .. }));
    assert!(bg.z_index < f.iter().find(|l| l.id == "cap.w2").unwrap().z_index);
    assert!(motions
        .iter()
        .any(|m| m.target == "cap.w2.bg" && (m.start - 0.6).abs() < 1e-6));
}

#[test]
fn single_word_shows_one_large_word_per_page() {
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let s = sentence("Less is always more", None, 0.0, 0.4);
    let (layers, motions) = build(CaptionTreatment::SingleWord, &frame, &s);
    assert_eq!(layers.len(), 4);
    for page in &layers {
        let LayerKind::Group { children } = &page.kind else {
            panic!("group")
        };
        let texts: Vec<_> = children.iter().filter_map(text_of).collect();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].font_size >= caption_size(&frame) * SINGLE_WORD_SCALE - 0.5);
    }
    for (i, w) in s.words.iter().enumerate() {
        let m = motions
            .iter()
            .find(|m| m.target == word_layer_id(i) && matches!(m.op, MotionOp::Fade { .. }))
            .unwrap();
        assert!((m.start - w.start).abs() < 1e-3);
    }
}

#[test]
fn handwritten_uses_the_voice_role_larger_than_body() {
    let frame = LayoutFrame::new(1080, 1350).unwrap();
    let s = sentence("Made by hand 100 times", None, 0.0, 0.3);
    let (layers, _) = build(CaptionTreatment::Handwritten, &frame, &s);
    let f = flat(&layers);
    let body = text_of(f.iter().find(|l| l.id == "cap.w0").unwrap()).unwrap();
    let hand = text_of(f.iter().find(|l| l.id == "cap.w3").unwrap()).unwrap();
    assert_eq!(hand.font_role, FontRole::SerifEmotional);
    assert!(hand.font_size > body.font_size);
}

#[test]
fn upper_lane_and_backing() {
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let mut s = sentence("Look up here", None, 0.0, 0.3);
    s.lane = Lane::Upper;
    s.backing = true;
    let (layers, motions) = build(CaptionTreatment::SerifItalic, &frame, &s);
    let f = flat(&layers);
    let band = f.iter().find(|l| l.id == "cap.p0.bg").expect("backing");
    let LayerKind::RoundedRectangle { fill, .. } = band.kind else {
        panic!("band")
    };
    assert!(fill.a <= 217, "alpha {} must be <= 0.85", fill.a);
    let w0 = f.iter().find(|l| l.id == "cap.w0").unwrap();
    assert!(w0.y < frame.h / 3.0, "upper lane at y {}", w0.y);
    assert!(motions.iter().any(|m| m.target == "cap.p0.bg"));
}

#[test]
fn captions_stay_inside_the_safe_area_on_every_aspect() {
    for (w, h) in [
        (1080, 1920),
        (1080, 1080),
        (1920, 1080),
        (1080, 1350),
        (1080, 2520),
    ] {
        let frame = LayoutFrame::new(w, h).unwrap();
        let s = sentence(LINE, Some("pipes"), 0.0, 0.25);
        for t in [
            CaptionTreatment::SerifItalic,
            CaptionTreatment::LabelPill,
            CaptionTreatment::Highlight,
            CaptionTreatment::SingleWord,
            CaptionTreatment::Handwritten,
        ] {
            let (layers, _) = build(t, &frame, &s);
            for l in flat(&layers) {
                if text_of(l).is_some() {
                    let r = crate::compiler::layout_frame::Rect {
                        x: l.x,
                        y: l.y,
                        w: l.width,
                        h: l.height,
                    };
                    let slack = 0.02 * l.width + 4.0 * frame.u;
                    let mut safe = frame.safe;
                    safe.x -= slack;
                    safe.y -= slack;
                    safe.w += 2.0 * slack;
                    safe.h += 2.0 * slack;
                    assert!(safe.contains(&r), "{t:?} {w}x{h} {} {r:?}", l.id);
                }
            }
        }
    }
}

#[test]
fn deterministic() {
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let s = sentence(LINE, Some("pipes"), 0.5, 0.25);
    let a = build(CaptionTreatment::Highlight, &frame, &s);
    let b = build(CaptionTreatment::Highlight, &frame, &s);
    assert_eq!(a.0, b.0);
    assert_eq!(a.1, b.1);
}

#[test]
fn captions_keep_the_statements_punctuation() {
    let mut s = sentence("Short, early, and on purpose.", None, 0.0, 0.3);
    // Spoken words arrive without punctuation.
    for w in &mut s.words {
        w.text = w.text.trim_end_matches(',').to_string();
    }
    let shown: Vec<String> = with_punctuation(&s).into_iter().map(|w| w.text).collect();
    assert_eq!(shown, ["Short,", "early,", "and", "on", "purpose"]);
    let two = sentence("Start small. Start this week.", None, 0.0, 0.3);
    let shown: Vec<String> = with_punctuation(&two).into_iter().map(|w| w.text).collect();
    assert_eq!(shown, ["Start", "small.", "Start", "this", "week"]);
}

#[test]
fn two_line_pages_do_not_orphan_a_word() {
    let s = sentence("Thirty percent leaks from old pipes", None, 0.0, 0.3);
    let toks: Vec<Tok> = s
        .words
        .iter()
        .map(|w| Tok {
            index: w.index,
            start: w.start,
            chars: w.text.chars().count(),
            text: w.text.clone(),
            voice: Voice::BODY,
            scale: 1.0,
            bg: Bg::None,
            color: Color::rgb(0, 0, 0),
            em_w: 0.5 * w.text.chars().count() as f32,
            pad_em: 0.0,
        })
        .collect();
    let first_len = toks[..5].iter().map(|t| t.chars).sum::<usize>();
    assert!(first_len > 0);
    // Greedy would put five words on line one and "pipes" alone on line two.
    let page = vec![toks[..5].to_vec(), toks[5..].to_vec()];
    let balanced = balance(page, 40.0, 820.0, 0.3);
    assert_eq!(balanced.len(), 2);
    assert!(
        balanced[1].len() >= 2,
        "orphan left: {:?}",
        balanced[1].len()
    );
}
