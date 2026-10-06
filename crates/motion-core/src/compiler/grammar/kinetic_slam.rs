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
//! (55 % of the layout height). A word slams in at scale 1.8 → 1 on a spring;
//! short words also cascade glyph by glyph. Spec: docs/GENRE_GRAMMARS.md.
//! Finishing effects (grain, shake, flashes, echo, pulse) come from the FX
//! director.
//!
//! Speech words are not available inside builders, so the units come from the
//! display title (the statement on screen); a keyword, primary value and
//! secondary value (the other side of a comparison) the title does not contain
//! are appended as their own units, in that order (the value is the payoff).
//! (0.22) An object's value is a unit too (the figure of a stat card), and
//! the unit that shows the picture is the one with the pictured object's
//! value, so the figure slams above its picture. With a voice-over the
//! keyword is appended only when the narrator says it in the beat (owner
//! rule: no slammed word nobody says).
//! Units run 0.6–1.0 s by syllable weight, stretched up to 1.8x when the
//! window has room and squeezed (down to 0.45 s) when it has not. The picture
//! is shown by exactly one unit (the last C-layout one, or the last unit when
//! the beat has fewer than three) so a carried subject is placed once.

use super::placement::{self, Rect, SubjectFacts};
use super::{plate, Composition};
use crate::assets::{AssetRole, ManifestEntry};
use crate::checks::TEXT_CONTRAST_DISPLAY;
use crate::compiler::explore::contrast_ratio;
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::typeset::{text_layer, Block, Voice};
use crate::compiler::{base_layer, direction, mix, mo, treatment, Carry, CompileError, Ctx, Which};
use crate::easing::Easing;
use crate::intent::{Beat, Subject, SubjectKind};
use crate::scene::{
    GlyphOrder, GlyphPose, Layer, LayerKind, Motion, MotionOp, SpringSpec, TextAlign,
};

/// Word slam: scale 1.8 -> 1 on this spring over [`SLAM_TIME`].
const SLAM: SpringSpec = SpringSpec {
    stiffness: 520.0,
    damping: 26.0,
    mass: 1.0,
};
const SLAM_TIME: f64 = 0.22;
const SLAM_FROM: f32 = 1.8;
/// Glyph cascade of short words: stagger between glyphs, start pose.
const CASCADE_STAGGER: f64 = 0.02;
const CASCADE_FROM_SCALE: f32 = 1.4;
/// Words with at most this many glyphs also cascade.
const CASCADE_MAX_GLYPHS: usize = 8;
/// A hard cut is an opacity step of this length.
const CUT_FADE: f64 = 0.01;
/// Unit length bounds (s) and the share of the last unit that must remain.
const UNIT_MIN: f64 = 0.6;
const UNIT_MAX: f64 = 1.0;
const UNIT_FLOOR: f64 = 0.45;
const LAST_MIN: f64 = 1.0;
const MAX_UNITS: usize = 7;
/// Picture height share of the layout height (layout C).
const PICTURE_HEIGHT: f32 = 0.55;
/// Slow hold zoom of a word after its slam.
const HOLD_ZOOM: f32 = 1.05;

#[derive(Debug, Clone, PartialEq)]
struct Unit {
    /// Display text (case is the voice's business).
    text: String,
    syllables: usize,
    /// The primary value or the keyword: never merged, never dropped first.
    pinned: bool,
}

