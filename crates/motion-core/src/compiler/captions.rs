//! (0.10) Word-synced kinetic captions for voice-led reels (docs/VOICE.md).
//!
//! One whole-piece scene (no lifecycle, drawn above every beat scene) whose
//! words appear on their spoken start times. Emphasis treatment follows the
//! emotion the taste evokes (curated table [`caption_treatment`]). Captions live
//! inside the LayoutFrame safe area, ≤ 2 lines, ≤ 32 chars per line, and
//! never over a delivered subject's head region.
//!
//! FROZEN: `caption_scene` signature, `CaptionTreatment`,
//! `caption_treatment`.
//!
//! # Rules
//! * **Pages.** Each sentence's words are grouped into pages: greedy line
//!   breaking measured with the compiler's measure, <= [`MAX_LINES`] lines,
//!   <= [`MAX_CHARS_PER_LINE`] chars and <= the safe width per line. A page is
//!   shown from its first word's start until the next page's first word start
//!   (last page of a sentence: sentence end + [`HOLD_SECONDS`], never past the
//!   next sentence's first word nor the beat scene end). Pages never cross
//!   sentences. A page is a group layer `cap.p{n}`; each word is a text layer
//!   `cap.w{i}` (`i` = index in `SpeechMap::words`) that fades in over
//!   [`APPEAR_SECONDS`] starting exactly at its word start (plus a rise of at
//!   most 6u); the page fades out over [`EXIT_SECONDS`] ending at the page end.
//!   Times are absolute (the caption scene starts at 0, so `fade.start == word.start`).
//! * **Size.** `max(min_type_px * 1.9, 44u)` (44u on every canvas with the
//!   20u floor); emphasis faces are scaled around that (pill 0.8, serif 1.08,
//!   handwritten 1.2); a page shrinks (never below `min_type_px`) only when its
//!   widest line would not fit the safe width. SingleWord: 1.6x, one word per page.
//! * **Lane.** Lower lane = the bottom of `frame.regions.caption`, above the
//!   bottom furniture lane, bottom-anchored (a one-line page sits on the last
//!   line), centred horizontally. When the beat's delivered subject image
//!   (placed like `subject_fit` into `regions.subject`) has its head region on
//!   the lower lane, the upper lane (just under the top furniture lane) is used
//!   unless the head is there too.
//! * **Backing.** Only when the chosen lane overlaps the delivered subject's
//!   bounds, or the palette has graphic colour fields (they can sit under the
//!   lane): a soft paper band at 0.85 opacity behind the page.
//! * **Emphasis.** Numbers / currency / percent tokens, words of the beat's
//!   `primary.value` (matched with `normalize_word`) and words in double
//!   quotes; set per [`CaptionTreatment`]. Pill / highlight text colour is
//!   chosen for >= 4.5:1 contrast on the accent.

use super::explore::contrast_ratio;
use super::grammar::placement::{self, SubjectFacts};
use super::grammar::plate;
use super::layout_frame::LayoutFrame;
use super::theme::Palette;
use super::typeset::{text_layer, Block, Typesetter, Voice};
use super::typography::{resolve_emotion, Emotion};
use super::{base_layer, mo, rect_layer, BeatPlan, CompileError, Ctx};
use crate::easing::Easing;
use crate::intent::Beat;
use crate::scene::{Color, InkBounds, Layer, LayerKind, Motion, Scene, TextAlign};
use crate::speech::{normalize_word, SpeechMap};

/// Scene id of the caption scene.
pub const CAPTION_SCENE_ID: &str = "captions";
/// Line budget.
pub const MAX_LINES: usize = 2;
pub const MAX_CHARS_PER_LINE: usize = 32;

/// How emphasis words (numbers, the beat's primary value, quoted words) are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionTreatment {
    /// trust · calm · warmth: emphasis words in the serif/voice italic face.
    SerifItalic,
    /// precision · urgency: emphasis words as label pills (mono/label face on an accent pill).
    LabelPill,
    /// joy · energy · playful_retro: emphasis words on an accent highlight field.
    Highlight,
    /// drama · luxury: one word at a time, large, centred in the caption lane.
    SingleWord,
    /// handmade: emphasis words in the handwritten voice face.
    Handwritten,
}

/// Curated table: emotion → caption treatment.
pub fn caption_treatment(emotion: Emotion) -> CaptionTreatment {
    use Emotion as E;
    match emotion {
        E::Trust | E::Calm | E::Warmth => CaptionTreatment::SerifItalic,
        E::Precision | E::Urgency => CaptionTreatment::LabelPill,
        E::Joy | E::Energy | E::PlayfulRetro => CaptionTreatment::Highlight,
        E::Drama | E::Luxury => CaptionTreatment::SingleWord,
        E::Handmade => CaptionTreatment::Handwritten,
    }
}

