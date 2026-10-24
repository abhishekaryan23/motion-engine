//! Lowering: MotionProgram → MotionScene (v0.2). Pure and deterministic:
//! one scene per component (scene time = component time, so a component
//! exists only between its start and end), z order = component order, every
//! verb expanded from the taste tables. The renderer, QA and export are the
//! v1 ones.

use std::collections::BTreeMap;
use std::path::PathBuf;

use motion_core::{ApproxMeasure, TextMeasure};

use motion_core::easing::Easing;
use motion_core::path::{self, BezPath};
use motion_core::scene::{
    Asset, AssetKind, Canvas, Color, Fill, FontRole, Gradient, GradientStop, Layer, LayerKind,
    LayerMask, Motion, MotionOp, MotionProject, PathFillRule, PostKind, ProjectMeta, Scene, Stroke,
    TextAlign, TextFill, TextStyle, Theme, SCENE_VERSION,
};

use crate::program::{
    Aspect, BackdropPreset, Component, ComponentBody, EnterStyle, ExitStyle, MotionProgram,
    ShapePreset, ShapeSpec, Size, Slot, Swatch, Tempo, TitlePreset, TypeCase, TypeFill, TypeStyle,
    Verb,
};
use crate::taste::{self, Spring, QUICK_FADE};

/// How lowering measures type: the same shaping as the renderer
/// (`motion_render::FontMeasure`) for real output, [`ApproxMeasure`] in
/// font-free tests. `assets` is the asset library root holding `fonts/`.
pub struct Typesetter<'a> {
    pub measure: &'a dyn TextMeasure,
    pub assets: PathBuf,
}

static APPROX: ApproxMeasure = ApproxMeasure;

impl Typesetter<'static> {
    /// Font-free estimates (tests, tooling).
    pub fn approx() -> Self {
        Typesetter {
            measure: &APPROX,
            assets: PathBuf::from("assets"),
        }
    }
}

/// Lowering switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowerOptions {
    /// The design-system layout passes (calm zone, role enforcement,
    /// texture falloff). Off only for A/B checks and regression fixtures.
    pub design_passes: bool,
    /// Layout follows content (per-step re-layout, `flip`). Off only for
    /// the `anticipatory_space` fixture.
    pub follow_content: bool,
    /// Camera moves (authored and default). Off for layout checks.
    pub camera: bool,
}

impl Default for LowerOptions {
    fn default() -> Self {
        LowerOptions {
            design_passes: true,
            follow_content: true,
            camera: true,
        }
    }
}

/// Lowers a program that passed [`crate::validate`]. `asset_root` is the
/// assets directory relative to where the scene file will live.
pub fn lower(p: &MotionProgram, asset_root: Option<String>, ts: &Typesetter) -> MotionProject {
    lower_with(p, asset_root, ts, LowerOptions::default())
}

/// [`lower`] with explicit switches.
pub fn lower_with(
    p: &MotionProgram,
    asset_root: Option<String>,
    ts: &Typesetter,
    opts: LowerOptions,
) -> MotionProject {
    let (w, h) = p.meta.aspect.size();
    let colours = Colours::of(p);
    let fonts = p.fonts();
    let mut font_map = BTreeMap::new();
    let mut assets: Vec<Asset> = Vec::new();
    for (roles, face) in [
        (
            &[FontRole::Display, FontRole::DisplayCondensed][..],
            fonts.display,
        ),
        (&[FontRole::SerifEmotional][..], fonts.serif),
        (&[FontRole::Mono][..], fonts.mono),
    ] {
        let (id, file) = face.asset();
        for r in roles {
            font_map.insert(*r, id.to_string());
        }
        if !assets.iter().any(|a| a.id == id) {
            assets.push(Asset {
                id: id.to_string(),
                kind: AssetKind::Font,
                path: file.to_string(),
                sprite: None,
            });
        }
    }
    // The UI sans (Body/Number roles): one family in two weights, both
    // loaded so the renderer picks the face by `font_weight`.
    for (k, (id, file, _)) in UI_SANS.iter().enumerate() {
        if k == 0 {
            for r in [FontRole::Body, FontRole::Number] {
                font_map.insert(r, id.to_string());
            }
        }
        assets.push(Asset {
            id: id.to_string(),
            kind: AssetKind::Font,
            path: file.to_string(),
            sprite: None,
        });
    }
    let palette = [
        ("background", colours.background),
        ("ink", colours.ink),
        ("accent", colours.accent),
        ("muted", colours.muted),
    ]
    .into_iter()
    .map(|(k, c)| (k.to_string(), c))
    .collect();

    let cx = Ctx {
        follow: opts.follow_content,
        aspect: p.meta.aspect,
        w: w as f32,
        h: h as f32,
        short: (w.min(h)) as f32,
        fps: p.meta.fps.value() as f64,
        tempo: p.tempo(),
        on: grounds_of(p, &colours),
        col: colours,
        fonts,
        ts,
    };
    // Two passes: everything else first, then anchored shapes, which take
    // their geometry from the measured bounds of what they mark.
    let anchored = |c: &Component| {
        matches!(
            &c.body,
            ComponentBody::Shape {
                anchor: Some(_),
                ..
            }
        )
    };
    let mut per: Vec<Vec<Scene>> = p
        .components
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if anchored(c) {
                Vec::new()
            } else {
                lower_component(p, &cx, c, i as i32 * 10, None)
            }
        })
        .collect();
    for (i, c) in p.components.iter().enumerate() {
        if let ComponentBody::Shape {
            anchor: Some(a), ..
        } = &c.body
        {
            let bounds = p
                .components
                .iter()
                .position(|o| &o.id == a)
                .and_then(|k| scene_bounds(&per[k], cx.w * cx.h));
            per[i] = lower_component(p, &cx, c, i as i32 * 10, bounds);
        }
    }
    layer_over(p, &mut per);
    let bounds_of = |id: &str| {
        p.components
            .iter()
            .position(|o| o.id == id)
            .and_then(|k| scene_bounds(&per[k], cx.w * cx.h))
    };
    let mut scenes: Vec<Scene> = Vec::new();
    {
        let mut all: Vec<Scene> = per.iter().flatten().cloned().collect();
        // Overlays (corner labels, chapter names) are screen-space.
        let overlays: Vec<&str> = p
            .components
            .iter()
            .filter(|c| {
                // Corner labels, chapters and any type in a top, bottom or
                // corner slot: the camera moves the content, not these.
                let edge = |s: &Slot| {
                    matches!(
                        s,
                        Slot::Top | Slot::Bottom | Slot::TopLeft | Slot::BottomLeft
                    )
                };
                match &c.body {
                    ComponentBody::Hud { .. } => true,
                    ComponentBody::Title { preset, slot, .. } => {
                        matches!(preset, TitlePreset::Chapter | TitlePreset::Label) || edge(slot)
                    }
                    ComponentBody::KineticText { slot, .. } => edge(slot),
                    _ => false,
                }
            })
            .map(|c| c.id.as_str())
            .collect();
        crate::camera::pin_grounds(&mut all, cx.w * cx.h, &overlays);
        if opts.camera {
            crate::camera::apply(p, &cx, &mut all, &bounds_of);
        }
        scenes.append(&mut all);
    }
    assets.extend(crate::photo::assets(p));
    // (v2.2) Images seen through type (`TypeFill::Image`).
    for c in &p.components {
        if let Some(path) = type_image(&c.body) {
            let id = type_image_id(path);
            if !assets.iter().any(|a| a.id == id) {
                assets.push(Asset {
                    id,
                    kind: AssetKind::Image,
                    path: path.to_string(),
                    sprite: None,
                });
            }
        }
    }
    for c in &p.components {
        if let ComponentBody::Backdrop {
            image: Some(path), ..
        } = &c.body
        {
            let id = format!("backdrop.{}", c.id);
            if !assets.iter().any(|a| a.id == id) {
                assets.push(Asset {
                    id,
                    kind: AssetKind::Image,
                    path: path.clone(),
                    sprite: None,
                });
            }
        }
    }
    if let Some(f) = p.film {
        scenes.push(film(f, p, &cx));
    }

    let mut project = MotionProject {
        version: SCENE_VERSION.to_string(),
        project: ProjectMeta {
            name: p.meta.name.clone(),
            duration_seconds: Some(p.meta.duration),
            exploration: None,
            speech: None,
            art: None,
            direction: None,
        },
        canvas: Canvas {
            width: w,
            height: h,
            fps: p.meta.fps.value(),
            background: cx.col.background,
        },
        theme: Theme {
            fonts: font_map,
            palette,
            ..Theme::default()
        },
        assets,
        asset_root,
        scenes,
        shared: Vec::new(),
        envelopes: Vec::new(),
    };
    // Design system passes (DESIGN_COLOR_SYSTEM.md §3): texture density
    // away from type, type in the role that reads on what is behind it,
    // texture fading smoothly under type.
    if opts.design_passes {
        crate::layout::calm_focus(p, &mut project);
        crate::layout::enforce_roles(&mut project, &cx.col);
    }
    project
}

