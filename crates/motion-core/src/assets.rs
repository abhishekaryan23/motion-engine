//! Asset planning boundary (0.4): what visual support a piece needs, and the
//! contract an external image generator returns. See docs/ASSET_PIPELINE.md.
//!
//! ```text
//! CreativeIntent → MotionCompiler (grammar selection) → AssetPlanner → AssetPlan { requests }
//!     → external generator (0.5, not in this crate) → AssetManifest
//!     → MotionCompiler (composition builders place the images) → MotionScene
//! ```
//!
//! Requests are art direction, never render instructions: no coordinates,
//! crops, transforms or timing. The manifest is vendor-neutral. The Renderer
//! never learns how an image was made — it only sees ordinary `image` layers.

use serde::{Deserialize, Serialize};

use crate::style::{
    AccentRole, CameraStyle, Depth, MaterialStyle, StyleFamily, StyleProfile, TextureStyle,
};

pub const ASSET_PLAN_VERSION: &str = "0.1";
/// Manifest version written since 0.5 (adds analysis, dedup and JPEG). "0.1" is still read.
pub const ASSET_MANIFEST_VERSION: &str = "0.2";
/// The 0.4 manifest version, still accepted.
pub const ASSET_MANIFEST_VERSION_V0_1: &str = "0.1";
/// Generator-facing prompt set version (0.5).
pub const ASSET_PROMPTS_VERSION: &str = "0.1";
/// Asset cache index version (0.5).
pub const ASSET_CACHE_VERSION: &str = "0.1";

/// Where a beat's visual support comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetSource {
    /// No image: typography / data carry the beat.
    None,
    /// Engine-drawn shapes and textures (cards, halftone plates, charts).
    Procedural,
    /// A vector asset from the bundled library.
    Svg,
    /// A raster the engine asks an external generator for.
    GeneratedImage,
    /// A file the user already supplied (library PNG or manifest entry).
    UserAsset,
}

/// What an asset does in the composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRole {
    HeroSubject,
    HeroObject,
    SupportingObject,
    Environment,
    EvidenceImage,
    Portrait,
    TransitionObject,
    ForegroundOccluder,
}

/// How the image should be cut/presented (art direction only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presentation {
    /// Subject isolated on transparency (collage cutout).
    IsolatedCutout,
    /// Full-bleed rectangular photograph/plate.
    FullFrame,
    /// A flat document/print/screenshot-like artifact.
    FlatArtifact,
    /// Soft environmental plate meant to sit behind type.
    BackgroundPlate,
}

/// Where the composition needs breathing room inside the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NegativeSpace {
    None,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    Transparent,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// The chosen composition expects this image (a placeholder is drawn without it).
    Required,
    /// The composition improves with it but is complete without it.
    Optional,
}

/// One engine-independent image request. Semantic / art-direction level only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRequest {
    /// Stable id: `beat_<n>.<role>` (e.g. `beat_2.hero_subject`).
    pub id: String,
    /// 1-based beat this image serves.
    pub beat: usize,
    pub source: AssetSource,
    pub role: AssetRole,
    /// What to depict, from the intent's own words (never invented facts).
    pub subject: String,
    pub presentation: Presentation,
    pub negative_space: NegativeSpace,
    pub background: Background,
    pub priority: Priority,
    /// Composition grammar that will place it (e.g. `type_image_interlock`).
    pub composition: String,
    /// Why the planner wants it.
    pub reason: String,
    /// Library asset name when `source` is `svg` / `user_asset` from the library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_asset: Option<String>,
    /// (0.5) The beat's statement, verbatim — context for the generator, never
    /// invented. Set on `generated_image` requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// (0.5) Identity of the depicted entity across beats (e.g. `office_worker`).
    /// Requests with the same key are served by ONE generated image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuity_key: Option<String>,
}

/// The planner's decision for one beat — including "no image needed".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeatAssetDecision {
    pub beat: usize,
    pub composition: String,
    /// Strongest source any request of this beat uses (`none` when no request).
    pub source: AssetSource,
    pub reason: String,
    /// Ids of this beat's requests (subset of `AssetPlan.requests`).
    pub requests: Vec<String>,
}

