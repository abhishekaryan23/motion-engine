//! (0.20) Story-level compile warnings: structures that would read better and
//! requests the builders dropped. Pure: they look at the intent and at what
//! was built, change nothing, and are printed by the CLI (never written into
//! the scene, so default compiles stay byte-identical).
//!
//! | code | rule |
//! |---|---|
//! | `unrelated_beats` | a story of >= 3 beats, at least three of them pictures (object primaries), where every beat is one unrelated atomic subject and no `layers`, collection, compare / contrast, relationship, `carry_*` or repeated subject ties them; suggests the structures that fit |
//! | `carry_ignored` | a beat asked for `continuity: carry_*` but no shared element carried over, with the reason: the look cannot carry library pictures yet (cinematic, documentary, studio, street, hype), the compile has no direction seed (pictures are carried on the product path), a neighbouring beat enters with an accent flood, or the builder placed none |
//! | `relationship_dropped` | (0.23 W8) a compare / contrast beat of two subjects names a `relationship` and its scene draws no connector of that relationship (see the id patterns below) |
//! | `title_states_conclusion` | the display title says the beat's number or keyword that the narration only says later than ENTER + 0.8 s, under a look whose `TitleReveal` is `Free` |
//! | `unnamed_picture` | a beat's picture (an object primary / secondary, or a delivered hero image) is never named in that beat's spoken line (narration, else statement), so it shows before anyone says what it is |
//!
//! Heuristics (all deterministic, no randomness, no clocks):
//!
//! * `unrelated_beats` fires once per story (`beat: None`) when there are at
//!   least 3 beats and every beat is atomic (primary phrase / number / object,
//!   an atomic or absent secondary), none is a compare / contrast with a
//!   secondary, none sets a `relationship` or a `continuity` carry, and no
//!   beat shares an object asset or a phrase / number text with the next one.
//!   The message suggests structures from the beats' words: two or more beats
//!   with figures -> compare / ratio; two or more reading as a sequence
//!   (first, then, next, finally, step) -> steps; two or more naming nested
//!   things (layer, zone, level, inside, within, part of) -> layers; else the
//!   generic list.
//! * `carry_ignored` fires per beat that asked for `carry_primary` /
//!   `carry_secondary` when no shared element has track keys in both the
//!   beat's scene and the next one (the previous one on the last beat).
//! * `relationship_dropped` fires per `compare` / `contrast` beat that has a
//!   `relationship` and a secondary subject of its own kind (two pictures, or
//!   two figures / phrases: a picture beside a note is a hero with a
//!   supporting line, and a picture whose asset is missing cannot be drawn at
//!   all), when no layer of the beat's scene (group children included) is a
//!   connector of that relationship. A layer's *name* is its id after the
//!   scene prefix (`b2.pair_arrow` -> `pair_arrow`, `b3.divider` -> `divider`;
//!   a nested id by its first segment); a name *has* a word when the word is
//!   the name or one of its `_` parts. The relation stage
//!   (`grammar/relation_stage.rs`, also the flat `StatPair`) draws
//!   `pair_arrow` + `pair_arrowhead`, `pair_divider` + `pair_badge` (text
//!   "VS"), `pair_badge` (text "+") and `pair_link`; the patterns accepted
//!   per relationship are:
//!
//!   | relationship | a connector is a layer whose name has the word |
//!   |---|---|
//!   | `grow`, `replace`, `compress` | `arrow` |
//!   | `accumulate` | `plus`, or a `badge` whose text is "+" |
//!   | `separate` | `divider`, `vs`, or a `badge` whose text is "VS" |
//!   | `carry` | `link` |
//!
//!   A builder that draws the relationship another way (a dossier marker)
//!   names its connector layers with these words to count.
//! * `title_states_conclusion` fires per beat that has a narration, when the
//!   display title contains one of the beat's figures (as digits or as said:
//!   "8" / "eight") or its keyword (plural or singular) and the narrator says
//!   that word more than [`SPOIL_AFTER_ENTER`] seconds after ENTER, under a
//!   look whose title policy is `TitleReveal::Free` (or no look).
//! * `unnamed_picture` fires per picture of a beat that its spoken line
//!   (`narration`, else the statement: the text, so it also fires on a
//!   compile without a voice-over) never names. A picture is named when the
//!   line says its `meaning` or its asset name (as `speech_plan::anchor_time`
//!   matches cue words: stemmed, numbers by value), or one distinctive word of
//!   it: a word of the asset name other than generic picture words (`flag`,
//!   `icon`, ...), or a capitalised word of the meaning (a proper noun). Two-
//!   letter words ("US", "UK") only match the same letters written in
//!   capitals, never the pronoun "us". A delivered hero / portrait image on a
//!   beat whose primary is not an object is named by the primary's meaning
//!   (no meaning: not judged).

use super::art_direction::{Look, TitleReveal};
use super::{
    taste_rules, temporal, CompileWarning, Ctx, WARN_CARRY_IGNORED, WARN_RELATIONSHIP_DROPPED,
    WARN_TITLE_STATES_CONCLUSION, WARN_UNNAMED_PICTURE, WARN_UNRELATED_BEATS,
};
use crate::intent::{
    Beat, Continuity, CreativeIntent, Purpose, Relationship, Subject, SubjectKind,
};
use crate::scene::{Layer, LayerKind, Scene};
use crate::speech::{normalize_word, statement_tokens, SpeechMap};

