//! Beat recipes: how each semantic purpose/relationship becomes layout and motion.
//!
//! Every beat gets the same editorial skeleton (camera stage, kicker + accent
//! rule, ghost word, overlap-aware exit) and a purpose-specific composition.
//! Subjects are placed through [`place_subject`], which transparently turns
//! carried subjects into SharedElement track keys.

use std::borrow::Cow;

use super::direction::{
    self, AccentMark, CameraMove, EntranceFamily, ExitFamily, MarkShape, Placed, Reveal,
};
use super::grammar::{self, Composition, Grammar, SelectInput};
use super::layout_frame::LayoutFrame;
use super::taste::{ExitGesture, Level};
use super::taste_rules;
use super::temporal;
use super::typeset::{text_layer, Block, HeadlineBudget, Typesetter, Voice};
use super::{
    base_layer, beat_camera, mix, mo, rect_layer, round3, slug, subject_key, texture_layer,
    track_key, wants_carry, BeatPlan, Carry, CompileError, Ctx, Which,
};
use crate::assets::AssetRole;
use crate::easing::Easing;
use crate::intent::{Beat, Format, Purpose, Relationship, Subject, SubjectKind};
use crate::motion::kinetic::{self, KineticParams, TextRun};
use crate::motion::language::{EmphasisMotion, HeadlineMotion, Language};
use crate::motion::{stagger, Expansion};
use crate::scene::{
    AssetKind, Camera, CameraMotion, CameraOp, Color, Direction, Fit, HAlign, KeyState, Layer,
    LayerKind, LayoutBinding, Material, Motion, MotionOp, Padding, Scene, SharedElement, Stroke,
    TextAlign, TextureSpec, VAlign,
};

/// Placement rectangle given by its center (canvas space).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slot {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
}

/// Semantic pressure applied to a subject after it has entered.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pressure {
    pub start: f64,
    pub end: f64,
    pub scale: f32,
    pub dy: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Entrance {
    /// Fade + rise by preset travel.
    Rise,
    /// Clip reveal upwards with a scale settle.
    Pop,
}

pub(super) struct B<'p> {
    pub(super) plan: &'p BeatPlan,
    pub(super) beat: &'p Beat,
    pub(super) stage: Vec<Layer>,
    pub(super) motions: Vec<Motion>,
    /// Where carried-in subjects that this beat doesn't mention go.
    pub(super) anchor: Option<Slot>,
    /// Indices of carries that received keys in this beat.
    pub(super) placed: Vec<usize>,
    /// (0.5) A SharedElement at this z must interleave with the stage: stage
    /// layers above it go to a second, identically animated front group.
    pub(super) split_above: Option<i32>,
    /// (0.18) The layer this beat is about (the amount, the hero picture):
    /// the camera focuses on it and emphasis devices land on it.
    pub(super) focal: Option<String>,
    /// (0.20) Which layer groups stand for which spoken words
    /// (`RevealAnchor`): the figure's value, the stamp's keyword, a label's
    /// part, the title's key word. Declared with [`B::reveal`]; word cues
    /// and speech QA read them through `ArtRecord.reveals`.
    pub(super) reveals: Vec<crate::speech::RevealAnchor>,
}

impl B<'_> {
    pub(super) fn id(&self, name: &str) -> String {
        format!("{}.{}", self.plan.prefix, name)
    }
    /// (0.20) Declare that the layers of `group` (a layer-id path after the
    /// scene prefix, e.g. `hero`, `card.0`, `title`) stand for `words`.
    /// Empty words are dropped; a group declared twice keeps both entries.
    pub(super) fn reveal<I, S>(&mut self, group: &str, words: I, role: crate::speech::RevealRole)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let words: Vec<String> = words
            .into_iter()
            .map(Into::into)
            .filter(|w| !w.trim().is_empty())
            .collect();
        if words.is_empty() {
            return;
        }
        self.reveals.push(crate::speech::RevealAnchor {
            group: group.to_string(),
            words,
            role,
        });
    }
    pub(super) fn push(&mut self, layer: Layer) {
        self.stage.push(layer);
    }
}

/// (0.20) The words a subject is named by when spoken: an object's asset name
/// (humanised), its value and its meaning. Used for reveal anchors.
pub(crate) fn subject_words(s: &Subject) -> Vec<String> {
    let mut out = Vec::new();
    if let Subject::Object(o) = s {
        out.push(o.asset.replace(['_', '-'], " "));
    }
    if let Some(v) = s.value() {
        out.push(v.to_string());
    }
    if let Some(m) = s.meaning() {
        out.push(m.to_string());
    }
    out.retain(|w| !w.trim().is_empty());
    out
}

/// (0.20) The words of a display `title` that state the beat's conclusion:
/// its numbers (with a following scale word, "$381 billion") and the keyword
/// when the title contains it. A title that only names the topic has none,
/// so under `TitleReveal::Hold` it enters with the beat; a title that says
/// the number waits for the narrator to say it.
pub(crate) fn title_key_words(beat: &Beat, title: &str) -> Vec<String> {
    const SCALES: [&str; 8] = [
        "hundred", "thousand", "million", "billion", "trillion", "lakh", "crore", "percent",
    ];
    let tokens = crate::speech::statement_tokens(title);
    let norm = |t: &str| crate::speech::normalize_word(t);
    let mut out: Vec<String> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if !t.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        match tokens.get(i + 1) {
            Some(next) if SCALES.contains(&norm(next).as_str()) => {
                out.push(format!("{t} {next}"));
            }
            _ => out.push(t.clone()),
        }
    }
    if let Some(k) = beat.keyword.as_deref() {
        let wanted: Vec<String> = k
            .split_whitespace()
            .map(norm)
            .filter(|w| !w.is_empty())
            .collect();
        let have: Vec<String> = tokens.iter().map(|t| norm(t)).collect();
        if !wanted.is_empty() && wanted.iter().all(|w| have.contains(w)) {
            out.push(k.to_string());
        }
    }
    out
}

pub(crate) fn build_beat(
    ctx: &mut Ctx,
    plan: &BeatPlan,
    beat: &Beat,
    carries: &mut Vec<Carry>,
    caption_reserve: Option<f32>,
) -> Result<Scene, CompileError> {
    // (0.23 W4) Product path: a beat whose speech-placed ENTER would leave only
    // the backdrop on screen past the dead-air limit enters with its sentence.
    let opened = if super::speech_lifecycle::dead_air_rules(ctx) {
        super::speech_lifecycle::open_with_the_sentence(plan, &ctx.spoken)
    } else {
        None
    };
    let plan = opened.as_ref().unwrap_or(plan);
    // (0.10 Q) The screen shows the beat's title: a statement too long to be a
    // title (narration written into `statement`) is replaced, for every
    // builder, by the short title `taste_rules::display_text` derived from it.
    // The full sentence is spoken (captions) or set as small body copy by
    // [`deck`]. Short statements pass through untouched.
    let titled = titled_beat(beat, plan);
    let beat: &Beat = &titled;
    // (0.10 Q) With a voice-over the captions own the lower lane: builders lay
    // the beat out on the canvas above it (`compose` swaps the frame), and a
    // beat whose content still reaches into the lane is laid out again on a
    // shorter canvas until it fits. Furniture, the stage and the captions keep
    // the real frame.
    let real_frame = ctx.frame;
    let limit = caption_reserve.map(|_| super::captions::content_limit(&real_frame));
    let mut reserve = caption_reserve;
    let mut attempts = 0;
    let mut b = loop {
        let saved = limit.map(|_| carries.clone());
        let b = compose(ctx, plan, beat, carries, &real_frame, reserve)?;
        let (Some(limit), Some(px)) = (limit, reserve) else {
            break b;
        };
        let over = content_bottom(ctx, &b) - limit;
        if over <= LANE_FIT_SLACK
            || attempts == LANE_FIT_ATTEMPTS
            || px + over > LANE_FIT_MAX_SHARE * real_frame.h
        {
            break b;
        }
        attempts += 1;
        reserve = Some(px + over * 1.1 + 2.0);
        if let Some(saved) = saved {
            *carries = saved;
        }
    };
    if let Some(f) = b.focal.clone() {
        ctx.focal.insert(plan.index, f);
    }
    if !b.reveals.is_empty() {
        ctx.reveals.insert(plan.index, b.reveals.clone());
    }
    super::furniture::furniture(ctx, &mut b);
    close_carries(ctx, &mut b, carries);
    let mut scene = wrap_stage(ctx, b);
    clamp_to_scene(&mut scene);
    // (0.23 A4-short) Product path: a text that would only appear once the
    // beat anticipates its exit is brought into view first.
    if ctx.direction_seed.is_some() {
        pull_late_text_into_view(&mut scene);
    }
    // (0.23 W4) No voice-over names a number here: counts settle by READ and
    // hold before ANTICIPATE. With one, `apply_word_cues` does it once the
    // cues have placed every group.
    if ctx.spoken.is_empty() {
        super::speech_plan::settle_counts(&mut scene, &[], &[]);
    }
    Ok(scene)
}

/// Extra layouts a beat may take to fit above the caption lane.
const LANE_FIT_ATTEMPTS: usize = 4;
/// Overshoot (px) into the lane limit that still counts as fitting.
const LANE_FIT_SLACK: f32 = 0.75;
/// The lane reserve never takes more than this share of the canvas height.
const LANE_FIT_MAX_SHARE: f32 = 0.5;

/// One layout of a beat: select the composition, run its builder, add the
/// kicker and place the block on the canvas. With a `reserve` the builders see
/// the canvas without its bottom `reserve` pixels ([`LayoutFrame::with_bottom_reserve`]);
/// `ctx` is back on the real canvas when this returns.
fn compose<'p>(
    ctx: &mut Ctx,
    plan: &'p BeatPlan,
    beat: &'p Beat,
    carries: &mut Vec<Carry>,
    real_frame: &LayoutFrame,
    reserve: Option<f32>,
) -> Result<B<'p>, CompileError> {
    if let Some(px) = reserve {
        ctx.frame = real_frame.with_bottom_reserve(px);
    }
    // (0.23) The direction layer may pick a curated alternate (B2a);
    // without a direction seed this is `grammar::select`.
    let candidates = grammar::candidates(SelectInput {
        beat,
        language: plan.lang.language,
        format: ctx.format,
        side_by_side: grammar::placement::side_by_side(&ctx.frame),
        seed: plan.seed,
        has_subject_image: grammar::plate::subject_image(ctx, plan.index).is_some(),
        has_object_image: grammar::plate::image(
            ctx,
            plan.index,
            crate::assets::AssetRole::HeroObject,
        )
        .is_some(),
        visual: &ctx.taste.visual,
        look_grammar: ctx.art.as_ref().and_then(|(a, _)| a.fx.grammar),
    });
    let comp = grammar::choose(ctx, plan.index, candidates);
    let mut b = B {
        reveals: Vec::new(),
        plan,
        beat,
        stage: Vec::new(),
        motions: Vec::new(),
        anchor: None,
        placed: Vec::new(),
        split_above: None,
        focal: None,
    };
    // (0.9) Tall canvases: lay the content out as on a 1920u-tall canvas, then
    // centre that block slightly high. Furniture stays pinned to the real edges.
    let (h_real, h_eff, dy) = tall_budget(ctx);
    ctx.h = h_eff;
    // (0.10 Q) The display headline obeys its budget (height share, line cap)
    // in every builder; see `Typesetter::fit_block`.
    let budget = headline_budget(ctx, real_frame, &comp, beat, plan.index);
    ctx.ts.set_headline_budget(Some(budget));
    let built = grammar::builder(comp)(ctx, &mut b, carries, comp);
    ctx.ts.set_headline_budget(None);
    if built.is_ok() {
        kicker(ctx, &mut b);
        if dy > 0.0 {
            shift_content(ctx.w, h_eff, h_real, dy, &mut b, carries);
        }
    }
    ctx.h = h_real;
    ctx.frame = *real_frame;
    built?;
    Ok(b)
}

/// Canvas y of the lowest content a beat builds on its stage: the static boxes
/// of text, cards and images (with the downward move a layer ends on), not the
/// ghost word, textures, furniture or a full-bleed ground.
fn content_bottom(ctx: &Ctx, b: &B) -> f32 {
    /// Final downward offset of `id` (the last `move` ending on it).
    fn final_dy(b: &B, id: &str) -> f32 {
        b.motions
            .iter()
            .filter(|m| m.target == id)
            .filter_map(|m| match m.op {
                MotionOp::Move { to, .. } => Some((m.start + m.duration, to[1])),
                _ => None,
            })
            .fold(None::<(f64, f32)>, |best, c| match best {
                Some(best) if best.0 >= c.0 => Some(best),
                _ => Some(c),
            })
            .map_or(0.0, |(_, dy)| dy.max(0.0))
    }
    /// A layer that starts transparent but fades in later (e.g. a direction
    /// line revealed at EVOLVE) still occupies its box once shown.
    fn fades_in(b: &B, id: &str) -> bool {
        b.motions
            .iter()
            .any(|m| m.target == id && matches!(m.op, MotionOp::Fade { to, .. } if to >= 0.2))
    }
    fn walk(layers: &[Layer], (ox, oy): (f32, f32), area: f32, b: &B, bottom: &mut f32) {
        for l in layers {
            if !l.visible
                || l.layout.is_some()
                || (l.opacity < 0.2 && !fades_in(b, &l.id))
                || crate::layout_qa::is_decorative(&l.id, &l.kind)
            {
                continue;
            }
            let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
            let (x0, y0) = (ox + l.x - l.anchor_x * w, oy + l.y - l.anchor_y * h);
            let dy = final_dy(b, &l.id);
            match &l.kind {
                LayerKind::Group { children } => walk(children, (x0, y0 + dy), area, b, bottom),
                LayerKind::Texture(_) => {}
                _ if w * h >= 0.6 * area => {}
                _ => *bottom = bottom.max(y0 + h + dy),
            }
        }
    }
    let mut bottom = 0.0f32;
    walk(&b.stage, (0.0, 0.0), ctx.w * ctx.h, b, &mut bottom);
    bottom
}

