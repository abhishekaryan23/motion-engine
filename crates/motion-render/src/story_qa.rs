//! (0.23 W4) Story QA: three structural checks on the timeline that the
//! 0.22 reports never made (`dead_air`, `count_unsettled`, `repeat_template`;
//! names and thresholds are `motion_core::checks`). Everything here runs on
//! the timeline's resolved frames (`evaluate_frame`, frame resolution); no
//! pixels are drawn. The pixel check `text_local_contrast` lives in
//! [`crate::contrast_qa`].
//!
//! # Beats and content
//! A *beat scene* is a scene carrying a lifecycle (`audio::beat_scenes`)
//! other than the whole-piece `backdrop` and the word-synced `captions`
//! scenes. A *content leaf* is a readable leaf ([`readable_leaves`]: opacity,
//! blur, on canvas) of a beat scene, with its clip window at least half
//! filled ([`revealed_share`]), that is not decorative: none of its ids from
//! the top-level ancestor down is a furniture / atmosphere layer by the
//! compiler's id convention (`layout_qa::is_decorative`: `backdrop.*`,
//! textures, ghost words, wipes, folio / index / ticks / stickers ...).
//! Captions and the backdrop never count because their leaves belong to
//! other scenes; shared elements (`scene == None`) are persistent furniture
//! and never count either. Shape leaves that are not furniture by id (a beat's
//! `*.kicker_rule`, `*.accent_slab`) count as content, like the regression
//! helper; excluding them changes no verdict over the bench matrix.
//!
//! # `dead_air`
//! Per beat, the first frame (from the beat's start) with a content leaf.
//! FAIL when the first content of the video appears after
//! `DEAD_AIR_FIRST_READABLE_S`, or any beat shows none for longer than
//! `DEAD_AIR_MAX_HOLD_S` after its own start (or never).
//!
//! # `count_unsettled`
//! Every text layer driven by `MotionOp::Count`. The displayed text at a
//! frame is the timeline's own (the latest started count on the layer,
//! formatted by `timeline::format_count`; [`count_shown_at`] mirrors it).
//! *Settle* is the first frame whose displayed text equals the final text
//! (not the motion's end). The *anchor* (scene-local seconds) is, in order:
//! the start of the spoken number for that value (SpeechMap), else the
//! earliest spoken word of the layer's reveal group (`ArtRecord.reveals`),
//! else the beat's READ; never earlier than the count's own start. A running
//! total (two or more Count motions on one layer, stepping with items) is
//! anchored to the start of its last count, the arrival of the last item it
//! counts. FAIL when settle comes later than `anchor + COUNT_SETTLE_S`.
//! *Hold* is how long the layer stays readable with the final text, from
//! the first readable frame at or after the settle frame (a number that
//! finishes while its layer is hidden is not held) to the layer's
//! exit or the beat's ANTICIPATE, whichever comes first (the *exit*); under
//! `COUNT_HOLD_S` it is WARN only when `exit - count start` is under
//! `COUNT_HOLD_S + COUNT_MIN_S` (no room for a real count of `COUNT_MIN_S`
//! plus the hold: the beat, not the count, limits it), and FAIL otherwise.
//! This mirrors the compiler's `speech_plan::settle_counts`, which never fits
//! a count under `COUNT_MIN_S`.
//!
//! # `carry_continuity`
//! (0.23 W8) For every shared element (`MotionProject.shared`) with track keys
//! in two consecutive beat scenes N and N+1, the element is readable
//! ([`readable_leaves`]: compounded opacity of at least `READABLE_OPACITY`,
//! blur, box on canvas) at every frame from beat N's READ to beat N+1's READ
//! (frames whose time lies in `[READ_N, READ_N+1]`). A frame where the element
//! is not drawn at all (outside its first and last key, or culled) counts as
//! not readable. FAIL lists each gap: the element, the beats, the first and
//! last frame time without it and why. SKIP when no element has keys in two
//! consecutive beats. Names and thresholds: `motion_core::checks`.
//!
//! # `repeat_template`
//! WARN. With `ProjectMeta.direction`: consecutive beats with the same
//! template, `params.entrance` and `params.camera`, where the later beat had
//! more than one template candidate. SKIP without a direction record.

use std::collections::BTreeMap;

