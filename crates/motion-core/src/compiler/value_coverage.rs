//! (0.23) `value_dropped`: every value, name and item the intent gives must be
//! readable on screen during its beat. Pure: it reads the intent and the
//! finished project, changes nothing, and its warnings are returned beside the
//! project (never written into it), so default compiles stay byte-identical.
//!
//! **What a beat must show** (from the intent):
//!
//! | role | what |
//! |---|---|
//! | `value` | a number subject's `value`; an object's `value` outside a comparison |
//! | `compare side` | each side of a `compare` / `contrast` beat: its `value` when it has one, else its phrase text / meaning |
//! | `name` | an object side of a comparison without a value: its `meaning` |
//! | `list item` | every collection item: its `value` when it has one, else its meaning |
//! | `result` | a derived metric's computed result (primary or secondary) |
//! | `from` / `to` | a state change's two states |
//!
//! Not required: the phrase primary of an emphasize / reveal / explain beat
//! (it is the headline, whatever the look does with it), the meaning of a lone
//! picture, the names of a layers subject (they live in the persistent column
//! of the backdrop scene).
//!
//! **Shown** means a text layer of the beat's scene (a carried shared element
//! counts, since it is on screen in that beat; never the caption scene, never
//! backdrop or decorative layers) is readable (compounded opacity >= 0.5, every
//! glyph of a cascade landed, its box on the canvas through the perspective) at
//! some frame sampled every 0.1 s from the beat's ENTER to its ANTICIPATE
//! boundary, and its text matches:
//!
//! * a value with digits matches on its number tokens: the digit sequence
//!   with separators and signs stripped (`$240,000` = `240000`), or the same
//!   quantity in the compact form the engine writes (`$240K`, `1.2M`), or said
//!   in words on screen (`FIFTY` for `50`: the studio look sets the spoken
//!   words as its type);
//! * a counting text matches on its final value (the `count` motion's target);
//! * words match case-insensitively on whole words, all content words of the
//!   item, in any of the beat's readable text layers.
//!
//! Decorative layers (`layout_qa::is_decorative`) never count, with one
//! exception: the street / studio punchword (`ghost_punch`) is listed there
//! because it sits behind the picture, but it is the look's own display of the
//! beat's figure, set large at full opacity, so it counts.
//!
//! The message is `"<value>" (<role>) is not shown`; the beat is the warning's
//! `beat` (the CLI prints it as `beat N:`).

use std::collections::BTreeMap;

use super::{derived, CompileWarning, WARN_VALUE_DROPPED};
use crate::intent::{Beat, CollectionItem, CreativeIntent, Purpose, Subject};
use crate::layout_qa::is_decorative;
use crate::scene::PostKind;
use crate::scene::{LayerKind, Lifecycle, MotionOp, MotionProject, Scene};
use crate::speech::{READABLE_BLUR_PX, READABLE_OPACITY};
use crate::timeline::{evaluate_frame, format_count, ResolvedFrame, ResolvedLayer};

/// A text is readable up to this blur radius, as a share of its font size
/// (the limit `reveal_qa` uses: 2 px at 1080, or 12 % of the type size).
const TEXT_BLUR_SHARE: f32 = 0.12;

/// Seconds between the sampled frames of a beat's window.
const SAMPLE_STEP: f64 = 0.1;

/// Words that carry no information of their own in an item's name.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "to", "in", "on", "at", "for", "and", "or", "is", "it",
];

// ---------------------------------------------------------------------------
// What a beat must show
// ---------------------------------------------------------------------------

/// One thing a beat must show.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Need {
    /// `value`, `compare side`, `list item`, `result`, `from`, `to`, `name`.
    role: &'static str,
    /// The text as the author (or the engine's arithmetic) gave it.
    text: String,
    /// What is matched on screen when that differs from `text` (a derived
    /// result: the number without its unit words).
    want: Option<String>,
}

