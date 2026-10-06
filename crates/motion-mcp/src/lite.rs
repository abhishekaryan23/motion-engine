//! The lite story (plan §5): the one small object a weak model writes, and its
//! deterministic mapping to CreativeIntent v0.2.
//!
//! ```json
//! { "title": "buffett cash",
//!   "beats": [
//!     { "say": "Warren Buffett is sitting on 381 billion dollars in cash.",
//!       "show": "Buffett's cash pile", "number": "$381B", "meaning": "cash reserves",
//!       "picture": "money_stack" },
//!     { "say": "Five tools now run the modern workflow.",
//!       "list": ["robot", "microchip", "brain_circuit", "chat_bubbles", "ai_sparkles"] },
//!     { "say": "Costs fell while output tripled.",
//!       "compare": { "a": "cost", "b": "output", "how": "separate" } },
//!     { "say": "Below the thermocline, the water turns cold and dark.",
//!       "layers": { "names": ["sunlit", "twilight", "midnight"], "focus": "twilight" } } ] }
//! ```
//!
//! Mapping, in priority order (a beat shows ONE structure; fields the chosen
//! structure cannot show are dropped and reported in [`Mapped::notes`]):
//!
//! | lite fields | intent |
//! |---|---|
//! | `layers` | primary `layers` (+ `focus`); `picture` → secondary object, else `number` → secondary number |
//! | `list` | primary `collection` (items: object when [`Pictures::is_picture`], else phrase; `meaning`); purpose emphasize |
//! | `compare` | purpose compare; primary `a`, secondary `b` (object when a picture, else phrase); relationship from `how` (default separate) |
//! | `change` | primary `state_change` {entity ← what, from, to, meaning} |
//! | `number` | primary number {value, meaning}; `picture` → secondary object; purpose reveal on the last beat, else emphasize |
//! | only `picture` | primary object {asset, meaning}; `picture2` → secondary object |
//! | none of these | primary phrase from `show`, else `keyword`, else the derived title |
//! | always | narration ← `say`; statement ← `show`, else [`derived_title`] (`keyword`, else the first words of `say`); `keyword`; `energy` |
//!
//! Purpose is emphasize unless the table says otherwise. `title` becomes
//! snake_case (derived from the first beat when missing). The mapping is pure:
//! the same story, format and picture set always give the same intent (golden
//! tests in `tests/lite_golden.rs`). Auto-fixes (picture suggestions, title
//! shortening, lenient parsing) happen BEFORE mapping, in [`crate::policy`].

use motion_core::compiler::taste_rules::TITLE_MAX_WORDS;
use motion_core::intent::{
    Atom, Beat, Collection, CollectionItem, Continuity, CreativeIntent, Energy, Format, LayerSpec,
    Layers, ObjectAtom, Purpose, Relationship, StateChange, Subject, INTENT_VERSION,
};
use serde::{Deserialize, Serialize};

/// A story as a weak model writes it. Parsed leniently by
/// [`crate::policy::parse_story`] (unknown fields are ignored with a note).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiteStory {
    /// Short name; becomes the snake_case intent title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub beats: Vec<LiteBeat>,
}

/// One beat: what the narrator says, plus at most one structure.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiteBeat {
    /// What the narrator says (8–30 words). Required.
    #[serde(default)]
    pub say: String,
    /// On-screen title, at most 6 words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<PictureRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture2: Option<PictureRef>,
    /// A figure as written: "$381B", "40%", "36 months".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// What the number / picture / list stands for, 1–3 words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
    /// One word shown large and faint behind the beat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    /// 3–6 items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare: Option<LiteCompare>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<LiteChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers: Option<LiteLayers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<Energy>,
}

/// A picture: a library noun, or one of the user's own images (plan §5a).
///
/// In JSON either a string — `"rocket"` (library) or `"file:my_trip/beach.jpg"`
/// (user image) — or `{ "file": "my_trip/beach.jpg", "role": "place" }`.
/// After [`crate::policy::parse_story`] a `Name` never starts with `file:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PictureRef {
    Name(String),
    File(FileRef),
}