/// Prefix of every caption layer id (pages `cap.p{n}`, words `cap.w{i}`,
/// their backings `<id>.bg`). Layout QA and `caption_report` key on it.
pub const CAPTION_LAYER_PREFIX: &str = "cap.";
/// A word fades in over this long, starting at its word start.
pub const APPEAR_SECONDS: f64 = 0.12;
/// A page fades out over this long, ending at the page end.
pub const EXIT_SECONDS: f64 = 0.15;
/// A sentence's last page is held this long after the sentence ends.
pub const HOLD_SECONDS: f64 = 0.35;

/// Caption body size in `u`: `max(min_type_px * 1.9, 44u)`.
const SIZE_U: f32 = 52.0;
const SIZE_FLOOR_X: f32 = 1.9;
const SINGLE_WORD_SCALE: f32 = 1.6;
/// Line pitch as a multiple of the caption size.
const PITCH: f32 = 1.45;
/// Gap between a lane and the furniture lane next to it (u).
const LANE_GAP_U: f32 = 8.0;
/// Word rise while fading in (u).
const RISE_U: f32 = 6.0;
/// Backing band alpha (0.85 opacity at most).
const BACKING_ALPHA: u8 = 0xD8;
/// Above every beat layer (beat z stays below 100).
const Z_PAGE: i32 = 1000;

/// Id of the text layer of speech word `index`.
pub fn word_layer_id(index: usize) -> String {
    format!("{CAPTION_LAYER_PREFIX}w{index}")
}

/// Longest a word stays highlighted when no next word follows on its page.
pub const ACTIVE_MAX_SECONDS: f64 = 0.9;
/// Fade-out of the active highlight.
const ACTIVE_OUT_SECONDS: f64 = 0.08;

/// Caption body size (px) on this canvas.
pub fn caption_size(frame: &LayoutFrame) -> f32 {
    (frame.min_type_px * SIZE_FLOOR_X).max(SIZE_U * frame.u)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    Lower,
    Upper,
}

/// One spoken word handed to the builder.
#[derive(Debug, Clone)]
pub(crate) struct WordIn {
    /// Index in `SpeechMap::words` (names the layer).
    pub index: usize,
    pub text: String,
    pub start: f64,
}

/// One sentence handed to the builder (everything already resolved).
#[derive(Debug, Clone)]
pub(crate) struct SentenceIn {
    pub words: Vec<WordIn>,
    pub statement: String,
    pub primary_value: Option<String>,
    /// Last page end (already clamped to the next sentence and the scene).
    pub end: f64,
    pub lane: Lane,
    pub backing: bool,
}

/// Everything the builder needs from the compile context.
pub(crate) struct CaptionEnv<'a> {
    pub frame: &'a LayoutFrame,
    pub palette: &'a Palette,
    pub treatment: CaptionTreatment,
    /// Advance width of (already prepared) `text` in `voice` at font size 1.
    pub width: &'a dyn Fn(&Voice, &str) -> f32,
    /// Measured glyph ink of a block (pixels from the box top); `None` without
    /// glyph outlines (pills then centre on the text box).
    pub ink: &'a dyn Fn(&Block) -> Option<InkBounds>,
}

/// The treatment for this compile. Single seam: the resolved taste's emotion.
/// (`CompileOptions.emotion` is not visible on `Ctx`; see the task report.)
fn treatment_for(ctx: &Ctx) -> CaptionTreatment {
    caption_treatment(ctx.emotion.unwrap_or_else(|| resolve_emotion(&ctx.taste)))
}

