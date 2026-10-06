//! Density furniture (0.6): supporting annotation layers chosen by the
//! style's [`VisualDensityProfile`](super::taste::VisualDensityProfile)
//! (`annotation`: none / editorial / structured / graphic).
//!
//! Furniture never competes with the primary focus: it lives in the margins
//! and corners outside safe text areas, is set small (mono labels, hairline
//! rules, tick marks) or low-contrast, and arrives after the headline.
//!
//! Everything sits at low z (below the headline and cards) inside the stage
//! group, so it fades and exits with the stage. `AnnotationStyle::None`
//! (classic styles) adds nothing at all.

use super::recipes::B;
use super::taste::{AnnotationStyle, DensityLevel};
use super::typeset::{text_layer, Voice};
use super::{base_layer, mo, rect_layer, Ctx};
use crate::easing::Easing;
use crate::intent::Purpose;
use crate::scene::{Color, Direction, Layer, LayerKind, TextAlign};

/// z of all furniture: above the ghost word (0), below every content layer.
const Z: i32 = 4;

/// Add this beat's annotation layers to the stage.
pub(crate) fn furniture(ctx: &Ctx, b: &mut B) {
    // (0.23) Whatever the look, a note card around a picture hugs it.
    if ctx.direction_seed.is_some() {
        hug_note_card(ctx, b);
    }
    let dense = ctx.taste.density.level == DensityLevel::Dense;
    match ctx.taste.density.annotation {
        AnnotationStyle::None => {}
        AnnotationStyle::Editorial => editorial(ctx, b, dense),
        AnnotationStyle::Structured => structured(ctx, b, dense),
        AnnotationStyle::Graphic => graphic(ctx, b, dense),
    }
}

/// (0.23) The evidence stack lays its note (the secondary subject) on a card
/// (`<prefix>.note.card`, with an accent tab `<prefix>.note.tab` on its left
/// edge) sized for a line of type. When the note is a picture the card is a
/// frame around it, and layout QA (`frame_too_loose`) wants a frame to hug the
/// subject: at most `FRAME_PAD_MAX` of its size beyond its alpha bounds on
/// each side; a small picture on a card four times its size reads as a
/// mistake. The card (and its tab) is fitted to the picture's subject bounds
/// plus a hand's-breadth of paper; a note that is not a picture is left as it
/// is. The layers and their motions keep their ids.
fn hug_note_card(ctx: &Ctx, b: &mut B) {
    use crate::compiler::subject_rules::shrink_wrap;
    let card_id = b.id("note.card");
    let tab_id = b.id("note.tab");
    let prefix = b.id("note.");
    let Some(pic) = b.stage.iter().find(|l| {
        l.id.starts_with(&prefix)
            && l.id != card_id
            && l.id != tab_id
            && matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. })
    }) else {
        return;
    };
    // The picture's box on the canvas.
    let (w, h) = (
        pic.width * pic.scale_x.abs(),
        pic.height * pic.scale_y.abs(),
    );
    let (x, y) = (pic.x - pic.anchor_x * w, pic.y - pic.anchor_y * h);
    // Its subject: the analysed alpha bounds of the image (drawn contained
    // in the box), else the whole box.
    let subject = match &pic.kind {
        LayerKind::Image { asset, .. } => ctx
            .manifest
            .assets
            .iter()
            .find(|e| asset.strip_prefix("asset.") == Some(e.id.as_str()))
            .and_then(|e| {
                let a = e.analysis.as_ref()?;
                let (iw, ih) = (e.width as f32, e.height as f32);
                if iw <= 0.0 || ih <= 0.0 {
                    return None;
                }
                let s = (w / iw).min(h / ih);
                let (dw, dh) = (iw * s, ih * s);
                let (dx, dy) = (x + (w - dw) / 2.0, y + (h - dh) / 2.0);
                let sb = a.subject_bounds;
                Some((
                    dx + sb.x * dw,
                    dy + sb.y * dh,
                    (sb.width * dw).max(1.0),
                    (sb.height * dh).max(1.0),
                ))
            })
            .unwrap_or((x, y, w, h)),
        _ => (x, y, w, h),
    };
    let (cx, cy, cw, ch) = shrink_wrap(subject, NOTE_FRAME_PAD);
    let tab_w = 10.0 * ctx.u;
    for l in b.stage.iter_mut() {
        if l.id == card_id {
            (l.x, l.y, l.width, l.height) = (cx, cy, cw, ch);
        } else if l.id == tab_id {
            (l.x, l.y, l.width, l.height) = (cx, cy, tab_w.min(cw), ch);
        }
    }
}

