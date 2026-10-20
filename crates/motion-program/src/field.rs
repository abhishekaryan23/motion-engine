//! Field: one instanced dot over the whole frame. Instances are ordered by
//! distance from the centre so `enter: draw` (a trim of the list) grows the
//! field outward; recolour flips it; nothing drifts or pulses.

use motion_core::easing::Easing;
use motion_core::scene::{Fill, LayerKind, MotionOp, Scene};

use crate::lower::{Ctx, Track};
use crate::program::{Component, EnterStyle, FieldPreset, Slot, Swatch, Verb};
use crate::taste::{self, Shape};

/// Unit circle centred on the origin.
const DOT: &str = "M-1 0A1 1 0 1 1 1 0A1 1 0 1 1 -1 0Z";
/// Unit rounded square centred on the origin.
const TILE: &str = "M-0.7 -1H0.7Q1 -1 1 -0.7V0.7Q1 1 0.7 1H-0.7Q-1 1 -1 0.7V-0.7Q-1 -1 -0.7 -1Z";

/// Dot placements, nearest `c` first (the halftone's largest dots are at
/// `c`, its focus).
pub(crate) fn instances(
    preset: FieldPreset,
    w: f32,
    h: f32,
    short: f32,
    c: [f32; 2],
) -> Vec<[f32; 3]> {
    // Distance from the focus to the farthest corner.
    let reach = [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]]
        .iter()
        .map(|q: &[f32; 2]| (q[0] - c[0]).hypot(q[1] - c[1]))
        .fold(0.0f32, f32::max);
    let mut out: Vec<[f32; 3]> = Vec::new();
    match preset {
        FieldPreset::Halftone | FieldPreset::Grid | FieldPreset::Tiles => {
            let step = short
                * match preset {
                    FieldPreset::Halftone => 0.045,
                    FieldPreset::Tiles => 0.085,
                    _ => 0.06,
                };
            let cols = (w / step).floor() as i32;
            let rows = (h / step).floor() as i32;
            let x0 = (w - (cols - 1) as f32 * step) / 2.0;
            let y0 = (h - (rows - 1) as f32 * step) / 2.0;
            for r in 0..rows {
                for k in 0..cols {
                    let (x, y) = (x0 + k as f32 * step, y0 + r as f32 * step);
                    let d = ((x - c[0]).powi(2) + (y - c[1]).powi(2)).sqrt() / reach;
                    let s = match preset {
                        FieldPreset::Halftone => step * 0.42 * (1.0 - d).max(0.0).powf(1.3),
                        FieldPreset::Tiles => step * 0.36,
                        _ => step * 0.09,
                    };
                    if s >= 0.6 {
                        out.push([x, y, s]);
                    }
                }
            }
        }
        FieldPreset::Rings => {
            let gap = short * 0.07;
            let rings = (reach / gap).ceil() as i32;
            for ring in 1..=rings {
                let rad = ring as f32 * gap;
                let count = ((std::f32::consts::TAU * rad) / (gap * 0.6)).round() as i32;
                for k in 0..count {
                    let a = std::f32::consts::TAU * k as f32 / count as f32;
                    let (x, y) = (c[0] + rad * a.cos(), c[1] + rad * a.sin());
                    if x >= -gap && x <= w + gap && y >= -gap && y <= h + gap {
                        out.push([x, y, short * 0.005]);
                    }
                }
            }
        }
    }
    // Centre outward; ties by position for a stable order.
    out.sort_by(|a, b| {
        let da = (a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2);
        let db = (b[0] - c[0]).powi(2) + (b[1] - c[1]).powi(2);
        da.total_cmp(&db)
            .then(a[1].total_cmp(&b[1]))
            .then(a[0].total_cmp(&b[0]))
    });
    out.iter().map(|p| [r2(p[0]), r2(p[1]), r2(p[2])]).collect()
}

fn r2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