use motion_core::audio::beat_scenes;
use motion_core::checks::{
    CARRY_CONTINUITY, COUNT_HOLD_S, COUNT_SETTLE_S, COUNT_UNSETTLED, DEAD_AIR,
    DEAD_AIR_FIRST_READABLE_S, DEAD_AIR_MAX_HOLD_S, REPEAT_TEMPLATE,
};
use motion_core::compiler::captions::CAPTION_SCENE_ID;
use motion_core::compiler::speech_plan::anchor_time;
use motion_core::easing::Easing;
use motion_core::layout_qa::is_decorative;
use motion_core::scene::{LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::speech::{SpeechMap, READABLE_OPACITY};
use motion_core::timeline::{
    evaluate_frame, format_count, frame_time, ResolvedFrame, ResolvedLayer,
};
use motion_core::TimelineError;

use crate::reveal_qa::{readable_leaves, scene_prefix, GroupRef, Limits, ReadableLeaf};
use crate::speech_qa::{CheckStatus, SpeechQaCheck};

/// The shortest count the compiler fits (`speech_plan::COUNT_MIN_S`): a
/// number that must still read as a count. A hold shorter than `COUNT_HOLD_S`
/// is only a WARN when the layer's exit leaves under `COUNT_HOLD_S +
/// COUNT_MIN_S` after the count starts. (Mirrored here: story QA does not
/// depend on the compiler's internals.)
pub const COUNT_MIN_S: f64 = 0.4;

/// Comparison slack on the seconds thresholds (one frame is 1/30 s, far above).
const EPS: f64 = 1e-6;

/// A content layer is on screen once at least this share of its clip window
/// shows content (see [`revealed_share`]).
pub const ON_SCREEN_SHARE: f32 = 0.5;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The beat scenes (see the module docs), in order.
pub fn story_beats(project: &MotionProject) -> Vec<&Scene> {
    beat_scenes(project)
        .into_iter()
        .filter(|s| s.id != "backdrop" && s.id != CAPTION_SCENE_ID)
        .collect()
}

/// 1-based beat number of `scene` (`beat_3` is 3; any other id keeps its
/// 1-based position among the beat scenes).
fn beat_number(scene: &Scene, ordinal: usize) -> usize {
    scene
        .id
        .strip_prefix("beat_")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(ordinal + 1)
}

/// The resolved layer at `chain` (ids from a top-level layer down).
pub fn find_layer<'f, 'a>(
    layers: &'f [ResolvedLayer<'a>],
    chain: &[&str],
) -> Option<&'f ResolvedLayer<'a>> {
    let (first, rest) = chain.split_first()?;
    let layer = layers.iter().find(|l| l.id == *first)?;
    if rest.is_empty() {
        Some(layer)
    } else {
        find_layer(&layer.children, rest)
    }
}

/// Share (0..=1) of a layer's clip window that its content fills this frame.
/// A `clip_reveal` slides the content through a fixed window: until it has
/// moved in nothing shows, though [`readable_leaves`] (opacity, blur, box)
/// already counts the layer as readable.
pub fn revealed_share(layer: &ResolvedLayer<'_>) -> f32 {
    let (w, h) = (layer.width, layer.height);
    let c = layer.clip.unwrap_or_default();
    let win = [
        c.left * w,
        c.top * h,
        (1.0 - c.right) * w,
        (1.0 - c.bottom) * h,
    ];
    let win_area = (win[2] - win[0]) * (win[3] - win[1]);
    if win_area <= 0.0 {
        return 0.0;
    }
    let ct = layer.content_transform;
    // The content's offset inside the box (box space).
    let (ox, oy) = if layer.projective.is_some() {
        (ct.e, ct.f)
    } else {
        let t = layer.transform;
        let det = t.a * t.d - t.b * t.c;
        if det.abs() < 1e-9 {
            (0.0, 0.0)
        } else {
            let (dx, dy) = (ct.e - t.e, ct.f - t.f);
            ((t.d * dx - t.c * dy) / det, (-t.b * dx + t.a * dy) / det)
        }
    };
    let content = [ox, oy, ox + w, oy + h];
    let iw = (win[2].min(content[2]) - win[0].max(content[0])).max(0.0);
    let ih = (win[3].min(content[3]) - win[1].max(content[1])).max(0.0);
    iw * ih / win_area
}

/// Whether a readable leaf of a beat scene is content: no id on its chain is
/// decorative (see the module docs).
pub fn is_content_chain(chain: &[&str], kind: &LayerKind) -> bool {
    !chain.iter().any(|id| is_decorative(id, kind))
}

/// The first frame whose time (`frame / fps`, as the timeline computes it)
/// is at least `t`.
fn first_frame_at(t: f64, fps: u32) -> u32 {
    // Far beyond any real project (about 92 hours at 30 fps).
    const LIMIT: u32 = 10_000_000;
    if t.is_nan() || t <= 0.0 {
        return 0;
    }
    let mut k = (t * f64::from(fps)).floor().min(f64::from(LIMIT)) as u32;
    while k < LIMIT && frame_time(fps, k) < t {
        k += 1;
    }
    while k > 0 && frame_time(fps, k - 1) >= t {
        k -= 1;
    }
    k
}

/// Frames `[first, end)` during which `scene` is active.
fn active_frames(scene: &Scene, fps: u32) -> (u32, u32) {
    let first = first_frame_at(scene.start_seconds, fps);
    let end = first_frame_at(scene.end_seconds(), fps);
    (first, end.max(first))
}

