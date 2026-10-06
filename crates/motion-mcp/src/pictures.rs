//! Picture lookup and "did you mean" (plan §8, Phase 1): what the compiler
//! would show for a noun, and the closest library pictures when it would show
//! text. Matching mirrors the compiler exactly (`AssetLibrary::find_object`,
//! then `catalog_match` over the look's families); suggestions add exact name,
//! stem and plural forms, shared tags or words, then edit distance ≤ 2, in the
//! look's family order, ties broken deterministically.
//!
//! The library is read once ([`PictureIndex::load`]); every query afterwards
//! is in memory (the motion-core library reads catalogs from disk per call).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{art_direction_for, look_defaults, Look};
use motion_core::compiler::catalog::{request_words, Catalog};
use motion_core::compiler::typography::Emotion;

use crate::lite::{snake, Pictures};

/// Score bonus of an asset whose whole id is spelled by the request (the
/// compiler's `NAMED_BONUS`).
const NAMED_BONUS: usize = 4;

/// How closely a suggestion matches the word asked for (best first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// Edit distance ≤ 2 on the name or a tag word (a likely typo; never
    /// applied automatically).
    Spelling,
    /// Shares a tag word (after singular/plural folding).
    SharedTag,
    /// Shares a name word (after singular/plural folding).
    SharedWord,
    /// The same words as the name (plural, other spacing: `berries` →
    /// `berry`, `piggybank` → `piggy_bank`).
    SameWords,
    /// Exactly the name.
    Exact,
}

impl Strength {
    /// Strong enough to replace a noun without asking (plan §8 auto-fix).
    pub fn is_strong(self) -> bool {
        self >= Strength::SharedTag
    }
}

/// One library picture eligible as an object (catalog role `object`, qa PASS
/// or WARN, with a manifest entry), or a named object.
#[derive(Debug, Clone)]
struct Entry {
    id: String,
    /// Raw tokens of the id (the compiler's `id_tokens`: stop words kept).
    id_tokens: BTreeSet<String>,
    /// Tokens of the id and tags, stop words dropped (what requests match).
    words: BTreeSet<String>,
    /// Singular forms of the id words / the tag words.
    id_stems: BTreeSet<String>,
    tag_stems: BTreeSet<String>,
    /// Id words for spelling matches. Tags are left out: a typo is of a
    /// picture's name, and a tag one letter away says nothing ("shark" is not
    /// a misspelt "share", the pie chart's tag).
    spell: BTreeSet<String>,
}

impl Entry {
    fn new(id: &str, tags: &[String]) -> Entry {
        let id_tokens = tokens(&[id]);
        let mut all: Vec<&str> = vec![id];
        all.extend(tags.iter().map(String::as_str));
        let words = request_words(&all);
        let id_words = request_words(&[id]);
        let tag_words = request_words(&tags.iter().map(String::as_str).collect::<Vec<_>>());
        Entry {
            id: id.to_string(),
            id_stems: id_words.iter().map(|w| singular(w)).collect(),
            tag_stems: tag_words.iter().map(|w| singular(w)).collect(),
            spell: id_words.clone(),
            id_tokens,
            words,
        }
    }
}

/// The library's pictures (named objects and every family catalog), loaded once.
#[derive(Debug, Clone, Default)]
pub struct PictureIndex {
    /// `library/<name>.svg|png` (the compiler's `find_object`).
    named: BTreeSet<String>,
    /// Family → its eligible object pictures, sorted by id.
    families: BTreeMap<String, Vec<Entry>>,
    /// `find` search order: the editorial look's families, then every other
    /// family a look can use, alphabetically.
    default_order: Vec<String>,
}

fn read_json<T: for<'de> serde::Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
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

