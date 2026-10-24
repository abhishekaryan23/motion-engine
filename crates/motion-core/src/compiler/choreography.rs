//! Choreography v0 (0.8): the WHEN layer for music. Retimes beat durations so
//! scene handoffs land on downbeats, BEFORE lifecycle planning. Taste tables
//! are never edited; without a MusicPlan nothing here runs. See
//! docs/SOUND_DESIGN.md §7.
//!
//! (0.24 Wave 1) Motion soul: HOW things arrive relative to each other.
//! [`plan`] is the one place a non-caption entrance gets its timing from the
//! look's [`MotionSignature`]: the lead lands on its speech-locked time, the
//! supporting layers overlap it by `role_lag`, ambient layers are already
//! moving. [`choreograph`] applies it to a built beat scene after the word
//! cues have placed every group (so WHEN stays the speech lock's: nothing but
//! ambient layers ever moves earlier, and none earlier than the lock allows).
//! See docs/MOTION_LANGUAGE.md.

use std::collections::HashMap;

use crate::compiler::signature::{self, MotionSignature};
pub use crate::compiler::speech_plan::Stress;
use crate::easing::Easing;
use crate::layout_qa::is_decorative;
use crate::scene::{Channel, Layer, LayerKind, Motion, MotionOp, Scene, SpringSpec};

/// How a layer relates to the beat's lead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The thing the word names: lands on the speech-locked time.
    Lead,
    /// Text, pictures and cards that explain the lead; overlap it.
    Support,
    /// Rules, bars and stamps that punctuate it.
    Accent,
    /// Ghost words and atmosphere: already moving when the lead lands.
    Ambient,
}

impl Role {
    pub fn index(self) -> usize {
        match self {
            Role::Lead => 0,
            Role::Support => 1,
            Role::Accent => 2,
            Role::Ambient => 3,
        }
    }
}

/// A beat's phases, from its lifecycle (scene-local seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct Phrase {
    pub beat: usize,
    pub t_enter: f64,
    pub t_read: f64,
    pub t_evolve: f64,
    /// (Wave 2) The beat's spoken stresses, by time (at most three).
    pub stresses: Vec<Stress>,
    /// (Wave 2) The breath before the payoff stress: everything at rest in
    /// this window (scene-local), when the beat has room for one.
    pub hold_before_payoff: Option<(f64, f64)>,
}

impl Phrase {
    pub fn new(beat: usize, life: crate::scene::Lifecycle, stresses: &[Stress]) -> Phrase {
        Phrase {
            beat,
            t_enter: life.enter,
            t_read: life.read,
            t_evolve: life.evolve,
            stresses: stresses.to_vec(),
            hold_before_payoff: None,
        }
    }

    /// The stress a word at `t` (within `tol`) carries, if any.
    pub fn stress_at(&self, t: f64, tol: f64) -> Option<&Stress> {
        self.stresses
            .iter()
            .filter(|s| (s.t - t).abs() <= tol)
            .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
    }

    /// The heaviest stress (the latest of equals): the payoff of the beat.
    pub fn payoff(&self) -> Option<&Stress> {
        self.stresses
            .iter()
            .max_by(|a, b| a.weight.cmp(&b.weight).then(a.t.total_cmp(&b.t)))
    }
}

/// What a grammar asks for: a role at a speech-locked time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cue {
    pub role: Role,
    /// The word's reveal time `t_w` (the lead's lock).
    pub at: f64,
    /// -1 left / +1 right: the side a slide comes from (`slide_side`).
    pub side: f32,
    /// (Wave 2) Onset of the spoken stress this lead carries: it peaks on it.
    pub stress: Option<f64>,
}

/// What it gets back. Travel is in `u` (px at a 1080 px short side).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub start: f64,
    pub dur: f64,
    pub spring: Option<SpringSpec>,
    pub ease: Easing,
    pub travel: [f32; 2],
    /// Seconds of counter-move before the travel (None = straight in).
    pub anticipate: Option<f64>,
    /// Share of the travel the counter-move covers.
    pub counter: f32,
    /// Start scale of a lead (1 = no scale).
    pub scale_from: f32,
    /// Scale the lead rises to before settling (1 = no overshoot).
    pub peak: f32,
    /// Seconds from start to that peak (None = 40 % of `dur`).
    pub up_secs: Option<f64>,
    /// (Wave 2) Onset of the stress the peak lands on (tags the motion).
    pub stress_at: Option<f64>,
    /// (Wave 2) Bulge of a slide's arc as a share of its chord (0 = line).
    pub arc: f32,
}

/// No entrance starts earlier than its word's lock by more than this (s).
pub const LOCK_LEAD: f64 = 0.12;
/// A lead's scale peak on its way to rest.
pub const SCALE_PEAK: f32 = 1.04;
const COUNTER_SECS: f64 = 0.1;
/// Shortest slide (u) that travels an arc: 3 % of a 1080 px width.
const ARC_MIN_CHORD: f32 = 32.4;
const SUPPORT_SPRING: SpringSpec = SpringSpec {
    stiffness: 220.0,
    damping: 24.0,
    mass: 1.0,
};
const FRAME: f64 = 1.0 / 30.0;

