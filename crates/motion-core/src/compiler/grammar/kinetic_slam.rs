//! (0.14) KineticSlam — hype: a run of hard cuts, each cut slams one or two
//! words (or the picture) into frame.
//!
//! The beat's display title is split into punch units (1–2 words each, the
//! primary value and the keyword always their own unit, numbers kept whole)
//! that fill ENTER..ANTICIPATE, 0.6–1.0 s apiece by syllable weight. Every
//! unit is a group (`b<N>.cut.<k>`) visible from its cut time to the next cut
//! (the cut is a 0.01 s opacity step, no crossfade); the last unit holds to
//! the scene end. Layouts rotate by unit index: A giant word, B inverse accent
//! field with the word in `on_accent`, C the word above the beat's picture
//! (55 % of the layout height). A word is the beat's lead (0.24 Wave 1,
//! `compiler::choreography`): it lands on its word, scales up through a short
//! overshoot and settles on the look's spring while it fades in, and then
//! stays still; pictures are its support and slide in a little later after a
//! counter-move. Spec: docs/GENRE_GRAMMARS.md. Finishing effects (grain, flashes) come
//! from the FX director.
//!
//! Without a voice-over the units come from the display title (the statement on
//! screen); a keyword, primary value and secondary value (the other side of a
//! comparison) the title does not contain are appended as their own units, in
//! that order (the value is the payoff). (0.22) An object's value is a unit
//! too (the figure of a stat card), and the unit that shows the picture is the
//! one with the pictured object's value, so the figure slams above its
//! picture. Units run 0.6–1.0 s by syllable weight, stretched up to 1.8x when
//! the window has room and squeezed (down to 0.45 s) when it has not. The
//! picture is shown by exactly one unit so a carried subject is placed once.
//!
//! (0.24) With a voice-over (`ctx.spoken`) every cut sits on its word:
//! a figure [`WORD_CUE_LEAD`] before its number, a title or a keyword stamp on
//! the word, in spoken order, never before ENTER or closer than 0.45 s to the
//! previous cut. A title or keyword whose words the narrator never says is not
//! slammed; a statement said only in part, or a beat whose first word comes
//! late with no figure to open on, stands in as one small title (never a
//! slam, never an eyebrow) from the beat's first content word. A figure the
//! narrator never reads out opens such a beat instead. The picture comes when
//! its name is said (the figure's word follows on its own cut, inside the same
//! group). Earning rules: a keyword slam needs a new word (not a stem of the
//! title or a value), the payoff or an impact beat whose spoken stress it is,
//! and a slot in the stamp budget; at most two big text units per beat (the
//! value plus the keyword or one title); the colour field or band belongs to
//! the focal unit (the figure, else the primary value). Every cut is declared
//! with `B::reveal` so word cues and speech QA hold it to the voice.

use super::placement::{self, Rect, SubjectFacts};
use super::{plate, Composition};
use crate::assets::{AssetRole, ManifestEntry};
use crate::checks::TEXT_CONTRAST_DISPLAY;
use crate::compiler::choreography;
use crate::compiler::explore::contrast_ratio;
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::signature::{self, MotionSignature};
use crate::compiler::speech_plan::{anchor_time, name_time, WORD_CUE_LEAD};
use crate::compiler::typeset::{text_layer, Block, Voice};
use crate::compiler::{
    base_layer, direction, mix, mo, round3, treatment, Carry, CompileError, Ctx, Which,
};
use crate::easing::Easing;
use crate::intent::{Beat, Energy, Purpose, Subject, SubjectKind};
use crate::scene::{Layer, LayerKind, MotionOp, TextAlign};
use crate::speech::RevealRole;

/// A hard cut is an opacity step of this length.
const CUT_FADE: f64 = 0.01;
/// (0.24 Wave 2) A cut leads its word by two frames.
const CUT_LEAD: f64 = 0.05;
/// Unit length bounds (s) and the share of the last unit that must remain.
const UNIT_MIN: f64 = 0.6;
const UNIT_MAX: f64 = 1.0;
const UNIT_FLOOR: f64 = 0.45;
const LAST_MIN: f64 = 1.0;
const MAX_UNITS: usize = 7;
/// Picture height share of the layout height (layout C).
const PICTURE_HEIGHT: f32 = 0.55;
/// (0.24 Wave 1) The word overshoots to this scale on its way to rest: it
/// still fits then.
const HOLD_ZOOM: f32 = 1.06;
/// (0.24) Type size of the small title that opens a beat whose first slam
/// word comes late (px at a 1080 px short side).
const SMALL_TITLE: f32 = 96.0;

#[derive(Debug, Clone, PartialEq)]
struct Unit {
    /// Display text (case is the voice's business).
    text: String,
    syllables: usize,
    /// The primary value or the keyword: never merged, never dropped first.
    pinned: bool,
    /// (0.24) What the unit stands for (reveal anchors, the earning rules).
    role: UnitRole,
}

/// (0.24) What a punch unit is: a piece of the statement, the keyword slam,
/// or a value (the primary's or the secondary's figure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitRole {
    Title,
    Keyword,
    Value,
}

impl Unit {
    fn new(words: &[&str], pinned: bool) -> Unit {
        Unit {
            text: words.join(" "),
            syllables: words.iter().map(|w| syllables(w)).sum(),
            pinned,
            role: UnitRole::Title,
        }
    }

    fn with_role(mut self, role: UnitRole) -> Unit {
        self.role = role;
        self
    }

    /// The words a narrator would say for this unit (small words left out).
    fn content_words(&self) -> Vec<&str> {
        self.text
            .split_whitespace()
            .filter(|w| !is_stop(w))
            .collect()
    }

