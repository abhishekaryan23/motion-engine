//! MotionEngine 2 bridge (Phase 4): a short brief or a v1 CreativeIntent
//! becomes a validated [`MotionProgram`] on a beat grid.
//!
//! The brief is **direction, not script**: lines that say how the video
//! should feel ("make a dynamic 15 s showreel, go all out") set length,
//! aspect and recipe and never reach the screen; only content (quoted text,
//! numbers, plain statements) does. The planner ([`plan`]) lays a bar plan
//! on the music's beat grid: bar 1 the hook, one technique per bar, the last
//! bar the peak, an end title on the final downbeat. Everything the gallery
//! prompts leave unsaid (beat clock, an event on every beat, no hold over a
//! beat or two, one accent, a carried dot, no crossfade exits, no free
//! decoration) is a recipe default here. Deterministic: same input, same
//! program bytes.

pub mod flow;
pub mod plan;

use motion_core::intent::{CollectionItem, Subject};
use motion_core::CreativeIntent;
use motion_program::program::Aspect;
use motion_program::MotionProgram;
use serde::Deserialize;

pub use plan::{plan, Bar, Clock, ReelPlan, Technique};

/// Make-recipes by name: `(name, json)`.
pub const RECIPES: [(&str, &str); 6] = [
    (
        "showreel",
        include_str!("../../../assets/recipes/make/showreel.json"),
    ),
    (
        "kinetic_poster",
        include_str!("../../../assets/recipes/make/kinetic_poster.json"),
    ),
    (
        "chart_reveal",
        include_str!("../../../assets/recipes/make/chart_reveal.json"),
    ),
    (
        "ledger_ink",
        include_str!("../../../assets/recipes/make/ledger_ink.json"),
    ),
    (
        "dot_story",
        include_str!("../../../assets/recipes/make/dot_story.json"),
    ),
    (
        "top5_countdown",
        include_str!("../../../assets/recipes/make/top5_countdown.json"),
    ),
];

/// A make-recipe: a look (style recipe or palette/fonts), a beat-led music
/// track and the presets each technique uses. Defaults encode the gallery
/// craft rules (see `docs/DESIGN_RULES.md`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MakeRecipe {
    pub description: String,
    #[serde(default)]
    pub style: Option<serde_json::Value>,
    #[serde(default)]
    pub palette: Option<serde_json::Value>,
    #[serde(default)]
    pub fonts: Option<serde_json::Value>,
    #[serde(default)]
    pub tempo: Option<String>,
    /// Bundled track whose analysis is the master clock (120–126 BPM).
    pub music: String,
    /// Full-frame dot field under everything (`grid`, `halftone`, `rings`).
    #[serde(default)]
    pub field: Option<String>,
    /// Kinetic preset for hook and peak (`poster_cuts`, `glyph_drop`).
    pub poster: String,
    /// Kinetic preset for captions and body words.
    pub body: String,
    /// Chart preset (`thin_axis`, `paper`).
    pub chart: String,
    /// Diagram preset (`outline`, `cards`, `rail`).
    pub diagram: String,
    /// Techniques for the middle bars, cycled in order.
    pub techniques: Vec<Technique>,
}

/// The recipe called `name`.
pub fn recipe(name: &str) -> Result<MakeRecipe, String> {
    let (_, text) = RECIPES
        .iter()
        .find(|(n, _)| *n == name)
        .ok_or_else(|| format!("unknown recipe {name}; use one of {}", names()))?;
    serde_json::from_str(text).map_err(|e| format!("recipe {name}: {e}"))
}

pub fn names() -> String {
    RECIPES.map(|(n, _)| n).join(", ")
}

/// One idea from the brief's content.
#[derive(Debug, Clone, PartialEq)]
pub enum Section {
    /// Words to show, cut one per beat; `stress` indices in the accent.
    Words {
        words: Vec<String>,
        stress: Vec<usize>,
    },
    /// Numbers: a chart and its claim headline (derived, never copied).
    Stat {
        values: Vec<f64>,
        labels: Vec<String>,
        callout: Option<String>,
        /// What the numbers say, in our words ("UP 137%", "8 IN 10").
        claim: String,
    },
}

