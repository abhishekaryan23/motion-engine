//! Transcript analysis: sentences, pauses, stresses and entities by fixed rules.

use serde::{Deserialize, Serialize};

/// Pauses at least this long (seconds) are cut/stress candidates.
pub const MIN_PAUSE: f64 = 0.18;

/// One transcript word with a normalised form for matching.
#[derive(Debug, Clone, PartialEq)]
pub struct Tok {
    pub raw: String,
    /// Lower-case, edge punctuation trimmed (currency symbols and `%` kept).
    pub norm: String,
    pub start: f64,
    pub end: f64,
}

impl Tok {
    pub fn new(raw: &str, start: f64, end: f64) -> Self {
        let norm = raw
            .trim_matches(|c: char| !(c.is_alphanumeric() || "₹$€£%".contains(c)))
            .to_lowercase();
        Tok {
            raw: raw.trim().to_string(),
            norm,
            start,
            end,
        }
    }
    fn ends_sentence(&self) -> bool {
        self.raw.ends_with(['.', '!', '?'])
    }
    fn capitalised(&self) -> bool {
        self.raw
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Number,
    Year,
    Comparison,
    ListCue,
    Proper,
    Question,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub kind: EntityKind,
    pub first: usize,
    pub last: usize,
    /// Display text (numbers: `₹40 lakh crore`, `3.5%`).
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pause {
    /// The word before the pause.
    pub after: usize,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// `(first, last)` word indices, inclusive.
    pub sentences: Vec<(usize, usize)>,
    pub pauses: Vec<Pause>,
    /// `(word, score_db)` for voice-band loudness peaks within their sentence.
    pub stresses: Vec<(usize, f64)>,
    pub entities: Vec<Entity>,
}

const CURRENCY: &str = "₹$€£";
const UNITS: &[&str] = &[
    "lakh", "lakhs", "crore", "crores", "hazaar", "thousand", "hundred", "million", "billion",
    "trillion", "percent", "dollars", "dollar", "rupees", "rupee", "euros", "pounds", "bucks",
    "rs", "inr", "usd",
];
const COUNTRIES: &[&str] = &[
    "india",
    "china",
    "japan",
    "russia",
    "germany",
    "france",
    "italy",
    "spain",
    "brazil",
    "canada",
    "australia",
    "mexico",
    "indonesia",
    "pakistan",
    "bangladesh",
    "nepal",
    "iran",
    "iraq",
    "israel",
    "egypt",
    "turkey",
    "ukraine",
    "poland",
    "argentina",
    "nigeria",
    "kenya",
    "vietnam",
    "thailand",
    "singapore",
    "malaysia",
    "korea",
    "america",
    "britain",
    "england",
    "europe",
    "asia",
    "africa",
    "dubai",
    "qatar",
    "kuwait",
    "oman",
    "saudi",
    "taiwan",
    "maharashtra",
    "gujarat",
    "kerala",
    "punjab",
    "rajasthan",
    "bihar",
    "assam",
    "karnataka",
    "tamil",
    "bengal",
    "odisha",
    "haryana",
    "delhi",
    "mumbai",
    "bengaluru",
    "kolkata",
    "chennai",
    "hyderabad",
];
const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "so", "of", "to", "in", "on", "at", "for", "with", "is",
    "are", "was", "were", "be", "been", "it", "its", "this", "that", "these", "those", "i", "you",
    "he", "she", "we", "they", "me", "my", "your", "our", "their", "his", "her", "as", "by",
    "from", "if", "then", "than", "not", "no", "do", "does", "did", "have", "has", "had", "will",
    "would", "can", "could", "just", "like", "very", "really", "about", "what", "when", "where",
    "which", "who", "how", "why", "there", "here", "also", "all", "some", "um", "uh", "okay",
    "yeah", "right", "now", "up", "out", "into", "over", "more", "much", "get", "got", "one",
    "going", "gonna", "don't", "it's", "that's", "i'm", "you're", "we're",
];

pub fn is_stopword(norm: &str) -> bool {
    STOP.contains(&norm) || norm.len() < 3
}

const ONES: &[(&str, u32)] = &[
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];

fn ones(w: &str) -> Option<u32> {
    ONES.iter().find(|(n, _)| *n == w).map(|(_, v)| *v)
}

/// Digits form of a numeral token: optional currency prefix, digits with `,`/`.`,
/// optional `%` or short scale suffix. Returns `(has_symbol_or_suffix, digits_only)`.
fn numeral(norm: &str) -> Option<(bool, String)> {
    let mut s = norm;
    let mut marked = false;
    if let Some(c) = s.chars().next().filter(|c| CURRENCY.contains(*c)) {
        s = &s[c.len_utf8()..];
        marked = true;
    }
    if let Some(r) = s.strip_suffix('%') {
        s = r;
        marked = true;
    }
    for suf in ["bn", "cr", "k", "m", "b"] {
        if let Some(r) = s.strip_suffix(suf) {
            if r.chars().last().is_some_and(|c| c.is_ascii_digit()) {
                s = r;
                marked = true;
                break;
            }
        }
    }
    let ok = !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
        && s.chars().filter(|c| *c == '.').count() <= 1;
    ok.then(|| (marked, s.to_string()))
}

/// A spelled number at `i` ("forty", "two hundred fifty"): `(value, tokens used)`.
fn spelled(toks: &[Tok], i: usize) -> Option<(u64, usize)> {
    let (mut total, mut cur, mut used, mut seen) = (0u64, 0u64, 0usize, false);
    let mut k = i;
    while k < toks.len() {
        let w = toks[k].norm.as_str();
        if let Some(v) = ones(w) {
            cur += u64::from(v);
            seen = true;
        } else if w == "hundred" && seen {
            cur = cur.max(1) * 100;
        } else if w == "thousand" && seen {
            total += cur.max(1) * 1000;
            cur = 0;
        } else if w == "and" && seen && toks.get(k + 1).is_some_and(|n| ones(&n.norm).is_some()) {
            // "two hundred and fifty"
        } else {
            break;
        }
        k += 1;
        used = k - i;
    }
    seen.then_some((total + cur, used))
}

fn unit_run(toks: &[Tok], mut j: usize) -> usize {
    let start = j;
    while j < toks.len() && j - start < 3 {
        let w = toks[j].norm.as_str();
        if UNITS.contains(&w) {
            j += 1;
        } else if w == "per" && toks.get(j + 1).is_some_and(|n| n.norm == "cent") {
            j += 2;
        } else {
            break;
        }
    }
    j - start
}

fn number_text(prefix: &str, toks: &[Tok], from: usize, to: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !prefix.is_empty() {
        parts.push(prefix.to_string());
    }
    let mut k = from;
    while k <= to {
        let w = toks[k].norm.as_str();
        let pct =
            w == "percent" || (w == "per" && toks.get(k + 1).is_some_and(|n| n.norm == "cent"));
        if pct {
            match parts.last_mut() {
                Some(p) => p.push('%'),
                None => parts.push("%".into()),
            }
            k += if w == "per" { 2 } else { 1 };
        } else {
            parts.push(w.to_string());
            k += 1;
        }
    }
    parts.join(" ")
}

fn entities(toks: &[Tok], sentences: &[(usize, usize)]) -> Vec<Entity> {
    let mut out: Vec<Entity> = Vec::new();
    let n = toks.len();
    let mut taken = vec![false; n];
    let sentence_start: Vec<usize> = sentences.iter().map(|s| s.0).collect();

    // Numbers and years.
    let mut i = 0;
    while i < n {
        let w = &toks[i].norm;
        if let Some((marked, digits)) = numeral(w) {
            let units = unit_run(toks, i + 1);
            let last = i + units;
            let year = !marked
                && units == 0
                && digits.len() == 4
                && digits.chars().all(|c| c.is_ascii_digit())
                && (1900..=2100).contains(&digits.parse::<u32>().unwrap_or(0));
            // A big written number ("50,000", "250000") is a callout on its own.
            let big = !year && (w.contains(',') || digits.len() >= 5) && !w.contains(':');
            if marked || units > 0 || big {
                out.push(Entity {
                    kind: EntityKind::Number,
                    first: i,
                    last,
                    text: number_text("", toks, i, last),
                });
            } else if year {
                out.push(Entity {
                    kind: EntityKind::Year,
                    first: i,
                    last: i,
                    text: digits,
                });
            } else {
                i += 1;
                continue;
            }
            (i..=last).for_each(|k| taken[k] = true);
            i = last + 1;
            continue;
        }
        if let Some((value, used)) = spelled(toks, i) {
            let units = unit_run(toks, i + used);
            if units > 0 {
                let last = i + used + units - 1;
                out.push(Entity {
                    kind: EntityKind::Number,
                    first: i,
                    last,
                    text: number_text(&value.to_string(), toks, i + used, last),
                });
                (i..=last).for_each(|k| taken[k] = true);
                i = last + 1;
                continue;
            }
        }
        i += 1;
    }

    // Comparisons and list cues.
    for i in 0..n {
        let w = toks[i].norm.as_str();
        if matches!(w, "vs" | "versus" | "v/s") && i > 0 && i + 1 < n {
            let last = (i + 1).min(n - 1);
            out.push(Entity {
                kind: EntityKind::Comparison,
                first: i - 1,
                last,
                text: join(toks, i - 1, last),
            });
        } else if w == "than" && i > 0 && i + 1 < n {
            let p = toks[i - 1].norm.as_str();
            let cmp = matches!(
                p,
                "more"
                    | "less"
                    | "fewer"
                    | "better"
                    | "worse"
                    | "higher"
                    | "lower"
                    | "bigger"
                    | "smaller"
                    | "cheaper"
                    | "faster"
                    | "slower"
                    | "greater"
            ) || (p.len() > 4 && p.ends_with("er"));
            if cmp {
                let last = (i + 2).min(n - 1);
                out.push(Entity {
                    kind: EntityKind::Comparison,
                    first: i - 1,
                    last,
                    text: join(toks, i - 1, last),
                });
            }
        }
        let count_word = matches!(
            w,
            "two"
                | "three"
                | "four"
                | "five"
                | "six"
                | "seven"
                | "eight"
                | "nine"
                | "ten"
                | "2"
                | "3"
                | "4"
                | "5"
                | "6"
                | "7"
                | "8"
                | "9"
                | "10"
        );
        if count_word
            && toks.get(i + 1).is_some_and(|x| {
                matches!(
                    x.norm.as_str(),
                    "reasons"
                        | "things"
                        | "ways"
                        | "steps"
                        | "points"
                        | "tips"
                        | "mistakes"
                        | "lessons"
                        | "types"
                        | "rules"
                        | "factors"
                )
            })
        {
            out.push(Entity {
                kind: EntityKind::ListCue,
                first: i,
                last: i + 1,
                text: join(toks, i, i + 1),
            });
        } else if matches!(w, "first" | "firstly") && sentence_start.contains(&i) {
            out.push(Entity {
                kind: EntityKind::ListCue,
                first: i,
                last: i,
                text: w.to_string(),
            });
        }
    }

    // Questions.
    for &(a, b) in sentences {
        if toks[b].raw.ends_with('?') {
            out.push(Entity {
                kind: EntityKind::Question,
                first: a,
                last: b,
                text: join(toks, a, b),
            });
        }
    }

    // Proper nouns: capitalised spans not at a sentence start, plus places.
    let mut i = 0;
    while i < n {
        let w = toks[i].norm.as_str();
        let place = COUNTRIES.contains(&w);
        let cap = toks[i].capitalised()
            && !sentence_start.contains(&i)
            && w != "i"
            && !is_stopword(w)
            && !taken[i]
            && !w.starts_with("i'");
        if place || cap {
            let mut j = i;
            while j + 1 < n
                && !toks[j].ends_sentence()
                && !toks[j].raw.ends_with(',')
                && toks[j + 1].capitalised()
                && !is_stopword(&toks[j + 1].norm)
                && !taken[j + 1]
            {
                j += 1;
            }
            out.push(Entity {
                kind: EntityKind::Proper,
                first: i,
                last: j,
                text: join(toks, i, j),
            });
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out.sort_by_key(|e| (e.first, e.last));
    out
}

fn join(toks: &[Tok], a: usize, b: usize) -> String {
    toks[a..=b]
        .iter()
        .map(|t| {
            t.raw
                .trim_matches(|c: char| !(c.is_alphanumeric() || "₹$€£%".contains(c)))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sentences(toks: &[Tok]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut a = 0;
    for i in 0..toks.len() {
        let gap_after = toks
            .get(i + 1)
            .map(|n| n.start - toks[i].end)
            .unwrap_or(f64::INFINITY);
        if toks[i].ends_sentence() || gap_after >= 0.7 || i + 1 == toks.len() {
            out.push((a, i));
            a = i + 1;
        }
    }
    out
}

/// Loudness peaks per word: the word's peak level (dB, 10 ms hop in `env_db`)
/// minus the median level of its sentence's words, kept at `>= 4.5 dB`.
fn stresses(toks: &[Tok], sents: &[(usize, usize)], env_db: &[f64]) -> Vec<(usize, f64)> {
    if env_db.is_empty() {
        return Vec::new();
    }
    let peak = |t: &Tok| {
        let a = ((t.start * 100.0) as usize).min(env_db.len() - 1);
        let b = (((t.end * 100.0).ceil()) as usize).clamp(a + 1, env_db.len());
        env_db[a..b]
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let mut out = Vec::new();
    for &(a, b) in sents {
        let mut peaks: Vec<f64> = (a..=b).map(|i| peak(&toks[i])).collect();
        let mut sorted = peaks.clone();
        sorted.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
        let med = sorted[sorted.len() / 2];
        for (k, p) in peaks.drain(..).enumerate() {
            let score = p - med;
            let t = &toks[a + k];
            if score >= 4.5 && t.end - t.start >= 0.12 {
                out.push((a + k, score));
            }
        }
    }
    out
}

pub fn analyse(toks: &[Tok], env_db: &[f64]) -> Analysis {
    let sents = sentences(toks);
    let pauses = toks
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[1].start - w[0].end >= MIN_PAUSE)
        .map(|(i, w)| Pause {
            after: i,
            start: w[0].end,
            end: w[1].start,
        })
        .collect();
    let stresses = stresses(toks, &sents, env_db);
    let entities = entities(toks, &sents);
    Analysis {
        sentences: sents,
        pauses,
        stresses,
        entities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        s.split_whitespace()
            .enumerate()
            .map(|(i, w)| Tok::new(w, i as f64 * 0.4, i as f64 * 0.4 + 0.3))
            .collect()
    }

    fn found(s: &str) -> Vec<(EntityKind, String)> {
        let t = toks(s);
        analyse(&t, &[])
            .entities
            .into_iter()
            .map(|e| (e.kind, e.text))
            .collect()
    }

    #[test]
    fn numbers_with_units() {
        let e = found("The deficit was ₹40 lakh crore last year.");
        assert!(
            e.contains(&(EntityKind::Number, "₹40 lakh crore".into())),
            "{e:?}"
        );
        let e = found("Growth fell to 3.5% in March");
        assert!(e.contains(&(EntityKind::Number, "3.5%".into())), "{e:?}");
        let e = found("inflation hit 7 percent again");
        assert!(e.contains(&(EntityKind::Number, "7%".into())), "{e:?}");
        let e = found("it cost twenty percent more");
        assert!(e.contains(&(EntityKind::Number, "20%".into())), "{e:?}");
        let e = found("nearly forty lakh crore rupees");
        assert!(
            e.iter()
                .any(|(k, t)| *k == EntityKind::Number && t.starts_with("40 lakh crore")),
            "{e:?}"
        );
    }

    #[test]
    fn years_and_comparisons() {
        let e = found("Compare 2016 vs 2026 for me");
        assert!(e.contains(&(EntityKind::Year, "2016".into())), "{e:?}");
        assert!(e.contains(&(EntityKind::Year, "2026".into())), "{e:?}");
        assert!(e.iter().any(|(k, _)| *k == EntityKind::Comparison), "{e:?}");
        let e = found("Rents are higher than wages here");
        assert!(e.iter().any(|(k, _)| *k == EntityKind::Comparison), "{e:?}");
    }

    #[test]
    fn plain_small_numbers_are_not_callouts() {
        let e = found("I have 3 dogs and 12 cats");
        assert!(!e.iter().any(|(k, _)| *k == EntityKind::Number), "{e:?}");
    }

    #[test]
    fn lists_questions_places() {
        let e = found("There are three reasons. Why does India import oil?");
        assert!(e.iter().any(|(k, _)| *k == EntityKind::ListCue));
        assert!(e.iter().any(|(k, _)| *k == EntityKind::Question));
        assert!(e.contains(&(EntityKind::Proper, "India".into())), "{e:?}");
    }

    #[test]
    fn pauses_and_stresses() {
        let mut t = toks("we really need this now");
        t[3].start += 0.5;
        t[3].end += 0.5;
        let mut env = vec![-40.0; 400];
        for v in env[(t[2].start * 100.0) as usize..(t[2].end * 100.0) as usize].iter_mut() {
            *v = -20.0;
        }
        let a = analyse(&t, &env);
        assert_eq!(a.pauses.len(), 1);
        assert_eq!(a.pauses[0].after, 2);
        assert!(a.stresses.iter().any(|(i, _)| *i == 2), "{:?}", a.stresses);
    }
}
