//! (0.9) Emotion typography: the curated registry (`assets/fonts/registry.json`)
//! maps a closed emotion vocabulary to curated face options, and the operator's
//! exploration level / emotion override choose among them. Weak models never
//! pick fonts; CreativeIntent and StyleProfile are unchanged.
//!
//! Contract (frozen 0.9 Phase 1):
//! * Level 0 without an emotion override keeps the 0.8 tone pairing
//!   (`ResolvedStyleProfile.typography`) byte-for-byte; the resolved emotion is
//!   still reported.
//! * `--emotion E` at level 0 = option 1 of E. Level 1 = options 1..=3 of the
//!   resolved emotion; level 2 adds the options of adjacent emotions; level 3
//!   adds every emotion listed for the tone. Choice = hash(seed, level,
//!   story_key, "typography") over the candidate list in registry order.
//! * Classic tone at level 0 is GroteskSerif; at level >= 1 its candidates are
//!   the legacy pairing plus the options of `tone_emotions.classic`.
//! * Reference-derived pairings keep their legacy pairing at every level
//!   (reference fidelity). (0.24) An explicit `typography_style` (and the
//!   Classic tone's GroteskSerif) is a category constraint at level >= 1: only
//!   options whose face classes fit the pairing are candidates (widening to
//!   the tone's emotions, then the legacy pairing, when none fit).
//! * A candidate whose face files are missing under the library root, or whose
//!   body/number faces lack digits, is skipped for the next option of the same
//!   emotion, then option 1, then the legacy pairing; the skip is recorded in
//!   `fallback_from`. A restricted face (licence forbids redistribution,
//!   `restricted_fallbacks`) is never missing while its open fallback file
//!   exists: the choice and the FontSet keep the original face and the
//!   loaders read the fallback ([`resolve_font_path`]).
//! * Every emotion option loads its role faces + `fallback_face` +
//!   `always_loaded` (glyph fallback for ₹ € digits).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::taste::{
    Genre, ResolvedStyleProfile, ResolvedTemperature, ResolvedTone, TemperamentKind,
    TypographyPairing,
};
use super::theme::{FontFace, FontSet};
use crate::scene::FontRole;

/// Closed emotion vocabulary (0.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Emotion {
    Trust,
    Warmth,
    Calm,
    Precision,
    Joy,
    Energy,
    Drama,
    Luxury,
    Urgency,
    Handmade,
    PlayfulRetro,
}

impl Emotion {
    pub const ALL: [Emotion; 11] = [
        Emotion::Trust,
        Emotion::Warmth,
        Emotion::Calm,
        Emotion::Precision,
        Emotion::Joy,
        Emotion::Energy,
        Emotion::Drama,
        Emotion::Luxury,
        Emotion::Urgency,
        Emotion::Handmade,
        Emotion::PlayfulRetro,
    ];
}

/// What the typography director chose (recorded in the project theme and in
/// `resolve-style --json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypographyChoice {
    /// Emotion resolved from tone × temperament × temperature (or the override).
    pub emotion: Emotion,
    /// Registry option id (e.g. `"trust.2"`), or `None` when the 0.8 legacy
    /// pairing is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    /// The legacy pairing in use when `option` is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<TypographyPairing>,
    /// Option ids skipped because a face was unavailable, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_from: Vec<String>,
}

/// Inputs the typography director reads besides the resolved taste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypographyRequest {
    /// Exploration level 0..=3.
    pub exploration: u8,
    /// Operator exploration seed.
    pub seed: u64,
    /// Story key ([`super::taste::story_key`]).
    pub story_key: u64,
    /// Operator `--emotion` override.
    pub emotion: Option<Emotion>,
}

/// Parse a snake_case emotion name (`"playful_retro"`).
pub fn parse_emotion(name: &str) -> Option<Emotion> {
    serde_json::from_value(serde_json::Value::String(name.to_string())).ok()
}

/// The snake_case name of an emotion.
pub fn emotion_name(e: Emotion) -> &'static str {
    match e {
        Emotion::Trust => "trust",
        Emotion::Warmth => "warmth",
        Emotion::Calm => "calm",
        Emotion::Precision => "precision",
        Emotion::Joy => "joy",
        Emotion::Energy => "energy",
        Emotion::Drama => "drama",
        Emotion::Luxury => "luxury",
        Emotion::Urgency => "urgency",
        Emotion::Handmade => "handmade",
        Emotion::PlayfulRetro => "playful_retro",
    }
}

fn tone_name(t: ResolvedTone) -> &'static str {
    match t {
        ResolvedTone::Classic => "classic",
        ResolvedTone::Editorial => "editorial",
        ResolvedTone::Technical => "technical",
        ResolvedTone::Playful => "playful",
    }
}

fn default_pairing(t: ResolvedTone) -> TypographyPairing {
    match t {
        ResolvedTone::Classic => TypographyPairing::GroteskSerif,
        ResolvedTone::Editorial => TypographyPairing::HumanistSerif,
        ResolvedTone::Technical => TypographyPairing::PrecisionGrotesk,
        ResolvedTone::Playful => TypographyPairing::FriendlyGeometric,
    }
}

/// snake_case role name as used in the registry.
pub fn role_name(r: FontRole) -> &'static str {
    match r {
        FontRole::Display => "display",
        FontRole::DisplayCondensed => "display_condensed",
        FontRole::SerifEmotional => "serif_emotional",
        FontRole::Body => "body",
        FontRole::Mono => "mono",
        FontRole::Number => "number",
    }
}

