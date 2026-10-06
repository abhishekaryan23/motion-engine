//! EvidenceStack: document / photo / number / source / highlight as a layered stack of evidence.
//!
//! ENTER: the statement headline, a tilted evidence card (a delivered
//! `evidence_image` on paper sheets, or the object on a collage card) and a
//! mono source label. EVOLVE events: a highlight bar sweeps over the key
//! phrase, then the number / phrase subject lands as a note card.

use super::placement;
use super::plate::{self, events};
use super::{Composition, Variant};
use crate::assets::{AssetRole, ManifestEntry};
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, mo, rect_layer, Carry, CompileError, Ctx, Which};
use crate::easing::Easing;
use crate::intent::SubjectKind;
use crate::scene::{Direction, LayerKind, Stroke, TextAlign};

/// Below this subject coverage a transparent evidence image is an object
/// cutout, not a flat sheet.
const OBJECT_CUTOUT_COVERAGE: f32 = 0.6;
/// Pad (share of the picture's subject size, per side) of the card behind a
/// delivered object picture.
const PICTURE_CARD_PAD: f32 = 0.06;
/// Layout QA's minimum type size (px) for labels.
const LABEL_FLOOR_PX: f32 = 20.0;

/// A paper sheet behind the evidence (tilted, revealed upward).
#[allow(clippy::too_many_arguments)]
fn sheet(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    center: (f32, f32),
    size: (f32, f32),
    tilt: f32,
    z: i32,
    start: f64,
) {
    let id = b.id(name);
    let mut l = base_layer(
        id.clone(),
        (center.0, center.1, size.0, size.1),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: 8.0 * ctx.u,
            stroke: Some(Stroke {
                color: ctx.palette.ink.with_alpha(0x22),
                width: 2.0 * ctx.u,
            }),
        },
        z,
    );
    l.anchor_x = 0.5;
    l.anchor_y = 0.5;
    l.rotation_degrees = tilt;
    b.motions
        .push(mo::mask(&id, start, 0.7, Direction::Up, Easing::OutQuint));
    b.push(l);
}

