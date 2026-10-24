//! Beat coverage: a music-led program must keep landing visual events on
//! beats right up to its final hit, with no long silent stretch, and that
//! final hit must be a downbeat followed by at most a bar of hold.

pub mod design;

use crate::program::{Component, ComponentBody, MotionProgram, Slot, Verb};

/// What `beat_coverage` measured.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatCoverage {
    /// Share of beats up to the final hit that carry an event (±1 frame).
    pub coverage: f64,
    /// Longest run of beats without any event.
    pub longest_gap_beats: usize,
    /// The final hit lands on a downbeat.
    pub ends_on_downbeat: bool,
    /// Beats between the final hit and the program end.
    pub hold_beats: f64,
}

impl BeatCoverage {
    pub const MIN_COVERAGE: f64 = 0.9;
    pub const MAX_GAP: usize = 1;
    /// The video ends before the beat after its final hit.
    pub const MAX_HOLD_BEATS: f64 = 1.0;

    pub fn passes(&self) -> bool {
        self.coverage >= Self::MIN_COVERAGE
            && self.longest_gap_beats <= Self::MAX_GAP
            && self.ends_on_downbeat
            && self.hold_beats <= Self::MAX_HOLD_BEATS
    }

    pub fn report(&self) -> String {
        format!(
            "beat_coverage {:.0}% (min {:.0}%), longest gap {} beats (max {}), final hit on downbeat: {}, hold {:.1} beats (max {})",
            self.coverage * 100.0,
            Self::MIN_COVERAGE * 100.0,
            self.longest_gap_beats,
            Self::MAX_GAP,
            self.ends_on_downbeat,
            self.hold_beats,
            Self::MAX_HOLD_BEATS
        )
    }
}

/// Event times (program seconds) of a beat-resolved program: component
/// starts and every timeline event.
pub fn event_times(p: &MotionProgram) -> Vec<f64> {
    let mut out = Vec::new();
    for c in &p.components {
        if !matches!(
            c.body,
            ComponentBody::Backdrop { .. } | ComponentBody::Hud { .. }
        ) {
            out.push(c.start);
        }
        match &c.body {
            ComponentBody::CodeBlock(b) => out.extend(b.steps.iter().map(|s| c.start + s.at)),
            ComponentBody::DeviceFrame(f) => {
                out.extend(f.resolved().steps.iter().map(|s| c.start + s.at))
            }
            ComponentBody::PhotoStack(ps) => {
                out.extend(crate::photo::step_events(ps, c.start, p.end_of(c)))
            }
            _ => {}
        }
        for e in &c.timeline {
            if let (Some(at), false) = (e.at, matches!(e.verb, Verb::Exit { .. })) {
                out.push(c.start + at);
            }
        }
    }
    out.extend(p.camera.iter().filter_map(|e| e.at));
    out.extend(p.transitions.iter().filter_map(|e| e.at));
    out.sort_by(f64::total_cmp);
    out
}

/// Longest settled hold with music, in beats.
pub const SETTLED_HOLD_BEATS: f64 = 2.0;
/// Longest settled hold without music, in seconds.
pub const SETTLED_HOLD_SECS: f64 = 1.0;
/// No settled hold inside the program's opening: an empty or frozen
/// opening is a mistake, not a read-hold.
pub const SETTLED_HOLD_NOT_BEFORE: f64 = 1.0;
/// Settled holds excuse at most this share of a program's beats (samples
/// without music) in all.
pub const SETTLED_HOLD_MAX_SHARE: f64 = 0.15;

/// One component's settled-hold window: once its last build step lands it
/// may stay still for up to `hold` seconds, once.
#[derive(Debug, Clone, PartialEq)]
pub struct SettledHold {
    pub component: String,
    /// When its last build step starts (program seconds).
    pub from: f64,
    /// When its build has landed (last step's motion, a sketch's strokes).
    pub landed: f64,
    /// Longest allowed stillness (seconds).
    pub hold: f64,
    /// While the component is on screen (its start to its end).
    pub visible: (f64, f64),
}

impl SettledHold {
    /// Latest moment the hold may run to.
    pub fn until(&self) -> f64 {
        (self.landed + self.hold).min(self.visible.1)
    }
}

