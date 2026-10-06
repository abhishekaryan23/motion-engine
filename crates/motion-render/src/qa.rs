//! Structural motion diagnostic.
//!
//! Renders a low-resolution luma profile of a project (animated grain removed)
//! and measures how much the composition changes from one sampled frame to the
//! next. It answers one question: is the pacing "static composition, giant
//! transition spike, static composition"? It is deliberately not a computer
//! vision system: no objects, no semantics, just frame-to-frame luma change.

use motion_core::scene::{LayerKind, Lifecycle, MotionProject, Phase, Scene};
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use rayon::prelude::*;
use resvg::tiny_skia::Pixmap;

use crate::{CpuRenderer, RenderError, Renderer};

/// A transition counts as static below this mean absolute luma difference.
const STATIC_DIFF: f32 = 0.002;
/// A transition is a spike above `max(SPIKE_MEDIAN_FACTOR * median, SPIKE_FLOOR)`.
const SPIKE_MEDIAN_FACTOR: f32 = 4.0;
const SPIKE_FLOOR: f32 = 0.02;
/// Spike peaks are reported as a multiple of `max(median, REF_FLOOR)`.
const REF_FLOOR: f32 = 0.005;
/// A static run must last at least this many seconds to count as a "hold" for the verdict.
const HOLD_SECONDS: f32 = 1.0;
/// A spike cluster may start up to this many sampled transitions after a hold ends.
const RAMP_TRANSITIONS: usize = 4;
/// Target linear downsample factor.
const BLOCK: u32 = 16;
/// A phase is `Low` below this mean diff (and not static).
const LOW_DIFF: f32 = 2.0 * STATIC_DIFF;
/// Motions starting within this many seconds of the previous cluster member are one evolve event.
const EVENT_CLUSTER_SECONDS: f64 = 0.12;
/// The transition spike window opens this many seconds before the bridge.
const SPIKE_WINDOW_SECONDS: f64 = 0.2;
/// A spike ratio above this is `High`.
const SPIKE_HIGH_RATIO: f32 = 8.0;
/// A dead hold at least this long (seconds) is a warning. A static 1.0–1.5 s
/// right after new information is deliberate reading time, not dead air.
const DEAD_HOLD_SECONDS: f32 = 1.5;
/// An EVOLVE phase at least this long (seconds) is expected to contain events.
const EVOLVE_MIN_SECONDS: f64 = 0.6;
/// The accumulated-change window: each sample is also diffed against the sample
/// this many seconds earlier (clamped at the scene start).
const ACCUM_WINDOW_SECONDS: f32 = 1.0;
/// A quiet (LOW/STATIC) phase whose mean 1 s accumulated change reaches this is
/// `Drift`: slow continuous motion whose per-frame diffs sit under the noise floor.
/// A truly still scene accumulates exactly 0 (grain is stripped, rendering is
/// deterministic).
const DRIFT_ACCUM: f32 = 0.001;
/// A cell is "content" when its luma differs from the frame's median luma by more than this.
const OCCUPANCY_DELTA: f32 = 0.04;
/// `empty lower third` fires when the bottom third is occupied below this...
const EMPTY_THIRD: f32 = 0.04;
/// ...while top + middle occupancy exceeds this.
const TOP_HEAVY_SUM: f32 = 0.25;

/// A run of consecutive spike frames.
#[derive(Debug, Clone, PartialEq)]
pub struct SpikeCluster {
    /// First and last frame of the cluster (inclusive).
    pub start: u32,
    pub end: u32,
    /// Frame with the largest diff, and that diff.
    pub peak_frame: u32,
    pub peak_diff: f32,
    /// `peak_diff / max(median, 0.005)`.
    pub peak_multiple: f32,
}