/// (0.23 W8a) The evidence picture as a shared element, when the story
/// carries it across beats (`None`: an ordinary picture of this beat).
#[allow(clippy::too_many_arguments)]
fn carry_picture(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    which: Which,
    entry: &ManifestEntry,
    role: AssetRole,
    rect: (f32, f32, f32, f32),
    z: i32,
    tilt: f32,
    start: f64,
) -> Option<plate::Carried> {
    plate::place_carried(
        ctx,
        b,
        carries,
        which,
        entry,
        role,
        rect,
        z,
        None,
        tilt,
        start,
        plate::Arrive {
            offset: (0.0, 40.0 * ctx.u),
            duration: 0.7,
        },
    )
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
    let t = plan.enter_at();
    let mirror = c.variant == Variant::Mirror;
    let sign = if mirror { -1.0 } else { 1.0 };
    let land = placement::side_by_side(&ctx.frame);

    recipes::ghost(ctx, b, 0.6 * h);

    // Headline (the statement); its key phrase gets highlighted at EVOLVE.
    let top = 0.17 * h;
    let (text_w, head_max_h) = if land {
        (0.5 * w - m, 0.5 * h)
    } else {
        (w - 2.0 * m, 0.2 * h)
    };
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &beat.statement,
        text_w,
        head_max_h,
        128.0 * u,
        3,
    );
    let (head_h, _) = recipes::headline_lines(
        ctx,
        b,
        "head",
        &head,
        top,
        TextAlign::Left,
        ctx.palette.ink,
        t + 0.05,
        true,
    );

    // Which subject is the evidence object, which is the note.
    let (obj_which, obj, note) = match (&beat.primary, &beat.secondary) {
        (p, s) if p.kind() == SubjectKind::Object => (Which::Primary, p, s.as_ref()),
        (p, Some(s)) => (Which::Secondary, s, Some(p)),
        (p, None) => (Which::Primary, p, None),
    };
    let note_which = if obj_which == Which::Primary {
        Which::Secondary
    } else {
        Which::Primary
    };

    // Evidence card. A delivered document sizes it from the image aspect
    // (contain-fit inside the maximum card box, so nothing is letterboxed);
    // portrait artifacts may use a taller box than landscape ones.
    let evidence = plate::image(ctx, plan.index, AssetRole::EvidenceImage);
    // A transparent cutout of an OBJECT (its subject covers well under the
    // frame, e.g. a key) is the evidence itself: no paper sheets behind it,
    // sized by what it depicts (subject bounds), not its transparent margins.
    // Sheet-like cutouts (documents, receipts flattened to a page) and opaque
    // scans keep the paper stack.
    let cutout = evidence.is_some_and(|e| {
        e.alpha
            && e.analysis
                .as_ref()
                .is_some_and(|a| a.coverage < OBJECT_CUTOUT_COVERAGE)
    });
    // (0.14) A delivered picture of the object (hero / supporting object
    // image) IS the evidence: it is sized up (the lower half of the frame
    // was empty) and its card hugs what it depicts (taste rule: frames hug
    // the asset) instead of floating small in a wide generic card.
    let obj_picture = if evidence.is_none() && obj.kind() == SubjectKind::Object {
        [AssetRole::HeroObject, AssetRole::SupportingObject]
            .into_iter()
            .find_map(|r| plate::image(ctx, plan.index, r).map(|e| (e, r)))
    } else {
        None
    };
    let picture_size = obj_picture.and_then(|(e, _)| {
        if e.width == 0 || e.height == 0 {
            return None;
        }
        let facts = placement::SubjectFacts::from_entry(e);
        let full = placement::Rect::new(0.0, 0.0, e.width as f32, e.height as f32);
        let sub = placement::to_canvas(&full, facts.subject);
        let aspect = (sub.w / sub.h.max(1.0)).max(0.05);
        let (max_w, max_h) = if land {
            (0.4 * w, 0.62 * h)
        } else {
            ((w - 2.0 * m) * 0.86, 0.4 * h)
        };
        let sw = max_w.min(max_h * aspect);
        Some((sw, sw / aspect))
    });
    let (cw, ch) = match (evidence, land) {
        _ if picture_size.is_some() => {
            let (sw, sh) = picture_size.unwrap_or_default();
            (
                sw * (1.0 + 2.0 * PICTURE_CARD_PAD),
                sh * (1.0 + 2.0 * PICTURE_CARD_PAD),
            )
        }
        (Some(e), _) if e.width > 0 && e.height > 0 => {
            let portrait = e.height > e.width;
            let (max_w, max_h) = if land {
                (0.38 * w, 0.6 * h)
            } else {
                (
                    (w - 2.0 * m) * 0.82,
                    if portrait { 0.32 * h } else { 0.26 * h },
                )
            };
            let s = (max_w / e.width as f32).min(max_h / e.height as f32);
            (e.width as f32 * s, e.height as f32 * s)
        }
        (_, true) => (0.38 * w, 0.6 * h),
        (_, false) => ((w - 2.0 * m) * 0.82, 0.26 * h),
    };
    let (cx, cy) = if land {
        (if mirror { 0.28 * w } else { 0.72 * w }, 0.5 * h)
    } else {
        (
            w / 2.0 + sign * 0.03 * w,
            top + head_h + 0.05 * h + ch / 2.0,
        )
    };
    let tilt = -3.0 * sign;
    let start = t + 0.3;
    if let Some(e) = evidence {
        let rect = if cutout {
            let facts = placement::SubjectFacts::from_entry(e);
            let img = placement::subject_fit(
                &facts,
                placement::Rect::new(cx - cw / 2.0, cy - ch / 2.0, cw, ch),
            );
            (img.x, img.y, img.w, img.h)
        } else {
            // (0.10 Q) The paper sheets are the evidence's own bounds (the alpha
            // bounds of a page-like cutout, the whole image when opaque), not
            // its transparent margins: a sheet never exceeds what it backs
            // (taste_rules::FRAME_PAD_MAX); the offset and tilt make it peek out.
            let image_box = placement::Rect::new(cx - cw / 2.0, cy - ch / 2.0, cw, ch);
            let sub =
                placement::to_canvas(&image_box, placement::SubjectFacts::from_entry(e).subject);
            let (scx, scy) = (sub.x + sub.w / 2.0, sub.y + sub.h / 2.0);
            sheet(
                ctx,
                b,
                "sheet.0",
                (scx + 16.0 * u, scy + 18.0 * u),
                (sub.w, sub.h),
                -tilt * 0.7,
                9,
                start,
            );
            sheet(
                ctx,
                b,
                "sheet.1",
                (scx - 8.0 * u, scy + 8.0 * u),
                (sub.w, sub.h),
                tilt * 0.3,
                10,
                start + 0.05,
            );
            (cx - cw / 2.0, cy - ch / 2.0, cw, ch)
        };
        // (0.23 W8a) The picture of a carried object is a shared element.
        let carried = (obj.kind() == SubjectKind::Object)
            .then(|| {
                carry_picture(
                    ctx,
                    b,
                    carries,
                    obj_which,
                    e,
                    AssetRole::EvidenceImage,
                    rect,
                    12,
                    tilt,
                    start + 0.1,
                )
            })
            .flatten();
        if carried.is_none() {
            let mut doc = plate::image_plate(
                ctx,
                b,
                "doc",
                AssetRole::EvidenceImage,
                rect,
                12,
                start + 0.1,
            );
            for l in &mut doc.layers {
                l.rotation_degrees = tilt;
            }
            b.motions.push(mo::rotate(
                &doc.id,
                start + 0.1,
                1.2,
                tilt * 1.6,
                tilt,
                Easing::OutCubic,
            ));
            for l in doc.layers {
                b.push(l);
            }
        }
    } else if let (Some((e, role)), Some((sw, sh))) = (obj_picture, picture_size) {
        let mut layers = Vec::new();
        recipes::collage_card(
            ctx,
            b,
            "doc",
            (cx, cy),
            (cw, ch),
            tilt,
            (-sign, -1.0),
            start,
            &mut layers,
        );
        for l in layers {
            b.push(l);
        }
        let facts = placement::SubjectFacts::from_entry(e);
        let img = placement::subject_fit(
            &facts,
            placement::Rect::new(cx - sw / 2.0, cy - sh / 2.0, sw, sh),
        );
        // (0.23 W8a) The picture of a carried object is a shared element.
        let rect = (img.x, img.y, img.w, img.h);
        if carry_picture(
            ctx,
            b,
            carries,
            obj_which,
            e,
            role,
            rect,
            22,
            tilt,
            start + 0.25,
        )
        .is_none()
        {
            let pic = plate::image_plate(ctx, b, "evidence", role, rect, 22, start + 0.25);
            for mut l in pic.layers {
                l.rotation_degrees = tilt;
                b.push(l);
            }
        }
    } else {
        sheet(
            ctx,
            b,
            "sheet.0",
            (cx + 14.0 * u, cy + 16.0 * u),
            (cw, ch),
            -tilt * 0.8,
            9,
            start,
        );
        let mut layers = Vec::new();
        recipes::collage_card(
            ctx,
            b,
            "doc",
            (cx, cy),
            (cw, ch),
            tilt,
            (-sign, -1.0),
            start,
            &mut layers,
        );
        for l in layers {
            b.push(l);
        }
        let side = cw.min(ch) * 0.78;
        let slot = Slot {
            cx,
            cy,
            w: side,
            h: side,
        };
        if let Some(mut l) = recipes::place_subject(
            ctx,
            b,
            carries,
            obj_which,
            obj,
            slot,
            (0.0, 0.0),
            ctx.palette.ink,
            start + 0.25,
            Entrance::Rise,
            None,
            None,
            "evidence",
        )? {
            l.z_index = 22;
            b.push(l);
        }
    }

    // Source label (the object's meaning, mono).
    let source = obj
        .meaning()
        .map(str::to_string)
        .or_else(|| obj.asset().map(|a| a.replace(['_', '-'], " ")))
        .unwrap_or_else(|| "document".into());
    let source_text = format!("SOURCE — {source}");
    let mut block = ctx.ts.fit_line(Voice::LABEL, &source_text, cw, 26.0 * u);
    if block.size < ctx.frame.min_type_px && !placement::is_legacy_canvas(&ctx.frame) {
        // A narrow card must not shrink the label below the legibility floor:
        // let it run on to the right margin (the legacy canvases are frozen).
        let room = (w - m - (cx - cw / 2.0)).max(cw);
        block = ctx.ts.fit_line(Voice::LABEL, &source_text, room, 26.0 * u);
    }
    let mut label_x = cx - cw / 2.0;
    // (Picture cards are new layout, so the legacy-canvas freeze does not apply.)
    let may_refit = picture_size.is_some() || !placement::is_legacy_canvas(&ctx.frame);
    if block.size < ctx.frame.min_type_px.max(LABEL_FLOOR_PX) && may_refit {
        // (0.14) Still too small beside a narrow card: start at the margin.
        label_x = m;
        block = ctx
            .ts
            .fit_line(Voice::LABEL, &source_text, w - 2.0 * m, 26.0 * u);
    }
    let label_id = b.id("source");
    let mut label = text_layer(label_id.clone(), &block, ctx.palette.muted, TextAlign::Left);
    label.x = label_x;
    label.y = cy + ch / 2.0 + 34.0 * u;
    label.z_index = 24;
    b.motions.push(mo::line_in(
        &label_id,
        start + 0.55,
        plan.lang.preset.duration * 0.9,
        Direction::Up,
        plan.lang.preset.easing,
    ));
    let label_bottom = label.y + block.height();
    b.push(label);

    // EVOLVE: highlight sweep over the key phrase, then the note card.
    let note = note.filter(|s| s.display_text().is_some() || s.kind() == SubjectKind::Object);
    let tail = note.map_or(0.65, |n| plate::subject_tail(b, n).max(0.65));
    let ev = events(b, 1 + usize::from(note.is_some()), tail);
    let (bx, by, bw, bh) = key_phrase_rect(ctx, &head, top, beat.keyword.as_deref(), m);
    let hl_id = b.id("highlight");
    let hl = rect_layer(
        hl_id.clone(),
        (bx, by, bw, bh),
        ctx.palette.accent.with_alpha(0x59),
        26,
    );
    b.motions.push(mo::mask(
        &hl_id,
        ev[0],
        0.6,
        Direction::Right,
        Easing::OutQuint,
    ));
    b.push(hl);

    if let (Some(note), Some(&at)) = (note, ev.get(1)) {
        let (nw, nh) = if land {
            (0.36 * w, 0.14 * h)
        } else {
            (0.56 * w, 0.115 * h)
        };
        let ncx = if land {
            if mirror {
                w - m - nw / 2.0
            } else {
                m + nw / 2.0
            }
        } else if mirror {
            m + nw / 2.0
        } else {
            w - m - nw / 2.0
        };
        let ncy = if land {
            0.74 * h
        } else {
            (label_bottom + 0.07 * h + nh / 2.0).min(0.93 * h - nh / 2.0)
        };
        let slot = Slot {
            cx: ncx + 12.0 * u,
            cy: ncy,
            w: nw * 0.8,
            h: nh * 0.68,
        };
        let color = ctx.palette.ink;
        if let Some(mut l) = recipes::place_subject(
            ctx,
            b,
            carries,
            note_which,
            note,
            slot,
            (0.0, 0.0),
            color,
            at,
            Entrance::Rise,
            None,
            None,
            "note",
        )? {
            let card_id = b.id("note.card");
            let mut card = base_layer(
                card_id.clone(),
                (ncx - nw / 2.0, ncy - nh / 2.0, nw, nh),
                LayerKind::RoundedRectangle {
                    fill: ctx.palette.card,
                    radius: 8.0 * u,
                    stroke: Some(Stroke {
                        color: ctx.palette.ink.with_alpha(0x22),
                        width: 2.0 * u,
                    }),
                },
                14,
            );
            card.rotation_degrees = 0.0;
            b.motions
                .push(mo::mask(&card_id, at, 0.6, Direction::Up, Easing::OutQuint));
            let tab_id = b.id("note.tab");
            b.motions.push(mo::mask(
                &tab_id,
                at + 0.1,
                0.5,
                Direction::Down,
                Easing::OutQuint,
            ));
            b.push(card);
            b.push(rect_layer(
                tab_id,
                (ncx - nw / 2.0, ncy - nh / 2.0, 10.0 * u, nh),
                ctx.palette.accent,
                15,
            ));
            l.z_index = 22;
            b.push(l);
        }
    }

    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

