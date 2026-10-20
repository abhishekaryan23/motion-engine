//! DeviceFrame: product UI to the premium iOS kit (UI rules: docs/DESIGN_RULES.md).
//!
//! Everything is laid out in points on a real 393 × 852 pt screen (a phone,
//! or an invisible stage of the same size; a laptop gets a desktop screen
//! with a centred column) and the *camera* does the framing: it enters from
//! a point and, after every state change, frames the new state. One
//! container morphs between states with continuous corners on a ζ 0.8
//! spring; its colour changes by a circular reveal from the tap (never an
//! sRGB cross-fade); its content is masked to the morphing shape, leaves in
//! 0.10 s and arrives after it (no ghosting), children staggered 40 ms.
//! Elevation is a shadow stack, never a hairline. A touch disc (phones) or a
//! pointer (desktop) taps inside its target and leaves after the last tap.
//! `hold` is the hero moment: progress, a collapse to a circle, a vector
//! tick drawn on; `toast` grows out of the island.

use motion_core::easing::Easing;
use motion_core::path::{self, BezPath, Contour, Cubic};
use motion_core::scene::{
    Camera, CameraMotion, CameraOp, Color, ContactShadow, Fill, FontRole, InkBounds, Layer,
    LayerKind, LayerMask, MotionOp, PathFillRule, Scene, Stroke, TextAlign, TextStyle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lower::{layer, r6, scene, Ctx, Track, LINE_HEIGHT};
use crate::palette::{contrast, from_oklch, oklch, Oklch};
use crate::program::{Component, EnterStyle, Slot, Swatch, Verb};
use crate::taste::{Shape, Spring};

// ---------------------------------------------------------------- schema

/// A phone, a laptop or a bare stage with one UI container that morphs
/// between `states`, driven by `steps` (`to_state`, `click`, `hold`,
/// `toast`, `zoom`). Sizes are real iOS points; the camera frames them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceFrame {
    #[serde(default)]
    pub preset: DevicePreset,
    #[serde(default)]
    pub device: Device,
    #[serde(default)]
    pub slot: Slot,
    /// A typed flow (`payment`, `signup`, ...): states, steps and copy
    /// are derived from its fields. Give either `flow` or `states`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<crate::flow::UiFlow>,
    /// 1..=8 UI states (free text, validated for point of view, one
    /// success at a time and app context); the first shows from the start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<UiState>,
    /// In time order, at least 0.45 s apart (zoom excepted); `at` is seconds
    /// from the component start, or `beat`/`bar` on the music grid. Every
    /// state holds for its read time (0.4 s + 0.3 s a line, 0.6 s at least).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<UiStep>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DevicePreset {
    /// Light iOS: a #F2F2F7-like screen, white surfaces.
    #[default]
    FlatMono,
    /// Dark iOS: a black screen, raised grey surfaces.
    PhoneDark,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// No hardware: the UI sits on the page (a pointer taps it).
    #[default]
    None,
    Phone,
    Laptop,
}

