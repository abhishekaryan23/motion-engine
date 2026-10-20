//! Style recipes: `assets/recipes/<name>.recipe.json`, embedded at build
//! time so lowering stays pure and deterministic.

use serde::Deserialize;

use crate::program::{Feature, Fonts, Palette, Recipe, Tempo};

#[derive(Debug, Clone, Deserialize)]
pub struct RecipeSpec {
    pub description: String,
    pub palette: Palette,
    #[serde(default)]
    pub fonts: Option<Fonts>,
    #[serde(default)]
    pub tempo: Option<Tempo>,
    #[serde(default)]
    pub allow: Vec<Feature>,
}

fn text(r: Recipe) -> &'static str {
    match r {
        Recipe::PosterKinetic => {
            include_str!("../../../assets/recipes/poster_kinetic.recipe.json")
        }
        Recipe::ThinAxis => include_str!("../../../assets/recipes/thin_axis.recipe.json"),
        Recipe::OrangeDot => include_str!("../../../assets/recipes/orange_dot.recipe.json"),
    }
}

/// The recipe's spec (the bundled files are checked by a test).
pub fn spec(r: Recipe) -> RecipeSpec {
    serde_json::from_str(text(r)).expect("bundled recipe parses")
}
