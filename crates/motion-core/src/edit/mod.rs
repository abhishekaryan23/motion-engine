//! (0.24 E1) Auto-edit: transcript -> analysis -> EditPlan (closed vocabulary,
//! word-indexed). Pure and deterministic; no pixels, no I/O. The CLI
//! (`motion-engine edit`) ingests the video, calls these rules and renders.

pub mod analyze;
pub mod cutaway;
pub mod layout;
pub mod normalize;
pub mod plan;
pub mod rules;
pub mod translit;

pub use analyze::{analyse, Analysis, Entity, EntityKind, Tok};
pub use layout::{face_union, intersects, place, zone, Rect, Slot};
pub use plan::{
    CutBeat, CutawayKind, EditEvent, EditPlan, EventKind, FaceSeg, Side, EDIT_PLAN_VERSION,
};
pub use rules::{plan_edit, PlanInput};