/// Frame-to-frame structural change of a rendered project.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionProfile {
    pub fps: u32,
    /// Sampling stride in frames (`diffs[i]` is the change into frame `(i + 1) * step`).
    pub step: u32,
    /// Per sampled transition, mean absolute luma difference in `0..=1`.
    pub diffs: Vec<f32>,
    pub median: f32,
    pub p95: f32,
    pub max: f32,
    /// Frames whose incoming transition exceeds `max(4 * median, 0.02)`.
    pub spikes: Vec<u32>,
    /// `spikes` grouped into consecutive ranges, each with its peak.
    pub spike_clusters: Vec<SpikeCluster>,
    /// `(first frame, last frame)` of runs lasting at least `fps / 2` frames with diff < 0.002.
    pub static_runs: Vec<(u32, u32)>,
    /// `"ok"`, or a short description of each static-then-spike pattern.
    pub verdict: String,
    /// Per sampled transition (aligned with `diffs`), mean absolute luma
    /// difference against the sample ~1 s earlier, clamped at the scene start.
    /// Empty when the profile was built from raw diffs with [`analyze`].
    pub accum: Vec<f32>,
    /// `(scene id, [top, middle, bottom] content occupancy)` measured on the
    /// middle frame of each lifecycle scene's READ phase. Empty from [`analyze`].
    pub scene_thirds: Vec<(String, [f32; 3])>,
}

/// Compute the structural profile of `project`, sampling every `step`-th frame.
///
/// Animated grain textures are dropped before rendering: film grain changes
/// every pixel every frame and is not scene motion.
pub fn structural_profile(
    project: &MotionProject,
    renderer: &CpuRenderer,
    step: u32,
) -> Result<MotionProfile, RenderError> {
    let step = step.max(1);
    let frames: Vec<u32> = (0..project.frame_count()).step_by(step as usize).collect();
    let grids: Vec<Grid> = frames
        .par_iter()
        .map(|&f| {
            let mut resolved = evaluate_frame(project, f)?;
            strip_animated_grain(&mut resolved.layers);
            let pm = renderer.render(&resolved)?;
            Ok(Grid::from_pixmap(&pm))
        })
        .collect::<Result<_, RenderError>>()?;
    let diffs: Vec<f32> = grids
        .windows(2)
        .map(|w| w[0].mean_abs_diff(&w[1]))
        .collect();

    // Accumulated change: sample j against sample j - window, never reaching
    // back across the start of the scene that contains it.
    let fps = project.canvas.fps;
    let window = ((fps as f32 * ACCUM_WINDOW_SECONDS / step as f32).round() as usize).max(1);
    let scene_starts: Vec<usize> = project
        .scenes
        .iter()
        .filter(|s| s.lifecycle.is_some())
        .map(|s| (abs_frame(s.start_seconds, fps) as usize).div_ceil(step as usize))
        .collect();
    let accum: Vec<f32> = (1..grids.len())
        .map(|j| {
            let scene_base = scene_starts
                .iter()
                .copied()
                .filter(|&b| b <= j)
                .max()
                .unwrap_or(0);
            let base = j.saturating_sub(window).max(scene_base).min(j - 1);
            grids[base].mean_abs_diff(&grids[j])
        })
        .collect();

    // Content occupancy by thirds on each scene's READ middle frame.
    let mut scene_thirds = Vec::new();
    for scene in &project.scenes {
        let Some(life) = &scene.lifecycle else {
            continue;
        };
        let (a, b) = life.range(Phase::Read, scene.duration_seconds);
        let mid = abs_frame(scene.start_seconds + (a + b) * 0.5, fps) as usize;
        let idx = ((mid as f32 / step as f32).round() as usize).min(grids.len().saturating_sub(1));
        if let Some(g) = grids.get(idx) {
            scene_thirds.push((scene.id.clone(), g.thirds()));
        }
    }

    let mut profile = analyze(fps, step, diffs);
    profile.accum = accum;
    profile.scene_thirds = scene_thirds;
    Ok(profile)
}