/// Timing for one entrance. Pure and seedless; `start >= at - LOCK_LEAD`.
pub fn plan(sig: &MotionSignature, phrase: &Phrase, cue: &Cue) -> Plan {
    let i = cue.role.index();
    let mut start = (cue.at + sig.role_lag[i])
        .max(cue.at - LOCK_LEAD)
        .max(phrase.t_enter.min(cue.at))
        .max(0.0);
    // A stressed lead peaks on its word: it starts a rise (LOCK_LEAD, never
    // earlier than the speech lock allows) before the onset.
    let stressed = (cue.role == Role::Lead && sig.punch.0 > 1.0)
        .then_some(cue.stress)
        .flatten();
    if let Some(t) = stressed {
        start = (t - LOCK_LEAD)
            .max(cue.at)
            .max(phrase.t_enter.min(cue.at))
            .max(0.0);
    }
    let dur = sig.hero_secs * sig.role_dur_mul[i] as f64;
    let (spring, ease, travel, scale_from) = match cue.role {
        Role::Lead => (
            Some(sig.spring),
            Easing::OutQuint,
            [0.0, 0.0],
            sig.hero_from,
        ),
        Role::Support => (
            Some(SUPPORT_SPRING),
            sig.slide_ease,
            [cue.side * sig.slide_px, 0.0],
            1.0,
        ),
        Role::Accent => (
            Some(SUPPORT_SPRING),
            Easing::OutCubic,
            [0.0, 0.5 * sig.slide_px],
            1.0,
        ),
        Role::Ambient => (None, Easing::InOutCubic, [0.0, 0.0], 1.0),
    };
    let moves = travel != [0.0, 0.0];
    let counter = if moves { sig.anticipate_frac } else { 0.0 };
    Plan {
        start,
        dur,
        spring,
        ease,
        travel,
        anticipate: (counter > 0.0).then_some(COUNTER_SECS.min(dur * 0.4)),
        counter,
        scale_from,
        peak: if stressed.is_some() {
            sig.punch.0
        } else {
            SCALE_PEAK
        },
        up_secs: stressed.map(|t| (t - start).clamp(0.05, dur - 0.1)),
        stress_at: stressed,
        arc: if moves { sig.arc_frac } else { 0.0 },
    }
}

/// The role of a layer by the name it was given below the beat prefix.
pub fn role_of(id: &str) -> Role {
    let mut segs = id.split('.');
    let first = segs.next().unwrap_or("");
    let prefixed =
        first.starts_with('b') && first.len() > 1 && first[1..].chars().all(|c| c.is_ascii_digit());
    let name = if prefixed {
        segs.next().unwrap_or("")
    } else {
        first
    };
    let starts = |ps: &[&str]| ps.iter().any(|p| name.starts_with(p));
    if starts(&["ghost", "backdrop", "scrim", "dust", "glow", "bokeh", "web"]) {
        Role::Ambient
    } else if starts(&[
        "head", "hero", "word", "title", "value", "figure", "amount", "count", "num",
    ]) {
        Role::Lead
    } else if starts(&[
        "rule",
        "underline",
        "accent",
        "stamp",
        "slab",
        "band",
        "bar",
        "marker",
        "strike",
        "tab",
    ]) {
        Role::Accent
    } else {
        Role::Support
    }
}

/// A snapped beat never shrinks below this fraction of its planned duration.
pub const MIN_STRETCH: f64 = 0.85;
/// A snapped beat never grows above this fraction of its planned duration.
pub const MAX_STRETCH: f64 = 1.20;
/// Absolute floor for a snapped beat (seconds).
pub const MIN_BEAT_SECONDS: f64 = 2.0;

/// Snap each handoff to the nearest downbeat within `tolerance` by changing
/// the duration of the beat BEFORE it. Pure and deterministic.
///
/// `durations[i]` and `overlaps_out[i]` are beat `i`'s planned duration and
/// outgoing overlap (seconds). Beat `i` starts at
/// `sum_{j<i} (durations[j] - overlaps_out[j])`; the handoff into beat `i ≥ 1`
/// is anchored at `start_i + overlaps_out[i-1] / 2` (the overlap midpoint,
/// the same anchor `plan_audio` uses). Handoffs are processed in order with
/// the already-snapped durations. A handoff snaps to the nearest downbeat `b`
/// with `|b - anchor| <= tolerance` (ties → the earlier downbeat) when the new
/// duration stays within `[MIN_STRETCH·d, MAX_STRETCH·d]` and `>=
/// MIN_BEAT_SECONDS`; otherwise it is left unchanged. Results are rounded to
/// milliseconds. Returns the new durations (same length).
pub fn snap_handoffs(
    durations: &[f64],
    overlaps_out: &[f64],
    downbeats: &[f64],
    tolerance: f64,
) -> Vec<f64> {
    let mut out = durations.to_vec();
    if downbeats.is_empty() || out.len() < 2 {
        return out;
    }
    let mut sorted: Vec<f64> = downbeats.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let overlap = |i: usize| overlaps_out.get(i).copied().unwrap_or(0.0);

    // Start of beat `i` from the (already snapped) preceding durations.
    let mut start = 0.0;
    for i in 1..out.len() {
        start += out[i - 1] - overlap(i - 1);
        let anchor = start + overlap(i - 1) / 2.0;
        // Nearest downbeat within tolerance; strict `<` keeps the earlier on ties.
        let mut best: Option<f64> = None;
        for &b in &sorted {
            let d = (b - anchor).abs();
            if d <= tolerance && best.is_none_or(|c| d < (c - anchor).abs()) {
                best = Some(b);
            }
        }
        let Some(b) = best else { continue };
        let planned = durations[i - 1];
        // The anchor moves one-for-one with the previous beat's duration.
        let new_d = round_ms(out[i - 1] + (b - anchor));
        if new_d >= MIN_BEAT_SECONDS
            && new_d >= planned * MIN_STRETCH - 1e-9
            && new_d <= planned * MAX_STRETCH + 1e-9
        {
            start += new_d - out[i - 1];
            out[i - 1] = new_d;
        }
    }
    out
}

fn round_ms(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// ---------------------------------------------------------------------------
// The beat pass
// ---------------------------------------------------------------------------

/// Longest motion that still counts as an entrance (s).
const ENTRY_MAX_SECS: f64 = 1.2;
/// Motion id of what [`choreograph`] adds (builders never set it).
pub const PASS_ID: &str = "choreography";

fn fade_in(op: &MotionOp) -> bool {
    matches!(op, MotionOp::Fade { from, to } if *from <= 0.5 && to > from)
}

fn is_entry_op(op: &MotionOp) -> bool {
    fade_in(op)
        || matches!(
            op,
            MotionOp::Move { .. }
                | MotionOp::Scale { .. }
                | MotionOp::MaskReveal { .. }
                | MotionOp::ClipReveal { .. }
                | MotionOp::AccentExpand { .. }
                | MotionOp::GlyphCascade { .. }
        )
}

fn arrives_op(op: &MotionOp) -> bool {
    is_entry_op(op) && !fade_in(op)
}

struct Walked<'a> {
    layer: &'a Layer,
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

fn is_stage(id: &str) -> bool {
    id.ends_with(".stage") || id.ends_with(".stage_front")
}

/// Whether `mo` (not in `skip`) writes `channel` of `target` anywhere in
/// `[from, to)`.
fn channel_busy(
    motions: &[Motion],
    skip: &[usize],
    target: &str,
    channel: Channel,
    from: f64,
    to: f64,
) -> bool {
    motions.iter().enumerate().any(|(i, m)| {
        !skip.contains(&i)
            && m.target == target
            && m.op.channel() == channel
            && m.start < to - 1e-6
            && m.start + m.duration > from + 1e-6
    })
}

/// The entrance cluster of a layer: indices of its entrance motions around
/// `first`.
fn cluster(motions: &[Motion], id: &str, first: f64) -> Vec<usize> {
    motions
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.target == id
                && is_entry_op(&m.op)
                && m.duration <= ENTRY_MAX_SECS
                && m.start >= first - 1e-6
                && m.start <= first + 0.7
        })
        .map(|(i, _)| i)
        .collect()
}

