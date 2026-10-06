//! (0.14) DocumentaryDossier — evidence lands on a desk: documents, a photo, a
//! stamp, a highlighter.
//!
//! Cards slide in on a spring, staggered 0.18 s: the clipping (a paper card
//! with the display title in the serif headline and seeded redaction bars), the
//! photo (the beat's picture on a taped, tilted card) and, when the primary
//! value is a figure, the figure card (the value large, its meaning as a
//! label, a red hand-drawn underline that draws at EVOLVE). At EVOLVE a
//! highlighter grows left to right behind the keyword's words in the headline
//! and the keyword is stamped onto the dossier (scale 2.4 → 1). The cards carry
//! `z` 0 / 80 / 160 and `tilt` [6, -4] for the perspective camera that the FX
//! director adds to this look. A delivered environment image is the desk
//! (full bleed, 35 % opacity, background plane). Spec: docs/GENRE_GRAMMARS.md.
//! Finishing effects (grain, vignette, shake) come from the FX director.
//!
//! Notes beyond the spec table: the figure card also carries the secondary
//! number of a comparison (a second entry), so it is not lost; a paper card
//! hugs an opaque photo or a cutout with measured subject bounds, while a
//! cutout of unknown extent (no analysis in the manifest) is taped to the desk
//! bare, because a card cannot hug its transparent margins (layout QA
//! `frame_too_loose`). `z` / `tilt` are read by the perspective camera on
//! top-level scene layers only; builder layers live in the beat's stage group,
//! so today they are inert (see the report of this work package).
//!
//! (0.22) Every value and picture shows. An object's value is a figure-card
//! entry (photo + figure card = a stat card). A `compare` / `contrast` of two
//! subjects that are pictures or figures (at least one picture) is a pair
//! ([`build_pair`]): the clipping across the top (focal, stamped), then two
//! co-equal evidence documents (photo card + figure card each, the meaning as
//! the figure's label), side by side on square and wide canvases, stacked on
//! tall ones; each document arrives when its subject is named and its figure
//! when it is said. Anchors `photo.{i}` (Content) and `figure.{i}` (Value).
//! With a voice-over a keyword is stamped only when the narrator says it
//! ([`stamp_allowed`]; the other genre looks use the same rule).

use super::placement::{self, Rect, SubjectFacts};
use super::{plate, Composition};
use crate::assets::{AssetRole, ManifestEntry};
use crate::compiler::explore::contrast_ratio;
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::typeset::{text_layer, Block, Voice};
use crate::compiler::{base_layer, mix, mo, subject_key, Carry, CompileError, Ctx, Which};
use crate::easing::Easing;
use crate::intent::{Purpose, Subject, SubjectKind};
use crate::scene::{
    Color, Direction, Layer, LayerKind, Motion, MotionOp, SpringSpec, Stroke, TextAlign,
};

/// Cards slide in on this spring.
const SLIDE: SpringSpec = SpringSpec {
    stiffness: 260.0,
    damping: 24.0,
    mass: 1.0,
};
const SLIDE_TIME: f64 = 0.9;
/// Delay between consecutive cards.
const STAGGER: f64 = 0.18;
/// The stamp slams from this scale in this time.
const STAMP_FROM: f32 = 2.4;
const STAMP_TIME: f64 = 0.18;
/// Highlighter opacity.
const HIGHLIGHT_OPACITY: f32 = 0.55;
/// Perspective: card depth (px) and tilt (degrees about x, y).
/// (0.18) Slot 0 is the focal card (on the focus plane), the others sit behind.
const CARD_Z: [f32; 3] = [0.0, 70.0, 140.0];
const CARD_TILT: [f32; 2] = [6.0, -4.0];
/// The figure's underline is a plain editorial red on any palette, or (0.21)
/// the brand's mark colour when the piece has a brand.
const RED: Color = Color::rgb(0xD6, 0x2B, 0x1E);

/// Seeded value in `[0, 1)`.
fn frac(seed: u64, salt: u64) -> f32 {
    (mix(seed, salt) % 10_000) as f32 / 10_000.0
}

fn sprung(mut m: Motion, s: SpringSpec) -> Motion {
    m.spring = Some(s);
    m
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn capitalised(s: &str) -> String {
    let mut c = s.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Half extents of the axis-aligned box of a `w x h` card turned by `deg`.
fn rotated_half(w: f32, h: f32, deg: f32) -> (f32, f32) {
    let (s, c) = deg.to_radians().sin_cos();
    (
        (w * c.abs() + h * s.abs()) / 2.0,
        (w * s.abs() + h * c.abs()) / 2.0,
    )
}

/// (0.18) Which card the beat is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focal {
    Figure,
    Photo,
    Clip,
}

/// A card's place on the canvas.
#[derive(Debug, Clone, Copy)]
struct Place {
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    rot: f32,
}

impl Place {
    fn left(&self) -> f32 {
        self.cx - self.w / 2.0
    }
    fn top(&self) -> f32 {
        self.cy - self.h / 2.0
    }
    fn bottom(&self) -> f32 {
        self.cy + self.h / 2.0
    }
    /// Canvas point of a card-local point (rotated about the card centre).
    fn canvas_point(&self, lx: f32, ly: f32) -> (f32, f32) {
        let (s, c) = self.rot.to_radians().sin_cos();
        let (dx, dy) = (lx - self.w / 2.0, ly - self.h / 2.0);
        (self.cx + dx * c - dy * s, self.cy + dx * s + dy * c)
    }
}

/// A card group: soft shadow, paper, then `kids` (card-local coordinates).
#[allow(clippy::too_many_arguments)]
fn card(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    at: Place,
    z_index: i32,
    depth_slot: usize,
    paper: bool,
    mut kids: Vec<Layer>,
) -> Layer {
    let u = ctx.u;
    let mut layers: Vec<Layer> = Vec::new();
    let shadows: &[(f32, f32, u8)] = if paper {
        &[(26.0, 22.0, 0x10), (14.0, 12.0, 0x16), (6.0, 6.0, 0x22)]
    } else {
        &[]
    };
    for (i, &(grow, drop, alpha)) in shadows.iter().enumerate() {
        let (g, d) = (grow * u, drop * u);
        let mut s = base_layer(
            b.id(&format!("{name}.shadow.{i}")),
            (-g, -g + d, at.w + 2.0 * g, at.h + 2.0 * g),
            LayerKind::RoundedRectangle {
                fill: Color::rgba(0, 0, 0, alpha),
                radius: 10.0 * u + g,
                stroke: None,
            },
            i as i32,
        );
        s.z_index = i as i32;
        layers.push(s);
    }
    if paper {
        layers.push(base_layer(
            b.id(&format!("{name}.paper")),
            (0.0, 0.0, at.w, at.h),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.card,
                radius: 6.0 * u,
                stroke: Some(Stroke {
                    color: ctx.palette.ink.with_alpha(0x40),
                    width: 2.0 * u,
                }),
            },
            3,
        ));
    }
    layers.append(&mut kids);
    let mut g = base_layer(
        b.id(name),
        (at.cx, at.cy, at.w, at.h),
        LayerKind::Group { children: layers },
        z_index,
    );
    g.anchor_x = 0.5;
    g.anchor_y = 0.5;
    g.rotation_degrees = at.rot;
    g.z = Some(CARD_Z[depth_slot.min(2)]);
    g.tilt = Some(CARD_TILT);
    g
}

/// Slide a card in: from `from` (px offset) to rest on the slide spring, a
/// short fade and a settling turn, starting at `start`.
fn slide_in(b: &mut B, id: &str, start: f64, from: [f32; 2], turn: f32) {
    b.motions
        .push(mo::fade(id, start, 0.25, 0.0, 1.0, Easing::OutCubic));
    b.motions.push(sprung(
        mo::shift(id, start, SLIDE_TIME, from, [0.0, 0.0], Easing::OutQuint),
        SLIDE,
    ));
    b.motions.push(sprung(
        mo::rotate(id, start, SLIDE_TIME, turn, 0.0, Easing::OutQuint),
        SLIDE,
    ));
}

/// A figure's underline draws for this long.
const UNDERLINE_TIME: f64 = 0.55;

/// EVOLVE of a dossier beat: the lifecycle's, but never before the last card
/// has landed (`t + 3 * STAGGER + 0.9`).
///
/// (0.23 A4-short) A continuous take leaves a beat 1.2-2 s, shorter than that
/// landing: EVOLVE (the highlighter, the underlines, the stamp) would then come
/// after ANTICIPATE and the last of `underlines` figure underlines would start
/// after the scene end ("motion ends after scene"). On the product path EVOLVE
/// is kept early enough that the last underline (with none, the highlighter)
/// has drawn by ANTICIPATE (never before `t + 0.3`, so the cards are on their
/// way); without `--variety` it is only moved when that last step would start
/// at or after the scene end, which does not validate, so every valid beat
/// keeps its timing.
fn evolve_at(ctx: &Ctx, b: &B, t: f64, underlines: usize) -> f64 {
    let life = b.plan.life;
    let want = life.evolve.max(t + 3.0 * STAGGER + 0.9);
    // The last underline starts `tail` after EVOLVE (`evolve + 0.15 + 0.3 i`)
    // and draws for `span`; without one the 0.5 s highlighter starts with it.
    let (tail, span) = if underlines > 0 {
        (0.15 + 0.3 * (underlines - 1) as f64, UNDERLINE_TIME)
    } else {
        (0.0, 0.5)
    };
    let latest = (life.anticipate - tail - span).max(t + 0.3);
    let past_the_end = want + tail >= b.plan.duration;
    if (ctx.direction_seed.is_some() && want > latest) || past_the_end {
        want.min(latest)
    } else {
        want
    }
}

/// (0.23) A library picture on the desk: the ink print border of
/// [`print_border`], and on the product path (a direction seed) a faded print
/// ([`faded_print`]) so that border separates it whatever its colour.
fn library_print(ctx: &Ctx, l: &mut Layer) {
    print_border(l, ctx.palette.ink, ctx.u);
    if ctx.direction_seed.is_some() {
        faded_print(l, ctx.palette.paper, ctx.palette.ink);
    }
}

