//! Deterministic key-frame selection.
//!
//! Pure functions of the temporal signals: no clocks, no hash-map iteration,
//! ties always resolve to the lower frame index.

use motion_core::reference::evidence::{round_ms, SampleKind};

use super::signal::TemporalSignals;

/// One chosen key frame (before ids/images are assigned).
#[derive(Debug, Clone, PartialEq)]
pub struct SamplePlan {
    /// Analysis-stream frame index.
    pub frame: usize,
    pub time_seconds: f64,
    /// Sorted, deduplicated.
    pub kinds: Vec<SampleKind>,
}

/// How many key frames a video of `duration_seconds` gets.
pub fn sample_budget(duration_seconds: f64) -> usize {
    let d = duration_seconds;
    if !d.is_finite() || d <= 0.0 {
        return 12;
    }
    if d <= 90.0 {
        ((12.0 + 0.4 * d).round() as usize).clamp(12, 30)
    } else {
        (30 + ((d - 90.0) / 30.0).floor() as usize).min(40)
    }
}

/// Higher wins when candidates merge: Stable > SceneChange > HighMotion > Periodic.
fn priority(kind: SampleKind) -> u8 {
    match kind {
        SampleKind::Periodic => 0,
        SampleKind::HighMotion => 1,
        SampleKind::SceneChange => 2,
        SampleKind::Stable => 3,
    }
}

fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn scene_change_candidates(
    signals: &TemporalSignals,
    fps: f64,
    frames: usize,
    quota: usize,
) -> Vec<usize> {
    let settle = (0.3 * fps).round() as usize;
    let events = &signals.events;
    let mut picked: Vec<(usize, f32, usize)> = events
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut frame = e.end.saturating_add(settle).min(frames - 1);
            if let Some(next) = events.get(i + 1) {
                frame = frame.min(next.start.saturating_sub(1));
            }
            (frame, e.magnitude, i)
        })
        .collect();
    if picked.len() > quota {
        // Largest magnitude first; ties: earlier event.
        picked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)));
        picked.truncate(quota);
    }
    let mut out: Vec<usize> = picked.into_iter().map(|p| p.0).collect();
    out.sort_unstable();
    out
}

