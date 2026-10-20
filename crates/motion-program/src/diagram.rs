//! Diagram: nodes and edges built step by step.
//!
//! Layout is automatic from the node count and aspect (a row, two rows, a
//! column or two columns inside the safe area; `rail` puts round nodes on
//! one line). Node 0 is shown at the start; each `advance` reveals the next
//! node *on* its time: the incoming edges draw just before (anticipation)
//! and one accent dot rides the tip of the drawing edge, so a single element
//! carries the eye through the whole build. Nothing is placed per example.

use motion_core::easing::Easing;
use motion_core::scene::{
    Color, Fill, FontRole, GlyphOrder, GlyphPose, Layer, LayerKind, MotionOp, PathFillRule, Scene,
    Stroke,
};

use crate::lower::{r6, Ctx, Track, Use};
use crate::program::{Aspect, Component, DiagramPreset, Slot, Swatch, Verb};
use crate::taste::{Shape, Spring};

struct Node {
    centre: [f32; 2],
    half: [f32; 2],
    round: bool,
}

/// The point where the segment from `n.centre` towards `to` leaves the node.
fn border(n: &Node, to: [f32; 2]) -> [f32; 2] {
    let (dx, dy) = (to[0] - n.centre[0], to[1] - n.centre[1]);
    let len = dx.hypot(dy).max(1e-3);
    if n.round {
        let r = n.half[0];
        return [n.centre[0] + dx / len * r, n.centre[1] + dy / len * r];
    }
    let sx = if dx.abs() > 1e-3 {
        n.half[0] / dx.abs()
    } else {
        f32::MAX
    };
    let sy = if dy.abs() > 1e-3 {
        n.half[1] / dy.abs()
    } else {
        f32::MAX
    };
    let s = sx.min(sy);
    [n.centre[0] + dx * s, n.centre[1] + dy * s]
}