/// When a sketch without beat-cut steps finishes drawing its strokes
/// (they stagger across about 60 % of its span), program seconds.
pub fn continuous_build_end(p: &MotionProgram, c: &Component) -> Option<f64> {
    let ComponentBody::Sketch(sk) = &c.body else {
        return None;
    };
    if c.timeline.iter().any(|e| matches!(e.verb, Verb::Advance)) {
        return None;
    }
    let length = p.end_of(c) - c.start;
    let estimate = c.start + 0.6 * length + 0.4f64.min(0.2 * length);
    let n = sk.elements.len();
    if sk.preset == crate::sketch::SketchPreset::Freeform && n >= 1 {
        // sketch.rs: element i starts at enter_at (0) + i·dt and its stroke
        // trims on over 0.65·tempo.
        let dt = (length * 0.6 / n as f64).clamp(0.08, 0.4);
        let last = c.start + (n - 1) as f64 * dt + 0.65 * p.tempo().factor();
        return Some(last.min(estimate));
    }
    Some(estimate)
}

/// The settled-hold window of every component that animates: a deliberate
/// read-hold after a build (a chart after its hero lands, a sketch after
/// its last stroke, a bento after its last tile, a lockup). `beat_len` is
/// the beat in seconds (music: the hold is [`SETTLED_HOLD_BEATS`]) or
/// `None` (the hold is [`SETTLED_HOLD_SECS`]). Grounds, fields and HUD
/// never build; a component on screen from 0 s with no timeline event or
/// step never animated, so it gets no hold.
pub fn settled_holds(p: &MotionProgram, beat_len: Option<f64>) -> Vec<SettledHold> {
    let tempo = p.tempo();
    let hold = beat_len.map_or(SETTLED_HOLD_SECS, |b| b * SETTLED_HOLD_BEATS);
    let mut out = Vec::new();
    for c in &p.components {
        if matches!(
            c.body,
            ComponentBody::Backdrop { .. }
                | ComponentBody::Hud { .. }
                | ComponentBody::Field { .. }
        ) {
            continue;
        }
        // (time a build step starts, time it lands), program seconds.
        let mut steps: Vec<(f64, f64)> = Vec::new();
        if c.start > 0.0 {
            steps.push((c.start, c.start));
        }
        match &c.body {
            ComponentBody::CodeBlock(b) => {
                steps.extend(b.steps.iter().map(|s| (c.start + s.at, c.start + s.at)))
            }
            ComponentBody::DeviceFrame(f) => steps.extend(
                f.resolved()
                    .steps
                    .iter()
                    .map(|s| (c.start + s.at, c.start + s.at)),
            ),
            ComponentBody::PhotoStack(ps) => steps.extend(
                crate::photo::step_events(ps, c.start, p.end_of(c))
                    .into_iter()
                    .map(|t| (t, t)),
            ),
            _ => {}
        }
        for e in &c.timeline {
            if let (Some(at), false) = (e.at, matches!(e.verb, Verb::Exit { .. })) {
                let t = c.start + at;
                steps.push((t, t + crate::taste::duration(&e.verb, tempo)));
            }
        }
        // A sketch drawing its strokes animates, even from 0 s.
        if let Some(end) = continuous_build_end(p, c) {
            steps.push((end, end));
        }
        let Some(&(from, _)) = steps.iter().max_by(|a, b| a.0.total_cmp(&b.0)) else {
            continue;
        };
        let landed = steps.iter().map(|s| s.1).fold(from, f64::max);
        out.push(SettledHold {
            component: c.id.clone(),
            from,
            landed,
            hold,
            visible: (c.start, p.end_of(c)),
        });
    }
    out
}