/// Art direction shared by every generated image of a piece, derived from the
/// StyleProfile (never from CreativeIntent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetStyleProfile {
    pub medium: String,
    pub realism: String,
    pub lighting: String,
    pub contrast: String,
    pub palette_tendency: String,
    pub edge_treatment: String,
    pub shadow_treatment: String,
    pub camera_feel: String,
    pub background_behavior: String,
}

impl AssetStyleProfile {
    /// Deterministic mapping from the project style (see docs/ASSET_PIPELINE.md).
    pub fn from_style(style: &StyleProfile) -> Self {
        // (0.6) Describe the resolved design system, not the raw fields.
        Self::from_resolved(&crate::compiler::taste::resolve(style))
    }

    /// (0.7) Describe a resolved design system (which may carry reference
    /// principles). A reference's image-treatment bias restates the medium,
    /// contrast and edge so generated images arrive already in character.
    pub fn from_resolved(resolved: &crate::compiler::taste::ResolvedStyleProfile) -> Self {
        let mut p = Self::from_resolved_base(resolved);
        use crate::compiler::taste::{tone_image_treatment, ImageTreatmentBias as B};
        if resolved.image_treatment == tone_image_treatment(resolved.tone) {
            return p;
        }
        let (medium, contrast, edge) = match resolved.image_treatment {
            B::Classic => return p,
            B::Natural => ("clean editorial photography", None, None),
            B::Monochrome => (
                "black-and-white documentary photography",
                Some("high contrast monochrome"),
                None,
            ),
            B::Duotone => (
                "graphic photography suited to a two-ink duotone",
                Some("strong shape contrast"),
                None,
            ),
            B::Muted => (
                "desaturated documentary photography",
                Some("low contrast, lifted blacks"),
                None,
            ),
            B::PaperCutout => (
                "editorial photo collage, printed on paper",
                None,
                Some("hand-cut paper edge"),
            ),
            B::PrintCutout => (
                "bold printed cut-out photography",
                Some("high contrast, flat print color"),
                Some("clean cutout edge"),
            ),
        };
        p.medium = medium.into();
        if let Some(c) = contrast {
            p.contrast = c.into();
        }
        if let Some(e) = edge {
            p.edge_treatment = e.into();
        }
        p
    }

    fn from_resolved_base(resolved: &crate::compiler::taste::ResolvedStyleProfile) -> Self {
        let style = &resolved.effective;
        let classic = resolved.tone == crate::compiler::taste::ResolvedTone::Classic;
        let collage = style.family == StyleFamily::EditorialCollage;
        let paper = style.material == MaterialStyle::Paper;
        AssetStyleProfile {
            medium: if collage {
                "editorial photo collage, printed on paper"
            } else {
                "clean editorial photography"
            }
            .into(),
            realism: "photographic, unretouched, documentary".into(),
            lighting: match style.camera_style {
                CameraStyle::Static => "soft even daylight",
                CameraStyle::SlowPush => "soft directional window light",
                CameraStyle::Drift => "natural side light with gentle falloff",
            }
            .into(),
            contrast: match style.texture_style {
                TextureStyle::HeavyPrint => "high contrast, crushed blacks",
                TextureStyle::SubtlePrint => "medium contrast",
                TextureStyle::None => "low-medium contrast",
            }
            .into(),
            palette_tendency: if classic {
                format!(
                    "muted neutrals with a {} accent",
                    match style.accent_role {
                        AccentRole::SignalRed => "signal red",
                        AccentRole::Cobalt => "cobalt blue",
                        AccentRole::Acid => "acid yellow-green",
                    }
                )
            } else {
                use crate::compiler::taste::PaletteFamily as F;
                match resolved.palette.family {
                    F::WarmPaper => "warm muted neutrals, soft paper tones",
                    F::CoolPaper => "cool grey neutrals, clean daylight tones",
                    F::DarkWarm => "low-key warm tones, deep shadows, amber highlights",
                    F::DarkCool => "low-key cool tones, deep shadows, cyan-blue highlights",
                    F::PrintBright => "bright saturated print colors, clean flat light",
                    F::PrintDark => "saturated colors against deep shadows",
                }
                .to_string()
            },
            edge_treatment: if collage && paper {
                "hand-cut paper edge"
            } else {
                "clean cutout edge"
            }
            .into(),
            shadow_treatment: if style.depth == Depth::Layered {
                "soft contact shadow, slight lift off the page"
            } else {
                "no cast shadow"
            }
            .into(),
            camera_feel: "35-50mm, eye level, subject centered in its own frame".into(),
            background_behavior: "isolated subject on transparency unless a plate is requested"
                .into(),
        }
    }
}