fn parse_role(name: &str) -> Option<FontRole> {
    FontRole::ALL.into_iter().find(|r| role_name(*r) == name)
}

// ---------------------------------------------------------------------------
// Registry data
// ---------------------------------------------------------------------------

/// Glyph coverage declared by the registry for a face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceCoverage {
    #[serde(default)]
    pub latin: bool,
    #[serde(default)]
    pub digits: bool,
    #[serde(default)]
    pub inr: bool,
    #[serde(default)]
    pub eur: bool,
}

#[derive(Deserialize, Default)]
struct RawFace {
    id: String,
    file: String,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    weight: u16,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    coverage: Option<FaceCoverage>,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    contrast: Option<Contrast>,
    #[serde(default)]
    width: Option<Width>,
    #[serde(default)]
    x_height: Option<f64>,
    #[serde(default)]
    tabular_digits: Option<bool>,
}

/// Stroke contrast of a face (registry metadata, hand-assigned).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contrast {
    Low,
    Medium,
    High,
}

/// Set width of a face (registry metadata, hand-assigned).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Width {
    Condensed,
    Normal,
    Extended,
}

/// Motion metadata of a face. `x_height` (x-glyph height over H-glyph height)
/// and `tabular_digits` (all 0-9 advances within 1 %) are measured from the
/// font file by `tests/motion_type.rs`; they are `None` for faces not bundled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FaceMeta {
    pub weight: u16,
    pub contrast: Contrast,
    pub width: Width,
    pub x_height: Option<f64>,
    pub tabular_digits: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RawUseFor {
    #[serde(default)]
    temperaments: Vec<String>,
}

#[derive(Deserialize)]
struct RawPairing {
    id: String,
    emotion: String,
    #[serde(default)]
    roles: BTreeMap<String, String>,
    #[serde(default)]
    use_for: RawUseFor,
}

