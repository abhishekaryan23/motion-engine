//! Chart: line, area, bar, donut and spring, with one hero value.
//!
//! `enter: draw` builds the chart. Line-like series are revealed left to
//! right by a mask whose edge moves with the series' own easing, and a dot
//! rides the tip: it lives in a group that slides along x on that easing
//! while the dot moves in y through the curve's own samples, so line and dot
//! never part. The `spring` chart draws the closed-form spring response
//! itself and moves its dot with the *same* spring spec. Bars grow on a
//! spring, donut segments draw in turn. `pop_label` pops the hero callout,
//! which counts up to its number. All geometry is in the chart box.

use motion_core::easing::Easing;
use motion_core::scene::{
    Axis, Color, Fill, FontRole, Layer, LayerKind, LayerMask, MotionOp, PathFillRule, Scene, Stroke,
};

use crate::lower::{r6, Ctx, Track, Use};
use crate::program::{
    Aspect, ChartKind, ChartPreset, Component, EnterStyle, Size, Slot, Swatch, Verb,
};
use crate::taste::{self, Shape, Spring};

#[derive(Clone, Copy)]
pub(crate) struct Chart<'a> {
    pub preset: ChartPreset,
    pub kind: ChartKind,
    pub values: &'a [f64],
    pub labels: &'a [String],
    pub hero: Option<usize>,
    pub callout: Option<&'a str>,
    pub slot: Slot,
    pub size: Size,
    pub colour: Option<Swatch>,
    pub title: Option<&'a str>,
    /// Size the plot for a callout that has not landed yet (the layout
    /// before the pop); placement ignores it.
    pub sized_for_callout: bool,
}

/// Chart box `[w, h]` in px for a slot and size.
fn box_size(cx: &Ctx, slot: Slot, size: Size) -> [f32; 2] {
    let side = matches!(slot, Slot::Lead | Slot::Trail | Slot::Left | Slot::Right);
    let (fw, fh) = match (cx.aspect, side) {
        (Aspect::Landscape, false) => (0.86, 0.62),
        (Aspect::Landscape, true) => (0.38, 0.50),
        (Aspect::Portrait, false) => (0.9, 0.5),
        (Aspect::Portrait, true) => (0.84, 0.22),
        (Aspect::Square, _) => (0.76, 0.5),
    };
    let k = match size {
        Size::S => 0.7,
        Size::M => 0.85,
        Size::L => 1.0,
        Size::Xl => 1.12,
    };
    [(cx.w * fw * k).round(), (cx.h * fh * k).round()]
}

fn at_origin(id: &str, kind: LayerKind, origin: [f32; 2], size: [f32; 2], z: i32) -> Layer {
    let mut l = crate::lower::layer(id, kind, origin, size, z);
    l.anchor_x = 0.0;
    l.anchor_y = 0.0;
    l
}

fn n2(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    format!("{r}")
}

fn polyline_d(pts: &[[f32; 2]]) -> String {
    let mut d = format!("M{} {}", n2(pts[0][0]), n2(pts[0][1]));
    for p in &pts[1..] {
        d.push_str(&format!("L{} {}", n2(p[0]), n2(p[1])));
    }
    d
}

/// Catmull-Rom through `pts` (uniform x spacing), sampled `per` points per
/// span. With uniform spacing the x of each span is linear in its
/// parameter, so samples are evenly spaced in x as well.
fn smooth_samples(pts: &[[f32; 2]], per: usize) -> Vec<[f32; 2]> {
    let n = pts.len();
    let mut out = vec![pts[0]];
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = pts[(i + 2).min(n - 1)];
        let c1 = [p1[0] + (p2[0] - p0[0]) / 6.0, p1[1] + (p2[1] - p0[1]) / 6.0];
        let c2 = [p2[0] - (p3[0] - p1[0]) / 6.0, p2[1] - (p3[1] - p1[1]) / 6.0];
        for k in 1..=per {
            let u = k as f32 / per as f32;
            let v = 1.0 - u;
            let (a, b, c, d) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
            out.push([
                a * p1[0] + b * c1[0] + c * c2[0] + d * p2[0],
                a * p1[1] + b * c1[1] + c * c2[1] + d * p2[1],
            ]);
        }
    }
    out
}