/// What the brief asks for: content to show plus direction (never shown).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Content {
    pub name: String,
    /// Line 1 when it is content: names the output, never shown (only
    /// quoted text reaches the screen verbatim).
    pub title: Option<String>,
    /// The brief itself (set as `meta.brief` for the core echo check).
    pub brief: Option<String>,
    /// Content lines that were not quoted and had no numbers: not shown.
    pub unshown: usize,
    /// A product / UI flow the brief describes (structured fields only).
    pub flow: Option<flow::FlowSpec>,
    pub sections: Vec<Section>,
    pub duration: Option<f64>,
    pub aspect: Option<Aspect>,
    pub recipe: Option<String>,
    /// Lowercase words (4+ letters) of the brief outside quotes: banned from
    /// the screen (prompt echo).
    pub direction_words: Vec<String>,
    /// Hash of the brief: picks bank variants.
    pub seed: u64,
}

/// Words that mark a line as direction (how the video should be made).
const DIRECTION_CUES: &[&str] = &[
    "make",
    "create",
    "build",
    "design",
    "render",
    "produce",
    "generate",
    "video",
    "reel",
    "showreel",
    "promo",
    "trailer",
    "animation",
    "animated",
    "motion",
    "dynamic",
    "energetic",
    "cinematic",
    "style",
    "vibe",
    "prove",
    "showcase",
    "show",
    "go",
    "all",
    "out",
    "seconds",
    "second",
    "sec",
    "minute",
    "vertical",
    "landscape",
    "square",
    "format",
    "please",
    "designer",
    "professional",
    "aesthetic",
    "modern",
    "sleek",
    "bold",
    "fun",
    "punchy",
    "explainer",
    "short",
    "shorts",
    "tiktok",
    "instagram",
    "youtube",
    "music",
    "beat",
    "recipe",
];

