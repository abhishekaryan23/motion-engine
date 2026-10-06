//! KineticPoster: typography itself is the dominant visual.
//!
//! An oversized display headline (the statement) fills most of the frame; a
//! serif contrast line carries the primary phrase (arriving as a tracking
//! reveal once the headline has landed), a second serif line the secondary.
//! In EVOLVE the beat keyword inside the headline is manipulated
//! (TypeScaleEmphasis / KeywordPunch) and, for a `replace` between two
//! phrases, the primary phrase TypeReplaces into the secondary one.
//!
//! Standard sets the headline left and the serif voice right; Mirror flips
//! both sides. Subjects go through `place_subject` whenever continuity needs
//! them (carried in or out); plain phrases are set as kinetic runs instead.

use super::{Composition, Variant};
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::typeset::{text_layer, Block, Voice};
use crate::compiler::{subject_key, wants_carry, Carry, CompileError, Ctx, Which};
use crate::intent::{Relationship, Subject, SubjectKind};
use crate::motion::kinetic::{self, KineticParams, TextRun};
use crate::motion::language::{EmphasisMotion, HeadlineMotion};
use crate::motion::Expansion;
use crate::scene::{Color, MotionOp, TextAlign};

/// Longest single-line phrase (characters) revealed glyph by glyph.
const TRACKING_MAX_CHARS: usize = 36;

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
    let kp = KineticParams {
        preset: p,
        stagger: lang.stagger,
        u,
    };
    let mirror = c.variant == Variant::Mirror;
    let (head_align, serif_align) = if mirror {
        (TextAlign::Right, TextAlign::Left)
    } else {
        (TextAlign::Left, TextAlign::Right)
    };
    let ink = ctx.palette.ink;
    // Nothing new may start at/after ANTICIPATE, and every arrival must end
    // inside the scene.
    let guard = (life.anticipate - 0.3)
        .min(plan.duration - 0.05 - 1.3 * p.duration)
        .max(life.enter);

    ghost_and_anchor(ctx, b);

    // ---- vertical budget ---------------------------------------------------
    let top0 = 0.17 * h;
    let bottom = 0.93 * h;
    let gap = 0.025 * h;
    let sw = w - 2.0 * m;
    let sec = beat.secondary.as_ref();
    let prim_h = if beat.primary.kind() == SubjectKind::Phrase {
        0.15 * h
    } else {
        0.16 * h
    };
    let sec_h = sec.map_or(0.0, |s| {
        if s.kind() == SubjectKind::Phrase {
            0.11 * h
        } else {
            0.16 * h
        }
    });
    // (0.10 Q) The body copy of a long statement without a voice-over sits
    // under the headline and takes its share of the vertical budget.
    let deck = recipes::deck_block(ctx, b);
    let deck_h = deck.as_ref().map_or(0.0, |d| recipes::deck_space(ctx, d));
    let reserve = prim_h + gap + if sec.is_some() { sec_h + gap } else { 0.0 } + deck_h;
    let head_max_h = (bottom - top0 - reserve).clamp(0.2 * h, 0.5 * h);

    // ---- headline ------------------------------------------------------------
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &beat.statement,
        sw,
        head_max_h,
        230.0 * u,
        5,
    );
    // Spare height is distributed above the block (poster block sits low-centre).
    let content = head.height() + deck_h + gap + prim_h + sec.map_or(0.0, |_| gap + sec_h);
    let top = top0 + ((bottom - top0) - content).max(0.0) * 0.45;
    let head_start = t + 0.05;
    let head_run = (lang.typography.headline == HeadlineMotion::WordCascade)
        .then(|| aligned_run(ctx, b, "head", &head, ink, head_align, top, false));
    let keyword = head_run.as_ref().and_then(|r| {
        beat.keyword
            .as_deref()
            .and_then(|k| recipes::find_word(r, k))
    });

    // EVOLVE events: keyword manipulation first, then the secondary phrase.
    let n_evo = keyword.is_some() as usize + sec.is_some() as usize;
    let events = life.evolve_events(n_evo.max(1));
    let ev = |i: usize| events.get(i).copied().unwrap_or(life.evolve).min(guard);
    let sec_event = ev(keyword.is_some() as usize);

    match &head_run {
        Some(run) => {
            let exp = keyword
                .and_then(|k| {
                    keyword_expansion(
                        run,
                        k,
                        head_start,
                        ev(0),
                        life.anticipate,
                        plan.duration - 0.05,
                        ctx.palette.accent,
                        lang.typography.emphasis,
                        &kp,
                    )
                })
                .unwrap_or_else(|| kinetic::word_cascade(run, head_start, &kp));
            let exp = match keyword {
                Some(k) => recipes::fit_emphasis_scale(ctx, run, k, exp),
                None => exp,
            };
            recipes::absorb(b, exp, 20);
        }
        None => {
            recipes::headline_lines(
                ctx, b, "head", &head, top, head_align, ink, head_start, true,
            );
        }
    }
    let head_h = head.height();
    // The primary lands at the end of ENTER, after the headline has started.
    let prim_at = (life.settle - 0.35).max(life.enter + 0.25);
    if let Some(deck) = &deck {
        recipes::deck_lines(ctx, b, deck, top + head_h, prim_at);
    }

    // ---- primary serif line -------------------------------------------------
    let y_prim = top + head_h + deck_h + gap;
    let y_sec = y_prim + prim_h + gap;

    let is_local = |carries: &[Carry], s: &Subject, which: Which| {
        let carried_in = carries
            .iter()
            .any(|c| c.open && c.key == subject_key(s) && c.last_beat + 1 == plan.index);
        let carried_out = wants_carry(beat, which) && !plan.is_last;
        !carried_in && !carried_out
    };
    let phrase = |s: &Subject| -> Option<String> {
        (s.kind() == SubjectKind::Phrase)
            .then(|| s.display_text())
            .flatten()
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty())
    };
    let prim_text =
        phrase(&beat.primary).filter(|_| is_local(carries, &beat.primary, Which::Primary));
    let sec_text = sec
        .and_then(phrase)
        .filter(|_| sec.is_some_and(|s| is_local(carries, s, Which::Secondary)));
    let replace_runs = beat.relationship == Some(Relationship::Replace)
        && prim_text.is_some()
        && sec_text.is_some();

    let prim_block = phrase(&beat.primary).map(|text| {
        ctx.ts
            .fit_block(Voice::SERIF, &text, sw, prim_h, 130.0 * u, 2)
    });
    let mut rule_width = prim_block.as_ref().map_or(0.0, Block::width);
    let mut rule_y = y_prim + prim_block.as_ref().map_or(0.0, Block::height) + 12.0 * u;

    match (&prim_text, &prim_block) {
        (Some(_), Some(block)) if !replace_runs => {
            // Local phrase: tracking reveal (single short line) or word cascade.
            let glyphs =
                block.lines.len() == 1 && block.lines[0].chars().count() <= TRACKING_MAX_CHARS;
            let run = aligned_run(ctx, b, "serif", block, ink, serif_align, y_prim, glyphs);
            let exp = if glyphs {
                kinetic::tracking_reveal(&run, prim_at, &kp)
            } else {
                kinetic::word_cascade(&run, prim_at, &kp)
            };
            recipes::absorb(b, exp, 22);
        }
        (Some(_), Some(_)) => {} // replace: both runs are built with the secondary below.
        _ => {
            // Number / carried phrase: continuity-aware placement.
            let (slot, block) = subject_slot(
                &beat.primary,
                prim_block.as_ref(),
                serif_align,
                y_prim,
                prim_h,
                sw,
                m,
                w,
            );
            placed_line(
                ctx,
                b,
                carries,
                Which::Primary,
                &beat.primary,
                slot,
                block,
                serif_align,
                y_prim,
                ink,
                prim_at,
                "poster",
            )?;
            if beat.primary.kind() == SubjectKind::Number {
                rule_width = slot.w * 0.5;
                rule_y = y_prim + prim_h + 4.0 * u;
            }
        }
    }

    // ---- secondary ---------------------------------------------------------------
    if let Some(sec) = sec {
        let sec_block = phrase(sec).map(|text| {
            ctx.ts
                .fit_block(Voice::SERIF, &text, sw, sec_h, 96.0 * u, 2)
        });
        if replace_runs {
            if let (Some(a), Some(text)) = (&prim_block, &sec_text) {
                let new_block = ctx
                    .ts
                    .fit_block(Voice::SERIF, text, sw, prim_h, 130.0 * u, 2);
                rule_width = rule_width.max(new_block.width());
                let old = aligned_run(ctx, b, "serif", a, ink, serif_align, y_prim, false);
                let new = aligned_run(
                    ctx,
                    b,
                    "serif_next",
                    &new_block,
                    ctx.palette.accent,
                    serif_align,
                    y_prim,
                    false,
                );
                let exp = replace_expansion(
                    &old,
                    &new,
                    prim_at,
                    sec_event,
                    life.anticipate,
                    plan.duration,
                    &kp,
                );
                recipes::absorb(b, exp, 22);
            }
        } else if let (Some(_), Some(block)) = (&sec_text, &sec_block) {
            recipes::headline_lines(
                ctx,
                b,
                "support",
                block,
                y_sec,
                serif_align,
                ctx.palette.muted,
                sec_event,
                false,
            );
        } else {
            let (slot, block) =
                subject_slot(sec, sec_block.as_ref(), serif_align, y_sec, sec_h, sw, m, w);
            placed_line(
                ctx,
                b,
                carries,
                Which::Secondary,
                sec,
                slot,
                block,
                serif_align,
                y_sec,
                ctx.palette.muted,
                sec_event,
                "support",
            )?;
        }
    }

    // ---- accent rule under the primary line (draws once the line has landed) ----
    if prim_block.is_some() || beat.primary.kind() == SubjectKind::Number {
        let rw = rule_width.clamp(60.0 * u, sw);
        let x = match serif_align {
            TextAlign::Right => w - m - rw,
            _ => m,
        };
        recipes::underline(
            ctx,
            b,
            "serif_rule",
            (x, rule_y, rw, 9.0 * u),
            ctx.palette.accent,
            // With nothing else to evolve, the rule completing is the EVOLVE step.
            if n_evo == 0 {
                ev(0).max(prim_at + 0.5)
            } else {
                prim_at + 0.5
            },
        );
    }
    Ok(())
}