/// (0.23) Paper around a note picture's subject, as a share of the subject's
/// size on each side (under `taste_rules::FRAME_PAD_MAX`, 12 %).
const NOTE_FRAME_PAD: f32 = 0.07;

/// Scene-local time the furniture arrives: after the headline has settled,
/// but always early enough to finish inside the beat.
fn arrival(b: &B) -> f64 {
    b.plan
        .life
        .settle
        .max(b.plan.enter_at() + 0.3)
        .min((b.plan.duration - 1.0).max(0.0))
}

/// Stage-space inset `(dx, dy)` from the canvas edges whose camera-pushed
/// position lands `dist` px inside the screen edge at the end of the beat.
/// The camera push (and track) scales the stage about the canvas centre, so
/// an element placed at the canvas margin drifts toward (and past) the edge;
/// furniture is placed so it is still at its margin *after* the push.
fn inset(ctx: &Ctx, b: &B, dist: f32) -> (f32, f32) {
    let cam = b.plan.lang.camera;
    let p = cam.push.max(1.0);
    let (cx, cy) = (ctx.w / 2.0, ctx.h / 2.0);
    let (tx, ty) = (cam.track[0].abs() * ctx.u, cam.track[1].abs() * ctx.u);
    (cx - (cx - dist - tx) / p, cy - (cy - dist - ty) / p)
}

/// (0.21) The beat's rank as a folio, e.g. `02`, only when the story counts
/// (a countdown or a list in order, [`super::sequence`]); `None` otherwise:
/// a number on screen says "item N", so ordinary beats carry none.
fn folio(ctx: &Ctx, b: &B) -> Option<String> {
    ctx.sequence
        .rank(b.plan.index)
        .map(super::sequence::rank_text)
}

/// A small text label; returns its `(width, height)`. Fades in at `t`.
#[allow(clippy::too_many_arguments)]
fn label(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    text: &str,
    size: f32,
    color: Color,
    at: (f32, f32),
    right_aligned: bool,
    t: f64,
) -> (f32, f32) {
    let block = ctx
        .ts
        .fit_line(Voice::LABEL, text, ctx.w - 2.0 * ctx.margin(), size);
    let id = b.id(name);
    let mut layer = text_layer(id.clone(), &block, color, TextAlign::Left);
    let width = layer.width;
    layer.x = if right_aligned { at.0 - width } else { at.0 };
    layer.y = at.1 - block.height();
    layer.z_index = Z;
    b.motions
        .push(mo::fade(&id, t, 0.5, 0.0, 1.0, Easing::OutCubic));
    let height = block.height();
    b.push(layer);
    (width, height)
}

/// A rule/rect that draws in from the left edge at `t`.
fn rule(b: &mut B, name: &str, rect: (f32, f32, f32, f32), color: Color, t: f64) {
    let id = b.id(name);
    let layer = rect_layer(id.clone(), rect, color, Z);
    b.motions
        .push(mo::mask(&id, t, 0.6, Direction::Right, Easing::OutCubic));
    b.push(layer);
}

fn purpose_word(p: Purpose) -> &'static str {
    match p {
        Purpose::Emphasize => "note",
        Purpose::Compare => "compare",
        Purpose::Contrast => "contrast",
        Purpose::Reveal => "result",
        Purpose::Explain => "why",
    }
}

/// Height of a label line at `size` (where the folio sits, with or without one).
fn label_height(ctx: &Ctx, size: f32) -> f32 {
    ctx.ts
        .fit_line(Voice::LABEL, "00", ctx.w - 2.0 * ctx.margin(), size)
        .height()
}