impl Unit {
    fn new(words: &[&str], pinned: bool) -> Unit {
        Unit {
            text: words.join(" "),
            syllables: words.iter().map(|w| syllables(w)).sum(),
            pinned,
        }
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

    fn glyphs(&self) -> usize {
        self.text.chars().filter(|c| !c.is_whitespace()).count()
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

fn title_tokens(text: &str) -> Vec<Tok> {
    let mut toks: Vec<Tok> = Vec::new();
    for raw in text.split_whitespace() {
        let word = shown(raw);
        let end = raw.ends_with(['.', ',', ';', ':', '!', '?', '…', '—']);
        if word.is_empty() || is_filler(&word) {
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
fn punch_units(beat: &Beat, keyword_spoken: bool) -> Vec<Unit> {
    let primary: Vec<String> = value_words(&beat.primary);
    let keyword: Vec<String> = beat.keyword.as_deref().map(words_of).unwrap_or_default();
    // The other side of a comparison: its written value is shown after the
    // primary (an object secondary's figure too; its picture is not a word).
    let secondary: Vec<String> = beat.secondary.as_ref().map(value_words).unwrap_or_default();
    let toks = title_tokens(&beat.statement);
    let keys: Vec<String> = toks.iter().map(|t| norm(&t.text)).collect();
    let find = |seq: &[String]| -> Option<usize> {
        let want: Vec<String> = seq.iter().map(|t| norm(t)).collect();
        if want.is_empty() || want.len() > keys.len() {
            return None;
        }
        (0..=keys.len() - want.len()).find(|&i| keys[i..i + want.len()] == want[..])
    };
    // Pinned spans of the title: (start, len).
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for seq in [&primary, &keyword, &secondary] {
        if let Some(i) = find(seq) {
            let overlaps = spans.iter().any(|&(s, l)| i < s + l && s < i + seq.len());
            if !overlaps {
                spans.push((i, seq.len()));
            }
        }
    }
    spans.sort_unstable();

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
        if let Some(&(s, l)) = spans.iter().find(|&&(s, _)| s == i) {
            flush(&mut pending, &mut units);
            let words: Vec<&str> = toks[s..s + l].iter().map(|t| t.text.as_str()).collect();
            units.push(Unit::new(&words, true));
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
            .filter(|_| !spans.iter().any(|&(s, _)| s == i));
        let wait = match (pending.len(), next) {
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
    let extra = |words: &[String], units: &mut Vec<Unit>| {
        if words.is_empty() {
            return;
        }
        let key = words.iter().map(|w| norm(w)).collect::<Vec<_>>().join(" ");
        if units.iter().any(|u| u.key() == key) {
            return;
        }
        // Nothing new to say: every content word is already on screen.
        let content: Vec<String> = words
            .iter()
            .filter(|w| !is_stop(w))
            .map(|w| norm(w))
            .collect();
        let on_screen = |c: &String| {
            units
                .iter()
                .any(|u| u.key().split(' ').any(|k| k == c.as_str()))
        };
        if !content.is_empty() && content.iter().all(on_screen) {
            return;
        }
        // A long value is shown in two-word chunks.
        for chunk in words.chunks(2) {
            let w: Vec<&str> = chunk.iter().map(String::as_str).collect();
            units.push(Unit::new(&w, true));
        }
    };
    if keyword_spoken {
        extra(&keyword, &mut units);
    }
    extra(&primary, &mut units);
    extra(&secondary, &mut units);

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

fn sprung(mut m: Motion, s: SpringSpec) -> Motion {
    m.spring = Some(s);
    m
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

pub(crate) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let life = plan.life;
    let t0 = plan.enter_at();
    let end = life
        .anticipate
        .min(plan.duration - 0.1)
        .max(t0 + LAST_MIN + UNIT_FLOOR);
    let land = placement::side_by_side(&ctx.frame);

    let keyword_spoken = b
        .beat
        .keyword
        .as_deref()
        .is_none_or(|k| super::dossier::stamp_allowed(ctx, k));
    let mut units = punch_units(b.beat, keyword_spoken);
    fit_units(&mut units, end - t0);
    if units.is_empty() {
        return Ok(());
    }
    let n = units.len();
    let cuts = cut_times(&units, t0, end);
    let picture = find_picture(ctx, b);
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
                    ink_reads_on_field
                        || layouts_from(n, picture.is_some(), on, s)[on_screen_at_read]
                            != Layout::Inverse
                })
                .unwrap_or(first);
            let name = ["giant", "inverse", "banded"][k];
            super::note_rotation(ctx, plan.index, "slam_layout", name);
            k
        }
        None => 0,
    };
    let plan_layouts = layouts_from(n, picture.is_some(), on, start);
    let geo = picture.as_ref().map(|p| picture_geo(ctx, p, land));
    let lean = if mix(plan.seed, 0x51A3).is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let tilt = lean * (1.0 + (mix(plan.seed, 0x2A) % 10) as f32 / 10.0);
    let bleed = 0.03 * w;
    let text_w = w - 2.0 * m;

    for (k, unit) in units.iter().enumerate() {
        let cut = cuts[k];
        let unit_end = if k + 1 < n { cuts[k + 1] } else { end };
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
            520.0 * u,
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
                        b.motions.push(sprung(
                            mo::scale(&id, cut, 0.3, 1.3, 1.0, Easing::OutQuint),
                            SLAM,
                        ));
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
                            b.motions.push(sprung(
                                mo::scale(&id, cut, 0.3, 1.3, 1.0, Easing::OutQuint),
                                SLAM,
                            ));
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
        let word_id = b.id(&format!("cut.{k}.word"));
        kids.push(word_layer(ctx, word_id.clone(), &block, color, centre));
        b.motions.push(sprung(
            mo::scale(&word_id, cut, SLAM_TIME, SLAM_FROM, 1.0, Easing::OutQuint),
            SLAM,
        ));
        if unit.glyphs() <= CASCADE_MAX_GLYPHS {
            let glyphs = unit.glyphs().max(1);
            b.motions.push(sprung(
                Motion {
                    id: None,
                    target: word_id.clone(),
                    start: cut,
                    duration: SLAM_TIME + CASCADE_STAGGER * (glyphs - 1) as f64,
                    easing: Easing::OutQuint,
                    spring: None,
                    op: MotionOp::GlyphCascade {
                        stagger: CASCADE_STAGGER,
                        from: GlyphPose {
                            dx: 0.0,
                            dy: 0.0,
                            scale: CASCADE_FROM_SCALE,
                            rotation: 0.0,
                            opacity: 0.0,
                        },
                        order: GlyphOrder::Forward,
                        seed: (plan.seed as u32) ^ (k as u32),
                    },
                },
                SLAM,
            ));
        }
        let hold_from = cut + SLAM_TIME + 0.02;
        if unit_end - hold_from > 0.25 {
            b.motions.push(mo::scale(
                &word_id,
                hold_from,
                unit_end - hold_from,
                1.0,
                HOLD_ZOOM,
                Easing::Linear,
            ));
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