impl Need {
    fn new(role: &'static str, text: &str) -> Need {
        Need {
            role,
            text: text.to_string(),
            want: None,
        }
    }
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// What a beat must show, in a stable order.
fn needs_of(beat: &Beat) -> Vec<Need> {
    let mut out = Vec::new();
    let comparing =
        matches!(beat.purpose, Purpose::Compare | Purpose::Contrast) && beat.secondary.is_some();
    for subject in std::iter::once(&beat.primary).chain(beat.secondary.as_ref()) {
        match subject {
            Subject::Number(a) => {
                if comparing {
                    if let Some(text) = nonempty(a.value.as_deref().or(a.meaning.as_deref())) {
                        out.push(Need::new("compare side", text));
                    }
                } else if let Some(text) = nonempty(a.value.as_deref()) {
                    out.push(Need::new("value", text));
                }
            }
            Subject::Object(o) => {
                if let Some(text) = nonempty(o.value.as_deref()) {
                    out.push(Need::new(
                        if comparing { "compare side" } else { "value" },
                        text,
                    ));
                } else if comparing {
                    if let Some(text) = nonempty(o.meaning.as_deref()) {
                        out.push(Need::new("name", text));
                    }
                }
            }
            Subject::Phrase(a) => {
                if comparing {
                    if let Some(text) = nonempty(a.value.as_deref().or(a.meaning.as_deref())) {
                        out.push(Need::new("compare side", text));
                    }
                }
            }
            Subject::Collection(c) => {
                for item in &c.items {
                    let (value, meaning) = match item {
                        CollectionItem::Phrase(a) | CollectionItem::Number(a) => {
                            (a.value.as_deref(), a.meaning.as_deref())
                        }
                        CollectionItem::Object(o) => (o.value.as_deref(), o.meaning.as_deref()),
                    };
                    if let Some(text) = nonempty(value).or_else(|| nonempty(meaning)) {
                        out.push(Need::new("list item", text));
                    }
                }
            }
            Subject::StateChange(s) => {
                out.push(Need::new("from", s.from.trim()));
                out.push(Need::new("to", s.to.trim()));
            }
            Subject::DerivedMetric(m) => {
                let value = derived::compute(m);
                if value.is_finite() {
                    let shown = derived::format_metric(value, m.format);
                    // The number is matched; its unit words ("per 1,000") may
                    // sit in another layer.
                    let number =
                        format_count(shown.shown, shown.decimals, shown.grouping(), "", "");
                    out.push(Need {
                        role: "result",
                        text: shown.text,
                        want: Some(number),
                    });
                }
            }
            Subject::Layers(_) => {}
        }
    }
    out.retain(|n| !n.text.is_empty());
    out
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Canonical strings one written number may stand for: itself, and (when a
/// compact suffix follows: `K`, `M`, `B`, `T`, thousand, million, billion) the
/// scaled quantity. See [`canon`].
type Forms = Vec<String>;

/// A number as a canonical string: integer digits without leading zeros, then
/// `.` and the fraction digits without trailing zeros (none when it is whole):
/// `"4.1"`, `"0.14"`, `"240000"`. The decimal point keeps its place, so 4.1 is
/// not 41, 0.5 is not 5 and $0.14 is not 14.
fn canon(int: &str, frac: &str) -> String {
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        int.to_string()
    } else {
        format!("{int}.{frac}")
    }
}

/// `int.frac` times 10^`power`, canonical (`1.2` and 6 -> `1200000`).
fn scaled(int: &str, frac: &str, power: usize) -> String {
    let take = power.min(frac.len());
    let (moved, rest) = frac.split_at(take);
    canon(&format!("{int}{moved}{}", "0".repeat(power - take)), rest)
}

/// The power of ten a compact suffix at `rest` stands for, if any.
fn suffix_power(rest: &[char]) -> Option<usize> {
    let mut i = 0;
    if rest.get(i) == Some(&' ') {
        i += 1;
    }
    let tail: String = rest[i..]
        .iter()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .to_lowercase();
    match tail.as_str() {
        "k" | "thousand" => Some(3),
        "m" | "mn" | "million" => Some(6),
        "b" | "bn" | "billion" => Some(9),
        "t" | "trillion" => Some(12),
        _ => None,
    }
}

/// The value of a number word (`seven`, `twenty`, `hundred`...), as
/// `(value, kind)`.
fn number_word(w: &str) -> Option<(u64, WordKind)> {
    const UNITS: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    if let Some(i) = UNITS.iter().position(|u| *u == w) {
        return Some((i as u64, WordKind::Unit));
    }
    if let Some(i) = TENS.iter().position(|t| *t == w) {
        return Some((20 + 10 * i as u64, WordKind::Tens));
    }
    match w {
        "hundred" => Some((100, WordKind::Hundred)),
        "thousand" => Some((1_000, WordKind::Scale)),
        "million" => Some((1_000_000, WordKind::Scale)),
        "billion" => Some((1_000_000_000, WordKind::Scale)),
        "trillion" => Some((1_000_000_000_000, WordKind::Scale)),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WordKind {
    Unit,
    Tens,
    Hundred,
    Scale,
}

/// The numbers a text says in words ("fifty", "twenty-five", "three hundred
/// and ten", "four point one", "two million"), as digit strings.
fn spoken_numbers(text: &str) -> Vec<Forms> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if number_word(&words[i]).is_none() {
            i += 1;
            continue;
        }
        let (mut total, mut cur) = (0u64, 0u64);
        while i < words.len() {
            match number_word(&words[i]) {
                Some((v, WordKind::Unit | WordKind::Tens)) => cur += v,
                Some((_, WordKind::Hundred)) => cur = cur.max(1) * 100,
                Some((v, WordKind::Scale)) => {
                    total = total.saturating_add(cur.max(1).saturating_mul(v));
                    cur = 0;
                }
                None if words[i] == "and"
                    && words.get(i + 1).is_some_and(|n| number_word(n).is_some()) => {}
                None => break,
            }
            i += 1;
        }
        let whole = total.saturating_add(cur).to_string();
        // "four point one": the digits after the point.
        let mut frac = String::new();
        if words.get(i).map(String::as_str) == Some("point") {
            let mut j = i + 1;
            while let Some((d, WordKind::Unit)) = words.get(j).and_then(|w| number_word(w)) {
                if d > 9 {
                    break;
                }
                frac.push_str(&d.to_string());
                j += 1;
            }
            if j > i + 1 {
                i = j;
            }
        }
        out.push(vec![canon(&whole, &frac)]);
    }
    out
}

/// The number tokens of a text, each with the digit strings it may stand for:
/// the numbers written in digits and those said in words.
fn number_tokens(text: &str) -> Vec<Forms> {
    let mut out = digit_tokens(text);
    out.extend(spoken_numbers(text));
    out
}

/// A text is numeric when it has digits, or when number words make up at
/// least half of its words ("three days": a figure; "No one wins": a phrase).
fn is_numeric(text: &str) -> bool {
    if text.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let numbers = words.iter().filter(|w| number_word(w).is_some()).count();
    numbers > 0 && 2 * numbers >= words.len()
}

/// The numbers a text writes in digits. `.` is the decimal point; `,` groups
/// digits when exactly three follow it (`240,000`), or two followed by another
/// group (the Indian lakh / crore grouping: `1,20,000` is 120000, `12,34,56,789`
/// is 123456789), and is a decimal comma otherwise (`3,5` is 3.5, `1,20` is 1.2).
fn digit_tokens(text: &str) -> Vec<Forms> {
    let chars: Vec<char> = text.chars().collect();
    let digits_from = |at: usize| {
        chars[at..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let (mut int, mut frac, mut in_frac) = (String::new(), String::new(), false);
        while i < chars.len() {
            let c = chars[i];
            let next_is_digit = chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if c.is_ascii_digit() {
                if in_frac {
                    frac.push(c);
                } else {
                    int.push(c);
                }
            } else if c == '.' && !in_frac && next_is_digit {
                in_frac = true;
            } else if c == ',' && !in_frac && next_is_digit {
                let run = digits_from(i + 1);
                // Two digits then another group: lakh / crore grouping.
                let indian = run == 2
                    && chars.get(i + 3) == Some(&',')
                    && chars.get(i + 4).is_some_and(char::is_ascii_digit);
                if run != 3 && !indian {
                    in_frac = true;
                }
            } else {
                break;
            }
            i += 1;
        }
        let mut forms = vec![canon(&int, &frac)];
        if let Some(power) = suffix_power(&chars[i..]) {
            // `1.2M`: the point moves right.
            forms.push(scaled(&int, &frac, power));
        }
        out.push(forms);
    }
    out
}

/// The content words of an item's text (lowercase, whole words).
fn content_words(text: &str) -> Vec<String> {
    let all: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let content: Vec<String> = all
        .iter()
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .cloned()
        .collect();
    if content.is_empty() {
        all
    } else {
        content
    }
}

/// How a need is matched.
enum Matcher {
    /// Every number token of the value, within one text layer.
    Numbers(Vec<Forms>),
    /// Every content word, within the beat's readable text.
    Words(Vec<String>),
}

fn matcher_for(need: &Need) -> Matcher {
    let text = need.want.as_deref().unwrap_or(&need.text);
    if text.chars().any(|c| c.is_ascii_digit()) {
        Matcher::Numbers(digit_tokens(text))
    } else if is_numeric(text) {
        Matcher::Numbers(spoken_numbers(text))
    } else {
        Matcher::Words(content_words(text))
    }
}

/// Whether `text` carries every required number token.
fn has_numbers(text: &str, want: &[Forms]) -> bool {
    let have = number_tokens(text);
    want.iter()
        .all(|w| have.iter().any(|h| h.iter().any(|f| w.contains(f))))
}

// ---------------------------------------------------------------------------
// Readability on the resolved frames
// ---------------------------------------------------------------------------

/// The text a layer finally shows: a counting layer's last value.
fn final_text(layer: &ResolvedLayer<'_>, counts: &BTreeMap<&str, String>) -> Option<String> {
    let LayerKind::Text(style) = layer.kind else {
        return None;
    };
    let text = counts
        .get(layer.id)
        .cloned()
        .or_else(|| layer.text.clone())
        .unwrap_or_else(|| style.text.clone());
    (!text.trim().is_empty()).then_some(text)
}

/// Final values of the scene's counting layers, by layer id.
fn count_finals(scene: &Scene) -> BTreeMap<&str, String> {
    let mut out = BTreeMap::new();
    for m in &scene.motions {
        if let MotionOp::Count {
            to,
            decimals,
            grouping,
            prefix,
            suffix,
            ..
        } = &m.op
        {
            out.insert(
                m.target.as_str(),
                format_count(*to, *decimals, *grouping, prefix, suffix),
            );
        }
    }
    out
}

/// Whether the layer's drawn box overlaps the canvas (through the
/// perspective homography when it has one).
fn on_canvas(layer: &ResolvedLayer<'_>, canvas: (f32, f32)) -> bool {
    let (w, h) = (layer.width, layer.height);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return false;
    }
    let c = layer.clip.unwrap_or_default();
    let rect = [
        c.left.max(0.0) * w,
        c.top.max(0.0) * h,
        (1.0 - c.right.max(0.0)) * w,
        (1.0 - c.bottom.max(0.0)) * h,
    ];
    if rect[0] >= rect[2] || rect[1] >= rect[3] {
        return false;
    }
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[2], rect[3]),
        (rect[0], rect[3]),
    ];
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let mut any = false;
    for (x, y) in corners {
        let mapped = match &layer.projective {
            Some(m) => {
                let d = m[6] * x + m[7] * y + m[8];
                if !(d.is_finite() && d > 1e-6) {
                    continue;
                }
                (
                    (m[0] * x + m[1] * y + m[2]) / d,
                    (m[3] * x + m[4] * y + m[5]) / d,
                )
            }
            None => layer.transform.apply(x, y),
        };
        if !(mapped.0.is_finite() && mapped.1.is_finite()) {
            continue;
        }
        any = true;
        b = [
            b[0].min(mapped.0),
            b[1].min(mapped.1),
            b[2].max(mapped.0),
            b[3].max(mapped.1),
        ];
    }
    any && b[0] < canvas.0 && b[2] > 0.0 && b[1] < canvas.1 && b[3] > 0.0
}

