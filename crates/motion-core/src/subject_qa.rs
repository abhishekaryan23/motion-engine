//! (0.10 Q) Layout QA for delivered subject images: the four `taste_rules`
//! budgets that need to know what an image actually depicts.
//!
//! | check | rule |
//! |---|---|
//! | `text_over_subject` | text drawn over a subject covers at most `TEXT_OVER_SUBJECT_MAX` of its alpha bounds, and never any of its head region (even behind it). Text set on its own opaque card or strip is a label, not type printed over the picture: it is exempt from the area rule, not from the head rule |
//! | `subject_too_small` | a beat's hero subject (`<beat>.subject`, or a carried `shared.asset.*`) has alpha bounds of at least `SUBJECT_MIN_AREA` of the canvas |
//! | `frame_too_loose` | a card behind a subject exceeds its alpha bounds by at most `FRAME_PAD_MAX` of the subject size on each side |
//! | `asset_low_contrast` | a subject's treated mean colour has `ASSET_GROUND_MIN_CONTRAST` against what touches it (the ground, the card it sits on, its paper edge), or it carries a sticker / keyline that does |
//!
//! The project carries no pixels, so the caller supplies what the analysis
//! measured per image ([`ImageIndex`], keyed by the project's asset path):
//! `motion_render::image_index` builds it from the files, [`ImageIndex::from_manifest`]
//! from a manifest. Without an index (or for an image it does not know) these
//! checks are skipped. Subjects are the beat's delivered or library image
//! layers; ghost words, background fields, furniture and the caption scene are
//! exempt, as for every layout check. Geometry is the resolved box on the
//! canvas at the sample (camera and stage included), so text and subject are
//! compared in the same space.

use std::collections::BTreeMap;

use crate::assets::{AssetManifest, ManifestEntry, NormBox};
use crate::compiler::explore::contrast_ratio;
use crate::compiler::layout_frame::{LayoutFrame, Rect};
use crate::compiler::subject_rules::{
    needs_contrast_treatment, neighbours, subject_min_area_for, treated_color, EDGE_SEPARATES_U,
};
use crate::compiler::taste_rules::{
    ASSET_GROUND_MIN_CONTRAST, FRAME_PAD_MAX, TEXT_OVER_SUBJECT_MAX,
};
use crate::layout_qa::{is_decorative, LayoutCheck, LayoutFinding};
use crate::scene::{Channel, Color, Fit, ImageTreatment, LayerKind, MotionProject, Scene};
use crate::timeline::{Affine, ResolvedLayer};

/// Share of a head region a text box may touch before it counts as covering it.
const HEAD_TOLERANCE: f32 = 0.02;
/// A card holds a subject when it covers at least this share of its bounds.
const FRAME_HOLDS: f32 = 0.6;
/// A card is a text's backing when it contains at least this share of its box.
const BACKING_CONTAINS: f32 = 0.85;
/// Slack on the frame pad (rotation and float noise), as a fraction of the subject.
const FRAME_SLACK: f32 = 0.01;
/// Images drawn larger than this share of the canvas are background plates.
const PLATE_SHARE: f32 = 0.8;
/// Opacity at which an image or text counts as shown.
const SHOWN_IMAGE: f32 = 0.9;
const SHOWN_TEXT: f32 = 0.5;
/// A card must be at least this opaque to count as a ground or a backing.
const OPAQUE_CARD: f32 = 0.9;
/// A motion at least this long, or scaling by at most [`GENTLE_SCALE`], or
/// moving at most [`GENTLE_MOVE_PX`], is a gentle drift, not an entrance.
const GENTLE_SECONDS: f64 = 2.5;
const GENTLE_SCALE: f32 = 0.05;
const GENTLE_MOVE_PX: f32 = 12.0;

/// What the analysis measured about one delivered image.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageFacts {
    pub width: u32,
    pub height: u32,
    /// The image is a cutout.
    pub alpha: bool,
    /// Alpha bounds (the whole image when opaque).
    pub subject: NormBox,
    /// Head region, for people (supplied, estimated, or around the face anchor).
    pub head: Option<NormBox>,
    /// Mean subject colour (sRGB); `None` when never measured.
    pub mean_color: Option<[u8; 3]>,
}

