//! CPU render backend on tiny-skia.
//!
//! All static assets are prepared once in [`CpuRenderer::new`] (fonts shaped
//! into paths, images decoded, SVGs rasterized, textures generated). After
//! that the renderer is immutable and `Sync`, so frames can render in parallel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use motion_core::scene::{
    self, AssetKind, Color, Fit, FontRole, GlyphPose, Layer, LayerKind, MotionProject, ScreenInsert,
};
use motion_core::timeline::{Affine, ResolvedFrame, ResolvedLayer, ResolvedTint};
use resvg::tiny_skia::{
    self, BlendMode, FillRule, FilterQuality, Mask, Paint, Path as SkPath, PathBuilder, Pixmap,
    PixmapPaint, Rect, Stroke, Transform,
};
use resvg::usvg;

use crate::text::{GlyphOutline, TextEngine};
use crate::{blur, postfx, texture, warp};
use crate::{RenderError, Renderer};

/// Supersampling factor for rasterized SVGs (headroom for animated scale).
const SVG_OVERSAMPLE: f32 = 2.0;
const SVG_MAX_SIDE: f32 = 2400.0;
/// Extra pixels generated around animated textures so per-frame offsets never expose an edge.
const ANIM_PAD: u32 = 64;
/// Transparent margin (offscreen px) around a projectively warped layer's box,
/// so strokes and overflowing ink are not cut at the box edge.
const PROJ_PAD: u32 = 16;
/// Longest side of a projective layer's box-space offscreen (px); larger boxes
/// are rendered scaled down.
const PROJ_MAX_SIDE: f32 = 4096.0;
/// Gaussian sigma per px of blur radius: the radius is the circle-of-confusion
/// radius, whose point-spread (a disc) has a standard deviation of radius / 2.
/// Radii whose sigma is below [`blur::MIN_SIGMA`] are invisible and not applied.
const BLUR_SIGMA_PER_RADIUS: f32 = 0.5;

pub struct CpuRenderer {
    width: u32,
    height: u32,
    /// Text layer id -> outline path in layer-local pixels.
    text: HashMap<String, SkPath>,
    /// Text layer id -> box width its static outline was laid out for.
    text_widths: HashMap<String, f32>,
    /// Image asset id -> decoded pixmap.
    images: HashMap<String, Pixmap>,
    /// Image layer id -> (treated pixmap, padding in image pixels). Present only
    /// for layers with a treatment; drawn instead of `images[asset]`.
    treated: HashMap<String, (Pixmap, u32)>,
    /// Svg layer id -> (raster, oversample factor).
    svgs: HashMap<String, (Pixmap, f32)>,
    /// Texture layer id -> (pixmap at the layer's base size (+ padding if animated), animated).
    textures: HashMap<String, (Pixmap, bool)>,
    /// Lazily shaped Count strings (see [`TextOverrides`]).
    overrides: Mutex<TextOverrides>,
    /// (0.11) Sprite-sequence asset id -> where its frames live.
    sprites: HashMap<String, SpriteSource>,
    /// (0.11) Image layer id -> static box size, for layers whose treatment is
    /// applied lazily per sprite frame (keeps the pixels-per-unit constant).
    sprite_layers: HashMap<String, (f32, f32)>,
    /// (0.11) Lazily decoded sprite frames and their treated versions.
    sprite_cache: Mutex<SpriteCache>,
    /// (gpu feature) Optional GPU backend for the post stage; `None` = CPU.
    #[cfg(feature = "gpu")]
    gpu_post: Option<GpuPostState>,
}

/// The GPU post backend and its health. The first GPU failure is recorded and
/// the renderer falls back to the CPU post effects for that frame and all later
/// ones (the pixmap is untouched when the GPU fails).
#[cfg(feature = "gpu")]
struct GpuPostState {
    gpu: Mutex<crate::gpu::GpuPost>,
    failure: Mutex<Option<String>>,
    disabled: std::sync::atomic::AtomicBool,
}

/// Directory + naming of one sprite-sequence asset.
struct SpriteSource {
    dir: PathBuf,
    pattern: String,
    frame_count: u32,
}

/// Memory bound for decoded sprite frames (RGBA bytes). When exceeded the
/// cache is dropped and frames are re-decoded on demand; decoding is
/// deterministic so this never changes pixels.
const SPRITE_CACHE_BYTES: usize = 384 * 1024 * 1024;

/// Decoded sprite frames shared by parallel frame renders.
#[derive(Default)]
struct SpriteCache {
    /// (asset id, frame) -> decoded frame.
    frames: HashMap<(String, u32), Arc<Pixmap>>,
    /// (layer id, frame) -> treated frame and its padding.
    treated: HashMap<(String, u32), Arc<(Pixmap, u32)>>,
    bytes: usize,
}

impl SpriteCache {
    fn reserve(&mut self, extra: usize) {
        if self.bytes + extra > SPRITE_CACHE_BYTES {
            self.frames.clear();
            self.treated.clear();
            self.bytes = 0;
        }
        self.bytes += extra;
    }
}

/// A still image or one decoded sprite frame.
enum ImgRef<'a> {
    Still(&'a Pixmap),
    Frame(Arc<Pixmap>),
}

impl ImgRef<'_> {
    fn pixmap(&self) -> &Pixmap {
        match self {
            ImgRef::Still(p) => p,
            ImgRef::Frame(p) => p,
        }
    }
}

/// Shaping state for text that differs from a layer's authored text (Count).
///
/// Holds the same `TextEngine` (same fonts, same load order) that prepared the
/// static text, plus a cache keyed by `(layer id, string)`. Rendering runs in
/// parallel, so the whole struct sits behind a `Mutex`: a frame locks it only
/// when it draws an overridden text layer, and a cache hit is one hash lookup
/// and an `Arc` clone. Each distinct string is shaped once for the whole render.
struct TextOverrides {
    engine: TextEngine,
    /// Font role -> loaded family name.
    families: HashMap<FontRole, String>,
    cache: HashMap<(String, String), Option<Arc<SkPath>>>,
    /// (0.14) Per-glyph outlines for `glyphs` poses, keyed like `cache`.
    glyph_cache: HashMap<(String, String), Option<Arc<Vec<GlyphOutline>>>>,
}