/// Everything a piece needs from outside the engine. Deterministic output of
/// `compiler::plan_assets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetPlan {
    pub version: String,
    pub style: AssetStyleProfile,
    /// One decision per beat, in beat order.
    pub beats: Vec<BeatAssetDecision>,
    /// Requests in beat order, then role order.
    pub requests: Vec<AssetRequest>,
}

/// Normalized point/box inside an image (fractions of width/height, 0..=1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormPoint {
    pub x: f32,
    pub y: f32,
}

/// One delivered image. Returned by any generator (or written by hand), or by
/// `ingest-assets` (0.5), which fills the analysis fields.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    /// The `AssetRequest.id` this fulfils (for ingested assets: the spec id,
    /// i.e. the first request it serves).
    pub id: String,
    /// PNG or JPEG path, relative to the manifest file.
    pub path: String,
    pub width: u32,
    pub height: u32,
    /// The image has a meaningful alpha channel (cutout).
    pub alpha: bool,
    /// Region that must stay visible (never cropped or covered by type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_bounds: Option<NormBox>,
    /// Visual center of the subject (default: image center).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_anchor: Option<NormPoint>,
    /// Face center for portraits, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_anchor: Option<NormPoint>,
    /// (0.5) Face box, when an external generator/analyzer supplies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_bounds: Option<NormBox>,
    /// (0.5) Head box, when an external generator/analyzer supplies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_bounds: Option<NormBox>,
    /// (0.5) Every request id this image serves (dedup / continuity). Empty = just `id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub serves: Vec<String>,
    /// (0.5) `AssetPromptSpec.fingerprint` this image was generated for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// (0.5) `AssetPromptSpec.continuity_key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuity_key: Option<String>,
    /// (0.5) Content hash of the file bytes ([`content_hash`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// (0.5) Deterministic geometry measured once at ingestion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AssetAnalysis>,
    /// (0.10 Q) `path` is an engine-keyed cutout: the delivered image was
    /// opaque on a flat ground and was keyed into alpha at ingestion
    /// (`motion_render::autokey`), cropped to the subject.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keyed: bool,
    /// Free-form generator metadata (vendor, model, prompt, seed ...). Never read by the engine.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub generator: serde_json::Map<String, serde_json::Value>,
}

impl ManifestEntry {
    /// Whether this entry fulfils request `id`.
    pub fn serves_request(&self, id: &str) -> bool {
        self.id == id || self.serves.iter().any(|s| s == id)
    }

    /// Best available head region: supplied `head_bounds`, else supplied
    /// `face_bounds` grown to a head, else the alpha estimate.
    pub fn head_region(&self) -> Option<NormBox> {
        self.head_bounds
            .or_else(|| {
                self.face_bounds.map(|f| {
                    let (gx, gy) = (0.25 * f.width, 0.3 * f.height);
                    NormBox {
                        x: (f.x - gx).max(0.0),
                        y: (f.y - gy).max(0.0),
                        width: (f.width + 2.0 * gx).min(1.0),
                        height: (f.height + 1.6 * gy).min(1.0),
                    }
                })
            })
            .or_else(|| self.analysis.as_ref().and_then(|a| a.head_estimate))
    }
}

/// A request that has no usable image after ingestion (0.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissingAsset {
    /// Request id.
    pub id: String,
    pub priority: Priority,
    /// Why (not delivered, failed to decode, failed a hard check ...).
    pub reason: String,
}

/// What an external generator returns: delivered images keyed by request id.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetManifest {
    pub version: String,
    pub assets: Vec<ManifestEntry>,
    /// (0.5) Requests generation was attempted for but that have no usable
    /// image. A `required` entry here makes compilation fail; `optional`
    /// ones fall back to the image-free composition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<MissingAsset>,
}

impl AssetManifest {
    pub fn empty() -> Self {
        AssetManifest {
            version: ASSET_MANIFEST_VERSION.into(),
            assets: Vec::new(),
            missing: Vec::new(),
        }
    }

    /// The entry serving request `id` (its own id or any of `serves`).
    pub fn get(&self, id: &str) -> Option<&ManifestEntry> {
        self.assets
            .iter()
            .find(|a| a.id == id)
            .or_else(|| self.assets.iter().find(|a| a.serves_request(id)))
    }