/// A user image, relative to a configured asset root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    pub file: String,
    /// Omitted: guessed on the engine side (Apple Vision on macOS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ImageRole>,
}

/// What a user image shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageRole {
    /// A person (→ manifest role `hero_subject`, cut out when opaque).
    Person,
    /// A thing (→ `hero_object`).
    Object,
    /// A place or scene (→ `environment`).
    Place,
}

impl PictureRef {
    /// The user-image reference, if this is one (`file:` strings included).
    pub fn file(&self) -> Option<FileRef> {
        match self {
            PictureRef::File(f) => Some(f.clone()),
            PictureRef::Name(n) => n.trim().strip_prefix("file:").map(|p| FileRef {
                file: p.trim().to_string(),
                role: None,
            }),
        }
    }

    /// The library noun, if this is one.
    pub fn noun(&self) -> Option<&str> {
        match self {
            PictureRef::Name(n) if !n.trim().starts_with("file:") => Some(n.trim()),
            _ => None,
        }
    }
}

/// Two things side by side.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiteCompare {
    pub a: String,
    pub b: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub how: Option<How>,
}

/// How the two sides of a comparison relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// Pull apart with a visible divide (the default).
    Separate,
    /// `b` gains weight.
    Grow,
    /// `b` squeezes `a`.
    Compress,
    /// `b` takes over from `a`.
    Replace,
}

impl How {
    pub fn relationship(self) -> Relationship {
        match self {
            How::Separate => Relationship::Separate,
            How::Grow => Relationship::Grow,
            How::Compress => Relationship::Compress,
            How::Replace => Relationship::Replace,
        }
    }
}

/// One thing changing state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiteChange {
    pub what: String,
    pub from: String,
    pub to: String,
}

/// Layers named top to bottom, and the one this beat is about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LiteLayers {
    pub names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

/// Which library pictures exist. Implemented over the real library by
/// [`crate::pictures::PictureIndex`]; tests use a fixed set.
pub trait Pictures {
    /// True when `noun` (any spelling; compared as snake_case) would be shown
    /// as a picture rather than as text.
    fn is_picture(&self, noun: &str) -> bool;
}

/// The mapped intent and what the mapping dropped (one line per field).
#[derive(Debug, Clone, PartialEq)]
pub struct Mapped {
    pub intent: CreativeIntent,
    /// e.g. `beat 3: 'number' not shown (the beat shows a list)`.
    pub notes: Vec<String>,
}

/// The structure a beat shows (priority order of the mapping table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Structure {
    Layers,
    List,
    Compare,
    Change,
    Number,
    Picture,
    Phrase,
}

impl Structure {
    pub fn name(self) -> &'static str {
        match self {
            Structure::Layers => "layers",
            Structure::List => "list",
            Structure::Compare => "compare",
            Structure::Change => "change",
            Structure::Number => "number",
            Structure::Picture => "picture",
            Structure::Phrase => "title",
        }
    }
}

impl LiteBeat {
    /// The structure this beat shows.
    pub fn structure(&self) -> Structure {
        if self.layers.is_some() {
            Structure::Layers
        } else if self.list.is_some() {
            Structure::List
        } else if self.compare.is_some() {
            Structure::Compare
        } else if self.change.is_some() {
            Structure::Change
        } else if self.number.is_some() {
            Structure::Number
        } else if self.picture.is_some() {
            Structure::Picture
        } else {
            Structure::Phrase
        }
    }
}

/// Lowercase snake_case noun, valid as an intent `asset` (`^[a-z0-9][a-z0-9_]{0,40}$`).
/// Empty input gives `"thing"`.
pub fn snake(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let mut out: String = out.trim_matches('_').chars().take(41).collect();
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "thing".to_string()
    } else {
        out
    }
}