impl TextOverrides {
    fn path(
        &mut self,
        id: &str,
        style: &motion_core::scene::TextStyle,
        text: &str,
        width: f32,
    ) -> Option<Arc<SkPath>> {
        let key = (id.to_string(), text.to_string());
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let family = self.families.get(&style.font_role)?.clone();
        let mut styled = style.clone();
        styled.text = text.to_string();
        let path = self.engine.outline(&family, &styled, width).map(Arc::new);
        self.cache.insert(key, path.clone());
        path
    }

    /// Per-glyph outlines of `text` (same layout as [`Self::path`]).
    fn glyphs(
        &mut self,
        id: &str,
        style: &motion_core::scene::TextStyle,
        text: &str,
        width: f32,
    ) -> Option<Arc<Vec<GlyphOutline>>> {
        let key = (id.to_string(), text.to_string());
        if let Some(hit) = self.glyph_cache.get(&key) {
            return hit.clone();
        }
        let family = self.families.get(&style.font_role)?.clone();
        let mut styled = style.clone();
        styled.text = text.to_string();
        let glyphs = self.engine.glyph_outlines(&family, &styled, width);
        let glyphs = (!glyphs.is_empty()).then(|| Arc::new(glyphs));
        self.glyph_cache.insert(key, glyphs.clone());
        glyphs
    }
}

#[cfg(feature = "gpu")]
impl CpuRenderer {
    /// Run the post stage on `gpu` instead of the CPU effects. The CPU stays
    /// the reference: output matches within a few levels (see
    /// `docs/POST_EFFECTS.md`), and any GPU failure falls back to the CPU.
    pub fn with_gpu_post(mut self, gpu: crate::gpu::GpuPost) -> Self {
        self.gpu_post = Some(GpuPostState {
            gpu: Mutex::new(gpu),
            failure: Mutex::new(None),
            disabled: std::sync::atomic::AtomicBool::new(false),
        });
        self
    }

    /// Why the GPU post stage stopped (the first failure), if it did.
    pub fn gpu_post_failure(&self) -> Option<String> {
        let st = self.gpu_post.as_ref()?;
        st.failure.lock().ok()?.clone()
    }
}