/// (0.10 Q) The beat as the builders see it: the statement is the display
/// title (`plan.display`). Borrowed (nothing cloned) when the statement is
/// already a title.
fn titled_beat<'a>(beat: &'a Beat, plan: &BeatPlan) -> Cow<'a, Beat> {
    if plan.display.derived {
        let mut titled = beat.clone();
        titled.statement = plan.display.title.clone();
        Cow::Owned(titled)
    } else {
        Cow::Borrowed(beat)
    }
}

/// Smallest size (u) a budgeted headline is shrunk to before its text is cut:
/// the body size of `explain` beats, so a headline never reads as body copy.
const HEADLINE_FLOOR_U: f32 = 46.0;

/// (0.10 Q) The display headline budget of a beat on the frame it is laid out
/// on: block height as a share of the layout height (a beat that shows a
/// subject image / object leaves the headline less room than a type-only
/// beat), lines by the real canvas class, and the size floor.
fn headline_budget(
    ctx: &Ctx,
    real: &LayoutFrame,
    comp: &Composition,
    beat: &Beat,
    index: usize,
) -> HeadlineBudget {
    let share = if shows_subject(ctx, comp, beat, index) {
        taste_rules::HEADLINE_MAX_H_WITH_SUBJECT
    } else {
        taste_rules::HEADLINE_MAX_H_TYPE_ONLY
    };
    let lines = if real.class == Format::Vertical {
        taste_rules::HEADLINE_MAX_LINES_TALL
    } else {
        taste_rules::HEADLINE_MAX_LINES_WIDE
    };
    HeadlineBudget {
        text: Typesetter::prepare(&Voice::HEADLINE, &beat.statement),
        max_h: share * ctx.h,
        max_lines: lines,
        min_size: (HEADLINE_FLOOR_U * ctx.u).max(real.min_type_px),
    }
}

/// Whether the beat shows a subject image, object or figure beside its
/// headline (the tighter headline budget): a subject-first grammar, an object
/// subject (also inside a collection), or a delivered image for the beat.
fn shows_subject(ctx: &Ctx, comp: &Composition, beat: &Beat, index: usize) -> bool {
    use crate::intent::CollectionItem;
    let object = |s: &Subject| match s {
        Subject::Object(_) => true,
        Subject::Collection(c) => c
            .items
            .iter()
            .any(|i| matches!(i, CollectionItem::Object(_))),
        _ => false,
    };
    matches!(
        comp.grammar,
        Grammar::TypeImageInterlock | Grammar::HeroObject | Grammar::EvidenceStack
    ) || object(&beat.primary)
        || beat.secondary.as_ref().is_some_and(object)
        || [
            AssetRole::HeroSubject,
            AssetRole::Portrait,
            AssetRole::HeroObject,
            AssetRole::EvidenceImage,
        ]
        .iter()
        .any(|r| grammar::plate::image(ctx, index, *r).is_some())
        || ctx.image_carry.contains(&index)
        || index
            .checked_sub(1)
            .is_some_and(|prev| ctx.image_carry.contains(&prev))
}

/// `(real height, layout height, content offset)` from the frame
/// ([`LayoutFrame::content_h`] / [`LayoutFrame::content_y0`]). At or below
/// 1920u (every legacy canvas) the offset is zero and nothing changes.
fn tall_budget(ctx: &Ctx) -> (f32, f32, f32) {
    (ctx.h, ctx.frame.content_h(), ctx.frame.content_y0())
}

/// Move a beat's content (laid out for height `h_eff`) down by `dy`: top-level
/// layers without a layout binding, absolute accent-expand targets, the anchor
/// slot and this beat's SharedElement keys. Full-bleed layers stay at the top
/// and grow to the real height.
fn shift_content(w: f32, h_eff: f32, h_real: f32, dy: f32, b: &mut B, carries: &mut [Carry]) {
    let mut shifted: Vec<String> = Vec::new();
    for l in &mut b.stage {
        let bleed =
            l.x.abs() < 0.5 && l.y.abs() < 0.5 && l.width >= w - 1.0 && l.height >= h_eff - 1.0;
        if bleed {
            l.height += h_real - h_eff;
        } else if l.layout.is_none() {
            l.y += dy;
            shifted.push(l.id.clone());
        }
    }
    for m in &mut b.motions {
        if let MotionOp::AccentExpand { to } = &mut m.op {
            if shifted.contains(&m.target) {
                to.y += dy;
            }
        }
    }
    if let Some(slot) = &mut b.anchor {
        slot.cy += dy;
    }
    for &ci in &b.placed {
        if let Some(c) = carries.get_mut(ci) {
            for k in c.element.track.iter_mut().filter(|k| k.scene == b.plan.id) {
                if let Some(y) = k.state.y.as_mut() {
                    *y += dy;
                }
            }
        }
    }
}

/// The validator's tolerance on a motion's end (`validate::EPS`).
const END_EPS: f64 = 1e-6;

/// `v` rounded down to the millisecond (a value already on one is kept).
fn floor3(v: f64) -> f64 {
    ((v * 1000.0) + 1e-9).floor() / 1000.0
}

/// (0.23) The duration `clamp_to_scene` gives a motion that starts at `start`
/// inside a scene ending at `end` but would run past it: `end - start` to the
/// millisecond. Rounding to the nearest millisecond can overshoot the end by
/// up to half of one (a motion that then fails validation: "ends after
/// scene"), so when the rounded duration overshoots by more than the
/// validator allows it is rounded down instead; every duration that already
/// fit is what it always was.
pub(super) fn clamped_duration(start: f64, end: f64) -> f64 {
    let nearest = round3(end - start);
    if start + nearest > end + END_EPS {
        floor3(end - start)
    } else {
        nearest
    }
}

/// Compiler invariant: no motion outlives its scene. Builders schedule from
/// lifecycle events with style-scaled durations; when a fast rhythm or
/// temperament shortens a beat, a late motion (e.g. an EVOLVE counter) could
/// otherwise end after the scene. Such a motion is shortened to land exactly
/// at the scene end (same from → to values). Motions that already fit are
/// untouched, so every valid output is unchanged.
///
/// (0.23) A motion that starts at or after the scene end (a continuous take
/// leaves beats of 1.2-2 s, shorter than some builders' fixed offsets) cannot
/// be shortened to fit; it is moved earlier instead, to end on the scene end,
/// but never into a motion of its own layer and channel (it is then shortened
/// to what is left; nothing else is touched). Only scenes that would not
/// validate change.
fn clamp_to_scene(scene: &mut Scene) {
    let end = scene.duration_seconds;
    for m in &mut scene.motions {
        if m.start < end && m.start + m.duration > end {
            m.duration = clamped_duration(m.start, end);
        }
    }
    let late: Vec<usize> = scene
        .motions
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.start.is_finite()
                && m.duration.is_finite()
                && m.start >= end
                && m.start + m.duration > end + END_EPS
        })
        .map(|(i, _)| i)
        .collect();
    for i in late {
        let (target, channel) = (
            scene.motions[i].target.clone(),
            scene.motions[i].op.channel(),
        );
        // The latest end of the motions already on this layer and channel.
        let busy = scene
            .motions
            .iter()
            .enumerate()
            .filter(|(j, o)| *j != i && o.target == target && o.op.channel() == channel)
            .filter(|(_, o)| o.start < end)
            .map(|(_, o)| o.start + o.duration)
            .fold(0.0_f64, f64::max);
        let room = (end - busy).max(0.0);
        let m = &mut scene.motions[i];
        if room < m.duration {
            m.duration = floor3(room);
        }
        m.start = (end - m.duration).max(0.0);
    }
}

/// (0.23 A4-short) A text must be at half opacity this long before ANTICIPATE
/// to be read in its beat (the value coverage samples every 0.1 s up to it).
const TEXT_IN_VIEW_BEFORE_ANTICIPATE: f64 = 0.18;

/// When the entrance fade `m` has its layer at half opacity (`None` for a
/// motion that is not a fade-in from below half).
fn half_opacity_at(m: &Motion) -> Option<f64> {
    let MotionOp::Fade { from, to } = m.op else {
        return None;
    };
    if from >= 0.5 || to < 0.5 {
        return None;
    }
    let want = f64::from((0.5 - from) / (to - from));
    let ease = |u: f64| {
        if m.spring.is_some() {
            Easing::OutQuint.apply(u)
        } else {
            m.easing.apply(u)
        }
    };
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if ease(mid) >= want {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(m.start + hi * m.duration)
}

/// (0.23 A4-short) Product path. A continuous take leaves beats too short for
/// some builders' arrival chains (a collection's items and its total step in
/// one after another and the last lands at ANTICIPATE): a text whose entrance
/// fade only has it at half opacity less than [`TEXT_IN_VIEW_BEFORE_ANTICIPATE`]
/// before ANTICIPATE is not read in its beat ("value_dropped"). Its own
/// motions, and the entrance of the layer it rests on (a slab it is bound to,
/// begun just before), move earlier by the shortfall - never before ENTER and
/// never into an earlier motion of the same layer and channel.
fn pull_late_text_into_view(scene: &mut Scene) {
    let Some(life) = scene.lifecycle else {
        return;
    };
    let limit = life.anticipate - TEXT_IN_VIEW_BEFORE_ANTICIPATE;
    // (text layer id, id of the layer it is bound to)
    fn texts(layers: &[Layer], out: &mut Vec<(String, Option<String>)>) {
        for l in layers {
            match &l.kind {
                LayerKind::Text(_) => {
                    out.push((l.id.clone(), l.layout.as_ref().map(|b| b.parent.clone())));
                }
                LayerKind::Group { children } => texts(children, out),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    texts(&scene.layers, &mut found);
    for (id, parent) in found {
        let entrance = scene
            .motions
            .iter()
            .filter(|m| m.target == id && half_opacity_at(m).is_some())
            .min_by(|a, b| a.start.total_cmp(&b.start));
        let Some(entrance) = entrance else { continue };
        let (start, Some(readable)) = (entrance.start, half_opacity_at(entrance)) else {
            continue;
        };
        if readable <= limit || start >= scene.duration_seconds {
            continue;
        }
        let mut shortfall = (readable - limit).min(start - life.enter);
        // What moves: the text from its entrance on, and the entrance of its
        // parent that began shortly before.
        let moving: Vec<usize> = scene
            .motions
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                (m.target == id && m.start >= start - 1e-9 && m.start <= start + 0.3)
                    || (parent.as_deref() == Some(m.target.as_str())
                        && m.start >= start - 0.5
                        && m.start <= start + 1e-9
                        && m.duration <= 1.2)
            })
            .map(|(i, _)| i)
            .collect();
        for &i in &moving {
            let m = &scene.motions[i];
            let channel = m.op.channel();
            let before = scene
                .motions
                .iter()
                .enumerate()
                .filter(|(j, o)| {
                    !moving.contains(j)
                        && o.target == m.target
                        && o.op.channel() == channel
                        && o.start + o.duration <= m.start + 1e-9
                })
                .map(|(_, o)| o.start + o.duration)
                .fold(0.0_f64, f64::max);
            shortfall = shortfall.min(m.start - before);
        }
        if shortfall < 0.02 {
            continue;
        }
        for i in moving {
            scene.motions[i].start -= shortfall;
        }
    }
}

/// Which structured composition a beat needs, decided by its subjects: the
/// primary's kind if structured, else the secondary's. `None` = the v0.1
/// purpose recipes (atomic subjects only), unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Structure {
    Collection,
    StateChange,
    DerivedMetric,
    /// (0.19) Layers: a persistent labelled column the beats move through.
    Layers,
}

pub(crate) fn structure_of(beat: &Beat) -> Option<Structure> {
    let of = |s: &Subject| match s.kind() {
        SubjectKind::Collection => Some(Structure::Collection),
        SubjectKind::StateChange => Some(Structure::StateChange),
        SubjectKind::DerivedMetric => Some(Structure::DerivedMetric),
        SubjectKind::Layers => Some(Structure::Layers),
        _ => None,
    };
    of(&beat.primary).or_else(|| beat.secondary.as_ref().and_then(of))
}

/// Canvas band below a structured beat's headline, plus when the headline has
/// been read (compositions start their own motion from there).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Region {
    /// Canvas y where the composition may start.
    pub top: f32,
    /// Canvas y the composition must stay above.
    pub bottom: f32,
    /// Left/right margin (canvas x) — content spans `[margin, w - margin]`.
    pub margin: f32,
    /// Scene-local seconds when the headline has read in.
    pub start: f64,
}

/// Shared skeleton of structured beats: background keyword, the statement as
/// a headline (motion-language driven) and the corner anchor for carried-in
/// subjects. Returns the free region below the headline.
pub(super) fn structured_head(ctx: &mut Ctx, b: &mut B) -> Region {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let t = b.plan.enter_at();
    ghost(ctx, b, 0.62 * h);
    let top = 0.17 * h;
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &b.beat.statement,
        w - 2.0 * m,
        0.15 * h,
        120.0 * u,
        3,
    );
    let (head_h, done) = headline_lines(
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
    let head_h = head_h + deck(ctx, b, top + head_h, done);
    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Region {
        top: top + head_h + 64.0 * u,
        bottom: h - 0.07 * h,
        margin: m,
        start: done,
    }
}

// ---------------------------------------------------------------------------
// Skeleton
// ---------------------------------------------------------------------------

