//! Policy and schema validator, auto-fix, and request preparation (plan §7).
//!
//! Runs before any work. Every problem is a [`Fix`]: the beat, the field,
//! what is wrong and the values that would work, so a model can fix it in one
//! retry. Fixable problems are fixed automatically (unless `strict`) and
//! reported in [`Prepared::changed`]:
//! * lenient parsing: unknown fields ignored, a number written as a JSON number
//!   becomes text, `"file:…"` strings become [`crate::lite::FileRef`]s,
//!   a picture written with spaces or capitals becomes snake_case, a missing
//!   `say` is taken from `show`, a `list` longer than 6 is cut …;
//! * pictures with no library match → the best suggestion
//!   ([`crate::pictures`]), else shown as text (both listed);
//! * `show` longer than 6 words → shortened;
//! * fields a profile does not offer (e.g. `options` from a weak model) are
//!   ignored with a note.
//!
//! Hard limits ([`Fix`]es even without `strict`): beat count, `say` length,
//! total narration, estimated duration, file references outside the asset
//! roots (§5a), unknown job ids.
//!
//! `changed` is ordered by importance (the reply keeps 5 lines): picture
//! swaps, then ignored or moved arguments, then structure fixes, then pictures
//! shown as text (merged across beats), then cosmetic normalisation.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

use motion_core::audio::MusicWord;
use motion_core::compiler::art_direction::{self, ArtMode, Look};
use motion_core::compiler::{resolve_taste, typography};
use motion_core::intent::{Beat, CollectionItem, Energy, Format, Subject};
use motion_core::{CreativeIntent, StyleProfile};
use serde_json::{json, Map, Value};

use crate::args::{BeatChange, MakeVideoArgs, Mode, ProductionOptions, ReviseVideoArgs};
use crate::byo::{self, CheckedFile, ImageEntry, PreparedImages, RefErrorKind, Sandbox};
use crate::job::{JobKey, StoredRequest, StoryKind};
use crate::lite::{self, FileRef, How, ImageRole, LiteBeat, LiteStory, PictureRef, Structure};
use crate::pictures::{edit_distance, PictureIndex};
use crate::profile::Profile;
use crate::reply::Fix;

pub const BEATS: RangeInclusive<usize> = 2..=12;
/// Words in one beat's `say`.
pub const SAY_WORDS: RangeInclusive<usize> = 3..=40;
/// Words of narration in the whole story.
pub const MAX_TOTAL_WORDS: usize = 300;
pub const MAX_DURATION_S: f64 = 120.0;
/// Words in `show` before it is shortened.
pub const SHOW_MAX_WORDS: usize = 6;
pub const LIST_ITEMS: RangeInclusive<usize> = 3..=6;
pub const LAYER_COUNT: RangeInclusive<usize> = 2..=6;
/// Any string field is trimmed, stripped of control characters and cut here.
pub const MAX_STRING_CHARS: usize = 400;
/// Duration estimate: spoken words per second, plus a pause per beat.
pub const WORDS_PER_SECOND: f64 = 2.5;
pub const BEAT_PAUSE_S: f64 = 0.6;

/// What a strict caller is told it may do instead of picking an option.
const LEAVE_AS_TEXT: &str = "leave it with strict off (shown as text)";

/// A story as received: lite, or a full CreativeIntent (a `version` key, or
/// beats with `purpose` / `primary`).
#[derive(Debug, Clone, PartialEq)]
pub enum Story {
    Lite(LiteStory),
    Intent(CreativeIntent),
}

/// A parsed story and what lenient parsing changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub story: Story,
    /// e.g. `beat 2: ignored unknown field 'colour'`.
    pub notes: Vec<String>,
}

/// What the checks need.
pub struct Context<'a> {
    pub config: &'a crate::profile::ServerConfig,
    pub pictures: &'a PictureIndex,
}

/// Everything a job needs, after checks and auto-fixes. Deterministic: the
/// same arguments, library and images give the same `Prepared` (and so the
/// same job id).
#[derive(Debug, Clone)]
pub struct Prepared {
    /// What `request.json` stores (the auto-fixed story).
    pub request: StoredRequest,
    pub intent: CreativeIntent,
    pub style: StyleProfile,
    pub options: ProductionOptions,
    /// The user's images, when the story uses them (plan §5a).
    pub images: Option<PreparedImages>,
    /// Auto-fixes and lenient-parsing notes, one line each.
    pub changed: Vec<String>,
    /// Warnings that do not block (style outliers, mapping notes …).
    pub findings: Vec<String>,
    /// Check-mode plan: one line per beat (structure; pictures found or shown
    /// as text; user images with role and size) and the estimated duration.
    pub plan: Vec<String>,
    pub estimated_s: f64,
    /// The job id input ([`crate::job::job_id`]).
    pub key: JobKey,
}

// ---------------------------------------------------------------------------
// Notes: ordered by importance, similar ones merged
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prio {
    /// A picture replaced (library suggestion or the user's own image).
    Swap,
    /// An argument ignored, moved or mapped (profile gating, tone synonyms …).
    Ignored,
    /// A structure fixed (list cut, list → compare, focus, show shortened …).
    Fixed,
    /// Pictures that will be shown as text.
    Text,
    /// Cosmetic normalisation (types, spelling, unknown fields).
    Cosmetic,
}

#[derive(Debug, Clone)]
struct Group {
    key: String,
    /// The line prefix when merged, e.g. `pictures shown as text`.
    many: String,
    item: Option<String>,
}

#[derive(Debug, Clone)]
struct Note {
    prio: Prio,
    beat: Option<usize>,
    text: String,
    group: Option<Group>,
}

#[derive(Debug, Clone, Default)]
struct Notes(Vec<Note>);

impl Notes {
    fn add(&mut self, prio: Prio, beat: Option<usize>, text: impl Into<String>) {
        self.0.push(Note {
            prio,
            beat,
            text: text.into(),
            group: None,
        });
    }

    /// A note merged with others of the same `key` across beats.
    fn group(
        &mut self,
        prio: Prio,
        beat: usize,
        key: &str,
        single: impl Into<String>,
        many: &str,
        item: Option<String>,
    ) {
        self.0.push(Note {
            prio,
            beat: Some(beat),
            text: single.into(),
            group: Some(Group {
                key: key.to_string(),
                many: many.to_string(),
                item,
            }),
        });
    }

    fn lines(&self) -> Vec<String> {
        let render = |beat: Option<usize>, text: &str| match beat {
            Some(n) => format!("beat {n}: {text}"),
            None => text.to_string(),
        };
        let mut out: Vec<(Prio, usize, String)> = Vec::new();
        // key → (prio, first index, beats, items, first note's line, many)
        type Merged = (Prio, usize, BTreeSet<usize>, Vec<String>, String, String);
        let mut groups: BTreeMap<String, Merged> = BTreeMap::new();
        for (i, n) in self.0.iter().enumerate() {
            match &n.group {
                None => out.push((n.prio, i, render(n.beat, &n.text))),
                Some(g) => {
                    let e = groups.entry(g.key.clone()).or_insert_with(|| {
                        (
                            n.prio,
                            i,
                            BTreeSet::new(),
                            Vec::new(),
                            render(n.beat, &n.text),
                            g.many.clone(),
                        )
                    });
                    e.2.extend(n.beat);
                    if let Some(item) = &g.item {
                        if !e.3.contains(item) {
                            e.3.push(item.clone());
                        }
                    }
                }
            }
        }
        for (prio, first, beats, items, single, many) in groups.into_values() {
            let line = if beats.len() <= 1 && items.len() <= 1 {
                single
            } else if items.is_empty() {
                format!("{}: {many}", beats_label(&beats))
            } else {
                format!("{}: {many}: {}", beats_label(&beats), items.join(", "))
            };
            out.push((prio, first, line));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut lines: Vec<String> = Vec::new();
        for (_, _, l) in out {
            if !lines.contains(&l) {
                lines.push(l);
            }
        }
        lines
    }
}

/// `beat 2`, `beats 1, 3`, `beats 2-5`.
fn beats_label(beats: &BTreeSet<usize>) -> String {
    let v: Vec<usize> = beats.iter().copied().collect();
    if v.len() == 1 {
        return format!("beat {}", v[0]);
    }
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        if j >= i + 2 {
            parts.push(format!("{}-{}", v[i], v[j]));
        } else {
            for x in &v[i..=j] {
                parts.push(x.to_string());
            }
        }
        i = j + 1;
    }
    format!("beats {}", parts.join(", "))
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// Trimmed, control characters replaced by spaces, whitespace collapsed, at
/// most [`MAX_STRING_CHARS`] characters. Idempotent.
pub fn clean_text(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = s.chars().take(MAX_STRING_CHARS).collect();
    cut.trim_end().to_string()
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

fn number_text(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    match n.as_f64() {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
        Some(f) => format!("{f}"),
        None => n.to_string(),
    }
}

/// A scalar as text (`None` for empty strings, objects, booleans).
fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(clean_text(s)).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(number_text(n)),
        Value::Array(a) if a.iter().all(|x| x.is_string() || x.is_number()) => {
            let parts: Vec<String> = a.iter().filter_map(text_of).collect();
            Some(clean_text(&parts.join(" "))).filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

/// A list written as text: `a, b and c` → `[a, b, c]`.
fn split_list(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = s
        .split([',', ';', '\n'])
        .map(clean_text)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() >= 2 {
        if let Some(last) = parts.pop() {
            let last = last
                .strip_prefix("and ")
                .or_else(|| last.strip_prefix("& "))
                .unwrap_or(&last)
                .to_string();
            match last.split_once(" and ") {
                Some((a, b)) if !parts.is_empty() => {
                    parts.push(clean_text(a));
                    parts.push(clean_text(b));
                }
                _ => parts.push(last),
            }
        }
    }
    parts.retain(|p| !p.is_empty());
    parts
}

/// The first 1-based beat number in a message (`beat 3: …`).
fn beat_in(msg: &str) -> Option<usize> {
    let i = msg.find("beat ")?;
    let digits: String = msg[i + 5..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The lite field a validation message is about.
fn field_of(msg: &str) -> &'static str {
    if msg.contains("focus") || msg.contains("layers") {
        "layers"
    } else if msg.contains("items") {
        "list"
    } else if msg.contains("entity") || msg.contains(".from") || msg.contains(".to") {
        "change"
    } else {
        "story"
    }
}

/// `show` cut to at most [`SHOW_MAX_WORDS`] words, at clause punctuation when
/// possible, without trailing function words.
pub fn shorten_title(s: &str) -> String {
    let all: Vec<&str> = s.split_whitespace().collect();
    if all.len() <= SHOW_MAX_WORDS {
        return s.trim().to_string();
    }
    let head = &all[..SHOW_MAX_WORDS];
    let is_dash = |w: &str| w.chars().all(|c| matches!(c, '—' | '–' | '-'));
    let mut boundary: Option<usize> = None;
    for (i, w) in head.iter().enumerate() {
        if is_dash(w) && i >= 2 {
            boundary = Some(i - 1);
        } else if i >= 1 && w.ends_with([',', '.', ';', ':', '?', '!', '—']) {
            boundary = Some(i);
        }
    }
    let mut kept: Vec<&str> = match boundary {
        Some(i) => head[..=i].to_vec(),
        None => head.to_vec(),
    };
    const TRAILING: &[&str] = &[
        "a", "an", "the", "of", "and", "or", "to", "in", "on", "for", "with", "at", "by", "from",
        "is", "are", "was", "that", "which", "its", "their", "your", "our", "but", "as",
    ];
    while kept.len() > 2 {
        let last = kept[kept.len() - 1]
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if TRAILING.contains(&last.as_str()) {
            kept.pop();
        } else {
            break;
        }
    }
    kept.join(" ")
        .trim_end_matches([',', ';', ':', '—', '–', '-'])
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// Word tables (deterministic synonym maps)
// ---------------------------------------------------------------------------

fn how_word(s: &str) -> Option<How> {
    let w = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    Some(match w.as_str() {
        "separate" | "vs" | "versus" | "against" | "apart" | "split" | "divide"
        | "side_by_side" | "contrast" | "compare" | "different" | "difference" => How::Separate,
        "grow" | "grows" | "growth" | "increase" | "rise" | "rises" | "more" | "bigger"
        | "gain" | "gains" | "up" => How::Grow,
        "compress" | "squeeze" | "squeezes" | "shrink" | "shrinks" | "pressure" | "crush"
        | "smaller" | "less" | "down" => How::Compress,
        "replace" | "replaces" | "swap" | "takeover" | "take_over" | "takes_over" | "instead"
        | "becomes" | "overtake" | "overtakes" => How::Replace,
        _ => return None,
    })
}

fn energy_word(s: &str) -> Option<Energy> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "calm" | "low" | "soft" | "gentle" | "slow" | "quiet" | "relaxed" | "chill" => Energy::Calm,
        "building" | "build" | "medium" | "steady" | "normal" | "rising" | "moderate" => {
            Energy::Building
        }
        "impact" | "high" | "strong" | "punchy" | "fast" | "intense" | "big" | "loud"
        | "dramatic" | "energetic" => Energy::Impact,
        _ => return None,
    })
}

/// Tone synonyms (a weak model's style word → a StyleProfile tone).
fn tone_synonym(w: &str) -> Option<&'static str> {
    Some(match w {
        "educational" | "education" | "explainer" | "explain" | "explanatory" | "informative"
        | "news" | "journalism" | "journalistic" | "investigative" | "history" | "historical"
        | "facts" | "factual" | "vox" | "report" | "science" | "learning" | "lesson"
        | "tutorial" => "documentary",
        "fun" | "funny" | "kids" | "kid" | "cheerful" | "happy" | "lighthearted" | "cute"
        | "whimsical" | "colorful" | "colourful" | "cartoon" | "silly" | "joyful" => "playful",
        "tech" | "technology" | "data" | "analytical" | "engineering" | "scientific"
        | "precise" | "numbers" | "infographic" => "technical",
        "epic" | "movie" | "film" | "cinema" | "3d" | "space" | "futuristic" | "future"
        | "trailer" | "dramatic" => "cinematic",
        "energetic" | "exciting" | "upbeat" | "punchy" | "intense" | "sport" | "sports"
        | "action" | "viral" | "promo" | "fast" => "hype",
        "urban" | "music" | "hiphop" | "grunge" | "gritty" | "edgy" | "skate" | "collage" => {
            "street"
        }
        "brand" | "creator" | "personal" | "vlog" | "influencer" | "product" | "marketing"
        | "commercial" => "studio",
        "magazine" | "elegant" | "classic" | "sophisticated" | "professional" | "business"
        | "corporate" | "formal" | "serious" | "calm" | "minimal" | "clean" | "simple" => {
            "editorial"
        }
        "default" | "any" | "none" | "normal" | "standard" => "auto",
        _ => return None,
    })
}

/// The tone for a style word: exact, else a synonym (`None` = unknown).
fn tone_for(word: &str) -> Option<(String, bool)> {
    let tones = crate::schema::tones();
    let w = word.trim().to_ascii_lowercase();
    if tones.contains(&w) {
        return Some((w, true));
    }
    let tokens: Vec<String> = w
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    for t in &tokens {
        if tones.contains(t) {
            return Some((t.clone(), false));
        }
    }
    let joined = tokens.concat();
    for t in tokens.iter().chain(std::iter::once(&joined)) {
        if let Some(s) = tone_synonym(t) {
            return Some((s.to_string(), false));
        }
    }
    None
}

fn format_synonym(f: &str) -> Option<Format> {
    let w = f.trim().to_ascii_lowercase();
    Some(match w.as_str() {
        "portrait" | "tall" | "reel" | "reels" | "story" | "stories" | "short" | "shorts"
        | "tiktok" | "9:16" | "9x16" | "mobile" | "phone" => Format::Vertical,
        "1:1" | "1x1" | "instagram" | "insta" | "post" => Format::Square,
        "horizontal" | "16:9" | "16x9" | "youtube" | "widescreen" | "desktop" | "tv" => {
            Format::Landscape
        }
        _ => return None,
    })
}

fn format_name(f: Format) -> &'static str {
    match f {
        Format::Vertical => "vertical",
        Format::Square => "square",
        Format::Landscape => "wide",
    }
}