    /// Required requests without a usable image.
    pub fn missing_required(&self) -> Vec<&MissingAsset> {
        self.missing
            .iter()
            .filter(|m| m.priority == Priority::Required)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// 0.5 — Generator-facing prompts (docs/GENERATED_ASSET_PROTOCOL.md)
// ---------------------------------------------------------------------------

/// How much of the subject the image shows (art direction; derived from role
/// and presentation, never from coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framing {
    /// A whole person, head to feet (or seated, whole).
    FullFigure,
    /// A person from the head to about the waist.
    HalfFigure,
    /// Head and shoulders.
    HeadAndShoulders,
    /// A single object, whole.
    Object,
    /// A place / environment, no dominant subject.
    Scene,
    /// A flat document-like artifact, straight on.
    Artifact,
}

/// Composition constraints *inside* the generated image. Never canvas
/// placement: the engine places the image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptComposition {
    pub framing: Framing,
    /// Empty room inside the image (only for full-frame / background plates;
    /// always `none` for cutouts — the engine makes negative space by placement).
    pub negative_space: NegativeSpace,
    /// The subject must not be cut by the image edges (cutouts: whole subject,
    /// with a margin; half figures may be cut at the bottom only).
    pub subject_whole: bool,
    /// The head must be fully inside the frame with headroom (people only).
    pub head_inside_frame: bool,
}

/// What file the engine expects back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptOutput {
    /// File stem the generator must write: `<stem>.png` (or `.jpg`/`.jpeg`
    /// when `alpha_required` is false) in the delivery directory. Optional
    /// sidecar `<stem>.json` = [`GeneratedSidecar`].
    pub file_stem: String,
    /// A real alpha channel is required (transparent cutout ⇒ PNG).
    pub alpha_required: bool,
    /// Minimum pixel size of the short side.
    pub min_short_side: u32,
    /// Preferred aspect, width:height (e.g. `2:3`).
    pub aspect: String,
}

/// One generator request: deterministic art direction produced by the asset
/// pipeline from an `AssetPlan`. Weak models never author it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetPromptSpec {
    /// Spec id = the first request id it serves (e.g. `beat_1.hero_subject`).
    pub id: String,
    /// Stable identity of the art direction ([`AssetPromptSpec::compute_fingerprint`]).
    pub fingerprint: String,
    /// The depicted entity (shared by every scene that shows it).
    pub continuity_key: String,
    /// Every request id this one image serves, in plan order (includes `id`).
    pub serves: Vec<String>,
    /// Role of the first served request.
    pub role: AssetRole,
    /// Required if ANY served request is required.
    pub priority: Priority,
    /// What to depict (the intent's words).
    pub subject: String,
    /// The beat statement, verbatim, as context (may be empty).
    pub context: String,
    pub presentation: Presentation,
    pub background: Background,
    pub style: AssetStyleProfile,
    pub composition: PromptComposition,
    /// Things the image must not contain (fixed list per presentation).
    pub avoid: Vec<String>,
    pub output: PromptOutput,
    /// Deterministic plain-language prompt assembled from the fields above,
    /// for generators that take one string. Not part of the fingerprint.
    pub prompt: String,
}

/// The file `motion-engine asset-prompts` writes and external generators read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetPromptSet {
    pub version: String,
    /// Specs in plan order of their first served request.
    pub specs: Vec<AssetPromptSpec>,
}

/// Optional metadata a generator writes next to an image (`<stem>.json`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedSidecar {
    /// Must equal the spec fingerprint when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_bounds: Option<NormBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_bounds: Option<NormBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_anchor: Option<NormPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_anchor: Option<NormPoint>,
    /// Free-form (vendor, model, seed ...). Copied to `ManifestEntry.generator`.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub generator: serde_json::Map<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// 0.5 — Analysis (docs/ASSET_ANALYSIS.md). Measured once at ingestion by
// motion-render; stored normalized (0..1 of image width/height).
// ---------------------------------------------------------------------------

/// Which image edges the subject touches (alpha > threshold within 1 % of the edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeContact {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

/// Coarse subject occupancy: `rows.len()` rows of `cols` hex digits each
/// (`0` empty … `f` full), top row first. 16×16 at ingestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccupancyGrid {
    pub cols: u32,
    pub rows: Vec<String>,
}