impl CpuRenderer {
    /// Full-frame post effects: the GPU backend when attached and healthy,
    /// otherwise the CPU reference.
    fn apply_post(&self, pm: &mut Pixmap, post: &[motion_core::timeline::ResolvedPost<'_>]) {
        #[cfg(feature = "gpu")]
        if let Some(st) = &self.gpu_post {
            use std::sync::atomic::Ordering;
            if post.is_empty() {
                return;
            }
            if !st.disabled.load(Ordering::Relaxed) {
                let result = match st.gpu.lock() {
                    Ok(mut g) => g.apply(pm, post).map_err(|e| e.to_string()),
                    Err(_) => Err("GPU backend poisoned by a panic".to_string()),
                };
                match result {
                    Ok(()) => return,
                    Err(e) => {
                        st.disabled.store(true, Ordering::Relaxed);
                        if let Ok(mut first) = st.failure.lock() {
                            first.get_or_insert(e);
                        }
                    }
                }
            }
        }
        postfx::apply_post(pm, post);
    }

    /// Prepare all assets for `project`. `base_dir` is the directory containing
    /// the motion file (asset paths resolve against it and `asset_root`).
    pub fn new(project: &MotionProject, base_dir: &Path) -> Result<Self, RenderError> {
        let root: PathBuf = base_dir.join(project.asset_root.as_deref().unwrap_or(""));
        let asset_path = |id: &str| -> Result<(AssetKind, PathBuf), RenderError> {
            let a = project
                .asset(id)
                .ok_or_else(|| RenderError::Asset(format!("unknown asset '{id}'")))?;
            Ok((a.kind, root.join(&a.path)))
        };

        // Fonts: load every font asset in declaration order (determines fallback order).
        let mut engine = TextEngine::new();
        let mut family_by_asset = HashMap::new();
        for a in project.assets.iter().filter(|a| a.kind == AssetKind::Font) {
            family_by_asset.insert(a.id.clone(), engine.load(&root.join(&a.path))?);
        }

        let mut images = HashMap::new();
        let mut sprites: HashMap<String, SpriteSource> = HashMap::new();
        let mut svg_trees: HashMap<String, usvg::Tree> = HashMap::new();
        for a in &project.assets {
            let path = root.join(&a.path);
            match a.kind {
                AssetKind::Image => {
                    let pm = crate::decode::decode_file(&path)
                        .map_err(|e| RenderError::Asset(format!("image {}: {e}", path.display())))?
                        .pixmap;
                    images.insert(a.id.clone(), pm);
                }
                AssetKind::Svg => {
                    let data = std::fs::read(&path)
                        .map_err(|e| RenderError::Asset(format!("svg {}: {e}", path.display())))?;
                    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
                        .map_err(|e| RenderError::Asset(format!("svg {}: {e}", path.display())))?;
                    svg_trees.insert(a.id.clone(), tree);
                }
                AssetKind::Font => {}
                // Frames are decoded lazily on first use (see `sprite_frame`).
                AssetKind::SpriteSequence => {
                    let spec = a.sprite.as_ref().ok_or_else(|| {
                        RenderError::Asset(format!("sprite asset '{}' has no sprite spec", a.id))
                    })?;
                    sprites.insert(
                        a.id.clone(),
                        SpriteSource {
                            dir: path,
                            pattern: spec.pattern.clone(),
                            frame_count: spec.frame_count,
                        },
                    );
                }
            }
        }

        let mut r = CpuRenderer {
            width: project.canvas.width,
            height: project.canvas.height,
            text: HashMap::new(),
            text_widths: HashMap::new(),
            images,
            treated: HashMap::new(),
            svgs: HashMap::new(),
            textures: HashMap::new(),
            overrides: Mutex::new(TextOverrides {
                engine: TextEngine::new(),
                families: HashMap::new(),
                cache: HashMap::new(),
                glyph_cache: HashMap::new(),
            }),
            sprites,
            sprite_layers: HashMap::new(),
            sprite_cache: Mutex::new(SpriteCache::default()),
            #[cfg(feature = "gpu")]
            gpu_post: None,
        };

        let mut all: Vec<&Layer> = Vec::new();
        for s in &project.scenes {
            collect(&s.layers, &mut all);
        }
        for e in &project.shared {
            collect(std::slice::from_ref(&e.layer), &mut all);
        }
        for layer in all {
            match &layer.kind {
                LayerKind::Text(style) => {
                    let asset = project.theme.fonts.get(&style.font_role).ok_or_else(|| {
                        RenderError::Asset(format!(
                            "layer '{}': no font for role {:?}",
                            layer.id, style.font_role
                        ))
                    })?;
                    let family = family_by_asset.get(asset).ok_or_else(|| {
                        RenderError::Asset(format!("font asset '{asset}' not loaded"))
                    })?;
                    if let Some(path) = engine.outline(family, style, layer.width) {
                        r.text.insert(layer.id.clone(), path);
                    }
                    r.text_widths.insert(layer.id.clone(), layer.width);
                }
                LayerKind::Svg { asset, fit } => {
                    let (_, path) = asset_path(asset)?;
                    let tree = svg_trees.get(asset).ok_or_else(|| {
                        RenderError::Asset(format!("svg '{}' not loaded", path.display()))
                    })?;
                    if let Some(raster) = rasterize_svg(tree, layer.width, layer.height, *fit) {
                        r.svgs.insert(layer.id.clone(), raster);
                    }
                }
                LayerKind::Texture(spec) => {
                    let (w, h) = (
                        layer.width.ceil().max(0.0) as u32,
                        layer.height.ceil().max(0.0) as u32,
                    );
                    let pad = if spec.animated { ANIM_PAD } else { 0 };
                    if let Some(pm) = texture::generate(spec, w + pad, h + pad) {
                        r.textures.insert(layer.id.clone(), (pm, spec.animated));
                    }
                }
                LayerKind::Image {
                    asset,
                    fit,
                    treatment,
                    ..
                } => {
                    asset_path(asset)?;
                    if r.sprites.contains_key(asset) {
                        if treatment.is_some() {
                            r.sprite_layers
                                .insert(layer.id.clone(), (layer.width, layer.height));
                        }
                        continue;
                    }
                    if let (Some(t), Some(img)) = (treatment, r.images.get(asset)) {
                        let ppu = image_px_per_unit(img, layer.width, layer.height, *fit);
                        let (pm, pad) = crate::treatment::apply(img, t, ppu);
                        r.treated.insert(layer.id.clone(), (pm, pad));
                    }
                }
                _ => {}
            }
        }
        // Hand the prepared engine (fonts already loaded, in order) to the
        // override cache so Count text shapes with identical fallback.
        let role_family: HashMap<FontRole, String> = project
            .theme
            .fonts
            .iter()
            .filter_map(|(role, asset)| family_by_asset.get(asset).map(|f| (*role, f.clone())))
            .collect();
        r.overrides = Mutex::new(TextOverrides {
            engine,
            families: role_family,
            cache: HashMap::new(),
            glyph_cache: HashMap::new(),
        });
        Ok(r)
    }

    fn overrides(&self) -> MutexGuard<'_, TextOverrides> {
        self.overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn draw(
        &self,
        target: &mut Pixmap,
        l: &ResolvedLayer<'_>,
        frame: u32,
    ) -> Result<(), RenderError> {
        // (0.16) Depth-of-field blur radius, if it is large enough to see.
        let blur_radius = l
            .blur
            .filter(|r| r.is_finite() && *r * BLUR_SIGMA_PER_RADIUS >= blur::MIN_SIGMA);
        if let LayerKind::Group { .. } = l.kind {
            let isolate = l.opacity < 1.0 || l.clip.is_some() || blur_radius.is_some();
            if !isolate {
                for c in &l.children {
                    self.draw(target, c, frame)?;
                }
                return Ok(());
            }
            let mut off = Self::blank_like(target)?;
            for c in &l.children {
                self.draw(&mut off, c, frame)?;
            }
            if let Some(r) = blur_radius {
                blur::gaussian_blur(&mut off, r * BLUR_SIGMA_PER_RADIUS);
            }
            let mask = Self::clip_mask(l, target);
            let paint = PixmapPaint {
                opacity: l.opacity,
                quality: FilterQuality::Nearest,
                ..Default::default()
            };
            target.draw_pixmap(
                0,
                0,
                off.as_ref(),
                &paint,
                Transform::identity(),
                mask.as_ref(),
            );
            return Ok(());
        }

        if let Some(radius) = blur_radius {
            return self.draw_blurred(target, l, radius, frame);
        }
        if let Some(h) = l.projective {
            return self.draw_projective(target, l, &h, frame);
        }

        // Raster layers are tinted through an offscreen: draw untinted at full
        // opacity, mix the tint colour in over the drawn pixels only
        // (SourceAtop keeps transparency), then composite with opacity/clip.
        if let Some(tint) = l.tint {
            if matches!(
                l.kind,
                LayerKind::Image { .. } | LayerKind::Svg { .. } | LayerKind::Texture(_)
            ) {
                let mut inner = l.clone();
                inner.tint = None;
                inner.opacity = 1.0;
                inner.clip = None;
                let mut off = Self::blank_like(target)?;
                self.draw(&mut off, &inner, frame)?;
                if let Some(full) =
                    Rect::from_xywh(0.0, 0.0, target.width() as f32, target.height() as f32)
                {
                    let mut p = Paint::default();
                    p.set_color(sk_color(tint.color.with_alpha(255), tint.amount));
                    p.blend_mode = BlendMode::SourceAtop;
                    p.anti_alias = false;
                    off.fill_rect(full, &p, Transform::identity(), None);
                }
                let mask = Self::clip_mask(l, target);
                let paint = PixmapPaint {
                    opacity: l.opacity,
                    quality: FilterQuality::Nearest,
                    ..Default::default()
                };
                target.draw_pixmap(
                    0,
                    0,
                    off.as_ref(),
                    &paint,
                    Transform::identity(),
                    mask.as_ref(),
                );
                return Ok(());
            }
        }

        let mask = Self::clip_mask(l, target);
        let ts = to_sk(l.content_transform);
        let (w, h) = (l.width, l.height);
        match l.kind {
            LayerKind::Polyline {
                points,
                stroke,
                closed,
                fill,
            } => draw_polyline(
                target,
                // (0.14) `path_morph` replaces the layer's own points.
                l.points.as_deref().unwrap_or(points),
                stroke,
                *closed,
                *fill,
                l.trim,
                l.opacity,
                l.tint,
                ts,
                mask.as_ref(),
            ),
            LayerKind::Rectangle { fill, stroke } => {
                if let Some(rect) = Rect::from_xywh(0.0, 0.0, w, h) {
                    let path = PathBuilder::from_rect(rect);
                    target.fill_path(
                        &path,
                        &paint(tinted(*fill, l.tint), l.opacity),
                        FillRule::Winding,
                        ts,
                        mask.as_ref(),
                    );
                    if let Some(s) = stroke {
                        stroke_path(target, &path, s, l.opacity, l.tint, ts, mask.as_ref());
                    }
                }
            }
            LayerKind::RoundedRectangle {
                fill,
                radius,
                stroke,
            } => {
                if let Some(path) = rounded_rect(w, h, *radius) {
                    target.fill_path(
                        &path,
                        &paint(tinted(*fill, l.tint), l.opacity),
                        FillRule::Winding,
                        ts,
                        mask.as_ref(),
                    );
                    if let Some(s) = stroke {
                        stroke_path(target, &path, s, l.opacity, l.tint, ts, mask.as_ref());
                    }
                }
            }
            LayerKind::Text(style) => {
                // Count override: shape the resolved string through the cache;
                // otherwise use the path prepared at load time (unchanged path).
                let override_text = l.text.as_deref().filter(|s| *s != style.text);
                // (0.14) Per-glyph poses; all-rest (or none) draws the layer
                // exactly as a plain text layer.
                let poses = l
                    .glyphs
                    .as_deref()
                    .filter(|g| g.iter().any(|p| *p != GlyphPose::REST));
                let glyph_set = poses.and_then(|_| {
                    // Same layout width as the plain path it replaces.
                    let (text, width) = match override_text {
                        Some(s) => (s, w),
                        None => (
                            style.text.as_str(),
                            self.text_widths.get(l.id).copied().unwrap_or(w),
                        ),
                    };
                    self.overrides().glyphs(l.id, style, text, width)
                });
                if let (Some(poses), Some(set)) = (poses, glyph_set) {
                    draw_posed_glyphs(
                        target,
                        &set,
                        poses,
                        tinted(style.color, l.tint),
                        l.opacity,
                        l.content_transform,
                        mask.as_ref(),
                    );
                    return Ok(());
                }
                let shaped;
                let path: Option<&SkPath> = match override_text {
                    Some(s) => {
                        shaped = self.overrides().path(l.id, style, s, w);
                        shaped.as_deref()
                    }
                    None => self.text.get(l.id),
                };
                if let Some(path) = path {
                    target.fill_path(
                        path,
                        &paint(tinted(style.color, l.tint), l.opacity),
                        FillRule::Winding,
                        ts,
                        mask.as_ref(),
                    );
                }
            }
            LayerKind::Image {
                asset,
                fit,
                treatment,
                insert,
                ..
            } => {
                let img = self.image_ref(asset, l.sprite_frame)?;
                let cover_mask;
                let mask = if *fit == Fit::Cover && mask.is_none() {
                    cover_mask = Self::rect_mask(
                        Rect::from_xywh(0.0, 0.0, w, h),
                        to_sk(l.transform),
                        target,
                    );
                    cover_mask.as_ref()
                } else {
                    mask.as_ref()
                };
                if let Some(ins) = insert {
                    self.draw_insert(target, l, ins, mask, ts)?;
                }
                if let Some(img) = img {
                    let img = img.pixmap();
                    let (iw, ih) = (img.width() as f32, img.height() as f32);
                    let fit_ts = fit_transform(iw, ih, w, h, *fit);
                    let paint = PixmapPaint {
                        opacity: l.opacity,
                        quality: FilterQuality::Bilinear,
                        ..Default::default()
                    };
                    let ts = ts.pre_concat(fit_ts);
                    let sprite_treated = match (treatment, l.sprite_frame) {
                        (Some(t), Some(f)) if self.sprite_layers.contains_key(l.id) => {
                            Some(self.sprite_treated(l.id, f, img, t, *fit))
                        }
                        _ => None,
                    };
                    match sprite_treated.as_deref().or_else(|| self.treated.get(l.id)) {
                        // Treated pixmap is padded: shift back so the image
                        // content lands where the untreated image would.
                        Some((pm, pad)) => {
                            let p = *pad as f32;
                            target.draw_pixmap(
                                0,
                                0,
                                pm.as_ref(),
                                &paint,
                                ts.pre_translate(-p, -p),
                                mask,
                            );
                        }
                        None => target.draw_pixmap(0, 0, img.as_ref(), &paint, ts, mask),
                    }
                }
            }
            LayerKind::Svg { .. } => {
                if let Some((pm, k)) = self.svgs.get(l.id) {
                    let paint = PixmapPaint {
                        opacity: l.opacity,
                        quality: FilterQuality::Bilinear,
                        ..Default::default()
                    };
                    target.draw_pixmap(
                        0,
                        0,
                        pm.as_ref(),
                        &paint,
                        ts.pre_scale(1.0 / k, 1.0 / k),
                        mask.as_ref(),
                    );
                }
            }
            LayerKind::Texture(_) => {
                if let Some((pm, animated)) = self.textures.get(l.id) {
                    let pad = if *animated { ANIM_PAD } else { 0 };
                    let (bw, bh) = (
                        pm.width().saturating_sub(pad),
                        pm.height().saturating_sub(pad),
                    );
                    let sx = if bw > 0 { w / bw as f32 } else { 1.0 };
                    let sy = if bh > 0 { h / bh as f32 } else { 1.0 };
                    let (dx, dy) = if *animated {
                        frame_offset(l.id, frame)
                    } else {
                        (0.0, 0.0)
                    };
                    let paint = PixmapPaint {
                        opacity: l.opacity,
                        quality: if *animated {
                            FilterQuality::Nearest
                        } else {
                            FilterQuality::Bilinear
                        },
                        ..Default::default()
                    };
                    let ts = ts.pre_scale(sx, sy).pre_translate(-dx, -dy);
                    target.draw_pixmap(0, 0, pm.as_ref(), &paint, ts, mask.as_ref());
                }
            }
            LayerKind::Group { .. } => {}
        }
        Ok(())
    }

    /// (0.16) A leaf layer with a blur radius: draw it (without opacity/clip)
    /// into an offscreen, blur, then composite with the layer's opacity and
    /// clip. Projectively warped layers keep opacity and clip inside the
    /// warp (their clip lives in box space), so the blur sits on top of them.
    fn draw_blurred(
        &self,
        target: &mut Pixmap,
        l: &ResolvedLayer<'_>,
        radius: f32,
        frame: u32,
    ) -> Result<(), RenderError> {
        let mut inner = l.clone();
        inner.blur = None;
        let warped = l.projective.is_some();
        if !warped {
            inner.opacity = 1.0;
            inner.clip = None;
        }
        let mut off = Self::blank_like(target)?;
        self.draw(&mut off, &inner, frame)?;
        blur::gaussian_blur(&mut off, radius * BLUR_SIGMA_PER_RADIUS);
        let (mask, opacity) = if warped {
            (None, 1.0)
        } else {
            (Self::clip_mask(l, target), l.opacity)
        };
        let paint = PixmapPaint {
            opacity,
            quality: FilterQuality::Nearest,
            ..Default::default()
        };
        target.draw_pixmap(
            0,
            0,
            off.as_ref(),
            &paint,
            Transform::identity(),
            mask.as_ref(),
        );
        Ok(())
    }

    /// (0.16) A leaf layer with a homography: render it in its own box space
    /// (opacity, clip, tint and `content_transform` as for a plain layer, but
    /// with the box at the offscreen origin) and warp the result onto the
    /// target with bilinear sampling. Groups are not warped: their children
    /// carry their own resolved transforms.
    fn draw_projective(
        &self,
        target: &mut Pixmap,
        l: &ResolvedLayer<'_>,
        h: &[f32; 9],
        frame: u32,
    ) -> Result<(), RenderError> {
        let (bw, bh) = (l.width, l.height);
        if !(bw.is_finite() && bh.is_finite() && bw > 0.0 && bh > 0.0) {
            return Ok(());
        }
        // Offscreen pixels per box pixel (< 1 only for enormous boxes).
        let k = (PROJ_MAX_SIDE / bw.max(bh)).min(1.0);
        let pad = PROJ_PAD as f32;
        let (ow, oh) = (
            (bw * k).ceil() as u32 + 2 * PROJ_PAD,
            (bh * k).ceil() as u32 + 2 * PROJ_PAD,
        );
        let mut off = Pixmap::new(ow, oh).ok_or(RenderError::Canvas(ow, oh))?;
        let to_off = Affine::translate(pad, pad).then_apply(Affine::scale(k, k));
        let mut inner = l.clone();
        inner.projective = None;
        inner.blur = None;
        inner.transform = to_off;
        inner.content_transform = to_off.then_apply(l.content_transform);
        self.draw(&mut off, &inner, frame)?;

        // offscreen px -> box px -> canvas px.
        let from_off = warp::mat3_affine(1.0 / k, 0.0, 0.0, 1.0 / k, -pad / k, -pad / k);
        let m = warp::mat3_mul(&warp::mat3_from_f32(h), &from_off);
        warp::warp_over(target, &off, &m);
        Ok(())
    }

    /// The pixmap an image layer or insert draws: the decoded still, or the
    /// sprite frame `frame` (0 when the Timeline gave none). `None` when the
    /// asset has no raster (never for validated projects).
    fn image_ref(
        &self,
        asset: &str,
        frame: Option<u32>,
    ) -> Result<Option<ImgRef<'_>>, RenderError> {
        if let Some(img) = self.images.get(asset) {
            return Ok(Some(ImgRef::Still(img)));
        }
        match self.sprites.get(asset) {
            Some(src) => Ok(Some(ImgRef::Frame(self.sprite_frame(
                asset,
                src,
                frame.unwrap_or(0),
            )?))),
            None => Ok(None),
        }
    }

