//! MotionScene validation. Reports every problem found, each with a path
//! identifying scene / layer / motion / field.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use crate::scene::{
    self, AssetKind, CameraOp, Channel, FontRole, Layer, LayerKind, LayoutBinding, Motion,
    MotionOp, MotionProject, PostKind, Scene, SharedElement, Stroke,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// Location, e.g. `scenes[beat_1].layers[headline].opacity`.
    pub path: String,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors(pub Vec<ValidationError>);

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} validation error(s):", self.0.len())?;
        for e in &self.0 {
            writeln!(f, "  - {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

/// Validate a project. When `base_dir` (the directory containing the motion
/// file) is given, asset files are also checked for existence on disk.
pub fn validate(project: &MotionProject, base_dir: Option<&Path>) -> Result<(), ValidationErrors> {
    let mut v = Validator {
        project,
        errors: Vec::new(),
        layer_ids: HashSet::new(),
    };
    v.run(base_dir);
    if v.errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(v.errors))
    }
}

/// Tolerance for float timing comparisons, in seconds.
const EPS: f64 = 1e-6;
/// Tighter tolerance for interval overlap so back-to-back motions never conflict.
const OVERLAP_EPS: f64 = 1e-9;
const MAX_CANVAS: u32 = 16384;
const MAX_FPS: u32 = 240;

/// `(start, end, motion path)` of a well-formed motion, for conflict checks.
type Span = (f64, f64, String);

struct Validator<'a> {
    project: &'a MotionProject,
    errors: Vec<ValidationError>,
    /// Layer ids seen so far, across all scenes, nested children and shared layers.
    layer_ids: HashSet<String>,
}

fn role_name(role: FontRole) -> &'static str {
    match role {
        FontRole::Display => "display",
        FontRole::DisplayCondensed => "display_condensed",
        FontRole::SerifEmotional => "serif_emotional",
        FontRole::Body => "body",
        FontRole::Mono => "mono",
        FontRole::Number => "number",
    }
}

fn kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "image",
        AssetKind::Svg => "svg",
        AssetKind::Font => "font",
        AssetKind::SpriteSequence => "sprite_sequence",
    }
}

fn collect_layer_kinds<'a>(layers: &'a [Layer], out: &mut HashMap<&'a str, &'a LayerKind>) {
    for l in layers {
        out.insert(l.id.as_str(), &l.kind);
        if let LayerKind::Group { children } = &l.kind {
            collect_layer_kinds(children, out);
        }
    }
}

fn collect_layer_ids<'a>(layers: &'a [Layer], out: &mut HashSet<&'a str>) {
    for l in layers {
        out.insert(l.id.as_str());
        if let LayerKind::Group { children } = &l.kind {
            collect_layer_ids(children, out);
        }
    }
}

impl<'a> Validator<'a> {
    fn err(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(ValidationError {
            path: path.into(),
            message: message.into(),
        });
    }

    /// (0.14–0.16) Envelopes, pulse references, post effects, 3D layer fields.
    fn motion_2(&mut self, p: &MotionProject) {
        let mut env_ids = HashSet::new();
        for (i, e) in p.envelopes.iter().enumerate() {
            let ep = format!("envelopes[{i}]");
            if !env_ids.insert(e.id.as_str()) {
                self.err(
                    format!("{ep}.id"),
                    format!("duplicate envelope id '{}'", e.id),
                );
            }
            if !(e.fps.is_finite() && e.fps > 0.0) {
                self.err(
                    format!("{ep}.fps"),
                    format!("envelope fps must be > 0, got {}", e.fps),
                );
            }
            if e.values
                .iter()
                .any(|v| !(v.is_finite() && (0.0..=1.0).contains(v)))
            {
                self.err(
                    format!("{ep}.values"),
                    "envelope values must be in [0, 1]".to_string(),
                );
            }
        }
        fn layers3d(v: &mut Validator, layers: &[Layer], path: &str) {
            for (i, l) in layers.iter().enumerate() {
                let lp = format!("{path}[{i}]");
                if l.z.is_some_and(|z| !z.is_finite())
                    || l.tilt
                        .is_some_and(|t| t.iter().any(|a| !a.is_finite() || a.abs() > 89.0))
                {
                    v.err(
                        lp.clone(),
                        "z must be finite and tilt angles within ±89°".to_string(),
                    );
                }
                if let LayerKind::Group { children } = &l.kind {
                    layers3d(v, children, &format!("{lp}.children"));
                }
            }
        }
        for (si, sc) in p.scenes.iter().enumerate() {
            let sp = format!("scenes[{si}]");
            layers3d(self, &sc.layers, &format!("{sp}.layers"));
            for (mi, m) in sc.motions.iter().enumerate() {
                if let MotionOp::Pulse { envelope, .. } = &m.op {
                    if !envelope.is_empty() && !env_ids.contains(envelope.as_str()) {
                        self.err(
                            format!("{sp}.motions[{mi}].envelope"),
                            format!("unknown envelope '{envelope}'"),
                        );
                    }
                }
                if let Some(sp2) = m.spring {
                    if !(sp2.stiffness.is_finite()
                        && sp2.stiffness > 0.0
                        && sp2.damping.is_finite()
                        && sp2.damping >= 0.0
                        && sp2.mass.is_finite()
                        && sp2.mass > 0.0)
                    {
                        self.err(
                            format!("{sp}.motions[{mi}].spring"),
                            "spring needs stiffness > 0, damping >= 0, mass > 0".to_string(),
                        );
                    }
                }
            }
            for (pi, e) in sc.post.iter().enumerate() {
                let pp = format!("{sp}.post[{pi}]");
                let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
                if !(e.start.is_finite()
                    && e.start >= 0.0
                    && e.duration.is_finite()
                    && e.duration >= 0.0)
                {
                    self.err(
                        pp.clone(),
                        "post start/duration must be finite and >= 0".to_string(),
                    );
                }
                if !unit(e.from) || !unit(e.to) {
                    self.err(pp.clone(), "post from/to must be in [0, 1]".to_string());
                }
                let ok = match &e.kind {
                    PostKind::Bloom { threshold, radius } => {
                        unit(*threshold) && radius.is_finite() && *radius > 0.0 && *radius <= 200.0
                    }
                    PostKind::ChromaticAberration { shift, angle_deg } => {
                        shift.is_finite() && shift.abs() <= 64.0 && angle_deg.is_finite()
                    }
                    PostKind::Glitch {
                        bands, max_shift, ..
                    } => {
                        (1..=64).contains(bands)
                            && max_shift.is_finite()
                            && *max_shift >= 0.0
                            && *max_shift <= 400.0
                    }
                    PostKind::Rays { center, length } => {
                        center.iter().all(|c| c.is_finite()) && unit(*length)
                    }
                    PostKind::DirectionalBlur { angle_deg, length } => {
                        angle_deg.is_finite()
                            && length.is_finite()
                            && *length >= 0.0
                            && *length <= 200.0
                    }
                    PostKind::Grain { amount, .. } => unit(*amount),
                    PostKind::Vignette { amount } => unit(*amount),
                };
                if !ok {
                    self.err(
                        pp,
                        format!("post effect parameters out of range: {:?}", e.kind),
                    );
                }
            }
        }
    }

