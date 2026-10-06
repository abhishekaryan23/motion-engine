//! Scene handoffs (0.6): graphic panel wipes chosen by the style's
//! [`TransitionCharacter`](super::taste::TransitionCharacter).
//!
//! The compiler decides *whether* a beat enters behind a wipe
//! (`BeatPlan::wipe_in`, from `temporal::handoff`); this module decides what
//! the wipe looks like for the style's transition family. The accent flood of
//! impact beats (`accent_in`) is unchanged and lives in the compositions.

use super::taste::TransitionFamily;
use super::{base_layer, mo, rect_layer, BeatPlan, Ctx};
use crate::easing::Easing;
use crate::scene::{Layer, LayerKind, Motion};

/// Fraction of the canvas the wipe extends beyond every edge, so the camera
/// push (and the stage's anticipation) never shows a panel edge.
const BLEED: f32 = 0.02;
/// Latest the wipe may still be on screen after the beat starts entering.
const CLEAR_AFTER_ENTER: f64 = 0.13;

/// Scene-level layers + motions (above the stage, below grain) for a beat
/// that enters behind a panel wipe. Empty when `!plan.wipe_in`. Times are
/// scene-local; the wipe covers the frame during PRE_ENTER (the overlap with
/// the outgoing scene) and uncovers the new stage before `plan.enter_at()`
/// plus a few frames. Sits at z 60: above every stage (10 / 40), below grain.
pub(crate) fn panel_wipe(ctx: &Ctx, plan: &BeatPlan) -> (Vec<Layer>, Vec<Motion>) {
    if !plan.wipe_in {
        return (Vec::new(), Vec::new());
    }
    match ctx.taste.transition.family {
        TransitionFamily::Geometric => geometric(ctx, plan),
        TransitionFamily::Kinetic => kinetic(ctx, plan),
        _ => (Vec::new(), Vec::new()),
    }
}

/// (0.23 W4) The longest a handoff wipe covers the frame on the product path
/// (seconds from the beat's start): a wipe that holds longer is dead air.
const WIPE_MAX_S: f64 = 0.4;

/// Scene-local time by which the wipe is completely off screen.
fn clear_at(ctx: &Ctx, plan: &BeatPlan) -> f64 {
    let end = plan.enter_at() + CLEAR_AFTER_ENTER;
    if super::speech_lifecycle::dead_air_rules(ctx) {
        end.min(WIPE_MAX_S)
    } else {
        end
    }
}

/// A straight panel with thin accent edges slides across the frame and out
/// the opposite side. Direction alternates with the beat index.
fn geometric(ctx: &Ctx, plan: &BeatPlan) -> (Vec<Layer>, Vec<Motion>) {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let dir = if plan.index.is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let end = clear_at(ctx, plan);
    let cover = end * 0.45;
    let uncover = end * 0.42;
    let (bx, by) = (w * BLEED, h * BLEED);
    let (pw, ph) = (w + 2.0 * bx, h + 2.0 * by);
    let edge = 8.0 * u;
    let fill = if ctx.palette.fields.is_empty() {
        ctx.palette.card
    } else {
        ctx.palette.field(plan.index)
    };

    let id = format!("{}.wipe", plan.prefix);
    let group = base_layer(
        id.clone(),
        (0.0, 0.0, w, h),
        LayerKind::Group {
            children: vec![
                rect_layer(format!("{id}.panel"), (-bx, -by, pw, ph), fill, 0),
                rect_layer(
                    format!("{id}.edge_a"),
                    (-bx, -by, edge, ph),
                    ctx.palette.accent,
                    1,
                ),
                rect_layer(
                    format!("{id}.edge_b"),
                    (w + bx - edge, -by, edge, ph),
                    ctx.palette.accent,
                    1,
                ),
            ],
        },
        60,
    );
    let travel = pw + 0.02 * w;
    let motions = vec![
        mo::shift(
            &id,
            0.0,
            cover,
            [-dir * travel, 0.0],
            [0.0, 0.0],
            Easing::OutQuint,
        ),
        mo::shift(
            &id,
            end - uncover,
            uncover,
            [0.0, 0.0],
            [dir * travel, 0.0],
            Easing::InOutCubic,
        ),
    ];
    (vec![group], motions)
}