/// The largest shift (same sign as `delta`, at most `|delta|`) that moves the
/// cluster without overlapping another motion on a shared channel, leaving
/// `[0, limit]`.
fn allowed_shift(motions: &[Motion], idx: &[usize], delta: f64, limit: f64) -> f64 {
    let mut ok = delta;
    for &i in idx {
        let m = &motions[i];
        let (s, e) = (m.start, m.start + m.duration);
        if delta > 0.0 {
            ok = ok.min(limit - e);
        } else {
            ok = ok.max(-s);
        }
        for (j, o) in motions.iter().enumerate() {
            if idx.contains(&j) || o.target != m.target || o.op.channel() != m.op.channel() {
                continue;
            }
            if delta > 0.0 && o.start >= e - 1e-6 {
                ok = ok.min(o.start - e);
            } else if delta < 0.0 && o.start + o.duration <= s + 1e-6 {
                ok = ok.max(-(s - (o.start + o.duration)));
            }
        }
    }
    if delta > 0.0 {
        ok.max(0.0)
    } else {
        ok.min(0.0)
    }
}

fn shift_cluster(motions: &mut [Motion], idx: &[usize], delta: f64) {
    for &i in idx {
        // Not rounded: sequential motions of one cluster stay exactly adjacent.
        motions[i].start += delta;
    }
}