    /// Decode (or fetch) frame `frame` of a sprite asset. Decoding happens
    /// outside the lock; two threads racing on a frame both decode the same
    /// pixels and one insert wins.
    fn sprite_frame(
        &self,
        asset: &str,
        src: &SpriteSource,
        frame: u32,
    ) -> Result<Arc<Pixmap>, RenderError> {
        let frame = frame.min(src.frame_count.saturating_sub(1));
        let key = (asset.to_string(), frame);
        if let Some(hit) = self.cache().frames.get(&key) {
            return Ok(hit.clone());
        }
        let name = scene::sprite_file_name(&src.pattern, frame + 1).ok_or_else(|| {
            RenderError::Asset(format!(
                "sprite '{asset}': bad frame pattern '{}'",
                src.pattern
            ))
        })?;
        let path = src.dir.join(name);
        let pm = crate::decode::decode_file(&path)
            .map_err(|e| RenderError::Asset(format!("sprite frame {}: {e}", path.display())))?
            .pixmap;
        let bytes = pm.data().len();
        let pm = Arc::new(pm);
        let mut cache = self.cache();
        if let Some(hit) = cache.frames.get(&key) {
            return Ok(hit.clone());
        }
        cache.reserve(bytes);
        cache.frames.insert(key, pm.clone());
        Ok(pm)
    }

