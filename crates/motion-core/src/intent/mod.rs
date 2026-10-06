//! CreativeIntent v0.2 — the small, model-facing semantic representation.
//!
//! A weak model describes WHAT each beat should communicate. It never supplies
//! coordinates, sizes, timings, easing or z-order: those belong to the
//! MotionCompiler. Keep this vocabulary deliberately small.
//!
//! v0.2 adds three subject kinds proven necessary by blind benchmarks
//! (`collection`, `state_change`, `derived_metric`), the `accumulate`
//! relationship and (0.19) the `layers` subject, for stories about layered
//! or stacked things (zones, levels, stages) that a single picture cannot tell. v0.1 documents stay supported through the frozen [`v0_1`]
//! types and convert losslessly into these.
//!
//! Doc comments on these types are the public descriptions in
//! `schema/creative-intent-v0.2.schema.json` (generated; see
//! `tests/public_schema.rs`). Keep them semantic and implementation-free.

pub mod v0_1;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Current (recommended) contract version.
pub const INTENT_VERSION: &str = "0.2";
/// Legacy contract version, still accepted.
pub const INTENT_VERSION_V0_1: &str = "0.1";

/// A short motion-graphics story: a title, an output format and 1+ beats.
/// Describes WHAT to communicate; MotionEngine decides layout, animation and rendering.
// Deserialize is implemented by hand below (version dispatch).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreativeIntent {
    /// Contract version. Must be exactly "0.2" for this contract
    /// (documents written for the legacy "0.1" contract are still accepted).
    #[schemars(extend("const" = "0.2"))]
    pub version: String,
    /// Short name of the piece. Also used as the render output folder and video
    /// file name, so prefer letters, digits, '_' and '-'.
    pub title: String,
    /// Output aspect ratio. Defaults to "vertical".
    #[serde(default)]
    pub format: Format,
    /// The story, in order. One beat = one idea. At least one beat is required.
    #[schemars(length(min = 1))]
    pub beats: Vec<Beat>,
}

/// Output aspect ratio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// 9:16 portrait (1080 x 1920) for shorts / reels. The primary, best-supported format.
    #[default]
    Vertical,
    /// 1:1 (1080 x 1080).
    Square,
    /// 16:9 (1920 x 1080).
    Landscape,
}

/// One idea on screen for a few seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Beat {
    /// What this beat is for.
    pub purpose: Purpose,
    /// The one sentence this beat communicates, as the viewer should read it. Keep it short (about 3–8 words).
    pub statement: String,
    /// The main subject of the beat.
    pub primary: Subject,
    /// An optional second subject (the other side of a comparison, a supporting phrase or object).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<Subject>,
    /// How primary and secondary relate. Meaningful for "compare" and "contrast" beats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Relationship>,
    /// Pace and intensity of the beat. Defaults to "building".
    #[serde(default)]
    pub energy: Energy,
    /// Whether a subject of this beat stays on screen into the next beat. Defaults to "none".
    #[serde(default)]
    pub continuity: Continuity,
    /// Optional single word shown very large and faint in the background.
    /// Defaults to the primary subject's meaning (or its value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    /// Optional voice-over line for this beat: what the narrator SAYS, written the way you would say it out loud to a friend (1–2 natural sentences, about 8–30 words; contractions, questions and asides are welcome). All beats' lines are read in ONE take by one voice, so write them as one continuous script: each line continues the previous one and never restarts the topic or re-introduces the subject. The on-screen `statement` stays a short title. Omit it and the narrator reads the statement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 400))]
    pub narration: Option<String>,
}