/// (0.10 Q) The kicker's label: "the result" for a reveal, else the
/// primary's, then the secondary's `meaning` when it reads as a label
/// (`taste_rules::kicker_ok`: at most three words; a description such as
/// "family walking together with child" never does), else the beat's purpose
/// word. (0.21) An emphasize beat without such a meaning has no label: a bare
/// "NOTE" says nothing.
fn kicker_label(beat: &Beat) -> Option<&str> {
    if beat.purpose == Purpose::Reveal {
        return Some("the result");
    }
    beat.primary
        .meaning()
        .into_iter()
        .chain(beat.secondary.as_ref().and_then(|s| s.meaning()))
        .find(|m| taste_rules::kicker_ok(m))
        .or(match beat.purpose {
            Purpose::Emphasize => None,
            Purpose::Compare => Some("compare"),
            Purpose::Contrast => Some("contrast"),
            Purpose::Reveal => Some("result"),
            Purpose::Explain => Some("why"),
        })
}

/// (0.21) The kicker's text. A number only when the story counts (a countdown
/// or a list in order, [`super::sequence`]): `05 — sharks`, or `05` alone;
/// otherwise the label alone. `None` = no kicker.
fn kicker_text(rank: Option<u32>, label: Option<&str>) -> Option<String> {
    match (rank.map(super::sequence::rank_text), label) {
        (Some(r), Some(l)) => Some(format!("{r} — {l}")),
        (Some(r), None) => Some(r),
        (None, Some(l)) => Some(l.to_string()),
        (None, None) => None,
    }
}

/// (0.23 W4) The label a hero that waits for its word shows meanwhile: the
/// primary's `meaning` when it reads as a label (`taste_rules::kicker_ok`) and
/// is not the value (no digits, not the value's own words), upper case. The
/// value itself never shows before its word; the label may.
pub(crate) fn placeholder_label(beat: &Beat) -> Option<String> {
    let meaning = beat
        .primary
        .meaning()
        .filter(|m| taste_rules::kicker_ok(m))?;
    let lower = meaning.to_lowercase();
    let gives_it_away = meaning.chars().any(|c| c.is_ascii_digit())
        || beat.primary.value().is_some_and(|v| {
            let v = v.trim().to_lowercase();
            !v.is_empty() && lower.contains(&v)
        });
    (!gives_it_away).then(|| meaning.to_uppercase())
}

fn kicker(ctx: &Ctx, b: &mut B) {
    let beat = b.beat;
    let Some(text) = kicker_text(ctx.sequence.rank(b.plan.index), kicker_label(beat)) else {
        return;
    };
    // (0.23 W4) A held hero's label line already says it: no second, smaller copy.
    let label_id = b.id("hero_label");
    if b.stage.iter().any(|l| {
        l.id == label_id
            && matches!(&l.kind, LayerKind::Text(t) if t.text.to_lowercase() == text.to_lowercase())
    }) {
        return;
    }
    let m = ctx.margin();
    let block = ctx
        .ts
        .fit_line(Voice::KICKER, &text, ctx.w - 2.0 * m, 30.0 * ctx.u);
    let y = 0.105 * ctx.h;
    let id = b.id("kicker");
    let mut layer = text_layer(id.clone(), &block, ctx.palette.ink, TextAlign::Left);
    layer.x = m;
    layer.y = y;
    layer.z_index = 30;
    let p = b.plan.lang.preset;
    let t = b.plan.enter_at();
    // (0.23 W2c) The planned entrance and accent; the identity is the old kicker.
    let bp = ctx.params_for(b.plan.index);
    b.motions.push(edge_entrance(
        &bp,
        &id,
        t,
        p.duration * 0.9,
        Reveal::Clip,
        Direction::Up,
        p.easing,
    ));
    b.push(layer);

    let rule_w = match bp.accent {
        AccentMark::None => return,
        AccentMark::Underline => block.width(),
        _ => 96.0,
    };
    let rule_h = if bp.accent == AccentMark::Underline {
        4.0
    } else {
        7.0
    };
    let rule_id = b.id("kicker_rule");
    let rule = rect_layer(
        rule_id.clone(),
        (
            m,
            y + block.height() + 14.0 * ctx.u,
            if bp.accent == AccentMark::Underline {
                rule_w
            } else {
                rule_w * ctx.u
            },
            rule_h * ctx.u,
        ),
        ctx.palette.accent,
        31,
    );
    b.motions.push(mo::mask(
        &rule_id,
        t + 0.12,
        0.6,
        Direction::Right,
        Easing::OutCubic,
    ));
    b.push(rule);
}

/// Large, faint background word with slow parallax drift.
pub(super) fn ghost(ctx: &Ctx, b: &mut B, cy: f32) {
    let beat = b.beat;
    let Some(word) = beat
        .keyword
        .as_deref()
        .or(beat.primary.meaning())
        .or(beat.primary.value())
    else {
        return;
    };
    let block = ctx
        .ts
        .fit_line(Voice::GHOST, word, ctx.w * 1.25, 0.36 * ctx.h);
    let id = b.id("ghost");
    let alpha = match ctx.taste.density.ghost_word {
        Level::Low => 0x08,
        Level::Medium => 0x0E,
        Level::High => 0x16,
    };
    let mut layer = text_layer(
        id.clone(),
        &block,
        ctx.palette.ink.with_alpha(alpha),
        TextAlign::Left,
    );
    layer.x = -0.05 * ctx.w;
    layer.y = cy - block.height() / 2.0;
    layer.z_index = 0;
    layer.depth = plane(b.plan.lang.depth.background);
    let t = b.plan.enter_at();
    // (0.23) The parallax pull-back below fades the same channel from
    // ANTICIPATE: when the 1.2 s fade-in would still be running then (a short
    // speech-led beat) the two overlap and the scene does not validate, so the
    // fade-in ends where the pull-back begins. A beat that fits keeps 1.2 s.
    let mut pulls_back =
        b.plan.lang.language == Language::Parallax && b.plan.life.anticipate < b.plan.duration;
    let mut fade_in = 1.2;
    if pulls_back && t + fade_in > b.plan.life.anticipate + 1e-9 {
        // Less than 0.3 s of fade-in left: the ghost stays and does not pull back.
        if b.plan.life.anticipate - t >= 0.3 {
            fade_in = floor3(b.plan.life.anticipate - t);
        } else {
            pulls_back = false;
        }
    }
    b.motions
        .push(mo::fade(&id, t, fade_in, 0.0, 1.0, Easing::InOutCubic));
    // READ: slow continuous drift in every language (layered: strong, flat:
    // a subtle camera-like drift), linear over the whole beat.
    let d = direction::amplitude(
        &ctx.params_for(b.plan.index),
        if ctx.layered() { 46.0 } else { 16.0 }
            * ctx.u
            * temporal::background_drift(ctx.taste.layers.background)
            * temporal::read_drift(&ctx.taste.motion),
    );
    b.motions.push(mo::shift(
        &id,
        0.0,
        b.plan.duration,
        [d, 0.0],
        [-d, 0.0],
        Easing::Linear,
    ));
    let life = b.plan.life;
    // (0.6) Active rhythms: the background word steps forward at EVOLVE —
    // a foreground/background handoff as the hierarchy shifts.
    if let Some(k) = temporal::background_handoff(ctx.taste.rhythm) {
        if life.anticipate - life.evolve > 0.6 {
            b.motions
                .push(mo::scale(&id, life.evolve, 0.6, 1.0, k, b.plan.lang.settle));
        }
    }
    // Parallax: the background prepares the transition by fading back.
    if pulls_back {
        b.motions.push(mo::fade(
            &id,
            life.anticipate,
            b.plan.duration - life.anticipate,
            1.0,
            0.5,
            Easing::InOutCubic,
        ));
    }
    b.push(layer);
}

/// Depth for a camera plane; the subject plane (1.0) is left implicit.
pub(super) fn plane(k: f32) -> Option<f32> {
    ((k - 1.0).abs() > 1e-4).then_some(k)
}

/// Latest start that lets a motion of `span` seconds end inside the scene
/// (short beats would otherwise push late EVOLVE events past their end).
fn latest(b: &B, start: f64, span: f64) -> f64 {
    start.min(b.plan.duration - span - 0.002).max(0.0)
}

/// Hero scale reached by the end of EVOLVE when an emphasis has no secondary.
const HERO_CROP_SCALE: f32 = 1.04;
/// Stage scale reached by the end of a scene (anticipation pull-back).
const ANTICIPATE_SCALE: f32 = 0.975;
/// Upward lead (× u) the stage travels during ANTICIPATE before its exit.
const ANTICIPATE_LIFT: f32 = 14.0;

/// Wrap the beat content in a full-canvas camera group with push + exit.
///
/// (0.6) The style's temporal character shapes the wrap: composition rhythm
/// adds stage reframes during EVOLVE, temperament sets how strongly the stage
/// anticipates its exit, and transition character picks the exit gesture.
/// Classic styles take the pre-0.6 path exactly (no reframes, lift exit).
fn wrap_stage(ctx: &Ctx, b: B) -> Scene {
    let plan = b.plan;
    let bp = ctx.params_for(plan.index);
    let stage_id = b.id("stage");
    let front_id = b.id("stage_front");
    let mut stage = base_layer(
        stage_id.clone(),
        (ctx.w / 2.0, ctx.h / 2.0, ctx.w, ctx.h),
        LayerKind::Group { children: b.stage },
        if plan.accent_in { 40 } else { 10 },
    );
    stage.anchor_x = 0.5;
    stage.anchor_y = 0.5;
    let front = b
        .split_above
        .and_then(|z| split_front(&mut stage, z, front_id));
    let mut motions = b.motions;
    let life = plan.life;
    let (antic_scale, lift_k) = temporal::anticipation(&ctx.taste.motion);
    let anticipate_scale = if lift_k == 1.0 {
        ANTICIPATE_SCALE
    } else {
        antic_scale
    };

    // EVOLVE reframes: the hierarchy visibly shifts (stage steps in) at
    // evenly spaced points inside [evolve, anticipate).
    let (shifts, step) = temporal::hierarchy_shifts(ctx.taste.rhythm);
    let mut held = 1.0f32;
    let span = life.anticipate - life.evolve;
    if shifts > 0 && span > 0.8 {
        let dur = (span / shifts as f64).min(0.7) * 0.8;
        for k in 0..shifts {
            let at = life.evolve + span * k as f64 / shifts as f64;
            let to = held * (1.0 + direction::amplitude(&bp, step));
            motions.push(mo::scale(&stage_id, at, dur, held, to, plan.lang.settle));
            held = to;
        }
    }

    if !plan.is_last {
        let o = plan.overlap_out;
        // ANTICIPATE: the whole stage starts easing back before the bridge,
        // so the transition grows out of motion instead of a static hold.
        // (0.23 W2c) A planned scale-down exit deepens this same pull-back
        // (one Scale motion on the stage: a second one would conflict).
        let shrink = if bp.exit == ExitFamily::ScaleDown {
            0.86
        } else {
            1.0
        };
        motions.push(mo::scale(
            &stage_id,
            life.anticipate,
            plan.duration - life.anticipate,
            held,
            held * anticipate_scale * shrink,
            Easing::InOutCubic,
        ));
        // (0.23 W2c) The planned exit family, else the taste's gesture.
        let exit = match bp.exit {
            ExitFamily::Fade => ExitGesture::Fade,
            ExitFamily::SlideUp => ExitGesture::Lift,
            ExitFamily::SlideLeft => ExitGesture::Slide,
            _ => temporal::exit_gesture(&ctx.taste.transition),
        };
        if plan.accent_out {
            // Covered by the incoming accent wipe: just clear out underneath it.
            motions.push(mo::fade(
                &stage_id,
                plan.exit_at() + o * 0.6,
                o * 0.4,
                1.0,
                0.0,
                Easing::Linear,
            ));
        } else if plan.wipe_out {
            // A panel wipe covers quickly and uncovers early: be gone before
            // it starts uncovering so no old text ghosts through.
            motions.push(mo::fade(
                &stage_id,
                plan.exit_at() + o * 0.15,
                o * 0.35,
                1.0,
                0.0,
                Easing::Linear,
            ));
        } else if bp.exit == ExitFamily::Cut {
            // A hard cut: the stage holds, then is gone in two frames.
            motions.push(mo::fade(
                &stage_id,
                plan.exit_at() + o * 0.3,
                0.066,
                1.0,
                0.0,
                Easing::Linear,
            ));
        } else if bp.exit == ExitFamily::ScaleDown {
            // The pull-back above shrinks the stage; it fades as it goes.
            motions.push(mo::fade(
                &stage_id,
                plan.exit_at(),
                o * 0.6,
                1.0,
                0.0,
                Easing::OutCubic,
            ));
        } else if exit == ExitGesture::Lift {
            // Clear quickly so the incoming headline never collides with the outgoing one.
            motions.push(mo::fade(
                &stage_id,
                plan.exit_at(),
                o * 0.7,
                1.0,
                0.0,
                Easing::OutCubic,
            ));
            let lead = [0.0, -ANTICIPATE_LIFT * lift_k * ctx.u];
            motions.push(mo::shift(
                &stage_id,
                life.anticipate,
                (plan.exit_at() - life.anticipate).max(0.0),
                [0.0, 0.0],
                lead,
                Easing::InOutCubic,
            ));
            motions.push(mo::shift(
                &stage_id,
                plan.exit_at(),
                o,
                lead,
                [0.0, -90.0 * ctx.u],
                Easing::InCubic,
            ));
        } else {
            motions.push(mo::fade(
                &stage_id,
                plan.exit_at(),
                o * if exit == ExitGesture::Fade { 0.9 } else { 0.6 },
                1.0,
                0.0,
                Easing::OutCubic,
            ));
            match exit {
                ExitGesture::Slide => {
                    let lead = [ANTICIPATE_LIFT * lift_k * ctx.u, 0.0];
                    motions.push(mo::shift(
                        &stage_id,
                        life.anticipate,
                        (plan.exit_at() - life.anticipate).max(0.0),
                        [0.0, 0.0],
                        lead,
                        Easing::InOutCubic,
                    ));
                    motions.push(mo::shift(
                        &stage_id,
                        plan.exit_at(),
                        o,
                        lead,
                        [-0.3 * ctx.w, 0.0],
                        Easing::InCubic,
                    ));
                }
                ExitGesture::Punch => {
                    let from = held * anticipate_scale;
                    motions.push(mo::scale(
                        &stage_id,
                        plan.exit_at(),
                        o * 0.6,
                        from,
                        from * 1.14,
                        Easing::InCubic,
                    ));
                }
                ExitGesture::Fade | ExitGesture::Lift => {}
            }
        }
    }
    let mut layers = vec![stage];
    if let Some(front) = front {
        // The front group moves exactly like the stage.
        let copies: Vec<Motion> = motions
            .iter()
            .filter(|m| m.target == stage_id)
            .map(|m| Motion {
                target: front.id.clone(),
                ..m.clone()
            })
            .collect();
        motions.extend(copies);
        layers.push(front);
    }
    if plan.wipe_in {
        let (wipe_layers, wipe_motions) = super::transition::panel_wipe(ctx, plan);
        layers.extend(wipe_layers);
        motions.extend(wipe_motions);
    }
    Scene {
        post: Vec::new(),
        id: plan.id.clone(),
        start_seconds: plan.start,
        duration_seconds: plan.duration,
        layers,
        motions,
        // (0.19) A layers beat has no camera of its own: the column is a
        // backdrop, and a pushing or drifting camera would pull the pinned
        // subject off its layer.
        camera: if matches!(b.beat.primary, Subject::Layers(_)) {
            None
        } else {
            planned_camera(ctx, plan, &bp)
        },
        lifecycle: Some(plan.life),
    }
}