impl ImageFacts {
    /// Facts from a manifest entry (its analysis, head and face metadata).
    pub fn from_entry(e: &ManifestEntry) -> Self {
        let analysis = e.analysis.as_ref();
        let subject = analysis
            .map(|a| a.subject_bounds)
            .or(e.safe_bounds)
            .unwrap_or(NormBox {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            });
        // Same head box as `placement::SubjectFacts` uses for a person.
        let head = e.head_region().or_else(|| {
            e.face_anchor.map(|p| NormBox {
                x: (p.x - 0.14).max(0.0),
                y: (p.y - 0.12).max(0.0),
                width: 0.28,
                height: 0.22,
            })
        });
        ImageFacts {
            width: e.width,
            height: e.height,
            alpha: e.alpha,
            subject,
            head,
            mean_color: analysis.and_then(|a| a.mean_color),
        }
    }
}

/// Image facts keyed by the project's asset path (`Asset.path`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageIndex {
    by_path: BTreeMap<String, ImageFacts>,
}

impl ImageIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: impl Into<String>, facts: ImageFacts) {
        self.by_path.insert(path.into(), facts);
    }

    pub fn get(&self, path: &str) -> Option<&ImageFacts> {
        self.by_path.get(path)
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// An index over a manifest's entries (keyed by their paths, which are the
    /// project's asset paths when the compiler placed them).
    pub fn from_manifest(manifest: &AssetManifest) -> Self {
        let mut index = ImageIndex::new();
        for e in &manifest.assets {
            index.insert(e.path.clone(), ImageFacts::from_entry(e));
        }
        index
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

fn area(r: &Rect) -> f32 {
    r.w.max(0.0) * r.h.max(0.0)
}

fn overlap(a: &Rect, b: &Rect) -> f32 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if w > 0.0 && h > 0.0 {
        w * h
    } else {
        0.0
    }
}

/// Canvas bounds of the local rectangle `(x, y, w, h)` under `xf`.
fn aabb(xf: &Affine, x: f32, y: f32, w: f32, h: f32) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
        let (cx, cy) = xf.apply(px, py);
        x0 = x0.min(cx);
        y0 = y0.min(cy);
        x1 = x1.max(cx);
        y1 = y1.max(cy);
    }
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

