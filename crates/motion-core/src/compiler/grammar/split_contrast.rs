//! SplitContrast: two semantic states share the frame (atomic subjects).
//!
//! Two panels of contrasting tone (paper card vs accent) hold the primary and
//! secondary subjects, layout-bound to their panel so they follow its
//! geometry. Lifecycle: the primary side arrives in ENTER; at the first EVOLVE
//! event the second panel and the divider rule arrive (earlier, ahead of its
//! text, when a voice-over names the second subject before READ); at the second the
//! relationship plays (separate: the panels drift apart, replace: the primary
//! phrase TypeReplaces into the secondary one, carry: the primary panel
//! crosses the divider, compare: the second side is weighed); at the third a
//! serif connective word names the relation.
//!
//! Variants: LeftRight (two flush halves), TopBottom (two bands below the
//! headline, the connective sitting on the divider row), CenterOpposition (two
//! staggered panels facing each other around a central accent divider).

use super::kinetic_poster::replace_expansion;
use super::{Composition, Variant};
use crate::checks::TEXT_CONTRAST_DISPLAY;
use crate::compiler::explore::contrast_ratio;
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::speech_plan::{self, WORD_CUE_LEAD};
use crate::compiler::typeset::{text_layer, Block, Voice};
use crate::compiler::{
    base_layer, mo, rect_layer, subject_key, wants_carry, Carry, CompileError, Ctx, Which,
};
use crate::easing::Easing;
use crate::intent::{Relationship, Subject, SubjectKind};
use crate::motion::kinetic::{KineticParams, TextRun};
use crate::scene::{
    Color, Direction, HAlign, LayerKind, LayoutBinding, Padding, Stroke, TextAlign, VAlign,
};

/// How long before its text the second panel starts coming in when a word cue
/// pulls the text ahead of the panel's planned arrival (seconds).
const PANEL_BEFORE_TEXT: f64 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rel {
    Separate,
    Replace,
    Carry,
    Compare,
}

/// Panel rectangles `(x, y, w, h)` (top-left) and the divider geometry.
struct Geo {
    a: (f32, f32, f32, f32),
    b: (f32, f32, f32, f32),
    /// Panel offsets reached when the panels drift apart (separate).
    a_drift: [f32; 2],
    b_drift: [f32; 2],
    /// Offset that carries panel A across the divider (carry).
    cross: [f32; 2],
    divider: (f32, f32, f32, f32),
    divider_color: Color,
    a_mask: Direction,
    b_mask: Direction,
    divider_mask: Direction,
    /// Canvas y of the connective's top and its alignment / left edge.
    conn_y: f32,
    conn_align: TextAlign,
    /// Share of a panel's width available to its subject.
    text_share: f32,
}

pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let beat = b.beat;
    let life = plan.life;
    let lang = plan.lang;
    let p = lang.preset;
    let t = plan.enter_at();
    let ink = ctx.palette.ink;
    let rel = match beat.relationship {
        Some(Relationship::Separate) => Rel::Separate,
        Some(Relationship::Replace) => Rel::Replace,
        Some(Relationship::Carry) => Rel::Carry,
        _ => Rel::Compare,
    };
    // Nothing new may start at/after ANTICIPATE; every arrival ends inside the scene.
    let guard = (life.anticipate - 0.3)
        .min(plan.duration - 0.7)
        .max(life.enter);
    // A fixed-length motion starting at `at`, shortened to end inside the scene.
    let fit = |at: f64, dur: f64| dur.min(plan.duration - 0.05 - at).max(0.1);
    let events: Vec<f64> = life
        .evolve_events(3)
        .into_iter()
        .map(|e| e.min(guard))
        .collect();
    let (e0, e1, e2) = (events[0], events[1], events[2]);
    let rel_dur = (plan.duration - 0.05 - e1).clamp(0.3, 1.1);

    recipes::ghost(ctx, b, 0.55 * h);

    // ---- headline ------------------------------------------------------------
    let top = 0.17 * h;
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &beat.statement,
        w - 2.0 * m,
        0.14 * h,
        120.0 * u,
        3,
    );
    let (head_h, head_done) = recipes::headline_lines(
        ctx,
        b,
        "head",
        &head,
        top,
        TextAlign::Left,
        ink,
        t + 0.05,
        true,
    );
    let head_h = head_h + recipes::deck(ctx, b, top + head_h, head_done);

    // ---- geometry ---------------------------------------------------------------
    let word = match rel {
        Rel::Compare => "versus",
        Rel::Replace => "becomes",
        Rel::Carry => "still",
        Rel::Separate => "apart",
    };
    let conn = ctx.ts.fit_line(Voice::SERIF, word, w * 0.4, 64.0 * u);
    let row_h = conn.height() + 20.0 * u;
    let r_top = top + head_h + 48.0 * u;
    // Leaves room for the camera push and the anticipation lift.
    let r_bottom = 0.88 * h;
    let drift = if rel == Rel::Separate { 26.0 * u } else { 0.0 };
    let geo = geometry(ctx, c.variant, r_top, r_bottom, row_h, drift, &conn);

    // ---- panels -----------------------------------------------------------------
    let tone_a = ctx.palette.card;
    let tone_b = ctx.palette.accent;
    let panel = |id: String, r: (f32, f32, f32, f32), fill: Color, stroke: Option<Stroke>, z| {
        let mut l = base_layer(
            id,
            (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0, r.2, r.3),
            LayerKind::RoundedRectangle {
                fill,
                radius: 14.0 * u,
                stroke,
            },
            z,
        );
        l.anchor_x = 0.5;
        l.anchor_y = 0.5;
        l
    };
    let id_divider = b.id("divider");
    let id_a = b.id("panel_a");
    let id_b = b.id("panel_b");
    // The divider sits under both panels; a panel crossing it covers it.
    b.push(rect_layer(
        id_divider.clone(),
        geo.divider,
        geo.divider_color,
        11,
    ));
    b.push(panel(
        id_a.clone(),
        geo.a,
        tone_a,
        Some(Stroke {
            color: ink,
            width: 3.0 * u,
        }),
        if rel == Rel::Carry { 13 } else { 12 },
    ));
    b.push(panel(id_b.clone(), geo.b, tone_b, None, 12));
    // (0.23 A4) The second panel arrives at the first EVOLVE event, after READ.
    // With a voice-over the word cue (speech_plan, by name) pulls the second
    // subject's text forward to the word that names it, and no word names a
    // panel: the text, drawn in the on-accent colour, then reads on the bare
    // ground (white on the paper, 1.8:1) until the panel comes in. When the
    // narrator names the subject before READ, the panel (and the divider)
    // come in ahead of the text, as in the plan. Only where that text cannot
    // be read on the ground (a palette whose on-accent colour contrasts with
    // its paper needs no change).
    // (0.23) The second subject's texts are its value and its meaning (the
    // label under it): each is pulled to its own words, so the panel is timed
    // from the earliest of them ("start at 35" is said before "$120,000").
    let panel_b_at = {
        let on_ground =
            contrast_ratio(ctx.palette.on_accent, ctx.palette.paper) < TEXT_CONTRAST_DISPLAY as f32;
        // The words of the second subject's texts (a picture has none to read).
        let named: Vec<&str> = match beat.secondary.as_ref() {
            Some(s @ (Subject::Phrase(_) | Subject::Number(_))) => {
                s.value().into_iter().chain(s.meaning()).collect()
            }
            _ => Vec::new(),
        };
        match speech_plan::name_time(&ctx.spoken, &named) {
            Some(at) if on_ground && (at - WORD_CUE_LEAD).max(life.enter) < life.read => {
                let text_at = (at - WORD_CUE_LEAD).max(life.enter);
                e0.min((text_at - PANEL_BEFORE_TEXT).max(life.enter))
            }
            _ => e0,
        }
    };
    b.motions
        .push(mo::mask(&id_a, t + 0.25, 0.8, geo.a_mask, Easing::OutQuint));
    b.motions.push(mo::mask(
        &id_b,
        panel_b_at,
        fit(panel_b_at, 0.8),
        geo.b_mask,
        Easing::OutQuint,
    ));
    b.motions.push(mo::mask(
        &id_divider,
        panel_b_at,
        fit(panel_b_at, 0.7),
        geo.divider_mask,
        Easing::OutQuint,
    ));

    // ---- relationship (second EVOLVE event) --------------------------------------------
    match rel {
        Rel::Separate => {
            b.motions.push(mo::shift(
                &id_a,
                e1,
                rel_dur,
                [0.0, 0.0],
                geo.a_drift,
                Easing::InOutCubic,
            ));
            b.motions.push(mo::shift(
                &id_b,
                e1,
                rel_dur,
                [0.0, 0.0],
                geo.b_drift,
                Easing::InOutCubic,
            ));
        }
        Rel::Carry => {
            b.motions.push(mo::shift(
                &id_a,
                e1,
                rel_dur,
                [0.0, 0.0],
                geo.cross,
                Easing::InOutCubic,
            ));
        }
        Rel::Replace => {
            b.motions
                .push(mo::fade(&id_a, e1, rel_dur, 1.0, 0.4, Easing::InOutCubic));
            b.motions
                .push(mo::scale(&id_b, e1, rel_dur, 1.0, 1.035, lang.settle));
        }
        Rel::Compare => {
            b.motions
                .push(mo::scale(&id_b, e1, rel_dur, 1.0, 1.025, lang.settle));
        }
    }

    // ---- subjects ----------------------------------------------------------------
    let sec = beat.secondary.as_ref();
    let a_at = t + 0.45;
    let b_at = (if rel == Rel::Replace {
        e1 + 0.15
    } else {
        e0 + 0.2
    })
    .min(plan.duration - 0.05 - 1.05 * p.duration)
    .max(e0);
    let slot_for = |r: (f32, f32, f32, f32)| Slot {
        cx: r.0 + r.2 / 2.0,
        cy: r.1 + r.3 / 2.0,
        w: r.2 * geo.text_share,
        h: r.3 * 0.55,
    };
    let (slot_a, slot_b) = (slot_for(geo.a), slot_for(geo.b));

    let is_local = |carries: &[Carry], s: &Subject, which: Which| {
        let carried_in = carries
            .iter()
            .any(|c| c.open && c.key == subject_key(s) && c.last_beat + 1 == plan.index);
        let carried_out = wants_carry(beat, which) && !plan.is_last;
        !carried_in && !carried_out
    };
    let phrase_of = |s: &Subject| -> Option<String> {
        (s.kind() == SubjectKind::Phrase)
            .then(|| s.display_text())
            .flatten()
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty())
    };
    let run_texts = match (sec, rel) {
        (Some(s), Rel::Replace)
            if is_local(carries, &beat.primary, Which::Primary)
                && is_local(carries, s, Which::Secondary) =>
        {
            phrase_of(&beat.primary).zip(phrase_of(s))
        }
        _ => None,
    };

    if let Some((text_a, text_b)) = run_texts {
        // Phrase into phrase: the primary text type-replaces into the secondary's.
        let old = centered_run(ctx, b, "replace_old", &text_a, ink, slot_a);
        let new = centered_run(
            ctx,
            b,
            "replace_new",
            &text_b,
            ctx.palette.on_accent,
            slot_b,
        );
        let kp = KineticParams {
            preset: p,
            stagger: lang.stagger,
            u,
        };
        let exp = replace_expansion(&old, &new, a_at, e1, life.anticipate, plan.duration, &kp);
        recipes::absorb(b, exp, 22);
    } else {
        // Continuity-aware placement, bound to the panel.
        if let Some(mut layer) = recipes::place_subject(
            ctx,
            b,
            carries,
            Which::Primary,
            &beat.primary,
            slot_a,
            (0.0, 0.0),
            ink,
            a_at,
            Entrance::Rise,
            None,
            Some(id_a.as_str()),
            "side_a",
        )? {
            layer.z_index = 22;
            // Replace between non-phrase / carried subjects: fade-swap.
            if rel == Rel::Replace && e1 >= a_at + 0.6 * p.duration + 0.05 {
                b.motions.push(mo::fade(
                    &layer.id,
                    e1,
                    fit(e1, 0.45),
                    1.0,
                    0.0,
                    Easing::InOutCubic,
                ));
            }
            b.push(layer);
        }
        if let Some(sec) = sec {
            if let Some(mut layer) = recipes::place_subject(
                ctx,
                b,
                carries,
                Which::Secondary,
                sec,
                slot_b,
                (0.0, 0.0),
                ctx.palette.on_accent,
                b_at,
                Entrance::Rise,
                None,
                Some(id_b.as_str()),
                "side_b",
            )? {
                layer.z_index = 22;
                b.push(layer);
            }
        }
    }

    // Meaning labels (only where the value is what is displayed).
    let label = |ctx: &mut Ctx,
                 b: &mut B,
                 name: &str,
                 s: &Subject,
                 parent: &str,
                 color: Color,
                 at: f64,
                 dim_at: Option<f64>,
                 max_w: f32| {
        let (Some(_), Some(meaning)) = (s.value(), s.meaning()) else {
            return;
        };
        if s.kind() == SubjectKind::Object {
            return;
        }
        let block = ctx.ts.fit_line(Voice::LABEL, meaning, max_w, 26.0 * ctx.u);
        let id = b.id(name);
        let mut l = recipes::with_ink(
            ctx,
            text_layer(id.clone(), &block, color, TextAlign::Left),
            &block,
        );
        l.z_index = 22;
        l.layout = Some(LayoutBinding {
            parent: parent.to_string(),
            horizontal: HAlign::Center,
            vertical: VAlign::Top,
            offset: [0.0, 0.0],
            padding: Padding {
                top: 22.0 * ctx.u,
                ..Padding::default()
            },
        });
        b.motions
            .push(mo::fade(&id, at, fit(at, 0.5), 0.0, 1.0, Easing::OutCubic));
        if let Some(d) = dim_at {
            b.motions
                .push(mo::fade(&id, d, fit(d, 0.5), 1.0, 0.4, Easing::InOutCubic));
        }
        b.push(l);
    };
    let dim = (rel == Rel::Replace && e1 >= a_at + 0.15 + 0.5).then_some(e1);
    label(
        ctx,
        b,
        "label_a",
        &beat.primary,
        &id_a,
        ctx.palette.muted,
        a_at + 0.15,
        dim,
        geo.a.2 * geo.text_share,
    );
    if let Some(sec) = sec {
        label(
            ctx,
            b,
            "label_b",
            sec,
            &id_b,
            ctx.palette.on_accent.with_alpha(0xCC),
            b_at + 0.15,
            None,
            geo.b.2 * geo.text_share,
        );
    }

    // ---- connective (third EVOLVE event) --------------------------------------------------
    // Own fixed-length entrance (independent of the preset) so it always ends
    // inside the scene.
    let conn_id = b.id("connective.0");
    let mut conn_layer = text_layer(conn_id.clone(), &conn, ctx.palette.muted, TextAlign::Left);
    conn_layer.x = match geo.conn_align {
        TextAlign::Left => m,
        TextAlign::Center => (w - conn_layer.width) / 2.0,
        TextAlign::Right => w - m - conn_layer.width,
    };
    conn_layer.y = geo.conn_y;
    conn_layer.z_index = 24;
    b.motions.push(mo::fade(
        &conn_id,
        e2,
        fit(e2, 0.5),
        0.0,
        1.0,
        Easing::OutCubic,
    ));
    b.motions.push(mo::shift(
        &conn_id,
        e2,
        fit(e2, 0.6),
        [0.0, 14.0 * u],
        [0.0, 0.0],
        p.easing,
    ));
    b.push(conn_layer);

    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

