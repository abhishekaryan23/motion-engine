//! KineticText: one scene per word, so every `cut` is a true hard cut on a
//! frame. Words are measured and set as large as the frame allows; each
//! arrives with a per-glyph snap (letters rise a little, one after another,
//! on a critically damped spring) and then holds with a slow push. With
//! `dot`, one accent dot ends every word like a full stop and hops to the
//! next word's end on each cut: one element carried across every cut.

use motion_core::easing::Easing;
use motion_core::scene::{
    Fill, FontRole, GlyphOrder, GlyphPose, LayerKind, MotionOp, PathFillRule, Scene,
};

use crate::lower::{r6, Ctx, Track, Use};
use crate::program::{
    Component, ComponentBody, ExitStyle, KineticPreset, MotionProgram, Slot, Swatch, TypeStyle,
    Verb,
};
use crate::taste::{self, Shape, Spring};

pub(crate) struct Kinetic<'a> {
    pub preset: KineticPreset,
    pub words: &'a [String],
    pub stress: &'a [usize],
    pub slot: Slot,
    pub colour: Option<Swatch>,
    pub dot: bool,
    pub ground: Option<Swatch>,
    /// (v2.2) Type treatment: fill, outline, case.
    pub style: Option<&'a TypeStyle>,
}

/// The length of a kinetic text that closes the program with an `end`
/// (an `end_beat`) set and no other content on screen after that end: its
/// final word keeps the remaining time to the program's end instead of
/// being cut to nothing when the last cut lands on the end beat.
pub(crate) fn closing_length(p: &MotionProgram, c: &Component) -> Option<f64> {
    let end = c.end?;
    if end >= p.meta.duration - 1e-6 {
        return None;
    }
    let follows = p.components.iter().any(|o| {
        o.id != c.id
            && !matches!(
                o.body,
                ComponentBody::Backdrop { .. }
                    | ComponentBody::Field { .. }
                    | ComponentBody::Hud { .. }
            )
            && p.end_of(o) > end + 1e-6
    });
    (!follows).then(|| ((p.meta.duration - c.start) * 1e6).round() / 1e6)
}

/// Word `i`'s window `[start, end)` in component seconds and whether it is
/// flipped (ground and ink swapped).
pub(crate) fn segments(c: &Component, words: usize, length: f64) -> Vec<(f64, f64, bool)> {
    let mut cuts: Vec<(f64, bool)> = c
        .timeline
        .iter()
        .filter_map(|e| match e.verb {
            Verb::Cut { flip } => e.at.map(|t| (t, flip)),
            _ => None,
        })
        .collect();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut start = 0.0;
    let mut flipped = false;
    for i in 0..words {
        let next = cuts.get(i);
        let end = next.map_or(length, |c| c.0).min(length);
        out.push((start, end, flipped));
        match next {
            Some(&(t, flip)) => {
                start = t;
                flipped ^= flip;
            }
            None => break,
        }
    }
    out
}

/// One word as set: size, centre and (for `dot`) where its full stop sits.
struct Set {
    size: f32,
    centre: [f32; 2],
    dot: [f32; 2],
}

/// Full stop: a disc a little wider than the stem, after a small gap (em).
const DOT_EM: f32 = 0.2;
const GAP_EM: f32 = 0.07;