/// The UI sans (FontRole::Body / Number): `(asset id, file, weight)`.
pub(crate) const UI_SANS: [(&str, &str, u16); 2] = [
    ("font.satoshi_bold", "fonts/Satoshi-Bold.otf", 700),
    ("font.satoshi_medium", "fonts/Satoshi-Medium.otf", 500),
];

/// The program's film finish: a layerless scene spanning the program whose
/// post effects (grain, fringe) run after every other effect.
fn film(f: crate::program::Film, p: &MotionProgram, cx: &Ctx) -> Scene {
    let mut s = scene("film", 0.0, p.meta.duration, Vec::new(), Vec::new());
    let whole = |kind: PostKind| motion_core::scene::PostEffect {
        start: 0.0,
        duration: r6(p.meta.duration),
        easing: Easing::Linear,
        from: 1.0,
        to: 1.0,
        kind,
    };
    if f.fringe > 0.0 {
        s.post.push(whole(PostKind::ChromaticAberration {
            shift: (f.fringe * cx.h / 1080.0).clamp(0.0, 8.0),
            angle_deg: 0.0,
        }));
    }
    if f.grain > 0.0 {
        s.post.push(whole(PostKind::Grain {
            amount: f.grain.clamp(0.0, 0.2),
            seed: 23,
        }));
    }
    s
}

/// What every component lowering needs.
pub(crate) struct Ctx<'a> {
    pub follow: bool,
    pub aspect: Aspect,
    pub w: f32,
    pub h: f32,
    pub short: f32,
    pub fps: f64,
    pub tempo: Tempo,
    pub col: Colours,
    pub fonts: crate::program::Fonts,
    pub ts: &'a Typesetter<'a>,
    /// Component id → the ground it sits on.
    pub on: BTreeMap<String, Color>,
}

/// Line height of every program text layer.
pub(crate) const LINE_HEIGHT: f32 = 1.1;

impl Ctx<'_> {
    /// The ground under component `id`.
    pub fn ground(&self, id: &str) -> Color {
        self.on.get(id).copied().unwrap_or(self.col.background)
    }

    /// Role set on component `id`'s ground.
    pub fn roles(&self, id: &str) -> crate::palette::Roles {
        self.col.roles(self.ground(id))
    }

    /// Swatch `s` used as `u` by component `id`, on its own ground.
    pub fn paint(&self, id: &str, s: Swatch, u: Use) -> Color {
        self.col.paint(self.ground(id), s, u)
    }

    fn face(&self, role: FontRole) -> PathBuf {
        let face = match role {
            FontRole::SerifEmotional => self.fonts.serif,
            FontRole::Mono => self.fonts.mono,
            FontRole::Body | FontRole::Number => return self.ui_face(700),
            _ => self.fonts.display,
        };
        self.ts.assets.join(face.asset().1)
    }

    /// The UI sans file for `weight` (500 or 700).
    pub fn ui_face(&self, weight: u16) -> PathBuf {
        let f = UI_SANS
            .iter()
            .find(|f| f.2 == weight)
            .unwrap_or(&UI_SANS[0]);
        self.ts.assets.join(f.1)
    }

    /// Advance width of one line as the renderer sets it.
    pub fn line_width(&self, text: &str, role: FontRole, size: f32, ls: f32, upper: bool) -> f32 {
        let text = if upper {
            text.to_uppercase()
        } else {
            text.to_string()
        };
        self.ts
            .measure
            .line_width(&self.face(role), 400, false, &text, size, ls)
    }

    /// The largest size (≤ `max_size`) at which `text` fits `max_width`.
    pub fn fit(
        &self,
        text: &str,
        role: FontRole,
        ls: f32,
        upper: bool,
        max_width: f32,
        max_size: f32,
    ) -> f32 {
        let w100 = self.line_width(text, role, 100.0, ls, upper).max(1.0);
        (max_width * 100.0 / w100).min(max_size).floor()
    }

    /// One line of type whose *ink* is centred on `center`: the box is the
    /// measured width (no wrapping) and is shifted by the measured ink
    /// offset, so caps sit optically centred instead of the line box.
    /// Coordinates are taken as given (group-local inside a group); see
    /// [`Ctx::text_canvas`] for top-level type.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &self,
        id: &str,
        text: &str,
        role: FontRole,
        font_size: f32,
        color: Color,
        letter_spacing: f32,
        uppercase: bool,
        center: [f32; 2],
        z: i32,
    ) -> Layer {
        let width = self.line_width(text, role, font_size, letter_spacing, uppercase);
        let height = (font_size * LINE_HEIGHT).round();
        let cased = if uppercase {
            text.to_uppercase()
        } else {
            text.to_string()
        };
        let ink = self.ts.measure.ink_bounds(
            &self.face(role),
            400,
            false,
            &cased,
            font_size,
            LINE_HEIGHT,
            letter_spacing,
        );
        let dy = ink.map_or(0.0, |(top, bottom)| height / 2.0 - (top + bottom) / 2.0);
        let mut l = layer(
            id,
            LayerKind::Text(TextStyle {
                text: text.to_string(),
                font_role: role,
                font_size,
                font_weight: 400,
                italic: false,
                color,
                align: TextAlign::Center,
                line_height: LINE_HEIGHT,
                letter_spacing,
                max_width: None,
                uppercase,
                ink: ink.map(|(top, bottom)| motion_core::scene::InkBounds { top, bottom }),
                fill: None,
                outline: None,
            }),
            [center[0], (center[1] + dy).round()],
            [width.ceil(), height],
            z,
        );
        l.anchor_y = 0.5;
        l
    }

    /// [`Ctx::text`] for a top-level layer in canvas space: the line is
    /// shifted (not clipped) into the 5 % safe area. Never for type inside a
    /// group, whose coordinates are group-local.
    #[allow(clippy::too_many_arguments)]
    pub fn text_canvas(
        &self,
        id: &str,
        text: &str,
        role: FontRole,
        font_size: f32,
        color: Color,
        letter_spacing: f32,
        uppercase: bool,
        center: [f32; 2],
        z: i32,
    ) -> Layer {
        let mut l = self.text(
            id,
            text,
            role,
            font_size,
            color,
            letter_spacing,
            uppercase,
            center,
            z,
        );
        let m = self.short * 0.05;
        let (hw, hh) = (l.width / 2.0, l.height / 2.0);
        if self.w - 2.0 * m > l.width {
            l.x = l.x.clamp(m + hw, self.w - m - hw);
        }
        if self.h - 2.0 * m > l.height {
            l.y = l.y.clamp(m + hh, self.h - m - hh);
        }
        l
    }

    /// Ink height (cap height for caps) of `text` at `size`, if measurable.
    pub fn ink_height(&self, text: &str, role: FontRole, size: f32, ls: f32, upper: bool) -> f32 {
        let cased = if upper {
            text.to_uppercase()
        } else {
            text.to_string()
        };
        self.ts
            .measure
            .ink_bounds(&self.face(role), 400, false, &cased, size, LINE_HEIGHT, ls)
            .map_or(size * 0.72, |(t, b)| b - t)
    }
}

