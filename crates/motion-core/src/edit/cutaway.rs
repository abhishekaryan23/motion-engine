//! (0.24 E2) Cutaway planning: transcript analysis -> candidate cutaways and
//! side panels -> a density-bound selection. Pure and deterministic.
//!
//! A candidate is a span of spoken words plus the CreativeIntent beat(s)
//! generated from them. The compiler (in the CLI) turns the beats into the
//! picture; this module only decides what, where and when.
//!
//! Voice relation (REFERENCE_LEVEL_GAP section 6): a cut lands in the
//! inter-word gap about 200 ms before its clause; a graphic holds at least
//! 0.8 s after the phrase ends and returns to the speaker in a gap.

use std::collections::{BTreeMap, BTreeSet};

use super::analyze::{is_stopword, Analysis, EntityKind, Tok};
use super::plan::{CutBeat, CutawayKind, EditEvent, EventKind, Side};
use crate::intent::{
    Atom, Beat, Collection, CollectionItem, Continuity, Energy, Purpose, Relationship, Subject,
};

/// The cut lands this long before its first word.
pub const CUT_LEAD: f64 = 0.2;
/// A graphic holds this long after its phrase.
pub const HOLD_AFTER: f64 = 0.8;
/// Shortest and longest cutaway (the speaker is back within 12 s).
pub const MIN_LEN: f64 = 2.5;
pub const MAX_LEN: f64 = 11.5;
/// Share of the runtime graphics (cutaways + side panels) aim to cover.
pub const COVER_LO: f64 = 0.30;
pub const COVER_HI: f64 = 0.45;
/// Concepts are at most one per this many seconds (relaxed only to reach COVER_LO).
pub const CONCEPT_SPACING: f64 = 60.0;
/// Every window of this many seconds holds at least one non-caption event.
pub const MAX_QUIET: f64 = 25.0;

const HI_STOP: &[&str] = &[
    "hai",
    "hain",
    "hota",
    "hoti",
    "hote",
    "hoon",
    "tha",
    "thi",
    "the",
    "toh",
    "kuch",
    "kyunki",
    "kyuki",
    "lekin",
    "aapko",
    "aapne",
    "apko",
    "apne",
    "aapka",
    "aapki",
    "humein",
    "hume",
    "humko",
    "hamara",
    "hamare",
    "hamari",
    "humara",
    "mujhe",
    "mera",
    "meri",
    "unka",
    "unki",
    "unke",
    "iska",
    "iske",
    "isse",
    "usse",
    "isme",
    "usme",
    "yahi",
    "wahi",
    "jaise",
    "jaisa",
    "tarah",
    "baare",
    "bhi",
    "nahi",
    "nahin",
    "karna",
    "karne",
    "karte",
    "karta",
    "karti",
    "kiya",
    "kaise",
    "kaisa",
    "kitna",
    "kitne",
    "jabki",
    "agar",
    "phir",
    "fir",
    "abhi",
    "bahut",
    "bohot",
    "thoda",
    "sirf",
    "matlab",
    "wala",
    "wali",
    "waale",
    "aisa",
    "aise",
    "waise",
    "iske",
    "uska",
    "uski",
    "uske",
    "jisse",
    "jinko",
    "jinke",
    "unhein",
    "tumne",
    "tumhare",
    "tumhara",
    "tumhari",
    "hoga",
    "hogi",
    "honge",
    "raha",
    "rahi",
    "rahe",
    "chahiye",
    "sakta",
    "sakte",
    "sakti",
    "padhta",
    "wahan",
    "yahan",
    "kabhi",
    "hamesha",
    "ho",
    "saath",
    "baad",
    "pehle",
    "pahle",
    "bilkul",
    "actually",
    "basically",
    "because",
    "which",
    "there",
    "their",
    "would",
    "could",
    "should",
    "about",
    "these",
    "those",
    "think",
    "being",
    "other",
    "every",
    "hamarah",
    "hoota",
    "cheez",
    "cheeze",
    "johai",
    "kyoki",
    "lagtai",
    "lieta",
    "saktata",
    "niche",
    "andar",
    "chhoti",
    "dousro",
    "dusre",
    "dusron",
    "apeni",
    "bankar",
    "taraf",
    "khatam",
    "lagta",
    "acchi",
    "andaja",
    "kaartata",
    "kaise",
    "bilkul",
    "yeh",
    "woh",
    "isliya",
    "isliye",
];

const ORDINALS: &[&str] = &[
    "pahla", "pehla", "pahli", "pehli", "first", "firstly", "dusra", "doosra", "dousra", "dosra",
    "dusri", "doosri", "second", "secondly", "teesra", "tisra", "teesri", "tesra", "third",
    "thirdly", "chautha", "fourth",
];