/// (0.23) A library picture's colour is unknown at compile time (no analysis)
/// and it wears the ink print border of [`print_border`]; layout QA
/// (`asset_low_contrast`) wants its mean colour at least 2:1 from the border's,
/// and a dark picture (a server rack, #323533) fails that: its border merges
/// into it. The picture is shown as a faded print instead: mixed into the paper
/// just far enough that even a black and a white picture stay 2:1 (with a
/// margin) from the border colour. A light picture hardly changes.
fn faded_print(l: &mut Layer, paper: Color, ink: Color) {
    let LayerKind::Image { treatment, .. } = &mut l.kind else {
        return;
    };
    let blend = |c: Color, t: f32| {
        let ch = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Color::rgb(ch(c.r, paper.r), ch(c.g, paper.g), ch(c.b, paper.b))
    };
    let apart = |a: f32| {
        [Color::rgb(0, 0, 0), Color::rgb(255, 255, 255)]
            .into_iter()
            .all(|extreme| contrast_ratio(blend(extreme, a), ink) >= FADED_PRINT_CONTRAST)
    };
    let Some(amount) = (0..=30).map(|k| k as f32 * 0.02).find(|&a| apart(a)) else {
        return;
    };
    if amount <= 0.0 {
        return;
    }
    if let Some(tr) = treatment {
        tr.tint = Some(crate::scene::TintSpec {
            color: paper,
            amount,
        });
    }
}

/// (0.23) The contrast a faded print keeps from its border: the layout QA's
/// 2:1 (`checks::ASSET_GROUND_MIN_CONTRAST`) with a margin for the mean colour
/// the estimate works from.
const FADED_PRINT_CONTRAST: f32 = 2.1;

/// (0.23) A delivered library picture taped to the desk without a paper card
/// ([`photo_geo`]) has no measured colour at compile time: it wears the print
/// of [`library_print`] (product path only).
fn bare_library_print(ctx: &Ctx, pic: &Picture, layers: &mut [Layer]) {
    let Picture::Delivered(entry, _) = pic else {
        return;
    };
    if ctx.direction_seed.is_none()
        || !entry.path.starts_with("library/")
        || entry
            .analysis
            .as_ref()
            .is_some_and(|a| a.mean_color.is_some())
    {
        return;
    }
    for l in layers.iter_mut() {
        library_print(ctx, l);
    }
}

/// (0.23) A library picture that was carried across beats is a shared element
/// (`recipes::place_subject` returns no layer for it): on the product path it
/// wears the same print as the pictures the beat places itself, or it sits on
/// the desk without a border (`asset_low_contrast`, a yellow bag on cream).
fn print_carried(ctx: &Ctx, carries: &mut [Carry], subject: &Subject, beat: usize) {
    if ctx.direction_seed.is_none() {
        return;
    }
    let key = subject_key(subject);
    for c in carries
        .iter_mut()
        .filter(|c| c.key == key && c.last_beat == beat)
    {
        library_print(ctx, &mut c.element.layer);
    }
}

/// (0.23) Whether the delivered library picture `path` becomes a looping
/// sprite once the beats are built: with art direction, `loops::animate_library_assets`
/// swaps every library still that has a hero loop for that sprite, after the
/// layout. The sprite's frames are cut differently from the still (a piggy
/// bank: x 0.10..0.92, y 0.18..0.94 against the still's 0.06..0.94 and
/// 0.05..0.95), so a card hugging the still's subject bounds hangs 24 % loose
/// around the sprite (`frame_too_loose`).
fn becomes_a_sprite(ctx: &Ctx, path: &str) -> bool {
    if ctx.art.is_none() {
        return false;
    }
    let Some((family, file)) = path
        .strip_prefix("library/")
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    let Some(stem) = file.strip_suffix(".png").filter(|s| !s.contains('/')) else {
        return false;
    };
    if !ctx.library.families.iter().any(|f| f == family) {
        return false;
    }
    let dir = ctx.library.root.join("library").join(family);
    let Some(catalog) = std::fs::read_to_string(dir.join("loops/catalog.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        return false;
    };
    catalog
        .get("loops")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|loops| {
            loops.iter().any(|l| {
                l.get("role").and_then(serde_json::Value::as_str) == Some("hero_loop")
                    && l.get("host").and_then(serde_json::Value::as_str) == Some(stem)
                    && l.get("path")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|p| dir.join(p).is_dir())
            })
        })
}

/// A print border in the ink colour on a library picture: with no analysis
/// its colour is unknown, so it reads against any desk this way (layout QA
/// `asset_low_contrast`, e.g. a dark flag or cart on a dark brand ground).
pub(super) fn print_border(l: &mut Layer, ink: Color, u: f32) {
    if let LayerKind::Image { treatment, .. } = &mut l.kind {
        let mut tr = treatment
            .take()
            .unwrap_or_else(crate::scene::ImageTreatment::natural);
        tr.sticker = Some(crate::scene::Sticker {
            color: ink,
            width_px: crate::compiler::subject_rules::STICKER_WIDTH_U * u,
        });
        tr.edge = None;
        tr.shadow = None;
        *treatment = Some(tr);
    }
}

/// The beat's picture: a delivered image, else the object subject.
enum Picture<'a> {
    Delivered(&'a ManifestEntry, AssetRole),
    Object(Which, Subject),
}

fn find_picture<'a>(ctx: &Ctx<'a>, b: &B) -> Option<Picture<'a>> {
    let index = b.plan.index;
    let delivered = [
        AssetRole::HeroSubject,
        AssetRole::Portrait,
        AssetRole::HeroObject,
        AssetRole::EvidenceImage,
    ]
    .into_iter()
    .find_map(|role| plate::image(ctx, index, role).map(|e| (e, role)));
    if let Some((entry, role)) = delivered {
        return Some(Picture::Delivered(entry, role));
    }
    let beat = b.beat;
    if beat.primary.kind() == SubjectKind::Object {
        return Some(Picture::Object(Which::Primary, beat.primary.clone()));
    }
    beat.secondary
        .as_ref()
        .filter(|s| s.kind() == SubjectKind::Object)
        .map(|s| Picture::Object(Which::Secondary, s.clone()))
}

/// A photo card's size: the picture's box and the paper around it.
struct Photo {
    w: f32,
    h: f32,
    inner: Rect,
    facts: Option<SubjectFacts>,
    /// A paper card behind the picture (see [`photo_geo`]).
    paper: bool,
}

/// The photo card that hugs `pic` inside `max_w x max_h` (subject box plus a
/// margin).
fn photo_geo(ctx: &Ctx, pic: &Picture, max_w: f32, max_h: f32) -> Photo {
    // A paper card hugs an opaque photo, or a cutout whose subject bounds
    // are known (analysis); a cutout of unknown extent has transparent
    // margins the card cannot hug (layout QA `frame_too_loose`), so it is
    // taped to the desk bare. (0.23) So is a library picture that becomes a
    // sprite after the layout ([`becomes_a_sprite`]) on the product path.
    let paper = match pic {
        Picture::Delivered(entry, _) => {
            (!entry.alpha || entry.analysis.is_some())
                && !(ctx.direction_seed.is_some() && becomes_a_sprite(ctx, &entry.path))
        }
        Picture::Object(..) => false,
    };
    // Polaroid pads proportional to the picture (frames hug the asset:
    // every side stays under taste_rules::FRAME_PAD_MAX).
    let (side_k, top_k, bottom_k) = if paper {
        (0.07, 0.07, 0.1)
    } else {
        (0.0, 0.0, 0.0)
    };
    let facts = match pic {
        Picture::Delivered(entry, _) => Some(SubjectFacts::from_entry(entry)),
        Picture::Object(..) => None,
    };
    // Subject aspect (w / h) of what the picture shows.
    let aspect = facts.as_ref().map_or(1.0, |f| {
        f.subject.width * f.aspect / f.subject.height.max(0.05)
    });
    let (iw_max, ih_max) = (
        max_w / (1.0 + 2.0 * side_k),
        max_h / (1.0 + top_k + bottom_k),
    );
    let iw = iw_max.min(ih_max * aspect);
    let ih = iw / aspect;
    let (pad_x, pad_top, pad_bottom) = (side_k * iw, top_k * ih, bottom_k * ih);
    Photo {
        w: iw + 2.0 * pad_x,
        h: ih + pad_top + pad_bottom,
        inner: Rect::new(pad_x, pad_top, iw, ih),
        facts,
        paper,
    }
}

