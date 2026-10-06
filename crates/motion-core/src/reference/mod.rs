//! Reference style interpretation (0.7): contracts and pure logic.
//!
//! ```text
//! reference video → (motion-render) deterministic evidence + samples
//!                 → interpreter-request.json            (bundle.rs)
//!                 → EXTERNAL multimodal model           (any provider)
//!                 → ReferenceStyleProfile JSON          (profile.rs)
//!                 → aliases + strict parse + validation (normalize.rs, validate.rs)
//!                 → ReferencePrinciples                 (normalize.rs)
//!                 → TasteDirector → ResolvedStyleProfile → compiler
//!                 → coverage report                     (coverage.rs)
//! ```
//!
//! A reference teaches principles (how it looks, how motion feels, how
//! compositions behave), never content: the profile has no free text, no
//! coordinates, no times, no assets. The engine embeds no model.

pub mod bundle;
pub mod coverage;
pub mod evidence;
pub mod normalize;
pub mod oklab;
pub mod profile;
pub mod profile_v0_1;
pub mod validate;
pub mod visual_language;

pub use bundle::InterpreterRequest;
pub use coverage::{coverage, CoverageReport};
pub use evidence::ReferenceEvidence;
pub use normalize::{normalize, NormalizedReference};
pub use profile::ReferenceStyleProfile;
pub use validate::{parse_profile, validate_profile, ParsedProfile, ProfileIssue};