/// Two bold bars sweep vertically: accent first, the field colour trailing by
/// 0.06 s and covering it. On the way out the field leaves first, so the
/// accent bar reads as a stripe at the trailing edge.
fn kinetic(ctx: &Ctx, plan: &BeatPlan) -> (Vec<Layer>, Vec<Motion>) {
    const TRAIL: f64 = 0.06;
    let (w, h) = (ctx.w, ctx.h);
    let dir = if plan.index.is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let end = clear_at(ctx, plan);
    let (bx, by) = (w * BLEED, h * BLEED);
    let (pw, ph) = (w + 2.0 * bx, h + 2.0 * by);
    let travel = ph + 0.02 * h;
    let cover = end * 0.34;
    let leave = end * 0.38;

    let a_id = format!("{}.wipe_a", plan.prefix);
    let b_id = format!("{}.wipe_b", plan.prefix);
    let bar_a = rect_layer(a_id.clone(), (-bx, -by, pw, ph), ctx.palette.accent, 60);
    let bar_b = rect_layer(
        b_id.clone(),
        (-bx, -by, pw, ph),
        ctx.palette.field(plan.index),
        61,
    );

    let mut motions = Vec::new();
    for (id, in_at, out_end) in [(&a_id, 0.0, end), (&b_id, TRAIL, end - TRAIL)] {
        let out_at = (out_end - leave).max(in_at + cover);
        motions.push(mo::shift(
            id,
            in_at,
            cover,
            [0.0, dir * travel],
            [0.0, 0.0],
            Easing::OutQuint,
        ));
        motions.push(mo::shift(
            id,
            out_at,
            (out_end - out_at).max(0.05),
            [0.0, 0.0],
            [0.0, -dir * travel],
            Easing::InCubic,
        ));
    }
    (vec![bar_a, bar_b], motions)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::compiler::{plan_timing, ApproxMeasure, AssetLibrary, FontSet, Palette, Typesetter};
    use crate::intent::{CreativeIntent, Format};
    use crate::scene::MotionOp;
    use crate::style::StyleProfile;

    const INTENT: &str = r#"{"version":"0.2","title":"t","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"One","primary":{"kind":"phrase","value":"one"},"energy":"calm"},
        {"purpose":"emphasize","statement":"Two","primary":{"kind":"phrase","value":"two"},"energy":"building"},
        {"purpose":"emphasize","statement":"Three","primary":{"kind":"phrase","value":"three"},"energy":"calm"}]}"#;

    /// `(layers, motions, plan)` of beat `index` (0-based) for a style JSON.
    fn wipe_for(style_json: &str, index: usize) -> (Vec<Layer>, Vec<Motion>, BeatPlan) {
        wipe_on(style_json, index, false)
    }

    /// As [`wipe_for`]; `product` is a compile with `--variety`.
    fn wipe_on(
        style_json: &str,
        index: usize,
        product: bool,
    ) -> (Vec<Layer>, Vec<Motion>, BeatPlan) {
        let style: StyleProfile = serde_json::from_str(style_json).expect("style");
        let intent: CreativeIntent = serde_json::from_str(INTENT).expect("intent");
        let plans = plan_timing(&intent, &style);
        let library = AssetLibrary::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let fonts = FontSet::for_style(&style);
        let ts = Typesetter::new(&ApproxMeasure, &library.root, &fonts);
        let manifest = crate::assets::AssetManifest::empty();
        let ctx = Ctx {
            reveals: Default::default(),
            warnings: Vec::new(),
            direction_seed: product.then_some(1),
            beat_params: BTreeMap::new(),
            direction_take: None,
            direction_beats: BTreeMap::new(),
            emotion: None,
            art: None,
            w: 1080.0,
            h: 1920.0,
            u: 1.0,
            style: &style,
            taste: crate::compiler::taste::resolve(&style),
            palette: Palette::for_style(&style),
            ts,
            library: &library,
            assets: BTreeMap::new(),
            format: Format::Vertical,
            frame: crate::compiler::layout_frame::LayoutFrame::for_format(Format::Vertical),
            manifest: &manifest,
            image_carry: Default::default(),
            focal: Default::default(),
            stack: None,
            spoken: Default::default(),
            beat_count: 1,
            sequence: Default::default(),
        };
        let plan = plans[index].clone();
        let (l, m) = panel_wipe(&ctx, &plan);
        (l, m, plan)
    }

    /// Move offset of `target` at scene-local time `t`.
    fn offset(motions: &[Motion], target: &str, t: f64) -> [f32; 2] {
        let mut ms: Vec<&Motion> = motions.iter().filter(|m| m.target == target).collect();
        ms.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut cur = None;
        for m in ms {
            let MotionOp::Move { from, to } = m.op else {
                continue;
            };
            if t < m.start {
                return cur.unwrap_or(from);
            }
            if t < m.start + m.duration {
                let k = m.easing.apply((t - m.start) / m.duration) as f32;
                return [
                    from[0] + (to[0] - from[0]) * k,
                    from[1] + (to[1] - from[1]) * k,
                ];
            }
            cur = Some(to);
        }
        cur.unwrap_or([0.0, 0.0])
    }

    /// Does the (bled) rect of `layer` at offset `off` cover the whole canvas?
    fn covers(l: &Layer, off: [f32; 2]) -> bool {
        l.x + off[0] <= 0.0
            && l.y + off[1] <= 0.0
            && l.x + off[0] + l.width >= 1080.0
            && l.y + off[1] + l.height >= 1920.0
    }

    /// Does the layer's rect intersect the canvas at offset `off`?
    fn on_screen(l: &Layer, off: [f32; 2]) -> bool {
        l.x + off[0] < 1080.0
            && l.x + off[0] + l.width > 0.0
            && l.y + off[1] < 1920.0
            && l.y + off[1] + l.height > 0.0
    }

    #[test]
    fn classic_and_editorial_styles_have_no_wipe() {
        for style in [
            r#"{"seed":1}"#,
            r#"{"tone":"editorial"}"#,
            r#"{"tone":"technical","temperament":"restrained"}"#,
        ] {
            let (l, m, _) = wipe_for(style, 1);
            assert!(l.is_empty() && m.is_empty(), "{style}");
        }
    }

    #[test]
    fn geometric_wipe_covers_then_clears() {
        let (layers, motions, plan) = wipe_for(r#"{"tone":"technical"}"#, 1);
        assert!(plan.wipe_in);
        assert_eq!(layers.len(), 1);
        let LayerKind::Group { children } = &layers[0].kind else {
            panic!("group");
        };
        let panel = &children[0];
        assert_eq!(layers[0].z_index, 60);
        let id = &layers[0].id;
        // Off-frame at the start, covering mid PRE_ENTER, gone after enter.
        assert!(!on_screen(panel, offset(&motions, id, 0.0)));
        let mid = plan.enter_at() * 0.5 + 0.08;
        assert!(covers(panel, offset(&motions, id, mid)));
        let gone = plan.enter_at() + 0.15;
        for k in 0..=10 {
            let t = gone + k as f64 * 0.1;
            assert!(!on_screen(panel, offset(&motions, id, t)), "t={t}");
        }
    }

    #[test]
    fn geometric_direction_alternates() {
        let (_, m1, _) = wipe_for(r#"{"tone":"technical"}"#, 1);
        let (_, m2, _) = wipe_for(r#"{"tone":"technical"}"#, 2);
        let start = |ms: &[Motion]| match ms[0].op {
            MotionOp::Move { from, .. } => from[0],
            _ => 0.0,
        };
        assert!(start(&m1) * start(&m2) < 0.0);
    }

    #[test]
    fn kinetic_wipe_covers_then_clears() {
        let (layers, motions, plan) = wipe_for(r#"{"tone":"playful"}"#, 1);
        assert!(plan.wipe_in);
        assert_eq!(layers.len(), 2);
        let mid = plan.enter_at() * 0.5 + 0.08;
        let gone = plan.enter_at() + 0.15;
        for l in &layers {
            assert!(!on_screen(l, offset(&motions, &l.id, 0.0)));
            assert!(on_screen(l, offset(&motions, &l.id, mid)));
            assert!(covers(l, offset(&motions, &l.id, mid)), "{}", l.id);
            for k in 0..=10 {
                let t = gone + k as f64 * 0.1;
                assert!(!on_screen(l, offset(&motions, &l.id, t)), "{} t={t}", l.id);
            }
        }
        // Same-layer motions never overlap.
        for l in &layers {
            let mut ms: Vec<&Motion> = motions.iter().filter(|m| m.target == l.id).collect();
            ms.sort_by(|a, b| a.start.total_cmp(&b.start));
            for pair in ms.windows(2) {
                assert!(pair[0].start + pair[0].duration <= pair[1].start + 1e-9);
            }
        }
    }

    #[test]
    fn no_wipe_when_plan_does_not_request_one() {
        // Beat 0 never enters behind a wipe.
        let (l, m, plan) = wipe_for(r#"{"tone":"technical"}"#, 0);
        assert!(!plan.wipe_in);
        assert!(l.is_empty() && m.is_empty());
    }

    /// When the last motion of any layer in `motions` ends.
    fn ends(motions: &[Motion]) -> f64 {
        motions
            .iter()
            .map(|m| m.start + m.duration)
            .fold(0.0, f64::max)
    }

    #[test]
    fn on_the_product_path_a_wipe_covers_the_frame_for_at_most_four_tenths() {
        for tone in ["technical", "playful"] {
            let style = format!(r#"{{"tone":"{tone}"}}"#);
            let (_, plain, plan) = wipe_on(&style, 1, false);
            let (layers, product, _) = wipe_on(&style, 1, true);
            // Without --variety the wipe runs to ENTER + 0.13 s, as ever.
            assert!(
                (ends(&plain) - (plan.enter_at() + CLEAR_AFTER_ENTER)).abs() < 1e-3,
                "{tone}: {} vs {}",
                ends(&plain),
                plan.enter_at() + CLEAR_AFTER_ENTER
            );
            // With it, never longer than WIPE_MAX_S; still a wipe that covers
            // the frame and then clears.
            assert!(
                ends(&product) <= WIPE_MAX_S + 1e-9,
                "{tone}: {}",
                ends(&product)
            );
            assert!(ends(&product) > 0.3, "{tone}: {}", ends(&product));
            for l in &layers {
                assert!(
                    product.iter().filter(|m| m.target == l.id).count() >= 2,
                    "{tone}: {} still moves in and out",
                    l.id
                );
            }
        }
    }
}