fn check(name: &str, status: CheckStatus, detail: impl Into<String>) -> SpeechQaCheck {
    SpeechQaCheck {
        name: name.to_string(),
        status,
        detail: detail.into(),
    }
}

/// `4.0 s` -> `4.00 s`.
fn secs(t: f64) -> String {
    format!("{t:.2} s")
}

// ---------------------------------------------------------------------------
// dead_air
// ---------------------------------------------------------------------------

/// One beat's wait for its first content.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatWait {
    /// 1-based beat number.
    pub beat: usize,
    pub scene: String,
    /// The beat's start (project seconds).
    pub start: f64,
    /// Project time of the first frame showing a content leaf of the beat;
    /// `None` when it never shows one.
    pub first_content: Option<f64>,
    /// `first_content - start`.
    pub wait: Option<f64>,
    /// The layer that ended the wait.
    pub layer: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeadAirReport {
    pub status: CheckStatus,
    /// First content of the whole video (project seconds).
    pub first_content: Option<f64>,
    pub beats: Vec<BeatWait>,
    /// One line per offence: the video opening and every beat over its limit.
    pub offences: Vec<String>,
}

impl DeadAirReport {
    pub fn check(&self) -> SpeechQaCheck {
        let detail = match self.status {
            CheckStatus::Skip => "no beat scenes".to_string(),
            _ if self.offences.is_empty() => {
                let worst = self
                    .beats
                    .iter()
                    .filter_map(|b| b.wait.map(|w| (w, b.beat)))
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                format!(
                    "first content at {} (limit {:.1} s); longest beat wait {} (beat {}, limit {:.1} s)",
                    self.first_content.map_or_else(|| "never".into(), secs),
                    DEAD_AIR_FIRST_READABLE_S,
                    worst.map_or_else(|| "n/a".into(), |w| secs(w.0)),
                    worst.map_or(0, |w| w.1),
                    DEAD_AIR_MAX_HOLD_S,
                )
            }
            _ => self.offences.join("; "),
        };
        check(DEAD_AIR, self.status, detail)
    }
}

/// `dead_air` over every frame the beats need (see the module docs).
pub fn dead_air(project: &MotionProject) -> Result<DeadAirReport, TimelineError> {
    let fps = project.canvas.fps;
    if fps == 0 {
        return Err(TimelineError::ZeroFps);
    }
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let beats = story_beats(project);
    let mut waits = Vec::new();
    for (i, scene) in beats.iter().enumerate() {
        let (first, end) = active_frames(scene, fps);
        let mut found: Option<(u32, String)> = None;
        'frames: for n in first..end {
            let resolved = evaluate_frame(project, n)?;
            for leaf in readable_leaves(&resolved, &limits) {
                if leaf.scene != Some(scene.id.as_str()) {
                    continue;
                }
                let Some(layer) = find_layer(&resolved.layers, &leaf.chain) else {
                    continue;
                };
                if is_content_chain(&leaf.chain, layer.kind)
                    && revealed_share(layer) >= ON_SCREEN_SHARE
                {
                    found = Some((n, layer.id.to_string()));
                    break 'frames;
                }
            }
        }
        let first_content = found.as_ref().map(|(n, _)| frame_time(fps, *n));
        waits.push(BeatWait {
            beat: beat_number(scene, i),
            scene: scene.id.clone(),
            start: scene.start_seconds,
            first_content,
            wait: first_content.map(|t| t - scene.start_seconds),
            layer: found.map(|(_, id)| id),
        });
    }
    let first_content = waits
        .iter()
        .filter_map(|b| b.first_content)
        .min_by(f64::total_cmp);
    let mut offences = Vec::new();
    if !waits.is_empty() && first_content.is_none_or(|t| t > DEAD_AIR_FIRST_READABLE_S + EPS) {
        let at = |b: &BeatWait| b.first_content.unwrap_or(f64::INFINITY);
        let opener = waits
            .iter()
            .filter(|b| b.first_content.is_some())
            .min_by(|a, b| at(a).total_cmp(&at(b)));
        offences.push(format!(
            "first content at {} into the video (limit {:.1} s){}",
            first_content.map_or_else(|| "never".into(), secs),
            DEAD_AIR_FIRST_READABLE_S,
            opener
                .and_then(|b| b.layer.as_deref())
                .map(|l| format!(" [{l}]"))
                .unwrap_or_default(),
        ));
    }
    for b in &waits {
        match b.wait {
            Some(w) if w <= DEAD_AIR_MAX_HOLD_S + EPS => {}
            Some(w) => offences.push(format!(
                "beat {} shows no content for {} after its start (limit {:.1} s) [{}]",
                b.beat,
                secs(w),
                DEAD_AIR_MAX_HOLD_S,
                b.layer.as_deref().unwrap_or("?"),
            )),
            None => offences.push(format!(
                "beat {} never shows content (limit {:.1} s after its start)",
                b.beat, DEAD_AIR_MAX_HOLD_S
            )),
        }
    }
    let status = if waits.is_empty() {
        CheckStatus::Skip
    } else if offences.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Ok(DeadAirReport {
        status,
        first_content,
        beats: waits,
        offences,
    })
}