/// One motion grammar for every layout (0.24 Wave 1): role offsets, a masked
/// rise for text that would only fade in, a slide with a counter-move for
/// pictures and cards that would only fade in. Runs on a built beat scene
/// after the word cues, so the speech lock has already placed everything;
/// layers only ever move later (ambient ones earlier, to where they are
/// already moving when the lead lands). Slam cuts (`.cut.`) carry their own
/// helpers. Pure and seedless.
pub(crate) fn choreograph(scene: &mut Scene, sig: &MotionSignature, u: f32, stresses: &[Stress]) {
    let Some(life) = scene.lifecycle else { return };
    let Some(index) = scene
        .id
        .strip_prefix("beat_")
        .and_then(|n| n.parse::<usize>().ok())
    else {
        return;
    };
    let side = signature::slide_side(index.saturating_sub(1));
    let phrase = Phrase::new(index, life, stresses);
    let mut layers = Vec::new();
    walk(&scene.layers, &mut Vec::new(), &mut layers);
    let eligible = |w: &Walked| {
        !is_stage(&w.layer.id)
            && !w.layer.id.contains(".cut.")
            && (!is_decorative(&w.layer.id, &w.layer.kind) || role_of(&w.layer.id) == Role::Ambient)
    };
    let mut motions = std::mem::take(&mut scene.motions);

    // First entrance start per layer (a fade in or a reveal before ANTICIPATE).
    let mut first: HashMap<&str, f64> = HashMap::new();
    for m in &motions {
        if (fade_in(&m.op)
            || matches!(
                m.op,
                MotionOp::MaskReveal { .. } | MotionOp::ClipReveal { .. }
            ))
            && m.start < life.anticipate
            && m.duration <= ENTRY_MAX_SECS
        {
            let e = first.entry(m.target.as_str()).or_insert(m.start);
            *e = e.min(m.start);
        }
    }
    let counted: Vec<&str> = motions
        .iter()
        .filter(|m| matches!(m.op, MotionOp::Count { .. }))
        .map(|m| m.target.as_str())
        .collect();
    let first: HashMap<String, f64> = first.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let counted: Vec<String> = counted.into_iter().map(str::to_string).collect();

    // 1. Role offsets: the lead keeps its time, the rest overlap it.
    let lead_start = layers
        .iter()
        .filter(|w| eligible(w) && role_of(&w.layer.id) == Role::Lead)
        .filter_map(|w| first.get(&w.layer.id).copied())
        .fold(f64::MAX, f64::min);
    let mut shifted: HashMap<String, f64> = HashMap::new();
    if lead_start < f64::MAX {
        for w in layers.iter().filter(|w| eligible(w)) {
            let id = w.layer.id.as_str();
            if counted.iter().any(|c| c == id) {
                continue;
            }
            let Some(&f) = first.get(id) else {
                // Children ride on a shifted group.
                continue;
            };
            let role = role_of(id);
            let inherited = w
                .ancestors
                .iter()
                .rev()
                .find_map(|a| shifted.get(*a).copied());
            let wanted = match (inherited, role) {
                (Some(d), _) => d,
                (None, Role::Lead) => 0.0,
                (None, Role::Support | Role::Accent) => {
                    let want = lead_start + sig.role_lag[role.index()];
                    if f >= lead_start - 1e-6 && f < want - 1e-6 {
                        want - f
                    } else {
                        0.0
                    }
                }
                // Already moving when the lead lands, never before 0.
                (None, Role::Ambient) => {
                    if f <= lead_start + 1e-6 {
                        -(-sig.role_lag[3]).min(f)
                    } else {
                        0.0
                    }
                }
            };
            if wanted.abs() < FRAME * 0.99 {
                continue;
            }
            let idx = cluster(&motions, id, f);
            if idx.is_empty() {
                continue;
            }
            let delta = allowed_shift(&motions, &idx, wanted, life.anticipate);
            if delta.abs() < FRAME * 0.99 {
                continue;
            }
            shift_cluster(&mut motions, &idx, delta);
            shifted.insert(id.to_string(), delta);
        }
    }

    // 2. Arrivals: nothing only fades in.
    let kinds_of = |motions: &[Motion], ids: &[&str]| -> (bool, bool, f64) {
        let (mut fade, mut arrives, mut at) = (false, false, f64::MAX);
        for m in motions {
            if ids.contains(&m.target.as_str())
                && is_entry_op(&m.op)
                && m.duration <= ENTRY_MAX_SECS
                && m.start < life.anticipate
            {
                if fade_in(&m.op) {
                    fade = true;
                    at = at.min(m.start);
                } else if arrives_op(&m.op) {
                    arrives = true;
                }
            }
        }
        (fade, arrives, at)
    };
    let before = motions.len();
    let mut splits: Vec<String> = Vec::new();
    for w in layers.iter().filter(|w| eligible(w)) {
        let l = w.layer;
        let mut ids: Vec<&str> = w.ancestors.clone();
        ids.push(l.id.as_str());
        let (fade, arrives, at) = kinds_of(&motions, &ids);
        if !fade || arrives || counted.iter().any(|c| c == &l.id) {
            continue;
        }
        let role = role_of(&l.id);
        match &l.kind {
            LayerKind::Text(t) if !t.text.trim().is_empty() && !is_decorative(&l.id, &l.kind) => {
                let mut p = plan(
                    sig,
                    &phrase,
                    &Cue {
                        role: if role == Role::Lead {
                            Role::Lead
                        } else {
                            Role::Support
                        },
                        at,
                        side,
                        stress: None,
                    },
                );
                p.start = at;
                p.dur = sig.line_secs;
                let lines = t.text.split('\n').count();
                let span = p.dur + sig.line_stagger * lines.saturating_sub(1) as f64;
                if channel_busy(&motions, &[], &l.id, Channel::Clip, at, at + span) {
                    continue;
                }
                let rise = if channel_busy(&motions, &[], &l.id, Channel::Offset, at, at + span) {
                    0.0
                } else {
                    sig.line_rise * t.font_size * t.line_height
                };
                // (Wave 2) A headline of several lines reveals line by line,
                // each line a layer of its own, staggered.
                if lines > 1 && t.max_width.is_none() && l.layout.is_none() {
                    for i in 0..lines {
                        let line = Plan {
                            start: p.start + sig.line_stagger * i as f64,
                            ..p
                        };
                        let mut v = crate::compiler::mo::reveal_lines(
                            &format!("{}.l{i}", l.id),
                            &line,
                            1,
                            rise,
                            0.0,
                        );
                        if rise == 0.0 {
                            v.truncate(1);
                        }
                        motions.extend(v);
                    }
                    splits.push(l.id.clone());
                    continue;
                }
                let mut v =
                    crate::compiler::mo::reveal_lines(&l.id, &p, lines, rise, sig.line_stagger);
                if rise == 0.0 {
                    v.truncate(1);
                }
                motions.extend(v);
            }
            LayerKind::Image { .. } | LayerKind::Svg { .. } | LayerKind::Group { .. }
                if role == Role::Support
                    && !is_decorative(&l.id, &l.kind)
                    && w.ancestors.iter().all(|a| is_stage(a)) =>
            {
                let mut p = plan(
                    sig,
                    &phrase,
                    &Cue {
                        role: Role::Support,
                        at,
                        side,
                        stress: None,
                    },
                );
                p.start = at;
                if channel_busy(&motions, &[], &l.id, Channel::Offset, at, at + p.dur) {
                    continue;
                }
                motions.extend(crate::compiler::mo::slide(&l.id, &p, u));
            }
            _ => {}
        }
    }
    // Nothing may run past the scene.
    let end = scene.duration_seconds;
    // (Wave 2) Pictures, cards and accents travel arcs, not chords: a straight
    // slide of 3 % of the width or more gets its control point.
    let arcing: Vec<&str> = layers
        .iter()
        .filter(|w| {
            eligible(w)
                && !matches!(w.layer.kind, LayerKind::Text(_))
                && matches!(role_of(&w.layer.id), Role::Support | Role::Accent)
        })
        .map(|w| w.layer.id.as_str())
        .collect();
    for m in motions.iter_mut() {
        if let MotionOp::Move { from, to, via } = &mut m.op {
            let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
            if via.is_none()
                && sig.arc_frac > 0.0
                && arcing.contains(&m.target.as_str())
                && (dx * dx + dy * dy).sqrt() >= ARC_MIN_CHORD * u
            {
                *via = Some([
                    (from[0] + to[0]) / 2.0 - dy * sig.arc_frac,
                    (from[1] + to[1]) / 2.0 + dx * sig.arc_frac,
                ]);
            }
        }
    }
    // (Wave 2) The stress punch of a lead headline and the breath of a count
    // landing, on layers whose scale channel is free.
    let mut punched = false;
    for w in layers.iter().filter(|w| eligible(w)) {
        let l = w.layer;
        let LayerKind::Text(t) = &l.kind else {
            continue;
        };
        if is_decorative(&l.id, &l.kind) || t.text.trim().is_empty() || sig.punch.0 <= 1.0 {
            continue;
        }
        let ids: Vec<&str> = w
            .ancestors
            .iter()
            .copied()
            .chain(std::iter::once(l.id.as_str()))
            .collect();
        if let Some(land) = motions
            .iter()
            .filter(|m| m.target == l.id && matches!(m.op, MotionOp::Count { .. }))
            .map(|m| m.start + m.duration)
            .reduce(f64::max)
        {
            let (a, z) = (land - LOCK_LEAD, land + 0.18);
            if z < life.anticipate
                && ids
                    .iter()
                    .all(|id| !channel_busy(&motions, &[], id, Channel::Scale, a, z))
            {
                let mut v =
                    crate::motion::kinetic::stress_punch(&l.id, land, LOCK_LEAD, 0.18, SCALE_PEAK);
                v[0].id = None;
                motions.extend(v);
            }
            continue;
        }
        if punched || role_of(&l.id) != Role::Lead {
            continue;
        }
        let words: Vec<String> = t
            .text
            .split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect()
            })
            .collect();
        let Some(st) = stresses.iter().filter(|s| s.weight >= 1).find(|s| {
            s.word.split_whitespace().any(|sw| {
                let sw: String = sw
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect();
                !sw.is_empty() && words.contains(&sw)
            })
        }) else {
            continue;
        };
        let (a, z) = (st.t - LOCK_LEAD, st.t + sig.punch.1);
        let shown = first.get(&l.id).copied().unwrap_or(0.0);
        if a < shown + 0.05
            || z >= life.anticipate
            || ids
                .iter()
                .any(|id| channel_busy(&motions, &[], id, Channel::Scale, a, z))
        {
            continue;
        }
        motions.extend(crate::motion::kinetic::stress_punch(
            &l.id,
            st.t,
            LOCK_LEAD,
            sig.punch.1,
            sig.punch.0,
        ));
        punched = true;
    }
    let mut added = motions.split_off(before);
    added.retain_mut(|m| {
        if m.id.is_none() {
            m.id = Some(PASS_ID.to_string());
        }
        if m.start + m.duration > end {
            m.duration = ((end - m.start) * 1000.0).floor() / 1000.0;
        }
        m.duration >= 0.05
    });
    motions.extend(added);
    scene.motions = motions;
    for id in splits {
        split_lines(&mut scene.layers, &id);
    }
}