fn words_of(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '%' && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A line is direction when it has no quote and no number-as-data, and at
/// least two of its words (or a third of them) are direction cues.
fn is_direction(line: &str) -> bool {
    if line.contains('"') || line.contains('“') {
        return false;
    }
    let w = words_of(line);
    let cues = w
        .iter()
        .filter(|x| DIRECTION_CUES.contains(&x.as_str()))
        .count();
    let data = numbers(line)
        .iter()
        .filter(|(_, tok)| !is_length_token(tok, line))
        .count();
    data == 0 && cues >= 2 && cues * 3 >= w.len().min(9)
}

fn is_length_token(tok: &str, line: &str) -> bool {
    let t = tok.to_lowercase();
    t.ends_with('s') && t.trim_end_matches('s').parse::<f64>().is_ok()
        || line.to_lowercase().contains(&format!("{t} s"))
        || line.to_lowercase().contains(&format!("{t} sec"))
        || line.to_lowercase().contains(&format!("{t}-second"))
}

/// Quoted spans (`"…"` or `“…”`).
fn quoted(line: &str) -> Vec<String> {
    let norm = line.replace(['“', '”'], "\"");
    norm.split('"')
        .skip(1)
        .step_by(2)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a brief: settings (length, aspect, recipe) from anywhere; content
/// from quoted text and from the lines that are not direction. Line 1 is the
/// title when it is content.
pub fn parse_brief(brief: &str) -> Content {
    let mut lines: Vec<String> = brief
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if lines.len() == 1 {
        lines = split_sentences(&lines[0]);
    }
    let mut c = Content {
        seed: fnv(brief),
        brief: Some(brief.to_string()),
        ..Content::default()
    };
    for (i, line) in lines.iter().enumerate() {
        // Length, aspect and recipe come from line 1 or a direction line;
        // "settles in 2 seconds" in content is data, not a length.
        let setting_line = i == 0 || is_direction(line);
        if setting_line {
            settings(line, &mut c);
        }
        let q = quoted(line);
        if !q.is_empty() {
            for s in q {
                c.sections.push(section_of(&s, &[]));
            }
            continue;
        }
        let text = if setting_line {
            strip_settings(line)
        } else {
            line.clone()
        };
        if c.flow.is_none() {
            if let Some(f) = flow::detect(&text) {
                c.flow = Some(f);
                continue;
            }
        }
        if text.trim().is_empty() || is_direction(line) {
            continue;
        }
        if numbers(&text).is_empty() {
            // Plain prose: line 1 names the piece; others are context the
            // engine cannot restate without a model, so they stay off screen.
            if i == 0 && lines.len() > 1 {
                c.title = Some(clip(text.trim_end_matches(['.', ',', ':']), 24));
            } else {
                c.unshown += 1;
            }
            continue;
        }
        c.sections.push(section_of(&text, &[]));
    }
    // Every brief word outside quotes is banned from the screen.
    c.direction_words = words_of(&strip_quotes(brief))
        .into_iter()
        .filter(|w| w.chars().count() >= 4)
        .collect();
    c.direction_words.sort();
    c.direction_words.dedup();
    c.name = file_name(c.title.as_deref().unwrap_or(match &c.recipe {
        Some(r) => r,
        None => "reel",
    }));
    c
}

fn split_sentences(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for i in 0..chars.len() {
        let ch = chars[i];
        cur.push(ch);
        let decimal = ch == '.' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if matches!(ch, '.' | '!' | '?') && !decimal {
            let trimmed = cur.trim().to_string();
            if !trimmed.is_empty() {
                out.push(trimmed);
            }
            cur.clear();
        }
    }
    let tail = cur.trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn strip_quotes(line: &str) -> String {
    let norm = line.replace(['“', '”'], "\"");
    norm.split('"').step_by(2).collect::<Vec<_>>().join(" ")
}

/// Length (`60 s`, `15s`, `15-second`), aspect and recipe name.
fn settings(line: &str, c: &mut Content) {
    let toks: Vec<String> = line
        .split_whitespace()
        .map(|t| {
            t.trim_matches(|ch: char| ",;.()".contains(ch))
                .to_lowercase()
        })
        .collect();
    for (i, t) in toks.iter().enumerate() {
        let next = toks.get(i + 1).map(String::as_str);
        let secs = t
            .strip_suffix("-second")
            .or_else(|| t.strip_suffix('s'))
            .and_then(|n| n.parse::<f64>().ok())
            .or_else(|| {
                matches!(next, Some("s" | "sec" | "secs" | "second" | "seconds"))
                    .then(|| t.parse().ok())?
            });
        if let Some(s) = secs {
            c.duration = Some(s);
        } else if let Some(a) = parse_aspect(t) {
            c.aspect = Some(a);
        } else if t == "vertical" {
            c.aspect = Some(Aspect::Portrait);
        } else if RECIPES.iter().any(|(n, _)| n == t) {
            c.recipe = Some(t.clone());
        }
    }
}

fn strip_settings(line: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let toks: Vec<&str> = line.split_whitespace().collect();
    let mut skip_next = false;
    for (i, raw) in toks.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        let t = raw
            .trim_matches(|ch: char| ",;.()".contains(ch))
            .to_lowercase();
        let next = toks.get(i + 1).map(|n| n.trim_matches(',').to_lowercase());
        let is_secs = t
            .strip_suffix('s')
            .is_some_and(|n| n.parse::<f64>().is_ok())
            || t.strip_suffix("-second")
                .is_some_and(|n| n.parse::<f64>().is_ok());
        if t.parse::<f64>().is_ok()
            && matches!(
                next.as_deref(),
                Some("s" | "sec" | "secs" | "second" | "seconds")
            )
        {
            skip_next = true;
            continue;
        }
        if is_secs || parse_aspect(&t).is_some() || RECIPES.iter().any(|(n, _)| *n == t) {
            continue;
        }
        out.push(raw);
    }
    out.join(" ").trim_matches([',', ' ']).to_string()
}

fn parse_aspect(s: &str) -> Option<Aspect> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

/// A CreativeIntent as content: each beat becomes one section, the title the
/// end title. Intents are content by construction (no direction lines).
pub fn from_intent(intent: &CreativeIntent) -> Content {
    let sections = intent
        .beats
        .iter()
        .map(|b| {
            let stress: Vec<&str> = b.keyword.as_deref().into_iter().collect();
            match &b.primary {
                Subject::Number(a) => match a.value.as_deref().and_then(first_number) {
                    Some(v) => Section::Stat {
                        values: vec![0.0, v],
                        labels: Vec::new(),
                        callout: a.value.as_deref().map(|s| clip(s, 12)),
                        claim: clip(&b.statement.to_uppercase(), 24),
                    },
                    None => section_of(&b.statement, &stress),
                },
                Subject::Collection(c) => {
                    let mut values = Vec::new();
                    let mut labels = Vec::new();
                    for it in &c.items {
                        if let CollectionItem::Number(a) = it {
                            if let Some(v) = a.value.as_deref().and_then(first_number) {
                                values.push(v);
                                labels.push(clip(a.meaning.as_deref().unwrap_or(""), 10));
                            }
                        }
                    }
                    if values.len() >= 2 {
                        let hero = max_index(&values);
                        if labels.iter().any(String::is_empty) {
                            labels.clear();
                        }
                        Section::Stat {
                            callout: Some(clip(&fmt_num(values[hero]), 12)),
                            values,
                            labels,
                            claim: clip(&b.statement.to_uppercase(), 24),
                        }
                    } else {
                        section_of(&b.statement, &stress)
                    }
                }
                Subject::StateChange(s) => Section::Words {
                    words: [s.from.as_str(), "to", s.to.as_str()]
                        .iter()
                        .map(|w| clip(&w.to_uppercase(), 24))
                        .collect(),
                    stress: vec![2],
                },
                _ => section_of(&b.statement, &stress),
            }
        })
        .collect();
    let title = intent.title.replace(['_', '-'], " ");
    Content {
        name: file_name(&intent.title),
        title: Some(clip(&title, 24)),
        sections,
        duration: None,
        aspect: Some(match intent.format {
            motion_core::intent::Format::Vertical => Aspect::Portrait,
            motion_core::intent::Format::Square => Aspect::Square,
            motion_core::intent::Format::Landscape => Aspect::Landscape,
        }),
        recipe: None,
        direction_words: Vec::new(),
        seed: fnv(&intent.title),
        brief: None,
        unshown: 0,
        flow: None,
    }
}

/// Upper-case words for kinetic type (sentence punctuation dropped).
fn kinetic_words(text: &str, max: usize) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            clip(
                &w.trim_matches(|c: char| ",;:.!?\"“”()".contains(c))
                    .to_uppercase(),
                24,
            )
        })
        .filter(|w| !w.is_empty())
        .take(max)
        .collect()
}