/// Drops the runs of `flagged` times (each a time in `samples` at which
/// nothing visibly changed or no event landed) that are a settled hold: a
/// run of consecutive samples that starts at or after
/// [`SETTLED_HOLD_NOT_BEFORE`], lasts at most the hold (measured from the
/// sample before it) and lies inside one on-screen component's window:
/// after its last build step starts and by the time its build landed plus
/// the hold. Each component's hold excuses at most one run, and all holds
/// together at most [`SETTLED_HOLD_MAX_SHARE`] of the samples. Returns the
/// times still flagged.
pub fn excuse_settled_holds(holds: &[SettledHold], flagged: &[f64], samples: &[f64]) -> Vec<f64> {
    const TOL: f64 = 0.1;
    let is_flagged = |t: f64| flagged.iter().any(|f| (f - t).abs() < 1e-9);
    // Runs of consecutive flagged samples, with the sample before each.
    let mut runs: Vec<(f64, Vec<f64>)> = Vec::new();
    let mut prev: Option<(f64, bool)> = None;
    for &t in samples {
        let f = is_flagged(t);
        if f {
            match prev {
                Some((_, true)) => {
                    if let Some(run) = runs.last_mut() {
                        run.1.push(t);
                    }
                }
                Some((before, false)) => runs.push((before, vec![t])),
                None => runs.push((t, vec![t])),
            }
        }
        prev = Some((t, f));
    }
    // Flagged times that are not samples stay flagged.
    let mut out: Vec<f64> = flagged
        .iter()
        .copied()
        .filter(|f| !samples.iter().any(|t| (f - t).abs() < 1e-9))
        .collect();
    let mut used = vec![false; holds.len()];
    // Program-wide cap: holds excuse at most this many samples in all
    // (earliest holds win), so a chain of holds cannot freeze the video.
    let cap = (SETTLED_HOLD_MAX_SHARE * samples.len() as f64).floor() as usize;
    let mut excused = 0;
    for (before, run) in runs {
        let (first, last) = (run[0], run[run.len() - 1]);
        let pick = holds.iter().enumerate().position(|(i, h)| {
            !used[i]
                && before >= SETTLED_HOLD_NOT_BEFORE - 1e-9
                && last - before <= h.hold + TOL
                && first > h.from + 1e-9
                && last <= h.until() + TOL
                && first >= h.visible.0
                && last < h.visible.1
        });
        match pick {
            Some(i) if excused + run.len() <= cap => {
                used[i] = true;
                excused += run.len();
            }
            _ => out.extend(run),
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

/// Coverage of `beats` (program seconds) by the program's events, up to its
/// final hit (the last non-exit event). A settled hold after a component's
/// last build step ([`settled_holds`]) counts as covered.
pub fn beat_coverage(p: &MotionProgram, beats: &[f64], downbeats: &[f64]) -> BeatCoverage {
    let frame = 1.0 / p.meta.fps.value() as f64;
    let events = event_times(p);
    let last = events.last().copied().unwrap_or(0.0);
    // An event "hits" a beat from the cut lead before it to a frame after.
    let lead = crate::taste::CUT_LEAD_FRAMES * frame;
    let hit = |t: f64| {
        events
            .iter()
            .any(|e| *e >= t - lead - frame - 1e-6 && *e <= t + frame + 1e-6)
    };
    let span: Vec<f64> = beats
        .iter()
        .copied()
        // Every beat while the video plays, not just up to the last event.
        .filter(|b| *b < p.meta.duration - frame)
        .collect();
    let beat_len = if beats.len() > 1 {
        (beats[beats.len() - 1] - beats[0]) / (beats.len() - 1) as f64
    } else {
        1.0
    };
    // A sketch drawing its strokes changes on every beat while it draws.
    let drawing: Vec<(f64, f64)> = p
        .components
        .iter()
        .filter_map(|c| continuous_build_end(p, c).map(|e| (c.start, e)))
        .collect();
    let missed: Vec<f64> = span
        .iter()
        .copied()
        .filter(|b| !hit(*b) && !drawing.iter().any(|(s, e)| *b > *s && *b <= *e))
        .collect();
    let missed = excuse_settled_holds(&settled_holds(p, Some(beat_len)), &missed, &span);
    let covered: Vec<bool> = span
        .iter()
        .map(|b| !missed.iter().any(|m| (m - b).abs() < 1e-9))
        .collect();
    let coverage = if span.is_empty() {
        0.0
    } else {
        covered.iter().filter(|c| **c).count() as f64 / span.len() as f64
    };
    let mut longest = 0;
    let mut run = 0;
    for c in &covered {
        run = if *c { 0 } else { run + 1 };
        longest = longest.max(run);
    }
    BeatCoverage {
        coverage,
        longest_gap_beats: longest,
        ends_on_downbeat: downbeats
            .iter()
            .any(|d| last >= d - lead - frame - 1e-6 && last <= d + frame + 1e-6),
        // Measured from the final beat (the hit leads it by the cut lead).
        hold_beats: (p.meta.duration - last - lead) / beat_len,
    }
}

/// One overlap between two components that are on screen together.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlap {
    pub a: String,
    pub b: String,
    /// Seconds of the worst sample.
    pub at: f64,
    /// Overlap area as a share of the smaller rect.
    pub share: f32,
}

fn component_slot(c: &Component) -> Option<Slot> {
    match &c.body {
        ComponentBody::Shape { slot, .. }
        | ComponentBody::Title { slot, .. }
        | ComponentBody::KineticText { slot, .. }
        | ComponentBody::Chart { slot, .. }
        | ComponentBody::Diagram { slot, .. } => Some(*slot),
        ComponentBody::CodeBlock(b) => Some(b.slot),
        ComponentBody::DeviceFrame(f) => Some(f.slot),
        ComponentBody::Bento(b) => Some(b.slot),
        ComponentBody::PhotoStack(ps) => Some(ps.slot),
        ComponentBody::Sketch(sk) => Some(sk.slot),
        _ => None,
    }
}

pub(crate) fn complementary_slots(p: &MotionProgram, a_id: &str, b_id: &str) -> bool {
    let ca = p.components.iter().find(|c| c.id == a_id);
    let cb = p.components.iter().find(|c| c.id == b_id);
    let (Some(sa), Some(sb)) = (ca.and_then(component_slot), cb.and_then(component_slot)) else {
        return false;
    };
    matches!(
        (sa, sb),
        (Slot::Left, Slot::Right)
            | (Slot::Right, Slot::Left)
            | (Slot::Top, Slot::Bottom)
            | (Slot::Bottom, Slot::Top)
            | (Slot::Lead, Slot::Trail)
            | (Slot::Trail, Slot::Lead)
    )
}

/// True when `a` is declared `over` `b` or `b` over `a` (either direction).
pub fn layered_pair(p: &MotionProgram, a: &str, b: &str) -> bool {
    p.components.iter().any(|c| {
        c.over
            .as_deref()
            .is_some_and(|t| (c.id == a && t == b) || (c.id == b && t == a))
    })
}

/// `component_overlap`: content components (not grounds, fields or HUD)
/// whose on-screen rects overlap by more than 2 % of the smaller one,
/// sampled every 0.25 s from the evaluated timeline (motion included,
/// near-invisible layers and full-frame grounds ignored). A shape that
/// anchors to a component may sit on it, and a component declared `over`
/// another may sit on exactly that one (intentional layering).
pub fn component_overlap(p: &MotionProgram, scene: &motion_core::MotionProject) -> Vec<Overlap> {
    let frame_area = (scene.canvas.width * scene.canvas.height) as f32;
    let owner = |sid: &str| {
        p.components
            .iter()
            .find(|c| sid == c.id || sid.starts_with(&format!("{}.", c.id)))
    };
    let anchored = |a: &str, b: &str| {
        p.components.iter().any(|c| {
            // Only a ring drawn around its anchor may overlap it.
            matches!(&c.body, ComponentBody::Shape { anchor: Some(x), mark, .. }
                if matches!(mark.unwrap_or_default(), crate::program::Mark::Ring)
                    && ((c.id == a && x == b) || (c.id == b && x == a)))
        })
    };
    let layered = |a: &str, b: &str| layered_pair(p, a, b);
    let fps = scene.canvas.fps.max(1);
    let step = (fps / 4).max(1);
    let mut worst: Vec<Overlap> = Vec::new();
    let mut f = 0;
    while f < scene.frame_count() {
        let Ok(rf) = motion_core::evaluate_frame(scene, f) else {
            f += step;
            continue;
        };
        let mut rects: Vec<(String, [f32; 4])> = Vec::new();
        for l in &rf.layers {
            let Some(c) = l.scene.and_then(owner) else {
                continue;
            };
            if matches!(
                c.body,
                ComponentBody::Backdrop { .. }
                    | ComponentBody::Field { .. }
                    | ComponentBody::Hud { .. }
            ) || l.opacity < 0.05
            {
                continue;
            }
            let pts = [
                l.transform.apply(0.0, 0.0),
                l.transform.apply(l.width, 0.0),
                l.transform.apply(0.0, l.height),
                l.transform.apply(l.width, l.height),
            ];
            let r = pts
                .iter()
                .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |q, (x, y)| {
                    [q[0].min(*x), q[1].min(*y), q[2].max(*x), q[3].max(*y)]
                });
            let area = (r[2] - r[0]) * (r[3] - r[1]);
            if area <= 1.0 || area >= frame_area * 0.9 {
                continue;
            }
            match rects.iter_mut().find(|(id, _)| *id == c.id) {
                Some((_, q)) => {
                    *q = [
                        q[0].min(r[0]),
                        q[1].min(r[1]),
                        q[2].max(r[2]),
                        q[3].max(r[3]),
                    ]
                }
                None => rects.push((c.id.clone(), r)),
            }
        }
        // Shots that only share their cut frame are not an overlap.
        let t_now = f as f64 / fps as f64;
        let at_cut = |id: &str| {
            p.components.iter().find(|c| c.id == id).is_some_and(|c| {
                (t_now - c.start).abs() < 1.5 / fps as f64
                    || (t_now - p.end_of(c)).abs() < 1.5 / fps as f64
            })
        };
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let ((ca, a), (cb, b)) = (&rects[i], &rects[j]);
                if anchored(ca, cb)
                    || layered(ca, cb)
                    || at_cut(ca)
                    || at_cut(cb)
                    || complementary_slots(p, ca, cb)
                {
                    continue;
                }
                let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
                let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
                let area = |r: &[f32; 4]| (r[2] - r[0]) * (r[3] - r[1]);
                let share = w * h / area(a).min(area(b)).max(1.0);
                if share > 0.02 {
                    let at = f as f64 / fps as f64;
                    match worst.iter_mut().find(|o| o.a == *ca && o.b == *cb) {
                        Some(o) if share > o.share => {
                            o.share = share;
                            o.at = at;
                        }
                        Some(_) => {}
                        None => worst.push(Overlap {
                            a: ca.clone(),
                            b: cb.clone(),
                            at,
                            share,
                        }),
                    }
                }
            }
        }
        f += step;
    }
    worst
}