/// One state of the container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiState {
    #[serde(default)]
    pub shape: UiShape,
    /// 1..=28 characters; no ✓/→/emoji (use `icon`).
    pub title: String,
    /// A quieter line under the title, at most 36 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Card and sheet rows (at most 4, 24 characters each).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    /// Card and sheet: the hero number (at most 10 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    /// Card and sheet: the primary button's label (at most 20 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cta: Option<String>,
    /// A leading vector icon (pills, buttons and toasts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<UiIcon>,
    /// Container colour (default: the surface; never the ground).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Swatch>,
    /// Rows with a switch instead of a chevron (on/off per row, in row
    /// order); tapping such a row flips it in place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub switches: Vec<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiShape {
    /// A 44 pt capsule sized to its label.
    #[default]
    Pill,
    /// A full-width 54 pt button (at the bottom of a phone screen).
    Button,
    /// A 64 pt notification under the island.
    Toast,
    /// A centred card: title, rows, optional amount and button.
    Card,
    /// A bottom sheet: grabber, title, amount, rows, button.
    Sheet,
    /// A 120 pt app icon.
    Icon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiIcon {
    /// A tick in a filled circle, drawn on.
    Check,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiStep {
    /// Seconds from the component start (fallback when no `beat`/`bar`).
    #[serde(default)]
    pub at: f64,
    /// On music beat n instead (program beat grid, snapped to a frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<u32>,
    /// On the downbeat of bar n (beat 4n) instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<u32>,
    #[serde(flatten)]
    pub op: UiOp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum UiOp {
    /// The container morphs to state `state`.
    ToState { state: usize },
    /// A tap: on the row the next state names (or `row`), else the button,
    /// else the container; with `then`, the container morphs to that state
    /// at the step time (the tap lands 0.12 s before it).
    Click {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        then: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row: Option<usize>,
    },
    /// Press and hold the button for `secs` (0.4..=2.0): progress fills it,
    /// it collapses to a circle and a tick draws on; with `then`, the
    /// container morphs to that state 1.1 s after the release.
    Hold {
        secs: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        then: Option<usize>,
    },
    /// State `state` (a toast) grows out of the island, holds `hold` s
    /// (default 1.6, at least its read time) and goes back in.
    Toast {
        state: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold: Option<f64>,
    },
    /// Camera: `in` frames the current state, `out` shows the whole device.
    Zoom {
        #[serde(default)]
        to: ZoomTo,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZoomTo {
    #[default]
    In,
    Out,
}

/// Characters that are not UI type: ticks, arrows, symbols, emoji (use
/// `icon`). Most UI faces have no such glyph, so they render as fallbacks.
pub fn banned_glyph(s: &str) -> Option<char> {
    s.chars().find(|&c| {
        matches!(c, '✓' | '✔' | '√' | '✗' | '✕')
            || ('\u{2190}'..='\u{21FF}').contains(&c)
            || ('\u{2300}'..='\u{2BFF}').contains(&c)
            || c >= '\u{1F000}'
    })
}

// ---------------------------------------------------------------- timing

/// Container morph (Settle spring, ζ 0.8: settled to 98 % by 0.30 s).
const MORPH: f64 = 0.42;
/// Old content out, new content in after it.
const OUT: f64 = 0.10;
const IN_FADE: f64 = 0.12;
const STAGGER: f64 = 0.04;
/// Tap → state change.
const AFTER_TAP: f64 = 0.12;
/// A growing card: the header, then one step per arrival.
const GROW_HEAD: f64 = 0.20;
const GROW_STEP: f64 = 0.14;
const PRESS_DOWN: f64 = 0.07;
/// Press release (Settle: 98 % by 0.30 s; Pop: by 0.35 s).
const PRESS_BACK: f64 = 0.40;
const REVEAL: f64 = 0.32;
const TRAVEL: f64 = 0.55;
const RIPPLE: f64 = 0.40;
const TICK: f64 = 0.32;
const CAMERA: f64 = 0.6;
const ENTER: f64 = 1.2;
const TOAST_IN: f64 = 0.45;
const TOAST_OUT: f64 = 0.25;
/// Release of a hold → `then` (collapse, tick, a beat to see it).
const HOLD_THEN: f64 = 1.1;
/// Minimum spacing of steps (a press springs back in this time).
pub const STEP_GAP: f64 = 0.45;

/// Read time of a state: 0.4 s + 0.3 s a line, at least the morph settle.
pub fn read_time(s: &UiState) -> f64 {
    let lines = 1
        + s.detail.is_some() as usize
        + s.items.len()
        + s.amount.is_some() as usize
        + s.cta.is_some() as usize;
    (0.4 + 0.3 * lines as f64).max(0.6)
}

/// The state changes `(time, state)` the steps produce (first at 0).
fn changes_of(f: &DeviceFrame) -> Vec<(f64, usize)> {
    let mut out = vec![(0.0, 0)];
    for s in &f.steps {
        match s.op {
            UiOp::ToState { state } => out.push((s.at, state)),
            UiOp::Click { then: Some(k), .. } => out.push((s.at, k)),
            UiOp::Hold {
                secs,
                then: Some(k),
            } => out.push((s.at + secs + HOLD_THEN, k)),
            _ => {}
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

impl DeviceFrame {
    /// Content problems as `(path suffix, message)`; `length` is the
    /// component's length in seconds.
    /// The frame with a flow's states and steps derived (itself otherwise).
    pub fn resolved(&self) -> std::borrow::Cow<'_, DeviceFrame> {
        match &self.flow {
            Some(fl) if self.states.is_empty() => {
                let (states, steps) = fl.expand();
                std::borrow::Cow::Owned(DeviceFrame {
                    states,
                    steps,
                    ..self.clone()
                })
            }
            _ => std::borrow::Cow::Borrowed(self),
        }
    }

    pub fn check(&self, length: f64) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(fl) = &self.flow {
            out.extend(fl.check());
            if !self.states.is_empty() || !self.steps.is_empty() {
                out.push((
                    ".flow".into(),
                    "a flow derives the states and steps: give flow or states, not both".into(),
                ));
            }
            if !out.is_empty() {
                return out;
            }
        }
        let resolved = self.resolved();
        let me = resolved.as_ref();
        let actor = self.flow.as_ref().and_then(|f| f.actor.as_deref());
        for (id, msg) in crate::flow::copy_problems(me.device, &me.states, &me.steps, actor) {
            out.push((".states".into(), format!("{msg} ({id})")));
        }
        me.check_states(length, out)
    }

    fn check_states(&self, length: f64, mut out: Vec<(String, String)>) -> Vec<(String, String)> {
        let n = self.states.len();
        if n == 0 || n > 8 {
            out.push((".states".into(), format!("1..=8 states, got {n}")));
            return out;
        }
        let len = |s: &str| s.chars().count();
        for (i, s) in self.states.iter().enumerate() {
            let p = format!(".states[{i}]");
            if s.title.trim().is_empty() || len(&s.title) > 28 {
                out.push((format!("{p}.title"), "1..=28 characters".into()));
            }
            for (k, t) in [
                ("title", Some(&s.title)),
                ("detail", s.detail.as_ref()),
                ("cta", s.cta.as_ref()),
                ("amount", s.amount.as_ref()),
            ] {
                if let Some(c) = t.and_then(|t| banned_glyph(t)) {
                    out.push((
                        format!("{p}.{k}"),
                        format!("'{c}' is not UI type (glyph_fallback): use \"icon\": \"check\""),
                    ));
                }
            }
            if s.detail.as_deref().is_some_and(|d| len(d) > 36) {
                out.push((format!("{p}.detail"), "at most 36 characters".into()));
            }
            if s.items.len() > 4
                || s.items
                    .iter()
                    .any(|t| len(t) > 24 || banned_glyph(t).is_some())
            {
                out.push((
                    format!("{p}.items"),
                    "at most 4 items of 24 plain characters".into(),
                ));
            }
            if s.switches.len() > s.items.len() {
                out.push((format!("{p}.switches"), "one switch per row at most".into()));
            }
            if s.amount.as_deref().is_some_and(|a| len(a) > 10) {
                out.push((format!("{p}.amount"), "at most 10 characters".into()));
            }
            if s.cta.as_deref().is_some_and(|a| len(a) > 20) {
                out.push((format!("{p}.cta"), "at most 20 characters".into()));
            }
            let rich = matches!(s.shape, UiShape::Card | UiShape::Sheet);
            if !rich && (s.amount.is_some() || s.cta.is_some() || !s.items.is_empty()) {
                out.push((
                    p.clone(),
                    "items, amount and cta belong to cards and sheets".into(),
                ));
            }
        }
        let mut last = f64::NEG_INFINITY;
        let mut last_act = f64::NEG_INFINITY;
        let mut cur = 0usize;
        for (i, s) in self.steps.iter().enumerate() {
            let p = format!(".steps[{i}]");
            if !(s.at.is_finite() && s.at >= 0.0) || s.at < last {
                out.push((format!("{p}.at"), "times are >= 0 and in order".into()));
            }
            last = s.at;
            if s.beat.is_some() || s.bar.is_some() {
                out.push((
                    format!("{p}.beat"),
                    "beat not in the music analysis (or no music)".into(),
                ));
            }
            if !matches!(s.op, UiOp::Zoom { .. }) {
                if s.at - last_act < STEP_GAP - 1e-6 {
                    out.push((
                        format!("{p}.at"),
                        format!("steps at least {STEP_GAP} s apart (a press must settle)"),
                    ));
                }
                last_act = s.at;
            }
            let target = match s.op {
                UiOp::ToState { state } | UiOp::Toast { state, .. } => Some(state),
                UiOp::Click { then, .. } | UiOp::Hold { then, .. } => then,
                UiOp::Zoom { .. } => None,
            };
            if target.is_some_and(|k| k >= n) {
                out.push((p.clone(), "no such state".into()));
                continue;
            }
            match s.op {
                UiOp::Hold { secs, .. } => {
                    if !(0.4..=2.0).contains(&secs) {
                        out.push((format!("{p}.secs"), "0.4..=2.0".into()));
                    }
                    let st = &self.states[cur];
                    if st.shape != UiShape::Button && st.cta.is_none() {
                        out.push((
                            p.clone(),
                            "hold needs a button state or a state with a cta".into(),
                        ));
                    }
                }
                UiOp::Toast { state, hold } => {
                    if self.states[state].shape != UiShape::Toast {
                        out.push((p.clone(), "toast shows a toast-shaped state".into()));
                    }
                    let need = read_time(&self.states[state]);
                    if hold.is_some_and(|h| h < need) {
                        out.push((
                            format!("{p}.hold"),
                            format!("at least the read time {need:.1} s"),
                        ));
                    }
                }
                UiOp::Click { row: Some(r), .. } if r >= self.states[cur].items.len() => {
                    out.push((format!("{p}.row"), "no such row".into()));
                }
                _ => {}
            }
            if let UiOp::ToState { state }
            | UiOp::Click {
                then: Some(state), ..
            }
            | UiOp::Hold {
                then: Some(state), ..
            } = s.op
            {
                cur = state;
            }
        }
        // Every state is readable before the next one replaces it.
        let ch = changes_of(self);
        for (k, &(t, s)) in ch.iter().enumerate() {
            let next = ch.get(k + 1).map_or(length, |c| c.0);
            let need = read_time(&self.states[s]);
            if next - t < need - 1e-6 {
                out.push((
                    ".steps".into(),
                    format!(
                        "state {s} shows {:.2} s from {t:.2} s; it needs {need:.1} s to read (read_time)",
                        next - t
                    ),
                ));
            }
        }
        out
    }
}

/// Resolves `beat`/`bar` steps to component seconds (frame-snapped beats).
pub(crate) fn resolve_steps(steps: &mut [UiStep], beats: &[f64], start: f64, fps: f64) {
    for s in steps {
        let n = s.beat.or(s.bar.map(|b| b * 4));
        if let Some(t) = n.and_then(|n| beats.get(n as usize)) {
            s.at = (((t * fps).round() / fps - start) * 1e6).round() / 1e6;
            s.beat = None;
            s.bar = None;
        }
    }
}

// ---------------------------------------------------------------- geometry

/// A rect with a corner radius, px.
#[derive(Debug, Clone, Copy, PartialEq)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
}

impl R {
    fn centre(&self) -> [f32; 2] {
        [self.x + self.w / 2.0, self.y + self.h / 2.0]
    }
    fn at(&self, dx: f32, dy: f32) -> R {
        R {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }
    fn local(&self) -> R {
        R {
            x: 0.0,
            y: 0.0,
            ..*self
        }
    }
    fn shrunk(&self, k: f32) -> R {
        let [cx, cy] = self.centre();
        R {
            x: cx - self.w * k / 2.0,
            y: cy - self.h * k / 2.0,
            w: self.w * k,
            h: self.h * k,
            r: self.r * k,
        }
    }
    fn path(&self) -> BezPath {
        squircle(self.x, self.y, self.w, self.h, self.r)
    }
}

fn line(from: [f32; 2], to: [f32; 2]) -> Cubic {
    let l = |t: f32| {
        [
            from[0] + (to[0] - from[0]) * t,
            from[1] + (to[1] - from[1]) * t,
        ]
    };
    Cubic {
        c1: l(1.0 / 3.0),
        c2: l(2.0 / 3.0),
        to,
    }
}

/// Points per corner: every container shape has the same topology, so
/// morphs match point for point.
const CORNER_PTS: usize = 14;

/// A rect with continuous ("squircle") corners: each corner is a
/// superellipse quadrant reaching 1.6 r along the edges (exponent 5),
/// easing to a circular arc (exponent 2) where the corner fills a pill end.
fn squircle(x: f32, y: f32, w: f32, h: f32, r: f32) -> BezPath {
    let half = (w.min(h) / 2.0).max(0.0);
    let r = r.clamp(0.0, half);
    let p = (r * 1.6).min(half);
    let n = if r <= 1e-3 {
        2.0
    } else {
        2.0 + 3.0 * ((p / r - 1.0) / 0.6).clamp(0.0, 1.0)
    };
    // Corner inner points and quadrant directions, clockwise from the
    // top-right; each corner runs from its first edge to its second.
    let corners = [
        ([x + w - p, y + p], [1.0f32, -1.0f32], true),
        ([x + w - p, y + h - p], [1.0, 1.0], false),
        ([x + p, y + h - p], [-1.0, 1.0], true),
        ([x + p, y + p], [-1.0, -1.0], false),
    ];
    let ex = 2.0 / n;
    let mut pts: Vec<[f32; 2]> = Vec::new();
    for (c, d, from_horizontal) in corners {
        for k in 0..=CORNER_PTS {
            let mut th = std::f32::consts::FRAC_PI_2 * k as f32 / CORNER_PTS as f32;
            if from_horizontal {
                th = std::f32::consts::FRAC_PI_2 - th;
            }
            let (cs, sn) = (th.cos().max(0.0), th.sin().max(0.0));
            pts.push([c[0] + d[0] * p * cs.powf(ex), c[1] + d[1] * p * sn.powf(ex)]);
        }
    }
    let start = pts[0];
    let mut segs = Vec::new();
    for i in 1..pts.len() {
        segs.push(line(pts[i - 1], pts[i]));
    }
    segs.push(line(*pts.last().expect("points"), start));
    BezPath {
        contours: vec![Contour {
            start,
            segs,
            closed: true,
        }],
    }
}

/// The kit tick in an `s`-px box at `o`: (5,12.5)→(10,17.5)→(19,7) of 24.
fn tick_path(o: [f32; 2], s: f32) -> BezPath {
    let k = s / 24.0;
    let p = |x: f32, y: f32| [o[0] + x * k, o[1] + y * k];
    BezPath {
        contours: vec![Contour {
            start: p(5.0, 12.5),
            segs: vec![
                line(p(5.0, 12.5), p(10.0, 17.5)),
                line(p(10.0, 17.5), p(19.0, 7.0)),
            ],
            closed: false,
        }],
    }
}

fn chevron(o: [f32; 2], h: f32) -> BezPath {
    let w = h * 0.55;
    BezPath {
        contours: vec![Contour {
            start: [o[0] - w / 2.0, o[1] - h / 2.0],
            segs: vec![
                line([o[0] - w / 2.0, o[1] - h / 2.0], [o[0] + w / 2.0, o[1]]),
                line([o[0] + w / 2.0, o[1]], [o[0] - w / 2.0, o[1] + h / 2.0]),
            ],
            closed: false,
        }],
    }
}

fn circle(c: [f32; 2], r: f32) -> BezPath {
    path::ellipse(c[0] - r, c[1] - r, 2.0 * r, 2.0 * r)
}

/// A path layer in a box with top-left `origin` and `size`; `d` in box px.
fn shape(id: &str, d: &BezPath, origin: [f32; 2], size: [f32; 2], fill: Option<Color>) -> Layer {
    let mut l = layer(
        id,
        LayerKind::Path {
            d: d.to_svg(),
            fill: fill.map(Fill::Solid),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        origin,
        size,
        0,
    );
    l.anchor_x = 0.0;
    l.anchor_y = 0.0;
    l
}

fn stroke_shape(id: &str, d: &BezPath, origin: [f32; 2], size: [f32; 2], s: Stroke) -> Layer {
    let mut l = shape(id, d, origin, size, None);
    if let LayerKind::Path { stroke, .. } = &mut l.kind {
        *stroke = Some(s);
    }
    l
}

fn set_shadows(l: &mut Layer, sh: Vec<ContactShadow>) {
    if let LayerKind::Path { shadows, .. } = &mut l.kind {
        *shadows = sh;
    }
}

fn alpha(c: Color, a: f32) -> Color {
    Color::rgba(c.r, c.g, c.b, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The kit's elevation (y / blur / alpha in pt), scaled by `pt`.
fn elevation(pt: f32, floating: bool, shade: Color) -> Vec<ContactShadow> {
    let mut v: Vec<(f32, f32, f32)> = vec![
        (1.0, 1.0, 0.04),
        (4.0, 4.0, 0.06),
        (12.0, 12.0, 0.08),
        (24.0, 24.0, 0.10),
    ];
    if floating {
        v.push((40.0, 40.0, 0.12));
    }
    v.into_iter()
        .map(|(y, b, a)| ContactShadow {
            color: alpha(shade, a),
            offset: [0.0, y * pt],
            blur: b * pt,
        })
        .collect()
}

// ---------------------------------------------------------------- type

/// The UI type scale (pt) and its two weights.
const T_HERO: f32 = 56.0;
const T_TITLE: f32 = 22.0;
const T_BODY: f32 = 17.0;
const T_SUB: f32 = 15.0;
const T_CAP: f32 = 13.0;
const BOLD: u16 = 700;
const MEDIUM: u16 = 500;

struct Type<'a, 'b> {
    cx: &'a Ctx<'b>,
    pt: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Centre,
    Left,
}

impl Type<'_, '_> {
    fn tracking(size_pt: f32) -> f32 {
        if size_pt > 20.0 {
            -0.02
        } else {
            0.0
        }
    }

    fn px(&self, size_pt: f32) -> f32 {
        (size_pt * self.pt * 4.0).round() / 4.0
    }

    fn width(&self, text: &str, size_pt: f32, weight: u16) -> f32 {
        self.cx.ts.measure.line_width(
            &self.cx.ui_face(weight),
            weight,
            false,
            text,
            self.px(size_pt),
            Self::tracking(size_pt),
        )
    }

    /// One line whose cap band is centred on `cap_y` (+0.5 px optical
    /// drop); `x` is the centre (`Centre`) or the left edge (`Left`).
    #[allow(clippy::too_many_arguments)]
    fn line(
        &self,
        id: &str,
        text: &str,
        size_pt: f32,
        weight: u16,
        colour: Color,
        align: Align,
        x: f32,
        cap_y: f32,
    ) -> Layer {
        let size = self.px(size_pt);
        let ls = Self::tracking(size_pt);
        let face = self.cx.ui_face(weight);
        let m = &self.cx.ts.measure;
        let width = m.line_width(&face, weight, false, text, size, ls);
        let height = (size * LINE_HEIGHT).round();
        let cap = m
            .ink_bounds(&face, weight, false, "H", size, LINE_HEIGHT, ls)
            .unwrap_or((height / 2.0 - 0.35 * size, height / 2.0 + 0.35 * size));
        let ink = m.ink_bounds(&face, weight, false, text, size, LINE_HEIGHT, ls);
        let top = cap_y + 0.5 - (cap.0 + cap.1) / 2.0;
        let mut l = layer(
            id,
            LayerKind::Text(TextStyle {
                text: text.to_string(),
                font_role: FontRole::Body,
                font_size: size,
                font_weight: weight,
                italic: false,
                color: colour,
                align: match align {
                    Align::Centre => TextAlign::Center,
                    Align::Left => TextAlign::Left,
                },
                line_height: LINE_HEIGHT,
                letter_spacing: ls,
                max_width: None,
                uppercase: false,
                ink: ink.map(|(top, bottom)| InkBounds { top, bottom }),
                fill: None,
                outline: None,
            }),
            [x, top + height / 2.0],
            [width.ceil().max(1.0), height],
            1,
        );
        l.anchor_y = 0.5;
        if align == Align::Left {
            l.anchor_x = 0.0;
        }
        l
    }
}

// ---------------------------------------------------------------- colours

#[derive(Debug, Clone, Copy)]
struct Look {
    /// What the screen (or the bare stage) shows behind the UI.
    screen: Color,
    /// Raised UI surfaces (cards, sheets, default containers).
    surface: Color,
    /// Rows on a surface.
    row: Color,
    ink: Color,
    accent: Color,
    /// The accent as a button: deep enough that its label (white or
    /// black) reads at 7:1.
    button: Color,
    /// Shadow colour (alpha set per shadow).
    shade: Color,
    dark: bool,
}

/// `c` at OKLCH lightness `l` (chroma capped: UI greys stay near-neutral).
fn with_l(c: Color, l: f32) -> Color {
    let o = oklch(c);
    from_oklch(Oklch {
        l: l.clamp(0.0, 1.0),
        c: o.c.min(0.012),
        h: o.h,
    })
}

impl Look {
    fn of(cx: &Ctx, id: &str, f: &DeviceFrame) -> Look {
        let ground = cx.roles(id).ground;
        let hardware = f.device != Device::None;
        let dark = match f.device {
            Device::None => oklch(ground).l < 0.5,
            _ => f.preset == DevicePreset::PhoneDark,
        };
        let screen = if !hardware {
            ground
        } else if dark {
            Color::rgb(0, 0, 0)
        } else {
            with_l(ground, 0.965)
        };
        let sl = oklch(screen).l;
        let surface = if dark {
            with_l(screen, (sl + 0.12).max(0.25))
        } else if hardware {
            Color::rgb(255, 255, 255)
        } else {
            with_l(screen, (sl + 0.06).min(1.0))
        };
        let row = if dark {
            with_l(surface, oklch(surface).l + 0.05)
        } else {
            with_l(surface, oklch(surface).l - 0.035)
        };
        let on = cx.col.roles(screen);
        Look {
            screen,
            surface,
            row,
            ink: on.ink,
            accent: on.accent,
            button: button_fill(on.accent),
            shade: Color::rgb(0, 0, 0),
            dark,
        }
    }

    /// A state's container colour: a surface by default, never the ground.
    fn fill(&self, cx: &Ctx, s: Option<Swatch>) -> Color {
        let c = match s {
            None | Some(Swatch::Background) => self.surface,
            Some(Swatch::Ink) => {
                if self.dark {
                    with_l(self.surface, 0.94)
                } else {
                    with_l(self.ink, 0.2)
                }
            }
            Some(Swatch::Accent) => self.button,
            Some(Swatch::Muted) => with_l(
                self.surface,
                oklch(self.surface).l + if self.dark { 0.08 } else { -0.06 },
            ),
            Some(g) => cx.col.get(g),
        };
        if contrast(c, self.screen) < 1.08 {
            self.surface
        } else {
            c
        }
    }

    fn row_on(&self, fill: Color) -> Color {
        if fill == self.surface {
            self.row
        } else {
            let l = oklch(fill).l;
            with_l(fill, l + if l < 0.5 { 0.06 } else { -0.04 })
        }
    }
}

/// Small UI type needs 7:1 (plus the resolver's margin) on its fill.
const LABEL_NEED: f32 = 7.9;

/// `accent` as a button fill: kept when white or black labels read on it,
/// else deepened (OKLCH lightness down, hue and chroma kept) until white
/// does.
fn button_fill(accent: Color) -> Color {
    let (w, k) = (Color::rgb(255, 255, 255), Color::rgb(0, 0, 0));
    if contrast(w, accent) >= LABEL_NEED || contrast(k, accent) >= LABEL_NEED {
        return accent;
    }
    let o = oklch(accent);
    (1..=40)
        .map(|i| {
            from_oklch(Oklch {
                l: o.l - 0.01 * i as f32,
                ..o
            })
        })
        .find(|c| contrast(w, *c) >= LABEL_NEED)
        .unwrap_or(accent)
}

/// Type colours on a fill: primary and secondary.
fn inks(cx: &Ctx, fill: Color) -> (Color, Color) {
    let r = cx.col.roles(fill);
    (r.ink, r.ink_secondary)
}

// ---------------------------------------------------------------- layout

/// The screen (or stage): origin in canvas px, size, px per pt.
struct Stage {
    origin: [f32; 2],
    w: f32,
    h: f32,
    pt: f32,
    /// Screen size in pt and the layout column width in pt.
    spt: [f32; 2],
    col: f32,
    phone: bool,
    /// Island rect (phones), screen px.
    island: Option<R>,
}

/// One state laid out: the container rect (screen px) and its content.
struct Plan {
    rect: R,
    /// Content layers in container-local px.
    kids: Vec<Layer>,
    /// Ids of children that arrive staggered (amount, rows).
    stagger: Vec<String>,
    /// Row rects (container-local px).
    rows: Vec<R>,
    /// The CTA rect (container-local px).
    cta: Option<R>,
    fill: Color,
    /// Cards size to what is shown: the container's height with the
    /// header only, then each arrival (ids) and the height it grows to.
    head: f32,
    grow: Vec<(Vec<String>, f32)>,
}

fn plan(cx: &Ctx, ty: &Type, look: &Look, st: &Stage, s: &UiState, gid: &str) -> Plan {
    let pt = st.pt;
    let (sw, sh, col) = (st.spt[0], st.spt[1], st.col);
    let fill = look.fill(cx, s.fill);
    let (ink, ink2) = inks(cx, fill);
    let mut kids: Vec<Layer> = Vec::new();
    let mut stagger = Vec::new();
    let mut rows = Vec::new();
    let mut cta = None;
    let mut head = 0.0f32;
    let mut grow: Vec<(Vec<String>, f32)> = Vec::new();
    let icon_w = if s.icon.is_some() { 22.0 + 8.0 } else { 0.0 };
    #[derive(PartialEq)]
    enum Anchor {
        Centre,
        Bottom,
        Top,
        Sheet,
    }
    // Container size in pt.
    let (w, h, r, anchor) = match s.shape {
        UiShape::Pill => {
            let tw = ty.width(&s.title, T_BODY, BOLD) / pt;
            (
                (tw + icon_w + 40.0).clamp(120.0, col - 32.0),
                44.0,
                22.0,
                Anchor::Centre,
            )
        }
        UiShape::Button => (col - 32.0, 54.0, 14.0, Anchor::Bottom),
        UiShape::Toast => (col - 24.0, 64.0, 24.0, Anchor::Top),
        UiShape::Icon => (120.0, 120.0, 120.0 * 0.2237, Anchor::Centre),
        UiShape::Card | UiShape::Sheet => {
            let sheet = s.shape == UiShape::Sheet;
            let mut hh = if sheet { 30.0 } else { 20.0 };
            hh += 28.0;
            if s.detail.is_some() {
                hh += 22.0;
            }
            if s.amount.is_some() {
                hh += 84.0;
            }
            if !s.items.is_empty() {
                hh += 12.0 + s.items.len() as f32 * 60.0 - 8.0;
            }
            if s.cta.is_some() {
                hh += 20.0 + 54.0;
            }
            hh += if sheet { 34.0 } else { 20.0 };
            if sheet {
                (col - 16.0, hh, 38.0, Anchor::Sheet)
            } else {
                (col - 32.0, hh, 20.0, Anchor::Centre)
            }
        }
    };
    let (wpx, hpx) = ((w * pt).round(), (h * pt).round());
    let x = ((sw * pt - wpx) / 2.0).round();
    let y = match anchor {
        Anchor::Bottom if st.phone => sh * pt - (34.0 + 16.0 + h) * pt,
        Anchor::Sheet => sh * pt - 8.0 * pt - hpx,
        Anchor::Top => (11.0 + 37.0 + 8.0) * pt,
        _ => (sh * pt - hpx) / 2.0,
    }
    .round();
    let rect = R {
        x,
        y,
        w: wpx,
        h: hpx,
        r: r * pt,
    };

    // Content, container-local px.
    match s.shape {
        UiShape::Pill | UiShape::Button | UiShape::Icon => {
            let size = if s.shape == UiShape::Icon {
                T_TITLE
            } else {
                T_BODY
            };
            let tw = ty.width(&s.title, size, BOLD);
            let total = tw + icon_w * pt;
            let x0 = (wpx - total) / 2.0;
            if let Some(UiIcon::Check) = s.icon {
                let on = if fill == look.button {
                    ink
                } else {
                    look.accent
                };
                icon_check(
                    cx,
                    &mut kids,
                    gid,
                    [x0 + 11.0 * pt, hpx / 2.0],
                    11.0 * pt,
                    on,
                );
            }
            let cap_y = hpx / 2.0;
            // An icon's label is its own role (title size).
            let lid = if s.shape == UiShape::Icon {
                format!("{gid}.icon_label")
            } else {
                format!("{gid}.label")
            };
            kids.push(ty.line(
                &lid,
                &s.title,
                size,
                BOLD,
                ink,
                Align::Centre,
                x0 + icon_w * pt + tw / 2.0,
                cap_y,
            ));
        }
        UiShape::Toast => {
            let mut tx = 16.0 * pt;
            if let Some(UiIcon::Check) = s.icon {
                icon_check(
                    cx,
                    &mut kids,
                    gid,
                    [(16.0 + 18.0) * pt, hpx / 2.0],
                    18.0 * pt,
                    look.button,
                );
                tx = (16.0 + 36.0 + 12.0) * pt;
            }
            let (y0, y1) = if s.detail.is_some() {
                (hpx / 2.0 - 9.0 * pt, hpx / 2.0 + 10.0 * pt)
            } else {
                (hpx / 2.0, 0.0)
            };
            kids.push(ty.line(
                &format!("{gid}.toast_title"),
                &s.title,
                T_SUB,
                BOLD,
                ink,
                Align::Left,
                tx,
                y0,
            ));
            if let Some(d) = &s.detail {
                kids.push(ty.line(
                    &format!("{gid}.toast_detail"),
                    d,
                    T_CAP,
                    MEDIUM,
                    ink2,
                    Align::Left,
                    tx,
                    y1,
                ));
            }
        }
        UiShape::Card | UiShape::Sheet => {
            let sheet = s.shape == UiShape::Sheet;
            let pad = 20.0 * pt;
            let mut y = if sheet { 30.0 * pt } else { 20.0 * pt };
            if sheet {
                let (gw, gh) = (36.0 * pt, 5.0 * pt);
                let gl = oklch(fill).l;
                kids.push(shape(
                    &format!("{gid}.grabber"),
                    &R {
                        x: 0.0,
                        y: 0.0,
                        w: gw,
                        h: gh,
                        r: gh / 2.0,
                    }
                    .path(),
                    [(wpx - gw) / 2.0, 6.0 * pt],
                    [gw, gh],
                    Some(with_l(fill, gl + if look.dark { 0.25 } else { -0.22 })),
                ));
            }
            kids.push(ty.line(
                &format!("{gid}.title"),
                &s.title,
                T_TITLE,
                BOLD,
                ink,
                Align::Left,
                pad,
                y + 8.0 * pt,
            ));
            y += 28.0 * pt;
            if let Some(d) = &s.detail {
                kids.push(ty.line(
                    &format!("{gid}.detail"),
                    d,
                    T_SUB,
                    MEDIUM,
                    ink2,
                    Align::Left,
                    pad,
                    y + 6.0 * pt,
                ));
                y += 22.0 * pt;
            }
            if let Some(a) = &s.amount {
                let id = format!("{gid}.amount");
                kids.push(ty.line(
                    &id,
                    a,
                    T_HERO,
                    BOLD,
                    ink,
                    Align::Centre,
                    wpx / 2.0,
                    y + 44.0 * pt,
                ));
                stagger.push(id);
                y += 84.0 * pt;
            }
            // A card grows as its rows and button arrive (never room kept
            // in advance); a sheet arrives whole.
            head = (y + 20.0 * pt).round();
            if !s.items.is_empty() {
                y += 12.0 * pt;
                for (j, item) in s.items.iter().enumerate() {
                    let rr = R {
                        x: 16.0 * pt,
                        y,
                        w: wpx - 32.0 * pt,
                        h: 52.0 * pt,
                        r: 12.0 * pt,
                    };
                    let rid = format!("{gid}.row{j}");
                    let row = shape(
                        &rid,
                        &rr.local().path(),
                        [0.0, 0.0],
                        [rr.w, rr.h],
                        Some(look.row_on(fill)),
                    );
                    let lab = ty.line(
                        &format!("{rid}.text"),
                        item,
                        T_BODY,
                        MEDIUM,
                        ink,
                        Align::Left,
                        16.0 * pt,
                        rr.h / 2.0,
                    );
                    let accessory = match s.switches.get(j) {
                        // An iOS switch: 51 × 31 pt track, 27 pt knob.
                        Some(&on) => {
                            let (tw, th) = (51.0 * pt, 31.0 * pt);
                            let o = [rr.w - 16.0 * pt - tw, (rr.h - th) / 2.0];
                            let track = R {
                                x: 0.0,
                                y: 0.0,
                                w: tw,
                                h: th,
                                r: th / 2.0,
                            };
                            let rl = oklch(look.row_on(fill)).l;
                            let off =
                                with_l(look.row_on(fill), rl + if rl < 0.5 { 0.12 } else { -0.12 });
                            let mut v = vec![
                                shape(
                                    &format!("{rid}.switch"),
                                    &track.path(),
                                    o,
                                    [tw, th],
                                    Some(off),
                                ),
                                shape(
                                    &format!("{rid}.switch_on"),
                                    &track.path(),
                                    o,
                                    [tw, th],
                                    Some(look.button),
                                ),
                            ];
                            let kd = 27.0 * pt;
                            let kx = if on { tw - 2.0 * pt - kd } else { 2.0 * pt };
                            let mut knob = shape(
                                &format!("{rid}.knob"),
                                &circle([kd / 2.0, kd / 2.0], kd / 2.0),
                                [o[0] + kx, o[1] + 2.0 * pt],
                                [kd, kd],
                                Some(Color::rgb(255, 255, 255)),
                            );
                            set_shadows(
                                &mut knob,
                                vec![ContactShadow {
                                    color: alpha(look.shade, 0.15),
                                    offset: [0.0, 3.0 * pt],
                                    blur: 8.0 * pt,
                                }],
                            );
                            v.push(knob);
                            v
                        }
                        None => vec![stroke_shape(
                            &format!("{rid}.chevron"),
                            &chevron([rr.w - 22.0 * pt, rr.h / 2.0], 11.0 * pt),
                            [0.0, 0.0],
                            [rr.w, rr.h],
                            Stroke {
                                color: ink2,
                                width: (2.0 * pt).max(1.5),
                                brush: None,
                            },
                        )],
                    };
                    // A row is one group: it staggers in as a unit.
                    let mut g = layer(
                        &format!("{rid}.g"),
                        LayerKind::Group {
                            children: [vec![row, lab], accessory].concat(),
                        },
                        [rr.x, rr.y],
                        [rr.w, rr.h],
                        0,
                    );
                    g.anchor_x = 0.0;
                    g.anchor_y = 0.0;
                    if sheet {
                        stagger.push(g.id.clone());
                    } else {
                        grow.push((vec![g.id.clone()], (rr.y + rr.h + 20.0 * pt).round()));
                    }
                    kids.push(g);
                    rows.push(rr);
                    y += 60.0 * pt;
                }
                y -= 8.0 * pt;
            }
            if let Some(label) = &s.cta {
                y += 20.0 * pt;
                let cr = R {
                    x: 16.0 * pt,
                    y,
                    w: wpx - 32.0 * pt,
                    h: 54.0 * pt,
                    r: 14.0 * pt,
                };
                let (cink, _) = inks(cx, look.button);
                let id = format!("{gid}.cta");
                let mut b = shape(
                    &id,
                    &cr.local().path(),
                    [cr.x, cr.y],
                    [cr.w, cr.h],
                    Some(look.button),
                );
                b.z_index = 2;
                kids.push(b);
                let mut l = ty.line(
                    &format!("{id}.label"),
                    label,
                    T_BODY,
                    BOLD,
                    cink,
                    Align::Centre,
                    cr.x + cr.w / 2.0,
                    cr.y + cr.h / 2.0,
                );
                l.z_index = 4;
                if !sheet {
                    grow.push((
                        vec![id.clone(), l.id.clone()],
                        (cr.y + cr.h + 20.0 * pt).round(),
                    ));
                }
                kids.push(l);
                cta = Some(cr);
            }
        }
    }
    Plan {
        rect,
        kids,
        stagger,
        rows,
        cta,
        fill,
        head,
        grow,
    }
}

/// A filled circle (radius `r`, centre `c`) with a tick to draw on.
fn icon_check(cx: &Ctx, kids: &mut Vec<Layer>, gid: &str, c: [f32; 2], r: f32, fill: Color) {
    let (tick_ink, _) = inks(cx, fill);
    kids.push(shape(
        &format!("{gid}.icon"),
        &circle([r, r], r),
        [c[0] - r, c[1] - r],
        [2.0 * r, 2.0 * r],
        Some(fill),
    ));
    let s = 2.0 * r * 0.72;
    let mut t = stroke_shape(
        &format!("{gid}.icon.tick"),
        &tick_path([0.0, 0.0], s),
        [c[0] - s / 2.0, c[1] - s / 2.0],
        [s, s],
        Stroke {
            color: tick_ink,
            width: (s * 2.25 / 24.0 * 1.2).max(1.5),
            brush: None,
        },
    );
    t.z_index = 2;
    kids.push(t);
}

// ---------------------------------------------------------------- camera

#[derive(Debug, Clone, Copy, PartialEq)]
struct View {
    zoom: f32,
    centre: [f32; 2],
}

#[derive(Clone, Copy)]
enum Curve {
    InOutExpo,
    InOutCubic,
    /// Fast start: a pull-back that keeps a growing state in frame.
    OutCubic,
}

impl Curve {
    fn at(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Curve::InOutCubic => Easing::InOutCubic.apply(u),
            Curve::OutCubic => Easing::OutCubic.apply(u),
            Curve::InOutExpo => {
                if u <= 0.0 {
                    0.0
                } else if u >= 1.0 {
                    1.0
                } else if u < 0.5 {
                    2f64.powf(20.0 * u - 10.0) / 2.0
                } else {
                    (2.0 - 2f64.powf(-20.0 * u + 10.0)) / 2.0
                }
            }
        }
    }
}

struct Move {
    t0: f64,
    t1: f64,
    to: View,
    curve: Curve,
}

/// The view `p` of the way from `a` to `b` (zoom geometric).
fn view_at(a: View, b: View, p: f32) -> View {
    View {
        zoom: a.zoom * (b.zoom / a.zoom).powf(p),
        centre: [
            a.centre[0] + (b.centre[0] - a.centre[0]) * p,
            a.centre[1] + (b.centre[1] - a.centre[1]) * p,
        ],
    }
}

/// Camera moves sampled into linear Push/Track pieces (pivot: the canvas
/// centre; Track = the framed point − the centre).
fn camera(cx: &Ctx, start: View, moves: &[Move], length: f64) -> Option<Camera> {
    if moves.is_empty() {
        return None;
    }
    let c = [cx.w / 2.0, cx.h / 2.0];
    let mut out = Vec::new();
    let push = |t0: f64, t1: f64, a: View, b: View, out: &mut Vec<CameraMotion>| {
        for op in [
            CameraOp::Push {
                from: a.zoom,
                to: b.zoom,
            },
            CameraOp::Track {
                from: [a.centre[0] - c[0], a.centre[1] - c[1]],
                to: [b.centre[0] - c[0], b.centre[1] - c[1]],
            },
        ] {
            // Rounded boundaries: consecutive pieces touch, never overlap.
            out.push(CameraMotion {
                start: r6(t0),
                duration: r6((r6(t1) - r6(t0)).max(0.0)),
                easing: Easing::Linear,
                velocity: None,
                op,
            });
        }
    };
    push(0.0, 0.0, start, start, &mut out);
    let mut prev_t = 0.0;
    let mut prev = start;
    for m in moves {
        let t1 = m.t1.min(length);
        let t0 = m.t0.max(prev_t);
        if t1 <= t0 {
            continue;
        }
        let n = 16;
        let mut a_t = t0;
        let mut a = prev;
        for k in 1..=n {
            let t = t0 + (t1 - t0) * k as f64 / n as f64;
            let b = view_at(prev, m.to, m.curve.at((t - t0) / (t1 - t0)) as f32);
            push(a_t, t, a, b, &mut out);
            a_t = t;
            a = b;
        }
        prev_t = t1;
        prev = m.to;
    }
    Some(Camera {
        pivot: Some(c),
        motions: out,
        perspective: None,
    })
}

// ---------------------------------------------------------------- lowering

fn fade(from: f32, to: f32) -> MotionOp {
    MotionOp::Fade { from, to }
}

fn mv(from: [f32; 2], to: [f32; 2]) -> MotionOp {
    MotionOp::Move {
        from,
        to,
        via: None,
    }
}

fn scale(from: f32, to: f32) -> MotionOp {
    MotionOp::Scale {
        from,
        to,
        axis: Default::default(),
    }
}

/// One layer's opacity schedule: fades stay sequential and start from the
/// current level (a hidden layer starts hidden).
struct Opacity {
    level: f32,
    busy: f64,
}

fn set_opacity(track: &mut Track, o: &mut Opacity, id: &str, t: f64, d: f64, to: f32, e: Easing) {
    let t = t.max(o.busy);
    track.push(id, t, d, Shape::Ease(e), fade(o.level, to));
    o.level = to;
    o.busy = t + d;
}

/// What a tap lands on.
#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Container,
    Row(usize),
    Cta,
}

struct Tap {
    t: f64,
    /// Screen px.
    at: [f32; 2],
    /// The colour under the finger.
    on: Color,
    hold: f64,
    state: usize,
    hit: Hit,
}

pub(crate) fn lower(cx: &Ctx, c: &Component, f: &DeviceFrame, z: i32, length: f64) -> Vec<Scene> {
    let resolved = f.resolved();
    let f = resolved.as_ref();
    let id = c.id.as_str();
    let look = Look::of(cx, id, f);
    let frame = 1.0 / cx.fps;
    let mut layers: Vec<Layer> = Vec::new();
    let mut track = Track::default();
    let centre = crate::taste::slot_center(f.slot, cx.aspect);

    // ---- hardware and the stage (pt → px).
    let st = match f.device {
        Device::Phone | Device::None => {
            let b = 9.0;
            let (pw, ph) = (393.0 + 2.0 * b, 852.0 + 2.0 * b);
            let pt = (cx.h * 0.86 / ph).min(cx.w * 0.9 / pw);
            let (sw, sh) = (393.0 * pt, 852.0 * pt);
            let origin = [
                (centre[0] - sw / 2.0).round(),
                (centre[1] - sh / 2.0).round(),
            ];
            let phone = f.device == Device::Phone;
            if phone {
                let (bw, bh) = (pw * pt, ph * pt);
                let bo = [origin[0] - b * pt, origin[1] - b * pt];
                // Side keys under the body edge.
                for (k, (left, y0, y1)) in [
                    (true, 120.0, 152.0),
                    (true, 178.0, 238.0),
                    (true, 252.0, 312.0),
                    (false, 200.0, 300.0),
                ]
                .into_iter()
                .enumerate()
                {
                    let (kw, out) = (5.0 * pt, 2.5 * pt);
                    let x = if left {
                        bo[0] - out
                    } else {
                        bo[0] + bw + out - kw
                    };
                    let kh = (y1 - y0) * pt;
                    layers.push(shape(
                        &format!("{id}.key{k}"),
                        &R {
                            x: 0.0,
                            y: 0.0,
                            w: kw,
                            h: kh,
                            r: 2.0 * pt,
                        }
                        .path(),
                        [x, bo[1] + y0 * pt],
                        [kw, kh],
                        Some(Color::rgb(0x2A, 0x2A, 0x2E)),
                    ));
                }
                let mut body = shape(
                    &format!("{id}.body"),
                    &R {
                        x: 0.0,
                        y: 0.0,
                        w: bw,
                        h: bh,
                        r: (55.0 + b) * pt,
                    }
                    .path(),
                    bo,
                    [bw, bh],
                    Some(Color::rgb(0x0E, 0x0E, 0x10)),
                );
                if let LayerKind::Path {
                    stroke, shadows, ..
                } = &mut body.kind
                {
                    *stroke = Some(Stroke {
                        color: Color::rgb(0x3A, 0x3A, 0x3C),
                        width: 2.0,
                        brush: None,
                    });
                    *shadows = vec![
                        ContactShadow {
                            color: alpha(look.shade, 0.18),
                            offset: [0.0, 30.0 * pt],
                            blur: 30.0 * pt,
                        },
                        ContactShadow {
                            color: alpha(look.shade, 0.12),
                            offset: [0.0, 80.0 * pt],
                            blur: 80.0 * pt,
                        },
                    ];
                }
                layers.push(body);
                layers.push(shape(
                    &format!("{id}.screen"),
                    &R {
                        x: 0.0,
                        y: 0.0,
                        w: sw,
                        h: sh,
                        r: 55.0 * pt,
                    }
                    .path(),
                    origin,
                    [sw, sh],
                    Some(look.screen),
                ));
            }
            Stage {
                origin,
                w: sw,
                h: sh,
                pt,
                spt: [393.0, 852.0],
                col: 393.0,
                phone,
                island: phone.then_some(R {
                    x: (sw - 126.0 * pt) / 2.0,
                    y: 11.0 * pt,
                    w: 126.0 * pt,
                    h: 37.0 * pt,
                    r: 18.5 * pt,
                }),
            }
        }
        Device::Laptop => {
            let lw = (cx.w * 0.66).min(cx.h * 0.78 / 0.66).round();
            let lh = (lw * 0.625).round();
            let base_h = (lw * 0.035).round();
            let top = centre[1] - (lh + base_h) / 2.0;
            let bez = (lw * 0.022).round();
            let (sw, sh) = (lw - 2.0 * bez, lh - 2.0 * bez - lw * 0.012);
            let mut lid = shape(
                &format!("{id}.lid"),
                &R {
                    x: 0.0,
                    y: 0.0,
                    w: lw,
                    h: lh,
                    r: lw * 0.03,
                }
                .path(),
                [centre[0] - lw / 2.0, top],
                [lw, lh],
                Some(Color::rgb(0x0E, 0x0E, 0x10)),
            );
            set_shadows(
                &mut lid,
                vec![ContactShadow {
                    color: alpha(look.shade, 0.16),
                    offset: [0.0, lw * 0.02],
                    blur: lw * 0.04,
                }],
            );
            layers.push(lid);
            let origin = [(centre[0] - sw / 2.0).round(), (top + bez).round()];
            layers.push(shape(
                &format!("{id}.screen"),
                &R {
                    x: 0.0,
                    y: 0.0,
                    w: sw,
                    h: sh,
                    r: lw * 0.008,
                }
                .path(),
                origin,
                [sw, sh],
                Some(look.screen),
            ));
            let bw = (lw * 1.16).round();
            let base = path::polygon(&[
                [0.0, 0.0],
                [bw, 0.0],
                [bw * 0.97, base_h],
                [bw * 0.03, base_h],
            ]);
            layers.push(shape(
                &format!("{id}.base"),
                &base,
                [centre[0] - bw / 2.0, top + lh],
                [bw, base_h],
                Some(Color::rgb(0xC9, 0xC9, 0xCE)),
            ));
            let pt = sw / 1180.0;
            Stage {
                origin,
                w: sw,
                h: sh,
                pt,
                spt: [1180.0, sh / pt],
                col: 430.0,
                phone: false,
                island: None,
            }
        }
    };
    let pt = st.pt;
    let to_canvas = |p: [f32; 2]| [st.origin[0] + p[0], st.origin[1] + p[1]];
    let ty = Type { cx, pt };

    // Status bar (one group: it leaves while the camera is in close) and
    // the home indicator (phones).
    let status_id = format!("{id}.status");
    if st.phone {
        let ink = look.ink;
        let mut kids = vec![ty.line(
            &format!("{id}.time"),
            "9:41",
            T_BODY,
            BOLD,
            ink,
            Align::Centre,
            64.0 * pt,
            30.0 * pt,
        )];
        for k in 0..4 {
            let bh = (4.0 + 2.4 * k as f32) * pt;
            kids.push(shape(
                &format!("{id}.bar{k}"),
                &R {
                    x: 0.0,
                    y: 0.0,
                    w: 3.0 * pt,
                    h: bh,
                    r: 1.0 * pt,
                }
                .path(),
                [(292.0 + 4.6 * k as f32) * pt, 35.5 * pt - bh],
                [3.0 * pt, bh],
                Some(ink),
            ));
        }
        // Battery: a filled ring (no hairline stroke) and its charge.
        let bo = [326.0 * pt, 24.0 * pt];
        let (bw, bh) = (25.0 * pt, 12.0 * pt);
        let mut ring = squircle(0.0, 0.0, bw, bh, 4.0 * pt);
        let k = 1.2 * pt;
        ring.contours
            .extend(squircle(k, k, bw - 2.0 * k, bh - 2.0 * k, 4.0 * pt - k).contours);
        let mut battery = shape(
            &format!("{id}.battery"),
            &ring,
            bo,
            [bw, bh],
            Some(alpha(ink, 0.4)),
        );
        if let LayerKind::Path { fill_rule, .. } = &mut battery.kind {
            *fill_rule = PathFillRule::EvenOdd;
        }
        kids.push(battery);
        kids.push(shape(
            &format!("{id}.charge"),
            &R {
                x: 0.0,
                y: 0.0,
                w: 19.0 * pt,
                h: 7.5 * pt,
                r: 2.0 * pt,
            }
            .path(),
            [bo[0] + 3.0 * pt, bo[1] + 2.25 * pt],
            [19.0 * pt, 7.5 * pt],
            Some(ink),
        ));
        let mut g = layer(
            &status_id,
            LayerKind::Group { children: kids },
            [st.origin[0], st.origin[1]],
            [st.w, 54.0 * pt],
            1,
        );
        g.anchor_x = 0.0;
        g.anchor_y = 0.0;
        layers.push(g);
        if let Some(isl) = st.island {
            let mut l = shape(
                &format!("{id}.island"),
                &isl.local().path(),
                to_canvas([isl.x, isl.y]),
                [isl.w, isl.h],
                Some(Color::rgb(0, 0, 0)),
            );
            l.z_index = 5;
            layers.push(l);
        }
        layers.push(shape(
            &format!("{id}.home"),
            &R {
                x: 0.0,
                y: 0.0,
                w: 134.0 * pt,
                h: 5.0 * pt,
                r: 2.5 * pt,
            }
            .path(),
            to_canvas([(393.0 - 134.0) / 2.0 * pt, (852.0 - 8.0 - 5.0) * pt]),
            [134.0 * pt, 5.0 * pt],
            Some(ink),
        ));
    }

    // ---- states.
    let plans: Vec<Plan> = f
        .states
        .iter()
        .enumerate()
        .map(|(k, s)| plan(cx, &ty, &look, &st, s, &format!("{id}.s{k}")))
        .collect();
    let rect_of = |k: usize| plans[k].rect;
    // Switches that start off hide their on-track.
    for (k, s) in f.states.iter().enumerate() {
        for (j, on) in s.switches.iter().enumerate() {
            if !on {
                track.push(
                    &format!("{id}.s{k}.row{j}.switch_on"),
                    0.0,
                    frame,
                    Shape::Ease(Easing::Linear),
                    fade(0.0, 0.0),
                );
            }
        }
    }

    // ---- the step script.
    let mut changes: Vec<(f64, usize, Option<[f32; 2]>)> = vec![(0.0, 0, None)];
    let mut taps: Vec<Tap> = Vec::new();
    let mut toasts: Vec<(f64, f64, usize)> = Vec::new();
    let mut zooms: Vec<(f64, ZoomTo)> = Vec::new();
    let mut cur = 0usize;
    for s in &f.steps {
        match s.op {
            UiOp::ToState { state } => {
                changes.push((s.at, state, None));
                cur = state;
            }
            UiOp::Click { then, row } => {
                let p = &plans[cur];
                let r = p.rect;
                let named = then.and_then(|k| {
                    let t = f.states[k].title.to_lowercase();
                    f.states[cur].items.iter().position(|i| {
                        let i = i.to_lowercase();
                        t.contains(&i) || i.contains(&t)
                    })
                });
                let (at, on, hit) = if let Some(j) = row.or(named).filter(|j| *j < p.rows.len()) {
                    let rr = p.rows[j];
                    (
                        [r.x + rr.x + rr.w * 0.74, r.y + rr.y + rr.h * 0.5],
                        look.row_on(p.fill),
                        Hit::Row(j),
                    )
                } else if let Some(cr) = p.cta {
                    (
                        [r.x + cr.x + cr.w * 0.84, r.y + cr.y + cr.h * 0.55],
                        look.button,
                        Hit::Cta,
                    )
                } else {
                    // Clear of the centred label, ≥ 0.25 h inside the edge.
                    let [cxr, cyr] = r.centre();
                    let reach = (r.w / 2.0 - r.h * 0.3).max(0.0);
                    // Pills are all label: tap their far end.
                    let dx = if f.states[cur].shape == UiShape::Pill {
                        reach
                    } else {
                        (r.w * 0.30).min(reach)
                    };
                    let dy = if r.h >= 80.0 * pt {
                        r.h * 0.25
                    } else {
                        r.h * 0.10
                    };
                    ([cxr + dx, cyr + dy], p.fill, Hit::Container)
                };
                // With `then`, the state changes on the step (the beat) and
                // the tap lands just before it.
                let t_tap = if then.is_some() {
                    (s.at - AFTER_TAP).max(0.0)
                } else {
                    s.at
                };
                taps.push(Tap {
                    t: t_tap,
                    at,
                    on,
                    hold: 0.0,
                    state: cur,
                    hit,
                });
                if let Some(k) = then {
                    changes.push((s.at, k, Some(at)));
                    cur = k;
                }
            }
            UiOp::Hold { secs, then } => {
                let p = &plans[cur];
                let r = p.rect;
                let (at, on, hit) = match p.cta {
                    Some(cr) => (
                        [r.x + cr.x + cr.w * 0.84, r.y + cr.y + cr.h * 0.55],
                        look.button,
                        Hit::Cta,
                    ),
                    None => (
                        [
                            r.centre()[0] + (r.w * 0.30).min((r.w / 2.0 - r.h * 0.3).max(0.0)),
                            r.centre()[1] + r.h * 0.08,
                        ],
                        p.fill,
                        Hit::Container,
                    ),
                };
                taps.push(Tap {
                    t: s.at,
                    at,
                    on,
                    hold: secs,
                    state: cur,
                    hit,
                });
                if let Some(k) = then {
                    changes.push((s.at + secs + HOLD_THEN, k, Some(at)));
                    cur = k;
                }
            }
            UiOp::Toast { state, hold } => {
                let h = hold.unwrap_or(1.6).max(read_time(&f.states[state]));
                toasts.push((s.at, h, state));
            }
            UiOp::Zoom { to } => zooms.push((s.at, to)),
        }
    }
    changes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let state_at = |t: f64| {
        changes
            .iter()
            .rev()
            .find(|c| c.0 <= t + 1e-9)
            .map_or(0, |c| c.1)
    };
    // Distinct changes only.
    let mut moves_to: Vec<(f64, usize, usize, Option<[f32; 2]>)> = Vec::new();
    {
        let mut shown = 0usize;
        for &(t, k, tap) in changes.iter().skip(1) {
            if k != shown {
                moves_to.push((t, shown, k, tap));
                shown = k;
            }
        }
    }

    // Success transforms the screen, it does not erase it: between two
    // states with the same container, what is unchanged stays put and only
    // the rest swaps.
    let in_place =
        |from: usize, k: usize| rect_of(from) == rect_of(k) && plans[from].fill == plans[k].fill;
    let norm = |l: &Layer, k: usize| {
        serde_json::to_string(l)
            .unwrap_or_default()
            .replace(&format!("{id}.s{k}."), "#.")
    };
    let held_cta = |k: usize| {
        taps.iter()
            .any(|t| t.state == k && t.hold > 0.0 && t.hit == Hit::Cta)
    };
    let persists = |from: usize, k: usize, l: &Layer| {
        if held_cta(from) && (l.id.ends_with(".cta") || l.id.ends_with(".cta.label")) {
            return false;
        }
        let me = norm(l, k);
        plans[from].kids.iter().any(|o| norm(o, from) == me)
    };

    // ---- container geometry: presses, collapses, state morphs.
    // (time, duration, rect, spring?)
    let mut geo: Vec<(f64, f64, R, bool)> = Vec::new();
    for tap in taps.iter().filter(|t| t.hit == Hit::Container) {
        let r = rect_of(tap.state);
        geo.push((tap.t, PRESS_DOWN, r.shrunk(0.96), false));
        if tap.hold > 0.0 {
            let d = r.h;
            let [cxr, cyr] = r.centre();
            geo.push((
                tap.t + tap.hold,
                MORPH,
                R {
                    x: cxr - d / 2.0,
                    y: cyr - d / 2.0,
                    w: d,
                    h: d,
                    r: d / 2.0,
                },
                true,
            ));
        } else if !moves_to
            .iter()
            .any(|m| m.0 > tap.t && m.0 < tap.t + PRESS_DOWN + PRESS_BACK)
        {
            geo.push((tap.t + PRESS_DOWN, PRESS_BACK, r, true));
        }
    }
    for &(t, from, k, _) in &moves_to {
        let p = &plans[k];
        if in_place(from, k) {
            continue;
        }
        if p.grow.is_empty() {
            geo.push((t, MORPH, rect_of(k), true));
            continue;
        }
        // Header first (top edge where it will stay), then one step per
        // arrival; the last settles on the spring.
        let at = |hh: f32| R {
            h: hh,
            ..rect_of(k)
        };
        geo.push((t, GROW_HEAD, at(p.head), false));
        for (j, (_, hh)) in p.grow.iter().enumerate() {
            let tj = t + GROW_HEAD + GROW_STEP * j as f64;
            if j + 1 == p.grow.len() {
                geo.push((tj, MORPH, rect_of(k), true));
            } else {
                geo.push((tj, GROW_STEP, at(*hh), false));
            }
        }
    }
    geo.sort_by(|a, b| a.0.total_cmp(&b.0));
    let geo_shape = |g: &(f64, f64, R, bool)| {
        if g.3 {
            Shape::Spring(Spring::Settle)
        } else {
            Shape::Ease(Easing::OutCubic)
        }
    };

    // ---- the container: a shadow plate and one fill per colour (circular
    // reveals from the tap).
    let sbox = [st.w, st.h];
    let mut plate = shape(
        &format!("{id}.plate"),
        &rect_of(0).path(),
        st.origin,
        sbox,
        None,
    );
    set_shadows(&mut plate, elevation(pt, false, look.shade));
    plate.z_index = 1;
    let mut fills: Vec<String> = vec![format!("{id}.fill0")];
    let mut fill_layers: Vec<Layer> = vec![{
        let mut l = shape(
            &fills[0],
            &rect_of(0).path(),
            st.origin,
            sbox,
            Some(plans[0].fill),
        );
        l.z_index = 2;
        l
    }];
    for &(t, from, k, tap) in &moves_to {
        if plans[k].fill == plans[from].fill {
            continue;
        }
        let o = tap.unwrap_or(rect_of(from).centre());
        let fid = format!("{id}.fill{}", fills.len());
        let mut l = shape(
            &fid,
            &rect_of(0).path(),
            st.origin,
            sbox,
            Some(plans[k].fill),
        );
        l.z_index = 2;
        // The circle ends covering the whole screen: every later state of
        // this colour stays inside it.
        let reach = [[0.0, 0.0], [st.w, 0.0], [0.0, st.h], [st.w, st.h]]
            .iter()
            .map(|q: &[f32; 2]| (q[0] - o[0]).hypot(q[1] - o[1]))
            .fold(0.0f32, f32::max)
            * 1.05;
        l.mask = Some(LayerMask {
            d: circle(o, 0.5).to_svg(),
            invert: false,
        });
        // Hidden (not just masked) until its reveal.
        track.push(
            &fid,
            0.0,
            frame,
            Shape::Ease(Easing::Linear),
            fade(0.0, 0.0),
        );
        track.push(&fid, t, frame, Shape::Ease(Easing::Linear), fade(0.0, 1.0));
        track.push(
            &fid,
            t,
            REVEAL,
            Shape::Ease(Easing::OutQuint),
            MotionOp::MaskMorph {
                to: circle(o, reach).to_svg(),
            },
        );
        // The colour underneath is not needed once covered.
        let below = fills.last().expect("fill").clone();
        track.push(
            &below,
            t + REVEAL,
            frame,
            Shape::Ease(Easing::Linear),
            fade(1.0, 0.0),
        );
        fills.push(fid);
        fill_layers.push(l);
    }
    for g in &geo {
        for cid in std::iter::once(&plate.id).chain(fills.iter()) {
            track.push(
                cid,
                g.0,
                g.1,
                geo_shape(g),
                MotionOp::Morph {
                    to: g.2.path().to_svg(),
                },
            );
        }
    }
    layers.push(plate);
    layers.extend(fill_layers);

    // ---- content: one group per state, masked to the container.
    let mut opacity: Vec<Opacity> = (0..plans.len())
        .map(|k| Opacity {
            level: if k == 0 { 1.0 } else { 0.0 },
            busy: 0.0,
        })
        .collect();
    let mut groups: Vec<Layer> = Vec::new();
    for (k, p) in plans.iter().enumerate() {
        let gid = format!("{id}.s{k}");
        let r = p.rect;
        let mut g = layer(
            &gid,
            LayerKind::Group {
                children: p.kids.clone(),
            },
            to_canvas(r.centre()),
            [r.w, r.h],
            3,
        );
        g.mask = Some(LayerMask {
            d: rect_of(0).at(-r.x, -r.y).path().to_svg(),
            invert: false,
        });
        for gev in &geo {
            track.push(
                &gid,
                gev.0,
                gev.1,
                geo_shape(gev),
                MotionOp::MaskMorph {
                    to: gev.2.at(-r.x, -r.y).path().to_svg(),
                },
            );
        }
        if k != 0 && !moves_to.iter().any(|m| m.2 == k) {
            g.visible = false;
        }
        groups.push(g);
    }
    for &(t, from, k, _) in &moves_to {
        if in_place(from, k) {
            let (gf, gk) = (format!("{id}.s{from}"), format!("{id}.s{k}"));
            // What changes leaves (the hold's tick and progress with it)...
            for l in plans[from].kids.iter().filter(|l| !persists(k, from, l)) {
                track.push(&l.id, t, OUT, Shape::Ease(Easing::OutCubic), fade(1.0, 0.0));
            }
            for (n, tap) in taps.iter().enumerate() {
                if tap.state == from && tap.hold > 0.0 {
                    track.push(
                        &format!("{gf}.done{n}"),
                        t,
                        OUT,
                        Shape::Ease(Easing::OutCubic),
                        fade(1.0, 0.0),
                    );
                }
            }
            // ...the groups swap on one frame (identical pixels)...
            let t_in = t + OUT;
            set_opacity(
                &mut track,
                &mut opacity[from],
                &gf,
                t_in,
                frame,
                0.0,
                Easing::Linear,
            );
            set_opacity(
                &mut track,
                &mut opacity[k],
                &gk,
                t_in,
                frame,
                1.0,
                Easing::Linear,
            );
            // ...and what is new arrives.
            for (j, l) in plans[k]
                .kids
                .iter()
                .filter(|l| !persists(from, k, l))
                .enumerate()
            {
                let tj = t_in + 0.04 * (j + 1) as f64;
                track.push(
                    &l.id,
                    t_in,
                    tj - t_in,
                    Shape::Ease(Easing::Linear),
                    fade(0.0, 0.0),
                );
                track.push(
                    &l.id,
                    tj,
                    IN_FADE,
                    Shape::Ease(Easing::OutCubic),
                    fade(0.0, 1.0),
                );
                if matches!(l.kind, LayerKind::Text(_)) {
                    let op = if l.id.ends_with(".label") {
                        scale(0.96, 1.0)
                    } else {
                        mv([0.0, 4.0 * pt], [0.0, 0.0])
                    };
                    track.push(&l.id, tj, MORPH, Shape::Spring(Spring::Settle), op);
                }
            }
            continue;
        }
        set_opacity(
            &mut track,
            &mut opacity[from],
            &format!("{id}.s{from}"),
            t,
            OUT,
            0.0,
            Easing::OutCubic,
        );
        let gid = format!("{id}.s{k}");
        let t_in = t + OUT;
        set_opacity(
            &mut track,
            &mut opacity[k],
            &gid,
            t_in,
            IN_FADE,
            1.0,
            Easing::OutCubic,
        );
        track.push(
            &gid,
            t_in,
            MORPH,
            Shape::Spring(Spring::Settle),
            scale(0.98, 1.0),
        );
        for (j, sid) in plans[k].stagger.iter().enumerate() {
            let tj = t_in + STAGGER * (j + 1) as f64;
            track.push(
                sid,
                t_in,
                tj - t_in,
                Shape::Ease(Easing::Linear),
                fade(0.0, 0.0),
            );
            track.push(
                sid,
                tj,
                IN_FADE,
                Shape::Ease(Easing::OutCubic),
                fade(0.0, 1.0),
            );
            track.push(
                sid,
                tj,
                MORPH,
                Shape::Spring(Spring::Settle),
                mv([0.0, 12.0 * pt], [0.0, 0.0]),
            );
        }
        // A card's rows and button arrive with the container growing to
        // them.
        for (j, (ids, _)) in plans[k].grow.iter().enumerate() {
            let tj = t + GROW_HEAD + GROW_STEP * j as f64;
            for sid in ids {
                track.push(
                    sid,
                    t_in.min(tj),
                    (tj - t_in).max(frame),
                    Shape::Ease(Easing::Linear),
                    fade(0.0, 0.0),
                );
                track.push(
                    sid,
                    tj.max(t_in + frame),
                    IN_FADE,
                    Shape::Ease(Easing::OutCubic),
                    fade(0.0, 1.0),
                );
                track.push(
                    sid,
                    tj,
                    MORPH,
                    Shape::Spring(Spring::Settle),
                    mv([0.0, 12.0 * pt], [0.0, 0.0]),
                );
            }
        }
        // A pill's icon tick draws on as it arrives.
        let tk = format!("{gid}.icon.tick");
        if plans[k].kids.iter().any(|l| l.id == tk) {
            track.push(
                &tk,
                0.0,
                frame,
                Shape::Ease(Easing::Linear),
                MotionOp::Trim { from: 0.0, to: 0.0 },
            );
            track.push(
                &tk,
                t_in + 0.18,
                TICK,
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }
    }

    // ---- press feedback on rows and CTAs; holds.
    for (n, tap) in taps.iter().enumerate() {
        let k = tap.state;
        let p = &plans[k];
        let gid = format!("{id}.s{k}");
        let leaves = moves_to
            .iter()
            .any(|m| m.1 == k && m.0 > tap.t && m.0 < tap.t + 0.5);
        match tap.hit {
            Hit::Row(j) => {
                let rid = format!("{gid}.row{j}");
                // A switch row flips in place: the knob slides on the
                // spring, the track takes the on colour.
                if let Some(&on) = f.states[k].switches.get(j).filter(|_| !leaves) {
                    let d = 20.0 * pt * if on { -1.0 } else { 1.0 };
                    track.push(
                        &format!("{rid}.knob"),
                        tap.t + PRESS_DOWN,
                        MORPH,
                        Shape::Spring(Spring::Settle),
                        mv([0.0, 0.0], [d, 0.0]),
                    );
                    let (a, b) = if on { (1.0, 0.0) } else { (0.0, 1.0) };
                    track.push(
                        &format!("{rid}.switch_on"),
                        tap.t + PRESS_DOWN,
                        0.2,
                        Shape::Ease(Easing::OutCubic),
                        fade(a, b),
                    );
                }
                let (ink, _) = inks(cx, look.row_on(p.fill));
                track.push(
                    &rid,
                    tap.t,
                    PRESS_DOWN,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Tint {
                        color: ink,
                        from: 0.0,
                        to: 0.08,
                    },
                );
                if !leaves {
                    track.push(
                        &rid,
                        tap.t + PRESS_DOWN,
                        0.3,
                        Shape::Ease(Easing::OutCubic),
                        MotionOp::Tint {
                            color: ink,
                            from: 0.08,
                            to: 0.0,
                        },
                    );
                }
            }
            Hit::Cta if tap.hold <= 0.0 => {
                let cr = p.cta.expect("cta");
                let cid = format!("{gid}.cta");
                track.push(
                    &cid,
                    tap.t,
                    PRESS_DOWN,
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Morph {
                        to: cr.local().shrunk(0.96).path().to_svg(),
                    },
                );
                if !leaves {
                    track.push(
                        &cid,
                        tap.t + PRESS_DOWN,
                        PRESS_BACK,
                        Shape::Spring(Spring::Settle),
                        MotionOp::Morph {
                            to: cr.local().path().to_svg(),
                        },
                    );
                }
            }
            _ => {}
        }
        if tap.hold <= 0.0 {
            continue;
        }
        // Hold: progress across the element, collapse, tick.
        let (el, in_group) = match (tap.hit, p.cta) {
            (Hit::Cta, Some(cr)) => (cr, true),
            _ => (p.rect.local(), false),
        };
        let base = if in_group { look.button } else { p.fill };
        let (on_ink, _) = inks(cx, base);
        let release = tap.t + tap.hold;
        // Progress: a slim bar along the bottom (never under the label) and
        // the button deepening as it fills.
        let pid = format!("{gid}.progress{n}");
        let bar = R {
            x: el.x + el.w * 0.08,
            y: el.y + el.h - 7.0 * pt,
            w: el.w * 0.84,
            h: 3.0 * pt,
            r: 1.5 * pt,
        };
        let mut prog = shape(
            &pid,
            &bar.local().path(),
            [bar.x, bar.y],
            [bar.w, bar.h],
            Some(alpha(on_ink, 0.9)),
        );
        prog.z_index = 3;
        prog.mask = Some(LayerMask {
            d: path::rounded_rect(-1.0, -2.0, 0.5, bar.h + 4.0, 0.0).to_svg(),
            invert: false,
        });
        track.push(
            &pid,
            tap.t,
            tap.hold,
            Shape::Ease(Easing::Linear),
            MotionOp::MaskMorph {
                to: path::rounded_rect(-1.0, -2.0, bar.w + 2.0, bar.h + 4.0, 0.0).to_svg(),
            },
        );
        track.push(
            &pid,
            release,
            OUT,
            Shape::Ease(Easing::OutCubic),
            fade(1.0, 0.0),
        );
        let deep = if oklch(on_ink).l > 0.5 {
            Color::rgb(0, 0, 0)
        } else {
            Color::rgb(255, 255, 255)
        };
        let deepen: Vec<String> = if in_group {
            vec![format!("{gid}.cta")]
        } else {
            fills.clone()
        };
        for did in &deepen {
            track.push(
                did,
                tap.t,
                tap.hold,
                Shape::Ease(Easing::Linear),
                MotionOp::Tint {
                    color: deep,
                    from: 0.0,
                    to: 0.14,
                },
            );
            track.push(
                did,
                release,
                MORPH,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Tint {
                    color: deep,
                    from: 0.14,
                    to: 0.0,
                },
            );
        }
        let d = el.h;
        if in_group {
            let cid = format!("{gid}.cta");
            track.push(
                &cid,
                tap.t,
                PRESS_DOWN,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Morph {
                    to: el.local().shrunk(0.96).path().to_svg(),
                },
            );
            track.push(
                &cid,
                release,
                MORPH,
                Shape::Spring(Spring::Settle),
                MotionOp::Morph {
                    to: R {
                        x: (el.w - d) / 2.0,
                        y: 0.0,
                        w: d,
                        h: d,
                        r: d / 2.0,
                    }
                    .path()
                    .to_svg(),
                },
            );
            track.push(
                &format!("{cid}.label"),
                release,
                OUT,
                Shape::Ease(Easing::OutCubic),
                fade(1.0, 0.0),
            );
        } else {
            for kid in &p.kids {
                if kid.id.ends_with("label")
                    || kid.id.ends_with(".icon")
                    || kid.id.ends_with(".tick")
                {
                    track.push(
                        &kid.id,
                        release,
                        OUT,
                        Shape::Ease(Easing::OutCubic),
                        fade(1.0, 0.0),
                    );
                }
            }
        }
        // The tick draws on 80 ms after the circle settles.
        let ts = d * 0.52;
        let c_local = [el.x + el.w / 2.0, el.y + el.h / 2.0];
        let tid = format!("{gid}.done{n}");
        let mut tick = stroke_shape(
            &tid,
            &tick_path([0.0, 0.0], ts),
            [c_local[0] - ts / 2.0, c_local[1] - ts / 2.0],
            [ts, ts],
            Stroke {
                color: on_ink,
                width: ts * 2.25 / 24.0 * 1.6,
                brush: None,
            },
        );
        tick.z_index = 5;
        track.push(
            &tid,
            0.0,
            frame,
            Shape::Ease(Easing::Linear),
            MotionOp::Trim { from: 0.0, to: 0.0 },
        );
        track.push(
            &tid,
            release + 0.30 + 0.08,
            TICK,
            Shape::Ease(Easing::OutQuint),
            MotionOp::Trim { from: 0.0, to: 1.0 },
        );
        if let Some(LayerKind::Group { children }) =
            groups.iter_mut().find(|g| g.id == gid).map(|g| &mut g.kind)
        {
            children.push(prog);
            children.push(tick);
        }
    }
    layers.extend(groups);

    // ---- toasts out of the island.
    for (n, &(t, hold, k)) in toasts.iter().enumerate() {
        let r = rect_of(k);
        let tid = format!("{id}.toast{n}");
        let from = st.island.unwrap_or(R {
            x: r.x + r.w * 0.2,
            y: r.y - 24.0 * pt,
            w: r.w * 0.6,
            h: r.h * 0.6,
            r: r.h * 0.3,
        });
        // On phones the island itself grows: black.
        let colour = if st.phone {
            Color::rgb(0, 0, 0)
        } else {
            plans[k].fill
        };
        let toast_look = Look {
            surface: colour,
            screen: if st.phone { look.surface } else { look.screen },
            ..look
        };
        let mut bubble = shape(&tid, &from.path(), st.origin, sbox, Some(colour));
        if !st.phone {
            set_shadows(&mut bubble, elevation(pt, true, look.shade));
        }
        bubble.z_index = 6;
        let t_out = t + TOAST_IN + hold;
        track.push(
            &tid,
            0.0,
            frame,
            Shape::Ease(Easing::Linear),
            fade(0.0, 0.0),
        );
        track.push(&tid, t, frame, Shape::Ease(Easing::Linear), fade(0.0, 1.0));
        track.push(
            &tid,
            t,
            TOAST_IN,
            Shape::Spring(Spring::Settle),
            MotionOp::Morph {
                to: r.path().to_svg(),
            },
        );
        track.push(
            &tid,
            t_out,
            TOAST_OUT,
            Shape::Ease(Easing::InCubic),
            MotionOp::Morph {
                to: from.path().to_svg(),
            },
        );
        track.push(
            &tid,
            t_out + TOAST_OUT,
            frame,
            Shape::Ease(Easing::Linear),
            fade(1.0, 0.0),
        );
        let mut s2 = f.states[k].clone();
        s2.fill = None;
        let gid = format!("{tid}.c");
        let kids = plan(cx, &ty, &toast_look, &st, &s2, &gid).kids;
        let mut g = layer(
            &gid,
            LayerKind::Group { children: kids },
            to_canvas(r.centre()),
            [r.w, r.h],
            7,
        );
        g.mask = Some(LayerMask {
            d: from.at(-r.x, -r.y).path().to_svg(),
            invert: false,
        });
        track.push(
            &gid,
            t,
            TOAST_IN,
            Shape::Spring(Spring::Settle),
            MotionOp::MaskMorph {
                to: r.at(-r.x, -r.y).path().to_svg(),
            },
        );
        track.push(
            &gid,
            t_out,
            TOAST_OUT,
            Shape::Ease(Easing::InCubic),
            MotionOp::MaskMorph {
                to: from.at(-r.x, -r.y).path().to_svg(),
            },
        );
        track.push(
            &gid,
            0.0,
            frame,
            Shape::Ease(Easing::Linear),
            fade(0.0, 0.0),
        );
        track.push(
            &gid,
            t + 0.12,
            IN_FADE,
            Shape::Ease(Easing::OutCubic),
            fade(0.0, 1.0),
        );
        track.push(
            &gid,
            t + 0.12,
            TOAST_IN,
            Shape::Spring(Spring::Settle),
            mv([0.0, -8.0 * pt], [0.0, 0.0]),
        );
        track.push(
            &gid,
            t_out,
            0.12,
            Shape::Ease(Easing::InCubic),
            fade(1.0, 0.0),
        );
        let tk = format!("{gid}.icon.tick");
        let has_tick =
            matches!(&g.kind, LayerKind::Group { children } if children.iter().any(|l| l.id == tk));
        if has_tick {
            track.push(
                &tk,
                0.0,
                frame,
                Shape::Ease(Easing::Linear),
                MotionOp::Trim { from: 0.0, to: 0.0 },
            );
            track.push(
                &tk,
                t + 0.32,
                TICK,
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }
        layers.push(bubble);
        layers.push(g);
    }

    // ---- the finger (phones) or the pointer, and ripples.
    if !taps.is_empty() {
        let tid = format!("{id}.touch");
        let mut actor = if st.phone {
            let d = 44.0 * pt;
            let mut l = shape(
                &tid,
                &circle([d / 2.0, d / 2.0], d / 2.0),
                [0.0, 0.0],
                [d, d],
                Some(alpha(Color::rgb(255, 255, 255), 0.35)),
            );
            if let LayerKind::Path {
                stroke, shadows, ..
            } = &mut l.kind
            {
                *stroke = Some(Stroke {
                    color: alpha(look.shade, 0.12),
                    width: (1.0 * pt).max(1.0),
                    brush: None,
                });
                *shadows = vec![ContactShadow {
                    color: alpha(look.shade, 0.15),
                    offset: [0.0, 4.0 * pt],
                    blur: 4.0 * pt,
                }];
            }
            l.anchor_x = 0.5;
            l.anchor_y = 0.5;
            l
        } else {
            // The macOS arrow: black, a white keyline, a soft shadow; tip
            // at the box origin.
            let s = 22.0 * pt / 17.8;
            let arrow = path::polygon(
                &[
                    [0.0, 0.0],
                    [0.0, 15.5],
                    [3.6, 12.1],
                    [6.1, 17.8],
                    [8.4, 16.8],
                    [5.9, 11.2],
                    [10.8, 11.2],
                ]
                .map(|q| [q[0] * s, q[1] * s]),
            );
            let s = 17.8 * s;
            let mut l = shape(&tid, &arrow, [0.0, 0.0], [s, s], Some(Color::rgb(0, 0, 0)));
            if let LayerKind::Path {
                stroke, shadows, ..
            } = &mut l.kind
            {
                *stroke = Some(Stroke {
                    color: Color::rgb(255, 255, 255),
                    width: (1.3 * pt).max(1.5),
                    brush: None,
                });
                *shadows = vec![ContactShadow {
                    color: alpha(look.shade, 0.3),
                    offset: [0.0, 1.0 * pt],
                    blur: 2.5 * pt,
                }];
            }
            l
        };
        actor.z_index = 9;
        let start = to_canvas([st.w * 0.86, st.h * 1.08]);
        actor.x = start[0];
        actor.y = start[1];
        let mut at = [0.0f32, 0.0];
        let mut free = 0.0f64;
        let mut op = Opacity {
            level: 0.0,
            busy: 0.0,
        };
        for (n, tap) in taps.iter().enumerate() {
            let p = to_canvas(tap.at);
            let target = [p[0] - start[0], p[1] - start[1]];
            let t0 = (tap.t - TRAVEL).max(free);
            if n == 0 || t0 - free > 0.6 {
                if n > 0 {
                    // Idle: fade out, come back for the next tap.
                    set_opacity(
                        &mut track,
                        &mut op,
                        &tid,
                        free + 0.1,
                        0.25,
                        0.0,
                        Easing::InCubic,
                    );
                }
                set_opacity(&mut track, &mut op, &tid, t0, 0.2, 1.0, Easing::OutCubic);
            }
            let (dx, dy) = (target[0] - at[0], target[1] - at[1]);
            let via = [
                (at[0] + target[0]) / 2.0 - dy * 0.15,
                (at[1] + target[1]) / 2.0 + dx * 0.15,
            ];
            track.push(
                &tid,
                t0,
                (tap.t - t0).max(frame),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Move {
                    from: at,
                    to: target,
                    via: Some(via),
                },
            );
            at = target;
            let release = tap.t + tap.hold.max(PRESS_DOWN);
            track.push(
                &tid,
                tap.t,
                0.06,
                Shape::Ease(Easing::OutCubic),
                scale(1.0, 0.85),
            );
            track.push(
                &tid,
                release,
                PRESS_BACK,
                Shape::Spring(Spring::Pop),
                scale(0.85, 1.0),
            );
            // Ripple in the colour that reads on what was tapped.
            let rid = format!("{id}.ripple{n}");
            let (rc, _) = inks(cx, tap.on);
            let rd = 44.0 * pt;
            let mut ring = shape(
                &rid,
                &circle([rd / 2.0, rd / 2.0], rd / 2.0),
                [0.0, 0.0],
                [rd, rd],
                Some(rc),
            );
            ring.anchor_x = 0.5;
            ring.anchor_y = 0.5;
            ring.x = p[0];
            ring.y = p[1];
            ring.z_index = 8;
            track.push(
                &rid,
                0.0,
                frame,
                Shape::Ease(Easing::Linear),
                fade(0.0, 0.0),
            );
            track.push(
                &rid,
                tap.t,
                frame,
                Shape::Ease(Easing::Linear),
                fade(0.0, 0.3),
            );
            track.push(
                &rid,
                tap.t + frame,
                RIPPLE,
                Shape::Ease(Easing::OutCubic),
                fade(0.3, 0.0),
            );
            track.push(
                &rid,
                tap.t,
                RIPPLE,
                Shape::Ease(Easing::OutCubic),
                scale(0.0, 1.6),
            );
            layers.push(ring);
            free = release + PRESS_BACK;
        }
        // Leaves 0.4 s after the last tap.
        let last = free - PRESS_BACK + 0.4;
        let away = [at[0] + st.w * 0.3, at[1] + st.h * 0.35];
        if last + 0.45 <= length {
            track.push(
                &tid,
                last,
                0.45,
                Shape::Ease(Easing::InCubic),
                MotionOp::Move {
                    from: at,
                    to: away,
                    via: None,
                },
            );
            set_opacity(
                &mut track,
                &mut op,
                &tid,
                last + 0.15,
                0.3,
                0.0,
                Easing::InCubic,
            );
        }
        layers.push(actor);
    }

    // ---- camera: in from a point, frame each state; toasts show the
    // device.
    let full = View {
        zoom: 1.0,
        centre: [cx.w / 2.0, cx.h / 2.0],
    };
    // Framed states land at the slot: centre, or a side column in
    // landscape (leaving the other side to type).
    let landscape = cx.w > cx.h;
    let side = landscape && matches!(f.slot, Slot::Lead | Slot::Trail);
    let width_share = if side {
        0.47
    } else if landscape {
        0.62
    } else {
        0.8
    };
    let spot = match f.slot {
        Slot::Lead if side => [cx.w * 0.26, cx.h / 2.0],
        Slot::Trail if side => [cx.w * 0.74, cx.h / 2.0],
        _ => [cx.w / 2.0, cx.h / 2.0],
    };
    let max_zoom = if st.phone {
        if side {
            2.3
        } else {
            3.0
        }
    } else {
        10.0
    };
    let frame_rect = |r: R, bias: Option<[f32; 2]>| {
        let zoom = (width_share * cx.w / r.w)
            .min(0.8 * cx.h / r.h)
            .clamp(1.0, max_zoom);
        let mut p = to_canvas(r.centre());
        if let Some(b) = bias.filter(|_| !side) {
            let b = to_canvas(b);
            p = [p[0] + (b[0] - p[0]) * 0.15, p[1] + (b[1] - p[1]) * 0.15];
        }
        // The framed point lands on `spot`.
        let c = [
            p[0] - (spot[0] - cx.w / 2.0) / zoom,
            p[1] - (spot[1] - cx.h / 2.0) / zoom,
        ];
        View { zoom, centre: c }
    };
    let mut moves: Vec<Move> = Vec::new();
    let enter_pop = c.timeline.iter().find_map(|e| match e.verb {
        Verb::Enter {
            style: EnterStyle::Pop,
        } => e.at,
        _ => None,
    });
    let mut start_view = full;
    let mut evs: Vec<(f64, View)> = Vec::new();
    let enter_rise = c.timeline.iter().find_map(|e| match e.verb {
        Verb::Enter {
            style: EnterStyle::Rise,
        } => e.at,
        _ => None,
    });
    if let Some(t) = enter_rise {
        // The device rises in while the camera settles from a little wide.
        start_view = View {
            zoom: 0.9,
            centre: full.centre,
        };
        moves.push(Move {
            t0: t,
            t1: t + 1.0,
            to: full,
            curve: Curve::InOutCubic,
        });
    }
    if let Some(t) = enter_pop {
        start_view = View {
            zoom: 0.02,
            centre: full.centre,
        };
        moves.push(Move {
            t0: t,
            t1: t + ENTER,
            to: full,
            curve: Curve::InOutExpo,
        });
    }
    if !st.phone {
        // No hardware to show: the first state is framed from the start.
        let v = frame_rect(rect_of(0), None);
        if moves.is_empty() {
            start_view = v;
        } else {
            moves[0].to = v;
        }
    } else {
        // A small first state (an icon, a pill) is framed once the device
        // is in.
        let r0 = rect_of(0);
        if r0.w * r0.h < 0.15 * st.w * st.h {
            let t0 = moves.last().map_or(0.3, |m| m.t1 + 0.1);
            let first_change = moves_to.first().map_or(length, |m| m.0);
            if t0 + CAMERA <= first_change {
                moves.push(Move {
                    t0,
                    t1: t0 + CAMERA,
                    to: frame_rect(r0, None),
                    curve: Curve::InOutCubic,
                });
            }
        }
    }
    for &(t, _, k, tap) in &moves_to {
        evs.push((t, frame_rect(rect_of(k), tap)));
    }
    for &(t, to) in &zooms {
        let v = match to {
            ZoomTo::In => frame_rect(rect_of(state_at(t)), None),
            ZoomTo::Out => full,
        };
        evs.push((t, v));
    }
    for &(t, _, k) in &toasts {
        let v = if st.phone {
            full
        } else {
            frame_rect(rect_of(k), None)
        };
        // Back on the whole device by the time the island opens, and in on
        // the state again once it has closed.
        evs.push(((t - CAMERA).max(0.0), v));
        let hold = toasts.iter().find(|x| x.0 == t).map_or(1.6, |x| x.1);
        let back = t + TOAST_IN + hold + TOAST_OUT;
        evs.push((back, frame_rect(rect_of(state_at(back)), None)));
    }
    evs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut busy = moves.last().map_or(0.0, |m| m.t1);
    let mut now = moves.last().map_or(start_view, |m| m.to);
    for (t, v) in evs {
        let t0 = t.max(busy);
        if t0 + CAMERA > length {
            continue;
        }
        // Pulling back keeps pace with the morph; pushing in eases.
        let curve = if v.zoom < now.zoom * 0.98 {
            Curve::OutCubic
        } else {
            Curve::InOutCubic
        };
        moves.push(Move {
            t0,
            t1: t0 + CAMERA,
            to: v,
            curve,
        });
        busy = t0 + CAMERA;
        now = v;
    }
    // Holds never sit dead: a slow 3 % push fills every gap between moves
    // (and the tail). Monotonic, never a pulse.
    moves.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    let mut filled: Vec<Move> = Vec::new();
    let mut at = (0.0f64, start_view);
    for m in moves.into_iter().chain(std::iter::once(Move {
        t0: length,
        t1: length,
        to: View {
            zoom: 0.0,
            centre: [0.0, 0.0],
        },
        curve: Curve::InOutCubic,
    })) {
        if m.t0 - at.0 > 0.4 {
            let to = View {
                zoom: at.1.zoom * 1.03,
                centre: at.1.centre,
            };
            filled.push(Move {
                t0: at.0 + 0.05,
                t1: m.t0 - 0.02,
                to,
                curve: Curve::InOutCubic,
            });
            at.1 = to;
        }
        if m.t1 > m.t0 {
            at = (m.t1, m.to);
            filled.push(m);
        }
    }
    let moves = filled;
    // The status bar leaves while the camera is in close (never cut by the
    // frame) and returns with the whole device.
    if st.phone {
        let mut op = Opacity {
            level: 1.0,
            busy: 0.0,
        };
        for m in &moves {
            if m.to.zoom > 1.05 && op.level > 0.5 {
                set_opacity(
                    &mut track,
                    &mut op,
                    &status_id,
                    m.t0,
                    0.2,
                    0.0,
                    Easing::OutCubic,
                );
            } else if m.to.zoom <= 1.05 && op.level < 0.5 {
                set_opacity(
                    &mut track,
                    &mut op,
                    &status_id,
                    (m.t1 - 0.25).max(m.t0),
                    0.25,
                    1.0,
                    Easing::OutCubic,
                );
            }
        }
    }
    let cam = camera(cx, start_view, &moves, length);

    // ---- the device group rides the camera (explicit depth); its box is
    // the hardware (or the stage), so QA sees the device as content.
    let bounds = match f.device {
        Device::Phone => {
            let b = 9.0 * pt + 3.0 * pt;
            [
                st.origin[0] - b,
                st.origin[1] - b,
                st.w + 2.0 * b,
                st.h + 2.0 * b,
            ]
        }
        Device::Laptop => {
            let lw = st.w * 1.2;
            [
                st.origin[0] - (lw - st.w) / 2.0,
                st.origin[1] - st.w * 0.03,
                lw,
                st.h * 1.15,
            ]
        }
        Device::None => [st.origin[0], st.origin[1], st.w, st.h],
    };
    for l in &mut layers {
        l.x -= bounds[0];
        l.y -= bounds[1];
    }
    let mut group = layer(
        id,
        LayerKind::Group { children: layers },
        [bounds[0] + bounds[2] / 2.0, bounds[1] + bounds[3] / 2.0],
        [bounds[2], bounds[3]],
        z,
    );
    group.depth = Some(1.0);
    for e in &c.timeline {
        let Some(at) = e.at else { continue };
        match e.verb {
            Verb::Enter {
                style: EnterStyle::Pop,
            } => {}
            Verb::Enter { style } => {
                let d = crate::taste::duration(&e.verb, cx.tempo);
                if style == EnterStyle::Rise {
                    // Up from below the frame on a settle spring, already
                    // half in at its first frame (never a blank frame).
                    track.push(
                        id,
                        at,
                        0.9,
                        Shape::Spring(Spring::Settle),
                        mv([0.0, (cx.h * 0.45).round()], [0.0, 0.0]),
                    );
                } else {
                    track.push(
                        id,
                        at,
                        d.min(0.4),
                        Shape::Ease(Easing::OutCubic),
                        fade(0.0, 1.0),
                    );
                    track.push(
                        id,
                        at,
                        d.min(0.4),
                        Shape::Spring(Spring::Settle),
                        scale(0.98, 1.0),
                    );
                }
            }
            Verb::Exit { .. } => {
                let d = crate::taste::duration(&e.verb, cx.tempo);
                track.push(id, at, d, Shape::Ease(Easing::InCubic), fade(1.0, 0.0));
            }
            _ => {}
        }
    }
    let mut sc = scene(id, c.start, length, vec![group], track.motions);
    sc.camera = cam;
    vec![sc]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squircle_topology_is_stable() {
        let a = squircle(0.0, 0.0, 300.0, 54.0, 14.0);
        let b = squircle(10.0, 10.0, 54.0, 54.0, 27.0);
        assert_eq!(a.contours[0].segs.len(), b.contours[0].segs.len());
        let bb = b.bounds().unwrap();
        assert!((bb[2] - bb[0] - 54.0).abs() < 0.5, "{bb:?}");
    }

    #[test]
    fn bans_fallback_glyphs() {
        assert_eq!(banned_glyph("Sent ✓"), Some('✓'));
        assert_eq!(banned_glyph("Next →"), Some('→'));
        assert_eq!(banned_glyph("€ 24.00 · ready"), None);
    }
}
