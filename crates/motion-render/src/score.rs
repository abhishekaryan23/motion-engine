//! (0.23 B3) Generate-and-rank: score best-of-N candidate directions, drop the
//! ones with a hard failure, ship the best of the rest.
//!
//! One compile is one roll of the dice: a seeded alternate can land a worse
//! layout. `compile_candidate` (motion-core) compiles candidate direction `k`
//! of a take; this module judges the candidates and picks one
//! ([`best_of`]). Selection is the repair loop; `qa` after the render stays the
//! final report. Everything here is a pure function of the candidates: no
//! clocks, no RNG, and the choice never depends on which thread finished first
//! (results are collected in candidate order).
//!
//! # Hard checks
//! A candidate that fails any of these is dropped while a clean one exists. The
//! failed check names go to `CandidateScore.hard`.
//!
//! | stage | name | rule |
//! |---|---|---|
//! | structural, every candidate | `compile` | the candidate compiled |
//! | | `validation` | the project validates (assets exist) |
//! | | `value_dropped` | no `value_dropped` compile warning |
//! | | the layout check names | `layout_report` has no finding (one name per kind: `text_outside_safe` ...) |
//! | | `dead_air` | `story_qa::dead_air` is not FAIL |
//! | | `count_unsettled` | `story_qa::count_unsettled` is not FAIL |
//! | | `timeline` | the frames the checks need evaluate |
//! | pixel, the top 2 by soft score | `text_local_contrast` | `contrast_qa::text_local_contrast` is not FAIL |
//! | | `render` | the READ frames render |
//!
//! The pixel stage renders each beat's READ frame once, in memory, with one
//! renderer prepared per candidate (`text_local_contrast` prepares it once and
//! never writes a frame). It runs on the two best structurally clean
//! candidates by soft score; when neither clears it, the next two run, and so
//! on, so a clean candidate that exists is never passed over for a
//! pixel-failing one. The frame is rendered at the project's size and
//! box-filtered by `text_local_contrast` itself to at most
//! [`crate::contrast_qa::MAX_WIDTH_PX`] (540 px) wide before the ring is
//! measured: the check is the QA's own, so what the scorer drops is exactly
//! what `qa --layout` would fail.
//!
//! # Soft score
//! Higher is better. Every part is in `CandidateScore.parts`; the total is
//! their sum. The weights below are the design, calibrated on the bench matrix
//! (8 stories x 9 tones x candidates 0..=3): the parts live on comparable
//! scales (a defect is worth about 0.25 to 1), and the total of a clean
//! candidate sits between about -3 and +2. Only differences between candidates
//! of one take matter.
//!
//! | part | range | rule |
//! |---|---|---|
//! | `variety` | `<= 0` | `-W_REPEAT` (1.0) per pair of consecutive beats with the same template, entrance and camera where the later beat had a choice (`alternates > 1`) |
//! | `focal` | `0..=W_FOCAL` (2.0) | mean over the beats with a focal record of the focal element's share of the on-screen content at READ, scaled linearly from [`FOCAL_SHARE_LOW`] (0.20, score 0) to [`FOCAL_SHARE_HIGH`] (0.75, full score) |
//! | `density` | `<= 0` | per beat, in each of ENTER `[0, read)`, EVOLVE and ANTICIPATE (not the last beat's): `-W_DENSITY_EMPTY` (0.25) when no content layer starts moving in a phase longer than [`MIN_PHASE_S`] (0.3 s); in every phase (READ too) `-W_DENSITY_OVER` (0.5) x `min(1, excess / max)` for the distinct content layers starting to move beyond the look's usual maximum ([`look_max`]). READ may be still: that is the reading hold |
//! | `balance` | `<= 0` | per beat at READ: `-W_BALANCE` (1.0) x the relative amount by which the text area leaves `[TEXT_MIN, TEXT_MAX]` of the canvas ([`TEXT_MIN`] 3 %, [`TEXT_MAX`] 45 %), capped at 1 per beat |
//! | `dead_air` | `<= 0` | `-W_DEAD_AIR` (1.0) per second the first content passes `DEAD_AIR_FIRST_READABLE_S` (0.5 s) plus per second each beat's first content passes `DEAD_AIR_MAX_HOLD_S` (1.2 s); zero for a candidate that passes the hard check, so it only orders failing candidates |
//!
//! # Choice
//! Drop hard failures; the highest soft score wins, ties go to the lowest `k`.
//! When every candidate fails a hard check the one with the fewest failures
//! ships (then the higher soft score, then the lower `k`) and
//! `shipped_with_failure` says so ([`choose`]). A candidate that did not
//! compile never ships while another one compiled.