const COUNTED: &[&str] = &[
    "tarike", "tarika", "tareeke", "wajah", "wajahein", "reasons", "reason", "ways", "steps",
    "step", "tips", "cheezein", "cheeze", "points", "types", "rules", "mistakes", "things",
    "factors", "methods", "method",
];

const COUNT_WORDS: &[&str] = &[
    "do", "two", "teen", "three", "char", "chaar", "four", "paanch", "five", "2", "3", "4", "5",
];

const SPEECH_VERBS: &[&str] = &["kaha", "kaha.", "bola", "kehte", "kahte", "said", "says"];

const SOURCES: &[&str] = &[
    "research",
    "researcher",
    "researchers",
    "study",
    "studies",
    "report",
    "survey",
    "scientist",
    "scientists",
    "professor",
    "university",
    "philosopher",
    "experiment",
    "book",
    "experts",
    "expert",
];

const CONTRAST: &[&str] = &[
    "vs", "versus", "jabki", "jabke", "whereas", "while", "than", "lekin", "but", "magar", "par",
    "parantu",
];
const CONTRAST_STRONG: &[&str] = &["vs", "versus", "jabki", "jabke", "whereas", "than"];

fn alpha(norm: &str) -> bool {
    !norm.is_empty() && norm.chars().all(|c| c.is_alphabetic())
}

fn content(t: &Tok) -> bool {
    alpha(&t.norm) && !is_stopword(&t.norm) && !HI_STOP.contains(&t.norm.as_str())
}

/// Topic words: repeated content words (a recogniser garbles a word the same
/// way rarely, so a repeat is a good sign the word is real).
pub struct Terms {
    freq: BTreeMap<String, usize>,
}

impl Terms {
    pub fn new(toks: &[Tok]) -> Self {
        let mut freq: BTreeMap<String, usize> = BTreeMap::new();
        for t in toks.iter().filter(|t| content(t) && t.norm.len() >= 5) {
            *freq.entry(t.norm.clone()).or_default() += 1;
        }
        Terms { freq }
    }
    pub fn is_term(&self, t: &Tok) -> bool {
        content(t) && self.freq.get(&t.norm).copied().unwrap_or(0) >= 3
    }
    fn weak(&self, t: &Tok) -> bool {
        content(t) && t.norm.len() >= 4 && self.freq.get(&t.norm).copied().unwrap_or(0) >= 2
    }
}

/// Clause spans `(first, last)` by punctuation and pauses.
pub fn clauses(toks: &[Tok]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut a = 0;
    for i in 0..toks.len() {
        let gap = toks
            .get(i + 1)
            .map(|n| n.start - toks[i].end)
            .unwrap_or(f64::INFINITY);
        let stop = toks[i].raw.ends_with(['.', '!', '?']);
        let comma = toks[i].raw.ends_with([',', ';', ':']) && gap >= 0.2;
        if i + 1 == toks.len() || stop || comma || gap >= 0.45 {
            out.push((a, i));
            a = i + 1;
        }
    }
    out
}