/// Content components (not grounds, fields or HUD) per sampled frame:
/// `(component id, canvas rect)`.
fn content_rects(
    p: &MotionProgram,
    rf: &motion_core::timeline::ResolvedFrame<'_>,
    frame_area: f32,
    with_fields: bool,
) -> Vec<(String, [f32; 4])> {
    let owner = |sid: &str| {
        p.components
            .iter()
            .find(|c| sid == c.id || sid.starts_with(&format!("{}.", c.id)))
    };
    let mut rects: Vec<(String, [f32; 4])> = Vec::new();
    for l in &rf.layers {
        let Some(c) = l.scene.and_then(owner) else {
            continue;
        };
        let field = matches!(c.body, ComponentBody::Field { .. });
        // A device or code_block group is content however far the camera is in.
        let device = matches!(
            c.body,
            ComponentBody::DeviceFrame(_) | ComponentBody::CodeBlock(_)
        );
        if matches!(
            c.body,
            ComponentBody::Backdrop { .. } | ComponentBody::Hud { .. }
        ) || (field && !with_fields)
            || l.opacity < 0.05
        {
            continue;
        }
        let pts = [
            l.transform.apply(0.0, 0.0),
            l.transform.apply(l.width, 0.0),
            l.transform.apply(0.0, l.height),
            l.transform.apply(l.width, l.height),
        ];
        let r = pts
            .iter()
            .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |q, (x, y)| {
                [q[0].min(*x), q[1].min(*y), q[2].max(*x), q[3].max(*y)]
            });
        let area = (r[2] - r[0]) * (r[3] - r[1]);
        // A field fills the frame by design: a hero, not a ground.
        if area <= 1.0 || (area >= frame_area * 0.9 && !field && !device) {
            continue;
        }
        match rects.iter_mut().find(|(id, _)| *id == c.id) {
            Some((_, q)) => {
                *q = [
                    q[0].min(r[0]),
                    q[1].min(r[1]),
                    q[2].max(r[2]),
                    q[3].max(r[3]),
                ]
            }
            None => rects.push((c.id.clone(), r)),
        }
    }
    rects
}