/// Palette colours after the resolver (`palette.rs`): the program-level
/// swatches plus the role set of every ground. Components ask for a role
/// on their own ground ([`Ctx::paint`]), never for a raw colour.
pub(crate) struct Colours {
    pub background: Color,
    pub ink: Color,
    pub accent: Color,
    pub muted: Color,
    pub grounds: Vec<Color>,
    pub resolved: crate::palette::Resolved,
}

/// What a swatch is used for: the role it maps to depends on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Use {
    /// A filled mark or shape.
    Mark,
    /// Type at this size (px).
    Text(f32),
    /// A stroke; `meaning` = it carries information (≥ 3:1).
    Line { meaning: bool },
}

impl Colours {
    pub(crate) fn of(p: &MotionProgram) -> Colours {
        let pal = p.palette().unwrap_or(crate::program::Palette {
            background: "#F2EEE6".into(),
            ink: "#111111".into(),
            accent: "#FF4D00".into(),
            muted: "#8A857C".into(),
            grounds: Vec::new(),
        });
        let resolved = crate::palette::resolve(&pal);
        Colours {
            background: resolved.main.ground,
            ink: resolved.main.ink,
            accent: resolved.main.accent,
            muted: resolved.main.structure,
            grounds: resolved.grounds.iter().map(|r| r.ground).collect(),
            resolved,
        }
    }

    /// Program-level colour of a swatch on the main ground.
    pub fn get(&self, s: Swatch) -> Color {
        match s {
            Swatch::Background => self.background,
            Swatch::Ink => self.ink,
            Swatch::Accent => self.accent,
            Swatch::Muted => self.muted,
            Swatch::Ground1 | Swatch::Ground2 | Swatch::Ground3 | Swatch::Ground4 => self
                .grounds
                .get(ground_index(s))
                .copied()
                .unwrap_or(self.background),
        }
    }

    /// Role set for any ground colour.
    pub fn roles(&self, ground: Color) -> crate::palette::Roles {
        if ground == self.resolved.main.ground {
            return self.resolved.main;
        }
        if let Some(r) = self.resolved.grounds.iter().find(|r| r.ground == ground) {
            return *r;
        }
        crate::palette::roles_on(ground, self.resolved.ink, self.resolved.accent)
    }

    /// A swatch used as `u` on `ground`, mapped to its role there.
    pub fn paint(&self, ground: Color, s: Swatch, u: Use) -> Color {
        let r = self.roles(ground);
        match (s, u) {
            (Swatch::Ink, _) => r.ink,
            (Swatch::Accent, Use::Text(fs)) => r.text(r.accent_text, fs),
            (Swatch::Accent, _) => r.accent,
            (Swatch::Muted, Use::Text(fs)) => r.text(r.ink_secondary, fs),
            (Swatch::Muted, Use::Line { meaning: false }) => r.structure,
            (Swatch::Muted, _) => r.structure_strong,
            (_, Use::Text(fs)) => r.text(self.get(s), fs),
            _ => self.get(s),
        }
    }
}

fn ground_index(s: Swatch) -> usize {
    match s {
        Swatch::Ground1 => 0,
        Swatch::Ground2 => 1,
        Swatch::Ground3 => 2,
        _ => 3,
    }
}