/// Panel and divider geometry for a variant. `drift` is how far each panel
/// travels outward in the separate relationship (panels start inset by it, so
/// they end exactly on the margins).
fn geometry(
    ctx: &Ctx,
    variant: Variant,
    r_top: f32,
    r_bottom: f32,
    row_h: f32,
    drift: f32,
    conn: &Block,
) -> Geo {
    let (w, u, m) = (ctx.w, ctx.u, ctx.margin());
    let sw = w - 2.0 * m;
    match variant {
        Variant::TopBottom => {
            let g0 = row_h + 36.0 * u;
            let g1 = g0 + 2.0 * drift;
            let ph = ((r_bottom - r_top - g1) / 2.0).max(60.0 * u);
            let a = (m, r_top + drift, sw, ph);
            let b = (m, r_top + drift + ph + g0, sw, ph);
            let y_mid = r_top + ph + g1 / 2.0;
            let rule_x = m + conn.width() * 1.02 + 28.0 * u;
            Geo {
                a,
                b,
                a_drift: [0.0, -drift],
                b_drift: [0.0, drift],
                cross: [0.0, g1 / 2.0 + 4.0 * u],
                divider: (rule_x, y_mid - 2.5 * u, w - m - rule_x, 5.0 * u),
                divider_color: ctx.palette.accent,
                a_mask: Direction::Down,
                b_mask: Direction::Down,
                divider_mask: Direction::Right,
                conn_y: y_mid - conn.height() / 2.0,
                conn_align: TextAlign::Left,
                text_share: 0.8,
            }
        }
        Variant::CenterOpposition => {
            let g0 = 0.09 * w;
            let g1 = g0 + 2.0 * drift;
            let region_h = (r_bottom - row_h - r_top).max(120.0 * u);
            let pw = (sw - g1) / 2.0;
            let ph = 0.82 * region_h;
            let a = (m + drift, r_top, pw, ph);
            let b = (m + drift + pw + g0, r_top + region_h - ph, pw, ph);
            Geo {
                a,
                b,
                a_drift: [-drift, 0.0],
                b_drift: [drift, 0.0],
                cross: [g1 / 2.0 + 24.0 * u, 0.0],
                divider: (w / 2.0 - 5.0 * u, r_top, 10.0 * u, region_h),
                divider_color: ctx.palette.accent,
                a_mask: Direction::Down,
                b_mask: Direction::Up,
                divider_mask: Direction::Down,
                conn_y: r_top + region_h + 10.0 * u,
                conn_align: TextAlign::Center,
                text_share: 0.62,
            }
        }
        // LeftRight (and any non-split variant): two flush halves.
        _ => {
            let g0 = 28.0 * u;
            let g1 = g0 + 2.0 * drift;
            let region_h = (r_bottom - row_h - r_top).max(120.0 * u);
            let pw = (sw - g1) / 2.0;
            let a = (m + drift, r_top, pw, region_h);
            let b = (m + drift + pw + g0, r_top, pw, region_h);
            Geo {
                a,
                b,
                a_drift: [-drift, 0.0],
                b_drift: [drift, 0.0],
                cross: [g1 / 2.0 + 24.0 * u, 0.0],
                divider: (w / 2.0 - 2.0 * u, r_top, 4.0 * u, region_h),
                divider_color: ctx.palette.ink,
                a_mask: Direction::Right,
                b_mask: Direction::Left,
                divider_mask: Direction::Down,
                conn_y: r_top + region_h + 10.0 * u,
                conn_align: TextAlign::Center,
                text_share: 0.8,
            }
        }
    }
}

/// A phrase run centered in `slot`, set in the headline voice like a bound
/// subject (per-line centering).
fn centered_run(ctx: &Ctx, b: &B, name: &str, text: &str, color: Color, slot: Slot) -> TextRun {
    let block = ctx
        .ts
        .fit_block(Voice::HEADLINE, text, slot.w, slot.h, 200.0 * ctx.u, 3);
    let bw = block.width();
    let x = slot.cx - (bw * 1.02 + 2.0) / 2.0;
    let y = slot.cy - block.height() / 2.0;
    let mut run = ctx.ts.text_run(b.id(name), &block, color, [x, y], false);
    for (line, lw) in run.lines.iter_mut().zip(&block.line_widths) {
        let dx = (bw - lw) / 2.0;
        for unit in line.iter_mut() {
            unit.x += dx;
        }
    }
    run
}