use std::collections::BTreeMap;
use std::path::Path;

use motion_core::checks::{
    COUNT_UNSETTLED, DEAD_AIR, DEAD_AIR_FIRST_READABLE_S, DEAD_AIR_MAX_HOLD_S, TEXT_LOCAL_CONTRAST,
};
use motion_core::compiler::direction::{candidate_seed, BeatDirection, CandidateScore};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{CompileError, CompileWarning, WARN_VALUE_DROPPED};
use motion_core::layout_qa::is_decorative;
use motion_core::scene::{Layer, LayerKind, MotionProject, Scene};
use motion_core::speech::SpeechMap;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use motion_core::{layout_report, validate};
use rayon::prelude::*;

use crate::contrast_qa::text_local_contrast;
use crate::reveal_qa::{readable_leaves, Limits};
use crate::speech_qa::CheckStatus;
use crate::story_qa::{
    count_unsettled, dead_air, find_layer, is_content_chain, revealed_share, story_beats,
    ON_SCREEN_SHARE,
};

// ---------------------------------------------------------------------------
// Names and weights
// ---------------------------------------------------------------------------

/// The candidate did not compile.
pub const HARD_COMPILE: &str = "compile";
/// The compiled project does not validate.
pub const HARD_VALIDATION: &str = "validation";
/// A frame a check needs could not be evaluated.
pub const HARD_TIMELINE: &str = "timeline";
/// The READ frames could not be rendered for the pixel check.
pub const HARD_RENDER: &str = "render";

/// `variety` part: cost of one consecutive repeat.
pub const W_REPEAT: f64 = 1.0;
/// `focal` part: the score of a beat whose focal element dominates.
pub const W_FOCAL: f64 = 2.0;
/// Focal share of the on-screen content at which the dominance score starts.
pub const FOCAL_SHARE_LOW: f64 = 0.20;
/// Focal share at which the dominance score is full.
pub const FOCAL_SHARE_HIGH: f64 = 0.75;
/// `density` part: cost of a phase with no motion.
pub const W_DENSITY_EMPTY: f64 = 0.25;
/// `density` part: cost of a phase far over the look's usual maximum.
pub const W_DENSITY_OVER: f64 = 0.5;
/// Phases shorter than this are not judged for "no motion" (seconds).
pub const MIN_PHASE_S: f64 = 0.3;
/// `balance` part: cost of a READ frame whose text area is out of range.
pub const W_BALANCE: f64 = 1.0;
/// Text area above this share of the canvas at READ is too much.
pub const TEXT_MAX: f64 = 0.45;
/// Text area below this share of the canvas at READ is too little.
pub const TEXT_MIN: f64 = 0.03;
/// `dead_air` part: cost per second over the budgets.
pub const W_DEAD_AIR: f64 = 1.0;

/// Pixel checks run on this many candidates at a time.
pub const PIXEL_WAVE: usize = 2;

/// Occupancy grid columns for the area measures.
const GRID_W: usize = 96;

/// Six decimals, and never a negative zero (it would print as `-0.0`).
fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6 + 0.0
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Canvas coverage of a set of rectangles (a coarse grid; overlaps count once).
struct Occupancy {
    w: usize,
    h: usize,
    canvas_w: f32,
    canvas_h: f32,
    cells: Vec<bool>,
}

impl Occupancy {
    fn new(canvas_w: f32, canvas_h: f32) -> Self {
        let w = GRID_W;
        let h = ((GRID_W as f32 * canvas_h / canvas_w.max(1.0)).round() as usize).max(1);
        Occupancy {
            w,
            h,
            canvas_w,
            canvas_h,
            cells: vec![false; w * h],
        }
    }

    /// Mark the cells whose centre lies inside `r` (`[x0, y0, x1, y1]`, canvas px).
    fn fill(&mut self, r: [f32; 4]) {
        let (sx, sy) = (
            self.w as f32 / self.canvas_w.max(1.0),
            self.h as f32 / self.canvas_h.max(1.0),
        );
        for gy in 0..self.h {
            let cy = (gy as f32 + 0.5) / sy;
            if cy < r[1] || cy >= r[3] {
                continue;
            }
            for gx in 0..self.w {
                let cx = (gx as f32 + 0.5) / sx;
                if cx >= r[0] && cx < r[2] {
                    self.cells[gy * self.w + gx] = true;
                }
            }
        }
    }

    fn covered(&self) -> usize {
        self.cells.iter().filter(|c| **c).count()
    }

    fn share(&self) -> f64 {
        self.covered() as f64 / self.cells.len().max(1) as f64
    }
}