fn role_name(r: ImageRole) -> &'static str {
    match r {
        ImageRole::Person => "person",
        ImageRole::Object => "object",
        ImageRole::Place => "place",
    }
}

// ---------------------------------------------------------------------------
// Argument normalisation (misplaced fields)
// ---------------------------------------------------------------------------

/// Top-level `make_video` fields a model sometimes writes inside `story`.
const HOISTED: [&str; 10] = [
    "mode", "style", "tone", "format", "assets", "strict", "describe", "force", "options", "brand",
];

fn looks_like_intent(story: &Value) -> bool {
    story.get("version").is_some()
        || story["beats"].as_array().is_some_and(|beats| {
            beats.iter().any(|b| {
                b.get("primary").is_some() || (b.get("purpose").is_some() && b.get("say").is_none())
            })
        })
}

fn bool_of(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|i| i != 0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Some(true),
            "false" | "no" | "0" | "off" | "" => Some(false),
            _ => None,
        },
        Value::Null => Some(false),
        _ => None,
    }
}

fn normalize_inner(raw: &Value, notes: &mut Notes) -> MakeVideoArgs {
    let mut args = MakeVideoArgs::default();
    let mut top: Map<String, Value> = match raw {
        Value::Null => return args,
        Value::Object(m) => m.clone(),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Object(_)) => {
                notes.add(Prio::Cosmetic, None, "arguments read from JSON text");
                return normalize_inner(&v, notes);
            }
            _ => {
                args.story = raw.clone();
                return args;
            }
        },
        other => {
            args.story = other.clone();
            return args;
        }
    };
    let mut story = top.remove("story");
    if story.is_none() {
        if let Some(v) = top.remove("intent") {
            story = Some(v);
        } else if let Some(beats) = top.remove("beats") {
            notes.add(
                Prio::Ignored,
                None,
                "the beats were outside 'story'; read them as the story",
            );
            story = Some(json!({ "beats": beats }));
        }
    }
    if let Some(Value::String(s)) = &story {
        if let Ok(v @ (Value::Object(_) | Value::Array(_))) = serde_json::from_str::<Value>(s) {
            notes.add(Prio::Cosmetic, None, "story read from JSON text");
            story = Some(v);
        }
    }
    let mut story = story.unwrap_or(Value::Null);
    if let Value::Object(s) = &mut story {
        let intent = looks_like_intent(&Value::Object(s.clone()));
        if let Some(title) = top.remove("title") {
            if s.get("title").is_some_and(|t| !t.is_null()) {
                notes.add(Prio::Ignored, None, "title ignored (the story has its own)");
            } else {
                s.insert("title".into(), title);
                notes.add(Prio::Ignored, None, "title moved into the story");
            }
        }
        let mut moved: Vec<&str> = Vec::new();
        let mut dropped: Vec<&str> = Vec::new();
        for key in HOISTED {
            if key == "format" && intent {
                continue;
            }
            let Some(v) = s.remove(key) else {
                continue;
            };
            let target = if key == "tone" { "style" } else { key };
            if top.contains_key(target) {
                dropped.push(key);
            } else {
                top.insert(target.into(), v);
                moved.push(key);
            }
        }
        if !moved.is_empty() {
            let list: Vec<String> = moved.iter().map(|k| format!("'{k}'")).collect();
            notes.add(
                Prio::Ignored,
                None,
                format!(
                    "{} moved out of the story (tool arguments)",
                    list.join(", ")
                ),
            );
        }
        if !dropped.is_empty() {
            let list: Vec<String> = dropped.iter().map(|k| format!("'{k}'")).collect();
            notes.add(
                Prio::Ignored,
                None,
                format!(
                    "{} inside the story ignored (given outside it)",
                    list.join(", ")
                ),
            );
        }
    } else if top.contains_key("title") {
        top.remove("title");
        notes.add(Prio::Ignored, None, "title ignored (no story object)");
    }
    if !top.contains_key("style") {
        if let Some(t) = top.remove("tone") {
            top.insert("style".into(), t);
        }
    }
    args.story = story;
    for (key, v) in top {
        match key.as_str() {
            "style" => args.style = (!v.is_null()).then_some(v),
            // (0.21) Checked and normalised in `prepare` (`brand_of`).
            "brand" | "colors" | "colours" | "brand_colors" | "brand_colours" => {
                args.brand = (!v.is_null()).then_some(v)
            }
            // (0.23) Checked and normalised in `prepare` (`take_of`).
            "take" => args.take = (!v.is_null()).then_some(v),
            // (0.23) Checked and normalised in `prepare` (`music_of`).
            "music" => args.music = (!v.is_null()).then_some(v),
            "format" => match text_of(&v) {
                Some(f) => args.format = Some(f),
                None if v.is_null() => {}
                None => notes.add(Prio::Ignored, None, "format ignored (not a word)"),
            },
            "assets" => match &v {
                Value::String(s) if !s.trim().is_empty() => args.assets = Some(s.trim().into()),
                Value::String(_) | Value::Null => {}
                _ => notes.add(Prio::Ignored, None, "assets ignored (not a folder name)"),
            },
            "mode" => {
                let m = v.as_str().unwrap_or_default().trim().to_ascii_lowercase();
                args.mode = match m.as_str() {
                    "" | "auto" | "final" | "full" | "make" | "create" | "go" | "video" => {
                        Mode::Auto
                    }
                    "render" => Mode::Render,
                    "check" | "plan" | "preview" | "dry_run" | "dry-run" | "validate" => {
                        Mode::Check
                    }
                    other => {
                        notes.add(
                            Prio::Ignored,
                            None,
                            format!("mode '{other}' unknown, used auto"),
                        );
                        Mode::Auto
                    }
                };
            }
            "strict" | "describe" | "force" => {
                let b = bool_of(&v).unwrap_or_else(|| {
                    notes.add(
                        Prio::Ignored,
                        None,
                        format!("{key} ignored (not true/false)"),
                    );
                    false
                });
                match key.as_str() {
                    "strict" => args.strict = b,
                    "describe" => args.describe = b,
                    _ => args.force = b,
                }
            }
            "options" => {
                if v.is_null() {
                    continue;
                }
                match serde_json::from_value::<ProductionOptions>(v) {
                    Ok(o) => args.options = Some(o),
                    Err(_) => notes.add(Prio::Ignored, None, "options ignored (not valid)"),
                }
            }
            "story" => {}
            other => notes.add(
                Prio::Ignored,
                None,
                format!("ignored unknown argument '{other}'"),
            ),
        }
    }
    args
}

/// Read raw `make_video` arguments leniently: a top-level `title` moves into
/// `story`; `mode`, `style`, `format`, `assets`, `strict` (…) written inside
/// `story` move to the top level (an explicit top-level value wins); the
/// story as JSON text is parsed; unknown arguments are ignored. Returns the
/// arguments and one note per change (for `changed`; see
/// [`prepare_normalized`]).
pub fn normalize_args(raw: &Value) -> (MakeVideoArgs, Vec<String>) {
    let mut notes = Notes::default();
    let args = normalize_inner(raw, &mut notes);
    (args, notes.lines())
}

/// [`normalize_args`] + [`prepare`], with the argument notes merged into
/// `changed` in importance order.
pub fn prepare_raw(raw: &Value, ctx: &Context) -> (MakeVideoArgs, Result<Prepared, Vec<Fix>>) {
    let mut notes = Notes::default();
    let args = normalize_inner(raw, &mut notes);
    let prepared = prepare_inner(&args, ctx, None, 0, MusicWord::Auto, notes);
    (args, prepared)
}

/// [`prepare`] with notes from [`normalize_args`] (kept with the other
/// argument notes in `changed`).
pub fn prepare_normalized(
    args: &MakeVideoArgs,
    arg_notes: &[String],
    ctx: &Context,
) -> Result<Prepared, Vec<Fix>> {
    let mut notes = Notes::default();
    for n in arg_notes {
        notes.add(Prio::Ignored, None, n.clone());
    }
    prepare_inner(args, ctx, None, 0, MusicWord::Auto, notes)
}

// ---------------------------------------------------------------------------
// Lenient story parsing
// ---------------------------------------------------------------------------

const BEAT_FIELDS: [&str; 12] = [
    "say", "show", "picture", "picture2", "number", "meaning", "keyword", "list", "compare",
    "change", "layers", "energy",
];

fn beat_alias(k: &str) -> Option<&'static str> {
    Some(match k {
        "narration" | "text" | "voice" | "voiceover" | "voice_over" | "script" | "line"
        | "says" | "speech" | "spoken" | "narrator" => "say",
        "title" | "headline" | "heading" | "caption" | "on_screen" | "onscreen" | "display"
        | "statement" => "show",
        "image" | "img" | "icon" | "photo" | "pic" | "visual" | "asset" | "object" => "picture",
        "picture_2" | "image2" | "image_2" | "second_picture" | "icon2" => "picture2",
        "value" | "figure" | "stat" | "amount" | "num" | "metric" => "number",
        "label" | "unit" | "means" => "meaning",
        "word" | "background_word" | "bg_word" | "key_word" => "keyword",
        "items" | "bullets" | "points" | "bullet_points" => "list",
        "comparison" | "vs" | "versus" | "contrast" => "compare",
        "layer" | "stack" | "levels" | "zones" => "layers",
        "mood" | "pace" | "intensity" | "tempo" => "energy",
        _ => return None,
    })
}

