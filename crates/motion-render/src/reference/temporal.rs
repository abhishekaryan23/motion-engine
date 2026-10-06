//! Deterministic temporal (visual-change) evidence.
//!
//! Visual change only (never audio). Everything is derived from the mean
//! absolute RGB difference (`signal::rgb_mean_abs_diff`) between consecutive analysis frames: single-frame
//! cuts, short multi-frame transitions that settle into a new composition, and
//! slow evolutions that accumulate into a visibly different one. These are
//! measurements, not semantic labels.

use motion_core::reference::evidence::{
    round_ms, ChangeKind, ChangeSummary, Distribution, DurationTendency, TemporalEvidence,
};

use super::signal::{
    region_rgb_mean_abs_diff, rgb_mean_abs_diff, AnalysisStream, ChangeEvent, Rect, TemporalSignals,
};
use crate::qa;

/// Minimum incoming diff for a single-frame cut.
const CUT_DIFF: f32 = 0.10;
/// Both neighbouring diffs of a cut must be below this fraction of its diff.
const CUT_ISOLATION: f32 = 0.35;
/// Absolute lower bound of the transition floor.
const TRANSITION_FLOOR_MIN: f32 = 0.015;
/// The transition floor is at least this multiple of the median diff.
const TRANSITION_MEDIAN_FACTOR: f32 = 3.0;
/// Elevated runs separated by at most this many frames are merged.
const TRANSITION_MERGE_GAP: usize = 2;
/// A transition must span at least this many diffs.
const TRANSITION_MIN_DIFFS: usize = 2;
/// A transition must change the composition by at least this much
/// (start-1 versus end), otherwise it is a shake that returns.
const SETTLE_CHANGE: f32 = 0.03;
/// Accumulated diff (against the anchor) that makes an evolution.
const EVOLVE_ACCUM: f32 = 0.05;
/// An evolution spans at least this many seconds.
const EVOLVE_MIN_SECONDS: f64 = 0.5;

/// Diffs below this are static.
const STATIC_DIFF: f32 = 0.002;
/// Diffs below this are low motion.
const LOW_MOTION_DIFF: f32 = 0.006;
/// Non-cut diffs above this are high motion.
const HIGH_MOTION_DIFF: f32 = 0.02;
/// Floor of the median in the spike ratio (as in Motion QA).
const SPIKE_MEDIAN_FLOOR: f32 = 0.005;
/// Margin (fraction of each side) separating background ring and centre.
const FOREGROUND_MARGIN: f32 = 0.15;

/// Hard-cut share from which the tendency is `Cut`.
const CUT_TENDENCY_FRACTION: f32 = 0.6;
/// Median non-cut change shorter than this is `Short`.
const SHORT_TRANSITION_SECONDS: f32 = 0.3;
/// Median non-cut change shorter than this is `Medium`, otherwise `Long`.
const MEDIUM_TRANSITION_SECONDS: f32 = 0.8;
/// Upper bound of the subtracted noise floor (a constantly moving video must
/// not have its motion mistaken for noise).
const MAX_NOISE_FLOOR: f32 = 0.01;
/// Maximum number of changes listed in the evidence.
const MAX_CHANGES: usize = 200;

