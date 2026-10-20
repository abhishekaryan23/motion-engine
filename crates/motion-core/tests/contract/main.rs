//! Public contract: intent/style parsing, validation, schema-adjacent rules, assets fallback.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test contract <module>::`.

mod brand_colors;
mod intent_v0_2;
mod language;
mod layers_docs_qa;
mod layers_intent;
mod marker;
mod missing_asset_fallback;
mod public_contract;
mod sprite_validation;
mod validation;