    fn run(&mut self, base_dir: Option<&Path>) {
        let p = self.project;
        self.motion_2(p);

        if p.version != scene::SCENE_VERSION {
            self.err(
                "version",
                format!(
                    "unsupported version '{}', expected '{}'",
                    p.version,
                    scene::SCENE_VERSION
                ),
            );
        }

        self.canvas();

        if let Some(d) = p.project.duration_seconds {
            if !d.is_finite() || d <= 0.0 {
                self.err(
                    "project.duration_seconds",
                    format!("invalid duration {d}: must be finite and > 0"),
                );
            }
        }

        self.assets(base_dir);
        self.theme();

        let mut scene_ids: HashSet<&str> = HashSet::new();
        for scene in &p.scenes {
            if !scene_ids.insert(scene.id.as_str()) {
                self.err(
                    format!("scenes[{}].id", scene.id),
                    format!("duplicate scene id '{}'", scene.id),
                );
            }
            self.scene(scene);
        }

        self.shared();
    }

    // -- canvas / assets / theme ---------------------------------------------

    fn canvas(&mut self) {
        let c = &self.project.canvas;
        if c.width == 0 || c.width > MAX_CANVAS {
            self.err(
                "canvas.width",
                format!(
                    "invalid canvas size: width {} must be in 1..={MAX_CANVAS}",
                    c.width
                ),
            );
        }
        if c.height == 0 || c.height > MAX_CANVAS {
            self.err(
                "canvas.height",
                format!(
                    "invalid canvas size: height {} must be in 1..={MAX_CANVAS}",
                    c.height
                ),
            );
        }
        if c.fps == 0 || c.fps > MAX_FPS {
            self.err(
                "canvas.fps",
                format!("invalid fps {}: must be in 1..={MAX_FPS}", c.fps),
            );
        }
    }

    fn assets(&mut self, base_dir: Option<&Path>) {
        let p = self.project;
        let mut seen: HashSet<&str> = HashSet::new();
        for a in &p.assets {
            let path = format!("assets[{}]", a.id);
            if !seen.insert(a.id.as_str()) {
                self.err(
                    format!("{path}.id"),
                    format!("duplicate asset id '{}'", a.id),
                );
            }
            if a.path.is_empty() {
                self.err(format!("{path}.path"), "asset path is empty");
                continue;
            }
            match (a.kind, &a.sprite) {
                (AssetKind::SpriteSequence, None) => self.err(
                    format!("{path}.sprite"),
                    "sprite_sequence asset needs a 'sprite' spec (frame_count, fps, mode)",
                ),
                (AssetKind::SpriteSequence, Some(sp)) => {
                    if sp.frame_count < 1 {
                        self.err(
                            format!("{path}.sprite.frame_count"),
                            "frame_count must be >= 1",
                        );
                    }
                    if !(sp.fps.is_finite() && sp.fps > 0.0) {
                        self.err(
                            format!("{path}.sprite.fps"),
                            format!("fps must be finite and > 0, got {}", sp.fps),
                        );
                    }
                    if scene::sprite_file_name(&sp.pattern, 1).is_none() {
                        self.err(
                            format!("{path}.sprite.pattern"),
                            format!(
                                "pattern '{}' must contain one integer conversion like %04d",
                                sp.pattern
                            ),
                        );
                    }
                }
                (_, Some(_)) => self.err(
                    format!("{path}.sprite"),
                    format!(
                        "only sprite_sequence assets carry 'sprite', but '{}' is {}",
                        a.id,
                        kind_name(a.kind)
                    ),
                ),
                (_, None) => {}
            }
            if let Some(base) = base_dir {
                let full = base
                    .join(p.asset_root.as_deref().unwrap_or(""))
                    .join(&a.path);
                if a.kind == AssetKind::SpriteSequence {
                    if !full.is_dir() {
                        self.err(
                            format!("{path}.path"),
                            format!("missing sprite directory '{}'", full.display()),
                        );
                    } else if let Some(sp) = &a.sprite {
                        // First and last frame must exist (cheap sanity check).
                        for n in [1, sp.frame_count] {
                            if let Some(name) = scene::sprite_file_name(&sp.pattern, n) {
                                let f = full.join(name);
                                if sp.frame_count >= 1 && !f.is_file() {
                                    self.err(
                                        format!("{path}.path"),
                                        format!("missing sprite frame '{}'", f.display()),
                                    );
                                }
                            }
                        }
                    }
                } else if !full.is_file() {
                    self.err(
                        format!("{path}.path"),
                        format!("missing asset file '{}'", full.display()),
                    );
                }
            }
        }
    }