/// Hero area on a frame (share of the frame, after the camera): the
/// largest content element, clipped to the frame; a field counts only when
/// it is the shot's hero (no other content of any size).
fn hero_area(p: &MotionProgram, scene: &motion_core::MotionProject, t: f64) -> Option<f32> {
    let (w, h) = (scene.canvas.width as f32, scene.canvas.height as f32);
    let f = frame_at(scene, t);
    let rf = motion_core::evaluate_frame(scene, f).ok()?;
    let clip = |r: &[f32; 4]| {
        let (x0, y0, x1, y1) = (r[0].max(0.0), r[1].max(0.0), r[2].min(w), r[3].min(h));
        ((x1 - x0).max(0.0) * (y1 - y0).max(0.0)) / (w * h)
    };
    let content = content_rects(p, &rf, w * h, false);
    // Type, photo-stack, and bento heroes are judged by extent (a giant word or
    // geometric layout is tall or wide, not area-filling); every other hero by area.
    let by_extent = |id: &str| {
        p.components.iter().any(|c| {
            c.id == id
                && matches!(
                    c.body,
                    ComponentBody::KineticText { .. }
                        | ComponentBody::Title { .. }
                        | ComponentBody::Shape { .. }
                        | ComponentBody::Lockup(_)
                        | ComponentBody::PhotoStack(_)
                        | ComponentBody::Bento(_)
                        | ComponentBody::Sketch(_)
                        | ComponentBody::CodeBlock(_)
                        | ComponentBody::Chart { .. }
                )
        })
    };
    let mut best = content
        .iter()
        .map(|(id, r)| {
            let is_portrait = h > w;
            let is_lockup = p
                .components
                .iter()
                .any(|c| c.id == *id && matches!(c.body, ComponentBody::Lockup(_)));
            if is_portrait && is_lockup {
                if (r[2] - r[0]) / w >= 0.55 && (r[3] - r[1]) / h >= 0.11 {
                    1.0
                } else {
                    clip(r)
                }
            } else if by_extent(id) {
                let ext = ((r[2] - r[0]) / w).max((r[3] - r[1]) / h);
                if ext >= 0.55 {
                    1.0
                } else {
                    clip(r)
                }
            } else {
                clip(r)
            }
        })
        .fold(0.0f32, f32::max);
    // Split-screen compositions (e.g. Slot::Left + Slot::Right) span the frame together.
    for (i, (id_a, r_a)) in content.iter().enumerate() {
        for (id_b, r_b) in content.iter().skip(i + 1) {
            if complementary_slots(p, id_a, id_b) {
                let u = [
                    r_a[0].min(r_b[0]),
                    r_a[1].min(r_b[1]),
                    r_a[2].max(r_b[2]),
                    r_a[3].max(r_b[3]),
                ];
                let ext = ((u[2] - u[0]) / w).max((u[3] - u[1]) / h);
                if ext >= 0.55 {
                    best = best.max(1.0);
                } else {
                    best = best.max(clip(&u));
                }
            }
        }
    }
    // Builds (diagrams, stepped charts, bento, sketch) are judged on their planned layout:
    // nodes and bars still hidden at this beat count toward the hero.
    for c in &p.components {
        if !matches!(
            c.body,
            ComponentBody::Diagram { .. }
                | ComponentBody::Chart { .. }
                | ComponentBody::Bento(_)
                | ComponentBody::Sketch(_)
        ) || c.start > t
            || t >= p.end_of(c)
        {
            continue;
        }
        let mut r: Option<[f32; 4]> = None;
        for sc in scene
            .scenes
            .iter()
            .filter(|sc| sc.id == c.id || sc.id.starts_with(&format!("{}.", c.id)))
        {
            for l in &sc.layers {
                let (lw, lh) = (l.width * l.scale_x, l.height * l.scale_y);
                if lw * lh >= w * h * 0.9 || lw * lh <= 1.0 {
                    continue;
                }
                let (x0, y0) = (l.x - l.anchor_x * lw, l.y - l.anchor_y * lh);
                let b = [x0, y0, x0 + lw, y0 + lh];
                r = Some(r.map_or(b, |q| {
                    [
                        q[0].min(b[0]),
                        q[1].min(b[1]),
                        q[2].max(b[2]),
                        q[3].max(b[3]),
                    ]
                }));
            }
        }
        if let Some(r) = r {
            best = best.max(clip(&r));
        }
    }
    if best >= 0.05 {
        return Some(best);
    }
    let field = content_rects(p, &rf, w * h, true)
        .iter()
        .filter(|(id, _)| !content.iter().any(|(c, _)| c == id))
        .map(|(_, r)| clip(r))
        .fold(0.0f32, f32::max);
    Some(best.max(field))
}

