//! Asset planning, catalog, prompts, manifests, plates and contrast.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test assets <module>::`.

mod asset_catalog;
mod asset_planner;
mod asset_prompts;
mod field_contrast;
mod living_assets;
mod manifest_objects;
mod video_plates;