    fn theme(&mut self) {
        let p = self.project;
        for (role, asset_id) in &p.theme.fonts {
            let path = format!("theme.fonts.{}", role_name(*role));
            match p.asset(asset_id) {
                None => self.err(path, format!("missing asset '{asset_id}'")),
                Some(a) if a.kind != AssetKind::Font => self.err(
                    path,
                    format!(
                        "unsupported type: font role '{}' expects font asset but '{asset_id}' is {}",
                        role_name(*role),
                        kind_name(a.kind)
                    ),
                ),
                Some(_) => {}
            }
        }
    }

    // -- scenes ----------------------------------------------------------------

    fn scene(&mut self, scene: &Scene) {
        let sp = format!("scenes[{}]", scene.id);

        if !scene.start_seconds.is_finite() || scene.start_seconds < 0.0 {
            self.err(
                format!("{sp}.start_seconds"),
                format!(
                    "invalid timing: start_seconds {} must be finite and >= 0",
                    scene.start_seconds
                ),
            );
        }
        let d = scene.duration_seconds;
        if !d.is_finite() {
            self.err(
                format!("{sp}.duration_seconds"),
                format!("invalid timing: duration {d} is not finite"),
            );
        } else if d < 0.0 {
            self.err(
                format!("{sp}.duration_seconds"),
                format!("negative duration {d}"),
            );
        } else if d == 0.0 {
            self.err(
                format!("{sp}.duration_seconds"),
                "invalid timing: zero duration",
            );
        }

        if let Some(life) = &scene.lifecycle {
            let b = life.starts();
            let ok = b.iter().all(|v| v.is_finite())
                && b.windows(2).all(|w| w[0] <= w[1] + EPS)
                && life.bridge <= d + EPS;
            if !ok {
                self.err(
                    format!("{sp}.lifecycle"),
                    format!(
                        "lifecycle phases must satisfy 0 <= enter <= settle <= read <= evolve <= anticipate <= bridge <= duration ({d}), got {b:?}"
                    ),
                );
            }
        }

        let mut earlier: Vec<&str> = Vec::new();
        for l in &scene.layers {
            self.layer(l, &format!("{sp}.layers[{}]", l.id), &earlier);
            earlier.push(l.id.as_str());
        }

        self.motions(scene, &sp);
        self.camera(scene, &sp);
    }

    // -- layers ----------------------------------------------------------------

    fn finite(&mut self, path: &str, field: &str, v: f32) -> bool {
        if v.is_finite() {
            true
        } else {
            self.err(
                format!("{path}.{field}"),
                format!("{field} must be finite, got {v}"),
            );
            false
        }
    }

    /// `earlier` = ids of the siblings preceding `l` in its children list.
    fn layer(&mut self, l: &Layer, path: &str, earlier: &[&str]) {
        if !self.layer_ids.insert(l.id.clone()) {
            self.err(
                format!("{path}.id"),
                format!("duplicate layer id '{}'", l.id),
            );
        }

        for (field, v) in [
            ("x", l.x),
            ("y", l.y),
            ("scale_x", l.scale_x),
            ("scale_y", l.scale_y),
            ("rotation_degrees", l.rotation_degrees),
            ("anchor_x", l.anchor_x),
            ("anchor_y", l.anchor_y),
        ] {
            self.finite(path, field, v);
        }
        for (field, v) in [("width", l.width), ("height", l.height)] {
            if self.finite(path, field, v) && v < 0.0 {
                self.err(
                    format!("{path}.{field}"),
                    format!("{field} must be >= 0, got {v}"),
                );
            }
        }
        if let Some(d) = l.depth {
            if !(d.is_finite() && d >= 0.0) {
                self.err(
                    format!("{path}.depth"),
                    format!("depth must be finite and >= 0, got {d}"),
                );
            }
        }
        if let Some(b) = &l.layout {
            self.layout_binding(b, &format!("{path}.layout"));
            if b.parent == l.id {
                self.err(
                    format!("{path}.layout.parent"),
                    format!("layout parent '{}' is the layer itself", b.parent),
                );
            } else if !earlier.contains(&b.parent.as_str()) {
                self.err(
                    format!("{path}.layout.parent"),
                    format!(
                        "layout parent '{}' must be an earlier sibling in the same children list",
                        b.parent
                    ),
                );
            }
        }
        if !(l.opacity.is_finite() && (0.0..=1.0).contains(&l.opacity)) {
            self.err(
                format!("{path}.opacity"),
                format!(
                    "invalid opacity {}: must be finite and in [0, 1]",
                    l.opacity
                ),
            );
        }

        if let Some(c) = &l.clip {
            let mut ok = true;
            for (field, v) in [
                ("left", c.left),
                ("top", c.top),
                ("right", c.right),
                ("bottom", c.bottom),
            ] {
                if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                    ok = false;
                    self.err(
                        format!("{path}.clip.{field}"),
                        format!("invalid clip inset {v}: must be finite and in [0, 1]"),
                    );
                }
            }
            if ok {
                if c.left + c.right > 1.0 + 1e-6 {
                    self.err(
                        format!("{path}.clip"),
                        format!(
                            "invalid clip: left + right = {} exceeds 1",
                            c.left + c.right
                        ),
                    );
                }
                if c.top + c.bottom > 1.0 + 1e-6 {
                    self.err(
                        format!("{path}.clip"),
                        format!(
                            "invalid clip: top + bottom = {} exceeds 1",
                            c.top + c.bottom
                        ),
                    );
                }
            }
        }