/// A thing the beat is about. `kind` decides which other fields apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Subject {
    /// Words: a name, a claim, a short phrase.
    Phrase(Atom),
    /// A figure: count, amount, percentage, measurement.
    Number(Atom),
    /// A pictured thing from the built-in asset library.
    Object(ObjectAtom),
    /// Several distinct items that belong together (2–6), e.g. the things that add up to a total.
    Collection(Collection),
    /// One thing moving from one state to another, e.g. delivery time: "three days" → "same day".
    StateChange(StateChange),
    /// A value MotionEngine calculates from two numbers (a ratio), e.g. 80 sign-ups out of 1000 visitors.
    DerivedMetric(DerivedMetric),
    /// Things that are layered or stacked, named in order from the top (or start) down, e.g. the zones of the ocean. Use it as the primary of every beat that is about the same layers, repeating the same list: the layers then stay on screen from beat to beat and the viewer sees where each beat is happening.
    Layers(Layers),
}

/// Fields of a phrase or number subject.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Atom {
    /// Text shown for the subject exactly as written: the phrase, or the number with its
    /// unit/symbol (e.g. "120", "+40%", "3.5 km").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// What the subject stands for, in 1–3 words (e.g. "passengers"). Shown as a small label and
    /// used as the default background keyword. Shown instead of `value` when `value` is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
}

/// Fields of an object subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObjectAtom {
    /// The pictured thing as a short lowercase snake_case noun, e.g. "piggy_bank", "alarm_clock", "coffee_cup". The engine finds a matching picture in its library; if none exists the object is shown as text (its meaning), so any concrete noun is safe.
    #[schemars(extend("pattern" = "^[a-z0-9][a-z0-9_]{0,40}$"))]
    pub asset: String,
    /// A short figure stamped next to the object in compare/contrast beats (e.g. "₹6,900").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// What the object stands for, in 1–3 words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
}

/// Several distinct items shown individually. Use the `accumulate` relationship when they
/// add up to one larger consequence; put that consequence in `secondary` if you know it,
/// otherwise MotionEngine shows the total itself (the count, or the sum when every item is a number).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    /// The items, in the order they should appear. 2 to 6 items; each is a phrase, number or object.
    #[schemars(length(min = 2, max = 6))]
    pub items: Vec<CollectionItem>,
    /// What the items are together, in 1–3 words (e.g. "drinks"). Used to name the total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
}

/// One item of a collection. Items cannot themselves be collections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CollectionItem {
    /// A named item, e.g. "Coffee".
    Phrase(Atom),
    /// An item that is a figure, e.g. "₹120" with meaning "coffee".
    Number(Atom),
    /// A pictured item from the built-in asset library.
    Object(ObjectAtom),
}

/// One thing changing state. Put a second state_change in `secondary` to show two changes
/// happening at the same time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateChange {
    /// What is changing, in 1–3 words (e.g. "delivery time").
    #[schemars(length(min = 1), extend("pattern" = "\\S"))]
    pub entity: String,
    /// The state before, in 1–4 words (e.g. "three days").
    #[schemars(length(min = 1), extend("pattern" = "\\S"))]
    pub from: String,
    /// The state after, in 1–4 words (e.g. "same day").
    #[schemars(length(min = 1), extend("pattern" = "\\S"))]
    pub to: String,
    /// What the change means, in 1–4 words (e.g. "faster").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
}

/// A value calculated by MotionEngine from two numbers. Give the raw numbers, not the result.
/// Two derived metrics (primary and secondary) in a compare or contrast beat are compared directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DerivedMetric {
    /// How the value is derived. Only "ratio" (numerator ÷ denominator) exists. Defaults to "ratio".
    #[serde(default)]
    pub operation: Operation,
    /// The part (e.g. 80 sign-ups).
    pub numerator: NumericTerm,
    /// The whole it is measured against (e.g. 1000 visitors). Must not be 0.
    #[schemars(extend("properties" = { "value": { "not": { "const": 0 } } }))]
    pub denominator: NumericTerm,
    /// How the result is written. Defaults to "percent".
    #[serde(default)]
    pub format: MetricFormat,
    /// Name of the calculated value, in 1–3 words (e.g. "sign-up rate").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
}