/// (0.24 Wave 2) What a beat is for, to the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatRole {
    /// The opening beat: one push in, then hold.
    Hook,
    /// Everything between: a slow track while nothing is being read.
    Body,
    /// The answer: still until its stress, then one push.
    Payoff,
}

/// (0.24 Wave 2) The camera of a beat with intent: replaces the 2D drift
/// (push + track over the whole beat) by the look's [`CameraIntent`]. A
/// hook pushes in once and holds; a body beat tracks one way, but not inside
/// READ; the payoff keeps still until its stress and then pushes in. Shakes
/// stay; perspective cameras and beats without a camera are left alone.
pub(crate) fn camera_pass(
    scene: &mut Scene,
    sig: &MotionSignature,
    role: BeatRole,
    phrase: &Phrase,
    u: f32,
) {
    use crate::scene::{CameraMotion, CameraOp};
    let Some(life) = scene.lifecycle else { return };
    let end = scene.duration_seconds;
    let Some(cam) = scene.camera.as_mut() else {
        return;
    };
    if cam.perspective.is_some() {
        return;
    }
    cam.motions
        .retain(|m| !matches!(m.op, CameraOp::Push { .. } | CameraOp::Track { .. }));
    let ci = sig.camera;
    let r3 = |x: f64| (x * 1000.0).round() / 1000.0;
    let stress = phrase
        .payoff()
        .map(|s| s.t)
        .filter(|t| *t < life.anticipate);
    let mut moves: Vec<CameraMotion> = Vec::new();
    // Every beat then drifts one way at the body speed (the life of the
    // frame while it is read); the hook and the payoff push first.
    let drift_from = match (role, stress) {
        (BeatRole::Hook, _) => {
            let secs = ci.hook_secs.min(end);
            moves.push(CameraMotion {
                start: 0.0,
                duration: r3(secs),
                easing: Easing::OutCubic,
                velocity: None,
                op: CameraOp::Push {
                    from: 1.0,
                    to: 1.0 + ci.hook_push.min(sig.camera_push_max),
                },
            });
            secs
        }
        (BeatRole::Payoff, Some(t)) => {
            let secs = ci.payoff_secs.min((end - t).max(0.1));
            moves.push(CameraMotion {
                start: r3(t),
                duration: r3(secs),
                easing: Easing::OutCubic,
                velocity: None,
                op: CameraOp::Push {
                    from: 1.0,
                    to: 1.0 + ci.payoff_push.min(sig.camera_push_max),
                },
            });
            t + secs
        }
        _ => 0.0,
    };
    if end - drift_from >= 0.5 {
        let d = ci.body_speed * u * signature::slide_side(phrase.beat) * (end - drift_from) as f32;
        moves.push(CameraMotion {
            start: r3(drift_from),
            duration: r3(end - drift_from),
            easing: Easing::Linear,
            velocity: None,
            op: CameraOp::Track {
                from: [0.0, 0.0],
                to: [d, -d * 0.4],
            },
        });
    }
    cam.motions.extend(moves);
}

/// Value of an eased motion at scene time `t` as progress.
fn progress(m: &Motion, t: f64) -> f32 {
    m.easing
        .apply(((t - m.start) / m.duration.max(1e-6)).clamp(0.0, 1.0)) as f32
}