fn section_of(text: &str, stress_words: &[&str]) -> Section {
    let nums = numbers(text);
    if !nums.is_empty() {
        let toks: Vec<String> = nums.iter().map(|(_, t)| t.clone()).collect();
        let values: Vec<f64> = nums.iter().map(|(v, _)| *v).take(24).collect();
        let (values, hero) = if values.len() == 1 {
            (vec![0.0, values[0]], 0)
        } else {
            let h = max_index(&values);
            (values, h)
        };
        return Section::Stat {
            callout: Some(clip(&number_only(&toks[hero]), 12)),
            claim: claim(&values, &toks),
            values,
            labels: Vec::new(),
        };
    }
    let words = kinetic_words(text, 40);
    let stress = words
        .iter()
        .enumerate()
        .filter(|(_, w)| stress_words.iter().any(|s| w.contains(&s.to_uppercase())))
        .map(|(i, _)| i)
        .collect();
    Section::Words { words, stress }
}

/// A token's number with its sign/unit symbols (`85%`, `$4`, `90`), no words.
fn number_only(tok: &str) -> String {
    tok.chars()
        .filter(|c| c.is_ascii_digit() || "%$€£¥¢.,+-×x".contains(*c))
        .collect::<String>()
        .trim_matches(['.', ','])
        .to_string()
}

/// The claim a stat makes, in the engine's own words: a share as "ONLY 7%"
/// or "8 IN 10", a series as its change ("UP 2.4×", "UP 37%", "DOWN 40%").
/// Short on purpose: a claim is set huge and held.
pub fn claim(values: &[f64], toks: &[String]) -> String {
    let pct = toks.iter().any(|t| t.contains('%'));
    if values.len() == 2 && values[0] == 0.0 {
        let v = values[1];
        let tok = number_only(toks.first().map(String::as_str).unwrap_or(""));
        return if pct && v <= 20.0 {
            format!("ONLY {tok}")
        } else if pct && v >= 60.0 {
            // Never over-claim: 85% is "NEARLY 9 IN 10", not "9 IN 10".
            let tens = v / 10.0;
            if tens.fract() >= 0.5 {
                format!("NEARLY {} IN 10", tens.ceil() as i64)
            } else {
                format!("{} IN 10", tens.floor() as i64)
            }
        } else if pct && (40.0..60.0).contains(&v) {
            format!("HALF: {tok}")
        } else if pct {
            format!("1 IN {}", (100.0 / v).round() as i64)
        } else {
            tok
        };
    }
    let (first, last) = (values[0], values[values.len() - 1]);
    if first.abs() < f64::EPSILON {
        return format!("TO {}", fmt_num(last));
    }
    let ratio = last / first;
    let change = (ratio - 1.0) * 100.0;
    if ratio >= 2.0 {
        format!("UP {:.1}×", ratio).replace(".0×", "×")
    } else if change >= 5.0 {
        format!("UP {:.0}%", change)
    } else if change <= -5.0 {
        format!("DOWN {:.0}%", -change)
    } else {
        "FLAT".into()
    }
}