/// What the readable-text walk needs of one resolved frame.
struct FrameCtx<'a> {
    counts: &'a BTreeMap<&'a str, String>,
    canvas: (f32, f32),
    /// Blur radius (px) a text may carry at this canvas size.
    blur_px: f32,
}

/// The squared blur radius the frame's `directional_blur` post effects add to
/// every layer (a box of `length x strength` px, counted as the Gaussian
/// radius of the same spread, `length / sqrt(3)`).
fn post_blur_sq(frame: &ResolvedFrame<'_>) -> f32 {
    frame
        .post
        .iter()
        .map(|p| match p.kind {
            PostKind::DirectionalBlur { length, .. } => {
                (length * p.strength).max(0.0) / 3f32.sqrt()
            }
            _ => 0.0,
        })
        .map(|r| r * r)
        .sum()
}

/// The texts of the readable, non-decorative text layers of `scene` (and of
/// carried shared elements) at one resolved frame.
fn readable_texts(
    frame: &ResolvedFrame<'_>,
    scene: &str,
    ctx: &FrameCtx<'_>,
    out: &mut Vec<String>,
) {
    let post = post_blur_sq(frame);
    for layer in &frame.layers {
        if layer.scene.is_some_and(|s| s != scene) {
            continue;
        }
        walk(layer, 1.0, post, ctx, out);
    }
}