/// Build the caption scene. FROZEN signature.
pub(crate) fn caption_scene(
    ctx: &mut Ctx,
    plans: &[BeatPlan],
    beats: &[Beat],
    speech: &SpeechMap,
    beat_scenes: &[Scene],
) -> Result<Scene, CompileError> {
    let ctx = &*ctx;
    let scene_end = plans
        .last()
        .map(|p| p.start + p.duration)
        .unwrap_or(speech.duration);
    let mut sentences = Vec::new();
    for (si, sentence) in speech.sentences.iter().enumerate() {
        let (Some(plan), Some(beat)) = (plans.get(sentence.beat), beats.get(sentence.beat)) else {
            continue;
        };
        let words: Vec<WordIn> = speech
            .words_in(sentence)
            .into_iter()
            .filter_map(|w| {
                let index = speech.words.iter().position(|x| std::ptr::eq(x, w))?;
                Some(WordIn {
                    index,
                    text: w.text.clone(),
                    start: w.start,
                })
            })
            .collect();
        if words.is_empty() {
            continue;
        }
        let next_first = speech.sentences.get(si + 1).map(|n| {
            speech
                .words_in(n)
                .first()
                .map(|w| w.start)
                .unwrap_or(n.start)
        });
        let limit = (plan.start + plan.duration).min(scene_end);
        let end = (sentence.end + HOLD_SECONDS)
            .min(next_first.unwrap_or(f64::MAX))
            .min(limit);
        let (lane, over_image) = lane_for(ctx, plan.index);
        // Graphic palettes lay large colour fields (dark or saturated) under
        // the caption lane: ink on them would not read, so those get a band too.
        // Beat content already sitting in the lane (cards, stamps, lines)
        // also gets a band so captions never print over it unreadably.
        let backing = over_image
            || !ctx.palette.fields.is_empty()
            || beat_scenes
                .get(plan.index)
                .is_some_and(|sc| lane_is_occupied(ctx, sc, lane));
        sentences.push(SentenceIn {
            words,
            // Captions show what is said: narration, else the statement.
            statement: super::taste_rules::display_text(beat, true).spoken,
            primary_value: beat.primary.value().map(str::to_string),
            end,
            lane,
            backing,
        });
    }
    let width = |v: &Voice, t: &str| ctx.ts.width(v, t, 1.0);
    let ink = |b: &Block| ctx.ts.ink(b);
    let env = CaptionEnv {
        frame: &ctx.frame,
        palette: &ctx.palette,
        treatment: treatment_for(ctx),
        width: &width,
        ink: &ink,
    };
    let (layers, motions) = build_layers(&env, &sentences, scene_end);
    Ok(Scene {
        post: Vec::new(),
        id: CAPTION_SCENE_ID.to_string(),
        start_seconds: 0.0,
        duration_seconds: scene_end,
        layers,
        motions,
        camera: None,
        lifecycle: None,
    })
}

// ---------------------------------------------------------------------------
// Lane choice
// ---------------------------------------------------------------------------

/// Lane rectangle (`height` tall) on the canvas.
fn lane_rect(frame: &LayoutFrame, lane: Lane, height: f32) -> placement::Rect {
    let top = match lane {
        Lane::Lower => lane_bottom(frame) - height,
        Lane::Upper => lane_top(frame),
    };
    placement::Rect::new(frame.safe.x, top, frame.safe.w, height)
}

/// Bottom edge of the lower lane: above the bottom furniture lane.
fn lane_bottom(frame: &LayoutFrame) -> f32 {
    frame.regions.furniture_bottom.y - LANE_GAP_U * frame.u
}

/// Top edge of the upper lane: under the top furniture lane.
fn lane_top(frame: &LayoutFrame) -> f32 {
    frame.regions.furniture_top.y + frame.regions.furniture_top.h + LANE_GAP_U * frame.u
}

/// (0.10 Q) Space (u) kept between beat content and the lower caption lane.
pub const CONTENT_GAP_U: f32 = 28.0;

/// (0.10 Q) Top edge (canvas y) of the lower caption lane: [`MAX_LINES`] caption
/// lines above the bottom furniture lane. With a voice-over, beat content stays
/// above this edge (minus [`CONTENT_GAP_U`]); see [`beat_bottom_reserve`].
pub fn lower_lane_top(frame: &LayoutFrame) -> f32 {
    lane_rect(
        frame,
        Lane::Lower,
        caption_size(frame) * PITCH * MAX_LINES as f32,
    )
    .y
}

/// (0.10 Q) Lowest canvas y beat content may reach with a voice-over: the lane
/// top minus [`CONTENT_GAP_U`].
pub fn content_limit(frame: &LayoutFrame) -> f32 {
    lower_lane_top(frame) - CONTENT_GAP_U * frame.u
}

/// (0.10 Q) Pixels to reserve at the bottom of the real canvas
/// ([`LayoutFrame::with_bottom_reserve`]) so that the frame builders lay out
/// on has its safe bottom edge `CONTENT_GAP_U` above the lower caption lane.
pub fn beat_bottom_reserve(frame: &LayoutFrame) -> f32 {
    (frame.h - (content_limit(frame) + frame.safe.y)).max(0.0)
}

