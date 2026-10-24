//! Composition grammars and data: collections, charts, derived values, relations, recipes.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test grammars <module>::`.

mod beat_numbers;
mod beat_params_recipes;
mod collection;
mod count_settle;
mod dataviz;
mod derived;
mod diagram_connectors;
mod genre_grammars;
mod genre_values;
mod grammar_assets;
mod grammar_data;
mod grammar_image;
mod grammar_typographic;
mod ranked_bars;
mod relation_stage;
mod stat_values;
mod state_change;
mod value_coverage;