/// Furniture, atmosphere and backdrop layers never count as showing a value.
/// The street / studio punchword (`ghost_punch`) is listed as decorative for
/// layout QA (it sits behind the picture) but it is the look's own display of
/// the beat's figure at full opacity, so it counts.
fn is_furniture(layer: &ResolvedLayer<'_>) -> bool {
    is_decorative(layer.id, layer.kind) && !layer.id.ends_with(".ghost_punch")
}

/// Depth-first walk carrying the compounded opacity and the summed squared
/// blur radii of the ancestors (nested Gaussian radii add in quadrature).
fn walk(
    layer: &ResolvedLayer<'_>,
    opacity: f32,
    blur_sq: f32,
    ctx: &FrameCtx<'_>,
    out: &mut Vec<String>,
) {
    if is_furniture(layer) {
        return;
    }
    let opacity = opacity * layer.opacity.clamp(0.0, 1.0);
    if opacity < READABLE_OPACITY {
        return;
    }
    let own = layer.blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0);
    let blur_sq = blur_sq + own * own;
    if let LayerKind::Group { .. } = layer.kind {
        for child in &layer.children {
            walk(child, opacity, blur_sq, ctx, out);
        }
        return;
    }
    let LayerKind::Text(style) = layer.kind else {
        return;
    };
    let glyph = layer
        .glyphs
        .as_ref()
        .and_then(|g| g.iter().map(|p| p.opacity.clamp(0.0, 1.0)).reduce(f32::min))
        .unwrap_or(1.0);
    // A big headline stays legible under more blur than small type.
    let limit = ctx.blur_px.max(TEXT_BLUR_SHARE * style.font_size);
    if opacity * glyph < READABLE_OPACITY
        || blur_sq.sqrt() > limit + 1e-4
        || !on_canvas(layer, ctx.canvas)
    {
        return;
    }
    if let Some(text) = final_text(layer, ctx.counts) {
        out.push(text);
    }
}