/// The intent title: snake_case of `title`, else of the first beat's `show`,
/// else of its first four spoken words; at most 40 characters.
pub fn intent_title(story: &LiteStory) -> String {
    let source = if !story.title.trim().is_empty() {
        story.title.clone()
    } else if let Some(b) = story.beats.first() {
        b.show
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                b.say
                    .split_whitespace()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
    } else {
        String::new()
    };
    let t: String = snake(&source).chars().take(40).collect();
    let t = t.trim_end_matches('_').to_string();
    if t == "thing" {
        "video".to_string()
    } else {
        t
    }
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn object(noun: &str, meaning: Option<String>) -> ObjectAtom {
    ObjectAtom {
        asset: snake(noun),
        value: None,
        meaning,
    }
}

/// A picture as a subject: library nouns and user files become objects (a
/// user file is delivered through the job's AssetManifest, [`crate::byo`]),
/// except a user photo of a person, which becomes a phrase.
///
/// (1c) Why the person branch: the compiler shows a delivered person image
/// (`beat_N.hero_subject`) in a beat whose subject is NOT an object — an
/// emphasized phrase picks the type-and-image interlock, and the genre looks
/// stage it as their hero — while an object subject picks the hero-object
/// grammars, which only read `hero_object` / `supporting_object` images and
/// would treat the person as a thing. The phrase carries no words of its own
/// (the beat's title and `meaning` are shown), so nothing new is printed.
/// Verified by compiling with `--asset-manifest` across the looks.
fn picture_subject(p: &PictureRef, meaning: Option<String>) -> Subject {
    if let Some(f) = p.file() {
        if f.role == Some(ImageRole::Person) {
            return Subject::Phrase(Atom {
                value: None,
                meaning,
            });
        }
        return Subject::Object(object(&file_noun(&f.file), meaning));
    }
    let noun = match p {
        PictureRef::Name(n) => n.trim().to_string(),
        PictureRef::File(f) => f.file.clone(),
    };
    Subject::Object(object(&noun, meaning))
}

/// The object name of a user image: its file stem (`my_trip/beach.jpg` →
/// `beach`), or `photo` for folder-convention names (`beat2_object.png`,
/// `background.jpg`).
///
/// (1c) Why not the plain stem: the job manifest serves every object role of
/// the beat (`crate::policy`), so the name is never used to find a picture,
/// but grammars without a `meaning` show it as a caption (the evidence stack
/// prints "SOURCE — BEAT2 OBJECT"), and a convention name says nothing.
fn file_noun(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let noun = snake(stem);
    if noun == "thing" || crate::byo::convention(&format!("{stem}.jpg")).is_some() {
        "photo".to_string()
    } else {
        noun
    }
}

/// A compare side or list item: an object when the library has a picture for
/// it, otherwise a phrase showing the words.
fn word_subject(word: &str, pictures: &dyn Pictures) -> Subject {
    let word = word.trim();
    if pictures.is_picture(word) {
        Subject::Object(object(word, None))
    } else {
        Subject::Phrase(Atom {
            value: Some(word.to_string()),
            meaning: None,
        })
    }
}

fn item(word: &str, pictures: &dyn Pictures) -> CollectionItem {
    match word_subject(word, pictures) {
        Subject::Object(o) => CollectionItem::Object(o),
        Subject::Phrase(a) => CollectionItem::Phrase(a),
        _ => unreachable!("word_subject returns an object or a phrase"),
    }
}

/// The on-screen title of a beat without `show`: the `keyword` when it has
/// at most 3 words, else the first [`TITLE_MAX_WORDS`] words of the spoken
/// line, cut back to the last clause end in that window (else ending in "…")
/// — the same cut `taste_rules::display_text` makes for long statements. It is
/// never a compare side or list item: the beat already shows those.
pub fn derived_title(say: &str, keyword: Option<&str>) -> String {
    if let Some(k) = keyword.map(str::trim).filter(|k| !k.is_empty()) {
        if k.split_whitespace().count() <= 3 {
            return k.to_string();
        }
    }
    let words: Vec<&str> = say.split_whitespace().collect();
    if words.len() <= TITLE_MAX_WORDS {
        return words
            .join(" ")
            .trim_end_matches(['.', ',', ';', ':'])
            .to_string();
    }
    let head = &words[..TITLE_MAX_WORDS];
    let cut = head
        .iter()
        .rposition(|w| w.ends_with([',', '.', ';', ':', '?', '!']))
        .filter(|&i| i >= 1);
    let kept = match cut {
        Some(i) => head[..=i].join(" "),
        None => head.join(" "),
    };
    let kept = kept.trim_end_matches([',', ';', ':', '.']).to_string();
    if cut.is_some() {
        kept
    } else {
        format!("{kept}…")
    }
}

/// Map a (checked, auto-fixed) lite story to CreativeIntent v0.2. Pure.
pub fn to_intent(story: &LiteStory, format: Format, pictures: &dyn Pictures) -> Mapped {
    let mut notes = Vec::new();
    let last = story.beats.len().saturating_sub(1);
    let beats = story
        .beats
        .iter()
        .enumerate()
        .map(|(i, b)| map_beat(i, i == last, b, pictures, &mut notes))
        .collect();
    Mapped {
        intent: CreativeIntent {
            version: INTENT_VERSION.to_string(),
            title: intent_title(story),
            format,
            beats,
        },
        notes,
    }
}

fn map_beat(
    i: usize,
    is_last: bool,
    b: &LiteBeat,
    pictures: &dyn Pictures,
    notes: &mut Vec<String>,
) -> Beat {
    let n = i + 1;
    let structure = b.structure();
    let meaning = clean(&b.meaning);
    let number = clean(&b.number);
    let mut dropped: Vec<&str> = Vec::new();
    let mut purpose = Purpose::Emphasize;
    let mut relationship = None;
    let (primary, secondary) = match structure {
        Structure::Layers => {
            let l = b.layers.as_ref().expect("layers");
            let layers = Layers {
                layers: l
                    .names
                    .iter()
                    .map(|name| LayerSpec {
                        name: name.trim().to_string(),
                        note: None,
                        boundary: false,
                    })
                    .collect(),
                meaning: None,
                focus: clean(&l.focus),
            };
            let secondary = match (&b.picture, &number) {
                (Some(p), _) => {
                    if number.is_some() {
                        dropped.push("number");
                    }
                    Some(picture_subject(p, meaning.clone()))
                }
                (None, Some(v)) => Some(Subject::Number(Atom {
                    value: Some(v.clone()),
                    meaning: meaning.clone(),
                })),
                (None, None) => None,
            };
            for (f, set) in [
                ("list", b.list.is_some()),
                ("compare", b.compare.is_some()),
                ("change", b.change.is_some()),
                ("picture2", b.picture2.is_some()),
            ] {
                if set {
                    dropped.push(f);
                }
            }
            (Subject::Layers(layers), secondary)
        }
        Structure::List => {
            let items = b.list.as_ref().expect("list");
            let collection = Collection {
                items: items.iter().map(|w| item(w, pictures)).collect(),
                meaning: meaning.clone(),
            };
            for (f, set) in [
                ("compare", b.compare.is_some()),
                ("change", b.change.is_some()),
                ("number", number.is_some()),
                ("picture", b.picture.is_some()),
                ("picture2", b.picture2.is_some()),
            ] {
                if set {
                    dropped.push(f);
                }
            }
            (Subject::Collection(collection), None)
        }
        Structure::Compare => {
            let c = b.compare.as_ref().expect("compare");
            purpose = Purpose::Compare;
            relationship = Some(c.how.unwrap_or(How::Separate).relationship());
            for (f, set) in [
                ("change", b.change.is_some()),
                ("number", number.is_some()),
                ("picture", b.picture.is_some()),
                ("picture2", b.picture2.is_some()),
            ] {
                if set {
                    dropped.push(f);
                }
            }
            (
                word_subject(&c.a, pictures),
                Some(word_subject(&c.b, pictures)),
            )
        }
        Structure::Change => {
            let c = b.change.as_ref().expect("change");
            for (f, set) in [
                ("number", number.is_some()),
                ("picture", b.picture.is_some()),
                ("picture2", b.picture2.is_some()),
            ] {
                if set {
                    dropped.push(f);
                }
            }
            (
                Subject::StateChange(StateChange {
                    entity: c.what.trim().to_string(),
                    from: c.from.trim().to_string(),
                    to: c.to.trim().to_string(),
                    meaning: meaning.clone(),
                }),
                None,
            )
        }
        Structure::Number => {
            if is_last {
                purpose = Purpose::Reveal;
            }
            if b.picture2.is_some() {
                dropped.push("picture2");
            }
            (
                Subject::Number(Atom {
                    value: number.clone(),
                    meaning: meaning.clone(),
                }),
                b.picture.as_ref().map(|p| picture_subject(p, None)),
            )
        }
        Structure::Picture => (
            picture_subject(b.picture.as_ref().expect("picture"), meaning.clone()),
            b.picture2.as_ref().map(|p| picture_subject(p, None)),
        ),
        Structure::Phrase => {
            if b.picture2.is_some() {
                dropped.push("picture2");
            }
            // The phrase is filled in below, once the statement is known.
            (Subject::Phrase(Atom::default()), None)
        }
    };
    for f in dropped {
        notes.push(format!(
            "beat {n}: '{f}' not shown (the beat shows a {})",
            structure.name()
        ));
    }

    let say = b.say.trim().to_string();
    let keyword = clean(&b.keyword);
    let mut beat = Beat {
        purpose,
        statement: say.clone(),
        primary,
        secondary,
        relationship,
        energy: b.energy.unwrap_or_default(),
        continuity: Continuity::None,
        keyword: keyword.clone(),
        narration: Some(say),
    };
    beat.statement = match clean(&b.show) {
        Some(show) => show,
        None => derived_title(&b.say, beat.keyword.as_deref()),
    };
    if structure == Structure::Phrase {
        let value = clean(&b.show)
            .or(keyword)
            .unwrap_or_else(|| beat.statement.clone());
        beat.primary = Subject::Phrase(Atom {
            value: Some(value),
            meaning: meaning.clone(),
        });
    }
    beat
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Set(&'static [&'static str]);
    impl Pictures for Set {
        fn is_picture(&self, noun: &str) -> bool {
            self.0.contains(&snake(noun).as_str())
        }
    }

    fn beat(json: serde_json::Value) -> LiteBeat {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn snake_and_title() {
        assert_eq!(snake("  Berry Bush!! "), "berry_bush");
        assert_eq!(snake("Ünïcode café"), "n_code_caf");
        assert_eq!(snake("!!!"), "thing");
        let story = LiteStory {
            title: String::new(),
            beats: vec![beat(
                serde_json::json!({"say": "Warren Buffett is sitting on cash."}),
            )],
        };
        assert_eq!(intent_title(&story), "warren_buffett_is_sitting");
    }

    #[test]
    fn mapping_follows_the_table_and_validates() {
        let story: LiteStory = serde_json::from_value(serde_json::json!({
            "title": "buffett cash",
            "beats": [
                {"say": "Warren Buffett is sitting on 381 billion dollars in cash.", "show": "Buffett's cash pile",
                 "number": "$381B", "meaning": "cash reserves", "picture": "money_stack"},
                {"say": "Five tools now run the modern workflow.", "list": ["robot", "microchip", "zorb"]},
                {"say": "Costs fell while output tripled.", "compare": {"a": "cost", "b": "robot", "how": "grow"}, "number": "3x"},
                {"say": "Below the thermocline, the water turns cold and dark.",
                 "layers": {"names": ["sunlit", "twilight", "midnight"], "focus": "twilight"}, "picture": "fish"},
                {"say": "Delivery went from three days to the same day.", "change": {"what": "delivery", "from": "3 days", "to": "same day"}},
                {"say": "That is the whole story of the year in one number.", "number": "36 months"}
            ]
        }))
        .unwrap();
        let m = to_intent(&story, Format::Vertical, &Set(&["robot", "microchip"]));
        let i = &m.intent;
        assert_eq!(i.title, "buffett_cash");
        assert!(i.validate().is_ok(), "{:?}", i.validate());
        assert_eq!(i.beats[0].statement, "Buffett's cash pile");
        assert_eq!(i.beats[0].purpose, Purpose::Emphasize);
        assert!(matches!(i.beats[0].primary, Subject::Number(_)));
        assert_eq!(
            i.beats[0].secondary.as_ref().unwrap().asset(),
            Some("money_stack")
        );
        let Subject::Collection(c) = &i.beats[1].primary else {
            panic!()
        };
        assert!(matches!(c.items[0], CollectionItem::Object(_)));
        assert!(matches!(c.items[2], CollectionItem::Phrase(_)));
        assert_eq!(i.beats[2].purpose, Purpose::Compare);
        // The title never repeats a compare side; it comes from the spoken line.
        assert_eq!(i.beats[2].statement, "Costs fell while output tripled");
        assert_eq!(i.beats[2].relationship, Some(Relationship::Grow));
        assert!(matches!(i.beats[2].primary, Subject::Phrase(_)));
        assert!(matches!(i.beats[2].secondary, Some(Subject::Object(_))));
        assert!(matches!(i.beats[3].primary, Subject::Layers(_)));
        assert!(matches!(i.beats[4].primary, Subject::StateChange(_)));
        assert_eq!(i.beats[5].purpose, Purpose::Reveal);
        assert_eq!(
            m.notes,
            vec!["beat 3: 'number' not shown (the beat shows a compare)".to_string()]
        );
        for b in &i.beats {
            assert!(b.narration.is_some());
            assert!(!b.statement.is_empty());
        }
        // Pure: the same input maps to the same intent.
        assert_eq!(
            m,
            to_intent(&story, Format::Vertical, &Set(&["robot", "microchip"]))
        );
    }

    #[test]
    fn derived_titles_are_short() {
        assert_eq!(
            derived_title(
                "Time does the heavy lifting here: the last ten years add more.",
                None
            ),
            "Time does the heavy lifting here"
        );
        assert_eq!(
            derived_title("Put one thousand dollars away today and wait", None),
            "Put one thousand dollars away today…"
        );
        assert_eq!(
            derived_title("Why does it work?", None),
            "Why does it work?"
        );
        assert_eq!(
            derived_title("anything at all here", Some("compounding")),
            "compounding"
        );
    }

    #[test]
    fn file_pictures_and_phrases() {
        let b = beat(
            serde_json::json!({"say": "We landed at the beach at dawn.", "picture": "file:my_trip/Beach Day.JPG"}),
        );
        assert_eq!(
            b.picture.as_ref().unwrap().file().unwrap().file,
            "my_trip/Beach Day.JPG"
        );
        let story = LiteStory {
            title: "trip".into(),
            beats: vec![
                b,
                beat(
                    serde_json::json!({"say": "And nothing else mattered that week at all, honestly, not one bit.", "keyword": "bliss"}),
                ),
            ],
        };
        let m = to_intent(&story, Format::Square, &Set(&[]));
        assert_eq!(m.intent.beats[0].primary.asset(), Some("beach_day"));
        let Subject::Phrase(a) = &m.intent.beats[1].primary else {
            panic!()
        };
        assert_eq!(a.value.as_deref(), Some("bliss"));
        assert_eq!(m.intent.format, Format::Square);
    }
}
