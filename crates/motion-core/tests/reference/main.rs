//! Reference profiles and the visual-language interpreter.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test reference <module>::`.

mod reference_integration;
mod reference_profile;
mod reference_taste;
mod reference_visual_language;
mod visual_language_assets;
mod visual_language_interpreter;
mod visual_language_select;