fn numbers(text: &str) -> Vec<(f64, String)> {
    text.split_whitespace()
        .filter_map(|w| {
            let w = w.trim_matches(|c: char| ",;:.()".contains(c));
            // A bare year ("by 2050") dates the claim; it is not a data point.
            let year = w.len() == 4
                && w.chars().all(|c| c.is_ascii_digit())
                && (1800..=2200).contains(&w.parse::<u32>().unwrap_or(0));
            first_number(w)
                .filter(|_| !year)
                .map(|v| (v, w.to_string()))
        })
        .collect()
}

fn first_number(s: &str) -> Option<f64> {
    let digits: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .filter(|c| *c != ',')
        .collect();
    digits.trim_end_matches('.').parse().ok()
}

fn max_index(v: &[f64]) -> usize {
    (0..v.len()).fold(0, |m, i| if v[i] > v[m] { i } else { m })
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect::<String>().trim().to_string()
}

fn file_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out
        .trim_end_matches('_')
        .chars()
        .take(40)
        .collect::<String>();
    if out.is_empty() {
        "video".into()
    } else {
        out
    }
}

pub(crate) fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// The recipe a content asks for: explicit, else named in the brief, else
/// `showreel` for pure direction, `chart_reveal` when most sections are
/// numbers, `kinetic_poster` otherwise.
pub fn pick_recipe(c: &Content, explicit: Option<&str>) -> String {
    if let Some(n) = explicit.or(c.recipe.as_deref()) {
        return n.to_string();
    }
    let stats = c
        .sections
        .iter()
        .filter(|s| matches!(s, Section::Stat { .. }))
        .count();
    if c.sections.is_empty() {
        "showreel".into()
    } else if stats * 2 > c.sections.len() {
        "chart_reveal".into()
    } else {
        "kinetic_poster".into()
    }
}

/// Words shown on screen that only the direction used (prompt echo).
pub fn prompt_echo(c: &Content, p: &MotionProgram) -> Vec<String> {
    let json = p.to_json();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
    let mut shown = Vec::new();
    collect_text(&v, &mut shown);
    let mut echo: Vec<String> = shown
        .iter()
        .flat_map(|s| words_of(s))
        .filter(|w| w.chars().count() >= 4 && c.direction_words.contains(w))
        .collect();
    echo.sort();
    echo.dedup();
    echo
}

fn collect_text(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x) {
                    ("text" | "callout", serde_json::Value::String(s)) => out.push(s.clone()),
                    ("words" | "nodes" | "labels" | "corners", serde_json::Value::Array(a)) => {
                        out.extend(a.iter().filter_map(|s| s.as_str().map(String::from)))
                    }
                    _ => collect_text(x, out),
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_text(x, out)),
        _ => {}
    }
}

/// Brief → program on `clock`. `variant` picks another concept (a retry).
pub fn make_from_brief(
    brief: &str,
    recipe_name: Option<&str>,
    aspect: Option<Aspect>,
    duration: Option<f64>,
    clock: &Clock,
    variant: u64,
) -> Result<(Content, ReelPlan, MotionProgram), String> {
    let c = parse_brief(brief);
    let r = recipe(&pick_recipe(&c, recipe_name))?;
    let plan = plan(&c, &r, clock, duration, variant);
    let p = plan::realize(&c, &r, &plan, clock, aspect.or(c.aspect))?;
    Ok((c, plan, p))
}

/// Intent → program on `clock`.
pub fn make_from_intent(
    intent: &CreativeIntent,
    recipe_name: Option<&str>,
    aspect: Option<Aspect>,
    duration: Option<f64>,
    clock: &Clock,
    variant: u64,
) -> Result<(Content, ReelPlan, MotionProgram), String> {
    let c = from_intent(intent);
    let r = recipe(&pick_recipe(&c, recipe_name))?;
    let plan = plan(&c, &r, clock, duration, variant);
    let p = plan::realize(&c, &r, &plan, clock, aspect.or(c.aspect))?;
    Ok((c, plan, p))
}