/// A plain number with what it counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NumericTerm {
    /// The number itself, as a JSON number (e.g. 1000, not "1,000").
    pub value: f64,
    /// What it counts, in 1–3 words (e.g. "visitors").
    #[schemars(length(min = 1), extend("pattern" = "\\S"))]
    pub meaning: String,
}

/// How a derived value is calculated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// numerator ÷ denominator.
    #[default]
    Ratio,
}

/// How a derived value is written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricFormat {
    /// As a percentage, e.g. 80 ÷ 1000 → "8%". The default.
    #[default]
    Percent,
    /// As a plain decimal, e.g. 1 ÷ 8 → "0.125".
    Decimal,
    /// Per thousand, e.g. 800 ÷ 60000 → "13.3 per 1,000".
    PerThousand,
}

/// Layers in a fixed order. Repeat the same list (same names, same order) in each beat that
/// is about it, and name the layer the beat is about in `focus`. The beat's `secondary`
/// (a picture, phrase or number) is shown inside the focused layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    /// The layers from top to bottom (or first to last). 2 to 6; the first and the last must
    /// be ordinary layers, not boundaries.
    #[schemars(length(min = 2, max = 6))]
    pub layers: Vec<LayerSpec>,
    /// What the layers make up together, in 1–3 words (e.g. "ocean water column").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
    /// The `name` of the layer this beat is about. Omit it for a beat about the whole stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

/// One layer of a [`Layers`] subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    /// The layer's name, in 1–4 words (e.g. "Photic zone"). Names must differ within a stack.
    #[schemars(length(min = 1), extend("pattern" = "\\S"))]
    pub name: String,
    /// A short note shown under the name, up to about 6 words (e.g. "sunlit, photosynthesis").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// True for a thin boundary between two layers (e.g. a thermocline) rather than a layer
    /// of its own. A boundary sits between two ordinary layers. Defaults to false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub boundary: bool,
}

impl Layers {
    /// Index of the layer named `focus` (case-insensitive), if any.
    pub fn focus_index(&self) -> Option<usize> {
        let want = self.focus.as_deref()?.trim().to_lowercase();
        self.layers
            .iter()
            .position(|l| l.name.trim().to_lowercase() == want)
    }

    /// True when `other` is the same stack: the same names, notes and boundaries in the same
    /// order (the focus may differ). Consecutive beats with the same stack share one column.
    pub fn same_stack(&self, other: &Layers) -> bool {
        self.layers == other.layers
    }
}

/// Kind of subject (derived from a subject's `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    Phrase,
    Number,
    Object,
    Collection,
    StateChange,
    DerivedMetric,
    Layers,
}

/// What a beat is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Make one idea land: a bold headline plus a hero subject.
    Emphasize,
    /// Put two subjects side by side neutrally. Default relationship: "separate".
    Compare,
    /// Put two subjects in tension. Default relationship: "compress".
    Contrast,
    /// Deliver a payoff or conclusion: the primary (usually a number) is unveiled as the answer.
    Reveal,
    /// Explain something: the primary becomes the headline and the statement is read as body text.
    Explain,
}

/// How the primary and secondary subjects relate (used by compare / contrast beats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Relationship {
    /// The secondary gains weight or size over the beat.
    Grow,
    /// The secondary creates pressure: the space and presence of the primary shrink while the secondary looms larger.
    Compress,
    /// The two subjects pull apart, with a visible divide between them.
    Separate,
    /// The secondary takes over from the primary, which recedes.
    Replace,
    /// The primary persists steadily; also keeps the primary on screen into the next beat (like continuity "carry_primary").
    Carry,
    /// Separate items add up, one after another, into one larger consequence. Best with a "collection" primary.
    Accumulate,
}