    fn key(&self) -> String {
        self.text
            .split_whitespace()
            .map(norm)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Natural on-screen time by syllable weight, 0.6–1.0 s.
    fn natural(&self) -> f64 {
        (0.2 + 0.2 * self.syllables as f64).clamp(UNIT_MIN, UNIT_MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// A: giant word, ink on paper.
    Giant,
    /// B: full-bleed accent field, word in `on_accent`.
    Inverse,
    /// C: the word above the beat's picture.
    Picture,
    /// C without a picture to show: the word on a tilted accent band.
    Banded,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Vowel groups, at least one; any token with a digit reads as three
/// syllables ("three eighty-one").
fn syllables(word: &str) -> usize {
    if word.chars().any(|c| c.is_ascii_digit()) {
        return 3;
    }
    let mut n = 0;
    let mut prev = false;
    for c in word.chars().flat_map(char::to_lowercase) {
        let v = matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
        if v && !prev {
            n += 1;
        }
        prev = v;
    }
    n.max(1)
}

/// Two normalised words that are one word to a viewer: equal, or one is the
/// other's stem ("tax" / "taxed", "double" / "doubled").
fn same_stem(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short == long || (short.len() >= 3 && long.starts_with(short))
}

fn is_stop(word: &str) -> bool {
    matches!(
        norm(word).as_str(),
        "a" | "an"
            | "the"
            | "of"
            | "to"
            | "in"
            | "on"
            | "at"
            | "by"
            | "is"
            | "it"
            | "or"
            | "and"
            | "for"
            | "as"
            | "my"
            | "our"
            | "your"
    )
}

/// Articles and conjunctions say nothing in a slam: dropped from units.
fn is_filler(word: &str) -> bool {
    matches!(norm(word).as_str(), "a" | "an" | "the" | "and" | "or")
}

/// A digit token or a spelled-out number: kept with the word it counts.
fn is_numberish(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit())
        || matches!(
            norm(word).as_str(),
            "one"
                | "two"
                | "three"
                | "four"
                | "five"
                | "six"
                | "seven"
                | "eight"
                | "nine"
                | "ten"
                | "twenty"
                | "thirty"
                | "forty"
                | "fifty"
                | "sixty"
                | "seventy"
                | "eighty"
                | "ninety"
                | "hundred"
                | "thousand"
                | "million"
                | "billion"
        )
}

/// A word as it is shown: surrounding punctuation dropped, currency and
/// percent signs kept, and (0.22) the sign of a figure ("+38%", "−27%").
fn shown(token: &str) -> String {
    let keep = |c: char| c.is_alphanumeric() || "$%€£₹¥".contains(c);
    let core = token.trim_matches(|c: char| !keep(c));
    let sign = token
        .find(core)
        .and_then(|at| token[..at].chars().last())
        .filter(|c| matches!(c, '+' | '-' | '−') && core.starts_with(|c: char| c.is_ascii_digit()));
    match sign {
        Some(s) if !core.is_empty() => format!("{s}{core}"),
        _ => core.to_string(),
    }
}

fn words_of(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(shown)
        .filter(|w| !w.is_empty() && !is_filler(w))
        .collect()
}

/// A word of the title and whether a clause ends after it (a unit never
/// reaches across one).
struct Tok {
    text: String,
    end: bool,
}

/// (0.24 M9) A statement of at most this many words is shown as written:
/// "Around the world" never becomes "AROUND WORLD".
const KEEP_WORDS: usize = 5;

fn title_tokens(text: &str) -> Vec<Tok> {
    let mut toks: Vec<Tok> = Vec::new();
    let as_written = text.split_whitespace().count() <= KEEP_WORDS;
    for raw in text.split_whitespace() {
        let word = shown(raw);
        let end = raw.ends_with(['.', ',', ';', ':', '!', '?', '…', '—']);
        if word.is_empty() || (is_filler(&word) && !as_written) {
            if let (true, Some(prev)) = (end, toks.last_mut()) {
                prev.end = true;
            }
            continue;
        }
        toks.push(Tok { text: word, end });
    }
    toks
}

/// The words of a subject's value when it is shown as a unit: a phrase, a
/// number, or (0.22) an object's figure.
fn value_words(s: &Subject) -> Vec<String> {
    match s.kind() {
        SubjectKind::Phrase | SubjectKind::Number | SubjectKind::Object => {
            s.value().map(words_of).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Punch units of a beat: its display title cut into 1–2-word units, plus the
/// primary value and keyword as their own units (see the module docs). The
/// keyword gets a unit of its own only when `keyword_spoken` (always without
/// a voice-over).
#[cfg(test)]
fn punch_units(beat: &Beat, keyword_spoken: bool) -> Vec<Unit> {
    punch_units_with(beat, keyword_spoken, true)
}

/// [`punch_units`]; `pin_keyword` false (a voice-over) leaves a keyword the
/// title already contains inside the title's unit ("TOP 1%", not "TOP" and
/// "1%" as two slams).
fn punch_units_with(beat: &Beat, keyword_spoken: bool, pin_keyword: bool) -> Vec<Unit> {
    let primary: Vec<String> = value_words(&beat.primary);
    let keyword: Vec<String> = beat.keyword.as_deref().map(words_of).unwrap_or_default();
    // The other side of a comparison: its written value is shown after the
    // primary (an object secondary's figure too; its picture is not a word).
    let secondary: Vec<String> = beat.secondary.as_ref().map(value_words).unwrap_or_default();
    let toks = title_tokens(&beat.statement);
    let as_written = beat.statement.split_whitespace().count() <= KEEP_WORDS;
    let keys: Vec<String> = toks.iter().map(|t| norm(&t.text)).collect();
    let find = |seq: &[String]| -> Option<usize> {
        let want: Vec<String> = seq.iter().map(|t| norm(t)).collect();
        if want.is_empty() || want.len() > keys.len() {
            return None;
        }
        (0..=keys.len() - want.len()).find(|&i| keys[i..i + want.len()] == want[..])
    };
    // Pinned spans of the title: (start, len, role).
    let mut spans: Vec<(usize, usize, UnitRole)> = Vec::new();
    for (seq, role) in [
        (&primary, UnitRole::Value),
        (&keyword, UnitRole::Keyword),
        (&secondary, UnitRole::Value),
    ] {
        if role == UnitRole::Keyword && !pin_keyword {
            continue;
        }
        if let Some(i) = find(seq) {
            let overlaps = spans
                .iter()
                .any(|&(s, l, _)| i < s + l && s < i + seq.len());
            if !overlaps {
                spans.push((i, seq.len(), role));
            }
        }
    }
    spans.sort_unstable_by_key(|&(s, l, _)| (s, l));

    let mut units: Vec<Unit> = Vec::new();
    let mut pending: Vec<&str> = Vec::new();
    // A unit of nothing but small words ("of", "is") says nothing: dropped.
    let flush = |pending: &mut Vec<&str>, units: &mut Vec<Unit>| {
        if pending.iter().any(|w| !is_stop(w)) {
            units.push(Unit::new(pending, false));
        }
        pending.clear();
    };
    let mut i = 0;
    while i < toks.len() {
        if let Some(&(s, l, role)) = spans.iter().find(|&&(s, _, _)| s == i) {
            flush(&mut pending, &mut units);
            let words: Vec<&str> = toks[s..s + l].iter().map(|t| t.text.as_str()).collect();
            units.push(Unit::new(&words, true).with_role(role));
            i += l;
            continue;
        }
        let tok = &toks[i];
        let t = tok.text.as_str();
        pending.push(t);
        i += 1;
        let next = toks
            .get(i)
            .map(|n| n.text.as_str())
            .filter(|_| !spans.iter().any(|&(s, _, _)| s == i));
        let wait = match (pending.len(), next) {
            // A short statement keeps its small words; they ride with the
            // word they lead ("IN THE RED", "OUT OF TIME").
            (_, Some(_)) if as_written && !tok.end && is_stop(t) => true,
            (1, Some(n)) if !tok.end => {
                is_stop(t)
                    || (is_numberish(t) && !is_stop(n))
                    || (syllables(t) < 3 && syllables(t) + syllables(n) <= 4)
            }
            _ => false,
        };
        if !wait {
            flush(&mut pending, &mut units);
        }
    }
    flush(&mut pending, &mut units);

    // The keyword, then the primary value (the payoff) always get a unit.
    let extra = |words: &[String], role: UnitRole, units: &mut Vec<Unit>| {
        if words.is_empty() {
            return;
        }
        let key = words.iter().map(|w| norm(w)).collect::<Vec<_>>().join(" ");
        if units.iter().any(|u| u.key() == key) {
            return;
        }
        // (0.24) A value of up to three words is one unit, a longer one is
        // shown in two-word chunks. Each chunk is dropped when every content
        // word of it (or its stem: TAXED / TAX) is already on screen.
        let chunks: Vec<&[String]> = if words.len() <= 3 {
            vec![words]
        } else {
            words.chunks(2).collect()
        };
        for chunk in chunks {
            let content: Vec<String> = chunk
                .iter()
                .filter(|w| !is_stop(w))
                .map(|w| norm(w))
                .collect();
            let on_screen = |c: &String| {
                units
                    .iter()
                    .any(|u| u.key().split(' ').any(|k| same_stem(k, c)))
            };
            if !content.is_empty() && content.iter().all(on_screen) {
                continue;
            }
            let w: Vec<&str> = chunk.iter().map(String::as_str).collect();
            units.push(Unit::new(&w, true).with_role(role));
        }
    };
    if keyword_spoken {
        extra(&keyword, UnitRole::Keyword, &mut units);
    }
    extra(&primary, UnitRole::Value, &mut units);
    extra(&secondary, UnitRole::Value, &mut units);

    if units.is_empty() {
        let all: Vec<String> = toks.iter().map(|t| t.text.clone()).collect();
        if !all.is_empty() {
            let words: Vec<&str> = all.iter().take(2).map(String::as_str).collect();
            units.push(Unit::new(&words, false));
        }
    }

    // Never more than MAX_UNITS: drop the earliest plain units first.
    while units.len() > MAX_UNITS {
        match units.iter().position(|u| !u.pinned) {
            Some(i) => {
                units.remove(i);
            }
            None => {
                units.remove(0);
            }
        }
    }
    units
}

/// Drop units until the window can hold them (each at least [`UNIT_FLOOR`] s
/// and the last [`LAST_MIN`] s).
fn fit_units(units: &mut Vec<Unit>, window: f64) {
    while units.len() > 1 && LAST_MIN + UNIT_FLOOR * (units.len() - 1) as f64 > window {
        match units.iter().position(|u| !u.pinned) {
            Some(i) => {
                units.remove(i);
            }
            None => {
                units.remove(0);
            }
        }
    }
}

/// Cut time of every unit, filling `[t0, end)`: non-last units take their
/// natural length (stretched up to 1.8x when the window has room, shrunk when
/// it does not); the last unit holds from its cut to the scene end.
fn cut_times(units: &[Unit], t0: f64, end: f64) -> Vec<f64> {
    let n = units.len();
    let window = (end - t0).max(0.0);
    let naturals: Vec<f64> = units[..n.saturating_sub(1)]
        .iter()
        .map(Unit::natural)
        .collect();
    let sum: f64 = naturals.iter().sum();
    let budget = (window - LAST_MIN).max(UNIT_FLOOR * naturals.len() as f64);
    let factor = if sum <= 0.0 {
        1.0
    } else if sum > budget {
        budget / sum
    } else {
        (window * 0.55 / sum).clamp(1.0, 1.8).min(budget / sum)
    };
    let mut cuts = Vec::with_capacity(n);
    let mut at = t0;
    for d in &naturals {
        cuts.push(at);
        at += (d * factor).max(UNIT_FLOOR.min(*d));
    }
    cuts.push(at);
    cuts
}

/// Layout per unit: A, B, C by index; with a picture the last C unit (or the
/// last unit when there is no C) shows it, other Cs are banded. (0.22) `on`
/// names the unit that must show the picture (the pictured object's figure).
#[cfg(test)]
fn layouts(n: usize, picture: bool, on: Option<usize>) -> Vec<Layout> {
    layouts_from(n, picture, on, 0)
}

/// [`layouts`] with the A, B, C cycle starting at `start % 3` (0.23: under a
/// direction seed each beat starts its cycle at
/// `choice(seed, beat, Dim::SlamLayout) % 3` instead of always at A).
fn layouts_from(n: usize, picture: bool, on: Option<usize>, start: usize) -> Vec<Layout> {
    let at = |k: usize| (k + start) % 3;
    let mut v: Vec<Layout> = (0..n)
        .map(|k| match at(k) {
            0 => Layout::Giant,
            1 => Layout::Inverse,
            _ => Layout::Banded,
        })
        .collect();
    if picture && n > 0 {
        let pic = on
            .filter(|&k| k < n)
            .or_else(|| (0..n).rev().find(|&k| at(k) == 2))
            .unwrap_or(n - 1);
        v[pic] = Layout::Picture;
    }
    v
}

/// (0.24 Wave 1) A picture arrives as a support to the beat's lead: it
/// slides `slide_px` from the side this beat opens toward (alternating by
/// beat) after a short counter-move, `role_lag` behind its cue, on the
/// look's support spring (`choreography::plan`).
fn arrive(b: &mut B, id: &str, at: f64, sig: &MotionSignature, u: f32, salt: usize) {
    b.motions.retain(|m| {
        !(m.target == id && matches!(m.op, MotionOp::Fade { .. } | MotionOp::Move { .. }))
    });
    let life = b.plan.life;
    let phrase = choreography::Phrase::new(b.plan.index, life, &[]);
    let cue = choreography::Cue {
        role: choreography::Role::Support,
        at,
        side: signature::slide_side(salt),
        stress: None,
    };
    let mut plan = choreography::plan(sig, &phrase, &cue);
    plan.dur = plan.dur.min(sig.slide_secs + 0.1);
    b.motions.extend(mo::arrive(id, &plan, u));
}

/// The beat's picture: a delivered image, else the object subject.
enum Picture<'a> {
    Delivered(&'a ManifestEntry, AssetRole),
    Object(Which, Subject),
}

fn delivered_picture<'a>(ctx: &Ctx<'a>, index: usize) -> Option<(&'a ManifestEntry, AssetRole)> {
    [
        AssetRole::HeroSubject,
        AssetRole::Portrait,
        AssetRole::HeroObject,
        AssetRole::EvidenceImage,
    ]
    .into_iter()
    .find_map(|role| plate::image(ctx, index, role).map(|e| (e, role)))
}

fn find_picture<'a>(ctx: &Ctx<'a>, b: &B) -> Option<Picture<'a>> {
    if let Some((entry, role)) = delivered_picture(ctx, b.plan.index) {
        return Some(Picture::Delivered(entry, role));
    }
    let beat = b.beat;
    if beat.primary.kind() == SubjectKind::Object {
        return Some(Picture::Object(Which::Primary, beat.primary.clone()));
    }
    beat.secondary
        .as_ref()
        .filter(|s| s.kind() == SubjectKind::Object)
        .map(|s| Picture::Object(Which::Secondary, s.clone()))
}

/// Where the picture goes: its visible box (canvas) and the layer rectangle
/// that puts it there.
struct PictureGeo {
    visible: Rect,
    /// Image rectangle for a delivered picture.
    image: Rect,
    slot: Slot,
}

fn picture_geo(ctx: &Ctx, pic: &Picture, land: bool) -> PictureGeo {
    let (w, h, m) = (ctx.w, ctx.h, ctx.margin());
    let bottom = h - 0.6 * m;
    let bh = PICTURE_HEIGHT * h;
    let target = if land {
        Rect::new(0.53 * w, bottom - bh, 0.42 * w, bh)
    } else {
        let bw = 0.9 * w;
        Rect::new((w - bw) / 2.0, bottom - bh, bw, bh)
    };
    match pic {
        Picture::Delivered(entry, _) => {
            let facts = SubjectFacts::from_entry(entry);
            let img = placement::subject_fit(&facts, target);
            let sub = placement::to_canvas(&img, facts.subject);
            let dy = bottom - (sub.y + sub.h);
            let image = Rect::new(img.x, img.y + dy, img.w, img.h);
            PictureGeo {
                visible: Rect::new(sub.x, sub.y + dy, sub.w, sub.h),
                image,
                slot: Slot {
                    cx: target.cx(),
                    cy: target.y + target.h / 2.0,
                    w: target.w,
                    h: target.h,
                },
            }
        }
        Picture::Object(..) => {
            let side = target.w.min(target.h);
            let slot = Slot {
                cx: target.cx(),
                cy: bottom - side / 2.0,
                w: side,
                h: side,
            };
            let visible = Rect::new(slot.cx - side / 2.0, bottom - side, side, side);
            PictureGeo {
                visible,
                image: visible,
                slot,
            }
        }
    }
}

/// The word set as large as the region allows, refit until even its widest
/// line fits (the layer box is 2 % wider than the text plus 2 px).
fn fit_word(ctx: &Ctx, text: &str, max_w: f32, max_h: f32, max_size: f32) -> Block {
    let mut block = ctx
        .ts
        .fit_block(Voice::HEADLINE, text, max_w, max_h, max_size, 2);
    for _ in 0..6 {
        let layer_w = block.width() * 1.02 + 2.0;
        if layer_w <= max_w + 0.5 || block.size < 24.0 * ctx.u {
            break;
        }
        let size = block.size * (max_w / layer_w) * 0.97;
        block = ctx
            .ts
            .fit_block(Voice::HEADLINE, text, max_w, max_h, size, 2);
    }
    block
}

/// A centred word layer: anchor at its middle, optically centred on `(cx, cy)`
/// when the measure knows the glyph outlines.
fn word_layer(
    ctx: &Ctx,
    id: String,
    block: &Block,
    color: crate::scene::Color,
    (cx, cy): (f32, f32),
) -> Layer {
    let mut l = text_layer(id, block, color, TextAlign::Center);
    l.anchor_x = 0.5;
    l.anchor_y = 0.5;
    let shift = ctx.ts.ink(block).map_or(0.0, |ink| {
        block.height() / 2.0 - (ink.top + ink.bottom) / 2.0
    });
    l.x = cx;
    l.y = cy + shift;
    l.z_index = 10;
    l
}

/// (0.24) The words that name the picture: its asset and what it stands for
/// (never the figure's value). `None` for a picture that is no object.
fn picture_name(picture: Option<&Picture>, beat: &Beat) -> Option<Vec<String>> {
    let obj = match picture {
        Some(Picture::Object(_, Subject::Object(o))) => Some(o),
        Some(Picture::Delivered(..)) => match &beat.primary {
            Subject::Object(o) => Some(o),
            _ => None,
        },
        _ => None,
    };
    obj.map(|o| {
        let mut v = vec![o.asset.replace(['_', '-'], " ")];
        v.extend(o.meaning.clone());
        v
    })
}

/// (0.24) The narrator's last content word of the beat is its spoken stress.
fn last_content_word(spoken: &[(String, f64)]) -> Option<String> {
    spoken
        .iter()
        .rev()
        .map(|(w, _)| shown(w))
        .find(|w| !w.is_empty() && !is_stop(w) && !is_numberish(w))
        .map(|w| norm(&w))
}

/// (0.24) A keyword slam is earned when the narrator says it in the beat, it
/// is new (no word of the title or the values), the beat is the payoff or an
/// impact beat whose spoken stress it is, and the story's stamp budget has a
/// slot for the beat. Without a voice-over: always (the old rule).
fn keyword_earned(ctx: &Ctx, beat: &Beat, index: usize) -> bool {
    let Some(keyword) = beat.keyword.as_deref() else {
        return true;
    };
    if ctx.spoken.is_empty() {
        return true;
    }
    if !super::dossier::stamp_allowed(ctx, keyword) {
        return false;
    }
    let words = words_of(keyword);
    let mut on_screen: Vec<String> = title_tokens(&beat.statement)
        .iter()
        .map(|t| norm(&t.text))
        .collect();
    on_screen.extend(value_words(&beat.primary).iter().map(|w| norm(w)));
    if let Some(s) = &beat.secondary {
        on_screen.extend(value_words(s).iter().map(|w| norm(w)));
    }
    let is_new = words
        .iter()
        .filter(|w| !is_stop(w))
        .any(|w| !on_screen.iter().any(|o| same_stem(&norm(w), o)));
    if !is_new {
        return false;
    }
    let stress = match beat.purpose {
        Purpose::Reveal => true,
        _ => {
            beat.energy == Energy::Impact
                && last_content_word(&ctx.spoken)
                    .zip(words.last().map(|w| norm(w)))
                    .is_some_and(|(last, k)| same_stem(&last, &k))
        }
    };
    stress && ctx.budget.stamp_ok(index)
}

/// A unit's lead before its word (a hard cut on the word itself for a title
/// or a stamp, [`WORD_CUE_LEAD`] before a figure).
fn lead_of(role: UnitRole) -> f64 {
    match role {
        // (Wave 2) Every cut leads its word by the rise to the word's peak.
        UnitRole::Value | UnitRole::Title | UnitRole::Keyword => WORD_CUE_LEAD,
    }
}

fn reveal_role(role: UnitRole) -> RevealRole {
    match role {
        UnitRole::Value => RevealRole::Value,
        UnitRole::Keyword => RevealRole::Stamp,
        UnitRole::Title => RevealRole::Title,
    }
}

/// (0.24) The spoken schedule of a voiced beat.
struct Schedule {
    units: Vec<Unit>,
    /// Group start (a picture unit may start before its word).
    starts: Vec<f64>,
    /// When the word itself slams in.
    words: Vec<f64>,
    /// The word was placed on its spoken onset (not pushed later by the
    /// floor between cuts, not a leftover).
    exact: Vec<bool>,
    /// The unit that shows the picture, when the beat has one.
    pic: Option<usize>,
    /// Picture start was placed on the country's name.
    pic_exact: bool,
    /// The only unit is the small title of an unspoken statement.
    small: bool,
}

/// Order the units by the voice, drop what nobody says (a title or keyword
/// whose words are never spoken), keep at most two text units, and place every
/// cut on its word. Units never come before ENTER (`t0`) or closer than
/// [`UNIT_FLOOR`] to each other.
fn spoken_schedule(
    units: Vec<Unit>,
    spoken: &[(String, f64)],
    beat: &Beat,
    picture: Option<&Picture>,
    t0: f64,
    end: f64,
) -> Schedule {
    let onset = |u: &Unit| {
        let words = u.content_words();
        // "C$300K" is said as "three hundred thousand dollars".
        let plain: Vec<String> = words
            .iter()
            .map(|w| match w.find(['$', '£', '€', '¥', '₹']) {
                Some(i) if i > 0 && w[..i].chars().all(|c| c.is_ascii_alphabetic()) => {
                    w[i..].to_string()
                }
                _ => (*w).to_string(),
            })
            .collect();
        let plain_refs: Vec<&str> = plain.iter().map(String::as_str).collect();
        anchor_time(spoken, &words)
            .or_else(|| anchor_time(spoken, &plain_refs))
            .or_else(|| {
                // "TAXED" is said as "tax", "DOUBLED" as "double".
                let stems: Vec<String> = words
                    .iter()
                    .flat_map(|w| {
                        let w = norm(w);
                        ["ing", "ed", "d", "s"]
                            .iter()
                            .filter_map(|suf| w.strip_suffix(suf).map(str::to_string))
                            .filter(|st| st.len() >= 3)
                            .collect::<Vec<_>>()
                    })
                    .collect();
                let refs: Vec<&str> = stems.iter().map(String::as_str).collect();
                if refs.is_empty() {
                    None
                } else {
                    anchor_time(spoken, &refs)
                }
            })
    };
    let mut timed: Vec<(Unit, Option<f64>)> = units
        .iter()
        .cloned()
        .map(|u| {
            let at = onset(&u);
            (u, at)
        })
        .collect();
    // Words nobody says are not slammed.
    timed.retain(|(u, at)| u.role == UnitRole::Value || at.is_some());
    // Spoken order; units nobody says (a figure the narrator never reads out)
    // follow the last spoken one.
    timed.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    // The picture belongs to the figure it is the object of.
    let pictured: Vec<String> = match picture {
        Some(Picture::Object(_, s)) => value_words(s),
        Some(Picture::Delivered(..)) if beat.primary.kind() == SubjectKind::Object => {
            value_words(&beat.primary)
        }
        _ => Vec::new(),
    };
    let key_of = |chunk: &[String]| chunk.iter().map(|w| norm(w)).collect::<Vec<_>>().join(" ");
    let figure_key = pictured.chunks(2).next().map(key_of);
    let figure_of = |timed: &[(Unit, Option<f64>)]| {
        figure_key
            .as_ref()
            .and_then(|key| timed.iter().position(|(u, _)| &u.key() == key))
    };
    let name_parts: Vec<String> = picture_name(picture, beat).unwrap_or_default();
    let pic_onset = (!name_parts.is_empty())
        .then(|| {
            let parts: Vec<&str> = name_parts.iter().map(String::as_str).collect();
            name_time(spoken, &parts)
        })
        .flatten();
    let first_cut = |timed: &[(Unit, Option<f64>)]| {
        timed
            .first()
            .and_then(|(u, at)| at.map(|a| a - lead_of(u.role)))
            .unwrap_or(t0)
    };
    // The beat opens on its picture when the picture is its first unit and its
    // name is said (or it needs no name) near ENTER.
    let pic_start = pic_onset.map_or(t0, |a| (a - WORD_CUE_LEAD).max(t0));
    let pic_first = |timed: &[(Unit, Option<f64>)]| {
        picture.is_some()
            && figure_of(timed).unwrap_or(timed.len().saturating_sub(1)) == 0
            && pic_start - t0 <= 0.6
    };
    // A statement the narrator does not say in full stands in as one small
    // title (never a slam) for a beat that would otherwise show nothing for a
    // while: it comes once the beat's first content word is spoken.
    let mut small = false;
    let first_word = spoken
        .iter()
        .find(|(w, _)| !is_stop(&shown(w)) && !shown(w).is_empty())
        .map(|(_, at)| *at);
    if timed.is_empty() {
        // Nothing of it is said: the units follow each other from ENTER.
        timed = units.iter().cloned().map(|u| (u, None)).collect();
    }
    // At most two text units: the value plus the keyword or one title.
    if timed.iter().any(|(u, _)| u.role != UnitRole::Title) {
        while timed.len() > 2 {
            match timed.iter().position(|(u, _)| u.role == UnitRole::Title) {
                Some(i) => {
                    timed.remove(i);
                }
                None => break,
            }
        }
    }
    // A beat whose first word comes late opens on its unspoken figure (the
    // payoff the narration leads up to) rather than on nothing.
    let mut late_open = !pic_first(&timed) && first_cut(&timed) - t0 > 0.6;
    if late_open {
        if let Some(i) = timed
            .iter()
            .position(|(u, at)| u.role == UnitRole::Value && at.is_none())
        {
            let v = timed.remove(i);
            timed.insert(0, v);
            late_open = false;
        }
    }
    if late_open {
        // The topic of the statement opens the beat; the words that state its
        // conclusion (a number, the keyword) wait for the narrator.
        let stated: Vec<String> = super::super::recipes::title_stated_words(beat, &beat.statement)
            .iter()
            .flat_map(|w| w.split_whitespace().map(norm).collect::<Vec<_>>())
            .collect();
        let statement: Vec<&str> = beat
            .statement
            .split_whitespace()
            .filter(|w| {
                let n = norm(w);
                !stated.iter().any(|s| same_stem(s, &n))
            })
            .collect();
        let has_content = statement.iter().any(|w| !is_stop(w));
        if has_content && statement.len() <= 8 {
            let mut u = Unit::new(&statement, false);
            u.role = UnitRole::Title;
            let at = first_word
                .map(|f| (f - WORD_CUE_LEAD).max(t0))
                .unwrap_or(t0);
            timed.insert(0, (u, Some(at)));
            small = true;
        }
    }
    let figure = figure_of(&timed);
    let pic = picture
        .is_some()
        .then(|| figure.unwrap_or(timed.len().saturating_sub(1)));
    // Raw start of each group, then the floor between cuts.
    let n = timed.len();
    let mut raw: Vec<(f64, f64, bool, bool)> = Vec::with_capacity(n);
    let mut prev_word = t0;
    for (k, (u, at)) in timed.iter().enumerate() {
        let want = at.map(|a| a - lead_of(u.role));
        let word = want
            .unwrap_or(if k == 0 { t0 } else { prev_word + UNIT_MIN })
            .max(t0);
        // (Wave 2) The cut (the ground changing) leads its word by two frames:
        // the word is the lead, the cut its support.
        let mut start = (word - CUT_LEAD).max(t0);
        let mut pic_exact = false;
        if pic == Some(k) {
            let want_pic = pic_onset.map(|a| a - WORD_CUE_LEAD);
            let at_pic = want_pic.unwrap_or(t0).max(t0).min(word);
            pic_exact = want_pic.is_some_and(|w| (w - at_pic).abs() < 0.01);
            start = at_pic;
        }
        prev_word = word;
        raw.push((
            start,
            word,
            want.is_some_and(|w| (w - word).abs() < 0.01),
            pic_exact,
        ));
    }
    let mut out = Schedule {
        units: Vec::new(),
        starts: Vec::new(),
        words: Vec::new(),
        exact: Vec::new(),
        pic: None,
        pic_exact: false,
        small,
    };
    let latest = (end - 0.2).max(t0);
    let mut prev_start: Option<f64> = None;
    for (k, ((u, _), (start, word, exact, pic_exact))) in timed.into_iter().zip(raw).enumerate() {
        let floor = prev_start.map_or(t0, |p| p + UNIT_FLOOR);
        let mut st = start.max(floor);
        if st > latest && !out.units.is_empty() {
            continue; // no room left to read it before ANTICIPATE
        }
        st = st.min(latest).max(floor.min(latest));
        // The word is the lead, the cut its support: never on the same frame.
        let w = word.max(st + CUT_LEAD).min(latest.max(st));
        if pic == Some(k) {
            out.pic = Some(out.units.len());
            out.pic_exact = pic_exact && (st - start).abs() < 0.01;
        }
        out.exact.push(exact && (w - word).abs() < CUT_LEAD + 0.01);
        out.units.push(u);
        out.starts.push(round3(st));
        out.words.push(round3(w));
        prev_start = Some(st);
    }
    out
}

pub(crate) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let sig = signature::for_look(ctx.art.as_ref().map(|(a, _)| a.look));
    let life = plan.life;
    let t0 = plan.enter_at();
    let end = life
        .anticipate
        .min(plan.duration - 0.1)
        .max(t0 + LAST_MIN + UNIT_FLOOR);
    let land = placement::side_by_side(&ctx.frame);

    let spoken = ctx.spoken.clone();
    let voiced = !spoken.is_empty();
    let keyword_spoken = if voiced {
        keyword_earned(ctx, b.beat, plan.index)
    } else {
        b.beat
            .keyword
            .as_deref()
            .is_none_or(|k| super::dossier::stamp_allowed(ctx, k))
    };
    let picture = find_picture(ctx, b);
    let mut units = punch_units_with(b.beat, keyword_spoken, !voiced);
    // (0.24) With a voice-over every cut sits on its word.
    let mut sched: Option<Schedule> = None;
    let cuts: Vec<f64> = if voiced {
        let s = spoken_schedule(units, &spoken, b.beat, picture.as_ref(), t0, end);
        units = s.units.clone();
        let starts = s.starts.clone();
        sched = Some(s);
        starts
    } else {
        fit_units(&mut units, end - t0);
        if units.is_empty() {
            return Ok(());
        }
        cut_times(&units, t0, end)
    };
    if units.is_empty() {
        return Ok(());
    }
    let n = units.len();
    // (0.22) The figure of the pictured object slams above its picture.
    let pictured_value = match &picture {
        Some(Picture::Object(_, s)) => value_words(s),
        Some(Picture::Delivered(..)) if b.beat.primary.kind() == SubjectKind::Object => {
            value_words(&b.beat.primary)
        }
        _ => Vec::new(),
    };
    let on = pictured_value.chunks(2).next().and_then(|chunk| {
        let key = chunk.iter().map(|w| norm(w)).collect::<Vec<_>>().join(" ");
        units.iter().position(|u| u.key() == key)
    });
    // The unit that shows the picture, and the unit the accent belongs to:
    // the figure (or else the primary value).
    let on = match &sched {
        Some(s) => s.pic.or(on),
        None => on,
    };
    let focal_unit = on.or_else(|| units.iter().position(|u| u.role == UnitRole::Value));
    let wipe_takes_field = plan.wipe_in
        && matches!(
            ctx.taste.transition.family,
            crate::compiler::taste::TransitionFamily::Geometric
                | crate::compiler::taste::TransitionFamily::Kinetic
        );
    let field_here = ctx.budget.field_ok(plan.index) && !wipe_takes_field;
    let layouts_for = |start: usize| -> Vec<Layout> {
        let mut v = layouts_from(n, picture.is_some(), on, start);
        if voiced {
            // (0.24) The colour belongs to the focal unit alone; the field
            // and band the budget allows are claimed by it below.
            for (k, l) in v.iter_mut().enumerate() {
                if *l != Layout::Picture {
                    *l = if Some(k) == focal_unit && !units[k].text.is_empty() {
                        if field_here {
                            Layout::Inverse
                        } else {
                            Layout::Banded
                        }
                    } else {
                        Layout::Giant
                    };
                }
            }
        }
        v
    };
    // (0.23) Under a direction seed the A / B / C cycle starts at the beat's
    // seeded layout instead of at A. The beat's kicker is set in the ink colour,
    // so where the ink does not read on the accent field (the same colour in
    // some looks) the unit on screen at READ must not be a full-bleed field:
    // the start moves on to the next layout that keeps it off one.
    let start = match ctx.direction_seed {
        Some(seed) => {
            let first =
                (direction::choice(seed, plan.index, direction::Dim::SlamLayout) % 3) as usize;
            let ink_reads_on_field =
                contrast_ratio(ctx.palette.ink, ctx.palette.accent) >= TEXT_CONTRAST_DISPLAY as f32;
            let on_screen_at_read = (0..n).rev().find(|&k| cuts[k] <= life.read).unwrap_or(0);
            let k = (0..3)
                .map(|d| (first + d) % 3)
                .find(|&s| {
                    ink_reads_on_field || layouts_for(s)[on_screen_at_read] != Layout::Inverse
                })
                .unwrap_or(first);
            let name = ["giant", "inverse", "banded"][k];
            super::note_rotation(ctx, plan.index, "slam_layout", name);
            k
        }
        None => 0,
    };
    let mut plan_layouts = layouts_for(start);
    // (0.24 M9) Accent budget: one colour field per video (the payoff beat,
    // and not under a colour wipe), one band per allowed beat. A cut that
    // loses its field or band keeps the giant word, set in the accent colour
    // where that reads on the ground.
    let mut field_left = ctx.budget.field_ok(plan.index) && !wipe_takes_field;
    // The band's slot is claimed by the first band the beat draws.
    let mut block_left: Option<bool> = None;
    let accent_word =
        contrast_ratio(ctx.palette.accent, ctx.palette.paper) >= TEXT_CONTRAST_DISPLAY as f32;
    let mut accent_words = vec![false; n];
    for (k, l) in plan_layouts.iter_mut().enumerate() {
        let keep = match l {
            Layout::Inverse => std::mem::replace(&mut field_left, false),
            Layout::Banded => {
                let ok = block_left.unwrap_or_else(|| ctx.budget.block_ok(plan.index));
                block_left = Some(false);
                ok
            }
            _ => continue,
        };
        if !keep {
            accent_words[k] = *l == Layout::Inverse && accent_word;
            *l = Layout::Giant;
        }
    }
    let geo = picture.as_ref().map(|p| picture_geo(ctx, p, land));
    let lean = if mix(plan.seed, 0x51A3).is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let tilt = lean * (1.0 + (mix(plan.seed, 0x2A) % 10) as f32 / 10.0);
    let bleed = 0.03 * w;
    let text_w = w - 2.0 * m;

    let stresses = ctx.stresses.clone();
    let mut punched = false;
    for (k, unit) in units.iter().enumerate() {
        let cut = cuts[k];
        let word_cut = sched.as_ref().map_or(cut, |s| s.words[k]);
        let small = sched.as_ref().is_some_and(|s| s.small && k == 0);
        let layout = plan_layouts[k];
        let gid = b.id(&format!("cut.{k}"));
        let mut kids: Vec<Layer> = Vec::new();

        // Region the word may use, and its colour.
        let tall_h = if land { 0.62 * h } else { 0.46 * h };
        let region = match layout {
            Layout::Giant | Layout::Inverse => Rect::new(m, 0.2 * h, text_w, tall_h),
            Layout::Banded => {
                Rect::new(m, 0.22 * h, text_w, if land { 0.56 * h } else { 0.34 * h })
            }
            Layout::Picture => match (&geo, land) {
                // Beside the picture.
                (Some(_), true) => Rect::new(m, 0.2 * h, (0.5 * w - 1.3 * m).max(0.2 * w), 0.6 * h),
                // Above the picture.
                (Some(g), false) => {
                    let top = 0.17 * h;
                    let bottom = (g.visible.y - 0.015 * h).max(top + 0.14 * h);
                    Rect::new(m, top, text_w, bottom - top)
                }
                (None, _) => Rect::new(m, 0.2 * h, text_w, 0.4 * h),
            },
        };
        let color = match layout {
            Layout::Giant if accent_words[k] => ctx.palette.accent,
            Layout::Giant | Layout::Picture => ctx.palette.ink,
            Layout::Inverse | Layout::Banded => ctx.palette.on_accent,
        };

        // Band padding comes off the word's width.
        let pad = if layout == Layout::Banded {
            0.05 * region.w
        } else {
            0.0
        };
        // The hold zoom grows the word by HOLD_ZOOM: it still fits then.
        let block = fit_word(
            ctx,
            &unit.text,
            (region.w - 2.0 * pad) / HOLD_ZOOM,
            region.h / HOLD_ZOOM,
            if small { SMALL_TITLE * u } else { 520.0 * u },
        );
        let centre = (region.x + region.w / 2.0, region.y + region.h / 2.0);

        // Background: accent field (B) or tilted band (C without a picture).
        match layout {
            Layout::Inverse => {
                let id = b.id(&format!("cut.{k}.field"));
                let mut field = base_layer(
                    id,
                    (-bleed, -bleed, w + 2.0 * bleed, h + 2.0 * bleed),
                    LayerKind::Rectangle {
                        fill: ctx.palette.accent,
                        stroke: None,
                    },
                    0,
                );
                field.z_index = 0;
                kids.push(field);
            }
            Layout::Banded => {
                let id = b.id(&format!("cut.{k}.band"));
                let (bw, bh) = (
                    block.width() * 1.02 + 2.0 + 2.0 * pad,
                    block.height() + 0.5 * block.size,
                );
                let mut band = base_layer(
                    id,
                    (centre.0, centre.1, bw, bh),
                    LayerKind::Rectangle {
                        fill: ctx.palette.accent,
                        stroke: None,
                    },
                    5,
                );
                band.anchor_x = 0.5;
                band.anchor_y = 0.5;
                band.rotation_degrees = -2.5 * lean;
                kids.push(band);
            }
            _ => {}
        }

        // The picture (layout C), behind the word.
        if layout == Layout::Picture {
            if let (Some(pic), Some(g)) = (picture.as_ref(), geo.as_ref()) {
                let enter = cut + 0.04;
                let pic_name = format!("cut.{k}.pic");
                match pic {
                    Picture::Delivered(_, role) => {
                        let r = (g.image.x, g.image.y, g.image.w, g.image.h);
                        let plated = plate::image_plate(ctx, b, &pic_name, *role, r, 5, enter);
                        let id = plated.id.clone();
                        b.motions.retain(|mo| {
                            !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. }))
                        });
                        arrive(b, &id, cut, &sig, u, plan.index + k);
                        for mut l in plated.layers {
                            l.rotation_degrees += tilt;
                            kids.push(l);
                        }
                    }
                    Picture::Object(which, subject) => {
                        if let Some(mut l) = recipes::place_subject(
                            ctx,
                            b,
                            carries,
                            *which,
                            subject,
                            g.slot,
                            (0.0, 0.0),
                            ctx.palette.ink,
                            enter,
                            Entrance::Pop,
                            None,
                            None,
                            &pic_name,
                        )? {
                            // The cut is the entrance: drop the Pop reveal and
                            // its settle, slam the picture in instead.
                            let id = l.id.clone();
                            b.motions.retain(|mo| {
                                !(mo.target == id
                                    && matches!(
                                        mo.op,
                                        MotionOp::ClipReveal { .. } | MotionOp::Scale { .. }
                                    ))
                            });
                            arrive(b, &id, cut, &sig, u, plan.index + k);
                            l.z_index = 5;
                            l.rotation_degrees += tilt;
                            // (0.23 A5a) A library picture has no contrast guard
                            // of its own, and a cut's picture sits on the bare
                            // ground: a mid-tone object on a yellow field gets
                            // the sticker a delivered picture would.
                            if let Some(entry) = recipes::object_request_id(ctx, b, subject)
                                .and_then(|request| ctx.manifest.get(&request))
                            {
                                treatment::guard_on_ground(
                                    &mut l,
                                    entry,
                                    ctx.palette.paper,
                                    &ctx.palette,
                                    ctx.u,
                                );
                            }
                            kids.push(l);
                        }
                    }
                }
            }
        }