#[derive(Deserialize)]
struct RawOption {
    id: String,
    #[serde(default)]
    roles: BTreeMap<String, String>,
    #[serde(default)]
    legacy_pairing: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawRegistry {
    #[serde(default)]
    fallback_face: String,
    #[serde(default)]
    always_loaded: Vec<String>,
    #[serde(default)]
    faces: Vec<RawFace>,
    #[serde(default)]
    legacy_pairings: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    emotions: BTreeMap<String, Vec<RawOption>>,
    #[serde(default)]
    adjacency: Vec<Vec<String>>,
    #[serde(default)]
    tone_emotions: BTreeMap<String, Vec<String>>,
    /// Face id -> face id used when the first face's file is missing.
    #[serde(default)]
    substitutes: BTreeMap<String, String>,
    /// Restricted face file -> open fallback file in the same folder, read
    /// in its place when the restricted file is absent (see
    /// [`resolve_font_path`]).
    #[serde(default)]
    restricted_fallbacks: BTreeMap<String, String>,
    /// (0.24) Pairing library v2.
    #[serde(default)]
    pairings: Vec<RawPairing>,
    /// (0.24) genre -> tone -> pairing ids.
    #[serde(default)]
    usage: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

struct FaceInfo {
    face: FontFace,
    family: Option<String>,
    coverage: Option<FaceCoverage>,
    /// grotesk | condensed | geometric | serif | slab | sans | mono | hand.
    class: Option<String>,
    meta: FaceMeta,
}

struct RegOption {
    id: String,
    emotion: Emotion,
    /// Role -> index into `Registry::faces` (only roles that resolved).
    roles: BTreeMap<FontRole, usize>,
    /// Structural problems (independent of the library root).
    problems: Vec<String>,
}

struct Registry {
    faces: Vec<FaceInfo>,
    fallback: Option<usize>,
    always: Vec<usize>,
    /// Emotions in `Emotion::ALL` order, options in registry order.
    options: Vec<RegOption>,
    adjacency: Vec<(Emotion, Emotion)>,
    tone_emotions: BTreeMap<String, Vec<Emotion>>,
    /// Face index -> substitute face index (used when the file is missing).
    substitutes: BTreeMap<usize, usize>,
    /// Restricted face file name -> open fallback file name (same folder).
    restricted: BTreeMap<String, String>,
    /// (0.24) Pairing library v2 (`emotion` is the closest emotion, for reports).
    pairings: Vec<RegOption>,
    /// Pairing id -> temperaments it suits (empty = all).
    pairing_temperaments: BTreeMap<String, Vec<String>>,
    usage: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// Fatal data errors (parse + structure).
    errors: Vec<String>,
}

/// Body text needs an x-height of at least this fraction of the cap height.
const MIN_BODY_X_HEIGHT: f64 = 0.66;

fn genre_name(g: Genre) -> &'static str {
    match g {
        Genre::Standard => "standard",
        Genre::Street => "street",
        Genre::Documentary => "documentary",
        Genre::Hype => "hype",
        Genre::Studio => "studio",
        Genre::Cinematic => "cinematic",
    }
}

fn temperament_name(k: TemperamentKind) -> &'static str {
    match k {
        TemperamentKind::Restrained => "restrained",
        TemperamentKind::Editorial => "editorial",
        TemperamentKind::Precise => "precise",
        TemperamentKind::Energetic => "energetic",
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

impl Registry {
    fn parse(json: &str) -> Registry {
        let mut errors = Vec::new();
        let raw: RawRegistry = match serde_json::from_str(json) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("registry.json does not parse: {e}"));
                RawRegistry::default()
            }
        };
        let mut faces = Vec::new();
        let mut face_index: HashMap<String, usize> = HashMap::new();
        for f in &raw.faces {
            if face_index.insert(f.id.clone(), faces.len()).is_some() {
                errors.push(format!("duplicate face id {}", f.id));
            }
            faces.push(FaceInfo {
                face: FontFace {
                    asset_id: leak(&f.id),
                    path: leak(&f.file),
                    weight: f.weight,
                    italic: f.italic,
                },
                family: f.family.clone(),
                coverage: f.coverage,
                class: f.class.clone(),
                meta: FaceMeta {
                    weight: f.weight,
                    contrast: f.contrast.unwrap_or(Contrast::Medium),
                    width: f.width.unwrap_or(Width::Normal),
                    x_height: f.x_height,
                    tabular_digits: f.tabular_digits,
                },
            });
        }
        let lookup = |id: &str| face_index.get(id).copied();
        let fallback = lookup(&raw.fallback_face);
        if fallback.is_none() {
            errors.push(format!("unknown fallback_face {:?}", raw.fallback_face));
        }
        let mut substitutes = BTreeMap::new();
        for (a, b) in &raw.substitutes {
            match (lookup(a), lookup(b)) {
                (Some(i), Some(j)) => {
                    substitutes.insert(i, j);
                }
                _ => errors.push(format!("substitutes names unknown face {a} -> {b}")),
            }
        }
        let mut restricted = BTreeMap::new();
        for (from, to) in &raw.restricted_fallbacks {
            let split = |p: &str| {
                let p = Path::new(p);
                (
                    p.parent().map(Path::to_path_buf),
                    p.file_name().and_then(|n| n.to_str()).map(str::to_string),
                )
            };
            let ((from_dir, from_name), (to_dir, to_name)) = (split(from), split(to));
            if !raw.faces.iter().any(|f| &f.file == from) {
                errors.push(format!(
                    "restricted_fallbacks names unknown face file {from}"
                ));
            }
            if raw.restricted_fallbacks.contains_key(to) {
                errors.push(format!(
                    "restricted_fallbacks: fallback {to} of {from} is itself restricted"
                ));
            }
            match (from_name, to_name) {
                (Some(a), Some(b)) if from_dir == to_dir => {
                    restricted.insert(a, b);
                }
                _ => errors.push(format!(
                    "restricted_fallbacks: {from} -> {to} must be files in one folder"
                )),
            }
        }
        let mut always = Vec::new();
        for id in &raw.always_loaded {
            match lookup(id) {
                Some(i) => always.push(i),
                None => errors.push(format!("always_loaded names unknown face {id}")),
            }
        }
        for (name, roles) in &raw.legacy_pairings {
            for role in FontRole::ALL {
                match roles.get(role_name(role)) {
                    None => errors.push(format!(
                        "legacy pairing {name} is missing role {}",
                        role_name(role)
                    )),
                    Some(id) if lookup(id).is_none() => errors.push(format!(
                        "legacy pairing {name} role {} references unknown face {id}",
                        role_name(role)
                    )),
                    Some(_) => {}
                }
            }
            for key in roles.keys() {
                if parse_role(key).is_none() {
                    errors.push(format!("legacy pairing {name} has unknown role {key}"));
                }
            }
        }
        let mut options = Vec::new();
        let build = |o: &RawOption, emotion: Emotion, errors: &mut Vec<String>| -> RegOption {
            let mut problems = Vec::new();
            let mut roles = BTreeMap::new();
            for role in FontRole::ALL {
                match o.roles.get(role_name(role)) {
                    None => problems.push(format!("missing role {}", role_name(role))),
                    Some(id) => match lookup(id) {
                        None => problems.push(format!(
                            "role {} references unknown face {id}",
                            role_name(role)
                        )),
                        Some(i) => {
                            roles.insert(role, i);
                            let needs_digits = matches!(role, FontRole::Body | FontRole::Number);
                            if needs_digits && faces[i].coverage.is_some_and(|c| !c.digits) {
                                problems.push(format!(
                                    "{} face {id} has no digit coverage",
                                    role_name(role)
                                ));
                            }
                        }
                    },
                }
            }
            for key in o.roles.keys() {
                if parse_role(key).is_none() {
                    problems.push(format!("unknown role {key}"));
                }
            }
            if let Some(lp) = &o.legacy_pairing {
                if !raw.legacy_pairings.contains_key(lp) {
                    problems.push(format!("unknown legacy_pairing {lp}"));
                }
            }
            for p in &problems {
                errors.push(format!("option {}: {p}", o.id));
            }
            RegOption {
                id: o.id.clone(),
                emotion,
                roles,
                problems,
            }
        };
        for (ename, opts) in &raw.emotions {
            let Some(emotion) = parse_emotion(ename) else {
                errors.push(format!("emotions names unknown emotion {ename}"));
                continue;
            };
            for o in opts {
                options.push(build(o, emotion, &mut errors));
            }
        }
        let mut pairings = Vec::new();
        let mut pairing_temperaments = BTreeMap::new();
        for q in &raw.pairings {
            let Some(emotion) = parse_emotion(&q.emotion) else {
                errors.push(format!(
                    "pairing {} names unknown emotion {}",
                    q.id, q.emotion
                ));
                continue;
            };
            let o = RawOption {
                id: q.id.clone(),
                roles: q.roles.clone(),
                legacy_pairing: None,
            };
            pairings.push(build(&o, emotion, &mut errors));
            pairing_temperaments.insert(q.id.clone(), q.use_for.temperaments.clone());
        }
        for (genre, tones) in &raw.usage {
            for (tone, ids) in tones {
                for id in ids {
                    if !raw.pairings.iter().any(|q| &q.id == id) {
                        errors.push(format!("usage.{genre}.{tone} names unknown pairing {id}"));
                    }
                }
            }
        }
        options.sort_by_key(|o| Emotion::ALL.iter().position(|e| *e == o.emotion));
        let mut adjacency = Vec::new();
        for pair in &raw.adjacency {
            let parsed: Vec<Option<Emotion>> = pair.iter().map(|n| parse_emotion(n)).collect();
            match parsed.as_slice() {
                [Some(a), Some(b)] => adjacency.push((*a, *b)),
                _ => errors.push(format!("adjacency pair {pair:?} names an unknown emotion")),
            }
        }
        let mut tone_emotions = BTreeMap::new();
        for (tone, names) in &raw.tone_emotions {
            let mut list = Vec::new();
            for n in names {
                match parse_emotion(n) {
                    Some(e) => list.push(e),
                    None => errors.push(format!("tone_emotions.{tone} names unknown emotion {n}")),
                }
            }
            tone_emotions.insert(tone.clone(), list);
        }
        Registry {
            faces,
            fallback,
            always,
            substitutes,
            restricted,
            options,
            adjacency,
            tone_emotions,
            pairings,
            pairing_temperaments,
            usage: raw.usage,
            errors,
        }
    }

    /// Does the option satisfy the category of a legacy pairing? Judged on the
    /// registry face classes of its display / voice / body roles.
    fn matches_category(&self, o: &RegOption, pairing: TypographyPairing) -> bool {
        use TypographyPairing as P;
        let class = |r: FontRole| {
            o.roles
                .get(&r)
                .and_then(|i| self.faces[*i].class.as_deref())
        };
        let display = class(FontRole::Display);
        let voice = class(FontRole::SerifEmotional);
        let body = class(FontRole::Body);
        let sans_display = matches!(display, Some("grotesk" | "condensed" | "geometric"));
        let serif_display = matches!(display, Some("serif" | "slab"));
        match pairing {
            P::GroteskSerif => sans_display && matches!(voice, Some("serif" | "slab")),
            P::PosterBold => sans_display,
            P::SerifSans | P::HumanistSerif => serif_display && body == Some("sans"),
            P::PrecisionGrotesk => matches!(display, Some("grotesk" | "condensed")),
            P::FriendlyGeometric => matches!(display, Some("geometric" | "grotesk")),
            P::CondensedMono => matches!(display, Some("condensed" | "grotesk")),
        }
    }

    /// (0.24) Motion gate for one face in one role. `words` is the number of
    /// words set in it (`usize::MAX` = unknown / long).
    fn gate_face(&self, i: usize, role: FontRole, words: usize) -> Result<(), String> {
        let m = &self.faces[i].meta;
        if m.weight < 400 {
            return Err(format!("weight {} < 400", m.weight));
        }
        let heavy_role = matches!(
            role,
            FontRole::Display | FontRole::DisplayCondensed | FontRole::Number
        );
        if heavy_role
            && m.weight < 600
            && !(role == FontRole::Display && m.contrast == Contrast::High)
        {
            return Err(format!("weight {} < 600 for {}", m.weight, role_name(role)));
        }
        if role == FontRole::Body {
            match m.x_height {
                None => return Err("x_height not measured".to_string()),
                Some(x) if x < MIN_BODY_X_HEIGHT => {
                    return Err(format!("x_height {x} < {MIN_BODY_X_HEIGHT} for body"))
                }
                Some(_) => {}
            }
            if m.contrast == Contrast::High {
                return Err("high stroke contrast at body size".to_string());
            }
        }
        // Count lays its text out per frame at the current width (no reserved
        // final width), so figures must be tabular or the number jitters.
        if role == FontRole::Number && m.tabular_digits != Some(true) {
            return Err("digits not tabular (count jitters)".to_string());
        }
        if m.width == Width::Extended && !(role == FontRole::Display && words <= 3) {
            return Err(format!(
                "extended: display with <= 3 words only ({} words, {})",
                if words == usize::MAX {
                    "many".to_string()
                } else {
                    words.to_string()
                },
                role_name(role)
            ));
        }
        Ok(())
    }

    /// Gate an option or pairing (all roles, unknown word count).
    fn gate_option(&self, o: &RegOption) -> Result<(), Vec<String>> {
        let mut why = Vec::new();
        for (role, i) in &o.roles {
            if let Err(e) = self.gate_face(*i, *role, usize::MAX) {
                why.push(format!(
                    "{} ({}): {e}",
                    self.faces[*i].face.asset_id,
                    role_name(*role)
                ));
            }
        }
        if why.is_empty() {
            Ok(())
        } else {
            Err(why)
        }
    }

    fn gate_ok(&self, o: &RegOption) -> bool {
        self.gate_option(o).is_ok()
    }

    /// Level-1 pairing candidates for a tone / genre / temperament.
    fn pairing_candidates(&self, genre: &str, tone: &str, kind: &str) -> Vec<&RegOption> {
        let ids = self
            .usage
            .get(genre)
            .and_then(|t| t.get(tone))
            .or_else(|| self.usage.get("standard").and_then(|t| t.get(tone)));
        let all: Vec<&RegOption> = ids
            .into_iter()
            .flatten()
            .filter_map(|id| self.pairings.iter().find(|p| &p.id == id))
            .filter(|p| self.gate_ok(p))
            .collect();
        let suited: Vec<&RegOption> = all
            .iter()
            .copied()
            .filter(|p| {
                self.pairing_temperaments
                    .get(&p.id)
                    .is_none_or(|t| t.is_empty() || t.iter().any(|k| k == kind))
            })
            .collect();
        if suited.is_empty() {
            all
        } else {
            suited
        }
    }

    fn options_of(&self, e: Emotion) -> impl Iterator<Item = &RegOption> {
        self.options.iter().filter(move |o| o.emotion == e)
    }

    fn adjacent(&self, e: Emotion) -> Vec<Emotion> {
        Emotion::ALL
            .into_iter()
            .filter(|x| {
                *x != e
                    && self
                        .adjacency
                        .iter()
                        .any(|(a, b)| (*a == e && b == x) || (*b == e && a == x))
            })
            .collect()
    }

    fn option(&self, id: &str) -> Option<&RegOption> {
        self.options
            .iter()
            .chain(self.pairings.iter())
            .find(|o| o.id == id)
    }

    /// `path`, or the open fallback of a restricted face when `path` is
    /// absent and the fallback exists (see [`resolve_font_path`]).
    fn font_path(&self, path: &Path) -> PathBuf {
        if path.is_file() {
            return path.to_path_buf();
        }
        let fallback = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| self.restricted.get(n))
            .map(|f| path.with_file_name(f));
        match fallback {
            Some(f) if f.is_file() => f,
            _ => path.to_path_buf(),
        }
    }

    /// Is the face file `rel` usable under `root` (itself or its restricted
    /// fallback)?
    fn face_file_usable(&self, root: &Path, rel: &str) -> bool {
        self.font_path(&root.join(rel)).is_file()
    }

    /// The face actually used for index `i` under `root`: itself when its file
    /// (or its restricted fallback) exists, else its registry substitute when
    /// that file exists.
    fn resolve_face(&self, i: usize, root: &Path) -> Option<usize> {
        let exists = |k: usize| self.face_file_usable(root, self.faces[k].face.path);
        if exists(i) {
            Some(i)
        } else {
            self.substitutes.get(&i).copied().filter(|j| exists(*j))
        }
    }

    fn face_files_exist(&self, o: &RegOption, root: &Path) -> bool {
        o.roles
            .values()
            .all(|i| self.resolve_face(*i, root).is_some())
    }

    /// Substitutions an option needs under `root`, as `"<from>-><to>"`.
    fn substitutions(&self, o: &RegOption, root: &Path) -> Vec<String> {
        let mut out: Vec<String> = o
            .roles
            .values()
            .filter_map(|i| {
                let j = self.resolve_face(*i, root)?;
                (j != *i).then(|| {
                    format!(
                        "{}->{}",
                        self.faces[*i].face.asset_id, self.faces[j].face.asset_id
                    )
                })
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// [`Self::option_fontset`] with missing faces replaced by their substitutes.
    fn option_fontset_at(&self, o: &RegOption, root: &Path) -> Option<FontSet> {
        let mut set = self.option_fontset(o)?;
        for (i, j) in &self.substitutes {
            let (from, to) = (&self.faces[*i].face, &self.faces[*j].face);
            if !self.face_file_usable(root, from.path) {
                for f in set.roles.values_mut() {
                    if f.asset_id == from.asset_id {
                        *f = to.clone();
                    }
                }
            }
        }
        let mut faces: Vec<FontFace> = set.roles.values().cloned().collect();
        faces.extend(
            set.faces
                .iter()
                .filter(|f| !set.roles.values().any(|r| r.asset_id == f.asset_id))
                .filter(|f| self.face_file_usable(root, f.path))
                .cloned(),
        );
        faces.sort_by_key(|f| f.asset_id);
        faces.dedup_by_key(|f| f.asset_id);
        set.faces = faces;
        Some(set)
    }

    fn available(&self, o: &RegOption, root: &Path) -> bool {
        o.problems.is_empty() && self.face_files_exist(o, root)
    }

    fn option_roles(&self, o: &RegOption) -> Option<BTreeMap<FontRole, FontFace>> {
        if o.roles.len() != FontRole::ALL.len() {
            return None;
        }
        Some(
            o.roles
                .iter()
                .map(|(r, i)| (*r, self.faces[*i].face.clone()))
                .collect(),
        )
    }

    fn option_fontset(&self, o: &RegOption) -> Option<FontSet> {
        let roles = self.option_roles(o)?;
        let mut faces: Vec<FontFace> = roles.values().cloned().collect();
        for i in self.fallback.iter().chain(self.always.iter()) {
            faces.push(self.faces[*i].face.clone());
        }
        faces.sort_by_key(|f| f.asset_id);
        faces.dedup_by_key(|f| f.asset_id);
        Some(FontSet { roles, faces })
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| Registry::parse(include_str!("../../../../assets/fonts/registry.json")))
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// One registry face and whether its file exists under the library root.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FaceReport {
    pub id: String,
    pub file: String,
    pub family: Option<String>,
    pub weight: u16,
    pub italic: bool,
    pub file_exists: bool,
    /// The open file read in its place when a restricted face's own file is
    /// absent (`None` when the file exists or has no usable fallback).
    pub fallback_file: Option<String>,
    pub coverage: Option<FaceCoverage>,
}

/// One emotion option and whether it can be used.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OptionReport {
    pub id: String,
    pub emotion: Emotion,
    pub available: bool,
    pub problems: Vec<String>,
}

/// Result of [`validate_registry`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegistryReport {
    pub faces: Vec<FaceReport>,
    pub options: Vec<OptionReport>,
    /// Fatal data errors (empty = registry is consistent).
    pub errors: Vec<String>,
}