/// The canvas-space bounding box of a resolved layer's box.
fn canvas_box(layer: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    let t = layer.transform;
    let (w, h) = (layer.width, layer.height);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| t.apply(x, y));
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

// ---------------------------------------------------------------------------
// READ-frame measures (focal dominance, text occupancy)
// ---------------------------------------------------------------------------

/// What one beat looks like at its READ moment.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadMeasure {
    pub scene: String,
    /// Share of the canvas the readable text layers cover.
    pub text_share: f64,
    /// The focal element's share of the on-screen content (`None` when the
    /// beat has no focal record or it is not drawn).
    pub focal_share: Option<f64>,
}

/// Measure every story beat at its READ frame (the frame layout QA samples).
pub fn read_measures(
    project: &MotionProject,
) -> Result<Vec<ReadMeasure>, motion_core::TimelineError> {
    let (cw, ch) = (project.canvas.width as f32, project.canvas.height as f32);
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let last = project.frame_count().saturating_sub(1);
    let fps = f64::from(project.canvas.fps);
    let mut out = Vec::new();
    for scene in story_beats(project) {
        let read = scene
            .lifecycle
            .map_or(scene.duration_seconds * 0.5, |l| l.read);
        let n = (((scene.start_seconds + read) * fps).round() as u32).min(last);
        let resolved = evaluate_frame(project, n)?;
        let focal = project
            .project
            .art
            .as_ref()
            .and_then(|a| a.focal.get(&scene.id))
            .map(String::as_str);
        let (mut text, mut content, mut focal_cells) = (
            Occupancy::new(cw, ch),
            Occupancy::new(cw, ch),
            Occupancy::new(cw, ch),
        );
        for leaf in readable_leaves(&resolved, &limits) {
            if leaf.scene != Some(scene.id.as_str()) {
                continue;
            }
            let Some(layer) = find_layer(&resolved.layers, &leaf.chain) else {
                continue;
            };
            if !is_content_chain(&leaf.chain, layer.kind) || revealed_share(layer) < ON_SCREEN_SHARE
            {
                continue;
            }
            let Some(b) = canvas_box(layer) else {
                continue;
            };
            let b = [b[0].max(0.0), b[1].max(0.0), b[2].min(cw), b[3].min(ch)];
            content.fill(b);
            if matches!(layer.kind, LayerKind::Text(_)) {
                text.fill(b);
            }
            if focal.is_some_and(|f| leaf.chain.contains(&f)) {
                focal_cells.fill(b);
            }
        }
        let focal_share = focal.and_then(|_| {
            let all = content.covered();
            (focal_cells.covered() > 0 && all > 0)
                .then(|| focal_cells.covered() as f64 / all as f64)
        });
        out.push(ReadMeasure {
            scene: scene.id.clone(),
            text_share: text.share(),
            focal_share,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Motion density per lifecycle phase
// ---------------------------------------------------------------------------

/// Distinct content layers that start to move in each judged phase of a beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseCounts {
    /// ENTER: `[0, read)` (the arrival, including the settle).
    pub enter: usize,
    /// READ: `[read, evolve)`.
    pub read: usize,
    /// EVOLVE: `[evolve, anticipate)`.
    pub evolve: usize,
    /// ANTICIPATE: `[anticipate, duration)`.
    pub anticipate: usize,
}

/// A beat's phase windows (scene-local seconds): ENTER, READ, EVOLVE, ANTICIPATE.
fn phase_windows(scene: &Scene) -> Option<[(f64, f64); 4]> {
    let l = scene.lifecycle?;
    Some([
        (0.0, l.read),
        (l.read, l.evolve),
        (l.evolve, l.anticipate),
        (l.anticipate, scene.duration_seconds),
    ])
}

fn collect_kinds<'a>(layers: &'a [Layer], out: &mut BTreeMap<&'a str, &'a LayerKind>) {
    for l in layers {
        out.insert(l.id.as_str(), &l.kind);
        if let LayerKind::Group { children } = &l.kind {
            collect_kinds(children, out);
        }
    }
}

/// Distinct content layers (not furniture, by `is_decorative`) with a motion
/// starting inside each phase of `scene`. `None` for a scene without a
/// lifecycle.
pub fn phase_counts(scene: &Scene) -> Option<PhaseCounts> {
    let windows = phase_windows(scene)?;
    let mut kinds = BTreeMap::new();
    collect_kinds(&scene.layers, &mut kinds);
    let mut counts = [0usize; 4];
    for (i, (lo, hi)) in windows.iter().enumerate() {
        let mut targets: Vec<&str> = scene
            .motions
            .iter()
            .filter(|m| m.start >= *lo - 1e-9 && m.start < *hi - 1e-9)
            .filter(|m| {
                kinds
                    .get(m.target.as_str())
                    .is_none_or(|k| !is_decorative(&m.target, k))
            })
            .map(|m| m.target.as_str())
            .collect();
        targets.sort_unstable();
        targets.dedup();
        counts[i] = targets.len();
    }
    Some(PhaseCounts {
        enter: counts[0],
        read: counts[1],
        evolve: counts[2],
        anticipate: counts[3],
    })
}

/// The look's usual maximum of distinct layers starting to move per phase
/// (ENTER, READ, EVOLVE, ANTICIPATE). Calibrated on the bench matrix (8
/// stories x 9 tones x candidates 0..=3, 936 beats): the 90th percentile of the
/// beats of the look, rounded up and given a little room. Looks the matrix does
/// not reach (`clay_pop`, `ornament_editorial`) and unknown looks use
/// [`DEFAULT_LOOK_MAX`].
pub fn look_max(look: Option<&str>) -> [usize; 4] {
    match look {
        Some("cinematic_3d") => [14, 5, 8, 6],
        Some("classical_neon") => [14, 2, 8, 2],
        Some("dossier") => [13, 5, 8, 5],
        Some("halftone_cutout") => [16, 2, 8, 2],
        Some("hype_slam") => [15, 4, 8, 2],
        Some("journey") => [14, 2, 8, 2],
        Some("street_collage") => [16, 4, 8, 2],
        Some("studio_pop") => [20, 6, 8, 5],
        _ => DEFAULT_LOOK_MAX,
    }
}

/// [`look_max`] of a look without its own row.
pub const DEFAULT_LOOK_MAX: [usize; 4] = [16, 4, 9, 4];

// ---------------------------------------------------------------------------
// The structural score
// ---------------------------------------------------------------------------

/// What the structural stage found.
#[derive(Debug, Clone, PartialEq)]
pub struct Structural {
    /// Failed hard checks (names, in a fixed order).
    pub hard: Vec<String>,
    /// Soft score parts by name.
    pub parts: BTreeMap<String, f64>,
    /// Sum of the parts.
    pub soft: f64,
}

/// The consecutive repeats of a direction record: pairs of beats with the same
/// template, entrance and camera where the later beat had more than one
/// template candidate. The entrance and camera are `params.*` and, where the
/// seeded rotations stand for them, the arrival and camera rotations.
fn repeats(beats: &[BeatDirection]) -> usize {
    let mut sorted: Vec<&BeatDirection> = beats.iter().collect();
    sorted.sort_by_key(|b| b.beat);
    let sig = |b: &BeatDirection| {
        (
            b.template.clone(),
            b.params.entrance,
            b.rotations.get("arrival").cloned(),
            b.params.camera,
            b.rotations.get("camera").cloned(),
        )
    };
    sorted
        .windows(2)
        .filter(|w| w[1].beat == w[0].beat + 1 && w[1].alternates > 1 && sig(w[0]) == sig(w[1]))
        .count()
}

/// Share by which `text_share` leaves `[TEXT_MIN, TEXT_MAX]`, capped at 1.
fn balance_penalty(text_share: f64) -> f64 {
    if text_share > TEXT_MAX {
        ((text_share - TEXT_MAX) / TEXT_MAX).min(1.0)
    } else if text_share < TEXT_MIN {
        ((TEXT_MIN - text_share) / TEXT_MIN).min(1.0)
    } else {
        0.0
    }
}

/// Dominance of a focal share, 0..=1.
fn focal_score(share: f64) -> f64 {
    ((share - FOCAL_SHARE_LOW) / (FOCAL_SHARE_HIGH - FOCAL_SHARE_LOW)).clamp(0.0, 1.0)
}

/// The `density` penalty (a positive number): see the module docs.
fn density_penalty(project: &MotionProject) -> f64 {
    let look = project.project.art.as_ref().map(|a| a.look.as_str());
    let max = look_max(look);
    let beats = story_beats(project);
    let mut penalty = 0.0;
    for (index, scene) in beats.iter().enumerate() {
        let (Some(counts), Some(windows)) = (phase_counts(scene), phase_windows(scene)) else {
            continue;
        };
        let last_beat = index + 1 == beats.len();
        let counts = [counts.enter, counts.read, counts.evolve, counts.anticipate];
        for (i, ((lo, hi), n)) in windows.iter().zip(counts).enumerate() {
            // READ is the reading hold (stillness there is by design); the
            // last beat has no exit to anticipate.
            let judged = i != 1 && !(i == 3 && last_beat);
            if judged && n == 0 && hi - lo >= MIN_PHASE_S {
                penalty += W_DENSITY_EMPTY;
            }
            if n > max[i] {
                penalty += W_DENSITY_OVER * ((n - max[i]) as f64 / max[i].max(1) as f64).min(1.0);
            }
        }
    }
    penalty
}

/// The structural stage for one compiled candidate: the hard checks that need
/// no pixels and the soft score. `warnings` are the compile's own;
/// `speech` anchors the counters (`count_unsettled`).
pub fn structural(
    project: &MotionProject,
    warnings: &[CompileWarning],
    speech: Option<&SpeechMap>,
) -> Structural {
    let mut hard: Vec<String> = Vec::new();
    if warnings.iter().any(|w| w.code == WARN_VALUE_DROPPED) {
        hard.push(WARN_VALUE_DROPPED.to_string());
    }
    match LayoutFrame::new(project.canvas.width, project.canvas.height) {
        Ok(frame) => {
            let mut names: Vec<&'static str> = layout_report(project, &frame)
                .findings
                .iter()
                .map(|f| f.check.name())
                .collect();
            names.sort_unstable();
            names.dedup();
            hard.extend(names.into_iter().map(str::to_string));
        }
        Err(_) => hard.push("layout".to_string()),
    }
    let mut timeline_failed = false;
    let mut dead_air_part = 0.0;
    match dead_air(project) {
        Ok(r) => {
            if r.status == CheckStatus::Fail {
                hard.push(DEAD_AIR.to_string());
            }
            let first = r
                .first_content
                .map_or(0.0, |t| (t - DEAD_AIR_FIRST_READABLE_S).max(0.0));
            let beats: f64 = r
                .beats
                .iter()
                .map(|b| b.wait.map_or(0.0, |w| (w - DEAD_AIR_MAX_HOLD_S).max(0.0)))
                .sum();
            dead_air_part = -W_DEAD_AIR * (first + beats);
        }
        Err(_) => timeline_failed = true,
    }
    match count_unsettled(project, speech) {
        Ok(r) => {
            if r.status == CheckStatus::Fail {
                hard.push(COUNT_UNSETTLED.to_string());
            }
        }
        Err(_) => timeline_failed = true,
    }
    let (mut focal_part, mut balance_part) = (0.0, 0.0);
    match read_measures(project) {
        Ok(measures) => {
            let scored: Vec<f64> = measures
                .iter()
                .filter_map(|m| m.focal_share.map(focal_score))
                .collect();
            if !scored.is_empty() {
                focal_part = W_FOCAL * scored.iter().sum::<f64>() / scored.len() as f64;
            }
            balance_part = -W_BALANCE
                * measures
                    .iter()
                    .map(|m| balance_penalty(m.text_share))
                    .sum::<f64>();
        }
        Err(_) => timeline_failed = true,
    }
    if timeline_failed {
        hard.push(HARD_TIMELINE.to_string());
    }
    let variety = project
        .project
        .direction
        .as_ref()
        .map_or(0.0, |d| -W_REPEAT * repeats(&d.beats) as f64);
    let mut parts = BTreeMap::new();
    parts.insert("variety".to_string(), round6(variety));
    parts.insert("focal".to_string(), round6(focal_part));
    parts.insert("density".to_string(), round6(-density_penalty(project)));
    parts.insert("balance".to_string(), round6(balance_part));
    parts.insert("dead_air".to_string(), round6(dead_air_part));
    let soft = round6(parts.values().sum());
    Structural { hard, parts, soft }
}

/// The pixel stage: the hard checks that need rendered pixels. `base_dir` is
/// the directory the project's `asset_root` is relative to (the motion file's).
/// Frames stay in memory.
pub fn pixel_hard(project: &MotionProject, base_dir: &Path) -> Vec<String> {
    match text_local_contrast(project, base_dir) {
        Ok(r) if r.findings.is_empty() => Vec::new(),
        Ok(_) => vec![TEXT_LOCAL_CONTRAST.to_string()],
        Err(_) => vec![HARD_RENDER.to_string()],
    }
}

// ---------------------------------------------------------------------------
// The choice
// ---------------------------------------------------------------------------

/// The outcome of [`choose`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The candidate that ships.
    pub k: u8,
    /// `Some("every candidate failed: <check names>")` when it is not clean.
    pub shipped_with_failure: Option<String>,
}

/// Pick the candidate to ship: among those with no hard failure the highest
/// soft score (ties to the lowest `k`); when every candidate fails, the one
/// with the fewest failures, then the highest soft score, then the lowest `k`.
/// A candidate that did not compile (`HARD_COMPILE`) never ships while another
/// one compiled: it has no project and a soft score of 0, which would otherwise
/// outrank the usual negative scores. `None` for an empty list.
pub fn choose(scores: &[CandidateScore]) -> Option<Choice> {
    let better =
        |a: &&CandidateScore, b: &&CandidateScore| b.soft.total_cmp(&a.soft).then(a.k.cmp(&b.k));
    if let Some(best) = scores
        .iter()
        .filter(|s| s.hard.is_empty())
        .min_by(|a, b| better(a, b))
    {
        return Some(Choice {
            k: best.k,
            shipped_with_failure: None,
        });
    }
    let compiled = |s: &&CandidateScore| !s.hard.iter().any(|h| h == HARD_COMPILE);
    let pool: Vec<&CandidateScore> = if scores.iter().any(|s| compiled(&s)) {
        scores.iter().filter(|s| compiled(s)).collect()
    } else {
        scores.iter().collect()
    };
    let best = pool
        .into_iter()
        .min_by(|a, b| a.hard.len().cmp(&b.hard.len()).then(better(a, b)))?;
    Some(Choice {
        k: best.k,
        shipped_with_failure: Some(format!("every candidate failed: {}", best.hard.join(", "))),
    })
}

// ---------------------------------------------------------------------------
// Best-of-N
// ---------------------------------------------------------------------------

/// Where the candidates are judged.
#[derive(Debug, Clone, Copy)]
pub struct BestOfInput<'a> {
    /// How many candidates (1..=8; the caller validates).
    pub candidates: u8,
    /// The take seed (`direction::take_seed`): candidate k's seed is
    /// `candidate_seed(take_seed, k)`.
    pub take_seed: u64,
    /// The take's speech map, when the compile is speech-led.
    pub speech: Option<&'a SpeechMap>,
    /// The directory the compiled scene will be written to: validation and the
    /// pixel check resolve the asset root against it.
    pub base_dir: &'a Path,
    /// The `asset_root` the compiled scene gets (relative to `base_dir`).
    pub asset_root: &'a str,
}