/// Detect cuts, transitions and evolutions on the analysis stream.
pub fn temporal_signals(stream: &AnalysisStream) -> TemporalSignals {
    if stream.len() < 2 {
        return TemporalSignals::default();
    }
    let frames = &stream.frames;
    let raw: Vec<f32> = frames
        .windows(2)
        .map(|w| rgb_mean_abs_diff(&w[0], &w[1]))
        .collect();
    // Grain and compression noise change every frame without moving anything.
    // Detect events on raw differences: subtraction must not move a cut or
    // dissolve across its threshold. Activity and sampling use corrected diffs.
    let noise_floor = Distribution::of(&raw).p10.min(MAX_NOISE_FLOOR);
    let diffs = raw;
    let n = diffs.len();
    let profile = qa::analyze(fps_u32(stream.fps), 1, diffs.clone());

    // Cuts: frame i has incoming diff diffs[i - 1].
    let mut cut_diff = vec![false; n];
    let mut events: Vec<ChangeEvent> = Vec::new();
    for i in 1..=n {
        let d = diffs[i - 1];
        if d < CUT_DIFF {
            continue;
        }
        let prev = if i >= 2 { diffs[i - 2] } else { 0.0 };
        let next = diffs.get(i).copied().unwrap_or(0.0);
        if prev < CUT_ISOLATION * d && next < CUT_ISOLATION * d {
            cut_diff[i - 1] = true;
            events.push(ChangeEvent {
                start: i,
                end: i,
                peak: i,
                kind: ChangeKind::Cut,
                magnitude: d,
            });
        }
    }

    // Transitions: runs of elevated non-cut diffs.
    let floor = TRANSITION_FLOOR_MIN.max(TRANSITION_MEDIAN_FACTOR * profile.median);
    let mut runs: Vec<(usize, usize)> = Vec::new(); // inclusive diff indices
    let mut j = 0;
    while j < n {
        if !cut_diff[j] && diffs[j] > floor {
            let first = j;
            while j < n && !cut_diff[j] && diffs[j] > floor {
                j += 1;
            }
            runs.push((first, j - 1));
        } else {
            j += 1;
        }
    }
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for run in runs {
        if let Some(last) = merged.last_mut() {
            let gap = run.0 - last.1 - 1;
            let has_cut = cut_diff[last.1 + 1..run.0].iter().any(|&c| c);
            if gap <= TRANSITION_MERGE_GAP && !has_cut {
                last.1 = run.1;
                continue;
            }
        }
        merged.push(run);
    }
    for (first, last) in merged {
        if last - first + 1 < TRANSITION_MIN_DIFFS {
            continue;
        }
        let (start, end) = (first + 1, last + 1);
        if rgb_mean_abs_diff(&frames[start - 1], &frames[end]) < SETTLE_CHANGE {
            continue;
        }
        let mut peak = first;
        for k in first..=last {
            if diffs[k] > diffs[peak] {
                peak = k;
            }
        }
        events.push(ChangeEvent {
            start,
            end,
            peak: peak + 1,
            kind: ChangeKind::Transition,
            magnitude: diffs[peak],
        });
    }
    events.sort_by_key(|e| e.start);

    // Evolutions: slow accumulation in the gaps between the other events.
    let min_span = ((EVOLVE_MIN_SECONDS * stream.fps).round() as usize).max(1);
    let last_frame = stream.len() - 1;
    let mut evolutions: Vec<ChangeEvent> = Vec::new();
    let mut anchor = 0usize;
    let mut gaps: Vec<usize> = Vec::new(); // gap end frames (inclusive)
    for e in &events {
        gaps.push(e.start.saturating_sub(1));
    }
    gaps.push(last_frame);
    for (gap_idx, &gap_end) in gaps.iter().enumerate() {
        for i in (anchor + 1)..=gap_end {
            let accum = rgb_mean_abs_diff(&frames[anchor], &frames[i]);
            if accum >= EVOLVE_ACCUM && i - anchor >= min_span {
                let mut peak = anchor + 1;
                for f in (anchor + 1)..=i {
                    if diffs[f - 1] > diffs[peak - 1] {
                        peak = f;
                    }
                }
                evolutions.push(ChangeEvent {
                    start: anchor + 1,
                    end: i,
                    peak,
                    kind: ChangeKind::Evolution,
                    magnitude: accum,
                });
                anchor = i;
            }
        }
        if let Some(e) = events.get(gap_idx) {
            anchor = e.end;
        }
    }
    events.extend(evolutions);
    events.sort_by_key(|e| e.start);

    TemporalSignals {
        diffs: diffs
            .into_iter()
            .map(|d| (d - noise_floor).max(0.0))
            .collect(),
        events,
        noise_floor,
    }
}

