//! Lockup: the closing typographic lockup. A headline in the display face,
//! an optional line under it in the serif, and three hairline rules (above
//! the headline, on its baseline, under the line) that arrive bent along a
//! curve and spring straight into baselines. `sheen` sends one specular
//! highlight across the headline (two masked copies of the type, a core and
//! a soft halo, in the accent), then everything holds.

use motion_core::easing::Easing;
use motion_core::path;
use motion_core::scene::{
    Fill, FontRole, GlyphOrder, GlyphPose, Layer, LayerKind, LayerMask, MotionOp, PathFillRule,
    Scene, Stroke,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lower::{layer, scene, Ctx, Track, Use};
use crate::program::{Aspect, Component, EnterStyle, Swatch, Verb};
use crate::taste::{self, Shape, Spring};

/// Distinct closing lockup ending archetypes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LockupPreset {
    /// Classic horizontal rules springing from `rules_from` with centered headline and subline.
    #[default]
    SplineFrame,
    /// Asymmetric Swiss / Brutalist split colophon: left-aligned brandmark,
    /// vertical accent pillar divider, right-hand bordered mono spec cards + accent status pill.
    MonolithSplit,
    /// Geometric / Scientific circular seal: solid backing disc, concentric precision reticle rings
    /// with cardinal ticks and counter-rotation, centered mark + subline.
    OrbitalSeal,
    /// Developer / CLI command stamp: dark terminal window with header dots, typed CLI prompt
    /// (`> headline`), subline readout, and spec line.
    TerminalPrompt,
    /// Editorial / Luxury framed plaque: double-bordered museum frame, custom top accent badge,
    /// mixed-case or upright headline, short accent bar, and custom subline.
    EditorialBadge,
}

/// Font role selector for a brand's closing lockup headline or subline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LockupFont {
    Display,
    Serif,
    Mono,
    Sans,
}

impl LockupFont {
    pub fn role(self) -> FontRole {
        match self {
            LockupFont::Display => FontRole::Display,
            LockupFont::Serif => FontRole::SerifEmotional,
            LockupFont::Mono => FontRole::Mono,
            LockupFont::Sans => FontRole::Body,
        }
    }
}

/// Letter-casing mode for a brand's closing lockup text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LockupCase {
    Upper,
    Preserve,
}

/// A headline, the line under it and the rules or container that frame them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lockup {
    #[serde(default)]
    pub preset: LockupPreset,
    /// 1..=24 characters, set in the brand's chosen headline face and case.
    pub headline: String,
    /// The line under it, at most 48 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subline: Option<String>,
    /// Optional font role override for the headline (`display`, `serif`, `mono`, `sans`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline_font: Option<LockupFont>,
    /// Optional font role override for the subline (`serif`, `mono`, `sans`, `display`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subline_font: Option<LockupFont>,
    /// Optional letter-case override for the headline (`upper`, `preserve`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline_case: Option<LockupCase>,
    /// Optional letter-case override for the subline (`upper`, `preserve`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subline_case: Option<LockupCase>,
    /// Optional custom badge / status pill label (`"EDITION"`, `"LIVE"`, `"SOVEREIGN"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    /// Where the rules arrive from: a cubic Bézier in frame fractions
    /// (x right, y down; start, two controls, end). They spring straight
    /// into baselines. Absent: they grow out from the centre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_from: Option<[[f32; 2]; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub specs: Vec<String>,
    /// Type colour (default: ink).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<Swatch>,
}

impl Lockup {
    pub fn head_role(&self, default: FontRole) -> FontRole {
        self.headline_font.map_or(default, LockupFont::role)
    }

    pub fn sub_role(&self, default: FontRole) -> FontRole {
        self.subline_font.map_or(default, LockupFont::role)
    }

    pub fn head_upper(&self, default: bool) -> bool {
        match self.headline_case {
            Some(LockupCase::Upper) => true,
            Some(LockupCase::Preserve) => false,
            None => default,
        }
    }

    pub fn sub_upper(&self, default: bool) -> bool {
        match self.subline_case {
            Some(LockupCase::Upper) => true,
            Some(LockupCase::Preserve) => false,
            None => default,
        }
    }

    pub fn check(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let n = self.headline.chars().count();
        if self.headline.trim().is_empty() || n > 24 || self.headline.contains('\n') {
            out.push((
                ".headline".to_string(),
                format!("one line of 1..=24 characters, got {n}"),
            ));
        }
        if let Some(s) = &self.subline {
            let n = s.chars().count();
            if s.trim().is_empty() || n > 48 || s.contains('\n') {
                out.push((
                    ".subline".to_string(),
                    format!("one line of 1..=48 characters, got {n}"),
                ));
            }
        }
        if let Some(b) = &self.badge {
            let n = b.chars().count();
            if b.trim().is_empty() || n > 18 || b.contains('\n') {
                out.push((
                    ".badge".to_string(),
                    format!("one line of 1..=18 characters, got {n}"),
                ));
            }
        }
        if let Some(c) = &self.rules_from {
            if c.iter().flatten().any(|v| !v.is_finite() || v.abs() > 2.0) {
                out.push((
                    ".rules_from".to_string(),
                    "four points in frame fractions (-2..=2)".to_string(),
                ));
            }
        }
        if self.specs.len() > 4 {
            out.push((
                ".specs".to_string(),
                format!("at most 4 specs, got {}", self.specs.len()),
            ));
        }
        for (i, s) in self.specs.iter().enumerate() {
            let n = s.chars().count();
            if s.trim().is_empty() || n > 36 || s.contains('\n') {
                out.push((
                    format!(".specs[{i}]"),
                    format!("one line of 1..=36 characters, got {n}"),
                ));
            }
        }
        out
    }
}

fn cubic(p: [[f32; 2]; 4]) -> String {
    format!(
        "M{:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
        p[0][0], p[0][1], p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1]
    )
}

/// A slanted band `b` wide whose left foot is at `x0` (box space).
fn band(x0: f32, b: f32, slant: f32, h: f32) -> String {
    format!(
        "M{:.1},0 L{:.1},0 L{:.1},{h:.1} L{:.1},{h:.1} Z",
        x0 + slant,
        x0 + slant + b,
        x0 + b,
        x0
    )
}

fn apply_sheen(
    c: &Component,
    cx: &Ctx,
    z: i32,
    head: &Layer,
    enter_at: f64,
    layers: &mut Vec<Layer>,
    track: &mut Track,
) {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    for e in &c.timeline {
        let (Verb::Sheen, Some(at)) = (&e.verb, e.at) else {
            continue;
        };
        let d = taste::duration(&e.verb, cx.tempo);
        let (w, h) = (head.width, head.height);
        let slant = h * 0.35;
        for (k, (bw, op)) in [(w * 0.22, 0.32f32), ((w * 0.07).max(h * 0.25), 1.0)]
            .into_iter()
            .enumerate()
        {
            let cid = format!("{id}.sheen{k}");
            let mut copy = head.clone();
            copy.id = cid.clone();
            copy.z_index = z + 3 + k as i32;
            copy.opacity = op;
            if let LayerKind::Text(t) = &mut copy.kind {
                t.color = cx.roles(id).accent_text;
            }
            copy.mask = Some(LayerMask {
                d: band(-bw - slant - 2.0, bw, slant, h),
                invert: false,
            });
            track.push(
                &cid,
                at,
                d,
                Shape::Ease(Easing::InOutCubic),
                MotionOp::MaskMorph {
                    to: band(w + 2.0, bw, slant, h),
                },
            );
            track.push(
                &cid,
                enter_at,
                0.32 * tempo,
                Shape::Spring(Spring::Snap),
                MotionOp::Scale {
                    from: 1.06,
                    to: 1.0,
                    axis: Default::default(),
                },
            );
            layers.push(copy);
        }
    }
}