fn contains_point(r: &Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

/// Whether a rounded rectangle of corner `radius` covers the point.
fn rounded_contains(r: &Rect, radius: f32, px: f32, py: f32) -> bool {
    if !contains_point(r, px, py) {
        return false;
    }
    let rad = radius.clamp(0.0, 0.5 * r.w.min(r.h));
    // A disc (radius = half the side) leaves a zero-width range that float
    // rounding can invert: clamp with ordered bounds.
    let ordered = |v: f32, a: f32, b: f32| v.clamp(a.min(b), a.max(b));
    let cx = ordered(px, r.x + rad, r.x + r.w - rad);
    let cy = ordered(py, r.y + rad, r.y + r.h - rad);
    (px - cx).hypot(py - cy) <= rad + 0.5
}

/// Local box rectangle an image of `facts` is drawn in (its `fit`).
fn drawn_local(layer: &ResolvedLayer<'_>, fit: Fit, facts: &ImageFacts) -> (f32, f32, f32, f32) {
    let (lw, lh) = (layer.width, layer.height);
    let (iw, ih) = (facts.width.max(1) as f32, facts.height.max(1) as f32);
    match fit {
        Fit::Contain => {
            let s = (lw / iw).min(lh / ih);
            let (w, h) = (iw * s, ih * s);
            ((lw - w) / 2.0, (lh - h) / 2.0, w, h)
        }
        Fit::Cover | Fit::Fill => (0.0, 0.0, lw, lh),
    }
}

/// The role segment of a layer id: `b3.subject` -> `subject`; ids without a
/// beat prefix (shared elements) -> their first segment.
fn role_of(id: &str) -> &str {
    let mut segs = id.split('.');
    let first = segs.next().unwrap_or("");
    let prefixed =
        first.starts_with('b') && first.len() > 1 && first[1..].chars().all(|c| c.is_ascii_digit());
    if prefixed {
        segs.next().unwrap_or("")
    } else {
        first
    }
}

/// A beat's hero subject: the TypeImageInterlock image or a carried one.
fn is_hero_subject(id: &str) -> bool {
    role_of(id) == "subject" || id.starts_with("shared.asset.")
}

// ---------------------------------------------------------------------------
// Flattened frame
// ---------------------------------------------------------------------------

/// One drawn (non-group) layer, in draw order.
struct Leaf<'b, 'a> {
    layer: &'b ResolvedLayer<'a>,
    /// Opacity including every ancestor.
    opacity: f32,
    /// A move / scale / rotate / reveal motion is running (entrance, reframe).
    moving: bool,
    /// Belongs to the sampled scene (or a shared element it uses).
    mine: bool,
}

fn is_stage(id: &str) -> bool {
    id.ends_with(".stage") || id.ends_with(".stage_front")
}

/// A slow, small drift (the READ push on a subject, a hero's 3 % creep, a
/// parallax glide) keeps the layout as authored: it is not "in flight".
fn is_gentle(m: &crate::scene::Motion) -> bool {
    use crate::scene::MotionOp;
    m.duration >= GENTLE_SECONDS
        || match &m.op {
            MotionOp::Scale { from, to, .. } => (to - from).abs() <= GENTLE_SCALE,
            MotionOp::Move { from, to } => {
                (to[0] - from[0]).hypot(to[1] - from[1]) <= GENTLE_MOVE_PX
            }
            _ => false,
        }
}

/// A motion that moves, scales, rotates or resizes `id` runs at `local`
/// (an entrance, a reframe, a pulse): its position is judged where it settles.
fn in_flight(scene: &Scene, id: &str, local: f64) -> bool {
    scene.motions.iter().any(|m| {
        m.target == id
            && matches!(
                m.op.channel(),
                Channel::Offset
                    | Channel::Scale
                    | Channel::Rotation
                    | Channel::ContentOffset
                    | Channel::Geometry
            )
            && !is_gentle(m)
            && local >= m.start
            && local < m.start + m.duration
    })
}

fn flatten<'b, 'a>(
    scene: &Scene,
    local: f64,
    project: &MotionProject,
    layers: &'b [ResolvedLayer<'a>],
    out: &mut Vec<Leaf<'b, 'a>>,
) {
    fn walk<'b, 'a>(
        scene: &Scene,
        local: f64,
        layers: &'b [ResolvedLayer<'a>],
        opacity: f32,
        moving: bool,
        mine: bool,
        out: &mut Vec<Leaf<'b, 'a>>,
    ) {
        for l in layers {
            let op = opacity * l.opacity;
            let mv = moving || (!is_stage(l.id) && in_flight(scene, l.id, local));
            if matches!(l.kind, LayerKind::Group { .. }) {
                walk(scene, local, &l.children, op, mv, mine, out);
            } else {
                out.push(Leaf {
                    layer: l,
                    opacity: op,
                    moving: mv,
                    mine,
                });
            }
        }
    }
    for l in layers {
        // A scene layer belongs to its scene; a shared element only to the
        // scenes its track has keys in.
        let mine = match l.scene {
            Some(id) => id == scene.id,
            None => project
                .shared
                .iter()
                .any(|e| e.layer.id == l.id && e.track.iter().any(|k| k.scene == scene.id)),
        };
        walk(scene, local, std::slice::from_ref(l), 1.0, false, mine, out);
    }
}

// ---------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------

/// A subject image resolved to canvas geometry.
struct Subject<'b, 'a> {
    /// Index of the leaf in draw order.
    at: usize,
    leaf: &'b Leaf<'b, 'a>,
    facts: &'b ImageFacts,
    treatment: ImageTreatment,
    bounds: Rect,
    head: Option<Rect>,
}

fn finding(scene: &Scene, layer: &str, check: LayoutCheck, detail: String) -> LayoutFinding {
    LayoutFinding {
        scene: scene.id.clone(),
        layer: layer.to_string(),
        check,
        detail,
    }
}

fn pct(v: f32) -> String {
    format!("{:.1}%", v * 100.0)
}

