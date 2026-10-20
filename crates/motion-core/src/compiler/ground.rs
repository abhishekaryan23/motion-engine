//! (0.24 V1) Backdrop worlds: the ground a video is set on, chosen per video
//! and changed per beat group.
//!
//! A [`Family`] is picked deterministically from `(style seed, story key, tone)`
//! (never by the operator), steered away from the families the channel used
//! last (`--backdrop-avoid`, kept by `reel` in `variety.ledger.json`). Inside
//! a video the ground changes every 2-3 beats ([`groups`]) through the
//! family's variants, so no ground holds for more than three beats in a row.
//!
//! - `ink`: the look's own ground with a soft light falloff (a pool of light
//!   from one side, from the other, from below);
//! - `paper`: fibre texture and a real shadow side (left, right, below);
//! - `colour_block`: two palette fields split on a vertical third or a diagonal;
//! - `gradient_light`: a soft wash from one corner toward a tint of the accent.
//!
//! Every colour is the ground mixed toward the accent or the ink, and the mix
//! is walked back until the palette's ink and accent still read on it
//! ([`safe_mix`]): contrast is the family's constraint, not an afterthought.
//! A beat with a footage plate or a full-bleed picture covers the ground; the
//! plate is its background (`plates.rs`), no family layer is needed there.
//! The light and shade pools are stacks of translucent ellipses (the scene has
//! no gradient primitive); animated grain over them hides the steps.

use super::explore::contrast_ratio;
use super::{base_layer, mix, mo, rect_layer, round3, texture_layer, BeatPlan, Ctx};
use crate::checks::{TEXT_CONTRAST_BODY, TEXT_CONTRAST_DISPLAY};
use crate::easing::Easing;
use crate::scene::{Color, Layer, LayerKind, Material, Motion, Scene, TextureSpec};

/// A backdrop family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Ink,
    Paper,
    ColourBlock,
    GradientLight,
}

/// Every family a video can be set on, in a fixed order.
pub const FAMILIES: [Family; 4] = [
    Family::Ink,
    Family::Paper,
    Family::ColourBlock,
    Family::GradientLight,
];

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Ink => "ink",
            Family::Paper => "paper",
            Family::ColourBlock => "colour_block",
            Family::GradientLight => "gradient_light",
        }
    }

    pub fn from_name(name: &str) -> Option<Family> {
        FAMILIES.iter().copied().find(|f| f.name() == name)
    }
}

/// The family of a video: hashed from the seed, the story and the tone, from
/// the families not in `avoid` (the channel's last renders; only the last
/// three count). Pure.
pub fn choose(seed: u64, story_key: u64, tone: &str, avoid: &[String]) -> Family {
    let recent: Vec<Family> = avoid
        .iter()
        .rev()
        .take(3)
        .filter_map(|n| Family::from_name(n))
        .collect();
    let allowed: Vec<Family> = FAMILIES
        .iter()
        .copied()
        .filter(|f| !recent.contains(f))
        .collect();
    let pool = if allowed.is_empty() {
        FAMILIES.to_vec()
    } else {
        allowed
    };
    let tone_hash = tone.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    });
    let h = mix(mix(mix(seed, 0x6A0D_0001), story_key), tone_hash);
    pool[(h % pool.len() as u64) as usize]
}

/// Beat index ranges `[start, end)` of the ground groups: 2-3 beats each.
pub fn groups(n: usize) -> Vec<(usize, usize)> {
    if n == 0 {
        return Vec::new();
    }
    let count = n.div_ceil(3).max(1);
    let (base, extra) = (n / count, n % count);
    let mut out = Vec::new();
    let mut at = 0;
    for g in 0..count {
        let len = base + usize::from(g < extra);
        out.push((at, at + len));
        at += len;
    }
    out
}

/// `a` mixed toward `b` by `t` (0..=1), opaque.
fn lerp(a: Color, b: Color, t: f32) -> Color {
    let f = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgb(f(a.r, b.r), f(a.g, b.g), f(a.b, b.b))
}