/// Which lane a beat's captions use, and whether they need a legibility backing.
fn lane_for(ctx: &Ctx, beat: usize) -> (Lane, bool) {
    let Some(entry) = plate::subject_image(ctx, beat) else {
        return (Lane::Lower, false);
    };
    let facts = SubjectFacts::from_entry(entry);
    let r = ctx.frame.regions.subject;
    let img = placement::subject_fit(&facts, placement::Rect::new(r.x, r.y, r.w, r.h));
    let head = placement::to_canvas(&img, facts.head);
    let body = placement::to_canvas(&img, facts.subject);
    let h = caption_size(&ctx.frame) * PITCH * MAX_LINES as f32;
    let (lower, upper) = (
        lane_rect(&ctx.frame, Lane::Lower, h),
        lane_rect(&ctx.frame, Lane::Upper, h),
    );
    let lane = if head.intersect(&lower).is_some() && head.intersect(&upper).is_none() {
        Lane::Upper
    } else {
        Lane::Lower
    };
    let chosen = if lane == Lane::Upper { upper } else { lower };
    (lane, body.intersect(&chosen).is_some())
}

/// True when a beat layer's static box (top level, not a full-canvas ground,
/// not a texture/ghost word) crosses the caption lane.
fn lane_is_occupied(ctx: &Ctx, scene: &Scene, lane: Lane) -> bool {
    use crate::scene::LayerKind;
    let h = caption_size(&ctx.frame) * PITCH * MAX_LINES as f32;
    let r = lane_rect(&ctx.frame, lane, h);
    let canvas = ctx.frame.w * ctx.frame.h;
    scene.layers.iter().any(|l| {
        if matches!(l.kind, LayerKind::Texture(_)) || l.id.contains("ghost") {
            return false;
        }
        let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
        if w * h >= 0.6 * canvas || l.opacity < 0.2 {
            return false;
        }
        let x0 = l.x - l.anchor_x * w;
        let y0 = l.y - l.anchor_y * h;
        x0 < r.x + r.w && x0 + w > r.x && y0 < r.y + r.h && y0 + h > r.y
    })
}

// ---------------------------------------------------------------------------
// Emphasis
// ---------------------------------------------------------------------------

/// Normalised words of `statement` inside double quotes.
fn quoted_words(statement: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_quote = false;
    for t in statement.split_whitespace() {
        let opens = t.starts_with(['"', '“', '‘']);
        let closes = t.ends_with(['"', '”', '’']);
        let was = in_quote;
        if !was && opens {
            in_quote = true;
        }
        if in_quote {
            let n = normalize_word(t);
            if !n.is_empty() {
                out.push(n);
            }
        }
        if closes && in_quote && !(opens && !was && t.chars().count() == 1) {
            in_quote = false;
        }
    }
    out
}

fn is_number_like(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit())
        || word.ends_with('%')
        || word.starts_with(['₹', '$', '€', '£'])
}

/// Function words never take emphasis from the primary value ("the water we
/// keep" stresses water/keep, not the/we).
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "of", "to", "in", "on", "at", "for", "by", "with",
    "from", "as", "is", "are", "was", "were", "be", "it", "its", "we", "you", "your", "our",
    "they", "their", "i", "my", "he", "she", "his", "her", "this", "that", "these", "those",
    "than", "then", "so", "not", "no", "do", "does",
];

/// At most this many emphasised words per spoken sentence: emphasis is a
/// spotlight, not a highlighter pen.
pub const MAX_EMPHASIS: usize = 2;

/// Which words of a sentence get the emotion treatment. Priority: numbers /
/// currency / percent, then quoted words, then the beat's primary value
/// (content words only, longest first). Each word form is emphasised at its
/// first occurrence only, and never more than [`MAX_EMPHASIS`] per sentence.
fn emphasis_flags(s: &SentenceIn) -> Vec<bool> {
    let quoted = quoted_words(&s.statement);
    let mut primary: Vec<String> = s
        .primary_value
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .map(normalize_word)
        .filter(|w| w.chars().count() >= 3 && !STOP_WORDS.contains(&w.as_str()))
        .collect();
    // Longest (most specific) first; ties keep phrase order (stable sort).
    primary.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    let norm: Vec<String> = s.words.iter().map(|w| normalize_word(&w.text)).collect();
    let first_index = |n: &str| norm.iter().position(|x| x == n);
    let mut picked: Vec<usize> = Vec::new();
    let take = |i: usize, picked: &mut Vec<usize>| {
        if picked.len() < MAX_EMPHASIS && !picked.contains(&i) {
            picked.push(i);
        }
    };
    for (i, w) in s.words.iter().enumerate() {
        if !norm[i].is_empty()
            && (is_number_like(&w.text) || is_number_like(&norm[i]))
            && first_index(&norm[i]) == Some(i)
        {
            take(i, &mut picked);
        }
    }
    for q in &quoted {
        if let Some(i) = first_index(q) {
            take(i, &mut picked);
        }
    }
    for p in &primary {
        if let Some(i) = first_index(p) {
            take(i, &mut picked);
        }
    }
    (0..s.words.len()).map(|i| picked.contains(&i)).collect()
}