/// `fill`: on each beat the hero covers at least `MIN_FILL` of the frame
/// area after the camera. Returns the share of beats that pass and the
/// failing beat times.
pub const MIN_FILL: f32 = 0.45;
pub fn fill(
    p: &MotionProgram,
    scene: &motion_core::MotionProject,
    times: &[f64],
) -> (f64, Vec<f64>) {
    // A shot mid-transition (whip, wipe) is not judged.
    let moving = |t: f64| {
        p.transitions
            .iter()
            .any(|e| e.at.is_some_and(|a| (a - t).abs() < 0.25))
    };
    let bad: Vec<f64> = times
        .iter()
        .copied()
        .filter(|t| {
            let a = hero_area(p, scene, *t);
            !moving(*t) && a.is_some_and(|a| a < MIN_FILL)
        })
        .collect();
    let share = if times.is_empty() {
        1.0
    } else {
        1.0 - bad.len() as f64 / times.len() as f64
    };
    (share, bad)
}

/// The frame a sampled time lands on (the frame the QA gates evaluate).
fn frame_at(scene: &motion_core::MotionProject, t: f64) -> u32 {
    ((t * scene.canvas.fps as f64).round() as u32).min(scene.frame_count().saturating_sub(1))
}

/// One near-blank sample: when, on which frame, and which content
/// components are on screen then (program order; backdrops, HUD and fully
/// faded components do not count).
#[derive(Debug, Clone, PartialEq)]
pub struct BlankAt {
    pub t: f64,
    pub frame: u32,
    pub on_screen: Vec<String>,
}

impl BlankAt {
    /// `2.38 s (f57; on screen: title_a, sketch_b)`; `on screen: none` when
    /// no content component is visible.
    pub fn describe(&self) -> String {
        let t = (self.t * 100.0).round() / 100.0;
        let ids = if self.on_screen.is_empty() {
            "none".to_string()
        } else {
            self.on_screen.join(", ")
        };
        format!("{t} s (f{}; on screen: {ids})", self.frame)
    }
}