/// Editorial: a hairline rule along the bottom margin, with the folio number
/// at its right end when the story counts.
fn editorial(ctx: &Ctx, b: &mut B, dense: bool) {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let (m, my) = inset(ctx, b, ctx.margin());
    let t = arrival(b);
    let base = h - my;
    let th = match folio(ctx, b) {
        Some(n) => {
            label(
                ctx,
                b,
                "folio",
                &n,
                22.0 * u,
                ctx.palette.muted,
                (w - m, base),
                true,
                t,
            )
            .1
        }
        None => label_height(ctx, 22.0 * u),
    };
    let rule_y = base - th - 16.0 * u;
    rule(
        b,
        "folio_rule",
        (m, rule_y, w - 2.0 * m, 2.0 * u),
        ctx.palette.muted.with_alpha(0x90),
        t,
    );
    if dense {
        // A short accent tick where the rule starts.
        rule(
            b,
            "folio_tick",
            (m, rule_y - 6.0 * u, 40.0 * u, 8.0 * u),
            ctx.palette.accent,
            t + 0.1,
        );
    }
}

/// Structured (technical): coordinate ticks down the left edge, a hairline
/// rule and a mono data label along the bottom margin.
fn structured(ctx: &Ctx, b: &mut B, dense: bool) {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let (m, my) = inset(ctx, b, ctx.margin());
    let t = arrival(b);
    let muted = ctx.palette.muted;
    let base = h - my;
    let folio = folio(ctx, b);
    let text = match &folio {
        Some(n) => format!("SEC {n} · T+{:.1}", b.plan.start),
        None => format!("T+{:.1}", b.plan.start),
    };
    let (_, th) = label(ctx, b, "data", &text, 20.0 * u, muted, (m, base), false, t);
    if let Some(n) = &folio {
        label(
            ctx,
            b,
            "index",
            n,
            20.0 * u,
            ctx.palette.ink.with_alpha(0xB0),
            (w - m, base),
            true,
            t,
        );
    }
    rule(
        b,
        "data_rule",
        (m, base - th - 14.0 * u, w - 2.0 * m, 2.0 * u),
        muted.with_alpha(0x80),
        t,
    );

    // Tick scale in the left margin: every fifth tick is long.
    let ticks = 21;
    let (top, span) = (0.2 * h, 0.6 * h);
    let group_id = b.id("ticks");
    let children: Vec<Layer> = (0..ticks)
        .map(|i| {
            let len = if i % 5 == 0 { 22.0 * u } else { 11.0 * u };
            rect_layer(
                format!("{group_id}.{i}"),
                (0.0, span * i as f32 / (ticks - 1) as f32, len, 2.0 * u),
                muted.with_alpha(if i % 5 == 0 { 0xC0 } else { 0x80 }),
                0,
            )
        })
        .collect();
    let group = base_layer(
        group_id.clone(),
        (
            inset(ctx, b, 0.4 * ctx.margin()).0,
            top,
            22.0 * u,
            span + 2.0 * u,
        ),
        LayerKind::Group { children },
        Z,
    );
    b.motions
        .push(mo::fade(&group_id, t, 0.6, 0.0, 1.0, Easing::OutCubic));
    b.push(group);

    if dense {
        // Registration cross in the right margin, mid-height.
        let (cx, cy, arm) = (w - inset(ctx, b, 0.5 * ctx.margin()).0, 0.5 * h, 9.0 * u);
        rule(
            b,
            "reg_h",
            (cx - arm, cy - u, 2.0 * arm, 2.0 * u),
            muted.with_alpha(0xB0),
            t + 0.1,
        );
        rule(
            b,
            "reg_v",
            (cx - u, cy - arm, 2.0 * u, 2.0 * arm),
            muted.with_alpha(0xB0),
            t + 0.1,
        );
    }
}

/// Graphic (playful): a big index numeral bleeding off the bottom-right
/// corner and a rounded sticker tag in the bottom-left margin.
fn graphic(ctx: &Ctx, b: &mut B, dense: bool) {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let (m, my) = inset(ctx, b, ctx.margin());
    let (ex, ey) = inset(ctx, b, 0.0);
    let t = arrival(b);

    // Numeral (only when the story counts): ~18% off the right edge, ~14%
    // off the bottom edge.
    let folio = folio(ctx, b);
    if let Some(n) = &folio {
        let block = ctx.ts.fit_line(Voice::HEADLINE, n, w, 140.0 * u);
        let id = b.id("numeral");
        let mut layer = text_layer(
            id.clone(),
            &block,
            ctx.palette.field(b.plan.index + 1),
            TextAlign::Left,
        );
        layer.x = w - ex - layer.width * 0.82;
        layer.y = h - ey - block.height() * 0.86;
        layer.z_index = Z;
        b.motions
            .push(mo::fade(&id, t, 0.5, 0.0, 1.0, Easing::OutCubic));
        b.motions.push(mo::shift(
            &id,
            t,
            0.6,
            [0.0, 30.0 * u],
            [0.0, 0.0],
            Easing::OutQuint,
        ));
        b.push(layer);
    }

    // Sticker: rounded tag with the beat's role, tilted a few degrees.
    let tag_w = sticker(
        ctx,
        b,
        "sticker",
        purpose_word(b.beat.purpose),
        (m, h - my),
        t + 0.1,
    );
    if let (true, Some(n)) = (dense, &folio) {
        // A second tag beside the first: the item's rank.
        let n = format!("no. {n}");
        sticker(
            ctx,
            b,
            "sticker2",
            &n,
            (m + tag_w + 14.0 * u, h - my),
            t + 0.25,
        );
    }
}