/// A written value that is a figure: it has a digit or a currency sign.
fn figure_text(v: &str) -> Option<String> {
    let v = v.trim();
    let figure = v.chars().any(|c| c.is_ascii_digit() || "$€£¥₹".contains(c));
    figure.then(|| v.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// (0.22) An object's value as shown: a figure, or a short word or two (1–3
/// words). The contract calls it "a short figure stamped next to the object".
fn object_value(s: &Subject) -> Option<String> {
    let v = s.value().filter(|_| s.kind() == SubjectKind::Object)?;
    let n = v.split_whitespace().count();
    figure_text(v).or_else(|| (1..=3).contains(&n).then(|| figure_text_words(v)))
}

/// Whitespace-normalised text.
fn figure_text_words(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The figure card's entries `(value, meaning)`: the primary value when it is
/// a figure, then the secondary number of a comparison (so it is not lost).
/// (0.18) A short primary phrase (1–3 words, the beat's verdict) gets the card
/// too: the primary is what the beat is about, so it is never left off.
/// (0.22) An object's value is an entry too (picture + figure card = a stat
/// card), and a secondary value shows even when the primary has none.
fn figure_values(b: &B) -> Vec<(String, Option<String>)> {
    let entry = |s: &Subject| {
        let v = match s.kind() {
            SubjectKind::Phrase | SubjectKind::Number => s.value().and_then(figure_text),
            SubjectKind::Object => object_value(s),
            _ => None,
        }?;
        Some((v, s.meaning().map(str::to_string)))
    };
    let verdict = |s: &Subject| {
        let v = s.value().filter(|_| s.kind() == SubjectKind::Phrase)?;
        let n = v.split_whitespace().count();
        (1..=3)
            .contains(&n)
            .then(|| (figure_text_words(v), s.meaning().map(str::to_string)))
    };
    let first = entry(&b.beat.primary).or_else(|| verdict(&b.beat.primary));
    let second = b.beat.secondary.as_ref().and_then(entry);
    first.into_iter().chain(second).collect()
}

/// (0.22, owner rule) With a voice-over a word is stamped only when the
/// narrator says it in this beat (`speech_plan::anchor_time`: stemmed, numbers
/// by value); without one, always. A stamp of a word nobody says lands on
/// nothing. The faint background keyword is not a stamp and always stays.
pub(super) fn stamp_allowed(ctx: &Ctx, word: &str) -> bool {
    ctx.spoken.is_empty()
        || crate::compiler::speech_plan::anchor_time(&ctx.spoken, &[word]).is_some()
}

/// Seeded wobble of the underline's points across `w x h` (box space).
fn underline_points(seed: u64, w: f32, h: f32) -> Vec<[f32; 2]> {
    (0..6)
        .map(|i| {
            let x = w * i as f32 / 5.0;
            let wobble = (frac(seed, 0x700 + i as u64) - 0.5) * 0.7 * h;
            [x, h / 2.0 + wobble]
        })
        .collect()
}

/// `(line, byte start, byte end)` of the keyword's words in the title block.
fn keyword_span(block: &Block, keyword: &str) -> Option<(usize, usize, usize)> {
    let want: Vec<String> = keyword
        .split_whitespace()
        .map(norm)
        .filter(|k| !k.is_empty())
        .collect();
    if want.is_empty() {
        return None;
    }
    for (li, line) in block.lines.iter().enumerate() {
        let mut words: Vec<(usize, usize, String)> = Vec::new();
        let mut start = None;
        for (i, c) in line.char_indices() {
            match (c.is_whitespace(), start) {
                (false, None) => start = Some(i),
                (true, Some(s)) => {
                    words.push((s, i, norm(&line[s..i])));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            words.push((s, line.len(), norm(&line[s..])));
        }
        if words.len() < want.len() {
            continue;
        }
        for i in 0..=words.len() - want.len() {
            if words[i..i + want.len()]
                .iter()
                .zip(&want)
                .all(|(w, k)| w.2 == *k)
            {
                return Some((li, words[i].0, words[i + want.len() - 1].1));
            }
        }
    }
    None
}

/// What the clipping card holds (card-local geometry).
struct ClipText<'a> {
    block: &'a Block,
    keyword: Option<&'a str>,
    pad: f32,
    text_w: f32,
    bar_h: f32,
    bar_gap: f32,
    bars_top: f32,
    bars: usize,
}

/// The clipping's layers (card-local): a highlighter behind the keyword's
/// words (else the first line) that grows at `evolve`, the serif title, and
/// the seeded redaction bars wiped on after the card lands (`t`).
fn clip_kids(ctx: &Ctx, b: &mut B, c: &ClipText, evolve: f64, t: f64, seed: u64) -> Vec<Layer> {
    let (title_block, pad) = (c.block, c.pad);
    let mut kids: Vec<Layer> = Vec::new();
    let title_id = b.id("clip.title");
    let mut title_layer = text_layer(
        title_id.clone(),
        title_block,
        ctx.palette.ink,
        TextAlign::Left,
    );
    title_layer.x = pad;
    title_layer.y = pad;
    title_layer.z_index = 5;
    // Highlighter: behind the keyword's words (else the first line).
    let (li, from_b, to_b) = c
        .keyword
        .and_then(|k| keyword_span(title_block, k))
        .unwrap_or_else(|| (0, 0, title_block.lines.first().map_or(0, String::len)));
    let line = title_block.lines.get(li).cloned().unwrap_or_default();
    let size = title_block.size;
    let before = ctx
        .ts
        .width(&Voice::SERIF, &line[..from_b.min(line.len())], size);
    let upto = ctx
        .ts
        .width(&Voice::SERIF, &line[..to_b.min(line.len())], size);
    let adv = title_block.line_advance();
    let hl_pad = 0.05 * size;
    let hl_id = b.id("clip.highlight");
    let mut hl = base_layer(
        hl_id.clone(),
        (
            pad + before - hl_pad,
            pad + li as f32 * adv + 0.14 * adv,
            (upto - before) + 2.0 * hl_pad,
            0.76 * adv,
        ),
        LayerKind::Rectangle {
            fill: ctx.palette.accent,
            stroke: None,
        },
        4,
    );
    hl.opacity = HIGHLIGHT_OPACITY;
    b.motions.push(mo::mask(
        &hl_id,
        evolve,
        0.5,
        Direction::Right,
        Easing::OutCubic,
    ));
    kids.push(hl);
    kids.push(title_layer);
    // Redaction bars: seeded widths, wiped on after the card lands.
    for i in 0..c.bars {
        let last = i + 1 == c.bars;
        let share = if last {
            0.35 + 0.3 * frac(seed, 0xE0 + i as u64)
        } else {
            0.72 + 0.28 * frac(seed, 0xE0 + i as u64)
        };
        let id = b.id(&format!("clip.bar.{i}"));
        let mut bar = base_layer(
            id.clone(),
            (
                pad,
                c.bars_top + i as f32 * (c.bar_h + c.bar_gap),
                c.text_w * share,
                c.bar_h,
            ),
            LayerKind::Rectangle {
                fill: ctx.palette.ink,
                stroke: None,
            },
            6,
        );
        bar.opacity = 0.92;
        b.motions.push(mo::mask(
            &id,
            t + 0.5 + 0.1 * i as f64,
            0.35,
            Direction::Right,
            Easing::OutCubic,
        ));
        kids.push(bar);
    }
    kids
}

/// Wrapped title, refit until its widest line fits `max_w` as a text layer.
fn fit_title(ctx: &Ctx, text: &str, max_w: f32, max_h: f32) -> Block {
    let mut size = 200.0 * ctx.u;
    let mut block = ctx.ts.fit_block(Voice::SERIF, text, max_w, max_h, size, 4);
    for _ in 0..6 {
        let layer_w = block.width() * 1.02 + 2.0;
        if layer_w <= max_w + 0.5 || block.size < 24.0 * ctx.u {
            break;
        }
        size = block.size * (max_w / layer_w) * 0.97;
        block = ctx.ts.fit_block(Voice::SERIF, text, max_w, max_h, size, 4);
    }
    block
}

/// A single line fitted to `max_w` as a text layer.
fn fit_line(ctx: &Ctx, voice: Voice, text: &str, max_w: f32, max_size: f32) -> Block {
    let mut block = ctx.ts.fit_line(voice, text, max_w, max_size);
    for _ in 0..4 {
        let layer_w = block.width() * 1.02 + 2.0;
        if layer_w <= max_w + 0.5 {
            break;
        }
        block = ctx
            .ts
            .fit_line(voice, text, max_w * (max_w / layer_w) * 0.99, max_size);
    }
    block
}

pub(crate) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let life = plan.life;
    let t = plan.enter_at();
    let seed = plan.seed;
    let land = placement::side_by_side(&ctx.frame);
    // Square and wider canvases lay the cards in two columns instead of a stack.
    let cols = land || ctx.frame.aspect >= 0.95;
    let by_class = |wide: f32, square: f32, tall: f32| {
        if land {
            wide
        } else if cols {
            square
        } else {
            tall
        }
    };

    // 1. The desk: the delivered environment image, dimmed, full bleed.
    if let Some(env) = plate::image(ctx, plan.index, AssetRole::Environment) {
        let (iw, ih) = (env.width.max(1) as f32, env.height.max(1) as f32);
        let s = (w * 1.04 / iw).max(h * 1.04 / ih);
        let (bw, bh) = (iw * s, ih * s);
        let rect = ((w - bw) / 2.0, (h - bh) / 2.0, bw, bh);
        let desk = plate::image_plate(ctx, b, "env", AssetRole::Environment, rect, 1, t);
        let id = desk.id.clone();
        b.motions
            .retain(|mo| !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. })));
        b.motions
            .push(mo::fade(&id, t, 0.6, 0.0, 1.0, Easing::OutCubic));
        // A slow push-in across the beat keeps the desk alive.
        b.motions.push(mo::scale(
            &id,
            t,
            plan.duration.max(0.5),
            1.0,
            1.05,
            Easing::Linear,
        ));
        for mut l in desk.layers {
            l.opacity = 0.35;
            l.depth = Some(0.2);
            b.push(l);
        }
    }

    // (0.22) Two subjects compared: two evidence documents, side by side.
    if let Some(sides) = pair_sides(ctx, b) {
        return build_pair(ctx, b, carries, sides);
    }

    let picture = find_picture(ctx, b);
    let figures = figure_values(b);
    let title = capitalised(&b.beat.statement);
    let keyword = b
        .beat
        .keyword
        .clone()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| b.beat.primary.meaning().map(str::to_string))
        .filter(|k| !k.trim().is_empty());
    // (0.20) Reveal anchors: the figure card lands on its amount, the
    // clipping's title waits for its key word (Hold), the photo arrives when
    // the pictured subject is named.
    // (0.23 W4) A beat whose cards all wait for their words (holding look,
    // voice-over, product path: the title and the figure or picture are said
    // late) shows nothing but its kicker until then. Its cards land at ENTER
    // with their content hidden instead: the card (paper, label, bars) is the
    // placeholder, only the value, the picture or the title text waits.
    let values: Vec<String> = figures.iter().map(|(v, _)| v.clone()).collect();
    let title_words = crate::compiler::recipes::title_key_words(b.beat, &b.beat.statement);
    let pictured: Vec<String> = if picture.is_some() {
        [&b.beat.primary]
            .into_iter()
            .chain(b.beat.secondary.as_ref())
            .find(|s| s.kind() == SubjectKind::Object)
            .map(crate::compiler::recipes::subject_words)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let (held_figure, held_clip, held_photo) = {
        use crate::compiler::speech_lifecycle::hero_waits;
        // Only when every card of the beat waits: a clipping whose title
        // reads from ENTER (a topic title) is on screen already, and a figure
        // that lands on its number then lands whole, as it always did.
        let all_wait = hero_waits(ctx, &title_words)
            && (values.is_empty() || hero_waits(ctx, &values))
            && (picture.is_none() || hero_waits(ctx, &pictured));
        (
            all_wait && !values.is_empty(),
            all_wait,
            all_wait && picture.is_some(),
        )
    };
    {
        use crate::speech::RevealRole;
        if held_figure {
            for (i, v) in values.iter().enumerate() {
                let group = if i == 0 {
                    "figure.value".to_string()
                } else {
                    format!("figure.value{}", i + 1)
                };
                // A value the narrator never says as such ("five" for "5
                // hours") comes with the earliest of the card's values, as
                // the whole card did.
                let said = crate::compiler::speech_plan::anchor_time(&ctx.spoken, &[v.as_str()]);
                let words = if said.is_some() {
                    vec![v.clone()]
                } else {
                    values.clone()
                };
                b.reveal(&group, words, RevealRole::Value);
            }
        } else {
            b.reveal("figure", values.iter().cloned(), RevealRole::Value);
        }
        b.reveal(
            if held_clip { "clip.title" } else { "clip" },
            title_words,
            RevealRole::Title,
        );
        if picture.is_some() {
            b.reveal(
                if held_photo { "photo.pic" } else { "photo" },
                pictured,
                RevealRole::Content,
            );
        }
    }

    // Vertical budget: below the kicker, above the caption lane / bottom edge.
    let top = 0.165 * h;
    let bottom = h - 0.8 * m;
    let avail = bottom - top;
    let edge_x = 0.6 * m;
    let n_cards = 1 + usize::from(picture.is_some()) + usize::from(!figures.is_empty());
    // Height shares of the available space: (clip, photo, figure). (0.18)
    // The figure is what the beat is about: it gets the most room.
    let (sc, sp, sf) = match (picture.is_some(), !figures.is_empty()) {
        (true, true) if figures.len() > 1 => (0.22, 0.32, 0.42),
        (true, true) => (0.24, 0.36, 0.36),
        (true, false) => (0.40, 0.55, 0.0),
        (false, true) if figures.len() > 1 => (0.32, 0.0, 0.56),
        (false, true) => (0.36, 0.0, 0.48),
        (false, false) => (0.62, 0.0, 0.0),
    };
    // (0.18) The focal card: on the focus plane, on top, carrying the stamp.
    let focal = if !figures.is_empty() {
        Focal::Figure
    } else if picture.is_some() {
        Focal::Photo
    } else {
        Focal::Clip
    };
    // The stamp names the keyword; when the focal card already says it, the
    // stamp names the primary's meaning instead and the card drops that label.
    let words_of = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(norm)
            .filter(|w| !w.is_empty())
            .collect()
    };
    let repeats = |a: &str, b: &str| {
        let (wa, wb) = (words_of(a), words_of(b));
        !wa.is_empty() && wa.iter().all(|x| wb.contains(x))
    };
    let focal_value = figures.first().map(|(v, _)| v.clone());
    let mut drop_label = false;
    let mut stamp_text = keyword
        .clone()
        .map(|k| match (&focal_value, figures.first()) {
            (Some(v), Some((_, Some(meaning)))) if repeats(&k, v) && !repeats(meaning, v) => {
                drop_label = true;
                meaning.clone()
            }
            _ => k,
        });
    // (0.22) Only a word the narrator says is stamped (with a voice-over);
    // the card then keeps its label.
    if stamp_text
        .as_deref()
        .is_some_and(|s| !stamp_allowed(ctx, s))
    {
        stamp_text = None;
        drop_label = false;
    }
    // (0.20) The stamp slams on the word it names.
    b.reveal(
        "stamp",
        stamp_text.iter().cloned(),
        crate::speech::RevealRole::Stamp,
    );

    // 2. The clipping: title in the serif headline + redaction bars.
    let clip_w = by_class(0.40, 0.47, 0.80) * w;
    let clip_max_h = if cols { 0.62 * avail } else { sc * avail };
    let pad = 0.05 * clip_w;
    let text_w = clip_w - 2.0 * pad;
    let bar_h = (0.03 * clip_w).max(10.0 * u);
    let bar_gap = 0.75 * bar_h;
    let n_bars = 3 + (frac(seed, 0xB4) * 3.0) as usize;
    let mut bars = n_bars.min(5);
    let title_max_h = (0.55 * clip_max_h - pad).max(40.0 * u);
    let title_block = fit_title(ctx, &title, text_w, title_max_h);
    let title_h = title_block.height();
    let bars_h = |n: usize| n as f32 * bar_h + (n.saturating_sub(1)) as f32 * bar_gap;
    let gap_title = 1.1 * bar_h;
    while bars > 3 && pad * 2.0 + title_h + gap_title + bars_h(bars) > clip_max_h {
        bars -= 1;
    }
    let clip_h = pad * 2.0 + title_h + gap_title + bars_h(bars);
    let clip_rot = (frac(seed, 0xC1) - 0.5) * 4.0; // -2..2 degrees

    // 3. The photo card hugs the picture (subject box plus a margin).
    let photo = picture.as_ref().map(|pic| {
        let max_w = if cols {
            by_class(0.34, 0.40, 0.0) * w
        } else if !figures.is_empty() {
            0.52 * w
        } else {
            0.62 * w
        };
        let max_h = if cols { 0.90 * avail } else { sp * avail };
        photo_geo(ctx, pic, max_w, max_h)
    });

    // 4. The figure card: per entry a label, the value, a red underline.
    struct Entry {
        value: Block,
        label: Option<Block>,
        label_gap: f32,
    }
    struct Figure {
        w: f32,
        h: f32,
        pad: f32,
        entries: Vec<Entry>,
        entry_gap: f32,
        under_gap: f32,
        under_h: f32,
    }
    let fig = (!figures.is_empty()).then(|| {
        let n = figures.len();
        let max_w = by_class(0.46, 0.55, 0.84) * w;
        let fpad = 0.06 * max_w;
        let inner_w = max_w - 2.0 * fpad;
        let max_h = if cols {
            (if n > 1 { 0.52 } else { 0.40 }) * avail
        } else {
            sf * avail
        };
        let labels: Vec<Option<Block>> = figures
            .iter()
            .enumerate()
            .map(|(i, (_, meaning))| {
                meaning
                    .as_deref()
                    .filter(|_| !(i == 0 && drop_label))
                    .map(|s| fit_line(ctx, Voice::LABEL, s, inner_w, 34.0 * u))
            })
            .collect();
        let label_h = labels
            .iter()
            .flatten()
            .map(Block::height)
            .fold(0.0, f32::max);
        let label_gap = if label_h > 0.0 { 0.3 * label_h } else { 0.0 };
        let under_h = 22.0 * u;
        let under_gap = 14.0 * u;
        let entry_gap = 0.5 * fpad;
        let value_max_h = ((max_h - 2.0 * fpad - (n - 1) as f32 * entry_gap) / n as f32
            - label_h
            - label_gap
            - under_gap
            - under_h)
            .max(40.0 * u);
        // One size for every value: the smallest that fits.
        let lh = Voice::HERO_NUMBER.line_height;
        let mut size = (value_max_h / lh).min(300.0 * u);
        for (v, _) in &figures {
            let vb = fit_line(ctx, Voice::HERO_NUMBER, v, inner_w, size);
            size = size.min(vb.size);
        }
        let entries: Vec<Entry> = figures
            .iter()
            .zip(labels)
            .map(|((v, _), label)| Entry {
                value: fit_line(ctx, Voice::HERO_NUMBER, v, inner_w, size),
                label_gap: if label.is_some() { label_gap } else { 0.0 },
                label,
            })
            .collect();
        let entry_h = |e: &Entry| {
            e.label.as_ref().map_or(0.0, Block::height)
                + e.label_gap
                + e.value.height()
                + under_gap
                + under_h
        };
        // The card is as wide as its content needs (never wider than max).
        let content_w = entries
            .iter()
            .map(|e| {
                e.value
                    .width()
                    .max(e.label.as_ref().map_or(0.0, Block::width))
            })
            .fold(0.0, f32::max)
            .min(inner_w);
        let w_card = (content_w * 1.02 + 2.0 + 2.0 * fpad).clamp(0.55 * max_w, max_w);
        let h_card =
            2.0 * fpad + entries.iter().map(entry_h).sum::<f32>() + (n - 1) as f32 * entry_gap;
        Figure {
            w: w_card,
            h: h_card,
            pad: fpad,
            entries,
            entry_gap,
            under_gap,
            under_h,
        }
    });

    // 5. Where the cards sit.
    let clamp_to_canvas = |p: Place| -> Place {
        let (hx, hy) = rotated_half(p.w, p.h, p.rot);
        let cx = if p.cx - hx >= edge_x && p.cx + hx <= w - edge_x {
            p.cx
        } else {
            p.cx.clamp(edge_x + hx, (w - edge_x - hx).max(edge_x + hx))
        };
        let cy =
            p.cy.clamp(top - 0.02 * h + hy, (bottom + 0.02 * h - hy).max(hy));
        Place { cx, cy, ..p }
    };
    let photo_rot = {
        let sign = if frac(seed, 0xD2) < 0.5 { 1.0 } else { -1.0 };
        sign * (3.0 + 3.0 * frac(seed, 0xD3))
    };
    let (clip_at, photo_at, fig_at);
    if cols {
        // Left column: clip over figure; photo centred in the space right of it.
        let col_x = m * 0.9 + clip_w / 2.0;
        let clip_cy = top + clip_h / 2.0 + 0.04 * avail;
        clip_at = Place {
            cx: col_x,
            cy: clip_cy,
            w: clip_w,
            h: clip_h,
            rot: clip_rot,
        };
        let clip_right = col_x + clip_w / 2.0;
        photo_at = photo.as_ref().map(|p| Place {
            cx: (clip_right + (w - edge_x - clip_right) / 2.0)
                .max(clip_right - 0.06 * w + p.w / 2.0),
            cy: top + avail / 2.0,
            w: p.w,
            h: p.h,
            rot: photo_rot,
        });
        fig_at = fig.as_ref().map(|f| Place {
            cx: col_x + 0.06 * w,
            cy: clip_at.bottom() + f.h / 2.0 + 0.012 * avail,
            w: f.w,
            h: f.h,
            rot: -clip_rot * 0.8 - 1.0,
        });
    } else {
        // Flow down: clip (left), photo (right, overlapping the clip's lower
        // corner), figure (left, overlapping the photo's lower corner).
        let total =
            clip_h + photo.as_ref().map_or(0.0, |p| p.h) + fig.as_ref().map_or(0.0, |f| f.h);
        let overlap = 0.035 * avail;
        let used = total - overlap * (n_cards - 1) as f32;
        let slack = (avail - used).max(0.0);
        let gap = if n_cards > 1 { slack * 0.25 } else { 0.0 };
        let mut y = top + slack * if n_cards == 1 { 0.3 } else { 0.2 };
        clip_at = Place {
            cx: edge_x + clip_w / 2.0 + 0.012 * w,
            cy: y + clip_h / 2.0,
            w: clip_w,
            h: clip_h,
            rot: clip_rot,
        };
        y += clip_h - overlap + gap;
        // (0.18) The focal figure sits in the middle of the stack.
        fig_at = fig.as_ref().map(|f| {
            let at = Place {
                cx: (0.5 * w).max(edge_x + f.w / 2.0 + 0.03 * w),
                cy: y + f.h / 2.0,
                w: f.w,
                h: f.h,
                rot: -clip_rot * 0.8 - 1.2,
            };
            y += f.h - overlap + gap;
            at
        });
        photo_at = photo.as_ref().map(|p| Place {
            cx: w - edge_x - p.w / 2.0 - 0.012 * w,
            cy: y + p.h / 2.0,
            w: p.w,
            h: p.h,
            rot: photo_rot,
        });
    }
    let clip_at = clamp_to_canvas(clip_at);
    let photo_at = photo_at.map(clamp_to_canvas);
    let fig_at = fig_at.map(clamp_to_canvas);

    // EVOLVE, but never before the last card has landed.
    let evolve = evolve_at(ctx, b, t, fig.as_ref().map_or(0, |f| f.entries.len()));

    // ---- clip card -------------------------------------------------------
    let bars_top = pad + title_h + gap_title;
    let clip = ClipText {
        block: &title_block,
        keyword: keyword.as_deref(),
        pad,
        text_w,
        bar_h,
        bar_gap,
        bars_top,
        bars,
    };
    let kids = clip_kids(ctx, b, &clip, evolve, t, seed);
    if held_clip {
        let title_id = b.id("clip.title");
        b.motions.push(mo::fade(
            &title_id,
            t + 0.2,
            0.3,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
    }
    let slot = |card: Focal| -> usize {
        if card == focal {
            0
        } else if card == Focal::Photo || focal == Focal::Photo {
            1
        } else {
            2
        }
    };
    b.focal = Some(b.id(match focal {
        Focal::Figure => "figure",
        Focal::Photo => "photo",
        Focal::Clip => "clip",
    }));
    let clip_group = card(ctx, b, "clip", clip_at, 10, slot(Focal::Clip), true, kids);
    slide_in(b, &clip_group.id, t, [0.0, 0.09 * h], 3.0);
    b.push(clip_group);

    // ---- photo card ------------------------------------------------------
    if let (Some(pic), Some(p), Some(at)) = (picture.as_ref(), photo.as_ref(), photo_at) {
        let start = t + STAGGER;
        let mut kids: Vec<Layer> = Vec::new();
        match pic {
            Picture::Delivered(_, role) => {
                if let Some(facts) = p.facts.as_ref() {
                    // The subject fills the inner box; the image is placed so.
                    let img = placement::subject_fit(facts, p.inner);
                    let r = (img.x, img.y, img.w, img.h);
                    let plated = plate::image_plate(ctx, b, "photo.pic", *role, r, 6, start + 0.2);
                    let id = plated.id.clone();
                    b.motions.retain(|mo| {
                        !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. }))
                    });
                    if held_photo {
                        b.motions
                            .push(mo::fade(&id, start + 0.2, 0.3, 0.0, 1.0, Easing::OutCubic));
                    }
                    let mut layers = plated.layers;
                    if !p.paper {
                        bare_library_print(ctx, pic, &mut layers);
                    }
                    kids.extend(layers);
                }
            }
            Picture::Object(which, subject) => {
                let slot = Slot {
                    cx: at.left() + p.inner.x + p.inner.w / 2.0,
                    cy: at.top() + p.inner.y + p.inner.h / 2.0,
                    w: p.inner.w,
                    h: p.inner.h,
                };
                if let Some(mut l) = recipes::place_subject(
                    ctx,
                    b,
                    carries,
                    *which,
                    subject,
                    slot,
                    (at.left(), at.top()),
                    ctx.palette.ink,
                    start + 0.2,
                    Entrance::Pop,
                    None,
                    None,
                    "photo.pic",
                )? {
                    // The card sliding in is the entrance.
                    let id = l.id.clone();
                    b.motions.retain(|mo| {
                        !(mo.target == id
                            && matches!(
                                mo.op,
                                MotionOp::ClipReveal { .. } | MotionOp::Scale { .. }
                            ))
                    });
                    // (0.22) Like the pair: a library picture has no
                    // analysis, so it always wears the print border.
                    library_print(ctx, &mut l);
                    l.z_index = 6;
                    if held_photo {
                        b.motions.push(mo::fade(
                            &l.id,
                            start + 0.2,
                            0.3,
                            0.0,
                            1.0,
                            Easing::OutCubic,
                        ));
                    }
                    kids.push(l);
                } else {
                    // Carried across beats: a shared element.
                    print_carried(ctx, carries, subject, b.plan.index);
                }
            }
        }
        // One piece of tape across the top edge of a paper card (0.22: a
        // picture of unknown extent has transparent margins it would float in).
        if p.paper {
            let tape_id = b.id("photo.tape");
            let tape_color = ctx
                .palette
                .fields
                .first()
                .copied()
                .unwrap_or(ctx.palette.accent);
            let (tw, th) = (0.42 * at.w, 0.1 * at.w * 0.6);
            let tape_x = at.w * (0.28 + 0.44 * frac(seed, 0xF1));
            let mut tape = base_layer(
                tape_id.clone(),
                (tape_x, 0.0, tw, th),
                LayerKind::RoundedRectangle {
                    fill: tape_color,
                    radius: 2.0 * u,
                    stroke: None,
                },
                8,
            );
            tape.anchor_x = 0.5;
            tape.anchor_y = 0.5;
            tape.rotation_degrees = -16.0 + 32.0 * frac(seed, 0xF2);
            tape.opacity = 0.92;
            b.motions.push(mo::mask(
                &tape_id,
                start + 0.45,
                0.35,
                Direction::Right,
                Easing::OutCubic,
            ));
            kids.push(tape);
        }
        let photo_group = card(ctx, b, "photo", at, 20, slot(Focal::Photo), p.paper, kids);
        slide_in(b, &photo_group.id, start, [0.35 * w, 0.03 * h], -4.0);
        b.push(photo_group);
    }

    // ---- figure card -----------------------------------------------------
    if let (Some(f), Some(at)) = (fig.as_ref(), fig_at) {
        let start = t + STAGGER * (n_cards - 1) as f64;
        let mut kids: Vec<Layer> = Vec::new();
        let mut held_values: Vec<String> = Vec::new();
        let mut y = f.pad;
        for (i, e) in f.entries.iter().enumerate() {
            // The first entry keeps the plain ids, later ones get a number.
            let name = |base: &str| {
                if i == 0 {
                    b.id(&format!("figure.{base}"))
                } else {
                    b.id(&format!("figure.{base}{}", i + 1))
                }
            };
            if let Some(label) = &e.label {
                let mut l = text_layer(name("label"), label, ctx.palette.muted, TextAlign::Left);
                l.x = f.pad;
                l.y = y;
                l.z_index = 5;
                kids.push(l);
                y += label.height() + e.label_gap;
            }
            let mut v = text_layer(name("value"), &e.value, ctx.palette.ink, TextAlign::Left);
            v.x = f.pad;
            v.y = y;
            v.z_index = 5;
            if held_figure {
                held_values.push(v.id.clone());
            }
            kids.push(v);
            y += e.value.height() + f.under_gap;
            let uw = (f.w - 2.0 * f.pad).min(e.value.width() * 1.02 + 2.0);
            let under_id = name("underline");
            let mut under = base_layer(
                under_id.clone(),
                (f.pad, y, uw, f.under_h),
                LayerKind::Polyline {
                    points: underline_points(seed ^ i as u64, uw, f.under_h),
                    stroke: Stroke {
                        color: crate::compiler::brand::mark_color(ctx.style, RED),
                        width: 8.0 * u,
                    },
                    closed: false,
                    fill: None,
                },
                6,
            );
            under.z_index = 6;
            b.motions.push(Motion {
                id: None,
                target: under_id,
                start: evolve + 0.15 + 0.3 * i as f64,
                duration: 0.55,
                easing: Easing::OutCubic,
                spring: None,
                op: MotionOp::Trim { from: 0.0, to: 1.0 },
            });
            kids.push(under);
            y += f.under_h + f.entry_gap;
        }
        for id in &held_values {
            b.motions
                .push(mo::fade(id, start + 0.2, 0.3, 0.0, 1.0, Easing::OutCubic));
        }
        let group = card(ctx, b, "figure", at, 28, slot(Focal::Figure), true, kids);
        slide_in(b, &group.id, start, [0.0, 0.09 * h], 3.0);
        b.push(group);
    }

    // ---- stamp -----------------------------------------------------------
    // (0.18) The stamp lands on the focal card: across the figure card's top
    // right corner (clear of its label), on the photo card's lower edge, or
    // across the clipping's redaction bars when the clipping is all there is.
    if let Some(kw) = stamp_text.as_deref() {
        let max_w = by_class(0.30, 0.40, 0.60) * w;
        let sblock = ctx
            .ts
            .fit_block(Voice::HEADLINE, kw, max_w, 0.09 * h, 120.0 * u, 2);
        let (px, py) = (0.22 * sblock.size, 0.16 * sblock.size);
        let (sw, sh) = (
            sblock.width() * 1.02 + 2.0 + 2.0 * px,
            sblock.height() + 2.0 * py,
        );
        let (hx, hy) = rotated_half(sw, sh, -8.0);
        let (bx, by) = match (focal, fig.as_ref(), fig_at, photo_at) {
            (Focal::Figure, Some(f), Some(at), _) => {
                let lx = at.w - 0.45 * sw;
                // Never over the first entry's text: above its label, else
                // above its value, wherever they reach under the stamp.
                let mut clear = f32::MAX;
                if let Some(e) = f.entries.first() {
                    let mut top = f.pad;
                    if let Some(lb) = &e.label {
                        if f.pad + lb.width() > lx - hx {
                            clear = clear.min(top);
                        }
                        top += lb.height() + e.label_gap;
                    }
                    if f.pad + e.value.width() > lx - hx {
                        clear = clear.min(top);
                    }
                }
                let ly = (0.08 * sh).min(clear - hy - 2.0 * u);
                at.canvas_point(lx, ly)
            }
            // On the paper's lower margin. A bare picture (no paper) has no
            // margin: the stamp straddles its top right corner, mostly above
            // it (0.22: clear of the subject, layout QA `text_over_subject`,
            // and of the captions below).
            (Focal::Photo, _, _, Some(at)) => {
                if photo.as_ref().is_some_and(|p| p.paper) {
                    at.canvas_point(at.w - 0.42 * sw, at.h - 0.12 * sh)
                } else if ctx.direction_seed.is_some() {
                    // (0.23) Its lettering clears the picture, its frame still
                    // rests on the picture's top edge (the stamp stays on the
                    // focal card): the type, turn included, never covers the
                    // subject (a satellite dish's rim reached under it: 6.7 %
                    // of the subject, max 4 %).
                    let (_, text_hy) =
                        rotated_half(sblock.width() * 1.02 + 2.0, sblock.height(), -8.0);
                    at.canvas_point(at.w - 0.42 * sw, -(text_hy + 0.004 * h))
                } else {
                    at.canvas_point(at.w - 0.42 * sw, -0.25 * sh)
                }
            }
            _ => {
                // Across the clipping's redaction bars, clear of the headline
                // even where the stamp's turn and the card's lift its end.
                let swing = 0.5 * sw * 8.0_f32.to_radians().sin()
                    + 0.5 * clip_at.w * clip_at.rot.abs().to_radians().sin();
                let clear_y = pad + title_h + 0.12 * sh + sh / 2.0 + swing;
                let local_y = (bars_top + bars_h(bars) / 2.0).max(clear_y);
                clip_at.canvas_point(0.06 * clip_at.w + sw / 2.0, local_y)
            }
        };
        let mut cx = bx.clamp(edge_x + hx, (w - edge_x - hx).max(edge_x + hx));
        let mut cy = by.clamp(hy, (h - hy).max(hy));
        // Off the photo when the photo is not the focal card: the stamp is
        // text and the photo is a subject. Rise first, then slide left (a
        // margin covers the camera's parallax between the cards).
        if let (Some(p), true) = (photo_at, focal != Focal::Photo) {
            let (px2, py2) = rotated_half(p.w, p.h, p.rot);
            let gap = 0.02 * h;
            let (pl, pt) = (p.cx - px2 - gap, p.cy - py2 - gap);
            let (pr, pb) = (p.cx + px2 + gap, p.cy + py2 + gap);
            let hits =
                |cx: f32, cy: f32| cx + hx > pl && cx - hx < pr && cy + hy > pt && cy - hy < pb;
            // On the figure it slides along the card only (rising would
            // leave the card it stamps).
            if hits(cx, cy) && focal != Focal::Figure {
                cy = (pt - hy).clamp(hy.min(cy), cy);
            }
            if hits(cx, cy) {
                cx = (pl - hx).clamp((edge_x + hx).min(cx), cx);
            }
        }
        let sid = b.id("stamp");
        let mut frame = base_layer(
            b.id("stamp.frame"),
            (0.0, 0.0, sw, sh),
            LayerKind::Rectangle {
                fill: ctx.palette.card.with_alpha(0xCC),
                stroke: Some(Stroke {
                    color: ctx.palette.accent,
                    width: 6.0 * u,
                }),
            },
            0,
        );
        frame.z_index = 0;
        let mut text = text_layer(
            b.id("stamp.text"),
            &sblock,
            ctx.palette.accent,
            TextAlign::Center,
        );
        text.x = px;
        text.y = py;
        text.z_index = 1;
        let mut stamp = base_layer(
            sid.clone(),
            (cx, cy, sw, sh),
            LayerKind::Group {
                children: vec![frame, text],
            },
            40,
        );
        stamp.anchor_x = 0.5;
        stamp.anchor_y = 0.5;
        stamp.rotation_degrees = -8.0;
        // Landed before ANTICIPATE even when EVOLVE is short.
        let at = (evolve + 0.45)
            .min(life.anticipate - 0.15 - STAMP_TIME)
            .max(evolve);
        b.motions
            .push(mo::fade(&sid, at, 0.06, 0.0, 1.0, Easing::Linear));
        b.motions.push(mo::scale(
            &sid,
            at,
            STAMP_TIME,
            STAMP_FROM,
            1.0,
            Easing::InCubic,
        ));
        b.push(stamp);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// (0.22) The documentary pair
// ---------------------------------------------------------------------------

/// The stamp's turn (degrees).
const STAMP_ROT: f32 = -8.0;
/// Pair cards are sized into this share of their cell, leaving room for their
/// small turn so neighbouring documents never touch.
const PAIR_ROOM: f32 = 0.93;

/// (0.22) One side of a documentary pair: a picture, a figure, or both.
struct Side<'a> {
    picture: Option<Picture<'a>>,
    /// The figure (the subject's `value`).
    value: Option<String>,
    /// The figure's label (the subject's `meaning`).
    label: Option<String>,
    /// The words the narrator names the subject by (reveal anchors).
    words: Vec<String>,
    /// When the narrator first names the subject (scene-local seconds), with
    /// a voice-over.
    named: Option<f64>,
    /// The words the figure stands for: the value as written and its first
    /// term ("$1.15 / L" -> "$1.15"), so "one fifteen per litre" still cues it.
    value_words: Vec<String>,
    /// When the narrator says the figure, with a voice-over.
    said: Option<f64>,
}