/// The shipped candidate and what was judged.
#[derive(Debug, Clone, PartialEq)]
pub struct BestOf {
    /// The chosen project, with `direction.chosen`, `direction.candidates` and
    /// `direction.shipped_with_failure` written and `asset_root` set.
    pub project: MotionProject,
    pub warnings: Vec<CompileWarning>,
    pub chosen: u8,
    pub scores: Vec<CandidateScore>,
    pub shipped_with_failure: Option<String>,
    /// Candidates other than the chosen one that failed a hard check.
    pub dropped: usize,
}

impl BestOf {
    /// `candidates: 4, chose 2 (soft 3.1; 1 hard failure dropped)`.
    pub fn summary(&self) -> String {
        let soft = self
            .scores
            .iter()
            .find(|s| s.k == self.chosen)
            .map_or(0.0, |s| s.soft);
        if self.shipped_with_failure.is_some() {
            // Nothing was clean: nothing was dropped, the best failing one shipped.
            return format!(
                "candidates: {}, chose {} (soft {:.1}; every candidate failed, the best failing one shipped)",
                self.scores.len(),
                self.chosen,
                soft
            );
        }
        format!(
            "candidates: {}, chose {} (soft {:.1}; {} hard failure{} dropped)",
            self.scores.len(),
            self.chosen,
            soft,
            self.dropped,
            if self.dropped == 1 { "" } else { "s" }
        )
    }
}