impl OccupancyGrid {
    /// Coverage of cell (col, row) in 0..=1 (0 when out of range or malformed).
    pub fn cell(&self, col: u32, row: u32) -> f32 {
        self.rows
            .get(row as usize)
            .and_then(|r| r.as_bytes().get(col as usize))
            .and_then(|b| (*b as char).to_digit(16))
            .map_or(0.0, |d| d as f32 / 15.0)
    }

    pub fn row_count(&self) -> u32 {
        self.rows.len() as u32
    }

    /// Mean coverage of the cells overlapping normalized box `b`, weighted by overlap area.
    pub fn coverage_in(&self, b: NormBox) -> f32 {
        let (cols, rows) = (self.cols.max(1) as f32, self.row_count().max(1) as f32);
        let (x0, y0, x1, y1) = (b.x, b.y, b.x + b.width, b.y + b.height);
        let (mut sum, mut area) = (0.0f32, 0.0f32);
        for r in 0..self.row_count() {
            let (cy0, cy1) = (r as f32 / rows, (r + 1) as f32 / rows);
            let oy = (y1.min(cy1) - y0.max(cy0)).max(0.0);
            if oy <= 0.0 {
                continue;
            }
            for c in 0..self.cols {
                let (cx0, cx1) = (c as f32 / cols, (c + 1) as f32 / cols);
                let ox = (x1.min(cx1) - x0.max(cx0)).max(0.0);
                if ox > 0.0 {
                    sum += self.cell(c, r) * ox * oy;
                    area += ox * oy;
                }
            }
        }
        if area > 0.0 {
            sum / area
        } else {
            0.0
        }
    }
}

/// Named candidate zones for type, relative to the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionName {
    LeftUpper,
    RightUpper,
    LeftMiddle,
    RightMiddle,
    LowerLeft,
    LowerRight,
    Top,
    Bottom,
}

/// A rectangle of the image that is (nearly) free of subject.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeRegion {
    pub name: RegionName,
    pub rect: NormBox,
    /// Mean occupancy inside `rect` (0 = empty).
    pub occupancy: f32,
    /// Deterministic usefulness score (0..1; area × emptiness). Higher is better.
    pub score: f32,
}

/// Geometry measured at ingestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetAnalysis {
    /// Non-transparent bounding box (whole image for opaque images).
    pub subject_bounds: NormBox,
    pub edges: EdgeContact,
    /// Fraction of pixels that belong to the subject (alpha ≥ threshold).
    pub coverage: f32,
    pub occupancy: OccupancyGrid,
    /// Candidate type regions, best first. Empty for opaque images.
    pub safe_regions: Vec<SafeRegion>,
    /// Head region estimated from the alpha silhouette (people cutouts only;
    /// `None` when the silhouette has no head-like top).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_estimate: Option<NormBox>,
    /// (0.9) Luminance-only artwork (line art, glyphs, greyscale): mean chroma
    /// over opaque pixels below the ingest threshold. The compiler recolors
    /// monochrome alpha cutouts to the palette ink so they read on any ground.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub monochrome: bool,
    /// (0.10 Q) Mean sRGB colour of the subject pixels (alpha-weighted;
    /// every pixel for an opaque image), rounded to 8 bits. The compiler and
    /// layout QA judge subject/ground contrast from it
    /// (`taste_rules::ASSET_GROUND_MIN_CONTRAST`); `None` in manifests written
    /// before 0.10 (no contrast treatment is decided then).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_color: Option<[u8; 3]>,
}

// ---------------------------------------------------------------------------
// 0.5 — Cache (fingerprint → file → analysis). Pure in-memory index; the CLI
// reads/writes it as `<cache dir>/index.json`.
// ---------------------------------------------------------------------------

/// One cached generated image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheEntry {
    pub fingerprint: String,
    pub continuity_key: String,
    /// File name inside the cache directory (`<fingerprint without "fp1-">.<ext>`).
    pub file: String,
    pub content_hash: String,
    pub width: u32,
    pub height: u32,
    pub alpha: bool,
    pub analysis: AssetAnalysis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_bounds: Option<NormBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_bounds: Option<NormBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_anchor: Option<NormPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_anchor: Option<NormPoint>,
    /// (0.10 Q) The cached file is an engine-keyed cutout ([`ManifestEntry::keyed`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keyed: bool,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub generator: serde_json::Map<String, serde_json::Value>,
}