/// Summarize `signals` (from [`temporal_signals`] on the same `stream`) as
/// evidence. With no changes at all the whole video is one hold and the
/// duration tendency is `Long` (no changes = long holds).
pub fn temporal_evidence(stream: &AnalysisStream, signals: &TemporalSignals) -> TemporalEvidence {
    let fps = stream.fps;
    let duration = if fps > 0.0 {
        stream.len() as f64 / fps
    } else {
        0.0
    };
    let diffs = &signals.diffs;
    let events = &signals.events;
    let n = diffs.len();

    let mut cut_diff = vec![false; n];
    for e in events.iter().filter(|e| e.kind == ChangeKind::Cut) {
        if let Some(c) = e.start.checked_sub(1).and_then(|i| cut_diff.get_mut(i)) {
            *c = true;
        }
    }
    let non_cut: Vec<f32> = diffs
        .iter()
        .zip(&cut_diff)
        .filter(|(_, &c)| !c)
        .map(|(&d, _)| d)
        .collect();

    let per_10s = |count: usize| {
        if duration > 0.0 {
            (count as f64 / duration * 10.0) as f32
        } else {
            0.0
        }
    };
    let cuts = events.iter().filter(|e| e.kind == ChangeKind::Cut).count();

    let changes: Vec<ChangeSummary> = events
        .iter()
        .take(MAX_CHANGES)
        .map(|e| ChangeSummary {
            time_seconds: round_ms(stream.time_of(e.start)),
            duration_seconds: round_ms((e.end - e.start + 1) as f64 / fps),
            kind: e.kind,
            magnitude: r4(e.magnitude),
        })
        .collect();

    // Holds: time between the starts of consecutive changes, bounded by the
    // video start and end. (Gaps between an event's end and the next start
    // collapse to zero when slow evolutions chain during continuous motion.)
    let mut holds: Vec<f32> = Vec::new();
    let mut prev_start = 0.0f64;
    for e in events {
        let t = stream.time_of(e.start);
        holds.push((t - prev_start).max(0.0) as f32);
        prev_start = t;
    }
    holds.push((duration - prev_start).max(0.0) as f32);
    let hold_dist = Distribution::of(&holds);

    let share = |count: usize| {
        if n == 0 {
            0.0
        } else {
            count as f32 / n as f32
        }
    };
    let static_fraction = share(diffs.iter().filter(|&&d| d < STATIC_DIFF).count());
    let low_motion_fraction = share(diffs.iter().filter(|&&d| d < LOW_MOTION_DIFF).count());
    let high_motion_fraction = share(non_cut.iter().filter(|&&d| d > HIGH_MOTION_DIFF).count());

    let activity = Distribution::of(&non_cut);
    let activity_variance = if activity.mean > 0.0 && !non_cut.is_empty() {
        let var = non_cut
            .iter()
            .map(|d| (d - activity.mean).powi(2))
            .sum::<f32>()
            / non_cut.len() as f32;
        var.sqrt() / activity.mean
    } else {
        0.0
    };

    let profile = qa::analyze(fps_u32(fps), 1, diffs.clone());
    let transition_spike_ratio = profile.p95 / profile.median.max(SPIKE_MEDIAN_FLOOR);

    let durations: Vec<f32> = events
        .iter()
        .filter(|e| e.kind != ChangeKind::Cut)
        .map(|e| ((e.end - e.start + 1) as f64 / fps) as f32)
        .collect();
    let median_transition_seconds = if durations.is_empty() {
        0.0
    } else {
        Distribution::of(&durations).p50
    };
    let hard_cut_fraction = if events.is_empty() {
        0.0
    } else {
        cuts as f32 / events.len() as f32
    };
    let transition_duration = if events.is_empty() {
        DurationTendency::Long
    } else if hard_cut_fraction >= CUT_TENDENCY_FRACTION {
        DurationTendency::Cut
    } else if median_transition_seconds < SHORT_TRANSITION_SECONDS {
        DurationTendency::Short
    } else if median_transition_seconds < MEDIUM_TRANSITION_SECONDS {
        DurationTendency::Medium
    } else {
        DurationTendency::Long
    };

    // Background ring versus centre activity over non-cut transitions.
    let (mut bg_sum, mut fg_sum, mut count) = (0.0f32, 0.0f32, 0usize);
    if stream.len() >= 2 {
        let rect = Rect::inset(stream.width, stream.height, FOREGROUND_MARGIN);
        for (j, &is_cut) in cut_diff.iter().enumerate() {
            if !is_cut {
                let (prev, next) = (&stream.frames[j], &stream.frames[j + 1]);
                let floor = signals.noise_floor;
                bg_sum += (region_rgb_mean_abs_diff(prev, next, stream.width, rect, false) - floor)
                    .max(0.0);
                fg_sum += (region_rgb_mean_abs_diff(prev, next, stream.width, rect, true) - floor)
                    .max(0.0);
                count += 1;
            }
        }
    }
    let (background_activity, foreground_activity) = if count == 0 {
        (0.0, 0.0)
    } else {
        (bg_sum / count as f32, fg_sum / count as f32)
    };

    TemporalEvidence {
        changes,
        noise_floor: r4(signals.noise_floor),
        cuts_per_10s: r4(per_10s(cuts)),
        changes_per_10s: r4(per_10s(events.len())),
        median_hold_seconds: r4(hold_dist.p50),
        hold_p90_seconds: r4(hold_dist.p90),
        static_fraction: r4(static_fraction),
        low_motion_fraction: r4(low_motion_fraction),
        high_motion_fraction: r4(high_motion_fraction),
        activity: Distribution {
            mean: r4(activity.mean),
            p10: r4(activity.p10),
            p50: r4(activity.p50),
            p90: r4(activity.p90),
        },
        activity_variance: r4(activity_variance),
        transition_spike_ratio: r4(transition_spike_ratio),
        median_transition_seconds: r4(median_transition_seconds),
        transition_duration,
        hard_cut_fraction: r4(hard_cut_fraction),
        background_activity: r4(background_activity),
        foreground_activity: r4(foreground_activity),
    }
}