/// A title word counts as spoiling when the narrator says it more than this
/// long (seconds) after the beat's ENTER boundary.
pub(crate) const SPOIL_AFTER_ENTER: f64 = 0.8;

/// Words that make a beat read as one step of a sequence.
const SEQUENCE_CUES: &[&str] = &[
    "first", "then", "next", "finally", "lastly", "step", "steps",
];

/// Words that make a beat read as something nested in something else.
const NESTING_CUES: &[&str] = &[
    "layer", "layers", "zone", "zones", "level", "levels", "inside", "within",
];

/// Run every story-level check over the built scenes. `scenes[0]` is the
/// backdrop; `scenes[1 + i]` is beat `i`. `carries` are the shared elements
/// that will be written to `project.shared`.
/// `speech` is the voice-over the beats were timed to (`--speech`), when any.
pub(crate) fn check(
    intent: &CreativeIntent,
    ctx: &Ctx,
    scenes: &[Scene],
    carries: &[super::Carry],
    speech: Option<&SpeechMap>,
) -> Vec<CompileWarning> {
    let mut out = Vec::new();
    out.extend(unrelated_beats(intent, ctx));
    out.extend(carry_ignored(intent, ctx, scenes, carries));
    out.extend(relationship_dropped(intent, ctx, scenes));
    out.extend(title_states_conclusion(intent, ctx, scenes, speech));
    out.extend(unnamed_pictures(intent, ctx));
    out
}

// ---------------------------------------------------------------------------
// unrelated_beats
// ---------------------------------------------------------------------------

/// Lowercased alphanumeric words of `text`.
fn words_lower(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Everything a beat says in words: statement, narration, keyword and the
/// values and meanings of its atomic subjects.
fn beat_words(beat: &Beat) -> Vec<String> {
    let mut text = vec![beat.statement.as_str()];
    text.extend(beat.narration.as_deref());
    text.extend(beat.keyword.as_deref());
    for s in std::iter::once(&beat.primary).chain(beat.secondary.as_ref()) {
        if s.is_atomic() {
            text.extend(s.value());
            text.extend(s.meaning());
        }
    }
    text.iter().flat_map(|t| words_lower(t)).collect()
}

/// The beat reads as one step of a sequence (first, then, next, ...).
fn reads_as_step(words: &[String]) -> bool {
    words.iter().any(|w| SEQUENCE_CUES.contains(&w.as_str()))
}

/// The beat names something nested in something else (layer, zone, inside,
/// within, part of, ...).
fn reads_as_nested(words: &[String]) -> bool {
    words.iter().any(|w| NESTING_CUES.contains(&w.as_str()))
        || words
            .windows(2)
            .any(|p| (p[0] == "part" || p[0] == "parts") && p[1] == "of")
}

/// The beat carries a figure: a digit in an atomic subject's value or in the
/// statement.
fn has_figure(beat: &Beat) -> bool {
    let digit = |s: &str| s.chars().any(|c| c.is_ascii_digit());
    digit(&beat.statement)
        || std::iter::once(&beat.primary)
            .chain(beat.secondary.as_ref())
            .any(|s| s.is_atomic() && s.value().is_some_and(digit))
}

/// What identifies an atomic subject across beats: an object's asset, or a
/// phrase / number's text (value, else meaning). `None` for structured ones.
fn identity(s: &Subject) -> Option<String> {
    match s {
        Subject::Object(o) => Some(format!("object:{}", o.asset.trim().to_lowercase())),
        Subject::Phrase(a) | Subject::Number(a) => {
            let text = a.value.as_deref().or(a.meaning.as_deref())?;
            let norm: Vec<String> = text
                .split_whitespace()
                .map(normalize_word)
                .filter(|w| !w.is_empty())
                .collect();
            (!norm.is_empty()).then(|| format!("text:{}", norm.join(" ")))
        }
        _ => None,
    }
}

fn identities(beat: &Beat) -> Vec<String> {
    std::iter::once(&beat.primary)
        .chain(beat.secondary.as_ref())
        .filter_map(identity)
        .collect()
}

/// An object asset or a phrase / number text recurs in two consecutive beats.
fn shares_subject(a: &Beat, b: &Beat) -> bool {
    let ids = identities(a);
    identities(b).iter().any(|k| ids.contains(k))
}

/// Plural name of the primary kinds of a story ("objects"), or "subjects".
fn primary_kinds(beats: &[Beat]) -> &'static str {
    let first = beats[0].primary.kind();
    if beats.iter().all(|b| b.primary.kind() == first) {
        match first {
            SubjectKind::Object => "objects",
            SubjectKind::Phrase => "phrases",
            SubjectKind::Number => "numbers",
            _ => "subjects",
        }
    } else {
        "subjects"
    }
}

