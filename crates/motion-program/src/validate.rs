//! Program validation: closed vocabulary, timing that fits, verbs that suit
//! their component, no two verbs fighting over one channel, bans respected
//! and legible colours. Every issue names a JSON path.

use std::collections::HashSet;
use std::fmt;

use motion_core::scene::Color;

use crate::program::{
    BackdropPreset, ChartKind, Component, ComponentBody, EnterStyle, ExitStyle, Feature,
    KineticPreset, MotionProgram, ShapePreset, ShapeSpec, Size, TypeFill, TypeStyle, Verb,
    PROGRAM_VERSION,
};
use crate::taste;

const EPS: f64 = 1e-6;
pub const MAX_COMPONENTS: usize = 64;
pub const MAX_EVENTS: usize = 32;
pub const MAX_DURATION: f64 = 120.0;
/// WCAG contrast ink vs background below which type is illegible.
pub const MIN_INK_CONTRAST: f32 = 4.5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issues(pub Vec<Issue>);

impl fmt::Display for Issues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}: {}", e.path, e.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for Issues {}

/// The animated property a verb writes (two verbs on one channel must not
/// overlap in time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Offset,
    Scale,
    Opacity,
    Trim,
    Mask,
    Path,
    Tint,
    Label,
    Rotation,
    Tilt,
    Echo,
}

/// Channels written by a verb.
pub fn channels(v: &Verb) -> &'static [Channel] {
    use Channel::*;
    match v {
        Verb::Enter { style } => match style {
            EnterStyle::Pop => &[Scale, Opacity],
            EnterStyle::Rise => &[Offset, Opacity],
            EnterStyle::Fade => &[Opacity],
            EnterStyle::Draw => &[Trim],
            EnterStyle::Wipe => &[Mask],
        },
        Verb::Exit { style } => match style {
            ExitStyle::Fade | ExitStyle::Cut => &[Opacity],
            ExitStyle::Shrink => &[Scale],
            ExitStyle::Drop => &[Offset, Opacity],
            ExitStyle::Wipe => &[Mask],
        },
        Verb::Morph { .. } => &[Path],
        Verb::Move { .. } => &[Offset],
        Verb::Recolour { .. } => &[Tint],
        Verb::Hit => &[Scale],
        Verb::Cut { .. } => &[],
        Verb::PopLabel => &[Label],
        Verb::Bounce => &[Offset, Scale],
        Verb::Orbit { .. } => &[Offset],
        Verb::Fill | Verb::Unfill => &[Scale],
        Verb::Spin => &[Rotation],
        Verb::Wave { .. } => &[],
        Verb::Advance => &[],
        Verb::Sheen => &[],
        Verb::Tilt { .. } => &[Tilt],
        Verb::Echo { .. } => &[Echo],
    }
}

/// WCAG 2 contrast ratio of two opaque colours.
pub fn contrast(a: Color, b: Color) -> f32 {
    let lum = |c: Color| {
        let ch = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn file_safe(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

struct V<'a> {
    p: &'a MotionProgram,
    out: Vec<Issue>,
}

impl V<'_> {
    fn err(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.out.push(Issue {
            path: path.into(),
            message: message.into(),
        });
    }
}

/// Validates `p`; `Err` lists every issue found.
pub fn validate(p: &MotionProgram) -> Result<(), Issues> {
    let mut v = V { p, out: Vec::new() };
    v.program();
    if v.out.is_empty() {
        Ok(())
    } else {
        Err(Issues(v.out))
    }
}

/// (v2.3) Prefix of a generated-asset reference in an image path field.
pub const GEN_PREFIX: &str = "gen:";

/// One image path field of a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// JSON path of the field.
    pub path: String,
    pub value: String,
    /// The field places an isolated object (a sprite), not a full picture.
    pub sprite: bool,
}