/// The scene-local window `[ENTER, ANTICIPATE]` of a beat scene, as absolute
/// seconds.
fn window(scene: &Scene) -> (f64, f64) {
    match scene.lifecycle {
        Some(Lifecycle {
            enter, anticipate, ..
        }) => (
            scene.start_seconds + enter,
            scene.start_seconds + anticipate.max(enter),
        ),
        None => (scene.start_seconds, scene.end_seconds()),
    }
}

/// The needs of one beat that no readable text of its scene shows.
fn unmet(project: &MotionProject, scene: &Scene, needs: &[Need]) -> Vec<usize> {
    let fps = project.canvas.fps;
    if fps == 0 {
        return Vec::new();
    }
    let matchers: Vec<Matcher> = needs.iter().map(matcher_for).collect();
    let counts = count_finals(scene);
    let canvas = (project.canvas.width as f32, project.canvas.height as f32);
    let frame_ctx = FrameCtx {
        counts: &counts,
        canvas,
        blur_px: READABLE_BLUR_PX * canvas.0.min(canvas.1) / 1080.0,
    };
    let (t0, t1) = window(scene);
    let mut shown = vec![false; needs.len()];
    let mut words: Vec<String> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut k = 0u32;
    loop {
        let t = t0 + SAMPLE_STEP * f64::from(k);
        if t > t1 + 1e-9 {
            break;
        }
        k += 1;
        let frame = (t * f64::from(fps)).round() as u32;
        let Ok(resolved) = evaluate_frame(project, frame) else {
            // A frame the timeline cannot resolve is a validation problem,
            // not a dropped value.
            return Vec::new();
        };
        texts.clear();
        readable_texts(&resolved, &scene.id, &frame_ctx, &mut texts);
        for text in &texts {
            words.extend(content_words(text));
        }
        for (i, m) in matchers.iter().enumerate() {
            if shown[i] {
                continue;
            }
            shown[i] = match m {
                Matcher::Numbers(want) => texts.iter().any(|t| has_numbers(t, want)),
                Matcher::Words(want) => want.iter().all(|w| words.contains(w)),
            };
        }
        if shown.iter().all(|s| *s) {
            break;
        }
    }
    shown
        .iter()
        .enumerate()
        .filter_map(|(i, s)| (!s).then_some(i))
        .collect()
}

