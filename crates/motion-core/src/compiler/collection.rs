//! Collection composition (CreativeIntent v0.2 `collection` subject).
//!
//! Several distinct items stay individually identifiable: one card per item,
//! arriving one after another with the language's stagger. With `accumulate`
//! the build-up is visible: a running total steps up as each item lands, a
//! progress bar grows in matching steps, and the aggregate consequence finally
//! lands on an accent slab. Without `accumulate` the items are a plain
//! sequential list (a consequence is shown only if the intent names one).
//!
//! Layout (canvas space, inside the region below the headline):
//! items first (2-4: full-width rows, 5-6: two columns), the bottom ~30 % is
//! the aggregate zone (progress bar over the slab). Everything is a pure
//! function of the intent, style and region; timing derives from the beat's
//! motion-language profile.

use super::direction::{self, BeatParams, EntranceFamily, Reveal, StaggerOrder, StaggerPreset};
use super::recipes::{edge_entrance, with_ink, Region, B};
use super::typeset::{text_layer, Block, Voice};
use super::{base_layer, mo, parse_count, rect_layer, round3, CompileError, Ctx};
use crate::easing::Easing;
use crate::intent::{Collection, CollectionItem, Purpose, Relationship, Subject};
use crate::scene::{
    AssetKind, Direction, Fit, HAlign, Layer, LayerKind, LayoutBinding, Lifecycle, Motion, Padding,
    Stroke, TextAlign, VAlign,
};
use crate::timeline::format_count;

/// Minimum seconds between consecutive item arrivals.
const MIN_ITEM_GAP: f64 = 0.3;
/// Lead between an item's card starting to land and its total stepping up.
const STEP_LEAD: f64 = 0.18;
/// (0.23) Seconds the last text of the beat (the aggregate, else the last
/// item's content) must be readable before ANTICIPATE on the product path
/// (with a direction seed).
const READABLE_BEFORE_ANTICIPATE: f64 = 0.6;
/// (0.23) What a rebuilt chain aims for: [`READABLE_BEFORE_ANTICIPATE`] and a
/// margin for the milliseconds the times are rounded to.
const READABLE_TARGET: f64 = 0.65;
/// (0.23) Without a direction seed a chain is only changed when it would drop
/// its last text: the value coverage's floor (`recipes::TEXT_IN_VIEW_BEFORE_ANTICIPATE`).
const READABLE_FLOOR: f64 = 0.18;
/// Seconds after the aggregate slab starts that its text starts to fade in.
const LAND_LEAD: f64 = 0.25;
/// (0.23) The same in a compact chain (the text still lands on a slab that is
/// well on its way: the slab's mask is an OutQuint of 0.5 s).
const LAND_LEAD_FAST: f64 = 0.2;
/// Seconds of its 0.35 s `OutCubic` fade after which the aggregate is at half
/// opacity (the point from which a text counts as read).
const AGGREGATE_HALF: f64 = 0.206 * 0.35;
/// Seconds after an item's card starts that its content starts to fade in.
const CONTENT_LEAD: f64 = 0.12;
/// (0.23) The quickest cascade a compact chain uses between two items, and
/// from the last item to the aggregate (seconds).
const FAST_GAP: f64 = 0.14;
const FAST_AGGREGATE_GAP: f64 = 0.12;
/// (0.23) A compact chain never starts its first item before ENTER plus this
/// (the headline is on its way by then).
const FIRST_AFTER_ENTER: f64 = 0.2;