/// (v2.3) Every image path field in `p`: backdrop plates, images seen
/// through type, and photo-stack sprites.
pub fn image_refs(p: &MotionProgram) -> Vec<ImageRef> {
    let mut out = Vec::new();
    for (i, c) in p.components.iter().enumerate() {
        let cp = format!("components[{i}]");
        match &c.body {
            ComponentBody::Backdrop {
                image: Some(img), ..
            } => out.push(ImageRef {
                path: format!("{cp}.image"),
                value: img.clone(),
                sprite: false,
            }),
            ComponentBody::Title { style, .. } | ComponentBody::KineticText { style, .. } => {
                if let Some(TypeFill::Image { image }) = style.as_ref().map(|s| &s.fill) {
                    out.push(ImageRef {
                        path: format!("{cp}.style.fill.image"),
                        value: image.clone(),
                        sprite: false,
                    });
                }
            }
            ComponentBody::PhotoStack(ps) => {
                for (k, s) in ps.steps.iter().enumerate() {
                    let sp = format!("{cp}.steps[{k}]");
                    if let Some(a) = &s.asset {
                        out.push(ImageRef {
                            path: format!("{sp}.asset"),
                            value: a.clone(),
                            sprite: true,
                        });
                    }
                    for (j, a) in s.assets.iter().enumerate() {
                        out.push(ImageRef {
                            path: format!("{sp}.assets[{j}]"),
                            value: a.clone(),
                            sprite: true,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// (v2.3) Every `gen:<id>` the program uses must be resolved in `resolved`
/// (id → file, from the assets lockfile). Read-only: no provider is called.
pub fn check_gen_resolved(
    p: &MotionProgram,
    resolved: &std::collections::BTreeMap<String, String>,
) -> Result<(), Issues> {
    let mut out = Vec::new();
    for r in image_refs(p) {
        if let Some(id) = r.value.strip_prefix(GEN_PREFIX) {
            if p.assets.iter().any(|a| a.id == id) && !resolved.contains_key(id) {
                out.push(Issue {
                    path: r.path,
                    message: format!(
                        "generated asset '{id}' is not resolved (missing from the assets lockfile or its file changed); run `program assets`"
                    ),
                });
            }
        }
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(Issues(out))
    }
}

impl V<'_> {
    /// (v2.3) `assets` entries are well formed, and every `gen:<id>` names
    /// one whose kind fits the field (a plate is never a sprite).
    fn gen_assets(&mut self) {
        let p = self.p;
        let mut seen = HashSet::new();
        for (i, a) in p.assets.iter().enumerate() {
            let ok_id = !a.id.is_empty()
                && a.id.len() <= 64
                && a.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !ok_id {
                self.err(
                    format!("assets[{i}].id"),
                    "1-64 lowercase letters, digits or _",
                );
            }
            if !seen.insert(a.id.as_str()) {
                self.err(
                    format!("assets[{i}].id"),
                    format!("duplicate id '{}'", a.id),
                );
            }
            let words = a.prompt.split_whitespace().count();
            if !(3..=120).contains(&words) {
                self.err(
                    format!("assets[{i}].prompt"),
                    "describe the image in 3-120 words",
                );
            }
        }
        for r in image_refs(p) {
            let Some(id) = r.value.strip_prefix(GEN_PREFIX) else {
                continue;
            };
            match p.assets.iter().find(|a| a.id == id) {
                None => self.err(r.path, format!("no generated asset '{id}' in `assets`")),
                Some(_) if r.sprite => self.err(
                    r.path,
                    "photo_stack places bundled cutouts by name; gen: is not supported here",
                ),
                Some(_) => {}
            }
        }
    }
}

/// (v2.2) Where a `TypeFill::Image` path is looked up: `assets/` under the
/// working directory, then the workspace's own `assets/`.
fn type_image_exists(path: &str) -> bool {
    let rel = std::path::Path::new(path);
    [
        std::path::PathBuf::from("assets"),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"),
    ]
    .iter()
    .any(|root| root.join(rel).is_file())
}

impl V<'_> {
    /// (v2.2) A type style: an image fill names a raster under `assets/`
    /// that exists (missing → an issue, never a render-time panic); a
    /// gradient's ends differ; the angle is finite.
    fn type_style(&mut self, cp: &str, style: Option<&TypeStyle>) {
        let Some(style) = style else {
            return;
        };
        match &style.fill {
            TypeFill::Solid => {}
            TypeFill::Gradient {
                from,
                to,
                angle_deg,
            } => {
                if from == to {
                    self.err(
                        format!("{cp}.style.fill"),
                        "a gradient needs two different swatches",
                    );
                }
                if !angle_deg.is_finite() {
                    self.err(format!("{cp}.style.fill.angle_deg"), "must be finite");
                }
            }
            // (v2.3) `gen:<id>` is checked by `gen_assets`.
            TypeFill::Image { image } if image.starts_with(GEN_PREFIX) => {}
            TypeFill::Image { image } => {
                let lower = image.to_ascii_lowercase();
                let raster = [".png", ".jpg", ".jpeg", ".webp"]
                    .iter()
                    .any(|e| lower.ends_with(e));
                let safe = !image.is_empty()
                    && !image.starts_with('/')
                    && !image.split(['/', '\\']).any(|seg| seg == "..");
                if !safe || !raster {
                    self.err(
                        format!("{cp}.style.fill.image"),
                        "a .png/.jpg/.webp path relative to assets/ (no '..')",
                    );
                } else if !type_image_exists(image) {
                    self.err(
                        format!("{cp}.style.fill.image"),
                        format!("no image at assets/{image}"),
                    );
                }
            }
        }
    }

    fn program(&mut self) {
        let p = self.p;
        if p.version != PROGRAM_VERSION {
            self.err(
                "version",
                format!("expected \"{PROGRAM_VERSION}\", got \"{}\"", p.version),
            );
        }
        if !file_safe(&p.meta.name) {
            self.err("meta.name", "use 1-64 lowercase letters, digits or _");
        }
        let d = p.meta.duration;
        if !(d.is_finite() && d > 0.0 && d <= MAX_DURATION) {
            self.err(
                "meta.duration",
                format!("must be in (0, {MAX_DURATION}] seconds, got {d}"),
            );
        }
        self.palette();
        self.gen_assets();
        if let Some(m) = &self.p.music {
            if !file_safe(&m.track) {
                self.err("music.track", "a bundled track name (lowercase, digits, _)");
            }
            if !(m.start.is_finite() && m.start >= 0.0) {
                self.err("music.start", "must be >= 0");
            }
        }

        if p.components.is_empty() || p.components.len() > MAX_COMPONENTS {
            self.err(
                "components",
                format!(
                    "needs 1..={MAX_COMPONENTS} components, got {}",
                    p.components.len()
                ),
            );
        }
        for (i, e) in p.camera.iter().enumerate() {
            if e.at.is_none() {
                self.err(
                    format!("camera[{i}].at"),
                    "needs `at` or a beat in the music",
                );
            }
            match &e.op {
                crate::program::CameraVerb::Frame { component } => {
                    if !p.components.iter().any(|c| &c.id == component) {
                        self.err(
                            format!("camera[{i}].component"),
                            format!("no component '{component}'"),
                        );
                    }
                }
                crate::program::CameraVerb::Orbit { yaw, pitch } => {
                    if !yaw.is_finite()
                        || !pitch.is_finite()
                        || !(-35.0..=35.0).contains(yaw)
                        || !(-35.0..=35.0).contains(pitch)
                    {
                        self.err(
                            format!("camera[{i}].orbit"),
                            "orbit yaw and pitch must be finite and within -35.0..=35.0",
                        );
                    }
                }
                crate::program::CameraVerb::Dolly { to } => {
                    if !to.is_finite() || !(-600.0..=600.0).contains(to) {
                        self.err(
                            format!("camera[{i}].dolly"),
                            "dolly to must be finite and within -600.0..=600.0",
                        );
                    }
                }
                crate::program::CameraVerb::Roll { deg } => {
                    if !deg.is_finite() || !(-25.0..=25.0).contains(deg) {
                        self.err(
                            format!("camera[{i}].roll"),
                            "roll deg must be finite and within -25.0..=25.0",
                        );
                    }
                }
                crate::program::CameraVerb::Focus { z, aperture } => {
                    if !z.is_finite() || !(-1200.0..=1200.0).contains(z) {
                        self.err(
                            format!("camera[{i}].focus.z"),
                            "focus z must be finite and within -1200.0..=1200.0",
                        );
                    }
                    if let Some(ap) = aperture {
                        if !ap.is_finite() || !(0.0..=1.0).contains(ap) {
                            self.err(
                                format!("camera[{i}].focus.aperture"),
                                "focus aperture must be finite and within 0.0..=1.0",
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        for (i, e) in p.transitions.iter().enumerate() {
            if e.at.is_none() {
                self.err(
                    format!("transitions[{i}].at"),
                    "needs `at` or a beat in the music",
                );
            }
        }
        let mut ids = HashSet::new();
        for (i, c) in p.components.iter().enumerate() {
            let cp = format!("components[{i}]");
            if !file_safe(&c.id) {
                self.err(
                    format!("{cp}.id"),
                    "use 1-64 lowercase letters, digits or _",
                );
            } else if !ids.insert(c.id.as_str()) {
                self.err(format!("{cp}.id"), format!("duplicate id '{}'", c.id));
            }
            self.component(c, &cp);
        }
    }

    fn palette(&mut self) {
        let Some(pal) = self.p.palette() else {
            self.err("palette", "needed unless a recipe gives one");
            return;
        };
        let pal = &pal;
        let mut parsed = Vec::new();
        for (name, hex) in [
            ("background", &pal.background),
            ("ink", &pal.ink),
            ("accent", &pal.accent),
            ("muted", &pal.muted),
        ] {
            match Color::parse_hex(hex).filter(|c| c.a == 255 && hex.len() == 7) {
                Some(c) => parsed.push(c),
                None => self.err(
                    format!("palette.{name}"),
                    format!("expected an opaque #RRGGBB colour, got \"{hex}\""),
                ),
            }
        }
        if let [bg, ink, ..] = parsed[..] {
            if parsed.len() == 4 {
                let c = contrast(bg, ink);
                if c < MIN_INK_CONTRAST {
                    self.err(
                        "palette.ink",
                        format!("ink on background contrast {c:.2} is below {MIN_INK_CONTRAST}"),
                    );
                }
            }
        }
    }

    fn component(&mut self, c: &Component, cp: &str) {
        let end = self.p.end_of(c);
        let ok_time = c.start.is_finite() && c.start >= 0.0 && end.is_finite() && end > c.start;
        if !ok_time {
            self.err(
                format!("{cp}.start"),
                format!("needs 0 <= start < end, got start {} end {end}", c.start),
            );
        } else if end > self.p.meta.duration + EPS {
            self.err(
                format!("{cp}.end"),
                format!(
                    "ends at {end}s, after the program ({}s)",
                    self.p.meta.duration
                ),
            );
        }
        let length = end - c.start;

        match &c.body {
            ComponentBody::Backdrop { preset, .. } => {
                if *preset != BackdropPreset::Flat && !self.p.allows(Feature::Gradient) {
                    self.err(
                        format!("{cp}.preset"),
                        "gradient backdrops are banned unless allow lists \"gradient\"",
                    );
                }
            }
            ComponentBody::Shape { shape, .. } => {
                if let ShapeSpec::Path(d) = shape {
                    match motion_core::path::parse(d) {
                        Ok(path) if !path.is_empty() => {
                            if let Some(b) = path.bounds() {
                                if b[0] < -10.0 || b[1] < -10.0 || b[2] > 110.0 || b[3] > 110.0 {
                                    self.err(
                                        format!("{cp}.shape.path"),
                                        "path must stay inside its 100×100 box",
                                    );
                                }
                            }
                        }
                        Ok(_) => self.err(format!("{cp}.shape.path"), "path has no segments"),
                        Err(e) => self.err(format!("{cp}.shape.path"), e.to_string()),
                    }
                }
            }
            ComponentBody::Title {
                text, size, style, ..
            } => {
                self.type_style(cp, style.as_ref());
                let n = text.chars().count();
                let max = if *size == Size::Xl { 24 } else { 60 };
                if text.trim().is_empty() || n > max || text.contains('\n') {
                    self.err(
                        format!("{cp}.text"),
                        format!("one line of 1..={max} characters, got {n}"),
                    );
                }
                if text.contains("//") {
                    self.err(
                        format!("{cp}.text"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
                if let Some(g) = crate::device::banned_glyph(text) {
                    self.err(
                        format!("{cp}.text"),
                        format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                    );
                }
            }
            ComponentBody::KineticText { words, style, .. } => {
                self.type_style(cp, style.as_ref());
                if words.iter().any(|w| w.contains("//")) {
                    self.err(
                        format!("{cp}.words"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
                if let Some(g) = words.iter().find_map(|w| crate::device::banned_glyph(w)) {
                    self.err(
                        format!("{cp}.words"),
                        format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                    );
                }
            }
            ComponentBody::Chart {
                title,
                labels,
                callout,
                ..
            } => {
                if title.as_deref().is_some_and(|t| t.contains("//"))
                    || labels.iter().any(|l| l.contains("//"))
                    || callout.as_deref().is_some_and(|c| c.contains("//"))
                {
                    self.err(
                        format!("{cp}.title"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
            }
            ComponentBody::Diagram { nodes, .. } => {
                if nodes.iter().any(|n| n.contains("//")) {
                    self.err(
                        format!("{cp}.nodes"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
            }
            ComponentBody::Hud { corners } => {
                if corners.iter().any(|t| t.contains("//")) {
                    self.err(
                        format!("{cp}.corners"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
            }
            ComponentBody::Field { .. } => {}
            ComponentBody::CodeBlock(b) => b
                .check()
                .into_iter()
                .for_each(|(p, m)| self.err(format!("{cp}{p}"), m)),
            ComponentBody::DeviceFrame(f) => f
                .check(length)
                .into_iter()
                .for_each(|(p, m)| self.err(format!("{cp}{p}"), m)),
            ComponentBody::Bento(b) => {
                if b.title.as_deref().is_some_and(|t| t.contains("//"))
                    || b.tiles.iter().any(|t| {
                        t.stat.contains("//")
                            || t.title.contains("//")
                            || t.detail.as_deref().is_some_and(|d| d.contains("//"))
                    })
                {
                    self.err(
                        format!("{cp}.title"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
                b.check()
                    .into_iter()
                    .for_each(|(p, m)| self.err(format!("{cp}{p}"), m));
            }
            ComponentBody::Lockup(l) => {
                if l.headline.contains("//")
                    || l.subline.as_deref().is_some_and(|s| s.contains("//"))
                    || l.badge.as_deref().is_some_and(|b| b.contains("//"))
                    || l.specs.iter().any(|s| s.contains("//"))
                {
                    self.err(
                        format!("{cp}.specs"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
                l.check()
                    .into_iter()
                    .for_each(|(p, m)| self.err(format!("{cp}{p}"), m));
            }
            ComponentBody::PhotoStack(ps) => ps
                .check(length)
                .into_iter()
                .for_each(|(p, m)| self.err(format!("{cp}{p}"), m)),
            ComponentBody::Sketch(sk) => {
                if sk.title.as_deref().is_some_and(|t| t.contains("//"))
                    || sk.caption.as_deref().is_some_and(|c| c.contains("//"))
                {
                    self.err(
                        format!("{cp}.title"),
                        "do not use '//' outside code_block (slop_double_slash)",
                    );
                }
                sk.check()
                    .into_iter()
                    .for_each(|(p, m)| self.err(format!("{cp}{p}"), m));
            }
        }

        if let Some(d) = c.depth {
            if !d.is_finite() || !(0.0..=3.0).contains(&d) {
                self.err(
                    format!("{cp}.depth"),
                    "depth must be finite and within 0.0..=3.0",
                );
            }
        }
        if let Some(z) = c.z {
            if !z.is_finite() || !(-1200.0..=1200.0).contains(&z) {
                self.err(
                    format!("{cp}.z"),
                    "z must be finite and within -1200.0..=1200.0",
                );
            }
        }
        if let Some([pitch, yaw]) = c.tilt {
            if !pitch.is_finite()
                || !yaw.is_finite()
                || !(-65.0..=65.0).contains(&pitch)
                || !(-65.0..=65.0).contains(&yaw)
            {
                self.err(
                    format!("{cp}.tilt"),
                    "tilt pitch and yaw must be finite and within -65.0..=65.0",
                );
            }
        }

        if let Some(t) = &c.over {
            self.over(c, t, cp, ok_time, end);
        }

        self.content(c, cp);
        if c.start_beat.is_some() {
            self.err(
                format!("{cp}.start_beat"),
                "beat not in the music analysis (or no music)",
            );
        }
        if c.timeline.len() > MAX_EVENTS {
            self.err(
                format!("{cp}.timeline"),
                format!("at most {MAX_EVENTS} events, got {}", c.timeline.len()),
            );
        }
        let mut spans: Vec<(Channel, f64, f64, usize)> = Vec::new();
        let (mut enters, mut exits, mut recolours) = (0, 0, 0);
        let enter_at = c
            .timeline
            .iter()
            .find(|e| matches!(e.verb, Verb::Enter { .. }))
            .and_then(|e| e.at);
        let exit_at = c
            .timeline
            .iter()
            .find(|e| matches!(e.verb, Verb::Exit { .. }))
            .and_then(|e| e.at);
        let mut order: Vec<usize> = (0..c.timeline.len()).collect();
        let when = |j: usize| c.timeline[j].at.unwrap_or(f64::INFINITY);
        order.sort_by(|&a, &b| when(a).total_cmp(&when(b)));
        if let ComponentBody::Shape { shape, .. } = &c.body {
            let mut current = shape;
            for &j in &order {
                if let Verb::Morph { to } = &c.timeline[j].verb {
                    if to == current {
                        self.err(
                            format!("{cp}.timeline[{j}].to"),
                            "morph to the shape it already is",
                        );
                    }
                    current = to;
                }
            }
        }
        // `beat_cuts: 0` is reported once on the sketch (see `Sketch::check`);
        // the advances the rhythm helper generated for it are not re-reported.
        let zero_cuts = matches!(&c.body, ComponentBody::Sketch(sk) if sk.beat_cuts == Some(0));
        for (j, e) in c.timeline.iter().enumerate() {
            if zero_cuts && matches!(e.verb, Verb::Advance) {
                continue;
            }
            let ep = format!("{cp}.timeline[{j}]");
            let dur = taste::duration(&e.verb, self.p.tempo());
            let Some(at) = e.at else {
                let msg = if e.beat.is_some() {
                    "beat not in the music analysis (or no music)"
                } else {
                    "needs `at` (seconds) or `beat`"
                };
                self.err(format!("{ep}.at"), msg);
                continue;
            };
            if !(at.is_finite() && at >= 0.0) {
                self.err(format!("{ep}.at"), format!("must be >= 0, got {at}"));
                continue;
            }
            if ok_time && at + dur > length + EPS {
                self.err(
                    format!("{ep}.at"),
                    format!(
                        "{} lasts {dur}s and would end at {:.3}s, after the component ({length:.3}s)",
                        e.verb.name(),
                        at + dur
                    ),
                );
            }
            self.verb_fits(&c.body, &e.verb, &ep);
            match e.verb {
                Verb::Enter { .. } => enters += 1,
                Verb::Exit { .. } => exits += 1,
                Verb::Recolour { .. } => recolours += 1,
                _ => {}
            }
            if !matches!(e.verb, Verb::Enter { .. }) && enter_at.is_some_and(|a| at < a) {
                self.err(format!("{ep}.at"), "comes before the component's enter");
            }
            if !matches!(e.verb, Verb::Exit { .. }) && exit_at.is_some_and(|a| at >= a) {
                self.err(format!("{ep}.at"), "comes after the component's exit");
            }
            for ch in channels(&e.verb) {
                spans.push((*ch, at, at + dur, j));
            }
        }
        for (what, n) in [("enter", enters), ("exit", exits), ("recolour", recolours)] {
            if n > 1 {
                self.err(
                    format!("{cp}.timeline"),
                    format!("at most one {what}, got {n}"),
                );
            }
        }
        // Overlaps per channel (reported once per pair, in event order).
        for a in 0..spans.len() {
            for b in a + 1..spans.len() {
                let (ca, sa, ea, ja) = spans[a];
                let (cb, sb, eb, jb) = spans[b];
                if ca == cb && ja != jb && sa < eb - EPS && sb < ea - EPS {
                    self.err(
                        format!("{cp}.timeline[{jb}]"),
                        format!(
                            "{} overlaps {} (event {ja}) on the {ca:?} channel",
                            c.timeline[jb].verb.name(),
                            c.timeline[ja].verb.name()
                        ),
                    );
                }
            }
        }
    }

    /// `over`: the target exists, is another component, shares screen time
    /// and the `over` chain never loops back.
    fn over(&mut self, c: &Component, t: &str, cp: &str, ok_time: bool, end: f64) {
        let path = format!("{cp}.over");
        if t == c.id {
            self.err(path, "a component cannot be over itself");
            return;
        }
        let Some(target) = self.p.components.iter().find(|o| o.id == t) else {
            self.err(path, format!("no component '{t}' to layer over"));
            return;
        };
        // Only non-type kinds may carry `over`; the target is unrestricted.
        if !matches!(
            c.body,
            ComponentBody::Sketch(_)
                | ComponentBody::Shape { .. }
                | ComponentBody::PhotoStack(_)
                | ComponentBody::Backdrop { .. }
                | ComponentBody::Field { .. }
        ) {
            self.err(
                path.clone(),
                "only a sketch, shape, photo_stack, backdrop or field can be declared over another component; type over anything is an overlap defect, not layering",
            );
        }
        let t_end = self.p.end_of(target);
        if ok_time && !(c.start < t_end - EPS && target.start < end - EPS) {
            self.err(
                path.clone(),
                format!(
                    "'{t}' is on screen {:.3}..{t_end:.3}s, which never meets this component ({:.3}..{end:.3}s)",
                    target.start, c.start
                ),
            );
        }
        let mut seen = vec![c.id.as_str()];
        let mut next = Some(t);
        while let Some(id) = next {
            if seen.contains(&id) {
                self.err(path, format!("`over` chain loops back to '{id}'"));
                return;
            }
            seen.push(id);
            next = self
                .p
                .components
                .iter()
                .find(|o| o.id == id)
                .and_then(|o| o.over.as_deref());
        }
    }

    /// Kind-specific content rules.
    fn content(&mut self, c: &Component, cp: &str) {
        match &c.body {
            ComponentBody::KineticText {
                preset,
                words,
                stress,
                ..
            } => {
                if words.is_empty() || words.len() > 40 {
                    self.err(format!("{cp}.words"), "needs 1..=40 words");
                }
                for (i, w) in words.iter().enumerate() {
                    let n = w.chars().count();
                    if w.trim().is_empty() || n > 24 || w.contains('\n') {
                        self.err(
                            format!("{cp}.words[{i}]"),
                            format!("one line of 1..=24 characters, got {n}"),
                        );
                    }
                }
                if let Some(s) = stress.iter().find(|s| **s >= words.len()) {
                    self.err(format!("{cp}.stress"), format!("no word {s}"));
                }
                let mut cuts: Vec<f64> = c
                    .timeline
                    .iter()
                    .filter(|e| matches!(e.verb, Verb::Cut { .. }))
                    .filter_map(|e| e.at)
                    .collect();
                if cuts.len() + 1 > words.len() {
                    self.err(
                        format!("{cp}.timeline"),
                        format!(
                            "{} cuts but only {} words (a cut shows the next word)",
                            cuts.len(),
                            words.len()
                        ),
                    );
                }
                cuts.sort_by(f64::total_cmp);
                let frame = 1.0 / self.p.meta.fps.value() as f64;
                if cuts.first().is_some_and(|t| *t < frame - EPS)
                    || cuts.windows(2).any(|w| w[1] - w[0] < frame - EPS)
                {
                    self.err(
                        format!("{cp}.timeline"),
                        "every word needs at least one frame between cuts",
                    );
                }
                if *preset != KineticPreset::PosterCuts
                    && c.timeline
                        .iter()
                        .any(|e| matches!(e.verb, Verb::Cut { flip: true }))
                {
                    self.err(
                        format!("{cp}.timeline"),
                        "flip needs the poster_cuts preset",
                    );
                }
            }
            ComponentBody::Chart {
                chart,
                values,
                labels,
                hero,
                callout,
                ..
            } => {
                let (lo, hi) = match chart {
                    ChartKind::Donut => (2, 8),
                    ChartKind::Spring => (2, 2),
                    _ => (2, 24),
                };
                if values.len() < lo || values.len() > hi {
                    self.err(
                        format!("{cp}.values"),
                        format!("needs {lo}..={hi} values, got {}", values.len()),
                    );
                }
                if values.iter().any(|v| !v.is_finite()) {
                    self.err(format!("{cp}.values"), "values must be finite");
                }
                if *chart == ChartKind::Donut
                    && (values.iter().any(|v| *v < 0.0) || values.iter().sum::<f64>() <= 0.0)
                {
                    self.err(
                        format!("{cp}.values"),
                        "donut values must be >= 0 with a positive sum",
                    );
                }
                if !labels.is_empty() && labels.len() != values.len() {
                    self.err(format!("{cp}.labels"), "one label per value, or none");
                }
                if labels.iter().any(|l| l.chars().count() > 10) {
                    self.err(format!("{cp}.labels"), "labels are at most 10 characters");
                }
                if hero.is_some_and(|h| h >= values.len()) {
                    self.err(format!("{cp}.hero"), "no such value");
                }
                if callout
                    .as_ref()
                    .is_some_and(|t| t.trim().is_empty() || t.chars().count() > 12)
                {
                    self.err(format!("{cp}.callout"), "1..=12 characters");
                }
                if hero.is_none() && c.timeline.iter().any(|e| matches!(e.verb, Verb::PopLabel)) {
                    self.err(format!("{cp}.hero"), "pop_label needs a hero value");
                }
            }
            ComponentBody::Shape { anchor, .. } => {
                let changes = c.timeline.iter().any(|e| {
                    matches!(
                        e.verb,
                        Verb::Morph { .. }
                            | Verb::Move { .. }
                            | Verb::Orbit { .. }
                            | Verb::Fill
                            | Verb::Recolour { .. }
                    )
                });
                match anchor {
                    Some(a) if !self.p.components.iter().any(|o| &o.id == a && o.id != c.id) => {
                        self.err(format!("{cp}.anchor"), format!("no component '{a}'"));
                    }
                    None if !changes => self.err(
                        cp.to_string(),
                        "slop_meaningless_shape: a shape that never changes must anchor to the content it marks",
                    ),
                    _ => {}
                }
            }
            ComponentBody::Diagram { nodes, edges, .. } => {
                if !(2..=8).contains(&nodes.len())
                    || nodes
                        .iter()
                        .any(|n| n.trim().is_empty() || n.chars().count() > 18)
                {
                    self.err(format!("{cp}.nodes"), "2..=8 nodes of 1..=18 characters");
                }
                if edges.iter().flatten().any(|i| *i >= nodes.len()) {
                    self.err(format!("{cp}.edges"), "edge names a missing node");
                }
                let adv = c
                    .timeline
                    .iter()
                    .filter(|e| matches!(e.verb, Verb::Advance))
                    .count();
                if adv + 1 > nodes.len() {
                    self.err(format!("{cp}.timeline"), "more advances than nodes");
                }
            }
            ComponentBody::Hud { corners } => {
                if corners.len() > 4 || corners.iter().any(|t| t.chars().count() > 24) {
                    self.err(
                        format!("{cp}.corners"),
                        "up to 4 corner labels of at most 24 characters",
                    );
                }
            }
            _ => {}
        }
    }

    fn verb_fits(&mut self, body: &ComponentBody, verb: &Verb, ep: &str) {
        use EnterStyle as E;
        use ExitStyle as X;
        let kind = body.kind_name();

        if let Verb::Tilt { from, to } = verb {
            let finite = from[0].is_finite()
                && from[1].is_finite()
                && to[0].is_finite()
                && to[1].is_finite();
            let in_range = (-65.0..=65.0).contains(&from[0])
                && (-65.0..=65.0).contains(&from[1])
                && (-65.0..=65.0).contains(&to[0])
                && (-65.0..=65.0).contains(&to[1]);
            if !finite || !in_range {
                self.err(
                    format!("{ep}.tilt"),
                    "tilt angles must be finite and within -65..=65 degrees",
                );
            }
            if !matches!(
                body,
                ComponentBody::Shape { .. }
                    | ComponentBody::Title { .. }
                    | ComponentBody::Chart { .. }
                    | ComponentBody::Diagram { .. }
                    | ComponentBody::CodeBlock(_)
                    | ComponentBody::DeviceFrame(_)
                    | ComponentBody::PhotoStack(_)
                    | ComponentBody::Bento(_)
                    | ComponentBody::Sketch(_)
                    | ComponentBody::Lockup(_)
            ) {
                self.err(
                    format!("{ep}.do"),
                    format!("tilt on a {kind}: tilt is not supported on this component"),
                );
            }
            return;
        }

        if let Verb::Echo { count } = verb {
            if !(2..=6).contains(count) {
                self.err(
                    format!("{ep}.echo"),
                    format!("echo count must be 2..=6, got {count}"),
                );
            }
            if !matches!(
                body,
                ComponentBody::Shape { .. }
                    | ComponentBody::Title { .. }
                    | ComponentBody::Lockup(_)
            ) {
                self.err(
                    format!("{ep}.do"),
                    format!("echo on a {kind}: echo is not supported on this component"),
                );
            }
            return;
        }

        let bad = match (body, verb) {
            (b, Verb::Sheen) if !matches!(b, ComponentBody::Lockup(_)) => {
                Some("sheen is for lockups")
            }
            (ComponentBody::Backdrop { .. }, v) => match v {
                Verb::Enter { style: E::Fade } | Verb::Exit { style: X::Fade | X::Cut } => None,
                Verb::Recolour { .. } => None,
                _ => Some("backdrops fade in/out and recolour only"),
            },
            (ComponentBody::Shape { preset, .. }, v) => match v {
                Verb::Enter { style: E::Draw } if *preset != ShapePreset::Outline => {
                    Some("draw needs an outline shape")
                }
                Verb::Enter { style: E::Wipe } | Verb::Exit { style: X::Wipe } => {
                    Some("wipe is for titles")
                }
                Verb::Cut { .. } | Verb::PopLabel => Some("not a shape verb"),
                _ => None,
            },
            (ComponentBody::Title { .. }, v) => match v {
                Verb::Enter { style: E::Draw } => Some("draw is for outline shapes"),
                Verb::Morph { .. }
                | Verb::Bounce
                | Verb::Orbit { .. }
                | Verb::Fill
                | Verb::Unfill
                | Verb::Spin => Some("morph, bounce, orbit, fill and spin are for shapes"),
                Verb::Cut { .. } | Verb::PopLabel => Some("not a title verb"),
                _ => None,
            },
            (ComponentBody::KineticText { .. }, v) => match v {
                Verb::Cut { .. }
                | Verb::Exit {
                    style: X::Fade | X::Cut | X::Shrink,
                } => None,
                _ => Some("kinetic text takes cut and exit (fade/shrink)"),
            },
            (ComponentBody::Chart { .. }, v) => match v {
                Verb::Advance => None,
                Verb::Enter {
                    style: E::Draw | E::Fade,
                }
                | Verb::Exit { style: X::Fade | X::Cut }
                | Verb::PopLabel => None,
                _ => Some("charts take enter (draw/fade), pop_label and exit (fade)"),
            },
            (ComponentBody::Diagram { .. }, v) => match v {
                Verb::Advance | Verb::Enter { style: E::Fade } | Verb::Exit { style: X::Fade | X::Cut } => {
                    None
                }
                _ => Some("diagrams take advance, enter (fade) and exit (fade)"),
            },
            (ComponentBody::Hud { .. }, v) => match v {
                Verb::Enter { style: E::Fade } | Verb::Exit { style: X::Fade | X::Cut } => None,
                _ => Some("hud labels fade in and out only"),
            },
            (
                ComponentBody::CodeBlock(_)
                | ComponentBody::DeviceFrame(_)
                | ComponentBody::PhotoStack(_),
                v,
            ) => match v {
                Verb::Enter {
                    style: E::Fade | E::Rise | E::Pop,
                }
                | Verb::Exit {
                    style: X::Fade | X::Cut,
                } => None,
                _ => Some(
                    "code blocks, devices and photo stacks take enter (fade/rise/pop) and exit (fade); the rest is in steps",
                ),
            },
            (ComponentBody::Bento(_) | ComponentBody::Sketch(_), v) => match v {
                Verb::Advance
                | Verb::Enter {
                    style: E::Fade | E::Pop | E::Rise | E::Draw,
                }
                | Verb::Exit {
                    style: X::Fade | X::Cut,
                } => None,
                _ => Some("bento and sketch take advance, enter (fade/pop/rise/draw) and exit (fade/cut)"),
            },
            (ComponentBody::Lockup(_), v) => match v {
                Verb::Enter {
                    style: E::Pop | E::Fade | E::Rise | E::Draw,
                }
                | Verb::Sheen
                | Verb::Advance
                | Verb::Recolour { .. } => None,
                _ => Some(
                    "lockups take enter (pop/fade/rise/draw), sheen, advance and recolour",
                ),
            },
            (ComponentBody::Field { .. }, v) => match v {
                Verb::Enter {
                    style: E::Draw | E::Fade,
                }
                | Verb::Exit { style: X::Fade | X::Cut }
                | Verb::Recolour { .. }
                | Verb::Wave { .. } => None,
                _ => Some("fields take enter (draw/fade), recolour, wave and exit (fade)"),
            },
        };
        if let Some(msg) = bad {
            self.err(
                format!("{ep}.do"),
                format!("{} on a {kind}: {msg}", verb.name()),
            );
        }
        if let Verb::Morph {
            to: ShapeSpec::Path(d),
        } = verb
        {
            if motion_core::path::parse(d).map_or(true, |p| p.is_empty()) {
                self.err(format!("{ep}.to"), "invalid path data");
            }
        }
    }
}