fn report(reg: &Registry, root: &Path) -> RegistryReport {
    let faces = reg
        .faces
        .iter()
        .map(|f| FaceReport {
            id: f.face.asset_id.to_string(),
            file: f.face.path.to_string(),
            family: f.family.clone(),
            weight: f.face.weight,
            italic: f.face.italic,
            file_exists: root.join(f.face.path).is_file(),
            fallback_file: {
                let full = root.join(f.face.path);
                let used = reg.font_path(&full);
                (used != full).then(|| {
                    Path::new(f.face.path)
                        .with_file_name(used.file_name().unwrap_or_default())
                        .to_string_lossy()
                        .into_owned()
                })
            },
            coverage: f.coverage,
        })
        .collect();
    let options = reg
        .options
        .iter()
        .map(|o| {
            let mut problems = o.problems.clone();
            for i in o.roles.values() {
                let f = &reg.faces[*i].face;
                let msg = format!("missing file {}", f.path);
                if !reg.face_file_usable(root, f.path) && !problems.contains(&msg) {
                    problems.push(msg);
                }
            }
            OptionReport {
                id: o.id.clone(),
                emotion: o.emotion,
                available: reg.available(o, root),
                problems,
            }
        })
        .collect();
    RegistryReport {
        faces,
        options,
        errors: reg.errors.clone(),
    }
}