pub(crate) fn lower(cx: &Ctx, c: &Component, k: &Kinetic, z: i32, length: f64) -> Vec<Scene> {
    let segs = segments(c, k.words.len(), length);
    let exit = c.timeline.iter().find_map(|e| match e.verb {
        Verb::Exit { style } => e.at.map(|t| (t, style)),
        _ => None,
    });
    let poster = k.preset == KineticPreset::PosterCuts;
    let upper = crate::lower::type_upper(k.style, poster);
    let ls = if poster { -0.03 } else { -0.02 };
    let slot = taste::slot_center(k.slot, cx.aspect);
    let asked = k.colour.unwrap_or(Swatch::Ink);
    // The ground the words sit on: their own poster ground, else the
    // component's.
    let home = k.ground.map_or(cx.ground(&c.id), |s| cx.col.get(s));
    // Posters fill the frame (90 % of the width, at most 60 % of the
    // height); the other presets set a line.
    let max_size = match k.preset {
        KineticPreset::PosterCuts => cx.h * 0.6,
        // A centred dropped word is the shot's hero: near poster size;
        // beside another element it stays a line.
        KineticPreset::GlyphDrop if k.slot == Slot::Center => cx.h * 0.35,
        KineticPreset::GlyphDrop => cx.short * 0.24,
        KineticPreset::TypeOn => cx.short * 0.12,
    };
    // Lines stay inside their slot's measure (side slots are narrower).
    // The safe width, less the 1.05 push a poster word grows by.
    let push = if k.preset == KineticPreset::PosterCuts {
        1.05
    } else {
        1.0
    };
    let measure = (cx.w * 0.88 / push).min(taste::slot_width(k.slot, cx.aspect) * 1.07);
    let sets: Vec<Set> = k
        .words
        .iter()
        .take(segs.len())
        .map(|word| {
            let room = if k.dot {
                // Word, gap and dot share the 90 % measure.
                let w100 = cx.line_width(word, FontRole::Display, 100.0, ls, upper);
                measure * w100 / (w100 + (DOT_EM + GAP_EM) * 100.0)
            } else {
                measure
            };
            let size = cx.fit(word, FontRole::Display, ls, upper, room, max_size);
            let width = cx.line_width(word, FontRole::Display, size, ls, upper);
            let extra = if k.dot { (DOT_EM + GAP_EM) * size } else { 0.0 };
            let centre = [slot[0] - extra / 2.0, slot[1]];
            let cap = cx.ink_height(word, FontRole::Display, size, ls, upper);
            let d = DOT_EM * size;
            let dot = [
                centre[0] + width / 2.0 + GAP_EM * size + d / 2.0,
                slot[1] + cap / 2.0 - d / 2.0,
            ];
            Set { size, centre, dot }
        })
        .collect();

    // Hard cuts sit on frames: each word's scene starts at the frame
    // nearest its cut, written rounded down so that frame shows it.
    let on_frame = |t: f64| {
        let f = ((c.start + t) * cx.fps).round() / cx.fps;
        ((f * 1e6 - 1e-3).floor() / 1e6).max(0.0)
    };
    let mut scenes = Vec::new();
    for (i, &(s0, s1, flipped)) in segs.iter().enumerate() {
        let start = on_frame(s0);
        let end = if s1 >= length {
            r6(c.start + length)
        } else {
            on_frame(s1)
        };
        let len = r6(end - start);
        if len <= 0.0 {
            continue;
        }
        let word = &k.words[i];
        let set = &sets[i];
        // Roles on this word's ground: a flip makes the type colour the
        // ground and sets the type in the role that reads on it.
        let (mut ground, mut colour) = if poster && flipped {
            let g = cx.col.paint(home, asked, Use::Mark);
            (g, cx.col.roles(g).text(home, set.size))
        } else {
            (home, cx.col.paint(home, asked, Use::Text(set.size)))
        };
        if k.stress.contains(&i) {
            // A stressed word is set in the accent; when the accent hue
            // cannot carry type on this ground it becomes an accent block.
            let r = cx.col.roles(ground);
            let need = crate::palette::ink_need(set.size);
            if crate::palette::contrast(r.accent_text, ground) >= need && r.accent_text != r.ink
                || !poster
            {
                colour = r.text(r.accent_text, set.size);
            } else {
                ground = r.accent;
                colour = cx.col.roles(r.accent).ink;
            }
        }
        let base = format!("{}.w{i}", c.id);
        let text_id = format!("{base}.text");
        let mut layers = Vec::new();
        if poster {
            layers.push(crate::lower::layer(
                &format!("{base}.ground"),
                LayerKind::Rectangle {
                    fill: ground,
                    stroke: None,
                },
                [cx.w / 2.0, cx.h / 2.0],
                [cx.w, cx.h],
                z,
            ));
        }
        let mut text = cx.text_canvas(
            &text_id,
            word,
            FontRole::Display,
            set.size,
            colour,
            ls,
            upper,
            set.centre,
            z + 1,
        );
        crate::lower::apply_type_style(&cx.col, ground, &mut text, k.style);
        layers.push(text);
        let mut t = Track::default();
        let n = word.chars().filter(|ch| !ch.is_whitespace()).count().max(1) as f64;
        match k.preset {
            KineticPreset::PosterCuts => {
                // Letters snap up into place, then the word holds and pushes.
                let stagger = 0.012;
                let d = (0.22 + (n - 1.0) * stagger).min(len);
                t.push(
                    &text_id,
                    0.0,
                    d,
                    Shape::Spring(Spring::Snap),
                    MotionOp::GlyphCascade {
                        stagger,
                        // Position only: a hard cut shows solid type on
                        // its first frame; letters settle up into place.
                        from: GlyphPose {
                            dy: 0.1 * set.size,
                            ..GlyphPose::REST
                        },
                        order: GlyphOrder::Forward,
                        seed: 0,
                    },
                );
                if len >= 0.2 {
                    t.push(
                        &text_id,
                        0.0,
                        len,
                        Shape::Ease(Easing::OutCubic),
                        MotionOp::Scale {
                            from: 1.0,
                            to: 1.05,
                            axis: Default::default(),
                        },
                    );
                }
            }
            KineticPreset::GlyphDrop => {
                // The word lands whole within the first half of its shot
                // (ledger `partial_typing`): a short shot drops faster.
                let budget = (0.45 * len).min(0.4 + (n - 1.0) * 0.035);
                let base = (0.6 * budget).min(0.4);
                let stagger = if n > 1.0 {
                    ((budget - base) / (n - 1.0)).clamp(0.0, 0.035)
                } else {
                    0.0
                };
                let d = r6(base + (n - 1.0) * stagger).max(1.0 / cx.fps);
                t.push(
                    &text_id,
                    0.0,
                    d,
                    Shape::Spring(Spring::Pop),
                    MotionOp::GlyphCascade {
                        stagger,
                        from: GlyphPose {
                            dy: -0.45 * set.size,
                            opacity: 0.0,
                            ..GlyphPose::REST
                        },
                        order: GlyphOrder::Forward,
                        seed: 0,
                    },
                );
            }
            KineticPreset::TypeOn => {
                // Fast enough that a word is whole within a fifth of a second.
                let stagger = r6((0.2 / n.max(1.0)).min(0.035));
                let d = ((n - 1.0) * stagger + 1.0 / cx.fps).min(len);
                t.push(
                    &text_id,
                    0.0,
                    d,
                    Shape::Ease(Easing::Linear),
                    MotionOp::GlyphCascade {
                        stagger,
                        from: GlyphPose {
                            opacity: 0.0,
                            ..GlyphPose::REST
                        },
                        order: GlyphOrder::Forward,
                        seed: 0,
                    },
                );
            }
        }
        if let Some((at, style)) = exit {
            if at >= s0 && at < s1 {
                let d = taste::duration(&Verb::Exit { style }, cx.tempo);
                let local = r6(at - s0);
                // A poster word already pushes on Scale: it fades instead.
                let op = match style {
                    ExitStyle::Shrink if !poster => MotionOp::Scale {
                        from: 1.0,
                        to: 0.0,
                        axis: Default::default(),
                    },
                    _ => MotionOp::Fade { from: 1.0, to: 0.0 },
                };
                t.push(
                    &text_id,
                    local,
                    d.min(len - local),
                    Shape::Ease(Easing::InCubic),
                    op,
                );
            }
        }
        scenes.push(crate::lower::scene(&base, start, len, layers, t.motions));
    }

    if k.dot && !sets.is_empty() {
        scenes.push(dot_scene(cx, c, &segs, &sets, exit, z + 2, length));
    }
    scenes
}

