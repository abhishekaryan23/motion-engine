//! EditPlan v0.1: the closed-vocabulary "video prompt" (serde, human-editable).

use serde::{Deserialize, Serialize};

pub const EDIT_PLAN_VERSION: &str = "0.2";
/// E1 plans (overlays only) still load.
pub const EDIT_PLAN_VERSION_V0_1: &str = "0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

/// The speaker's face box over `[start, end)` seconds, in canvas pixels
/// `[x, y, w, h]`. `detected` is false for the fixed central-third fallback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceSeg {
    pub start: f64,
    pub end: f64,
    pub rect: [f32; 4],
    pub detected: bool,
}

/// What a full-frame cutaway shows (each compiled from generated beats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutawayKind {
    /// A spoken number with meaning.
    Figure,
    /// "X vs Y", more/less, before/after.
    Compare,
    /// "Three reasons", first/second: a progressive build.
    ListBuild,
    /// A source or a quoted claim on a document card.
    QuoteCard,
    /// A key claim as kinetic type.
    Concept,
}

impl CutawayKind {
    pub fn name(self) -> &'static str {
        match self {
            CutawayKind::Figure => "figure",
            CutawayKind::Compare => "compare",
            CutawayKind::ListBuild => "list_build",
            CutawayKind::QuoteCard => "quote_card",
            CutawayKind::Concept => "concept",
        }
    }
}

/// One generated CreativeIntent beat of a cutaway and the spoken words
/// (transcript indices, inclusive) it is locked to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutBeat {
    pub first: usize,
    pub last: usize,
    pub beat: crate::intent::Beat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// Once at the start. `text` is a placeholder the user fills.
    LowerThird { text: String },
    /// A spoken number, shown as it is said.
    NumberCallout { value: String },
    /// The stressed word of a key claim.
    Keyword { text: String },
    /// A topic shift.
    ChapterCard { title: String, index: usize },
    /// Karaoke captions for the whole talk (chunks are derived from the words).
    Captions {},
    /// (E2) A full-frame motion graphic over a span of words; the speaker's
    /// audio runs on underneath.
    Cutaway {
        sub: CutawayKind,
        beats: Vec<CutBeat>,
    },
    /// (E2) A small graphic in the free third beside the speaker: a number
    /// stack or key idea, with an optional proportion bar (0..1).
    SidePanel {
        value: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bar: Option<f32>,
    },
    /// (E2) A digital 1.00 -> 1.12 reframe on the face box.
    PunchIn {},
}

impl EventKind {
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::LowerThird { .. } => "lower_third",
            EventKind::NumberCallout { .. } => "number_callout",
            EventKind::Keyword { .. } => "keyword",
            EventKind::ChapterCard { .. } => "chapter_card",
            EventKind::Captions {} => "captions",
            EventKind::Cutaway { .. } => "cutaway",
            EventKind::SidePanel { .. } => "side_panel",
            EventKind::PunchIn {} => "punch_in",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditEvent {
    #[serde(flatten)]
    pub kind: EventKind,
    /// First spoken word (index into the transcript) the event is about.
    pub at_word: usize,
    pub until_word: usize,
    /// Seconds on the source timeline (derived from the words by the rules).
    pub start: f64,
    pub end: f64,
    pub side: Side,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditPlan {
    pub version: String,
    pub source: String,
    pub canvas: [u32; 2],
    pub fps: u32,
    pub duration: f64,
    pub language: String,
    pub words: usize,
    pub face_boxes: Vec<FaceSeg>,
    pub events: Vec<EditEvent>,
}

impl EditPlan {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn count(&self, name: &str) -> usize {
        self.events.iter().filter(|e| e.kind.name() == name).count()
    }

    /// Seconds covered by cutaways and side panels (the union).
    pub fn graphics_seconds(&self) -> f64 {
        let mut iv: Vec<(f64, f64)> = self
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    EventKind::Cutaway { .. } | EventKind::SidePanel { .. }
                )
            })
            .map(|e| (e.start, e.end))
            .collect();
        iv.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let (mut total, mut cur): (f64, Option<(f64, f64)>) = (0.0, None);
        for (a, b) in iv {
            cur = match cur {
                Some((s, e)) if a <= e => Some((s, e.max(b))),
                Some((s, e)) => {
                    total += e - s;
                    Some((a, b))
                }
                None => Some((a, b)),
            };
        }
        total + cur.map(|(s, e)| e - s).unwrap_or(0.0)
    }

    /// Structural problems (empty = valid).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.version != EDIT_PLAN_VERSION && self.version != EDIT_PLAN_VERSION_V0_1 {
            out.push(format!(
                "version {} (expected {EDIT_PLAN_VERSION})",
                self.version
            ));
        }
        for (i, e) in self.events.iter().enumerate() {
            if !(e.start.is_finite() && e.end.is_finite()) || e.end <= e.start || e.start < 0.0 {
                out.push(format!("event {i} ({}): bad time range", e.kind.name()));
            }
            if e.at_word > e.until_word || e.until_word >= self.words.max(1) {
                out.push(format!(
                    "event {i} ({}): word range out of bounds",
                    e.kind.name()
                ));
            }
            if let EventKind::Cutaway { beats, .. } = &e.kind {
                if beats.is_empty()
                    || beats
                        .iter()
                        .any(|b| b.first > b.last || b.last >= self.words.max(1))
                {
                    out.push(format!("event {i} (cutaway): bad beats"));
                }
            }
        }
        out
    }
}