/// (0.22) A `compare` / `contrast` beat of two subjects that are each a
/// picture (object) or a figure (a number, or a phrase whose value is a
/// figure), at least one of them a picture: the documentary lays them out as
/// two co-equal evidence documents. `None` for every other beat.
///
/// The first picture resolves as a single picture does (a delivered hero
/// image, else the library object); the second from a delivered
/// `supporting_object` image, else the library object (`recipes::place_subject`
/// resolves the planner's request first, then the library by name).
fn pair_sides<'a>(ctx: &Ctx<'a>, b: &B) -> Option<[Side<'a>; 2]> {
    let beat = b.beat;
    if !matches!(beat.purpose, Purpose::Compare | Purpose::Contrast) {
        return None;
    }
    let second = beat.secondary.as_ref()?;
    let figure = |s: &Subject| s.value().and_then(figure_text);
    let eligible = |s: &Subject| match s.kind() {
        SubjectKind::Object => true,
        SubjectKind::Number | SubjectKind::Phrase => figure(s).is_some(),
        _ => false,
    };
    let is_object = |s: &Subject| s.kind() == SubjectKind::Object;
    if !eligible(&beat.primary)
        || !eligible(second)
        || !(is_object(&beat.primary) || is_object(second))
    {
        return None;
    }
    let index = b.plan.index;
    let first_pic = if is_object(&beat.primary) {
        find_picture(ctx, b)
    } else {
        None
    };
    let second_pic = match second {
        Subject::Object(o) => {
            // A plan that filed the library primary under `supporting_object`
            // (pre-0.22 dossier roles) must not show that picture twice.
            let supporting = plate::request_id(index, AssetRole::SupportingObject);
            let taken = matches!(first_pic, Some(Picture::Object(..)))
                && recipes::object_request_id(ctx, b, &beat.primary).as_deref()
                    == Some(supporting.as_str());
            match plate::image(ctx, index, AssetRole::SupportingObject).filter(|_| !taken) {
                Some(entry) => Some(Picture::Delivered(entry, AssetRole::SupportingObject)),
                // A picture the planner requested or the library has by name;
                // otherwise the side keeps its figure and label (better than
                // losing the value to the missing-asset fallback).
                None => {
                    let requested = recipes::object_request_id(ctx, b, second)
                        .is_some_and(|id| ctx.manifest.get(&id).is_some());
                    (requested || ctx.library.find_object(&o.asset).is_some())
                        .then(|| Picture::Object(Which::Secondary, second.clone()))
                }
            }
        }
        _ => None,
    };
    let side = |s: &Subject, picture: Option<Picture<'a>>| {
        use crate::compiler::speech_plan::{anchor_time, first_term, name_time};
        let value = match s.kind() {
            SubjectKind::Object => object_value(s),
            _ => figure(s),
        };
        // The figure: as written and its first term ("$1.15 / L" -> "$1.15"),
        // so "one fifteen per litre" still cues it.
        let value_words: Vec<String> = value
            .iter()
            .cloned()
            .chain(value.as_deref().and_then(first_term))
            .collect();
        let said = anchor_time(&ctx.spoken, &strs(&value_words));
        // The subject: its name, meaning and figure, matched as the word cues
        // match them; any single word of the name as the last resort.
        let mut words = recipes::subject_words(s);
        words.extend(value.as_deref().and_then(first_term));
        let named = anchor_time(&ctx.spoken, &strs(&words)).or_else(|| match s {
            Subject::Object(o) => name_time(
                &ctx.spoken,
                &[o.asset.as_str(), o.meaning.as_deref().unwrap_or_default()],
            ),
            _ => None,
        });
        Side {
            picture,
            value,
            value_words,
            said,
            label: s
                .meaning()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_string),
            words,
            named,
        }
    };
    Some([side(&beat.primary, first_pic), side(second, second_pic)])
}