/// (0.23 W2c) An edge entrance (clip or mask reveal) steered by the beat's
/// params: the builder's reveal, direction and easing for the identity. The
/// pattern builders copy for their own entrances.
pub(super) fn edge_entrance(
    bp: &direction::BeatParams,
    id: &str,
    start: f64,
    dur: f64,
    reveal: Reveal,
    dir: Direction,
    easing: Easing,
) -> Motion {
    let dir = direction::entrance_direction(bp, dir);
    let easing = direction::entrance_easing(bp, easing);
    match direction::entrance_reveal(bp, reveal) {
        Reveal::Clip => mo::line_in(id, start, dur, dir, easing),
        Reveal::Mask => mo::mask(id, start, dur, dir, easing),
    }
}

/// (0.23 W2c) The scene camera: the language's own (`beat_camera`) unless the
/// planner chose a move. Moves are gentle and whole-beat; `amplitude` scales
/// them, and a still camera is `None`.
fn planned_camera(ctx: &Ctx, plan: &BeatPlan, bp: &direction::BeatParams) -> Option<Camera> {
    let push = |from: f32, to: f32| CameraMotion {
        start: 0.0,
        duration: plan.duration,
        easing: Easing::InOutCubic,
        velocity: None,
        op: CameraOp::Push { from, to },
    };
    let track = |to: [f32; 2]| CameraMotion {
        start: 0.0,
        duration: plan.duration,
        easing: Easing::InOutCubic,
        velocity: None,
        op: CameraOp::Track {
            from: [0.0, 0.0],
            to,
        },
    };
    let k = direction::amplitude(bp, 1.0);
    let motions = match bp.camera {
        CameraMove::Builder | CameraMove::Orbit => return beat_camera(ctx, plan),
        CameraMove::Still => return None,
        CameraMove::PushIn => vec![push(1.0, 1.0 + 0.05 * k)],
        CameraMove::PullOut => vec![push(1.0 + 0.05 * k, 1.0)],
        CameraMove::Truck => vec![track([-28.0 * k * ctx.u, 0.0])],
        CameraMove::Crane => vec![track([0.0, -28.0 * k * ctx.u])],
    };
    Some(Camera {
        perspective: None,
        pivot: None,
        motions,
    })
}

/// Move the stage children drawn above `z` (and not tied to a back layer by a
/// layout binding) into a front group placed just above `z`, so a
/// SharedElement at `z` sits between the two halves of the stage (0.5).
fn split_front(stage: &mut Layer, z: i32, id: String) -> Option<Layer> {
    let LayerKind::Group { children } = &mut stage.kind else {
        return None;
    };
    let parents: std::collections::BTreeSet<String> = children
        .iter()
        .filter_map(|l| l.layout.as_ref().map(|b| b.parent.clone()))
        .collect();
    let (front, back): (Vec<Layer>, Vec<Layer>) = std::mem::take(children)
        .into_iter()
        .partition(|l| l.z_index > z && l.layout.is_none() && !parents.contains(&l.id));
    *children = back;
    if front.is_empty() {
        return None;
    }
    let mut group = base_layer(
        id,
        (stage.x, stage.y, stage.width, stage.height),
        LayerKind::Group { children: front },
        stage.z_index.max(z + 2),
    );
    group.anchor_x = stage.anchor_x;
    group.anchor_y = stage.anchor_y;
    Some(group)
}

/// Headline block, animated by the beat's motion language: one layer per line
/// (line reveal / mask reveal / fade-rise) or one per word (word cascade, with
/// keyword emphasis when the beat's keyword is in the block). Line and word
/// timing follow the language's irregular stagger. Returns `(block height,
/// time the block has read in)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn headline_lines(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    block: &Block,
    top: f32,
    align: TextAlign,
    color: Color,
    start: f64,
    entrance_up: bool,
) -> (f32, f64) {
    let lang = b.plan.lang;
    let p = lang.preset;
    let line_x = |width: f32| match align {
        TextAlign::Left => ctx.margin(),
        TextAlign::Center => (ctx.w - width) / 2.0,
        TextAlign::Right => ctx.w - ctx.margin() - width,
    };
    // (0.23 W2c) Word cascades take the beat's easing, travel and stagger.
    let kp = direction::kinetic_params(
        &ctx.params_for(b.plan.index),
        KineticParams {
            preset: p,
            stagger: lang.stagger,
            u: ctx.u,
        },
    );

    if entrance_up && lang.typography.headline == HeadlineMotion::WordCascade {
        let block_w = block.width();
        let mut run = ctx.ts.text_run(
            b.id(name),
            block,
            color,
            [line_x(block_w * 1.02 + 2.0), top],
            false,
        );
        if align != TextAlign::Left {
            for (line, lw) in run.lines.iter_mut().zip(&block.line_widths) {
                let dx = match align {
                    TextAlign::Center => (block_w - lw) / 2.0,
                    _ => block_w - lw,
                };
                for unit in line.iter_mut() {
                    unit.x += dx;
                }
            }
        }
        let units: usize = run.lines.iter().map(Vec::len).sum();
        let last = stagger::offsets(kp.stagger, units)
            .into_iter()
            .fold(0.0, f64::max);
        let read = start + last + p.duration * 0.6;
        let keyword = b.beat.keyword.as_deref().and_then(|k| find_word(&run, k));
        // The keyword emphasis is secondary information: it fires in EVOLVE,
        // never later than 0.6 s before ANTICIPATE (unless the headline only
        // finishes reading later than that).
        let life = b.plan.life;
        let emphasis_at = (read + 0.2).max(life.evolve.min(life.anticipate - 0.6));
        let exp = match (lang.typography.emphasis, keyword) {
            (EmphasisMotion::ScaleEmphasis, Some(k)) => fit_emphasis_scale(
                ctx,
                &run,
                k,
                kinetic::type_scale_emphasis(&run, k, start, emphasis_at, &kp),
            ),
            (EmphasisMotion::KeywordPunch, Some(k)) => {
                kinetic::keyword_punch(&run, k, start, emphasis_at, ctx.palette.accent, &kp)
            }
            _ => kinetic::word_cascade(&run, start, &kp),
        };
        // (0.23) A headline that only finishes reading near the end of a short
        // beat (a slow cascade in a continuous take) would have its keyword
        // emphasis start at or after the scene end ("motion ends after scene"):
        // the headline then enters without it. Only such a beat changes.
        let exp = if exp.motions.iter().any(|m| m.start >= b.plan.duration) {
            kinetic::word_cascade(&run, start, &kp)
        } else {
            exp
        };
        absorb(b, exp, 20);
        // "Done" is when the headline has read in (ENTER scheduling), not the
        // later emphasis.
        return (block.height(), read);
    }

    let bp = ctx.params_for(b.plan.index);
    let offsets =
        direction::stagger_offsets(&bp, stagger::offsets(lang.stagger, block.lines.len()));
    let mut done = start;
    for (i, line) in block.lines.iter().enumerate() {
        let single = Block {
            lines: vec![line.clone()],
            size: block.size,
            line_widths: vec![block.line_widths[i]],
            voice: block.voice,
        };
        let id = b.id(&format!("{name}.{i}"));
        let mut layer = text_layer(id.clone(), &single, color, TextAlign::Left);
        layer.x = line_x(layer.width);
        layer.y = top + i as f32 * block.line_advance();
        layer.z_index = 20;
        let span = match (entrance_up, lang.typography.headline) {
            (true, HeadlineMotion::MaskReveal) => p.duration * 1.3,
            _ => p.duration,
        };
        let t = latest(b, start + offsets[i], span);
        done = done.max(t + p.duration * 0.6);
        match (entrance_up, lang.typography.headline) {
            (true, HeadlineMotion::MaskReveal) => {
                b.motions.push(edge_entrance(
                    &bp,
                    &id,
                    t,
                    p.duration * 1.1,
                    Reveal::Mask,
                    Direction::Right,
                    p.easing,
                ));
                b.motions.push(mo::shift(
                    &id,
                    t,
                    p.duration * 1.3,
                    [0.0, direction::travel(&bp, p.travel * 0.25 * ctx.u)],
                    [0.0, 0.0],
                    lang.settle,
                ));
            }
            (true, _) => {
                b.motions.push(edge_entrance(
                    &bp,
                    &id,
                    t,
                    p.duration,
                    Reveal::Clip,
                    Direction::Up,
                    p.easing,
                ));
            }
            (false, _) => {
                b.motions.push(mo::fade(
                    &id,
                    t,
                    p.duration * 0.7,
                    0.0,
                    1.0,
                    Easing::OutCubic,
                ));
                b.motions.push(mo::shift(
                    &id,
                    t,
                    p.duration,
                    [0.0, p.travel * 0.5 * ctx.u],
                    [0.0, 0.0],
                    p.easing,
                ));
            }
        }
        b.push(layer);
    }
    (block.height(), done)
}

/// Gap between a title block and its body copy (u).
const DECK_GAP_U: f32 = 22.0;
/// Body copy size under a derived title, and the smallest it may shrink to (u).
const DECK_SIZE_U: f32 = 40.0;
const DECK_MIN_U: f32 = 30.0;

/// (0.10 Q) The small body copy of a derived title, measured: a statement too
/// long to be a title, on a beat without a voice-over (with one, the captions
/// carry the sentence). The full statement in the body voice, at most
/// `taste_rules::BODY_MAX_LINES` lines (shrunk toward a floor, then cut with
/// "…"), never display size. `None` when the beat has no body copy.
pub(super) fn deck_block(ctx: &Ctx, b: &B) -> Option<Block> {
    let text = b.plan.display.body.as_deref()?;
    Some(ctx.ts.fit_body(
        Voice::BODY,
        text,
        DECK_SIZE_U * ctx.u,
        DECK_MIN_U * ctx.u,
        ctx.w - 2.0 * ctx.margin(),
        taste_rules::BODY_MAX_LINES,
    ))
}

/// Vertical space a measured deck takes under a title block (gap included).
pub(super) fn deck_space(ctx: &Ctx, block: &Block) -> f32 {
    DECK_GAP_U * ctx.u + block.height()
}

/// Place a measured deck under a title block whose bottom is `top`; it reads
/// in at `start` (when the title has read in).
pub(super) fn deck_lines(ctx: &Ctx, b: &mut B, block: &Block, top: f32, start: f64) {
    headline_lines(
        ctx,
        b,
        "deck",
        block,
        top + DECK_GAP_U * ctx.u,
        TextAlign::Left,
        ctx.palette.ink,
        start,
        false,
    );
}

/// (0.10 Q) The deck under a title block, for builders that flow what follows
/// from the headline's returned height: places it (see [`deck_lines`]) and
/// returns the vertical space it takes (0 without body copy).
pub(super) fn deck(ctx: &Ctx, b: &mut B, top: f32, start: f64) -> f32 {
    match deck_block(ctx, b) {
        Some(block) => {
            deck_lines(ctx, b, &block, top, start);
            deck_space(ctx, &block)
        }
        None => 0.0,
    }
}

/// `(line, unit)` of the first unit matching `word` (case/punctuation-insensitive).
pub(super) fn find_word(run: &TextRun, word: &str) -> Option<(usize, usize)> {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let key = norm(word);
    if key.is_empty() {
        return None;
    }
    run.lines.iter().enumerate().find_map(|(li, line)| {
        line.iter()
            .position(|u| norm(&u.text) == key)
            .map(|ui| (li, ui))
    })
}

/// Insert an expansion's layers (at `z`) and motions into the stage.
pub(super) fn absorb(b: &mut B, exp: Expansion, z: i32) {
    for mut l in exp.layers {
        l.z_index = l.z_index.max(z);
        b.push(l);
    }
    b.motions.extend(exp.motions);
}

pub(super) fn underline(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    rect: (f32, f32, f32, f32),
    color: Color,
    start: f64,
) {
    underline_with(
        ctx,
        b,
        &direction::BeatParams::default(),
        name,
        rect,
        color,
        start,
    );
}