/// Lowercase ASCII-alphanumeric tokens (the compiler's catalog `tokens`).
fn tokens(parts: &[&str]) -> BTreeSet<String> {
    parts
        .iter()
        .flat_map(|p| p.split(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// A comparison key that folds English plurals: `berries` → `berry`,
/// `leaves` → `leaf`, `knives`/`knife` → `knif`, `boxes` → `box`,
/// `rockets` → `rocket`. Applied to both sides of a comparison only.
pub fn singular(w: &str) -> String {
    let w = w.to_ascii_lowercase();
    let n = w.len();
    if n > 4 && w.ends_with("ies") {
        return format!("{}y", &w[..n - 3]);
    }
    if n > 4 && w.ends_with("ves") {
        return format!("{}f", &w[..n - 3]);
    }
    if n > 3 && w.ends_with("fe") {
        return format!("{}f", &w[..n - 2]);
    }
    for suffix in ["xes", "ches", "shes", "sses", "zes", "oes"] {
        if n > suffix.len() + 1 && w.ends_with(suffix) {
            return w[..n - 2].to_string();
        }
    }
    if n > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !w.ends_with("is") {
        return w[..n - 1].to_string();
    }
    w
}

/// Edit distance (chars) counting insertions, deletions, substitutions and
/// adjacent transpositions (`rokcet` → `rocket` is 1).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

/// The spelling slack for a word: none under 4 letters, 1 up to 6, else 2.
fn spelling_limit(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=6 => 1,
        _ => 2,
    }
}

/// A likely typo of `word`: same first letter, within the spelling slack.
fn spelled_like(query: &str, word: &str) -> Option<usize> {
    let limit = spelling_limit(query.chars().count());
    if limit == 0 || word.chars().count() < 4 || query.chars().next() != word.chars().next() {
        return None;
    }
    let d = edit_distance(query, word);
    (d <= limit).then_some(d)
}

/// Every family any look (or emotion row) can search, in a stable order.
fn look_families() -> Vec<String> {
    let mut out = BTreeSet::new();
    for look in Look::ALL {
        out.extend(look_defaults(look).families.iter().map(|f| f.to_string()));
    }
    for e in Emotion::ALL {
        out.extend(art_direction_for(e).families.iter().map(|f| f.to_string()));
    }
    out.into_iter().collect()
}

impl PictureIndex {
    /// Read `<assets>/library/` (named objects and family catalogs). Missing
    /// or invalid files are skipped, as the compiler skips them.
    pub fn load(assets: &Path) -> PictureIndex {
        let lib = assets.join("library");
        let mut named = BTreeSet::new();
        let mut families = BTreeMap::new();
        let mut dir: Vec<std::path::PathBuf> = std::fs::read_dir(&lib)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        dir.sort();
        for path in dir {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.is_file() {
                if let Some(stem) = name
                    .strip_suffix(".svg")
                    .or_else(|| name.strip_suffix(".png"))
                {
                    if safe_name(stem) {
                        named.insert(stem.to_string());
                    }
                }
                continue;
            }
            if !path.is_dir() || !safe_name(name) {
                continue;
            }
            let Some(catalog) = read_json::<Catalog>(&path.join("catalog.json")) else {
                continue;
            };
            let Some(manifest) = read_json::<AssetManifest>(&path.join("manifest.json")) else {
                continue;
            };
            let mut entries: Vec<Entry> = catalog
                .assets
                .iter()
                .filter(|a| a.role == "object" && matches!(a.qa.as_str(), "PASS" | "WARN"))
                .filter(|a| safe_rel(&a.file))
                .filter(|a| {
                    let wanted = format!("library.{}", a.id);
                    manifest
                        .assets
                        .iter()
                        .any(|e| e.id == wanted && safe_rel(&e.path))
                })
                .map(|a| Entry::new(&a.id, &a.tags))
                .collect();
            entries.sort_by(|a, b| a.id.cmp(&b.id));
            families.insert(name.to_string(), entries);
        }
        let editorial: Vec<String> = look_defaults(Look::OrnamentEditorial)
            .families
            .iter()
            .map(|f| f.to_string())
            .collect();
        let mut default_order: Vec<String> = editorial
            .iter()
            .filter(|f| families.contains_key(*f))
            .cloned()
            .collect();
        for f in look_families() {
            if families.contains_key(&f) && !default_order.contains(&f) {
                default_order.push(f);
            }
        }
        PictureIndex {
            named,
            families,
            default_order,
        }
    }

    /// Families with a readable catalog, alphabetically.
    pub fn family_names(&self) -> Vec<String> {
        self.families.keys().cloned().collect()
    }

    /// The `find` search order (the editorial look's families first).
    pub fn default_families(&self) -> &[String] {
        &self.default_order
    }

    /// Number of named objects plus eligible catalog pictures.
    pub fn len(&self) -> usize {
        self.named.len() + self.families.values().map(Vec::len).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the compiler would show `noun` as a picture with these
    /// families enabled (the look's families, in order).
    pub fn is_picture_in(&self, noun: &str, families: &[String]) -> bool {
        self.is_picture_with(noun, &[], families)
    }

    /// Like [`PictureIndex::is_picture_in`], with the subject's other words
    /// (`value`, `meaning`) that the compiler adds to the catalog request.
    pub fn is_picture_with(&self, noun: &str, extra: &[&str], families: &[String]) -> bool {
        self.shown_as(noun, extra, families).is_some()
    }

    /// The picture the compiler would show for `noun` (+ `extra` words):
    /// the named object, else the best catalog match (highest score, then
    /// family order, then id). `None` = shown as text.
    pub fn shown_as(&self, noun: &str, extra: &[&str], families: &[String]) -> Option<String> {
        let asset = snake(noun);
        if self.named.contains(&asset) {
            return Some(asset);
        }
        let mut parts: Vec<&str> = vec![asset.as_str()];
        parts.extend(extra.iter().copied());
        let words = request_words(&parts);
        if words.is_empty() {
            return None;
        }
        let mut best: Option<(usize, &str)> = None;
        for family in families {
            let Some(entries) = self.families.get(family) else {
                continue;
            };
            let mut family_best: Option<(usize, &str)> = None;
            for e in entries {
                let matched = e.words.intersection(&words).count();
                if matched == 0 {
                    continue;
                }
                let named = !e.id_tokens.is_empty() && e.id_tokens.is_subset(&words);
                let score = matched + if named { NAMED_BONUS } else { 0 };
                if family_best.is_some_and(|(s, _)| score <= s) {
                    continue;
                }
                family_best = Some((score, e.id.as_str()));
            }
            if let Some((s, id)) = family_best {
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, id));
                }
            }
        }
        best.map(|(_, id)| id.to_string())
    }

    /// Up to `k` picture names close to `word`, best first.
    pub fn suggest_in(&self, word: &str, k: usize, families: &[String]) -> Vec<String> {
        self.suggest_scored(word, k, families)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    /// [`PictureIndex::suggest_in`] with each suggestion's [`Strength`].
    /// Every name returned is itself a picture under `families`.
    pub fn suggest_scored(
        &self,
        word: &str,
        k: usize,
        families: &[String],
    ) -> Vec<(String, Strength)> {
        let q = snake(word);
        if k == 0 || q == "thing" && word.trim().is_empty() {
            return Vec::new();
        }
        let q_words = request_words(&[q.as_str()]);
        let q_stems: BTreeSet<String> = q_words.iter().map(|w| singular(w)).collect();
        let q_joined = q.replace('_', "");
        // (strength, overlap, family rank, id)
        let mut scored: Vec<(Strength, usize, usize, String)> = Vec::new();
        let named: Vec<Entry> = self.named.iter().map(|n| Entry::new(n, &[])).collect();
        let mut sources: Vec<(usize, &[Entry])> = vec![(0, named.as_slice())];
        for (i, f) in families.iter().enumerate() {
            if let Some(entries) = self.families.get(f) {
                sources.push((i + 1, entries.as_slice()));
            }
        }
        for (rank, entries) in sources {
            for e in entries {
                if let Some((s, overlap)) = score(e, &q, &q_joined, &q_stems, &q_words) {
                    scored.push((s, overlap, rank, e.id.clone()));
                }
            }
        }
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        // Spelling matches are a fallback: only when nothing shares a word.
        if scored.first().is_some_and(|s| s.0 > Strength::Spelling) {
            scored.retain(|s| s.0 > Strength::Spelling);
        }
        let mut out: Vec<(String, Strength)> = Vec::new();
        for (s, _, _, id) in scored {
            if out.iter().any(|(n, _)| *n == id) || !self.is_picture_in(&id, families) {
                continue;
            }
            out.push((id, s));
            if out.len() == k {
                break;
            }
        }
        out
    }

    /// `find_assets`: for each word, up to `k` picture names (empty = shown as
    /// text), searching every family in the default order. A word that is a
    /// picture lists the picture it would show first.
    pub fn find(&self, words: &[String], k: usize) -> BTreeMap<String, Vec<String>> {
        let families = &self.default_order;
        words
            .iter()
            .map(|w| {
                let mut names: Vec<String> = Vec::new();
                if let Some(shown) = self.shown_as(w, &[], families) {
                    names.push(shown);
                }
                for s in self.suggest_in(w, k + 1, families) {
                    if !names.contains(&s) {
                        names.push(s);
                    }
                }
                names.truncate(k);
                (w.clone(), names)
            })
            .collect()
    }

    /// A [`Pictures`] view restricted to `families` (for the lite mapping).
    pub fn with_families<'a>(&'a self, families: &'a [String]) -> InFamilies<'a> {
        InFamilies {
            index: self,
            families,
        }
    }
}

/// How `e` matches the query, if at all: (strength, overlapping words).
fn score(
    e: &Entry,
    q: &str,
    q_joined: &str,
    q_stems: &BTreeSet<String>,
    q_words: &BTreeSet<String>,
) -> Option<(Strength, usize)> {
    if e.id == q {
        return Some((Strength::Exact, q_stems.len()));
    }
    if q_stems.is_empty() {
        return None;
    }
    let id_joined = e.id.replace(['_', '-'], "").to_ascii_lowercase();
    if id_joined == q_joined || (!e.id_stems.is_empty() && e.id_stems == *q_stems) {
        return Some((Strength::SameWords, q_stems.len()));
    }
    let id_overlap = e.id_stems.intersection(q_stems).count();
    if id_overlap > 0 {
        return Some((Strength::SharedWord, id_overlap));
    }
    let tag_overlap = e.tag_stems.intersection(q_stems).count();
    if tag_overlap > 0 {
        return Some((Strength::SharedTag, tag_overlap));
    }
    // Spelling: the closest id word to any query word, or the whole name.
    let mut best: Option<usize> = None;
    for qw in q_words {
        for w in &e.spell {
            if let Some(d) = spelled_like(qw, w) {
                if best.is_none_or(|b| d < b) {
                    best = Some(d);
                }
            }
        }
    }
    if let Some(whole) = spelled_like(q_joined, &id_joined) {
        if best.is_none_or(|b| whole < b) {
            best = Some(whole);
        }
    }
    // Fewer edits rank higher (the overlap slot carries 3 - distance).
    best.map(|d| (Strength::Spelling, 3usize.saturating_sub(d)))
}

/// [`PictureIndex`] seen through one family list.
pub struct InFamilies<'a> {
    pub index: &'a PictureIndex,
    pub families: &'a [String],
}

impl Pictures for InFamilies<'_> {
    fn is_picture(&self, noun: &str) -> bool {
        self.index.is_picture_in(noun, self.families)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singular_folds_common_plurals() {
        for (a, b) in [
            ("berries", "berry"),
            ("leaves", "leaf"),
            ("boxes", "box"),
            ("rockets", "rocket"),
            ("watches", "watch"),
            ("glass", "glass"),
            ("bus", "bus"),
            ("tomatoes", "tomato"),
        ] {
            assert_eq!(singular(a), singular(b), "{a} vs {b}");
        }
        assert_eq!(singular("knives"), singular("knife"));
    }

    #[test]
    fn edit_distance_counts_edits() {
        assert_eq!(edit_distance("rocket", "rokcet"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("rocket", "rockets"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
    }
}