/// The cache index (`index.json`). Entries sorted by fingerprint.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetCacheIndex {
    pub version: String,
    pub entries: Vec<CacheEntry>,
}

/// Why a cache insert was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheConflict {
    /// The fingerprint is cached with different bytes and replacement was not requested.
    #[error("asset {fingerprint} is already cached with different content ({cached} ≠ {incoming}); pass --replace to regenerate it explicitly")]
    ContentChanged {
        fingerprint: String,
        cached: String,
        incoming: String,
    },
}

/// Result of a successful insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheInsert {
    /// New fingerprint.
    Added,
    /// Same fingerprint, same content: nothing changed.
    Unchanged,
    /// Same fingerprint, new content, `replace = true`.
    Replaced,
}

/// Stable 64-bit FNV-1a of `bytes`, as 16 lowercase hex digits.
pub fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Content hash of a file's bytes: `fnv1a64:<16 hex>`.
pub fn content_hash(bytes: &[u8]) -> String {
    format!("fnv1a64:{}", fnv1a64_hex(bytes))
}

fn snake<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Lowercase, whitespace-collapsed text (fingerprint normalization).
pub fn normalize_text(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl AssetPromptSpec {
    /// Canonical art-direction string the fingerprint hashes (FROZEN, v1).
    /// Includes what changes the pixels a generator should make; excludes
    /// bookkeeping (id, serves, role, priority, continuity key, context,
    /// prompt, file stem) so the same depicted entity is reused across scenes.
    pub fn canonical_art_direction(&self) -> String {
        let st = &self.style;
        let c = &self.composition;
        let o = &self.output;
        [
            "fp1".to_string(),
            format!("subject={}", normalize_text(&self.subject)),
            format!("presentation={}", snake(&self.presentation)),
            format!("background={}", snake(&self.background)),
            format!("framing={}", snake(&c.framing)),
            format!("negative_space={}", snake(&c.negative_space)),
            format!("subject_whole={}", c.subject_whole),
            format!("head_inside_frame={}", c.head_inside_frame),
            format!("style.medium={}", normalize_text(&st.medium)),
            format!("style.realism={}", normalize_text(&st.realism)),
            format!("style.lighting={}", normalize_text(&st.lighting)),
            format!("style.contrast={}", normalize_text(&st.contrast)),
            format!(
                "style.palette_tendency={}",
                normalize_text(&st.palette_tendency)
            ),
            format!(
                "style.edge_treatment={}",
                normalize_text(&st.edge_treatment)
            ),
            format!(
                "style.shadow_treatment={}",
                normalize_text(&st.shadow_treatment)
            ),
            format!("style.camera_feel={}", normalize_text(&st.camera_feel)),
            format!(
                "style.background_behavior={}",
                normalize_text(&st.background_behavior)
            ),
            format!("alpha_required={}", o.alpha_required),
            format!("min_short_side={}", o.min_short_side),
            format!("aspect={}", o.aspect),
            format!(
                "avoid={}",
                self.avoid
                    .iter()
                    .map(|a| normalize_text(a))
                    .collect::<Vec<_>>()
                    .join("|")
            ),
        ]
        .join("\n")
    }

    /// `fp1-<16 hex>` = FNV-1a-64 of [`Self::canonical_art_direction`].
    pub fn compute_fingerprint(&self) -> String {
        format!(
            "fp1-{}",
            fnv1a64_hex(self.canonical_art_direction().as_bytes())
        )
    }
}

impl AssetPromptSet {
    pub fn from_json(s: &str) -> Result<Self, AssetError> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The spec serving request `id`.
    pub fn for_request(&self, id: &str) -> Option<&AssetPromptSpec> {
        self.specs
            .iter()
            .find(|s| s.id == id || s.serves.iter().any(|r| r == id))
    }
}

impl AssetCacheIndex {
    pub fn new() -> Self {
        AssetCacheIndex {
            version: ASSET_CACHE_VERSION.into(),
            entries: Vec::new(),
        }
    }

    pub fn from_json(s: &str) -> Result<Self, AssetError> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The cached image for `fingerprint`.
    pub fn lookup(&self, fingerprint: &str) -> Option<&CacheEntry> {
        crate::asset_prompts::cache_lookup(self, fingerprint)
    }

    /// Insert (or confirm) an entry. Same fingerprint + same content_hash →
    /// `Unchanged`; different content → `ContentChanged` unless `replace`
    /// (then `Replaced`). Keeps `entries` sorted by fingerprint.
    pub fn insert(
        &mut self,
        entry: CacheEntry,
        replace: bool,
    ) -> Result<CacheInsert, CacheConflict> {
        crate::asset_prompts::cache_insert(self, entry, replace)
    }
}

/// Raster formats the engine ingests and renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

/// Format by file extension (case-insensitive): `.png`, `.jpg`, `.jpeg`.
pub fn image_format_of(path: &str) -> Option<ImageFormat> {
    let p = path.to_ascii_lowercase();
    if p.ends_with(".png") {
        Some(ImageFormat::Png)
    } else if p.ends_with(".jpg") || p.ends_with(".jpeg") {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

/// Errors reading an asset manifest.
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("invalid asset manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl AssetPlan {
    /// Pretty-printed JSON (deterministic: field order follows the struct).
    pub fn to_json_pretty(&self) -> String {
        // Serializing plain strings/enums cannot fail; an empty string is the
        // unreachable fallback rather than a panic.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

impl AssetManifest {
    /// Parse a manifest document (strict: unknown fields are rejected).
    pub fn from_json(s: &str) -> Result<Self, AssetError> {
        Ok(serde_json::from_str(s)?)
    }

    /// Structural checks: version, unique ids, PNG paths, positive sizes and
    /// normalized (0..=1) boxes and points. Returns every problem found.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        const EPS: f32 = 1e-6;
        let mut errs = Vec::new();
        if self.version != ASSET_MANIFEST_VERSION && self.version != ASSET_MANIFEST_VERSION_V0_1 {
            errs.push(format!(
                "unsupported manifest version '{}' (expected \"{ASSET_MANIFEST_VERSION}\" or \"{ASSET_MANIFEST_VERSION_V0_1}\")",
                self.version
            ));
        }
        let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        let mut seen = std::collections::BTreeSet::new();
        let mut seen_serves = std::collections::BTreeSet::new();
        for (i, a) in self.assets.iter().enumerate() {
            let who = if a.id.is_empty() {
                format!("assets[{i}]")
            } else {
                format!("asset '{}'", a.id)
            };
            if a.id.trim().is_empty() {
                errs.push(format!("{who}: id must not be empty"));
            } else if !seen.insert(a.id.as_str()) {
                errs.push(format!("{who}: duplicate id"));
            }
            if a.path.trim().is_empty() {
                errs.push(format!("{who}: path must not be empty"));
            } else if image_format_of(&a.path).is_none() {
                errs.push(format!(
                    "{who}: path '{}' must be a .png, .jpg or .jpeg file",
                    a.path
                ));
            } else if a.alpha && image_format_of(&a.path) == Some(ImageFormat::Jpeg) {
                errs.push(format!("{who}: a JPEG cannot carry alpha"));
            }
            if a.width == 0 || a.height == 0 {
                errs.push(format!("{who}: width and height must be greater than 0"));
            }
            for (name, b) in [
                ("safe_bounds", a.safe_bounds),
                ("face_bounds", a.face_bounds),
                ("head_bounds", a.head_bounds),
            ] {
                let Some(b) = b else { continue };
                let ok = unit(b.x)
                    && unit(b.y)
                    && b.width.is_finite()
                    && b.height.is_finite()
                    && b.width > 0.0
                    && b.height > 0.0
                    && b.x + b.width <= 1.0 + EPS
                    && b.y + b.height <= 1.0 + EPS;
                if !ok {
                    errs.push(format!(
                        "{who}: {name} must be a box inside 0..1 with positive width and height"
                    ));
                }
            }
            for r in &a.serves {
                if r != &a.id && !seen_serves.insert(r.as_str()) {
                    errs.push(format!(
                        "{who}: request '{r}' is served by more than one asset"
                    ));
                }
            }
            for (name, p) in [
                ("subject_anchor", a.subject_anchor),
                ("face_anchor", a.face_anchor),
            ] {
                if let Some(p) = p {
                    if !(unit(p.x) && unit(p.y)) {
                        errs.push(format!("{who}: {name} must be inside 0..1"));
                    }
                }
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }
}