fn unrelated_beats(intent: &CreativeIntent, ctx: &Ctx) -> Option<CompileWarning> {
    let beats = &intent.beats;
    if beats.len() < 3 || ctx.stack.is_some() {
        return None;
    }
    for beat in beats {
        let atomic_primary = matches!(
            beat.primary.kind(),
            SubjectKind::Object | SubjectKind::Phrase | SubjectKind::Number
        );
        let atomic_secondary = beat.secondary.as_ref().is_none_or(Subject::is_atomic);
        let compares = matches!(beat.purpose, Purpose::Compare | Purpose::Contrast)
            && beat.secondary.is_some();
        if !atomic_primary
            || !atomic_secondary
            || compares
            || beat.relationship.is_some()
            || beat.continuity != Continuity::None
        {
            return None;
        }
    }
    if beats.windows(2).any(|p| shares_subject(&p[0], &p[1])) {
        return None;
    }
    // The owner's complaint was one picture per beat: a story told in
    // phrases and numbers (hook, fact, payoff) is narrative, not a slideshow,
    // so the warning needs at least three beats that are pictures.
    let pictures = beats
        .iter()
        .filter(|b| b.primary.kind() == SubjectKind::Object)
        .count();
    if pictures < 3 {
        return None;
    }

    let words: Vec<Vec<String>> = beats.iter().map(beat_words).collect();
    let nested = words.iter().filter(|w| reads_as_nested(w)).count();
    let steps = words.iter().filter(|w| reads_as_step(w)).count();
    let figures = beats.iter().filter(|b| has_figure(b)).count();
    let mut fits: Vec<&str> = Vec::new();
    if nested >= 2 {
        fits.push("layers (one `layers` primary repeated in every beat)");
    }
    if figures >= 2 {
        fits.push("compare / ratio (compare beats with a secondary, or a derived_metric)");
    }
    if steps >= 2 {
        fits.push(
            "steps (beats in order; keep the thread on screen with carry_primary, or \
             carry_secondary on an explain beat)",
        );
    }
    let suggestion = if fits.is_empty() {
        "pick the structure that fits the topic: layers, compare, steps (beats in order with \
         carry_primary), cycle, hierarchy, flow or timeline"
            .to_string()
    } else {
        format!("try {}", fits.join("; or "))
    };
    Some(CompileWarning {
        code: WARN_UNRELATED_BEATS.into(),
        beat: None,
        message: format!(
            "{} beats, each a separate {} with nothing tying them together (no layers, collection, \
             compare / contrast, relationship, carry_* or subject repeated in the next beat), so \
             the viewer gets {} unrelated moments instead of one explanation; {} \
             (see \"Which structure fits which topic\" in docs/AI_AUTHORING_GUIDE.md)",
            beats.len(),
            primary_kinds(beats).trim_end_matches('s'),
            beats.len(),
            suggestion
        ),
    })
}

// ---------------------------------------------------------------------------
// carry_ignored
// ---------------------------------------------------------------------------

/// `c` has a track key in the scene `scene`.
fn keyed_in(c: &super::Carry, scene: &str) -> bool {
    c.element.track.iter().any(|k| k.scene == scene)
}

/// A short description of a subject for a message.
fn describe(s: &Subject) -> String {
    match s {
        Subject::Object(o) => format!("object \"{}\"", o.asset),
        Subject::Phrase(a) | Subject::Number(a) => {
            let text = a.value.as_deref().or(a.meaning.as_deref()).unwrap_or("");
            format!("{} \"{}\"", s.kind_name(), text)
        }
        other => other.kind_name().to_string(),
    }
}

fn carry_ignored(
    intent: &CreativeIntent,
    ctx: &Ctx,
    scenes: &[Scene],
    carries: &[super::Carry],
) -> Vec<CompileWarning> {
    let n = intent.beats.len();
    let look: Option<Look> = ctx.art.as_ref().map(|(a, _)| a.look);
    let mut out = Vec::new();
    for (i, beat) in intent.beats.iter().enumerate() {
        let (flag, role, subject) = match beat.continuity {
            Continuity::None => continue,
            Continuity::CarryPrimary => ("carry_primary", "primary", Some(&beat.primary)),
            Continuity::CarrySecondary => ("carry_secondary", "secondary", beat.secondary.as_ref()),
        };
        // The scene the carried element must also appear in: the next beat's,
        // or the previous one's on the last beat.
        let neighbour = if i + 1 < n {
            Some(i + 1)
        } else {
            i.checked_sub(1)
        };
        let carried = match (scenes.get(i + 1), neighbour.and_then(|j| scenes.get(j + 1))) {
            (Some(here), Some(other)) => carries
                .iter()
                .any(|c| keyed_in(c, &here.id) && keyed_in(c, &other.id)),
            // No scene to judge against: nothing to say.
            (None, _) => continue,
            (Some(_), None) => false,
        };
        if carried {
            continue;
        }
        let reason = match subject {
            None => "the beat has no secondary subject".to_string(),
            Some(s) if !s.is_atomic() => format!(
                "{} subjects never carry (only phrase, number and object subjects do)",
                s.kind_name()
            ),
            Some(_) if i + 1 == n => {
                "this is the last beat, so there is no next beat to carry into".to_string()
            }
            Some(_) if beat.purpose == Purpose::Explain && flag == "carry_primary" => {
                "an explain beat sets its primary as the headline, so carry_primary has no effect \
                 there (carry_secondary works)"
                    .to_string()
            }
            // (0.23 W8) The flat looks carry library pictures (`plate::place_carried`);
            // these looks do not yet (documentary, studio, street, hype: W8b,
            // cinematic: W8c).
            Some(Subject::Object(_))
                if matches!(
                    look,
                    Some(
                        Look::Cinematic3d
                            | Look::Dossier
                            | Look::StudioPop
                            | Look::StreetCollage
                            | Look::HypeSlam
                    )
                ) =>
            {
                format!(
                    "the {} look does not carry library pictures between beats",
                    look.map_or("", Look::name)
                )
            }
            // (0.23 W8a) A delivered or library picture is carried on the product
            // path (a direction seed); the default compile keeps it as it was.
            Some(Subject::Object(_))
                if ctx.direction_seed.is_none()
                    && super::grammar::plate::has_delivered_picture(
                        ctx,
                        i,
                        flag == "carry_primary",
                    ) =>
            {
                "pictures are carried on the product path only (compile with --variety, as \
                 `reel` does); the default compile keeps them as it always did"
                    .to_string()
            }
            // An accent flood stacks its beat's stage above every shared
            // picture, so a picture is not carried into or out of such a beat.
            Some(Subject::Object(_)) if accent_beside(ctx, intent, i) => {
                "the beat or the next one enters with an accent flood (energy impact), which \
                 would cover the picture; give a neighbour another energy to carry it"
                    .to_string()
            }
            Some(_) => match look {
                Some(l) => format!("the builder placed no shared element (look {})", l.name()),
                None => "the builder placed no shared element".to_string(),
            },
        };
        let what = subject.map(describe).unwrap_or_else(|| "subject".into());
        out.push(CompileWarning {
            code: WARN_CARRY_IGNORED.into(),
            beat: Some(i),
            message: format!(
                "continuity {flag} was dropped: the {role} {what} was to stay on screen into the \
                 next beat, but {reason}"
            ),
        });
    }
    out
}