fn words_text(toks: &[Tok], a: usize, z: usize) -> String {
    toks[a..=z]
        .iter()
        .map(|t| t.raw.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

fn upper_term(t: &Tok) -> String {
    t.norm.to_uppercase()
}

fn titled(mut b: Beat, title: &str) -> Beat {
    b.statement = title.to_string();
    b
}

fn atom(value: &str, meaning: Option<&str>) -> Atom {
    Atom {
        value: Some(value.to_string()),
        meaning: meaning.map(str::to_string),
    }
}

fn beat(
    purpose: Purpose,
    toks: &[Tok],
    a: usize,
    z: usize,
    primary: Subject,
    secondary: Option<Subject>,
    energy: Energy,
) -> Beat {
    let said = words_text(toks, a, z);
    Beat {
        purpose,
        statement: said.clone(),
        primary,
        secondary,
        relationship: None,
        energy,
        continuity: Continuity::None,
        keyword: None,
        // The voice-over line IS what the speaker says, so the compiler's
        // speech alignment is exact.
        narration: Some(said),
    }
}

/// One candidate before timing.
#[derive(Debug, Clone)]
pub struct Cand {
    pub sub: CutawayKind,
    pub first: usize,
    pub last: usize,
    pub beats: Vec<CutBeat>,
    /// Number text for figures (a short one becomes a side panel).
    pub number: Option<String>,
}

fn weight(k: CutawayKind) -> u32 {
    match k {
        CutawayKind::Figure | CutawayKind::ListBuild => 5,
        CutawayKind::Compare | CutawayKind::QuoteCard => 4,
        CutawayKind::Concept => 2,
    }
}

fn clause_of(cl: &[(usize, usize)], w: usize) -> (usize, usize) {
    cl.iter()
        .copied()
        .find(|&(a, z)| w >= a && w <= z)
        .unwrap_or((w, w))
}

/// Sentence span containing word `w` (breaks only at . ! ? or 0.7 s pauses).
fn sentence_of(an: &Analysis, w: usize) -> (usize, usize) {
    an.sentences
        .iter()
        .copied()
        .find(|&(a, z)| w >= a && w <= z)
        .unwrap_or((w, w))
}

fn figure_cands(toks: &[Tok], an: &Analysis, terms: &Terms, cl: &[(usize, usize)]) -> Vec<Cand> {
    let mut out = Vec::new();
    for e in &an.entities {
        let is_year = e.kind == EntityKind::Year
            && toks[e.first]
                .norm
                .parse::<u32>()
                .is_ok_and(|y| (1950..=2040).contains(&y));
        if e.kind != EntityKind::Number && !is_year {
            continue;
        }
        let (ca, cz) = clause_of(cl, e.first);
        let a = ca.max(e.first.saturating_sub(5));
        let z = cz.min(e.last + 9).max(e.last);
        let meaning = (a..=z)
            .filter(|&i| i > e.last && terms.weak(&toks[i]))
            .chain((a..e.first).rev().filter(|&i| terms.weak(&toks[i])))
            .map(|i| toks[i].norm.clone())
            .next();
        let value = e.text.clone();
        let b = titled(
            beat(
                Purpose::Emphasize,
                toks,
                a,
                z,
                Subject::Number(atom(&value, meaning.as_deref())),
                None,
                Energy::Impact,
            ),
            &meaning.clone().unwrap_or_else(|| value.clone()),
        );
        out.push(Cand {
            sub: CutawayKind::Figure,
            first: a,
            last: z,
            beats: vec![CutBeat {
                first: a,
                last: z,
                beat: b,
            }],
            number: Some(value),
        });
    }
    out
}

fn compare_cands(toks: &[Tok], an: &Analysis, terms: &Terms) -> Vec<Cand> {
    let mut out = Vec::new();
    let mut last_end = 0usize;
    for m in 0..toks.len() {
        let w = toks[m].norm.as_str();
        if !CONTRAST.contains(&w) || m < last_end {
            continue;
        }
        let (sa, sz) = sentence_of(an, m);
        let lo = m.saturating_sub(6).max(sa);
        let hi = (m + 6).min(sz);
        let a = (lo..m).rev().find(|&i| terms.is_term(&toks[i]));
        let b = (m + 1..=hi).find(|&i| terms.is_term(&toks[i]));
        let (Some(a), Some(b)) = (a, b) else { continue };
        if toks[a].norm == toks[b].norm {
            continue;
        }
        let strong = CONTRAST_STRONG.contains(&w);
        let (ra, rz) = (a.saturating_sub(2).max(sa), (b + 2).min(sz));
        let mut bt = beat(
            Purpose::Contrast,
            toks,
            ra,
            rz,
            Subject::Phrase(atom(&upper_term(&toks[a]), None)),
            Some(Subject::Phrase(atom(&upper_term(&toks[b]), None))),
            Energy::Building,
        );
        bt.relationship = Some(Relationship::Separate);
        let _ = strong;
        bt.statement = format!("{} vs {}", toks[a].norm, toks[b].norm);
        out.push(Cand {
            sub: CutawayKind::Compare,
            first: ra,
            last: rz,
            beats: vec![CutBeat {
                first: ra,
                last: rz,
                beat: bt,
            }],
            number: None,
        });
        last_end = rz;
    }
    out
}

fn list_cands(toks: &[Tok], an: &Analysis, terms: &Terms, cl: &[(usize, usize)]) -> Vec<Cand> {
    let marks: Vec<usize> = (0..toks.len())
        .filter(|&i| ORDINALS.contains(&toks[i].norm.as_str()))
        .collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < marks.len() {
        // A run of markers, each within 24 s of the previous.
        let mut run = vec![marks[k]];
        let mut j = k + 1;
        while j < marks.len() && toks[marks[j]].start - toks[*run.last().unwrap_or(&0)].end < 24.0 {
            run.push(marks[j]);
            j += 1;
        }
        k = j;
        if run.len() < 2 {
            continue;
        }
        // Items: (marker, end of its first sentence, headline words).
        let mut items: Vec<(usize, usize, String)> = Vec::new();
        for (n, &m) in run.iter().take(4).enumerate() {
            let next = run.get(n + 1).copied().unwrap_or(toks.len());
            let (_, sz) = sentence_of(an, m);
            let cap = (m + 20).min(next.saturating_sub(1)).min(toks.len() - 1);
            let mut z = sz.min(cap);
            if z < m + 3 {
                // Too short: run on to the end of the next clause.
                z = clause_of(cl, (m + 3).min(toks.len() - 1)).1.min(cap).max(m);
            }
            let pick = |f: &dyn Fn(&Tok) -> bool| -> Vec<String> {
                (m + 1..=z)
                    .filter(|&i| f(&toks[i]))
                    .take(3)
                    .map(|i| toks[i].norm.to_uppercase())
                    .collect()
            };
            let mut words = pick(&|t| terms.is_term(t));
            if words.is_empty() {
                words = pick(&|t| terms.weak(t));
            }
            if words.is_empty() {
                words = pick(&|t| content(t));
            }
            if words.is_empty() {
                continue;
            }
            items.push((m, z, words.join(" ")));
        }
        if items.len() < 2 {
            continue;
        }
        // One cutaway per item: the list so far, the newest item spoken in it.
        for n in 0..items.len() {
            let (m, z, _) = items[n];
            let first = if n == 0 {
                // Include a count cue ("do tarike hain") just before the first marker.
                (m.saturating_sub(10)..m)
                    .find(|&i| {
                        COUNT_WORDS.contains(&toks[i].norm.as_str())
                            && toks
                                .get(i + 1)
                                .is_some_and(|x| COUNTED.contains(&x.norm.as_str()))
                    })
                    .unwrap_or(m)
            } else {
                m
            };
            let col = Collection {
                items: items[..=n]
                    .iter()
                    .map(|x| CollectionItem::Phrase(atom(&x.2, None)))
                    .collect(),
                meaning: None,
            };
            if col.items.len() < 2 {
                // The first item alone is a concept-sized graphic: pair it with the next.
                let mut two = col.items.clone();
                two.push(CollectionItem::Phrase(atom(&items[1].2, None)));
                let b = beat(
                    Purpose::Explain,
                    toks,
                    first,
                    z,
                    Subject::Collection(Collection {
                        items: two,
                        meaning: None,
                    }),
                    None,
                    Energy::Building,
                );
                let b = titled(b, &items[n].2);
                out.push(Cand {
                    sub: CutawayKind::ListBuild,
                    first,
                    last: z,
                    beats: vec![CutBeat {
                        first,
                        last: z,
                        beat: b,
                    }],
                    number: None,
                });
                continue;
            }
            let b = beat(
                Purpose::Explain,
                toks,
                first,
                z,
                Subject::Collection(col),
                None,
                Energy::Building,
            );
            let b = titled(b, &items[n].2);
            out.push(Cand {
                sub: CutawayKind::ListBuild,
                first,
                last: z,
                beats: vec![CutBeat {
                    first,
                    last: z,
                    beat: b,
                }],
                number: None,
            });
        }
    }
    out
}

fn is_proper(t: &Tok) -> bool {
    t.raw
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase())
        && t.norm.len() >= 3
}

fn quote_cands(toks: &[Tok], an: &Analysis) -> Vec<Cand> {
    let mut out = Vec::new();
    let mut used_until = 0usize;
    for m in 0..toks.len() {
        let w = toks[m].norm.as_str();
        let verb = SPEECH_VERBS.contains(&w);
        let src = SOURCES.contains(&w);
        if (!verb && !src) || m < used_until {
            continue;
        }
        let (sa, sz) = sentence_of(an, m);
        let (a, z, claim_from, who): (usize, usize, usize, String);
        if verb {
            // "Socrates ne kaha tha, <claim>"
            let mut c = m + 1;
            while c <= sz && matches!(toks[c].norm.as_str(), "tha" | "thi" | "ki" | "hai" | "ne") {
                c += 1;
            }
            let end = (c + 13).min(toks.len() - 1);
            let zz = (c..=end)
                .find(|&i| toks[i].raw.ends_with(['.', '!', '?']))
                .unwrap_or(end);
            if zz < c + 3 {
                continue;
            }
            let names: Vec<String> = (sa.max(m.saturating_sub(5))..m)
                .filter(|&i| is_proper(&toks[i]) && i != sa)
                .chain((sa.max(m.saturating_sub(5))..m).filter(|&i| i == sa && is_proper(&toks[i])))
                .map(|i| {
                    toks[i]
                        .raw
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_string()
                })
                .collect();
            who = names.join(" ");
            a = sa.max(m.saturating_sub(5));
            z = zz;
            claim_from = c;
        } else {
            // "... Yale University ka researcher ne ..." the sentence is the claim.
            let zz = (m + 14).min(sz);
            if zz < m + 3 {
                continue;
            }
            let near: Vec<String> = (m.saturating_sub(2)..=(m + 1).min(toks.len() - 1))
                .filter(|&i| is_proper(&toks[i]))
                .map(|i| {
                    toks[i]
                        .raw
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_string()
                })
                .collect();
            who = if near.is_empty() {
                let mut s = toks[m].norm.clone();
                if let Some(f) = s.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                s
            } else {
                near.join(" ")
            };
            a = m.saturating_sub(4).max(sa);
            z = zz;
            claim_from = m;
        }
        let claim = words_text(toks, claim_from, z);
        let claim = claim.trim_end_matches(['.', ',', ';', '!']).to_string();
        let who = if who.trim().is_empty() {
            "Source".to_string()
        } else {
            who
        };
        let b = titled(
            beat(
                Purpose::Explain,
                toks,
                a,
                z,
                Subject::Phrase(atom(&claim, Some(&who))),
                None,
                Energy::Calm,
            ),
            &who,
        );
        used_until = z + 1;
        out.push(Cand {
            sub: CutawayKind::QuoteCard,
            first: a,
            last: z,
            beats: vec![CutBeat {
                first: a,
                last: z,
                beat: b,
            }],
            number: None,
        });
    }
    out
}

fn concept_cands(toks: &[Tok], an: &Analysis, terms: &Terms) -> Vec<Cand> {
    let mut out = Vec::new();
    let clause_list = clauses(toks);
    let stressed: BTreeSet<usize> = an.stresses.iter().map(|x| x.0).collect();
    for &(sa, sz) in &an.sentences {
        if sz < sa + 4 {
            continue;
        }
        let z = sz.min(sa + 16);
        let mut seen = BTreeSet::new();
        // Topic words first; a stressed, longer content word otherwise.
        let mut ts: Vec<(usize, String)> = (sa..=z)
            .filter(|&i| terms.is_term(&toks[i]) || terms.weak(&toks[i]))
            .filter(|&i| seen.insert(toks[i].norm.clone()))
            .take(3)
            .map(|i| (i, upper_term(&toks[i])))
            .collect();
        if ts.is_empty() {
            ts = (sa..=z)
                .filter(|&i| {
                    stressed.contains(&i) && content(&toks[i]) && toks[i].norm.chars().count() >= 6
                })
                .take(2)
                .map(|i| (i, upper_term(&toks[i])))
                .collect();
        }
        if ts.is_empty() {
            // The longest content word of the sentence (a weak but real hook).
            ts = (sa..=z)
                .filter(|&i| content(&toks[i]) && toks[i].norm.chars().count() >= 7)
                .max_by_key(|&i| toks[i].norm.len())
                .map(|i| vec![(i, upper_term(&toks[i]))])
                .unwrap_or_default();
        }
        if ts.is_empty() {
            continue;
        }
        let ts: Vec<String> = ts.into_iter().map(|x| x.1).collect();
        // The key term is the title; the clause around it is the supporting line.
        let hit = (sa..=z)
            .find(|&i| ts.iter().any(|t| *t == upper_term(&toks[i])))
            .unwrap_or(sa);
        let (ca, cz) = clause_of(&clause_list, hit);
        let support = words_text(toks, ca, cz.min(ca + 7))
            .trim_end_matches(['.', ',', ';', '!', '?'])
            .to_string();
        let b = titled(
            beat(
                Purpose::Emphasize,
                toks,
                sa,
                z,
                Subject::Phrase(atom(&support, None)),
                None,
                Energy::Building,
            ),
            &ts.join(" "),
        );
        out.push(Cand {
            sub: CutawayKind::Concept,
            first: sa,
            last: z,
            beats: vec![CutBeat {
                first: sa,
                last: z,
                beat: b,
            }],
            number: None,
        });
    }
    out
}

/// Seconds `(start, end)` of a candidate on the source timeline: the cut lands
/// in the gap before the first word, the return in a gap after the hold.
pub fn timing(toks: &[Tok], fps: u32, c: &Cand, duration: f64) -> (f64, f64) {
    let first = &toks[c.first];
    let prev_end = if c.first == 0 {
        0.0
    } else {
        toks[c.first - 1].end
    };
    let mut start = (first.start - CUT_LEAD).clamp(prev_end.min(first.start), first.start);
    if first.start - prev_end > 0.0 && first.start - prev_end < CUT_LEAD {
        start = prev_end + (first.start - prev_end) / 2.0;
    }
    let min_end = toks[c.last].end + HOLD_AFTER;
    let mut end = min_end;
    // Return in the first real gap after the hold.
    for i in c.last..toks.len().saturating_sub(1) {
        if toks[i].end < min_end {
            continue;
        }
        let gap = toks[i + 1].start - toks[i].end;
        if gap >= 0.12 {
            end = toks[i].end + gap / 2.0;
            break;
        }
        if toks[i].end > min_end + 2.5 {
            break;
        }
    }
    if end - start < MIN_LEN {
        end = start + MIN_LEN;
    }
    let f = fps as f64;
    let q = |t: f64| (t * f).round() / f;
    (q(start.max(0.0)), q(end.min(duration)))
}

/// Everything the planner needs.
pub struct CutInput<'a> {
    pub toks: &'a [Tok],
    pub analysis: &'a Analysis,
    pub fps: u32,
    pub duration: f64,
    /// Nothing before this (the lower third and the speaker's hello).
    pub earliest: f64,
}