/// (0.23 C2b) [`underline`] whose wipe takes the beat's planned direction,
/// reveal kind and easing (the identity is the old underline). Builders that
/// plan their marks call this; the shared [`underline`] keeps the old wipe for
/// every other builder.
fn underline_with(
    ctx: &Ctx,
    b: &mut B,
    bp: &direction::BeatParams,
    name: &str,
    rect: (f32, f32, f32, f32),
    color: Color,
    start: f64,
) {
    let id = b.id(name);
    b.push(rect_layer(id.clone(), rect, color, 26));
    let _ = ctx;
    let start = latest(b, start, 0.7);
    b.motions.push(edge_entrance(
        bp,
        &id,
        start,
        0.7,
        Reveal::Mask,
        Direction::Right,
        Easing::OutQuint,
    ));
}

/// (0.23 C2b) A decoration-only rule or underline a builder draws (never a
/// value or a label) under the beat's accent: removed for `None`, drawn as the
/// short `rule` bar or the long `underline` line for `Rule` / `Underline`, the
/// builder's `own` shape for any other accent (`direction::accent_shape`).
#[allow(clippy::too_many_arguments)]
fn accent_rule(
    ctx: &Ctx,
    b: &mut B,
    bp: &direction::BeatParams,
    name: &str,
    own: MarkShape,
    rule: (f32, f32, f32, f32),
    underline: (f32, f32, f32, f32),
    start: f64,
) {
    let Some(shape) = direction::accent_shape(bp, own) else {
        return;
    };
    let rect = match shape {
        MarkShape::Rule => rule,
        MarkShape::Underline => underline,
    };
    underline_with(ctx, b, bp, name, rect, ctx.palette.accent, start);
}

/// Tilted collage card with an offset halftone patch behind it.
#[allow(clippy::too_many_arguments)]
pub(super) fn collage_card(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    center: (f32, f32),
    size: (f32, f32),
    tilt: f32,
    halftone_dir: (f32, f32),
    start: f64,
    out: &mut Vec<Layer>,
) {
    collage_card_with(
        ctx,
        b,
        &direction::BeatParams::default(),
        name,
        center,
        size,
        tilt,
        halftone_dir,
        start,
        out,
    );
}

/// (0.23 C2b) [`collage_card`] steered by the beat's params; the identity is
/// the old card. The card and its halftone patch wipe in from the planned side
/// with the planned easing, the card settles from `amplitude` x its tilt,
/// `ScalePop` pops the card, and `None` leaves the patch out (a decoration:
/// the card and what it holds stay). Called by the recipes builders that plan
/// their beats; every other builder keeps [`collage_card`].
#[allow(clippy::too_many_arguments)]
fn collage_card_with(
    ctx: &Ctx,
    b: &mut B,
    bp: &direction::BeatParams,
    name: &str,
    center: (f32, f32),
    size: (f32, f32),
    tilt: f32,
    halftone_dir: (f32, f32),
    start: f64,
    out: &mut Vec<Layer>,
) {
    let u = ctx.u;
    if ctx.layered() && bp.accent != AccentMark::None {
        let (hw, hh) = (size.0 * 0.9, size.1 * 0.9);
        let hx = center.0 + halftone_dir.0 * (size.0 / 2.0 + 56.0 * u)
            - hw / 2.0
            - halftone_dir.0 * hw / 2.0;
        let hy = center.1 + halftone_dir.1 * (size.1 / 2.0 + 48.0 * u)
            - hh / 2.0
            - halftone_dir.1 * hh / 2.0;
        let ht_id = b.id(&format!("{name}.halftone"));
        let ht = texture_layer(
            &ht_id,
            (hx, hy, hw, hh),
            TextureSpec {
                material: Material::Halftone,
                seed: mix(b.plan.seed, 0x4A1F),
                color: ctx.palette.accent,
                intensity: 0.95,
                scale: 15.0 * u,
                animated: false,
            },
            8,
        );
        let mut ht = ht;
        ht.depth = plane(b.plan.lang.depth.midground);
        b.motions.push(edge_entrance(
            bp,
            &ht_id,
            start,
            0.9,
            Reveal::Mask,
            Direction::Down,
            Easing::OutCubic,
        ));
        out.push(ht);
    }
    let card_id = b.id(&format!("{name}.card"));
    let mut card = base_layer(
        card_id.clone(),
        (center.0, center.1, size.0, size.1),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: 10.0 * u,
            stroke: Some(Stroke {
                color: ctx.palette.ink.with_alpha(0x22),
                width: 2.0 * u,
            }),
        },
        10,
    );
    card.anchor_x = 0.5;
    card.anchor_y = 0.5;
    card.rotation_degrees = tilt;
    b.motions.push(edge_entrance(
        bp,
        &card_id,
        start + 0.08,
        0.75,
        Reveal::Mask,
        Direction::Up,
        Easing::OutQuint,
    ));
    b.motions.push(mo::rotate(
        &card_id,
        start + 0.08,
        1.2,
        direction::amplitude(bp, tilt * 0.5),
        0.0,
        direction::entrance_easing(bp, Easing::OutCubic),
    ));
    // ScalePop: the card pops about its centre (the card is not a parent of
    // the layers on it, so only the card moves).
    if bp.entrance == EntranceFamily::ScalePop {
        b.motions.push(mo::scale(
            &card_id,
            start + 0.08,
            0.9,
            direction::scale_amplitude(bp, 0.9),
            1.0,
            direction::entrance_easing(bp, Easing::OutQuint),
        ));
    }
    out.push(card);
}

// ---------------------------------------------------------------------------
// Subjects & continuity
// ---------------------------------------------------------------------------

/// (0.9) The asset-request id the planner (`asset_plan::plan_beat`, rule 2)
/// assigned to this beat's object `subject`, or `None` when it has none. Must
/// mirror the planner's role choice: primary object -> `hero_object` (hero
/// grammar) / `evidence_image` (evidence grammar) / `supporting_object`; a
/// second object falls back to `supporting_object`; a third gets none.
/// (0.22) A compare / contrast beat of two pictures (two objects): in every
/// look its primary picture is requested as `hero_object` and its secondary as
/// `supporting_object`, so both resolve (the genre looks used to give the
/// primary `supporting_object` and leave the secondary without a request).
///
/// (0.23) Two pictures that each carry a figure are a pair whatever the beat's
/// purpose: the street and studio looks stage them as a stat pair, which needs
/// both pictures requested (an emphasize beat of two valued pictures used to
/// get one request, and its second figure was dropped).
pub(crate) fn picture_pair(beat: &Beat) -> bool {
    let valued = |s: &Subject| s.value().is_some_and(|v| !v.trim().is_empty());
    beat.primary.kind() == SubjectKind::Object
        && beat
            .secondary
            .as_ref()
            .is_some_and(|s| s.kind() == SubjectKind::Object)
        && (matches!(beat.purpose, Purpose::Compare | Purpose::Contrast)
            || (valued(&beat.primary) && beat.secondary.as_ref().is_some_and(valued)))
}

pub(super) fn object_request_id(ctx: &Ctx, b: &B, subject: &Subject) -> Option<String> {
    let beat = b.beat;
    // The planner selects with no delivered images; so must we.
    let grammar = grammar::select(SelectInput {
        beat,
        language: b.plan.lang.language,
        format: ctx.format,
        side_by_side: grammar::placement::side_by_side(&ctx.frame),
        seed: b.plan.seed,
        has_subject_image: false,
        has_object_image: false,
        visual: &ctx.taste.visual,
        look_grammar: ctx.art.as_ref().and_then(|(a, _)| a.fx.grammar),
    })
    .grammar;
    let objects: Vec<&Subject> = [Some(&beat.primary), beat.secondary.as_ref()]
        .into_iter()
        .flatten()
        .collect();
    let this = objects
        .iter()
        .position(|s| std::ptr::eq(*s, subject))
        .or_else(|| objects.iter().position(|s| *s == subject))?;
    let mut taken: Vec<AssetRole> = Vec::new();
    for (i, s) in objects.into_iter().enumerate() {
        if s.kind() != SubjectKind::Object {
            continue;
        }
        let preferred = if grammar == Grammar::EvidenceStack {
            AssetRole::EvidenceImage
        } else if (matches!(grammar, Grammar::HeroObject | Grammar::Cinematic3d)
            || picture_pair(beat))
            && i == 0
        {
            AssetRole::HeroObject
        } else {
            AssetRole::SupportingObject
        };
        let role = [preferred, AssetRole::SupportingObject]
            .into_iter()
            .find(|r| !taken.contains(r));
        let is_this = i == this;
        match role {
            Some(role) if is_this => return Some(grammar::plate::request_id(b.plan.index, role)),
            Some(role) => taken.push(role),
            None if is_this => return None,
            None => {}
        }
    }
    None
}

/// Build a subject's layer centered on `slot` (coordinates as given).
pub(super) fn subject_layer(
    ctx: &mut Ctx,
    b: &B,
    id: String,
    subject: &Subject,
    slot: Slot,
    color: Color,
) -> Result<Layer, CompileError> {
    let mut layer = match subject.kind() {
        SubjectKind::Number => {
            let text = subject.display_text().unwrap_or("");
            let block = ctx.ts.fit_line(
                Voice::HERO_NUMBER,
                text,
                slot.w,
                slot.h / Voice::HERO_NUMBER.line_height,
            );
            with_ink(
                ctx,
                text_layer(id, &block, color, TextAlign::Center),
                &block,
            )
        }
        // Structured kinds never reach here (they have their own compositions);
        // fall back to their display text.
        SubjectKind::Phrase
        | SubjectKind::Collection
        | SubjectKind::StateChange
        | SubjectKind::DerivedMetric
        | SubjectKind::Layers => {
            let text = subject.display_text().unwrap_or("");
            let block = ctx
                .ts
                .fit_block(Voice::HEADLINE, text, slot.w, slot.h, 200.0 * ctx.u, 3);
            with_ink(
                ctx,
                text_layer(id, &block, color, TextAlign::Center),
                &block,
            )
        }
        SubjectKind::Object => {
            let name = subject.asset().ok_or_else(|| CompileError::Beat {
                beat: b.plan.index + 1,
                message: "object subject needs an 'asset'".into(),
            })?;
            let request = object_request_id(ctx, b, subject);
            let (asset, kind) = ctx.object_asset_for(name, b.plan.index + 1, request.as_deref())?;
            let side = slot.w.min(slot.h);
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
            base_layer(id, (0.0, 0.0, side, side), kind, 0)
        }
    };
    layer.x = slot.cx;
    layer.y = slot.cy;
    layer.anchor_x = 0.5;
    layer.anchor_y = 0.5;
    Ok(layer)
}

/// Attach measured ink bounds so layout can center the text optically.
pub(super) fn with_ink(ctx: &Ctx, mut layer: Layer, block: &Block) -> Layer {
    if let LayerKind::Text(style) = &mut layer.kind {
        style.ink = ctx.ts.ink(block);
    }
    layer
}

/// Centered binding to a container layer of this beat.
pub(super) fn centered_in(parent: &str, offset: [f32; 2]) -> LayoutBinding {
    LayoutBinding {
        parent: parent.to_string(),
        horizontal: HAlign::Center,
        vertical: VAlign::Center,
        offset,
        padding: Padding::default(),
    }
}

/// (0.10) Cap a keyword's emphasis growth so the grown word (scaled about
/// its centre) stays between the side margins. Words already at a margin do
/// not grow sideways past it; legacy output only changes where the word used
/// to cross the safe area.
pub(super) fn fit_emphasis_scale(
    ctx: &Ctx,
    run: &kinetic::TextRun,
    keyword: (usize, usize),
    mut exp: crate::motion::Expansion,
) -> crate::motion::Expansion {
    let Some(unit) = run.lines.get(keyword.0).and_then(|l| l.get(keyword.1)) else {
        return exp;
    };
    let width = unit.width * 1.02 + 2.0;
    let center = run.origin[0] + unit.x + width * 0.5;
    let m = ctx.margin();
    let half = (center - m).min(ctx.w - m - center);
    let max_scale = (2.0 * half / width).max(1.0);
    let id = format!("{}.{}.{}", run.id, keyword.0, keyword.1);
    for mo in exp.motions.iter_mut().filter(|mo| mo.target == id) {
        if let MotionOp::Scale { to, .. } = &mut mo.op {
            if *to > max_scale {
                *to = max_scale;
            }
        }
    }
    exp
}