/// Run the subject checks on one sampled frame of `scene`. Findings are
/// appended unless the same (scene, layer, check) is already reported.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_subjects(
    project: &MotionProject,
    scene: &Scene,
    frame: &LayoutFrame,
    n: u32,
    local: f64,
    layers: &[ResolvedLayer<'_>],
    images: &ImageIndex,
    findings: &mut Vec<LayoutFinding>,
) {
    if images.is_empty() {
        return;
    }
    let mut leaves: Vec<Leaf<'_, '_>> = Vec::new();
    flatten(scene, local, project, layers, &mut leaves);
    let canvas = Rect {
        x: 0.0,
        y: 0.0,
        w: frame.w,
        h: frame.h,
    };
    let canvas_area = area(&canvas);

    // Subjects: delivered image layers of this beat the index knows.
    let mut subjects: Vec<Subject<'_, '_>> = Vec::new();
    for (at, leaf) in leaves.iter().enumerate() {
        let layer = leaf.layer;
        let LayerKind::Image {
            asset,
            fit,
            treatment,
            ..
        } = layer.kind
        else {
            continue;
        };
        if !leaf.mine
            || leaf.moving
            || leaf.opacity < SHOWN_IMAGE
            || is_decorative(layer.id, layer.kind)
        {
            continue;
        }
        let Some(path) = project
            .assets
            .iter()
            .find(|a| a.id == *asset)
            .map(|a| a.path.as_str())
        else {
            continue;
        };
        let Some(facts) = images.get(path) else {
            continue;
        };
        let (dx, dy, dw, dh) = drawn_local(layer, *fit, facts);
        let drawn = aabb(&layer.transform, dx, dy, dw, dh);
        // Background plates (full-bleed environments) are not subjects.
        if area(&drawn) > PLATE_SHARE * canvas_area {
            continue;
        }
        let local_box = |b: NormBox| {
            aabb(
                &layer.transform,
                dx + b.x * dw,
                dy + b.y * dh,
                b.width * dw,
                b.height * dh,
            )
        };
        subjects.push(Subject {
            at,
            leaf,
            facts,
            treatment: treatment.clone().unwrap_or_else(ImageTreatment::natural),
            bounds: local_box(facts.subject),
            head: facts.head.map(local_box),
        });
    }

    let mut push = |f: LayoutFinding| {
        if !findings
            .iter()
            .any(|g| g.scene == f.scene && g.layer == f.layer && g.check == f.check)
        {
            findings.push(f);
        }
    };

    for s in &subjects {
        let id = s.leaf.layer.id;

        // subject_too_small
        if is_hero_subject(id) {
            let visible = overlap(&s.bounds, &canvas) / canvas_area.max(1.0);
            let aspect = (s.bounds.h > 0.0).then(|| s.bounds.w / s.bounds.h);
            let min = subject_min_area_for(frame, aspect);
            let tall_enough = s.bounds.h / frame.h.max(1.0)
                >= crate::compiler::taste_rules::SUBJECT_MIN_HEIGHT - 1e-4;
            if visible + 1e-4 < min && !tall_enough {
                push(finding(
                    scene,
                    id,
                    LayoutCheck::SubjectTooSmall,
                    format!(
                        "frame {n}: the subject covers {} of the canvas (min {})",
                        pct(visible),
                        pct(min)
                    ),
                ));
            }
        }

        // text_over_subject
        let mut covered = 0.0f32;
        for (i, t) in leaves.iter().enumerate() {
            let LayerKind::Text(style) = t.layer.kind else {
                continue;
            };
            let shown = t.layer.text.as_deref().unwrap_or(&style.text);
            if !t.mine
                || t.moving
                || t.opacity < SHOWN_TEXT
                || shown.trim().is_empty()
                || is_decorative(t.layer.id, t.layer.kind)
            {
                continue;
            }
            let (top, bottom) = match style.ink {
                Some(ink) => (ink.top, ink.bottom),
                None => (0.0, t.layer.height),
            };
            let tb = aabb(
                &t.layer.transform,
                0.0,
                top,
                t.layer.width,
                (bottom - top).max(0.0),
            );
            // Heads are never covered, whether the text is above or behind.
            if let Some(h) = &s.head {
                if overlap(&tb, h) > HEAD_TOLERANCE * area(h) {
                    push(finding(
                        scene,
                        t.layer.id,
                        LayoutCheck::TextOverSubject,
                        format!("frame {n}: text over the head of {id}"),
                    ));
                }
            }
            if i < s.at {
                continue;
            }
            // Text on its own opaque card or strip is a label, not type
            // printed over the picture.
            let backed = leaves[s.at + 1..i].iter().any(|c| {
                matches!(
                    c.layer.kind,
                    LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. }
                        if fill.a >= 0xE0
                ) && c.opacity >= OPAQUE_CARD
                    && overlap(
                        &aabb(&c.layer.transform, 0.0, 0.0, c.layer.width, c.layer.height),
                        &tb,
                    ) >= BACKING_CONTAINS * area(&tb)
            });
            if !backed {
                covered += overlap(&tb, &s.bounds);
            }
        }
        let share = covered / area(&s.bounds).max(1.0);
        if share > TEXT_OVER_SUBJECT_MAX {
            push(finding(
                scene,
                id,
                LayoutCheck::TextOverSubject,
                format!(
                    "frame {n}: text covers {} of the subject (max {})",
                    pct(share),
                    pct(TEXT_OVER_SUBJECT_MAX)
                ),
            ));
        }

        // frame_too_loose
        let sw = s.bounds.w.max(1.0);
        let sh = s.bounds.h.max(1.0);
        for c in &leaves[..s.at] {
            let (LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. }) =
                c.layer.kind
            else {
                continue;
            };
            if !c.mine
                || c.opacity < 0.3
                || fill.a < 0x40
                || is_decorative(c.layer.id, c.layer.kind)
            {
                continue;
            }
            let card = aabb(&c.layer.transform, 0.0, 0.0, c.layer.width, c.layer.height);
            if area(&card) > PLATE_SHARE * canvas_area
                || overlap(&card, &s.bounds) < FRAME_HOLDS * area(&s.bounds)
            {
                continue;
            }
            let pad = [
                (s.bounds.x - card.x) / sw,
                ((card.x + card.w) - (s.bounds.x + s.bounds.w)) / sw,
                (s.bounds.y - card.y) / sh,
                ((card.y + card.h) - (s.bounds.y + s.bounds.h)) / sh,
            ]
            .into_iter()
            .fold(f32::MIN, f32::max);
            if pad > FRAME_PAD_MAX + FRAME_SLACK {
                push(finding(
                    scene,
                    c.layer.id,
                    LayoutCheck::FrameTooLoose,
                    format!(
                        "frame {n}: the card exceeds the subject {id} by {} on a side (max {})",
                        pct(pad),
                        pct(FRAME_PAD_MAX)
                    ),
                ));
            }
        }

        // asset_low_contrast
        if let Some(mean) = s.facts.mean_color {
            let u = frame.u;
            let treated = treated_color(mean, &s.treatment);
            let low = match s.treatment.sticker {
                // A sticker works when it contrasts with the subject.
                Some(st) => contrast_ratio(treated, st.color) < ASSET_GROUND_MIN_CONTRAST,
                None => {
                    let ground = ground_under(project, &leaves[..s.at], &s.bounds);
                    let near = neighbours(&s.treatment, u, s.facts.alpha, &[ground]);
                    needs_contrast_treatment(treated, &near)
                        && !keyline_separates(&s.treatment, treated, u)
                }
            };
            if low {
                push(finding(
                    scene,
                    id,
                    LayoutCheck::AssetLowContrast,
                    format!(
                        "frame {n}: mean colour {} has under {ASSET_GROUND_MIN_CONTRAST}:1 against its ground and no sticker or keyline",
                        treated.to_hex()
                    ),
                ));
            }
        }
    }
}