// ---------------------------------------------------------------------------
// count_unsettled
// ---------------------------------------------------------------------------

/// Where a counter's anchor came from.
#[derive(Debug, Clone, PartialEq)]
pub enum AnchorSource {
    /// The narrator starts saying the final value (the word).
    SpokenValue(String),
    /// The earliest spoken word of the layer's reveal group (group, word).
    RevealGroup(String, String),
    /// The beat's READ (no speech, or the words are not spoken).
    Read,
    /// A running total: the arrival of the last item it counts.
    LastItem,
    /// The word or READ comes before the count can start: its own start.
    CountStart,
}

impl AnchorSource {
    fn label(&self) -> String {
        match self {
            AnchorSource::SpokenValue(w) => format!("\"{w}\""),
            AnchorSource::RevealGroup(g, w) => format!("\"{w}\" ({g})"),
            AnchorSource::Read => "READ".to_string(),
            AnchorSource::LastItem => "the last item".to_string(),
            AnchorSource::CountStart => "the count start".to_string(),
        }
    }
}

/// What the check measured for one counting layer. Times are scene-local
/// seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterReport {
    /// 1-based beat number.
    pub beat: usize,
    pub scene: String,
    pub layer: String,
    pub final_text: String,
    pub anchor: f64,
    pub source: AnchorSource,
    /// Start of the count (its last count for a running total).
    pub count_start: f64,
    /// First frame showing the final text; `None` when it never does.
    pub settle: Option<f64>,
    /// Seconds the final text stays readable, from the first readable frame at
    /// or after the settle to the layer's exit / the beat's ANTICIPATE.
    pub hold: Option<f64>,
    pub status: CheckStatus,
    /// One line when the counter is not clean.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountReport {
    pub status: CheckStatus,
    pub counters: Vec<CounterReport>,
}

impl CountReport {
    pub fn check(&self) -> SpeechQaCheck {
        let notes: Vec<&str> = self
            .counters
            .iter()
            .filter_map(|c| c.note.as_deref())
            .collect();
        let detail = if self.counters.is_empty() {
            "no counting text layers".to_string()
        } else if notes.is_empty() {
            let worst = self
                .counters
                .iter()
                .filter_map(|c| c.settle.map(|s| (s - c.anchor, c)))
                .max_by(|a, b| a.0.total_cmp(&b.0));
            let hold = self
                .counters
                .iter()
                .filter_map(|c| c.hold)
                .min_by(f64::total_cmp);
            format!(
                "{} counter(s) settle within {:.1} s of their anchor (worst {} on beat {} {}) and hold at least {:.1} s (shortest {})",
                self.counters.len(),
                COUNT_SETTLE_S,
                worst.map_or_else(|| "n/a".into(), |w| secs(w.0)),
                worst.map_or(0, |w| w.1.beat),
                worst.map(|w| w.1.layer.as_str()).unwrap_or(""),
                COUNT_HOLD_S,
                hold.map_or_else(|| "n/a".into(), secs),
            )
        } else {
            notes.join("; ")
        };
        check(COUNT_UNSETTLED, self.status, detail)
    }
}

/// The text a counting layer shows at scene-local time `t`, from its Count
/// motions sorted by start: the timeline's own rule (the latest motion that
/// has started, or the first one at progress 0; a spring lands as
/// `OutQuint` so numbers never overshoot).
pub fn count_shown_at(motions: &[&Motion], t: f64) -> Option<String> {
    let mut first: Option<&Motion> = None;
    let mut current: Option<&Motion> = None;
    for m in motions
        .iter()
        .copied()
        .filter(|m| matches!(m.op, MotionOp::Count { .. }))
    {
        if first.is_none() {
            first = Some(m);
        }
        if m.start <= t {
            current = Some(m);
        }
    }
    let (m, elapsed) = match current {
        Some(m) => (m, t - m.start),
        None => (first?, 0.0),
    };
    let MotionOp::Count {
        from,
        to,
        decimals,
        grouping,
        prefix,
        suffix,
    } = &m.op
    else {
        return None;
    };
    let raw = if m.duration <= 0.0 {
        1.0
    } else {
        elapsed / m.duration
    };
    let p = match m.spring {
        None => m.easing.apply(raw),
        Some(_) => Easing::OutQuint.apply(raw),
    };
    Some(format_count(
        from + (to - from) * p,
        *decimals,
        *grouping,
        prefix,
        suffix,
    ))
}