/// Pace and intensity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Energy {
    /// Unhurried, gentle, reflective.
    Calm,
    /// Steady editorial momentum. The default.
    #[default]
    Building,
    /// Punchy and fast. A "reveal" beat with impact energy that follows another beat arrives with a bold
    /// full-screen accent-color transition.
    Impact,
}

/// Whether a subject of this beat stays on screen into the next beat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Continuity {
    /// Nothing carries over. The default.
    #[default]
    None,
    /// The primary subject stays on screen and travels into the next beat.
    CarryPrimary,
    /// The secondary subject stays on screen and travels into the next beat.
    CarrySecondary,
}

/// Private mirror of the v0.2 top level (derived, strict).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreativeIntentV02 {
    version: String,
    title: String,
    #[serde(default)]
    format: Format,
    beats: Vec<Beat>,
}

impl From<CreativeIntentV02> for CreativeIntent {
    fn from(m: CreativeIntentV02) -> Self {
        CreativeIntent {
            version: m.version,
            title: m.title,
            format: m.format,
            beats: m.beats,
        }
    }
}

/// Which contract a document declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclaredVersion {
    V0_1,
    V0_2,
}

fn unsupported_version(shown: &str) -> String {
    format!(
        "unsupported CreativeIntent version '{shown}' (expected \"{INTENT_VERSION}\", or legacy \"{INTENT_VERSION_V0_1}\")"
    )
}

/// Tolerant peek at the top-level `version` of a parsed document.
fn declared_version(root: &serde_json::Value) -> Result<DeclaredVersion, String> {
    let shown = match root.get("version") {
        Some(serde_json::Value::String(v)) => match v.as_str() {
            INTENT_VERSION => return Ok(DeclaredVersion::V0_2),
            INTENT_VERSION_V0_1 => return Ok(DeclaredVersion::V0_1),
            other => other.to_string(),
        },
        Some(other) => other.to_string(),
        None => "(missing)".to_string(),
    };
    Err(unsupported_version(&shown))
}

fn requires_v0_2(beat: usize, what: &str) -> String {
    format!("beat {beat}: {what} requires \"version\": \"{INTENT_VERSION}\"")
}

/// Find a v0.2-only feature in a raw document that claims version "0.1".
fn v0_2_feature_in_json(root: &serde_json::Value) -> Option<String> {
    let beats = root.get("beats")?.as_array()?;
    for (i, beat) in beats.iter().enumerate() {
        let n = i + 1;
        for slot in ["primary", "secondary"] {
            let kind = beat
                .get(slot)
                .and_then(|s| s.get("kind"))
                .and_then(|k| k.as_str());
            if let Some(k @ ("collection" | "state_change" | "derived_metric" | "layers")) = kind {
                return Some(requires_v0_2(n, &format!("{slot} kind '{k}'")));
            }
        }
        if beat.get("relationship").and_then(|r| r.as_str()) == Some("accumulate") {
            return Some(requires_v0_2(n, "relationship 'accumulate'"));
        }
    }
    None
}

fn json_error(message: impl std::fmt::Display) -> serde_json::Error {
    <serde_json::Error as serde::de::Error>::custom(message)
}