        self.layer_kind(l, path);
    }

    /// Value checks shared by layer and track-key layout bindings.
    fn layout_binding(&mut self, b: &LayoutBinding, path: &str) {
        for (i, v) in b.offset.iter().enumerate() {
            if !v.is_finite() {
                self.err(
                    format!("{path}.offset"),
                    format!("offset[{i}] must be finite, got {v}"),
                );
            }
        }
        let pad = b.padding;
        for (field, v) in [
            ("left", pad.left),
            ("top", pad.top),
            ("right", pad.right),
            ("bottom", pad.bottom),
        ] {
            if !(v.is_finite() && v >= 0.0) {
                self.err(
                    format!("{path}.padding.{field}"),
                    format!("padding {field} must be finite and >= 0, got {v}"),
                );
            }
        }
    }

    fn stroke(&mut self, stroke: &Option<Stroke>, path: &str) {
        if let Some(s) = stroke {
            if !(s.width.is_finite() && s.width >= 0.0) {
                self.err(
                    format!("{path}.stroke.width"),
                    format!("stroke width must be finite and >= 0, got {}", s.width),
                );
            }
        }
    }

    fn asset_ref(&mut self, path: &str, layer_type: &str, asset_id: &str, expected: AssetKind) {
        match self.project.asset(asset_id) {
            None => self.err(
                format!("{path}.asset"),
                format!("missing asset '{asset_id}'"),
            ),
            Some(a) if a.kind != expected => self.err(
                format!("{path}.asset"),
                format!(
                    "unsupported type: layer {layer_type} expects {} asset but '{asset_id}' is {}",
                    kind_name(expected),
                    kind_name(a.kind)
                ),
            ),
            Some(_) => {}
        }
    }

    fn is_sprite(&self, asset_id: &str) -> bool {
        self.project
            .asset(asset_id)
            .is_some_and(|a| a.kind == AssetKind::SpriteSequence)
    }

    /// Image layers and screen inserts accept still images or sprite sequences.
    fn image_asset_ref(&mut self, path: &str, what: &str, asset_id: &str) {
        match self.project.asset(asset_id) {
            None => self.err(
                format!("{path}.asset"),
                format!("missing asset '{asset_id}'"),
            ),
            Some(a) if !matches!(a.kind, AssetKind::Image | AssetKind::SpriteSequence) => self
                .err(
                    format!("{path}.asset"),
                    format!(
                        "unsupported type: {what} expects image asset but '{asset_id}' is {} (a sprite_sequence also works)",
                        kind_name(a.kind)
                    ),
                ),
            Some(_) => {}
        }
    }

    fn playback(&mut self, path: &str, pb: &scene::SpritePlayback, asset_id: &str) {
        if !pb.start.is_finite() {
            self.err(format!("{path}.start"), "start must be finite");
        }
        let count = self
            .project
            .asset(asset_id)
            .and_then(|a| a.sprite.as_ref())
            .map_or(u32::MAX, |s| s.frame_count);
        if pb.in_frame >= count {
            self.err(
                format!("{path}.in_frame"),
                format!("in_frame {} is outside 0..{count}", pb.in_frame),
            );
        }
        if let Some(out) = pb.out_frame {
            if out >= count || out < pb.in_frame {
                self.err(
                    format!("{path}.out_frame"),
                    format!(
                        "out_frame {out} must be >= in_frame {} and < {count}",
                        pb.in_frame
                    ),
                );
            }
        }
    }

    fn layer_kind(&mut self, l: &Layer, path: &str) {
        match &l.kind {
            LayerKind::Rectangle { stroke, .. } => self.stroke(stroke, path),
            LayerKind::RoundedRectangle { radius, stroke, .. } => {
                if !(radius.is_finite() && *radius >= 0.0) {
                    self.err(
                        format!("{path}.radius"),
                        format!("radius must be finite and >= 0, got {radius}"),
                    );
                }
                self.stroke(stroke, path);
            }
            LayerKind::Polyline { points, stroke, .. } => {
                if points.len() < 2 {
                    self.err(
                        format!("{path}.points"),
                        format!("polyline needs at least 2 points, got {}", points.len()),
                    );
                }
                for (i, pt) in points.iter().enumerate() {
                    if !(pt[0].is_finite() && pt[1].is_finite()) {
                        self.err(
                            format!("{path}.points[{i}]"),
                            format!("point coordinates must be finite, got {pt:?}"),
                        );
                    }
                }
                if !(stroke.width.is_finite() && stroke.width > 0.0) {
                    self.err(
                        format!("{path}.stroke.width"),
                        format!(
                            "polyline stroke width must be finite and > 0, got {}",
                            stroke.width
                        ),
                    );
                }
            }
            LayerKind::Text(t) => {
                if let Some(ink) = t.ink {
                    if !(ink.top.is_finite() && ink.bottom.is_finite() && ink.top <= ink.bottom) {
                        self.err(
                            format!("{path}.ink"),
                            format!(
                                "ink bounds must be finite with top <= bottom, got top {} bottom {}",
                                ink.top, ink.bottom
                            ),
                        );
                    }
                }
                if !(t.font_size.is_finite() && t.font_size > 0.0) {
                    self.err(
                        format!("{path}.font_size"),
                        format!("font_size must be finite and > 0, got {}", t.font_size),
                    );
                }
                if !(t.line_height.is_finite() && t.line_height > 0.0) {
                    self.err(
                        format!("{path}.line_height"),
                        format!("line_height must be finite and > 0, got {}", t.line_height),
                    );
                }
                if let Some(w) = t.max_width {
                    if !(w.is_finite() && w > 0.0) {
                        self.err(
                            format!("{path}.max_width"),
                            format!("max_width must be finite and > 0, got {w}"),
                        );
                    }
                }
                if !self.project.theme.fonts.contains_key(&t.font_role) {
                    self.err(
                        format!("{path}.font_role"),
                        format!(
                            "no font assigned to role '{}' in theme.fonts",
                            role_name(t.font_role)
                        ),
                    );
                }
            }
            LayerKind::Image {
                asset,
                playback,
                insert,
                ..
            } => {
                self.image_asset_ref(path, "layer image", asset);
                let is_sprite = self.is_sprite(asset);
                if let Some(pb) = playback {
                    if !is_sprite {
                        self.err(
                            format!("{path}.playback"),
                            format!("playback needs a sprite_sequence asset but '{asset}' is not"),
                        );
                    } else {
                        self.playback(&format!("{path}.playback"), pb, asset);
                    }
                }
                if let Some(ins) = insert {
                    let ip = format!("{path}.insert");
                    self.image_asset_ref(&ip, "insert", &ins.asset);
                    let b = ins.screen_box;
                    if !b.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
                        self.err(
                            format!("{ip}.screen_box"),
                            format!("screen_box values must be finite and in [0, 1], got {b:?}"),
                        );
                    } else if !(b[2] > 0.0 && b[3] > 0.0) {
                        self.err(
                            format!("{ip}.screen_box"),
                            format!("screen_box width and height must be > 0, got {b:?}"),
                        );
                    }
                    if let Some(pb) = &ins.playback {
                        if !self.is_sprite(&ins.asset) {
                            self.err(
                                format!("{ip}.playback"),
                                format!(
                                    "playback needs a sprite_sequence asset but '{}' is not",
                                    ins.asset
                                ),
                            );
                        } else {
                            self.playback(&format!("{ip}.playback"), pb, &ins.asset);
                        }
                    }
                }
            }
            LayerKind::Svg { asset, .. } => self.asset_ref(path, "svg", asset, AssetKind::Svg),
            LayerKind::Group { children } => {
                let mut earlier: Vec<&str> = Vec::new();
                for c in children {
                    self.layer(c, &format!("{path}.children[{}]", c.id), &earlier);
                    earlier.push(c.id.as_str());
                }
            }
            LayerKind::Texture(t) => {
                if !(t.intensity.is_finite() && (0.0..=1.0).contains(&t.intensity)) {
                    self.err(
                        format!("{path}.intensity"),
                        format!(
                            "intensity must be finite and in [0, 1], got {}",
                            t.intensity
                        ),
                    );
                }
                if !(t.scale.is_finite() && t.scale > 0.0) {
                    self.err(
                        format!("{path}.scale"),
                        format!("texture scale must be finite and > 0, got {}", t.scale),
                    );
                }
            }
        }
    }

    // -- motions ---------------------------------------------------------------

    fn motions(&mut self, scene: &Scene, sp: &str) {
        let mut ids: HashSet<&str> = HashSet::new();
        collect_layer_ids(&scene.layers, &mut ids);
        let mut kinds: HashMap<&str, &LayerKind> = HashMap::new();
        collect_layer_kinds(&scene.layers, &mut kinds);

        // (target, channel) -> [(start, end, motion path)] of well-formed motions.
        let mut by_channel: HashMap<(&str, Channel), Vec<Span>> = HashMap::new();
        let mut order: Vec<(&str, Channel)> = Vec::new();

        for (i, m) in scene.motions.iter().enumerate() {
            let label = m.id.clone().unwrap_or_else(|| i.to_string());
            let mp = format!("{sp}.motions[{label}]");

            let shared = if ids.contains(m.target.as_str()) {
                None
            } else {
                self.project.shared.iter().find(|e| e.layer.id == m.target)
            };
            if let Some(el) = shared {
                if !el.track.iter().any(|k| k.scene == scene.id) {
                    self.err(
                        format!("{mp}.target"),
                        format!(
                            "shared element '{}' has no track key in scene '{}': motions may only target it where it lives",
                            el.id, scene.id
                        ),
                    );
                }
                if !matches!(
                    m.op,
                    MotionOp::Move { .. }
                        | MotionOp::Scale { .. }
                        | MotionOp::Rotate { .. }
                        | MotionOp::Fade { .. }
                        | MotionOp::Count { .. }
                        | MotionOp::Tint { .. }
                ) {
                    self.err(
                        format!("{mp}.op"),
                        format!(
                            "shared element '{}' accepts move, scale, rotate, fade, count and tint motions only",
                            el.id
                        ),
                    );
                }
            } else if !ids.contains(m.target.as_str()) {
                self.err(
                    format!("{mp}.target"),
                    format!(
                        "motion target missing: no layer '{}' in scene '{}'",
                        m.target, scene.id
                    ),
                );
            }

            let mut timing_ok = true;
            if !m.start.is_finite() || m.start < 0.0 {
                timing_ok = false;
                self.err(
                    format!("{mp}.start"),
                    format!("invalid timing: start {} must be finite and >= 0", m.start),
                );
            }
            if !m.duration.is_finite() {
                timing_ok = false;
                self.err(
                    format!("{mp}.duration"),
                    format!("invalid timing: duration {} is not finite", m.duration),
                );
            } else if m.duration < 0.0 {
                timing_ok = false;
                self.err(
                    format!("{mp}.duration"),
                    format!("negative duration {}", m.duration),
                );
            }
            if timing_ok {
                let end = m.start + m.duration;
                if end > scene.duration_seconds + EPS {
                    self.err(
                        format!("{mp}.duration"),
                        format!(
                            "invalid timing: motion ends after scene (ends at {end}s, scene '{}' lasts {}s)",
                            scene.id, scene.duration_seconds
                        ),
                    );
                }
                let key = (m.target.as_str(), m.op.channel());
                by_channel
                    .entry(key)
                    .or_default()
                    .push((m.start, end, mp.clone()));
                if !order.contains(&key) {
                    order.push(key);
                }
            }

            let kind = kinds
                .get(m.target.as_str())
                .copied()
                .or(shared.map(|e| &e.layer.kind));
            self.motion_op(m, &mp, kind);
        }

        // Conflicts, reported in first-appearance order for determinism.
        for key in order {
            let Some(list) = by_channel.get(&key) else {
                continue;
            };
            for (j, b) in list.iter().enumerate() {
                for a in &list[..j] {
                    if intervals_overlap((a.0, a.1), (b.0, b.1)) {
                        self.err(
                            b.2.clone(),
                            format!(
                                "conflicting animations on channel {:?}: overlaps {} on layer '{}'",
                                key.1, a.2, key.0
                            ),
                        );
                    }
                }
            }
        }
    }

    fn motion_op(&mut self, m: &Motion, mp: &str, target: Option<&LayerKind>) {
        let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        match &m.op {
            MotionOp::Move { from, to } => {
                for v in from.iter().chain(to.iter()) {
                    if !v.is_finite() {
                        self.err(
                            format!("{mp}.op"),
                            format!("move offsets must be finite, got {v}"),
                        );
                    }
                }
            }
            MotionOp::Scale { from, to, .. } => {
                for (field, v) in [("from", *from), ("to", *to)] {
                    self.finite(mp, field, v);
                }
            }
            MotionOp::Rotate { from, to } => {
                for (field, v) in [("from", *from), ("to", *to)] {
                    self.finite(mp, field, v);
                }
            }
            MotionOp::Fade { from, to } => {
                for (field, v) in [("from", *from), ("to", *to)] {
                    if !unit(v) {
                        self.err(
                            format!("{mp}.{field}"),
                            format!(
                                "invalid opacity {v}: fade values must be finite and in [0, 1]"
                            ),
                        );
                    }
                }
            }
            MotionOp::MaskReveal { .. } | MotionOp::ClipReveal { .. } => {}
            MotionOp::Shake {
                amplitude,
                rotation,
                frequency,
                decay,
                ..
            } => {
                for v in amplitude.iter().chain([rotation, decay]) {
                    if !(v.is_finite() && *v >= 0.0) {
                        self.err(
                            format!("{mp}.op"),
                            format!(
                                "shake amplitude/rotation/decay must be finite and >= 0, got {v}"
                            ),
                        );
                    }
                }
                if !(frequency.is_finite() && *frequency > 0.0 && *frequency <= 60.0) {
                    self.err(
                        format!("{mp}.frequency"),
                        format!("shake frequency must be in (0, 60] Hz, got {frequency}"),
                    );
                }
            }
            MotionOp::GlyphCascade { stagger, from, .. } => {
                if let Some(kind) = target {
                    if !matches!(kind, LayerKind::Text(_)) {
                        self.err(
                            format!("{mp}.target"),
                            format!(
                                "glyph_cascade requires a text layer, but '{}' is {}",
                                m.target,
                                kind.type_name()
                            ),
                        );
                    }
                }
                if !(stagger.is_finite() && *stagger >= 0.0) {
                    self.err(
                        format!("{mp}.stagger"),
                        format!("stagger must be finite and >= 0, got {stagger}"),
                    );
                }
                let pose = [from.dx, from.dy, from.scale, from.rotation, from.opacity];
                if pose.iter().any(|v| !v.is_finite()) || !unit(from.opacity) || from.scale < 0.0 {
                    self.err(
                        format!("{mp}.from"),
                        "glyph pose must be finite, opacity in [0, 1], scale >= 0".to_string(),
                    );
                }
            }
            MotionOp::PathMorph { to } => {
                if let Some(kind) = target {
                    if !matches!(kind, LayerKind::Polyline { .. }) {
                        self.err(
                            format!("{mp}.target"),
                            format!(
                                "path_morph requires a polyline layer, but '{}' is {}",
                                m.target,
                                kind.type_name()
                            ),
                        );
                    }
                }
                if to.len() < 2 || to.iter().flatten().any(|v| !v.is_finite()) {
                    self.err(
                        format!("{mp}.to"),
                        "path_morph needs >= 2 finite points".to_string(),
                    );
                }
            }
            MotionOp::Echo {
                count,
                spacing,
                decay,
            } => {
                if *count == 0 || *count > 8 {
                    self.err(
                        format!("{mp}.count"),
                        format!("echo count must be 1..=8, got {count}"),
                    );
                }
                if !(spacing.is_finite() && *spacing > 0.0 && *spacing <= 0.5) {
                    self.err(
                        format!("{mp}.spacing"),
                        format!("echo spacing must be in (0, 0.5] s, got {spacing}"),
                    );
                }
                if !unit(*decay) {
                    self.err(
                        format!("{mp}.decay"),
                        format!("echo decay must be in [0, 1], got {decay}"),
                    );
                }
            }
            MotionOp::Revolve {
                radius,
                at,
                from,
                to,
            } => {
                if !(radius.is_finite() && *radius > 0.0) {
                    self.err(
                        format!("{mp}.radius"),
                        format!("revolve radius must be finite and > 0, got {radius}"),
                    );
                }
                for v in at.iter().chain(from).chain(to) {
                    if !v.is_finite() {
                        self.err(
                            format!("{mp}.op"),
                            format!("revolve angles must be finite, got {v}"),
                        );
                    }
                }
            }
            MotionOp::Tilt { from, to } => {
                for v in from.iter().chain(to) {
                    if !v.is_finite() || v.abs() > 89.0 {
                        self.err(
                            format!("{mp}.op"),
                            format!("tilt angles must be finite and within ±89°, got {v}"),
                        );
                    }
                }
            }
            MotionOp::Pulse { envelope, gain } => {
                if envelope.is_empty() {
                    self.err(
                        format!("{mp}.envelope"),
                        "pulse needs an envelope id".to_string(),
                    );
                }
                if !(gain.is_finite() && gain.abs() <= 1.0) {
                    self.err(
                        format!("{mp}.gain"),
                        format!("pulse gain must be finite and in [-1, 1], got {gain}"),
                    );
                }
            }
            MotionOp::Tint { from, to, .. } => {
                for (field, v) in [("from", *from), ("to", *to)] {
                    if !unit(v) {
                        self.err(
                            format!("{mp}.{field}"),
                            format!("tint {field} {v} must be finite and in [0, 1]"),
                        );
                    }
                }
            }
            MotionOp::Count {
                from, to, decimals, ..
            } => {
                if let Some(kind) = target {
                    if !matches!(kind, LayerKind::Text(_)) {
                        self.err(
                            format!("{mp}.target"),
                            format!(
                                "count requires a text layer, but '{}' is {}",
                                m.target,
                                kind.type_name()
                            ),
                        );
                    }
                }
                for (field, v) in [("from", from), ("to", to)] {
                    if !v.is_finite() {
                        self.err(
                            format!("{mp}.{field}"),
                            format!("count {field} must be finite, got {v}"),
                        );
                    }
                }
                if *decimals > 6 {
                    self.err(
                        format!("{mp}.decimals"),
                        format!("decimals must be <= 6, got {decimals}"),
                    );
                }
            }
            MotionOp::Trim { from, to } => {
                if let Some(kind) = target {
                    if !matches!(kind, LayerKind::Polyline { .. }) {
                        self.err(
                            format!("{mp}.target"),
                            format!(
                                "trim requires a polyline layer, but '{}' is {}",
                                m.target,
                                kind.type_name()
                            ),
                        );
                    }
                }
                for (field, v) in [("from", *from), ("to", *to)] {
                    if !unit(v) {
                        self.err(
                            format!("{mp}.{field}"),
                            format!("trim {field} {v} must be finite and in [0, 1]"),
                        );
                    }
                }
            }
            MotionOp::AccentExpand { to } => {
                self.finite(mp, "to.x", to.x);
                self.finite(mp, "to.y", to.y);
                for (field, v) in [("to.width", to.width), ("to.height", to.height)] {
                    if !(v.is_finite() && v >= 0.0) {
                        self.err(
                            format!("{mp}.{field}"),
                            format!("{field} must be finite and >= 0, got {v}"),
                        );
                    }
                }
            }
        }
    }

    // -- camera ----------------------------------------------------------------

    fn camera(&mut self, scene: &Scene, sp: &str) {
        let Some(cam) = &scene.camera else {
            return;
        };
        let cp = format!("{sp}.camera");
        if let Some(pv) = cam.pivot {
            if !(pv[0].is_finite() && pv[1].is_finite()) {
                self.err(
                    format!("{cp}.pivot"),
                    format!("pivot must be finite, got {pv:?}"),
                );
            }
        }
        if let Some(pp) = cam.perspective {
            if !(pp.fov_deg.is_finite() && (10.0..=120.0).contains(&pp.fov_deg)) {
                self.err(
                    format!("{cp}.perspective.fov_deg"),
                    format!("fov must be in [10, 120] degrees, got {}", pp.fov_deg),
                );
            }
            if !(pp.focus_z.is_finite() && pp.aperture.is_finite() && pp.aperture >= 0.0) {
                self.err(
                    format!("{cp}.perspective"),
                    "focus_z must be finite and aperture finite and >= 0".to_string(),
                );
            }
        }
        // Spans of well-formed motions per camera channel.
        let mut push: Vec<Span> = Vec::new();
        let mut track: Vec<Span> = Vec::new();
        let mut shake: Vec<Span> = Vec::new();
        let mut roll: Vec<Span> = Vec::new();
        let mut dolly: Vec<Span> = Vec::new();
        let mut orbit: Vec<Span> = Vec::new();
        let mut focus: Vec<Span> = Vec::new();
        let camera = cam;
        for (i, m) in cam.motions.iter().enumerate() {
            let mp = format!("{cp}.motions[{i}]");
            let mut timing_ok = true;
            if !m.start.is_finite() || m.start < 0.0 {
                timing_ok = false;
                self.err(
                    format!("{mp}.start"),
                    format!("invalid timing: start {} must be finite and >= 0", m.start),
                );
            }
            if !m.duration.is_finite() || m.duration < 0.0 {
                timing_ok = false;
                self.err(
                    format!("{mp}.duration"),
                    format!(
                        "invalid timing: duration {} must be finite and >= 0",
                        m.duration
                    ),
                );
            }
            if let Some(v) = m.velocity {
                if !v.iter().all(|s| s.is_finite() && (0.0..=3.0).contains(s)) {
                    self.err(
                        format!("{mp}.velocity"),
                        format!("velocity slopes must be finite and in [0, 3], got {v:?}"),
                    );
                }
            }
            match &m.op {
                CameraOp::Push { from, to } => {
                    for (field, v) in [("from", *from), ("to", *to)] {
                        if !(v.is_finite() && v > 0.0) {
                            self.err(
                                format!("{mp}.{field}"),
                                format!("push {field} must be finite and > 0, got {v}"),
                            );
                        }
                    }
                }
                CameraOp::Track { from, to } | CameraOp::Orbit { from, to } => {
                    for v in from.iter().chain(to.iter()) {
                        if !v.is_finite() {
                            self.err(
                                format!("{mp}.op"),
                                format!("camera values must be finite, got {v}"),
                            );
                        }
                    }
                }
                CameraOp::Roll { from, to }
                | CameraOp::Dolly { from, to }
                | CameraOp::Focus { from, to } => {
                    for (field, v) in [("from", *from), ("to", *to)] {
                        self.finite(&mp, field, v);
                    }
                }
                CameraOp::Shake {
                    trauma,
                    frequency,
                    decay,
                    ..
                } => {
                    if !(trauma.is_finite() && (0.0..=1.0).contains(trauma)) {
                        self.err(
                            format!("{mp}.trauma"),
                            format!("trauma must be in [0, 1], got {trauma}"),
                        );
                    }
                    if !(frequency.is_finite() && *frequency > 0.0 && *frequency <= 60.0) {
                        self.err(
                            format!("{mp}.frequency"),
                            format!("shake frequency must be in (0, 60] Hz, got {frequency}"),
                        );
                    }
                    if !(decay.is_finite() && *decay >= 0.0) {
                        self.err(
                            format!("{mp}.decay"),
                            format!("decay must be finite and >= 0, got {decay}"),
                        );
                    }
                }
            }
            if matches!(
                m.op,
                CameraOp::Dolly { .. } | CameraOp::Orbit { .. } | CameraOp::Focus { .. }
            ) && camera.perspective.is_none()
            {
                self.err(
                    format!("{mp}.op"),
                    "dolly/orbit/focus require camera.perspective".to_string(),
                );
            }
            if timing_ok {
                let span = (m.start, m.start + m.duration, mp.clone());
                match &m.op {
                    CameraOp::Push { .. } => push.push(span),
                    CameraOp::Track { .. } => track.push(span),
                    CameraOp::Shake { .. } => shake.push(span),
                    CameraOp::Roll { .. } => roll.push(span),
                    CameraOp::Dolly { .. } => dolly.push(span),
                    CameraOp::Orbit { .. } => orbit.push(span),
                    CameraOp::Focus { .. } => focus.push(span),
                }
            }
        }
        for (name, list) in [
            ("push", &push),
            ("track", &track),
            ("shake", &shake),
            ("roll", &roll),
            ("dolly", &dolly),
            ("orbit", &orbit),
            ("focus", &focus),
        ] {
            for (j, b) in list.iter().enumerate() {
                for a in &list[..j] {
                    if intervals_overlap((a.0, a.1), (b.0, b.1)) {
                        self.err(
                            b.2.clone(),
                            format!(
                                "conflicting camera animations on channel {name}: overlaps {}",
                                a.2
                            ),
                        );
                    }
                }
            }
        }
    }

    // -- shared elements -------------------------------------------------------

    fn shared(&mut self) {
        let p = self.project;
        let mut seen: HashSet<&str> = HashSet::new();
        for el in &p.shared {
            if !seen.insert(el.id.as_str()) {
                self.err(
                    format!("shared[{}].id", el.id),
                    format!("duplicate shared element id '{}'", el.id),
                );
            }
            self.shared_element(el);
            self.shared_motions(el);
        }
    }

    /// Motions from different scenes on one shared element must not overlap
    /// on a channel in absolute time (same-scene overlaps are caught per scene).
    fn shared_motions(&mut self, el: &SharedElement) {
        let mut spans: Vec<(Channel, f64, f64, &str, String)> = Vec::new();
        for scene in &self.project.scenes {
            for (i, m) in scene.motions.iter().enumerate() {
                if m.target != el.layer.id || !m.start.is_finite() || !m.duration.is_finite() {
                    continue;
                }
                let a = scene.start_seconds + m.start;
                let label = m.id.clone().unwrap_or_else(|| i.to_string());
                spans.push((
                    m.op.channel(),
                    a,
                    a + m.duration,
                    scene.id.as_str(),
                    format!("scenes[{}].motions[{label}]", scene.id),
                ));
            }
        }
        let mut errors = Vec::new();
        for (j, b) in spans.iter().enumerate() {
            for a in &spans[..j] {
                if a.0 == b.0 && a.3 != b.3 && intervals_overlap((a.1, a.2), (b.1, b.2)) {
                    errors.push((
                        b.4.clone(),
                        format!(
                            "conflicting animations on shared element '{}' channel {:?}: overlaps {}",
                            el.id, b.0, a.4
                        ),
                    ));
                }
            }
        }
        for (path, message) in errors {
            self.err(path, message);
        }
    }

    fn shared_element(&mut self, el: &SharedElement) {
        let sp = format!("shared[{}]", el.id);
        self.layer(&el.layer, &format!("{sp}.layer"), &[]);

        if el.track.is_empty() {
            self.err(
                format!("{sp}.track"),
                "track is empty: needs at least one key",
            );
        }

        let mut prev: Option<f64> = None;
        for (i, k) in el.track.iter().enumerate() {
            let kp = format!("{sp}.track[{i}]");

            let mut at_ok = true;
            if !k.at.is_finite() || k.at < 0.0 {
                at_ok = false;
                self.err(
                    format!("{kp}.at"),
                    format!("invalid timing: at {} must be finite and >= 0", k.at),
                );
            }
            if let Some(b) = &k.layout {
                self.layout_binding(b, &format!("{kp}.layout"));
                if let Some(scene) = self.project.scenes.iter().find(|s| s.id == k.scene) {
                    let mut ids: HashSet<&str> = HashSet::new();
                    collect_layer_ids(&scene.layers, &mut ids);
                    if !ids.contains(b.parent.as_str()) {
                        self.err(
                            format!("{kp}.layout.parent"),
                            format!(
                                "layout parent '{}' not found in scene '{}'",
                                b.parent, scene.id
                            ),
                        );
                    }
                }
            }
            let scene_start = self.project.scene_start(&k.scene);
            if scene_start.is_none() {
                self.err(
                    format!("{kp}.scene"),
                    format!("missing scene '{}'", k.scene),
                );
            }
            if let (Some(start), true) = (scene_start, at_ok) {
                let abs = start + k.at;
                if let Some(pv) = prev {
                    if abs < pv - EPS {
                        self.err(
                            format!("{kp}.at"),
                            format!(
                                "invalid timing: key at {abs}s is earlier than the previous key at {pv}s"
                            ),
                        );
                    }
                }
                prev = Some(abs);
            }

            if let Some(o) = k.state.opacity {
                if !(o.is_finite() && (0.0..=1.0).contains(&o)) {
                    self.err(
                        format!("{kp}.state.opacity"),
                        format!("invalid opacity {o}: must be finite and in [0, 1]"),
                    );
                }
            }
            if let Some(s) = k.state.scale {
                if !(s.is_finite() && s > 0.0) {
                    self.err(
                        format!("{kp}.state.scale"),
                        format!("scale must be finite and > 0, got {s}"),
                    );
                }
            }
            for (field, v) in [
                ("x", k.state.x),
                ("y", k.state.y),
                ("rotation_degrees", k.state.rotation_degrees),
            ] {
                if let Some(v) = v {
                    self.finite(&format!("{kp}.state"), field, v);
                }
            }
        }
    }
}

/// Overlap of two half-open intervals `[start, end)`. A zero-length interval
/// `[t, t]` only overlaps when `t` lies strictly inside the other interval.
fn intervals_overlap(a: (f64, f64), b: (f64, f64)) -> bool {
    let a_zero = a.1 - a.0 <= OVERLAP_EPS;
    let b_zero = b.1 - b.0 <= OVERLAP_EPS;
    match (a_zero, b_zero) {
        (true, true) => false,
        (true, false) => a.0 > b.0 + OVERLAP_EPS && a.0 < b.1 - OVERLAP_EPS,
        (false, true) => b.0 > a.0 + OVERLAP_EPS && b.0 < a.1 - OVERLAP_EPS,
        (false, false) => a.0 < b.1 - OVERLAP_EPS && b.0 < a.1 - OVERLAP_EPS,
    }
}