/// The final text of a counting layer (what its last count ends on).
fn final_text(motions: &[&Motion]) -> Option<String> {
    let last = motions
        .iter()
        .rev()
        .find(|m| matches!(m.op, MotionOp::Count { .. }))?;
    match &last.op {
        MotionOp::Count {
            to,
            decimals,
            grouping,
            prefix,
            suffix,
            ..
        } => Some(format_count(*to, *decimals, *grouping, prefix, suffix)),
        _ => None,
    }
}

/// Whether the scene has a text layer with this id.
fn is_text_layer(layers: &[motion_core::scene::Layer], id: &str) -> bool {
    layers.iter().any(|l| {
        (l.id == id && matches!(l.kind, LayerKind::Text(_)))
            || matches!(&l.kind, LayerKind::Group { children } if is_text_layer(children, id))
    })
}

/// The script words of beat `beat` (0-based), scene-local.
fn spoken_words(speech: &SpeechMap, beat: usize, scene: &Scene) -> Vec<(String, f64)> {
    speech
        .sentences
        .iter()
        .find(|s| s.beat == beat)
        .map(|s| {
            speech
                .words_in(s)
                .iter()
                .map(|w| (w.text.clone(), w.start - scene.start_seconds))
                .collect()
        })
        .unwrap_or_default()
}

/// The word at scene-local `at` among `spoken` (for the finding text).
fn word_at(spoken: &[(String, f64)], at: f64) -> String {
    spoken
        .iter()
        .find(|(_, t)| (t - at).abs() < 1e-6)
        .map(|(w, _)| w.clone())
        .unwrap_or_default()
}

/// The anchor of a counting layer (before the "never earlier than the count's
/// own start" clamp).
fn anchor_of(
    project: &MotionProject,
    scene: &Scene,
    layer: &str,
    final_text: &str,
    spoken: &[(String, f64)],
    read: f64,
) -> (f64, AnchorSource) {
    if !spoken.is_empty() {
        // The spoken number for that value: the text as displayed, then the
        // digits alone (grouping commas dropped).
        let bare: String = final_text.chars().filter(|c| *c != ',').collect();
        let mut tries = vec![final_text.to_string()];
        if bare != final_text {
            tries.push(bare);
        }
        for t in &tries {
            if let Some(at) = anchor_time(spoken, &[t.as_str()]) {
                return (at, AnchorSource::SpokenValue(word_at(spoken, at)));
            }
        }
        // The earliest word of the layer's reveal group.
        let reveals = project
            .project
            .art
            .as_ref()
            .and_then(|a| a.reveals.get(&scene.id));
        let prefix = scene_prefix(scene);
        if let (Some(reveals), Some(prefix)) = (reveals, prefix) {
            let mut best: Option<(f64, String)> = None;
            for a in reveals {
                let g = GroupRef {
                    scene: scene.id.clone(),
                    prefix: prefix.clone(),
                    group: a.group.clone(),
                };
                if !g.matches_id(layer) {
                    continue;
                }
                let entries: Vec<&str> = a.words.iter().map(String::as_str).collect();
                if let Some(at) = anchor_time(spoken, &entries) {
                    if best.as_ref().is_none_or(|(b, _)| at < *b) {
                        best = Some((at, a.group.clone()));
                    }
                }
            }
            if let Some((at, group)) = best {
                return (at, AnchorSource::RevealGroup(group, word_at(spoken, at)));
            }
        }
    }
    (read, AnchorSource::Read)
}