fn n2(v: f32) -> String {
    format!("{}", (v * 100.0).round() / 100.0)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower(
    cx: &Ctx,
    c: &Component,
    preset: DiagramPreset,
    slot: Slot,
    labels: &[String],
    // All the labels, for node size (uniform across steps).
    size_labels: &[String],
    edges: &[[usize; 2]],
    colour: Option<Swatch>,
    z: i32,
    length: f64,
    free: [f32; 2],
) -> Scene {
    let n = labels.len();
    let id = c.id.as_str();
    // Roles on the diagram's ground: nodes in ink (or the asked swatch),
    // connectors strong structure, the travelling dot the accent.
    let r = cx.roles(id);
    let ink = cx.paint(id, colour.unwrap_or(Swatch::Ink), Use::Mark);
    let edges: Vec<[usize; 2]> = if edges.is_empty() {
        (1..n).map(|i| [i - 1, i]).collect()
    } else {
        edges.to_vec()
    };
    let landscape = cx.aspect != Aspect::Portrait;
    let is_split = matches!(slot, Slot::Left | Slot::Right);
    let slot_cx = if is_split {
        crate::taste::slot_center(slot, cx.aspect)[0]
    } else {
        cx.w / 2.0
    };
    let slot_w = if is_split {
        crate::taste::slot_width(slot, cx.aspect)
    } else {
        cx.w * 0.88
    };

    // Type and node sizes.
    let (role, base_fs, ls, upper) = match preset {
        DiagramPreset::Cards => (FontRole::Display, (cx.short * 0.056).round(), -0.01, false),
        _ => (FontRole::Mono, (cx.short * 0.05).round(), 0.06, true),
    };
    let fs = if is_split {
        (base_fs * 0.62).round().max(16.0)
    } else if !landscape && n <= 5 && preset != DiagramPreset::Rail {
        (base_fs * 1.4).round()
    } else {
        base_fs
    };
    let widest = size_labels
        .iter()
        .map(|l| cx.line_width(l, role, fs, ls, upper))
        .fold(0.0f32, f32::max);
    let rail = preset == DiagramPreset::Rail;
    let node_r = if is_split {
        (cx.short * 0.044).round()
    } else {
        (cx.short * 0.06).round()
    };
    let half = if rail {
        [node_r, node_r]
    } else {
        [(widest / 2.0 + fs * 1.1).round(), (fs * 1.25).round()]
    };

    // Grid: one line when it fits, else two (row-major, reading order).
    // The span between the outermost node centres (nodes stay inside).
    // Vertical room: the free band left by concurrent top/bottom text.
    let band_mid = (free[0] + free[1]) / 2.0;
    let band_h = free[1] - free[0];
    // Rail labels sit under the nodes; cards and outlines hold theirs.
    let label_room = if rail { fs * 2.0 } else { 0.0 };
    let (area_w, area_h) = (slot_w - half[0] * 2.0, band_h - half[1] * 2.0 - label_room);
    // The grid is set by all the nodes (no reflow between steps); the
    // nodes shown so far are re-centred in the band below.
    let grid = size_labels.len().max(n);
    let per_line = if (!is_split && rail)
        || grid
            <= if is_split {
                2
            } else if landscape {
                4
            } else {
                3
            } {
        grid
    } else {
        grid.div_ceil(2)
    };
    let lines = grid.div_ceil(per_line);
    let mut nodes: Vec<Node> = (0..grid)
        .map(|i| {
            // Serpentine: odd lines run back, so each step stays adjacent.
            let (a, b) = (i % per_line, i / per_line);
            let a = if b % 2 == 1 { per_line - 1 - a } else { a };
            // Steps are spaced by the node size (at most ~3 node lengths
            // apart), never stretched across the whole frame.
            let along = |len: f32, k: usize, of: usize| {
                if of == 1 {
                    0.0
                } else {
                    -len / 2.0 + len * k as f32 / (of - 1) as f32
                }
            };
            let cap = |len: f32, node: f32, of: usize| len.min(node * 4.0 * (of.max(2) - 1) as f32);
            let (main, cross) = if landscape {
                (
                    slot_cx + along(cap(area_w, half[0] * 2.0, per_line), a, per_line),
                    band_mid + along(cap(area_h, half[1] * 2.0, lines), b, lines),
                )
            } else {
                (
                    band_mid
                        + along(
                            cap(band_h - half[1] * 2.0 - label_room, half[1] * 2.0, per_line),
                            a,
                            per_line,
                        ),
                    slot_cx + along(cap(slot_w * 0.48, half[0] * 2.0, lines), b, lines),
                )
            };
            let centre = if landscape {
                [main, cross]
            } else {
                [cross, main]
            };
            Node {
                centre,
                half,
                round: rail,
            }
        })
        .collect();
    nodes.truncate(n);
    // A line still filling sits centred, not at one end of its slots.
    for b in 0..lines {
        let idx: Vec<usize> = (b * per_line..((b + 1) * per_line).min(n)).collect();
        if idx.is_empty() || idx.len() == per_line {
            continue;
        }
        let axis = if landscape { 0 } else { 1 };
        let mid = if landscape { slot_cx } else { band_mid };
        let mean = idx.iter().map(|&i| nodes[i].centre[axis]).sum::<f32>() / idx.len() as f32;
        for &i in &idx {
            nodes[i].centre[axis] += mid - mean;
        }
    }
    // Layout follows content: the shown nodes sit centred in the band.
    if n < grid {
        let lo = nodes.iter().fold([f32::MAX; 2], |q, nd| {
            [
                q[0].min(nd.centre[0] - nd.half[0]),
                q[1].min(nd.centre[1] - nd.half[1]),
            ]
        });
        let hi = nodes.iter().fold([f32::MIN; 2], |q, nd| {
            [
                q[0].max(nd.centre[0] + nd.half[0]),
                q[1].max(nd.centre[1] + nd.half[1]),
            ]
        });
        let shift = [
            slot_cx - (lo[0] + hi[0]) / 2.0,
            band_mid - (lo[1] + hi[1]) / 2.0,
        ];
        for nd in &mut nodes {
            nd.centre = [nd.centre[0] + shift[0], nd.centre[1] + shift[1]];
        }
    }

    // Reveal times: node 0 at the start, then each advance in time order.
    let mut steps: Vec<f64> = c
        .timeline
        .iter()
        .filter(|e| matches!(e.verb, Verb::Advance))
        .filter_map(|e| e.at)
        .collect();
    steps.sort_by(f64::total_cmp);
    let mut shown = vec![0.0f64];
    shown.extend(steps.iter().copied().take(n - 1));

    let mut layers: Vec<Layer> = Vec::new();
    let mut t = Track::default();
    let stroke_w = (cx.short * 0.005).max(2.0).round();
    let (fill, text_col): (Option<Color>, Color) = match preset {
        DiagramPreset::Cards => (Some(ink), cx.col.roles(ink).text(r.ground, fs)),
        DiagramPreset::Rail => (Some(r.ground), ink),
        DiagramPreset::Outline => (None, ink),
    };

    // Edges first (under the nodes).
    let mut dot_path: Vec<(f64, f64, [f32; 2], [f32; 2])> = Vec::new();
    for (k, &[a, b]) in edges.iter().enumerate() {
        let (Some(na), Some(nb)) = (nodes.get(a), nodes.get(b)) else {
            continue;
        };
        let Some(&tb) = shown.get(b) else { continue };
        let p0 = border(na, nb.centre);
        let p1 = border(nb, na.centre);
        let gap = stroke_w * 2.0;
        let dir = [(p1[0] - p0[0]), (p1[1] - p0[1])];
        let len = dir[0].hypot(dir[1]).max(1e-3);
        let u = [dir[0] / len, dir[1] / len];
        let (s, e) = (
            [p0[0] + u[0] * gap, p0[1] + u[1] * gap],
            [p1[0] - u[0] * gap, p1[1] - u[1] * gap],
        );
        // Arrowhead: a small chevron at the end.
        let ah = (cx.short * 0.016).max(6.0);
        let back = [e[0] - u[0] * ah, e[1] - u[1] * ah];
        let perp = [-u[1] * ah * 0.6, u[0] * ah * 0.6];
        let d = format!(
            "M{} {}L{} {}M{} {}L{} {}L{} {}",
            n2(s[0]),
            n2(s[1]),
            n2(e[0]),
            n2(e[1]),
            n2(back[0] + perp[0]),
            n2(back[1] + perp[1]),
            n2(e[0]),
            n2(e[1]),
            n2(back[0] - perp[0]),
            n2(back[1] - perp[1])
        );
        let eid = format!("{id}.edge{k}");
        let mut l = crate::lower::layer(
            &eid,
            LayerKind::Path {
                d,
                fill: None,
                stroke: Some(Stroke {
                    color: r.structure_strong,
                    width: stroke_w,
                    brush: None,
                }),
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [0.0, 0.0],
            [cx.w, cx.h],
            z,
        );
        l.anchor_x = 0.0;
        l.anchor_y = 0.0;
        // Draws as its target node lands, while the siblings make room
        // (layout follows content: the edge exists only once its node does).
        let dur = 0.22f64.max(1.0 / cx.fps);
        let start = r6(tb);
        // The token follows one incoming edge per node.
        let first_in = edges.iter().position(|e2| e2[1] == b) == Some(k);
        if tb > 0.0 {
            t.push(
                &eid,
                start,
                dur,
                Shape::Ease(Easing::InOutCubic),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
            if first_in {
                dot_path.push((start, dur, s, e));
            }
        }
        layers.push(l);
    }

    // Nodes.
    for (i, node) in nodes.iter().enumerate() {
        let Some(&at) = shown.get(i) else { continue };
        let nid = format!("{id}.node{i}");
        let (w, h) = (node.half[0] * 2.0, node.half[1] * 2.0);
        let d = if node.round {
            motion_core::path::ellipse(0.0, 0.0, w, h).to_svg()
        } else {
            motion_core::path::rounded_rect(0.0, 0.0, w, h, h * 0.22).to_svg()
        };
        layers.push(crate::lower::layer(
            &nid,
            LayerKind::Path {
                d,
                fill: fill.map(Fill::Solid),
                stroke: Some(Stroke {
                    color: ink,
                    width: stroke_w,
                    brush: None,
                }),
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            node.centre,
            [w, h],
            z + 1,
        ));
        let label_at = if node.round {
            if landscape {
                [node.centre[0], node.centre[1] + node.half[1] + fs * 1.4]
            } else {
                [
                    node.centre[0]
                        + node.half[0]
                        + fs * 0.6
                        + cx.line_width(&labels[i], role, fs, ls, upper) / 2.0,
                    node.centre[1],
                ]
            }
        } else {
            node.centre
        };
        let lid = format!("{id}.label{i}");
        let tc = if node.round { ink } else { text_col };
        layers.push(cx.text_canvas(&lid, &labels[i], role, fs, tc, ls, upper, label_at, z + 2));
        // Lands on its time: pop with one small overshoot, label types on.
        t.push(
            &nid,
            at,
            0.3,
            Shape::Spring(Spring::Pop),
            MotionOp::Scale {
                from: 0.6,
                to: 1.0,
                axis: Default::default(),
            },
        );
        t.push(
            &nid,
            at,
            0.12,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        let chars = labels[i]
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .count()
            .max(1) as f64;
        let stagger = 0.018;
        t.push(
            &lid,
            r6(at + 0.05),
            r6((chars - 1.0) * stagger + 0.12),
            Shape::Ease(Easing::OutCubic),
            MotionOp::GlyphCascade {
                stagger,
                from: GlyphPose {
                    opacity: 0.0,
                    dy: fs * 0.3,
                    ..GlyphPose::REST
                },
                order: GlyphOrder::Forward,
                seed: 0,
            },
        );
    }

    // The carried dot: rides each drawing edge's tip.
    if let Some(&(_, _, first, _)) = dot_path.first() {
        let did = format!("{id}.dot");
        let s = (cx.short * 0.022).round().max(8.0);
        let mut dot = crate::lower::layer(
            &did,
            LayerKind::Path {
                d: motion_core::path::ellipse(0.0, 0.0, s, s).to_svg(),
                fill: Some(Fill::Solid(r.accent)),
                stroke: None,
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [0.0, 0.0],
            [s, s],
            z + 3,
        );
        dot.opacity = 1.0;
        // Offsets are canvas positions (home at the origin), so the rides
        // of different step layouts compose (layout follows content).
        t.push(
            &did,
            0.0,
            r6(1.0 / cx.fps),
            Shape::Ease(Easing::Linear),
            MotionOp::Move {
                from: first,
                to: first,
                via: None,
            },
        );
        // Hidden until its first edge draws (nothing waits on screen).
        t.push(
            &did,
            0.0,
            r6(1.0 / cx.fps),
            Shape::Ease(Easing::Linear),
            MotionOp::Scale {
                from: 0.0,
                to: 0.0,
                axis: Default::default(),
            },
        );
        let mut prev = first;
        dot_path.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut last_end = 0.0f64;
        let count = dot_path.len();
        for i in 0..count {
            let (start, dur, s0, e) = dot_path[i];
            let (from, to) = (s0, e);
            let start = start.max(last_end);
            if i == 0 {
                let rise = r6((start * 0.5).clamp(1.0 / 120.0, 0.1));
                t.push(
                    &did,
                    r6((start - rise).max(1.0 / cx.fps)),
                    rise,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Scale {
                        from: 0.0,
                        to: 1.0,
                        axis: Default::default(),
                    },
                );
            } else {
                // Rise again where this edge starts (it sank into the last
                // node), in the gap before the edge draws.
                let gap = (start - last_end).max(0.0);
                let rise = r6((gap * 0.45).clamp(1.0 / 120.0, 0.1));
                if from != prev {
                    t.push(
                        &did,
                        r6(last_end),
                        r6((start - last_end).max(1.0 / 120.0)),
                        Shape::Ease(Easing::InOutCubic),
                        MotionOp::Move {
                            from: prev,
                            to: from,
                            via: None,
                        },
                    );
                }
                t.push(
                    &did,
                    r6((start - rise).max(last_end)),
                    rise,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Scale {
                        from: 0.0,
                        to: 1.0,
                        axis: Default::default(),
                    },
                );
            }
            t.push(
                &did,
                r6(start),
                dur,
                Shape::Ease(Easing::InOutCubic),
                MotionOp::Move {
                    from,
                    to,
                    via: None,
                },
            );
            // Dock: sink into the node it reached, within the gap before the
            // next edge (never resting half over a node).
            let next = dot_path
                .get(i + 1)
                .map_or(start + dur + 0.3, |n| n.0.max(start + dur));
            let dock = r6(((next - start - dur) * 0.45).clamp(1.0 / 120.0, 0.12));
            t.push(
                &did,
                r6(start + dur),
                dock,
                Shape::Ease(Easing::InCubic),
                MotionOp::Scale {
                    from: 1.0,
                    to: 0.0,
                    axis: Default::default(),
                },
            );
            prev = to;
            last_end = start + dur + dock;
        }
        layers.push(dot);
        // Completion: the last node fills with the accent as the token docks.
        if let Some(&done) = shown.get(n - 1) {
            if done > 0.0 {
                t.push(
                    &format!("{id}.node{}", n - 1),
                    r6(done + 0.05),
                    r6(0.5f64.min(length - done - 0.05).max(1.0 / 120.0)),
                    Shape::Spring(Spring::Settle),
                    MotionOp::Tint {
                        color: r.accent,
                        from: 0.0,
                        to: 1.0,
                    },
                );
            }
        }
    }

    // Whole-diagram fades.
    for e in &c.timeline {
        let Some(at) = e.at else { continue };
        let op = match e.verb {
            Verb::Exit { .. } => MotionOp::Fade { from: 1.0, to: 0.0 },
            _ => continue,
        };
        let d = crate::taste::duration(&e.verb, cx.tempo).min(length - at);
        let ids: Vec<String> = layers.iter().map(|l| l.id.clone()).collect();
        for lid in ids {
            if !lid.contains(".node") {
                t.push(&lid, at, d, Shape::Ease(Easing::InCubic), op.clone());
            }
        }
        for i in 0..n {
            t.push(
                &format!("{id}.node{i}"),
                at,
                d,
                Shape::Ease(Easing::InCubic),
                op.clone(),
            );
        }
    }
    crate::lower::scene(id, c.start, length, layers, t.motions)
}