/// Build a profile (statistics, spikes, static runs, verdict) from raw diffs.
pub fn analyze(fps: u32, step: u32, diffs: Vec<f32>) -> MotionProfile {
    let step = step.max(1);
    let mut sorted = diffs.clone();
    sorted.sort_by(f32::total_cmp);
    let n = sorted.len();
    let median = match n {
        0 => 0.0,
        _ if n % 2 == 1 => sorted[n / 2],
        _ => (sorted[n / 2 - 1] + sorted[n / 2]) * 0.5,
    };
    let p95 = if n == 0 {
        0.0
    } else {
        sorted[((n as f32 * 0.95).ceil() as usize).clamp(1, n) - 1]
    };
    let max = sorted.last().copied().unwrap_or(0.0);
    let threshold = (SPIKE_MEDIAN_FACTOR * median).max(SPIKE_FLOOR);
    let frame_of = |i: usize| (i as u32 + 1) * step;

    let spikes: Vec<u32> = diffs
        .iter()
        .enumerate()
        .filter(|(_, &d)| d > threshold)
        .map(|(i, _)| frame_of(i))
        .collect();

    // Static runs: consecutive static transitions covering >= fps/2 frames.
    // Each run is (start, end, index of its last transition).
    let min_len = (fps / 2).max(1);
    let mut runs: Vec<(u32, u32, usize)> = Vec::new();
    let mut i = 0;
    while i < diffs.len() {
        if diffs[i] < STATIC_DIFF {
            let first = i;
            while i < diffs.len() && diffs[i] < STATIC_DIFF {
                i += 1;
            }
            let (start, end) = (frame_of(first) - step, frame_of(i - 1));
            if end - start >= min_len {
                runs.push((start, end, i - 1));
            }
        } else {
            i += 1;
        }
    }

    // Cluster consecutive spike transitions.
    let mut clusters: Vec<(usize, SpikeCluster)> = Vec::new(); // (first transition index, cluster)
    let mut i = 0;
    while i < diffs.len() {
        if diffs[i] > threshold {
            let first = i;
            let mut peak = i;
            while i < diffs.len() && diffs[i] > threshold {
                if diffs[i] > diffs[peak] {
                    peak = i;
                }
                i += 1;
            }
            clusters.push((
                first,
                SpikeCluster {
                    start: frame_of(first),
                    end: frame_of(i - 1),
                    peak_frame: frame_of(peak),
                    peak_diff: diffs[peak],
                    peak_multiple: diffs[peak] / median.max(REF_FLOOR),
                },
            ));
        } else {
            i += 1;
        }
    }

    // A spike cluster starting within RAMP_TRANSITIONS after a hold (static run >= 1 s).
    let fps_f = fps.max(1) as f32;
    let mut notes = Vec::new();
    for (first, c) in &clusters {
        let hold = runs
            .iter()
            .rfind(|&&(s, e, last)| last < *first && (e - s) as f32 / fps_f >= HOLD_SECONDS);
        if let Some(&(s, e, last)) = hold {
            if *first - last <= RAMP_TRANSITIONS {
                let range = if c.start == c.end {
                    format!("{}", c.start)
                } else {
                    format!("{}\u{2013}{}", c.start, c.end)
                };
                notes.push(format!(
                    "static {:.1}s then spike {range} peak x{:.1} at {:.1}s",
                    (e - s) as f32 / fps_f,
                    c.peak_multiple,
                    c.peak_frame as f32 / fps_f
                ));
            }
        }
    }
    let verdict = if notes.is_empty() {
        "ok".to_string()
    } else {
        notes.join("; ")
    };
    let spike_clusters: Vec<SpikeCluster> = clusters.into_iter().map(|(_, c)| c).collect();

    MotionProfile {
        fps,
        step,
        diffs,
        median,
        p95,
        max,
        spikes,
        spike_clusters,
        static_runs: runs.iter().map(|&(s, e, _)| (s, e)).collect(),
        verdict,
        accum: Vec::new(),
        scene_thirds: Vec::new(),
    }
}

impl MotionProfile {
    /// One character per sampled transition (`▁..█`), scaled to the larger of
    /// the profile maximum and the spike floor so tiny noise stays flat.
    pub fn sparkline(&self) -> String {
        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let scale = self.max.max(SPIKE_FLOOR);
        self.diffs
            .iter()
            .map(|&d| BARS[((d / scale * 7.0).round() as usize).min(7)])
            .collect()
    }

