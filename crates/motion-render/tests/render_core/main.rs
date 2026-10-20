//! Rasterising: primitives, layer effects, post, sprites, plates, v2 paths.
//! One test binary per area (fewer links, fewer first-launch scans); each
//! file is a module. Run one: `cargo test -p motion-render --test render_core <module>::`.

mod brand_mark;
mod field_bleed;
mod golden_assets;
mod image_treatment;
mod layer_effects;
mod monochrome_assets;
mod postfx;
mod postfx_timing;
mod primitives;
mod render_speed;
mod sprites;
mod stream_render;
mod subject_first;
mod tint_render;
mod type_styles;
mod v2_paths;
mod video_plates;