/// Near-blank beats, measured on content (grounds, dot grids and HUD do
/// not count as ink): the hero covers under 3 % of the frame. Each carries
/// its frame number and the components on screen, for the failure message.
pub fn near_blank(
    p: &MotionProgram,
    scene: &motion_core::MotionProject,
    times: &[f64],
) -> Vec<BlankAt> {
    let area = scene.canvas.width as f32 * scene.canvas.height as f32;
    times
        .iter()
        .copied()
        .filter(|t| hero_area(p, scene, *t).is_some_and(|a| a < 0.03))
        .map(|t| {
            let frame = frame_at(scene, t);
            let visible: Vec<String> = motion_core::evaluate_frame(scene, frame)
                .map(|rf| {
                    content_rects(p, &rf, area, true)
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect()
                })
                .unwrap_or_default();
            let on_screen = p
                .components
                .iter()
                .filter(|c| visible.contains(&c.id))
                .map(|c| c.id.clone())
                .collect();
            BlankAt {
                t,
                frame,
                on_screen,
            }
        })
        .collect()
}

/// `text_contrast`: every text layer against the ground it sits on (the
/// highest full-frame layer below it at its start): WCAG ratio ≥ 4.5, or
/// ≥ 3.0 for large type (≥ 5 % of the short side). Returns failures as
/// `(layer id, ratio)`.
pub fn text_contrast(scene: &motion_core::MotionProject) -> Vec<(String, f32)> {
    let short = (scene.canvas.width as f32).min(scene.canvas.height as f32);
    let mut out = Vec::new();
    for (s, l, style) in text_layers(scene) {
        let ground = backing(scene, s, l);
        let ratio = crate::validate::contrast(style.color, ground);
        let need = if style.font_size >= short * 0.05 {
            3.0
        } else {
            4.5
        };
        if ratio < need && style.color.a > 200 {
            out.push((l.id.clone(), ratio));
        }
    }
    out
}

fn text_layers(
    scene: &motion_core::MotionProject,
) -> Vec<(
    &motion_core::scene::Scene,
    &motion_core::scene::Layer,
    &motion_core::scene::TextStyle,
)> {
    use motion_core::scene::LayerKind;
    let mut out = Vec::new();
    for s in &scene.scenes {
        for l in &s.layers {
            if let LayerKind::Text(style) = &l.kind {
                out.push((s, l, style));
            }
        }
    }
    out
}

/// The colour behind a text layer at its start: the highest filled layer
/// below it (same or other active scene) whose box contains its centre.
pub fn backing(
    scene: &motion_core::MotionProject,
    s: &motion_core::scene::Scene,
    l: &motion_core::scene::Layer,
) -> motion_core::scene::Color {
    use motion_core::scene::{Fill, LayerKind};
    let t = s.start_seconds + 0.01;
    let c = [
        l.x + (0.5 - l.anchor_x) * l.width,
        l.y + (0.5 - l.anchor_y) * l.height,
    ];
    let mut best = (i32::MIN, scene.canvas.background);
    for g in &scene.scenes {
        if !(g.start_seconds <= t && t < g.end_seconds()) {
            continue;
        }
        for gl in &g.layers {
            if std::ptr::eq(gl, l) || gl.z_index > l.z_index || !gl.visible {
                continue;
            }
            let (x0, y0) = (
                gl.x - gl.anchor_x * gl.width,
                gl.y - gl.anchor_y * gl.height,
            );
            if !(c[0] >= x0 && c[0] <= x0 + gl.width && c[1] >= y0 && c[1] <= y0 + gl.height) {
                continue;
            }
            let col = match &gl.kind {
                LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => {
                    Some(*fill)
                }
                LayerKind::Path {
                    fill: Some(Fill::Solid(c)),
                    ..
                } => Some(*c),
                LayerKind::Path {
                    fill: Some(Fill::Gradient(g)),
                    ..
                } => g.stops().first().map(|s| s.color),
                _ => None,
            };
            if let Some(col) = col.filter(|c| c.a > 200) {
                if gl.z_index >= best.0 {
                    best = (gl.z_index, col);
                }
            }
        }
    }
    best.1
}

