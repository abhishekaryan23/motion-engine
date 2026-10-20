//! (0.24 M1) Anti-slop QA: does a compiled project read as generic AI output?
//!
//! Pure and deterministic: [`slop_report`] checks each beat scene against the
//! editable ban lists in `assets/taste/slop.json` ([`SlopRules`]) and
//! [`critique`] maps those findings plus the [`layout_report`] findings onto
//! eight scored axes. Decorative layers are exempt via
//! [`layout_qa::is_decorative`]. A check whose input the scene does not carry
//! is listed in [`SlopReport::not_measurable`] instead of being guessed.
//!
//! | check | rule |
//! |---|---|
//! | `slop_stock_phrase` | a text layer contains a banned phrase |
//! | `slop_emoji_bullet` | a text line starts with an emoji |
//! | `slop_ai_palette` | purple+blue pair, neon cyan on near-black, or a pure #000/#fff ground |
//! | `slop_uniform_entrance` | >= 3 layers enter within one frame, or all share one easing and one entry kind |
//! | `slop_no_hold` | text is fully in for less than 0.3 s + 0.06 s/word before the beat ends |
//! | `slop_fx_stack` | more than `max_fx_per_beat` post effects in a beat |
//! | `slop_repeated_grammar` | consecutive beats share a layout grammar |
//! | `slop_showy_transitions` | wipe/glitch transitions exceed the ratio |
//! | `slop_family_sprawl` | more than `max_font_families` font families |
//! | `slop_accent_overuse` | (0.24 M9) more than one full-frame colour field, or more than one accent block / stamp per three beats |
//! | `slop_flicker` | (0.24 M10) a `pulse`, `glitch` or `echo` op is present, or more than one camera shake in the video |
//! | `slop_skeleton_lines` | (0.24 M9) a cluster of thin equal bars that pretends to be lines of text but covers none |
//! | `slop_things_appear` | (0.24 Wave 1) a text layer's only entrance is a fade: it appears, it does not arrive (no mask line, rise, slide or scale) |
//! | `slop_boxy` | (0.24 V1) more than one rect / card behind text in a beat, or more than one decorative rule |
//! | `slop_eyebrow_label` | (0.24) a <= 3-word, small text layer near the top with a thin rule just below it: the eyebrow kicker |
//!
//! **Grammar.** A scene does not record its layout grammar unless the
//! direction record is present (`project.direction.beats[i].template`). Without
//! it the check uses a layer signature: the sorted set of non-decorative
//! layer names (id below the beat prefix) with their kinds.
//! **Families.** The scene carries font assets, not families; the family is
//! the font file stem up to the first `-` (`Inter-Bold.ttf` -> `inter`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::audio::beat_scenes;
use crate::compiler::layout_frame::LayoutFrame;
use crate::layout_qa::{is_decorative, layout_report, LayoutCheck, LayoutFinding, LayoutVerdict};
use crate::scene::{Color, Layer, LayerKind, Motion, MotionOp, MotionProject, PostKind, Scene};
use crate::style::StyleProfile;

const DEFAULT_RULES: &str = include_str!("../../../assets/taste/slop.json");

/// A hue window plus minimum saturation / value (HSV, hue in degrees).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HueRange {
    pub hue_min: f32,
    pub hue_max: f32,
    pub min_sat: f32,
    pub min_val: f32,
}

impl HueRange {
    fn hits(&self, c: Color) -> bool {
        if c.a < 128 {
            return false;
        }
        let (h, s, v) = hsv(c);
        h >= self.hue_min && h <= self.hue_max && s >= self.min_sat && v >= self.min_val
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiPalette {
    pub purple: HueRange,
    pub blue: HueRange,
    pub neon_cyan: HueRange,
    /// Relative luma at or under which a ground counts as near-black.
    pub near_black_max_luma: f32,
    pub flag_pure_grounds: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlopThresholds {
    pub max_fx_per_beat: u32,
    /// Layers entering within `max(1 frame, this)` of each other are "together".
    pub min_stagger_ms: u32,
    pub max_showy_transition_ratio: f32,
    pub max_font_families: u32,
    /// (0.24) `slop_eyebrow_label`.
    #[serde(default)]
    pub eyebrow: EyebrowRule,
    /// (0.24 M9) `slop_accent_overuse`.
    #[serde(default)]
    pub accent: AccentRule,
}

/// (0.24 M9) The accent budget (`compiler::accent_budget` spends it, this
/// measures it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccentRule {
    /// Full-frame colour fields in the whole video.
    pub max_fields: usize,
    /// One accent block (or stamp) per this many beats, at least one.
    pub beats_per_block: usize,
    /// A rectangle covering this share of the canvas width is a field ...
    pub full_frame: f32,
    /// ... when it also covers this share of the height (a caption lane
    /// shortens the layout canvas a cut's field fills).
    pub min_height: f32,
}

impl Default for AccentRule {
    fn default() -> Self {
        AccentRule {
            max_fields: 1,
            beats_per_block: 3,
            full_frame: 0.9,
            min_height: 0.75,
        }
    }
}

/// (0.24) The eyebrow-kicker pattern: a short, small label near the top with a
/// thin rule right under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EyebrowRule {
    pub max_words: usize,
    /// Cap height (0.7 x font size) at most this share of the canvas height.
    pub max_cap_height: f32,
    /// The label sits within this share of the canvas height from the top.
    pub top_band: f32,
    /// The rule is at most this many units tall (1 u = 1/1080 of the short side).
    pub max_rule_u: f32,
    /// The rule starts within this share of the canvas height below the label.
    pub rule_gap: f32,
}

impl Default for EyebrowRule {
    fn default() -> Self {
        EyebrowRule {
            max_words: 3,
            max_cap_height: 0.035,
            top_band: 0.14,
            max_rule_u: 8.0,
            rule_gap: 0.03,
        }
    }
}

/// The editable ban lists (`assets/taste/slop.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlopRules {
    /// Lowercase substrings.
    pub stock_phrases: Vec<String>,
    pub ai_palette: AiPalette,
    pub thresholds: SlopThresholds,
}

impl SlopRules {
    /// The bundled `assets/taste/slop.json`.
    pub fn bundled() -> SlopRules {
        // The bundled file is covered by a test; the fallback keeps this total.
        serde_json::from_str(DEFAULT_RULES).unwrap_or_else(|_| SlopRules::fallback())
    }

    pub fn from_json(s: &str) -> Result<SlopRules, serde_json::Error> {
        serde_json::from_str(s)
    }