fn high_motion_candidates(
    signals: &TemporalSignals,
    fps: f64,
    frames: usize,
    quota: usize,
) -> Vec<(f32, usize)> {
    let diffs = &signals.diffs;
    if diffs.is_empty() || quota == 0 {
        return Vec::new();
    }
    let window = ((0.5 * fps).round() as usize).max(1);
    let threshold = (2.0 * median(diffs)).max(0.01);
    // Include the final partial window, where a closing motion burst may sit.
    let windows = (0..diffs.len())
        .step_by(window)
        .map(|lo| (lo, (lo + window).min(diffs.len())));
    let mut scored: Vec<(f32, usize)> = Vec::new();
    for (lo, hi) in windows {
        let slice = &diffs[lo..hi];
        let score = slice.iter().sum::<f32>() / slice.len() as f32;
        if score > threshold {
            // First maximum (ties: lower index). diffs[i] leads into frame i + 1.
            let mut best = 0usize;
            for (i, d) in slice.iter().enumerate() {
                if *d > slice[best] {
                    best = i;
                }
            }
            scored.push((score, (lo + best + 1).min(frames - 1)));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(quota);
    scored
}

fn add_motion_context(
    plans: &mut Vec<SamplePlan>,
    motion: &[(f32, usize)],
    fps: f64,
    frames: usize,
    budget: usize,
) {
    let offset = ((0.18 * fps).round() as usize).max(1);
    // A few strongest peaks are enough to reveal motion amplitude and settling.
    for &(_, peak) in motion.iter().take(3) {
        for frame in [
            peak,
            peak.saturating_sub(offset),
            peak.saturating_add(offset).min(frames - 1),
        ] {
            if plans.len() >= budget {
                return;
            }
            if let Err(at) = plans.binary_search_by_key(&frame, |p| p.frame) {
                plans.insert(
                    at,
                    SamplePlan {
                        frame,
                        time_seconds: round_ms(frame as f64 / fps),
                        kinds: vec![SampleKind::HighMotion],
                    },
                );
            }
        }
    }
}

fn stable_candidates(signals: &TemporalSignals, frames: usize, quota: usize) -> Vec<usize> {
    let mut holds: Vec<(usize, usize)> = Vec::new(); // inclusive [lo, hi]
    let events = &signals.events;
    match events.first() {
        None => holds.push((0, frames - 1)),
        Some(first) => {
            if first.start > 0 {
                holds.push((0, first.start - 1));
            }
            for pair in events.windows(2) {
                if pair[1].start > pair[0].end {
                    holds.push((pair[0].end, pair[1].start - 1));
                }
            }
            if let Some(last) = events.last() {
                if last.end < frames {
                    holds.push((last.end, frames - 1));
                }
            }
        }
    }
    // Longest first; ties: earlier.
    holds.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(a.0.cmp(&b.0)));
    holds.truncate(quota);
    let mut out: Vec<usize> = holds
        .into_iter()
        .map(|(lo, hi)| (lo + (hi - lo) / 2).min(frames - 1))
        .collect();
    out.sort_unstable();
    out
}

/// Merge candidates closer than `gap` frames into one plan.
fn merge(mut cands: Vec<(usize, SampleKind)>, gap: usize, fps: f64) -> Vec<SamplePlan> {
    cands.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut plans: Vec<SamplePlan> = Vec::new();
    let mut i = 0;
    while i < cands.len() {
        let anchor = cands[i].0;
        let mut j = i;
        while j < cands.len() && cands[j].0 - anchor < gap {
            j += 1;
        }
        let group = &cands[i..j];
        // Highest-priority kind wins; among equals the earlier frame.
        let mut keep = group[0];
        for c in group {
            if priority(c.1) > priority(keep.1) {
                keep = *c;
            }
        }
        let mut kinds: Vec<SampleKind> = group.iter().map(|c| c.1).collect();
        kinds.sort();
        kinds.dedup();
        plans.push(SamplePlan {
            frame: keep.0,
            time_seconds: round_ms(keep.0 as f64 / fps),
            kinds,
        });
        i = j;
    }
    plans
}

/// Smallest distance from `plans[i]` to a neighbouring plan.
fn neighbour_distance(plans: &[SamplePlan], i: usize) -> usize {
    let prev = i
        .checked_sub(1)
        .map(|p| plans[i].frame - plans[p].frame)
        .unwrap_or(usize::MAX);
    let next = plans
        .get(i + 1)
        .map(|n| n.frame - plans[i].frame)
        .unwrap_or(usize::MAX);
    prev.min(next)
}

fn trim_to_budget(plans: &mut Vec<SamplePlan>, budget: usize) {
    while plans.len() > budget {
        // Periodic-only samples go first, then the lowest-priority ones; within a
        // class the most crowded sample (smallest neighbour distance), then the
        // lower frame index.
        let victim = (0..plans.len())
            .min_by_key(|&i| {
                let best = plans[i]
                    .kinds
                    .iter()
                    .map(|k| priority(*k))
                    .max()
                    .unwrap_or(0);
                let periodic_only = plans[i].kinds.iter().all(|k| *k == SampleKind::Periodic);
                (
                    !periodic_only,
                    best,
                    neighbour_distance(plans, i),
                    plans[i].frame,
                )
            })
            .unwrap_or(0);
        plans.remove(victim);
    }
}

/// Choose at most `budget` key frames, time-ordered with strictly increasing,
/// unique frame indices. `fps` is the analysis fps; `frames` the analysis
/// frame count.
pub fn select_samples(
    signals: &TemporalSignals,
    fps: f64,
    frames: usize,
    budget: usize,
) -> Vec<SamplePlan> {
    if frames == 0 || budget == 0 || !fps.is_finite() || fps <= 0.0 {
        return Vec::new();
    }
    let quota = |share: f64| (share * budget as f64).ceil() as usize;
    let sc_quota = quota(0.35);
    let hm_quota = quota(0.10);
    let st_quota = quota(0.15).max(1);

    let scene = scene_change_candidates(signals, fps, frames, sc_quota);
    let motion = high_motion_candidates(signals, fps, frames, hm_quota);
    let stable = stable_candidates(signals, frames, st_quota);

    let others = scene.len() + motion.len() + stable.len();
    let mut periodic = budget.saturating_sub(others);
    if frames >= 4 {
        periodic = periodic.max(4);
    }
    let periodic = periodic.min(frames);

    let mut cands: Vec<(usize, SampleKind)> = Vec::new();
    cands.extend(scene.into_iter().map(|f| (f, SampleKind::SceneChange)));
    cands.extend(motion.iter().map(|&(_, f)| (f, SampleKind::HighMotion)));
    cands.extend(stable.into_iter().map(|f| (f, SampleKind::Stable)));
    for k in 0..periodic {
        let f =
            (((k as f64 + 0.5) * frames as f64 / periodic as f64).floor() as usize).min(frames - 1);
        cands.push((f, SampleKind::Periodic));
    }

    let gap = ((0.25 * fps).round() as usize).max(1);
    let mut plans = merge(cands, gap, fps);
    trim_to_budget(&mut plans, budget);
    add_motion_context(&mut plans, &motion, fps, frames, budget);
    plans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_curve() {
        assert_eq!(sample_budget(3.0), 13);
        assert_eq!(sample_budget(0.0), 12);
        assert_eq!(sample_budget(45.0), 30);
        assert_eq!(sample_budget(90.0), 30);
        assert_eq!(sample_budget(150.0), 32);
        assert_eq!(sample_budget(10_000.0), 40);
    }

    #[test]
    fn strongest_motion_peak_gets_separate_temporal_context() {
        let mut signals = TemporalSignals {
            diffs: vec![0.0; 99],
            ..Default::default()
        };
        signals.diffs[49] = 1.0; // Peak leads into frame 50.
        let plans = select_samples(&signals, 30.0, 100, 20);
        assert_eq!(plans, select_samples(&signals, 30.0, 100, 20));
        assert!(plans.len() <= 20);
        assert!(plans.windows(2).all(|pair| pair[0].frame < pair[1].frame));
        for frame in [45, 50, 55] {
            assert!(plans.iter().any(|plan| plan.frame == frame), "{plans:?}");
        }
    }
}
