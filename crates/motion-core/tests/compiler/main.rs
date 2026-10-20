//! Compiler: choreography, lifecycle, layout, aspect handling, takes and exploration.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test compiler <module>::`.

mod aspect_sweep;
mod canvas_builders_a;
mod choreography;
mod compiler;
mod exploration;
mod landscape_native;
mod layer_stack;
mod layout;
mod lifecycle;
mod ornament_layout;
mod read_phase;
mod reveal_anchors;
mod short_beats;
mod takes;
mod widescreen_editorial;