fn norm_key(k: &str) -> String {
    k.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

/// A picture value → normalized JSON (`"noun"` or `{"file", "role"}`).
fn parse_picture(n: usize, field: &str, v: &Value, notes: &mut Notes) -> Option<Value> {
    match v {
        Value::String(s) => {
            let s = clean_text(s);
            if s.is_empty() {
                return None;
            }
            let lower = s.to_ascii_lowercase();
            if lower.starts_with("file:") {
                let path = s[5..].trim().to_string();
                return Some(json!({ "file": path }));
            }
            let snaked = lite::snake(&s);
            if snaked != s {
                notes.add(
                    Prio::Cosmetic,
                    Some(n),
                    format!("{field} '{s}' → '{snaked}'"),
                );
            }
            Some(Value::String(snaked))
        }
        Value::Object(o) => {
            let file = o
                .get("file")
                .or_else(|| o.get("path"))
                .and_then(text_of)
                .map(|f| f.strip_prefix("file:").unwrap_or(&f).trim().to_string());
            match file {
                Some(file) => {
                    let mut out = json!({ "file": file });
                    if let Some(r) = o.get("role").and_then(text_of) {
                        match byo::role_word(&r) {
                            Some(role) => out["role"] = json!(role),
                            None => notes.add(
                                Prio::Cosmetic,
                                Some(n),
                                format!(
                                    "{field} role '{r}' unknown (person, object or place), guessed"
                                ),
                            ),
                        }
                    }
                    Some(out)
                }
                None => {
                    let noun = o
                        .get("name")
                        .or_else(|| o.get("noun"))
                        .or_else(|| o.get("asset"))
                        .and_then(text_of)?;
                    parse_picture(n, field, &Value::String(noun), notes)
                }
            }
        }
        Value::Number(_) => None,
        _ => {
            notes.add(
                Prio::Cosmetic,
                Some(n),
                format!("ignored '{field}' (not a picture name)"),
            );
            None
        }
    }
}

fn list_items(v: &Value) -> Option<(Vec<String>, bool)> {
    match v {
        Value::Array(a) => Some((
            a.iter()
                .filter_map(|x| match x {
                    Value::Object(o) => o
                        .get("name")
                        .or_else(|| o.get("text"))
                        .or_else(|| o.get("item"))
                        .and_then(text_of),
                    other => text_of(other),
                })
                .collect(),
            false,
        )),
        Value::String(s) => Some((split_list(s), true)),
        _ => None,
    }
}

/// Split `a vs b` / `a versus b` / `a or b`.
fn split_versus(s: &str) -> Option<(String, String)> {
    let lower = s.to_ascii_lowercase();
    for sep in [" vs. ", " vs ", " versus ", " against ", " or "] {
        if let Some(i) = lower.find(sep) {
            let a = clean_text(&s[..i]);
            let b = clean_text(&s[i + sep.len()..]);
            if !a.is_empty() && !b.is_empty() {
                return Some((a, b));
            }
        }
    }
    None
}

#[allow(clippy::too_many_lines)]
fn parse_beat(n: usize, v: &Value, notes: &mut Notes, fixes: &mut Vec<Fix>) -> Map<String, Value> {
    let mut out = Map::new();
    let obj = match v {
        Value::String(s) => {
            notes.add(Prio::Cosmetic, Some(n), "plain text read as 'say'");
            out.insert("say".into(), Value::String(clean_text(s)));
            return out;
        }
        Value::Object(o) => o,
        _ => {
            fixes.push(
                Fix::new(Some(n), "beat", "not a beat")
                    .otherwise("write each beat as {\"say\": \"…\"}"),
            );
            return out;
        }
    };
    // Canonical field → (key as written, value).
    let mut fields: BTreeMap<&'static str, (String, &Value)> = BTreeMap::new();
    let explicit: BTreeSet<String> = obj.keys().map(|k| norm_key(k)).collect();
    for (k, val) in obj {
        let nk = norm_key(k);
        if let Some(f) = BEAT_FIELDS.iter().find(|f| **f == nk) {
            fields.insert(f, (k.clone(), val));
            continue;
        }
        match beat_alias(&nk) {
            Some(f) if explicit.contains(f) || fields.contains_key(f) => notes.add(
                Prio::Cosmetic,
                Some(n),
                format!("ignored '{k}' ('{f}' is given)"),
            ),
            Some(f) => {
                notes.add(Prio::Cosmetic, Some(n), format!("'{k}' read as '{f}'"));
                fields.insert(f, (k.clone(), val));
            }
            None => notes.group(
                Prio::Cosmetic,
                n,
                &format!("unknown:{k}"),
                format!("ignored unknown field '{k}'"),
                &format!("ignored unknown field '{k}'"),
                None,
            ),
        }
    }
    for (field, (_, val)) in &fields {
        let field = *field;
        match field {
            "say" | "show" | "meaning" | "keyword" => {
                if let Some(t) = text_of(val) {
                    out.insert(field.into(), Value::String(t));
                } else if !val.is_null() && !matches!(val, Value::String(_)) {
                    notes.add(
                        Prio::Cosmetic,
                        Some(n),
                        format!("ignored '{field}' (not text)"),
                    );
                }
            }
            "number" => match val {
                Value::Number(num) => {
                    let t = number_text(num);
                    notes.add(
                        Prio::Cosmetic,
                        Some(n),
                        format!("number {num} read as text '{t}'"),
                    );
                    out.insert("number".into(), Value::String(t));
                }
                other => {
                    if let Some(t) = text_of(other) {
                        out.insert("number".into(), Value::String(t));
                    }
                }
            },
            "picture" | "picture2" => {
                if let Some(p) = parse_picture(n, field, val, notes) {
                    out.insert(field.into(), p);
                }
            }
            "list" => match list_items(val) {
                Some((items, from_text)) => {
                    if from_text {
                        notes.add(
                            Prio::Cosmetic,
                            Some(n),
                            format!("list written as text, split into {} items", items.len()),
                        );
                    }
                    out.insert("list".into(), json!(items));
                }
                None if val.is_null() => {}
                None => notes.add(Prio::Cosmetic, Some(n), "ignored 'list' (not a list)"),
            },
            "compare" => match parse_compare(n, val, notes, fixes) {
                Some(CompareOut::Compare(c)) => {
                    out.insert("compare".into(), c);
                }
                Some(CompareOut::List(items)) => {
                    if !fields.contains_key("list") {
                        notes.add(
                            Prio::Fixed,
                            Some(n),
                            format!("compare of {} shown as a list", items.len()),
                        );
                        out.insert("list".into(), json!(items));
                    }
                }
                None => {}
            },
            "change" => {
                if let Some(c) = parse_change(n, val, fixes) {
                    out.insert("change".into(), c);
                }
            }
            "layers" => {
                let (names, focus) = match val {
                    Value::Object(o) => (
                        o.get("names")
                            .or_else(|| o.get("layers"))
                            .or_else(|| o.get("items"))
                            .and_then(list_items)
                            .map(|(v, _)| v),
                        o.get("focus").and_then(text_of),
                    ),
                    other => (list_items(other).map(|(v, _)| v), None),
                };
                match names {
                    Some(names) => {
                        let mut l = json!({ "names": names });
                        if let Some(f) = focus {
                            l["focus"] = Value::String(f);
                        }
                        out.insert("layers".into(), l);
                    }
                    None if val.is_null() => {}
                    None => fixes.push(
                        Fix::new(Some(n), "layers", "needs the layer names").otherwise(
                            "write layers {\"names\": [top, …, bottom], \"focus\": name}",
                        ),
                    ),
                }
            }
            "energy" => {
                let word = text_of(val).unwrap_or_default();
                match energy_word(&word) {
                    Some(e) => {
                        out.insert("energy".into(), json!(e));
                    }
                    None if val.is_null() => {}
                    None => notes.add(
                        Prio::Cosmetic,
                        Some(n),
                        format!("energy '{word}' unknown (calm, building or impact), dropped"),
                    ),
                }
            }
            _ => {}
        }
    }
    let say_missing = out
        .get("say")
        .and_then(Value::as_str)
        .is_none_or(|s| s.trim().is_empty());
    if say_missing {
        if let Some(show) = out.get("show").cloned() {
            notes.add(
                Prio::Fixed,
                Some(n),
                "no 'say'; the narrator reads the 'show' text",
            );
            out.insert("say".into(), show);
        }
    }
    out
}

enum CompareOut {
    Compare(Value),
    List(Vec<String>),
}

fn parse_compare(
    n: usize,
    v: &Value,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> Option<CompareOut> {
    let need = || {
        Fix::new(Some(n), "compare", "needs two sides")
            .otherwise("write compare {\"a\": …, \"b\": …}")
    };
    let (a, b, how) = match v {
        Value::Object(o) => {
            let side = |keys: &[&str]| keys.iter().find_map(|k| o.get(*k).and_then(text_of));
            let a = side(&["a", "left", "first", "x", "from", "this"]);
            let b = side(&["b", "right", "second", "y", "to", "that"]);
            for k in o.keys() {
                let known = [
                    "a", "b", "how", "left", "right", "first", "second", "x", "y", "from", "to",
                    "this", "that",
                ];
                if !known.contains(&norm_key(k).as_str()) {
                    notes.add(
                        Prio::Cosmetic,
                        Some(n),
                        format!("compare: ignored unknown field '{k}'"),
                    );
                }
            }
            match (a, b) {
                (Some(a), Some(b)) => (a, b, o.get("how").and_then(text_of)),
                _ => {
                    fixes.push(need());
                    return None;
                }
            }
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().filter_map(text_of).collect();
            match items.len() {
                2 => {
                    notes.add(
                        Prio::Cosmetic,
                        Some(n),
                        "compare written as [a, b], read as {a, b}",
                    );
                    (items[0].clone(), items[1].clone(), None)
                }
                3.. => return Some(CompareOut::List(items)),
                _ => {
                    fixes.push(need());
                    return None;
                }
            }
        }
        Value::String(s) => match split_versus(s) {
            Some((a, b)) => {
                notes.add(
                    Prio::Cosmetic,
                    Some(n),
                    format!("compare '{s}' read as '{a}' vs '{b}'"),
                );
                (a, b, None)
            }
            None => {
                fixes.push(need());
                return None;
            }
        },
        Value::Null => return None,
        _ => {
            fixes.push(need());
            return None;
        }
    };
    let mut c = json!({ "a": a, "b": b });
    if let Some(h) = how {
        match how_word(&h) {
            Some(how) => {
                let canonical = serde_json::to_value(how).unwrap_or(Value::Null);
                if canonical.as_str() != Some(h.trim()) {
                    notes.add(
                        Prio::Cosmetic,
                        Some(n),
                        format!(
                            "compare how '{h}' read as '{}'",
                            canonical.as_str().unwrap_or("")
                        ),
                    );
                }
                c["how"] = canonical;
            }
            None => notes.add(
                Prio::Cosmetic,
                Some(n),
                format!("compare how '{h}' unknown, used separate"),
            ),
        }
    }
    Some(CompareOut::Compare(c))
}

fn parse_change(n: usize, v: &Value, fixes: &mut Vec<Fix>) -> Option<Value> {
    let need = || {
        Fix::new(Some(n), "change", "needs what, from and to")
            .otherwise("write change {\"what\": …, \"from\": …, \"to\": …}")
    };
    let Value::Object(o) = v else {
        if !v.is_null() {
            fixes.push(need());
        }
        return None;
    };
    let get = |keys: &[&str]| keys.iter().find_map(|k| o.get(*k).and_then(text_of));
    match (
        get(&["what", "entity", "subject", "thing"]),
        get(&["from", "before", "old", "start"]),
        get(&["to", "after", "new", "end"]),
    ) {
        (Some(what), Some(from), Some(to)) => Some(json!({ "what": what, "from": from, "to": to })),
        _ => {
            fixes.push(need());
            None
        }
    }
}

fn parse_lite(raw: &Value, notes: &mut Notes, fixes: &mut Vec<Fix>) -> Option<LiteStory> {
    let (title, beats) = match raw {
        Value::Object(o) => {
            let mut beats = None;
            for (k, v) in o {
                match norm_key(k).as_str() {
                    "beats" => beats = Some(v),
                    "title" | "name" | "topic" => {}
                    "scenes" | "slides" | "steps" if !o.contains_key("beats") => {
                        notes.add(Prio::Cosmetic, None, format!("'{k}' read as 'beats'"));
                        beats = Some(v);
                    }
                    _ => notes.add(
                        Prio::Cosmetic,
                        None,
                        format!("story: ignored unknown field '{k}'"),
                    ),
                }
            }
            let title = o
                .get("title")
                .or_else(|| o.get("name"))
                .or_else(|| o.get("topic"))
                .and_then(text_of)
                .unwrap_or_default();
            (title, beats)
        }
        Value::Array(_) => (String::new(), Some(raw)),
        Value::String(s) => {
            return match serde_json::from_str::<Value>(s) {
                Ok(v @ (Value::Object(_) | Value::Array(_))) => parse_lite(&v, notes, fixes),
                _ => {
                    fixes.push(story_shape_fix());
                    None
                }
            };
        }
        _ => {
            fixes.push(story_shape_fix());
            return None;
        }
    };
    let Some(Value::Array(beats)) = beats else {
        fixes.push(
            Fix::new(None, "beats", "missing")
                .otherwise("write 2 to 12 beats, each {\"say\": \"…\"}"),
        );
        return None;
    };
    let before = fixes.len();
    let beats: Vec<Value> = beats
        .iter()
        .enumerate()
        .map(|(i, b)| Value::Object(parse_beat(i + 1, b, notes, fixes)))
        .collect();
    if fixes.len() > before {
        // Structural problems: keep checking the rest, but there is no story.
        return None;
    }
    let mut story_json = json!({ "beats": beats });
    if !title.is_empty() {
        story_json["title"] = Value::String(title);
    }
    match serde_json::from_value::<LiteStory>(story_json) {
        Ok(s) => Some(s),
        Err(e) => {
            fixes.push(Fix::new(None, "story", format!("not a lite story: {e}")));
            None
        }
    }
}

fn story_shape_fix() -> Fix {
    Fix::new(None, "story", "not a story object")
        .otherwise("write {\"title\": …, \"beats\": [{\"say\": …}, …]}")
}

fn parse_intent(raw: &Value) -> Result<CreativeIntent, Vec<Fix>> {
    CreativeIntent::from_value(raw.clone()).map_err(|e| {
        let beat = raw["beats"].as_array().and_then(|beats| {
            beats.iter().enumerate().find_map(|(i, b)| {
                serde_json::from_value::<Beat>(b.clone())
                    .err()
                    .map(|e| (i + 1, e.to_string()))
            })
        });
        vec![match beat {
            Some((n, msg)) => Fix::new(Some(n), "beat", msg)
                .otherwise("fix the beat to match CreativeIntent v0.2"),
            None => Fix::new(None, "story", e.to_string())
                .otherwise("send a lite story {title, beats: [{say, …}]} or a CreativeIntent v0.2"),
        }]
    })
}

fn parse_story_notes(raw: &Value, notes: &mut Notes) -> Result<Story, Vec<Fix>> {
    if raw.is_object() && looks_like_intent(raw) {
        return parse_intent(raw).map(Story::Intent);
    }
    let mut fixes = Vec::new();
    match parse_lite(raw, notes, &mut fixes) {
        Some(s) if fixes.is_empty() => Ok(Story::Lite(s)),
        _ => {
            if fixes.is_empty() {
                fixes.push(story_shape_fix());
            }
            Err(fixes)
        }
    }
}

/// Parse a raw story leniently (see module docs). Weak and creator profiles
/// both accept a lite story or a full intent.
pub fn parse_story(raw: &serde_json::Value) -> Result<Parsed, Vec<Fix>> {
    let mut notes = Notes::default();
    let story = parse_story_notes(raw, &mut notes)?;
    Ok(Parsed {
        story,
        notes: notes.lines(),
    })
}

// ---------------------------------------------------------------------------
// Format and style
// ---------------------------------------------------------------------------

/// The intent format for a `format` argument (default vertical).
pub fn format_of(format: Option<&str>) -> Result<Format, Fix> {
    match format.map(str::trim).unwrap_or("vertical") {
        "" | "vertical" => Ok(Format::Vertical),
        "square" => Ok(Format::Square),
        "wide" | "landscape" => Ok(Format::Landscape),
        other => Err(
            Fix::new(None, "format", format!("unknown format '{other}'"))
                .options(["vertical", "square", "wide"]),
        ),
    }
}

/// The StyleProfile for a `style` argument: none / `"auto"` → `{}`, a tone
/// word → `{"tone": …}`, an object (creator and up) → that profile.
pub fn style_of(style: Option<&serde_json::Value>) -> Result<StyleProfile, Fix> {
    match style {
        None | Some(Value::Null) => Ok(StyleProfile::default()),
        Some(Value::String(s)) if s.trim().is_empty() || s.trim() == "auto" => {
            Ok(StyleProfile::default())
        }
        Some(Value::String(s)) => {
            serde_json::from_value(json!({ "tone": s.trim() })).map_err(|_| {
                Fix::new(None, "style", format!("unknown style '{}'", s.trim()))
                    .options(crate::schema::tones())
            })
        }
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| Fix::new(None, "style", format!("not a StyleProfile: {e}"))),
    }
}

fn tone_profile(tone: &str) -> StyleProfile {
    if tone == "auto" {
        return StyleProfile::default();
    }
    serde_json::from_value(json!({ "tone": tone })).unwrap_or_default()
}

/// A style word, leniently: exact tone, else a synonym (noted), else auto
/// (noted; a Fix under `strict`).
fn style_word(word: &str, strict: bool, notes: &mut Notes, fixes: &mut Vec<Fix>) -> StyleProfile {
    let w = word.trim();
    if w.is_empty() || w.eq_ignore_ascii_case("auto") {
        return StyleProfile::default();
    }
    let unknown = || {
        Fix::new(None, "style", format!("unknown style '{w}'"))
            .options(
                crate::schema::tones()
                    .into_iter()
                    .filter(|t| t != "auto")
                    .take(3),
            )
            .otherwise("use one of the tone words, or auto")
    };
    match tone_for(w) {
        Some((tone, true)) => tone_profile(&tone),
        Some((tone, false)) if strict => {
            fixes.push(unknown().options([tone]));
            StyleProfile::default()
        }
        Some((tone, false)) => {
            notes.add(Prio::Ignored, None, format!("style '{w}' → '{tone}'"));
            tone_profile(&tone)
        }
        None if strict => {
            fixes.push(unknown());
            StyleProfile::default()
        }
        None => {
            notes.add(
                Prio::Ignored,
                None,
                format!("style '{w}' unknown, used auto"),
            );
            StyleProfile::default()
        }
    }
}

fn resolve_style(
    style: Option<&Value>,
    profile: Profile,
    strict: bool,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> StyleProfile {
    match style {
        None | Some(Value::Null) => StyleProfile::default(),
        Some(Value::String(s)) => style_word(s, strict, notes, fixes),
        Some(Value::Object(o)) if !profile.full_control() => {
            notes.add(Prio::Ignored, None, "style: only the tone word is used");
            match o.get("tone").and_then(Value::as_str) {
                Some(t) => style_word(t, strict, notes, fixes),
                None => StyleProfile::default(),
            }
        }
        Some(v @ Value::Object(_)) => match style_of(Some(v)) {
            Ok(s) => s,
            Err(f) => {
                fixes.push(f);
                StyleProfile::default()
            }
        },
        Some(_) => {
            notes.add(Prio::Ignored, None, "style ignored (not a tone word)");
            StyleProfile::default()
        }
    }
}

/// (0.21) A brand colour, leniently: `#0A84FF`, `0a84ff`, `#0af` → `#0A84FF`.
pub fn brand_hex(s: &str) -> Option<String> {
    let t = s.trim();
    let t = if t.starts_with('#') {
        t.to_string()
    } else {
        format!("#{t}")
    };
    motion_core::compiler::brand::parse_color(&t).map(|c| c.to_hex())
}

/// (0.21) The `brand` argument, leniently: `{primary, secondary, background,
/// text}` (also `main`/`accent`, `second`, `bg`, `ink`/`font`), a list, or a
/// comma string in that order. A colour that is not hex is ignored with a
/// note, or a fix under `strict`. `None` when no colour survives.
fn brand_of(
    raw: &Value,
    strict: bool,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> Option<motion_core::style::BrandColors> {
    const ORDER: [&str; 4] = ["primary", "secondary", "background", "text"];
    let mut given: Vec<(String, String)> = Vec::new();
    let mut positional = |items: Vec<String>, notes: &mut Notes| {
        for (i, v) in items.into_iter().enumerate() {
            match ORDER.get(i) {
                Some(f) => given.push((f.to_string(), v)),
                None => notes.add(
                    Prio::Ignored,
                    None,
                    format!("brand: colour '{v}' ignored (at most 4)"),
                ),
            }
        }
    };
    match raw {
        Value::Null => return None,
        Value::String(s) => positional(
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect(),
            notes,
        ),
        Value::Array(a) => positional(a.iter().filter_map(text_of).collect(), notes),
        Value::Object(o) => {
            for (k, v) in o {
                let field = match k.trim().to_ascii_lowercase().as_str() {
                    "primary" | "main" | "accent" | "brand" => "primary",
                    "secondary" | "second" => "secondary",
                    "background" | "bg" | "ground" => "background",
                    "text" | "ink" | "font" => "text",
                    other => {
                        notes.add(
                            Prio::Ignored,
                            None,
                            format!("brand: unknown colour '{other}' ignored"),
                        );
                        continue;
                    }
                };
                match text_of(v) {
                    Some(s) => given.push((field.to_string(), s)),
                    None if v.is_null() => {}
                    None => notes.add(
                        Prio::Ignored,
                        None,
                        format!("brand {field} ignored (not a colour)"),
                    ),
                }
            }
        }
        _ => {
            notes.add(Prio::Ignored, None, "brand ignored (not colours)");
            return None;
        }
    }
    let mut brand = motion_core::style::BrandColors::default();
    for (field, value) in given {
        let Some(hex) = brand_hex(&value) else {
            if strict {
                fixes.push(
                    Fix::new(
                        None,
                        "brand",
                        format!("{field} '{value}' is not a hex colour"),
                    )
                    .options(["#0A84FF"]),
                );
            } else {
                notes.add(
                    Prio::Ignored,
                    None,
                    format!("brand {field} '{value}' is not a hex colour; ignored"),
                );
            }
            continue;
        };
        let slot = match field.as_str() {
            "primary" => &mut brand.primary,
            "secondary" => &mut brand.secondary,
            "background" => &mut brand.background,
            _ => &mut brand.text,
        };
        *slot = Some(hex);
    }
    (!brand.is_empty()).then_some(brand)
}

/// The highest take number (`take` is 0 to 99).
pub const MAX_TAKE: u64 = 99;

/// A whole number from a JSON number or a numeric string (`2`, `2.0`, `"2"`,
/// `" 2 "`); `None` for anything else.
fn whole_number(v: &Value) -> Option<i128> {
    fn whole(f: f64) -> Option<i128> {
        (f.is_finite() && f.fract() == 0.0).then(|| f.clamp(-1e18, 1e18) as i128)
    }
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| n.as_f64().and_then(whole)),
        Value::String(s) => {
            let t = s.trim();
            t.parse::<i128>()
                .ok()
                .or_else(|| t.parse::<f64>().ok().and_then(whole))
        }
        _ => None,
    }
}

/// (0.23) The `take` argument, leniently: a whole number 0 to 99, as a number
/// or a numeric string. Above 99 or below 0 it is clamped, with a note;
/// anything that is not a whole number is ignored, with a note, and the
/// `inherited` take (the revised job's; 0 for a new video) stays. Under
/// `strict` both are a fix instead. Not given (or `null`) is `inherited`.
fn take_of(
    raw: Option<&Value>,
    inherited: u64,
    strict: bool,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> u64 {
    let Some(raw) = raw.filter(|v| !v.is_null()) else {
        return inherited;
    };
    let shown = match raw {
        Value::String(s) => format!("'{}'", clean_text(s)),
        other => other.to_string(),
    };
    let n = whole_number(raw);
    if let Some(n) = n.filter(|n| (0..=MAX_TAKE as i128).contains(n)) {
        return n as u64;
    }
    if strict {
        fixes.push(
            Fix::new(
                None,
                "take",
                format!("{shown} is not a take number (a whole number from 0 to {MAX_TAKE})"),
            )
            .options(["1", "2", "3"])
            .otherwise("leave take out"),
        );
        return inherited;
    }
    match n {
        Some(n) => {
            let clamped = if n < 0 { 0 } else { MAX_TAKE };
            notes.add(
                Prio::Ignored,
                None,
                format!("take {shown} → {clamped} (takes are 0 to {MAX_TAKE})"),
            );
            clamped
        }
        None => {
            notes.add(
                Prio::Ignored,
                None,
                format!("take {shown} ignored (not a whole number from 0 to {MAX_TAKE})"),
            );
            inherited
        }
    }
}

/// Words that carry no mood in a music phrase ("some calm music, please").
const MUSIC_FILLER: [&str; 24] = [
    "music",
    "a",
    "an",
    "the",
    "some",
    "very",
    "bed",
    "track",
    "soundtrack",
    "sound",
    "please",
    "and",
    "or",
    "with",
    "for",
    "of",
    "it",
    "be",
    "should",
    "something",
    "mood",
    "vibe",
    "feel",
    "turn",
];

/// The music word one token stands for (a weak model's synonym).
fn music_token(t: &str) -> Option<MusicWord> {
    Some(match t {
        "auto" | "automatic" | "automatically" | "default" | "any" | "whatever" | "anything"
        | "standard" | "normal" | "story" | "choose" | "best" | "true" | "on" => MusicWord::Auto,
        "none" | "no" | "off" | "without" | "mute" | "muted" | "silent" | "silence" | "nothing"
        | "nomusic" | "false" => MusicWord::None,
        "calm" | "calming" | "chill" | "chilled" | "chilling" | "relaxed" | "relaxing"
        | "relax" | "soft" | "gentle" | "quiet" | "peaceful" | "mellow" | "soothing"
        | "ambient" | "lofi" | "slow" | "serene" | "tranquil" | "sleepy" | "warm" => {
            MusicWord::Calm
        }
        "upbeat" | "happy" | "cheerful" | "bright" | "lively" | "energetic" | "energy"
        | "positive" | "uplifting" | "inspiring" | "inspirational" | "motivational"
        | "motivating" | "fast" | "hype" | "exciting" | "excited" | "joyful" | "pop"
        | "optimistic" | "hopeful" | "driving" => MusicWord::Upbeat,
        "serious" | "solemn" | "somber" | "sombre" | "sad" | "grave" | "sober" | "heavy"
        | "melancholic" | "melancholy" | "mournful" | "gloomy" | "dark" | "formal" | "grim"
        | "emotional" => MusicWord::Serious,
        "dramatic" | "epic" | "cinematic" | "intense" | "tense" | "suspense" | "suspenseful"
        | "orchestral" | "powerful" | "thriller" | "trailer" | "big" | "grand" | "majestic"
        | "heroic" | "tension" => MusicWord::Dramatic,
        "playful" | "fun" | "funny" | "quirky" | "whimsical" | "light" | "lighthearted"
        | "cheeky" | "silly" | "retro" | "cute" | "kids" | "jaunty" | "bouncy" | "comedic"
        | "comedy" => MusicWord::Playful,
        "neutral" | "corporate" | "explainer" | "professional" | "tech" | "technical"
        | "business" | "plain" | "background" | "simple" | "clean" | "minimal" | "documentary"
        | "informative" | "educational" | "steady" | "subtle" | "understated" | "modern" => {
            MusicWord::Neutral
        }
        _ => return None,
    })
}

/// (0.23) The music word for a model's text: the exact word (any case), else
/// a synonym or a short phrase whose words all mean the same word ("no
/// music" → `none`, "chill" → `calm`, "epic" → `dramatic`, "calm and
/// relaxing" → `calm`). The flag is true for the exact word. `None`: unknown,
/// or words that disagree ("upbeat but serious") or negate ("not dramatic").
pub fn music_word_for(text: &str) -> Option<(MusicWord, bool)> {
    if let Some(w) = MusicWord::parse(text) {
        return Some((w, true));
    }
    let lower = text.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty() && !MUSIC_FILLER.contains(t))
        .collect();
    let mut word: Option<MusicWord> = None;
    for t in &tokens {
        let w = music_token(t)?;
        if word.is_some_and(|prev| prev != w) {
            return None;
        }
        word = Some(w);
    }
    word.map(|w| (w, false))
}

/// A model's value as text for `music_word_for`: a string, a list of words,
/// or `true` / `false` (`false` is "no music"). `None` for anything else.
fn music_text(v: &Value) -> Option<String> {
    match v {
        Value::Bool(b) => Some(b.to_string()),
        other => text_of(other),
    }
}

/// (0.23) The `music` argument, leniently: one of the eight words
/// ([`MusicWord`]) in any case, a synonym (noted) or an unknown value (ignored
/// with a note; the `inherited` word stays: the revised job's, else `auto`).
/// Under `strict` a synonym and an unknown value are a fix instead. Not given
/// (or `null`, or empty) is `inherited`.
fn music_of(
    raw: Option<&Value>,
    inherited: MusicWord,
    strict: bool,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> MusicWord {
    let Some(raw) = raw.filter(|v| !v.is_null()) else {
        return inherited;
    };
    if matches!(raw, Value::String(s) if s.trim().is_empty()) {
        return inherited;
    }
    let shown = match raw {
        Value::String(s) => clean_text(s),
        other => other.to_string(),
    };
    // A fix line is one reply line (≤ 110 characters): a long value is cut.
    let short = if shown.chars().count() > 16 {
        format!("{}…", shown.chars().take(15).collect::<String>())
    } else {
        shown.clone()
    };
    match music_text(raw).as_deref().and_then(music_word_for) {
        Some((word, true)) => word,
        Some((word, false)) if strict => {
            fixes.push(
                Fix::new(None, "music", format!("'{short}' is not a music word"))
                    .options([word.as_str()]),
            );
            inherited
        }
        Some((word, false)) => {
            notes.add(
                Prio::Ignored,
                None,
                format!("music '{shown}' → '{}'", word.as_str()),
            );
            word
        }
        None if strict => {
            fixes.push(
                Fix::new(None, "music", format!("'{short}' is not a music word"))
                    .options(["calm", "upbeat", "dramatic"])
                    .otherwise("leave music out"),
            );
            inherited
        }
        None => {
            notes.add(
                Prio::Ignored,
                None,
                format!("music '{shown}' unknown, used {}", inherited.as_str()),
            );
            inherited
        }
    }
}

/// `format`, leniently (`None` = not given).
fn resolve_format(
    format: Option<&str>,
    strict: bool,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> Option<Format> {
    let f = format.map(str::trim).filter(|f| !f.is_empty())?;
    if let Ok(fmt) = format_of(Some(&f.to_ascii_lowercase())) {
        return Some(fmt);
    }
    match (format_synonym(f), strict) {
        (Some(fmt), false) => {
            notes.add(
                Prio::Ignored,
                None,
                format!("format '{f}' → '{}'", format_name(fmt)),
            );
            Some(fmt)
        }
        (_, true) => {
            if let Err(fix) = format_of(Some(f)) {
                fixes.push(fix);
            }
            None
        }
        (None, false) => {
            notes.add(
                Prio::Ignored,
                None,
                format!("format '{f}' unknown, used vertical"),
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Profile gating and options
// ---------------------------------------------------------------------------

/// What a profile may use of `options`, `describe` and `force`.
fn gate(args: &MakeVideoArgs, profile: Profile, notes: &mut Notes) -> (ProductionOptions, bool) {
    let mut options = args.options.clone().unwrap_or_default();
    let mut describe = args.describe;
    match profile {
        Profile::Weak => {
            if !options.is_empty() {
                notes.add(
                    Prio::Ignored,
                    None,
                    "options ignored (not offered to this model)",
                );
                options = ProductionOptions::default();
            }
            if describe {
                notes.add(
                    Prio::Ignored,
                    None,
                    "describe ignored (not offered to this model)",
                );
                describe = false;
            }
            if args.force {
                notes.add(Prio::Ignored, None, "force ignored (operator only)");
            }
        }
        Profile::Creator => {
            if args.force {
                notes.add(Prio::Ignored, None, "force ignored (operator only)");
            }
            if options.tts_model.take().is_some() {
                notes.add(
                    Prio::Ignored,
                    None,
                    "options.tts_model ignored (the free narrator is used)",
                );
            }
            if options.keep_frames {
                options.keep_frames = false;
                notes.add(
                    Prio::Ignored,
                    None,
                    "options.keep_frames ignored (operator only)",
                );
            }
        }
        Profile::Operator => {}
    }
    (options, describe)
}

/// Check `options.art` and `options.families` (unknown → dropped with a
/// note; a Fix under `strict`).
fn check_options(
    options: &mut ProductionOptions,
    pictures: &PictureIndex,
    strict: bool,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) {
    if let Some(art) = options.art.clone() {
        let a = art.trim().to_ascii_lowercase();
        if a == "auto" || a == "none" || Look::parse(&a).is_some() {
            options.art = Some(a);
        } else if strict {
            let mut looks: Vec<(usize, &str)> = Look::ALL
                .iter()
                .map(|l| (edit_distance(&a, l.name()), l.name()))
                .collect();
            looks.sort();
            fixes.push(
                Fix::new(None, "options", format!("unknown look '{art}'"))
                    .options(looks.iter().take(3).map(|(_, n)| n.to_string()))
                    .otherwise("or auto"),
            );
        } else {
            notes.add(
                Prio::Ignored,
                None,
                format!("options.art '{art}' unknown, used auto"),
            );
            options.art = None;
        }
    }
    if !options.families.is_empty() {
        let known = pictures.family_names();
        let (ok, bad): (Vec<String>, Vec<String>) = options
            .families
            .iter()
            .map(|f| f.trim().to_string())
            .partition(|f| known.contains(f));
        if !bad.is_empty() {
            if strict {
                fixes.push(
                    Fix::new(
                        None,
                        "options",
                        format!("unknown asset families: {}", bad.join(", ")),
                    )
                    .options(known.iter().take(3).cloned())
                    .otherwise("see list_options"),
                );
            } else {
                notes.add(
                    Prio::Ignored,
                    None,
                    format!("unknown asset families ignored: {}", bad.join(", ")),
                );
            }
        }
        options.families = ok;
    }
}

/// The families the compiler searches for this story: `options.families`,
/// else the look's (`--art auto` or the forced look), exactly as the
/// compiler resolves them.
pub fn families_for(
    intent: &CreativeIntent,
    style: &StyleProfile,
    options: &ProductionOptions,
) -> Vec<String> {
    if !options.families.is_empty() {
        return options.families.clone();
    }
    let mode = match options.art.as_deref().map(str::trim) {
        None | Some("") | Some("auto") => ArtMode::Auto,
        Some("none") => return Vec::new(),
        Some(name) => match Look::parse(name) {
            Some(l) => ArtMode::Force(l),
            None => ArtMode::Auto,
        },
    };
    let resolved = resolve_taste(intent, style, None);
    let emotion = typography::resolve_emotion(&resolved);
    art_direction::resolve_with_genre(mode, emotion, resolved.genre)
        .families
        .iter()
        .map(|f| f.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Lite checks and auto-fixes
// ---------------------------------------------------------------------------

/// Estimated duration of a lite story (as of its mapped intent).
fn lite_duration_s(story: &LiteStory) -> f64 {
    let w: usize = story.beats.iter().map(|b| words(&b.say)).sum();
    w as f64 / WORDS_PER_SECOND + story.beats.len() as f64 * BEAT_PAUSE_S
}

fn count_fixes(n_beats: usize, says: &[(usize, &str, usize)], total: usize, est: f64) -> Vec<Fix> {
    let mut fixes = Vec::new();
    if n_beats < *BEATS.start() {
        fixes.push(
            Fix::new(None, "beats", format!("need 2 to 12 beats (got {n_beats})"))
                .otherwise("write one beat per idea, at least 2"),
        );
    } else if n_beats > *BEATS.end() {
        fixes.push(
            Fix::new(None, "beats", format!("need 2 to 12 beats (got {n_beats})"))
                .otherwise("merge or remove beats"),
        );
    }
    for &(n, field, w) in says {
        if w < *SAY_WORDS.start() {
            fixes.push(
                Fix::new(Some(n), field, format!("has {w} words, needs 3 to 40"))
                    .otherwise("write one full sentence for the narrator"),
            );
        } else if w > *SAY_WORDS.end() {
            fixes.push(
                Fix::new(Some(n), field, format!("has {w} words, at most 40"))
                    .otherwise("shorten it or split it into two beats"),
            );
        }
    }
    if total > MAX_TOTAL_WORDS {
        fixes.push(
            Fix::new(
                None,
                "say",
                format!("the narration has {total} words, at most {MAX_TOTAL_WORDS}"),
            )
            .otherwise("shorten the beats or remove some"),
        );
    }
    if est > MAX_DURATION_S {
        fixes.push(
            Fix::new(
                None,
                "beats",
                format!("the video would run about {est:.0} s, at most {MAX_DURATION_S:.0} s"),
            )
            .otherwise("shorten the narration"),
        );
    }
    fixes
}

fn lite_limits(story: &LiteStory) -> Vec<Fix> {
    let says: Vec<(usize, &str, usize)> = story
        .beats
        .iter()
        .enumerate()
        .map(|(i, b)| (i + 1, "say", words(&b.say)))
        .collect();
    let total = says.iter().map(|s| s.2).sum();
    count_fixes(story.beats.len(), &says, total, lite_duration_s(story))
}

/// The closest layer name to `focus` (case, containment, spelling).
fn closest_layer(focus: &str, names: &[String]) -> Option<String> {
    let f = focus.trim().to_lowercase();
    if f.is_empty() {
        return None;
    }
    if let Some(n) = names.iter().find(|n| n.trim().to_lowercase() == f) {
        return Some(n.clone());
    }
    let contains: Vec<&String> = names
        .iter()
        .filter(|n| {
            let l = n.trim().to_lowercase();
            l.contains(&f) || f.contains(&l)
        })
        .collect();
    if contains.len() == 1 {
        return Some(contains[0].clone());
    }
    let prefix: Vec<&String> = names
        .iter()
        .filter(|n| {
            let l = n.trim().to_lowercase();
            let common = l.chars().zip(f.chars()).take_while(|(a, b)| a == b).count();
            common >= 4 && common * 2 >= f.chars().count().max(l.chars().count())
        })
        .collect();
    if prefix.len() == 1 {
        return Some(prefix[0].clone());
    }
    let limit = if f.chars().count() >= 5 { 2 } else { 1 };
    let mut best: Option<(usize, &String)> = None;
    for n in names {
        let d = edit_distance(&f, &n.trim().to_lowercase());
        if d <= limit && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, n));
        }
    }
    best.map(|(_, n)| n.clone())
}

/// List / layers / focus / show fixes (auto, or Fixes under `strict`).
fn fix_structures(story: &mut LiteStory, strict: bool, notes: &mut Notes, fixes: &mut Vec<Fix>) {
    for (i, b) in story.beats.iter_mut().enumerate() {
        let n = i + 1;
        if let Some(list) = b.list.as_mut() {
            list.retain(|x| !x.trim().is_empty());
            let len = list.len();
            if len > *LIST_ITEMS.end() {
                if strict {
                    fixes.push(
                        Fix::new(Some(n), "list", format!("has {len} items, at most 6"))
                            .otherwise("keep the 6 most important"),
                    );
                } else {
                    let dropped: Vec<String> = list
                        .drain(LIST_ITEMS.end()..)
                        .map(|d| format!("'{d}'"))
                        .collect();
                    notes.add(
                        Prio::Fixed,
                        Some(n),
                        format!("list cut to 6 items (dropped {})", dropped.join(", ")),
                    );
                }
            } else if len == 2 {
                if strict {
                    fixes.push(
                        Fix::new(Some(n), "list", "has 2 items, needs 3 to 6")
                            .otherwise("add an item, or write compare {a, b}"),
                    );
                } else if b.compare.is_none() {
                    let (a, c) = (list[0].clone(), list[1].clone());
                    notes.add(
                        Prio::Fixed,
                        Some(n),
                        format!("list of 2 shown as compare '{a}' vs '{c}'"),
                    );
                    b.compare = Some(lite::LiteCompare { a, b: c, how: None });
                    b.list = None;
                } else {
                    notes.add(
                        Prio::Fixed,
                        Some(n),
                        "list of 2 dropped (the beat has a compare)",
                    );
                    b.list = None;
                }
            } else if len < 2 {
                fixes.push(
                    Fix::new(Some(n), "list", format!("has {len} item(s), needs 3 to 6"))
                        .otherwise("add items, or use picture or show instead"),
                );
            }
        }
        if let Some(layers) = b.layers.as_mut() {
            layers.names.retain(|x| !x.trim().is_empty());
            let len = layers.names.len();
            if !LAYER_COUNT.contains(&len) {
                fixes.push(
                    Fix::new(Some(n), "layers", format!("has {len} layers, needs 2 to 6"))
                        .otherwise("name 2 to 6 layers from top to bottom"),
                );
            } else if let Some(focus) = layers.focus.clone() {
                match closest_layer(&focus, &layers.names) {
                    Some(name) if name.trim().to_lowercase() == focus.trim().to_lowercase() => {
                        layers.focus = Some(name);
                    }
                    Some(name) if strict => fixes.push(
                        Fix::new(
                            Some(n),
                            "layers",
                            format!("focus '{focus}' is not a layer name"),
                        )
                        .options([name])
                        .otherwise("or remove focus"),
                    ),
                    Some(name) => {
                        notes.add(Prio::Fixed, Some(n), format!("focus '{focus}' → '{name}'"));
                        layers.focus = Some(name);
                    }
                    None if strict => fixes.push(
                        Fix::new(
                            Some(n),
                            "layers",
                            format!("focus '{focus}' is not a layer name"),
                        )
                        .options(layers.names.iter().take(3).cloned())
                        .otherwise("or remove focus"),
                    ),
                    None => {
                        notes.add(
                            Prio::Fixed,
                            Some(n),
                            format!("focus '{focus}' dropped (not a layer name)"),
                        );
                        layers.focus = None;
                    }
                }
            }
        }
        if let Some(show) = b.show.clone() {
            let w = words(&show);
            if w > SHOW_MAX_WORDS {
                let short = shorten_title(&show);
                if strict {
                    fixes.push(
                        Fix::new(Some(n), "show", format!("has {w} words, at most 6"))
                            .options([short]),
                    );
                } else {
                    notes.add(Prio::Fixed, Some(n), format!("show shortened to '{short}'"));
                    b.show = Some(short);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Picture,
    Picture2,
}

impl Slot {
    fn name(self) -> &'static str {
        match self {
            Slot::Picture => "picture",
            Slot::Picture2 => "picture2",
        }
    }

    /// Whether the beat's structure shows this slot (the mapping table).
    fn used(self, s: Structure) -> bool {
        match self {
            Slot::Picture => matches!(
                s,
                Structure::Layers | Structure::Number | Structure::Picture
            ),
            Slot::Picture2 => s == Structure::Picture,
        }
    }

    fn get(self, b: &LiteBeat) -> Option<&PictureRef> {
        match self {
            Slot::Picture => b.picture.as_ref(),
            Slot::Picture2 => b.picture2.as_ref(),
        }
    }

    fn set(self, b: &mut LiteBeat, p: Option<PictureRef>) {
        match self {
            Slot::Picture => b.picture = p,
            Slot::Picture2 => b.picture2 = p,
        }
    }
}

/// The other words the compiler adds to a picture's catalog request.
fn picture_extra(b: &LiteBeat, slot: Slot) -> Vec<String> {
    match (slot, b.structure()) {
        (Slot::Picture, Structure::Picture | Structure::Layers) => {
            b.meaning.iter().map(|m| m.trim().to_string()).collect()
        }
        _ => Vec::new(),
    }
}

/// Library nouns that would show as text: replaced by a strong suggestion,
/// else kept (noted); Fixes with options under `strict`. `skip`: beats whose
/// `picture` is replaced by the user's own image.
fn check_pictures(
    story: &mut LiteStory,
    families: &[String],
    pictures: &PictureIndex,
    strict: bool,
    skip: &BTreeSet<usize>,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) {
    for (i, b) in story.beats.iter_mut().enumerate() {
        let n = i + 1;
        let structure = b.structure();
        for slot in [Slot::Picture, Slot::Picture2] {
            if !slot.used(structure) || (slot == Slot::Picture && skip.contains(&n)) {
                continue;
            }
            let Some(noun) = slot.get(b).and_then(|p| p.noun()).map(str::to_string) else {
                continue;
            };
            let extra = picture_extra(b, slot);
            let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
            if pictures.is_picture_with(&noun, &extra, families) {
                continue;
            }
            let suggestions = pictures.suggest_scored(&noun, 3, families);
            if strict {
                fixes.push(
                    Fix::new(Some(n), slot.name(), format!("no picture for '{noun}'"))
                        .options(suggestions.iter().map(|(s, _)| s.clone()))
                        .otherwise(LEAVE_AS_TEXT),
                );
                continue;
            }
            match suggestions.first() {
                Some((top, strength)) if strength.is_strong() => {
                    notes.add(
                        Prio::Swap,
                        Some(n),
                        format!("{} '{noun}' → '{top}'", slot.name()),
                    );
                    slot.set(b, Some(PictureRef::Name(top.clone())));
                }
                Some((top, _)) => notes.add(
                    Prio::Text,
                    Some(n),
                    format!(
                        "{} '{noun}' shown as text (did you mean '{top}'?)",
                        slot.name()
                    ),
                ),
                None => notes.group(
                    Prio::Text,
                    n,
                    "text",
                    format!("{} '{noun}' shown as text", slot.name()),
                    "pictures shown as text",
                    Some(noun.clone()),
                ),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// User images
// ---------------------------------------------------------------------------

/// A resolved `assets` folder.
struct Folder {
    /// Root-relative folder path (`my_trip`).
    rel: String,
    dir: std::path::PathBuf,
    scan: byo::FolderScan,
}

fn resolve_folder(
    assets: Option<&str>,
    sandbox: &Sandbox,
    strict: bool,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> Option<Folder> {
    let a = assets.map(str::trim).filter(|a| !a.is_empty())?;
    match sandbox.resolve_folder(a) {
        Ok((dir, root)) => {
            let rel = dir
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            match byo::scan_folder(&dir) {
                Ok(scan) => Some(Folder { rel, dir, scan }),
                Err(e) => {
                    notes.add(
                        Prio::Ignored,
                        None,
                        format!("assets '{a}' cannot be read ({e}); using library pictures"),
                    );
                    None
                }
            }
        }
        Err(e) if e.kind == RefErrorKind::Outside || strict => {
            fixes.push(e.to_fix(None, "assets"));
            None
        }
        Err(e) => {
            let why = if e.kind == RefErrorKind::NotFound {
                "not found".to_string()
            } else {
                e.problem
            };
            notes.add(
                Prio::Ignored,
                None,
                format!("assets '{a}' {why}; using library pictures"),
            );
            None
        }
    }
}

/// A user image, after the role decision and treatment.
struct Treated {
    role: ImageRole,
    guessed: bool,
    source: std::path::PathBuf,
    cutout: bool,
    width: u32,
    height: u32,
    /// Height of the original file (size feedback).
    original_height: u32,
    alpha: bool,
    labels: Vec<String>,
}

/// Role (given or guessed with Apple Vision), cutout of opaque people and
/// objects, a PNG copy for WebP places. Problems become findings; a WebP
/// that cannot be converted is a Fix.
#[allow(clippy::too_many_arguments)]
fn treat(
    beat: Option<usize>,
    field: &str,
    file: &CheckedFile,
    given: Option<ImageRole>,
    describe: bool,
    ctx: &Context,
    findings: &mut Vec<(u8, String)>,
    fixes: &mut Vec<Fix>,
) -> Option<Treated> {
    let who = match beat {
        Some(n) => format!("beat {n}: "),
        None => String::new(),
    };
    let need_probe = given.is_none() || describe || file.header.alpha;
    let probe = if need_probe {
        match byo::probe(file, ctx.config) {
            Ok(p) => Some(p),
            Err(e) => {
                if given.is_none() {
                    let hint = match beat {
                        Some(n) => {
                            format!(" — name it beat{n}_person.jpg (or _object, _place) to choose")
                        }
                        None => String::new(),
                    };
                    findings.push((
                        1,
                        format!(
                            "{who}could not tell what {} shows ({e}), used object{hint}",
                            file.name()
                        ),
                    ));
                }
                None
            }
        }
    } else {
        None
    };
    let (role, guessed) = match (given, &probe) {
        (Some(r), _) => (r, false),
        (None, Some(p)) => (byo::guess_role(p), true),
        (None, None) => (ImageRole::Object, true),
    };
    let opaque = probe.as_ref().map_or(!file.header.alpha, |p| p.opaque);
    let labels = probe.as_ref().map(|p| p.labels.clone()).unwrap_or_default();
    if file.header.height < byo::MIN_IMAGE_HEIGHT {
        findings.push((
            0,
            format!(
                "{who}photo is {} px tall, needs ≥ {} — use a larger file or a library picture",
                file.header.height,
                byo::MIN_IMAGE_HEIGHT
            ),
        ));
    }
    let mut out = Treated {
        role,
        guessed,
        source: file.path.clone(),
        cutout: false,
        width: file.header.width,
        height: file.header.height,
        original_height: file.header.height,
        alpha: file.header.alpha && !opaque,
        labels,
    };
    if role != ImageRole::Place && opaque {
        match byo::cutout(file, ctx.config) {
            Ok(path) => match byo::read_header(&path) {
                Some(h) => {
                    out.source = path;
                    out.cutout = true;
                    out.width = h.width;
                    out.height = h.height;
                    out.alpha = true;
                }
                None => findings.push((
                    1,
                    format!(
                        "{who}the cutout of {} is unreadable, used the photo",
                        file.name()
                    ),
                )),
            },
            Err(e) => findings.push((
                1,
                format!(
                    "{who}could not cut out the {} in {} ({e}), used the photo as is",
                    role_name(role),
                    file.name()
                ),
            )),
        }
    }
    if !out.cutout && file.header.format == byo::ImageFormat::Webp {
        match byo::png_copy(file, ctx.config) {
            Ok(png) => out.source = png,
            Err(e) => {
                fixes.push(
                    Fix::new(
                        beat,
                        field,
                        format!("{} could not be used ({e})", file.name()),
                    )
                    .otherwise("use a jpg or png file"),
                );
                return None;
            }
        }
    }
    Some(out)
}

fn ext_of_source(p: &std::path::Path) -> &'static str {
    match byo::read_header(p).map(|h| h.format) {
        Some(byo::ImageFormat::Png) => "png",
        Some(byo::ImageFormat::Jpeg) => "jpg",
        _ => "png",
    }
}

/// Manifest request ids a user image serves, by role (person → hero
/// subject/portrait; object → the object roles; place → environment plus the
/// object roles, so the photo is shown in every look). `taken` keeps ids
/// unique within a beat.
fn serve_ids(
    n: usize,
    role: ImageRole,
    own_place: bool,
    taken: &mut BTreeSet<String>,
) -> Vec<String> {
    let roles: &[&str] = match role {
        ImageRole::Person => &["hero_subject", "portrait"],
        ImageRole::Object => &["hero_object", "supporting_object", "evidence_image"],
        ImageRole::Place if own_place => &[
            "environment",
            "hero_object",
            "supporting_object",
            "evidence_image",
        ],
        ImageRole::Place => &["environment"],
    };
    let mut out = Vec::new();
    for r in roles {
        let id = format!("beat_{n}.{r}");
        if taken.insert(id.clone()) {
            out.push(id);
        }
    }
    out
}

/// Plan text of a user image, e.g. `photo beat1.jpg (person, cut out)`.
fn image_text(name: &str, t: &Treated, describe: bool) -> String {
    let mut bits = vec![format!(
        "{}{}",
        role_name(t.role),
        if t.guessed { ", guessed" } else { "" }
    )];
    if t.cutout {
        bits.push("cut out".into());
    }
    if t.original_height < byo::MIN_IMAGE_HEIGHT {
        bits.push(format!("{} px tall", t.original_height));
    }
    if describe && !t.labels.is_empty() {
        bits.push(byo::label_text(&t.labels, 3));
    }
    format!("photo {name} ({})", bits.join(", "))
}

/// The user images of a request.
#[derive(Default)]
struct Images {
    entries: Vec<ImageEntry>,
    /// (beat, slot) → plan text.
    texts: BTreeMap<(usize, Slot), String>,
    /// Image lines (check mode).
    lines: Vec<String>,
    /// Beats with their own image (no background for them).
    own: BTreeSet<usize>,
    background_line: Option<String>,
}

impl Images {
    fn prepared(&self) -> Option<PreparedImages> {
        (!self.entries.is_empty()).then(|| PreparedImages {
            entries: self.entries.clone(),
            lines: self.lines.clone(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn add_image(
    images: &mut Images,
    taken: &mut BTreeSet<String>,
    n: usize,
    slot: Slot,
    file: &CheckedFile,
    t: &Treated,
    serve_role: ImageRole,
    describe: bool,
) {
    let serves = serve_ids(n, serve_role, true, taken);
    let Some(id) = serves.first().cloned() else {
        return;
    };
    let suffix = if slot == Slot::Picture2 { "_2" } else { "" };
    let file_name = format!(
        "beat{n}_{}{suffix}.{}",
        role_name(t.role),
        ext_of_source(&t.source)
    );
    let text = image_text(&file.name(), t, describe);
    images.lines.push(format!("beat {n}: {text}"));
    images.texts.insert((n, slot), text);
    images.own.insert(n);
    images.entries.push(ImageEntry {
        id,
        source: t.source.clone(),
        file_name,
        sha256: file.sha256.clone(),
        role: t.role,
        role_guessed: t.guessed,
        width: t.width,
        height: t.height,
        alpha: t.alpha,
        cutout: t.cutout,
        labels: t.labels.clone(),
        serves,
        meta: None,
    });
}

/// `background.*` → the environment of every beat without its own image.
#[allow(clippy::too_many_arguments)]
fn add_background(
    images: &mut Images,
    taken: &mut BTreeSet<String>,
    folder: &Folder,
    n_beats: usize,
    sandbox: &Sandbox,
    describe: bool,
    ctx: &Context,
    findings: &mut Vec<(u8, String)>,
    fixes: &mut Vec<Fix>,
) {
    let Some(bg) = &folder.scan.background else {
        return;
    };
    let reference = join_rel(&folder.rel, bg);
    let file = match sandbox.resolve(&reference) {
        Ok(f) => f,
        Err(e) => {
            fixes.push(e.to_fix(None, "assets"));
            return;
        }
    };
    let Some(t) = treat(
        None,
        "assets",
        &file,
        Some(ImageRole::Place),
        describe,
        ctx,
        findings,
        fixes,
    ) else {
        return;
    };
    let beats: BTreeSet<usize> = (1..=n_beats).filter(|n| !images.own.contains(n)).collect();
    if beats.is_empty() {
        findings.push((2, format!("'{bg}' not used (every beat has its own image)")));
        return;
    }
    let ext = ext_of_source(&t.source);
    for &n in &beats {
        for id in serve_ids(n, ImageRole::Place, false, taken) {
            images.entries.push(ImageEntry {
                id: id.clone(),
                source: t.source.clone(),
                file_name: format!("background.{ext}"),
                sha256: file.sha256.clone(),
                role: ImageRole::Place,
                role_guessed: false,
                width: t.width,
                height: t.height,
                alpha: t.alpha,
                cutout: false,
                labels: t.labels.clone(),
                serves: vec![id],
                meta: None,
            });
        }
    }
    let text = image_text(bg, &t, describe);
    let line = format!("background: {text} for {}", beats_label(&beats));
    images.lines.push(line.clone());
    images.background_line = Some(line);
}

fn join_rel(folder: &str, file: &str) -> String {
    if folder.is_empty() {
        file.to_string()
    } else {
        format!("{folder}/{file}")
    }
}

/// A ready `manifest.json` in the folder: every entry's file passes the
/// sandbox; ids and other fields pass through.
fn manifest_images(folder: &Folder, sandbox: &Sandbox, images: &mut Images, fixes: &mut Vec<Fix>) {
    let path = folder.dir.join("manifest.json");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&text) else {
        fixes.push(
            Fix::new(None, "assets", "manifest.json is not valid JSON")
                .otherwise("fix it, or remove it and name the images beatN.jpg"),
        );
        return;
    };
    let Some(entries) = m.get("assets").and_then(Value::as_array) else {
        fixes.push(Fix::new(None, "assets", "manifest.json has no assets list"));
        return;
    };
    for (i, e) in entries.iter().enumerate() {
        let Some(o) = e.as_object() else {
            fixes.push(Fix::new(
                None,
                "assets",
                format!("manifest entry {} is not an object", i + 1),
            ));
            continue;
        };
        let id = o
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let p = o.get("path").and_then(Value::as_str).unwrap_or_default();
        if id.is_empty() {
            fixes.push(Fix::new(
                None,
                "assets",
                format!("manifest entry {} has no id", i + 1),
            ));
            continue;
        }
        let file = match sandbox.resolve(&join_rel(&folder.rel, p)) {
            Ok(f) => f,
            Err(err) => {
                fixes.push(err.to_fix(None, "assets"));
                continue;
            }
        };
        if file.header.format == byo::ImageFormat::Webp {
            fixes.push(
                Fix::new(None, "assets", format!("manifest image '{p}' is WebP"))
                    .otherwise("use png or jpg in a manifest"),
            );
            continue;
        }
        let role = if id.ends_with("hero_subject") || id.ends_with("portrait") {
            ImageRole::Person
        } else if id.ends_with("environment") {
            ImageRole::Place
        } else {
            ImageRole::Object
        };
        let mut meta = o.clone();
        let serves: Vec<String> = meta
            .remove("serves")
            .and_then(|s| serde_json::from_value(s).ok())
            .unwrap_or_default();
        let alpha = meta.get("alpha").and_then(Value::as_bool).unwrap_or(false)
            && file.header.format == byo::ImageFormat::Png;
        for k in ["id", "path", "width", "height", "alpha"] {
            meta.remove(k);
        }
        let safe: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let n = beat_in(&id.replace('_', " ")).unwrap_or(0);
        images.lines.push(format!(
            "{id}: {} ({}×{})",
            file.name(),
            file.header.width,
            file.header.height
        ));
        if n > 0 {
            images.own.insert(n);
        }
        images.entries.push(ImageEntry {
            id: id.clone(),
            source: file.path.clone(),
            file_name: format!("{safe}.{}", file.header.format.ext()),
            sha256: file.sha256.clone(),
            role,
            role_guessed: false,
            width: file.header.width,
            height: file.header.height,
            alpha,
            cutout: false,
            labels: Vec::new(),
            serves,
            meta: Some(Value::Object(meta)),
        });
    }
}

/// Folder images by convention for an intent (no story change).
#[allow(clippy::too_many_arguments)]
fn intent_folder_images(
    folder: &Folder,
    n_beats: usize,
    sandbox: &Sandbox,
    describe: bool,
    ctx: &Context,
    images: &mut Images,
    findings: &mut Vec<(u8, String)>,
    fixes: &mut Vec<Fix>,
) {
    let mut taken = BTreeSet::new();
    for (&n, (name, role)) in &folder.scan.beats {
        if n > n_beats {
            findings.push((
                2,
                format!("'{name}' not used (the story has {n_beats} beats)"),
            ));
            continue;
        }
        let file = match sandbox.resolve(&join_rel(&folder.rel, name)) {
            Ok(f) => f,
            Err(e) => {
                fixes.push(e.to_fix(Some(n), "picture"));
                continue;
            }
        };
        if let Some(t) = treat(
            Some(n),
            "picture",
            &file,
            *role,
            describe,
            ctx,
            findings,
            fixes,
        ) {
            add_image(
                images,
                &mut taken,
                n,
                Slot::Picture,
                &file,
                &t,
                t.role,
                describe,
            );
        }
    }
    add_background(
        images, &mut taken, folder, n_beats, sandbox, describe, ctx, findings, fixes,
    );
}

// ---------------------------------------------------------------------------
// Plan lines
// ---------------------------------------------------------------------------

fn lib_picture_text(
    pictures: &PictureIndex,
    noun: &str,
    extra: &[&str],
    families: &[String],
) -> String {
    match pictures.shown_as(noun, extra, families) {
        None => format!("picture {noun} (text)"),
        Some(id) if id != lite::snake(noun) => format!("picture {noun} (as {id})"),
        Some(_) => format!("picture {noun}"),
    }
}

fn slot_text(
    b: &LiteBeat,
    n: usize,
    slot: Slot,
    pictures: &PictureIndex,
    families: &[String],
    images: &Images,
) -> Option<String> {
    if let Some(t) = images.texts.get(&(n, slot)) {
        return Some(t.clone());
    }
    let p = slot.get(b)?;
    match p.noun() {
        Some(noun) => {
            let extra = picture_extra(b, slot);
            let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
            Some(lib_picture_text(pictures, noun, &extra, families))
        }
        None => p.file().map(|f| format!("photo {}", f.file)),
    }
}

/// `robot, microchip (pictures), zorb (text)`.
fn items_text(items: &[String], pictures: &PictureIndex, families: &[String]) -> String {
    let mut runs: Vec<(bool, Vec<&str>)> = Vec::new();
    for it in items {
        let pic = pictures.is_picture_in(it, families);
        match runs.last_mut() {
            Some((p, v)) if *p == pic => v.push(it),
            _ => runs.push((pic, vec![it])),
        }
    }
    runs.iter()
        .map(|(pic, v)| {
            let label = match (pic, v.len()) {
                (true, 1) => "picture",
                (true, _) => "pictures",
                (false, _) => "text",
            };
            format!("{} ({label})", v.join(", "))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn lite_plan_line(
    n: usize,
    b: &LiteBeat,
    statement: &str,
    pictures: &PictureIndex,
    families: &[String],
    images: &Images,
) -> String {
    let pic = |slot| slot_text(b, n, slot, pictures, families, images);
    let side = |w: &str| {
        if pictures.is_picture_in(w, families) {
            format!("{w} (picture)")
        } else {
            w.to_string()
        }
    };
    let head = match b.structure() {
        Structure::Layers => {
            let l = b.layers.as_ref();
            let names = l.map(|l| l.names.join(", ")).unwrap_or_default();
            let focus = l
                .and_then(|l| l.focus.clone())
                .map(|f| format!(" · focus {f}"))
                .unwrap_or_default();
            let extra = pic(Slot::Picture)
                .map(|p| format!(" + {p}"))
                .or_else(|| b.number.as_ref().map(|v| format!(" + number {v}")))
                .unwrap_or_default();
            format!("{n} layers {names}{focus}{extra}")
        }
        Structure::List => {
            let items = b.list.clone().unwrap_or_default();
            format!("{n} list: {}", items_text(&items, pictures, families))
        }
        Structure::Compare => {
            let c = b.compare.clone().unwrap_or_default();
            let how = match c.how {
                Some(h) if h != How::Separate => {
                    format!(
                        " · {}",
                        serde_json::to_value(h)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_default()
                    )
                }
                _ => String::new(),
            };
            format!("{n} compare {} vs {}{how}", side(&c.a), side(&c.b))
        }
        Structure::Change => {
            let c = b.change.clone().unwrap_or_default();
            format!("{n} change {}: {} → {}", c.what, c.from, c.to)
        }
        Structure::Number => {
            let v = b.number.clone().unwrap_or_default();
            match pic(Slot::Picture) {
                Some(p) => format!("{n} number {v} + {p}"),
                None => format!("{n} number {v}"),
            }
        }
        Structure::Picture => {
            let p1 = pic(Slot::Picture).unwrap_or_default();
            match pic(Slot::Picture2) {
                Some(p2) => format!("{n} {p1} + {p2}"),
                None => format!("{n} {p1}"),
            }
        }
        Structure::Phrase => format!("{n} title"),
    };
    format!("{head} · \"{statement}\"")
}

fn subject_text(s: &Subject, pictures: &PictureIndex, families: &[String]) -> String {
    match s {
        Subject::Phrase(a) => match a.value.as_deref().or(a.meaning.as_deref()) {
            Some(v) => format!("phrase {v}"),
            None => "phrase".into(),
        },
        Subject::Number(a) => format!("number {}", a.value.as_deref().unwrap_or_default()),
        Subject::Object(o) => {
            let extra: Vec<&str> = [o.value.as_deref(), o.meaning.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            lib_picture_text(pictures, &o.asset, &extra, families)
        }
        Subject::Collection(c) => {
            let items: Vec<String> = c
                .items
                .iter()
                .map(|i| match i {
                    CollectionItem::Object(o) => o.asset.clone(),
                    CollectionItem::Phrase(a) | CollectionItem::Number(a) => {
                        a.value.clone().or(a.meaning.clone()).unwrap_or_default()
                    }
                })
                .collect();
            format!("list: {}", items.join(", "))
        }
        Subject::StateChange(c) => format!("change {}: {} → {}", c.entity, c.from, c.to),
        Subject::DerivedMetric(m) => format!("metric {}", m.meaning.as_deref().unwrap_or("ratio")),
        Subject::Layers(l) => {
            let names: Vec<&str> = l.layers.iter().map(|x| x.name.as_str()).collect();
            format!("layers {}", names.join(", "))
        }
    }
}

fn intent_plan_line(
    n: usize,
    b: &Beat,
    pictures: &PictureIndex,
    families: &[String],
    images: &Images,
) -> String {
    let purpose = serde_json::to_value(b.purpose)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut text = format!(
        "{n} {purpose} {}",
        subject_text(&b.primary, pictures, families)
    );
    if let Some(s) = &b.secondary {
        text.push_str(&format!(" + {}", subject_text(s, pictures, families)));
    }
    if let Some(img) = images.texts.get(&(n, Slot::Picture)) {
        text.push_str(&format!(" + {img}"));
    }
    format!("{text} · \"{}\"", b.statement)
}

/// Beat lines + extra lines + the duration line, within the reply's plan cap.
fn finish_plan(
    mut beat_lines: Vec<String>,
    extra: Vec<String>,
    est: f64,
    n_beats: usize,
) -> Vec<String> {
    let cap = crate::reply::MAX_PLAN_LINES;
    while beat_lines.len() + extra.len() + 1 > cap && beat_lines.len() >= 2 {
        let last = beat_lines.pop().unwrap_or_default();
        if let Some(prev) = beat_lines.last_mut() {
            *prev = format!("{prev}; {last}");
        }
    }
    let mut plan: Vec<String> = beat_lines.into_iter().chain(extra).collect();
    plan.truncate(cap.saturating_sub(1));
    plan.push(format!("about {est:.0} s, {n_beats} beats"));
    plan.into_iter().map(|l| crate::reply::cut(&l)).collect()
}

// ---------------------------------------------------------------------------
// prepare
// ---------------------------------------------------------------------------

fn validation_fixes(intent: &CreativeIntent) -> Vec<Fix> {
    match intent.validate() {
        Ok(()) => Vec::new(),
        Err(errs) => errs
            .into_iter()
            .map(|e| {
                let field = field_of(&e);
                Fix::new(beat_in(&e), field, e)
            })
            .collect(),
    }
}

/// Check, auto-fix and map a `make_video` call.
pub fn prepare(args: &MakeVideoArgs, ctx: &Context) -> Result<Prepared, Vec<Fix>> {
    prepare_inner(args, ctx, None, 0, MusicWord::Auto, Notes::default())
}

#[allow(clippy::too_many_lines)]
fn prepare_inner(
    args: &MakeVideoArgs,
    ctx: &Context,
    inherited_style: Option<StyleProfile>,
    inherited_take: u64,
    inherited_music: MusicWord,
    mut notes: Notes,
) -> Result<Prepared, Vec<Fix>> {
    let profile = ctx.config.profile;
    let strict = args.strict;
    let mut fixes: Vec<Fix> = Vec::new();
    let mut findings: Vec<(u8, String)> = Vec::new();

    let (mut options, describe) = gate(args, profile, &mut notes);
    let inherited_brand = inherited_style.as_ref().and_then(|s| s.brand.clone());
    let mut style = match (&args.style, inherited_style) {
        (None, Some(s)) => s,
        (style, _) => resolve_style(style.as_ref(), profile, strict, &mut notes, &mut fixes),
    };
    // (0.21) Brand colours, every profile: the `brand` argument, else a style
    // object's own `brand`, else (on revise) the job's; always normalised.
    let raw_brand = args.brand.clone().or_else(|| {
        args.style
            .as_ref()
            .and_then(|s| s.get("brand"))
            .filter(|b| !b.is_null())
            .cloned()
    });
    style.brand = match raw_brand {
        Some(raw) => brand_of(&raw, strict, &mut notes, &mut fixes),
        None => inherited_brand,
    };
    // (0.23) The take: the given one, else (on revise) the job's, else 0.
    let take = take_of(
        args.take.as_ref(),
        inherited_take,
        strict,
        &mut notes,
        &mut fixes,
    );
    // (0.23) The music mood word: the given one, else (on revise) the job's,
    // else `auto`.
    let music = music_of(
        args.music.as_ref(),
        inherited_music,
        strict,
        &mut notes,
        &mut fixes,
    );
    let format = resolve_format(args.format.as_deref(), strict, &mut notes, &mut fixes);
    check_options(&mut options, ctx.pictures, strict, &mut notes, &mut fixes);

    let story = match parse_story_notes(&args.story, &mut notes) {
        Ok(s) => s,
        Err(mut f) => {
            fixes.append(&mut f);
            return Err(fixes);
        }
    };
    let sandbox = Sandbox::new(ctx.config);
    let folder = resolve_folder(
        args.assets.as_deref(),
        &sandbox,
        strict,
        &mut notes,
        &mut fixes,
    );
    let mut images = Images::default();

    let (kind, story_json, intent, beat_lines, est) = match story {
        Story::Lite(mut lite_story) => {
            fix_structures(&mut lite_story, strict, &mut notes, &mut fixes);
            fixes.extend(lite_limits(&lite_story));
            let fmt = format.unwrap_or_default();

            // Folder convention: which beats get the user's image in `picture`.
            let mut folder_refs: BTreeMap<usize, FileRef> = BTreeMap::new();
            if let Some(f) = &folder {
                if f.scan.manifest {
                    manifest_images(f, &sandbox, &mut images, &mut fixes);
                } else {
                    for (&n, (name, role)) in &f.scan.beats {
                        let Some(b) = lite_story.beats.get(n - 1) else {
                            findings.push((
                                2,
                                format!(
                                    "'{name}' not used (the story has {} beats)",
                                    lite_story.beats.len()
                                ),
                            ));
                            continue;
                        };
                        let structure = b.structure();
                        let shown = Slot::Picture.used(structure)
                            || (structure == Structure::Phrase && b.picture2.is_none());
                        if !shown {
                            findings.push((
                                2,
                                format!(
                                    "beat {n}: '{name}' not used (the beat shows a {})",
                                    structure.name()
                                ),
                            ));
                            continue;
                        }
                        if b.picture.as_ref().and_then(PictureRef::file).is_some() {
                            findings.push((
                                2,
                                format!(
                                    "beat {n}: '{name}' not used (the beat has its own picture)"
                                ),
                            ));
                            continue;
                        }
                        if let Some(noun) = b.picture.as_ref().and_then(|p| p.noun()) {
                            notes.add(
                                Prio::Swap,
                                Some(n),
                                format!("picture '{noun}' → your image {name}"),
                            );
                        }
                        folder_refs.insert(
                            n,
                            FileRef {
                                file: join_rel(&f.rel, name),
                                role: *role,
                            },
                        );
                    }
                }
            }

            // The look's families (taste does not depend on pictures).
            let first = lite::to_intent(&lite_story, fmt, &ctx.pictures.with_families(&[]));
            let families = families_for(&first.intent, &style, &options);
            let skip: BTreeSet<usize> = folder_refs.keys().copied().collect();
            check_pictures(
                &mut lite_story,
                &families,
                ctx.pictures,
                strict,
                &skip,
                &mut notes,
                &mut fixes,
            );

            // The mapped copy: folder images and the user images' roles.
            let mut mapped_story = lite_story.clone();
            for (&n, r) in &folder_refs {
                if let Some(b) = mapped_story.beats.get_mut(n - 1) {
                    b.picture = Some(PictureRef::File(r.clone()));
                }
            }
            let mut taken: BTreeSet<String> = BTreeSet::new();
            for i in 0..mapped_story.beats.len() {
                let n = i + 1;
                for slot in [Slot::Picture, Slot::Picture2] {
                    let Some(fref) = slot.get(&mapped_story.beats[i]).and_then(PictureRef::file)
                    else {
                        continue;
                    };
                    let file = match sandbox.resolve(&fref.file) {
                        Ok(f) => f,
                        Err(e) => {
                            fixes.push(e.to_fix(Some(n), slot.name()));
                            continue;
                        }
                    };
                    if !slot.used(mapped_story.beats[i].structure()) {
                        continue;
                    }
                    if let Some(t) = treat(
                        Some(n),
                        slot.name(),
                        &file,
                        fref.role,
                        describe,
                        ctx,
                        &mut findings,
                        &mut fixes,
                    ) {
                        // A layers beat pins its picture as an object in the
                        // lit layer (the stack reads `supporting_object`).
                        let structure = mapped_story.beats[i].structure();
                        let serve_role =
                            if structure == Structure::Layers && t.role == ImageRole::Person {
                                ImageRole::Object
                            } else {
                                t.role
                            };
                        add_image(
                            &mut images,
                            &mut taken,
                            n,
                            slot,
                            &file,
                            &t,
                            serve_role,
                            describe,
                        );
                        slot.set(
                            &mut mapped_story.beats[i],
                            Some(PictureRef::File(FileRef {
                                file: fref.file.clone(),
                                role: Some(serve_role),
                            })),
                        );
                    }
                }
            }
            if let Some(f) = &folder {
                if !f.scan.manifest {
                    let n_beats = mapped_story.beats.len();
                    add_background(
                        &mut images,
                        &mut taken,
                        f,
                        n_beats,
                        &sandbox,
                        describe,
                        ctx,
                        &mut findings,
                        &mut fixes,
                    );
                }
            }

            let mapped =
                lite::to_intent(&mapped_story, fmt, &ctx.pictures.with_families(&families));
            for note in &mapped.notes {
                findings.push((3, note.clone()));
            }
            if fixes.is_empty() {
                // A backstop: the checks above already name these problems.
                fixes.extend(validation_fixes(&mapped.intent));
            }
            let beat_lines: Vec<String> = mapped_story
                .beats
                .iter()
                .zip(&mapped.intent.beats)
                .enumerate()
                .map(|(i, (b, ib))| {
                    lite_plan_line(i + 1, b, &ib.statement, ctx.pictures, &families, &images)
                })
                .collect();
            let story_json = serde_json::to_value(&lite_story).map_err(|e| {
                vec![Fix::new(
                    None,
                    "story",
                    format!("cannot store the story: {e}"),
                )]
            })?;
            let est = estimate_duration_s(&mapped.intent);
            (StoryKind::Lite, story_json, mapped.intent, beat_lines, est)
        }
        Story::Intent(mut intent) => {
            if let Some(f) = format {
                intent.format = f;
            }
            let says: Vec<(usize, &str, usize)> = intent
                .beats
                .iter()
                .enumerate()
                .filter_map(|(i, b)| {
                    b.narration
                        .as_deref()
                        .map(|t| (i + 1, "narration", words(t)))
                })
                .collect();
            let total: usize = intent
                .beats
                .iter()
                .map(|b| words(b.narration.as_deref().unwrap_or(&b.statement)))
                .sum();
            let est = estimate_duration_s(&intent);
            fixes.extend(count_fixes(intent.beats.len(), &says, total, est));
            fixes.extend(validation_fixes(&intent));
            let families = families_for(&intent, &style, &options);
            for (i, b) in intent.beats.iter().enumerate() {
                for s in [Some(&b.primary), b.secondary.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    if let Subject::Object(o) = s {
                        let extra: Vec<&str> = [o.value.as_deref(), o.meaning.as_deref()]
                            .into_iter()
                            .flatten()
                            .collect();
                        if !ctx.pictures.is_picture_with(&o.asset, &extra, &families) {
                            notes.group(
                                Prio::Text,
                                i + 1,
                                "text",
                                format!("picture '{}' shown as text", o.asset),
                                "pictures shown as text",
                                Some(o.asset.clone()),
                            );
                        }
                    }
                }
            }
            if let Some(f) = &folder {
                if f.scan.manifest {
                    manifest_images(f, &sandbox, &mut images, &mut fixes);
                } else {
                    intent_folder_images(
                        f,
                        intent.beats.len(),
                        &sandbox,
                        describe,
                        ctx,
                        &mut images,
                        &mut findings,
                        &mut fixes,
                    );
                }
            }
            let beat_lines: Vec<String> = intent
                .beats
                .iter()
                .enumerate()
                .map(|(i, b)| intent_plan_line(i + 1, b, ctx.pictures, &families, &images))
                .collect();
            let story_json = serde_json::to_value(&intent).map_err(|e| {
                vec![Fix::new(
                    None,
                    "story",
                    format!("cannot store the story: {e}"),
                )]
            })?;
            (StoryKind::Intent, story_json, intent, beat_lines, est)
        }
    };
    if let Some(f) = &folder {
        for line in &f.scan.ignored {
            findings.push((2, line.clone()));
        }
    }
    let prepared_images = images.prepared();
    if let Some(p) = &prepared_images {
        if let Err(e) = byo::manifest_json(p) {
            fixes.push(Fix::new(
                None,
                "assets",
                format!("the images do not form a valid manifest: {e}"),
            ));
        }
    }
    if !fixes.is_empty() {
        return Err(fixes);
    }

    let style_json = serde_json::to_value(&style).map_err(|e| {
        vec![Fix::new(
            None,
            "style",
            format!("cannot store the style: {e}"),
        )]
    })?;
    let options_json = serde_json::to_value(&options).map_err(|e| {
        vec![Fix::new(
            None,
            "options",
            format!("cannot store the options: {e}"),
        )]
    })?;
    let intent_json = serde_json::to_value(&intent).map_err(|e| {
        vec![Fix::new(
            None,
            "story",
            format!("cannot store the intent: {e}"),
        )]
    })?;
    let mut key_input = json!({
        "intent": intent_json,
        "style": style_json,
        "options": options_json,
        "assets": prepared_images.as_ref().map(PreparedImages::fingerprint).unwrap_or(Value::Null),
    });
    // (0.23) Only a take other than 0 is part of the id, so every take-0
    // request keeps the id it had before takes existed.
    if take != 0 {
        key_input["take"] = json!(take);
    }
    // (0.23) The same for the music word: only a word other than `auto` is
    // part of the id.
    if music != MusicWord::Auto {
        key_input["music"] = json!(music.as_str());
    }
    let key = JobKey {
        input: key_input,
        engine: ctx.config.engine_version.clone(),
        voice: ctx.config.tts_model.clone(),
    };
    let extra: Vec<String> = images.background_line.iter().cloned().collect();
    let plan = finish_plan(beat_lines, extra, est, intent.beats.len());
    findings.sort_by_key(|(p, _)| *p);
    let mut finding_lines: Vec<String> = Vec::new();
    for (_, l) in findings {
        if !finding_lines.contains(&l) {
            finding_lines.push(l);
        }
    }
    Ok(Prepared {
        request: StoredRequest {
            profile,
            kind,
            story: story_json,
            style: style_json,
            format: format_name(intent.format).to_string(),
            assets: folder
                .as_ref()
                .map(|_| args.assets.clone().unwrap_or_default()),
            options: options.clone(),
            take,
            music,
        },
        intent,
        style,
        options,
        images: prepared_images,
        changed: notes.lines(),
        findings: finding_lines,
        plan,
        estimated_s: est,
        key,
    })
}

// ---------------------------------------------------------------------------
// revise
// ---------------------------------------------------------------------------

/// RFC 7396 JSON merge patch: objects merge key by key, `null` removes a
/// key, anything else replaces the target.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(p) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    if let Value::Object(t) = target {
        for (k, v) in p {
            if v.is_null() {
                t.remove(k);
            } else {
                merge_patch(t.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
    }
}

/// Lite fields a change may set.
const LITE_FIELDS: [&str; 12] = BEAT_FIELDS;

/// Apply `set` to a lite beat (raw JSON): known fields set (null clears),
/// unknown ignored with a note.
fn set_lite(beat: &mut Map<String, Value>, n: usize, set: &Map<String, Value>, notes: &mut Notes) {
    for (k, v) in set {
        let nk = norm_key(k);
        let field = LITE_FIELDS
            .iter()
            .find(|f| **f == nk)
            .copied()
            .or_else(|| beat_alias(&nk));
        match field {
            Some(f) if v.is_null() => {
                beat.remove(f);
            }
            Some(f) => {
                beat.insert(f.to_string(), v.clone());
            }
            None => notes.add(
                Prio::Ignored,
                Some(n),
                format!("change: ignored unknown field '{k}'"),
            ),
        }
    }
}

/// Apply `set` to a full-intent beat (raw JSON): say → narration, show →
/// statement, picture → the primary object's asset, keyword, energy.
fn set_intent(
    beat: &mut Map<String, Value>,
    n: usize,
    set: &Map<String, Value>,
    notes: &mut Notes,
) {
    for (k, v) in set {
        let nk = norm_key(k);
        let field = LITE_FIELDS
            .iter()
            .find(|f| **f == nk)
            .copied()
            .or_else(|| beat_alias(&nk))
            .unwrap_or("");
        match field {
            "say" => match text_of(v) {
                Some(t) => {
                    beat.insert("narration".into(), Value::String(t));
                }
                None => {
                    beat.remove("narration");
                }
            },
            "show" => match text_of(v) {
                Some(t) => {
                    beat.insert("statement".into(), Value::String(t));
                }
                None => notes.add(
                    Prio::Ignored,
                    Some(n),
                    "change: show cannot be cleared in a full intent",
                ),
            },
            "keyword" => match text_of(v) {
                Some(t) => {
                    beat.insert("keyword".into(), Value::String(t));
                }
                None => {
                    beat.remove("keyword");
                }
            },
            "energy" => match text_of(v).as_deref().and_then(energy_word) {
                Some(e) => {
                    beat.insert("energy".into(), json!(e));
                }
                None => {
                    beat.remove("energy");
                }
            },
            "picture" => {
                let is_object = beat
                    .get("primary")
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    == Some("object");
                match (is_object, text_of(v)) {
                    (true, Some(t)) => {
                        if let Some(Value::Object(p)) = beat.get_mut("primary") {
                            p.insert("asset".into(), Value::String(lite::snake(&t)));
                        }
                    }
                    _ => notes.add(
                        Prio::Ignored,
                        Some(n),
                        "change: picture only replaces an object primary (use patch)",
                    ),
                }
            }
            _ => notes.add(
                Prio::Ignored,
                Some(n),
                format!("change: '{k}' cannot be set on a full intent (use patch)"),
            ),
        }
    }
}

/// A new intent beat built from lite fields (`insert_after` on an intent).
fn intent_beat_from(
    set: &Map<String, Value>,
    n: usize,
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) -> Option<Value> {
    let raw = parse_beat(n, &Value::Object(set.clone()), notes, fixes);
    let beat: LiteBeat = serde_json::from_value(Value::Object(raw)).ok()?;
    let story = LiteStory {
        title: "beat".into(),
        beats: vec![beat],
    };
    let mapped = lite::to_intent(&story, Format::Vertical, &NoPictures);
    mapped
        .intent
        .beats
        .into_iter()
        .next()
        .and_then(|b| serde_json::to_value(b).ok())
}

struct NoPictures;

impl lite::Pictures for NoPictures {
    fn is_picture(&self, _noun: &str) -> bool {
        false
    }
}

/// Apply `changes` in order. Beat numbers refer to the stored story.
fn apply_changes(
    story: &mut Value,
    kind: StoryKind,
    changes: &[BeatChange],
    notes: &mut Notes,
    fixes: &mut Vec<Fix>,
) {
    let Some(beats) = story.get_mut("beats").and_then(Value::as_array_mut) else {
        if !changes.is_empty() {
            fixes.push(Fix::new(None, "changes", "the stored story has no beats"));
        }
        return;
    };
    let len = beats.len();
    // (original 1-based number or None for inserted beats, beat JSON)
    let mut work: Vec<(Option<usize>, Value)> = beats
        .drain(..)
        .enumerate()
        .map(|(i, b)| (Some(i + 1), b))
        .collect();
    let range = |what: &str, got: usize, lo: usize| {
        Fix::new(None, "changes", format!("{what} {got} is out of range"))
            .otherwise(format!("use {lo} to {len}"))
    };
    for c in changes {
        if let Some(after) = c.insert_after {
            if after > len {
                fixes.push(range("insert_after", after, 0));
                continue;
            }
            if c.beat.is_some() || c.remove {
                notes.add(
                    Prio::Ignored,
                    None,
                    "change: beat/remove ignored with insert_after",
                );
            }
            let new_beat = match kind {
                StoryKind::Lite => {
                    let mut m = Map::new();
                    set_lite(&mut m, after + 1, &c.set, notes);
                    Some(Value::Object(m))
                }
                StoryKind::Intent => intent_beat_from(&c.set, after + 1, notes, fixes),
            };
            let Some(new_beat) = new_beat else {
                continue;
            };
            // After the original beat `after` and anything inserted after it.
            let mut pos = if after == 0 {
                0
            } else {
                match work.iter().position(|(o, _)| *o == Some(after)) {
                    Some(p) => p + 1,
                    None => {
                        fixes.push(Fix::new(
                            None,
                            "changes",
                            format!("beat {after} was removed by an earlier change"),
                        ));
                        continue;
                    }
                }
            };
            while pos < work.len() && work[pos].0.is_none() {
                pos += 1;
            }
            work.insert(pos, (None, new_beat));
            continue;
        }
        let Some(n) = c.beat else {
            fixes.push(
                Fix::new(None, "changes", "a change needs beat or insert_after")
                    .otherwise(format!("use beat 1 to {len}, or insert_after 0 to {len}")),
            );
            continue;
        };
        if n == 0 || n > len {
            fixes.push(range("beat", n, 1));
            continue;
        }
        let Some(pos) = work.iter().position(|(o, _)| *o == Some(n)) else {
            fixes.push(Fix::new(
                Some(n),
                "changes",
                "this beat was removed by an earlier change",
            ));
            continue;
        };
        if c.remove {
            work.remove(pos);
            continue;
        }
        let beat = &mut work[pos].1;
        if !beat.is_object() {
            *beat = json!({ "say": beat.as_str().unwrap_or_default() });
        }
        if let Value::Object(m) = beat {
            match kind {
                StoryKind::Lite => set_lite(m, n, &c.set, notes),
                StoryKind::Intent => set_intent(m, n, &c.set, notes),
            }
        }
    }
    *beats = work.into_iter().map(|(_, b)| b).collect();
}

/// Apply a `revise_video` call to a stored request, then [`prepare`] it.
/// Beat numbers in `changes` refer to the stored story; removals and
/// insertions are applied in the order given.
pub fn revise(
    stored: &StoredRequest,
    args: &ReviseVideoArgs,
    ctx: &Context,
) -> Result<Prepared, Vec<Fix>> {
    let mut notes = Notes::default();
    let mut fixes: Vec<Fix> = Vec::new();
    let mut story = stored.story.clone();
    if let Some(patch) = &args.patch {
        if ctx.config.profile.full_control() {
            merge_patch(&mut story, patch);
        } else {
            notes.add(
                Prio::Ignored,
                None,
                "patch ignored (not offered to this model)",
            );
        }
    }
    apply_changes(
        &mut story,
        stored.kind,
        &args.changes,
        &mut notes,
        &mut fixes,
    );
    if !fixes.is_empty() {
        return Err(fixes);
    }
    let inherited: Option<StyleProfile> = serde_json::from_value(stored.style.clone()).ok();
    let make = MakeVideoArgs {
        story,
        style: args.style.clone(),
        brand: args.brand.clone(),
        format: args.format.clone().or_else(|| Some(stored.format.clone())),
        assets: stored.assets.clone(),
        // (0.23) A given take (or music word) replaces the job's; without one
        // `prepare_inner` keeps `stored.take` / `stored.music`.
        take: args.take.clone(),
        music: args.music.clone(),
        mode: args.mode,
        strict: args.strict,
        options: Some(
            args.options
                .clone()
                .unwrap_or_else(|| stored.options.clone()),
        ),
        describe: false,
        force: false,
    };
    prepare_inner(
        &make,
        ctx,
        Some(inherited.unwrap_or_default()),
        stored.take,
        stored.music,
        notes,
    )
}

/// Estimated spoken duration of an intent (narration, else statement).
pub fn estimate_duration_s(intent: &CreativeIntent) -> f64 {
    let words: usize = intent
        .beats
        .iter()
        .map(|b| {
            b.narration
                .as_deref()
                .unwrap_or(&b.statement)
                .split_whitespace()
                .count()
        })
        .sum();
    words as f64 / WORDS_PER_SECOND + intent.beats.len() as f64 * BEAT_PAUSE_S
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_shorten_at_clauses() {
        assert_eq!(
            shorten_title("First, there's the cue — a trigger that tells"),
            "First, there's the cue"
        );
        assert_eq!(
            shorten_title("The habit loop is a simple three-part cycle that explains"),
            "The habit loop is a simple"
        );
        assert_eq!(
            shorten_title("Costs fell while output tripled in the end"),
            "Costs fell while output tripled"
        );
        assert_eq!(
            shorten_title("Warren Buffett is sitting on a mountain of cash"),
            "Warren Buffett is sitting"
        );
        assert_eq!(shorten_title("Short title"), "Short title");
    }

    #[test]
    fn notes_merge_and_order() {
        let mut n = Notes::default();
        n.add(Prio::Cosmetic, Some(1), "number 3 read as text '3'");
        for (b, w) in [(2, "a"), (3, "b"), (4, "c"), (5, "d")] {
            n.group(
                Prio::Text,
                b,
                "text",
                format!("picture '{w}' shown as text"),
                "pictures shown as text",
                Some(w.into()),
            );
        }
        n.add(Prio::Swap, Some(2), "picture 'x' → 'y'");
        assert_eq!(
            n.lines(),
            vec![
                "beat 2: picture 'x' → 'y'".to_string(),
                "beats 2-5: pictures shown as text: a, b, c, d".to_string(),
                "beat 1: number 3 read as text '3'".to_string(),
            ]
        );
    }

    #[test]
    fn merge_patch_follows_rfc7396() {
        let mut t = json!({"a": "b", "c": {"d": "e", "f": "g"}});
        merge_patch(&mut t, &json!({"a": "z", "c": {"f": null}}));
        assert_eq!(t, json!({"a": "z", "c": {"d": "e"}}));
        let mut t = json!({"a": [1]});
        merge_patch(&mut t, &json!({"a": [2, 3], "b": {"x": 1}}));
        assert_eq!(t, json!({"a": [2, 3], "b": {"x": 1}}));
    }
}