        // The word.
        let word_id = b.id(&format!("cut.{k}.{}", if small { "small" } else { "word" }));
        let mut word = word_layer(ctx, word_id.clone(), &block, color, centre);
        // (Wave 2) A word of several lines reveals line by line.
        let line_rise = match &word.kind {
            LayerKind::Text(t) => sig.line_rise * t.font_size * t.line_height,
            _ => 0.0,
        };
        // (A stand-in small title is shown from the beat's first content word,
        // not in reading order: its lines stay one block for the speech QA.)
        let word_lines = if small {
            None
        } else {
            choreography::split_into_lines(&mut word)
        };
        // (0.24) A figure over its picture lands on its own word.
        let delayed = word_cut > cut + 0.02;
        if unit.role == UnitRole::Keyword && sched.is_some() {
            // (0.24) A slammed keyword is a stamp to the accent QA.
            kids.push(base_layer(
                b.id(&format!("cut.{k}.stamp")),
                (0.0, 0.0, w, h),
                LayerKind::Group {
                    children: vec![word],
                },
                10,
            ));
        } else {
            kids.push(word);
        }
        // (0.24 Wave 1) The hero is the lead: it lands on its word (no
        // lag), scales up through a short overshoot to rest on the look's
        // spring and fades in. No hold zoom: at rest it is still.
        {
            let life = plan.life;
            let phrase = choreography::Phrase::new(plan.index, life, &stresses);
            // (Wave 2) The beat's one stress punch: the cut whose word is the
            // spoken stress peaks on it.
            let stress = (!punched && !small && sched.as_ref().is_some_and(|s| s.exact[k]))
                .then(|| {
                    phrase
                        .stress_at(word_cut + WORD_CUE_LEAD, 0.15)
                        .map(|s| s.t)
                })
                .flatten();
            punched |= stress.is_some();
            let cue = choreography::Cue {
                role: choreography::Role::Lead,
                at: word_cut,
                side: 1.0,
                stress,
            };
            let lead = choreography::plan(&sig, &phrase, &cue);
            b.motions.extend(mo::arrive(&word_id, &lead, u));
            for i in 0..word_lines.unwrap_or(0) {
                let line = choreography::Plan {
                    start: lead.start + sig.line_stagger * i as f64,
                    dur: sig.line_secs,
                    ..lead
                };
                b.motions.extend(mo::reveal_lines(
                    &format!("{word_id}.l{i}"),
                    &line,
                    1,
                    line_rise,
                    0.0,
                ));
            }
        }