fn rect_d(x: f32, y: f32, w: f32, h: f32) -> String {
    format!(
        "M{} {}H{}V{}H{}Z",
        n2(x),
        n2(y),
        n2(x + w),
        n2(y + h),
        n2(x)
    )
}

/// Arc from angle `a0` to `a1` (radians, clockwise from 12 o'clock).
fn arc_d(c: [f32; 2], r: f32, a0: f32, a1: f32) -> String {
    let pt = |a: f32| [c[0] + r * a.sin(), c[1] - r * a.cos()];
    let mut d = format!("M{} {}", n2(pt(a0)[0]), n2(pt(a0)[1]));
    let steps = ((a1 - a0) / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    for k in 1..=steps {
        let a = a0 + (a1 - a0) * k as f32 / steps as f32;
        let p = pt(a);
        d.push_str(&format!(
            "A{} {} 0 0 1 {} {}",
            n2(r),
            n2(r),
            n2(p[0]),
            n2(p[1])
        ));
    }
    d
}

fn with_alpha(c: Color, a: u8) -> Color {
    Color { a, ..c }
}

/// `"+22%"` → `("+", 22.0, 0, "%")`; `None` when there is no number.
pub(crate) fn parse_number(s: &str) -> Option<(String, f64, u8, String)> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let rest = &s[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(rest.len());
    let num = rest[..end].trim_end_matches(['.', ',']);
    let value: f64 = num.replace(',', "").parse().ok()?;
    let decimals = num.split('.').nth(1).map_or(0, |f| f.len() as u8);
    Some((
        s[..start].to_string(),
        value,
        decimals,
        rest[num.len()..].to_string(),
    ))
}

/// The on-screen form of a stat or callout string. A bare integer of five or
/// more digits (`1284960`, `$1284960`) gets `,` thousands separators
/// (`1,284,960`); anything the author already formatted (`1,284`), decimals,
/// percentages, units (`3.2x`, `98%`, `12345 km`) and shorter numbers are
/// left as written.
pub(crate) fn group_thousands(s: &str) -> String {
    let Some((prefix, _, 0, suffix)) = parse_number(s) else {
        return s.to_string();
    };
    let digits = &s[prefix.len()..];
    let bare = suffix.is_empty()
        && digits.len() >= 5
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0')
        && prefix.chars().all(|c| "$€£¥₹+-−".contains(c));
    if !bare {
        return s.to_string();
    }
    let mut out = prefix;
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Inverse of OutCubic: the time fraction at which it reaches `p`.
fn inv_out_cubic(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).cbrt()
}

fn pop_in(t: &mut Track, target: &str, at: f64, d: f64) {
    t.push(
        target,
        at,
        d,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.6,
            to: 1.0,
            axis: Default::default(),
        },
    );
    t.push(
        target,
        at,
        taste::QUICK_FADE,
        Shape::Ease(Easing::OutCubic),
        MotionOp::Fade { from: 0.0, to: 1.0 },
    );
}