/// `prompt_echo`: on-screen words (4+ letters) copied from the brief,
/// unless the brief quoted them.
pub fn prompt_echo(p: &MotionProgram, scene: &motion_core::MotionProject) -> Vec<String> {
    use motion_core::scene::LayerKind;
    let Some(brief) = &p.meta.brief else {
        return Vec::new();
    };
    let lower = brief.to_lowercase();
    let quoted: Vec<String> = lower
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, q)| q.to_string())
        .collect();
    let words = |t: &str| -> Vec<String> {
        t.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 4)
            .map(String::from)
            .collect()
    };
    let brief_words = words(&lower);
    // Names, amounts and items a UI flow takes from the brief are data,
    // like quoted text.
    let fielded: Vec<String> = p
        .components
        .iter()
        .filter_map(|c| match &c.body {
            ComponentBody::DeviceFrame(f) => f.flow.as_ref(),
            _ => None,
        })
        .flat_map(|f| {
            [
                &f.app,
                &f.actor,
                &f.counterparty,
                &f.amount,
                &f.item,
                &f.note,
            ]
            .into_iter()
            .flatten()
            .flat_map(|t| words(t))
            .collect::<Vec<_>>()
        })
        .collect();
    let mut out = Vec::new();
    for s in &scene.scenes {
        for l in &s.layers {
            let LayerKind::Text(style) = &l.kind else {
                continue;
            };
            for wd in words(&style.text) {
                if brief_words.contains(&wd)
                    && !quoted.iter().any(|q| q.contains(&wd))
                    && !fielded.contains(&wd)
                    && !out.contains(&wd)
                {
                    out.push(wd);
                }
            }
        }
    }
    out
}

/// `meta_strings`: technical or self-referential text that must never be
/// on screen (frame rates, tempos, the program's own name, the engine).
pub fn meta_strings(p: &MotionProgram, scene: &motion_core::MotionProject) -> Vec<String> {
    use motion_core::scene::LayerKind;
    let name_words: Vec<String> = p
        .meta
        .name
        .split('_')
        .filter(|w| w.len() >= 4)
        .map(str::to_lowercase)
        .collect();
    let mut out = Vec::new();
    for s in &scene.scenes {
        for l in &s.layers {
            let LayerKind::Text(style) = &l.kind else {
                continue;
            };
            let t = style.text.to_lowercase();
            let toks: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).collect();
            let tech = toks
                .iter()
                .any(|w| matches!(*w, "fps" | "bpm" | "motionengine"));
            let own =
                name_words.len() >= 2 && name_words.iter().all(|w| toks.contains(&w.as_str()));
            if (tech || own) && !out.contains(&style.text) {
                out.push(style.text.clone());
            }
        }
    }
    out
}

/// The scene owning a non-ground filled box directly behind a text layer
/// (`None` when the text sits on a full-frame ground).
pub fn backing_owner(
    scene: &motion_core::MotionProject,
    s: &motion_core::scene::Scene,
    l: &motion_core::scene::Layer,
) -> Option<String> {
    use motion_core::scene::{Fill, LayerKind};
    let frame = (scene.canvas.width * scene.canvas.height) as f32;
    let t = s.start_seconds + 0.01;
    let c = [
        l.x + (0.5 - l.anchor_x) * l.width,
        l.y + (0.5 - l.anchor_y) * l.height,
    ];
    let mut best: Option<(i32, String)> = None;
    for g in &scene.scenes {
        if !(g.start_seconds <= t && t < g.end_seconds()) {
            continue;
        }
        for gl in &g.layers {
            if std::ptr::eq(gl, l) || gl.z_index > l.z_index || !gl.visible {
                continue;
            }
            let filled = match &gl.kind {
                LayerKind::Rectangle { .. } | LayerKind::RoundedRectangle { .. } => true,
                LayerKind::Path {
                    fill: Some(Fill::Solid(c)),
                    ..
                } => c.a > 200,
                _ => false,
            };
            if !filled || gl.width * gl.height >= frame * 0.9 {
                continue;
            }
            let (x0, y0) = (
                gl.x - gl.anchor_x * gl.width,
                gl.y - gl.anchor_y * gl.height,
            );
            if c[0] >= x0
                && c[0] <= x0 + gl.width
                && c[1] >= y0
                && c[1] <= y0 + gl.height
                && best.as_ref().is_none_or(|(z, _)| gl.z_index >= *z)
            {
                best = Some((gl.z_index, g.id.clone()));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// Stroke-only paths (hairlines, axes, connectors) below 3:1 on their
/// backing: `(layer id, ratio)`.
pub fn hairlines(scene: &motion_core::MotionProject) -> Vec<(String, f32)> {
    use motion_core::scene::LayerKind;
    let mut out = Vec::new();
    for s in &scene.scenes {
        for l in &s.layers {
            if let LayerKind::Path {
                fill: None,
                stroke: Some(st),
                ..
            } = &l.kind
            {
                let back = backing(scene, s, l);
                let r = crate::validate::contrast(st.color, back);
                if r < 3.0 || st.color.a < 200 {
                    out.push((l.id.clone(), r));
                }
            }
        }
    }
    out
}