/// Intentional layering: every layer of a component declared `over`
/// another is lifted above that component's top layer, whatever the
/// declaration order (chains resolve in order; validation rejects loops).
fn layer_over(p: &MotionProgram, per: &mut [Vec<Scene>]) {
    let z_range = |scenes: &[Scene]| {
        scenes
            .iter()
            .flat_map(|s| s.layers.iter().map(|l| l.z_index))
            .fold(None, |r: Option<(i32, i32)>, z| {
                Some(r.map_or((z, z), |(lo, hi)| (lo.min(z), hi.max(z))))
            })
    };
    for _ in 0..p.components.len() {
        let mut moved = false;
        for (i, c) in p.components.iter().enumerate() {
            let Some(k) = c
                .over
                .as_deref()
                .and_then(|t| p.components.iter().position(|o| o.id == t))
            else {
                continue;
            };
            let (Some((_, top)), Some((low, _))) = (z_range(&per[k]), z_range(&per[i])) else {
                continue;
            };
            if low <= top {
                let lift = top - low + 1;
                for l in per[i].iter_mut().flat_map(|s| s.layers.iter_mut()) {
                    l.z_index += lift;
                }
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

/// The ground each component sits on: the colour of the nearest backdrop
/// below it (earlier in the list) that is on screen when it starts, else
/// the background.
fn grounds_of(p: &MotionProgram, col: &Colours) -> BTreeMap<String, Color> {
    let mut out = BTreeMap::new();
    for (i, c) in p.components.iter().enumerate() {
        let t = c.start + 0.01;
        let g = p.components[..i]
            .iter()
            .rev()
            .find_map(|b| match &b.body {
                ComponentBody::Backdrop { colour, .. } if b.start <= t && t < p.end_of(b) => {
                    Some(colour.map_or(col.background, |s| col.get(s)))
                }
                _ => None,
            })
            .unwrap_or(col.background);
        out.insert(c.id.clone(), g);
    }
    out
}

/// (v2.2) Title `xl`: the measured line spans this share of the frame width.
pub(crate) const XL_MEASURE: f32 = 0.66;
/// (v2.2) Title `xl`: font size at most this share of the frame height (a
/// word of 4..=8 letters spans ≥ 55 % of the frame; 2-3 letters may stop
/// at this cap).
pub(crate) const XL_MAX_H: f32 = 0.62;
/// (v2.2) Title `xl` fits XL_MEASURE only in slots at least this share of
/// the frame width wide (centre, top, bottom); narrower slots keep the
/// preset share.
pub(crate) const XL_WIDE_SLOT: f32 = 0.55;

/// (v2.2) Whether a line is set in capitals: the preset's case unless the
/// type style asks for `as_written` or `upper`.
pub(crate) fn type_upper(style: Option<&TypeStyle>, preset_upper: bool) -> bool {
    match style.map_or(TypeCase::Preset, |s| s.case) {
        TypeCase::Preset => preset_upper,
        TypeCase::AsWritten => false,
        TypeCase::Upper => true,
    }
}

/// (v2.2) The image path a Title/KineticText `style` shows through its type.
pub(crate) fn type_image(body: &ComponentBody) -> Option<&str> {
    let style = match body {
        ComponentBody::Title { style, .. } | ComponentBody::KineticText { style, .. } => style,
        _ => return None,
    };
    match &style.as_ref()?.fill {
        TypeFill::Image { image } => Some(image.as_str()),
        _ => None,
    }
}

/// (v2.2) Asset id of an image seen through type.
pub(crate) fn type_image_id(path: &str) -> String {
    format!("type.{path}")
}

/// (v2.2) Outline stroke width for type at `font_size` (px).
pub(crate) fn outline_width(font_size: f32) -> f32 {
    (font_size * 0.025).max(2.0)
}

/// (v2.2) Apply a type style's fill and outline to a lowered text layer on
/// `ground`. Gradient ends are the swatches as type on that ground (the
/// same role mapping as solid type); the gradient runs across the line's
/// measured box at `angle_deg` (0 = left to right). Case is applied when the
/// line is set ([`type_upper`]). No style (or a solid, filled one) leaves
/// the layer byte-identical.
pub(crate) fn apply_type_style(
    col: &Colours,
    ground: Color,
    l: &mut Layer,
    style: Option<&TypeStyle>,
) {
    let Some(style) = style else {
        return;
    };
    let (w, h) = (l.width, l.height);
    let LayerKind::Text(ts) = &mut l.kind else {
        return;
    };
    match &style.fill {
        TypeFill::Solid => {}
        TypeFill::Gradient {
            from,
            to,
            angle_deg,
        } => {
            let a = angle_deg.to_radians();
            let (dx, dy) = (a.cos(), a.sin());
            let half = (w / 2.0 * dx).abs() + (h / 2.0 * dy).abs();
            let c = [w / 2.0, h / 2.0];
            let fs = ts.font_size;
            ts.fill = Some(TextFill::Gradient(Gradient::Linear {
                from: [r2(c[0] - dx * half), r2(c[1] - dy * half)],
                to: [r2(c[0] + dx * half), r2(c[1] + dy * half)],
                stops: vec![
                    GradientStop {
                        at: 0.0,
                        color: col.paint(ground, *from, Use::Text(fs)),
                    },
                    GradientStop {
                        at: 1.0,
                        color: col.paint(ground, *to, Use::Text(fs)),
                    },
                ],
            }));
        }
        TypeFill::Image { image } => ts.fill = Some(TextFill::Image(type_image_id(image))),
    }
    if style.outline {
        ts.outline = Some(outline_width(ts.font_size));
    }
}

fn r2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

pub(crate) fn layer(id: &str, kind: LayerKind, center: [f32; 2], size: [f32; 2], z: i32) -> Layer {
    Layer {
        id: id.to_string(),
        x: center[0],
        y: center[1],
        width: size[0],
        height: size[1],
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_degrees: 0.0,
        anchor_x: 0.5,
        anchor_y: 0.5,
        opacity: 1.0,
        z_index: z,
        visible: true,
        clip: None,
        mask: None,
        depth: None,
        z: None,
        tilt: None,
        layout: None,
        kind,
    }
}

/// Collects a scene's motions with ids `<target>.<n>`.
#[derive(Default)]
pub(crate) struct Track {
    pub motions: Vec<Motion>,
}

impl Track {
    pub fn push(
        &mut self,
        target: &str,
        start: f64,
        duration: f64,
        shape: taste::Shape,
        op: MotionOp,
    ) {
        let (easing, spring) = match shape {
            taste::Shape::Ease(e) => (e, None),
            taste::Shape::Spring(s) => (Easing::Linear, Some(s.spec())),
        };
        self.motions.push(Motion {
            id: Some(format!("{target}.{}", self.motions.len())),
            target: target.to_string(),
            start: r6(start),
            duration: r6(duration),
            easing,
            spring,
            op,
        });
    }
}

pub(crate) fn scene(
    id: &str,
    start: f64,
    length: f64,
    layers: Vec<Layer>,
    motions: Vec<Motion>,
) -> Scene {
    Scene {
        id: id.to_string(),
        start_seconds: r6(start),
        duration_seconds: r6(length),
        layers,
        motions,
        camera: None,
        lifecycle: None,
        post: Vec::new(),
    }
}

use taste::Shape::{Ease, Spring as Spr};

/// Union of the top-level layer boxes of `scenes` (base geometry), ignoring
/// full-frame grounds: what an anchored shape marks.
fn scene_bounds(scenes: &[Scene], frame_area: f32) -> Option<[f32; 4]> {
    let mut r: Option<[f32; 4]> = None;
    for s in scenes {
        for l in s.layers.iter().filter(|l| l.visible) {
            let (w, h) = (l.width * l.scale_x, l.height * l.scale_y);
            if w * h >= frame_area * 0.9 || w * h <= 1.0 {
                continue;
            }
            // Where the last re-layout (`flip`) leaves it: the final layout.
            let off = crate::flip::final_offset(s, &l.id);
            let (x0, y0) = (l.x + off[0] - l.anchor_x * w, l.y + off[1] - l.anchor_y * h);
            let b = [x0, y0, x0 + w, y0 + h];
            r = Some(r.map_or(b, |q| {
                [
                    q[0].min(b[0]),
                    q[1].min(b[1]),
                    q[2].max(b[2]),
                    q[3].max(b[3]),
                ]
            }));
        }
    }
    r
}

fn apply_component_25d(c: &Component, scenes: &mut [Scene], tempo: f64) {
    let has_tilt_verb = c
        .timeline
        .iter()
        .any(|e| matches!(e.verb, Verb::Tilt { .. }));
    if c.depth.is_none() && c.z.is_none() && c.tilt.is_none() && !has_tilt_verb {
        return;
    }
    for s in scenes.iter_mut() {
        for l in &mut s.layers {
            if c.depth.is_some() && l.depth.is_none() {
                l.depth = c.depth;
            }
            if c.z.is_some() && l.z.is_none() {
                l.z = c.z;
            }
            if c.tilt.is_some() && l.tilt.is_none() {
                l.tilt = c.tilt;
            }
        }
        for e in &c.timeline {
            if let Verb::Tilt { from, to } = &e.verb {
                let at = e.at.unwrap_or(0.0);
                let d = (0.65 * tempo).min(s.duration_seconds - at);
                for l in &s.layers {
                    s.motions.push(motion_core::scene::Motion {
                        id: None,
                        target: l.id.clone(),
                        start: r6(at),
                        duration: r6(d),
                        easing: Easing::Linear,
                        spring: Some(taste::Spring::Settle.spec()),
                        op: MotionOp::Tilt {
                            from: *from,
                            to: *to,
                        },
                    });
                }
            }
        }
    }
}

fn lower_component(
    p: &MotionProgram,
    cx: &Ctx,
    c: &Component,
    z: i32,
    anchor_bounds: Option<[f32; 4]>,
) -> Vec<Scene> {
    let col = &cx.col;
    let (wf, hf, short) = (cx.w, cx.h, cx.short);
    let length = r6(p.end_of(c) - c.start);
    let id = c.id.as_str();
    let early = match &c.body {
        ComponentBody::KineticText {
            preset,
            words,
            stress,
            slot,
            colour,
            dot,
            ground,
            style,
            ..
        } => {
            let k = crate::kinetic::Kinetic {
                style: style.as_ref(),
                ground: *ground,
                preset: *preset,
                words,
                stress,
                slot: *slot,
                colour: *colour,
                dot: *dot,
            };
            let length = crate::kinetic::closing_length(p, c).unwrap_or(length);
            Some(crate::kinetic::lower(cx, c, &k, z, length))
        }
        ComponentBody::Chart {
            preset,
            chart,
            values,
            labels,
            hero,
            callout,
            slot,
            size,
            colour,
            title,
            ..
        } => {
            let ch = crate::chart::Chart {
                title: title.as_deref(),
                sized_for_callout: false,
                colour: *colour,
                preset: *preset,
                kind: *chart,
                values,
                labels,
                hero: *hero,
                callout: callout.as_deref(),
                slot: *slot,
                size: *size,
            };
            let free = free_band(p, c, cx);
            let mut fin = crate::chart::lower(cx, c, &ch, z, length, free);
            // The callout makes its room when it pops (layout follows content).
            let pop = c
                .timeline
                .iter()
                .find(|e| matches!(e.verb, Verb::PopLabel))
                .and_then(|e| e.at);
            if let (Some(at), Some(_), true) = (pop, ch.callout, cx.follow) {
                let before = crate::chart::lower(
                    cx,
                    c,
                    &crate::chart::Chart {
                        callout: None,
                        sized_for_callout: true,
                        ..ch
                    },
                    z,
                    length,
                    free,
                );
                crate::flip::follow(&mut fin, &[(at, before)], &|_| false);
            }
            Some(vec![fin])
        }
        ComponentBody::Field {
            preset,
            focus,
            colour,
        } => Some(vec![crate::field::lower(
            cx, c, *preset, *focus, *colour, z, length,
        )]),
        ComponentBody::Diagram {
            preset,
            nodes,
            edges,
            colour,
            slot,
            ..
        } => {
            let free = free_band(p, c, cx);
            let mut fin = crate::diagram::lower(
                cx, c, *preset, *slot, nodes, nodes, edges, *colour, z, length, free,
            );
            // One layout per step from the nodes shown so far (layout
            // follows content): siblings make room as each node lands.
            let mut adv: Vec<f64> = c
                .timeline
                .iter()
                .filter(|e| matches!(e.verb, Verb::Advance))
                .filter_map(|e| e.at)
                .collect();
            adv.sort_by(f64::total_cmp);
            let steps: Vec<(f64, Scene)> = (1..nodes.len())
                .filter_map(|k| {
                    let at = *adv.get(k - 1)?;
                    let part: Vec<[usize; 2]> = edges
                        .iter()
                        .copied()
                        .filter(|e| e[0] < k && e[1] < k)
                        .collect();
                    let mut ck = c.clone();
                    ck.timeline.retain(|e| {
                        !matches!(e.verb, Verb::Advance) || e.at.is_some_and(|t| t < at)
                    });
                    let s = crate::diagram::lower(
                        cx,
                        &ck,
                        *preset,
                        *slot,
                        &nodes[..k],
                        nodes,
                        &part,
                        *colour,
                        z,
                        length,
                        free,
                    );
                    Some((at, s))
                })
                .collect();
            let dot = format!("{}.dot", c.id);
            if !cx.follow {
                Some(vec![fin])
            } else {
                crate::flip::follow(&mut fin, &steps, &|l| l == dot);
                // Token moves: each edge's ride from the layout it was drawn in.
                let is_ride = |m: &motion_core::scene::Motion| {
                    m.target == dot && matches!(m.op, MotionOp::Move { .. })
                };
                if !steps.is_empty() {
                    // Step j (j + 1 nodes) owns the ride into node j: its
                    // moves after the previous arrival.
                    let after = |j: usize| if j >= 2 { adv[j - 2] + 1e-6 } else { f64::MIN };
                    let mut rides: Vec<motion_core::scene::Motion> = Vec::new();
                    for (j, (_, s)) in steps.iter().enumerate().skip(1) {
                        rides.extend(
                            s.motions
                                .iter()
                                .filter(|m| is_ride(m) && m.start > after(j) && m.start > 0.0)
                                .cloned(),
                        );
                    }
                    let last = after(steps.len());
                    fin.motions
                        .retain(|m| !is_ride(m) || m.start > last || m.start == 0.0);
                    for (i, mut m) in rides.into_iter().enumerate() {
                        m.id = Some(format!("{dot}.ride{i}"));
                        fin.motions.push(m);
                    }
                }
                Some(vec![fin])
            }
        }
        ComponentBody::Hud { corners } => Some(vec![crate::field::hud(cx, c, corners, z, length)]),
        ComponentBody::CodeBlock(b) => Some(crate::code::lower(cx, c, b, z, length)),
        ComponentBody::DeviceFrame(f) => Some(crate::device::lower(cx, c, f, z, length)),
        ComponentBody::Bento(b) => {
            let total = b.tiles.len();
            let mut fin = crate::bento::lower(cx, c, b, total, z, length);
            let mut adv: Vec<f64> = c
                .timeline
                .iter()
                .filter(|e| matches!(e.verb, Verb::Advance))
                .filter_map(|e| e.at)
                .collect();
            adv.sort_by(f64::total_cmp);
            if cx.follow && !adv.is_empty() && total > 1 {
                let steps: Vec<(f64, Scene)> = (1..total)
                    .zip(adv.iter())
                    .map(|(k, &at)| (at, crate::bento::lower(cx, c, b, k, z, length)))
                    .collect();
                crate::flip::follow(&mut fin, &steps, &|_| false);
            }
            Some(vec![fin])
        }
        ComponentBody::Lockup(l) => Some(crate::lockup::lower(cx, c, l, z, length)),
        ComponentBody::PhotoStack(ps) => Some(crate::photo::lower(cx, c, ps, z, length)),
        ComponentBody::Sketch(sk) => Some(vec![crate::sketch::lower(cx, c, sk, z, length)]),
        _ => None,
    };
    if let Some(mut scs) = early {
        apply_component_25d(c, &mut scs, cx.tempo.factor());
        return scs;
    }
    let rise = (short * 0.06).round();

    // Base layer and the slot it starts in.
    let (mut base, slot) = match &c.body {
        ComponentBody::Backdrop { preset, colour, .. } => {
            let bg = colour.map_or(col.background, |s| col.get(s));
            let kind = match preset {
                BackdropPreset::Flat => LayerKind::Rectangle {
                    fill: bg,
                    stroke: None,
                },
                BackdropPreset::Glow => gradient_rect(
                    wf,
                    hf,
                    Gradient::Radial {
                        center: [wf / 2.0, hf / 2.0],
                        radius: wf.max(hf) * 0.75,
                        stops: vec![
                            GradientStop {
                                at: 0.0,
                                // The glow is a texture-strength lift of the ground.
                                color: cx.col.roles(bg).texture,
                            },
                            GradientStop { at: 1.0, color: bg },
                        ],
                    },
                ),
                BackdropPreset::Fall => gradient_rect(
                    wf,
                    hf,
                    Gradient::Linear {
                        from: [0.0, 0.0],
                        to: [0.0, hf],
                        stops: vec![
                            GradientStop { at: 0.0, color: bg },
                            GradientStop {
                                at: 1.0,
                                color: cx.col.roles(bg).texture,
                            },
                        ],
                    },
                ),
            };
            (
                layer(id, kind, [wf / 2.0, hf / 2.0], [wf, hf], z),
                Slot::Center,
            )
        }
        ComponentBody::Shape {
            preset,
            shape,
            slot,
            size,
            colour,
            anchor,
            mark,
        } => {
            let s = (taste::shape_share(*size) * short).round();
            if let (Some(_), Some(b)) = (anchor, anchor_bounds) {
                // Geometry from the anchor's measured bounds.
                let colour = cx.paint(
                    id,
                    colour.unwrap_or(Swatch::Accent),
                    if matches!(mark.unwrap_or_default(), crate::program::Mark::Ring) {
                        Use::Line { meaning: true }
                    } else {
                        Use::Mark
                    },
                );
                let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
                let (cxm, cym) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                let sw_ = (bh * 0.05).clamp(3.0, 10.0).round();
                let (centre, size, d, fill, stroke) = match mark.unwrap_or_default() {
                    crate::program::Mark::Ring => {
                        let (w, h) = (bw * 1.1 + bh * 0.3, bh * 1.35);
                        (
                            [cxm, cym],
                            [w, h],
                            path::rounded_rect(0.0, 0.0, w, h, h / 2.0).to_svg(),
                            None,
                            Some(Stroke {
                                color: colour,
                                width: sw_,
                                brush: None,
                            }),
                        )
                    }
                    crate::program::Mark::Underline => {
                        let (w, h) = (bw, (bh * 0.08).max(4.0));
                        (
                            [cxm, b[3] + bh * 0.12 + h / 2.0],
                            [w, h],
                            path::rounded_rect(0.0, 0.0, w, h, h / 2.0).to_svg(),
                            Some(Fill::Solid(colour)),
                            None,
                        )
                    }
                    crate::program::Mark::Marker => {
                        let dm = (bh * 0.3).max(8.0);
                        // Beside the right edge, or under its end when the
                        // frame has no room beside it.
                        let beside = b[2] + dm * 1.4 < cx.w - short * 0.05;
                        (
                            if beside {
                                [b[2] + dm * 0.9, cym]
                            } else {
                                [b[2] - dm / 2.0, b[3] + dm * 0.9]
                            },
                            [dm, dm],
                            path::ellipse(0.0, 0.0, dm, dm).to_svg(),
                            Some(Fill::Solid(colour)),
                            None,
                        )
                    }
                };
                let kind = LayerKind::Path {
                    d,
                    fill,
                    stroke,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                };
                let mut l = layer(id, kind, centre, size, z);
                if matches!(mark.unwrap_or_default(), crate::program::Mark::Ring) {
                    l.mask = None;
                }
                (l, *slot)
            } else {
                let colour = cx.paint(
                    id,
                    colour.unwrap_or(Swatch::Accent),
                    match preset {
                        ShapePreset::Solid => Use::Mark,
                        ShapePreset::Outline => Use::Line { meaning: true },
                    },
                );
                let (fill, stroke) = match preset {
                    ShapePreset::Solid => (Some(Fill::Solid(colour)), None),
                    ShapePreset::Outline => (
                        None,
                        Some(Stroke {
                            color: colour,
                            width: (s * 0.035).max(4.0).round(),
                            brush: None,
                        }),
                    ),
                };
                let kind = LayerKind::Path {
                    d: shape_path(shape, s, id).to_svg(),
                    fill,
                    stroke,
                    fill_rule: PathFillRule::EvenOdd,
                    shadows: Vec::new(),
                };
                (
                    layer(id, kind, taste::slot_center(*slot, cx.aspect), [s, s], z),
                    *slot,
                )
            }
        }
        ComponentBody::Title {
            preset,
            text,
            slot,
            size,
            colour,
            style,
        } => {
            let width = taste::slot_width(*slot, cx.aspect);
            let (role, letter_spacing, preset_upper) = match preset {
                TitlePreset::Display => (FontRole::Display, -0.02, false),
                TitlePreset::Serif => (FontRole::SerifEmotional, 0.0, false),
                TitlePreset::Label => (FontRole::Mono, 0.12, true),
                TitlePreset::Chapter => (FontRole::Display, 0.02, true),
                TitlePreset::Stacked => (FontRole::Display, -0.03, true),
            };
            let uppercase = type_upper(style.as_ref(), preset_upper);
            // One line: the size shrinks until the measured width fits.
            let preset_size = cx.fit(
                text,
                role,
                letter_spacing,
                uppercase,
                width,
                (taste::type_share(*preset, *size) * short).round(),
            );
            let font_size = if *size == Size::Xl && width >= wf * XL_WIDE_SLOT {
                // (v2.2) `xl` in a wide slot is a poster line: the measured
                // line spans XL_MEASURE of the frame width (≥ 55 %, owner
                // rule), at most XL_MAX_H of the frame height, and never
                // smaller than before. Narrow slots keep the preset share.
                preset_size.max(cx.fit(
                    text,
                    role,
                    letter_spacing,
                    uppercase,
                    (wf * XL_MEASURE).min(width),
                    (hf * XL_MAX_H).round(),
                ))
            } else {
                preset_size
            };
            let colour = cx.paint(id, colour.unwrap_or(Swatch::Ink), Use::Text(font_size));
            let mut at = taste::slot_center(*slot, cx.aspect);
            if matches!(slot, Slot::TopLeft | Slot::BottomLeft) {
                // Corner slots are left edges: align the measured line.
                let w = cx.line_width(text, role, font_size, letter_spacing, uppercase);
                at[0] += w / 2.0;
            }
            let mut main = cx.text_canvas(
                id,
                text,
                role,
                font_size,
                colour,
                letter_spacing,
                uppercase,
                at,
                z,
            );
            apply_type_style(col, cx.ground(id), &mut main, style.as_ref());
            let layer_out = if *preset == TitlePreset::Stacked {
                // Two offset echoes behind the type (muted farthest, accent
                // nearer), grouped so every verb moves them as one.
                let (gw, gh) = (main.width, main.height);
                let off = (font_size * 0.045).round().max(3.0);
                let mut kids = Vec::new();
                // Echoes are structure (farthest) and accent (nearer) on
                // this title's ground.
                let r = cx.roles(id);
                for (k, sw) in [(2.0, r.structure), (1.0, r.accent)] {
                    let mut e = cx.text_canvas(
                        &format!("{id}.echo{}", k as i32),
                        text,
                        role,
                        font_size,
                        sw,
                        letter_spacing,
                        uppercase,
                        at,
                        0,
                    );
                    e.x = gw / 2.0 + off * k;
                    e.y = (e.y - at[1]) + gh / 2.0 + off * k;
                    kids.push(e);
                }
                let mut m = main.clone();
                m.id = format!("{id}.type");
                m.x = gw / 2.0;
                m.y = (m.y - at[1]) + gh / 2.0;
                m.z_index = 0;
                kids.push(m);
                layer(
                    id,
                    LayerKind::Group { children: kids },
                    at,
                    [gw + off * 2.0, gh + off * 2.0],
                    z,
                )
            } else {
                main
            };
            (layer_out, *slot)
        }
        ComponentBody::KineticText { .. }
        | ComponentBody::Chart { .. }
        | ComponentBody::Field { .. }
        | ComponentBody::Hud { .. }
        | ComponentBody::Diagram { .. }
        | ComponentBody::CodeBlock(_)
        | ComponentBody::DeviceFrame(_)
        | ComponentBody::Bento(_)
        | ComponentBody::Lockup(_)
        | ComponentBody::PhotoStack(_)
        | ComponentBody::Sketch(_) => unreachable!("lowered above"),
    };

    base.depth = c.depth;
    base.z = c.z;
    base.tilt = c.tilt;

    // Shapes need their geometry for morph chains; titles their box for wipes.
    let side = base.width;
    let (bw, bh) = (base.width, base.height);
    let full = path::rounded_rect(-0.1 * bw, -0.5 * bh, 1.2 * bw, 2.0 * bh, 0.0);
    let left = path::rounded_rect(-0.1 * bw, -0.5 * bh, 0.0, 2.0 * bh, 0.0);
    let right = path::rounded_rect(1.1 * bw, -0.5 * bh, 0.0, 2.0 * bh, 0.0);

    let mut events: Vec<&crate::program::Event> = c.timeline.iter().collect();
    events.sort_by(|a, b| a.at.unwrap_or(0.0).total_cmp(&b.at.unwrap_or(0.0)));
    let mut track = Track::default();
    let mut offset = [0.0f32, 0.0];
    let mut scale = 1.0f32;
    let mut turn = 0.0f32;
    let home = taste::slot_center(slot, cx.aspect);

    for e in events {
        let d = taste::duration(&e.verb, cx.tempo);
        let Some(at) = e.at else { continue };
        match &e.verb {
            Verb::Enter { style } => match style {
                EnterStyle::Pop => {
                    track.push(
                        id,
                        at,
                        d,
                        Spr(Spring::Pop),
                        // Already a third of its size on the beat frame:
                        // the hit reads at once, never an empty frame.
                        MotionOp::Scale {
                            from: 0.35,
                            to: 1.0,
                            axis: Default::default(),
                        },
                    );
                    track.push(id, at, 0.06, Ease(Easing::OutCubic), fade(0.0, 1.0));
                }
                EnterStyle::Rise => {
                    track.push(
                        id,
                        at,
                        d,
                        Spr(Spring::Settle),
                        MotionOp::Move {
                            from: [offset[0], offset[1] + rise],
                            to: offset,
                            via: None,
                        },
                    );
                    track.push(
                        id,
                        at,
                        2.0 * QUICK_FADE,
                        Ease(Easing::OutCubic),
                        fade(0.0, 1.0),
                    );
                }
                EnterStyle::Fade => track.push(id, at, d, Ease(Easing::OutCubic), fade(0.0, 1.0)),
                EnterStyle::Draw => track.push(
                    id,
                    at,
                    d,
                    Ease(Easing::InOutCubic),
                    MotionOp::Trim { from: 0.0, to: 1.0 },
                ),
                EnterStyle::Wipe => {
                    base.mask = Some(LayerMask {
                        d: left.to_svg(),
                        invert: false,
                    });
                    track.push(
                        id,
                        at,
                        d,
                        Ease(Easing::InOutCubic),
                        MotionOp::MaskMorph { to: full.to_svg() },
                    );
                }
            },
            Verb::Exit { style } => match style {
                ExitStyle::Fade | ExitStyle::Cut => {
                    track.push(id, at, d, Ease(Easing::InCubic), fade(1.0, 0.0))
                }
                ExitStyle::Shrink => track.push(
                    id,
                    at,
                    d,
                    Ease(Easing::InCubic),
                    MotionOp::Scale {
                        from: 1.0,
                        to: 0.0,
                        axis: Default::default(),
                    },
                ),
                ExitStyle::Drop => {
                    track.push(
                        id,
                        at,
                        d,
                        Ease(Easing::InCubic),
                        MotionOp::Move {
                            from: offset,
                            to: [offset[0], offset[1] + rise],
                            via: None,
                        },
                    );
                    track.push(id, at, d, Ease(Easing::InCubic), fade(1.0, 0.0));
                }
                ExitStyle::Wipe => {
                    if base.mask.is_none() {
                        base.mask = Some(LayerMask {
                            d: full.to_svg(),
                            invert: false,
                        });
                    }
                    track.push(
                        id,
                        at,
                        d,
                        Ease(Easing::InOutCubic),
                        MotionOp::MaskMorph { to: right.to_svg() },
                    );
                }
            },
            Verb::Morph { to } => track.push(
                id,
                at,
                d,
                Spr(Spring::Settle),
                MotionOp::Morph {
                    to: shape_path(to, side, id).to_svg(),
                },
            ),
            Verb::Move { to } => {
                let target = taste::slot_center(*to, cx.aspect);
                let next = [target[0] - home[0], target[1] - home[1]];
                track.push(
                    id,
                    at,
                    d,
                    Spr(Spring::Settle),
                    MotionOp::Move {
                        from: offset,
                        to: next,
                        via: None,
                    },
                );
                offset = next;
            }
            Verb::Recolour { to } => track.push(
                id,
                at,
                d,
                Ease(Easing::OutCubic),
                MotionOp::Tint {
                    color: cx.paint(
                        id,
                        *to,
                        match &base.kind {
                            LayerKind::Text(st) => Use::Text(st.font_size),
                            _ => Use::Mark,
                        },
                    ),
                    from: 0.0,
                    to: 1.0,
                },
            ),
            Verb::Bounce => {
                // Squash (anticipation), stretch on the way up, spring
                // landing with one small overshoot.
                let hop = (short * 0.12).round();
                let (squash, up) = (0.08, 0.18);
                track.push(
                    id,
                    at,
                    squash,
                    Ease(Easing::OutCubic),
                    MotionOp::Scale {
                        from: 1.0,
                        to: 0.84,
                        axis: motion_core::scene::Axis::Y,
                    },
                );
                track.push(
                    id,
                    at + squash,
                    0.36,
                    Spr(Spring::Pop),
                    MotionOp::Scale {
                        from: 0.84,
                        to: 1.0,
                        axis: motion_core::scene::Axis::Y,
                    },
                );
                track.push(
                    id,
                    at + squash,
                    up,
                    Ease(Easing::OutCubic),
                    MotionOp::Move {
                        from: offset,
                        to: [offset[0], offset[1] - hop],
                        via: None,
                    },
                );
                track.push(
                    id,
                    at + squash + up,
                    d - squash - up,
                    Spr(Spring::Pop),
                    MotionOp::Move {
                        from: [offset[0], offset[1] - hop],
                        to: offset,
                        via: None,
                    },
                );
            }
            Verb::Fill => {
                // Cover the frame from where the shape is now: twice the
                // distance to the farthest corner, plus a margin.
                let at_now = [home[0] + offset[0], home[1] + offset[1]];
                let reach = [[0.0, 0.0], [wf, 0.0], [0.0, hf], [wf, hf]]
                    .iter()
                    .map(|q: &[f32; 2]| (q[0] - at_now[0]).hypot(q[1] - at_now[1]))
                    .fold(0.0f32, f32::max);
                let k = (2.0 * reach / side * 1.06).max(1.0);
                track.push(
                    id,
                    at,
                    d,
                    Ease(Easing::InOutCubic),
                    MotionOp::Scale {
                        from: scale,
                        to: k,
                        axis: Default::default(),
                    },
                );
                scale = k;
            }
            Verb::Unfill => {
                track.push(
                    id,
                    at,
                    d,
                    Spr(Spring::Settle),
                    MotionOp::Scale {
                        from: scale,
                        to: 1.0,
                        axis: Default::default(),
                    },
                );
                scale = 1.0;
            }
            Verb::Spin => {
                track.push(
                    id,
                    at,
                    d,
                    Spr(Spring::Settle),
                    MotionOp::Rotate {
                        from: turn,
                        to: turn + 180.0,
                    },
                );
                turn += 180.0;
            }
            Verb::Orbit { to } => {
                let target = taste::slot_center(*to, cx.aspect);
                let next = [target[0] - home[0], target[1] - home[1]];
                // Control point off the chord's middle, a third of its
                // length to the left of travel: an arc, not a line.
                let (dx, dy) = (next[0] - offset[0], next[1] - offset[1]);
                let via = [
                    (offset[0] + next[0]) / 2.0 + dy * 0.35,
                    (offset[1] + next[1]) / 2.0 - dx * 0.35,
                ];
                track.push(
                    id,
                    at,
                    d,
                    Spr(Spring::Settle),
                    MotionOp::Move {
                        from: offset,
                        to: next,
                        via: Some(via),
                    },
                );
                offset = next;
            }
            Verb::Cut { .. } | Verb::PopLabel | Verb::Wave { .. } | Verb::Advance | Verb::Sheen => {
            }
            Verb::Hit => {
                let strike = 0.08;
                track.push(
                    id,
                    at,
                    strike,
                    Ease(Easing::OutCubic),
                    MotionOp::Scale {
                        from: 1.0,
                        to: 0.88,
                        axis: Default::default(),
                    },
                );
                track.push(
                    id,
                    at + strike,
                    d - strike,
                    Spr(Spring::Pop),
                    MotionOp::Scale {
                        from: 0.88,
                        to: 1.0,
                        axis: Default::default(),
                    },
                );
            }
            Verb::Tilt { from, to } => {
                track.push(
                    id,
                    at,
                    d,
                    Spr(Spring::Settle),
                    MotionOp::Tilt {
                        from: *from,
                        to: *to,
                    },
                );
            }
            Verb::Echo { count } => {
                track.push(
                    id,
                    at,
                    d,
                    Ease(Easing::OutCubic),
                    MotionOp::Echo {
                        count: *count,
                        spacing: 0.028,
                        decay: 0.55,
                    },
                );
            }
        }
    }

    let mut layers = vec![base];
    if let ComponentBody::Backdrop {
        image: Some(_),
        image_opacity,
        ..
    } = &c.body
    {
        layers[0].depth = Some(0.0);
        let mut plate = layer(
            &format!("{id}.plate"),
            LayerKind::Image {
                asset: format!("backdrop.{id}"),
                fit: motion_core::scene::Fit::Cover,
                treatment: None,
                insert: None,
                playback: None,
            },
            [wf / 2.0, hf / 2.0],
            [(wf * 1.12).round(), (hf * 1.12).round()],
            z + 1,
        );
        plate.opacity = image_opacity.unwrap_or(0.24).clamp(0.05, 1.0);
        plate.depth = c.depth.or(Some(0.12));
        layers.push(plate);
    }

    vec![scene(id, c.start, length, layers, track.motions)]
}

/// Seconds rounded to the microsecond: computed times stay byte-stable
/// through a JSON round trip.
/// The vertical span left free by other components' top/bottom text that
/// is on screen at the same time as `c` (shared layout avoidance).
pub(crate) fn free_band(p: &MotionProgram, c: &Component, cx: &Ctx) -> [f32; 2] {
    let (s0, e0) = (c.start, p.end_of(c));
    let mut band = [cx.h * 0.08, cx.h * 0.92];
    for o in &p.components {
        if o.id == c.id || !(o.start < e0 - 0.05 && s0 < p.end_of(o) - 0.05) {
            continue;
        }
        let Some(slot) = component_slot(&o.body) else {
            continue;
        };
        let text = matches!(
            o.body,
            ComponentBody::Title { .. } | ComponentBody::KineticText { .. }
        );
        if !text {
            continue;
        }
        let y = taste::slot_center(slot, cx.aspect)[1];
        let pad = cx.short * 0.08;
        match slot {
            Slot::Top | Slot::TopLeft => band[0] = band[0].max(y + pad),
            Slot::Bottom | Slot::BottomLeft => band[1] = band[1].min(y - pad),
            _ => {}
        }
    }
    band
}

/// The slot a component is placed in, when it has one.
fn component_slot(b: &ComponentBody) -> Option<Slot> {
    match b {
        ComponentBody::Shape { slot, .. }
        | ComponentBody::Title { slot, .. }
        | ComponentBody::KineticText { slot, .. }
        | ComponentBody::Chart { slot, .. }
        | ComponentBody::Bento(crate::bento::Bento { slot, .. })
        | ComponentBody::Sketch(crate::sketch::Sketch { slot, .. }) => Some(*slot),
        _ => None,
    }
}

pub(crate) fn r6(t: f64) -> f64 {
    (t * 1e6).round() / 1e6
}

fn fade(from: f32, to: f32) -> MotionOp {
    MotionOp::Fade { from, to }
}

fn gradient_rect(w: f32, h: f32, g: Gradient) -> LayerKind {
    LayerKind::Path {
        d: path::rounded_rect(0.0, 0.0, w, h, 0.0).to_svg(),
        fill: Some(Fill::Gradient(g)),
        stroke: None,
        fill_rule: PathFillRule::Nonzero,
        shadows: Vec::new(),
    }
}

/// FNV-1a of the component id: the seed of seeded shapes.
fn seed_of(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A primitive filling an `s`×`s` box, optically balanced.
pub fn shape_path(spec: &ShapeSpec, s: f32, id: &str) -> BezPath {
    let c = s / 2.0;
    match spec {
        ShapeSpec::Circle => path::ellipse(0.0, 0.0, s, s),
        ShapeSpec::Square => path::rounded_rect(s * 0.06, s * 0.06, s * 0.88, s * 0.88, 0.0),
        ShapeSpec::Rounded => path::rounded_rect(s * 0.04, s * 0.04, s * 0.92, s * 0.92, s * 0.22),
        ShapeSpec::Pill => path::rounded_rect(0.0, s * 0.3, s, s * 0.4, s * 0.2),
        ShapeSpec::Triangle => {
            // Equilateral, centroid on the box centre.
            let r = s * 0.56;
            let pts: Vec<[f32; 2]> = (0..3)
                .map(|k| {
                    let a = (-90.0 + 120.0 * k as f32).to_radians();
                    [c + r * a.cos(), c + s * 0.06 + r * a.sin()]
                })
                .collect();
            path::polygon(&pts)
        }
        ShapeSpec::Diamond => path::polygon(&[[c, 0.0], [s, c], [c, s], [0.0, c]]),
        ShapeSpec::Star => {
            let pts: Vec<[f32; 2]> = (0..10)
                .map(|k| {
                    let r = if k % 2 == 0 { c } else { c * 0.45 };
                    let a = (-90.0 + 36.0 * k as f32).to_radians();
                    [c + r * a.cos(), c + s * 0.03 + r * a.sin()]
                })
                .collect();
            path::polygon(&pts)
        }
        ShapeSpec::Ring => {
            let mut p = path::ellipse(0.0, 0.0, s, s);
            p.contours
                .extend(path::ellipse(s * 0.24, s * 0.24, s * 0.52, s * 0.52).contours);
            p
        }
        ShapeSpec::Blob => blob(s, seed_of(id)),
        ShapeSpec::Path(d) => {
            let mut p = path::parse(d).unwrap_or_default();
            let k = s / 100.0;
            for ct in &mut p.contours {
                ct.start = [ct.start[0] * k, ct.start[1] * k];
                for g in &mut ct.segs {
                    g.c1 = [g.c1[0] * k, g.c1[1] * k];
                    g.c2 = [g.c2[0] * k, g.c2[1] * k];
                    g.to = [g.to[0] * k, g.to[1] * k];
                }
            }
            p
        }
    }
}

/// A smooth closed blob: 6 seeded radii (0.38–0.5 of the side) joined by
/// Catmull-Rom cubics.
fn blob(s: f32, seed: u64) -> BezPath {
    let c = s / 2.0;
    let mut x = seed | 1;
    let pts: Vec<[f32; 2]> = (0..6)
        .map(|k| {
            // xorshift64*
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            let u = (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32;
            let r = s * (0.38 + 0.12 * u);
            let a = (-90.0 + 60.0 * k as f32).to_radians();
            [c + r * a.cos(), c + r * a.sin()]
        })
        .collect();
    let n = pts.len();
    let segs = (0..n)
        .map(|i| {
            let p0 = pts[(i + n - 1) % n];
            let p1 = pts[i];
            let p2 = pts[(i + 1) % n];
            let p3 = pts[(i + 2) % n];
            path::Cubic {
                c1: [p1[0] + (p2[0] - p0[0]) / 6.0, p1[1] + (p2[1] - p0[1]) / 6.0],
                c2: [p2[0] - (p3[0] - p1[0]) / 6.0, p2[1] - (p3[1] - p1[1]) / 6.0],
                to: p2,
            }
        })
        .collect();
    BezPath {
        contours: vec![path::Contour {
            start: pts[0],
            segs,
            closed: true,
        }],
    }
}

/// (v2.3) Point every lowered image asset named `gen:<id>` at its generated
/// file (id → absolute path, from the assets lockfile). Returns the ids that
/// had no path (the caller validated them with
/// [`crate::validate::check_gen_resolved`], so normally none).
pub fn resolve_gen_paths(
    project: &mut MotionProject,
    paths: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut missing = Vec::new();
    for a in &mut project.assets {
        if let Some(id) = a.path.strip_prefix(crate::validate::GEN_PREFIX) {
            match paths.get(id) {
                Some(p) => a.path = p.clone(),
                None => missing.push(id.to_string()),
            }
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ledger `local_clamp`: type inside a group keeps its group-local
    /// position (the frame's safe area applies to top-level type only).
    #[test]
    fn text_is_group_local() {
        let p = MotionProgram::from_json(
            r##"{"version":"2.0","meta":{"name":"t","duration":1.0},
            "palette":{"background":"#FFFFFF","ink":"#111111","accent":"#FF4D00","muted":"#777777"},
            "components":[{"id":"a","kind":"backdrop"}]}"##,
        )
        .expect("program");
        let ts = Typesetter::approx();
        let col = Colours::of(&p);
        let cx = Ctx {
            follow: true,
            aspect: p.meta.aspect,
            w: 1920.0,
            h: 1080.0,
            short: 1080.0,
            fps: 30.0,
            tempo: p.tempo(),
            on: BTreeMap::new(),
            col,
            fonts: p.fonts(),
            ts: &ts,
        };
        // A label in a 125 px group: near the group's corner, far inside
        // the frame's 54 px safe margin.
        let l = cx.text(
            "g.label",
            "Go",
            FontRole::Body,
            20.0,
            Color::rgb(0, 0, 0),
            0.0,
            false,
            [30.0, 20.0],
            0,
        );
        assert_eq!(l.x, 30.0);
        assert!((l.y - 20.0).abs() < 12.0, "y {}", l.y);
        // Top-level type is still kept inside the safe area.
        let t = cx.text_canvas(
            "t",
            "Go",
            FontRole::Body,
            20.0,
            Color::rgb(0, 0, 0),
            0.0,
            false,
            [30.0, 20.0],
            0,
        );
        assert!(t.x >= 54.0 && t.y >= 54.0, "{} {}", t.x, t.y);
    }
}