pub(crate) fn lower(
    cx: &Ctx,
    c: &Component,
    preset: FieldPreset,
    focus: Option<Slot>,
    colour: Option<Swatch>,
    z: i32,
    length: f64,
) -> Scene {
    let focus = crate::taste::slot_center(focus.unwrap_or_default(), cx.aspect);
    let id = c.id.as_str();
    // A field is texture: a shade/tint of the ground it sits on, never
    // the ink. A named swatch only lends its hue.
    let roles = cx.roles(&c.id);
    let fill = match colour {
        None | Some(Swatch::Muted) | Some(Swatch::Ink) | Some(Swatch::Background) => roles.texture,
        Some(s) => roles.texture_in(cx.col.get(s)),
    };
    let proto = if preset == FieldPreset::Tiles {
        TILE
    } else {
        DOT
    };
    let at = instances(preset, cx.w, cx.h, cx.short, focus);
    let mut l = crate::lower::layer(
        id,
        LayerKind::Instances {
            d: proto.to_string(),
            fill: Fill::Solid(fill),
            at: at.clone(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [cx.w, cx.h],
        z,
    );
    l.anchor_x = 0.5;
    let mut t = Track::default();
    for e in &c.timeline {
        let Some(at) = e.at else { continue };
        let d = taste::duration(&e.verb, cx.tempo);
        let (shape, op) = match &e.verb {
            Verb::Enter {
                style: EnterStyle::Draw,
            } => (
                Shape::Ease(Easing::OutCubic),
                MotionOp::Trim { from: 0.2, to: 1.0 },
            ),
            Verb::Enter { .. } => (
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            ),
            Verb::Exit { .. } => (
                Shape::Ease(Easing::InCubic),
                MotionOp::Fade { from: 1.0, to: 0.0 },
            ),
            Verb::Recolour { to } => (
                Shape::Ease(Easing::OutCubic),
                MotionOp::Tint {
                    color: roles.texture_in(cx.col.get(*to)),
                    from: 0.0,
                    to: 1.0,
                },
            ),
            _ => continue,
        };
        t.push(id, at, d, shape, op);
    }
    // Waves: each is a copy of the field in the new colour that builds out
    // from the focus over the one below (later waves on top).
    let mut layers = vec![l];
    for (k, e) in c
        .timeline
        .iter()
        .filter(|e| matches!(e.verb, Verb::Wave { .. }))
        .enumerate()
    {
        let (Some(at_t), Verb::Wave { to }) = (e.at, &e.verb) else {
            continue;
        };
        let wid = format!("{id}.wave{k}");
        let mut w = crate::lower::layer(
            &wid,
            LayerKind::Instances {
                d: proto.to_string(),
                fill: Fill::Solid(roles.texture_in(cx.col.get(*to))),
                at: at.clone(),
            },
            [cx.w / 2.0, cx.h / 2.0],
            [cx.w, cx.h],
            z,
        );
        w.anchor_x = 0.5;
        t.push(
            &wid,
            at_t,
            crate::taste::duration(&e.verb, cx.tempo),
            Shape::Ease(Easing::InOutCubic),
            MotionOp::Trim { from: 0.0, to: 1.0 },
        );
        layers.push(w);
    }
    // A field exit fades every wave layer too.
    if let Some((at_t, d)) = c.timeline.iter().find_map(|e| match e.verb {
        Verb::Exit { .. } => e.at.map(|a| (a, crate::taste::duration(&e.verb, cx.tempo))),
        _ => None,
    }) {
        for lw in layers.iter().skip(1) {
            t.push(
                &lw.id,
                at_t,
                d,
                Shape::Ease(Easing::InCubic),
                MotionOp::Fade { from: 1.0, to: 0.0 },
            );
        }
    }
    crate::lower::scene(id, c.start, length, layers, t.motions)
}

/// Corner labels: tiny tracked mono caps in the muted swatch, inset from
/// the frame edge, fading in one after another.
pub(crate) fn hud(cx: &Ctx, c: &Component, corners: &[String], z: i32, length: f64) -> Scene {
    use motion_core::scene::FontRole;
    let fs = (cx.short * 0.02).round().max(14.0);
    let m = (cx.short * 0.05).round();
    let ls = 0.14;
    let mut layers = Vec::new();
    let mut t = Track::default();
    let enter = c.timeline.iter().find_map(|e| match e.verb {
        Verb::Enter { .. } => e.at,
        _ => None,
    });
    let exit = c.timeline.iter().find_map(|e| match e.verb {
        Verb::Exit { .. } => e.at,
        _ => None,
    });
    for (i, text) in corners.iter().enumerate().take(4) {
        if text.trim().is_empty() {
            continue;
        }
        let w = cx.line_width(text, FontRole::Mono, fs, ls, true);
        let x = if i % 2 == 0 {
            m + w / 2.0
        } else {
            cx.w - m - w / 2.0
        };
        let y = if i < 2 {
            m + fs / 2.0
        } else {
            cx.h - m - fs / 2.0
        };
        let id = format!("{}.c{i}", c.id);
        layers.push(cx.text_canvas(
            &id,
            text,
            FontRole::Mono,
            fs,
            cx.paint(&c.id, Swatch::Muted, crate::lower::Use::Text(fs)),
            ls,
            true,
            [x, y],
            z,
        ));
        if let Some(at) = enter {
            t.push(
                &id,
                at + 0.06 * i as f64,
                0.4,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }
        if let Some(at) = exit {
            t.push(
                &id,
                at,
                0.35f64.min(length - at),
                Shape::Ease(Easing::InCubic),
                MotionOp::Fade { from: 1.0, to: 0.0 },
            );
        }
    }
    crate::lower::scene(&c.id, c.start, length, layers, t.motions)
}
