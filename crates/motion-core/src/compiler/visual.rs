//! Visual language (0.7.1, core-owned semantics): HOW scenes are constructed —
//! what fills the frame, the role of type versus imagery/objects, preferred
//! composition families, visual explanation mode.
//!
//! It comes only from a reference (`reference::normalize` maps the public
//! `visual_language` section); StyleProfile has no field for it. The default
//! is [`VisualLanguage::neutral`], under which every compiler path is the
//! pre-0.7.1 one (byte-identical output).
//!
//! It is a *bias*: meaning (CreativeIntent structure) always decides first.
//! Structured subjects (collection, state change, derived metric), numbers
//! and object subjects keep their grammars; only atomic phrase beats may be
//! constructed differently (see `grammar::select` and docs/VISUAL_LANGUAGE.md).

use serde::{Deserialize, Serialize};

use super::grammar::Grammar;
use crate::assets::AssetRole;

/// Internal medium (mirrors the public vocabulary; `Neutral` = no reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Medium {
    #[default]
    Neutral,
    TypeOnly,
    TypeLed,
    ImageLed,
    ObjectLed,
    Diagrammatic,
    Collage,
    InterfaceLed,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Usage {
    None,
    Sparse,
    Balanced,
    Dense,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Balance {
    TypeDominant,
    Balanced,
    VisualDominant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Character {
    Photographic,
    Cutout,
    Illustrative,
    Diagrammatic,
    ObjectCentric,
    Collage,
    Interface,
    Procedural,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Explanation {
    Literal,
    Diagrammatic,
    Symbolic,
    Metaphorical,
    EvidenceBased,
    Mixed,
}

/// The resolved visual-construction language. Every field optional except
/// the medium (`Neutral` when unknown).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VisualLanguage {
    pub medium: Medium,
    pub usage: Option<Usage>,
    pub balance: Option<Balance>,
    pub character: Option<Character>,
    /// Typical roles of visual material, most typical first (≤ 4).
    pub roles: Vec<AssetRole>,
    /// Composition-family preferences (disjoint).
    pub preferred: Vec<Grammar>,
    pub secondary: Vec<Grammar>,
    pub avoid: Vec<Grammar>,
    pub explanation: Option<Explanation>,
}

/// Where a visual language leans overall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weight {
    /// No usable visual-language information: pre-0.7.1 behaviour.
    Neutral,
    /// Typography carries the story.
    Type,
    /// Imagery / objects / diagrams carry the story.
    Visual,
}

impl VisualLanguage {
    /// No reference: every compiler path unchanged.
    pub fn neutral() -> Self {
        Self::default()
    }

    /// Overall lean. Rules (first match):
    /// 1. medium type_only / type_led → Type; image_led / object_led /
    ///    diagrammatic / collage / interface_led → Visual.
    /// 2. (mixed or unknown medium) usage none or balance type_dominant → Type;
    ///    balance visual_dominant or usage dense → Visual; mixed with usage
    ///    balanced → Visual.
    /// 3. otherwise Neutral.
    pub fn weight(&self) -> Weight {
        match self.medium {
            Medium::TypeOnly | Medium::TypeLed => return Weight::Type,
            Medium::ImageLed
            | Medium::ObjectLed
            | Medium::Diagrammatic
            | Medium::Collage
            | Medium::InterfaceLed => return Weight::Visual,
            Medium::Mixed | Medium::Neutral => {}
        }
        if self.usage == Some(Usage::None) || self.balance == Some(Balance::TypeDominant) {
            return Weight::Type;
        }
        if self.balance == Some(Balance::VisualDominant)
            || self.usage == Some(Usage::Dense)
            || (self.medium == Medium::Mixed && self.usage == Some(Usage::Balanced))
        {
            return Weight::Visual;
        }
        Weight::Neutral
    }

    /// Visual explanation should be built from engine-made shapes (procedural
    /// entity tokens, process tracks) rather than requested images:
    /// object_led / diagrammatic media, diagrammatic / procedural /
    /// object-centric material, or a diagrammatic explanation mode.
    pub fn prefers_procedural(&self) -> bool {
        self.weight() == Weight::Visual
            && (matches!(self.medium, Medium::ObjectLed | Medium::Diagrammatic)
                || matches!(
                    self.character,
                    Some(
                        Character::Diagrammatic | Character::Procedural | Character::ObjectCentric
                    )
                )
                || self.explanation == Some(Explanation::Diagrammatic))
    }

    /// Single-entity emphasis may ask an external generator for an (optional)
    /// image: a visual language led by images/collage/interfaces/mixed media
    /// with photographic / cutout / illustrative / collage / interface / mixed
    /// material, and some asset usage. Never when procedural is preferred.
    pub fn wants_images(&self) -> bool {
        self.weight() == Weight::Visual
            && !self.prefers_procedural()
            && matches!(
                self.medium,
                Medium::ImageLed | Medium::Collage | Medium::InterfaceLed | Medium::Mixed
            )
            && !matches!(self.usage, Some(Usage::None))
            && matches!(
                self.character,
                None | Some(
                    Character::Photographic
                        | Character::Cutout
                        | Character::Illustrative
                        | Character::Collage
                        | Character::Interface
                        | Character::Mixed
                )
            )
    }

    /// Reference preference for a grammar: preferred +2, secondary +1,
    /// avoid −3, otherwise 0.
    pub fn preference(&self, g: Grammar) -> i32 {
        if self.preferred.contains(&g) {
            2
        } else if self.secondary.contains(&g) {
            1
        } else if self.avoid.contains(&g) {
            -3
        } else {
            0
        }
    }
}