// ---------------------------------------------------------------------------
// Tokens, colours
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bg {
    None,
    Pill,
    Highlight,
}

#[derive(Debug, Clone)]
struct Tok {
    index: usize,
    start: f64,
    text: String,
    voice: Voice,
    /// Font size as a multiple of the page size.
    scale: f32,
    bg: Bg,
    color: Color,
    /// Text width per unit page size.
    em_w: f32,
    /// Horizontal padding (each side) per unit page size.
    pad_em: f32,
    chars: usize,
}

impl Tok {
    /// Layout advance: the word only. The active highlight's padding bleeds
    /// into the neighbouring word gaps (only one word is active at a time).
    fn advance_em(&self) -> f32 {
        self.em_w
    }
}

const BLACK: Color = Color::rgb(0, 0, 0);
const WHITE: Color = Color::rgb(255, 255, 255);

/// Text colour with >= 4.5:1 contrast on `bg` (the palette's own pairs first).
fn legible_on(bg: Color, pal: &Palette) -> Color {
    let candidates = [pal.on_accent, pal.ink, pal.paper, BLACK, WHITE];
    candidates
        .into_iter()
        .find(|c| contrast_ratio(*c, bg) >= 4.5)
        .unwrap_or_else(|| {
            if contrast_ratio(BLACK, bg) >= contrast_ratio(WHITE, bg) {
                BLACK
            } else {
                WHITE
            }
        })
}

/// The accent where it reads on paper, else the ink.
fn accent_text(pal: &Palette) -> Color {
    if contrast_ratio(pal.accent, pal.paper) >= 4.5 {
        pal.accent
    } else {
        pal.ink
    }
}

fn make_token(env: &CaptionEnv, w: &WordIn, emph: bool) -> Tok {
    let pal = env.palette;
    // Keywords stand out quietly (face/colour); the moving highlight belongs
    // to the word being spoken (`bg` = the treatment's ACTIVE style).
    let (voice, scale, color) = if !emph {
        (Voice::BODY, 1.0, pal.ink)
    } else {
        match env.treatment {
            CaptionTreatment::SerifItalic | CaptionTreatment::SingleWord => {
                (Voice::SERIF, 1.08, accent_text(pal))
            }
            CaptionTreatment::Handwritten => (Voice::SERIF, 1.2, accent_text(pal)),
            CaptionTreatment::LabelPill => (Voice::LABEL, 0.9, accent_text(pal)),
            CaptionTreatment::Highlight => (Voice::SERIF, 1.08, accent_text(pal)),
        }
    };
    let (bg, pad_em) = match env.treatment {
        CaptionTreatment::LabelPill => (Bg::Pill, 0.22),
        CaptionTreatment::Highlight => (Bg::Highlight, 0.12),
        _ => (Bg::None, 0.0),
    };
    // Punctuation reads as part of the sentence, not inside a pill/field.
    let raw = w.text.as_str();
    let text = Typesetter::prepare(&voice, raw);
    let em_w = (env.width)(&voice, &text) * scale;
    Tok {
        index: w.index,
        start: w.start,
        chars: text.chars().count(),
        text,
        voice,
        scale,
        bg,
        color,
        em_w,
        pad_em,
    }
}

/// Pill corner radius as a fraction of its height (soft rectangle, not a
/// capsule: a full capsule's ends crowd the first and last letters).
const PILL_RADIUS: f32 = 0.34;

/// Vertical offset from the text box centre to the optical centre of the
/// face's cap band, at `size`. 0 without glyph outlines.
fn optical_offset(env: &CaptionEnv, voice: Voice, size: f32) -> f32 {
    let probe = Block {
        lines: vec!["Hx".to_string()],
        size,
        line_widths: vec![0.0],
        voice,
    };
    match (env.ink)(&probe) {
        Some(ink) => (ink.top + ink.bottom) / 2.0 - size * voice.line_height / 2.0,
        None => 0.0,
    }
}