/// `value_dropped` warnings for every value, name or item the intent gives
/// that no readable text of its beat shows. `project` is the compiled project
/// (beat `i` is the scene `beat_{i+1}`; the caption scene is never searched).
pub fn check(intent: &CreativeIntent, project: &MotionProject) -> Vec<CompileWarning> {
    let mut out = Vec::new();
    for (i, beat) in intent.beats.iter().enumerate() {
        let needs = needs_of(beat);
        if needs.is_empty() {
            continue;
        }
        let id = format!("beat_{}", i + 1);
        let Some(scene) = project.scenes.iter().find(|s| s.id == id) else {
            continue;
        };
        for n in unmet(project, scene, &needs) {
            let need = &needs[n];
            out.push(CompileWarning {
                code: WARN_VALUE_DROPPED.into(),
                beat: Some(i),
                message: format!("\"{}\" ({}) is not shown", need.text, need.role),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(text: &str) -> Vec<Forms> {
        number_tokens(text)
    }

    #[test]
    fn numbers_match_on_digits_not_separators() {
        let want = forms("$240,000");
        assert!(has_numbers("240,000", &want));
        assert!(has_numbers("240000", &want));
        assert!(has_numbers("USD 240 000 total", &forms("$240 000")));
        assert!(!has_numbers("$120,000", &want));
        assert!(!has_numbers("7 hours", &forms("5 hours")));
        assert!(has_numbers("+3.5 km", &forms("3.5 km")));
        assert!(has_numbers("3,5", &forms("3.5")));
    }

    #[test]
    fn the_decimal_point_keeps_its_place() {
        // 4.1 is not 41, 0.5 is not 5, $0.14 is not 14 (either way round).
        assert!(!has_numbers("41%", &forms("4.1%")));
        assert!(!has_numbers("4.1%", &forms("41%")));
        assert!(!has_numbers("5", &forms("0.5")));
        assert!(!has_numbers("0.5", &forms("5")));
        assert!(!has_numbers("14", &forms("$0.14")));
        assert!(!has_numbers("$0.14 / L", &forms("$1.15 / L")));
        // Trailing zeros and a decimal comma are the same number.
        assert!(has_numbers("4.10%", &forms("4.1%")));
        assert!(has_numbers("0,14", &forms("$0.14")));
        assert!(has_numbers("$0.14 / L", &forms("$0.14 / L")));
        // Thousands groups are not decimals.
        assert!(has_numbers("1,200,000", &forms("1200000")));
        assert!(!has_numbers("1.200", &forms("1200")));
        // Compact forms move the point: 1.2M is 1200000, not 12.
        assert!(has_numbers("1.2M", &forms("1,200,000")));
        assert!(!has_numbers("12M", &forms("1,200,000")));
        // Numbers said in words keep it too: "four point one" is 4.1.
        assert!(has_numbers("four point one", &forms("4.1")));
        assert!(!has_numbers("four point one", &forms("41")));
        assert!(!has_numbers("forty-one", &forms("4.1")));
    }

    #[test]
    fn indian_grouping_is_a_whole_number() {
        // "1,20,000" is 120000, not 1.2 and 0.
        assert!(has_numbers("\u{20b9}1,20,000", &forms("\u{20b9}120,000")));
        assert!(has_numbers("120000", &forms("\u{20b9}1,20,000")));
        assert!(has_numbers("\u{20b9}1,20,000", &forms("120000")));
        assert!(has_numbers("12,34,56,789", &forms("123456789")));
        assert!(has_numbers("\u{20b9}1,20,000 total", &forms("1,20,000")));
        assert!(!has_numbers("\u{20b9}1,20,000", &forms("1.2")));
        assert!(!has_numbers("\u{20b9}1,20,000", &forms("0")));
        assert!(!has_numbers("\u{20b9}1,20,000", &forms("12,000")));
        // Two digits after a comma that ends the number are still a decimal.
        assert!(has_numbers("1,20", &forms("1.2")));
        assert!(!has_numbers("1,20", &forms("120")));
        assert_eq!(forms("1,20,000").len(), 1, "one token");
    }

    #[test]
    fn a_compact_form_matches_both_ways() {
        assert!(has_numbers("$240K", &forms("$240,000")));
        assert!(has_numbers("$240,000", &forms("$240K")));
        assert!(has_numbers("1.2M", &forms("1,200,000")));
        assert!(has_numbers("1.2 million", &forms("1200000")));
        // A unit that merely starts with the letter is not a suffix.
        assert!(!has_numbers("240km", &forms("$240,000")));
        assert!(!has_numbers("$250K", &forms("$240,000")));
    }

    #[test]
    fn numbers_said_in_words_match_the_digits() {
        assert!(has_numbers("FIFTY LITRES", &forms("50 L")));
        assert!(has_numbers("twenty-five", &forms("25")));
        assert!(has_numbers("three hundred and ten", &forms("310")));
        assert!(has_numbers("four point one", &forms("4.1%")));
        assert!(has_numbers("two million", &forms("$2M")));
        assert!(!has_numbers("fifteen", &forms("50")));
        // A figure in words is a figure; "No one wins" is a phrase.
        assert!(is_numeric("three days"));
        assert!(!is_numeric("No one wins"));
        assert!(has_numbers("3 DAYS", &spoken_numbers("three days")));
    }

    #[test]
    fn words_are_whole_content_words() {
        assert_eq!(content_words("Raise yearly"), ["raise", "yearly"]);
        assert_eq!(content_words("The brain"), ["brain"]);
        assert_eq!(content_words("The"), ["the"]);
    }

    #[test]
    fn needs_follow_the_intent() {
        let beat: Beat = serde_json::from_value(serde_json::json!({
            "purpose": "compare", "statement": "x",
            "primary": {"kind": "number", "value": "7 hours", "meaning": "needed"},
            "secondary": {"kind": "number", "value": "5 hours", "meaning": "typical"}
        }))
        .unwrap();
        let n = needs_of(&beat);
        assert_eq!(n.len(), 2);
        assert!(n.iter().all(|n| n.role == "compare side"));
        let beat: Beat = serde_json::from_value(serde_json::json!({
            "purpose": "emphasize", "statement": "x",
            "primary": {"kind": "phrase", "value": "Maintenance"}
        }))
        .unwrap();
        assert!(needs_of(&beat).is_empty());
    }
}