    /// Compact human-readable report.
    pub fn report(&self) -> String {
        let fps = self.fps.max(1) as f32;
        let mut s = String::new();
        s.push_str(&format!(
            "transitions {} (step {})  median {:.4}  p95 {:.4}  max {:.4}\n",
            self.diffs.len(),
            self.step,
            self.median,
            self.p95,
            self.max
        ));
        if self.spike_clusters.is_empty() {
            s.push_str("spikes: none\n");
        } else {
            let list: Vec<String> = self
                .spike_clusters
                .iter()
                .map(|c| {
                    let range = if c.start == c.end {
                        format!("{}", c.start)
                    } else {
                        format!("{}\u{2013}{}", c.start, c.end)
                    };
                    format!(
                        "{range} peak x{:.1} at frame {} ({:.2}s)",
                        c.peak_multiple,
                        c.peak_frame,
                        c.peak_frame as f32 / fps
                    )
                })
                .collect();
            s.push_str(&format!("spikes: {}\n", list.join(", ")));
        }
        if self.static_runs.is_empty() {
            s.push_str("static runs: none\n");
        } else {
            let list: Vec<String> = self
                .static_runs
                .iter()
                .map(|&(a, b)| {
                    format!(
                        "{:.1}s @ {:.2}s (frames {a}-{b})",
                        (b - a) as f32 / fps,
                        a as f32 / fps
                    )
                })
                .collect();
            s.push_str(&format!("static runs: {}\n", list.join(", ")));
        }
        s.push_str(&format!("verdict: {}\n", self.verdict));
        let chars: Vec<char> = self.sparkline().chars().collect();
        for line in chars.chunks(100) {
            s.push_str(&line.iter().collect::<String>());
            s.push('\n');
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Lifecycle-aware diagnostics
// ---------------------------------------------------------------------------

/// Classification of the structural activity inside one lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Healthy activity.
    Pass,
    /// Mean diff below `2 x` the static threshold.
    Low,
    /// Every diff below the static threshold.
    Static,
    /// Per-frame activity is LOW/STATIC but the 1 s accumulated change is
    /// visible: slow continuous motion. Counts as alive.
    Drift,
    /// The phase covers no sampled transition.
    Empty,
}

impl Status {
    /// Upper-case label used in reports and JSON.
    pub fn name(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Low => "LOW",
            Status::Static => "STATIC",
            Status::Drift => "DRIFT",
            Status::Empty => "EMPTY",
        }
    }

    fn is_quiet(self) -> bool {
        matches!(self, Status::Static | Status::Low)
    }
}

/// Whether the outgoing transition is abrupt relative to the scene's own motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpikeLevel {
    Ok,
    High,
}

impl SpikeLevel {
    /// Upper-case label used in reports and JSON.
    pub fn name(self) -> &'static str {
        match self {
            SpikeLevel::Ok => "OK",
            SpikeLevel::High => "HIGH",
        }
    }
}

/// Structural activity of one lifecycle phase of one scene.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseActivity {
    pub phase: Phase,
    /// First absolute frame of the phase.
    pub start_frame: u32,
    /// Absolute frame after the last frame of the phase (exclusive).
    pub end_frame: u32,
    /// Mean of the sampled diffs inside the phase (0 when empty).
    pub mean: f32,
    /// Largest sampled diff inside the phase (0 when empty).
    pub max: f32,
    /// Mean of the 1 s accumulated change inside the phase (0 when empty or unavailable).
    pub accum: f32,
    pub status: Status,
}

/// Lifecycle diagnostics of one scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneQa {
    pub scene: String,
    /// Seven entries, in `Phase::ALL` order.
    pub phases: Vec<PhaseActivity>,
    /// Mean diff over READ.
    pub read_activity: f32,
    /// Distinct motion start times in `[read, anticipate)` (from the motion file).
    pub evolve_events: usize,
    /// Longest run of static diffs within `[settle, bridge)`, in seconds.
    pub longest_static_hold: f32,
    /// Peak diff around the outgoing transition over the scene's typical diff.
    pub spike_ratio: f32,
    pub spike: SpikeLevel,
    /// `[top, middle, bottom]` fraction of cells holding content on the READ
    /// middle frame (all zero when it was not measured).
    pub thirds: [f32; 3],
    pub warnings: Vec<String>,
}

fn abs_frame(seconds: f64, fps: u32) -> i64 {
    (seconds * f64::from(fps)).round().max(0.0) as i64
}