    /// Treated version of sprite frame `frame` for layer `layer`, cached per
    /// (layer, frame) so every render of that frame sees the same pixels.
    fn sprite_treated(
        &self,
        layer: &str,
        frame: u32,
        img: &Pixmap,
        treatment: &motion_core::scene::ImageTreatment,
        fit: Fit,
    ) -> Arc<(Pixmap, u32)> {
        let key = (layer.to_string(), frame);
        if let Some(hit) = self.cache().treated.get(&key) {
            return hit.clone();
        }
        let (w, h) = self.sprite_layers.get(layer).copied().unwrap_or((0.0, 0.0));
        let ppu = image_px_per_unit(img, w, h, fit);
        let treated = crate::treatment::apply(img, treatment, ppu);
        let bytes = treated.0.data().len();
        let treated = Arc::new(treated);
        let mut cache = self.cache();
        if let Some(hit) = cache.treated.get(&key) {
            return hit.clone();
        }
        cache.reserve(bytes);
        cache.treated.insert(key, treated.clone());
        treated
    }

    fn cache(&self) -> MutexGuard<'_, SpriteCache> {
        self.sprite_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Draw a screen insert under the host: `screen_box` is a fraction of the
    /// host's box, drawn in box space (so it follows the host's transform),
    /// clipped to that rectangle (and the host's own mask), at the host's
    /// opacity. The host image is drawn afterwards, over the insert.
    fn draw_insert(
        &self,
        target: &mut Pixmap,
        l: &ResolvedLayer<'_>,
        ins: &ScreenInsert,
        host_mask: Option<&Mask>,
        ts: Transform,
    ) -> Result<(), RenderError> {
        let [bx, by, bw, bh] = ins.screen_box;
        let (rx, ry, rw, rh) = (bx * l.width, by * l.height, bw * l.width, bh * l.height);
        let Some(rect) = Rect::from_xywh(rx, ry, rw, rh) else {
            return Ok(());
        };
        let Some(img) = self.image_ref(&ins.asset, l.insert_frame)? else {
            return Ok(());
        };
        let img = img.pixmap();
        let Some(mut mask) = Self::rect_mask(Some(rect), ts, target) else {
            return Ok(());
        };
        if let Some(host) = host_mask {
            for (m, h) in mask.data_mut().iter_mut().zip(host.data()) {
                *m = ((*m as u16 * *h as u16 + 127) / 255) as u8;
            }
        }
        let fit_ts = fit_transform(img.width() as f32, img.height() as f32, rw, rh, ins.fit);
        let paint = PixmapPaint {
            opacity: l.opacity,
            quality: FilterQuality::Bilinear,
            ..Default::default()
        };
        let ts = ts.pre_translate(rx, ry).pre_concat(fit_ts);
        target.draw_pixmap(0, 0, img.as_ref(), &paint, ts, Some(&mask));
        Ok(())
    }