fn luma(c: Color) -> f32 {
    (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
}

/// The ground mixed toward `toward` by `t`, walked back until the ink reads on
/// it and the accent still reads as display type.
fn safe_mix(ctx: &Ctx, toward: Color, t: f32) -> Color {
    let (paper, ink, accent) = (ctx.palette.paper, ctx.palette.ink, ctx.palette.accent);
    let mut t = t;
    loop {
        let c = lerp(paper, toward, t);
        let ok = contrast_ratio(ink, c) as f64 >= TEXT_CONTRAST_BODY
            && contrast_ratio(accent, c) as f64 >= TEXT_CONTRAST_DISPLAY;
        if ok || t <= 0.0 {
            return if ok { c } else { paper };
        }
        t -= 0.02;
    }
}

/// A soft pool of `color` centred on `(cx, cy)`: `steps` concentric
/// translucent ellipses of radii `(rx, ry)` down to a fifth of it, `peak`
/// (0..1) opaque at the centre.
#[allow(clippy::too_many_arguments)]
fn pool(
    id: &str,
    (cx, cy): (f32, f32),
    (rx, ry): (f32, f32),
    color: Color,
    peak: f32,
    steps: u32,
    z: i32,
    out: &mut Vec<Layer>,
) {
    let each = ((peak * 255.0 / steps as f32).round() as u8).max(1);
    for k in 0..steps {
        let f = 1.0 - 0.8 * k as f32 / steps as f32;
        let (w, h) = (2.0 * rx * f, 2.0 * ry * f);
        let mut l = base_layer(
            format!("{id}.{k}"),
            (cx, cy, w, h),
            LayerKind::RoundedRectangle {
                fill: color.with_alpha(each),
                radius: w.min(h) / 2.0,
                stroke: None,
            },
            z,
        );
        l.anchor_x = 0.5;
        l.anchor_y = 0.5;
        out.push(l);
    }
}

/// The layers of one ground variant, as children of a full-canvas group.
fn variant(ctx: &Ctx, family: Family, v: usize, id: &str) -> Vec<Layer> {
    let (w, h) = (ctx.w, ctx.h);
    let (paper, ink, accent) = (ctx.palette.paper, ctx.palette.ink, ctx.palette.accent);
    let dark = luma(paper) < 0.5;
    let mut out = Vec::new();
    match family {
        Family::Ink => {
            // Light falls from one side; on a light ground the far side shades.
            let origin: (f32, f32) = [(0.18, 0.28), (0.86, 0.38), (0.5, 1.05)][v % 3];
            let (ox, oy) = if dark {
                origin
            } else {
                (1.0 - origin.0, (1.0 - origin.1).max(0.0))
            };
            let tone = safe_mix(ctx, ink, if dark { 0.3 } else { 0.34 });
            let color = lerp(paper, tone, 1.0);
            pool(
                &format!("{id}.light"),
                (ox * w, oy * h),
                (0.7 * w, 0.8 * h),
                color,
                1.0,
                14,
                1,
                &mut out,
            );
        }
        Family::Paper => {
            out.push(texture_layer(
                &format!("{id}.fibre"),
                (0.0, 0.0, w, h),
                TextureSpec {
                    material: Material::Paper,
                    seed: mix(ctx.style.seed, 0xFAB1),
                    color: paper,
                    intensity: 0.9,
                    scale: 8.0,
                    animated: false,
                },
                0,
            ));
            // The shadow side: left, right, then below.
            let (ox, oy) = [(-0.12, 0.5), (1.12, 0.5), (0.5, 1.2)][v % 3];
            let shade = safe_mix(ctx, if dark { Color::rgb(0, 0, 0) } else { ink }, 0.5);
            pool(
                &format!("{id}.shade"),
                (ox * w, oy * h),
                (0.55 * w, 0.8 * h),
                shade,
                0.85,
                14,
                1,
                &mut out,
            );
        }
        Family::ColourBlock => {
            let field = safe_mix(ctx, accent, if dark { 0.22 } else { 0.26 });
            match v % 3 {
                0 => out.push(rect_layer(
                    format!("{id}.field"),
                    (w * 2.0 / 3.0, 0.0, w / 3.0, h),
                    field,
                    1,
                )),
                1 => {
                    let mut l = rect_layer(
                        format!("{id}.field"),
                        (0.64 * w, 0.5 * h, 0.9 * w, 2.4 * h),
                        field,
                        1,
                    );
                    l.anchor_x = 0.5;
                    l.anchor_y = 0.5;
                    l.rotation_degrees = 12.0;
                    out.push(l);
                }
                _ => out.push(rect_layer(
                    format!("{id}.field"),
                    (0.0, 0.0, w / 3.0, h),
                    field,
                    1,
                )),
            }
        }
        Family::GradientLight => {
            let wash = safe_mix(ctx, accent, if dark { 0.3 } else { 0.34 });
            let (ox, oy) = [(1.05, -0.05), (-0.05, 1.05), (0.5, 0.5)][v % 3];
            pool(
                &format!("{id}.wash"),
                (ox * w, oy * h),
                (1.15 * w, 1.15 * h),
                wash,
                0.95,
                18,
                1,
                &mut out,
            );
        }
    }
    out
}

/// Add the family's grounds to `scene` (the backdrop scene): one full-canvas
/// group per beat group, cross-faded at the group boundaries, over the base
/// ground and under everything else. Returns the number of distinct grounds.
pub(crate) fn apply(
    ctx: &Ctx,
    family: Family,
    plans: &[BeatPlan],
    total: f64,
    scene: &mut Scene,
) -> usize {
    let spans = groups(plans.len());
    if spans.is_empty() {
        return 0;
    }
    // Neighbouring groups never share a variant; the start is the seed's.
    let start = (mix(ctx.style.seed, 0x6A0D_0002) % 3) as usize;
    let at = scene
        .layers
        .iter()
        .position(|l| l.id == "backdrop.paper")
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut groups: Vec<Layer> = Vec::new();
    let mut motions: Vec<Motion> = Vec::new();
    let fade = 0.8;
    for (g, (a, b)) in spans.iter().enumerate() {
        let id = format!("backdrop.ground{g}");
        let children = variant(ctx, family, start + g, &id);
        let mut group = base_layer(
            id.clone(),
            (0.0, 0.0, ctx.w, ctx.h),
            LayerKind::Group { children },
            1,
        );
        let t0 = plans[*a].start;
        let t1 = plans[*b - 1].start + plans[*b - 1].duration;
        if g > 0 {
            group.opacity = 0.0;
            motions.push(mo::fade(
                &id,
                (t0 - fade / 2.0).max(0.0),
                fade,
                0.0,
                1.0,
                Easing::InOutCubic,
            ));
        }
        if g + 1 < spans.len() {
            motions.push(mo::fade(
                &id,
                round3(t1 - fade / 2.0).max(0.0),
                fade,
                1.0,
                0.0,
                Easing::InOutCubic,
            ));
        }
        groups.push(group);
    }
    let _ = total;
    let n = groups.len();
    for (i, g) in groups.into_iter().enumerate() {
        scene.layers.insert(at + i, g);
    }
    scene.motions.extend(motions);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_are_two_or_three_beats_and_cover_every_beat() {
        for n in 1..=14 {
            let g = groups(n);
            assert_eq!(g.first().map(|x| x.0), Some(0));
            assert_eq!(g.last().map(|x| x.1), Some(n));
            for w in g.windows(2) {
                assert_eq!(w[0].1, w[1].0);
            }
            if n >= 2 {
                assert!(
                    g.iter().all(|(a, b)| (2..=3).contains(&(b - a))),
                    "{n}: {g:?}"
                );
            }
        }
        assert_eq!(groups(9), vec![(0, 3), (3, 6), (6, 9)]);
    }

    #[test]
    fn the_family_is_deterministic_and_avoids_the_channels_last() {
        let a = choose(11, 99, "editorial", &[]);
        assert_eq!(a, choose(11, 99, "editorial", &[]));
        let b = choose(11, 99, "editorial", &[a.name().to_string()]);
        assert_ne!(a, b);
        let three: Vec<String> = FAMILIES[..3].iter().map(|f| f.name().to_string()).collect();
        assert_eq!(choose(1, 2, "x", &three), FAMILIES[3]);
    }
}