/// Validate the embedded registry against the files under `library_root`
/// (fonts live at `<library_root>/fonts/...`). `errors` are fatal data errors;
/// an option whose face files are missing is merely unavailable.
pub fn validate_registry(library_root: &Path) -> RegistryReport {
    report(registry(), library_root)
}

/// Same as [`validate_registry`] for an arbitrary registry document (tests).
pub fn validate_registry_str(json: &str, library_root: &Path) -> RegistryReport {
    report(&Registry::parse(json), library_root)
}

/// The file to read for the font at `path`: `path` itself when it exists;
/// otherwise, when `path` names a restricted face (its licence forbids
/// redistribution; `restricted_fallbacks` in the registry) and its open
/// fallback exists in the same folder, that fallback; otherwise `path`
/// unchanged (the caller reports it missing). Deterministic: the same files
/// present give the same answer.
pub fn resolve_font_path(path: &Path) -> PathBuf {
    registry().font_path(path)
}

/// The open fallback file (registry-relative, e.g. `fonts/Lato-Bold.ttf`)
/// of a restricted face file (e.g. `fonts/Satoshi-Bold.otf`), if any.
pub fn restricted_fallback(file: &str) -> Option<String> {
    let p = Path::new(file);
    let name = p.file_name()?.to_str()?;
    registry()
        .restricted
        .get(name)
        .map(|f| p.with_file_name(f).to_string_lossy().into_owned())
}

