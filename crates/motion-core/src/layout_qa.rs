//! (0.9) Layout QA: does a compiled project actually fit its canvas?
//!
//! Pure and deterministic: [`layout_report`] resolves each beat scene at its
//! READ time (`scene.start + lifecycle.read`) and at the READ/EVOLVE midpoint
//! through [`evaluate_frame`] (so camera push, stage scale and every motion are
//! included) and checks the resolved layer boxes against the [`LayoutFrame`]:
//!
//! | check | rule |
//! |---|---|
//! | `text_outside_safe` | text box inside the safe area (+-2u tolerance), judged in stage space |
//! | `text_clipped` | text box not cut by the canvas edge, judged in stage space |
//! | `text_too_small` | font size (font_size x stage-space scale) >= `frame.min_type_px` (1px rounding slack) |
//! | `subject_image_outside` | an image layer has >= 60% of its box inside the canvas |
//! | `layer_off_canvas` | no other non-decorative layer lies entirely off the canvas |
//! | `headline_too_big` | (0.10 Q) the display headline block (`<beat>.head*`, display face) at READ is at most `HEADLINE_MAX_H_*` of the canvas height (0.30 with a subject image/object on screen, 0.46 type-only) and at most `HEADLINE_MAX_LINES_*` lines (4 tall / 3 square and wide) |
//!
//! **Stage space.** Beat content lives in the `<beat>.stage` group, which
//! carries the camera push, EVOLVE hierarchy reframes and the anticipation
//! shrink (all about the canvas centre). Safe-area and type-size checks undo the
//! stage's transform, so they judge the authored layout rather than a moving
//! camera; image coverage and off-canvas layers use the real canvas position. Layers
//! outside a stage (shared elements) are measured with a push of up to 6%
//! removed.
//!
//! **In flight.** A layer with a running move / scale / rotate / reveal /
//! geometry motion at the sample (an entrance, a reframe), or inside such a
//! group, is skipped for the position checks: its position is judged at the
//! samples where it has settled. For that, a third sample is taken at the end
//! of EVOLVE (`anticipate - 0.05s`) next to READ and the READ/EVOLVE midpoint.
//!
//! Invisible layers (effective opacity under 5%, including parent opacity) are
//! ignored: they cannot be seen violating anything.
//!
//! # Decorative exemptions
//! Decorative layers are exempt from every check (furniture labels are
//! authored below the body-text minimum on purpose). They are recognised by the compiler's
//! existing layer-id conventions, never by content:
//! * the whole-piece background scene (`backdrop`) and any id starting with
//!   `backdrop.` (paper, grain, grid, pulse lines, fields: `backdrop.field*`);
//! * every `Texture` layer (grain / paper);
//! * `<beat>.ghost` (the oversized ghost word, bleeds off the canvas by design);
//! * `<beat>.wipe*` (panel-wipe transition panels that travel across the canvas);
//! * density furniture (`furniture.rs`), first segment after the beat prefix:
//!   `folio`, `folio_rule`, `folio_tick`, `index`, `data_rule`, `ticks`,
//!   `reg_h`, `reg_v`, `numeral`, `sticker`, `sticker2` (labels, rules and tick
//!   marks that live in the canvas-edge lanes);
//! * `<beat>.note.tab`, the accent bar on the edge of the evidence note card.
//!
//! # Captions (0.10)
//! The word-synced caption scene (`captions`, no lifecycle) is **not** exempt:
//! it is sampled where each caption page is fully shown and settled
//! ([`caption_qa::sample_times`]) and its layers (`cap.*`) go through the same
//! checks. [`caption_qa::caption_report`] adds the speech-sync checks.
//!
//! Each (scene, layer, check) is reported once, at its first failing sample.

use serde::{Deserialize, Serialize};

use crate::audio::beat_scenes;
use crate::caption_qa;
use crate::compiler::captions::{CAPTION_LAYER_PREFIX, CAPTION_SCENE_ID};
use crate::compiler::layout_frame::{LayoutFrame, Rect};
use crate::compiler::taste_rules::{
    HEADLINE_MAX_H_TYPE_ONLY, HEADLINE_MAX_H_WITH_SUBJECT, HEADLINE_MAX_LINES_TALL,
    HEADLINE_MAX_LINES_WIDE,
};
use crate::intent::Format;
use crate::scene::{Channel, FontRole, LayerKind, MotionProject, Scene};
use crate::subject_qa::{self, ImageIndex};
use crate::timeline::{evaluate_frame, Affine, ResolvedLayer};