    fn blank(&self) -> Result<Pixmap, RenderError> {
        Pixmap::new(self.width, self.height).ok_or(RenderError::Canvas(self.width, self.height))
    }

    /// A transparent pixmap the size of `like`.
    fn blank_like(like: &Pixmap) -> Result<Pixmap, RenderError> {
        let (w, h) = (like.width(), like.height());
        Pixmap::new(w, h).ok_or(RenderError::Canvas(w, h))
    }

    fn clip_mask(l: &ResolvedLayer<'_>, target: &Pixmap) -> Option<Mask> {
        let c = l.clip?;
        let (w, h) = (l.width, l.height);
        let rect = Rect::from_ltrb(
            c.left * w,
            c.top * h,
            (1.0 - c.right) * w,
            (1.0 - c.bottom) * h,
        );
        // A degenerate clip hides everything: return an empty mask.
        match Self::rect_mask(rect, to_sk(l.transform), target) {
            Some(m) => Some(m),
            None => Mask::new(target.width(), target.height()),
        }
    }

    fn rect_mask(rect: Option<Rect>, ts: Transform, target: &Pixmap) -> Option<Mask> {
        let path = PathBuilder::from_rect(rect?);
        let mut mask = Mask::new(target.width(), target.height())?;
        mask.fill_path(&path, FillRule::Winding, true, ts);
        Some(mask)
    }
}

impl Renderer for CpuRenderer {
    type Frame = Pixmap;

    fn render(&self, frame: &ResolvedFrame<'_>) -> Result<Pixmap, RenderError> {
        let mut pm = self.blank()?;
        pm.fill(sk_color(frame.background, 1.0));
        for l in &frame.layers {
            self.draw(&mut pm, l, frame.frame)?;
        }
        // (0.15) Full-frame post effects on the composited frame.
        self.apply_post(&mut pm, &frame.post);
        Ok(pm)
    }
}

fn collect<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            collect(children, out);
        }
    }
}