/// Every restricted face file with its open fallback, both registry-relative,
/// sorted by face file.
pub fn restricted_fallbacks() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = registry()
        .faces
        .iter()
        .filter_map(|f| restricted_fallback(f.face.path).map(|t| (f.face.path.to_string(), t)))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Option ids of an emotion, in registry order.
pub fn option_ids(emotion: Emotion) -> Vec<String> {
    registry()
        .options_of(emotion)
        .map(|o| o.id.clone())
        .collect()
}

/// The emotion an option id belongs to.
pub fn option_emotion(option_id: &str) -> Option<Emotion> {
    registry().option(option_id).map(|o| o.emotion)
}

/// Complete FontSet of a registry option: its role faces + `fallback_face` +
/// `always_loaded`, sorted by asset id and deduplicated.
pub fn option_fontset(option_id: &str) -> Option<FontSet> {
    let reg = registry();
    reg.option(option_id).and_then(|o| reg.option_fontset(o))
}

/// [`option_fontset`] as resolved under `library_root` (missing faces replaced
/// by their registry substitutes).
pub fn option_fontset_in(option_id: &str, library_root: &Path) -> Option<FontSet> {
    let reg = registry();
    reg.option(option_id)
        .and_then(|o| reg.option_fontset_at(o, library_root))
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Emotion for a resolved taste (curated table).
pub fn resolve_emotion(taste: &ResolvedStyleProfile) -> Emotion {
    use TemperamentKind as K;
    let kind = taste.motion.kind;
    match taste.tone {
        ResolvedTone::Editorial => match kind {
            K::Restrained => Emotion::Calm,
            K::Editorial | K::Precise => {
                if taste.palette.temperature == ResolvedTemperature::Warm {
                    Emotion::Warmth
                } else {
                    Emotion::Trust
                }
            }
            K::Energetic => Emotion::Drama,
        },
        ResolvedTone::Technical => match kind {
            K::Restrained => Emotion::Calm,
            K::Editorial | K::Precise => Emotion::Precision,
            K::Energetic => Emotion::Energy,
        },
        ResolvedTone::Playful => match kind {
            K::Restrained => Emotion::Warmth,
            K::Editorial => Emotion::Joy,
            K::Precise => Emotion::PlayfulRetro,
            K::Energetic => Emotion::Energy,
        },
        // Reporting only: level 0 stays GroteskSerif.
        ResolvedTone::Classic => match kind {
            K::Energetic => Emotion::Energy,
            _ => Emotion::Drama,
        },
    }
}

#[derive(Clone, Copy)]
enum Candidate<'a> {
    Legacy,
    Option(&'a RegOption),
}

/// FNV-1a 64 of `"typography"`: the domain separator of the pick hash.
const TYPOGRAPHY_DOMAIN: u64 = {
    let bytes = b"typography";
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    h
};

/// Emotions whose options join the candidate list: `base`, then (level >= 2)
/// its adjacent emotions in `Emotion::ALL` order, then (level >= 3) every
/// emotion of the tone list not yet included.
fn included_emotions(
    reg: &Registry,
    base: Emotion,
    level: u8,
    tone_list: &[Emotion],
) -> Vec<Emotion> {
    let mut included = vec![base];
    if level >= 2 {
        included.extend(reg.adjacent(base));
    }
    if level >= 3 {
        for t in tone_list {
            if !included.contains(t) {
                included.push(*t);
            }
        }
    }
    included
}

/// Choose the typography for `taste` and build its FontSet. `library_root`
/// is where `fonts/<file>` resolve (availability check).
///
/// Order of rules: an explicit `--emotion` wins; otherwise a pairing that
/// differs from the tone default (reference / explicit `typography_style`) is
/// legacy-locked; otherwise level 0 is legacy; otherwise the candidate list
/// of the level is hashed: `mix(mix(mix(seed, level), story_key),
/// FNV("typography")) % len`.
pub fn resolve_typography(
    taste: &ResolvedStyleProfile,
    req: &TypographyRequest,
    library_root: &Path,
) -> (TypographyChoice, FontSet) {
    let reg = registry();
    let level = req.exploration.min(3);
    let resolved = resolve_emotion(taste);
    let legacy = |emotion: Emotion, fallback_from: Vec<String>| {
        (
            TypographyChoice {
                emotion,
                option: None,
                legacy: Some(taste.typography),
                fallback_from,
            },
            FontSet::for_pairing(taste.typography),
        )
    };
    // A reference-derived pairing stays pinned; an explicit `typography_style`
    // (and the Classic tone's own GroteskSerif) is a category constraint.
    let locked = taste.typography != default_pairing(taste.tone) && !taste.typography_explicit;
    if req.emotion.is_none() && (locked || level == 0) {
        return legacy(resolved, Vec::new());
    }
    let category = (req.emotion.is_none()
        && (taste.typography_explicit || taste.tone == ResolvedTone::Classic))
        .then_some(taste.typography);
    let tone_list: &[Emotion] = reg
        .tone_emotions
        .get(tone_name(taste.tone))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut cands: Vec<Candidate> = Vec::new();
    let report_emotion = req.emotion.unwrap_or(resolved);
    // (0.24) Level >= 1 only draws motion-safe faces: emotion options must pass
    // the gate; level 1 without an override draws the pairing library v2.
    let safe = |o: &&RegOption| reg.gate_ok(o);
    let pairings = || {
        reg.pairing_candidates(
            genre_name(taste.genre),
            tone_name(taste.tone),
            temperament_name(taste.motion.kind),
        )
    };
    match req.emotion {
        Some(e) if level == 0 => {
            cands.extend(reg.options_of(e).take(1).map(Candidate::Option));
        }
        Some(e) => {
            for em in included_emotions(reg, e, level, tone_list) {
                // An explicit `--emotion` is the operator's choice: not gated.
                cands.extend(reg.options_of(em).map(Candidate::Option));
            }
        }
        None => {
            if taste.tone == ResolvedTone::Classic {
                cands.push(Candidate::Legacy);
            }
            cands.extend(pairings().into_iter().map(Candidate::Option));
            if level >= 2 {
                let emotions = if taste.tone == ResolvedTone::Classic {
                    tone_list.to_vec()
                } else {
                    included_emotions(reg, resolved, level, tone_list)
                };
                for em in emotions {
                    cands.extend(reg.options_of(em).filter(safe).map(Candidate::Option));
                }
            }
        }
    }
    if let Some(pairing) = category {
        let keep = |c: &Candidate| match c {
            Candidate::Legacy => true,
            Candidate::Option(o) => reg.matches_category(o, pairing),
        };
        let mut matching: Vec<Candidate> = cands.iter().copied().filter(keep).collect();
        if !matching.iter().any(|c| matches!(c, Candidate::Option(_))) {
            // Nothing in the level's set fits: widen to every safe pairing, then
            // every safe emotion option of the tone.
            matching = reg
                .pairings
                .iter()
                .filter(|o| reg.gate_ok(o) && reg.matches_category(o, pairing))
                .map(Candidate::Option)
                .collect();
            if matching.is_empty() {
                matching = tone_list
                    .iter()
                    .flat_map(|em| reg.options_of(*em))
                    .filter(|o| reg.gate_ok(o) && reg.matches_category(o, pairing))
                    .map(Candidate::Option)
                    .collect();
            }
        }
        if matching.is_empty() {
            return legacy(report_emotion, Vec::new());
        }
        cands = matching;
    }
    if cands.is_empty() {
        return legacy(report_emotion, Vec::new());
    }
    let h = super::mix(
        super::mix(super::mix(req.seed, level as u64), req.story_key),
        TYPOGRAPHY_DOMAIN,
    );
    let Candidate::Option(picked) = cands[(h % cands.len() as u64) as usize] else {
        return legacy(report_emotion, Vec::new());
    };
    // Availability fallback: next option of the same emotion (wrapping, which
    // reaches option 1), then the legacy pairing.
    let is_pairing = reg.pairings.iter().any(|p| p.id == picked.id);
    let siblings: Vec<&RegOption> = if is_pairing {
        vec![picked]
    } else {
        reg.options_of(picked.emotion)
            .filter(|o| category.is_none_or(|p| reg.matches_category(o, p)))
            .filter(|o| req.emotion.is_some() || reg.gate_ok(o))
            .collect()
    };
    let start = siblings.iter().position(|o| o.id == picked.id).unwrap_or(0);
    let mut skipped = Vec::new();
    for k in 0..siblings.len() {
        let o = siblings[(start + k) % siblings.len()];
        if reg.available(o, library_root) {
            if let Some(fonts) = reg.option_fontset_at(o, library_root) {
                skipped.extend(
                    reg.substitutions(o, library_root)
                        .into_iter()
                        .map(|s| format!("{}:{s}", o.id)),
                );
                return (
                    TypographyChoice {
                        emotion: if is_pairing {
                            report_emotion
                        } else {
                            o.emotion
                        },
                        option: Some(o.id.clone()),
                        legacy: None,
                        fallback_from: skipped,
                    },
                    fonts,
                );
            }
        }
        skipped.push(o.id.clone());
    }
    legacy(report_emotion, skipped)
}

/// Role -> face for one registry option (for specimens and tests).
pub fn option_faces(option_id: &str) -> Option<BTreeMap<FontRole, FontFace>> {
    let reg = registry();
    reg.option(option_id).and_then(|o| reg.option_roles(o))
}

// ---------------------------------------------------------------------------
// (0.24) Motion gate: public API
// ---------------------------------------------------------------------------

/// Motion metadata of a registry face.
pub fn face_meta(face_id: &str) -> Option<FaceMeta> {
    registry()
        .faces
        .iter()
        .find(|f| f.face.asset_id == face_id)
        .map(|f| f.meta)
}

/// Is `face_id` safe for `role` in motion when `words` words are set in it?
/// `Err` carries the reason. Rules: no weight < 400; display / number need
/// weight >= 600 unless the face is high-contrast and the role is display;
/// body needs x-height >= 0.66 of cap height and low/medium contrast; number
/// needs tabular digits; extended faces only for display with <= 3 words.
pub fn motion_safe(face_id: &str, role: FontRole, words: usize) -> Result<(), String> {
    let reg = registry();
    let i = reg
        .faces
        .iter()
        .position(|f| f.face.asset_id == face_id)
        .ok_or_else(|| format!("unknown face {face_id}"))?;
    reg.gate_face(i, role, words)
}

/// Pairing library v2 ids, in registry order.
pub fn pairing_ids() -> Vec<String> {
    registry().pairings.iter().map(|p| p.id.clone()).collect()
}

/// Does a pairing or emotion option pass the motion gate (every role, long
/// text)? `Err` lists one reason per failing face.
pub fn pairing_gate(id: &str) -> Result<(), Vec<String>> {
    let reg = registry();
    match reg.option(id) {
        Some(o) => reg.gate_option(o),
        None => Err(vec![format!("unknown pairing or option {id}")]),
    }
}

/// Display-face class of a pairing / option (`serif`, `slab`, `grotesk`, ...).
pub fn display_class(id: &str) -> Option<String> {
    let reg = registry();
    let o = reg.option(id)?;
    let i = o.roles.get(&FontRole::Display)?;
    reg.faces[*i].class.clone()
}

/// One option or pairing the motion gate excludes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GateExclusion {
    pub id: String,
    pub reasons: Vec<String>,
}

/// Every emotion option and pairing that fails the motion gate.
pub fn gate_exclusions() -> Vec<GateExclusion> {
    let reg = registry();
    reg.options
        .iter()
        .chain(reg.pairings.iter())
        .filter_map(|o| {
            reg.gate_option(o).err().map(|reasons| GateExclusion {
                id: o.id.clone(),
                reasons,
            })
        })
        .collect()
}

/// The level-1 pairing candidates for a taste (ids, registry usage order).
pub fn level1_candidates(taste: &ResolvedStyleProfile) -> Vec<String> {
    registry()
        .pairing_candidates(
            genre_name(taste.genre),
            tone_name(taste.tone),
            temperament_name(taste.motion.kind),
        )
        .into_iter()
        .map(|o| o.id.clone())
        .collect()
}