/// Ghost keyword behind the type and the parking slot for carried-in subjects.
fn ghost_and_anchor(ctx: &Ctx, b: &mut B) {
    recipes::ghost(ctx, b, 0.6 * ctx.h);
    b.anchor = Some(Slot {
        cx: ctx.w - ctx.margin() - 130.0 * ctx.u,
        cy: 0.115 * ctx.h,
        w: 260.0 * ctx.u,
        h: 64.0 * ctx.u,
    });
}

/// The old run entering at `enter`, then (when it still fits before ANTICIPATE)
/// type-replaced by the new run at `want` or as late as still fits. Shared by
/// the split-contrast replace.
pub(super) fn replace_expansion(
    old: &TextRun,
    new: &TextRun,
    enter: f64,
    want: f64,
    anticipate: f64,
    duration: f64,
    kp: &KineticParams,
) -> Expansion {
    let mut exp = kinetic::word_cascade(old, enter, kp);
    let entered = exp
        .motions
        .iter()
        .map(|m| m.start + m.duration)
        .fold(enter, f64::max);
    let earliest = entered + 0.08;
    // How long the swap keeps starting / running motions after `at`
    // (stagger + arrival): nothing may start at ANTICIPATE or end after the scene.
    let probe = kinetic::type_replace(old, new, earliest, kp);
    let span_start = probe
        .motions
        .iter()
        .map(|m| m.start)
        .fold(earliest, f64::max)
        - earliest;
    let span_end = probe
        .motions
        .iter()
        .map(|m| m.start + m.duration)
        .fold(earliest, f64::max)
        - earliest;
    let at = want
        .max(earliest)
        .min(anticipate - span_start - 0.05)
        .min(duration - span_end - 0.05);
    if at >= earliest {
        merge(&mut exp, kinetic::type_replace(old, new, at, kp));
    }
    exp
}