/// Tolerance on the safe-area test, in units of `frame.u`.
const SAFE_TOLERANCE_U: f32 = 2.0;
/// Largest stage / camera push compensated for in the safe-area test.
const MAX_PUSH: f32 = 1.06;
/// Minimum share of an image layer's box that must lie inside the canvas.
const MIN_IMAGE_INSIDE: f32 = 0.6;
/// Layers whose effective opacity is below this are invisible.
const MIN_VISIBLE_OPACITY: f32 = 0.05;
/// Slack (px) on the minimum type size (font_size is rounded to 0.01).
const TYPE_SLACK_PX: f32 = 1.0;

/// Furniture / decoration names (first id segment after the beat prefix).
const DECORATIVE_NAMES: &[&str] = &[
    "ghost",
    // (0.14–0.17) atmosphere: background type, dust, glows, bokeh, web lines
    "ghost_word",
    "ghost_punch",
    // (0.19) the paper-coloured gradient under a layers beat's headline
    "scrim",
    "dust",
    "glow",
    "bokeh",
    "web",
    "wipe",
    "wipe_a",
    "wipe_b",
    "folio",
    "folio_rule",
    "folio_tick",
    "index",
    "data_rule",
    "ticks",
    "reg_h",
    "reg_v",
    "numeral",
    "sticker",
    "sticker2",
];