/// Whether beat `i` or the next one enters with an accent flood
/// (`temporal::handoff`): the beat's energy is impact under a look whose
/// transitions flood.
fn accent_beside(ctx: &Ctx, intent: &CreativeIntent, i: usize) -> bool {
    let accent = |j: usize| {
        j > 0
            && intent
                .beats
                .get(j)
                .is_some_and(|b| temporal::handoff(&ctx.taste.transition, b.energy).0)
    };
    accent(i) || accent(i + 1)
}

// ---------------------------------------------------------------------------
// relationship_dropped
// ---------------------------------------------------------------------------

/// The words of a layer's name: its id after the scene prefix (`b3.` or any
/// first segment of a beat layer), the first segment of what is left, split
/// on `_`.
fn name_words(id: &str) -> Vec<String> {
    let prefixed = id.split('.').next().is_some_and(|p| {
        p.starts_with('b') && p.len() > 1 && p[1..].chars().all(|c| c.is_ascii_digit())
    });
    let rest = if prefixed {
        id.split_once('.').map_or("", |(_, r)| r)
    } else {
        id
    };
    rest.split('.')
        .next()
        .unwrap_or("")
        .split('_')
        .map(str::to_string)
        .collect()
}

/// The text a layer shows, when it is a text layer.
fn text_of(l: &Layer) -> Option<&str> {
    match &l.kind {
        LayerKind::Text(t) => Some(t.text.as_str()),
        _ => None,
    }
}

/// Whether the layer tree has a connector of `relationship` (see the module
/// docs for the patterns).
fn has_connector(layers: &[Layer], relationship: Relationship) -> bool {
    layers.iter().any(|l| {
        let words = name_words(&l.id);
        let has = |w: &str| words.iter().any(|x| x == w);
        let badge_text = |t: &str| {
            has("badge")
                && matches!(&l.kind, LayerKind::Group { children }
                    if children.iter().any(|c| text_of(c).is_some_and(|x| x.trim().eq_ignore_ascii_case(t))))
        };
        let here = match relationship {
            Relationship::Grow | Relationship::Replace | Relationship::Compress => has("arrow"),
            Relationship::Accumulate => has("plus") || badge_text("+"),
            Relationship::Separate => has("divider") || has("vs") || badge_text("VS"),
            Relationship::Carry => has("link"),
        };
        here || matches!(&l.kind, LayerKind::Group { children } if has_connector(children, relationship))
    })
}

fn relationship_name(r: Relationship) -> &'static str {
    match r {
        Relationship::Grow => "grow",
        Relationship::Replace => "replace",
        Relationship::Compress => "compress",
        Relationship::Accumulate => "accumulate",
        Relationship::Separate => "separate",
        Relationship::Carry => "carry",
    }
}

fn connector_name(r: Relationship) -> &'static str {
    match r {
        Relationship::Grow | Relationship::Replace | Relationship::Compress => "an arrow",
        Relationship::Accumulate => "a plus",
        Relationship::Separate => "a VS badge or a divider",
        Relationship::Carry => "a link",
    }
}

/// An object subject of beat `index` that no picture serves: neither a
/// delivered / catalog image for the role its place has (the primary's hero /
/// evidence images, the secondary's supporting image) nor a library object.
/// Such a beat cannot draw its pair at all (the asset is missing, a different
/// problem from a builder that does not draw the relationship).
fn picture_missing(ctx: &Ctx, index: usize, s: &Subject, primary: bool) -> bool {
    let Subject::Object(o) = s else {
        return false;
    };
    !super::grammar::plate::has_delivered_picture(ctx, index, primary)
        && ctx.library.find_object(&o.asset).is_none()
}