/// Merge `other` into `into`, keeping the first layer for a repeated id (the
/// old run of a type-replace is already present with its entrance).
fn merge(into: &mut Expansion, other: Expansion) {
    for l in other.layers {
        if !into.layers.iter().any(|x| x.id == l.id) {
            into.layers.push(l);
        }
    }
    into.motions.extend(other.motions);
}

/// Left edge (canvas x) of a block set with `align` inside the margins.
fn block_x(align: TextAlign, block: &Block, w: f32, m: f32) -> f32 {
    let width = block.width() * 1.02 + 2.0;
    match align {
        TextAlign::Left => m,
        TextAlign::Center => (w - width) / 2.0,
        TextAlign::Right => w - m - width,
    }
}

/// A measured run of `block` set with `align` (per-line offsets for right/center).
#[allow(clippy::too_many_arguments)]
fn aligned_run(
    ctx: &Ctx,
    b: &B,
    name: &str,
    block: &Block,
    color: Color,
    align: TextAlign,
    y: f32,
    glyphs: bool,
) -> TextRun {
    let x = block_x(align, block, ctx.w, ctx.margin());
    let bw = block.width();
    let mut run = ctx.ts.text_run(b.id(name), block, color, [x, y], glyphs);
    if align != TextAlign::Left {
        for (line, lw) in run.lines.iter_mut().zip(&block.line_widths) {
            let dx = match align {
                TextAlign::Center => (bw - lw) / 2.0,
                _ => bw - lw,
            };
            for unit in line.iter_mut() {
                unit.x += dx;
            }
        }
    }
    run
}