/// Deterministic per-frame sampling offset for animated textures.
fn frame_offset(id: &str, frame: u32) -> (f32, f32) {
    // FNV-1a over the layer id, mixed with the frame number.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h ^= (frame as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    let pad = ANIM_PAD as u64;
    ((h % pad) as f32, ((h >> 32) % pad) as f32)
}

fn to_sk(a: Affine) -> Transform {
    Transform::from_row(a.a, a.b, a.c, a.d, a.e, a.f)
}

fn sk_color(c: Color, opacity: f32) -> tiny_skia::Color {
    let a = (c.a as f32 / 255.0) * opacity.clamp(0.0, 1.0);
    tiny_skia::Color::from_rgba(
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        a,
    )
    .unwrap_or(tiny_skia::Color::TRANSPARENT)
}

/// `c` moved toward the tint colour by the tint amount (RGB only, alpha kept).
fn tinted(c: Color, tint: Option<ResolvedTint>) -> Color {
    let Some(t) = tint else {
        return c;
    };
    let a = t.amount.clamp(0.0, 1.0);
    let mix = |from: u8, to: u8| -> u8 {
        (from as f32 + (to as f32 - from as f32) * a)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color {
        r: mix(c.r, t.color.r),
        g: mix(c.g, t.color.g),
        b: mix(c.b, t.color.b),
        a: c.a,
    }
}

fn paint(c: Color, opacity: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(sk_color(c, opacity));
    p.anti_alias = true;
    p
}

fn stroke_path(
    target: &mut Pixmap,
    path: &SkPath,
    s: &motion_core::scene::Stroke,
    opacity: f32,
    tint: Option<ResolvedTint>,
    ts: Transform,
    mask: Option<&Mask>,
) {
    if s.width <= 0.0 {
        return;
    }
    let stroke = Stroke {
        width: s.width,
        ..Default::default()
    };
    target.stroke_path(
        path,
        &paint(tinted(s.color, tint), opacity),
        &stroke,
        ts,
        mask,
    );
}

/// Draw a text layer glyph by glyph. Glyphs at rest are drawn together as one
/// path (so an all-rest layer is pixel-identical to the plain text path); the
/// others are drawn on their own with `translate(dx, dy)`, rotation and scale
/// about their own centre, and `opacity * pose.opacity`. Glyphs without a pose
/// entry are at rest.
fn draw_posed_glyphs(
    target: &mut Pixmap,
    glyphs: &[GlyphOutline],
    poses: &[GlyphPose],
    color: Color,
    opacity: f32,
    content: Affine,
    mask: Option<&Mask>,
) {
    let pose_of = |g: &GlyphOutline| poses.get(g.index).copied().unwrap_or(GlyphPose::REST);
    let ts = to_sk(content);

    let mut rest = PathBuilder::new();
    let mut any_rest = false;
    for g in glyphs {
        if pose_of(g) == GlyphPose::REST {
            rest.push_path(&g.path);
            any_rest = true;
        }
    }
    if any_rest {
        if let Some(path) = rest.finish() {
            target.fill_path(&path, &paint(color, opacity), FillRule::Winding, ts, mask);
        }
    }

    for g in glyphs {
        let pose = pose_of(g);
        if pose == GlyphPose::REST {
            continue;
        }
        let finite = [pose.dx, pose.dy, pose.scale, pose.rotation, pose.opacity]
            .iter()
            .all(|v| v.is_finite());
        let alpha = pose.opacity.clamp(0.0, 1.0) * opacity;
        if !finite || alpha <= 0.0 || pose.scale == 0.0 {
            continue;
        }
        let (cx, cy) = g.center;
        let local = Affine::translate(cx + pose.dx, cy + pose.dy)
            .then_apply(Affine::rotate_degrees(pose.rotation))
            .then_apply(Affine::scale(pose.scale, pose.scale))
            .then_apply(Affine::translate(-cx, -cy));
        target.fill_path(
            &g.path,
            &paint(color, alpha),
            FillRule::Winding,
            to_sk(content.then_apply(local)),
            mask,
        );
    }
}

/// Points of `points` (optionally closed back to the first point) covering
/// the first `fraction` of the total length, the last segment cut exactly.
/// `fraction >= 1` returns the whole path; `<= 0` (or NaN) returns empty.
fn trim_points(points: &[[f32; 2]], closed: bool, fraction: f32) -> Vec<[f32; 2]> {
    if points.is_empty() || fraction.is_nan() || fraction <= 0.0 {
        return Vec::new();
    }
    let mut path: Vec<[f32; 2]> = points.to_vec();
    if closed && points.len() > 1 {
        path.push(points[0]);
    }
    if fraction >= 1.0 || path.len() < 2 {
        return path;
    }
    let seg = |a: [f32; 2], b: [f32; 2]| f64::hypot((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
    let total: f64 = path.windows(2).map(|w| seg(w[0], w[1])).sum();
    if total <= 0.0 {
        return vec![path[0]];
    }
    let target = total * fraction as f64;
    let mut out = vec![path[0]];
    let mut walked = 0.0;
    for w in path.windows(2) {
        let len = seg(w[0], w[1]);
        if walked + len >= target {
            let t = if len > 0.0 {
                ((target - walked) / len) as f32
            } else {
                0.0
            };
            out.push([
                w[0][0] + (w[1][0] - w[0][0]) * t,
                w[0][1] + (w[1][1] - w[0][1]) * t,
            ]);
            return out;
        }
        walked += len;
        out.push(w[1]);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_polyline(
    target: &mut Pixmap,
    points: &[[f32; 2]],
    stroke: &motion_core::scene::Stroke,
    closed: bool,
    fill: Option<Color>,
    trim: Option<f32>,
    opacity: f32,
    tint: Option<ResolvedTint>,
    ts: Transform,
    mask: Option<&Mask>,
) {
    let fraction = trim.unwrap_or(1.0);
    let pts = trim_points(points, closed, fraction);
    if pts.is_empty() {
        return;
    }
    let complete = fraction >= 1.0;
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0][0], pts[0][1]);
    for p in &pts[1..] {
        pb.line_to(p[0], p[1]);
    }
    if pts.len() == 1 {
        // A lone point still draws a round dot.
        pb.line_to(pts[0][0], pts[0][1]);
    }
    let Some(open_path) = pb.finish() else {
        return;
    };
    if let Some(color) = fill {
        if pts.len() >= 3 {
            target.fill_path(
                &open_path,
                &paint(tinted(color, tint), opacity),
                FillRule::Winding,
                ts,
                mask,
            );
        }
    }
    if stroke.width <= 0.0 {
        return;
    }
    let sk_stroke = Stroke {
        width: stroke.width,
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Default::default()
    };
    let stroke_src = if closed && complete && pts.len() > 2 {
        // The closing segment is already the last point; close for clean joins.
        let mut pb = PathBuilder::new();
        pb.move_to(points[0][0], points[0][1]);
        for p in &points[1..] {
            pb.line_to(p[0], p[1]);
        }
        pb.close();
        pb.finish()
    } else {
        Some(open_path)
    };
    if let Some(path) = stroke_src {
        target.stroke_path(
            &path,
            &paint(tinted(stroke.color, tint), opacity),
            &sk_stroke,
            ts,
            mask,
        );
    }
}

fn rounded_rect(w: f32, h: f32, r: f32) -> Option<SkPath> {
    let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return Rect::from_xywh(0.0, 0.0, w, h).map(PathBuilder::from_rect);
    }
    // Cubic approximation of quarter circles.
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(w - r, 0.0);
    pb.cubic_to(w - r + k, 0.0, w, r - k, w, r);
    pb.line_to(w, h - r);
    pb.cubic_to(w, h - r + k, w - r + k, h, w - r, h);
    pb.line_to(r, h);
    pb.cubic_to(r - k, h, 0.0, h - r + k, 0.0, h - r);
    pb.line_to(0.0, r);
    pb.cubic_to(0.0, r - k, r - k, 0.0, r, 0.0);
    pb.close();
    pb.finish()
}

/// Map content of size `cw x ch` into a `w x h` box.
fn fit_transform(cw: f32, ch: f32, w: f32, h: f32, fit: Fit) -> Transform {
    if cw <= 0.0 || ch <= 0.0 {
        return Transform::identity();
    }
    let (sx, sy) = match fit {
        Fit::Fill => (w / cw, h / ch),
        Fit::Contain => {
            let s = (w / cw).min(h / ch);
            (s, s)
        }
        Fit::Cover => {
            let s = (w / cw).max(h / ch);
            (s, s)
        }
    };
    Transform::from_row(sx, 0.0, 0.0, sy, (w - cw * sx) / 2.0, (h - ch * sy) / 2.0)
}

/// Image pixels per layer pixel for an `iw x ih` image fit into `w x h`
/// (the inverse of the scale [`fit_transform`] applies; mean of both axes for Fill).
fn image_px_per_unit(img: &Pixmap, w: f32, h: f32, fit: Fit) -> f32 {
    let t = fit_transform(img.width() as f32, img.height() as f32, w, h, fit);
    let s = (t.sx.abs() + t.sy.abs()) / 2.0;
    if s.is_finite() && s > 0.0 {
        1.0 / s
    } else {
        1.0
    }
}

fn rasterize_svg(tree: &usvg::Tree, w: f32, h: f32, fit: Fit) -> Option<(Pixmap, f32)> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let k = SVG_OVERSAMPLE.min(SVG_MAX_SIDE / w.max(h)).max(0.25);
    let (pw, ph) = ((w * k).ceil() as u32, (h * k).ceil() as u32);
    let mut pm = Pixmap::new(pw, ph)?;
    let size = tree.size();
    let ts = fit_transform(size.width(), size.height(), w * k, h * k, fit);
    resvg::render(tree, ts, &mut pm.as_mut());
    Some((pm, k))
}

#[cfg(test)]
mod tests {
    use super::trim_points;

    const L: [[f32; 2]; 3] = [[0.0, 0.0], [4.0, 0.0], [4.0, 2.0]];

    #[test]
    fn trim_cuts_partial_segments_exactly() {
        assert!(trim_points(&L, false, 0.0).is_empty());
        assert!(trim_points(&L, false, f32::NAN).is_empty());
        assert_eq!(trim_points(&L, false, 1.0), L.to_vec());
        // 0.5 of length 6 = 3 -> inside the first segment.
        assert_eq!(trim_points(&L, false, 0.5), vec![[0.0, 0.0], [3.0, 0.0]]);
        // 5/6 of the length = 5 -> one unit down the second segment.
        let p = trim_points(&L, false, 5.0 / 6.0);
        assert_eq!(p.len(), 3);
        assert!((p[2][0] - 4.0).abs() < 1e-5 && (p[2][1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn closed_paths_include_the_closing_segment() {
        let full = trim_points(&L, true, 1.0);
        assert_eq!(full.len(), 4);
        assert_eq!(full[3], [0.0, 0.0]);
        // Closing segment length is sqrt(20); trimming near the end lands on it.
        let p = trim_points(&L, true, 0.99);
        assert_eq!(p.len(), 4);
        assert!(p[3][0] > 0.0 && p[3][0] < 1.0);
    }

    #[test]
    fn degenerate_paths_do_not_panic() {
        assert!(trim_points(&[], false, 0.5).is_empty());
        assert_eq!(trim_points(&[[1.0, 1.0]], false, 0.5), vec![[1.0, 1.0]]);
        assert_eq!(
            trim_points(&[[1.0, 1.0], [1.0, 1.0]], false, 0.5),
            vec![[1.0, 1.0]]
        );
    }
}