pub(crate) fn lower(cx: &Ctx, c: &Component, l: &Lockup, z: i32, length: f64) -> Vec<Scene> {
    match l.preset {
        LockupPreset::SplineFrame => lower_spline_frame(cx, c, l, z, length),
        LockupPreset::MonolithSplit => lower_monolith_split(cx, c, l, z, length),
        LockupPreset::OrbitalSeal => lower_orbital_seal(cx, c, l, z, length),
        LockupPreset::TerminalPrompt => lower_terminal_prompt(cx, c, l, z, length),
        LockupPreset::EditorialBadge => lower_editorial_badge(cx, c, l, z, length),
    }
}

fn lower_spline_frame(cx: &Ctx, c: &Component, l: &Lockup, z: i32, length: f64) -> Vec<Scene> {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let enter_at = c
        .timeline
        .iter()
        .find(|e| {
            matches!(
                e.verb,
                Verb::Enter {
                    style: EnterStyle::Pop
                }
            )
        })
        .and_then(|e| e.at)
        .unwrap_or(0.0);

    // Measure.
    let h_role = l.head_role(FontRole::Display);
    let h_up = l.head_upper(true);
    let s_role = l.sub_role(FontRole::SerifEmotional);
    let s_up = l.sub_upper(false);
    let (ls_h, ls_s) = (0.04, 0.0);
    let (max_w, max_hs, max_ss) = if cx.aspect == Aspect::Portrait {
        (
            cx.w * 0.84,
            (cx.short * 0.28).round(),
            (cx.short * 0.068).round(),
        )
    } else {
        (
            cx.w * 0.6,
            (cx.short * 0.25).round(),
            (cx.short * 0.058).round(),
        )
    };
    let hs = cx.fit(&l.headline, h_role, ls_h, h_up, max_w, max_hs);
    let h_ink = cx.ink_height(&l.headline, h_role, hs, ls_h, h_up);
    let h_w = cx.line_width(&l.headline, h_role, hs, ls_h, h_up);
    let sub = l.subline.as_deref();
    let ss = sub.map_or(0.0, |s| cx.fit(s, s_role, ls_s, s_up, h_w, max_ss));
    let s_ink = sub.map_or(0.0, |s| cx.ink_height(s, s_role, ss, ls_s, s_up));

    // Stack (ink boxes): rule · headline · rule · subline · rule.
    let gap = if cx.aspect == Aspect::Portrait {
        (h_ink * 0.55).round().max((cx.h * 0.042).round())
    } else {
        (h_ink * 0.42).round()
    };
    let sgap = (gap * 0.75).round();
    let total = if sub.is_some() {
        gap + h_ink + gap + sgap + s_ink + sgap
    } else {
        gap + h_ink + gap
    };
    let y0 = (cx.h / 2.0 - total / 2.0).round();
    let y_top = y0;
    let y_head = y0 + gap + h_ink / 2.0;
    let y_mid = y0 + gap + h_ink + gap;
    let y_sub = y_mid + sgap + s_ink / 2.0;
    let y_bot = y_mid + sgap + s_ink + sgap;
    let mut rule_ys = vec![y_top, y_mid];
    if sub.is_some() {
        rule_ys.push(y_bot);
    }

    let colour = l.colour.unwrap_or(Swatch::Ink);
    let ink = cx.paint(id, colour, Use::Text(hs));
    let sub_ink = cx.paint(id, colour, Use::Text(ss.max(1.0)));
    // Rules frame the type, quieter than it (strong structure, >= 3:1).
    let rule_col = cx.roles(id).structure_strong;
    let stroke = (cx.short / 540.0).round().max(2.0);

    let mut layers = Vec::new();
    let mut track = Track::default();
    let fps = cx.fps;

    // Rules: bent along `rules_from`, springing straight.
    let rule_w = (h_w * 1.16).round();
    let bx = (cx.w / 2.0 - rule_w / 2.0).round();
    let bh = stroke * 4.0;
    let mid_y = (rule_ys[0] + rule_ys[rule_ys.len() - 1]) / 2.0;
    for (k, ry) in rule_ys.iter().enumerate() {
        let rid = format!("{id}.rule{k}");
        let by = ry - bh / 2.0;
        let flat = cubic([
            [0.0, bh / 2.0],
            [rule_w / 3.0, bh / 2.0],
            [rule_w * 2.0 / 3.0, bh / 2.0],
            [rule_w, bh / 2.0],
        ]);
        let start = match l.rules_from {
            Some(p) => {
                let dy = (ry - mid_y) * 0.25;
                cubic(p.map(|q| [q[0] * cx.w - bx, q[1] * cx.h + dy - by]))
            }
            None => cubic([[rule_w / 2.0, bh / 2.0]; 4]),
        };
        let mut rl = layer(
            &rid,
            LayerKind::Path {
                d: start,
                fill: None,
                stroke: Some(Stroke {
                    color: rule_col,
                    width: stroke,
                    brush: None,
                }),
                fill_rule: Default::default(),
                shadows: Vec::new(),
            },
            [cx.w / 2.0, *ry],
            [rule_w, bh],
            z,
        );
        rl.anchor_y = 0.5;
        layers.push(rl);
        track.push(
            &rid,
            enter_at + k as f64 * 0.025,
            0.3 * tempo,
            Shape::Spring(Spring::Pop),
            MotionOp::Morph { to: flat },
        );
        if enter_at > 0.0 {
            track.push(
                &rid,
                enter_at,
                1.0 / fps,
                Shape::Ease(Easing::Linear),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }
    }

    // Headline: lands with the impact (a small settle), no fade.
    let hid = format!("{id}.headline");
    let head = cx.text(
        &hid,
        &l.headline,
        h_role,
        hs,
        ink,
        ls_h,
        h_up,
        [cx.w / 2.0, y_head],
        z + 2,
    );
    track.push(
        &hid,
        enter_at,
        0.32 * tempo,
        Shape::Spring(Spring::Snap),
        MotionOp::Scale {
            from: 1.06,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &hid,
            enter_at,
            1.0 / fps,
            Shape::Ease(Easing::Linear),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // Subline: rises in with the rules.
    if let Some(s) = sub {
        let sid = format!("{id}.subline");
        let sl = cx.text(
            &sid,
            s,
            s_role,
            ss,
            sub_ink,
            ls_s,
            s_up,
            [cx.w / 2.0, y_sub],
            z + 2,
        );
        let at = enter_at;
        track.push(
            &sid,
            at,
            0.5 * tempo,
            Shape::Spring(Spring::Settle),
            MotionOp::Move {
                from: [0.0, (s_ink * 0.6).round()],
                to: [0.0, 0.0],
                via: None,
            },
        );
        track.push(
            &sid,
            at,
            taste::QUICK_FADE * 2.0,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        layers.push(sl);
    }

    apply_sheen(c, cx, z, &head, enter_at, &mut layers, &mut track);
    layers.push(head);

    vec![scene(id, c.start, length, layers, track.motions)]
}

fn lower_monolith_split(cx: &Ctx, c: &Component, l: &Lockup, z: i32, length: f64) -> Vec<Scene> {
    if cx.aspect == Aspect::Portrait {
        return lower_monolith_split_portrait(cx, c, l, z, length);
    }

    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let enter_at = c
        .timeline
        .iter()
        .find(|e| matches!(e.verb, Verb::Enter { .. }))
        .and_then(|e| e.at)
        .unwrap_or(0.0);
    let r = cx.roles(id);
    let mut layers = Vec::new();
    let mut track = Track::default();

    // 1. Container card
    let w = (cx.w * 0.82).round();
    let h = (cx.h * 0.56).round();
    let card_radius = (cx.short * 0.024).round();
    let card_d = path::rounded_rect(0.0, 0.0, w, h, card_radius).to_svg();
    let card_id = format!("{id}.card");
    let card_layer = layer(
        &card_id,
        LayerKind::Path {
            d: card_d,
            fill: Some(Fill::Solid(r.ground)),
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: 2.0,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [w, h],
        z,
    );
    layers.push(card_layer);
    track.push(
        &card_id,
        enter_at,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.94,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &card_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // 2. Vertical accent pillar divider at Golden Section (61.8% / 38.2%)
    let card_left = (cx.w / 2.0 - w / 2.0).round();
    let major_w = (w * taste::INV_PHI).round();
    let minor_w = w - major_w;
    let div_x = (card_left + major_w).round();
    let y0 = (cx.h / 2.0 - h * 0.36).round();
    let y1 = (cx.h / 2.0 + h * 0.36).round();
    let div_w = (cx.short * 0.006).max(3.0).round();
    let div_id = format!("{id}.divider");
    let mut div_layer = layer(
        &div_id,
        LayerKind::Path {
            d: format!("M{div_x},{y0} L{div_x},{y1}"),
            fill: None,
            stroke: Some(Stroke {
                color: r.accent,
                width: div_w,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [0.0, 0.0],
        [cx.w, cx.h],
        z + 1,
    );
    div_layer.anchor_x = 0.0;
    div_layer.anchor_y = 0.0;
    layers.push(div_layer);
    track.push(
        &div_id,
        enter_at,
        0.45 * tempo,
        Shape::Ease(Easing::OutCubic),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );
    if enter_at > 0.0 {
        track.push(
            &div_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::Linear),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // 3. Left column (Major Golden Section 61.8%)
    let left_w = major_w;
    let left_cx = (card_left + left_w / 2.0).round();
    let head_role = l.head_role(FontRole::Display);
    let head_upper = l.head_upper(true);
    let max_head_fs = (cx.short * 0.14).round().max(24.0);
    let mut head_fs = cx.fit(
        &l.headline,
        head_role,
        -0.02,
        head_upper,
        left_w * 0.90,
        max_head_fs,
    );
    while cx.line_width(&l.headline, head_role, head_fs, -0.02, head_upper) > left_w * 0.90
        && head_fs > 14.0
    {
        head_fs -= 1.0;
    }
    let colour = l.colour.unwrap_or(Swatch::Ink);
    let ink = cx.paint(id, colour, Use::Text(head_fs));
    let hid = format!("{id}.headline");
    let head = cx.text(
        &hid,
        &l.headline,
        head_role,
        head_fs,
        ink,
        -0.02,
        head_upper,
        [left_cx, (cx.h * 0.44).round()],
        z + 2,
    );
    track.push(
        &hid,
        enter_at,
        0.32 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.92,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &hid,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    if let Some(ref s) = l.subline {
        let sub_role = l.sub_role(FontRole::SerifEmotional);
        let sub_upper = l.sub_upper(false);
        let sub_ls = if sub_upper { 0.05 } else { 0.0 };
        let max_sub_fs = (cx.short * 0.048).round().max(16.0);
        let mut sub_fs = cx.fit(s, sub_role, sub_ls, sub_upper, left_w * 0.90, max_sub_fs);
        while cx.line_width(s, sub_role, sub_fs, sub_ls, sub_upper) > left_w * 0.90 && sub_fs > 12.0
        {
            sub_fs -= 1.0;
        }
        let sub_ink = cx.paint(id, colour, Use::Text(sub_fs));
        let sid = format!("{id}.subline");
        let sl = cx.text(
            &sid,
            s,
            sub_role,
            sub_fs,
            sub_ink,
            sub_ls,
            sub_upper,
            [left_cx, (cx.h * 0.56).round()],
            z + 2,
        );
        track.push(
            &sid,
            enter_at + 0.05,
            0.35 * tempo,
            Shape::Spring(Spring::Settle),
            MotionOp::Move {
                from: [0.0, 16.0],
                to: [0.0, 0.0],
                via: None,
            },
        );
        track.push(
            &sid,
            enter_at + 0.05,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        layers.push(sl);
    }

    // 4. Right column specs (Minor Golden Section 38.2%)
    let right_w = minor_w;
    let right_cx = (div_x + right_w / 2.0).round();
    let specs = if l.specs.is_empty() {
        vec!["STATUS: VERIFIED".to_string()]
    } else {
        l.specs.clone()
    };
    for (i, spec) in specs.iter().take(3).enumerate() {
        let spec_id = format!("{id}.spec{i}");
        let base_spec_fs = (cx.short * 0.024).clamp(16.0, 28.0).round();
        let mut spec_fs = cx.fit(
            spec,
            FontRole::Mono,
            0.04,
            true,
            right_w * 0.86,
            base_spec_fs,
        );
        while cx.line_width(spec, FontRole::Mono, spec_fs, 0.04, true) > right_w * 0.86
            && spec_fs > 11.0
        {
            spec_fs -= 1.0;
        }
        let spec_y = (cx.h * 0.38 + i as f32 * (cx.h * 0.10)).round();
        let spec_layer = cx.text(
            &spec_id,
            spec,
            FontRole::Mono,
            spec_fs,
            r.ink,
            0.04,
            true,
            [right_cx, spec_y],
            z + 2,
        );
        track.push(
            &spec_id,
            enter_at + 0.08 * i as f64,
            0.3 * tempo,
            Shape::Spring(Spring::Pop),
            MotionOp::Scale {
                from: 0.92,
                to: 1.0,
                axis: Default::default(),
            },
        );
        if enter_at > 0.0 {
            track.push(
                &spec_id,
                enter_at + 0.08 * i as f64,
                taste::QUICK_FADE,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }
        layers.push(spec_layer);
    }

    // 5. Right column accent status pill
    let badge_text = l.badge.as_deref().unwrap_or("VERIFIED");
    let pill_w = (right_w * 0.64).round();
    let pill_h = (cx.short * 0.052).round();
    let pill_d = path::rounded_rect(0.0, 0.0, pill_w, pill_h, pill_h * 0.5).to_svg();
    let pill_id = format!("{id}.pill");
    let pill_layer = layer(
        &pill_id,
        LayerKind::Path {
            d: pill_d,
            fill: Some(Fill::Solid(r.accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [right_cx, (cx.h * 0.64).round()],
        [pill_w, pill_h],
        z + 1,
    );
    layers.push(pill_layer);
    track.push(
        &pill_id,
        enter_at + 0.15,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.8,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &pill_id,
            enter_at + 0.15,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    let base_pill_fs = (pill_h * 0.44).round().clamp(12.0, 20.0);
    let mut pill_fs = cx.fit(
        badge_text,
        FontRole::Mono,
        0.06,
        true,
        pill_w * 0.84,
        base_pill_fs,
    );
    while cx.line_width(badge_text, FontRole::Mono, pill_fs, 0.06, true) > pill_w * 0.84
        && pill_fs > 10.0
    {
        pill_fs -= 1.0;
    }
    let pill_txt_col = cx.col.roles(r.accent).text(r.accent, pill_fs);
    let pill_txt_id = format!("{id}.pill_text");
    let pill_txt = cx.text(
        &pill_txt_id,
        badge_text,
        FontRole::Mono,
        pill_fs,
        pill_txt_col,
        0.06,
        true,
        [right_cx, (cx.h * 0.64).round()],
        z + 2,
    );
    track.push(
        &pill_txt_id,
        enter_at + 0.15,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.8,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &pill_txt_id,
            enter_at + 0.15,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }
    layers.push(pill_txt);

    // Sheen on headline
    apply_sheen(c, cx, z, &head, enter_at, &mut layers, &mut track);
    layers.push(head);

    vec![scene(id, c.start, length, layers, track.motions)]
}

fn lower_monolith_split_portrait(
    cx: &Ctx,
    c: &Component,
    l: &Lockup,
    z: i32,
    length: f64,
) -> Vec<Scene> {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let enter_at = c
        .timeline
        .iter()
        .find(|e| matches!(e.verb, Verb::Enter { .. }))
        .and_then(|e| e.at)
        .unwrap_or(0.0);
    let r = cx.roles(id);
    let mut layers = Vec::new();
    let mut track = Track::default();

    // 1. Container card: w = cx.w * 0.88, h = cx.h * 0.72
    let w = (cx.w * 0.88).round();
    let h = (cx.h * 0.72).round();
    let card_radius = (cx.short * 0.024).round();
    let card_d = path::rounded_rect(0.0, 0.0, w, h, card_radius).to_svg();
    let card_id = format!("{id}.card");
    let card_layer = layer(
        &card_id,
        LayerKind::Path {
            d: card_d,
            fill: Some(Fill::Solid(r.ground)),
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: 2.0,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [w, h],
        z,
    );
    layers.push(card_layer);
    track.push(
        &card_id,
        enter_at,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.94,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &card_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // 2. Horizontal accent pillar divider bar spanning w * 0.84 at div_y = (cx.h * 0.5 - h * 0.04).round()
    let div_y = (cx.h * 0.5 - h * 0.04).round();
    let div_span = (w * 0.84).round();
    let div_x0 = (cx.w * 0.5 - div_span * 0.5).round();
    let div_x1 = (cx.w * 0.5 + div_span * 0.5).round();
    let div_w = (cx.short * 0.006).max(3.0).round();
    let div_id = format!("{id}.divider");
    let mut div_layer = layer(
        &div_id,
        LayerKind::Path {
            d: format!("M{div_x0},{div_y} L{div_x1},{div_y}"),
            fill: None,
            stroke: Some(Stroke {
                color: r.accent,
                width: div_w,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [0.0, 0.0],
        [cx.w, cx.h],
        z + 1,
    );
    div_layer.anchor_x = 0.0;
    div_layer.anchor_y = 0.0;
    layers.push(div_layer);
    track.push(
        &div_id,
        enter_at,
        0.45 * tempo,
        Shape::Ease(Easing::OutCubic),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );
    if enter_at > 0.0 {
        track.push(
            &div_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::Linear),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // 3. Top chamber (y < div_y): centered at [cx.w * 0.5, cx.h * 0.5 - h * 0.24]
    let content_w = (w * 0.84).round();
    let head_role = l.head_role(FontRole::Display);
    let head_upper = l.head_upper(true);
    let base_head_fs = (h * 0.065).clamp(48.0, 84.0).round();
    let mut head_fs = cx.fit(
        &l.headline,
        head_role,
        -0.02,
        head_upper,
        content_w,
        base_head_fs,
    );
    while cx.line_width(&l.headline, head_role, head_fs, -0.02, head_upper) > content_w
        && head_fs > 24.0
    {
        head_fs -= 1.0;
    }
    let colour = l.colour.unwrap_or(Swatch::Ink);
    let ink = cx.paint(id, colour, Use::Text(head_fs));

    let (head_y, sub_y) = if l.subline.is_some() {
        (
            (cx.h * 0.5 - h * 0.28).round(),
            (cx.h * 0.5 - h * 0.16).round(),
        )
    } else {
        (
            (cx.h * 0.5 - h * 0.24).round(),
            (cx.h * 0.5 - h * 0.16).round(),
        )
    };

    let hid = format!("{id}.headline");
    let head = cx.text(
        &hid,
        &l.headline,
        head_role,
        head_fs,
        ink,
        -0.02,
        head_upper,
        [cx.w * 0.5, head_y],
        z + 2,
    );
    track.push(
        &hid,
        enter_at,
        0.32 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.92,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &hid,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    if let Some(ref s) = l.subline {
        let sub_role = l.sub_role(FontRole::SerifEmotional);
        let sub_upper = l.sub_upper(false);
        let sub_ls = if sub_upper { 0.05 } else { 0.0 };
        let base_sub_fs = (h * 0.028).clamp(22.0, 36.0).round();
        let mut sub_fs = cx.fit(s, sub_role, sub_ls, sub_upper, content_w, base_sub_fs);
        while cx.line_width(s, sub_role, sub_fs, sub_ls, sub_upper) > content_w && sub_fs > 14.0 {
            sub_fs -= 1.0;
        }
        let sub_ink = cx.paint(id, colour, Use::Text(sub_fs));
        let sid = format!("{id}.subline");
        let sl = cx.text(
            &sid,
            s,
            sub_role,
            sub_fs,
            sub_ink,
            sub_ls,
            sub_upper,
            [cx.w * 0.5, sub_y],
            z + 2,
        );
        track.push(
            &sid,
            enter_at + 0.05,
            0.35 * tempo,
            Shape::Spring(Spring::Settle),
            MotionOp::Move {
                from: [0.0, 16.0],
                to: [0.0, 0.0],
                via: None,
            },
        );
        track.push(
            &sid,
            enter_at + 0.05,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        layers.push(sl);
    }

    // 4. Bottom chamber (y > div_y): centered at cx.w * 0.5, rendering up to 3 specs rows
    let specs = if l.specs.is_empty() {
        vec!["STATUS: VERIFIED".to_string()]
    } else {
        l.specs.clone()
    };
    let specs_to_render = specs.iter().take(3).collect::<Vec<_>>();
    let n_specs = specs_to_render.len();
    for (i, spec) in specs_to_render.into_iter().enumerate() {
        let spec_id = format!("{id}.spec{i}");
        let base_spec_fs = (cx.short * 0.024).clamp(16.0, 28.0).round();
        let mut spec_fs = cx.fit(spec, FontRole::Mono, 0.04, true, content_w, base_spec_fs);
        while cx.line_width(spec, FontRole::Mono, spec_fs, 0.04, true) > content_w && spec_fs > 11.0
        {
            spec_fs -= 1.0;
        }
        let spec_y = match n_specs {
            1 => (cx.h * 0.5 + h * 0.14).round(),
            2 => (cx.h * 0.5 + h * 0.08 + i as f32 * (h * 0.12)).round(),
            _ => (cx.h * 0.5 + h * 0.06 + i as f32 * (h * 0.09)).round(),
        };
        let spec_layer = cx.text(
            &spec_id,
            spec,
            FontRole::Mono,
            spec_fs,
            r.ink,
            0.04,
            true,
            [cx.w * 0.5, spec_y],
            z + 2,
        );
        track.push(
            &spec_id,
            enter_at + 0.08 * i as f64,
            0.3 * tempo,
            Shape::Spring(Spring::Pop),
            MotionOp::Scale {
                from: 0.92,
                to: 1.0,
                axis: Default::default(),
            },
        );
        if enter_at > 0.0 {
            track.push(
                &spec_id,
                enter_at + 0.08 * i as f64,
                taste::QUICK_FADE,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }
        layers.push(spec_layer);
    }

    // 5. Accent status pill at [cx.w * 0.5, cx.h * 0.5 + h * 0.36]
    let badge_text = l.badge.as_deref().unwrap_or("VERIFIED");
    let pill_w = (content_w * 0.55).round().max(200.0);
    let pill_h = (cx.short * 0.052).round().max(42.0);
    let pill_d = path::rounded_rect(0.0, 0.0, pill_w, pill_h, pill_h * 0.5).to_svg();
    let pill_id = format!("{id}.pill");
    let pill_pos = [cx.w * 0.5, (cx.h * 0.5 + h * 0.36).round()];
    let pill_layer = layer(
        &pill_id,
        LayerKind::Path {
            d: pill_d,
            fill: Some(Fill::Solid(r.accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        pill_pos,
        [pill_w, pill_h],
        z + 1,
    );
    layers.push(pill_layer);
    track.push(
        &pill_id,
        enter_at + 0.15,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.8,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &pill_id,
            enter_at + 0.15,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    let base_pill_fs = (pill_h * 0.44).round().clamp(12.0, 20.0);
    let mut pill_fs = cx.fit(
        badge_text,
        FontRole::Mono,
        0.06,
        true,
        pill_w * 0.84,
        base_pill_fs,
    );
    while cx.line_width(badge_text, FontRole::Mono, pill_fs, 0.06, true) > pill_w * 0.84
        && pill_fs > 10.0
    {
        pill_fs -= 1.0;
    }
    let pill_txt_col = cx.col.roles(r.accent).text(r.accent, pill_fs);
    let pill_txt_id = format!("{id}.pill_text");
    let pill_txt = cx.text(
        &pill_txt_id,
        badge_text,
        FontRole::Mono,
        pill_fs,
        pill_txt_col,
        0.06,
        true,
        pill_pos,
        z + 2,
    );
    track.push(
        &pill_txt_id,
        enter_at + 0.15,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.8,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &pill_txt_id,
            enter_at + 0.15,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }
    layers.push(pill_txt);

    // Sheen on headline
    apply_sheen(c, cx, z, &head, enter_at, &mut layers, &mut track);
    layers.push(head);

    vec![scene(id, c.start, length, layers, track.motions)]
}

fn lower_orbital_seal(cx: &Ctx, c: &Component, l: &Lockup, z: i32, length: f64) -> Vec<Scene> {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let enter_at = c
        .timeline
        .iter()
        .find(|e| matches!(e.verb, Verb::Enter { .. }))
        .and_then(|e| e.at)
        .unwrap_or(0.0);
    let r = cx.roles(id);
    let mut layers = Vec::new();
    let mut track = Track::default();

    let center_pos = [cx.w / 2.0, cx.h / 2.0];
    let d_outer = (cx.short * 0.74).round();
    let d_inner = (cx.short * 0.58).round();
    let (cx_pos, cy_pos) = (center_pos[0], center_pos[1]);

    // Horizontal and vertical precision architectural datum lines behind the seal (z)
    let x_left_margin = (cx.w * 0.08).round();
    let x_right_margin = (cx.w * 0.92).round();
    let y_top_margin = (cx.h * 0.08).round();
    let y_bottom_margin = (cx.h * 0.92).round();
    let r_datum = (d_outer * 0.54).round();

    let datum_d = format!(
        "M{x_left_margin:.1},{cy_pos:.1} L{:.1},{cy_pos:.1} M{:.1},{cy_pos:.1} L{x_right_margin:.1},{cy_pos:.1} M{cx_pos:.1},{y_top_margin:.1} L{cx_pos:.1},{:.1} M{cx_pos:.1},{:.1} L{cx_pos:.1},{y_bottom_margin:.1}",
        cx_pos - r_datum,
        cx_pos + r_datum,
        cy_pos - r_datum,
        cy_pos + r_datum
    );
    let datum_id = format!("{id}.datum_lines");
    let mut datum_layer = layer(
        &datum_id,
        LayerKind::Path {
            d: datum_d,
            fill: None,
            stroke: Some(Stroke {
                color: r.structure,
                width: 1.5,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [0.0, 0.0],
        [cx.w, cx.h],
        z,
    );
    datum_layer.anchor_x = 0.0;
    datum_layer.anchor_y = 0.0;
    layers.push(datum_layer);
    track.push(
        &datum_id,
        enter_at,
        0.55 * tempo,
        Shape::Ease(Easing::OutCubic),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );
    if enter_at > 0.0 {
        track.push(
            &datum_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::Linear),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    let outer_d = path::ellipse(0.0, 0.0, d_outer, d_outer).to_svg();
    let outer_id = format!("{id}.outer_ring");
    let outer_layer = layer(
        &outer_id,
        LayerKind::Path {
            d: outer_d,
            fill: None,
            stroke: Some(Stroke {
                color: r.accent,
                width: (cx.short * 0.004).max(2.5).round(),
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        center_pos,
        [d_outer, d_outer],
        z,
    );
    layers.push(outer_layer);
    track.push(
        &outer_id,
        enter_at,
        0.6 * tempo,
        Shape::Ease(Easing::InOutCubic),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );
    track.push(
        &outer_id,
        enter_at,
        0.6 * tempo,
        Shape::Ease(Easing::OutCubic),
        MotionOp::Rotate {
            from: -30.0,
            to: 0.0,
        },
    );
    if enter_at > 0.0 {
        track.push(
            &outer_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::Linear),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // Inner solid seal disc
    let inner_d = path::ellipse(0.0, 0.0, d_inner, d_inner).to_svg();
    let disc_id = format!("{id}.seal_disc");
    let disc_layer = layer(
        &disc_id,
        LayerKind::Path {
            d: inner_d,
            fill: Some(Fill::Solid(r.ground)),
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: (cx.short * 0.004).max(2.0).round(),
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        center_pos,
        [d_inner, d_inner],
        z + 1,
    );
    layers.push(disc_layer);
    track.push(
        &disc_id,
        enter_at,
        0.45 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.72,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &disc_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // 4 cardinal crosshair ticks placed outside d_inner
    let r0 = (d_inner * 0.52).round();
    let r1 = (d_outer * 0.54).round();
    let ticks_d = format!(
        "M{cx_pos},{:.1} L{cx_pos},{:.1} M{cx_pos},{:.1} L{cx_pos},{:.1} M{:.1},{cy_pos} L{:.1},{cy_pos} M{:.1},{cy_pos} L{:.1},{cy_pos}",
        cy_pos - r1,
        cy_pos - r0,
        cy_pos + r0,
        cy_pos + r1,
        cx_pos - r1,
        cx_pos - r0,
        cx_pos + r0,
        cx_pos + r1
    );
    let ticks_id = format!("{id}.cardinal_ticks");
    let mut ticks_layer = layer(
        &ticks_id,
        LayerKind::Path {
            d: ticks_d,
            fill: None,
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: (cx.short * 0.003).max(1.5).round(),
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [0.0, 0.0],
        [cx.w, cx.h],
        z + 1,
    );
    ticks_layer.anchor_x = 0.0;
    ticks_layer.anchor_y = 0.0;
    layers.push(ticks_layer);
    track.push(
        &ticks_id,
        enter_at,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.8,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &ticks_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // Optional top mono badge inside d_inner
    let max_text_w = (d_inner * 0.82).round();
    if let Some(ref btxt) = l.badge {
        let base_bfs = (d_inner * 0.048).round().clamp(12.0, 20.0);
        let mut bfs = cx.fit(
            btxt,
            FontRole::Mono,
            0.08,
            true,
            max_text_w * 0.78,
            base_bfs,
        );
        while cx.line_width(btxt, FontRole::Mono, bfs, 0.08, true) > max_text_w * 0.78 && bfs > 10.0
        {
            bfs -= 1.0;
        }
        let bcol = if crate::validate::contrast(r.accent, r.ground) >= 4.5 {
            r.accent
        } else {
            r.ink
        };
        let bid = format!("{id}.badge");
        let bl = cx.text(
            &bid,
            btxt,
            FontRole::Mono,
            bfs,
            bcol,
            0.08,
            true,
            [cx.w / 2.0, (cx.h * 0.37).round()],
            z + 2,
        );
        track.push(
            &bid,
            enter_at + 0.05,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        layers.push(bl);
    }

    // Headline inside d_inner
    let head_role = l.head_role(FontRole::Display);
    let head_upper = l.head_upper(true);
    let max_head_w = (d_inner * 0.82).round();
    let max_head_fs = (d_inner * 0.16).clamp(36.0, 74.0).round();
    let mut head_fs = cx.fit(
        &l.headline,
        head_role,
        0.02,
        head_upper,
        max_head_w,
        max_head_fs,
    );
    while cx.line_width(&l.headline, head_role, head_fs, 0.02, head_upper) > max_head_w
        && head_fs > 14.0
    {
        head_fs -= 1.0;
    }
    let colour = l.colour.unwrap_or(Swatch::Ink);
    let ink = cx.paint(id, colour, Use::Text(head_fs));
    let hid = format!("{id}.headline");
    let head = cx.text(
        &hid,
        &l.headline,
        head_role,
        head_fs,
        ink,
        0.02,
        head_upper,
        [cx.w / 2.0, (cx.h * 0.46).round()],
        z + 2,
    );
    track.push(
        &hid,
        enter_at,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.85,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &hid,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    if let Some(ref s) = l.subline {
        let sub_role = l.sub_role(FontRole::SerifEmotional);
        let sub_upper = l.sub_upper(false);
        let sub_ls = if sub_upper { 0.05 } else { 0.0 };
        let max_sub_w = (d_inner * 0.80).round();
        let max_sub_fs = (d_inner * 0.068).clamp(18.0, 32.0).round();
        let mut sub_fs = cx.fit(s, sub_role, sub_ls, sub_upper, max_sub_w, max_sub_fs);
        while cx.line_width(s, sub_role, sub_fs, sub_ls, sub_upper) > max_sub_w && sub_fs > 11.0 {
            sub_fs -= 1.0;
        }
        let sub_ink = cx.paint(id, colour, Use::Text(sub_fs));
        let sid = format!("{id}.subline");
        let sl = cx.text(
            &sid,
            s,
            sub_role,
            sub_fs,
            sub_ink,
            sub_ls,
            sub_upper,
            [cx.w / 2.0, (cx.h * 0.56).round()],
            z + 2,
        );
        track.push(
            &sid,
            enter_at + 0.05,
            0.35 * tempo,
            Shape::Spring(Spring::Settle),
            MotionOp::Move {
                from: [0.0, 12.0],
                to: [0.0, 0.0],
                via: None,
            },
        );
        track.push(
            &sid,
            enter_at + 0.05,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        layers.push(sl);
    }

    apply_sheen(c, cx, z, &head, enter_at, &mut layers, &mut track);
    layers.push(head);

    vec![scene(id, c.start, length, layers, track.motions)]
}

fn lower_terminal_prompt(cx: &Ctx, c: &Component, l: &Lockup, z: i32, length: f64) -> Vec<Scene> {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let enter_at = c
        .timeline
        .iter()
        .find(|e| matches!(e.verb, Verb::Enter { .. }))
        .and_then(|e| e.at)
        .unwrap_or(0.0);
    let r = cx.roles(id);
    let mut layers = Vec::new();
    let mut track = Track::default();

    // Terminal window card
    let w = (cx.w * 0.78).round();
    let h = (cx.h * 0.54).round();
    let card_radius = (cx.short * 0.022).round();
    let card_d = path::rounded_rect(0.0, 0.0, w, h, card_radius).to_svg();
    let card_id = format!("{id}.terminal_card");
    let card_layer = layer(
        &card_id,
        LayerKind::Path {
            d: card_d,
            fill: Some(Fill::Solid(r.ground)),
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: 2.0,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [w, h],
        z,
    );
    layers.push(card_layer);
    track.push(
        &card_id,
        enter_at,
        0.35 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.94,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &card_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // Top header rule
    let y_hdr = (cx.h / 2.0 - h * 0.34).round();
    let rule_x0 = (cx.w / 2.0 - w * 0.46).round();
    let rule_x1 = (cx.w / 2.0 + w * 0.46).round();
    let hdr_rule_id = format!("{id}.hdr_rule");
    let mut hdr_layer = layer(
        &hdr_rule_id,
        LayerKind::Path {
            d: format!("M{rule_x0},{y_hdr} L{rule_x1},{y_hdr}"),
            fill: None,
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: 1.5,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [0.0, 0.0],
        [cx.w, cx.h],
        z + 1,
    );
    hdr_layer.anchor_x = 0.0;
    hdr_layer.anchor_y = 0.0;
    layers.push(hdr_layer);

    // 3 small header dots
    let d_dot = (cx.short * 0.014).max(10.0).round();
    let dot_y = (y_hdr - d_dot * 1.3).round();
    let dot_colors = [r.accent, r.structure_strong, r.structure_strong];
    for (k, dot_col) in dot_colors.into_iter().enumerate() {
        let dot_id = format!("{id}.hdr_dot{k}");
        let dot_layer = layer(
            &dot_id,
            LayerKind::Path {
                d: path::ellipse(0.0, 0.0, d_dot, d_dot).to_svg(),
                fill: Some(Fill::Solid(dot_col)),
                stroke: None,
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [rule_x0 + d_dot + k as f32 * d_dot * 1.8, dot_y],
            [d_dot, d_dot],
            z + 2,
        );
        layers.push(dot_layer);
    }

    // Command prompt line at [cx.w / 2.0, cx.h * 0.45]
    let prompt_text = format!("> {}", l.headline);
    let max_cmd_fs = (cx.short * 0.052).round();
    let mut cmd_fs = cx.fit(
        &prompt_text,
        FontRole::Mono,
        0.02,
        false,
        w * 0.84,
        max_cmd_fs,
    );
    while cx.line_width(&prompt_text, FontRole::Mono, cmd_fs, 0.02, false) > w * 0.84
        && cmd_fs > 12.0
    {
        cmd_fs -= 1.0;
    }
    let cmd_color = if crate::validate::contrast(r.accent, r.ground) >= 4.5 {
        r.accent
    } else {
        r.ink
    };
    let cmd_id = format!("{id}.command");
    let cmd_layer = cx.text(
        &cmd_id,
        &prompt_text,
        FontRole::Mono,
        cmd_fs,
        cmd_color,
        0.02,
        false,
        [cx.w / 2.0, (cx.h * 0.45).round()],
        z + 2,
    );
    let n_chars = prompt_text.chars().count().max(1);
    let ch = 0.035 * tempo;
    track.push(
        &cmd_id,
        enter_at,
        (n_chars as f64 - 1.0) * ch + 1.0 / cx.fps,
        Shape::Ease(Easing::Linear),
        MotionOp::GlyphCascade {
            stagger: ch,
            from: GlyphPose {
                opacity: 0.0,
                ..GlyphPose::REST
            },
            order: GlyphOrder::Forward,
            seed: 0,
        },
    );
    layers.push(cmd_layer);

    // Subline (if present)
    if let Some(ref s) = l.subline {
        let sub_role = l.sub_role(FontRole::SerifEmotional);
        let sub_upper = l.sub_upper(false);
        let max_sub_fs = (cx.short * 0.032).round().max(14.0);
        let mut sub_fs = cx.fit(s, sub_role, 0.0, sub_upper, w * 0.84, max_sub_fs);
        while cx.line_width(s, sub_role, sub_fs, 0.0, sub_upper) > w * 0.84 && sub_fs > 11.0 {
            sub_fs -= 1.0;
        }
        let sid = format!("{id}.subline");
        let sl = cx.text(
            &sid,
            s,
            sub_role,
            sub_fs,
            r.ink,
            0.0,
            sub_upper,
            [cx.w / 2.0, (cx.h * 0.56).round()],
            z + 2,
        );
        track.push(
            &sid,
            enter_at + 0.15,
            0.35 * tempo,
            Shape::Spring(Spring::Settle),
            MotionOp::Move {
                from: [0.0, 10.0],
                to: [0.0, 0.0],
                via: None,
            },
        );
        track.push(
            &sid,
            enter_at + 0.15,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        layers.push(sl);
    }

    // Spec line at [cx.w / 2.0, cx.h * 0.66]
    let spec_text = l
        .specs
        .first()
        .cloned()
        .unwrap_or_else(|| "BUILD: DETERMINISTIC [OK]".to_string());
    let base_spec_fs = (cx.short * 0.022).clamp(15.0, 24.0).round();
    let mut spec_fs = cx.fit(
        &spec_text,
        FontRole::Mono,
        0.04,
        true,
        w * 0.84,
        base_spec_fs,
    );
    while cx.line_width(&spec_text, FontRole::Mono, spec_fs, 0.04, true) > w * 0.84
        && spec_fs > 11.0
    {
        spec_fs -= 1.0;
    }
    let spec_id = format!("{id}.spec");
    let spec_layer = cx.text(
        &spec_id,
        &spec_text,
        FontRole::Mono,
        spec_fs,
        r.ink,
        0.04,
        true,
        [cx.w / 2.0, (cx.h * 0.66).round()],
        z + 2,
    );
    track.push(
        &spec_id,
        enter_at + 0.25,
        0.3 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.9,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &spec_id,
            enter_at + 0.25,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }
    layers.push(spec_layer);

    vec![scene(id, c.start, length, layers, track.motions)]
}

fn lower_editorial_badge(cx: &Ctx, c: &Component, l: &Lockup, z: i32, length: f64) -> Vec<Scene> {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let enter_at = c
        .timeline
        .iter()
        .find(|e| matches!(e.verb, Verb::Enter { .. }))
        .and_then(|e| e.at)
        .unwrap_or(0.0);
    let r = cx.roles(id);
    let mut layers = Vec::new();
    let mut track = Track::default();

    // 1. Outer solid plaque card + inner inset hairline border
    let w = (cx.w * 0.80).round();
    let h = (cx.h * 0.54).round();
    let radius = (cx.short * 0.018).round();
    let card_id = format!("{id}.plaque");
    let card_layer = layer(
        &card_id,
        LayerKind::Path {
            d: path::rounded_rect(0.0, 0.0, w, h, radius).to_svg(),
            fill: Some(Fill::Solid(r.ground)),
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: 2.5,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [w, h],
        z,
    );
    layers.push(card_layer);
    track.push(
        &card_id,
        enter_at,
        0.38 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.94,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &card_id,
            enter_at,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    let inset = (cx.short * 0.024).max(16.0).round();
    let iw = (w - inset * 2.0).max(40.0);
    let ih = (h - inset * 2.0).max(40.0);
    let inner_id = format!("{id}.inset_border");
    let inner_layer = layer(
        &inner_id,
        LayerKind::Path {
            d: path::rounded_rect(0.0, 0.0, iw, ih, (radius - 4.0).max(4.0)).to_svg(),
            fill: None,
            stroke: Some(Stroke {
                color: r.structure,
                width: 1.5,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [iw, ih],
        z + 1,
    );
    layers.push(inner_layer);
    track.push(
        &inner_id,
        enter_at + 0.04,
        0.45 * tempo,
        Shape::Ease(Easing::InOutCubic),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );

    // 2. Top editorial badge pill
    let badge_text = l.badge.as_deref().unwrap_or("EDITION");
    let pill_h = (cx.short * 0.046).round().clamp(26.0, 44.0);
    let base_bfs = (pill_h * 0.44).round().clamp(12.0, 20.0);
    let pill_w = (cx.line_width(badge_text, FontRole::Mono, base_bfs, 0.08, true) + pill_h * 1.4)
        .clamp(cx.short * 0.16, iw * 0.65)
        .round();
    let badge_y = (cx.h * 0.34).round();
    let raw_txt_col = cx.col.roles(r.accent).text(r.accent, base_bfs);
    let (pill_fill, badge_txt_col) = if crate::validate::contrast(raw_txt_col, r.accent) >= 4.7 {
        (r.accent, raw_txt_col)
    } else {
        (r.ink, r.ground)
    };
    let badge_pill_id = format!("{id}.badge_pill");
    let badge_pill = layer(
        &badge_pill_id,
        LayerKind::Path {
            d: path::rounded_rect(0.0, 0.0, pill_w, pill_h, pill_h * 0.5).to_svg(),
            fill: Some(Fill::Solid(pill_fill)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, badge_y],
        [pill_w, pill_h],
        z + 2,
    );
    layers.push(badge_pill);
    track.push(
        &badge_pill_id,
        enter_at + 0.06,
        0.32 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.85,
            to: 1.0,
            axis: Default::default(),
        },
    );

    let badge_txt_id = format!("{id}.badge_text");
    let badge_txt = cx.text(
        &badge_txt_id,
        badge_text,
        FontRole::Mono,
        base_bfs,
        badge_txt_col,
        0.08,
        true,
        [cx.w / 2.0, badge_y],
        z + 3,
    );
    track.push(
        &badge_txt_id,
        enter_at + 0.06,
        0.32 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.85,
            to: 1.0,
            axis: Default::default(),
        },
    );
    layers.push(badge_txt);

    // 3. Headline (defaults to upright/editorial Serif in preserved case for distinct brand signature)
    let head_role = l.head_role(FontRole::SerifEmotional);
    let head_upper = l.head_upper(false);
    let head_ls = if head_upper { 0.02 } else { -0.01 };
    let max_head_w = iw * 0.86;
    let max_head_fs = (cx.short * 0.11).round().max(28.0);
    let mut head_fs = cx.fit(
        &l.headline,
        head_role,
        head_ls,
        head_upper,
        max_head_w,
        max_head_fs,
    );
    while cx.line_width(&l.headline, head_role, head_fs, head_ls, head_upper) > max_head_w
        && head_fs > 16.0
    {
        head_fs -= 1.0;
    }
    let colour = l.colour.unwrap_or(Swatch::Ink);
    let ink = cx.paint(id, colour, Use::Text(head_fs));
    let hid = format!("{id}.headline");
    let head = cx.text(
        &hid,
        &l.headline,
        head_role,
        head_fs,
        ink,
        head_ls,
        head_upper,
        [cx.w / 2.0, (cx.h * 0.46).round()],
        z + 3,
    );
    track.push(
        &hid,
        enter_at + 0.08,
        0.36 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.90,
            to: 1.0,
            axis: Default::default(),
        },
    );
    if enter_at > 0.0 {
        track.push(
            &hid,
            enter_at + 0.08,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
    }

    // 4. Short centered accent divider rule
    let div_w = (iw * 0.18).round();
    let div_y = (cx.h * 0.55).round();
    let div_x0 = (cx.w / 2.0 - div_w * 0.5).round();
    let div_x1 = (cx.w / 2.0 + div_w * 0.5).round();
    let div_id = format!("{id}.divider");
    let mut div_layer = layer(
        &div_id,
        LayerKind::Path {
            d: format!("M{div_x0},{div_y} L{div_x1},{div_y}"),
            fill: None,
            stroke: Some(Stroke {
                color: r.accent,
                width: 3.0,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [0.0, 0.0],
        [cx.w, cx.h],
        z + 2,
    );
    div_layer.anchor_x = 0.0;
    div_layer.anchor_y = 0.0;
    layers.push(div_layer);
    track.push(
        &div_id,
        enter_at + 0.12,
        0.35 * tempo,
        Shape::Ease(Easing::OutCubic),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );

    // 5. Subline (defaults to uppercase Mono colophon caption)
    if let Some(ref s) = l.subline {
        let sub_role = l.sub_role(FontRole::Mono);
        let sub_upper = l.sub_upper(true);
        let sub_ls = if sub_upper { 0.06 } else { 0.0 };
        let max_sub_fs = (cx.short * 0.030).round().clamp(14.0, 26.0);
        let mut sub_fs = cx.fit(s, sub_role, sub_ls, sub_upper, max_head_w, max_sub_fs);
        while cx.line_width(s, sub_role, sub_fs, sub_ls, sub_upper) > max_head_w && sub_fs > 11.0 {
            sub_fs -= 1.0;
        }
        let sub_ink = cx.paint(id, colour, Use::Text(sub_fs));
        let sid = format!("{id}.subline");
        let sl = cx.text(
            &sid,
            s,
            sub_role,
            sub_fs,
            sub_ink,
            sub_ls,
            sub_upper,
            [cx.w / 2.0, (cx.h * 0.63).round()],
            z + 3,
        );
        track.push(
            &sid,
            enter_at + 0.14,
            0.35 * tempo,
            Shape::Spring(Spring::Settle),
            MotionOp::Move {
                from: [0.0, 10.0],
                to: [0.0, 0.0],
                via: None,
            },
        );
        track.push(
            &sid,
            enter_at + 0.14,
            taste::QUICK_FADE,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Fade { from: 0.0, to: 1.0 },
        );
        layers.push(sl);
    }

    apply_sheen(c, cx, z, &head, enter_at, &mut layers, &mut track);
    layers.push(head);

    vec![scene(id, c.start, length, layers, track.motions)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::{lower, Typesetter};
    use crate::program::MotionProgram;
    use crate::validate::validate;

    #[test]
    fn test_monolith_split_golden_ratio_divider_and_columns() {
        let program_json = r##"{
            "version": "2.0",
            "meta": {"name": "monolith_test", "duration": 4, "aspect": "16:9"},
            "palette": {"background": "#0D0E11", "ink": "#EDEDED", "accent": "#4E80EE", "muted": "#7A7E85"},
            "components": [
                {
                    "id": "monolith",
                    "kind": "lockup",
                    "preset": "monolith_split",
                    "headline": "CHRONOS",
                    "subline": "High-throughput deterministic engine",
                    "specs": ["LATENCY: 0.2MS", "JITTER: ZERO"],
                    "start": 0.0,
                    "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}]
                }
            ]
        }"##;
        let p = MotionProgram::from_json(program_json).expect("validates json");
        validate(&p).expect("semantic validation passes");
        let project = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&project, None)
            .expect("lowered project passes motion-core validation");

        let sc = project
            .scenes
            .iter()
            .find(|s| s.id == "monolith")
            .expect("scene exists");
        let div_layer = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.divider")
            .expect("divider exists");
        if let LayerKind::Path { d, .. } = &div_layer.kind {
            let cx_w = 1920.0f32;
            let w = (cx_w * 0.82f32).round();
            let card_left = (cx_w / 2.0 - w / 2.0).round();
            let major_w = (w * taste::INV_PHI).round();
            let expected_div_x = (card_left + major_w).round();
            assert!(
                d.contains(&format!("M{expected_div_x},")),
                "divider must be at Golden Split x={expected_div_x}, got d={d}"
            );
        } else {
            panic!("divider must be a Path");
        }

        // Verify headline is centered in the major golden section
        let head_layer = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.headline")
            .expect("headline exists");
        let cx_w = 1920.0f32;
        let w = (cx_w * 0.82f32).round();
        let card_left = (cx_w / 2.0 - w / 2.0).round();
        let major_w = (w * taste::INV_PHI).round();
        let expected_left_cx = (card_left + major_w / 2.0).round();
        assert_eq!(head_layer.x, expected_left_cx);

        // Verify spec is centered in the minor golden section
        let spec_layer = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.spec0")
            .expect("spec exists");
        let minor_w = w - major_w;
        let div_x = (card_left + major_w).round();
        let expected_right_cx = (div_x + minor_w / 2.0).round();
        assert_eq!(spec_layer.x, expected_right_cx);
    }

    #[test]
    fn test_monolith_split_portrait_horizontal_layout() {
        let program_json = r##"{
            "version": "2.0",
            "meta": {"name": "monolith_portrait", "duration": 4, "aspect": "9:16"},
            "palette": {"background": "#0D0E11", "ink": "#EDEDED", "accent": "#4E80EE", "muted": "#7A7E85"},
            "components": [
                {
                    "id": "monolith",
                    "kind": "lockup",
                    "preset": "monolith_split",
                    "headline": "CHRONOS",
                    "subline": "High-throughput deterministic engine",
                    "specs": ["LATENCY: 0.2MS", "JITTER: ZERO"],
                    "start": 0.0,
                    "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}]
                }
            ]
        }"##;
        let p = MotionProgram::from_json(program_json).expect("validates json");
        validate(&p).expect("semantic validation passes");
        let project = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&project, None)
            .expect("lowered project passes motion-core validation");

        let sc = project
            .scenes
            .iter()
            .find(|s| s.id == "monolith")
            .expect("scene exists");

        // Verify container card dimensions in portrait: w = 0.88, h = 0.72
        let card = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.card")
            .expect("card exists");
        let expected_w = (1080.0f32 * 0.88).round();
        let expected_h = (1920.0f32 * 0.72).round();
        assert_eq!(card.width, expected_w);
        assert_eq!(card.height, expected_h);

        // Verify horizontal divider at div_y = (cx.h * 0.5 - h * 0.04).round()
        let div_layer = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.divider")
            .expect("divider exists");
        let expected_div_y = (1920.0f32 * 0.5 - expected_h * 0.04).round();
        if let LayerKind::Path { d, .. } = &div_layer.kind {
            assert!(
                d.contains(&format!(",{expected_div_y}")),
                "divider must be horizontal at y={expected_div_y}, got d={d}"
            );
        } else {
            panic!("divider must be a Path");
        }

        // Verify headline and specs are centered horizontally at cx.w * 0.5
        let head_layer = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.headline")
            .expect("headline exists");
        assert_eq!(head_layer.x, 540.0);

        let spec0_layer = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.spec0")
            .expect("spec0 exists");
        assert_eq!(spec0_layer.x, 540.0);

        // Verify pill is at [cx.w * 0.5, cx.h * 0.5 + h * 0.36]
        let pill_layer = sc
            .layers
            .iter()
            .find(|l| l.id == "monolith.pill")
            .expect("pill exists");
        let expected_pill_y = (1920.0f32 * 0.5 + expected_h * 0.36).round();
        assert_eq!(pill_layer.x, 540.0);
        assert_eq!(pill_layer.y, expected_pill_y);
    }

    #[test]
    fn test_orbital_seal_reticle_and_datum_lines() {
        let program_json = r##"{
            "version": "2.0",
            "meta": {"name": "orbital_test", "duration": 4, "aspect": "16:9"},
            "palette": {"background": "#0D0E11", "ink": "#EDEDED", "accent": "#4E80EE", "muted": "#7A7E85"},
            "components": [
                {
                    "id": "orbital",
                    "kind": "lockup",
                    "preset": "orbital_seal",
                    "headline": "CHRONOS",
                    "subline": "Architectural Foundation",
                    "start": 0.0,
                    "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}]
                }
            ]
        }"##;
        let p = MotionProgram::from_json(program_json).expect("validates json");
        validate(&p).expect("semantic validation passes");
        let project = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&project, None)
            .expect("lowered project passes motion-core validation");

        let sc = project
            .scenes
            .iter()
            .find(|s| s.id == "orbital")
            .expect("scene exists");

        // Verify outer ring and inner disc diameters
        let outer = sc
            .layers
            .iter()
            .find(|l| l.id == "orbital.outer_ring")
            .expect("outer ring exists");
        let expected_d_outer = (1080.0f32 * 0.74).round();
        assert_eq!(outer.width, expected_d_outer);
        assert_eq!(outer.height, expected_d_outer);

        let inner = sc
            .layers
            .iter()
            .find(|l| l.id == "orbital.seal_disc")
            .expect("inner disc exists");
        let expected_d_inner = (1080.0f32 * 0.58).round();
        assert_eq!(inner.width, expected_d_inner);
        assert_eq!(inner.height, expected_d_inner);

        // Verify architectural datum lines exist behind seal (z) with stroke width 1.5
        let datum = sc
            .layers
            .iter()
            .find(|l| l.id == "orbital.datum_lines")
            .expect("datum lines exist");
        if let LayerKind::Path { stroke, .. } = &datum.kind {
            let strk = stroke.as_ref().expect("has stroke");
            assert_eq!(strk.width, 1.5);
        } else {
            panic!("datum lines must be Path");
        }

        // Verify headline sizing scaled up inside d_inner
        let head = sc
            .layers
            .iter()
            .find(|l| l.id == "orbital.headline")
            .expect("headline exists");
        if let LayerKind::Text(t) = &head.kind {
            assert!(
                t.font_size >= 36.0 && t.font_size <= 74.0,
                "headline font size must be within 36..=74, got {}",
                t.font_size
            );
        } else {
            panic!("headline must be Text");
        }
    }
}