fn line_em(line: &[Tok], space_em: f32) -> f32 {
    line.iter().map(Tok::advance_em).sum::<f32>() + space_em * line.len().saturating_sub(1) as f32
}

fn line_chars(line: &[Tok]) -> usize {
    line.iter().map(|t| t.chars).sum::<usize>() + line.len().saturating_sub(1)
}

type Page = Vec<Vec<Tok>>;

/// Greedy pages of <= MAX_LINES lines (lines of tokens).
/// Spoken words shown with the statement's own punctuation (commas, full
/// stops between sentences, ?, !), matched in order by normalised form; the
/// statement's final full stop is dropped (subtitle style). Words without a
/// statement match keep their spoken text.
fn with_punctuation(s: &SentenceIn) -> Vec<WordIn> {
    let tokens: Vec<&str> = s.statement.split_whitespace().collect();
    let mut cursor = 0usize;
    let n = s.words.len();
    s.words
        .iter()
        .enumerate()
        .map(|(wi, w)| {
            let target = normalize_word(&w.text);
            let found = tokens[cursor.min(tokens.len())..]
                .iter()
                .position(|t| normalize_word(t) == target)
                .map(|k| cursor + k);
            let mut text = w.text.clone();
            if let Some(ti) = found {
                cursor = ti + 1;
                let tok = tokens[ti].trim_matches(|c| matches!(c, '"' | '“' | '”' | '\''));
                if !tok.is_empty() {
                    text = tok.to_string();
                }
                if wi + 1 == n {
                    text = text.trim_end_matches('.').to_string();
                }
            }
            WordIn {
                index: w.index,
                text,
                start: w.start,
            }
        })
        .collect()
}

/// Two-line pages break where the longer line is shortest (no orphan word on
/// line two), within the char and width budgets. Single lines are unchanged.
fn balance(page: Page, size: f32, max_w: f32, space_em: f32) -> Page {
    if page.len() != 2 {
        return page;
    }
    let all: Vec<Tok> = page.into_iter().flatten().collect();
    let width = |l: &[Tok]| line_em(l, space_em) * size;
    let ok = |l: &[Tok]| line_chars(l) <= MAX_CHARS_PER_LINE && width(l) <= max_w;
    let mut best: Option<(usize, f32)> = None;
    for k in 1..all.len() {
        let (a, b) = all.split_at(k);
        if ok(a) && ok(b) {
            let worst = width(a).max(width(b));
            if best.is_none_or(|(_, w)| worst < w - 1e-3) {
                best = Some((k, worst));
            }
        }
    }
    let k = best.map(|(k, _)| k).unwrap_or(all.len() / 2).max(1);
    let mut first = all;
    let second = first.split_off(k);
    vec![first, second]
}

fn paginate(toks: Vec<Tok>, size: f32, max_w: f32, space_em: f32, single: bool) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut lines: Vec<Vec<Tok>> = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    for t in toks {
        if single {
            pages.push(vec![vec![t]]);
            continue;
        }
        let (chars, em) = if cur.is_empty() {
            (t.chars, t.advance_em())
        } else {
            (
                line_chars(&cur) + 1 + t.chars,
                line_em(&cur, space_em) + space_em + t.advance_em(),
            )
        };
        let fits = chars <= MAX_CHARS_PER_LINE && em * size <= max_w;
        if !cur.is_empty() && !fits {
            lines.push(std::mem::take(&mut cur));
            if lines.len() == MAX_LINES {
                pages.push(std::mem::take(&mut lines));
            }
        }
        cur.push(t);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if !lines.is_empty() {
        pages.push(lines);
    }
    pages
}

// ---------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------