/// Compose a beat whose structure is a collection (primary, or secondary when
/// the primary is atomic). The headline, ghost word, kicker and exit are
/// already handled by the caller; place everything inside `region`, starting
/// at `region.start`, and finish before the beat's exit (`b.plan.exit_at()`).
pub(crate) fn compose(ctx: &mut Ctx, b: &mut B, region: Region) -> Result<(), CompileError> {
    let beat = b.beat;
    let beat_no = b.plan.index + 1;
    let (coll, other): (&Collection, Option<&Subject>) = match (&beat.primary, &beat.secondary) {
        (Subject::Collection(c), sec) => (c, sec.as_ref()),
        (prim, Some(Subject::Collection(c))) => (c, Some(prim)),
        _ => {
            return Err(CompileError::Beat {
                beat: beat_no,
                message: "collection composition needs a collection subject".into(),
            })
        }
    };
    let n = coll.items.len();
    if n == 0 {
        return Err(CompileError::Beat {
            beat: beat_no,
            message: "a collection needs at least one item".into(),
        });
    }

    let (w, u) = (ctx.w, ctx.u);
    let m = region.margin;
    let width = w - 2.0 * m;
    let lang = b.plan.lang;
    let p = lang.preset;
    let exit_at = b.plan.exit_at();
    let mstart = b.motions.len();
    // (0.23 C2b) The beat's planned direction; the identity is the old beat.
    let bp = ctx.params_for(b.plan.index);

    let accumulate = beat.relationship == Some(Relationship::Accumulate);
    // Only an atomic *secondary* is the collection's consequence. An atomic
    // primary beside a collection secondary is context, not a total; any
    // structured other subject is summarized (computed by the engine).
    let collection_is_primary = matches!(beat.primary, Subject::Collection(_));
    let consequence_text: Option<String> = other
        .filter(|s| s.is_atomic() && collection_is_primary)
        .and_then(super::subject_summary);
    let caption: Option<String> = other
        .filter(|s| !(s.is_atomic() && collection_is_primary))
        .and_then(super::subject_summary);
    let run = accumulate.then(|| running_totals(coll));
    let aggregate = aggregate_text(coll, consequence_text.as_deref(), run.as_ref(), accumulate);
    let zone = aggregate.is_some();

    // ---- geometry ----------------------------------------------------------
    let region_h = (region.bottom - region.top).max(240.0 * u);
    let gap = 16.0 * u;
    let caption_h = if caption.is_some() { 46.0 * u } else { 0.0 };
    let area_h = if zone {
        0.68 * region_h
    } else {
        region_h - caption_h
    };
    let rows_for = |cols: usize| n.div_ceil(cols);
    let row_h_for = |cols: usize| {
        let rows = rows_for(cols) as f32;
        (area_h - (rows - 1.0) * gap) / rows
    };
    let mut cols = if n >= 5 { 2 } else { 1 };
    if cols == 1 && n >= 3 && row_h_for(1) < 120.0 * u {
        cols = 2;
    }
    let rows = rows_for(cols);
    let max_row = match (cols, n) {
        (1, 0..=3) => 240.0 * u,
        (1, _) => 200.0 * u,
        _ => 230.0 * u,
    };
    let row_h = row_h_for(cols).clamp(64.0 * u, max_row);
    let card_w = if cols == 1 {
        width
    } else {
        (width - gap) / 2.0
    };
    let items_h = rows as f32 * row_h + (rows as f32 - 1.0) * gap;

    let zone_top = region.top + items_h + 36.0 * u;
    let cap_y = if zone {
        zone_top
    } else {
        region.top + items_h + 24.0 * u
    };
    let bar_h = 14.0 * u;
    let bar_y = zone_top + caption_h;
    let slab_y = if accumulate {
        bar_y + bar_h + 18.0 * u
    } else {
        zone_top + caption_h
    };
    let reveal = beat.purpose == Purpose::Reveal;
    let slab_cap = if reveal { 300.0 * u } else { 240.0 * u };
    let slab_h = (region.bottom - slab_y).clamp(90.0 * u, slab_cap);

    // ---- timing (scene lifecycle) -------------------------------------------
    // The first item arrives at the end of ENTER; every later item is one
    // EVOLVE event, and the aggregate / consequence takes the final event.
    // (0.23) A beat too short for that leaves the aggregate to land after
    // ANTICIPATE: its chain is compressed so the last text is read in time
    // (see `arrival_chain`).
    let life = b.plan.life;
    let card_dur = (p.duration).clamp(0.45, 0.9);
    let fade_dur = (p.duration * 0.55).clamp(0.3, 0.5);
    // The last text the chain places is a value the intent gives (the
    // consequence, or the last item) unless it is an aggregate the engine
    // computed (a running sum or a count): only a given value can be dropped.
    let given_value = !zone || consequence_text.is_some();
    let chain = arrival_chain(
        &life,
        region.start,
        n,
        zone,
        fade_dur,
        ctx.direction_seed.is_some(),
        given_value,
    );
    // (0.23 C2b) The beat's stagger on the arrival chain (none: the chain as is).
    let chain = restagger(chain, &bp, accumulate);
    let t0 = chain.at.iter().copied().fold(f64::INFINITY, f64::min);
    let ta = chain.ta;
    let compact = chain.compact;
    let at = chain.at;
    let land_lead = chain.lead;
    let pop_dur = (p.duration * 1.4).clamp(0.6, 1.2);

    // ---- cards + content ---------------------------------------------------
    for (i, item) in coll.items.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let mut x = m + c as f32 * (card_w + gap);
        if cols == 2 && n % 2 == 1 && i == n - 1 {
            x = m + (width - card_w) / 2.0;
        }
        let y = region.top + r as f32 * (row_h + gap);
        let card_id = b.id(&format!("items.{i}.card"));
        b.push(base_layer(
            card_id.clone(),
            (x, y, card_w, row_h),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.card,
                radius: 14.0 * u,
                stroke: Some(Stroke {
                    color: ctx.palette.ink,
                    width: 2.0 * u,
                }),
            },
            12,
        ));
        let a = at[i];
        // (0.23 C2b) The card wipes in from the planned side, travels the
        // planned distance and takes the planned easing.
        b.motions.push(edge_entrance(
            &bp,
            &card_id,
            a,
            card_dur,
            Reveal::Mask,
            Direction::Right,
            Easing::OutQuint,
        ));
        b.motions
            .push(mo::fade(&card_id, a, fade_dur, 0.0, 1.0, Easing::OutCubic));
        b.motions.push(mo::shift(
            &card_id,
            a,
            p.duration.max(0.5),
            [0.0, direction::travel(&bp, p.travel * 0.5 * u)],
            [0.0, 0.0],
            direction::entrance_easing(&bp, p.easing),
        ));

        let pieces = item_content(ctx, b, i, item, &card_id, (card_w, row_h), cols == 2)?;
        for piece in pieces {
            let id = piece.layer.id.clone();
            b.motions.push(mo::fade(
                &id,
                a + 0.12,
                fade_dur,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
            let mut counted = false;
            if lang.data_numbers && piece.countable {
                if let Some(text) = &piece.text {
                    if let Some(cm) = mo::count(
                        &id,
                        a + 0.12,
                        (p.duration * 1.4).max(0.6),
                        text,
                        lang.settle,
                    ) {
                        b.motions.push(cm);
                        counted = true;
                    }
                }
            }
            // (0.23 C2b) ScalePop pops the card's content; GlyphCascade rises
            // its text glyph by glyph (never a counting number: its text
            // changes while it counts). Both fall back to the plain fade.
            if let Some(pop) = direction::pop_entrance(
                &bp,
                &id,
                a + 0.12,
                fade_dur,
                direction::entrance_easing(&bp, lang.settle),
            ) {
                b.motions.push(pop);
            } else if let (Some(text), false) = (&piece.text, counted) {
                let glyphs = text.chars().filter(|c| !c.is_whitespace()).count();
                if let Some(cascade) = direction::glyph_entrance(
                    &bp,
                    &id,
                    a + 0.12,
                    fade_dur.max(0.5),
                    [0.0, direction::travel(&bp, p.travel * 0.4 * u)],
                    glyphs,
                    direction::entrance_easing(&bp, p.easing),
                ) {
                    b.motions.push(cascade);
                }
            }
            b.push(piece.layer);
        }
    }

    // ---- caption (another structured subject, shown small) -----------------
    if let Some(text) = &caption {
        let block = ctx.ts.fit_line(Voice::SERIF, text, width, 38.0 * u);
        let id = b.id("caption");
        let mut layer = text_layer(id.clone(), &block, ctx.palette.muted, TextAlign::Left);
        layer.x = m;
        layer.y = cap_y;
        layer.z_index = 18;
        b.motions.push(mo::fade(
            &id,
            t0 + 0.2,
            fade_dur,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        b.push(layer);
    }

    // ---- aggregate zone ----------------------------------------------------
    if let Some((agg_text, agg_countable)) = &aggregate {
        let slab_id = b.id("aggregate.slab");
        let slab_rect = (m, slab_y, width, slab_h);
        b.push(rect_layer(
            slab_id.clone(),
            slab_rect,
            ctx.palette.accent,
            20,
        ));
        b.motions.push(edge_entrance(
            &bp,
            &slab_id,
            ta,
            0.7,
            Reveal::Mask,
            Direction::Right,
            Easing::OutQuint,
        ));

        if let Some(run) = &run {
            accumulation(
                ctx,
                b,
                run,
                coll,
                &Zone {
                    slab_id: &slab_id,
                    slab_h,
                    bar: (m, bar_y, width, bar_h),
                },
                &at,
                ta,
                compact,
            );
        }

        // The aggregate lands with emphasis: pop-settle on the slab.
        let agg_id = b.id("aggregate");
        let agg = if *agg_countable {
            let block = ctx
                .ts
                .fit_line(Voice::HERO_NUMBER, agg_text, width * 0.88, slab_h * 0.62);
            bound_text(
                ctx,
                agg_id.clone(),
                &block,
                ctx.palette.on_accent,
                &slab_id,
                Bind::center(),
                24,
            )
        } else {
            let block = ctx.ts.fit_block(
                Voice::HEADLINE,
                agg_text,
                width * 0.88,
                slab_h * 0.78,
                130.0 * u,
                2,
            );
            bound_text(
                ctx,
                agg_id.clone(),
                &block,
                ctx.palette.on_accent,
                &slab_id,
                Bind::center(),
                24,
            )
        };
        let land = ta + land_lead;
        b.motions
            .push(mo::fade(&agg_id, land, 0.35, 0.0, 1.0, Easing::OutCubic));
        // (0.23 C2b) The landing pop: `amplitude` scales its overshoot, ScalePop
        // makes it a bigger one. The count (when the aggregate counts up) keeps
        // its own landing curve and timing.
        let pop = if reveal { 1.2 } else { 1.08 };
        let pop = if bp.entrance == EntranceFamily::ScalePop {
            pop + 0.12
        } else {
            pop
        };
        b.motions.push(mo::scale(
            &agg_id,
            land,
            pop_dur,
            direction::scale_amplitude(&bp, pop),
            1.0,
            direction::entrance_easing(&bp, lang.settle),
        ));
        // A consequence written as a number counts up as it lands.
        if consequence_text.is_some() {
            if let Some(cm) = mo::count(&agg_id, land, pop_dur, agg_text, lang.settle) {
                b.motions.push(cm);
            }
        }
        b.push(agg);
    }

    clamp_motions(&mut b.motions[mstart..], exit_at);
    Ok(())
}

/// When the items arrive and when the aggregate slab starts.
struct Chain {
    /// Start of each item's card (`n` entries, increasing).
    at: Vec<f64>,
    /// Start of the aggregate slab (the first item's time without an aggregate).
    ta: f64,
    /// Seconds after `ta` that the aggregate text starts to fade in.
    lead: f64,
    /// The chain was rebuilt (see [`arrival_chain`]): the aggregate slab comes
    /// in early, over the running total.
    compact: bool,
}

/// The arrival chain of a collection beat: the first item at the end of
/// ENTER, every later item and the aggregate one EVOLVE event ([`evolve_slots`]).
///
/// (0.23) A short beat (a continuous take leaves 2-3 s) gives that chain no
/// room: the events are squeezed against ANTICIPATE and the aggregate text
/// lands after it, unread (`value_dropped`). When the last text of the beat
/// (the aggregate, else the last item's content) would be read less than
/// [`READABLE_BEFORE_ANTICIPATE`] before ANTICIPATE (with a direction seed;
/// without one only when it would be read less than [`READABLE_FLOOR`] before
/// it, i.e. dropped, and only when that text is a value the intent gives:
/// `given_value`), the chain is built backwards from that deadline instead:
/// a quick cascade of [`FAST_GAP`] between the items, the aggregate right
/// behind the last one, and the first item as soon as the headline is on its
/// way ([`FIRST_AFTER_ENTER`]) when the cascade needs the room. A beat whose
/// chain already has room is untouched.
fn arrival_chain(
    life: &Lifecycle,
    region_start: f64,
    n: usize,
    zone: bool,
    fade_dur: f64,
    seeded: bool,
    given_value: bool,
) -> Chain {
    let t0 = round3(region_start.max(life.settle));
    let events = evolve_slots(life, n - 1 + usize::from(zone), t0);
    let at: Vec<f64> = std::iter::once(t0)
        .chain(events.iter().copied().take(n - 1))
        .collect();
    let ta = if zone {
        events.last().copied().unwrap_or(t0)
    } else {
        t0
    };
    let usual = Chain {
        at,
        ta,
        lead: LAND_LEAD,
        compact: false,
    };
    // When the beat's last text is at half opacity.
    let item_half = CONTENT_LEAD + 0.206 * fade_dur;
    let readable = |c: &Chain| {
        if zone {
            c.ta + c.lead + AGGREGATE_HALF
        } else {
            c.at.last().copied().unwrap_or(t0) + item_half
        }
    };
    let floor = if seeded {
        READABLE_BEFORE_ANTICIPATE
    } else {
        READABLE_FLOOR
    };
    if readable(&usual) <= life.anticipate - floor + 1e-9 || !(seeded || given_value) {
        return usual;
    }

    // Backwards from the deadline: the last item (the aggregate follows it).
    let last_item_by = if zone {
        life.anticipate - READABLE_TARGET - LAND_LEAD_FAST - AGGREGATE_HALF - FAST_AGGREGATE_GAP
    } else {
        life.anticipate - READABLE_TARGET - item_half
    };
    let steps = (n - 1) as f64;
    let earliest = (life.enter + FIRST_AFTER_ENTER).min(t0);
    let gap = if n > 1 {
        ((last_item_by - t0) / steps).clamp(FAST_GAP, MIN_ITEM_GAP)
    } else {
        FAST_GAP
    };
    let first = round3((last_item_by - steps * gap).min(t0).max(earliest));
    // Where even the earliest start leaves no room the cascade tightens
    // (never below half the quick gap) rather than run late.
    let gap = if n > 1 {
        gap.min(((last_item_by - first) / steps).max(0.5 * FAST_GAP))
    } else {
        gap
    };
    let at: Vec<f64> = (0..n).map(|i| round3(first + i as f64 * gap)).collect();
    let last = at.last().copied().unwrap_or(first);
    let compact = Chain {
        ta: if zone {
            round3(last + FAST_AGGREGATE_GAP)
        } else {
            first
        },
        at,
        lead: LAND_LEAD_FAST,
        compact: true,
    };
    // Never later than the chain it replaces.
    if readable(&compact) < readable(&usual) {
        compact
    } else {
        usual
    }
}

/// (0.23 C2b) The beat's stagger on the arrival chain. `Tight` pulls the
/// arrivals toward the first item (the last card comes earlier, never later)
/// and `Loose` pushes the middle ones toward the last (never past it); every
/// gap keeps the smaller of what the chain gave it and [`MIN_ITEM_GAP`], the
/// first card stays and no card arrives after the last one did, so the
/// aggregate and the read time are not touched. A `Reverse` / `CenterOut` order
/// reassigns the arrival times among the cards of a plain list; a running total
/// (`accumulate`) counts its cards in order, so it keeps the forward order. A
/// compact chain (already squeezed to be read in time) is left as built. The
/// identity (`Builder` / `Even` pace, forward order) returns the chain as is.
fn restagger(mut chain: Chain, bp: &BeatParams, accumulate: bool) -> Chain {
    let paced = matches!(bp.stagger, StaggerPreset::Tight | StaggerPreset::Loose);
    let reordered = bp.stagger_order != StaggerOrder::Forward && !accumulate;
    let n = chain.at.len();
    if !(paced || reordered) || chain.compact || n < 2 {
        return chain;
    }
    let (t0, last) = (chain.at[0], chain.at[n - 1]);
    let floors: Vec<f64> = chain
        .at
        .windows(2)
        .map(|w| (w[1] - w[0]).min(MIN_ITEM_GAP))
        .collect();
    // The pace: the helper scales the offsets from the first arrival.
    let pace = BeatParams {
        stagger_order: StaggerOrder::Forward,
        ..*bp
    };
    let offsets: Vec<f64> = chain.at.iter().map(|a| a - t0).collect();
    let mut at: Vec<f64> = direction::stagger_offsets(&pace, offsets)
        .into_iter()
        .map(|o| t0 + o)
        .collect();
    // Keep the gaps going forward, then keep the last arrival where it was.
    for i in 1..n {
        at[i] = at[i].max(at[i - 1] + floors[i - 1]);
    }
    at[n - 1] = at[n - 1].min(last);
    for i in (0..n - 1).rev() {
        at[i] = at[i].min(at[i + 1] - floors[i]);
    }
    if reordered {
        // The order: the helper reassigns the (increasing) offsets.
        let order = BeatParams {
            stagger: StaggerPreset::Builder,
            ..*bp
        };
        at = direction::stagger_offsets(&order, at.iter().map(|a| a - t0).collect())
            .into_iter()
            .map(|o| t0 + o)
            .collect();
    }
    chain.at = at.into_iter().map(round3).collect();
    chain
}

/// Arrival times of the items after the first (and the aggregate): `m` evolve
/// events in `[life.evolve, life.anticipate)`, strictly after `first`. When the
/// EVOLVE window is too short for `MIN_ITEM_GAP` the events are compressed
/// backwards from `life.anticipate` so every arrival still starts before it.
fn evolve_slots(life: &Lifecycle, m: usize, first: f64) -> Vec<f64> {
    if m == 0 {
        return Vec::new();
    }
    let mut slots = life.evolve_events(m);
    let step = (life.anticipate - life.evolve) / m as f64;
    if step < MIN_ITEM_GAP {
        let last = (life.anticipate - 0.05).max(first + 0.1);
        let g = MIN_ITEM_GAP.min((last - first) / m as f64);
        for (i, s) in slots.iter_mut().enumerate() {
            *s = last - (m - 1 - i) as f64 * g;
        }
    }
    let mut prev = first;
    for s in &mut slots {
        *s = round3(s.max(prev + 0.05));
        prev = *s;
    }
    slots
}

/// Never let a motion of this composition run past the beat's exit.
fn clamp_motions(motions: &mut [Motion], exit_at: f64) {
    for mo in motions {
        let room = (exit_at - mo.start).max(0.0);
        if mo.duration > room {
            mo.duration = round3(room);
        }
    }
}

// ---------------------------------------------------------------------------
// Running totals
// ---------------------------------------------------------------------------

/// Cumulative values shown by the running total, and how to format them.
struct Running {
    /// Cumulative total after each item.
    totals: Vec<f64>,
    decimals: u8,
    grouping: bool,
    prefix: String,
    suffix: String,
    /// Sum of parsed numbers (`true`) or a plain item count (`false`).
    is_sum: bool,
}

impl Running {
    fn fmt(&self, v: f64) -> String {
        format_count(v, self.decimals, self.grouping, &self.prefix, &self.suffix)
    }
}

/// `parse_count` result: `(value, decimals, grouping, prefix, suffix)`.
type Parsed = (f64, u8, bool, String, String);

/// The sum when every item is a number with the same prefix/suffix, else a count.
fn running_totals(c: &Collection) -> Running {
    let parsed: Option<Vec<Parsed>> = c
        .items
        .iter()
        .map(|i| match i {
            CollectionItem::Number(a) => a.value.as_deref().and_then(parse_count),
            _ => None,
        })
        .collect();
    if let Some(list) = parsed {
        let same = list.iter().all(|x| x.3 == list[0].3 && x.4 == list[0].4);
        if same && !list.is_empty() {
            let decimals = list.iter().map(|x| x.1).max().unwrap_or(0);
            let mut acc = 0.0;
            let totals = list
                .iter()
                .map(|x| {
                    acc += x.0;
                    acc
                })
                .collect();
            return Running {
                totals,
                decimals,
                grouping: list.iter().any(|x| x.2),
                prefix: list[0].3.clone(),
                suffix: list[0].4.clone(),
                is_sum: true,
            };
        }
    }
    Running {
        totals: (1..=c.items.len()).map(|i| i as f64).collect(),
        decimals: 0,
        grouping: false,
        prefix: String::new(),
        suffix: String::new(),
        is_sum: false,
    }
}

/// The aggregate text and whether it is a number-like value (set in the number
/// voice), or `None` when there is nothing to aggregate (no accumulate and no
/// consequence subject).
fn aggregate_text(
    c: &Collection,
    consequence: Option<&str>,
    run: Option<&Running>,
    accumulate: bool,
) -> Option<(String, bool)> {
    if let Some(text) = consequence {
        return Some((text.to_string(), parse_count(text).is_some()));
    }
    if !accumulate {
        return None;
    }
    let run = run?;
    if run.is_sum {
        let total = run.totals.last().copied().unwrap_or(0.0);
        return Some((run.fmt(total), true));
    }
    let meaning = c
        .meaning
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("items");
    Some((format!("{} {}", c.items.len(), meaning), true))
}

struct Zone<'a> {
    slab_id: &'a str,
    slab_h: f32,
    /// Progress bar track (x, y, w, h).
    bar: (f32, f32, f32, f32),
}

/// Progress bar + running total: one step per item, non-overlapping.
#[allow(clippy::too_many_arguments)]
fn accumulation(
    ctx: &Ctx,
    b: &mut B,
    run: &Running,
    coll: &Collection,
    zone: &Zone,
    at: &[f64],
    ta: f64,
    give_way: bool,
) {
    let u = ctx.u;
    let n = at.len();
    let p = b.plan.lang.preset;
    let fade_dur = (p.duration * 0.55).clamp(0.3, 0.5);

    // Progress bar: chained expansions, each starting where the last ended.
    let (bx, by, bw, bh) = zone.bar;
    let track_id = b.id("bar.track");
    let fill_id = b.id("bar.fill");
    b.push(rect_layer(
        track_id.clone(),
        (bx, by, bw, bh),
        ctx.palette.ink.with_alpha(0x26),
        14,
    ));
    b.push(rect_layer(
        fill_id.clone(),
        (bx, by, 0.0, bh),
        ctx.palette.accent,
        15,
    ));
    b.motions.push(mo::fade(
        &track_id,
        at[0],
        fade_dur,
        0.0,
        1.0,
        Easing::OutCubic,
    ));

    // Running total (number + label), bound to the slab it is later covered by.
    let label_text = coll
        .meaning
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(if run.is_sum { "total" } else { "items" });
    let widest = run.fmt(run.totals.last().copied().unwrap_or(0.0));
    let num_block = ctx.ts.fit_line(
        Voice::HERO_NUMBER,
        &widest,
        bw * 0.7,
        (zone.slab_h * 0.5).min(132.0 * u),
    );
    let label_block = ctx
        .ts
        .fit_line(Voice::LABEL, label_text, bw * 0.6, 30.0 * u);
    let stack_h = num_block.height() + 10.0 * u + label_block.height();
    let num_dy = -stack_h / 2.0 + num_block.height() / 2.0;
    let label_dy = stack_h / 2.0 - label_block.height() / 2.0;

    let total_id = b.id("total");
    let mut total = bound_text(
        ctx,
        total_id.clone(),
        &num_block,
        ctx.palette.ink,
        zone.slab_id,
        Bind {
            offset: [0.0, num_dy],
            ..Bind::center()
        },
        16,
    );
    if let LayerKind::Text(t) = &mut total.kind {
        t.text = run.fmt(0.0);
    }
    let label_id = b.id("total.label");
    let label = bound_text(
        ctx,
        label_id.clone(),
        &label_block,
        ctx.palette.muted,
        zone.slab_id,
        Bind {
            offset: [0.0, label_dy],
            ..Bind::center()
        },
        16,
    );
    // The running total appears together with the first item's content, and
    // its first step starts as it fades in (never a visible "0").
    // (0.23) In a compact chain the items cascade 0.14 s apart and the
    // aggregate slab comes in while the beat is still being read, covering the
    // running total from the left: the total would count 1, 2, 3 in 0.4 s (too
    // quick to read) and stand half covered on the accent colour at READ. The
    // progress bar keeps the count; the total and its label are not drawn.
    if !give_way {
        for id in [&total_id, &label_id] {
            b.motions.push(mo::fade(
                id,
                at[0] + 0.12,
                fade_dur,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
        }
    }

    let mut prev = 0.0;
    for i in 0..n {
        // Each step ends before the next item's step (or the aggregate) begins.
        let next = if i + 1 < n { at[i + 1] } else { ta };
        let lead = if i == 0 { 0.12 } else { STEP_LEAD }.min(0.4 * (next - at[i]).max(0.0));
        let start = round3(at[i] + lead);
        let dur = (next - start).clamp(0.05, if i + 1 < n { 0.42 } else { 0.5 });
        let to = run.totals[i];
        if !give_way {
            b.motions.push(Motion {
                spring: None,
                id: None,
                target: total_id.clone(),
                start,
                duration: round3(dur),
                easing: Easing::OutCubic,
                op: crate::scene::MotionOp::Count {
                    from: prev,
                    to,
                    decimals: run.decimals,
                    grouping: run.grouping,
                    prefix: run.prefix.clone(),
                    suffix: run.suffix.clone(),
                },
            });
        }
        prev = to;
        // Bar steps to (i + 1) / n of the track, chained from the last step.
        b.motions.push(mo::expand(
            &fill_id,
            start,
            dur,
            (bx, by, bw * (i + 1) as f32 / n as f32, bh),
            Easing::OutCubic,
        ));
    }
    if !give_way {
        b.push(total);
        b.push(label);
    }
}

// ---------------------------------------------------------------------------
// Item content
// ---------------------------------------------------------------------------

/// A content layer of an item card.
struct Piece {
    layer: Layer,
    /// Text a data-language count can animate (numbers only).
    text: Option<String>,
    countable: bool,
}

/// Where a bound layer sits inside its container.
#[derive(Clone, Copy)]
struct Bind {
    h: HAlign,
    v: VAlign,
    pad: Padding,
    offset: [f32; 2],
}

impl Bind {
    fn center() -> Bind {
        Bind {
            h: HAlign::Center,
            v: VAlign::Center,
            pad: Padding::default(),
            offset: [0.0, 0.0],
        }
    }
    fn side(h: HAlign, inset: f32) -> Bind {
        Bind {
            h,
            v: VAlign::Center,
            pad: Padding {
                left: if h == HAlign::Left { inset } else { 0.0 },
                right: if h == HAlign::Right { inset } else { 0.0 },
                top: 0.0,
                bottom: 0.0,
            },
            offset: [0.0, 0.0],
        }
    }
}

fn binding(parent: &str, bind: Bind) -> LayoutBinding {
    LayoutBinding {
        parent: parent.to_string(),
        horizontal: bind.h,
        vertical: bind.v,
        offset: bind.offset,
        padding: bind.pad,
    }
}

/// A text layer optically centered (ink-aware) in `parent` per `bind`.
fn bound_text(
    ctx: &Ctx,
    id: String,
    block: &Block,
    color: crate::scene::Color,
    parent: &str,
    bind: Bind,
    z: i32,
) -> Layer {
    let mut layer = with_ink(ctx, text_layer(id, block, color, TextAlign::Center), block);
    layer.z_index = z;
    layer.layout = Some(binding(parent, bind));
    layer
}

/// Written text of an item: `(main text, label, number-like)`.
fn item_texts(item: &CollectionItem) -> (String, Option<String>, bool) {
    let clean = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let (value, meaning, number) = match item {
        CollectionItem::Phrase(a) => (a.value.as_deref(), a.meaning.as_deref(), false),
        CollectionItem::Number(a) => (a.value.as_deref(), a.meaning.as_deref(), true),
        CollectionItem::Object(o) => (o.value.as_deref(), o.meaning.as_deref(), true),
    };
    let value = value.map(clean).filter(|s| !s.is_empty());
    let meaning = meaning.map(clean).filter(|s| !s.is_empty());
    match (value, meaning) {
        (Some(v), m) => (v, m, number),
        (None, Some(m)) => (m, None, false),
        (None, None) => (String::new(), None, false),
    }
}

/// The content layers of one item, each bound to its card.
fn item_content(
    ctx: &mut Ctx,
    b: &B,
    i: usize,
    item: &CollectionItem,
    card_id: &str,
    (card_w, card_h): (f32, f32),
    stacked: bool,
) -> Result<Vec<Piece>, CompileError> {
    let u = ctx.u;
    let ink = ctx.palette.ink;
    let pad_x = 40.0 * u;
    let pad_y = 14.0 * u;
    let main_id = b.id(&format!("items.{i}.content"));
    let label_id = b.id(&format!("items.{i}.label"));
    let mut out = Vec::new();

    if let CollectionItem::Object(o) = item {
        // Asset on the left; meaning label beside it (or the figure, in grids).
        let (asset, kind) = ctx.object_asset(&o.asset, b.plan.index + 1)?;
        let side = card_h * 0.62;
        let kind = match kind {
            AssetKind::Image => LayerKind::Image {
                asset,
                fit: Fit::Contain,
                treatment: None,
                playback: None,
                insert: None,
            },
            _ => LayerKind::Svg {
                asset,
                fit: Fit::Contain,
            },
        };
        let mut layer = base_layer(main_id.clone(), (0.0, 0.0, side, side), kind, 16);
        layer.layout = Some(binding(card_id, Bind::side(HAlign::Left, 28.0 * u)));
        out.push(Piece {
            layer,
            text: None,
            countable: false,
        });
        let (main, meaning, _) = item_texts(item);
        let has_value = o.value.as_deref().is_some_and(|v| !v.trim().is_empty());
        let (value, meaning) = if has_value {
            (Some(main), meaning)
        } else {
            (None, Some(main))
        };
        let text_left = 28.0 * u + side + 24.0 * u;
        let avail = (card_w - text_left - pad_x).max(40.0 * u);
        if let Some(v) = &value {
            let share = if stacked { 1.0 } else { 0.5 };
            let vblock = ctx
                .ts
                .fit_line(Voice::HERO_NUMBER, v, avail * share, card_h * 0.5);
            out.push(Piece {
                layer: bound_text(
                    ctx,
                    b.id(&format!("items.{i}.value")),
                    &vblock,
                    ink,
                    card_id,
                    Bind::side(HAlign::Right, pad_x),
                    17,
                ),
                text: Some(v.clone()),
                countable: parse_count(v).is_some(),
            });
        }
        // In grid cells a figure takes the label's place.
        if let (Some(l), true) = (meaning, value.is_none() || !stacked) {
            let lw = if value.is_some() { avail * 0.5 } else { avail };
            let block = ctx.ts.fit_line(Voice::LABEL, &l, lw, 40.0 * u);
            out.push(Piece {
                layer: bound_text(
                    ctx,
                    label_id,
                    &block,
                    ink,
                    card_id,
                    Bind::side(HAlign::Left, text_left),
                    17,
                ),
                text: None,
                countable: false,
            });
        }
        return Ok(out);
    }

    let (text, label, number) = item_texts(item);
    let voice = if number {
        Voice::HERO_NUMBER
    } else {
        Voice::HEADLINE
    };
    let inner_w = card_w - 2.0 * pad_x;
    let inner_h = card_h - 2.0 * pad_y;
    let countable = number && parse_count(&text).is_some();

    match label {
        // Figure/name with its meaning: beside (rows) or stacked (grid cells).
        Some(l) if !stacked => {
            let main_block = if number {
                ctx.ts
                    .fit_line(voice, &text, inner_w * 0.52, inner_h * 0.85)
            } else {
                ctx.ts
                    .fit_block(voice, &text, inner_w * 0.56, inner_h, 84.0 * u, 2)
            };
            let label_block = ctx.ts.fit_line(Voice::LABEL, &l, inner_w * 0.34, 34.0 * u);
            out.push(Piece {
                layer: bound_text(
                    ctx,
                    main_id,
                    &main_block,
                    ink,
                    card_id,
                    Bind::side(HAlign::Left, pad_x),
                    16,
                ),
                text: Some(text),
                countable,
            });
            out.push(Piece {
                layer: bound_text(
                    ctx,
                    label_id,
                    &label_block,
                    ctx.palette.muted,
                    card_id,
                    Bind::side(HAlign::Right, pad_x),
                    17,
                ),
                text: None,
                countable: false,
            });
        }
        Some(l) => {
            let main_block = if number {
                ctx.ts.fit_line(voice, &text, inner_w, inner_h * 0.62)
            } else {
                ctx.ts
                    .fit_block(voice, &text, inner_w, inner_h * 0.62, 64.0 * u, 1)
            };
            let label_block = ctx.ts.fit_line(Voice::LABEL, &l, inner_w, 26.0 * u);
            let total = main_block.height() + 6.0 * u + label_block.height();
            let main_dy = -total / 2.0 + main_block.height() / 2.0;
            let label_dy = total / 2.0 - label_block.height() / 2.0;
            out.push(Piece {
                layer: bound_text(
                    ctx,
                    main_id,
                    &main_block,
                    ink,
                    card_id,
                    Bind {
                        offset: [0.0, main_dy],
                        ..Bind::center()
                    },
                    16,
                ),
                text: Some(text),
                countable,
            });
            out.push(Piece {
                layer: bound_text(
                    ctx,
                    label_id,
                    &label_block,
                    ctx.palette.muted,
                    card_id,
                    Bind {
                        offset: [0.0, label_dy],
                        ..Bind::center()
                    },
                    17,
                ),
                text: None,
                countable: false,
            });
        }
        None => {
            let (mw, mh, cap) = if stacked {
                (inner_w, inner_h, 64.0 * u)
            } else {
                (inner_w, inner_h, 84.0 * u)
            };
            let main_block = if number {
                ctx.ts.fit_line(voice, &text, mw, mh * 0.85)
            } else {
                ctx.ts.fit_block(voice, &text, mw, mh, cap, 2)
            };
            out.push(Piece {
                layer: bound_text(ctx, main_id, &main_block, ink, card_id, Bind::center(), 16),
                text: Some(text),
                countable,
            });
        }
    }
    Ok(out)
}