/// Keyword manipulation at `at` (EVOLVE). `None` when it cannot start before
/// ANTICIPATE (the caller falls back to the plain cascade).
#[allow(clippy::too_many_arguments)]
fn keyword_expansion(
    run: &TextRun,
    keyword: (usize, usize),
    start: f64,
    at: f64,
    limit: f64,
    end_limit: f64,
    accent: Color,
    emphasis: EmphasisMotion,
    kp: &KineticParams,
) -> Option<Expansion> {
    let mut exp = match emphasis {
        EmphasisMotion::KeywordPunch => kinetic::keyword_punch(run, keyword, start, at, accent, kp),
        _ => kinetic::type_scale_emphasis(run, keyword, start, at, kp),
    };
    if emphasis != EmphasisMotion::KeywordPunch {
        // Dimming a unit that would only start after ANTICIPATE is optional.
        exp.motions.retain(|m| {
            (m.start < limit && m.start + m.duration <= end_limit)
                || !matches!(m.op, MotionOp::Fade { .. })
        });
    }
    exp.motions
        .iter()
        .all(|m| m.start < limit && m.start + m.duration <= end_limit)
        .then_some(exp)
}

/// Slot (and serif block when the subject is a phrase) for a continuity-aware
/// subject on the poster's serif side.
#[allow(clippy::too_many_arguments)]
fn subject_slot<'a>(
    subject: &Subject,
    block: Option<&'a Block>,
    align: TextAlign,
    y: f32,
    band_h: f32,
    sw: f32,
    m: f32,
    w: f32,
) -> (Slot, Option<&'a Block>) {
    if subject.kind() == SubjectKind::Phrase {
        let slot = Slot {
            cx: w / 2.0,
            cy: y + band_h / 2.0,
            w: sw,
            h: band_h,
        };
        return (slot, block);
    }
    let side = (sw * 0.5).min(band_h * 2.4);
    let cx = match align {
        TextAlign::Right => w - m - side / 2.0,
        _ => m + side / 2.0,
    };
    (
        Slot {
            cx,
            cy: y + band_h / 2.0,
            w: side,
            h: band_h,
        },
        None,
    )
}

/// Place a subject through `place_subject` and push the resulting layer. A
/// phrase is restyled into the serif voice (its id and motions are kept).
#[allow(clippy::too_many_arguments)]
fn placed_line(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    which: Which,
    subject: &Subject,
    slot: Slot,
    block: Option<&Block>,
    align: TextAlign,
    y: f32,
    color: Color,
    at: f64,
    role: &str,
) -> Result<(), CompileError> {
    let (w, m) = (ctx.w, ctx.margin());
    if let Some(mut layer) = recipes::place_subject(
        ctx,
        b,
        carries,
        which,
        subject,
        slot,
        (0.0, 0.0),
        color,
        at,
        Entrance::Rise,
        None,
        None,
        role,
    )? {
        if let Some(block) = block {
            let mut styled = text_layer(layer.id.clone(), block, color, align);
            styled.x = block_x(align, block, w, m);
            styled.y = y;
            layer = styled;
        }
        layer.z_index = 22;
        b.push(layer);
    }
    Ok(())
}