pub struct CutPlan {
    pub cutaways: Vec<EditEvent>,
    /// Number stacks for figures too short (or too crowded) for a cutaway.
    pub panels: Vec<EditEvent>,
}

fn event(c: &Cand, start: f64, end: f64) -> EditEvent {
    EditEvent {
        kind: EventKind::Cutaway {
            sub: c.sub,
            beats: c.beats.clone(),
        },
        at_word: c.first,
        until_word: c.last,
        start,
        end,
        side: Side::Left,
    }
}

/// Split `1.5%`, `₹40 lakh crore` into the panel value and label.
fn panel_of(value: &str, fallback: &str) -> (String, String, Option<f32>) {
    let (v, l) = match value.split_once(' ') {
        Some((a, b)) => (a.to_string(), b.to_uppercase()),
        None => (value.to_string(), fallback.to_uppercase()),
    };
    let bar = v
        .strip_suffix('%')
        .and_then(|p| p.parse::<f32>().ok())
        .map(|p| (p / 100.0).clamp(0.02, 1.0));
    (v, l, bar)
}

pub fn plan_cutaways(inp: &CutInput) -> CutPlan {
    let toks = inp.toks;
    let mut plan = CutPlan {
        cutaways: Vec::new(),
        panels: Vec::new(),
    };
    if toks.len() < 20 {
        return plan;
    }
    let terms = Terms::new(toks);
    let cl = clauses(toks);
    let mut cands: Vec<Cand> = Vec::new();
    cands.extend(figure_cands(toks, inp.analysis, &terms, &cl));
    cands.extend(compare_cands(toks, inp.analysis, &terms));
    cands.extend(list_cands(toks, inp.analysis, &terms, &cl));
    cands.extend(quote_cands(toks, inp.analysis));
    cands.extend(concept_cands(toks, inp.analysis, &terms));
    let latest = inp.duration - 8.0;
    // (cand, start, end), only those that fit.
    let mut timed: Vec<(Cand, f64, f64)> = Vec::new();
    for c in cands {
        let (s, e) = timing(toks, inp.fps, &c, inp.duration);
        if s < inp.earliest || e > latest || e - s > MAX_LEN {
            continue;
        }
        timed.push((c, s, e));
    }
    timed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let covered = |v: &[(Cand, f64, f64)]| v.iter().map(|x| x.2 - x.1).sum::<f64>();
    let fits = |v: &[(Cand, f64, f64)], s: f64, e: f64, gap: f64| {
        v.iter().all(|x| e + gap <= x.1 || s >= x.2 + gap)
    };
    // Even spread: one cutaway per slot, the strongest that fits. Wider slots
    // first; a pass that leaves coverage under COVER_LO is redone with
    // narrower slots and looser concept spacing.
    const PASSES: [(f64, f64, f64); 6] = [
        (18.0, 3.5, CONCEPT_SPACING),
        (14.0, 3.0, 22.0),
        (11.0, 2.6, 14.0),
        (9.0, 2.5, 10.0),
        (7.0, 2.5, 8.0),
        (5.5, 2.5, 8.0),
    ];
    let mut chosen: Vec<(Cand, f64, f64)> = Vec::new();
    let mut best_pass: Vec<(Cand, f64, f64)> = Vec::new();
    for (slot, gap, spacing) in PASSES {
        if covered(&chosen) > covered(&best_pass) {
            best_pass = chosen.clone();
        }
        chosen.clear();
        let mut t = inp.earliest;
        while t < latest {
            let mut best: Option<usize> = None;
            for (k, (c, s, e)) in timed.iter().enumerate() {
                if *s < t || *s >= t + slot {
                    continue;
                }
                if !fits(&chosen, *s, *e, gap) {
                    continue;
                }
                if c.sub == CutawayKind::Concept
                    && !chosen
                        .iter()
                        .filter(|x| x.0.sub == CutawayKind::Concept)
                        .all(|x| (x.1 - s).abs() >= spacing)
                {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some(b) => weight(c.sub) > weight(timed[b].0.sub),
                };
                if better {
                    best = Some(k);
                }
            }
            match best {
                Some(k) => {
                    let (c, s, e) = timed[k].clone();
                    t = e + gap;
                    chosen.push((c, s, e));
                }
                None => t += slot,
            }
        }
        if covered(&chosen) / inp.duration >= COVER_LO {
            best_pass.clear();
            break;
        }
    }
    if covered(&best_pass) > covered(&chosen) {
        chosen = best_pass;
    }
    // Over the ceiling: drop the weakest, latest first.
    while covered(&chosen) / inp.duration > COVER_HI {
        let Some((k, _)) = chosen.iter().enumerate().min_by(|a, b| {
            weight(a.1 .0.sub).cmp(&weight(b.1 .0.sub)).then(
                b.1 .1
                    .partial_cmp(&a.1 .1)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        }) else {
            break;
        };
        chosen.remove(k);
    }
    chosen.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    // A figure under 2.5 s of cutaway time, or squeezed out, becomes a number stack.
    let mut cut: Vec<(Cand, f64, f64)> = Vec::new();
    for (c, s, e) in chosen {
        if c.sub == CutawayKind::Figure && e - s < MIN_LEN + 0.01 && toks[c.last].end - s < 2.0 {
            continue;
        }
        cut.push((c, s, e));
    }
    for (c, s, e) in &cut {
        plan.cutaways.push(event(c, *s, *e));
    }
    // Number stacks beside the speaker for figures that did not get a cutaway.
    for (c, _, _) in timed.iter().filter(|x| x.0.sub == CutawayKind::Figure) {
        if plan.cutaways.iter().any(|e| e.at_word == c.first) {
            continue;
        }
        let Some(value) = &c.number else { continue };
        let ws = toks[c.first].start;
        let s = ((ws - 0.1).max(0.0) * inp.fps as f64).round() / inp.fps as f64;
        let e = s + 3.2;
        let clash = plan
            .cutaways
            .iter()
            .chain(plan.panels.iter())
            .any(|x| s < x.end + 0.3 && e + 0.3 > x.start);
        if clash || e > inp.duration {
            continue;
        }
        let meaning = match &c.beats[0].beat.primary {
            Subject::Number(a) => a.meaning.clone().unwrap_or_default(),
            _ => String::new(),
        };
        let (v, l, bar) = panel_of(value, &meaning);
        plan.panels.push(EditEvent {
            kind: EventKind::SidePanel {
                value: v,
                label: l,
                bar,
            },
            at_word: c.first,
            until_word: c.last,
            start: s,
            end: e,
            side: Side::Right,
        });
    }
    plan
}

/// Key-idea side panels for stretches with no event (every `MAX_QUIET` s).
/// `busy` are the non-caption event spans so far. Returns the panels to add.
pub fn fill_quiet(
    toks: &[Tok],
    an: &Analysis,
    fps: u32,
    duration: f64,
    busy: &[(f64, f64)],
) -> Vec<EditEvent> {
    let terms = Terms::new(toks);
    let mut spans: Vec<(f64, f64)> = busy.to_vec();
    spans.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::new();
    let mut t = 0.0f64;
    let q = |x: f64| (x * fps as f64).round() / fps as f64;
    loop {
        // End of the quiet stretch starting at `t`.
        let next = spans
            .iter()
            .filter(|s| s.1 > t)
            .map(|s| s.0.max(t))
            .fold(f64::INFINITY, f64::min);
        let quiet_end = next.min(duration);
        if quiet_end - t > MAX_QUIET {
            // A stressed term near t + 18 s.
            let target = t + 18.0;
            let pick = an
                .stresses
                .iter()
                .filter(|&&(w, _)| terms.is_term(&toks[w]))
                .map(|&(w, _)| w)
                .filter(|&w| toks[w].start > t + 10.0 && toks[w].start < quiet_end - 4.0)
                .min_by(|&a, &b| {
                    (toks[a].start - target)
                        .abs()
                        .partial_cmp(&(toks[b].start - target).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .or_else(|| {
                    (0..toks.len())
                        .filter(|&w| {
                            terms.is_term(&toks[w])
                                && toks[w].start > t + 10.0
                                && toks[w].start < quiet_end - 4.0
                        })
                        .min_by(|&a, &b| {
                            (toks[a].start - target)
                                .abs()
                                .partial_cmp(&(toks[b].start - target).abs())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                });
            match pick {
                Some(w) => {
                    let s = q(toks[w].start);
                    let e = q(s + 2.8);
                    out.push(EditEvent {
                        kind: EventKind::SidePanel {
                            value: upper_term(&toks[w]),
                            label: "KEY IDEA".into(),
                            bar: None,
                        },
                        at_word: w,
                        until_word: w,
                        start: s,
                        end: e,
                        side: if out.len() % 2 == 0 {
                            Side::Right
                        } else {
                            Side::Left
                        },
                    });
                    spans.push((s, e));
                    t = e;
                    continue;
                }
                None => t += MAX_QUIET,
            }
        } else {
            // Jump to the end of the next busy span.
            let end = spans
                .iter()
                .filter(|s| s.0 <= quiet_end + 1e-9 && s.1 > t)
                .map(|s| s.1)
                .fold(t, f64::max);
            if end <= t || end >= duration {
                break;
            }
            t = end;
        }
        if t >= duration {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::analyze::analyse;

    fn talk() -> Vec<Tok> {
        // 150 words at 0.45 s: a list, a comparison, a source and numbers.
        let text = "Hello everyone welcome back today we talk about confidence and competence. \
            Familiarity gives confidence while competence needs real practice and practice. \
            There are two practical tarike hain. Pahla the interrogator method for practice, practice matters. \
            Dusra the blank paper method for confidence, confidence needs paper. \
            In 2002 a Yale University researcher tested familiarity and confidence with students. \
            Socrates ne kaha tha, real wisdom is knowing that you know nothing at all today. \
            Growth fell to 35% in March for competence and confidence among students. \
            So familiarity and competence and confidence and practice matter for everyone watching this video today thanks.";
        let mut t = 1.0;
        text.split_whitespace()
            .map(|w| {
                let tok = Tok::new(w, t, t + 0.32);
                t += 0.45 + if w.ends_with(['.', ',']) { 0.35 } else { 0.0 };
                tok
            })
            .collect()
    }

    fn run() -> (Vec<Tok>, CutPlan) {
        let toks = talk();
        let a = analyse(&toks, &[]);
        let dur = toks.last().map(|t| t.end).unwrap_or(0.0) + 3.0;
        let p = plan_cutaways(&CutInput {
            toks: &toks,
            analysis: &a,
            fps: 30,
            duration: dur,
            earliest: 4.0,
        });
        (toks, p)
    }

    #[test]
    fn variety_and_density_rules() {
        let (toks, p) = run();
        assert!(!p.cutaways.is_empty());
        let kinds: BTreeSet<&str> = p
            .cutaways
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Cutaway { sub, .. } => Some(sub.name()),
                _ => None,
            })
            .collect();
        assert!(kinds.len() >= 2, "{kinds:?}");
        for w in p.cutaways.windows(2) {
            assert!(w[1].start >= w[0].end + 2.4, "back to back");
        }
        for e in &p.cutaways {
            assert!(e.end - e.start <= MAX_LEN + 0.05 && e.end - e.start >= MIN_LEN - 0.05);
        }
        let dur = toks.last().map(|t| t.end).unwrap_or(0.0) + 3.0;
        let cover: f64 = p.cutaways.iter().map(|e| e.end - e.start).sum::<f64>() / dur;
        assert!(cover <= COVER_HI, "cover {cover}");
    }

    #[test]
    fn cuts_land_in_the_gap_before_the_clause_and_hold_after() {
        let (toks, p) = run();
        for e in &p.cutaways {
            let first = &toks[e.at_word];
            let prev_end = if e.at_word == 0 {
                0.0
            } else {
                toks[e.at_word - 1].end
            };
            assert!(e.start <= first.start + 1.0 / 30.0, "cut before the clause");
            assert!(
                e.start + 1e-6 >= prev_end - 1.0 / 30.0,
                "cut inside the previous word: {} vs {} / {}",
                e.start,
                prev_end,
                first.start
            );
            assert!(first.start - e.start <= CUT_LEAD + 1.0 / 30.0 + 1e-6);
            assert!(e.end + 1e-6 >= toks[e.until_word].end + HOLD_AFTER - 1.0 / 30.0);
        }
    }

    #[test]
    fn planning_is_deterministic() {
        let (_, a) = run();
        let (_, b) = run();
        assert_eq!(
            serde_json::to_string(&a.cutaways).unwrap(),
            serde_json::to_string(&b.cutaways).unwrap()
        );
    }
}
