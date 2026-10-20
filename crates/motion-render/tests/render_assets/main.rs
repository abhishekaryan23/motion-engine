//! Asset analysis, decoding, ingest, indexes and reference media.
//! One test binary per area (fewer links, fewer first-launch scans); each
//! file is a module. Run one: `cargo test -p motion-render --test render_assets <module>::`.

mod asset_analysis;
mod asset_decode;
mod asset_hygiene;
mod asset_ingest;
mod image_index;
mod reference_analyze;
mod reference_media;
mod visual_language_calibration;