    fn fallback() -> SlopRules {
        let range = |lo, hi, s, v| HueRange {
            hue_min: lo,
            hue_max: hi,
            min_sat: s,
            min_val: v,
        };
        SlopRules {
            stock_phrases: vec!["game-changer".to_string()],
            ai_palette: AiPalette {
                purple: range(255.0, 295.0, 0.4, 0.35),
                blue: range(205.0, 255.0, 0.4, 0.35),
                neon_cyan: range(165.0, 200.0, 0.8, 0.85),
                near_black_max_luma: 0.12,
                flag_pure_grounds: true,
            },
            thresholds: SlopThresholds {
                max_fx_per_beat: 3,
                min_stagger_ms: 34,
                max_showy_transition_ratio: 0.34,
                max_font_families: 3,
                eyebrow: EyebrowRule::default(),
                accent: AccentRule::default(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlopCheck {
    StockPhrase,
    EmojiBullet,
    AiPalette,
    UniformEntrance,
    NoHold,
    FxStack,
    RepeatedGrammar,
    ShowyTransitions,
    FamilySprawl,
    EyebrowLabel,
    AccentOveruse,
    SkeletonLines,
    Flicker,
    ThingsAppear,
    Boxy,
}

impl SlopCheck {
    pub fn name(&self) -> &'static str {
        match self {
            SlopCheck::StockPhrase => "slop_stock_phrase",
            SlopCheck::EmojiBullet => "slop_emoji_bullet",
            SlopCheck::AiPalette => "slop_ai_palette",
            SlopCheck::UniformEntrance => "slop_uniform_entrance",
            SlopCheck::NoHold => "slop_no_hold",
            SlopCheck::FxStack => "slop_fx_stack",
            SlopCheck::RepeatedGrammar => "slop_repeated_grammar",
            SlopCheck::ShowyTransitions => "slop_showy_transitions",
            SlopCheck::FamilySprawl => "slop_family_sprawl",
            SlopCheck::EyebrowLabel => "slop_eyebrow_label",
            SlopCheck::AccentOveruse => "slop_accent_overuse",
            SlopCheck::SkeletonLines => "slop_skeleton_lines",
            SlopCheck::Flicker => "slop_flicker",
            SlopCheck::ThingsAppear => "slop_things_appear",
            SlopCheck::Boxy => "slop_boxy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlopFinding {
    pub scene: String,
    pub layer: String,
    pub check: SlopCheck,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlopReport {
    pub verdict: LayoutVerdict,
    pub findings: Vec<SlopFinding>,
    /// Checks skipped because the scene lacks their input, with the reason.
    pub not_measurable: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hsv(c: Color) -> (f32, f32, f32) {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

fn luma(c: Color) -> f32 {
    (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x2B50 | 0x2B55 | 0x2B06 | 0x2B07)
}

/// Layer name below the beat prefix (`b3.head.0` -> `head`).
fn layer_name(id: &str) -> &str {
    let mut segs = id.split('.');
    let first = segs.next().unwrap_or("");
    let prefixed =
        first.starts_with('b') && first.len() > 1 && first[1..].chars().all(|c| c.is_ascii_digit());
    if prefixed {
        segs.next().unwrap_or("")
    } else {
        first
    }
}

fn is_stage(id: &str) -> bool {
    id.ends_with(".stage") || id.ends_with(".stage_front")
}

struct Walked<'a> {
    layer: &'a Layer,
    /// Ids of the enclosing groups, outermost first.
    ancestors: Vec<&'a str>,
}

fn walk<'a>(layers: &'a [Layer], ancestors: &mut Vec<&'a str>, out: &mut Vec<Walked<'a>>) {
    for l in layers {
        out.push(Walked {
            layer: l,
            ancestors: ancestors.clone(),
        });
        if let LayerKind::Group { children } = &l.kind {
            ancestors.push(&l.id);
            walk(children, ancestors, out);
            ancestors.pop();
        }
    }
}

fn walk_scene(scene: &Scene) -> Vec<Walked<'_>> {
    let mut out = Vec::new();
    walk(&scene.layers, &mut Vec::new(), &mut out);
    out
}

/// Entry kind of a motion that brings a layer in, if it is one.
fn entry_kind(op: &MotionOp) -> Option<&'static str> {
    match op {
        MotionOp::Fade { from, to } if *from <= 0.5 && to > from => Some("fade"),
        MotionOp::Move { from, to, .. } if from[0].hypot(from[1]) > to[0].hypot(to[1]) + 1e-3 => {
            Some("move")
        }
        MotionOp::Scale { from, to, .. }
            if (*from - 1.0).abs() > (*to - 1.0).abs() + 1e-3 && *to > 0.5 =>
        {
            Some("scale")
        }
        MotionOp::MaskReveal { .. } => Some("mask_reveal"),
        MotionOp::ClipReveal { .. } => Some("clip_reveal"),
        MotionOp::GlyphCascade { .. } => Some("glyph_cascade"),
        _ => None,
    }
}

fn entry_motions(scene: &Scene) -> BTreeMap<&str, Vec<&Motion>> {
    let settle = scene.lifecycle.map_or(f64::MAX, |l| l.settle);
    let mut map: BTreeMap<&str, Vec<&Motion>> = BTreeMap::new();
    for m in &scene.motions {
        if m.start <= settle + 1e-9 && entry_kind(&m.op).is_some() {
            map.entry(m.target.as_str()).or_default().push(m);
        }
    }
    map
}

/// (0.24 Wave 1) Every entrance before ANTICIPATE (a delayed word cue enters
/// after SETTLE, which [`entry_motions`] ignores).
fn entries_until_anticipate(scene: &Scene) -> BTreeMap<&str, Vec<&Motion>> {
    let until = scene.lifecycle.map_or(f64::MAX, |l| l.anticipate);
    let mut map: BTreeMap<&str, Vec<&Motion>> = BTreeMap::new();
    for m in &scene.motions {
        if m.start < until && m.duration <= 1.2 && entry_kind(&m.op).is_some() {
            map.entry(m.target.as_str()).or_default().push(m);
        }
    }
    map
}

fn mk(scene: &str, layer: &str, check: SlopCheck, detail: String) -> SlopFinding {
    SlopFinding {
        scene: scene.to_string(),
        layer: layer.to_string(),
        check,
        detail,
    }
}

// ---------------------------------------------------------------------------
// Per-check logic
// ---------------------------------------------------------------------------

fn text_checks(scene: &Scene, rules: &SlopRules, out: &mut Vec<SlopFinding>) {
    for w in walk_scene(scene) {
        let LayerKind::Text(t) = &w.layer.kind else {
            continue;
        };
        if is_decorative(&w.layer.id, &w.layer.kind) {
            continue;
        }
        let lower = t.text.to_lowercase();
        if let Some(p) = rules
            .stock_phrases
            .iter()
            .find(|p| lower.contains(p.as_str()))
        {
            out.push(mk(
                &scene.id,
                &w.layer.id,
                SlopCheck::StockPhrase,
                format!("contains stock phrase \"{p}\""),
            ));
        }
        if t.text
            .lines()
            .any(|l| l.trim_start().chars().next().is_some_and(is_emoji))
        {
            out.push(mk(
                &scene.id,
                &w.layer.id,
                SlopCheck::EmojiBullet,
                "a text line starts with an emoji".to_string(),
            ));
        }
    }
}

/// (0.24) A small label of at most `max_words` words near the top of the
/// canvas with a thin rule right below it, in the same group: the eyebrow
/// kicker that every beat of a template repeats.
fn eyebrow_check(
    project: &MotionProject,
    scene: &Scene,
    rules: &SlopRules,
    out: &mut Vec<SlopFinding>,
) {
    let r = &rules.thresholds.eyebrow;
    let (cw, ch) = (project.canvas.width as f32, project.canvas.height as f32);
    let u = cw.min(ch) / 1080.0;
    let walked = walk_scene(scene);
    for w in &walked {
        let LayerKind::Text(t) = &w.layer.kind else {
            continue;
        };
        let l = w.layer;
        let words = t.text.split_whitespace().count();
        if words == 0
            || words > r.max_words
            || 0.7 * t.font_size > r.max_cap_height * ch
            || l.y > r.top_band * ch
        {
            continue;
        }
        let bottom = l.y + l.height;
        let rule = walked.iter().find(|o| {
            o.ancestors == w.ancestors
                && matches!(
                    o.layer.kind,
                    LayerKind::Rectangle { .. } | LayerKind::RoundedRectangle { .. }
                )
                && o.layer.height <= r.max_rule_u * u
                && o.layer.width >= 2.0 * o.layer.height
                && o.layer.y >= l.y
                && o.layer.y <= bottom + r.rule_gap * ch
        });
        if let Some(rule) = rule {
            out.push(mk(
                &scene.id,
                &l.id,
                SlopCheck::EyebrowLabel,
                format!(
                    "\"{}\" is an eyebrow label over rule {}",
                    t.text, rule.layer.id
                ),
            ));
        }
    }
}

fn palette_checks(
    project: &MotionProject,
    scene: &Scene,
    rules: &SlopRules,
    out: &mut Vec<SlopFinding>,
) {
    let pal = &rules.ai_palette;
    let (cw, ch) = (project.canvas.width as f32, project.canvas.height as f32);
    let mut colours: Vec<(String, Color)> = Vec::new();
    let mut grounds: Vec<(String, Color)> = vec![("canvas".to_string(), project.canvas.background)];
    for w in walk_scene(scene) {
        let l = w.layer;
        match &l.kind {
            LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => {
                if fill.a >= 200 {
                    colours.push((l.id.clone(), *fill));
                    if l.width >= 0.9 * cw && l.height >= 0.9 * ch {
                        grounds.push((l.id.clone(), *fill));
                    }
                }
            }
            LayerKind::Text(t) if !is_decorative(&l.id, &l.kind) => {
                colours.push((l.id.clone(), t.color));
            }
            _ => {}
        }
    }
    let first =
        |r: &HueRange, set: &[(String, Color)]| set.iter().find(|(_, c)| r.hits(*c)).cloned();
    if let (Some(p), Some(b)) = (first(&pal.purple, &colours), first(&pal.blue, &colours)) {
        out.push(mk(
            &scene.id,
            &p.0,
            SlopCheck::AiPalette,
            format!(
                "purple {} with blue {} ({}): the AI gradient pair",
                p.1.to_hex(),
                b.1.to_hex(),
                b.0
            ),
        ));
    }
    let dark = grounds
        .iter()
        .any(|(_, c)| luma(*c) <= pal.near_black_max_luma);
    if dark {
        if let Some(n) = first(&pal.neon_cyan, &colours) {
            out.push(mk(
                &scene.id,
                &n.0,
                SlopCheck::AiPalette,
                format!("neon cyan {} on a near-black ground", n.1.to_hex()),
            ));
        }
    }
    if pal.flag_pure_grounds {
        let pure = |c: &Color| {
            c.a >= 200 && ((c.r, c.g, c.b) == (0, 0, 0) || (c.r, c.g, c.b) == (255, 255, 255))
        };
        if let Some((id, c)) = grounds.iter().find(|(_, c)| pure(c)) {
            out.push(mk(
                &scene.id,
                id,
                SlopCheck::AiPalette,
                format!("pure {} ground", c.to_hex()),
            ));
        }
    }
}

fn entrance_check(
    project: &MotionProject,
    scene: &Scene,
    rules: &SlopRules,
    out: &mut Vec<SlopFinding>,
) {
    let entries = entry_motions(scene);
    // (layer id, entry start, key)
    let mut ent: Vec<(&str, f64, String)> = Vec::new();
    let mut seen_stems: BTreeSet<String> = BTreeSet::new();
    for w in walk_scene(scene) {
        let l = w.layer;
        if is_stage(&l.id) || is_decorative(&l.id, &l.kind) {
            continue;
        }
        let Some(ms) = entries.get(l.id.as_str()) else {
            continue;
        };
        if scene.lifecycle.is_none() {
            let stem = l.id.trim_end_matches(|c: char| c.is_ascii_digit());
            if stem.len() < l.id.len() && !seen_stems.insert(stem.to_string()) {
                continue;
            }
        }
        let start = ms.iter().map(|m| m.start).fold(f64::MAX, f64::min);
        let mut kinds: BTreeSet<&str> = BTreeSet::new();
        for m in ms {
            if let Some(k) = entry_kind(&m.op) {
                kinds.insert(k);
            }
        }
        let first = ms.iter().min_by(|a, b| a.start.total_cmp(&b.start));
        let easing = first.map_or_else(String::new, |m| format!("{:?}:{:?}", m.easing, m.spring));
        let key = format!(
            "{}|{easing}",
            kinds.into_iter().collect::<Vec<_>>().join("+")
        );
        ent.push((l.id.as_str(), start, key));
    }
    if ent.len() < 3 {
        return;
    }
    let window = (1.0 / project.canvas.fps.max(1) as f64)
        .max(rules.thresholds.min_stagger_ms as f64 / 1000.0);
    let mut sorted: Vec<&(&str, f64, String)> = ent.iter().collect();
    sorted.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(b.0)));
    for i in 0..sorted.len() - 2 {
        if sorted[i + 2].1 - sorted[i].1 <= window + 1e-9 {
            out.push(mk(
                &scene.id,
                sorted[i].0,
                SlopCheck::UniformEntrance,
                format!(
                    "3 layers enter within {:.0} ms of each other",
                    window * 1000.0
                ),
            ));
            return;
        }
    }
    let keys: BTreeSet<&str> = ent.iter().map(|e| e.2.as_str()).collect();
    if keys.len() == 1 {
        let key = keys.iter().next().copied().unwrap_or("");
        out.push(mk(
            &scene.id,
            ent[0].0,
            SlopCheck::UniformEntrance,
            format!(
                "all {} entering layers share one entrance ({key})",
                ent.len()
            ),
        ));
    }
}

/// (0.24 Wave 1) A text layer whose entrance (its own motions and its
/// groups') is a fade and nothing else appears instead of arriving.
fn things_appear_check(scene: &Scene, out: &mut Vec<SlopFinding>) {
    let entries = entries_until_anticipate(scene);
    for w in walk_scene(scene) {
        let LayerKind::Text(t) = &w.layer.kind else {
            continue;
        };
        if is_decorative(&w.layer.id, &w.layer.kind) || t.text.trim().is_empty() {
            continue;
        }
        // A number that counts up arrives by counting.
        if scene
            .motions
            .iter()
            .any(|m| m.target == w.layer.id && matches!(m.op, MotionOp::Count { .. }))
        {
            continue;
        }
        let mut fade = false;
        let mut arrives = false;
        for id in w
            .ancestors
            .iter()
            .copied()
            .chain(std::iter::once(w.layer.id.as_str()))
        {
            for m in entries.get(id).into_iter().flatten() {
                match entry_kind(&m.op) {
                    Some("fade") => fade = true,
                    Some(_) => arrives = true,
                    None => {}
                }
            }
        }
        if fade && !arrives {
            out.push(mk(
                &scene.id,
                &w.layer.id,
                SlopCheck::ThingsAppear,
                "text only fades in: give it a masked rise, slide or settle".to_string(),
            ));
        }
    }
}

/// (0.24 Wave 1) Soul metrics of a compiled project (see
/// docs/MOTION_LANGUAGE.md).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SoulMetrics {
    pub beats: usize,
    /// Share of beats where at least two entering layers start within 0.25 s
    /// of each other and not on the same frame (target >= 0.6).
    pub overlap_ratio: f32,
    /// Beats where three or more entering layers share one start frame.
    pub same_frame_beats: usize,
    /// Text layers whose only entrance is a fade (target 0).
    pub things_appear: usize,
    /// (Wave 2) Picture/accent moves (chord >= 3 % of the width) that travel
    /// an arc (`via`) over all of them (target >= 0.5; 1 when there are none).
    #[serde(default)]
    pub arc_ratio: f32,
    /// Picture/accent moves counted by `arc_ratio`.
    #[serde(default)]
    pub arc_moves: usize,
    /// (Wave 2) Stress punches (`stress:<onset>` scale motions) whose peak is
    /// within 60 ms of the stressed word over all of them (target >= 0.9).
    #[serde(default)]
    pub stress_alignment: f32,
    /// Stress punches counted by `stress_alignment`.
    #[serde(default)]
    pub stresses: usize,
    /// (Wave 2) Share of beat time at full rest (no motion, camera included,
    /// for 0.3 s or more), footage plates aside (target 0.08..0.20).
    #[serde(default)]
    pub hold_ratio: f32,
    /// (Wave 2) Shared elements visible across a cut (target >= beats / 3).
    #[serde(default)]
    pub carry_count: usize,
    /// (Wave 2) Coefficient of variation of entrance durations (0.25..0.6).
    #[serde(default)]
    pub duration_cv: f32,
}

/// Measure [`SoulMetrics`] from the entrance motions of the beat scenes.
pub fn soul_metrics(project: &MotionProject) -> SoulMetrics {
    let beats = beat_scenes(project);
    let frame = 1.0 / project.canvas.fps.max(1) as f64;
    let mut overlapped = 0usize;
    let mut same_frame_beats = 0usize;
    let mut appear = Vec::new();
    let (mut arcs, mut arc_all) = (0usize, 0usize);
    let (mut aligned, mut stress_all) = (0usize, 0usize);
    let (mut rest, mut total_secs) = (0.0f64, 0.0f64);
    let mut durs: Vec<f64> = Vec::new();
    let min_chord = 0.03 * project.canvas.width as f32;
    for s in &beats {
        let entries = entries_until_anticipate(s);
        for w in walk_scene(s) {
            let l = w.layer;
            if is_stage(&l.id) || is_decorative(&l.id, &l.kind) {
                continue;
            }
            if let Some(ms) = entries.get(l.id.as_str()) {
                if let Some(d) = ms
                    .iter()
                    .filter(|m| m.duration > 0.011)
                    .map(|m| m.duration)
                    .reduce(f64::max)
                {
                    durs.push(d);
                }
            }
            if !matches!(l.kind, LayerKind::Text(_))
                && crate::compiler::choreography::role_of(&l.id)
                    != crate::compiler::choreography::Role::Ambient
            {
                for m in s.motions.iter().filter(|m| m.target == l.id) {
                    if let MotionOp::Move { from, to, via } = &m.op {
                        let chord = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
                        if chord >= min_chord {
                            arc_all += 1;
                            arcs += usize::from(via.is_some());
                        }
                    }
                }
            }
        }
        for m in &s.motions {
            if let (MotionOp::Scale { .. }, Some(id)) = (&m.op, m.id.as_deref()) {
                if let Some(onset) = id
                    .strip_prefix("stress:")
                    .and_then(|t| t.parse::<f64>().ok())
                {
                    stress_all += 1;
                    if (m.start + m.duration - onset).abs() <= 0.06 + 1e-6 {
                        aligned += 1;
                    }
                }
            }
        }
        // Rest: gaps of at least 0.3 s with no motion at all.
        let mut busy: Vec<(f64, f64)> = s
            .motions
            .iter()
            .filter(|m| m.duration > 0.011 && !m.target.starts_with("plate."))
            .map(|m| (m.start, m.start + m.duration))
            .collect();
        if let Some(c) = &s.camera {
            busy.extend(
                c.motions
                    .iter()
                    .filter(|m| !matches!(m.op, crate::scene::CameraOp::Shake { .. }))
                    .map(|m| (m.start, m.start + m.duration)),
            );
        }
        busy.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut at = 0.0f64;
        for (a, b) in busy {
            if a - at >= 0.3 {
                rest += a - at;
            }
            at = at.max(b);
        }
        if s.duration_seconds - at >= 0.3 {
            rest += s.duration_seconds - at;
        }
        total_secs += s.duration_seconds;
        let mut starts: Vec<f64> = Vec::new();
        for w in walk_scene(s) {
            let l = w.layer;
            if is_stage(&l.id) || is_decorative(&l.id, &l.kind) {
                continue;
            }
            if let Some(ms) = entries.get(l.id.as_str()) {
                let st = ms.iter().map(|m| m.start).fold(f64::MAX, f64::min);
                starts.push(st);
            }
        }
        starts.sort_by(|a, b| a.total_cmp(b));
        let staggered = starts.iter().enumerate().any(|(i, a)| {
            starts[i + 1..]
                .iter()
                .any(|b| *b - a >= frame * 0.99 && *b - a <= 0.25)
        });
        if std::env::var("MOTION_SOUL_DEBUG").is_ok() {
            eprintln!("SOUL {} staggered={} starts={:?}", s.id, staggered, starts);
        }
        if staggered {
            overlapped += 1;
        }
        if starts.windows(3).any(|w| w[2] - w[0] < frame * 0.99) {
            same_frame_beats += 1;
        }
        things_appear_check(s, &mut appear);
    }
    SoulMetrics {
        beats: beats.len(),
        overlap_ratio: if beats.is_empty() {
            0.0
        } else {
            (overlapped as f32 / beats.len() as f32 * 100.0).round() / 100.0
        },
        same_frame_beats,
        things_appear: appear.len(),
        arc_ratio: if arc_all == 0 {
            1.0
        } else {
            (arcs as f32 / arc_all as f32 * 100.0).round() / 100.0
        },
        arc_moves: arc_all,
        stress_alignment: if stress_all == 0 {
            0.0
        } else {
            (aligned as f32 / stress_all as f32 * 100.0).round() / 100.0
        },
        stresses: stress_all,
        hold_ratio: if total_secs > 0.0 {
            ((rest / total_secs) as f32 * 100.0).round() / 100.0
        } else {
            0.0
        },
        carry_count: project
            .shared
            .iter()
            .filter(|e| {
                let mut scenes: Vec<&str> = e.track.iter().map(|k| k.scene.as_str()).collect();
                scenes.sort_unstable();
                scenes.dedup();
                scenes.len() >= 2
                    && e.track
                        .iter()
                        .all(|k| k.state.opacity.is_none_or(|o| o > 0.0))
            })
            .count(),
        duration_cv: {
            let n = durs.len() as f64;
            if n < 2.0 {
                0.0
            } else {
                let mean = durs.iter().sum::<f64>() / n;
                let var = durs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n;
                ((var.sqrt() / mean.max(1e-9)) as f32 * 100.0).round() / 100.0
            }
        },
    }
}

fn hold_check(scene: &Scene, out: &mut Vec<SlopFinding>) {
    let end = scene.lifecycle.map_or(scene.duration_seconds, |l| l.bridge);
    let entries = entry_motions(scene);
    for w in walk_scene(scene) {
        let LayerKind::Text(t) = &w.layer.kind else {
            continue;
        };
        if is_decorative(&w.layer.id, &w.layer.kind) || t.text.trim().is_empty() {
            continue;
        }
        let mut fully_in = 0.0f64;
        for id in w
            .ancestors
            .iter()
            .copied()
            .chain(std::iter::once(w.layer.id.as_str()))
        {
            for m in entries.get(id).into_iter().flatten() {
                if matches!(&m.op, MotionOp::GlyphCascade { from, .. } if from.opacity >= 0.95) {
                    continue;
                }
                fully_in = fully_in.max(m.start + m.duration);
            }
        }
        let words = t.text.split_whitespace().count() as f64;
        let raw_need = 0.3 + 0.06 * words;
        let need = if scene.lifecycle.is_none() {
            raw_need.min(end * 0.5)
        } else {
            raw_need
        };
        let hold = end - fully_in;
        if hold < need - 1e-9 {
            out.push(mk(
                &scene.id,
                &w.layer.id,
                SlopCheck::NoHold,
                format!("held fully in for {hold:.2}s, needs {need:.2}s for {words:.0} word(s)"),
            ));
        }
    }
}

fn post_name(k: &PostKind) -> &'static str {
    match k {
        PostKind::Bloom { .. } => "bloom",
        PostKind::ChromaticAberration { .. } => "chromatic_aberration",
        PostKind::Glitch { .. } => "glitch",
        PostKind::Rays { .. } => "rays",
        PostKind::DirectionalBlur { .. } => "directional_blur",
        PostKind::Grain { .. } => "grain",
        PostKind::Vignette { .. } => "vignette",
        PostKind::Shockwave { .. } => "shockwave",
    }
}

fn fx_check(scene: &Scene, rules: &SlopRules, out: &mut Vec<SlopFinding>) {
    let active: BTreeSet<&str> = scene
        .post
        .iter()
        .filter(|p| p.from > 0.0 || p.to > 0.0)
        .map(|p| post_name(&p.kind))
        .collect();
    if active.len() as u32 > rules.thresholds.max_fx_per_beat {
        out.push(mk(
            &scene.id,
            "-",
            SlopCheck::FxStack,
            format!(
                "{} post effects ({}), max {}",
                active.len(),
                active.iter().copied().collect::<Vec<_>>().join(", "),
                rules.thresholds.max_fx_per_beat
            ),
        ));
    }
}

/// (0.24 M10) Nothing flickers or pulsates: no music pulse, glitch or echo
/// anywhere, and at most one camera shake in the whole video (the payoff hit).
fn flicker_check(beats: &[&Scene], out: &mut Vec<SlopFinding>) {
    let mut shakes = 0usize;
    for s in beats {
        let mut bad: BTreeSet<&str> = BTreeSet::new();
        for m in &s.motions {
            match m.op {
                MotionOp::Pulse { .. } => {
                    bad.insert("pulse");
                }
                MotionOp::Echo { .. } => {
                    bad.insert("echo");
                }
                _ => {}
            }
        }
        if s.post
            .iter()
            .any(|p| (p.from > 0.0 || p.to > 0.0) && matches!(p.kind, PostKind::Glitch { .. }))
        {
            bad.insert("glitch");
        }
        if !bad.is_empty() {
            out.push(mk(
                &s.id,
                "-",
                SlopCheck::Flicker,
                format!(
                    "{} in the beat: nothing may flicker or pulsate",
                    bad.into_iter().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
        if let Some(cam) = &s.camera {
            shakes += cam
                .motions
                .iter()
                .filter(|m| matches!(m.op, crate::scene::CameraOp::Shake { .. }))
                .count();
        }
    }
    if shakes > 1 {
        out.push(mk(
            "-",
            "-",
            SlopCheck::Flicker,
            format!("{shakes} camera shakes in the video, max 1"),
        ));
    }
}

fn showy(scene: &Scene) -> bool {
    let wipe = walk_scene(scene)
        .iter()
        .any(|w| layer_name(&w.layer.id).starts_with("wipe"));
    let fx = scene.post.iter().any(|p| {
        p.duration > 0.0
            && matches!(
                p.kind,
                PostKind::Glitch { .. }
                    | PostKind::DirectionalBlur { .. }
                    | PostKind::ChromaticAberration { .. }
            )
    });
    wipe || fx
}

fn grammar_signatures(project: &MotionProject, beats: &[&Scene]) -> Vec<String> {
    if let Some(d) = &project.project.direction {
        if d.beats.len() == beats.len() {
            return d.beats.iter().map(|b| b.template.clone()).collect();
        }
    }
    let cw = (project.canvas.width.max(1) as f32) * 0.25;
    let ch = (project.canvas.height.max(1) as f32) * 0.25;
    beats
        .iter()
        .map(|s| {
            let mut set: BTreeSet<String> = BTreeSet::new();
            for w in walk_scene(s) {
                let l = w.layer;
                if is_stage(&l.id) || is_decorative(&l.id, &l.kind) {
                    continue;
                }
                // Words and lines of one headline are one element.
                let name = l.id.split('.').skip(1).take(1).collect::<String>();
                if s.lifecycle.is_none() {
                    let sx = (l.x / cw).round() as i32;
                    let sy = (l.y / ch).round() as i32;
                    set.insert(format!("{name}:{}:{sx},{sy}", l.kind.type_name()));
                } else {
                    set.insert(format!("{name}:{}", l.kind.type_name()));
                }
            }
            set.into_iter().collect::<Vec<_>>().join(",")
        })
        .collect()
}

fn font_family(project: &MotionProject, role: &crate::scene::FontRole) -> String {
    let asset = project.theme.fonts.get(role);
    let path = asset
        .and_then(|id| project.assets.iter().find(|a| &a.id == id))
        .map(|a| a.path.as_str());
    let stem = path
        .and_then(|p| p.rsplit('/').next())
        .map(|f| f.split('.').next().unwrap_or(f))
        .or(asset.map(String::as_str));
    match stem {
        Some(s) => s.split('-').next().unwrap_or(s).to_lowercase(),
        None => format!("{role:?}").to_lowercase(),
    }
}

/// A layer's box in its parent's space, honouring anchor and scale.
fn local_rect(l: &Layer) -> (f32, f32, f32, f32) {
    let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
    (l.x - l.anchor_x * w, l.y - l.anchor_y * h, w, h)
}

fn rgb_eq(a: Color, b: Color) -> bool {
    (a.r, a.g, a.b) == (b.r, b.g, b.b)
}

fn last_segment(id: &str) -> &str {
    id.rsplit('.').next().unwrap_or(id)
}

/// (0.24 M9) Count the loud colour moves of the whole video: full-frame
/// colour fields (a wipe's second bar belongs to its first), accent blocks
/// behind words and stamps.
fn accent_check(
    project: &MotionProject,
    beats: &[&Scene],
    rules: &SlopRules,
    out: &mut Vec<SlopFinding>,
) {
    let r = &rules.thresholds.accent;
    let (cw, ch) = (project.canvas.width as f32, project.canvas.height as f32);
    let accent = project.theme.palette.get("accent").copied();
    let colours: Vec<Color> = project
        .theme
        .palette
        .iter()
        .filter(|(k, _)| k.as_str() == "accent" || k.starts_with("field_"))
        .map(|(_, c)| *c)
        .collect();
    let mut fields: Vec<(&str, String)> = Vec::new();
    let mut blocks: Vec<(&str, String)> = Vec::new();
    let mut stamps: Vec<(&str, String)> = Vec::new();
    for s in beats {
        let walked = walk_scene(s);
        for w in &walked {
            let l = w.layer;
            if last_segment(&l.id) == "stamp" && matches!(l.kind, LayerKind::Group { .. }) {
                stamps.push((&s.id, l.id.clone()));
            }
            let fill = match &l.kind {
                LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => {
                    *fill
                }
                _ => continue,
            };
            if fill.a < 200 || l.id.ends_with("wipe_b") {
                continue;
            }
            let (x, y, rw, rh) = local_rect(l);
            let full = rw >= r.full_frame * cw && rh >= r.min_height * ch;
            if full {
                // Only colour counts: the accent or a field colour. The
                // canvas ground, paper and card stock are neutral surfaces.
                if colours.iter().any(|c| rgb_eq(fill, *c)) {
                    fields.push((&s.id, l.id.clone()));
                }
                continue;
            }
            let is_accent = accent.is_some_and(|a| rgb_eq(fill, a));
            if !is_accent {
                continue;
            }
            let (px, py) = (0.1 * rw, 0.1 * rh);
            let behind_words = walked.iter().any(|o| {
                o.ancestors == w.ancestors
                    && matches!(&o.layer.kind, LayerKind::Text(t) if !t.text.trim().is_empty())
                    && {
                        let (tx, ty, tw, th) = local_rect(o.layer);
                        let (cx, cy) = (tx + tw / 2.0, ty + th / 2.0);
                        cx >= x - px && cx <= x + rw + px && cy >= y - py && cy <= y + rh + py
                    }
            });
            if behind_words {
                blocks.push((&s.id, l.id.clone()));
            }
        }
    }
    let allowed = (beats.len() / r.beats_per_block.max(1)).max(1);
    let mut over = |what: &str, found: &[(&str, String)], max: usize| {
        if found.len() > max {
            let (scene, layer) = &found[max];
            out.push(mk(
                scene,
                layer,
                SlopCheck::AccentOveruse,
                format!(
                    "{} {what} in {} beats, max {max}: spend the accent once, on the payoff",
                    found.len(),
                    beats.len()
                ),
            ));
        }
    };
    over("full-frame colour fields", &fields, r.max_fields);
    over("accent blocks behind words", &blocks, allowed);
    over("stamps", &stamps, allowed);
}

/// (0.24 M9) Three or more thin, equal, left-aligned bars stacked like lines
/// of text, with no text on them: placeholder copy pretending to be content.
fn skeleton_check(project: &MotionProject, scene: &Scene, out: &mut Vec<SlopFinding>) {
    let (cw, ch) = (project.canvas.width as f32, project.canvas.height as f32);
    let u = cw.min(ch) / 1080.0;
    let walked = walk_scene(scene);
    type Bar<'a> = (&'a Layer, (f32, f32, f32, f32));
    let mut groups: BTreeMap<Vec<&str>, Vec<Bar>> = BTreeMap::new();
    for w in &walked {
        let l = w.layer;
        let LayerKind::Rectangle { fill, .. } = &l.kind else {
            continue;
        };
        let r = local_rect(l);
        if fill.a >= 0x40
            && r.3 >= 6.0 * u
            && r.3 <= 0.04 * ch
            && r.2 >= 4.0 * r.3
            && r.2 < 0.9 * cw
        {
            groups.entry(w.ancestors.clone()).or_default().push((l, r));
        }
    }
    for (anc, mut bars) in groups {
        bars.sort_by(|a, b| a.1 .1.total_cmp(&b.1 .1));
        let mut i = 0;
        while i < bars.len() {
            // A run of bars sharing left edge and height, at line spacing.
            let mut j = i + 1;
            while j < bars.len() {
                let (a, b) = (bars[j - 1].1, bars[j].1);
                let aligned = (a.0 - b.0).abs() <= 2.0 * u && (a.3 - b.3).abs() <= 1.5 * u;
                let gap = b.1 - (a.1 + a.3);
                if !(aligned && gap >= 0.0 && gap <= 2.5 * a.3) {
                    break;
                }
                j += 1;
            }
            if j - i >= 3 {
                let run = &bars[i..j];
                let x0 = run.iter().map(|b| b.1 .0).fold(f32::MAX, f32::min);
                let y0 = run.iter().map(|b| b.1 .1).fold(f32::MAX, f32::min);
                let x1 = run.iter().map(|b| b.1 .0 + b.1 .2).fold(f32::MIN, f32::max);
                let y1 = run.iter().map(|b| b.1 .1 + b.1 .3).fold(f32::MIN, f32::max);
                let pad = run[0].1 .3;
                let covers_text = walked.iter().any(|o| {
                    o.ancestors == anc
                        && matches!(&o.layer.kind, LayerKind::Text(t) if !t.text.trim().is_empty())
                        && {
                            let (tx, ty, tw, th) = local_rect(o.layer);
                            tx < x1 + pad
                                && tx + tw > x0 - pad
                                && ty < y1 + pad
                                && ty + th > y0 - pad
                        }
                });
                if !covers_text {
                    out.push(mk(
                        &scene.id,
                        &run[0].0.id,
                        SlopCheck::SkeletonLines,
                        format!(
                            "{} equal bars stacked like lines of text, with no text on them",
                            run.len()
                        ),
                    ));
                }
            }
            i = j.max(i + 1);
        }
    }
}

/// Layer names whose rectangles are the content itself: chart bars and
/// tracks, an evidence note, a dossier document, a collection item.
const CONTENT_BOXES: &[&str] = &[
    "bar", "track", "chart", "note", "dossier", "doc", "items", "item", "row", "axis", "fill",
    "progress", "seg", "cell", "legend", "caption", "pill", "underlay",
];

fn is_content_box(id: &str) -> bool {
    id.split('.')
        .skip(1)
        .any(|seg| CONTENT_BOXES.iter().any(|c| seg.starts_with(c)))
}

/// (0.24 V1) The boxy video: more than one rect / card behind text in a beat
/// (a card is allowed only when it is the content), or more than one
/// decorative rule (page rules, title underlines, hairline frames).
fn boxy_check(project: &MotionProject, scene: &Scene, out: &mut Vec<SlopFinding>) {
    let (cw, ch) = (project.canvas.width as f32, project.canvas.height as f32);
    let u = cw.min(ch) / 1080.0;
    let walked = walk_scene(scene);
    let mut boxes: Vec<&Layer> = Vec::new();
    let mut rules: Vec<&Layer> = Vec::new();
    for w in &walked {
        let l = w.layer;
        let alpha = match &l.kind {
            LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => fill.a,
            // A flourish divider is a rule drawn as a picture.
            LayerKind::Image { .. } if last_segment(&l.id) == "divider" => {
                rules.push(l);
                continue;
            }
            _ => continue,
        };
        if !l.visible || l.id.starts_with("backdrop.") || is_stage(&l.id) {
            continue;
        }
        let (x, y, rw, rh) = local_rect(l);
        if rw >= 0.9 * cw && rh >= 0.75 * ch {
            continue; // a colour field, not a box
        }
        let thin = rh.min(rw) <= 6.0 * u && rh.max(rw) >= 8.0 * rh.min(rw);
        if thin {
            let named = last_segment(&l.id).ends_with("rule") || is_decorative(&l.id, &l.kind);
            if named && !is_content_box(&l.id) {
                rules.push(l);
            }
            continue;
        }
        if alpha < 0x40 || is_content_box(&l.id) || is_decorative(&l.id, &l.kind) {
            continue;
        }
        let (px, py) = (0.05 * rw, 0.05 * rh);
        let behind_text = walked.iter().any(|o| {
            o.ancestors == w.ancestors
                && matches!(&o.layer.kind, LayerKind::Text(t) if !t.text.trim().is_empty())
                && {
                    let (tx, ty, tw, th) = local_rect(o.layer);
                    let (cx, cy) = (tx + tw / 2.0, ty + th / 2.0);
                    cx >= x - px && cx <= x + rw + px && cy >= y - py && cy <= y + rh + py
                }
        });
        if behind_text {
            boxes.push(l);
        }
    }
    if boxes.len() > 1 {
        out.push(mk(
            &scene.id,
            &boxes[1].id,
            SlopCheck::Boxy,
            format!(
                "{} rects / cards behind text, max 1: set the type on the ground",
                boxes.len()
            ),
        ));
    }
    if rules.len() > 1 {
        out.push(mk(
            &scene.id,
            &rules[1].id,
            SlopCheck::Boxy,
            format!("{} decorative rules, max 1", rules.len()),
        ));
    }
}

/// Run every slop check on `project`.
pub fn slop_report(project: &MotionProject, rules: &SlopRules) -> SlopReport {
    let beats = beat_scenes(project);
    let mut findings = Vec::new();
    let mut not_measurable = Vec::new();

    for s in &beats {
        text_checks(s, rules, &mut findings);
        eyebrow_check(project, s, rules, &mut findings);
        palette_checks(project, s, rules, &mut findings);
        entrance_check(project, s, rules, &mut findings);
        hold_check(s, &mut findings);
        fx_check(s, rules, &mut findings);
        skeleton_check(project, s, &mut findings);
        things_appear_check(s, &mut findings);
        boxy_check(project, s, &mut findings);
    }
    accent_check(project, &beats, rules, &mut findings);
    flicker_check(&beats, &mut findings);

    if beats.len() < 2 {
        not_measurable.push("slop_repeated_grammar: needs at least 2 beats".to_string());
        not_measurable.push("slop_showy_transitions: needs at least 2 beats".to_string());
    } else {
        let sigs = grammar_signatures(project, &beats);
        for i in 1..beats.len() {
            if !sigs[i].is_empty() && sigs[i] == sigs[i - 1] {
                findings.push(mk(
                    &beats[i].id,
                    "-",
                    SlopCheck::RepeatedGrammar,
                    format!("same layout as {}", beats[i - 1].id),
                ));
            }
        }
        let n = beats.iter().filter(|s| showy(s)).count();
        let ratio = n as f32 / beats.len() as f32;
        if ratio > rules.thresholds.max_showy_transition_ratio {
            findings.push(mk(
                "-",
                "-",
                SlopCheck::ShowyTransitions,
                format!(
                    "{n} of {} beats use a showy transition ({:.0}%), max {:.0}%",
                    beats.len(),
                    ratio * 100.0,
                    rules.thresholds.max_showy_transition_ratio * 100.0
                ),
            ));
        }
    }

    let mut families: BTreeSet<String> = BTreeSet::new();
    for s in &beats {
        for w in walk_scene(s) {
            if let LayerKind::Text(t) = &w.layer.kind {
                if !is_decorative(&w.layer.id, &w.layer.kind) {
                    families.insert(font_family(project, &t.font_role));
                }
            }
        }
    }
    if families.is_empty() {
        not_measurable.push("slop_family_sprawl: no text layers".to_string());
    } else if families.len() as u32 > rules.thresholds.max_font_families {
        findings.push(mk(
            "-",
            "-",
            SlopCheck::FamilySprawl,
            format!(
                "{} font families ({}), max {}",
                families.len(),
                families.iter().cloned().collect::<Vec<_>>().join(", "),
                rules.thresholds.max_font_families
            ),
        ));
    }

    SlopReport {
        verdict: if findings.is_empty() {
            LayoutVerdict::Pass
        } else {
            LayoutVerdict::Fail
        },
        findings,
        not_measurable,
    }
}

// ---------------------------------------------------------------------------
// Critique
// ---------------------------------------------------------------------------

pub const AXES: [&str; 8] = [
    "hierarchy",
    "type",
    "colour",
    "motion_rhythm",
    "variety",
    "legibility",
    "asset_coherence",
    "audio",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisScore {
    pub axis: String,
    /// 0..=5; `None` = n/a (no data).
    pub score: Option<u8>,
    pub findings: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CritiqueFinding {
    pub axis: String,
    pub check: String,
    pub scene: String,
    pub layer: String,
    pub detail: String,
}

/// (0.24 M9) Anti-Slop's three dials, each 1..=10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dials {
    /// How loud: operations and camera moves per second.
    pub energy: u8,
    /// How often the picture changes: cuts per minute and layout changes.
    pub rhythm: u8,
    /// How varied the movement is: entrance kinds and operation rate.
    pub motion: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Critique {
    /// Scored axes rescaled to /40 (one decimal).
    pub total: f32,
    pub axes: Vec<AxisScore>,
    pub findings: Vec<CritiqueFinding>,
    pub not_measurable: Vec<String>,
    /// (0.24 M9) `Reading this as: ...`: the design read in one line.
    #[serde(default)]
    pub reading: String,
    /// (0.24 M9) The dials shown in the reading: the style's where it
    /// declares them, else the measured ones.
    #[serde(default)]
    pub dials: Option<Dials>,
    /// (0.24 M9) The dials measured from the scene.
    #[serde(default)]
    pub measured: Option<Dials>,
    /// (0.24 M9) Advice that is not a failure (`note_unsourced_numbers`).
    #[serde(default)]
    pub notes: Vec<String>,
    /// (0.24 Wave 1) Motion soul metrics (`soul_metrics`).
    #[serde(default)]
    pub soul: Option<SoulMetrics>,
}

fn layout_axis(c: LayoutCheck) -> &'static str {
    match c {
        LayoutCheck::HeadlineTooBig
        | LayoutCheck::FocalOutOfFocus
        | LayoutCheck::EmphasisOffFocal
        | LayoutCheck::FocalNotDominant
        | LayoutCheck::TextOverlap => "hierarchy",
        LayoutCheck::TextOutsideSafe | LayoutCheck::TextClipped | LayoutCheck::TextTooSmall => {
            "type"
        }
        LayoutCheck::TextLocalContrast | LayoutCheck::TextOverSubject | LayoutCheck::Evaluation => {
            "legibility"
        }
        LayoutCheck::SubjectImageOutside
        | LayoutCheck::LayerOffCanvas
        | LayoutCheck::SubjectTooSmall
        | LayoutCheck::FrameTooLoose
        | LayoutCheck::AssetLowContrast => "asset_coherence",
    }
}

fn slop_axis(c: SlopCheck) -> &'static str {
    match c {
        SlopCheck::StockPhrase
        | SlopCheck::EmojiBullet
        | SlopCheck::FamilySprawl
        | SlopCheck::SkeletonLines
        | SlopCheck::EyebrowLabel => "type",
        SlopCheck::AiPalette | SlopCheck::AccentOveruse => "colour",
        SlopCheck::UniformEntrance
        | SlopCheck::NoHold
        | SlopCheck::FxStack
        | SlopCheck::Flicker
        | SlopCheck::ThingsAppear => "motion_rhythm",
        SlopCheck::Boxy => "hierarchy",
        SlopCheck::RepeatedGrammar | SlopCheck::ShowyTransitions => "variety",
    }
}

const DIAL_MISMATCH_GAP: u8 = 4;

fn dial(x: f32) -> u8 {
    x.round().clamp(1.0, 10.0) as u8
}

/// The dials the scene shows (see [`Dials`]). Pure.
pub fn measure_dials(project: &MotionProject) -> Option<Dials> {
    let beats = beat_scenes(project);
    let secs: f64 = beats.iter().map(|s| s.duration_seconds).sum();
    if beats.is_empty() || secs <= 0.0 {
        return None;
    }
    let ops: usize = beats.iter().map(|s| s.motions.len()).sum();
    let cam: usize = beats
        .iter()
        .map(|s| s.camera.as_ref().map_or(0, |c| c.motions.len()))
        .sum();
    let kinds: BTreeSet<&str> = beats
        .iter()
        .flat_map(|s| s.motions.iter().filter_map(|m| entry_kind(&m.op)))
        .collect();
    let ops_rate = ops as f32 / secs as f32;
    let per_min = beats.len() as f32 * 60.0 / secs as f32;
    let sigs = grammar_signatures(project, &beats);
    let change = if sigs.len() < 2 {
        0.0
    } else {
        sigs.windows(2).filter(|w| w[0] != w[1]).count() as f32 / (sigs.len() - 1) as f32
    };
    Some(Dials {
        energy: dial(1.2 * ops_rate + 0.6 * cam as f32 / beats.len() as f32),
        rhythm: dial(4.0 * change + per_min / 3.0),
        motion: dial(0.8 * ops_rate + 0.8 * kinds.len() as f32),
    })
}

/// The dials a style declares; `None` per dial left on `auto`.
fn declared_dials(style: &StyleProfile) -> [Option<u8>; 3] {
    use crate::style::{Density, MotionLanguage, Temperament};
    [
        match style.temperament {
            Temperament::Auto => None,
            Temperament::Restrained => Some(3),
            Temperament::Balanced => Some(5),
            Temperament::Energetic => Some(8),
        },
        match style.density {
            Density::Auto => None,
            Density::Sparse => Some(3),
            Density::Balanced => Some(5),
            Density::Dense => Some(8),
        },
        match style.motion_language {
            MotionLanguage::Auto => None,
            MotionLanguage::Minimal => Some(2),
            MotionLanguage::Parallax => Some(6),
            MotionLanguage::Sequential | MotionLanguage::Data => Some(5),
            MotionLanguage::Kinetic => Some(8),
        },
    ]
}

fn is_figure(text: &str) -> bool {
    let digits = text.chars().filter(char::is_ascii_digit).count();
    digits >= 2 || (digits >= 1 && text.contains(['$', '%', '£', '€', '₹', '¥']))
}

/// "source: ...", "via ...", or a name with a year ("SmartAsset, 2024").
fn is_provenance(text: &str) -> bool {
    let l = text.trim().to_lowercase();
    let year = l.as_bytes().windows(4).any(|w| {
        w.iter().all(u8::is_ascii_digit) && (w.starts_with(b"19") || w.starts_with(b"20"))
    });
    l.starts_with("source") || l.starts_with("via ") || (year && l.contains(','))
}

fn tone_name(t: crate::style::Tone) -> &'static str {
    use crate::style::Tone;
    match t {
        Tone::Auto => "auto",
        Tone::Editorial => "editorial",
        Tone::Technical => "technical",
        Tone::Playful => "playful",
        Tone::Street => "street",
        Tone::Documentary => "documentary",
        Tone::Hype => "hype",
        Tone::Studio => "studio",
        Tone::Cinematic => "cinematic",
    }
}

fn format_name(project: &MotionProject) -> String {
    let (w, h) = (project.canvas.width, project.canvas.height);
    let shape = if h as f32 >= 1.2 * w as f32 {
        "vertical"
    } else if w as f32 >= 1.2 * h as f32 {
        "landscape"
    } else {
        "square"
    };
    format!("{shape} {w}x{h}")
}

/// Score `project` on the eight axes (see the module docs).
pub fn critique(project: &MotionProject, rules: &SlopRules) -> Critique {
    critique_with(project, rules, None)
}

/// [`critique`] with the style the project was compiled from: the design
/// read names its tone, the dials come from its temperament, density and
/// motion language, and `dial_mismatch` flags a scene that moves against them.
pub fn critique_with(
    project: &MotionProject,
    rules: &SlopRules,
    style: Option<&StyleProfile>,
) -> Critique {
    let slop = slop_report(project, rules);
    let layout = LayoutFrame::new(project.canvas.width, project.canvas.height)
        .map(|f| layout_report(project, &f).findings)
        .unwrap_or_default();
    let mut findings: Vec<CritiqueFinding> = Vec::new();
    for LayoutFinding {
        scene,
        layer,
        check,
        detail,
    } in layout
    {
        findings.push(CritiqueFinding {
            axis: layout_axis(check).to_string(),
            check: check.name().to_string(),
            scene,
            layer,
            detail,
        });
    }
    for f in slop.findings {
        findings.push(CritiqueFinding {
            axis: slop_axis(f.check).to_string(),
            check: f.check.name().to_string(),
            scene: f.scene,
            layer: f.layer,
            detail: f.detail,
        });
    }
    let measured = measure_dials(project);
    let declared = style.map(declared_dials);
    if let (Some(m), Some(d)) = (measured, declared) {
        for (name, got, want) in [
            ("ENERGY", m.energy, d[0]),
            ("RHYTHM", m.rhythm, d[1]),
            ("MOTION", m.motion, d[2]),
        ] {
            if let Some(want) = want.filter(|w| w.abs_diff(got) >= DIAL_MISMATCH_GAP) {
                findings.push(CritiqueFinding {
                    axis: "motion_rhythm".to_string(),
                    check: "dial_mismatch".to_string(),
                    scene: "-".to_string(),
                    layer: "-".to_string(),
                    detail: format!(
                        "the style declares {name} {want} but the scene measures {got}"
                    ),
                });
            }
        }
    }
    let shown = measured.map(|m| {
        let d = declared.unwrap_or([None; 3]);
        Dials {
            energy: d[0].unwrap_or(m.energy),
            rhythm: d[1].unwrap_or(m.rhythm),
            motion: d[2].unwrap_or(m.motion),
        }
    });
    let tone = match style {
        Some(st) if st.tone != crate::style::Tone::Auto => tone_name(st.tone).to_string(),
        _ => project
            .project
            .art
            .as_ref()
            .map_or("auto".to_string(), |a| a.look.replace('_', " ")),
    };
    let genre = match (&project.project.art, style) {
        (Some(a), Some(st)) if st.tone != crate::style::Tone::Auto => {
            format!("{tone} ({})", a.look.replace('_', " "))
        }
        _ => tone,
    };
    let reading = match shown {
        Some(d) => format!(
            "Reading this as: {genre} for {}, dial ENERGY {} / RHYTHM {} / MOTION {}",
            format_name(project),
            d.energy,
            d.rhythm,
            d.motion
        ),
        None => format!(
            "Reading this as: {genre} for {}, no beats to read dials from",
            format_name(project)
        ),
    };
    // R-17: figures with no source anywhere is advice, never a failure.
    let mut notes = Vec::new();
    let mut figures = false;
    let mut sourced = false;
    for s in beat_scenes(project) {
        for w in walk_scene(s) {
            if let LayerKind::Text(t) = &w.layer.kind {
                if is_decorative(&w.layer.id, &w.layer.kind) {
                    continue;
                }
                figures |= is_figure(&t.text);
                sourced |= is_provenance(&t.text);
            }
        }
    }
    if figures && !sourced {
        notes.push(
            "note_unsourced_numbers: the story shows number figures and no beat carries a source (add a source tag, e.g. meaning: 'SmartAsset, 2024')"
                .to_string(),
        );
    }
    let mut axes = Vec::new();
    let mut sum = 0u32;
    let mut scored = 0u32;
    for axis in AXES {
        if axis == "audio" {
            axes.push(AxisScore {
                axis: axis.to_string(),
                score: None,
                findings: 0,
            });
            continue;
        }
        let n = findings.iter().filter(|f| f.axis == axis).count();
        let score = 5u8.saturating_sub(n.min(5) as u8);
        sum += score as u32;
        scored += 1;
        axes.push(AxisScore {
            axis: axis.to_string(),
            score: Some(score),
            findings: n,
        });
    }
    let total = if scored == 0 {
        0.0
    } else {
        ((sum as f32 * 40.0 / (scored as f32 * 5.0)) * 10.0).round() / 10.0
    };
    Critique {
        total,
        axes,
        findings,
        not_measurable: slop.not_measurable,
        reading,
        dials: shown,
        measured,
        notes,
        soul: Some(soul_metrics(project)),
    }
}

impl Critique {
    /// At most 14 lines: total, the design read, one per axis, top 3
    /// findings, then the notes.
    pub fn to_text(&self) -> String {
        let mut out = format!("critique: {:.1}/40\n", self.total);
        if !self.reading.is_empty() {
            out.push_str(&self.reading);
            out.push('\n');
        }
        for a in &self.axes {
            let s = a.score.map_or("n/a".to_string(), |s| format!("{s}/5"));
            out.push_str(&format!(
                "  {:<16} {} ({} finding(s))\n",
                a.axis, s, a.findings
            ));
        }
        for f in self.findings.iter().take(3) {
            out.push_str(&format!(
                "  ! {} {} {}: {}\n",
                f.scene, f.layer, f.check, f.detail
            ));
        }
        if let Some(m) = &self.soul {
            out.push_str(&format!(
                "  soul: overlap {:.2}, arc {:.2} ({}), stress {:.2} ({}), hold {:.2}, carry {}, duration_cv {:.2}, things_appear {}, same_frame {}\n",
                m.overlap_ratio,
                m.arc_ratio,
                m.arc_moves,
                m.stress_alignment,
                m.stresses,
                m.hold_ratio,
                m.carry_count,
                m.duration_cv,
                m.things_appear,
                m.same_frame_beats
            ));
        }
        for n in self.notes.iter().take(2) {
            out.push_str(&format!("  note: {n}\n"));
        }
        out
    }
}