/// An opaque image's keyline (a 2u+ edge) separates it when its colour
/// contrasts with the image.
fn keyline_separates(t: &ImageTreatment, subject: Color, u: f32) -> bool {
    t.edge.is_some_and(|e| {
        e.width >= EDGE_SEPARATES_U * u
            && contrast_ratio(subject, e.color) >= ASSET_GROUND_MIN_CONTRAST
    })
}

/// The colour under a subject: the topmost opaque card or colour field behind
/// it that holds its centre, else the canvas background.
fn ground_under(project: &MotionProject, behind: &[Leaf<'_, '_>], bounds: &Rect) -> Color {
    let (cx, cy) = (bounds.x + bounds.w / 2.0, bounds.y + bounds.h / 2.0);
    behind
        .iter()
        .rev()
        .find_map(|c| {
            let (fill, radius) = match c.layer.kind {
                LayerKind::Rectangle { fill, .. } => (*fill, 0.0),
                LayerKind::RoundedRectangle { fill, radius, .. } => (*fill, *radius),
                _ => return None,
            };
            if fill.a < 0xE0 || c.opacity < OPAQUE_CARD {
                return None;
            }
            let r = aabb(&c.layer.transform, 0.0, 0.0, c.layer.width, c.layer.height);
            rounded_contains(&r, radius * c.layer.transform.a.abs(), cx, cy).then_some(fill)
        })
        .unwrap_or(project.canvas.background)
}