/// (0.24 Wave 2) The breath before the payoff: everything at rest for
/// 0.35 to 0.5 s ending where the lead's rise to the beat's heaviest stress
/// begins. The window is the free stretch before that stress (no entrance, no
/// shake, no camera move in it); long drifts (ambient move / scale / rotate
/// of 1.5 s or more, camera tracks) are paused across it and resume where
/// they left off. Returns the window, when the beat has one.
pub(crate) fn hold_pass(scene: &mut Scene, phrase: &Phrase) -> Option<(f64, f64)> {
    use crate::scene::CameraOp;
    let life = scene.lifecycle?;
    let stress = phrase.payoff()?;
    let we = stress.t - LOCK_LEAD;
    if we < life.enter + 0.4 || stress.t >= life.anticipate {
        return None;
    }
    let freezable = |m: &Motion| {
        m.duration >= 1.5
            && m.spring.is_none()
            && matches!(
                m.op,
                MotionOp::Move { .. } | MotionOp::Scale { .. } | MotionOp::Rotate { .. }
            )
            && !m.target.ends_with(".stage")
    };
    let wmin = we - 0.5;
    let mut ws = wmin;
    for m in &scene.motions {
        if freezable(m) || m.duration <= 0.011 {
            continue;
        }
        let e = m.start + m.duration;
        if m.start < we - 1e-6 && e > wmin + 1e-6 {
            ws = ws.max(e);
        }
    }
    if let Some(c) = &scene.camera {
        for m in &c.motions {
            let e = m.start + m.duration;
            let drift = matches!(m.op, CameraOp::Track { .. }) && m.duration >= 1.5;
            if !drift && m.start < we - 1e-6 && e > wmin + 1e-6 {
                ws = ws.max(e);
            }
        }
    }
    if we - ws < 0.35 {
        return None;
    }
    let (ws, we) = (
        (ws * 1000.0).ceil() / 1000.0,
        (we * 1000.0).floor() / 1000.0,
    );
    let mut out: Vec<Motion> = Vec::new();
    for m in std::mem::take(&mut scene.motions) {
        let e = m.start + m.duration;
        if freezable(&m) && m.start < we && e > ws {
            out.extend(pause(m, ws, we));
        } else {
            out.push(m);
        }
    }
    scene.motions = out;
    if let Some(c) = scene.camera.as_mut() {
        let mut cm = Vec::new();
        for m in std::mem::take(&mut c.motions) {
            let e = m.start + m.duration;
            let CameraOp::Track { from, to } = m.op else {
                cm.push(m);
                continue;
            };
            if !(m.duration >= 1.5 && m.start < we && e > ws) {
                cm.push(m);
                continue;
            }
            let at = |t: f64| {
                let p = progress_cam(&m, t);
                [
                    from[0] + (to[0] - from[0]) * p,
                    from[1] + (to[1] - from[1]) * p,
                ]
            };
            let piece = |s: f64, e: f64, a: [f32; 2], b: [f32; 2]| crate::scene::CameraMotion {
                start: (s * 1000.0).round() / 1000.0,
                duration: ((e - s) * 1000.0).round() / 1000.0,
                easing: Easing::Linear,
                velocity: None,
                op: CameraOp::Track { from: a, to: b },
            };
            if m.start < ws {
                cm.push(piece(m.start, ws, from, at(ws)));
            }
            if e > we {
                cm.push(piece(we, e, at(we), to));
            }
        }
        c.motions = cm;
    }
    Some((ws, we))
}

fn progress_cam(m: &crate::scene::CameraMotion, t: f64) -> f32 {
    m.easing
        .apply(((t - m.start) / m.duration.max(1e-6)).clamp(0.0, 1.0)) as f32
}

/// Pause the drift `m` over `[ws, we]`: it runs to its value at `ws`, rests,
/// and resumes from its value at `we` to its end (linear in the pieces).
fn pause(m: Motion, ws: f64, we: f64) -> Vec<Motion> {
    let (s, e) = (m.start, m.start + m.duration);
    let r3 = |x: f64| (x * 1000.0).round() / 1000.0;
    let lerp = |a: f32, b: f32, p: f32| a + (b - a) * p;
    let (pa, pz) = (progress(&m, ws), progress(&m, we));
    let part = |from_t: f64, to_t: f64, p0: f32, p1: f32| -> Option<Motion> {
        if to_t - from_t < 0.05 {
            return None;
        }
        let op = match &m.op {
            MotionOp::Move { from, to, via } => MotionOp::Move {
                from: [lerp(from[0], to[0], p0), lerp(from[1], to[1], p0)],
                to: [lerp(from[0], to[0], p1), lerp(from[1], to[1], p1)],
                via: *via,
            },
            MotionOp::Scale { from, to, axis } => MotionOp::Scale {
                from: lerp(*from, *to, p0),
                to: lerp(*from, *to, p1),
                axis: *axis,
            },
            MotionOp::Rotate { from, to } => MotionOp::Rotate {
                from: lerp(*from, *to, p0),
                to: lerp(*from, *to, p1),
            },
            other => other.clone(),
        };
        Some(Motion {
            id: m.id.clone(),
            target: m.target.clone(),
            start: r3(from_t),
            duration: r3(to_t - from_t),
            easing: Easing::Linear,
            spring: None,
            op,
        })
    };
    let mut out = Vec::new();
    if s < ws {
        out.extend(part(s, ws, 0.0, pa));
        if e > we {
            out.extend(part(we, e, pz, 1.0));
        }
    } else if e > we {
        // Starts inside the rest: it waits for the end of it.
        out.extend(part(we, e, 0.0, 1.0));
    }
    out
}

/// (0.24 Wave 2) Replace the multi-line text layer `layer` by a group of the
/// same id and box holding one single-line text layer per line (`<id>.l<i>`),
/// so each line can reveal on its own. The group keeps the layer's transform,
/// opacity and depth, so every motion on the id still applies to all lines.
/// Returns the number of lines (`None`: not multi-line text, left as it was).
pub(crate) fn split_into_lines(layer: &mut Layer) -> Option<usize> {
    let LayerKind::Text(t) = &layer.kind else {
        return None;
    };
    if t.max_width.is_some() || layer.layout.is_some() || !t.text.contains('\n') {
        return None;
    }
    let lines: Vec<&str> = t.text.split('\n').collect();
    let advance = t.font_size * t.line_height;
    let id = layer.id.clone();
    let children: Vec<Layer> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let mut style = t.clone();
            style.text = (*line).to_string();
            style.ink = None;
            Layer {
                id: format!("{id}.l{i}"),
                x: 0.0,
                y: advance * i as f32,
                width: layer.width,
                height: advance,
                scale_x: 1.0,
                scale_y: 1.0,
                rotation_degrees: 0.0,
                anchor_x: 0.0,
                anchor_y: 0.0,
                opacity: 1.0,
                z_index: i as i32,
                visible: true,
                clip: None,
                mask: None,
                depth: None,
                z: None,
                tilt: None,
                layout: None,
                kind: LayerKind::Text(style),
            }
        })
        .collect();
    let n = children.len();
    layer.kind = LayerKind::Group { children };
    Some(n)
}

