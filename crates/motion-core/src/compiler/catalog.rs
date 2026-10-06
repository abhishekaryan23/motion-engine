//! (0.8) Asset-library catalogs: opt-in asset families the AssetPlanner may
//! match before asking an external generator for an image.
//!
//! A family is a directory `<root>/library/<family>/` holding `catalog.json`
//! (what each asset depicts) and `manifest.json` (an `AssetManifest` written
//! by `ingest-assets`; entry ids are `library.<asset id>`, paths relative to
//! the family directory). Matching is EXACT word overlap only: no stemming,
//! no synonyms, no guessing. Missing or invalid files skip the family.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use super::explore::ExploreDimension;
use super::AssetLibrary;
use crate::assets::{AssetManifest, ManifestEntry};

/// Words that never count as evidence of a match.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "of", "and", "or", "to", "in", "on", "for", "with", "at", "by", "from", "is",
    "are",
];

/// Which catalog role a request may be served from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// Objects (catalog role `object`).
    Object,
    /// People (catalog role `figure`).
    Human,
    /// (0.19) Environment plates (catalog role `texture`): a scene behind the
    /// beat. A generic texture carries generic tags ("sky", "paper"), so an
    /// environment match needs at least [`ENVIRONMENT_MIN_WORDS`] matching words.
    Environment,
}

/// Matching words an environment plate needs (one shared word is not enough
/// evidence that a plate depicts the story's place).
pub const ENVIRONMENT_MIN_WORDS: usize = 2;

/// `catalog.json` of a family. Unknown fields are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub assets: Vec<CatalogAsset>,
}

/// One catalog asset. Unknown fields are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogAsset {
    pub id: String,
    pub file: String,
    /// `object`, `figure` or `texture`.
    pub role: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// `PASS`, `WARN`, `FAIL` or `MISSING`.
    #[serde(default)]
    pub qa: String,
}

/// The best catalog asset for a request.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogMatch {
    pub family: String,
    pub id: String,
    /// `library/<family>/<file>`.
    pub rel_path: String,
    /// The family manifest entry (id is rewritten to the request id on delivery).
    pub entry: ManifestEntry,
    /// The distinct request words found in the asset's words (sorted).
    pub matched: Vec<String>,
}

/// Lowercase ASCII-alphanumeric tokens of `parts`, stop words dropped.
pub fn request_words(parts: &[&str]) -> BTreeSet<String> {
    tokens(parts)
        .into_iter()
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect()
}

fn tokens(parts: &[&str]) -> BTreeSet<String> {
    parts
        .iter()
        .flat_map(|p| p.split(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn safe_rel(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.split('/').any(|s| s == ".." || s.is_empty())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn load_family(root: &Path, family: &str) -> Option<(Catalog, AssetManifest)> {
    if !safe_name(family) {
        return None;
    }
    let dir = root.join("library").join(family);
    let catalog: Catalog = read_json(&dir.join("catalog.json"))?;
    let manifest: AssetManifest = read_json(&dir.join("manifest.json"))?;
    Some((catalog, manifest))
}

/// Score bonus for an asset whose whole id is spelled by the request.
const NAMED_BONUS: usize = 4;

impl AssetLibrary {
    /// The best eligible catalog asset for `words` among the enabled families
    /// (see module docs). Highest score, then family order, then asset id. With
    /// exploration >= 2 a cross-family tie for the best score is picked by
    /// `explore::pick` instead of family order.
    pub fn catalog_match(&self, words: &BTreeSet<String>, kind: MatchKind) -> Option<CatalogMatch> {
        if words.is_empty() {
            return None;
        }
        let role = match kind {
            MatchKind::Object => "object",
            MatchKind::Human => "figure",
            MatchKind::Environment => "texture",
        };
        let min_score = match kind {
            MatchKind::Environment => ENVIRONMENT_MIN_WORDS,
            _ => 1,
        };
        // Best asset per family (score, then asset id), in family order.
        let mut per_family: Vec<(usize, CatalogMatch)> = Vec::new();
        for family in &self.families {
            let Some((catalog, manifest)) = load_family(&self.root, family) else {
                continue;
            };
            let mut assets: Vec<&CatalogAsset> = catalog
                .assets
                .iter()
                .filter(|a| a.role == role && matches!(a.qa.as_str(), "PASS" | "WARN"))
                .collect();
            assets.sort_by(|a, b| a.id.cmp(&b.id));
            let mut best: Option<(usize, CatalogMatch)> = None;
            for asset in assets {
                if !safe_rel(&asset.file) {
                    continue;
                }
                let wanted = format!("library.{}", asset.id);
                let Some(entry) = manifest
                    .assets
                    .iter()
                    .find(|e| e.id == wanted && safe_rel(&e.path))
                else {
                    continue;
                };
                let mut own: Vec<&str> = vec![asset.id.as_str()];
                own.extend(asset.tags.iter().map(String::as_str));
                let own = tokens(&own);
                let matched: Vec<String> = words.intersection(&own).cloned().collect();
                // (0.14) An asset whose whole id is spelled by the request
                // (`robot`, `neural_network`) beats partial tag matches on
                // meaning words ("a blank model" must not pick atom_model).
                let id_tokens = tokens(&[asset.id.as_str()]);
                let named =
                    !matched.is_empty() && !id_tokens.is_empty() && id_tokens.is_subset(words);
                let score = matched.len() + if named { NAMED_BONUS } else { 0 };
                if score < min_score || best.as_ref().is_some_and(|(s, _)| score <= *s) {
                    continue;
                }
                best = Some((
                    score,
                    CatalogMatch {
                        family: family.clone(),
                        id: asset.id.clone(),
                        rel_path: format!("library/{family}/{}", asset.file),
                        entry: entry.clone(),
                        matched,
                    },
                ));
            }
            per_family.extend(best);
        }
        let top = per_family.iter().map(|(s, _)| *s).max()?;
        let mut ties: Vec<CatalogMatch> = per_family
            .into_iter()
            .filter(|(s, _)| *s == top)
            .map(|(_, m)| m)
            .collect();
        let index = match self.explore {
            Some((level, seed)) if level >= 2 && ties.len() > 1 => {
                let mut key: u64 = 0xcbf2_9ce4_8422_2325;
                for w in words {
                    for b in w.bytes().chain(std::iter::once(0xff)) {
                        key ^= b as u64;
                        key = key.wrapping_mul(0x0100_0000_01b3);
                    }
                }
                super::explore::pick(seed, level, key, ExploreDimension::AssetFamily, ties.len())
            }
            _ => 0,
        };
        Some(ties.swap_remove(index))
    }

    /// The manifest entry delivering a planned library match
    /// (`library_asset` = `<family>/<id>`) for request `request_id`: the family
    /// manifest entry with `id`/`serves` set to the request and `path` made
    /// relative to `root` (`library/<family>/<entry path>`).
    pub fn catalog_delivery(&self, library_asset: &str, request_id: &str) -> Option<ManifestEntry> {
        let (family, id) = library_asset.split_once('/')?;
        if !self.families.iter().any(|f| f == family) {
            return None;
        }
        let (_, manifest) = load_family(&self.root, family)?;
        let wanted = format!("library.{id}");
        let mut entry = manifest
            .assets
            .into_iter()
            .find(|e| e.id == wanted && safe_rel(&e.path))?;
        entry.path = format!("library/{family}/{}", entry.path);
        entry.id = request_id.to_string();
        entry.serves = vec![request_id.to_string()];
        Some(entry)
    }
}