/// Box (x, y, w, h) of the key phrase inside the left-aligned headline: the
/// keyword when the headline contains it, else the first line.
fn key_phrase_rect(
    ctx: &Ctx,
    head: &crate::compiler::typeset::Block,
    top: f32,
    keyword: Option<&str>,
    margin: f32,
) -> (f32, f32, f32, f32) {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let key = keyword.map(norm).filter(|k| !k.is_empty());
    let found = key.and_then(|k| {
        head.lines.iter().enumerate().find_map(|(li, line)| {
            let words: Vec<&str> = line.split(' ').collect();
            words
                .iter()
                .position(|wd| norm(wd) == k)
                .map(|wi| (li, words, wi))
        })
    });
    let pad = 8.0 * ctx.u;
    let (li, x0, width) = match found {
        Some((li, words, wi)) => {
            let x0 = if wi == 0 {
                0.0
            } else {
                ctx.ts.width(
                    &head.voice,
                    &format!("{} ", words[..wi].join(" ")),
                    head.size,
                )
            };
            (li, x0, ctx.ts.width(&head.voice, words[wi], head.size))
        }
        None => (0, 0.0, head.line_widths.first().copied().unwrap_or(0.0)),
    };
    (
        margin + x0 - pad,
        top + li as f32 * head.line_advance() + head.size * 0.1,
        width + 2.0 * pad,
        head.size * 0.82,
    )
}