fn split_lines(layers: &mut [Layer], id: &str) -> bool {
    for l in layers.iter_mut() {
        if l.id == id {
            return split_into_lines(l).is_some();
        }
        if let LayerKind::Group { children } = &mut l.kind {
            if split_lines(children, id) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phrase() -> Phrase {
        Phrase {
            beat: 1,
            t_enter: 0.0,
            t_read: 1.0,
            t_evolve: 2.0,
            stresses: Vec::new(),
            hold_before_payoff: None,
        }
    }
    fn cue(role: Role, at: f64) -> Cue {
        Cue {
            role,
            at,
            side: 1.0,
            stress: None,
        }
    }

    #[test]
    fn roles_lag_the_lead_by_the_signature() {
        for sig in [
            MotionSignature::BASE,
            MotionSignature::HYPE,
            MotionSignature::DOCUMENTARY,
            MotionSignature::CINEMATIC,
        ] {
            let lead = plan(&sig, &phrase(), &cue(Role::Lead, 3.0));
            let sup = plan(&sig, &phrase(), &cue(Role::Support, 3.0));
            let acc = plan(&sig, &phrase(), &cue(Role::Accent, 3.0));
            assert_eq!(lead.start, 3.0);
            assert!((sup.start - 3.0 - sig.role_lag[1]).abs() < 1e-9);
            assert!((acc.start - 3.0 - sig.role_lag[2]).abs() < 1e-9);
            assert!(sup.start > lead.start && acc.start > sup.start);
            assert!(sup.dur > lead.dur && acc.dur < sup.dur);
        }
    }

    #[test]
    fn nothing_starts_earlier_than_the_speech_lock_allows() {
        let sig = MotionSignature::HYPE;
        let amb = plan(&sig, &phrase(), &cue(Role::Ambient, 3.0));
        assert!((amb.start - (3.0 - LOCK_LEAD)).abs() < 1e-9);
        let early = plan(&sig, &phrase(), &cue(Role::Ambient, 0.05));
        assert!(early.start >= 0.0);
        for role in [Role::Lead, Role::Support, Role::Accent, Role::Ambient] {
            for at in [0.0, 0.4, 2.5, 9.9] {
                let p = plan(&sig, &phrase(), &cue(role, at));
                assert!(p.start >= at - LOCK_LEAD - 1e-9 && p.start >= 0.0);
            }
        }
    }

    #[test]
    fn plan_is_deterministic_and_anticipates_only_travel() {
        let sig = MotionSignature::HYPE;
        let a = plan(&sig, &phrase(), &cue(Role::Support, 1.25));
        assert_eq!(a, plan(&sig, &phrase(), &cue(Role::Support, 1.25)));
        assert!(a.anticipate.is_some() && a.counter > 0.0);
        let lead = plan(&sig, &phrase(), &cue(Role::Lead, 1.25));
        assert!(lead.anticipate.is_none());
        assert!(lead.scale_from < 1.0 && lead.spring.is_some());
        let cin = plan(
            &MotionSignature::CINEMATIC,
            &phrase(),
            &cue(Role::Support, 1.0),
        );
        assert!(cin.anticipate.is_none());
    }

    #[test]
    fn role_names_map_to_roles() {
        assert_eq!(role_of("b2.head.0"), Role::Lead);
        assert_eq!(role_of("b2.ghost"), Role::Ambient);
        assert_eq!(role_of("b2.items.0.card"), Role::Support);
        assert_eq!(role_of("b2.rule"), Role::Accent);
    }

    fn life() -> crate::scene::Lifecycle {
        crate::scene::Lifecycle {
            enter: 0.3,
            settle: 1.2,
            read: 1.6,
            evolve: 4.0,
            anticipate: 5.0,
            bridge: 5.4,
        }
    }

    fn stress(t: f64, weight: u8) -> Stress {
        Stress {
            t,
            end: t + 0.3,
            word: "thirteen".into(),
            weight,
        }
    }

    #[test]
    fn a_stressed_lead_peaks_on_its_word_and_never_starts_before_the_lock() {
        let sig = MotionSignature::HYPE;
        let ph = Phrase::new(2, life(), &[stress(2.0, 2)]);
        let mut cue = cue(Role::Lead, 1.88);
        cue.stress = Some(2.0);
        let p = plan(&sig, &ph, &cue);
        assert!((p.start - 1.88).abs() < 1e-9, "{}", p.start);
        assert!((p.start + p.up_secs.unwrap() - 2.0).abs() < 1e-9);
        assert_eq!((p.peak, p.stress_at), (sig.punch.0, Some(2.0)));
        let m = crate::compiler::mo::arrive("w", &p, 1.0);
        let rise = m
            .iter()
            .find(|m| m.id.as_deref() == Some("stress:2.000"))
            .expect("tagged rise");
        assert!((rise.start + rise.duration - 2.0).abs() < 1e-6);
        // A look without a punch (cinematic) rises to rest, untagged.
        let c = plan(&MotionSignature::CINEMATIC, &ph, &cue);
        assert!(c.stress_at.is_none() && c.peak <= 1.0 + 1e-6 + (SCALE_PEAK - 1.0));
        assert_eq!(ph.payoff().map(|s| s.weight), Some(2));
        assert!(ph.stress_at(2.1, 0.15).is_some() && ph.stress_at(3.0, 0.15).is_none());
    }

    #[test]
    fn a_planned_slide_travels_an_arc_not_the_chord() {
        let sig = MotionSignature::HYPE;
        let ph = Phrase::new(2, life(), &[]);
        let p = plan(&sig, &ph, &cue(Role::Support, 1.0));
        let m = crate::compiler::mo::slide("pic", &p, 1.0);
        let MotionOp::Move { from, to, via } = &m[1].op else {
            panic!("move");
        };
        let via = via.expect("arc");
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        // The control point is off the chord, perpendicular to it.
        let (mx, my) = ((from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0);
        let dot = (via[0] - mx) * dx + (via[1] - my) * dy;
        assert!(dot.abs() < 1e-3);
        let off = ((via[0] - mx).powi(2) + (via[1] - my).powi(2)).sqrt();
        let chord = (dx * dx + dy * dy).sqrt();
        assert!((off / chord - sig.arc_frac).abs() < 1e-4);
        // The curve is at most 1.25x the chord (a quadratic of this bulge).
        assert!(1.0 + 2.0 * (off / chord).powi(2) < 1.25);
    }

    fn bare_scene(motions: Vec<Motion>, camera: Option<crate::scene::Camera>) -> Scene {
        Scene {
            id: "beat_2".into(),
            start_seconds: 0.0,
            duration_seconds: 6.0,
            layers: Vec::new(),
            motions,
            camera,
            lifecycle: Some(life()),
            post: Vec::new(),
        }
    }

    fn mv(target: &str, start: f64, dur: f64, from: [f32; 2], to: [f32; 2]) -> Motion {
        Motion {
            id: None,
            target: target.into(),
            start,
            duration: dur,
            easing: Easing::Linear,
            spring: None,
            op: MotionOp::Move {
                from,
                to,
                via: None,
            },
        }
    }

    #[test]
    fn camera_has_intent_per_beat_purpose() {
        use crate::scene::{Camera, CameraOp};
        let sig = MotionSignature::HYPE;
        let cam = || {
            Some(Camera {
                pivot: None,
                motions: vec![crate::scene::CameraMotion {
                    start: 0.0,
                    duration: 6.0,
                    easing: Easing::InOutCubic,
                    velocity: None,
                    op: CameraOp::Push {
                        from: 1.0,
                        to: 1.012,
                    },
                }],
                perspective: None,
            })
        };
        let ph = Phrase::new(2, life(), &[stress(3.0, 2)]);
        let ops = |role| {
            let mut s = bare_scene(Vec::new(), cam());
            camera_pass(&mut s, &sig, role, &ph, 1.0);
            s.camera.expect("camera").motions
        };
        let hook = ops(BeatRole::Hook);
        assert!(matches!(hook[0].op, CameraOp::Push { .. }) && hook[0].start == 0.0);
        let payoff = ops(BeatRole::Payoff);
        // Still until the stress, then one push.
        assert!(payoff.iter().all(|m| m.start >= 3.0 - 1e-9));
        assert!(matches!(payoff[0].op, CameraOp::Push { .. }) && payoff[0].start == 3.0);
        let body = ops(BeatRole::Body);
        assert!(matches!(body[0].op, CameraOp::Track { .. }) && body.len() == 1);
        // Never above the look's push budget.
        for m in hook.iter().chain(&payoff) {
            if let CameraOp::Push { to, .. } = m.op {
                assert!(to - 1.0 <= sig.camera_push_max + 1e-6);
            }
        }
        // Different beats differ.
        assert_ne!(hook[0].duration, body[0].duration);
        // No camera: nothing is made up.
        let mut none = bare_scene(Vec::new(), None);
        camera_pass(&mut none, &sig, BeatRole::Hook, &ph, 1.0);
        assert!(none.camera.is_none());
    }

    #[test]
    fn the_breath_before_the_payoff_pauses_drifts_and_resumes_them() {
        let ph = Phrase::new(2, life(), &[stress(3.0, 2)]);
        // A 6 s drift on the ghost and a shorter entrance that ended at 2.0.
        let drift = mv("b2.ghost", 0.0, 6.0, [0.0, 0.0], [60.0, 0.0]);
        let enter = mv("b2.card", 1.5, 0.5, [0.0, 40.0], [0.0, 0.0]);
        let mut s = bare_scene(vec![drift, enter], None);
        let w = hold_pass(&mut s, &ph).expect("hold");
        // The window ends where the lead's rise begins (3.0 - LOCK_LEAD) and
        // is 0.35-0.5 s long.
        assert!((w.1 - (3.0 - LOCK_LEAD)).abs() < 1e-6);
        assert!((0.35..=0.5 + 1e-6).contains(&(w.1 - w.0)), "{w:?}");
        let ghost: Vec<&Motion> = s
            .motions
            .iter()
            .filter(|m| m.target == "b2.ghost")
            .collect();
        assert_eq!(ghost.len(), 2);
        // Nothing runs inside the window, and the drift picks up at the same
        // speed (10 px/s) where it left off.
        for m in &s.motions {
            assert!(
                m.start + m.duration <= w.0 + 1e-6 || m.start >= w.1 - 1e-6,
                "{m:?}"
            );
        }
        let MotionOp::Move { to: a, .. } = ghost[0].op else {
            panic!()
        };
        let MotionOp::Move { from: b, .. } = ghost[1].op else {
            panic!()
        };
        assert!((a[0] - 10.0 * w.0 as f32).abs() < 0.1 && (b[0] - 10.0 * w.1 as f32).abs() < 0.1);
        // An entrance inside the window leaves no room: no hold.
        let busy = mv("b2.card", 2.3, 0.5, [0.0, 40.0], [0.0, 0.0]);
        let mut s2 = bare_scene(vec![busy], None);
        assert!(hold_pass(&mut s2, &ph).is_none());
        assert!(hold_pass(
            &mut bare_scene(Vec::new(), None),
            &Phrase::new(2, life(), &[])
        )
        .is_none());
    }

    #[test]
    fn a_multi_line_headline_splits_into_lines_that_keep_its_box() {
        let mut l = crate::scene::Layer {
            id: "b2.head.0".into(),
            x: 40.0,
            y: 100.0,
            width: 900.0,
            height: 300.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            opacity: 1.0,
            z_index: 3,
            visible: true,
            clip: None,
            mask: None,
            depth: None,
            z: None,
            tilt: None,
            layout: None,
            kind: LayerKind::Text(crate::scene::TextStyle {
                text: "INCOME\nIS NOT\nWEALTH".into(),
                font_role: crate::scene::FontRole::Display,
                font_size: 100.0,
                font_weight: 900,
                italic: false,
                color: crate::scene::Color::rgb(0, 0, 0),
                align: crate::scene::TextAlign::Left,
                line_height: 1.1,
                letter_spacing: 0.0,
                max_width: None,
                uppercase: false,
                ink: None,
                fill: None,
                outline: None,
            }),
        };
        assert_eq!(split_into_lines(&mut l), Some(3));
        assert_eq!(
            (l.id.as_str(), l.x, l.y, l.z_index),
            ("b2.head.0", 40.0, 100.0, 3)
        );
        let LayerKind::Group { children } = &l.kind else {
            panic!("group")
        };
        let ys: Vec<f32> = children.iter().map(|c| c.y).collect();
        assert_eq!(ys, [0.0, 110.0, 220.0]);
        assert_eq!(children[1].id, "b2.head.0.l1");
        // One line stays one layer.
        assert_eq!(split_into_lines(&mut children[0].clone()), None);
    }
}