/// Borrowed views of owned words (for `speech_plan::anchor_time`).
fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

/// A pair figure card: its size and content (card-local layout from the top).
struct PairFigure {
    w: f32,
    h: f32,
    label: Option<Block>,
    value: Option<Block>,
}

/// (0.22) DocumentaryDossier for a pair: the clipping across the top (it
/// carries the stamp and is the focal card: the comparison is what the beat is
/// about), then two co-equal evidence documents, side by side on square and
/// wide canvases, stacked on tall ones. A document is a photo card (the
/// picture) with its figure card (the value large, the meaning as its label,
/// the brand-aware underline drawn at EVOLVE), beside it in a wide cell and
/// under it in a tall one. The first document lands with the clipping; the
/// second when the narrator names its subject (right after the first without
/// a voice-over). Nothing overlaps; every card stays inside the safe area.
fn build_pair(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    sides: [Side; 2],
) -> Result<(), CompileError> {
    use crate::compiler::recipes::title_key_words;
    use crate::speech::RevealRole;
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let life = plan.life;
    let t = plan.enter_at();
    let seed = plan.seed;
    let land = placement::side_by_side(&ctx.frame);
    // Square and wider canvases set the documents side by side; tall ones
    // stack them.
    let across = land || ctx.frame.aspect >= 0.95;
    let by_class = |wide: f32, square: f32, tall: f32| {
        if land {
            wide
        } else if across {
            square
        } else {
            tall
        }
    };

    let title = capitalised(&b.beat.statement);
    let keyword = b.beat.keyword.clone().filter(|k| !k.trim().is_empty());
    // The stamp names the keyword when the narrator says it, and never
    // repeats a figure already on a card.
    let words_of = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(norm)
            .filter(|w| !w.is_empty())
            .collect()
    };
    let stamp_text = keyword.clone().filter(|k| {
        let kw = words_of(k);
        let repeats = sides.iter().filter_map(|s| s.value.as_deref()).any(|v| {
            let vw = words_of(v);
            !kw.is_empty() && kw.iter().all(|x| vw.contains(x))
        });
        !repeats && stamp_allowed(ctx, k)
    });

    // Reveal anchors: each picture arrives when its subject is named, each
    // figure lands on its amount, the stamp on its word.
    b.reveal(
        "clip",
        title_key_words(b.beat, &b.beat.statement),
        RevealRole::Title,
    );
    for (i, s) in sides.iter().enumerate() {
        if s.picture.is_some() {
            b.reveal(
                &format!("photo.{i}"),
                s.words.iter().cloned(),
                RevealRole::Content,
            );
        }
        b.reveal(
            &format!("figure.{i}"),
            s.value_words.iter().cloned(),
            RevealRole::Value,
        );
    }
    b.reveal("stamp", stamp_text.iter().cloned(), RevealRole::Stamp);

    // Vertical budget: below the kicker; a little above the single layout's
    // bottom, because the camera pivots on the clipping at the top and its
    // push carries the lower documents down.
    let top = 0.165 * h;
    let bottom = h - 1.1 * m;
    let avail = bottom - top;
    let edge_x = 0.6 * m;
    let x0 = edge_x + 0.012 * w;

    // 1. The clipping across the top: title, redaction bars, and room for the
    //    stamp in its lower right corner.
    let clip_w = by_class(0.56, 0.72, 0.80) * w;
    let clip_max_h = by_class(0.30, 0.22, 0.22) * avail;
    let pad = 0.05 * clip_w;
    let text_w = clip_w - 2.0 * pad;
    let bar_h = (0.03 * clip_w).max(10.0 * u);
    let bar_gap = 0.75 * bar_h;
    let mut bars = (3 + (frac(seed, 0xB4) * 3.0) as usize).min(5);
    let title_max_h = (0.55 * clip_max_h - pad).max(40.0 * u);
    let title_block = fit_title(ctx, &title, text_w, title_max_h);
    let title_h = title_block.height();
    let bars_h = |n: usize| n as f32 * bar_h + (n.saturating_sub(1)) as f32 * bar_gap;
    let gap_title = 1.1 * bar_h;
    while bars > 3 && pad * 2.0 + title_h + gap_title + bars_h(bars) > clip_max_h {
        bars -= 1;
    }
    let stamp_geo = stamp_text.as_deref().map(|kw| {
        let max_w = by_class(0.24, 0.30, 0.42) * w;
        let block = ctx
            .ts
            .fit_block(Voice::HEADLINE, kw, max_w, 0.07 * h, 110.0 * u, 2);
        let (px, py) = (0.22 * block.size, 0.16 * block.size);
        let (sw, sh) = (
            block.width() * 1.02 + 2.0 + 2.0 * px,
            block.height() + 2.0 * py,
        );
        (block, px, py, sw, sh)
    });
    let clip_rot = (frac(seed, 0xC1) - 0.5) * 3.0; // -1.5..1.5 degrees
                                                   // The stamp's half extents on the card (its turn against the card's).
    let stamp_local = |sw: f32, sh: f32| rotated_half(sw, sh, STAMP_ROT.abs() + clip_rot.abs());
    let mut clip_h = pad * 2.0 + title_h + gap_title + bars_h(bars);
    if let Some((_, _, _, sw, sh)) = &stamp_geo {
        let (_, hy) = stamp_local(*sw, *sh);
        clip_h = clip_h.max(pad + title_h + 0.4 * gap_title + 2.0 * hy + 0.5 * pad);
    }
    let clip_at = Place {
        cx: x0 + clip_w / 2.0,
        cy: top + clip_h / 2.0,
        w: clip_w,
        h: clip_h,
        rot: clip_rot,
    };

    // 2. Two cells for the documents under the clipping.
    let (_, clip_hy) = rotated_half(clip_w, clip_h, clip_rot);
    let docs_top = clip_at.cy + clip_hy + 0.035 * avail;
    let region = Rect::new(
        x0,
        docs_top,
        w - 2.0 * x0,
        (bottom - docs_top).max(0.3 * avail),
    );
    let cells = if across {
        let gap = 0.05 * w;
        let cw = (region.w - gap) / 2.0;
        [
            Rect::new(region.x, region.y, cw, region.h),
            Rect::new(region.x + cw + gap, region.y, cw, region.h),
        ]
    } else {
        let gap = 0.04 * avail;
        let ch = (region.h - gap) / 2.0;
        [
            Rect::new(region.x, region.y, region.w, ch),
            Rect::new(region.x, region.y + ch + gap, region.w, ch),
        ]
    };
    let cell = cells[0];
    // Inside a document the photo sits beside its figure in a wide cell,
    // above it in a tall one.
    let beside = cell.w >= 1.25 * cell.h;
    let inner_gap = if beside { 0.05 * cell.w } else { 0.04 * cell.h };
    let ((pw_max, ph_max), (fw_max, fh_max)) = if beside {
        let room = cell.w - inner_gap;
        ((0.46 * room, cell.h), (0.54 * room, cell.h))
    } else {
        let room = cell.h - inner_gap;
        ((cell.w, 0.45 * room), (cell.w, 0.55 * room))
    };
    let photos: Vec<Option<Photo>> = sides
        .iter()
        .map(|s| {
            s.picture
                .as_ref()
                .map(|pic| photo_geo(ctx, pic, PAIR_ROOM * pw_max, PAIR_ROOM * ph_max))
        })
        .collect();

    // 3. The figure cards: one value size for both (co-equal), the same card
    //    size for both.
    let fig_w_max = PAIR_ROOM * fw_max;
    let fig_h_max = PAIR_ROOM * fh_max;
    let fpad = 0.06 * fig_w_max;
    let inner_w = fig_w_max - 2.0 * fpad;
    let labels: Vec<Option<Block>> = sides
        .iter()
        .map(|s| {
            s.label
                .as_deref()
                .map(|l| fit_line(ctx, Voice::LABEL, l, inner_w, 34.0 * u))
        })
        .collect();
    let label_h = labels
        .iter()
        .flatten()
        .map(Block::height)
        .fold(0.0, f32::max);
    let label_gap = if label_h > 0.0 { 0.3 * label_h } else { 0.0 };
    let under_h = 22.0 * u;
    let under_gap = 14.0 * u;
    let value_max_h =
        (fig_h_max - 2.0 * fpad - label_h - label_gap - under_gap - under_h).max(40.0 * u);
    let mut size = (value_max_h / Voice::HERO_NUMBER.line_height).min(300.0 * u);
    for v in sides.iter().filter_map(|s| s.value.as_deref()) {
        size = size.min(fit_line(ctx, Voice::HERO_NUMBER, v, inner_w, size).size);
    }
    let mut figs: Vec<Option<PairFigure>> = sides
        .iter()
        .zip(labels)
        .map(|(s, label)| {
            let value = s
                .value
                .as_deref()
                .map(|v| fit_line(ctx, Voice::HERO_NUMBER, v, inner_w, size));
            if value.is_none() && label.is_none() {
                return None;
            }
            let content_w = value
                .as_ref()
                .map_or(0.0, Block::width)
                .max(label.as_ref().map_or(0.0, Block::width))
                .min(inner_w);
            let both = label.is_some() && value.is_some();
            let h_card = 2.0 * fpad
                + label.as_ref().map_or(0.0, Block::height)
                + if both { label_gap } else { 0.0 }
                + value
                    .as_ref()
                    .map_or(0.0, |v| v.height() + under_gap + under_h);
            Some(PairFigure {
                w: (content_w * 1.02 + 2.0 + 2.0 * fpad).clamp(0.55 * fig_w_max, fig_w_max),
                h: h_card,
                label,
                value,
            })
        })
        .collect();
    let (card_w, card_h) = figs
        .iter()
        .flatten()
        .fold((0.0f32, 0.0f32), |(a, c), f| (a.max(f.w), c.max(f.h)));
    for f in figs.iter_mut().flatten() {
        f.w = card_w;
        f.h = card_h;
    }

    // 4. Where each document's cards sit: centred in its cell, with a small
    //    seeded turn (the two documents turn opposite ways).
    let turn = |i: usize| {
        let sign = if (frac(seed, 0xD2) < 0.5) == (i == 0) {
            1.0
        } else {
            -1.0
        };
        sign * (1.5 + 1.5 * frac(seed, 0xD3 + i as u64))
    };
    // (width, height, turn, rotated half extents)
    type Box4 = (f32, f32, f32, (f32, f32));
    let place = |(bw, bh, rot, _): Box4, cx: f32, cy: f32| Place {
        cx,
        cy,
        w: bw,
        h: bh,
        rot,
    };
    let mut photo_at: [Option<Place>; 2] = [None, None];
    let mut fig_at: [Option<Place>; 2] = [None, None];
    for i in 0..2 {
        let c = cells[i];
        let (pr, fr) = (turn(i), -0.4 * turn(i));
        let p: Option<Box4> = photos[i]
            .as_ref()
            .map(|p| (p.w, p.h, pr, rotated_half(p.w, p.h, pr)));
        let f: Option<Box4> = figs[i]
            .as_ref()
            .map(|f| (f.w, f.h, fr, rotated_half(f.w, f.h, fr)));
        let gap = if p.is_some() && f.is_some() {
            inner_gap
        } else {
            0.0
        };
        let (cx, cy) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
        if beside {
            let pw = p.map_or(0.0, |p| 2.0 * p.3 .0);
            let total = pw + gap + f.map_or(0.0, |f| 2.0 * f.3 .0);
            let left = cx - total / 2.0;
            photo_at[i] = p.map(|p| place(p, left + p.3 .0, cy));
            fig_at[i] = f.map(|f| place(f, left + pw + gap + f.3 .0, cy));
        } else {
            let ph = p.map_or(0.0, |p| 2.0 * p.3 .1);
            let total = ph + gap + f.map_or(0.0, |f| 2.0 * f.3 .1);
            let upper = cy - total / 2.0;
            photo_at[i] = p.map(|p| place(p, cx, upper + p.3 .1));
            fig_at[i] = f.map(|f| place(f, cx, upper + ph + gap + f.3 .1));
        }
    }

    // 5. Timing: the first document lands with the clipping, the second
    //    right after it; with a voice-over a picture waits until its subject
    //    is named and a figure until it is said (never before its picture).
    // A picture keeps 0.8 s before ANTICIPATE to be seen, a figure 0.5 s.
    let on_word = |at: Option<f64>, earliest: f64, read: f64| {
        at.map_or(earliest, |at| {
            (at - crate::compiler::speech_plan::WORD_CUE_LEAD)
                .min(life.anticipate - read)
                .max(earliest)
        })
    };
    let starts = [
        on_word(sides[0].named, t + STAGGER, 0.8),
        on_word(sides[1].named, t + 3.0 * STAGGER, 0.8),
    ];
    // EVOLVE, but never before the first document has landed.
    // The last figure with a value draws the last underline (`0.3 * side`).
    let underlines = figs
        .iter()
        .rposition(|f| f.as_ref().is_some_and(|f| f.value.is_some()))
        .map_or(0, |i| i + 1);
    let evolve = evolve_at(ctx, b, t, underlines);

    // ---- clip card (focal: the comparison) --------------------------------
    let bars_top = pad + title_h + gap_title;
    let clip = ClipText {
        block: &title_block,
        keyword: keyword.as_deref(),
        pad,
        text_w,
        bar_h,
        bar_gap,
        bars_top,
        bars,
    };
    let kids = clip_kids(ctx, b, &clip, evolve, t, seed);
    b.focal = Some(b.id("clip"));
    let clip_group = card(ctx, b, "clip", clip_at, 10, 0, true, kids);
    slide_in(b, &clip_group.id, t, [0.0, 0.09 * h], 3.0);
    b.push(clip_group);

    for i in 0..2 {
        let start = starts[i];
        let from_x = if i == 0 { -0.35 * w } else { 0.35 * w };
        // ---- photo card ---------------------------------------------------
        if let (Some(pic), Some(p), Some(at)) =
            (sides[i].picture.as_ref(), photos[i].as_ref(), photo_at[i])
        {
            let name = format!("photo.{i}");
            let pic_name = format!("photo.{i}.pic");
            let mut kids: Vec<Layer> = Vec::new();
            match pic {
                Picture::Delivered(_, role) => {
                    if let Some(facts) = p.facts.as_ref() {
                        let img = placement::subject_fit(facts, p.inner);
                        let r = (img.x, img.y, img.w, img.h);
                        let plated =
                            plate::image_plate(ctx, b, &pic_name, *role, r, 6, start + 0.2);
                        let id = plated.id.clone();
                        b.motions.retain(|mo| {
                            !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. }))
                        });
                        let mut layers = plated.layers;
                        if !p.paper {
                            bare_library_print(ctx, pic, &mut layers);
                        }
                        kids.extend(layers);
                    }
                }
                Picture::Object(which, subject) => {
                    let slot = Slot {
                        cx: at.left() + p.inner.x + p.inner.w / 2.0,
                        cy: at.top() + p.inner.y + p.inner.h / 2.0,
                        w: p.inner.w,
                        h: p.inner.h,
                    };
                    if let Some(mut l) = recipes::place_subject(
                        ctx,
                        b,
                        carries,
                        *which,
                        subject,
                        slot,
                        (at.left(), at.top()),
                        ctx.palette.ink,
                        start + 0.2,
                        Entrance::Pop,
                        None,
                        None,
                        &pic_name,
                    )? {
                        // The card sliding in is the entrance.
                        let id = l.id.clone();
                        b.motions.retain(|mo| {
                            !(mo.target == id
                                && matches!(
                                    mo.op,
                                    MotionOp::ClipReveal { .. } | MotionOp::Scale { .. }
                                ))
                        });
                        library_print(ctx, &mut l);
                        l.z_index = 6;
                        kids.push(l);
                    } else {
                        // Carried across beats: a shared element.
                        print_carried(ctx, carries, subject, b.plan.index);
                    }
                }
            }
            // Tape only across a paper card: a picture of unknown extent has
            // transparent margins the tape would float in.
            if p.paper {
                let tape_id = b.id(&format!("{name}.tape"));
                let tape_color = ctx
                    .palette
                    .fields
                    .first()
                    .copied()
                    .unwrap_or(ctx.palette.accent);
                let (tw, th) = (0.42 * at.w, 0.06 * at.w);
                let tape_x = at.w * (0.28 + 0.44 * frac(seed, 0xF1 + i as u64));
                let mut tape = base_layer(
                    tape_id.clone(),
                    (tape_x, 0.0, tw, th),
                    LayerKind::RoundedRectangle {
                        fill: tape_color,
                        radius: 2.0 * u,
                        stroke: None,
                    },
                    8,
                );
                tape.anchor_x = 0.5;
                tape.anchor_y = 0.5;
                tape.rotation_degrees = -16.0 + 32.0 * frac(seed, 0xF2 + i as u64);
                tape.opacity = 0.92;
                b.motions.push(mo::mask(
                    &tape_id,
                    start + 0.45,
                    0.35,
                    Direction::Right,
                    Easing::OutCubic,
                ));
                kids.push(tape);
            }
            let group = card(ctx, b, &name, at, 20 + i as i32, 1, p.paper, kids);
            slide_in(b, &group.id, start, [from_x, 0.03 * h], -4.0);
            b.push(group);
        }

        // ---- figure card --------------------------------------------------
        if let (Some(f), Some(at)) = (figs[i].as_ref(), fig_at[i]) {
            let name = format!("figure.{i}");
            let fig_start = on_word(
                sides[i].said,
                if photo_at[i].is_some() {
                    start + STAGGER
                } else {
                    start
                },
                0.5,
            );
            let mut kids: Vec<Layer> = Vec::new();
            let mut y = fpad;
            if let Some(label) = &f.label {
                let mut l = text_layer(
                    b.id(&format!("{name}.label")),
                    label,
                    ctx.palette.muted,
                    TextAlign::Left,
                );
                l.x = fpad;
                l.y = y;
                l.z_index = 5;
                kids.push(l);
                y += label.height() + if f.value.is_some() { label_gap } else { 0.0 };
            }
            if let Some(value) = &f.value {
                let mut v = text_layer(
                    b.id(&format!("{name}.value")),
                    value,
                    ctx.palette.ink,
                    TextAlign::Left,
                );
                v.x = fpad;
                v.y = y;
                v.z_index = 5;
                kids.push(v);
                y += value.height() + under_gap;
                let uw = (f.w - 2.0 * fpad).min(value.width() * 1.02 + 2.0);
                let under_id = b.id(&format!("{name}.underline"));
                let mut under = base_layer(
                    under_id.clone(),
                    (fpad, y, uw, under_h),
                    LayerKind::Polyline {
                        points: underline_points(seed ^ i as u64, uw, under_h),
                        stroke: Stroke {
                            color: crate::compiler::brand::mark_color(ctx.style, RED),
                            width: 8.0 * u,
                        },
                        closed: false,
                        fill: None,
                    },
                    6,
                );
                under.z_index = 6;
                // (0.23 A4-short) Waits for its card to settle, but draws by
                // ANTICIPATE where the beat is that short (a figure that is
                // said late in a continuous take): always on the product path,
                // else only where the underline would start at or after the
                // scene end, which does not validate.
                let mut under_at = evolve.max(fig_start + 0.6) + 0.15 + 0.3 * i as f64;
                if ctx.direction_seed.is_some() || under_at >= plan.duration {
                    under_at =
                        under_at.min((life.anticipate - UNDERLINE_TIME).max(fig_start + 0.25));
                }
                b.motions.push(Motion {
                    id: None,
                    target: under_id,
                    start: under_at,
                    duration: UNDERLINE_TIME,
                    easing: Easing::OutCubic,
                    spring: None,
                    op: MotionOp::Trim { from: 0.0, to: 1.0 },
                });
                kids.push(under);
            }
            let group = card(ctx, b, &name, at, 28 + i as i32, 0, true, kids);
            slide_in(b, &group.id, fig_start, [0.0, 0.09 * h], 3.0);
            b.push(group);
        }
    }

    // ---- stamp: the clipping's lower right corner -------------------------
    if let Some((sblock, px, py, sw, sh)) = stamp_geo {
        let (hx, hy) = rotated_half(sw, sh, STAMP_ROT);
        // Inside the clipping's lower edge, clear of the title.
        let (_, local_hy) = stamp_local(sw, sh);
        let clear = pad + title_h + 0.4 * gap_title + local_hy;
        let ly = (clip_h - 0.5 * pad - local_hy).max(clear);
        let lx = clip_w - 0.42 * sw;
        let (bx, by) = clip_at.canvas_point(lx, ly);
        let cx = bx.clamp(edge_x + hx, (w - edge_x - hx).max(edge_x + hx));
        let cy = by.clamp(hy, (h - hy).max(hy));
        let sid = b.id("stamp");
        let mut frame = base_layer(
            b.id("stamp.frame"),
            (0.0, 0.0, sw, sh),
            LayerKind::Rectangle {
                fill: ctx.palette.card.with_alpha(0xCC),
                stroke: Some(Stroke {
                    color: ctx.palette.accent,
                    width: 6.0 * u,
                }),
            },
            0,
        );
        frame.z_index = 0;
        let mut text = text_layer(
            b.id("stamp.text"),
            &sblock,
            ctx.palette.accent,
            TextAlign::Center,
        );
        text.x = px;
        text.y = py;
        text.z_index = 1;
        let mut stamp = base_layer(
            sid.clone(),
            (cx, cy, sw, sh),
            LayerKind::Group {
                children: vec![frame, text],
            },
            40,
        );
        stamp.anchor_x = 0.5;
        stamp.anchor_y = 0.5;
        stamp.rotation_degrees = STAMP_ROT;
        let at = (evolve + 0.45)
            .min(life.anticipate - 0.15 - STAMP_TIME)
            .max(evolve);
        b.motions
            .push(mo::fade(&sid, at, 0.06, 0.0, 1.0, Easing::Linear));
        b.motions.push(mo::scale(
            &sid,
            at,
            STAMP_TIME,
            STAMP_FROM,
            1.0,
            Easing::InCubic,
        ));
        b.push(stamp);
    }
    Ok(())
}