/// Place a subject. Carried-in subjects receive track keys; subjects this beat
/// carries forward become new SharedElements; otherwise a normal layer is
/// returned for the caller to insert (with entrance and pressure motions
/// already added). `offset` converts canvas slot coordinates into the caller's
/// container space for the returned layer. With `bind`, the subject is
/// layout-bound to that container layer (centered in its *resolved* box, so it
/// follows the container's geometry animation) instead of sitting at the slot
/// center; the returned layer must then be pushed after the container.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_subject(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    which: Which,
    subject: &Subject,
    slot: Slot,
    offset: (f32, f32),
    color: Color,
    enter: f64,
    entrance: Entrance,
    pressure: Option<Pressure>,
    bind: Option<&str>,
    role: &str,
) -> Result<Option<Layer>, CompileError> {
    let plan = b.plan;
    let p = plan.lang.preset;
    let key = subject_key(subject);
    let carry_on = wants_carry(b.beat, which) && !plan.is_last;

    // 1. Carried in from the previous beat.
    if let Some(ci) = carries
        .iter()
        .position(|c| c.open && c.key == key && c.last_beat + 1 == plan.index)
    {
        let c = &mut carries[ci];
        let s = (slot.w / c.base_w).min(slot.h / c.base_h);
        let track = &mut c.element.track;
        // (0.10) Keep the carried subject inside the safe area after this
        // beat's camera drift (track + push), like edge furniture does.
        let cx = {
            let cam = plan.lang.camera;
            let drift = cam.track[0].abs() * ctx.u + ctx.w / 2.0 * (cam.push.max(1.0) - 1.0);
            let half = c.base_w * s / 2.0;
            let lo = ctx.frame.safe.x + half + drift;
            let hi = ctx.frame.safe.x + ctx.frame.safe.w - half - drift;
            if lo <= hi {
                slot.cx.clamp(lo, hi)
            } else {
                slot.cx
            }
        };
        let (x, y) = if bind.is_some() {
            (None, None)
        } else {
            (Some(cx), Some(slot.cy))
        };
        let mut arrive = track_key(
            &plan.id,
            enter + p.duration * 0.6,
            role,
            p.settle,
            KeyState {
                x,
                y,
                scale: Some(s),
                opacity: Some(1.0),
                ..Default::default()
            },
        );
        arrive.layout = bind.map(|parent| centered_in(parent, [0.0, 0.0]));
        track.push(arrive);
        let mut state = (cx, slot.cy, s);
        if let Some(pr) = pressure {
            track.push(track_key(
                &plan.id,
                pr.start,
                role,
                Easing::Linear,
                KeyState::default(),
            ));
            let dy = if bind.is_some() { 0.0 } else { pr.dy };
            state = (slot.cx, slot.cy + dy, s * pr.scale);
            track.push(track_key(
                &plan.id,
                pr.end,
                "pressure",
                Easing::InOutCubic,
                KeyState {
                    y: bind.is_none().then_some(state.1),
                    scale: Some(state.2),
                    ..Default::default()
                },
            ));
        }
        c.state = state;
        c.last_beat = plan.index;
        c.open = carry_on;
        b.placed.push(ci);
        // A carried subject the beat is about gets emphasized once it has
        // been re-read: a pulse on the shared element itself (EVOLVE).
        if matches!(b.beat.purpose, Purpose::Emphasize | Purpose::Reveal) && pressure.is_none() {
            let target = c.element.layer.id.clone();
            shared_pulse(ctx, b, &target, plan.life.evolve);
        }
        return Ok(None);
    }

    // 2. Starts here and is carried forward: a new shared element.
    if carry_on {
        // (0.23 W8a follow-up) On the product path a carried subject is read
        // from this beat's READ to the next one's (`carry_continuity`), so it
        // has half arrived by READ, like a carried picture: a secondary that
        // would wait for its word or for EVOLVE comes in earlier.
        let enter = if ctx.direction_seed.is_some() {
            enter.min((plan.life.read - 0.5 * p.duration).max(plan.enter_at()))
        } else {
            enter
        };
        let name = slug(&key);
        let mut layer = subject_layer(ctx, b, format!("shared.{name}"), subject, slot, color)?;
        layer.z_index = 45;
        let (bw, bh) = (layer.width, layer.height);
        let travel = p.travel * ctx.u;
        let bound = bind.is_some();
        let mut first = track_key(
            &plan.id,
            enter,
            role,
            Easing::Linear,
            KeyState {
                x: (!bound).then_some(slot.cx),
                y: (!bound).then_some(slot.cy + travel),
                opacity: Some(0.0),
                ..Default::default()
            },
        );
        first.layout = bind.map(|parent| centered_in(parent, [0.0, travel]));
        let mut rest = track_key(
            &plan.id,
            enter + p.duration,
            role,
            p.easing,
            KeyState {
                y: (!bound).then_some(slot.cy),
                opacity: Some(1.0),
                ..Default::default()
            },
        );
        rest.layout = bind.map(|parent| centered_in(parent, [0.0, 0.0]));
        let mut track = vec![first, rest];
        let mut state = (slot.cx, slot.cy, 1.0);
        if let Some(pr) = pressure {
            track.push(track_key(
                &plan.id,
                pr.start,
                role,
                Easing::Linear,
                KeyState::default(),
            ));
            let dy = if bound { 0.0 } else { pr.dy };
            state = (slot.cx, slot.cy + dy, pr.scale);
            track.push(track_key(
                &plan.id,
                pr.end,
                "pressure",
                Easing::InOutCubic,
                KeyState {
                    y: (!bound).then_some(state.1),
                    scale: Some(state.2),
                    ..Default::default()
                },
            ));
        }
        // Data language: a shared number counts up while it lands, like a local one.
        if plan.lang.data_numbers && subject.kind() == SubjectKind::Number {
            if let Some(count) = subject.value().and_then(|v| {
                mo::count(
                    &layer.id,
                    enter + p.duration * 0.15,
                    (p.duration * 2.0).max(1.1),
                    v,
                    plan.lang.settle,
                )
            }) {
                b.motions.push(count);
            }
        }
        carries.push(Carry {
            element: SharedElement {
                id: name,
                layer,
                track,
            },
            key,
            base_w: bw.max(1.0),
            base_h: bh.max(1.0),
            open: true,
            last_beat: plan.index,
            state,
        });
        b.placed.push(carries.len() - 1);
        return Ok(None);
    }

    // 3. Local layer.
    let id = b.id(&format!("{}.{}", role, slug(&key)));
    let local = Slot {
        cx: slot.cx - offset.0,
        cy: slot.cy - offset.1,
        ..slot
    };
    let mut layer = subject_layer(ctx, b, id.clone(), subject, local, color)?;
    layer.layout = bind.map(|parent| centered_in(parent, [0.0, 0.0]));
    // Data language: a number counts up to its value while it lands.
    if plan.lang.data_numbers && subject.kind() == SubjectKind::Number {
        if let Some(count) = subject.value().and_then(|v| {
            mo::count(
                &id,
                enter + p.duration * 0.15,
                (p.duration * 2.0).max(1.1),
                v,
                plan.lang.settle,
            )
        }) {
            b.motions.push(count);
        }
    }
    match entrance {
        Entrance::Rise => {
            b.motions.push(mo::fade(
                &id,
                enter,
                p.duration * 0.6,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
            b.motions.push(mo::shift(
                &id,
                enter,
                p.duration,
                [0.0, p.travel * ctx.u],
                [0.0, 0.0],
                p.easing,
            ));
        }
        Entrance::Pop => {
            b.motions
                .push(mo::line_in(&id, enter, p.duration, Direction::Up, p.easing));
        }
    }
    let scale_from = if entrance == Entrance::Pop {
        Some(1.12)
    } else {
        None
    };
    match (pressure, scale_from) {
        (Some(pr), _) => {
            b.motions.push(mo::scale(
                &id,
                pr.start,
                pr.end - pr.start,
                1.0,
                pr.scale,
                Easing::InOutCubic,
            ));
            if pr.dy != 0.0 && bind.is_none() {
                b.motions.push(mo::shift(
                    &id,
                    pr.start.max(enter + p.duration),
                    (pr.end - pr.start.max(enter + p.duration)).max(0.1),
                    [0.0, 0.0],
                    [0.0, pr.dy],
                    Easing::InOutCubic,
                ));
            }
        }
        (None, Some(from)) => {
            b.motions
                .push(mo::scale(&id, enter, p.duration * 1.4, from, 1.0, p.settle));
        }
        _ => {}
    }
    Ok(Some(layer))
}

/// Emphasis pulse on a shared element (scale up, then back to neutral so the
/// element leaves the scene exactly as its track says).
pub(super) fn shared_pulse(ctx: &Ctx, b: &mut B, target: &str, at: f64) {
    let limit = b.plan.life.anticipate;
    let pulse = 1.0 + (PULSE_SCALE - 1.0) * temporal::secondary_gain(&ctx.taste.motion);
    if at + PULSE_UP + PULSE_DOWN > limit {
        return;
    }
    b.motions.push(mo::scale(
        target,
        at,
        PULSE_UP,
        1.0,
        pulse,
        Easing::OutCubic,
    ));
    b.motions.push(mo::scale(
        target,
        at + PULSE_UP,
        PULSE_DOWN,
        pulse,
        1.0,
        Easing::InOutCubic,
    ));
}

const PULSE_SCALE: f32 = 1.1;
const PULSE_UP: f64 = 0.3;
const PULSE_DOWN: f64 = 0.55;

/// After a beat's recipe ran: park carried-in subjects the beat didn't mention
/// in its anchor slot, hold continuing ones until the transition, and retire
/// the rest.
fn close_carries(ctx: &Ctx, b: &mut B, carries: &mut [Carry]) {
    let plan = b.plan;
    let p = plan.lang.preset;
    for (ci, c) in carries.iter_mut().enumerate() {
        let carried_in = c.open && c.last_beat + 1 == plan.index;
        if carried_in {
            let slot = b.anchor.unwrap_or(Slot {
                cx: ctx.w - ctx.margin() - 120.0 * ctx.u,
                cy: 0.12 * ctx.h,
                w: 240.0 * ctx.u,
                h: 70.0 * ctx.u,
            });
            let s = (slot.w / c.base_w).min(slot.h / c.base_h);
            c.element.track.push(track_key(
                &plan.id,
                plan.enter_at() + p.duration,
                "anchor",
                p.settle,
                KeyState {
                    x: Some(slot.cx),
                    y: Some(slot.cy),
                    scale: Some(s),
                    opacity: Some(1.0),
                    ..Default::default()
                },
            ));
            c.state = (slot.cx, slot.cy, s);
            c.last_beat = plan.index;
            c.open = false;
            b.placed.push(ci);
        }
    }
    for &ci in &b.placed {
        let c = &mut carries[ci];
        if c.last_beat != plan.index {
            continue;
        }
        let last_at = c.element.track.last().map(|k| k.at).unwrap_or(0.0);
        if plan.is_last {
            c.element.track.push(track_key(
                &plan.id,
                plan.duration.max(last_at),
                "hold",
                Easing::Linear,
                KeyState::default(),
            ));
            c.open = false;
        } else {
            let hold_at = plan.exit_at().max(last_at);
            c.element.track.push(track_key(
                &plan.id,
                hold_at,
                "hold",
                Easing::Linear,
                KeyState::default(),
            ));
            if !c.open {
                c.element.track.push(track_key(
                    &plan.id,
                    round3(hold_at + plan.overlap_out * 0.8),
                    "exit",
                    Easing::InCubic,
                    KeyState {
                        opacity: Some(0.0),
                        ..Default::default()
                    },
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Purposes
// ---------------------------------------------------------------------------

/// emphasize: hero typography, clear hierarchy, mask reveals, collage support.
pub(super) fn emphasize(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let t = b.plan.enter_at();
    // (0.23 C2b) The beat's planned direction; the identity is the old beat.
    let bp = ctx.params_for(b.plan.index);
    // Layers placed by `place_subject`, whose entrance is steered at the end.
    let mut placed: Vec<Placed> = Vec::new();
    ghost(ctx, b, 0.60 * h);

    let top = 0.2 * h;
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &b.beat.statement,
        w - 2.0 * m,
        0.27 * h,
        176.0 * u,
        4,
    );
    let (head_h, lines_done) = headline_lines(
        ctx,
        b,
        "head",
        &head,
        top,
        TextAlign::Left,
        ctx.palette.ink,
        t + 0.1,
        true,
    );
    let head_h = head_h + deck(ctx, b, top + head_h, lines_done);

    // Vertical budget (non-legacy canvases): when the headline, hero card and
    // the secondary serif note would run past the safe bottom edge, the gaps
    // and the card shrink together.
    let sec_need = match &b.beat.secondary {
        Some(sec) if sec.kind() != SubjectKind::Object => {
            let text = sec.display_text().unwrap_or("");
            ctx.ts
                .fit_block(Voice::SERIF, text, w - 2.0 * m, 0.1 * h, 84.0 * u, 2)
                .height()
                + 23.0 * u
        }
        _ => 0.0,
    };
    let fit = grammar::placement::stack_fit(
        &ctx.frame,
        (h - m) - (top + head_h),
        220.0 * u + 0.2 * h + sec_need,
    );
    let gap = 110.0 * u * fit;
    let hero_top = top + head_h + gap;
    let hero_h = 0.2 * h * fit;
    let slot = Slot {
        cx: w / 2.0,
        cy: hero_top + hero_h / 2.0,
        w: w - 2.0 * m - 40.0 * u,
        h: hero_h,
    };
    let mut mid = Vec::new();
    collage_card_with(
        ctx,
        b,
        &bp,
        "hero",
        (slot.cx, slot.cy),
        (w - 2.0 * m + 30.0 * u, hero_h + 70.0 * u),
        -2.2,
        (-1.0, -1.0),
        lines_done - 0.2,
        &mut mid,
    );
    for l in mid {
        b.push(l);
    }
    let beat = b.beat;
    let mut hero_local: Option<String> = None;
    if let Some(layer) = place_subject(
        ctx,
        b,
        carries,
        Which::Primary,
        &beat.primary,
        slot,
        (0.0, 0.0),
        ctx.palette.ink,
        lines_done,
        Entrance::Rise,
        None,
        None,
        "hero",
    )? {
        let mut layer = layer;
        layer.z_index = 22;
        hero_local = Some(layer.id.clone());
        placed.push(Placed::of(&layer));
        b.push(layer);
    }

    // EVOLVE: the secondary line arrives at the first event, its underline
    // completes at the second.
    let life = b.plan.life;
    let events = life.evolve_events(2);
    let (ev0, ev1) = (events[0], events[1]);
    if let (None, Some(id)) = (&beat.secondary, &hero_local) {
        // Nothing secondary to reveal: a slow crop evolution on the hero keeps
        // the read alive without inventing information.
        b.motions.push(mo::scale(
            id,
            ev0,
            (life.anticipate - ev0).max(0.1),
            1.0,
            direction::scale_amplitude(
                &bp,
                1.0 + (HERO_CROP_SCALE - 1.0) * temporal::secondary_gain(&ctx.taste.motion),
            ),
            Easing::InOutCubic,
        ));
    }
    if b.plan.lang.language == Language::Minimal {
        // Minimal: a thin accent rule under the hero card extends at the last
        // evolve event (the kicker rule itself is added later by the skeleton).
        // (0.23 C2b) A decoration: `accent` removes it or draws it as a line
        // as wide as the card.
        let y = hero_top + hero_h + 62.0 * u;
        accent_rule(
            ctx,
            b,
            &bp,
            "hero_rule",
            MarkShape::Rule,
            (m, y, 140.0 * u, 5.0 * u),
            (m, y, w - 2.0 * m, 4.0 * u),
            ev1,
        );
    }
    let mut cursor = hero_top + hero_h + gap;
    if let Some(sec) = &beat.secondary {
        match sec.kind() {
            SubjectKind::Object => {
                let side = (h - cursor - 0.06 * h).min(0.3 * w);
                let slot = Slot {
                    cx: w - m - side / 2.0,
                    cy: cursor + side / 2.0,
                    w: side,
                    h: side,
                };
                if let Some(mut l) = place_subject(
                    ctx,
                    b,
                    carries,
                    Which::Secondary,
                    sec,
                    slot,
                    (0.0, 0.0),
                    ctx.palette.ink,
                    ev0,
                    Entrance::Rise,
                    None,
                    None,
                    "support",
                )? {
                    l.z_index = 24;
                    placed.push(Placed::of(&l));
                    b.push(l);
                }
            }
            _ => {
                let text = sec.display_text().unwrap_or("");
                let block = ctx
                    .ts
                    .fit_block(Voice::SERIF, text, w - 2.0 * m, 0.1 * h, 84.0 * u, 2);
                let start = ev0;
                headline_lines(
                    ctx,
                    b,
                    "serif",
                    &block,
                    cursor,
                    TextAlign::Left,
                    ctx.palette.ink,
                    start,
                    false,
                );
                cursor += block.height() + 14.0 * u;
                let uw = block.width().min(w - 2.0 * m);
                // (0.23 C2b) A decoration under the serif note: its own line
                // as long as the note, or the short bar, or none.
                accent_rule(
                    ctx,
                    b,
                    &bp,
                    "serif_rule",
                    MarkShape::Underline,
                    (m, cursor, 140.0 * u, 9.0 * u),
                    (m, cursor, uw, 9.0 * u),
                    // Second event if the 0.7 s wipe still fits before ANTICIPATE.
                    if ev1 + 0.7 <= life.anticipate {
                        ev1
                    } else {
                        ev0 + 0.4
                    },
                );
            }
        }
    }
    direction::steer_entrances(&bp, &mut b.motions, &placed);
    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

/// contrast / compare: primary in a bounded panel, secondary in a collage zone;
/// the relationship plays out as a semantic transformation over the beat.
pub(super) fn contrast(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let beat = b.beat;
    let p = plan.lang.preset;
    let t = plan.enter_at();
    // (0.23 C2b) The beat's planned direction; the identity is the old beat.
    let bp = ctx.params_for(plan.index);
    // Layers placed by `place_subject`, whose entrance is steered at the end.
    let mut placed: Vec<Placed> = Vec::new();
    let relationship = beat.relationship.unwrap_or(match beat.purpose {
        Purpose::Compare => Relationship::Separate,
        _ => Relationship::Compress,
    });
    ghost(ctx, b, 0.52 * h);

    let top = 0.17 * h;
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &beat.statement,
        w - 2.0 * m,
        0.14 * h,
        120.0 * u,
        3,
    );
    let (head_h, head_done) = headline_lines(
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
    let head_h = head_h + deck(ctx, b, top + head_h, head_done);

    // ENTER: headline, panel, primary. EVOLVE (one step per event): the
    // secondary zone arrives, the relationship transforms, the connective
    // names it. Nothing new starts after ANTICIPATE.
    let life = plan.life;
    let events = life.evolve_events(3);
    let (ev_zone, ev_rel, ev_conn) = (events[0], events[1], events[2]);
    let rel_start = ev_rel;
    let rel_end = (life.anticipate - 0.05)
        .max(rel_start + 0.5)
        .min(plan.duration - 0.05)
        .max(rel_start + 0.1);

    // Zone A: bounded panel holding the primary subject.
    let panel_top = top + head_h + 56.0 * u;
    let panel_h = 0.2 * h;
    let panel = (m, panel_top, w - 2.0 * m, panel_h);
    let panel_id = b.id("panel");
    b.push(base_layer(
        panel_id.clone(),
        panel,
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: 14.0 * u,
            stroke: Some(Stroke {
                color: ctx.palette.ink,
                width: 3.0 * u,
            }),
        },
        12,
    ));
    b.motions.push(edge_entrance(
        &bp,
        &panel_id,
        t + 0.25,
        0.8,
        Reveal::Mask,
        Direction::Down,
        Easing::OutQuint,
    ));

    let (primary_pressure, panel_to, zone_scale, zone_dy, connective) = match relationship {
        Relationship::Compress => (
            Some(Pressure {
                start: rel_start,
                end: rel_end,
                scale: 0.7,
                dy: 0.02 * h,
            }),
            Some((
                m + 70.0 * u,
                panel_top + 0.04 * h,
                w - 2.0 * m - 140.0 * u,
                panel_h * 0.62,
            )),
            (0.84, 1.16),
            -0.045 * h,
            "meanwhile,",
        ),
        // Accumulate between two plain subjects reads like growth.
        Relationship::Grow | Relationship::Accumulate => {
            (None, None, (0.8, 1.2), -0.02 * h, "and still")
        }
        Relationship::Separate => (
            Some(Pressure {
                start: rel_start,
                end: rel_end,
                scale: 1.0,
                dy: -0.03 * h,
            }),
            Some((m, panel_top - 0.03 * h, w - 2.0 * m, panel_h)),
            (1.0, 1.0),
            0.03 * h,
            "versus",
        ),
        Relationship::Replace => (
            Some(Pressure {
                start: rel_start,
                end: rel_end,
                scale: 0.55,
                dy: -0.02 * h,
            }),
            None,
            (1.0, 1.08),
            -0.1 * h,
            "instead,",
        ),
        Relationship::Carry => (None, None, (1.0, 1.04), 0.0, "still"),
    };
    if let Some(to) = panel_to {
        b.motions.push(mo::expand(
            &panel_id,
            rel_start,
            rel_end - rel_start,
            to,
            Easing::InOutCubic,
        ));
    }
    let slot_a = Slot {
        cx: w / 2.0,
        cy: panel_top + panel_h / 2.0,
        w: (w - 2.0 * m) * 0.8,
        h: panel_h * 0.7,
    };
    if let Some(mut l) = place_subject(
        ctx,
        b,
        carries,
        Which::Primary,
        &beat.primary,
        slot_a,
        (0.0, 0.0),
        ctx.palette.ink,
        t + 0.45,
        Entrance::Rise,
        primary_pressure,
        Some(panel_id.as_str()),
        "evidence",
    )? {
        l.z_index = 22;
        placed.push(Placed::of(&l));
        b.push(l);
    }

    // Connective: the serif voice names the relationship.
    let conn = ctx.ts.fit_line(Voice::SERIF, connective, w * 0.6, 78.0 * u);
    let conn_top = panel_top + panel_h + 0.022 * h;
    headline_lines(
        ctx,
        b,
        "connective",
        &conn,
        conn_top,
        TextAlign::Left,
        ctx.palette.muted,
        ev_conn,
        false,
    );

    // Zone B: collage group holding the secondary subject; the whole group
    // carries the relationship's scale/approach.
    let zb_top = conn_top + conn.height() + 0.01 * h;
    let zb_h = (h - zb_top - 0.05 * h).max(0.2 * h);
    let zone_id = b.id("zone_b");
    let mut zone_children = Vec::new();
    let card_side = (zb_h * 0.78).min(w * 0.56);
    let center = (w / 2.0, zb_h / 2.0);
    collage_card_with(
        ctx,
        b,
        &bp,
        "zone_b",
        center,
        (card_side, card_side),
        3.0,
        (1.0, 1.0),
        ev_zone,
        &mut zone_children,
    );
    if let Some(sec) = &beat.secondary {
        let slot = Slot {
            cx: w / 2.0,
            cy: zb_top + zb_h / 2.0,
            w: card_side * 0.78,
            h: card_side * 0.78,
        };
        let object_like = sec.without_value();
        let shown = if sec.kind() == SubjectKind::Object {
            &object_like
        } else {
            sec
        };
        if let Some(mut l) = place_subject(
            ctx,
            b,
            carries,
            Which::Secondary,
            shown,
            slot,
            (0.0, zb_top),
            ctx.palette.ink,
            ev_zone + 0.15,
            Entrance::Rise,
            None,
            None,
            "pressure",
        )? {
            l.z_index = 14;
            placed.push(Placed::of(&l));
            zone_children.push(l);
        }
        // An object's value becomes an accent stamp overlapping the card.
        if let (SubjectKind::Object, Some(value)) = (sec.kind(), sec.value()) {
            let stamp = ctx
                .ts
                .fit_line(Voice::HERO_NUMBER, value, w * 0.42, 0.11 * h);
            let id = b.id("stamp");
            let mut l = text_layer(id.clone(), &stamp, ctx.palette.accent, TextAlign::Left);
            // (0.23) The zone grows about its centre while the relationship
            // plays (`zone_scale`), carrying the stamp with it: keep the grown,
            // rotated stamp inside the safe area, not only the resting one
            // ("+38%" crossed the right safe edge near ANTICIPATE).
            let grow = f32::max(f32::max(zone_scale.0, zone_scale.1), 1.0);
            let right_edge = center.0 + (w / 2.0 - m) / grow;
            let tilt_slack = 0.1 * l.height;
            l.x = (center.0 + card_side * 0.5 - l.width * 0.55)
                .min(w - 1.3 * m - l.width)
                .min(right_edge - l.width - tilt_slack);
            l.y = center.1 - card_side * 0.5 - l.height * 0.45;
            l.rotation_degrees = -5.0;
            l.z_index = 30;
            // The stamp lands with a strike, except in restrained languages.
            // (0.23 C2b) `Stamp` makes it a strike in any language; the planned
            // entrance (side, wipe, easing) steers it like any other.
            let strike = match plan.lang.language {
                _ if bp.accent == AccentMark::Stamp => Easing::ImpactSpring,
                Language::Minimal | Language::Data => p.easing,
                _ => Easing::ImpactSpring,
            };
            let stamp_at = ev_zone + 0.3;
            let count = if plan.lang.data_numbers {
                mo::count(&id, stamp_at, p.duration * 2.2, value, plan.lang.settle)
            } else {
                None
            };
            // A stamp that counts up never takes a glyph cascade.
            let glyphs = if count.is_some() {
                0
            } else {
                value.chars().filter(|c| !c.is_whitespace()).count()
            };
            b.motions.extend(direction::stamp_entrance(
                &bp,
                &id,
                stamp_at,
                p.duration,
                strike,
                glyphs,
                p.travel * 0.5 * u,
            ));
            b.motions.extend(count);
            zone_children.push(l);
        }
    }
    let mut zone = base_layer(
        zone_id.clone(),
        (w / 2.0, zb_top + zb_h / 2.0, w, zb_h),
        LayerKind::Group {
            children: zone_children,
        },
        14,
    );
    zone.anchor_x = 0.5;
    zone.anchor_y = 0.5;
    if zone_scale.0 != zone_scale.1 {
        b.motions.push(mo::scale(
            &zone_id,
            rel_start,
            rel_end - rel_start,
            zone_scale.0,
            zone_scale.1,
            Easing::InOutCubic,
        ));
    }
    if zone_dy != 0.0 {
        b.motions.push(mo::shift(
            &zone_id,
            rel_start,
            rel_end - rel_start,
            [0.0, 0.0],
            [0.0, zone_dy],
            Easing::InOutCubic,
        ));
    }
    b.push(zone);

    // (0.23 C2b) The divider is a decoration: `accent` removes it, or draws it
    // as a thin line instead of the bar.
    if let (Relationship::Separate, Some(shape)) =
        (relationship, direction::accent_shape(&bp, MarkShape::Rule))
    {
        let id = b.id("divider");
        let y = conn_top - 0.012 * h;
        let thick = match shape {
            MarkShape::Rule => 6.0,
            MarkShape::Underline => 3.0,
        } * u;
        b.push(rect_layer(
            id.clone(),
            (w / 2.0, y, 0.0, thick),
            ctx.palette.accent,
            26,
        ));
        b.motions.push(mo::expand(
            &id,
            rel_start,
            rel_end - rel_start,
            (m, y, w - 2.0 * m, thick),
            Easing::InOutCubic,
        ));
    }
    direction::steer_entrances(&bp, &mut b.motions, &placed);
    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

/// explain: headline from the primary phrase, statement as body copy, an
/// optional supporting object on a collage card.
pub(super) fn explain(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let beat = b.beat;
    let p = b.plan.lang.preset;
    let t = b.plan.enter_at();
    // (0.23 C2b) The beat's planned direction; the identity is the old beat.
    let bp = ctx.params_for(b.plan.index);
    // Layers placed by `place_subject`, whose entrance is steered at the end.
    let mut placed: Vec<Placed> = Vec::new();
    ghost(ctx, b, 0.58 * h);

    let top = 0.17 * h;
    let head_text = beat
        .primary
        .display_text()
        .unwrap_or(&beat.statement)
        .to_string();
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &head_text,
        w - 2.0 * m,
        0.22 * h,
        150.0 * u,
        3,
    );
    let (head_h, head_done) = headline_lines(
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
    let body_top = top + head_h + 50.0 * u;
    // (0.10 Q) A statement too long to be a title was reduced to one (the
    // headline here is the primary's text anyway): without a voice-over the
    // full sentence is the body, kept to `BODY_MAX_LINES`; with one, the
    // captions carry it and the body is left out.
    let body = if b.plan.display.derived {
        b.plan.display.body.as_deref().map(|text| {
            ctx.ts.fit_body(
                Voice::BODY,
                text,
                46.0 * u,
                DECK_MIN_U * u,
                w - 2.0 * m,
                taste_rules::BODY_MAX_LINES,
            )
        })
    } else {
        Some(
            ctx.ts
                .set_wrapped(Voice::BODY, &beat.statement, 46.0 * u, w - 2.0 * m),
        )
    };
    let body_lines = body.as_ref().map_or(0, |bl| bl.lines.len());
    let _ = head_done;
    // ENTER: the headline. EVOLVE, one step per event: the statement (body),
    // then the supporting subject, then the accent consequence mark.
    let has_support = beat.secondary.is_some();
    let events = b.plan.life.evolve_events(if has_support { 3 } else { 1 });
    let start = events[0];
    let bh = match &body {
        Some(body) => {
            headline_lines(
                ctx,
                b,
                "body",
                body,
                body_top,
                TextAlign::Left,
                ctx.palette.ink,
                start,
                false,
            )
            .0
        }
        None => 0.0,
    };
    let anticipate = b.plan.life.anticipate;
    let rule_at = if has_support {
        events[2]
    } else if start + 0.4 < anticipate {
        start + 0.4
    } else {
        start
    };
    // (0.23 C2b) The rule under the body copy is a decoration: `accent` removes
    // it or draws it as a line as long as the copy.
    let rule_y = body_top + bh + 24.0 * u;
    let copy_w = body
        .as_ref()
        .map_or(140.0 * u, |bl| bl.width().min(w - 2.0 * m));
    accent_rule(
        ctx,
        b,
        &bp,
        "body_rule",
        MarkShape::Rule,
        (m, rule_y, 140.0 * u, 8.0 * u),
        (m, rule_y, copy_w, 4.0 * u),
        rule_at,
    );

    if let Some(sec) = &beat.secondary {
        let support_at = events[1];
        // The earlier item recedes once the next arrives (after its own
        // entrance fade, so the two fades never overlap on a layer).
        // (0.23 C2b) The body lines stagger as the beat plans (`headline_lines`).
        let lag =
            direction::stagger_offsets(&bp, stagger::offsets(b.plan.lang.stagger, body_lines))
                .into_iter()
                .fold(0.0, f64::max);
        let dim_at = support_at.max(start + lag + p.duration * 0.7 + 0.02);
        if body_lines > 0 && dim_at + 0.4 < anticipate {
            for i in 0..body_lines {
                let id = b.id(&format!("body.{i}"));
                b.motions
                    .push(mo::fade(&id, dim_at, 0.4, 1.0, 0.72, Easing::InOutCubic));
            }
        }
        let cy = body_top + bh + 0.2 * h;
        // Wide canvases: the support card must not outgrow the height left
        // below the body (legacy canvases keep 0.42 w).
        let side = if grammar::placement::is_legacy_canvas(&ctx.frame) {
            0.42 * w
        } else {
            (0.42 * w).min(2.0 * ((h - m) - cy)).max(0.25 * h)
        };
        let mut mid = Vec::new();
        collage_card_with(
            ctx,
            b,
            &bp,
            "support",
            (w / 2.0, cy),
            (side * 1.2, side * 1.2),
            -2.5,
            (1.0, -1.0),
            support_at,
            &mut mid,
        );
        for l in mid {
            b.push(l);
        }
        let slot = Slot {
            cx: w / 2.0,
            cy,
            w: side,
            h: side,
        };
        if let Some(mut l) = place_subject(
            ctx,
            b,
            carries,
            Which::Secondary,
            sec,
            slot,
            (0.0, 0.0),
            ctx.palette.ink,
            support_at + 0.2,
            Entrance::Rise,
            None,
            None,
            "support",
        )? {
            l.z_index = 22;
            placed.push(Placed::of(&l));
            b.push(l);
        }
    }
    direction::steer_entrances(&bp, &mut b.motions, &placed);
    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

#[cfg(test)]
mod short_beat_tests {
    use super::*;
    use crate::scene::{FontRole, Lifecycle, TextStyle};

    fn layer(id: &str, kind: LayerKind) -> Layer {
        Layer {
            id: id.to_string(),
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            opacity: 1.0,
            z_index: 0,
            visible: true,
            clip: None,
            depth: None,
            z: None,
            tilt: None,
            layout: None,
            kind,
        }
    }

    fn text(id: &str) -> Layer {
        layer(
            id,
            LayerKind::Text(TextStyle {
                text: "9 kg".to_string(),
                font_role: FontRole::Display,
                font_size: 48.0,
                font_weight: 700,
                italic: false,
                color: Color::rgb(20, 20, 20),
                align: TextAlign::Left,
                line_height: 1.1,
                letter_spacing: 0.0,
                max_width: None,
                uppercase: false,
                ink: None,
            }),
        )
    }

    fn fade(target: &str, start: f64, duration: f64, from: f32, to: f32) -> Motion {
        mo::fade(target, start, duration, from, to, Easing::OutCubic)
    }

    fn scene(duration: f64, layers: Vec<Layer>, motions: Vec<Motion>) -> Scene {
        Scene {
            id: "beat_1".to_string(),
            start_seconds: 0.0,
            duration_seconds: duration,
            layers,
            motions,
            camera: None,
            lifecycle: Some(Lifecycle {
                enter: 0.25,
                settle: 0.6,
                read: 0.8,
                evolve: 1.2,
                anticipate: 2.5,
                bridge: 3.0,
            }),
            post: Vec::new(),
        }
    }

    #[test]
    fn a_clamped_duration_never_ends_past_the_scene() {
        // Rounding 0.7667 to the nearest millisecond ends 0.3 ms after the scene.
        assert_eq!(clamped_duration(1.0333, 1.8), 0.766);
        assert!(1.0333 + clamped_duration(1.0333, 1.8) <= 1.8 + END_EPS);
        assert!(0.0004 + clamped_duration(0.0004, 1.0) <= 1.0 + END_EPS);
        // A duration that already fits is what it always was, also when the
        // nearest millisecond is a hair (inside the validator's tolerance) over.
        assert_eq!(clamped_duration(0.5, 1.0), 0.5);
        assert_eq!(clamped_duration(1.0000004, 2.0), 1.0);
        // Rounding down is what the nearest millisecond does here anyway.
        assert_eq!(clamped_duration(0.3336, 2.0), 1.666);
        assert_eq!(clamped_duration(0.3333, 2.0), 1.666);
    }

    #[test]
    fn a_motion_that_starts_after_the_scene_ends_is_moved_to_end_on_it() {
        let mut s = scene(
            2.0,
            vec![text("b1.late"), text("b1.busy")],
            vec![
                // Starts after the scene end: moved to end on it.
                fade("b1.late", 2.3, 0.4, 0.0, 1.0),
                // Starts after the scene end, but its layer's channel is busy
                // until 1.8: shortened to what is left.
                fade("b1.busy", 1.0, 0.8, 0.0, 0.5),
                fade("b1.busy", 2.6, 0.5, 0.5, 1.0),
                // Starts inside and runs past: shortened (as it always was).
                mo::scale("b1.late", 1.9, 0.4, 1.0, 1.1, Easing::OutCubic),
                // Valid: a zero-length motion on the scene end stays.
                mo::scale("b1.busy", 2.0, 0.0, 1.0, 1.0, Easing::Linear),
            ],
        );
        let before_valid = s.motions[1].clone();
        clamp_to_scene(&mut s);
        let m = &s.motions;
        assert_eq!((m[0].start, m[0].duration), (1.6, 0.4));
        assert_eq!(m[1], before_valid);
        assert_eq!((m[2].start, m[2].duration), (1.8, 0.2));
        assert_eq!((m[3].start, m[3].duration), (1.9, 0.1));
        assert_eq!((m[4].start, m[4].duration), (2.0, 0.0));
        for m in &s.motions {
            assert!(m.start + m.duration <= 2.0 + END_EPS, "{m:?}");
        }
    }

    #[test]
    fn a_text_that_would_appear_after_anticipate_is_brought_into_view() {
        // ANTICIPATE at 2.5: a text must be at half opacity by 2.32.
        let slab = layer(
            "b1.slab",
            LayerKind::Rectangle {
                fill: Color::rgb(200, 0, 0),
                stroke: None,
            },
        );
        let mut late = text("b1.total");
        late.layout = Some(LayoutBinding {
            parent: "b1.slab".into(),
            horizontal: HAlign::Center,
            vertical: VAlign::Center,
            offset: [0.0, 0.0],
            padding: Padding::default(),
        });
        let early = text("b1.early");
        let mut s = scene(
            3.0,
            vec![slab, late, early],
            vec![
                mo::mask("b1.slab", 2.1, 0.5, Direction::Right, Easing::OutQuint),
                fade("b1.total", 2.4, 0.4, 0.0, 1.0),
                mo::scale("b1.total", 2.4, 0.5, 1.1, 1.0, Easing::OutCubic),
                fade("b1.total", 2.7, 0.3, 1.0, 0.0),
                fade("b1.early", 0.5, 0.4, 0.0, 1.0),
            ],
        );
        let early_before = s.motions[4].clone();
        pull_late_text_into_view(&mut s);
        let entrance = &s.motions[1];
        let half = half_opacity_at(entrance).expect("a fade-in");
        assert!(
            half <= 2.5 - TEXT_IN_VIEW_BEFORE_ANTICIPATE + 1e-6,
            "{half}"
        );
        // Its pop and the slab it sits on move with it; its exit stays.
        let shift = 2.4 - entrance.start;
        assert!(shift > 0.0);
        assert!((s.motions[2].start - (2.4 - shift)).abs() < 1e-9);
        assert!((s.motions[0].start - (2.1 - shift)).abs() < 1e-9);
        assert_eq!(s.motions[3].start, 2.7);
        assert_eq!(s.motions[4], early_before);
    }

    #[test]
    fn a_late_text_never_moves_before_enter() {
        let mut s = scene(
            3.0,
            vec![text("b1.total")],
            vec![fade("b1.total", 0.3, 5.0, 0.0, 1.0)],
        );
        pull_late_text_into_view(&mut s);
        assert!(s.motions[0].start >= 0.25 - 1e-9);
    }
}

#[cfg(test)]
mod beat_params_tests {
    //! (0.23 W2c) The shared skeleton honours planned BeatParams: identity
    //! params build the exact old scene; planned ones steer the kicker
    //! entrance and rule, the stage exit and the camera.
    use std::collections::BTreeMap;

    use super::*;
    use crate::compiler::direction::{BeatParams, EntranceFamily};
    use crate::compiler::{plan_timing, ApproxMeasure, AssetLibrary, FontSet, Palette};
    use crate::intent::CreativeIntent;
    use crate::style::StyleProfile;

    const INTENT: &str = r#"{"version":"0.2","title":"t","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"One idea","primary":{"kind":"phrase","value":"one"},"energy":"calm"},
        {"purpose":"compare","statement":"Cost against output","primary":{"kind":"number","value":"40%","meaning":"cost"},
         "secondary":{"kind":"number","value":"3x","meaning":"output"},"relationship":"separate","energy":"building"},
        {"purpose":"emphasize","statement":"The end","primary":{"kind":"phrase","value":"end"},"energy":"calm"}]}"#;

    fn scene_with(params: Option<BeatParams>) -> Scene {
        let style: StyleProfile = serde_json::from_str(r#"{"tone":"editorial"}"#).expect("style");
        let intent: CreativeIntent = serde_json::from_str(INTENT).expect("intent");
        let plans = plan_timing(&intent, &style);
        let library = AssetLibrary::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let fonts = FontSet::for_style(&style);
        let ts = Typesetter::new(&ApproxMeasure, &library.root, &fonts);
        let manifest = crate::assets::AssetManifest::empty();
        let mut ctx = Ctx {
            reveals: Default::default(),
            warnings: Vec::new(),
            direction_seed: None,
            beat_params: params
                .map(|p| BTreeMap::from([(1usize, p)]))
                .unwrap_or_default(),
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
            frame: LayoutFrame::for_format(Format::Vertical),
            manifest: &manifest,
            image_carry: Default::default(),
            focal: Default::default(),
            stack: None,
            spoken: Default::default(),
            beat_count: 3,
            sequence: crate::compiler::sequence::Sequence {
                order: None,
                ranks: vec![None, None, None],
            },
        };
        let mut carries = Vec::new();
        build_beat(&mut ctx, &plans[1], &intent.beats[1], &mut carries, None).expect("beat")
    }

    fn json(s: &Scene) -> String {
        serde_json::to_string(s).expect("json")
    }

    #[test]
    fn identity_params_build_the_old_scene() {
        assert_eq!(
            json(&scene_with(None)),
            json(&scene_with(Some(BeatParams::default())))
        );
    }

    #[test]
    fn planned_params_steer_the_shared_skeleton() {
        let base = scene_with(None);
        let p = BeatParams {
            entrance: EntranceFamily::WipeLeft,
            accent: AccentMark::Underline,
            exit: ExitFamily::Cut,
            camera: CameraMove::Still,
            ..BeatParams::default()
        };
        let s = scene_with(Some(p));
        assert_ne!(json(&base), json(&s));
        // The kicker wipes in from the planned side.
        let kicker = s
            .motions
            .iter()
            .find(|m| m.target.ends_with(".kicker"))
            .expect("kicker motion");
        assert!(
            matches!(
                kicker.op,
                MotionOp::MaskReveal {
                    direction: Direction::Left,
                    ..
                }
            ),
            "{:?}",
            kicker.op
        );
        // The stage leaves on a hard cut.
        assert!(s.motions.iter().any(|m| m.target.ends_with(".stage")
            && (m.duration - 0.066).abs() < 1e-9
            && matches!(m.op, MotionOp::Fade { .. })));
        // A still camera.
        assert!(s.camera.is_none());
    }
}