impl CreativeIntent {
    /// Parse a CreativeIntent document, dispatching on its `version`:
    /// "0.2" is parsed strictly as the current contract; "0.1" is parsed strictly as the
    /// frozen legacy contract and converted losslessly (version stays "0.1").
    /// Anything else is rejected. Syntax and type errors keep line/column information.
    /// Semantic rules are checked by [`CreativeIntent::validate`] (called by `compile`).
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        let root: serde_json::Value = serde_json::from_str(s)?;
        match declared_version(&root).map_err(json_error)? {
            DeclaredVersion::V0_2 => serde_json::from_str::<CreativeIntentV02>(s).map(Into::into),
            DeclaredVersion::V0_1 => match serde_json::from_str::<v0_1::CreativeIntent>(s) {
                Ok(old) => CreativeIntent::try_from(old).map_err(json_error),
                Err(e) => Err(match v0_2_feature_in_json(&root) {
                    Some(msg) => json_error(msg),
                    None => e,
                }),
            },
        }
    }

    /// Same dispatch as [`CreativeIntent::from_json`], from an already-parsed document.
    /// This is also what `serde_json::from_value::<CreativeIntent>` and the `Deserialize`
    /// impl use, so all entry points agree.
    pub fn from_value(root: serde_json::Value) -> Result<Self, serde_json::Error> {
        match declared_version(&root).map_err(json_error)? {
            DeclaredVersion::V0_2 => {
                serde_json::from_value::<CreativeIntentV02>(root).map(Into::into)
            }
            DeclaredVersion::V0_1 => {
                let feature = v0_2_feature_in_json(&root);
                match serde_json::from_value::<v0_1::CreativeIntent>(root) {
                    Ok(old) => CreativeIntent::try_from(old).map_err(json_error),
                    Err(e) => Err(match feature {
                        Some(msg) => json_error(msg),
                        None => e,
                    }),
                }
            }
        }
    }

    /// Semantic checks that the type shapes cannot express. Returns every problem found.
    /// Beat numbers are 1-based. An empty beat list is reported by `compile`.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.version != INTENT_VERSION && self.version != INTENT_VERSION_V0_1 {
            errs.push(unsupported_version(&self.version));
        }
        let legacy = self.version == INTENT_VERSION_V0_1;
        for (i, beat) in self.beats.iter().enumerate() {
            let n = i + 1;
            if legacy && beat.relationship == Some(Relationship::Accumulate) {
                errs.push(requires_v0_2(n, "relationship 'accumulate'"));
            }
            for (slot, subject) in [
                ("primary", Some(&beat.primary)),
                ("secondary", beat.secondary.as_ref()),
            ] {
                if let Some(subject) = subject {
                    if legacy && !subject.is_atomic() {
                        errs.push(requires_v0_2(
                            n,
                            &format!("{slot} kind '{}'", subject.kind_name()),
                        ));
                    }
                    validate_subject(n, slot, subject, &mut errs);
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

fn validate_subject(n: usize, slot: &str, subject: &Subject, errs: &mut Vec<String>) {
    match subject {
        Subject::Phrase(_) | Subject::Number(_) | Subject::Object(_) => {}
        Subject::Collection(c) => {
            if !(2..=6).contains(&c.items.len()) {
                errs.push(format!(
                    "beat {n}: {slot}.items must contain 2 to 6 items (got {})",
                    c.items.len()
                ));
            }
        }
        Subject::StateChange(s) => {
            for (field, text) in [("entity", &s.entity), ("from", &s.from), ("to", &s.to)] {
                if text.trim().is_empty() {
                    errs.push(format!("beat {n}: {slot}.{field} must not be empty"));
                }
            }
        }
        Subject::Layers(l) => validate_layers(n, slot, l, errs),
        Subject::DerivedMetric(m) => {
            for (field, term) in [("numerator", &m.numerator), ("denominator", &m.denominator)] {
                if !term.value.is_finite() {
                    errs.push(format!(
                        "beat {n}: {slot}.{field}.value must be a finite number"
                    ));
                }
                if term.meaning.trim().is_empty() {
                    errs.push(format!(
                        "beat {n}: {slot}.{field}.meaning must not be empty"
                    ));
                }
            }
            if m.denominator.value == 0.0 {
                errs.push(format!("beat {n}: {slot}.denominator.value must not be 0"));
            } else if m.numerator.value.is_finite()
                && m.denominator.value.is_finite()
                && !(m.numerator.value / m.denominator.value).is_finite()
            {
                errs.push(format!(
                    "beat {n}: {slot}.numerator / denominator is out of range"
                ));
            }
        }
    }
}

fn validate_layers(n: usize, slot: &str, l: &Layers, errs: &mut Vec<String>) {
    if slot != "primary" {
        errs.push(format!(
            "beat {n}: a layers subject must be the primary, not the {slot}"
        ));
    }
    if !(2..=6).contains(&l.layers.len()) {
        errs.push(format!(
            "beat {n}: {slot}.layers must contain 2 to 6 layers (got {})",
            l.layers.len()
        ));
        return;
    }
    let mut seen = std::collections::BTreeSet::new();
    for (i, layer) in l.layers.iter().enumerate() {
        let name = layer.name.trim().to_lowercase();
        if name.is_empty() {
            errs.push(format!(
                "beat {n}: {slot}.layers[{i}].name must not be empty"
            ));
        } else if !seen.insert(name) {
            errs.push(format!(
                "beat {n}: {slot}.layers names must differ ('{}' appears twice)",
                layer.name.trim()
            ));
        }
    }
    let boundary = |i: usize| l.layers[i].boundary;
    let last = l.layers.len() - 1;
    if boundary(0) || boundary(last) {
        errs.push(format!(
            "beat {n}: {slot}.layers: the first and the last layer cannot be boundaries"
        ));
    }
    if (1..=last).any(|i| boundary(i) && boundary(i - 1)) {
        errs.push(format!(
            "beat {n}: {slot}.layers: a boundary must sit between two ordinary layers"
        ));
    }
    if let Some(focus) = &l.focus {
        if l.focus_index().is_none() {
            errs.push(format!(
                "beat {n}: {slot}.focus '{focus}' is not the name of any layer"
            ));
        }
    }
}

impl<'de> Deserialize<'de> for CreativeIntent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{Error, MapAccess, Visitor};

        /// Buffers the top-level object (rejecting duplicate keys) so the version can be
        /// peeked before choosing the contract to parse against.
        struct Buffer;
        impl<'de> Visitor<'de> for Buffer {
            type Value = serde_json::Value;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a CreativeIntent object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                    if out.contains_key(&key) {
                        return Err(A::Error::custom(format!("duplicate field `{key}`")));
                    }
                    out.insert(key, value);
                }
                Ok(serde_json::Value::Object(out))
            }
        }

        let root = deserializer.deserialize_map(Buffer)?;
        CreativeIntent::from_value(root).map_err(D::Error::custom)
    }
}