fn relationship_dropped(
    intent: &CreativeIntent,
    ctx: &Ctx,
    scenes: &[Scene],
) -> Vec<CompileWarning> {
    let mut out = Vec::new();
    for (i, beat) in intent.beats.iter().enumerate() {
        let (Some(relationship), Some(secondary)) = (beat.relationship, beat.secondary.as_ref())
        else {
            continue;
        };
        if !matches!(beat.purpose, Purpose::Compare | Purpose::Contrast) {
            continue;
        }
        // A picture beside a phrase or a figure is a hero with a supporting
        // line, not a pair: the hero's own motion tells the relationship (a
        // pair whose second picture is missing is built this way too).
        let pictures = [&beat.primary, secondary]
            .iter()
            .filter(|s| s.kind() == SubjectKind::Object)
            .count();
        if pictures == 1
            || picture_missing(ctx, i, &beat.primary, true)
            || picture_missing(ctx, i, secondary, false)
        {
            continue;
        }
        let Some(scene) = scenes.get(i + 1) else {
            continue;
        };
        if has_connector(&scene.layers, relationship) {
            continue;
        }
        out.push(CompileWarning {
            code: WARN_RELATIONSHIP_DROPPED.into(),
            beat: Some(i),
            message: format!(
                "relationship {} was dropped: the beat compares two subjects, but its scene draws \
                 no connector for it ({} between them)",
                relationship_name(relationship),
                connector_name(relationship)
            ),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// title_states_conclusion
// ---------------------------------------------------------------------------

/// The numeric core of a figure token ("$381" -> "381", "23%" -> "23",
/// "6.7x" -> "6.7", "1,200" -> "1200"). `None` for anything that is not a
/// figure ("3D", "A4", "minutes").
fn number_core(token: &str) -> Option<String> {
    let t = token.trim_start_matches(['$', '€', '£', '₹', '¥', '+', '-', '−', '~']);
    let digits: String = t
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '.'))
        .collect();
    let rest = &t[digits.len()..];
    let unit_ok = matches!(
        rest.to_lowercase().as_str(),
        "" | "%" | "x" | "×" | "k" | "m" | "b" | "bn" | "st" | "nd" | "rd" | "th"
    );
    let core = digits.replace(',', "").trim_end_matches('.').to_string();
    (unit_ok && core.chars().any(|c| c.is_ascii_digit())).then_some(core)
}

enum NumberWord {
    Unit(u64),
    Hundred,
    Scale(u64),
}

fn number_word(w: &str) -> Option<NumberWord> {
    const SMALL: [&str; 20] = [
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
    if let Some(i) = SMALL.iter().position(|s| *s == w) {
        return Some(NumberWord::Unit(i as u64));
    }
    if let Some(i) = TENS.iter().position(|s| *s == w) {
        return Some(NumberWord::Unit((i as u64 + 2) * 10));
    }
    match w {
        "hundred" => Some(NumberWord::Hundred),
        "thousand" => Some(NumberWord::Scale(1_000)),
        "million" => Some(NumberWord::Scale(1_000_000)),
        "billion" => Some(NumberWord::Scale(1_000_000_000)),
        "trillion" => Some(NumberWord::Scale(1_000_000_000_000)),
        _ => None,
    }
}

/// The figures the narrator says, with the scene-local second each starts:
/// digit tokens as their core, English number words as their value ("eight"
/// -> 8, "three hundred eighty one billion" -> 381000000000, plus the part
/// before a million / billion scale word, 381, so "$381 billion" matches).
fn spoken_figures(spoken: &[(String, f64)]) -> Vec<(String, f64)> {
    let words: Vec<String> = spoken
        .iter()
        .map(|(w, _)| normalize_word(w).replace('-', " "))
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < spoken.len() {
        if let Some(core) = number_core(&spoken[i].0) {
            out.push((core, spoken[i].1));
            i += 1;
            continue;
        }
        let (mut total, mut current, mut mantissa) = (0u64, 0u64, None);
        let (mut j, mut any) = (i, false);
        'run: while j < words.len() {
            // A hyphenated number ("twenty-three") normalises to two words.
            let parts: Vec<&str> = words[j].split_whitespace().collect();
            if parts.is_empty() {
                break;
            }
            let mut next = (total, current, mantissa);
            for part in &parts {
                match number_word(part) {
                    Some(NumberWord::Unit(n)) => next.1 += n,
                    Some(NumberWord::Hundred) => next.1 = next.1.max(1) * 100,
                    Some(NumberWord::Scale(s)) => {
                        if s >= 1_000_000 && next.0 == 0 {
                            next.2 = Some(next.1.max(1));
                        }
                        next.0 += next.1.max(1) * s;
                        next.1 = 0;
                    }
                    // "and" inside a run ("one hundred and five") only when a
                    // number word follows.
                    None if *part == "and" && j + 1 < words.len() => {
                        let follows = words[j + 1]
                            .split_whitespace()
                            .next()
                            .is_some_and(|w| number_word(w).is_some());
                        if !follows || !any {
                            break 'run;
                        }
                    }
                    None => break 'run,
                }
            }
            (total, current, mantissa) = next;
            any = any || parts.iter().any(|p| number_word(p).is_some());
            j += 1;
        }
        if any {
            out.push(((total + current).to_string(), spoken[i].1));
            if let Some(m) = mantissa {
                out.push((m.to_string(), spoken[i].1));
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// Singular form for matching a plural against its singular ("minutes" ->
/// "minute", "berries" -> "berry").
fn fold_word(w: &str) -> String {
    let n = normalize_word(w);
    let len = n.chars().count();
    if len > 4 && n.ends_with("ies") {
        return format!("{}y", &n[..n.len() - 3]);
    }
    if len > 4
        && ["ses", "xes", "zes", "ches", "shes"]
            .iter()
            .any(|s| n.ends_with(s))
    {
        return n[..n.len() - 2].to_string();
    }
    match n.strip_suffix('s') {
        Some(base) if len > 3 && !base.ends_with('s') => base.to_string(),
        _ => n,
    }
}

/// The figures a beat is about: the numeric cores (digits or number words)
/// of its atomic subjects' values.
fn beat_figures(beat: &Beat) -> Vec<String> {
    std::iter::once(&beat.primary)
        .chain(beat.secondary.as_ref())
        .filter(|s| s.is_atomic())
        .filter_map(Subject::value)
        .flat_map(|v| {
            let tokens: Vec<(String, f64)> =
                statement_tokens(v).into_iter().map(|t| (t, 0.0)).collect();
            spoken_figures(&tokens)
        })
        .map(|(core, _)| core)
        .collect()
}

/// The keyword's stand-in word: its longest token (at least 3 letters).
fn keyword_word(beat: &Beat) -> Option<String> {
    let kw = beat.keyword.as_deref()?;
    let mut best: Option<String> = None;
    for t in statement_tokens(kw) {
        let n = fold_word(&t);
        if n.chars().count() >= 3
            && best
                .as_ref()
                .is_none_or(|b| n.chars().count() > b.chars().count())
        {
            best = Some(n);
        }
    }
    best
}

fn title_states_conclusion(
    intent: &CreativeIntent,
    ctx: &Ctx,
    scenes: &[Scene],
    speech: Option<&SpeechMap>,
) -> Vec<CompileWarning> {
    let Some(speech) = speech else {
        return Vec::new();
    };
    // A look that holds titles cues them to their word: nothing to say.
    if ctx
        .art
        .as_ref()
        .is_some_and(|(a, _)| a.fx.title_reveal == TitleReveal::Hold)
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, beat) in intent.beats.iter().enumerate() {
        // Without a narration the narrator reads the title itself.
        if beat
            .narration
            .as_deref()
            .is_none_or(|n| n.trim().is_empty())
        {
            continue;
        }
        let (Some(scene), Some(sentence)) = (
            scenes.get(i + 1),
            speech.sentences.iter().find(|s| s.beat == i),
        ) else {
            continue;
        };
        let title = taste_rules::display_text(beat, true).title;
        let title_tokens = statement_tokens(&title);
        let enter = scene.lifecycle.map(|l| l.enter).unwrap_or(0.0);
        let spoken: Vec<(String, f64)> = speech
            .words_in(sentence)
            .iter()
            .map(|w| (w.text.clone(), w.start - scene.start_seconds))
            .collect();

        // (title word as written, seconds the narrator says it after ENTER)
        let mut found: Vec<(String, f64)> = Vec::new();
        let figures = beat_figures(beat);
        let said = spoken_figures(&spoken);
        let on_title: Vec<(String, f64)> = title_tokens
            .iter()
            .enumerate()
            .map(|(k, t)| (t.clone(), k as f64))
            .collect();
        for (core, at) in spoken_figures(&on_title) {
            if !figures.contains(&core) {
                continue;
            }
            if let Some((_, t)) = said.iter().find(|(c, _)| *c == core) {
                found.push((title_tokens[at as usize].clone(), t - enter));
            }
        }
        if let Some(kw) = keyword_word(beat) {
            if let Some(token) = title_tokens.iter().find(|t| fold_word(t) == kw) {
                if let Some((_, t)) = spoken.iter().find(|(w, _)| fold_word(w) == kw) {
                    found.push((token.clone(), t - enter));
                }
            }
        }
        // The worst spoiler (largest gap) names the warning; ties keep order.
        let mut worst: Option<&(String, f64)> = None;
        for f in found.iter().filter(|f| f.1 > SPOIL_AFTER_ENTER) {
            if worst.is_none_or(|w| f.1 > w.1) {
                worst = Some(f);
            }
        }
        if let Some((word, gap)) = worst {
            out.push(CompileWarning {
                code: WARN_TITLE_STATES_CONCLUSION.into(),
                beat: Some(i),
                message: format!(
                    "title \"{title}\" says \"{word}\" {gap:.1} s before the narrator does; keep \
                     the title to the topic or use a look that holds titles"
                ),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// unnamed_picture
// ---------------------------------------------------------------------------

/// Words of an asset name that say what kind of picture it is, not what it
/// shows ("flag_us" is named by "us", not by "flag").
const GENERIC_PICTURE_WORDS: &[&str] = &[
    "flag",
    "icon",
    "logo",
    "image",
    "picture",
    "photo",
    "illustration",
    "emoji",
    "symbol",
    "sticker",
    "cutout",
    "clipart",
    "graphic",
];

/// A picture a beat shows: the name it goes by and the words that name it.
struct Pictured {
    /// Shown in the message: the meaning, else the humanised asset name.
    name: String,
    /// The meaning, if any.
    meaning: Option<String>,
    /// The asset name, humanised ("flag us"), for object pictures.
    asset: Option<String>,
}

/// The pictures of a beat: its object primary and secondary, or a delivered
/// hero / portrait image standing for a non-object primary that has a meaning.
fn pictures_of(ctx: &Ctx, index: usize, beat: &Beat) -> Vec<Pictured> {
    let mut out = Vec::new();
    for s in std::iter::once(&beat.primary).chain(beat.secondary.as_ref()) {
        if let Subject::Object(o) = s {
            let asset = o.asset.replace(['_', '-'], " ");
            let meaning = o
                .meaning
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_string);
            let name = meaning.clone().unwrap_or_else(|| asset.clone());
            out.push(Pictured {
                name,
                meaning,
                asset: Some(asset),
            });
        }
    }
    if beat.primary.kind() != SubjectKind::Object
        && super::grammar::plate::subject_image(ctx, index).is_some()
    {
        if let Some(m) = beat
            .primary
            .meaning()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            out.push(Pictured {
                name: m.to_string(),
                meaning: Some(m.to_string()),
                asset: None,
            });
        }
    }
    out
}

/// Whether the spoken `line` names the picture (see the module docs).
fn names_picture(line: &str, p: &Pictured) -> bool {
    use super::speech_plan::anchor_time;
    let tokens = statement_tokens(line);
    // The line's words with dummy times: the text decides, not the audio.
    let spoken: Vec<(String, f64)> = tokens
        .iter()
        .enumerate()
        .map(|(i, t)| (t.clone(), i as f64))
        .collect();
    let said = |entry: &str| anchor_time(&spoken, &[entry]).is_some();
    if p.meaning.as_deref().is_some_and(said) || p.asset.as_deref().is_some_and(said) {
        return true;
    }
    // One distinctive word: an asset word that is not generic, or a
    // capitalised word of the meaning (a proper noun or an abbreviation).
    let asset_words = p
        .asset
        .iter()
        .flat_map(|a| a.split_whitespace())
        .filter(|w| !GENERIC_PICTURE_WORDS.contains(&w.to_lowercase().as_str()));
    let proper = p
        .meaning
        .iter()
        .flat_map(|m| statement_tokens(m))
        .filter(|w| w.chars().next().is_some_and(char::is_uppercase))
        .collect::<Vec<_>>();
    let capitals = |w: &str| {
        tokens.iter().any(|t| {
            t.chars().count() >= 2
                && t.chars().all(|c| c.is_ascii_uppercase())
                && t.eq_ignore_ascii_case(w)
        })
    };
    asset_words.map(str::to_string).chain(proper).any(|w| {
        if w.chars().count() <= 2 {
            capitals(&w)
        } else {
            said(&w) || capitals(&w)
        }
    })
}

fn unnamed_pictures(intent: &CreativeIntent, ctx: &Ctx) -> Vec<CompileWarning> {
    let lines: Vec<String> = intent
        .beats
        .iter()
        .map(|b| taste_rules::display_text(b, true).spoken)
        .collect();
    let mut out = Vec::new();
    for (i, beat) in intent.beats.iter().enumerate() {
        if lines[i].trim().is_empty() {
            continue;
        }
        for p in pictures_of(ctx, i, beat) {
            if names_picture(&lines[i], &p) {
                continue;
            }
            // Where the story does name it, if anywhere.
            let elsewhere = (0..lines.len())
                .find(|&j| j != i && names_picture(&lines[j], &p))
                .map(|j| format!(" (beat {} does)", j + 1))
                .unwrap_or_default();
            out.push(CompileWarning {
                code: WARN_UNNAMED_PICTURE.into(),
                beat: Some(i),
                message: format!(
                    "the picture '{}' is never mentioned in this beat; it shows from the start \
                     — name it in the narration or move it to the beat that does{elsewhere}",
                    p.name
                ),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(list: &[(&str, f64)]) -> Vec<(String, f64)> {
        list.iter().map(|(w, t)| (w.to_string(), *t)).collect()
    }

    #[test]
    fn figure_cores_ignore_units_and_symbols() {
        assert_eq!(number_core("$381").as_deref(), Some("381"));
        assert_eq!(number_core("23%").as_deref(), Some("23"));
        assert_eq!(number_core("6.7x").as_deref(), Some("6.7"));
        assert_eq!(number_core("1,200").as_deref(), Some("1200"));
        assert_eq!(number_core("0.5").as_deref(), Some("0.5"));
        assert_eq!(number_core("3D"), None);
        assert_eq!(number_core("minutes"), None);
    }

    #[test]
    fn spoken_figures_read_digits_and_number_words() {
        let s = spoken(&[
            ("It", 0.0),
            ("takes", 0.3),
            ("eight", 0.6),
            ("minutes", 0.9),
            ("or", 1.2),
            ("twenty-three", 1.5),
            ("percent", 1.9),
            ("and", 2.2),
            ("40", 2.5),
        ]);
        let f = spoken_figures(&s);
        assert!(f.contains(&("8".to_string(), 0.6)), "{f:?}");
        assert!(f.contains(&("23".to_string(), 1.5)), "{f:?}");
        assert!(f.contains(&("40".to_string(), 2.5)), "{f:?}");
    }

    #[test]
    fn spoken_figures_read_big_numbers_with_their_mantissa() {
        let s = spoken(&[
            ("three", 1.0),
            ("hundred", 1.2),
            ("and", 1.4),
            ("eighty", 1.5),
            ("one", 1.7),
            ("billion", 1.9),
            ("dollars", 2.3),
        ]);
        let f = spoken_figures(&s);
        assert!(f.contains(&("381000000000".to_string(), 1.0)), "{f:?}");
        assert!(f.contains(&("381".to_string(), 1.0)), "{f:?}");
    }

    fn object(asset: &str, meaning: Option<&str>) -> Pictured {
        let asset = asset.replace('_', " ");
        Pictured {
            name: meaning.map_or(asset.clone(), str::to_string),
            meaning: meaning.map(str::to_string),
            asset: Some(asset),
        }
    }

    #[test]
    fn a_picture_is_named_by_its_meaning_asset_or_a_proper_word() {
        let us = object("flag_us", Some("United States"));
        assert!(names_picture("In the United States it costs $58.", &us));
        assert!(!names_picture(
            "Fifty litres of fuel. Same tank. Five countries.",
            &us
        ));
        // "US" in capitals names it; the pronoun never does.
        let us_tax = object("flag_us", Some("US fuel tax"));
        assert!(names_picture("The US adds just $0.14.", &us_tax));
        assert!(!names_picture("Let us add it up.", &us_tax));
        // An asset word that is not generic; "flag" alone names nothing.
        let de = object("flag_germany", Some("German fuel tax"));
        assert!(names_picture("Germany adds over $1.15 of tax.", &de));
        assert!(!names_picture("Every flag waves; fuel is taxed.", &de));
        let uk = object("flag_uk", Some("United Kingdom"));
        assert!(names_picture("The Bank of England warns UK inflation", &uk));
        // A plural names its singular.
        assert!(names_picture(
            "Robots are cheap.",
            &object("robot", Some("a robot"))
        ));
    }

    #[test]
    fn plurals_fold_to_their_singular() {
        assert_eq!(fold_word("minutes"), "minute");
        assert_eq!(fold_word("Berries,"), "berry");
        assert_eq!(fold_word("boxes"), "box");
        assert_eq!(fold_word("glass"), "glass");
    }

    // -- relationship_dropped: the connector id patterns ---------------------

    fn layer(json: &str) -> Layer {
        serde_json::from_str(json).expect("layer")
    }

    fn rect(id: &str) -> Layer {
        layer(&format!(
            r##"{{ "id": "{id}", "type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "fill": "#000000" }}"##
        ))
    }

    /// The relation stage's badge: a group of a disc and a text.
    fn badge(id: &str, text: &str) -> Layer {
        layer(&format!(
            r##"{{ "id": "{id}", "type": "group", "x": 0, "y": 0, "width": 10, "height": 10,
                  "children": [
                    {{ "id": "{id}.disc", "type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "fill": "#000000" }},
                    {{ "id": "{id}.text", "type": "text", "x": 0, "y": 0, "width": 10, "height": 10,
                      "text": "{text}", "font_role": "number", "font_size": 20, "color": "#111111" }} ] }}"##
        ))
    }

    /// A stage group holding `children` (the connectors sit inside the stage).
    fn stage(children: Vec<Layer>) -> Vec<Layer> {
        let mut g = rect("b2.stage");
        g.kind = LayerKind::Group { children };
        vec![g]
    }

    const ALL: [Relationship; 6] = [
        Relationship::Grow,
        Relationship::Replace,
        Relationship::Compress,
        Relationship::Accumulate,
        Relationship::Separate,
        Relationship::Carry,
    ];

    #[test]
    fn layer_names_are_read_after_the_scene_prefix() {
        assert_eq!(name_words("b2.pair_arrow"), ["pair", "arrow"]);
        assert_eq!(name_words("b12.divider"), ["divider"]);
        assert_eq!(name_words("b3.pair_badge.text"), ["pair", "badge"]);
        // No scene prefix (a shared element): the first segment.
        assert_eq!(name_words("shared.pic.apple"), ["shared"]);
        // A name that merely starts with a b is not a prefix.
        assert_eq!(name_words("badge.disc"), ["badge"]);
    }

    #[test]
    fn an_arrow_is_the_connector_of_grow_replace_and_compress() {
        let scene = stage(vec![rect("b2.pair_arrow"), rect("b2.pair_arrowhead")]);
        for r in ALL {
            let arrow = matches!(
                r,
                Relationship::Grow | Relationship::Replace | Relationship::Compress
            );
            assert_eq!(has_connector(&scene, r), arrow, "{r:?}");
        }
    }

    #[test]
    fn a_badge_is_a_plus_or_a_vs_by_its_text() {
        let plus = stage(vec![badge("b2.pair_badge", "+")]);
        assert!(has_connector(&plus, Relationship::Accumulate));
        assert!(!has_connector(&plus, Relationship::Separate));
        let vs = stage(vec![badge("b2.pair_badge", "VS")]);
        assert!(has_connector(&vs, Relationship::Separate));
        assert!(!has_connector(&vs, Relationship::Accumulate));
        // A divider alone is a separator too (split_contrast, the relation stage).
        let divider = stage(vec![rect("b2.divider")]);
        assert!(has_connector(&divider, Relationship::Separate));
        assert!(!has_connector(&divider, Relationship::Accumulate));
    }

    #[test]
    fn a_link_is_the_connector_of_carry() {
        let scene = stage(vec![rect("b2.pair_link")]);
        assert!(has_connector(&scene, Relationship::Carry));
        assert!(!has_connector(&scene, Relationship::Grow));
    }

    #[test]
    fn a_scene_with_the_connector_removed_has_none() {
        // The pair's pictures, labels and figures without any connector.
        let scene = stage(vec![
            rect("b2.hero"),
            rect("b2.prop"),
            rect("b2.pair_label.0"),
            rect("b2.pair_stamp.0"),
            rect("b2.pair_node"),
        ]);
        for r in ALL {
            assert!(!has_connector(&scene, r), "{r:?}");
        }
    }
}