        // The unit: a group visible from its cut to the next (hard cut).
        let group = base_layer(
            gid.clone(),
            (0.0, 0.0, w, h),
            LayerKind::Group { children: kids },
            10,
        );
        b.motions
            .push(mo::fade(&gid, cut, CUT_FADE, 0.0, 1.0, Easing::Linear));
        if k + 1 < n {
            b.motions.push(mo::fade(
                &gid,
                cuts[k + 1],
                CUT_FADE,
                1.0,
                0.0,
                Easing::Linear,
            ));
        }
        b.push(group);
        // (0.24) Declare what each cut stands for so the word cues, the
        // lint and the speech QA hold it to the voice. A cut pushed off its
        // word by the floor between cuts is not declared.
        if let Some(s) = sched.as_ref().filter(|s| !(s.small && k == 0)) {
            // "C$300K" is said as "three hundred thousand dollars": the
            // figure without its currency letters is an entry too.
            let mut words: Vec<String> = Vec::new();
            for w in unit.content_words() {
                words.push(w.to_string());
                if let Some(i) = w.find(['$', '£', '€', '¥', '₹']) {
                    if i > 0 && w[..i].chars().all(|c| c.is_ascii_alphabetic()) {
                        words.push(w[i..].to_string());
                    }
                }
            }
            let role = reveal_role(unit.role);
            let group_name = format!("cut.{k}");
            if delayed {
                if s.pic == Some(k) && s.pic_exact {
                    if let Some(parts) = picture_name(picture.as_ref(), b.beat) {
                        b.reveal(&group_name, parts, RevealRole::Content);
                    }
                }
                if s.exact[k] {
                    b.reveal(&format!("{group_name}.word"), words, role);
                }
            } else if s.exact[k] {
                b.reveal(&group_name, words, role);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn beat(v: serde_json::Value) -> Beat {
        serde_json::from_value(v).expect("beat")
    }

    fn texts(units: &[Unit]) -> Vec<String> {
        units.iter().map(|u| u.text.clone()).collect()
    }

    #[test]
    fn units_isolate_the_primary_value_and_keyword_and_keep_numbers_whole() {
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "He sold 36 months of stocks.",
            "primary": {"kind": "phrase", "value": "36 months"},
            "keyword": "stocks"
        }));
        let u = punch_units(&b, true);
        assert_eq!(texts(&u), ["He sold", "36 months", "stocks"]);
        assert!(u[1].pinned && u[2].pinned && !u[0].pinned);
    }

    #[test]
    fn missing_value_and_keyword_are_appended_value_last() {
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "cash",
            "primary": {"kind": "phrase", "value": "$381 BILLION"},
            "keyword": "cash"
        }));
        assert_eq!(texts(&punch_units(&b, true)), ["cash", "$381 BILLION"]);
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "Nothing about it",
            "primary": {"kind": "phrase", "value": "big idea"},
            "keyword": "waiting"
        }));
        let t = texts(&punch_units(&b, true));
        assert_eq!(t.last().map(String::as_str), Some("big idea"));
        assert!(t.contains(&"waiting".to_string()));
    }

    #[test]
    fn an_object_value_is_a_unit_and_signs_stay_on_figures() {
        let b = beat(json!({
            "purpose": "compare",
            "statement": "US vs Germany",
            "primary": {"kind": "object", "asset": "flag_us", "value": "$58"},
            "secondary": {"kind": "object", "asset": "flag_germany", "value": "+38%"},
            "keyword": "price"
        }));
        assert_eq!(
            texts(&punch_units(&b, true)),
            ["US vs", "Germany", "price", "$58", "+38%"]
        );
        // With a voice-over that never says the keyword, it gets no unit.
        assert_eq!(
            texts(&punch_units(&b, false)),
            ["US vs", "Germany", "$58", "+38%"]
        );
        assert_eq!(shown("−27%,"), "−27%");
        assert_eq!(shown("(-4)"), "-4");
        assert_eq!(shown("-well-"), "well");
    }

    #[test]
    fn the_pictured_figure_takes_the_picture_layout() {
        use Layout::*;
        assert_eq!(layouts(4, true, Some(3)), [Giant, Inverse, Banded, Picture]);
        assert_eq!(layouts(4, true, Some(1)), [Giant, Picture, Banded, Giant]);
        assert_eq!(layouts(4, false, Some(1)), [Giant, Inverse, Banded, Giant]);
    }

    #[test]
    fn units_are_one_or_two_words_and_capped() {
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu",
            "primary": {"kind": "phrase", "value": "alpha"}
        }));
        let u = punch_units(&b, true);
        assert!(u.len() <= MAX_UNITS);
        assert!(u
            .iter()
            .all(|u| (1..=2).contains(&u.text.split_whitespace().count())));
    }

    #[test]
    fn cuts_fill_the_window_and_the_last_unit_keeps_its_hold() {
        let units: Vec<Unit> = ["one", "two words", "three", "four"]
            .iter()
            .map(|t| Unit::new(&t.split(' ').collect::<Vec<_>>(), false))
            .collect();
        let cuts = cut_times(&units, 0.25, 4.5);
        assert_eq!(cuts.len(), units.len());
        assert_eq!(cuts[0], 0.25);
        assert!(cuts.windows(2).all(|p| p[1] > p[0]));
        // The last unit (cuts[n-1]) has at least LAST_MIN left before `end`.
        assert!(4.5 - cuts[units.len() - 1] >= LAST_MIN - 1e-9);
    }

    #[test]
    fn the_cycle_can_start_at_any_layout_and_a_picture_takes_the_last_c_of_it() {
        use Layout::*;
        assert_eq!(layouts_from(4, false, None, 0), layouts(4, false, None));
        assert_eq!(
            layouts_from(4, false, None, 1),
            [Inverse, Banded, Giant, Inverse]
        );
        assert_eq!(
            layouts_from(4, false, None, 2),
            [Banded, Giant, Inverse, Banded]
        );
        // The last C of the shifted cycle shows the picture; the figure's
        // own unit still wins.
        assert_eq!(
            layouts_from(4, true, None, 1),
            [Inverse, Picture, Giant, Inverse]
        );
        assert_eq!(
            layouts_from(4, true, Some(3), 2),
            [Banded, Giant, Inverse, Picture]
        );
        assert_eq!(layouts_from(1, true, None, 2), [Picture]);
    }

    fn said(words: &[(&str, f64)]) -> Vec<(String, f64)> {
        words.iter().map(|(w, t)| ((*w).to_string(), *t)).collect()
    }

    #[test]
    fn a_title_with_the_keywords_stem_keeps_the_title_only() {
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "Taxed",
            "primary": {"kind": "phrase", "value": "200K"},
            "keyword": "tax"
        }));
        assert_eq!(texts(&punch_units(&b, true)), ["Taxed", "200K"]);
        // A value that says the keyword again is one unit, not two.
        let b = beat(json!({
            "purpose": "explain",
            "statement": "Your turn",
            "primary": {"kind": "phrase", "value": "Drop your flag"},
            "keyword": "flag"
        }));
        assert_eq!(
            texts(&punch_units_with(&b, false, false)),
            ["Your turn", "Drop your flag"]
        );
    }

    #[test]
    fn cuts_follow_the_voice_and_unspoken_words_are_not_slammed() {
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "Taxed",
            "primary": {"kind": "phrase", "value": "$200K"},
            "keyword": "tax"
        }));
        let units = punch_units_with(&b, false, false);
        let spoken = said(&[
            ("In", 0.1),
            ("the", 0.2),
            ("UK", 0.3),
            ("two", 1.0),
            ("hundred", 1.2),
            ("thousand", 1.4),
            ("goes", 2.0),
            ("to", 2.3),
            ("tax", 2.5),
        ]);
        let s = spoken_schedule(units, &spoken, &b, None, 0.2, 5.0);
        assert_eq!(texts(&s.units), ["$200K", "Taxed"]);
        // Every word comes WORD_CUE_LEAD before its spoken onset (it peaks on
        // it); the cut leads the word by CUT_LEAD.
        assert!(
            (s.words[0] - (1.0 - WORD_CUE_LEAD)).abs() < 0.01,
            "{:?}",
            s.words
        );
        assert!(
            (s.words[1] - (2.5 - WORD_CUE_LEAD)).abs() < 0.01,
            "{:?}",
            s.words
        );
        assert!(
            (s.starts[1] - (s.words[1] - CUT_LEAD)).abs() < 0.01,
            "{:?}",
            s.starts
        );
        assert!(s.exact.iter().all(|e| *e));
        // A statement nobody says is not slammed.
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "Closer",
            "primary": {"kind": "phrase", "value": "$300K"}
        }));
        let units = punch_units_with(&b, false, false);
        let spoken = said(&[
            ("about", 0.2),
            ("three", 0.3),
            ("hundred", 0.5),
            ("thousand", 0.7),
        ]);
        let s = spoken_schedule(units, &spoken, &b, None, 0.2, 5.0);
        assert_eq!(texts(&s.units), ["$300K"]);
        assert!(!s.small);
    }

    #[test]
    fn a_late_first_word_opens_on_a_small_title_or_the_unspoken_figure() {
        // The statement is said late and a figure is never read out: the
        // figure opens the beat, the title follows on its word.
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "Top 1%",
            "primary": {"kind": "number", "value": "$731,000"}
        }));
        let units = punch_units_with(&b, false, false);
        let spoken = said(&[
            ("This", 0.1),
            ("is", 0.3),
            ("the", 0.5),
            ("top", 1.7),
            ("one", 2.1),
            ("percent", 2.3),
        ]);
        let s = spoken_schedule(units, &spoken, &b, None, 0.25, 5.0);
        assert_eq!(texts(&s.units), ["$731,000", "Top 1%"]);
        assert!((s.starts[0] - 0.25).abs() < 0.01);
        assert!((s.words[1] - (1.7 - WORD_CUE_LEAD)).abs() < 0.01);
        // No figure to open on: the statement is set small at ENTER.
        let b = beat(json!({
            "purpose": "emphasize",
            "statement": "Your turn",
            "primary": {"kind": "phrase", "value": "Drop your flag"}
        }));
        let units = punch_units_with(&b, false, false);
        let spoken = said(&[
            ("So", 0.1),
            ("which", 0.4),
            ("is", 0.6),
            ("easiest", 0.9),
            ("drop", 4.0),
            ("your", 4.2),
            ("flag", 4.4),
        ]);
        let s = spoken_schedule(units, &spoken, &b, None, 0.25, 8.0);
        assert!(s.small);
        assert_eq!(texts(&s.units), ["Your turn", "Drop your flag"]);
        assert!(
            s.starts[0] < 0.6 && (s.words[1] - (4.0 - WORD_CUE_LEAD)).abs() < 0.01,
            "{:?}",
            s.starts
        );
    }

    #[test]
    fn layouts_rotate_and_a_picture_takes_the_last_c() {
        use Layout::*;
        assert_eq!(layouts(4, false, None), [Giant, Inverse, Banded, Giant]);
        assert_eq!(layouts(2, true, None), [Giant, Picture]);
        assert_eq!(layouts(1, true, None), [Picture]);
        assert_eq!(
            layouts(6, true, None),
            [Giant, Inverse, Banded, Giant, Inverse, Picture]
        );
    }
}