/// v0.1 → current types, lossless. The version stays "0.1" so the compiler knows the
/// document is legacy. `asset` on phrase/number subjects was documented as ignored and
/// is dropped; an object subject without `asset` is an error.
impl TryFrom<v0_1::CreativeIntent> for CreativeIntent {
    type Error = String;

    fn try_from(old: v0_1::CreativeIntent) -> Result<Self, String> {
        let mut beats = Vec::with_capacity(old.beats.len());
        for (i, b) in old.beats.into_iter().enumerate() {
            let n = i + 1;
            beats.push(Beat {
                purpose: b.purpose.into(),
                statement: b.statement,
                primary: convert_subject(b.primary, n)?,
                secondary: b.secondary.map(|s| convert_subject(s, n)).transpose()?,
                relationship: b.relationship.map(Into::into),
                energy: b.energy.into(),
                continuity: b.continuity.into(),
                keyword: b.keyword,
                narration: None,
            });
        }
        Ok(CreativeIntent {
            version: old.version,
            title: old.title,
            format: old.format.into(),
            beats,
        })
    }
}

fn convert_subject(s: v0_1::Subject, beat: usize) -> Result<Subject, String> {
    Ok(match s.kind {
        v0_1::SubjectKind::Phrase => Subject::Phrase(Atom {
            value: s.value,
            meaning: s.meaning,
        }),
        v0_1::SubjectKind::Number => Subject::Number(Atom {
            value: s.value,
            meaning: s.meaning,
        }),
        v0_1::SubjectKind::Object => Subject::Object(ObjectAtom {
            asset: s
                .asset
                .ok_or_else(|| format!("beat {beat}: object subject needs an 'asset'"))?,
            value: s.value,
            meaning: s.meaning,
        }),
    })
}