/// A rounded tag whose bottom-left corner sits at `at`; returns its width.
fn sticker(ctx: &Ctx, b: &mut B, name: &str, text: &str, at: (f32, f32), t: f64) -> f32 {
    let u = ctx.u;
    let block = ctx.ts.fit_line(Voice::LABEL, text, ctx.w * 0.5, 22.0 * u);
    let (padx, pady) = (18.0 * u, 11.0 * u);
    let mut label_layer = text_layer(
        b.id(&format!("{name}.text")),
        &block,
        ctx.palette.on_accent,
        TextAlign::Left,
    );
    let (gw, gh) = (label_layer.width + 2.0 * padx, block.height() + 2.0 * pady);
    label_layer.x = padx;
    label_layer.y = pady;
    label_layer.z_index = 1;
    let mut pill = base_layer(
        b.id(&format!("{name}.pill")),
        (0.0, 0.0, gw, gh),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.accent,
            radius: 14.0 * u,
            stroke: None,
        },
        0,
    );
    pill.opacity = 1.0;
    let id = b.id(name);
    let mut group = base_layer(
        id.clone(),
        (at.0 + gw / 2.0, at.1 - gh / 2.0, gw, gh),
        LayerKind::Group {
            children: vec![pill, label_layer],
        },
        Z,
    );
    group.anchor_x = 0.5;
    group.anchor_y = 0.5;
    group.rotation_degrees = -3.0;
    b.motions
        .push(mo::fade(&id, t, 0.35, 0.0, 1.0, Easing::OutCubic));
    b.motions
        .push(mo::scale(&id, t, 0.45, 0.85, 1.0, Easing::OutQuint));
    b.push(group);
    gw
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::compiler::{plan_timing, ApproxMeasure, AssetLibrary, FontSet, Palette, Typesetter};
    use crate::intent::{CreativeIntent, Format};
    use crate::scene::Motion;
    use crate::style::StyleProfile;

    const INTENT: &str = r#"{"version":"0.2","title":"t","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"One","primary":{"kind":"phrase","value":"one"},"energy":"calm"},
        {"purpose":"contrast","statement":"Two","primary":{"kind":"phrase","value":"two"},"energy":"building"}]}"#;

    /// Stage layers + motions added by `furniture` for beat 2 of `INTENT`.
    fn added(style_json: &str) -> (Vec<Layer>, Vec<Motion>, f64) {
        added_ranked(style_json, None)
    }

    /// The same, with beat 2 ranked `rank` (a story that counts).
    fn added_ranked(style_json: &str, rank: Option<u32>) -> (Vec<Layer>, Vec<Motion>, f64) {
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
            direction_seed: None,
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
            sequence: crate::compiler::sequence::Sequence {
                order: rank.map(|_| crate::compiler::sequence::Order::Ascending),
                ranks: vec![rank.map(|r| r - 1), rank],
            },
        };
        let mut b = B {
            reveals: Vec::new(),
            plan: &plans[1],
            beat: &intent.beats[1],
            stage: Vec::new(),
            motions: Vec::new(),
            anchor: None,
            placed: Vec::new(),
            split_above: None,
            focal: None,
        };
        furniture(&ctx, &mut b);
        (b.stage, b.motions, plans[1].life.settle)
    }

    fn has(layers: &[Layer], needle: &str) -> bool {
        layers.iter().any(|l| l.id.contains(needle))
    }

    #[test]
    fn classic_and_sparse_add_nothing() {
        for style in [
            r#"{"seed":1}"#,
            r#"{"tone":"technical","density":"sparse"}"#,
            r#"{"tone":"playful","density":"sparse"}"#,
        ] {
            let (layers, motions, _) = added(style);
            assert!(layers.is_empty() && motions.is_empty(), "{style}");
        }
    }

    /// Every text, including those inside groups (stickers).
    fn texts(layers: &[Layer]) -> Vec<String> {
        layers
            .iter()
            .flat_map(|l| match &l.kind {
                LayerKind::Text(t) => vec![t.text.clone()],
                LayerKind::Group { children } => texts(children),
                _ => Vec::new(),
            })
            .collect()
    }

    /// (0.21) A number on screen only when the story counts.
    fn has_number(layers: &[Layer]) -> bool {
        texts(layers)
            .iter()
            .any(|t| t.chars().any(|c| c.is_ascii_digit()) && !t.starts_with("T+"))
    }

    #[test]
    fn editorial_adds_a_rule_and_a_folio_only_when_the_story_counts() {
        let (layers, _, _) = added(r#"{"tone":"editorial"}"#);
        assert!(has(&layers, "folio_rule"));
        assert!(!layers.iter().any(|l| l.id.ends_with(".folio")));
        assert!(!has_number(&layers), "{:?}", texts(&layers));
        let (ranked, _, _) = added_ranked(r#"{"tone":"editorial"}"#, Some(2));
        assert!(ranked.iter().any(|l| l.id.ends_with(".folio")));
        assert!(texts(&ranked).contains(&"02".to_string()));
        // The rule sits at the same height either way.
        let rule_y = |ls: &[Layer]| ls.iter().find(|l| l.id.ends_with("folio_rule")).unwrap().y;
        assert!((rule_y(&layers) - rule_y(&ranked)).abs() < 1e-3);
    }

    #[test]
    fn technical_adds_index_ticks_and_data_label() {
        let (plain, _, _) = added(r#"{"tone":"technical"}"#);
        assert!(!has(&plain, ".index"));
        assert!(texts(&plain).iter().any(|t| t.starts_with("T+")));
        assert!(!has_number(&plain), "{:?}", texts(&plain));
        let (layers, motions, settle) = added_ranked(r#"{"tone":"technical"}"#, Some(2));
        assert!(has(&layers, ".ticks"));
        assert!(has(&layers, ".index"));
        assert!(layers.iter().any(|l| matches!(&l.kind,
            LayerKind::Text(t) if t.text.starts_with("SEC 02 · T+"))));
        // Arrives after the headline settled; every layer is animated in.
        assert!(motions.iter().all(|m| m.start >= settle - 1e-9));
        for l in &layers {
            assert!(l.z_index < 10, "{} sits below content", l.id);
            assert!(
                motions.iter().any(|m| m.target == l.id),
                "{} has an entrance",
                l.id
            );
        }
    }

    #[test]
    fn playful_adds_numeral_and_sticker_and_dense_adds_more() {
        let (plain, _, _) = added(r#"{"tone":"playful","density":"dense"}"#);
        assert!(has(&plain, ".sticker"));
        assert!(!has(&plain, ".numeral") && !has(&plain, ".sticker2"));
        assert!(!has_number(&plain), "{:?}", texts(&plain));
        let (layers, _, _) = added_ranked(r#"{"tone":"playful"}"#, Some(2));
        assert!(has(&layers, ".numeral"));
        assert!(has(&layers, ".sticker"));
        let (balanced, _, _) = added_ranked(r#"{"tone":"playful","density":"balanced"}"#, Some(2));
        let (dense, _, _) = added_ranked(r#"{"tone":"playful","density":"dense"}"#, Some(2));
        assert!(dense.len() > balanced.len());
        assert!(
            texts(&dense)
                .iter()
                .any(|t| t.eq_ignore_ascii_case("no. 02")),
            "{:?}",
            texts(&dense)
        );
    }

    #[test]
    fn furniture_stays_inside_the_canvas_margins() {
        for style in [r#"{"tone":"editorial"}"#, r#"{"tone":"technical"}"#] {
            let (layers, _, _) = added(style);
            for l in &layers {
                assert!(l.x >= 0.0 && l.x + l.width <= 1080.0, "{} x", l.id);
                assert!(l.y >= 0.0 && l.y + l.height <= 1920.0, "{} y", l.id);
            }
        }
    }
}