/// `count_unsettled` over every counting text layer of every beat. `speech`
/// anchors the counts on the narrator's words; without it every anchor is the
/// beat's READ (or the count's own start when later).
pub fn count_unsettled(
    project: &MotionProject,
    speech: Option<&SpeechMap>,
) -> Result<CountReport, TimelineError> {
    let fps = project.canvas.fps;
    if fps == 0 {
        return Err(TimelineError::ZeroFps);
    }
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let mut counters = Vec::new();
    for (i, scene) in story_beats(project).into_iter().enumerate() {
        let beat = beat_number(scene, i);
        // Count motions per layer, sorted by start (the timeline's order).
        let mut by_layer: BTreeMap<&str, Vec<&Motion>> = BTreeMap::new();
        for m in &scene.motions {
            if matches!(m.op, MotionOp::Count { .. }) && is_text_layer(&scene.layers, &m.target) {
                by_layer.entry(m.target.as_str()).or_default().push(m);
            }
        }
        let read = scene.lifecycle.map(|l| l.read);
        let anticipate = scene
            .lifecycle
            .map_or(scene.duration_seconds, |l| l.anticipate);
        let spoken = speech
            .map(|s| spoken_words(s, beat - 1, scene))
            .unwrap_or_default();
        for (layer, mut motions) in by_layer {
            motions.sort_by(|a, b| a.start.total_cmp(&b.start));
            let Some(final_text) = final_text(&motions) else {
                continue;
            };
            let chain = motions.len() >= 2;
            let count_start = motions.last().map_or(0.0, |m| m.start);
            let (raw_anchor, mut source) = if chain {
                (count_start, AnchorSource::LastItem)
            } else {
                anchor_of(
                    project,
                    scene,
                    layer,
                    &final_text,
                    &spoken,
                    read.unwrap_or(count_start),
                )
            };
            // Never earlier than the count's own start.
            let anchor = if raw_anchor < count_start {
                source = AnchorSource::CountStart;
                count_start
            } else {
                raw_anchor
            };

            let (first, end) = active_frames(scene, fps);
            let local = |n: u32| frame_time(fps, n) - scene.start_seconds;
            // Settle: the first frame whose displayed text is the final text
            // and stays so (a landing count never leaves it; a curve that
            // overshoots and returns settles when it is back).
            let last_other = (first..end)
                .rev()
                .find(|&n| count_shown_at(&motions, local(n)).as_deref() != Some(&final_text));
            let settle_frame = match last_other {
                None => (first < end).then_some(first),
                Some(n) => (n + 1 < end).then_some(n + 1),
            };
            let mut report = CounterReport {
                beat,
                scene: scene.id.clone(),
                layer: layer.to_string(),
                final_text: final_text.clone(),
                anchor,
                source: source.clone(),
                count_start,
                settle: settle_frame.map(local),
                hold: None,
                status: CheckStatus::Pass,
                note: None,
            };
            let mut problems: Vec<(CheckStatus, String)> = Vec::new();
            match settle_frame {
                None => problems.push((CheckStatus::Fail, "never shows its final value".into())),
                Some(sf) => {
                    let settle = local(sf);
                    if settle - (anchor + COUNT_SETTLE_S) > EPS {
                        problems.push((
                            CheckStatus::Fail,
                            format!(
                                "settles {} after {} at {} (limit {:.1} s; reads {} at READ)",
                                secs(settle - anchor),
                                source.label(),
                                secs(anchor),
                                COUNT_SETTLE_S,
                                read.map_or_else(
                                    || "?".to_string(),
                                    |r| count_shown_at(&motions, r).unwrap_or_default()
                                ),
                            ),
                        ));
                    }
                    // Hold: how long the final text stays readable.
                    let (hold, exit) =
                        hold_after(project, scene, layer, (sf, end), anticipate, &limits)?;
                    report.hold = Some(hold);
                    if hold + EPS < COUNT_HOLD_S {
                        // The beat limits the hold when the exit leaves no room
                        // for a real count plus the hold after the count starts.
                        let beat_limits = exit - count_start < COUNT_HOLD_S + COUNT_MIN_S - EPS;
                        problems.push((
                            if beat_limits {
                                CheckStatus::Warn
                            } else {
                                CheckStatus::Fail
                            },
                            format!(
                                "holds {} before the exit (needs {:.1} s{})",
                                secs(hold),
                                COUNT_HOLD_S,
                                if beat_limits {
                                    format!(
                                        "; the exit comes {} after the count starts, under {:.1} s",
                                        secs(exit - count_start),
                                        COUNT_HOLD_S + COUNT_MIN_S
                                    )
                                } else {
                                    String::new()
                                }
                            ),
                        ));
                    }
                }
            }
            report.status = problems.iter().map(|p| p.0).fold(CheckStatus::Pass, worse);
            if !problems.is_empty() {
                let lines: Vec<&str> = problems.iter().map(|p| p.1.as_str()).collect();
                report.note = Some(format!(
                    "beat {beat} {layer} \"{final_text}\": {}",
                    lines.join(" and ")
                ));
            }
            counters.push(report);
        }
    }
    let status = counters
        .iter()
        .map(|c| c.status)
        .fold(CheckStatus::Pass, worse);
    Ok(CountReport { status, counters })
}