/// Decorative layers matched on the whole id below the beat prefix.
/// `note.tab`: the 10u accent bar on the left edge of the evidence note card.
/// It is glued to the card edge, so when a stage reframe zooms the beat in the
/// bar can leave the canvas a few pixels before the card does; the card and
/// the note content are still checked.
const DECORATIVE_PATHS: &[&str] = &["note.tab"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutVerdict {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutCheck {
    TextOutsideSafe,
    TextClipped,
    TextTooSmall,
    SubjectImageOutside,
    LayerOffCanvas,
    /// The timeline could not resolve the sampled frame.
    Evaluation,
    /// (0.10 Q) Display headline taller than its budget or over its line cap
    /// (`taste_rules::HEADLINE_MAX_*`).
    HeadlineTooBig,
    /// (0.10 Q) Text covers more than `TEXT_OVER_SUBJECT_MAX` of a subject's
    /// alpha bounds (or any of its head region).
    TextOverSubject,
    /// (0.10 Q) The beat's hero subject image is smaller than `SUBJECT_MIN_AREA`.
    SubjectTooSmall,
    /// (0.10 Q) A card/frame/plate behind a subject exceeds it by more than
    /// `FRAME_PAD_MAX` per side.
    FrameTooLoose,
    /// (0.10 Q) A subject's mean colour is below `ASSET_GROUND_MIN_CONTRAST`
    /// against the ground under it and has no contrast treatment.
    AssetLowContrast,
    /// (0.18) The layer the beat is about (`ArtRecord.focal`) is blurred by
    /// more than `FOCAL_BLUR_MAX` px while it is read.
    FocalOutOfFocus,
    /// (0.18) The beat's stamp does not touch the focal layer: emphasis must
    /// land on what the beat is about.
    EmphasisOffFocal,
    /// (0.23) Rendered: text readable at READ does not contrast with the
    /// pixels around its ink box (`checks::TEXT_LOCAL_CONTRAST`). Emitted by
    /// the renderer's pixel pass, never by the structural report.
    TextLocalContrast,
    /// (0.23) Two readable non-decorative text layers overlap
    /// (`checks::TEXT_OVERLAP`), outside the impact allowance.
    TextOverlap,
}

impl LayoutCheck {
    pub fn name(&self) -> &'static str {
        match self {
            LayoutCheck::TextOutsideSafe => "text_outside_safe",
            LayoutCheck::TextClipped => "text_clipped",
            LayoutCheck::TextTooSmall => "text_too_small",
            LayoutCheck::SubjectImageOutside => "subject_image_outside",
            LayoutCheck::LayerOffCanvas => "layer_off_canvas",
            LayoutCheck::Evaluation => "evaluation",
            LayoutCheck::HeadlineTooBig => "headline_too_big",
            LayoutCheck::TextOverSubject => "text_over_subject",
            LayoutCheck::SubjectTooSmall => "subject_too_small",
            LayoutCheck::FrameTooLoose => "frame_too_loose",
            LayoutCheck::AssetLowContrast => "asset_low_contrast",
            LayoutCheck::FocalOutOfFocus => "focal_out_of_focus",
            LayoutCheck::EmphasisOffFocal => "emphasis_off_focal",
            LayoutCheck::TextLocalContrast => crate::checks::TEXT_LOCAL_CONTRAST,
            LayoutCheck::TextOverlap => crate::checks::TEXT_OVERLAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutFinding {
    pub scene: String,
    pub layer: String,
    pub check: LayoutCheck,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutReport {
    pub verdict: LayoutVerdict,
    pub findings: Vec<LayoutFinding>,
}

impl LayoutReport {
    pub fn passed(&self) -> bool {
        self.verdict == LayoutVerdict::Pass
    }

    /// Human-readable text (one line per finding).
    pub fn to_text(&self) -> String {
        let mut out = String::from("layout\n");
        for f in &self.findings {
            out.push_str(&format!(
                "  FAIL {} {} {}: {}\n",
                f.scene,
                f.layer,
                f.check.name(),
                f.detail
            ));
        }
        out.push_str(&format!(
            "layout: {} ({} finding(s))\n",
            match self.verdict {
                LayoutVerdict::Pass => "PASS",
                LayoutVerdict::Fail => "FAIL",
            },
            self.findings.len()
        ));
        out
    }
}

/// Evaluate `project` against `frame` (see the module docs).
/// The project in layout space: scene cameras and whole-stage reframe zooms
/// (scale moves on `*.stage` / `*.stage_front` groups) removed.
fn without_camera(project: &MotionProject) -> MotionProject {
    let mut p = project.clone();
    for s in &mut p.scenes {
        s.camera = None;
        s.motions.retain(|m| {
            !(is_stage_id(&m.target) && matches!(m.op, crate::scene::MotionOp::Scale { .. }))
        });
    }
    p
}

fn is_stage_id(id: &str) -> bool {
    id.ends_with(".stage") || id.ends_with(".stage_front")
}

pub fn layout_report(project: &MotionProject, frame: &LayoutFrame) -> LayoutReport {
    layout_report_with(project, frame, &ImageIndex::default())
}

/// (0.10 Q) [`layout_report`] plus the subject checks (`text_over_subject`,
/// `subject_too_small`, `frame_too_loose`, `asset_low_contrast`, see
/// [`crate::subject_qa`]), which need what the analysis measured about each
/// delivered image. With an empty index it is exactly [`layout_report`].
pub fn layout_report_with(
    project: &MotionProject,
    frame: &LayoutFrame,
    images: &ImageIndex,
) -> LayoutReport {
    let mut findings: Vec<LayoutFinding> = Vec::new();
    let last = project.frame_count().saturating_sub(1);
    let fps = project.canvas.fps as f64;
    let mut still: Option<MotionProject> = None;
    for scene in beat_scenes(project) {
        if scene.id == CAPTION_SCENE_ID {
            continue;
        }
        let (read, mid) = match scene.lifecycle {
            Some(l) => (l.read, (l.read + l.evolve) / 2.0),
            None => (scene.duration_seconds * 0.5, scene.duration_seconds * 0.5),
        };
        // READ, the READ/EVOLVE midpoint, and the end of EVOLVE (just before
        // ANTICIPATE), when EVOLVE arrivals have settled.
        let settled = scene
            .lifecycle
            .map(|l| (l.anticipate - 0.05).max(l.read))
            .unwrap_or(read);
        let mut times = vec![read];
        for t in [mid, settled] {
            if times.iter().all(|x| (x - t).abs() > 1e-9) {
                times.push(t);
            }
        }
        for local in times {
            let n = (((scene.start_seconds + local) * fps).round() as u32).min(last);
            match evaluate_frame(project, n) {
                Ok(resolved) => {
                    let sample = Sample {
                        scene,
                        frame,
                        n,
                        local,
                    };
                    // Caption layers are judged in their own pass below.
                    let mut found = Vec::new();
                    for layer in resolved.layers.iter().filter(|l| !is_caption_layer(l.id)) {
                        check_layer(&sample, layer, 1.0, None, false, &mut found);
                    }
                    // (0.10) A camera push/track deliberately carries margin
                    // content outward; the safe area judges the layout, so a
                    // text_outside_safe finding stands only when the same text
                    // is outside without the camera too. Clipping by the canvas
                    // (text_clipped) is still judged with the camera.
                    if found
                        .iter()
                        .any(|f| f.check == LayoutCheck::TextOutsideSafe)
                    {
                        let still = still.get_or_insert_with(|| without_camera(project));
                        let mut without = Vec::new();
                        if let Ok(r) = evaluate_frame(still, n) {
                            for layer in r.layers.iter().filter(|l| !is_caption_layer(l.id)) {
                                check_layer(&sample, layer, 1.0, None, false, &mut without);
                            }
                        }
                        found.retain(|f| {
                            f.check != LayoutCheck::TextOutsideSafe
                                || without.iter().any(|w| {
                                    w.check == LayoutCheck::TextOutsideSafe && w.layer == f.layer
                                })
                        });
                    }
                    findings.extend(found);
                    subject_qa::check_subjects(
                        project,
                        scene,
                        frame,
                        n,
                        local,
                        &resolved.layers,
                        images,
                        &mut findings,
                    );
                    focal_findings(project, scene, n, &resolved.layers, &mut findings);
                }
                Err(e) => push(
                    &mut findings,
                    scene,
                    &scene.id,
                    LayoutCheck::Evaluation,
                    format!("frame {n}: {e}"),
                ),
            }
        }
    }
    // (0.10 Q) Each beat's display headline obeys its budget at READ.
    headline_findings(project, frame, &mut findings);
    // (0.10) The caption scene has no lifecycle: sample it where each page is
    // fully shown and settled. Captions are not exempt.
    for scene in project
        .scenes
        .iter()
        .filter(|s| s.id == CAPTION_SCENE_ID && s.lifecycle.is_none())
    {
        for t in caption_qa::sample_times(scene) {
            let n = ((t * fps).round() as u32).min(last);
            match evaluate_frame(project, n) {
                Ok(resolved) => {
                    let sample = Sample {
                        scene,
                        frame,
                        n,
                        local: t - scene.start_seconds,
                    };
                    for layer in resolved.layers.iter().filter(|l| is_caption_layer(l.id)) {
                        check_layer(&sample, layer, 1.0, None, false, &mut findings);
                    }
                }
                Err(e) => push(
                    &mut findings,
                    scene,
                    &scene.id,
                    LayoutCheck::Evaluation,
                    format!("frame {n}: {e}"),
                ),
            }
        }
    }
    LayoutReport {
        verdict: if findings.is_empty() {
            LayoutVerdict::Pass
        } else {
            LayoutVerdict::Fail
        },
        findings,
    }
}

fn push(
    findings: &mut Vec<LayoutFinding>,
    scene: &Scene,
    layer: &str,
    check: LayoutCheck,
    detail: String,
) {
    if findings
        .iter()
        .any(|f| f.scene == scene.id && f.layer == layer && f.check == check)
    {
        return;
    }
    findings.push(LayoutFinding {
        scene: scene.id.clone(),
        layer: layer.to_string(),
        check,
        detail,
    });
}

/// (0.18) Depth-of-field blur (px) above which the focal layer is out of focus.
pub const FOCAL_BLUR_MAX: f32 = 1.5;

/// (0.18) The beat's focal layer is sharp, and its stamp (`<prefix>.stamp`)
/// touches it. Beats without a recorded focal layer are not judged.
fn focal_findings(
    project: &MotionProject,
    scene: &Scene,
    n: u32,
    layers: &[ResolvedLayer<'_>],
    findings: &mut Vec<LayoutFinding>,
) {
    let Some(focal) = project
        .project
        .art
        .as_ref()
        .and_then(|a| a.focal.get(&scene.id))
    else {
        return;
    };
    fn find<'a, 'b>(l: &'b ResolvedLayer<'a>, id: &str) -> Option<&'b ResolvedLayer<'a>> {
        if l.id == id {
            return Some(l);
        }
        l.children.iter().find_map(|c| find(c, id))
    }
    let Some((top, node)) = layers
        .iter()
        .find_map(|t| find(t, focal).map(|node| (t, node)))
    else {
        // Not drawn while it is read (hidden, culled or a dangling id).
        push(
            findings,
            scene,
            focal,
            LayoutCheck::FocalOutOfFocus,
            format!("frame {n}: not drawn"),
        );
        return;
    };
    if let Some(b) = top.blur.filter(|b| *b > FOCAL_BLUR_MAX) {
        push(
            findings,
            scene,
            focal,
            LayoutCheck::FocalOutOfFocus,
            format!("frame {n}: blurred {b:.1} px (max {FOCAL_BLUR_MAX})"),
        );
    }
    let Some(prefix) = scene_prefix(scene) else {
        return;
    };
    let stamp_id = format!("{prefix}.stamp");
    let Some(stamp) = layers.iter().find_map(|t| find(t, &stamp_id)) else {
        return;
    };
    if stamp.opacity < 0.5 {
        return;
    }
    let s = bounds(stamp.width, &stamp.transform, 0.0, stamp.height);
    let f = bounds(node.width, &node.transform, 0.0, node.height);
    let touches = s.x < f.x + f.w && f.x < s.x + s.w && s.y < f.y + f.h && f.y < s.y + s.h;
    if !touches {
        push(
            findings,
            scene,
            &stamp_id,
            LayoutCheck::EmphasisOffFocal,
            format!(
                "frame {n}: stamp {} misses focal {}",
                describe(&s),
                describe(&f)
            ),
        );
    }
}

/// A layer of the (0.10) caption scene (pages `cap.p*`, words `cap.w*`).
fn is_caption_layer(id: &str) -> bool {
    id.starts_with(CAPTION_LAYER_PREFIX)
}

// ---------------------------------------------------------------------------
// (0.10 Q) Headline budget
// ---------------------------------------------------------------------------

/// Pixels of slack on the block height (rounded font sizes, per-line boxes).
const HEADLINE_SLACK_PX: f32 = 1.5;
/// An image / svg covering at least this share of the canvas is a background
/// plate, not a subject.
const SUBJECT_MAX_CANVAS_SHARE: f32 = 0.6;

/// One beat's display headline block as it is laid out (the READ layout:
/// nothing moves at READ; entrance offsets, the stage camera and reframes are
/// not part of it), next to its budget (`taste_rules::HEADLINE_MAX_*`).
#[derive(Debug, Clone, PartialEq)]
pub struct HeadlineBlock {
    pub scene: String,
    /// `<beat prefix>.head`
    pub layer: String,
    /// Distinct text lines.
    pub lines: usize,
    /// Top of the first line box to the bottom of the last (px).
    pub height: f32,
    /// A subject image / object / figure shares the beat: the tighter budget.
    pub with_subject: bool,
    pub max_lines: usize,
    pub max_height: f32,
}

impl HeadlineBlock {
    /// Over the height budget or the line cap.
    pub fn too_big(&self) -> bool {
        self.lines > self.max_lines || self.height > self.max_height + HEADLINE_SLACK_PX
    }
}

/// The display headline of every beat scene (`<beat>.head*` text layers in the
/// display face; ghost and background words are not headlines). Beats without
/// one are absent.
pub fn headline_blocks(project: &MotionProject, frame: &LayoutFrame) -> Vec<HeadlineBlock> {
    beat_scenes(project)
        .into_iter()
        .filter_map(|scene| measure_headline(project, frame, scene))
        .collect()
}

fn headline_findings(
    project: &MotionProject,
    frame: &LayoutFrame,
    findings: &mut Vec<LayoutFinding>,
) {
    for scene in beat_scenes(project) {
        let Some(block) = measure_headline(project, frame, scene) else {
            continue;
        };
        if block.too_big() {
            push(
                findings,
                scene,
                &block.layer,
                LayoutCheck::HeadlineTooBig,
                format!(
                    "{} line(s), {:.0}px tall; budget {} line(s), {:.0}px ({})",
                    block.lines,
                    block.height,
                    block.max_lines,
                    block.max_height,
                    if block.with_subject {
                        "with a subject"
                    } else {
                        "type only"
                    }
                ),
            );
        }
    }
}

/// The headline of one beat scene, from its authored layers (stage children are
/// in canvas coordinates): the text layers named `<prefix>.head[.…]` in the
/// display face. The subject test looks at the scene's image / svg layers and at
/// the shared elements that have a key in it.
fn measure_headline(
    project: &MotionProject,
    frame: &LayoutFrame,
    scene: &Scene,
) -> Option<HeadlineBlock> {
    if scene.id == CAPTION_SCENE_ID {
        return None;
    }
    let prefix = scene_prefix(scene)?;
    let mut boxes: Vec<(f32, f32)> = Vec::new(); // (top, height)
    let mut subject = false;
    scan_layers(&scene.layers, &prefix, frame, &mut boxes, &mut subject);
    for shared in &project.shared {
        if shared.track.iter().any(|k| k.scene == scene.id) {
            subject |= is_subject_layer(&shared.layer, frame);
        }
    }
    let (first_top, first_h) = *boxes.first()?;
    let top = boxes.iter().map(|b| b.0).fold(first_top, f32::min);
    let bottom = boxes
        .iter()
        .map(|b| b.0 + b.1)
        .fold(first_top + first_h, f32::max);
    // Lines: distinct box tops (half-pixel resolution).
    let mut tops: Vec<i32> = boxes.iter().map(|b| (b.0 * 2.0).round() as i32).collect();
    tops.sort_unstable();
    tops.dedup();
    let share = if subject {
        HEADLINE_MAX_H_WITH_SUBJECT
    } else {
        HEADLINE_MAX_H_TYPE_ONLY
    };
    Some(HeadlineBlock {
        scene: scene.id.clone(),
        layer: format!("{prefix}.head"),
        lines: tops.len(),
        height: bottom - top,
        with_subject: subject,
        max_lines: if frame.class == Format::Vertical {
            HEADLINE_MAX_LINES_TALL
        } else {
            HEADLINE_MAX_LINES_WIDE
        },
        max_height: share * frame.h,
    })
}

/// Collect the headline boxes and the subject flag of an authored layer tree.
fn scan_layers(
    layers: &[crate::scene::Layer],
    prefix: &str,
    frame: &LayoutFrame,
    boxes: &mut Vec<(f32, f32)>,
    subject: &mut bool,
) {
    for l in layers.iter().filter(|l| l.visible) {
        match &l.kind {
            LayerKind::Group { children } => scan_layers(children, prefix, frame, boxes, subject),
            LayerKind::Text(style) => {
                if style.font_role == FontRole::Display
                    && is_headline_id(prefix, &l.id)
                    && !style.text.trim().is_empty()
                {
                    let h = l.height * l.scale_y.abs();
                    boxes.push((l.y - l.anchor_y * h, h));
                }
            }
            _ => *subject |= is_subject_layer(l, frame),
        }
    }
}

/// An image / svg that is a subject (not a full-bleed background plate).
fn is_subject_layer(l: &crate::scene::Layer, frame: &LayoutFrame) -> bool {
    matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. })
        && !is_decorative(&l.id, &l.kind)
        && l.width * l.scale_x.abs() * l.height * l.scale_y.abs()
            < SUBJECT_MAX_CANVAS_SHARE * frame.w * frame.h
}

/// Layer-id prefix of a compiler beat scene (`beat_3` -> `b3`).
fn scene_prefix(scene: &Scene) -> Option<String> {
    let n = scene.id.strip_prefix("beat_")?;
    (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then(|| format!("b{n}"))
}

/// Whether `id` names a layer of this beat's display headline: `<prefix>.head`
/// or `<prefix>.head.<line>[.<word>]`.
fn is_headline_id(prefix: &str, id: &str) -> bool {
    id.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix(".head"))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// Whether a layer is decorative (see the module docs).
pub fn is_decorative(layer_id: &str, kind: &LayerKind) -> bool {
    if matches!(kind, LayerKind::Texture(_)) || layer_id.starts_with("backdrop.") {
        return true;
    }
    let mut segs = layer_id.split('.');
    let first = segs.next().unwrap_or("");
    // Beat layers are "<prefix>.<name>[.<child>]"; shared / stray ids have no prefix.
    let prefixed = first.starts_with('b') && first[1..].chars().all(|c| c.is_ascii_digit());
    let path = if prefixed {
        layer_id.split_once('.').map_or("", |(_, rest)| rest)
    } else {
        layer_id
    };
    let name = if prefixed {
        segs.next().unwrap_or("")
    } else {
        first
    };
    DECORATIVE_NAMES.contains(&name) || DECORATIVE_PATHS.contains(&path)
}

/// The stage group wrapping a beat's content (`wrap_stage`): it carries the
/// camera push, hierarchy reframes and the anticipation shrink.
fn is_stage(id: &str) -> bool {
    id.ends_with(".stage") || id.ends_with(".stage_front")
}

/// Inverse of an affine transform (`None` when singular).
fn invert(t: &Affine) -> Option<Affine> {
    let det = t.a * t.d - t.b * t.c;
    if det.abs() < 1e-9 {
        return None;
    }
    let (a, b, c, d) = (t.d / det, -t.b / det, -t.c / det, t.a / det);
    Some(Affine {
        a,
        b,
        c,
        d,
        e: -(a * t.e + c * t.f),
        f: -(b * t.e + d * t.f),
    })
}

/// Uniform scale factor of a transform.
fn scale_of(t: &Affine) -> f32 {
    (t.a * t.d - t.b * t.c).abs().sqrt()
}

/// Axis-aligned bounds of the box rows `top..bottom` mapped through `xf`.
fn bounds(width: f32, xf: &Affine, top: f32, bottom: f32) -> Rect {
    let corners = [(0.0, top), (width, top), (0.0, bottom), (width, bottom)];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        let (cx, cy) = xf.apply(x, y);
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

/// Box padding the compiler adds around measured text so glyph overhangs never
/// clip: `typeset::text_layer` uses `measured * 1.02 + 2px`, counter slots
/// `widest * 1.04 + 4px`. The text may legitimately sit this far inside the
/// box edges (a centred box with left-aligned glyphs puts the ink up to the
/// padding off-centre), so the safe-area test grants it as extra tolerance.
fn box_padding(box_w: f32, scale: f32, counted: bool) -> f32 {
    let (k, px) = if counted { (0.04, 4.0) } else { (0.02, 2.0) };
    (k * box_w + px * scale) / (1.0 + k)
}

/// Layers outside any stage group (shared elements, hand-authored scenes)
/// still carry the camera push: measure them as if a push of up to
/// [`MAX_PUSH`] about the canvas centre had not happened.
fn relax_push(r: Rect, scale: f32, frame: &LayoutFrame) -> Rect {
    let s = scale.clamp(1.0, MAX_PUSH);
    if s <= 1.0 {
        return r;
    }
    let (cx, cy) = (frame.w / 2.0, frame.h / 2.0);
    Rect {
        x: cx + (r.x - cx) / s,
        y: cy + (r.y - cy) / s,
        w: r.w / s,
        h: r.h / s,
    }
}

fn inside_fraction(r: &Rect, canvas: &Rect) -> f32 {
    let area = r.w * r.h;
    if area <= 0.0 {
        return 1.0;
    }
    let ix = (r.x + r.w).min(canvas.x + canvas.w) - r.x.max(canvas.x);
    let iy = (r.y + r.h).min(canvas.y + canvas.h) - r.y.max(canvas.y);
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    ix * iy / area
}

fn describe(r: &Rect) -> String {
    format!(
        "box x {:.0}..{:.0} y {:.0}..{:.0}",
        r.x,
        r.x + r.w,
        r.y,
        r.y + r.h
    )
}

/// One sampled frame of one scene.
struct Sample<'a> {
    scene: &'a Scene,
    frame: &'a LayoutFrame,
    n: u32,
    /// Scene-local time of the sample.
    local: f64,
}

/// Whether a motion that moves, scales, rotates or resizes `id` is running at
/// `local`: such a layer is in flight (an entrance or a reframe), so its
/// position is judged at the samples where it has settled.
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
            && local >= m.start
            && local < m.start + m.duration
    })
}

/// `stage_inv`: inverse of the enclosing stage group's transform. Layout
/// (safe area, type size) is judged in stage space, i.e. without the camera
/// push, hierarchy reframes and anticipation shrink; clipping and image
/// coverage are judged on the real canvas.
fn check_layer(
    s: &Sample,
    layer: &ResolvedLayer,
    parent_opacity: f32,
    stage_inv: Option<Affine>,
    moving: bool,
    findings: &mut Vec<LayoutFinding>,
) {
    let opacity = parent_opacity * layer.opacity;
    let moving = moving || (!is_stage(layer.id) && in_flight(s.scene, layer.id, s.local));
    let child_inv = if is_stage(layer.id) {
        invert(&layer.transform)
    } else {
        stage_inv
    };
    for child in &layer.children {
        check_layer(s, child, opacity, child_inv, moving, findings);
    }
    if opacity < MIN_VISIBLE_OPACITY || layer.width <= 0.0 || layer.height <= 0.0 {
        return;
    }
    let (scene, frame, n) = (s.scene, s.frame, s.n);
    let decorative = scene.id == "backdrop" || is_decorative(layer.id, layer.kind);
    let canvas = Rect {
        x: 0.0,
        y: 0.0,
        w: frame.w,
        h: frame.h,
    };
    match layer.kind {
        LayerKind::Text(style) => {
            let shown = layer.text.as_deref().unwrap_or(&style.text);
            if decorative || shown.trim().is_empty() {
                return;
            }
            // Stage-space transform (the real one when there is no stage).
            let rest = match stage_inv {
                Some(inv) => inv.then_apply(layer.transform),
                None => layer.transform,
            };
            let scale = scale_of(&rest);
            let size = style.font_size * scale;
            if size + TYPE_SLACK_PX < frame.min_type_px {
                push(
                    findings,
                    scene,
                    layer.id,
                    LayoutCheck::TextTooSmall,
                    format!("frame {n}: {size:.1}px < min {:.1}px", frame.min_type_px),
                );
            }
            if moving {
                return;
            }
            let (top, bottom) = match style.ink {
                Some(ink) => (ink.top, ink.bottom),
                None => (0.0, layer.height),
            };
            let counted = layer.text.is_some();
            let mut r = bounds(layer.width, &rest, top, bottom);
            if stage_inv.is_none() {
                r = relax_push(r, scale, frame);
            }
            // Text the stage camera pushes past the edge (EVOLVE reframes) is
            // intentional; clipping is judged on the authored (stage-space) box.
            let pad = box_padding(r.w, scale, counted) + 0.5;
            let clipped =
                r.x < -pad || r.y < -0.5 || r.x + r.w > frame.w + pad || r.y + r.h > frame.h + 0.5;
            if clipped {
                push(
                    findings,
                    scene,
                    layer.id,
                    LayoutCheck::TextClipped,
                    format!("frame {n}: {} on {}x{}", describe(&r), frame.w, frame.h),
                );
                return;
            }
            let tol = SAFE_TOLERANCE_U * frame.u + box_padding(r.w, scale, counted);
            let sf = &frame.safe;
            if r.x < sf.x - tol
                || r.y < sf.y - tol
                || r.x + r.w > sf.x + sf.w + tol
                || r.y + r.h > sf.y + sf.h + tol
            {
                push(
                    findings,
                    scene,
                    layer.id,
                    LayoutCheck::TextOutsideSafe,
                    format!(
                        "frame {n}: {} vs safe x {:.0}..{:.0} y {:.0}..{:.0}",
                        describe(&r),
                        sf.x,
                        sf.x + sf.w,
                        sf.y,
                        sf.y + sf.h
                    ),
                );
            }
        }
        LayerKind::Group { .. } => {}
        _ if decorative || moving => {}
        kind => {
            let r = bounds(layer.width, &layer.transform, 0.0, layer.height);
            let frac = inside_fraction(&r, &canvas);
            if matches!(kind, LayerKind::Image { .. }) {
                if frac < MIN_IMAGE_INSIDE {
                    push(
                        findings,
                        scene,
                        layer.id,
                        LayoutCheck::SubjectImageOutside,
                        format!(
                            "frame {n}: {:.0}% of the image inside the canvas ({})",
                            frac * 100.0,
                            describe(&r)
                        ),
                    );
                }
            } else if frac <= 0.0 {
                push(
                    findings,
                    scene,
                    layer.id,
                    LayoutCheck::LayerOffCanvas,
                    format!(
                        "frame {n}: {} {} entirely off the canvas ({})",
                        kind.type_name(),
                        layer.id,
                        describe(&r)
                    ),
                );
            }
        }
    }
}
