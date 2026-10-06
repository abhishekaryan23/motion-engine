//! (0.22) StatPair: two pictures compared, each with its figure.
//!
//! A `compare` / `contrast` beat whose primary and secondary are both pictures
//! (objects) used to go to HeroObject in the flat looks: one big picture, the
//! second as a small prop, and only the first `value` stamped; the genre looks
//! (street, studio, hype) kept one picture and no value at all. The intent
//! contract says an object's `value` is "a short figure stamped next to the
//! object in compare/contrast beats", so both sides are shown as equals: each
//! picture with its label (`meaning`) and its stamped figure (`value`), and
//! the relationship drawn between them.
//!
//! The pair is the cinematic relation stage ([`relation_stage`]) built flat:
//! same arrangement (side by side or stacked, whichever lets the pictures be
//! larger), labels, stamps and connector, no depth, flat entrances. The
//! headline sits above it like HeroObject's. When a picture cannot be resolved
//! (no delivered image and no library picture) the beat falls back to
//! HeroObject.

use super::relation_stage::{self, Stage};
use super::{hero_object, Composition};
use crate::compiler::recipes::{self, B};
use crate::compiler::typeset::Voice;
use crate::compiler::{Carry, CompileError, Ctx};
use crate::scene::TextAlign;

pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    c: Composition,
) -> Result<(), CompileError> {
    if !relation_stage::pair_ready(ctx, b) {
        return hero_object::build(ctx, b, carries, c);
    }
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let t = b.plan.enter_at();

    recipes::ghost(ctx, b, 0.6 * h);

    // The headline above the pair, as HeroObject sets it.
    let head_top = 0.165 * h;
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &b.beat.statement,
        w - 2.0 * m,
        0.15 * h,
        104.0 * u,
        3,
    );
    recipes::headline_lines(
        ctx,
        b,
        "head",
        &head,
        head_top,
        TextAlign::Left,
        ctx.palette.ink,
        t + 0.05,
        true,
    );
    let title_bottom = head_top + head.height();

    let built = relation_stage::build(ctx, b, carries, title_bottom, Stage::Flat)?;
    debug_assert!(built, "pair_ready said the pair resolves");
    Ok(())
}