macro_rules! convert_enum {
    ($from:ty => $to:ty { $($v:ident),+ $(,)? }) => {
        impl From<$from> for $to {
            fn from(v: $from) -> Self {
                match v { $(<$from>::$v => <$to>::$v),+ }
            }
        }
    };
}

convert_enum!(v0_1::Format => Format { Vertical, Square, Landscape });
convert_enum!(v0_1::Purpose => Purpose { Emphasize, Compare, Contrast, Reveal, Explain });
convert_enum!(v0_1::Relationship => Relationship { Grow, Compress, Separate, Replace, Carry });
convert_enum!(v0_1::Energy => Energy { Calm, Building, Impact });
convert_enum!(v0_1::Continuity => Continuity { None, CarryPrimary, CarrySecondary });

impl Subject {
    pub fn kind(&self) -> SubjectKind {
        match self {
            Subject::Phrase(_) => SubjectKind::Phrase,
            Subject::Number(_) => SubjectKind::Number,
            Subject::Object(_) => SubjectKind::Object,
            Subject::Collection(_) => SubjectKind::Collection,
            Subject::StateChange(_) => SubjectKind::StateChange,
            Subject::DerivedMetric(_) => SubjectKind::DerivedMetric,
            Subject::Layers(_) => SubjectKind::Layers,
        }
    }

    /// The snake_case `kind` tag of this subject.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Subject::Phrase(_) => "phrase",
            Subject::Number(_) => "number",
            Subject::Object(_) => "object",
            Subject::Collection(_) => "collection",
            Subject::StateChange(_) => "state_change",
            Subject::DerivedMetric(_) => "derived_metric",
            Subject::Layers(_) => "layers",
        }
    }

    /// Phrase, number or object (the v0.1 kinds).
    pub fn is_atomic(&self) -> bool {
        matches!(
            self,
            Subject::Phrase(_) | Subject::Number(_) | Subject::Object(_)
        )
    }

    /// The written value of an atomic subject.
    pub fn value(&self) -> Option<&str> {
        match self {
            Subject::Phrase(a) | Subject::Number(a) => a.value.as_deref(),
            Subject::Object(o) => o.value.as_deref(),
            _ => None,
        }
    }

    pub fn meaning(&self) -> Option<&str> {
        match self {
            Subject::Phrase(a) | Subject::Number(a) => a.meaning.as_deref(),
            Subject::Object(o) => o.meaning.as_deref(),
            Subject::Collection(c) => c.meaning.as_deref(),
            Subject::StateChange(c) => c.meaning.as_deref().or(Some(c.entity.as_str())),
            Subject::DerivedMetric(m) => m.meaning.as_deref(),
            Subject::Layers(l) => l.meaning.as_deref(),
        }
    }

    pub fn asset(&self) -> Option<&str> {
        match self {
            Subject::Object(o) => Some(o.asset.as_str()),
            _ => None,
        }
    }

    /// Text to display for an atomic subject (value, else meaning); the
    /// meaning for structured subjects.
    pub fn display_text(&self) -> Option<&str> {
        self.value().or(self.meaning())
    }

    /// The same subject with its written value removed (atomic kinds).
    pub fn without_value(&self) -> Subject {
        let mut s = self.clone();
        match &mut s {
            Subject::Phrase(a) | Subject::Number(a) => a.value = None,
            Subject::Object(o) => o.value = None,
            _ => {}
        }
        s
    }
}

impl CollectionItem {
    /// The item as a standalone subject.
    pub fn as_subject(&self) -> Subject {
        match self {
            CollectionItem::Phrase(a) => Subject::Phrase(a.clone()),
            CollectionItem::Number(a) => Subject::Number(a.clone()),
            CollectionItem::Object(o) => Subject::Object(o.clone()),
        }
    }
}