/// Round to 4 decimals (stable JSON).
fn r4(x: f32) -> f32 {
    (x * 10_000.0).round() / 10_000.0
}

fn fps_u32(fps: f64) -> u32 {
    fps.round().clamp(1.0, u32::MAX as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 32;
    const H: u32 = 56;
    const FPS: f64 = 30.0;

    /// Grayscale stream from a per-(frame, x, y) level in `0..=1`.
    fn stream_with(
        w: u32,
        h: u32,
        frames: usize,
        level: impl Fn(usize, u32, u32) -> f32,
    ) -> AnalysisStream {
        let frames = (0..frames)
            .map(|t| {
                let mut buf = Vec::with_capacity((w * h * 3) as usize);
                for y in 0..h {
                    for x in 0..w {
                        let v = (level(t, x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
                        buf.extend_from_slice(&[v, v, v]);
                    }
                }
                buf
            })
            .collect();
        AnalysisStream {
            width: w,
            height: h,
            fps: FPS,
            frames,
        }
    }

    fn run(stream: &AnalysisStream) -> (TemporalSignals, TemporalEvidence) {
        let signals = temporal_signals(stream);
        let evidence = temporal_evidence(stream, &signals);
        (signals, evidence)
    }

    /// Two distinct compositions (checker-ish halves).
    fn composition(k: usize, x: u32, y: u32) -> f32 {
        let a = if (x / 8 + y / 8).is_multiple_of(2) {
            0.1
        } else {
            0.6
        };
        if k == 0 {
            a
        } else {
            0.7 - a
        }
    }

    #[test]
    fn static_clip_has_no_events() {
        let s = stream_with(W, H, 90, |_, _, _| 0.4);
        let (sig, ev) = run(&s);
        assert!(sig.events.is_empty());
        assert!(ev.changes.is_empty());
        assert_eq!(ev.static_fraction, 1.0);
        assert_eq!(ev.changes_per_10s, 0.0);
        assert_eq!(ev.median_hold_seconds, 3.0);
        assert_eq!(ev.transition_duration, DurationTendency::Long);
    }

    #[test]
    fn low_level_noise_is_removed_from_all_activity_measures() {
        // A deterministic one-code-value changing texture, without motion.
        let s = stream_with(W, H, 90, |t, x, y| {
            (100 + ((t + x as usize + y as usize) % 2) as u8) as f32 / 255.0
        });
        let (sig, ev) = run(&s);
        assert!(sig.noise_floor > 0.003 && sig.noise_floor < 0.004);
        assert!(sig.diffs.iter().all(|&d| d == 0.0));
        assert!(sig.events.is_empty());
        assert_eq!(ev.static_fraction, 1.0);
        assert_eq!(ev.low_motion_fraction, 1.0);
        assert_eq!(ev.activity, Distribution::default());
        assert_eq!(ev.background_activity, 0.0);
        assert_eq!(ev.foreground_activity, 0.0);
        let samples = super::super::sample::select_samples(&sig, FPS, s.len(), 12);
        assert!(samples.iter().all(|s| !s
            .kinds
            .contains(&motion_core::reference::evidence::SampleKind::HighMotion)));
    }

    #[test]
    fn noise_floor_is_capped_and_cut_magnitude_stays_raw() {
        let motion = stream_with(W, H, 30, |t, _, _| if t % 2 == 0 { 0.2 } else { 0.4 });
        let (sig, ev) = run(&motion);
        assert_eq!(sig.noise_floor, MAX_NOISE_FLOOR);
        assert!(ev.activity.mean > 0.18);
        assert_eq!(ev.high_motion_fraction, 1.0);

        let cut = stream_with(W, H, 60, |t, _, _| {
            (100 + (t % 2) as u8 + if t >= 30 { 27 } else { 0 }) as f32 / 255.0
        });
        let (sig, _) = run(&cut);
        assert_eq!(sig.events.len(), 1);
        assert_eq!(sig.events[0].kind, ChangeKind::Cut);
        assert_eq!(sig.events[0].start, 30);
        assert_eq!(
            sig.events[0].magnitude,
            rgb_mean_abs_diff(&cut.frames[29], &cut.frames[30])
        );
        assert!(sig.diffs[29] < sig.events[0].magnitude);
    }

    #[test]
    fn short_streams_are_empty() {
        let s = stream_with(W, H, 1, |_, _, _| 0.4);
        assert_eq!(temporal_signals(&s), TemporalSignals::default());
        let s = stream_with(W, H, 0, |_, _, _| 0.4);
        let (sig, ev) = run(&s);
        assert!(sig.events.is_empty());
        assert_eq!(ev.changes_per_10s, 0.0);
    }

    #[test]
    fn fast_cuts_are_cuts() {
        let s = stream_with(
            W,
            H,
            300,
            |t, _, _| if (t / 10) % 2 == 0 { 0.1 } else { 0.9 },
        );
        let (sig, ev) = run(&s);
        assert_eq!(sig.events.len(), 29);
        assert!(sig
            .events
            .iter()
            .all(|e| e.kind == ChangeKind::Cut && e.start == e.end && e.peak == e.start));
        assert!((ev.cuts_per_10s - 30.0).abs() < 2.0, "{}", ev.cuts_per_10s);
        assert_eq!(ev.hard_cut_fraction, 1.0);
        assert_eq!(ev.transition_duration, DurationTendency::Cut);
        assert_eq!(ev.median_transition_seconds, 0.0);
        // Cuts are excluded from the activity and motion shares.
        assert_eq!(ev.high_motion_fraction, 0.0);
        assert_eq!(ev.activity.mean, 0.0);
    }

    #[test]
    fn slow_editorial_crossfades_are_transitions() {
        // Composition switches with a 15-frame crossfade every 120 frames.
        let s = stream_with(W, H, 480, |t, x, y| {
            let k = t / 120;
            let phase = t % 120;
            let (from, to) = (k % 2, (k + 1) % 2);
            // Crossfade occupies frames 105..=119 of each period.
            let mix = if phase >= 105 {
                (phase - 104) as f32 / 15.0
            } else {
                0.0
            };
            composition(from, x, y) * (1.0 - mix) + composition(to, x, y) * mix
        });
        let (sig, ev) = run(&s);
        let transitions: Vec<_> = sig
            .events
            .iter()
            .filter(|e| e.kind == ChangeKind::Transition)
            .collect();
        assert_eq!(transitions.len(), 4, "{:?}", sig.events);
        assert!(sig.events.iter().all(|e| e.kind != ChangeKind::Cut));
        assert_eq!(ev.cuts_per_10s, 0.0);
        assert!(matches!(
            ev.transition_duration,
            DurationTendency::Short | DurationTendency::Medium
        ));
        assert!(
            (ev.median_hold_seconds - 4.0).abs() < 0.2,
            "{}",
            ev.median_hold_seconds
        );
    }

    #[test]
    fn slow_drift_is_evolution() {
        // A bar moving 1 px/frame on a flat ground.
        let s = stream_with(96, 56, 60, |t, x, y| {
            let bx = t as u32;
            if x >= bx && x < bx + 6 && (12..44).contains(&y) {
                0.9
            } else {
                0.1
            }
        });
        let (sig, ev) = run(&s);
        assert!(!sig.events.is_empty());
        assert!(sig.events.iter().all(|e| e.kind == ChangeKind::Evolution));
        assert!(
            ev.high_motion_fraction < 0.01,
            "{}",
            ev.high_motion_fraction
        );
        // Each evolution spans ~0.5 s.
        assert_eq!(ev.transition_duration, DurationTendency::Medium);
    }

    #[test]
    fn high_motion_pattern() {
        // A large stripe pattern shifting every frame.
        let s = stream_with(W, H, 60, |t, x, _| {
            if ((x as usize + t * 3) / 4).is_multiple_of(2) {
                0.15
            } else {
                0.35
            }
        });
        let (_, ev) = run(&s);
        assert!(ev.high_motion_fraction > 0.5, "{}", ev.high_motion_fraction);
    }

    #[test]
    fn shake_that_returns_is_not_a_change() {
        // A, B, A, B, A then static.
        let s = stream_with(W, H, 40, |t, x, y| {
            if t < 5 && t % 2 == 1 {
                composition(1, x, y)
            } else {
                composition(0, x, y)
            }
        });
        let (sig, _) = run(&s);
        assert!(
            sig.events.iter().all(|e| e.kind != ChangeKind::Transition),
            "{:?}",
            sig.events
        );
    }

    #[test]
    fn background_versus_foreground_activity() {
        let border = |x: u32, y: u32| {
            let r = Rect::inset(W, H, FOREGROUND_MARGIN);
            !r.contains(x, y)
        };
        let s = stream_with(W, H, 30, |t, x, y| {
            if border(x, y) && t % 2 == 1 {
                0.55
            } else {
                0.5
            }
        });
        let (_, ev) = run(&s);
        assert!(ev.background_activity > ev.foreground_activity);
        assert_eq!(ev.foreground_activity, 0.0);

        let s = stream_with(W, H, 30, |t, x, y| {
            if !border(x, y) && t % 2 == 1 {
                0.55
            } else {
                0.5
            }
        });
        let (_, ev) = run(&s);
        assert!(ev.foreground_activity > ev.background_activity);
        assert_eq!(ev.background_activity, 0.0);
    }

    #[test]
    fn deterministic_ordered_non_overlapping() {
        // Cuts, a crossfade, a drift and holds in one clip.
        let s = stream_with(96, 56, 400, |t, x, y| {
            if t < 100 {
                if (t / 25) % 2 == 0 {
                    0.1
                } else {
                    0.9
                }
            } else if t < 200 {
                let mix = ((t as f32 - 140.0) / 15.0).clamp(0.0, 1.0);
                composition(0, x, y) * (1.0 - mix) + composition(1, x, y) * mix
            } else {
                let bx = (t - 200) as u32 / 2;
                if x >= bx && x < bx + 8 && (8..48).contains(&y) {
                    0.9
                } else {
                    0.2
                }
            }
        });
        let (sig, ev) = run(&s);
        let (sig2, ev2) = run(&s);
        assert_eq!(sig, sig2);
        assert_eq!(ev, ev2);
        assert!(!sig.events.is_empty());
        for w in sig.events.windows(2) {
            assert!(w[0].start <= w[0].end);
            assert!(w[0].end < w[1].start, "{:?}", sig.events);
        }
        for e in &sig.events {
            assert!(e.start <= e.peak && e.peak <= e.end);
        }
        assert_eq!(ev.changes.len(), sig.events.len());
    }
}