/// The carried full stop: pops in with the first word, then before every
/// cut hops on an arc so it lands on the next word's end exactly on the cut.
fn dot_scene(
    cx: &Ctx,
    c: &Component,
    segs: &[(f64, f64, bool)],
    sets: &[Set],
    exit: Option<(f64, ExitStyle)>,
    z: i32,
    length: f64,
) -> Scene {
    let id = format!("{}.dot", c.id);
    let d0 = (sets[0].size * DOT_EM).round();
    let l = crate::lower::layer(
        &id,
        LayerKind::Path {
            d: motion_core::path::ellipse(0.0, 0.0, d0, d0).to_svg(),
            fill: Some(Fill::Solid(cx.roles(&c.id).accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        sets[0].dot,
        [d0, d0],
        z,
    );
    let mut t = Track::default();
    // Pops in with the first word, done before the first hop starts.
    let first_hop = segs
        .get(1)
        .map_or(f64::MAX, |s| s.0 - 0.2f64.min((s.0 - segs[0].0) * 0.8));
    let pop = (first_hop - segs[0].0).clamp(1.0 / cx.fps, 0.3);
    t.push(
        &id,
        segs[0].0,
        r6(pop),
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.0,
            to: 1.0,
            axis: Default::default(),
        },
    );
    let home = sets[0].dot;
    let mut prev = [0.0f32, 0.0];
    // Each word pushes 1.0 -> 1.05 about its centre: the dot rides that
    // growth so it never touches the last letter.
    let ride = |i: usize, prev: [f32; 2]| {
        let w = sets[i].dot[0] - sets[i].centre[0];
        let h = sets[i].dot[1] - sets[i].centre[1];
        [prev[0] + w * 0.05, prev[1] + h * 0.05]
    };
    let mut scale_from = 1.0f32;
    for i in 1..segs.len().min(sets.len()) {
        let cut = segs[i].0;
        let room = cut - segs[i - 1].0;
        let hop = r6(0.2f64.min(room * 0.8));
        // Ride word i-1's push until the hop leaves.
        let pushed = ride(i - 1, prev);
        let seg = cut - hop - segs[i - 1].0;
        if seg > 0.05 {
            t.push(
                &id,
                segs[i - 1].0,
                r6(seg),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Move {
                    from: prev,
                    to: pushed,
                    via: None,
                },
            );
            prev = pushed;
        }
        let to = [sets[i].dot[0] - home[0], sets[i].dot[1] - home[1]];
        let lift = (sets[i - 1].size.min(sets[i].size) * 0.35).max(cx.short * 0.04);
        let via = [(prev[0] + to[0]) / 2.0, prev[1].min(to[1]) - lift];
        t.push(
            &id,
            cut - hop,
            hop,
            Shape::Ease(Easing::InOutCubic),
            MotionOp::Move {
                from: prev,
                to,
                via: Some(via),
            },
        );
        // The dot keeps its size relative to each word.
        let k = sets[i].size / sets[0].size;
        if (k - scale_from).abs() > 1e-3 {
            t.push(
                &id,
                cut - hop,
                hop,
                Shape::Ease(Easing::InOutCubic),
                MotionOp::Scale {
                    from: scale_from,
                    to: k,
                    axis: Default::default(),
                },
            );
        }
        scale_from = k;
        prev = to;
    }
    if let Some((at, style)) = exit {
        let d = taste::duration(&Verb::Exit { style }, cx.tempo);
        t.push(
            &id,
            at,
            d.min(length - at),
            Shape::Ease(Easing::InCubic),
            MotionOp::Fade { from: 1.0, to: 0.0 },
        );
    }
    crate::lower::scene(&id, c.start, length, vec![l], t.motions)
}