/// Sampled diffs whose incoming frame lies in `[start, end)`, as `(index, diff)`.
fn diffs_in(
    profile: &MotionProfile,
    start: i64,
    end: i64,
) -> impl Iterator<Item = (usize, f32)> + '_ {
    let step = i64::from(profile.step.max(1));
    profile
        .diffs
        .iter()
        .copied()
        .enumerate()
        .filter(move |&(i, _)| {
            let f = (i as i64 + 1) * step;
            f >= start && f < end
        })
}

/// Accumulated (1 s) changes for the same transitions as [`diffs_in`]. Empty
/// when the profile carries no accumulated series.
fn accum_in(profile: &MotionProfile, start: i64, end: i64) -> Vec<f32> {
    let step = i64::from(profile.step.max(1));
    profile
        .accum
        .iter()
        .copied()
        .enumerate()
        .filter(|&(i, _)| {
            let f = (i as i64 + 1) * step;
            f >= start && f < end
        })
        .map(|(_, a)| a)
        .collect()
}

fn mean_of(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

fn classify(values: &[f32], accum: &[f32]) -> Status {
    if values.is_empty() {
        return Status::Empty;
    }
    let mean = mean_of(values);
    let quiet = if values.iter().all(|&d| d < STATIC_DIFF) {
        Status::Static
    } else if mean < LOW_DIFF {
        Status::Low
    } else {
        return Status::Pass;
    };
    if mean_of(accum) >= DRIFT_ACCUM {
        Status::Drift
    } else {
        quiet
    }
}

fn median_of(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    let n = values.len();
    match n {
        0 => 0.0,
        _ if n % 2 == 1 => values[n / 2],
        _ => (values[n / 2 - 1] + values[n / 2]) * 0.5,
    }
}

/// Number of distinct motion start times in `[from, to)`. Starts within 0.12 s
/// of the previous cluster member are one event; `.stage` and `.ghost` motions
/// (continuous camera-like motion) are excluded.
fn count_evolve_events(scene: &Scene, from: f64, to: f64) -> usize {
    let mut starts: Vec<f64> = scene
        .motions
        .iter()
        .filter(|m| !m.target.ends_with(".stage") && !m.target.ends_with(".ghost"))
        .map(|m| m.start)
        .filter(|&t| t >= from && t < to)
        .collect();
    starts.sort_by(f64::total_cmp);
    let mut events = 0;
    let mut prev: Option<f64> = None;
    for t in starts {
        match prev {
            Some(p) if t - p <= EVENT_CLUSTER_SECONDS + 1e-9 => {}
            _ => events += 1,
        }
        prev = Some(t);
    }
    events
}

/// Per-scene lifecycle diagnostics. Scenes without a `lifecycle` are skipped.
pub fn lifecycle_report(project: &MotionProject, profile: &MotionProfile) -> Vec<SceneQa> {
    project
        .scenes
        .iter()
        .filter_map(|scene| {
            scene
                .lifecycle
                .as_ref()
                .map(|life| scene_qa(scene, life, profile))
        })
        .collect()
}

fn scene_qa(scene: &Scene, life: &Lifecycle, profile: &MotionProfile) -> SceneQa {
    let fps = profile.fps.max(1);
    let fps_f = fps as f32;
    let step = profile.step.max(1) as f32;
    let duration = scene.duration_seconds;
    let frame_at = |local: f64| abs_frame(scene.start_seconds + local, fps);

    let phases: Vec<PhaseActivity> = Phase::ALL
        .iter()
        .map(|&phase| {
            let (a, b) = life.range(phase, duration);
            let (fa, fb) = (frame_at(a), frame_at(b));
            let values: Vec<f32> = diffs_in(profile, fa, fb).map(|(_, d)| d).collect();
            let accum = accum_in(profile, fa, fb);
            let mean = mean_of(&values);
            PhaseActivity {
                phase,
                start_frame: fa as u32,
                end_frame: fb.max(fa) as u32,
                mean,
                max: values.iter().copied().fold(0.0, f32::max),
                accum: mean_of(&accum),
                status: classify(&values, &accum),
            }
        })
        .collect();
    let status_of = |p: Phase| {
        phases
            .iter()
            .find(|a| a.phase == p)
            .map_or(Status::Empty, |a| a.status)
    };
    let read_activity = phases
        .iter()
        .find(|a| a.phase == Phase::Read)
        .map_or(0.0, |a| a.mean);

    let evolve_events = count_evolve_events(scene, life.read, life.anticipate);

    // Longest static run inside [settle, bridge), by consecutive transition index.
    // A sample inside a drift span (visible 1 s accumulated change) is not static.
    let mut longest = 0usize;
    let mut run = 0usize;
    for (i, d) in diffs_in(profile, frame_at(life.settle), frame_at(life.bridge)) {
        let drifting = profile.accum.get(i).is_some_and(|&a| a >= DRIFT_ACCUM);
        if d < STATIC_DIFF && !drifting {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let longest_static_hold = longest as f32 * step / fps_f;

    // Outgoing transition spike relative to the scene's own typical motion.
    let is_last = life.bridge >= duration - 1e-9;
    let (spike_ratio, spike) = if is_last {
        (0.0, SpikeLevel::Ok)
    } else {
        let peak = diffs_in(
            profile,
            frame_at(life.bridge - SPIKE_WINDOW_SECONDS),
            frame_at(duration) + 1,
        )
        .map(|(_, d)| d)
        .fold(0.0, f32::max);
        let mut body: Vec<f32> = diffs_in(profile, frame_at(life.enter), frame_at(life.bridge))
            .map(|(_, d)| d)
            .collect();
        let ratio = peak / median_of(&mut body).max(REF_FLOOR);
        let level = if ratio > SPIKE_HIGH_RATIO {
            SpikeLevel::High
        } else {
            SpikeLevel::Ok
        };
        (ratio, level)
    };

    let evolve_status = status_of(Phase::Evolve);
    let mut warnings = Vec::new();
    if status_of(Phase::Read).is_quiet()
        && (evolve_status.is_quiet() || evolve_status == Status::Empty)
        && spike == SpikeLevel::High
    {
        warnings.push("Static read phase before abrupt transition".to_string());
    }
    if longest_static_hold >= DEAD_HOLD_SECONDS {
        warnings.push(format!(
            "Dead hold {longest_static_hold:.1}s in READ/EVOLVE"
        ));
    }
    if evolve_events == 0 && life.anticipate - life.evolve >= EVOLVE_MIN_SECONDS {
        warnings.push("No evolve events".to_string());
    }
    if status_of(Phase::Enter) == Status::Static {
        warnings.push("ENTER shows no activity".to_string());
    }

    let thirds = profile
        .scene_thirds
        .iter()
        .find(|(id, _)| *id == scene.id)
        .map_or([0.0; 3], |&(_, t)| t);
    if thirds[2] < EMPTY_THIRD && thirds[0] + thirds[1] > TOP_HEAVY_SUM {
        warnings.push(format!(
            "Empty lower third (top-heavy composition): content top {:.2}, middle {:.2}, bottom {:.2}",
            thirds[0], thirds[1], thirds[2]
        ));
    }

    SceneQa {
        scene: scene.id.clone(),
        phases,
        read_activity,
        evolve_events,
        longest_static_hold,
        spike_ratio,
        spike,
        thirds,
        warnings,
    }
}

impl SceneQa {
    fn status_of(&self, phase: Phase) -> Status {
        self.phases
            .iter()
            .find(|a| a.phase == phase)
            .map_or(Status::Empty, |a| a.status)
    }

    /// Compact human-readable report of this scene.
    pub fn report(&self) -> String {
        let mut s = format!("{}\n", self.scene);
        s.push_str(&format!(
            "  ENTER activity: {}\n",
            self.status_of(Phase::Enter).name()
        ));
        s.push_str(&format!(
            "  SETTLE: {}\n",
            self.status_of(Phase::Settle).name()
        ));
        let read = self.status_of(Phase::Read);
        if read == Status::Pass {
            s.push_str("  READ structural activity: PASS\n");
        } else if read == Status::Drift {
            let accum = self
                .phases
                .iter()
                .find(|a| a.phase == Phase::Read)
                .map_or(0.0, |a| a.accum);
            s.push_str(&format!(
                "  READ structural activity: DRIFT (mean {:.4}, 1s change {accum:.4})\n",
                self.read_activity
            ));
        } else {
            s.push_str(&format!(
                "  READ structural activity: {} (mean {:.4})\n",
                read.name(),
                self.read_activity
            ));
        }
        s.push_str(&format!(
            "  EVOLVE events: {} (activity {})\n",
            self.evolve_events,
            self.status_of(Phase::Evolve).name()
        ));
        s.push_str(&format!(
            "  ANTICIPATE: {}\n",
            self.status_of(Phase::Anticipate).name()
        ));
        s.push_str(&format!(
            "  transition spike ratio: {} (x{:.1})\n",
            self.spike.name(),
            self.spike_ratio
        ));
        s.push_str(&format!(
            "  longest static hold: {:.1}s\n",
            self.longest_static_hold
        ));
        s.push_str(&format!(
            "  thirds (top/middle/bottom): {:.2} / {:.2} / {:.2}\n",
            self.thirds[0], self.thirds[1], self.thirds[2]
        ));
        for w in &self.warnings {
            s.push_str(&format!("  WARNING: {w}\n"));
        }
        s
    }
}

/// Remove animated texture layers (film grain), recursing into groups.
fn strip_animated_grain(layers: &mut Vec<ResolvedLayer<'_>>) {
    layers.retain(|l| !matches!(l.kind, LayerKind::Texture(spec) if spec.animated));
    for l in layers.iter_mut() {
        strip_animated_grain(&mut l.children);
    }
}

/// Blurred, box-downsampled luma image.
struct Grid {
    data: Vec<f32>,
    gw: usize,
    gh: usize,
}

impl Grid {
    fn from_pixmap(pm: &Pixmap) -> Self {
        let (w, h) = (pm.width() as usize, pm.height() as usize);
        let block = BLOCK.min((w.min(h) as u32 / 8).max(1)) as usize;
        let (gw, gh) = (w.div_ceil(block), h.div_ceil(block));
        let px = pm.data();
        let mut small = vec![0.0f32; gw * gh];
        for gy in 0..gh {
            for gx in 0..gw {
                let (x0, y0) = (gx * block, gy * block);
                let (x1, y1) = ((x0 + block).min(w), (y0 + block).min(h));
                let mut sum = 0.0f32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = (y * w + x) * 4;
                        // Premultiplied RGBA; the canvas background is opaque.
                        sum += 0.2126 * px[i] as f32
                            + 0.7152 * px[i + 1] as f32
                            + 0.0722 * px[i + 2] as f32;
                    }
                }
                small[gy * gw + gx] = sum / (255.0 * ((x1 - x0) * (y1 - y0)) as f32);
            }
        }
        // 3x3 box blur with clamped edges.
        let mut data = vec![0.0f32; gw * gh];
        for y in 0..gh {
            for x in 0..gw {
                let mut sum = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let sx = (x as i32 + dx).clamp(0, gw as i32 - 1) as usize;
                        let sy = (y as i32 + dy).clamp(0, gh as i32 - 1) as usize;
                        sum += small[sy * gw + sx];
                    }
                }
                data[y * gw + x] = sum / 9.0;
            }
        }
        Grid { data, gw, gh }
    }

    /// Fraction of cells per horizontal third (top, middle, bottom) whose luma
    /// differs from the frame's median luma by more than `OCCUPANCY_DELTA`.
    fn thirds(&self) -> [f32; 3] {
        if self.data.is_empty() || self.gw == 0 {
            return [0.0; 3];
        }
        let mut sorted = self.data.clone();
        let background = median_of(&mut sorted);
        let mut hit = [0usize; 3];
        let mut total = [0usize; 3];
        for (i, &v) in self.data.iter().enumerate() {
            let band = ((i / self.gw) * 3 / self.gh).min(2);
            total[band] += 1;
            if (v - background).abs() > OCCUPANCY_DELTA {
                hit[band] += 1;
            }
        }
        let frac = |k: usize| {
            if total[k] == 0 {
                0.0
            } else {
                hit[k] as f32 / total[k] as f32
            }
        };
        [frac(0), frac(1), frac(2)]
    }

    fn mean_abs_diff(&self, other: &Grid) -> f32 {
        if self.data.is_empty() || self.data.len() != other.data.len() {
            return 0.0;
        }
        let sum: f32 = self
            .data
            .iter()
            .zip(&other.data)
            .map(|(a, b)| (a - b).abs())
            .sum();
        sum / self.data.len() as f32
    }
}
