//! Layout follows content (owner: "layout must never anticipate").
//!
//! A component is laid out once per step from the elements visible at that
//! step; [`follow`] turns the per-step layouts into one scene: every layer
//! starts where the layout of its first step puts it and springs to its
//! place in each later layout at the step's time (FLIP). Containers grow,
//! siblings are pushed, the composition re-centres; an arriving element
//! makes its own room, nothing waits for it.
//!
//! Shared API: any component that reveals content in steps (lists, cards,
//! UI states, charts, diagrams, kinetic) lowers its final scene plus the
//! scene of each earlier step and calls [`follow`].

use motion_core::scene::{Layer, LayerKind, MotionOp, Scene};

use crate::lower::Track;
use crate::taste::{self, Spring};

/// Seconds a re-layout takes (one settle spring).
pub const RELAYOUT: f64 = 0.35;
/// How far ahead of an arrival its siblings start to move.
pub const LEAD: f64 = 0.2;

fn find<'a>(s: &'a Scene, id: &str) -> Option<&'a Layer> {
    s.layers.iter().find(|l| l.id == id)
}

fn path_d(l: &Layer) -> Option<&str> {
    match &l.kind {
        LayerKind::Path { d, .. } => Some(d.as_str()),
        _ => None,
    }
}

/// `fin`: the final layout (all content), with its motions. `steps`: the
/// earlier layouts in order, each with the time it gives way to the next.
/// Only top-level positions and path shapes are taken from the steps.
/// `skip`: layers that place themselves (e.g. a token riding the edges).
pub fn follow(fin: &mut Scene, steps: &[(f64, Scene)], skip: &dyn Fn(&str) -> bool) {
    if steps.is_empty() {
        return;
    }
    let mut t = Track::default();
    let n = steps.len();
    for li in 0..fin.layers.len() {
        let id = fin.layers[li].id.clone();
        if skip(&id) {
            continue;
        }
        // Layouts in time order: steps, then the final one.
        let at = |k: usize| -> Option<&Layer> {
            if k < n {
                find(&steps[k].1, &id)
            } else {
                Some(&fin.layers[li])
            }
        };
        let Some(first) = (0..=n).find(|&k| at(k).is_some()) else {
            continue;
        };
        let snap: Vec<Option<([f32; 2], Option<String>)>> = (0..=n)
            .map(|k| at(k).map(|l| ([l.x, l.y], path_d(l).map(String::from))))
            .collect();
        let (home, home_d) = snap[first].clone().expect("present");
        // The offset the layer's own moves leave at time `s`.
        let base = |s: f64| -> [f32; 2] {
            fin.motions
                .iter()
                .filter(|m| m.target == id && m.start + m.duration <= s + 1e-6)
                .filter_map(|m| match &m.op {
                    MotionOp::Move { to, .. } => Some((m.start + m.duration, *to)),
                    _ => None,
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map_or([0.0, 0.0], |(_, to)| to)
        };
        let mut prev = snap[first].clone().expect("present");
        for (k, cur) in snap.iter().enumerate().skip(first + 1) {
            let Some(cur) = cur else { continue };
            // Siblings start making room just before the arrival and
            // settle as it lands, so nothing lands on a moving sibling.
            let when = (steps[k - 1].0 - LEAD).max(0.0);
            // After any move of the layer's own that is still running.
            let when = fin
                .motions
                .iter()
                .filter(|m| {
                    m.target == id
                        && matches!(m.op, MotionOp::Move { .. })
                        && m.start < when + RELAYOUT
                        && m.start + m.duration > when
                })
                .map(|m| m.start + m.duration)
                .fold(when, f64::max);
            let span = RELAYOUT.min(fin.duration_seconds - when);
            if span < 0.02 {
                prev = cur.clone();
                continue;
            }
            let b = base(when);
            let off = |p: [f32; 2]| [b[0] + p[0] - home[0], b[1] + p[1] - home[1]];
            if (cur.0[0] - prev.0[0]).abs() > 0.5 || (cur.0[1] - prev.0[1]).abs() > 0.5 {
                t.push(
                    &id,
                    when,
                    span,
                    taste::Shape::Spring(Spring::Settle),
                    MotionOp::Move {
                        from: off(prev.0),
                        to: off(cur.0),
                        via: None,
                    },
                );
            }
            if let (Some(a), Some(b)) = (&prev.1, &cur.1) {
                if a != b {
                    t.push(
                        &id,
                        when,
                        span,
                        taste::Shape::Spring(Spring::Settle),
                        MotionOp::Morph { to: b.clone() },
                    );
                }
            }
            prev = cur.clone();
        }
        let l = &mut fin.layers[li];
        l.x = home[0];
        l.y = home[1];
        if let (Some(d), LayerKind::Path { d: base_d, .. }) = (home_d, &mut l.kind) {
            *base_d = d;
        }
    }
    for (i, mut m) in t.motions.into_iter().enumerate() {
        m.id = Some(format!("{}.flip{i}", m.target));
        fin.motions.push(m);
    }
}

/// The offset the last re-layout leaves on a layer (zero without one).
pub fn final_offset(s: &Scene, id: &str) -> [f32; 2] {
    s.motions
        .iter()
        .filter(|m| m.target == id && m.id.as_deref().is_some_and(|i| i.contains(".flip")))
        .filter_map(|m| match &m.op {
            MotionOp::Move { to, .. } => Some((m.start, *to)),
            _ => None,
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map_or([0.0, 0.0], |(_, to)| to)
}