/// Build the caption layers and motions (absolute times, scene start 0).
pub(crate) fn build_layers(
    env: &CaptionEnv,
    sentences: &[SentenceIn],
    scene_end: f64,
) -> (Vec<Layer>, Vec<Motion>) {
    let frame = env.frame;
    let (u, w, h) = (frame.u, frame.w, frame.h);
    let single = env.treatment == CaptionTreatment::SingleWord;
    let nominal = caption_size(frame) * if single { SINGLE_WORD_SCALE } else { 1.0 };
    let max_w = (frame.safe.w - 4.0 * u).max(1.0);
    // Space advance per unit size, from the body face.
    let space_em = ((env.width)(&Voice::BODY, "i i") - (env.width)(&Voice::BODY, "ii")).max(0.2);
    // Pill/field treatments pad every word for its active highlight; the pads
    // already open the gap, so the word space shrinks to keep text tight.
    // Highlights bleed `pad_em` into the gaps: keep the gap a little wider
    // than the pad so a lit word's pill never touches its neighbours.
    let space_em = match env.treatment {
        CaptionTreatment::LabelPill | CaptionTreatment::Highlight => space_em.max(0.36),
        _ => space_em,
    };
    let mut layers: Vec<Layer> = Vec::new();
    let mut motions: Vec<Motion> = Vec::new();
    let mut page_no = 0usize;

    for sentence in sentences {
        let flags = emphasis_flags(sentence);
        let limit = sentence.end.min(scene_end);
        let shown = with_punctuation(sentence);
        let toks: Vec<Tok> = shown
            .iter()
            .zip(&flags)
            // A word that starts at or after the page limit cannot be shown.
            .filter(|(wd, _)| wd.start < limit - 0.01)
            .map(|(wd, e)| make_token(env, wd, *e))
            .collect();
        let pages = paginate(toks, nominal, max_w, space_em, single)
            .into_iter()
            .map(|p| balance(p, nominal, max_w, space_em))
            .collect::<Vec<_>>();
        let firsts: Vec<f64> = pages
            .iter()
            .filter_map(|p| p.first().and_then(|l| l.first()).map(|t| t.start))
            .collect();
        for (pi, page) in pages.iter().enumerate() {
            let Some(first) = firsts.get(pi).copied() else {
                continue;
            };
            let last_start = page.iter().flatten().map(|t| t.start).fold(first, f64::max);
            let page_end = firsts
                .get(pi + 1)
                .copied()
                .unwrap_or(limit)
                .max(last_start + 0.06)
                .min(scene_end);
            let page_id = format!("{CAPTION_LAYER_PREFIX}p{page_no}");
            page_no += 1;

            // Size and geometry.
            let widest = page
                .iter()
                .map(|l| line_em(l, space_em))
                .fold(0.0_f32, f32::max)
                .max(0.01);
            let min_scale = page
                .iter()
                .flatten()
                .map(|t| t.scale)
                .fold(1.0_f32, f32::min);
            let floor = frame.min_type_px / min_scale + 0.5;
            let size = nominal.min(max_w / widest).max(floor.min(nominal));
            let pitch = size * PITCH;
            let n = page.len();
            let centre_y = |k: usize| match sentence.lane {
                // A one-word page sits in the caption region but never above
                // the reserved lane, where beat content is allowed.
                Lane::Lower if single => ((frame.regions.caption.y + lane_bottom(frame)) / 2.0)
                    .max(lower_lane_top(frame) + pitch / 2.0),
                Lane::Lower => lane_bottom(frame) - pitch / 2.0 - (n - 1 - k) as f32 * pitch,
                Lane::Upper => lane_top(frame) + pitch / 2.0 + k as f32 * pitch,
            };

            let mut children: Vec<Layer> = Vec::new();
            let (mut bx0, mut bx1) = (f32::MAX, f32::MIN);
            let appear = |start: f64| APPEAR_SECONDS.min((page_end - start).max(0.01));
            // The word being spoken is the active one: from its onset until
            // the next word's onset (the last word: until it has had
            // ACTIVE_MAX_SECONDS or the page ends). One active word at a time.
            let starts: Vec<f64> = page.iter().flatten().map(|t| t.start).collect();
            let active_end = |start: f64| {
                starts
                    .iter()
                    .copied()
                    .find(|&s| s > start + 1e-6)
                    .unwrap_or(page_end)
                    .min(start + ACTIVE_MAX_SECONDS)
                    .min(page_end)
            };
            for (k, line) in page.iter().enumerate() {
                let cy = centre_y(k);
                let lw = line_em(line, space_em) * size;
                let mut x = (w - lw) / 2.0;
                bx0 = bx0.min(x);
                bx1 = bx1.max(x + lw);
                for t in line {
                    let tsize = size * t.scale;
                    let id = format!("{CAPTION_LAYER_PREFIX}w{}", t.index);
                    let pad = t.pad_em * size;
                    let tw = t.em_w * size;
                    let off = active_end(t.start);
                    // (0.20) Measured word times put the next onset as little
                    // as 60 ms away: the rise takes at most 60 % of the active
                    // window and the fade-out starts strictly after it.
                    let window = (off - t.start).max(0.02);
                    let dur = super::round3(appear(t.start).min(0.6 * window));
                    let off_dur = super::round3(ACTIVE_OUT_SECONDS.min(window - dur).max(0.01));
                    let off_at = super::round3((off - off_dur).max(t.start + dur + 0.001));
                    if t.bg != Bg::None {
                        let bg_id = format!("{id}.bg");
                        let (bg_h, bg_w) = match t.bg {
                            Bg::Pill => (size * 1.05, tw + 2.0 * pad),
                            _ => (size * 1.18, tw + 2.0 * pad),
                        };
                        // Optical centre: the cap band (cap top → baseline) of
                        // this face, the same for every word of the line, so
                        // descenders do not pull the pill down.
                        let dy = optical_offset(env, t.voice, tsize);
                        let rect = (x - pad, cy + dy - bg_h / 2.0, bg_w, bg_h);
                        let mut bg = rect_layer(bg_id.clone(), rect, env.palette.accent, 1);
                        if t.bg == Bg::Pill {
                            bg.kind = LayerKind::RoundedRectangle {
                                fill: env.palette.accent,
                                radius: bg_h * PILL_RADIUS,
                                stroke: None,
                            };
                        }
                        motions.push(mo::fade(&bg_id, t.start, dur, 0.0, 1.0, Easing::OutCubic));
                        motions.push(mo::fade(&bg_id, off_at, off_dur, 1.0, 0.0, Easing::Linear));
                        children.push(bg);
                    }
                    let block = Block {
                        lines: vec![t.text.clone()],
                        size: tsize,
                        line_widths: vec![tw],
                        voice: t.voice,
                    };
                    let mut text = text_layer(id.clone(), &block, t.color, TextAlign::Left);
                    text.x = x;
                    text.y = cy - text.height / 2.0;
                    text.z_index = 2;
                    motions.push(mo::fade(&id, t.start, dur, 0.0, 1.0, Easing::OutCubic));
                    motions.push(mo::shift(
                        &id,
                        t.start,
                        dur,
                        [0.0, RISE_U * u],
                        [0.0, 0.0],
                        Easing::OutCubic,
                    ));
                    // Active-state copy on top: contrast-safe on the highlight,
                    // accent on plain treatments; gone when the next word starts.
                    let on_color = if t.bg == Bg::None {
                        accent_text(env.palette)
                    } else {
                        legible_on(env.palette.accent, env.palette)
                    };
                    if on_color != t.color || t.bg != Bg::None {
                        let on_id = format!("{id}.on");
                        let mut on = text.clone();
                        on.id = on_id.clone();
                        on.z_index = 3;
                        if let LayerKind::Text(style) = &mut on.kind {
                            style.color = on_color;
                        }
                        motions.push(mo::fade(&on_id, t.start, dur, 0.0, 1.0, Easing::OutCubic));
                        motions.push(mo::shift(
                            &on_id,
                            t.start,
                            dur,
                            [0.0, RISE_U * u],
                            [0.0, 0.0],
                            Easing::OutCubic,
                        ));
                        motions.push(mo::fade(&on_id, off_at, off_dur, 1.0, 0.0, Easing::Linear));
                        children.push(on);
                    }
                    children.push(text);
                    x += t.advance_em() * size + space_em * size;
                }
            }

            if sentence.backing {
                let pad = 22.0 * u;
                let top = (centre_y(0) - pitch / 2.0 - pad * 0.5).max(0.0);
                let bottom = (centre_y(n - 1) + pitch / 2.0 + pad * 0.5).min(h);
                let x0 = (bx0 - pad).max(0.0);
                let x1 = (bx1 + pad).min(w);
                let bg_id = format!("{page_id}.bg");
                let fill = env.palette.paper.with_alpha(BACKING_ALPHA);
                let mut band = rect_layer(bg_id.clone(), (x0, top, x1 - x0, bottom - top), fill, 0);
                band.kind = LayerKind::RoundedRectangle {
                    fill,
                    radius: 18.0 * u,
                    stroke: None,
                };
                motions.push(mo::fade(
                    &bg_id,
                    first,
                    appear(first),
                    0.0,
                    1.0,
                    Easing::OutCubic,
                ));
                children.insert(0, band);
            }

            let group = base_layer(
                page_id.clone(),
                (0.0, 0.0, w, h),
                LayerKind::Group { children },
                Z_PAGE,
            );
            let exit = (page_end - EXIT_SECONDS).max(first);
            motions.push(mo::fade(
                &page_id,
                exit,
                page_end - exit,
                1.0,
                0.0,
                Easing::Linear,
            ));
            layers.push(group);
        }
    }
    (layers, motions)
}

#[cfg(test)]
mod tests;