struct Judged {
    score: CandidateScore,
    built: Result<(MotionProject, Vec<CompileWarning>), CompileError>,
}

/// Compile and structurally score candidate `k`.
fn judge<F>(input: &BestOfInput<'_>, compile: &F, k: u8) -> Judged
where
    F: Fn(u8) -> Result<(MotionProject, Vec<CompileWarning>), CompileError> + Sync,
{
    let seed = candidate_seed(input.take_seed, k);
    match compile(k) {
        Ok((mut project, warnings)) => {
            project.asset_root = Some(input.asset_root.to_string());
            let mut s = structural(&project, &warnings, input.speech);
            if validate(&project, Some(input.base_dir)).is_err() {
                s.hard.insert(0, HARD_VALIDATION.to_string());
            }
            Judged {
                score: CandidateScore {
                    k,
                    seed,
                    hard: s.hard,
                    soft: s.soft,
                    parts: s.parts,
                    pixel: false,
                },
                built: Ok((project, warnings)),
            }
        }
        // Reported through the score; the error itself is returned only when
        // no candidate compiled.
        Err(e) => Judged {
            score: CandidateScore {
                k,
                seed,
                hard: vec![HARD_COMPILE.to_string()],
                soft: 0.0,
                parts: BTreeMap::new(),
                pixel: false,
            },
            built: Err(e),
        },
    }
}

/// Compile `input.candidates` candidates in parallel (`compile(k)` is
/// `motion_core::compiler::compile_candidate` for candidate `k`), score them,
/// and return the one to ship. The structural stage runs on every candidate;
/// the pixel stage on the best two by soft score (more only while none of them
/// is clean).
///
/// Errors only when no candidate compiled (the error of candidate 0, which is
/// what a plain compile would have returned).
pub fn best_of<F>(input: &BestOfInput<'_>, compile: F) -> Result<BestOf, CompileError>
where
    F: Fn(u8) -> Result<(MotionProject, Vec<CompileWarning>), CompileError> + Sync,
{
    let n = input.candidates.max(1);
    // Candidate order is the iteration order; rayon collects in that order, so
    // no thread timing reaches the choice.
    let mut judged: Vec<Judged> = (0..n)
        .into_par_iter()
        .map(|k| judge(input, &compile, k))
        .collect();
    if judged.iter().all(|j| j.built.is_err()) {
        // Nothing compiled: surface candidate 0's own error.
        return match judged.swap_remove(0).built {
            Err(e) => Err(e),
            Ok(_) => Err(CompileError::Invalid("no candidate compiled".into())),
        };
    }

    // Pixel stage: structurally clean candidates by soft score, two at a time.
    let mut ranked: Vec<usize> = (0..judged.len())
        .filter(|&i| judged[i].score.hard.is_empty())
        .collect();
    ranked.sort_by(|&a, &b| {
        judged[b]
            .score
            .soft
            .total_cmp(&judged[a].score.soft)
            .then(judged[a].score.k.cmp(&judged[b].score.k))
    });
    for wave in ranked.chunks(PIXEL_WAVE) {
        let checked: Vec<(usize, Vec<String>)> = wave
            .par_iter()
            .map(|&i| {
                let failed = judged[i]
                    .built
                    .as_ref()
                    .map(|(p, _)| pixel_hard(p, input.base_dir))
                    .unwrap_or_default();
                (i, failed)
            })
            .collect();
        let mut any_clean = false;
        for (i, failed) in checked {
            judged[i].score.pixel = true;
            any_clean |= failed.is_empty();
            judged[i].score.hard.extend(failed);
        }
        if any_clean {
            break;
        }
    }

    let scores: Vec<CandidateScore> = judged.iter().map(|j| j.score.clone()).collect();
    let choice = choose(&scores).ok_or_else(|| CompileError::Invalid("no candidates".into()))?;
    let dropped = scores
        .iter()
        .filter(|s| s.k != choice.k && !s.hard.is_empty())
        .count();
    let Some(chosen) = judged.into_iter().find(|j| j.score.k == choice.k) else {
        return Err(CompileError::Invalid("chosen candidate missing".into()));
    };
    let (mut project, warnings) = chosen.built?;
    if let Some(d) = project.project.direction.as_mut() {
        d.chosen = choice.k;
        d.candidates = scores.clone();
        d.shipped_with_failure = choice.shipped_with_failure.clone();
    }
    Ok(BestOf {
        project,
        warnings,
        chosen: choice.k,
        scores,
        shipped_with_failure: choice.shipped_with_failure,
        dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(k: u8, hard: &[&str], soft: f64) -> CandidateScore {
        CandidateScore {
            k,
            seed: u64::from(k),
            hard: hard.iter().map(|s| s.to_string()).collect(),
            soft,
            parts: BTreeMap::new(),
            pixel: false,
        }
    }

    #[test]
    fn highest_soft_wins_and_ties_go_to_the_lowest_k() {
        let s = vec![
            score(0, &[], 1.0),
            score(1, &[], 2.5),
            score(2, &[], 2.5),
            score(3, &[], 0.1),
        ];
        let c = choose(&s).expect("choice");
        assert_eq!(c.k, 1);
        assert!(c.shipped_with_failure.is_none());
    }

    #[test]
    fn hard_failures_are_dropped_whatever_their_soft_score() {
        let s = vec![score(0, &["dead_air"], 9.0), score(1, &[], 0.2)];
        assert_eq!(choose(&s).map(|c| c.k), Some(1));
    }

    #[test]
    fn all_failing_ships_the_fewest_failures_and_says_so() {
        let s = vec![
            score(0, &["dead_air", "text_overlap"], 5.0),
            score(1, &["dead_air"], 1.0),
            score(2, &["dead_air"], 3.0),
        ];
        let c = choose(&s).expect("choice");
        assert_eq!(c.k, 2);
        assert_eq!(
            c.shipped_with_failure.as_deref(),
            Some("every candidate failed: dead_air")
        );
        assert!(choose(&[]).is_none());
    }

    #[test]
    fn a_candidate_that_did_not_compile_never_ships_while_another_compiled() {
        // Its soft score is 0.0, above any failing candidate's negative one.
        let s = vec![
            score(0, &["text_outside_safe"], -0.24),
            score(1, &["compile"], 0.0),
        ];
        let c = choose(&s).expect("choice");
        assert_eq!(c.k, 0);
        assert_eq!(
            c.shipped_with_failure.as_deref(),
            Some("every candidate failed: text_outside_safe")
        );
        // Only when none compiled does the full list decide.
        let none = vec![score(0, &["compile"], 0.0), score(1, &["compile"], 0.0)];
        assert_eq!(choose(&none).map(|c| c.k), Some(0));
    }

    #[test]
    fn balance_and_focal_curves() {
        assert_eq!(balance_penalty(0.2), 0.0);
        assert_eq!(balance_penalty(TEXT_MAX), 0.0);
        assert!(balance_penalty(0.6) > 0.0);
        assert_eq!(balance_penalty(0.9), 1.0);
        assert_eq!(balance_penalty(0.0), 1.0);
        assert!(balance_penalty(0.02) > 0.0 && balance_penalty(0.02) < 1.0);
        assert_eq!(focal_score(FOCAL_SHARE_LOW - 0.1), 0.0);
        assert_eq!(focal_score(FOCAL_SHARE_HIGH + 0.1), 1.0);
        let mid = focal_score((FOCAL_SHARE_LOW + FOCAL_SHARE_HIGH) / 2.0);
        assert!((mid - 0.5).abs() < 1e-9, "{mid}");
    }

    fn direction(beat: usize, template: &str, alternates: u8, arrival: &str) -> BeatDirection {
        let mut rotations = BTreeMap::new();
        rotations.insert("arrival".to_string(), arrival.to_string());
        BeatDirection {
            beat,
            template: template.to_string(),
            alternate: 0,
            alternates,
            params: Default::default(),
            rotations,
            role: String::new(),
            reason: String::new(),
        }
    }

    #[test]
    fn a_repeat_needs_the_same_template_entrance_camera_and_a_choice() {
        let same = [
            direction(0, "data_story", 1, "left"),
            direction(1, "data_story", 2, "left"),
            direction(2, "data_story", 2, "left"),
        ];
        assert_eq!(repeats(&same), 2);
        // The later beat had no alternative: the repeat was forced.
        assert_eq!(
            repeats(&[
                direction(0, "data_story", 2, "left"),
                direction(1, "data_story", 1, "left"),
            ]),
            0
        );
        // Another template or another arrival side is not a repeat.
        assert_eq!(
            repeats(&[
                direction(0, "data_story", 2, "left"),
                direction(1, "kinetic_poster", 2, "left"),
                direction(2, "kinetic_poster", 2, "right"),
            ]),
            0
        );
        // Beats out of order are judged in beat order; a gap breaks the pair.
        assert_eq!(
            repeats(&[
                direction(2, "a", 2, "left"),
                direction(0, "a", 2, "left"),
                direction(1, "a", 2, "left"),
            ]),
            2
        );
        assert_eq!(
            repeats(&[direction(0, "a", 2, "x"), direction(2, "a", 2, "x")]),
            0
        );
    }

    #[test]
    fn every_look_has_a_density_maximum_and_unknown_looks_use_the_default() {
        for look in [
            "cinematic_3d",
            "classical_neon",
            "dossier",
            "halftone_cutout",
            "hype_slam",
            "journey",
            "street_collage",
            "studio_pop",
        ] {
            assert_ne!(look_max(Some(look)), [0; 4], "{look}");
        }
        assert_eq!(look_max(Some("not_a_look")), DEFAULT_LOOK_MAX);
        assert_eq!(look_max(None), DEFAULT_LOOK_MAX);
    }

    #[test]
    fn occupancy_counts_overlap_once() {
        let mut o = Occupancy::new(960.0, 960.0);
        o.fill([0.0, 0.0, 480.0, 960.0]);
        o.fill([240.0, 0.0, 720.0, 960.0]);
        assert!((o.share() - 0.75).abs() < 0.02, "{}", o.share());
    }
}