/// The worse of two statuses (FAIL > WARN > PASS / SKIP).
fn worse(a: CheckStatus, b: CheckStatus) -> CheckStatus {
    let rank = |s: CheckStatus| match s {
        CheckStatus::Skip | CheckStatus::Pass => 0,
        CheckStatus::Warn => 1,
        CheckStatus::Fail => 2,
    };
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

/// Seconds the final text holds, and the layer's exit (scene-local seconds).
/// The hold runs from the first readable frame at or after the settle frame
/// `range.0` (a layer that is still hidden when the text settles is waited for:
/// a number that finishes unseen is not held) to the layer's exit or
/// scene-local `anticipate`, whichever comes first. The layer's exit is the
/// first frame at which it stops being readable after it was. Stops scanning
/// once the hold reaches `COUNT_HOLD_S` (more is not needed; the exit is then
/// the end of the hold). 0 when the layer is never readable before ANTICIPATE
/// (the exit is ANTICIPATE).
fn hold_after(
    project: &MotionProject,
    scene: &Scene,
    layer: &str,
    range: (u32, u32),
    anticipate: f64,
    limits: &Limits,
) -> Result<(f64, f64), TimelineError> {
    let fps = project.canvas.fps;
    let local = |n: u32| frame_time(fps, n) - scene.start_seconds;
    // Where the hold starts: the first readable frame at or after the settle.
    let mut from = local(range.0);
    let mut seen = false;
    let mut exit: Option<f64> = None;
    for n in range.0..range.1 {
        if local(n) >= anticipate - EPS {
            break;
        }
        let resolved: ResolvedFrame<'_> = evaluate_frame(project, n)?;
        let readable = readable_leaves(&resolved, limits).iter().any(|l| {
            l.scene == Some(scene.id.as_str()) && l.chain.last().is_some_and(|id| *id == layer)
        });
        if readable {
            if !seen {
                seen = true;
                from = local(n);
            }
            if local(n) - from >= COUNT_HOLD_S {
                // Long enough: no need to look further.
                return Ok((local(n) - from, local(n)));
            }
        } else if seen {
            exit = Some(local(n));
            break;
        }
    }
    let exit = exit.unwrap_or(anticipate);
    Ok(if seen {
        ((exit - from).max(0.0), exit)
    } else {
        (0.0, exit)
    })
}

// ---------------------------------------------------------------------------
// repeat_template
// ---------------------------------------------------------------------------

/// Two consecutive beats built alike.
#[derive(Debug, Clone, PartialEq)]
pub struct Repeat {
    /// 1-based numbers of the two beats.
    pub beats: (usize, usize),
    pub template: String,
    pub entrance: String,
    pub camera: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepeatReport {
    pub status: CheckStatus,
    pub repeats: Vec<Repeat>,
    /// The number of consecutive pairs judged.
    pub pairs: usize,
}

impl RepeatReport {
    pub fn check(&self) -> SpeechQaCheck {
        let detail = match self.status {
            CheckStatus::Skip => "no direction record".to_string(),
            _ if self.repeats.is_empty() => format!(
                "no consecutive beats share template, entrance and camera ({} pair(s))",
                self.pairs
            ),
            _ => self
                .repeats
                .iter()
                .map(|r| {
                    format!(
                        "beats {}-{}: template {}, entrance {}, camera {}",
                        r.beats.0, r.beats.1, r.template, r.entrance, r.camera
                    )
                })
                .collect::<Vec<_>>()
                .join("; "),
        };
        check(REPEAT_TEMPLATE, self.status, detail)
    }
}

/// The snake_case name an enum serialises to (`SlideLeft` -> `slide_left`).
fn serde_name<T: serde::Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// `repeat_template` (WARN) from `ProjectMeta.direction`.
pub fn repeat_template(project: &MotionProject) -> RepeatReport {
    let Some(direction) = project.project.direction.as_ref() else {
        return RepeatReport {
            status: CheckStatus::Skip,
            repeats: Vec::new(),
            pairs: 0,
        };
    };
    let mut beats: Vec<&motion_core::compiler::direction::BeatDirection> =
        direction.beats.iter().collect();
    beats.sort_by_key(|b| b.beat);
    let mut repeats = Vec::new();
    let mut pairs = 0;
    for w in beats.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b.beat != a.beat + 1 {
            continue;
        }
        pairs += 1;
        let same = a.template == b.template
            && a.params.entrance == b.params.entrance
            && a.params.camera == b.params.camera;
        if same && b.alternates > 1 {
            repeats.push(Repeat {
                beats: (a.beat + 1, b.beat + 1),
                template: b.template.clone(),
                entrance: serde_name(&b.params.entrance),
                camera: serde_name(&b.params.camera),
            });
        }
    }
    RepeatReport {
        status: if repeats.is_empty() {
            CheckStatus::Pass
        } else {
            CheckStatus::Warn
        },
        repeats,
        pairs,
    }
}

// ---------------------------------------------------------------------------
// carry_continuity
// ---------------------------------------------------------------------------

/// One run of frames in which a carried element is not readable.
#[derive(Debug, Clone, PartialEq)]
pub struct CarryGap {
    /// The shared element's layer id.
    pub element: String,
    /// 1-based numbers of the two beats the element is carried between.
    pub beats: (usize, usize),
    /// Project time of the first and of the last frame of the run.
    pub from: f64,
    pub to: f64,
    /// Why the first frame of the run is not readable.
    pub reason: String,
}

/// A carried element and the span that was judged.
#[derive(Debug, Clone, PartialEq)]
pub struct CarriedSpan {
    pub element: String,
    pub beats: (usize, usize),
    /// Beat N's READ and beat N+1's READ (project seconds).
    pub from: f64,
    pub to: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CarryReport {
    pub status: CheckStatus,
    pub spans: Vec<CarriedSpan>,
    pub gaps: Vec<CarryGap>,
}

impl CarryReport {
    pub fn check(&self) -> SpeechQaCheck {
        let detail = match self.status {
            CheckStatus::Skip => "no element is carried between consecutive beats".to_string(),
            _ if self.gaps.is_empty() => format!(
                "{} carried element(s) readable from READ of beat N to READ of beat N+1 ({})",
                self.spans.len(),
                self.spans
                    .iter()
                    .map(|s| format!("{} beats {}-{}", s.element, s.beats.0, s.beats.1))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            _ => self
                .gaps
                .iter()
                .map(|g| {
                    format!(
                        "{} (beats {}-{}) not readable {}-{} ({})",
                        g.element,
                        g.beats.0,
                        g.beats.1,
                        secs(g.from),
                        secs(g.to),
                        g.reason
                    )
                })
                .collect::<Vec<_>>()
                .join("; "),
        };
        check(CARRY_CONTINUITY, self.status, detail)
    }
}

/// Why the shared element `id` is not readable in `frame` (`None` = it is).
fn carried_unreadable(
    frame: &ResolvedFrame<'_>,
    readable: &[ReadableLeaf<'_>],
    id: &str,
) -> Option<String> {
    let Some(layer) = frame
        .layers
        .iter()
        .find(|l| l.id == id && l.scene.is_none())
    else {
        return Some("not drawn".to_string());
    };
    if readable
        .iter()
        .any(|l| l.scene.is_none() && l.chain.first() == Some(&id))
    {
        return None;
    }
    let opacity = layer.opacity.clamp(0.0, 1.0);
    Some(if opacity < READABLE_OPACITY {
        format!("opacity {opacity:.2}")
    } else {
        "off canvas or blurred".to_string()
    })
}

/// `carry_continuity` over every frame between the READs (see the module docs).
pub fn carry_continuity(project: &MotionProject) -> Result<CarryReport, TimelineError> {
    let fps = project.canvas.fps;
    if fps == 0 {
        return Err(TimelineError::ZeroFps);
    }
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let beats = story_beats(project);
    let read_at = |scene: &Scene| {
        scene.start_seconds
            + scene
                .lifecycle
                .map_or(0.0, |l| l.read.min(scene.duration_seconds))
    };
    let mut spans = Vec::new();
    let mut gaps = Vec::new();
    for element in &project.shared {
        let keyed = |scene: &Scene| element.track.iter().any(|k| k.scene == scene.id);
        for (i, pair) in beats.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            if !(keyed(a) && keyed(b)) {
                continue;
            }
            let (from, to) = (read_at(a), read_at(b));
            let numbers = (beat_number(a, i), beat_number(b, i + 1));
            spans.push(CarriedSpan {
                element: element.id.clone(),
                beats: numbers,
                from,
                to,
            });
            let mut open: Option<CarryGap> = None;
            // Frames whose time lies in [from, to].
            let mut n = first_frame_at(from, fps);
            loop {
                let t = frame_time(fps, n);
                if t > to + EPS {
                    break;
                }
                let resolved = evaluate_frame(project, n)?;
                let readable = readable_leaves(&resolved, &limits);
                match carried_unreadable(&resolved, &readable, &element.layer.id) {
                    Some(reason) => match open.as_mut() {
                        Some(gap) => gap.to = t,
                        None => {
                            open = Some(CarryGap {
                                element: element.id.clone(),
                                beats: numbers,
                                from: t,
                                to: t,
                                reason,
                            });
                        }
                    },
                    None => gaps.extend(open.take()),
                }
                n += 1;
            }
            gaps.extend(open.take());
        }
    }
    let status = if spans.is_empty() {
        CheckStatus::Skip
    } else if gaps.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Ok(CarryReport {
        status,
        spans,
        gaps,
    })
}

// ---------------------------------------------------------------------------
// All three
// ---------------------------------------------------------------------------

/// The three structural checks as report entries, in the order `dead_air`,
/// `count_unsettled`, `repeat_template`.
pub fn story_checks(
    project: &MotionProject,
    speech: Option<&SpeechMap>,
) -> Result<Vec<SpeechQaCheck>, TimelineError> {
    Ok(vec![
        dead_air(project)?.check(),
        count_unsettled(project, speech)?.check(),
        repeat_template(project).check(),
    ])
}

/// (0.23 W8) [`story_checks`] plus `carry_continuity`, in the order
/// `dead_air`, `count_unsettled`, `repeat_template`, `carry_continuity`: what
/// `qa` and `qa --speech` report. (`story_checks` keeps its three entries.)
pub fn motion_checks(
    project: &MotionProject,
    speech: Option<&SpeechMap>,
) -> Result<Vec<SpeechQaCheck>, TimelineError> {
    let mut checks = story_checks(project, speech)?;
    checks.push(carry_continuity(project)?.check());
    Ok(checks)
}