/// `free`: the vertical span `[top, bottom]` not taken by concurrent
/// top/bottom text (the layout avoids it by shrinking the box).
pub(crate) fn lower(
    cx: &Ctx,
    c: &Component,
    ch: &Chart,
    z: i32,
    length: f64,
    free: [f32; 2],
) -> Scene {
    let [bw, bh0] = box_size(cx, ch.slot, ch.size);
    let mut centre = taste::slot_center(ch.slot, cx.aspect);
    // Labels under the baseline need room too.
    let below = if ch.labels.is_empty() {
        0.0
    } else {
        cx.short * 0.06
    };
    // A callout above the series needs headroom inside the free band.
    let above_callout = if ch.callout.is_some() && ch.kind != ChartKind::Donut {
        cx.short * 0.11
    } else {
        0.0
    };
    let title_fs = (cx.short * 0.04).round();
    let above = above_callout
        + if ch.title.is_some() {
            title_fs * 1.8
        } else {
            0.0
        };
    // Sized so the callout fits once it lands; placed without it until
    // then (layout follows content: lower.rs re-lays out on the pop).
    // Sized so the callout fits once it lands (no empty band shows: the
    // layout before the pop is placed without it and re-centred, `flip`).
    let size_extra = if ch.sized_for_callout && ch.kind != ChartKind::Donut {
        cx.short * 0.11
    } else {
        0.0
    };
    let room = free[1] - free[0] - above - size_extra - below;
    let free = [free[0] + above, free[1]];
    let bh = bh0.min(room).max(cx.h * 0.15).round();
    // The whole composition (title and callout above the plot) is centred,
    // so a callout landing re-centres it rather than hanging off the top.
    let top = (centre[1] - bh / 2.0 + above / 2.0).max(free[0]);
    let top = top.min(free[1] - below - bh);
    centre[1] = top + bh / 2.0;
    let origin = [
        (centre[0] - bw / 2.0).round(),
        (centre[1] - bh / 2.0).round(),
    ];
    let id = c.id.as_str();
    // Roles on the chart's ground: the series is the accent (thin axis) or
    // the ink (paper); other bars, axes and guides are strong structure
    // (>= 3:1: a reference line that reads, not an artefact); the hero is
    // the accent.
    let r = cx.roles(id);
    let (series, other) = match ch.preset {
        ChartPreset::ThinAxis => (r.accent, r.structure_strong),
        ChartPreset::Paper => (r.ink, r.structure_strong),
    };
    let guide = r.structure_strong;
    // Axes and base lines: >= 2 px at 1080p.
    let guide_w = (cx.short / 540.0).max(2.0).round();
    let (series, hero_colour) = match ch.colour {
        Some(s) => {
            let c = cx.paint(id, s, Use::Line { meaning: true });
            (c, c)
        }
        None => (series, r.accent),
    };
    let line_w = (cx.short * 0.009).max(4.0).round();
    let draw_at = c.timeline.iter().find_map(|e| match e.verb {
        Verb::Enter {
            style: EnterStyle::Draw,
        } => e.at,
        _ => None,
    });
    // Beat steps: value 0 with the draw, value i on the i-th advance.
    let mut steps: Vec<f64> = c
        .timeline
        .iter()
        .filter(|e| matches!(e.verb, Verb::Advance))
        .filter_map(|e| e.at)
        .collect();
    steps.sort_by(f64::total_cmp);
    // A stepped chart with no authored entrance draws in at its start.
    let draw_at = draw_at.or(if steps.is_empty() { None } else { Some(0.0) });
    let pop_at = c.timeline.iter().find_map(|e| match e.verb {
        Verb::PopLabel => e.at,
        _ => None,
    });
    let d_draw = taste::duration(
        &Verb::Enter {
            style: EnterStyle::Draw,
        },
        cx.tempo,
    );
    let mut layers: Vec<Layer> = Vec::new();
    let mut t = Track::default();
    let n = ch.values.len();
    let hero = ch.hero.filter(|h| *h < n);
    let hero_pt: Option<[f32; 2]>;
    // A dot that rode the series tip and now marks the hero.
    let mut rider: Option<String> = None;

    match ch.kind {
        ChartKind::Line | ChartKind::Area | ChartKind::Spring => {
            let spring = ch.kind == ChartKind::Spring;
            // The series values, the reveal's shape and its duration.
            let (ys, ease, dur): (Vec<f64>, Shape, f64) = if spring {
                let (from, to) = (ch.values[0], ch.values[1]);
                let dur = r6(2.0 * cx.tempo.factor());
                let spec = Spring::Lively.spec();
                let m = 96;
                let ys = (0..=m)
                    .map(|k| {
                        let tau = dur * k as f64 / m as f64;
                        from + (to - from) * motion_core::easing::spring_progress(tau, dur, spec)
                    })
                    .collect();
                // x is time on a spring graph: the reveal is linear in it.
                (ys, Shape::Ease(Easing::Linear), dur)
            } else {
                (ch.values.to_vec(), Shape::Ease(Easing::OutCubic), d_draw)
            };
            let lo = ys.iter().cloned().fold(f64::MAX, f64::min).min(0.0);
            let hi = ys.iter().cloned().fold(f64::MIN, f64::max);
            let hi = if hi > lo { hi } else { lo + 1.0 };
            let y_of = |v: f64| bh - ((v - lo) / (hi - lo)) as f32 * bh * 0.86;
            let data: Vec<[f32; 2]> = ys
                .iter()
                .enumerate()
                .map(|(i, v)| [bw * i as f32 / (ys.len() - 1) as f32, y_of(*v)])
                .collect();
            let curve = if spring {
                data.clone()
            } else {
                smooth_samples(&data, 12)
            };
            // Axis: a baseline plus guides (the target level for springs).
            let mut axis = format!("M0 {b}H{w}", b = n2(bh), w = n2(bw));
            if spring {
                axis.push_str(&format!(
                    "M0 {y}H{w}",
                    y = n2(y_of(ch.values[1])),
                    w = n2(bw)
                ));
            } else {
                axis.push_str(&format!(
                    "M0 {g1}H{w}M0 {g2}H{w}",
                    w = n2(bw),
                    g1 = n2(bh * 0.6),
                    g2 = n2(bh * 0.2)
                ));
            }
            let axis_id = format!("{id}.axis");
            layers.push(at_origin(
                &axis_id,
                LayerKind::Path {
                    d: axis,
                    fill: None,
                    stroke: Some(Stroke {
                        color: guide,
                        width: guide_w,
                        brush: None,
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                origin,
                [bw, bh],
                z,
            ));
            // Reveal mask: a rectangle whose right edge travels with the
            // series' easing.
            // Already 10 % drawn on the first frame (never an empty shot).
            let collapsed = rect_d(-line_w * 2.0, -bh, bw * 0.1 + line_w * 2.0, 3.0 * bh);
            let full = rect_d(-line_w * 2.0, -bh, bw + line_w * 4.0, 3.0 * bh);
            let reveal = |l: &mut Layer, t: &mut Track, at: f64| {
                l.mask = Some(LayerMask {
                    d: collapsed.clone(),
                    invert: false,
                });
                t.push(
                    &l.id,
                    at,
                    dur,
                    ease,
                    MotionOp::MaskMorph { to: full.clone() },
                );
            };
            let reveal_series = |l: &mut Layer, t: &mut Track, at: f64| {
                if steps.is_empty() || spring {
                    reveal(l, t, r6(at + 0.2));
                } else {
                    // Stepped: to value 1 with the draw, then one value per
                    // step (n values, n - 2 steps; `beat_cuts` plans that).
                    l.mask = Some(LayerMask {
                        d: collapsed.clone(),
                        invert: false,
                    });
                    let n_pts = data.len();
                    let mut times = vec![at];
                    times.extend(steps.iter().copied());
                    for (k, t0) in times.iter().enumerate() {
                        let x = data[(k + 1).min(n_pts - 1)][0];
                        t.push(
                            &l.id,
                            r6(*t0),
                            0.3,
                            Shape::Ease(Easing::OutCubic),
                            MotionOp::MaskMorph {
                                to: rect_d(-line_w * 2.0, -bh, x + line_w * 2.0, 3.0 * bh),
                            },
                        );
                    }
                }
            };
            if ch.kind == ChartKind::Area {
                let mut d = polyline_d(&curve);
                d.push_str(&format!("L{} {}L0 {}Z", n2(bw), n2(bh), n2(bh)));
                let mut l = at_origin(
                    &format!("{id}.area"),
                    LayerKind::Path {
                        d,
                        fill: Some(Fill::Solid(with_alpha(series, 36))),
                        stroke: None,
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    origin,
                    [bw, bh],
                    z + 1,
                );
                if let Some(at) = draw_at {
                    reveal_series(&mut l, &mut t, at);
                }
                layers.push(l);
            }
            // A spring graph is the hero of its shot: a heavier stroke.
            let line_w = if spring { line_w * 1.6 } else { line_w };
            let mut line = at_origin(
                &format!("{id}.line"),
                LayerKind::Path {
                    d: polyline_d(&curve),
                    fill: None,
                    stroke: Some(Stroke {
                        color: series,
                        width: line_w,
                        brush: None,
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                origin,
                [bw, bh],
                z + 2,
            );
            if let Some(at) = draw_at {
                // The axis is there from the first frame (no empty frame).
                t.push(
                    &axis_id,
                    at,
                    0.3,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Trim { from: 0.6, to: 1.0 },
                );
                reveal_series(&mut line, &mut t, at);
            }
            layers.push(line);

            let end = *curve.last().expect("two or more values");
            let rides = spring || (hero == Some(n - 1) && steps.is_empty());
            hero_pt = if rides {
                Some(end)
            } else {
                hero.map(|h| data[h])
            };
            if let (true, Some(at)) = (rides, draw_at) {
                let start = r6(at + 0.2);
                let s = (line_w * 3.6).round();
                let group_id = format!("{id}.ride");
                let dot_id = format!("{id}.dot");
                let dot = crate::lower::layer(
                    &dot_id,
                    LayerKind::Path {
                        d: motion_core::path::ellipse(0.0, 0.0, s, s).to_svg(),
                        fill: Some(Fill::Solid(hero_colour)),
                        stroke: None,
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    [0.0, curve[0][1]],
                    [s, s],
                    0,
                );
                let group = at_origin(
                    &group_id,
                    LayerKind::Group {
                        children: vec![dot],
                    },
                    origin,
                    [bw, bh],
                    z + 4,
                );
                // x: the group slides with the reveal's edge.
                t.push(
                    &group_id,
                    start,
                    dur,
                    ease,
                    MotionOp::Move {
                        from: [0.0, 0.0],
                        to: [bw, 0.0],
                        via: None,
                    },
                );
                if spring {
                    // y: the very spring that drew the curve.
                    t.push(
                        &dot_id,
                        start,
                        dur,
                        Shape::Spring(Spring::Lively),
                        MotionOp::Move {
                            from: [0.0, 0.0],
                            to: [0.0, y_of(ch.values[1]) - curve[0][1]],
                            via: None,
                        },
                    );
                } else {
                    // y: through the curve's own samples, each reached when
                    // the reveal edge reaches its x.
                    let mut prev_t = start;
                    let mut prev_y = 0.0f32;
                    let last = curve.len() - 1;
                    for (k, p) in curve.iter().enumerate().skip(1) {
                        if k % 3 != 0 && k != last {
                            continue;
                        }
                        let tk = start + dur * inv_out_cubic(p[0] / bw) as f64;
                        let tk = r6(tk.max(prev_t + 1e-3));
                        let y = p[1] - curve[0][1];
                        t.push(
                            &dot_id,
                            prev_t,
                            r6(tk - prev_t),
                            Shape::Ease(Easing::Linear),
                            MotionOp::Move {
                                from: [0.0, prev_y],
                                to: [0.0, y],
                                via: None,
                            },
                        );
                        prev_t = tk;
                        prev_y = y;
                    }
                }
                t.push(
                    &dot_id,
                    start,
                    0.2,
                    Shape::Spring(Spring::Pop),
                    MotionOp::Scale {
                        from: 0.0,
                        to: 1.0,
                        axis: Default::default(),
                    },
                );
                layers.push(group);
                rider = Some(dot_id);
            }
        }
        ChartKind::Bar => {
            let max = ch.values.iter().cloned().fold(f64::MIN, f64::max);
            let min = ch.values.iter().cloned().fold(f64::MAX, f64::min);
            let lo = min.min(0.0);
            let hi = if max > lo { max } else { lo + 1.0 };
            let y_of = |v: f64| bh - ((v - lo) / (hi - lo)) as f32 * bh * 0.86;
            let slotw = bw / n as f32;
            let barw = (slotw * 0.62).round();
            let base_id = format!("{id}.base");
            layers.push(at_origin(
                &base_id,
                LayerKind::Path {
                    d: format!("M0 {b}H{w}", b = n2(bh), w = n2(bw)),
                    fill: None,
                    stroke: Some(Stroke {
                        color: guide,
                        width: guide_w,
                        brush: None,
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                origin,
                [bw, bh],
                z,
            ));
            if let Some(at) = draw_at {
                t.push(
                    &base_id,
                    at,
                    0.3,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Trim { from: 0.0, to: 1.0 },
                );
            }
            let bar_gap = (guide_w * 2.5).round().max(4.0);
            for (i, v) in ch.values.iter().enumerate() {
                let bar_id = format!("{id}.bar{i}");
                let h = (bh - bar_gap - y_of(*v)).max(1.0);
                let fill = match hero {
                    Some(hh) if hh == i => hero_colour,
                    Some(_) => other,
                    None => series,
                };
                let mut l = crate::lower::layer(
                    &bar_id,
                    LayerKind::Rectangle { fill, stroke: None },
                    [
                        origin[0] + slotw * (i as f32 + 0.5),
                        origin[1] + bh - bar_gap,
                    ],
                    [barw, h],
                    z + 1,
                );
                l.anchor_y = 1.0;
                layers.push(l);
                if let Some(at) = draw_at {
                    let start = r6(at + 0.08 + 0.05 * i as f64);
                    let d = 0.50f64.min((length - start).max(1.0 / cx.fps));
                    t.push(
                        &bar_id,
                        start,
                        d,
                        Shape::Spring(Spring::Settle),
                        MotionOp::Scale {
                            from: 0.05,
                            to: 1.0,
                            axis: Axis::Y,
                        },
                    );
                }
            }
            hero_pt = hero.map(|h| [slotw * (h as f32 + 0.5), y_of(ch.values[h])]);
        }
        ChartKind::Donut => {
            let total: f64 = ch.values.iter().sum();
            let r = bw.min(bh) * 0.4;
            let sw = (r * 0.24).round();
            let cpt = [bw / 2.0, bh / 2.0];
            let gap = sw / r * 1.1;
            let mut a = 0.0f32;
            let mut seg_at = draw_at;
            for (i, v) in ch.values.iter().enumerate() {
                let span = (*v / total) as f32 * std::f32::consts::TAU;
                let (a0, a1) = (a + gap / 2.0, a + span - gap / 2.0);
                a += span;
                if a1 <= a0 {
                    continue;
                }
                let seg_id = format!("{id}.seg{i}");
                let colour = match hero {
                    Some(hh) if hh == i => hero_colour,
                    Some(_) => other,
                    None => series,
                };
                layers.push(at_origin(
                    &seg_id,
                    LayerKind::Path {
                        d: arc_d(cpt, r, a0, a1),
                        fill: None,
                        stroke: Some(Stroke {
                            color: colour,
                            width: sw,
                            brush: None,
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    origin,
                    [bw, bh],
                    z + 1,
                ));
                if !steps.is_empty() {
                    seg_at = if i == 0 {
                        draw_at
                    } else {
                        steps.get(i - 1).copied()
                    };
                }
                if let Some(at) = seg_at {
                    let d = if steps.is_empty() {
                        r6((d_draw * (*v / total)).max(1.0 / cx.fps))
                    } else {
                        0.35
                    };
                    // One continuous sweep: ease in on the first segment,
                    // out on the last, steady between.
                    let e = if i == 0 {
                        Easing::InCubic
                    } else if i + 1 == n {
                        Easing::OutCubic
                    } else {
                        Easing::Linear
                    };
                    t.push(
                        &seg_id,
                        r6(at),
                        d,
                        Shape::Ease(e),
                        MotionOp::Trim { from: 0.0, to: 1.0 },
                    );
                    seg_at = Some(at + d);
                }
            }
            hero_pt = Some(cpt);
        }
    }

    // What the chart measures, set above it, left-aligned with the box.
    if let Some(title) = ch.title {
        let tid = format!("{id}.title");
        let fit_title_fs = cx.fit(title, FontRole::Display, -0.01, false, bw, title_fs);
        let w = cx.line_width(title, FontRole::Display, fit_title_fs, -0.01, false);
        layers.push(cx.text_canvas(
            &tid,
            title,
            FontRole::Display,
            fit_title_fs,
            r.ink,
            -0.01,
            false,
            [
                origin[0] + w / 2.0,
                origin[1] - above_callout - fit_title_fs * 0.9,
            ],
            z + 3,
        ));
    }

    // Value labels under the baseline.
    if !ch.labels.is_empty()
        && matches!(ch.kind, ChartKind::Line | ChartKind::Area | ChartKind::Bar)
    {
        let fs = (cx.short * 0.02).round().max(13.0);
        let slotw = bw / n as f32;
        for (i, lab) in ch.labels.iter().enumerate() {
            let x = match ch.kind {
                ChartKind::Bar => slotw * (i as f32 + 0.5),
                _ => bw * i as f32 / (n - 1) as f32,
            };
            let lid = format!("{id}.label{i}");
            layers.push(cx.text_canvas(
                &lid,
                lab,
                FontRole::Mono,
                fs,
                cx.paint(id, Swatch::Muted, Use::Text(fs)),
                0.1,
                true,
                [origin[0] + x, origin[1] + bh + fs * 1.6],
                z + 3,
            ));
            if let Some(at) = draw_at {
                t.push(
                    &lid,
                    r6(at + 0.3 + 0.02 * i as f64),
                    0.35,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Fade { from: 0.0, to: 1.0 },
                );
            }
        }
    }

    // Hero marker and callout.
    if let Some(p) = hero_pt {
        let abs = [origin[0] + p[0], origin[1] + p[1]];
        let pd = taste::duration(&Verb::PopLabel, cx.tempo);
        match (&rider, pop_at) {
            // The rider is the hero: it answers the pop with a small hit.
            (Some(dot), Some(at)) => {
                t.push(
                    dot,
                    at,
                    0.08,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Scale {
                        from: 1.0,
                        to: 1.5,
                        axis: Default::default(),
                    },
                );
                t.push(
                    dot,
                    at + 0.08,
                    pd - 0.08,
                    Shape::Spring(Spring::Pop),
                    MotionOp::Scale {
                        from: 1.5,
                        to: 1.15,
                        axis: Default::default(),
                    },
                );
            }
            (None, _) if matches!(ch.kind, ChartKind::Line | ChartKind::Area) => {
                let dot_id = format!("{id}.dot");
                let s = (line_w * 3.6).round();
                layers.push(crate::lower::layer(
                    &dot_id,
                    LayerKind::Path {
                        d: motion_core::path::ellipse(0.0, 0.0, s, s).to_svg(),
                        fill: Some(Fill::Solid(hero_colour)),
                        stroke: None,
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    abs,
                    [s, s],
                    z + 4,
                ));
                if let Some(at) = pop_at {
                    pop_in(&mut t, &dot_id, at, pd);
                }
            }
            _ => {}
        }
        if let Some(text) = ch.callout {
            // Large bare integers read with thousands separators.
            let shown = group_thousands(text);
            let text = shown.as_str();
            let callout_id = format!("{id}.callout");
            let (fs, at) = match ch.kind {
                ChartKind::Donut => {
                    let r = bw.min(bh) * 0.4;
                    let fs = cx.fit(text, FontRole::Display, -0.02, false, r * 1.2, r * 0.5);
                    (fs, abs)
                }
                _ => {
                    let fs = (cx.short * 0.08).round();
                    // Above the plot, over the hero: clear of the series,
                    // the hero dot and the guides.
                    (
                        fs,
                        [
                            abs[0],
                            (abs[1] - fs * 0.85 - line_w * 3.0).min(origin[1] - fs * 0.7),
                        ],
                    )
                }
            };
            let mut l = cx.text_canvas(
                &callout_id,
                text,
                FontRole::Display,
                fs,
                if ch.preset == ChartPreset::Paper {
                    r.text(hero_colour, fs)
                } else {
                    r.ink
                },
                -0.02,
                false,
                at,
                z + 5,
            );
            // Keep the callout inside the frame (and inside its slot), with room for a camera lean.
            let half = l.width / 2.0;
            let m = cx.short * 0.1;
            let (min_x, max_x) = if matches!(ch.slot, Slot::Left | Slot::Right) {
                (
                    (origin[0] + half).max(half + m),
                    (origin[0] + bw - half).min(cx.w - m - half),
                )
            } else {
                (half + m, cx.w - m - half)
            };
            l.x = l.x.clamp(min_x, max_x.max(min_x));
            l.y = l.y.max(m + l.height / 2.0);
            layers.push(l);
            if let Some(at) = pop_at {
                pop_in(&mut t, &callout_id, at, pd);
                // The number counts up as it lands.
                if let Some((prefix, value, decimals, suffix)) = parse_number(text) {
                    t.push(
                        &callout_id,
                        at,
                        r6(0.35f64.min(length - at)),
                        Shape::Ease(Easing::OutQuint),
                        MotionOp::Count {
                            from: 0.0,
                            to: value,
                            decimals,
                            // Every intermediate value is grouped like the end value.
                            grouping: text.contains(','),
                            prefix,
                            suffix,
                        },
                    );
                }
            }
        }
    }

    for e in &c.timeline {
        let Some(at) = e.at else { continue };
        let d = taste::duration(&e.verb, cx.tempo);
        let op = match &e.verb {
            Verb::Enter {
                style: EnterStyle::Fade,
            } => MotionOp::Fade { from: 0.0, to: 1.0 },
            Verb::Exit { .. } => MotionOp::Fade { from: 1.0, to: 0.0 },
            _ => continue,
        };
        // Whole-chart fades act on every top-level layer; layers with their
        // own entrance skip the fade-in.
        let fade_in = matches!(op, MotionOp::Fade { from, .. } if from == 0.0);
        let ids: Vec<String> = layers.iter().map(|l| l.id.clone()).collect();
        for lid in ids {
            let pops = pop_at.is_some() && (lid.ends_with(".callout") || lid.ends_with(".dot"));
            let draws = draw_at.is_some() && lid.contains(".label");
            if fade_in && (pops || draws) {
                continue;
            }
            t.push(&lid, at, d, Shape::Ease(Easing::OutCubic), op.clone());
        }
    }
    crate::lower::scene(id, c.start, length, layers, t.motions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::timeline::format_count;

    #[test]
    fn four_digit_and_short_numbers_are_unchanged() {
        assert_eq!(group_thousands("1234"), "1234");
        assert_eq!(group_thousands("98"), "98");
    }

    #[test]
    fn five_or_more_digit_integers_get_separators() {
        assert_eq!(group_thousands("12345"), "12,345");
        assert_eq!(group_thousands("1284960"), "1,284,960");
        assert_eq!(group_thousands("$1284960"), "$1,284,960");
        assert_eq!(group_thousands("+120000"), "+120,000");
    }

    #[test]
    fn formatted_decimal_percent_and_unit_strings_are_left_alone() {
        for s in [
            "3.2x",
            "98%",
            "1,284,960",
            "12,345",
            "12345.5",
            "12345%",
            "12345 km",
            "2024Q3",
            "00123456",
            "",
            "ABC",
        ] {
            assert_eq!(group_thousands(s), s, "{s}");
        }
    }

    /// A counter built from the grouped text formats every intermediate value
    /// with separators, ending on exactly the static text.
    #[test]
    fn counter_intermediates_follow_the_grouped_text() {
        let text = group_thousands("1284960");
        let (prefix, value, decimals, suffix) = parse_number(&text).unwrap();
        let grouping = text.contains(',');
        assert!(grouping);
        let at = |p: f64| format_count(value * p, decimals, grouping, &prefix, &suffix);
        assert_eq!(at(0.0), "0");
        assert_eq!(at(0.5), "642,480");
        assert_eq!(at(0.001), "1,285");
        assert_eq!(at(1.0), text);
    }
}
