//! (0.10) Speech-led beat timing: the WHEN layer for a voice-over.
//!
//! With a repaired [`SpeechMap`] (`CompileOptions.speech`) beats follow the
//! spoken sentences instead of the planned/music-snapped durations: beat
//! `i + 1` starts exactly `lead` before sentence `i + 1`, where `lead` is the
//! silence before the first sentence. Lifecycle planning then runs on the new
//! durations and each beat's EVOLVE event snaps to a word start. Pure and
//! deterministic. See docs/VOICE.md.

use super::art_direction::TitleReveal;
use super::{round3, BeatPlan, CompileError};
use crate::intent::{Beat, Subject};
use crate::speech::{
    EvolveSnap, RevealAnchor, RevealRole, SpeechMap, EVOLVE_SNAP_WINDOW, REVEAL_LATE_MAX,
    REVEAL_LEAD_MAX,
};

/// Hold after the last sentence is at least this long (seconds).
pub const MIN_TAIL: f64 = 0.9;
/// The last beat never gets shorter than this (seconds); a short closing
/// sentence extends the hold instead.
pub const MIN_LAST_BEAT: f64 = 3.0;
/// A beat must outlast its outgoing overlap by at least this much (seconds).
const MIN_LIVE: f64 = 0.3;

/// Beat durations that put every beat start `lead` before its sentence.
///
/// `d_i = (s_{i+1}.start - lead) - start_i + overlap_out_i` (so beat `i + 1`
/// starts at `s_{i+1}.start - lead`, measured from the accumulated, rounded
/// starts so rounding never drifts); the last beat holds until
/// `s_last.end + tail`, `tail = max(MIN_TAIL, MIN_TAIL · pace)` (`pace` is the
/// rhythm's duration scale), extended to [`MIN_LAST_BEAT`]. Music downbeats
/// are not consulted: speech overrides music snapping.
pub(crate) fn speech_durations(
    speech: &SpeechMap,
    overlaps_out: &[f64],
    pace: f64,
) -> Result<Vec<f64>, CompileError> {
    let n = overlaps_out.len();
    let s = &speech.sentences;
    if s.len() != n {
        return Err(CompileError::Invalid(format!(
            "speech has {} sentences but the intent has {n} beats \
             (re-run `motion-engine voice` for this intent)",
            s.len()
        )));
    }
    if let Some(i) = s.iter().enumerate().position(|(i, x)| x.beat != i) {
        return Err(CompileError::Invalid(format!(
            "speech sentence {i} speaks beat {}, expected beat {i}",
            s[i].beat
        )));
    }
    let lead = s[0].start;
    let mut durations = Vec::with_capacity(n);
    let mut start = 0.0f64;
    for i in 0..n {
        let overlap = overlaps_out[i];
        let d = if i + 1 < n {
            if s[i + 1].start <= s[i].start {
                return Err(CompileError::Invalid(format!(
                    "speech sentences {i} and {} are not in time order",
                    i + 1
                )));
            }
            round3((s[i + 1].start - lead) - start + overlap)
        } else {
            let tail = MIN_TAIL.max(MIN_TAIL * pace);
            round3((s[i].end + tail - start).max(MIN_LAST_BEAT))
        };
        if d < overlap + MIN_LIVE {
            return Err(CompileError::Invalid(format!(
                "speech sentences {i} and {} are too close together for beat {} \
                 (beat would last {d:.3} s)",
                i + 1,
                i + 1
            )));
        }
        durations.push(d);
        start += d - overlap;
    }
    Ok(durations)
}

/// Move each beat's EVOLVE boundary onto the nearest word start within
/// [`EVOLVE_SNAP_WINDOW`] when the lifecycle stays strictly ordered
/// (`read < evolve < anticipate`). Runs after lifecycle planning and before any
/// beat is built, so every motion keyed to `life.evolve` / `evolve_events`
/// moves with it. Beats in `keep` (EVOLVE placed from speech) are left
/// alone. Returns what moved (absolute project seconds).
pub(crate) fn snap_evolve(
    plans: &mut [BeatPlan],
    speech: &SpeechMap,
    keep: &[usize],
) -> Vec<EvolveSnap> {
    let mut snaps = Vec::new();
    for plan in plans.iter_mut() {
        // (0.20) An EVOLVE the speech lifecycle placed already sits on a word.
        if keep.contains(&plan.index) {
            continue;
        }
        let from = plan.start + plan.life.evolve;
        let mut best: Option<(f64, f64)> = None; // (distance, local time)
        for w in &speech.words {
            let d = (w.start - from).abs();
            if d > EVOLVE_SNAP_WINDOW + 1e-9 {
                continue;
            }
            let local = round3(w.start - plan.start);
            if local <= plan.life.read || local >= plan.life.anticipate {
                continue;
            }
            // Strict `<` keeps the earlier word on ties (words are in time order).
            if best.is_none_or(|(bd, _)| d < bd - 1e-12) {
                best = Some((d, local));
            }
        }
        let Some((_, local)) = best else { continue };
        if (local - plan.life.evolve).abs() < 5e-4 {
            continue;
        }
        plan.life.evolve = local;
        snaps.push(EvolveSnap {
            beat: plan.index,
            from: round3(from),
            to: round3(plan.start + local),
        });
    }
    snaps
}

// ---------------------------------------------------------------------------
// (0.10 Q, 0.20 C2) Word cues: content enters when it is spoken
// ---------------------------------------------------------------------------

/// A content group starts entering this long before the word that names it.
pub const WORD_CUE_LEAD: f64 = 0.12;
/// Shifts smaller than this are not worth a cue.
const WORD_CUE_MIN_SHIFT: f64 = 0.15;
/// A multi-word anchor matches when its words are spoken in order with at
/// most this many other spoken words between its first and its last word.
const SEQUENCE_GAP: usize = 2;
/// Fixed-point scale of [`Num`]: values are held in millionths.
const MICRO: u128 = 1_000_000;

/// Groups that never move: titles, statement copy, furniture, decoration.
const FIXED_GROUPS: &[&str] = &[
    "stage",
    "stage_front",
    "ghost",
    "wipe",
    "kicker",
    "kicker_rule",
    "data",
    "data_rule",
    "index",
    "ticks",
    "head",
    "body",
    "body_rule",
    "serif",
    "serif_rule",
    "statement",
    "statement_rule",
    "field",
    "accent_slab",
    "fg_bar",
    "fg_plate",
    "folio",
    "rule",
    // (0.18) SphereGallery labels: timed to the rotation dwells.
    "sphere_label",
    // (0.20) Titles and decoration never follow a stray matching word: they
    // move only through an explicit anchor (`RevealRole::Title` under a
    // holding look), and explicit anchors are cued before this list applies.
    "title",
    "clip",
    "ghost_word",
    "hero_word",
    // (0.23 W4) The label line a held hero shows at ENTER (cinematic).
    "hero_label",
];

const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "your", "you", "are", "was", "were",
    "its", "our", "their", "they", "but", "not", "has", "have", "had", "into", "than", "then",
];

/// Words that end in `s` but are not plurals: never stemmed. (Words ending
/// in `ss` or `us` keep their `s` anyway.)
const NOT_PLURAL: &[&str] = &[
    "is",
    "this",
    "was",
    "has",
    "bus",
    "plus",
    "gas",
    "yes",
    "us",
    "focus",
    "basis",
    "its",
    "his",
    "does",
    "news",
    "series",
    "species",
    "always",
    "perhaps",
    "lens",
    "atlas",
    "canvas",
    "chaos",
    "cosmos",
    "physics",
    "economics",
    "mathematics",
    "politics",
];

/// Words that end in `-ing` / `-ed` but are not inflections: never stemmed
/// ("evening" is not "even").
const NOT_INFLECTED: &[&str] = &[
    "evening",
    "morning",
    "during",
    "nothing",
    "something",
    "anything",
    "everything",
    "ceiling",
    "wedding",
    "pudding",
    "spring",
    "string",
    "naked",
    "sacred",
    "wicked",
];

fn norm(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric() || *c == '%')
        .flat_map(char::to_lowercase)
        .collect()
}

// --- words -----------------------------------------------------------------

/// Lowercase, without surrounding punctuation and a possessive `'s` (a
/// plural possessive `s'` keeps its `s` for the plural rule).
fn strip_possessive(raw: &str) -> String {
    let lower = raw.to_lowercase();
    let t = lower.trim_matches(|c: char| !c.is_alphanumeric());
    t.strip_suffix("'s")
        .or_else(|| t.strip_suffix("\u{2019}s"))
        .unwrap_or(t)
        .to_string()
}

/// Abbreviations a title or value may use for a word the narrator says in
/// full (normalised form → the full word, which is then stemmed as usual).
const ABBREVIATIONS: &[(&str, &str)] = &[
    ("min", "minute"),
    ("mins", "minutes"),
    ("sec", "second"),
    ("secs", "seconds"),
    ("hr", "hour"),
    ("hrs", "hours"),
    ("yr", "year"),
    ("yrs", "years"),
    ("wk", "week"),
    ("wks", "weeks"),
    ("mo", "month"),
    ("mos", "months"),
    ("bn", "billion"),
    ("mn", "million"),
    ("tn", "trillion"),
    ("pct", "percent"),
    ("kg", "kilogram"),
    ("km", "kilometre"),
    ("lb", "pound"),
    ("lbs", "pounds"),
    ("ft", "feet"),
    ("vs", "versus"),
    ("approx", "approximately"),
];

/// The stemmed forms of a normalised word, primary form first.
///
/// Light English stemming: a plural loses `-s` (`-es` after s / x / z / ch /
/// sh / o, `-ies` → `-y`), then an inflection loses `-ing` / `-ed` (`-ied` →
/// `-y`; a doubled consonant is undoubled, "running" → "run"; a dropped `e`
/// is tried both ways, "making" → "mak" / "make"). Words under four letters,
/// words with digits and the exception lists stay as they are; forms under
/// three letters are dropped. Two words match when they share a form.
fn word_forms(n: &str) -> Vec<String> {
    // (0.20) Written abbreviations match their spoken expansion ("8 min" =
    // "eight minutes", "$2bn" = "two billion").
    if let Some((_, full)) = ABBREVIATIONS.iter().find(|(a, _)| *a == n) {
        return word_forms(full);
    }
    let len = n.chars().count();
    if len < 4
        || NOT_PLURAL.contains(&n)
        || NOT_INFLECTED.contains(&n)
        || n.chars().any(|c| c.is_ascii_digit())
    {
        return vec![n.to_string()];
    }
    let es = |b: &&str| {
        ["s", "x", "z", "ch", "sh", "o"]
            .iter()
            .any(|e| b.ends_with(e))
    };
    let mut forms: Vec<String> = if let Some(base) = n.strip_suffix("ies").filter(|_| len > 4) {
        vec![format!("{base}y"), format!("{base}ie")]
    } else if let Some(base) = n.strip_suffix("es").filter(es) {
        vec![base.to_string(), format!("{base}e")]
    } else if let Some(base) = n
        .strip_suffix('s')
        .filter(|b| !b.ends_with('s') && !b.ends_with('u'))
    {
        vec![base.to_string()]
    } else {
        vec![n.to_string()]
    };
    if let [single] = forms.as_slice() {
        if !NOT_INFLECTED.contains(&single.as_str()) {
            if let Some(v) = inflection(single) {
                forms = v;
            }
        }
    }
    forms.retain(|f| f.chars().count() >= 3);
    if forms.is_empty() {
        forms.push(n.to_string());
    }
    forms
}

/// The forms of `w` without `-ing` / `-ied` / `-ed`, or `None` when it has
/// no such ending (or under three letters would be left).
fn inflection(w: &str) -> Option<Vec<String>> {
    if let Some(base) = w.strip_suffix("ied").filter(|_| w.chars().count() > 4) {
        return Some(vec![format!("{base}y"), format!("{base}ie")]);
    }
    // "-eed" (speed, need, agreed) is never an "-ed" inflection.
    let base = w
        .strip_suffix("ing")
        .or_else(|| w.strip_suffix("ed").filter(|b| !b.ends_with('e')))?;
    let chars: Vec<char> = base.chars().collect();
    let [.., prev, last] = chars.as_slice() else {
        return None;
    };
    if chars.len() < 3 {
        return None;
    }
    let vowel = |c: char| "aeiouy".contains(c);
    if last == prev && !vowel(*last) && !"lsfz".contains(*last) {
        return Some(vec![chars[..chars.len() - 1].iter().collect()]);
    }
    if !vowel(*last) && !"wx".contains(*last) {
        return Some(vec![base.to_string(), format!("{base}e")]);
    }
    Some(vec![base.to_string()])
}

/// The primary stem of one written or spoken word ("berries" → "berry",
/// "Buffett's" → "buffett").
#[cfg(test)]
fn stem(raw: &str) -> String {
    let n = norm(&strip_possessive(raw));
    word_forms(&n).into_iter().next().unwrap_or(n)
}

// --- numbers ---------------------------------------------------------------

fn small_number(w: &str) -> Option<u64> {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    if let Some(i) = ONES.iter().position(|o| *o == w) {
        return Some(i as u64);
    }
    TENS.iter()
        .position(|t| *t == w)
        .map(|i| (i as u64 + 2) * 10)
}

fn scale(w: &str) -> Option<u64> {
    match w {
        "hundred" => Some(100),
        "thousand" => Some(1_000),
        "lakh" | "lakhs" => Some(100_000),
        "million" => Some(1_000_000),
        "crore" | "crores" => Some(10_000_000),
        "billion" => Some(1_000_000_000),
        "trillion" => Some(1_000_000_000_000),
        "quadrillion" => Some(1_000_000_000_000_000),
        _ => None,
    }
}

/// A number as written or said: its value in millionths and whether it is a
/// percentage. Two numbers match when both agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Num {
    micro: u128,
    percent: bool,
}

/// Millionths of a decimal fraction written as `digits` ("5" → 500 000).
fn frac_micro(digits: &str) -> u128 {
    let padded: String = digits
        .chars()
        .chain(std::iter::repeat('0'))
        .take(6)
        .collect();
    padded.parse().unwrap_or(0)
}

/// A written number token (`"$381"`, `"₹5,200"`, `"4.5M"`, `"$1.2bn"`,
/// `"23%"`, `"1969"`) as (millionths, percent, carried a scale suffix).
/// Leading currency / sign symbols and trailing punctuation are ignored.
fn number_token(raw: &str) -> Option<(u128, bool, bool)> {
    let t = raw
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '%');
    let (t, percent) = match t.strip_suffix('%') {
        Some(rest) => (rest, true),
        None => (t, false),
    };
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.'))
        .unwrap_or(t.len());
    let (digits, suffix) = t.split_at(split);
    let mult: u128 = match suffix.to_ascii_lowercase().as_str() {
        "" => 1,
        "k" => 1_000,
        "m" | "mn" => 1_000_000,
        "b" | "bn" => 1_000_000_000,
        "t" | "tn" => 1_000_000_000_000,
        _ => return None,
    };
    if percent && mult != 1 {
        return None;
    }
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    if !int.starts_with(|c: char| c.is_ascii_digit())
        || frac.contains(['.', ','])
        || (digits.contains('.') && frac.is_empty())
    {
        return None;
    }
    let whole: u128 = int
        .chars()
        .filter(|c| *c != ',')
        .collect::<String>()
        .parse()
        .ok()?;
    let micro = whole.checked_mul(MICRO)?.checked_add(frac_micro(frac))?;
    Some((micro.checked_mul(mult)?, percent, mult != 1))
}

/// Canonical number of a layer text token ("₹5,200" → "5200", "10%" →
/// "10%"): every digit of the token, `None` for a mid-token `.`. The lenient
/// reading of tokens [`number_token`] rejects ("1990s", "COVID-19").
fn text_number(token: &str) -> Option<String> {
    let digits: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || token.contains('.') && !token.ends_with('.') {
        return None;
    }
    let v: u64 = digits.trim_start_matches('0').parse().unwrap_or(0);
    Some(format!("{v}{}", if token.contains('%') { "%" } else { "" }))
}

/// [`text_number`] as (millionths, percent, no suffix).
fn lenient_number(raw: &str) -> Option<(u128, bool, bool)> {
    let canon = text_number(raw)?;
    let (digits, percent) = match canon.strip_suffix('%') {
        Some(d) => (d, true),
        None => (canon.as_str(), false),
    };
    let v: u128 = digits.parse().ok()?;
    Some((v.checked_mul(MICRO)?, percent, false))
}

/// Where an English number phrase is in its grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Said {
    Start,
    Ones,
    Teen,
    Tens,
    TensOnes,
    Hundred,
    Scale,
    And,
    Decimal,
    YearTens,
    Year,
}

/// The English number phrase starting at `words[i]`, as (millionths, index
/// after the phrase): "eight", "twenty three", "three hundred and eighty one
/// billion", "four point five million", "nineteen sixty nine" (two tens-pairs
/// in a row are a year). "a" counts as one before a scale word. `None` when
/// `words[i]` starts no number.
fn spoken_number(words: &[&str], i: usize) -> Option<(u128, usize)> {
    let at = |k: usize| words.get(k).map(|w| norm(w));
    let digit = |k: usize| at(k).and_then(|w| small_number(&w)).filter(|d| *d < 10);
    let (mut total, mut cur, mut frac) = (0u128, 0u128, 0u128);
    let mut said = Said::Start;
    let mut j = i;
    while let Some(w) = at(j) {
        if let Some(d) = small_number(&w).map(u128::from) {
            let year = total == 0
                && (10..100).contains(&cur)
                && matches!(said, Said::Teen | Said::Tens | Said::TensOnes);
            said = match (d, said) {
                (0..=9, Said::Start | Said::Hundred | Said::Scale | Said::And) => {
                    cur += d;
                    Said::Ones
                }
                (1..=9, Said::Tens) => {
                    cur += d;
                    Said::TensOnes
                }
                (1..=9, Said::YearTens) => {
                    cur += d;
                    Said::Year
                }
                (10..=19, Said::Start | Said::Hundred | Said::Scale | Said::And) => {
                    cur += d;
                    Said::Teen
                }
                (20..=90, Said::Start | Said::Hundred | Said::Scale | Said::And) => {
                    cur += d;
                    Said::Tens
                }
                (10..=19, _) if year => {
                    cur = cur * 100 + d;
                    Said::Year
                }
                (20..=90, _) if year => {
                    cur = cur * 100 + d;
                    Said::YearTens
                }
                _ => break,
            };
        } else if let Some(s) = scale(&w).map(u128::from) {
            if s == 100 {
                said = match said {
                    Said::Start => {
                        cur = 100;
                        Said::Hundred
                    }
                    Said::Ones | Said::Teen | Said::Tens | Said::TensOnes if cur < 100 => {
                        cur = cur.max(1) * 100;
                        Said::Hundred
                    }
                    _ => break,
                };
            } else {
                let group = match said {
                    Said::Start => MICRO,
                    Said::Ones
                    | Said::Teen
                    | Said::Tens
                    | Said::TensOnes
                    | Said::Hundred
                    | Said::Decimal => cur.checked_mul(MICRO)?.checked_add(frac)?,
                    _ => break,
                };
                total = total.checked_add(group.checked_mul(s)?)?;
                (cur, frac, said) = (0, 0, Said::Scale);
            }
        } else if w == "and"
            && matches!(said, Said::Hundred | Said::Scale)
            && at(j + 1).is_some_and(|n| small_number(&n).is_some())
        {
            said = Said::And;
        } else if w == "point"
            && matches!(
                said,
                Said::Ones | Said::Teen | Said::Tens | Said::TensOnes | Said::Hundred
            )
            && digit(j + 1).is_some()
        {
            let mut digits = String::new();
            while let Some(d) = digit(j + 1) {
                digits.push_str(&d.to_string());
                j += 1;
            }
            frac = frac_micro(&digits);
            said = Said::Decimal;
        } else if w == "a" && said == Said::Start && at(j + 1).is_some_and(|n| scale(&n).is_some())
        {
            cur = 1;
            said = Said::Ones;
        } else {
            break;
        }
        j += 1;
    }
    if said == Said::Start {
        return None;
    }
    let value = total.checked_add(cur.checked_mul(MICRO)?.checked_add(frac)?)?;
    Some((value, j))
}

/// (0.22) Currency names a spoken amount may carry before its cents.
const CURRENCY_WORDS: &[&str] = &[
    "dollar", "dollars", "buck", "bucks", "pound", "pounds", "euro", "euros", "rupee", "rupees",
];
/// (0.22) Minor-unit names after the cents.
const CENT_WORDS: &[&str] = &["cent", "cents", "pence", "penny", "paise"];

/// (0.22) The cents of a spoken amount after its whole part `whole`
/// (millionths), read from the currency word `words[j]` on: "dollar
/// fifteen", "dollars fifteen cents", "pounds and fifty pence". The cents
/// are a whole number from 1 to 99. Without the minor unit after them they
/// count only after a whole part under a thousand and never after "and"
/// ("two dollars and fifty people" is not $2.50); a currency word that ends
/// a clause ("dollars, fifty") takes no cents. Returns (cents in millionths
/// of the major unit, index after them).
fn spoken_cents(words: &[&str], j: usize, whole: u128) -> Option<(u128, usize)> {
    let at = |k: usize| words.get(k).map(|w| norm(w));
    let currency = words.get(j)?;
    let clause_end = currency.ends_with(|c: char| !c.is_alphanumeric());
    if clause_end || !CURRENCY_WORDS.contains(&norm(currency).as_str()) {
        return None;
    }
    let and = at(j + 1).as_deref() == Some("and");
    let (cents, next) = spoken_number(words, j + 1 + usize::from(and))?;
    if cents % MICRO != 0 || !(1..=99).contains(&(cents / MICRO)) {
        return None;
    }
    let unit = at(next).is_some_and(|w| CENT_WORDS.contains(&w.as_str()));
    if !unit && (and || whole >= 1_000 * MICRO) {
        return None;
    }
    Some((cents / 100, next + usize::from(unit)))
}

/// The number starting at `words[i]` (a digit token with optional scale
/// words after it, or an English number phrase, (0.22) with the cents of a
/// spoken amount: "one dollar fifteen" = "a dollar fifteen" = 1.15, see
/// [`spoken_cents`]), with a following "percent" / "per cent" / "%" folded
/// in, and the index after it.
fn number_at(words: &[&str], i: usize) -> Option<(Num, usize)> {
    let raw = words.get(i)?;
    let (micro, mut percent, mut j) = if raw.chars().any(|c| c.is_ascii_digit()) {
        let (mut v, percent, suffixed) = number_token(raw).or_else(|| lenient_number(raw))?;
        let mut j = i + 1;
        if !suffixed && !percent {
            while let Some(s) = words.get(j).and_then(|w| scale(&norm(w))) {
                match v.checked_mul(u128::from(s)) {
                    Some(next) => v = next,
                    None => break,
                }
                j += 1;
            }
        }
        (v, percent, j)
    } else if let Some((cents, next)) = (norm(raw) == "a")
        .then(|| spoken_cents(words, i + 1, MICRO))
        .flatten()
    {
        // "a dollar fifty" (a bare "a dollar" stays words).
        (MICRO + cents, false, next)
    } else {
        let (v, j) = spoken_number(words, i)?;
        match spoken_cents(words, j, v) {
            Some((cents, next)) => (v.checked_add(cents)?, false, next),
            None => (v, false, j),
        }
    };
    if !percent {
        match words.get(j).map(|w| norm(w)).as_deref() {
            Some("percent" | "%") => {
                percent = true;
                j += 1;
            }
            Some("per") if words.get(j + 1).is_some_and(|w| norm(w) == "cent") => {
                percent = true;
                j += 2;
            }
            _ => {}
        }
    }
    Some((Num { micro, percent }, j))
}

// --- tokens and lookup -----------------------------------------------------

/// One matchable token: a word (normalised, with its stemmed forms) or a
/// whole number phrase.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word { norm: String, forms: Vec<String> },
    Num(Num),
}

impl Tok {
    /// Equal numbers (value and percent), or words that share a stem.
    fn matches(&self, other: &Tok) -> bool {
        match (self, other) {
            (Tok::Num(a), Tok::Num(b)) => a == b,
            (Tok::Word { forms: a, .. }, Tok::Word { forms: b, .. }) => {
                a.iter().any(|f| b.contains(f))
            }
            _ => false,
        }
    }

    /// Whether the token may match on its own: every number; words of three
    /// letters or more that are not stop words.
    fn content(&self) -> bool {
        match self {
            Tok::Num(_) => true,
            Tok::Word { norm, .. } => norm.chars().count() >= 3 && !STOP.contains(&norm.as_str()),
        }
    }
}

/// One token of a text and the raw words it came from (`first..=last`).
struct Span {
    tok: Tok,
    first: usize,
    last: usize,
}

/// Tokens of a word sequence (written or spoken): a number phrase is one
/// token ("$381 BILLION", "three hundred and eighty one billion", "23 %"),
/// every other word is one token. Hyphenated number words ("twenty-three")
/// are read as separate words.
fn tokenize(raw: &[&str]) -> Vec<Span> {
    let mut pieces: Vec<(&str, usize)> = Vec::new();
    for (i, r) in raw.iter().enumerate() {
        let parts: Vec<&str> = r.split('-').filter(|p| !p.is_empty()).collect();
        let number_words = parts.len() > 1
            && parts.iter().all(|p| {
                let n = norm(p);
                small_number(&n).is_some() || scale(&n).is_some()
            });
        if number_words {
            pieces.extend(parts.into_iter().map(|p| (p, i)));
        } else {
            pieces.push((r, i));
        }
    }
    let words: Vec<&str> = pieces.iter().map(|p| p.0).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&(word, first)) = pieces.get(i) {
        if let Some((num, next)) = number_at(&words, i).filter(|(_, next)| *next > i) {
            let last = pieces.get(next - 1).map_or(first, |p| p.1);
            out.push(Span {
                tok: Tok::Num(num),
                first,
                last,
            });
            i = next;
            continue;
        }
        let n = norm(&strip_possessive(word));
        if !n.is_empty() {
            out.push(Span {
                tok: Tok::Word {
                    forms: word_forms(&n),
                    norm: n,
                },
                first,
                last: first,
            });
        }
        i += 1;
    }
    out
}

/// The tokens of one anchor entry that may match: stop words and words
/// under three letters are dropped (they never match on their own); numbers
/// always stay.
fn entry_tokens(entry: &str) -> Vec<Tok> {
    let raw: Vec<&str> = entry
        .split(|c: char| c.is_whitespace() || c == '_')
        .filter(|w| !w.is_empty())
        .collect();
    let all: Vec<Tok> = tokenize(&raw).into_iter().map(|s| s.tok).collect();
    let content: Vec<Tok> = all.iter().filter(|t| t.content()).cloned().collect();
    if content.is_empty() {
        // A deliberate single short anchor ("AI", "VR", "8") matches exactly;
        // function words are never anchors.
        if let [only] = all.as_slice() {
            let function_word = matches!(only, Tok::Word { norm, .. }
                if STOP.contains(&norm.as_str()) || SHORT_FUNCTION_WORDS.contains(&norm.as_str()));
            if !function_word {
                return vec![only.clone()];
            }
        }
    }
    content
}

/// Short function words that are never an anchor on their own (longer ones
/// are in [`STOP`]).
const SHORT_FUNCTION_WORDS: &[&str] = &[
    "a", "an", "to", "of", "in", "on", "at", "by", "as", "is", "be", "it", "or", "do", "up", "no",
    "my", "me", "he", "we", "us", "so", "if", "am", "an", "any", "all", "can", "did", "get", "got",
    "his", "her", "him", "how", "let", "may", "now", "off", "one", "out", "own", "per", "put",
    "say", "see", "she", "too", "two", "use", "way", "who", "why", "yet",
];

/// (0.20) The first term of a text: its leading number phrase, else its
/// first word ("$381 BILLION owed" → "$381 BILLION", "4.5 million tons" →
/// "4.5 million", "flat salary" → "flat").
pub(crate) fn first_term(text: &str) -> Option<String> {
    let raw: Vec<&str> = text.split_whitespace().collect();
    let span = tokenize(&raw).into_iter().next()?;
    Some(raw.get(span.first..=span.last)?.join(" "))
}

/// Where an anchor was heard: the start time of its first word and the
/// spoken words (normalised).
#[derive(Debug, Clone, PartialEq)]
struct Hit {
    at: f64,
    word: String,
}

/// The words of one beat as heard: each token with the start time of its
/// first word and its spoken text.
struct Heard {
    toks: Vec<(Tok, f64, String)>,
}

impl Heard {
    fn new(spoken: &[(String, f64)]) -> Heard {
        let raw: Vec<&str> = spoken.iter().map(|(w, _)| w.as_str()).collect();
        let toks = tokenize(&raw)
            .into_iter()
            .filter_map(|s| {
                let at = spoken.get(s.first)?.1;
                let text = raw
                    .get(s.first..=s.last)?
                    .iter()
                    .map(|w| norm(w))
                    .filter(|w| !w.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                Some((s.tok, at, text))
            })
            .collect();
        Heard { toks }
    }

    /// The earliest hit of any entry.
    fn find<S: AsRef<str>>(&self, entries: &[S]) -> Option<Hit> {
        entries
            .iter()
            .filter_map(|e| self.sequence(&entry_tokens(e.as_ref())))
            .fold(None, earliest)
    }

    /// The earliest place where every token of `entry` is spoken in order,
    /// with at most [`SEQUENCE_GAP`] other spoken tokens between the first
    /// and the last. `None` for an empty entry.
    fn sequence(&self, entry: &[Tok]) -> Option<Hit> {
        let (head, rest) = entry.split_first()?;
        let end = self.toks.len().saturating_sub(1);
        let mut best: Option<Hit> = None;
        for (p, (tok, at, _)) in self.toks.iter().enumerate() {
            if !tok.matches(head) || best.as_ref().is_some_and(|b| b.at <= *at) {
                continue;
            }
            let limit = (p + rest.len() + SEQUENCE_GAP).min(end);
            let mut last = Some(p);
            for e in rest {
                last = last.and_then(|l| (l + 1..=limit).find(|&q| self.toks[q].0.matches(e)));
            }
            let Some(last) = last else { continue };
            let word = self.toks[p..=last]
                .iter()
                .map(|t| t.2.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            best = Some(Hit { at: *at, word });
        }
        best
    }
}

fn earliest(best: Option<Hit>, hit: Hit) -> Option<Hit> {
    match best {
        Some(b) if b.at <= hit.at => Some(b),
        _ => Some(hit),
    }
}

/// (0.20) Scene-local time at which the narrator first says any of the
/// anchor `words` (the shared lookup for word cues, flashes, shakes and
/// stamps). Each entry may be several words, matched as a sequence: every
/// word spoken in order, with at most two other words in between. Words
/// match by light stemming ("berries" = "berry", "running" = "run",
/// "Buffett's" = "buffett"); numbers match by value, written or said
/// ("$381 BILLION" = "three hundred and eighty one billion", "4.5M" = "four
/// point five million", "23%" = "twenty three percent", "1969" = "nineteen
/// sixty nine", "4.1%" = "four point one per cent", "$1.15" = "one dollar
/// fifteen"). Inside an entry, stop words and words under three letters
/// never match on their own; an entry that is one short word ("AI", "8")
/// matches exactly. `None` when no entry is spoken.
pub fn anchor_time(spoken: &[(String, f64)], words: &[&str]) -> Option<f64> {
    Heard::new(spoken).find(words).map(|h| h.at)
}

/// (0.20) The words a beat is about, from the intent alone: the primary's
/// asset / value / meaning (group `primary`), the secondary's (`secondary`)
/// and the keyword (`keyword`). These are intent-level anchors: their
/// `group` names the subject, not a layer. The speech-aware lifecycle places
/// phases from them, speech QA treats their words as content words, and
/// builders that declare no layer anchors fall back to them.
#[allow(dead_code)] // used by 0.20 workstream D (speech-aware lifecycle)
pub(crate) fn beat_anchors(beat: &Beat) -> Vec<RevealAnchor> {
    fn words_of(s: &Subject) -> Vec<String> {
        let mut out = Vec::new();
        if let Subject::Object(o) = s {
            out.push(o.asset.replace(['_', '-'], " "));
        }
        if let Some(v) = s.value() {
            out.push(v.to_string());
        }
        if let Some(m) = s.meaning() {
            out.push(m.to_string());
        }
        out.retain(|w| !w.trim().is_empty());
        out
    }
    let mut out = Vec::new();
    let primary = words_of(&beat.primary);
    if !primary.is_empty() {
        out.push(RevealAnchor {
            group: "primary".into(),
            words: primary,
            role: RevealRole::Content,
        });
    }
    if let Some(words) = beat.secondary.as_ref().map(words_of) {
        if !words.is_empty() {
            out.push(RevealAnchor {
                group: "secondary".into(),
                words,
                role: RevealRole::Content,
            });
        }
    }
    if let Some(k) = beat.keyword.as_deref().filter(|k| !k.trim().is_empty()) {
        out.push(RevealAnchor {
            group: "keyword".into(),
            words: vec![k.to_string()],
            role: RevealRole::Content,
        });
    }
    out
}

/// (0.19) Scene-local time at which the narrator first speaks any of the words
/// in `parts` (each word on its own: stop words and words under three letters
/// never count; a plural matches its singular). `None` when none of them is
/// spoken in the beat. A thin wrapper over [`anchor_time`].
pub(crate) fn name_time(spoken: &[(String, f64)], parts: &[&str]) -> Option<f64> {
    let words: Vec<&str> = parts
        .iter()
        .flat_map(|p| p.split(|c: char| c.is_whitespace() || c == '_' || c == '-'))
        .filter(|w| !w.is_empty())
        .collect();
    anchor_time(spoken, &words)
}

// --- cues ------------------------------------------------------------------

/// Layer ids of a scene in tree order, with their raw word tokens (text
/// words, or asset-name parts).
fn collect_ids(layers: &[crate::scene::Layer], out: &mut Vec<(String, Vec<String>)>) {
    use crate::scene::LayerKind;
    for l in layers {
        let mut tokens = Vec::new();
        match &l.kind {
            LayerKind::Text(t) => tokens.extend(t.text.split_whitespace().map(str::to_string)),
            LayerKind::Image { asset, .. } | LayerKind::Svg { asset, .. } => tokens.extend(
                asset
                    .trim_start_matches("asset.")
                    .split(['_', '-', '.'])
                    .map(str::to_string),
            ),
            _ => {}
        }
        out.push((l.id.clone(), tokens));
        if let LayerKind::Group { children } = &l.kind {
            collect_ids(children, out);
        }
    }
}

/// (0.20) How word cues may move a group.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CuePolicy {
    /// A group may enter later than planned when its word comes later; its
    /// arrival must still finish by `anticipate - min_read`, else it stays
    /// where it was and the move is recorded as clamped.
    pub allow_delay: bool,
    /// Read time a delayed group keeps before ANTICIPATE (seconds).
    pub min_read: f64,
    /// Whether a `RevealRole::Title` anchor cues the title (`Hold`) or the
    /// title enters with the beat (`Free`).
    pub title: TitleReveal,
    /// (0.22) The beat's focal layer id (`B::focal`). The beat is read
    /// around it from READ on (layout QA judges it there), so a delayed
    /// group that holds it must have arrived by READ.
    pub focal: Option<String>,
}

impl CuePolicy {
    /// The latest time a delayed arrival of `members` may end: the read
    /// floor `anticipate - min_read`, and READ for the group holding the
    /// focal layer.
    fn floor(&self, members: &[String], life: &crate::scene::Lifecycle) -> f64 {
        let floor = life.anticipate - self.min_read;
        // (0.23 W4) A held card's value or picture is a descendant of the
        // focal card: the card is there from ENTER, what it holds must have
        // arrived by READ all the same.
        let holds_focal = |f: &String| {
            members.iter().any(|m| {
                m == f
                    || m.strip_prefix(f.as_str())
                        .is_some_and(|rest| rest.starts_with('.'))
            })
        };
        match &self.focal {
            Some(f) if holds_focal(f) => floor.min(life.read),
            _ => floor,
        }
    }
}

/// (0.20) One group retimed by a word cue (scene-local seconds).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WordCueMove {
    pub group: String,
    pub word: String,
    pub from: f64,
    pub to: f64,
    /// The explicit anchor's role; `None` for the name-matching fallback.
    pub role: Option<RevealRole>,
    /// The move was limited by ENTER or by the read-time floor.
    pub clamped: bool,
}

/// (0.22) An anchored group whose word is spoken but which no cue could move
/// toward it: it keeps its planned entrance (scene-local seconds). Recorded
/// only when that entrance starts more than `REVEAL_LEAD_MAX` before the
/// word (or `REVEAL_LATE_MAX` after it); closer, it still reads on its word.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WordCueDrop {
    pub group: String,
    /// The spoken words that matched the anchor.
    pub word: String,
    /// When the word starts.
    pub at: f64,
    /// When the group starts entering (unchanged).
    pub from: f64,
    pub role: RevealRole,
}

impl WordCueDrop {
    /// How long before its word the group starts entering (negative: after).
    pub fn early(&self) -> f64 {
        self.at - self.from
    }
}

/// (0.22) What the word cues did to one beat scene: the groups they moved
/// and the anchored groups they could not move (never silent).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct WordCues {
    pub moves: Vec<WordCueMove>,
    pub dropped: Vec<WordCueDrop>,
}

/// (0.22) An entrance a word cue shortens never runs shorter than this
/// (seconds); motions already shorter keep their length.
pub const CUE_MIN_ENTRANCE: f64 = 0.25;

/// (0.22) How freely a word cue may rework a group's motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freedom {
    /// Shift every motion before ANTICIPATE together, or not at all (the
    /// name-matching fallback: a guess never reshapes a group).
    Rigid,
    /// When the rigid shift has no room, move the group's arrival alone and
    /// leave its later events (an underline at EVOLVE) where they are,
    /// pushing them only behind the new arrival (explicit Title / Label
    /// anchors).
    Arrival,
    /// As [`Freedom::Arrival`], and the arrival may also be shortened (each
    /// motion down to [`CUE_MIN_ENTRANCE`]) so it lands before the read floor
    /// (explicit Content / Value / Stamp anchors: the things the words name).
    Shorten,
}

/// What [`retime`] did with one group.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Retimed {
    /// Moved: `(from, to, clamped)` (scene-local seconds).
    Moved(f64, f64, bool),
    /// Nothing to do: already on its word, delays not allowed, or nothing
    /// enters before ANTICIPATE.
    Stays,
    /// The group needed to move but could not: it still starts entering
    /// at this time (scene-local seconds).
    Dropped(f64),
}

/// (0.18) A group on a rotating sphere (SphereGallery items) keeps the
/// rotation schedule: it enters with the sphere and each item's scale pop
/// belongs to its dwell, so no spoken word may retime it.
fn on_sphere(scene: &crate::scene::Scene, members: &[String]) -> bool {
    scene.motions.iter().any(|m| {
        matches!(m.op, crate::scene::MotionOp::Revolve { .. }) && members.contains(&m.target)
    })
}

/// End of motion `i` (scene-local seconds).
fn end_of(scene: &crate::scene::Scene, i: usize) -> f64 {
    scene.motions[i].start + scene.motions[i].duration
}

/// (0.22) Split a group's motions (indices) into its arrival and its later
/// events. The arrival is the chain of motions that overlap (or touch) the
/// earliest one, in start order; anything that starts after the chain has
/// ended (an underline drawn at EVOLVE, a punch on a later word) is a later
/// event.
fn split_arrival(scene: &crate::scene::Scene, entering: &[usize]) -> (Vec<usize>, Vec<usize>) {
    let mut order = entering.to_vec();
    order.sort_by(|&a, &b| {
        scene.motions[a]
            .start
            .total_cmp(&scene.motions[b].start)
            .then(a.cmp(&b))
    });
    let (mut arrival, mut later) = (Vec::new(), Vec::new());
    let mut chain_end = f64::NEG_INFINITY;
    for i in order {
        if arrival.is_empty() || scene.motions[i].start <= chain_end + 1e-9 {
            chain_end = chain_end.max(end_of(scene, i));
            arrival.push(i);
        } else {
            later.push(i);
        }
    }
    (arrival, later)
}

/// The validator's tolerances (`validate::EPS`, `validate::OVERLAP_EPS`).
const END_EPS: f64 = 1e-6;
const OVERLAP_EPS: f64 = 1e-9;

/// (0.23) Whether the motions `which` (indices), moved from `before` (their
/// old starts) by `delta` and rounded each to the millisecond, still stand to
/// each other and to the scene end as they would after an exact move: no pair
/// on one layer and channel that did not overlap does now, and no motion that
/// ended inside the scene ends after it. `retime` falls back to the exact move
/// when this fails, so a cue never makes a scene that validated stop doing so.
fn shift_keeps_offsets(
    scene: &crate::scene::Scene,
    which: &[usize],
    before: &[f64],
    delta: f64,
) -> bool {
    let end = scene.duration_seconds;
    // A motion's `(exact, rounded)` interval after the move.
    let span = |j: usize| -> ((f64, f64), (f64, f64)) {
        let m = &scene.motions[j];
        let rounded = (m.start, m.start + m.duration);
        let exact = match which.iter().position(|&w| w == j) {
            Some(k) => (before[k] + delta, before[k] + delta + m.duration),
            None => rounded,
        };
        (exact, rounded)
    };
    let overlap = |a: (f64, f64), b: (f64, f64)| -> bool {
        let (a_zero, b_zero) = (a.1 - a.0 <= OVERLAP_EPS, b.1 - b.0 <= OVERLAP_EPS);
        match (a_zero, b_zero) {
            (true, true) => false,
            (true, false) => a.0 > b.0 + OVERLAP_EPS && a.0 < b.1 - OVERLAP_EPS,
            (false, true) => b.0 > a.0 + OVERLAP_EPS && b.0 < a.1 - OVERLAP_EPS,
            (false, false) => a.0 < b.1 - OVERLAP_EPS && b.0 < a.1 - OVERLAP_EPS,
        }
    };
    for &i in which {
        let (exact, rounded) = span(i);
        if exact.1 <= end + END_EPS && rounded.1 > end + END_EPS {
            return false;
        }
        let mi = &scene.motions[i];
        let channel = mi.op.channel();
        for (j, mj) in scene.motions.iter().enumerate() {
            if j == i || mj.target != mi.target || mj.op.channel() != channel {
                continue;
            }
            let (exact_j, rounded_j) = span(j);
            if !overlap(exact, exact_j) && overlap(rounded, rounded_j) {
                return false;
            }
        }
    }
    true
}

/// Shift every motion of `members` that starts before ANTICIPATE so the
/// first of them starts at `at - lead`, keeping their relative offsets.
///
/// Earlier: never before ENTER (clamped when the word itself is spoken
/// before ENTER), and the motions must still end by ANTICIPATE. Later (only
/// with `policy.allow_delay`): the last of them must end by `anticipate -
/// policy.min_read`, and (0.22) the arrival of the group holding the beat's
/// focal layer by READ ([`CuePolicy::floor`]).
///
/// (0.22) When that rigid shift has no room, a group with
/// [`Freedom::Arrival`] or more moves its arrival alone ([`split_arrival`]):
/// later events keep their times and are only pushed behind the new arrival
/// (never past ANTICIPATE). With [`Freedom::Shorten`] an arrival that would
/// still end after the read floor is shortened to end on it, each motion
/// down to [`CUE_MIN_ENTRANCE`]. A word too late even for that moves the
/// group as late as the floor allows (clamped) when that is worth a cue.
/// Anything else that cannot move is [`Retimed::Dropped`] (it keeps its
/// motions).
fn retime(
    scene: &mut crate::scene::Scene,
    members: &[String],
    life: &crate::scene::Lifecycle,
    at: f64,
    lead: f64,
    policy: &CuePolicy,
    freedom: Freedom,
) -> Retimed {
    let entering: Vec<usize> = scene
        .motions
        .iter()
        .enumerate()
        .filter(|(_, m)| members.contains(&m.target) && m.start < life.anticipate - 1e-6)
        .map(|(i, _)| i)
        .collect();
    let Some(first) = entering
        .iter()
        .map(|&i| scene.motions[i].start)
        .reduce(f64::min)
    else {
        return Retimed::Stays;
    };
    let last_end = entering
        .iter()
        .map(|&i| end_of(scene, i))
        .fold(first, f64::max);
    let target = at - lead;
    let shift = |scene: &mut crate::scene::Scene, which: &[usize], delta: f64| {
        let before: Vec<f64> = which.iter().map(|&i| scene.motions[i].start).collect();
        for &i in which {
            scene.motions[i].start = round3(scene.motions[i].start + delta);
        }
        // (0.23) Rounding each start on its own can move two back-to-back
        // motions of one layer into each other by up to a millisecond (the
        // validator's "conflicting animations"), or a motion that ended on
        // the scene end past it. Then the group moves by exactly `delta`,
        // which keeps every offset between its motions.
        if !shift_keeps_offsets(scene, which, &before, delta) {
            for (&i, &s) in which.iter().zip(&before) {
                scene.motions[i].start = s + delta;
            }
        }
    };
    if target < first {
        let to = target.max(life.enter);
        if first - to < WORD_CUE_MIN_SHIFT {
            return Retimed::Stays; // already on time
        }
        let clamped = at < life.enter - 1e-9;
        // Entrances must still finish before ANTICIPATE.
        if last_end + (to - first) <= life.anticipate {
            shift(scene, &entering, to - first);
            return Retimed::Moved(round3(first), round3(to), clamped);
        }
        // (0.22) A later motion already runs past ANTICIPATE: the arrival
        // alone comes forward, later events stay.
        if freedom == Freedom::Rigid {
            return Retimed::Dropped(first);
        }
        let (arrival, _) = split_arrival(scene, &entering);
        let arrival_end = arrival
            .iter()
            .map(|&i| end_of(scene, i))
            .fold(first, f64::max);
        if arrival_end + (to - first) > life.anticipate {
            return Retimed::Dropped(first);
        }
        shift(scene, &arrival, to - first);
        return Retimed::Moved(round3(first), round3(to), clamped);
    }
    if !policy.allow_delay || target - first < WORD_CUE_MIN_SHIFT {
        return Retimed::Stays;
    }
    // Every motion must leave `min_read` before ANTICIPATE; (0.22) the
    // arrival of the group holding the focal layer must also land by READ.
    let (arrival, later) = split_arrival(scene, &entering);
    let arrival_end = arrival
        .iter()
        .map(|&i| end_of(scene, i))
        .fold(first, f64::max);
    let floor = policy.floor(members, life);
    let room = (life.anticipate - policy.min_read - last_end).min(floor - arrival_end);
    if target - first <= room + 1e-9 {
        shift(scene, &entering, target - first);
        return Retimed::Moved(round3(first), round3(target), false);
    }
    if freedom == Freedom::Rigid {
        if room >= WORD_CUE_MIN_SHIFT {
            shift(scene, &entering, room);
            return Retimed::Moved(round3(first), round3(first + room), true);
        }
        return Retimed::Dropped(first);
    }

    // (0.22) Make room: the arrival alone moves; later events stay.
    let full = arrival_end - first;
    // The shortest arrival: every motion keeps its offset, its length cut
    // to CUE_MIN_ENTRANCE (shorter ones keep theirs).
    let shortest = arrival
        .iter()
        .map(|&i| {
            let m = &scene.motions[i];
            m.start - first + m.duration.min(CUE_MIN_ENTRANCE)
        })
        .fold(0.0, f64::max);
    let shorten = freedom == Freedom::Shorten;
    let (to, cut, clamped) = if target + full <= floor + 1e-9 {
        (target, false, false)
    } else if shorten && target + shortest <= floor + 1e-9 {
        (target, true, false)
    } else {
        // Too late for the arrival to land before the floor: as late as the
        // floor allows (never earlier than the rigid clamp: the arrival is
        // at most as long as all the motions), if that is worth a cue.
        let to = floor - if shorten { shortest } else { full };
        if to - first < WORD_CUE_MIN_SHIFT {
            return Retimed::Dropped(first);
        }
        (to, shorten, true)
    };
    let delta = to - first;
    shift(scene, &arrival, delta);
    if cut {
        for &i in &arrival {
            let m = &mut scene.motions[i];
            let keep = m.duration.min(CUE_MIN_ENTRANCE);
            if m.start + m.duration > floor + 1e-9 {
                m.duration = round3((floor - m.start).max(keep));
            }
        }
    }
    // Later events that would now fire before the arrival lands follow it,
    // keeping their order and offsets, and still end by ANTICIPATE.
    let landed = arrival.iter().map(|&i| end_of(scene, i)).fold(to, f64::max);
    let early: Vec<usize> = later
        .iter()
        .copied()
        .filter(|&i| scene.motions[i].start < landed - 1e-9)
        .collect();
    if let Some(head) = early
        .iter()
        .map(|&i| scene.motions[i].start)
        .reduce(f64::min)
    {
        let push = landed - head;
        for &i in &early {
            let m = &mut scene.motions[i];
            let keep = m.duration.min(CUE_MIN_ENTRANCE);
            m.start = round3((m.start + push).min(life.anticipate - keep));
            if m.start + m.duration > life.anticipate {
                m.duration = round3((life.anticipate - m.start).max(keep));
            }
        }
    }
    Retimed::Moved(round3(first), round3(to), clamped)
}

/// Retime the groups of one built beat scene so each starts entering when
/// the narrator says the words it stands for (scene-local `words`).
///
/// Explicit `anchors` (`B::reveal`) come first, in declaration order: a
/// layer belongs to an anchor when its id minus the scene prefix equals the
/// anchor's group or starts with group + `"."`; the earliest spoken entry
/// among the group's anchors cues it (none spoken: the group stays).
/// Content, value and label groups start [`WORD_CUE_LEAD`] before their
/// word; titles and stamps start on it. Titles move only under
/// `TitleReveal::Hold`. Groups no anchor touches (by the first segment of
/// their ids) fall back to name matching on their layer text and asset
/// names, in group-name order, with the content lead; titles, statement copy
/// and furniture (`FIXED_GROUPS`) never move that way.
///
/// Earlier moves keep the ENTER floor; with `policy.allow_delay` a group may
/// also move later, as long as its arrival still finishes `policy.min_read`
/// before ANTICIPATE (else it moves as late as that allows and the move is
/// recorded as clamped). (0.22) An explicit anchor whose motions leave no
/// room moves its arrival alone, and a Content / Value / Stamp anchor may
/// shorten its arrival to land on its word ([`retime`]); an explicit anchor
/// that still cannot move is recorded in [`WordCues::dropped`], never
/// skipped silently. Groups on a rotating sphere never move. Returns every
/// move and every drop in scene time.
pub(crate) fn apply_word_cues(
    scene: &mut crate::scene::Scene,
    words: &[(String, f64)],
    anchors: &[RevealAnchor],
    policy: &CuePolicy,
) -> WordCues {
    let Some(life) = scene.lifecycle else {
        return WordCues::default();
    };
    let prefix = format!(
        "{}.",
        scene
            .layers
            .first()
            .map(|l| l.id.split('.').next().unwrap_or(""))
            .unwrap_or("")
    );
    let heard = Heard::new(words);
    let mut ids = Vec::new();
    collect_ids(&scene.layers, &mut ids);
    let mut moves = Vec::new();
    let mut dropped = Vec::new();

    // Explicit anchors, in declaration order; a group declared twice is cued
    // by the earliest of its entries.
    let mut done: Vec<&str> = Vec::new();
    let mut claimed: Vec<String> = Vec::new();
    for anchor in anchors {
        let group = anchor.group.as_str();
        if group.is_empty() || done.contains(&group) {
            continue;
        }
        done.push(group);
        let entries: Vec<&RevealAnchor> = anchors.iter().filter(|a| a.group == group).collect();
        if policy.title != TitleReveal::Hold && entries.iter().any(|a| a.role == RevealRole::Title)
        {
            continue; // the title enters with the beat
        }
        let nested = format!("{group}.");
        let members: Vec<String> = ids
            .iter()
            .map(|(id, _)| id)
            .filter(|id| {
                id.strip_prefix(&prefix)
                    .is_some_and(|rest| rest == group || rest.starts_with(&nested))
            })
            .filter(|id| !claimed.contains(id))
            .cloned()
            .collect();
        if members.is_empty() || on_sphere(scene, &members) {
            continue;
        }
        let mut best: Option<(Hit, RevealRole)> = None;
        for a in &entries {
            if let Some(hit) = heard.find(&a.words) {
                if best.as_ref().is_none_or(|(b, _)| hit.at < b.at) {
                    best = Some((hit, a.role));
                }
            }
        }
        let Some((hit, role)) = best else { continue };
        let lead = match role {
            RevealRole::Title | RevealRole::Stamp => 0.0,
            RevealRole::Content | RevealRole::Value | RevealRole::Label => WORD_CUE_LEAD,
        };
        let freedom = match role {
            RevealRole::Content | RevealRole::Value | RevealRole::Stamp => Freedom::Shorten,
            RevealRole::Title | RevealRole::Label => Freedom::Arrival,
        };
        match retime(scene, &members, &life, hit.at, lead, policy, freedom) {
            Retimed::Moved(from, to, clamped) => {
                claimed.extend(members);
                moves.push(WordCueMove {
                    group: group.to_string(),
                    word: hit.word,
                    from,
                    to,
                    role: Some(role),
                    clamped,
                });
            }
            Retimed::Dropped(from) => {
                let drop = WordCueDrop {
                    group: group.to_string(),
                    word: hit.word,
                    at: round3(hit.at),
                    from: round3(from),
                    role,
                };
                // Within the reveal tolerances it still reads on its word.
                let early = drop.early();
                if early > REVEAL_LEAD_MAX + 1e-9 || -early > REVEAL_LATE_MAX + 1e-9 {
                    dropped.push(drop);
                }
            }
            Retimed::Stays => {}
        }
    }

    // Name-matching fallback for the groups no anchor covers.
    let covered: Vec<&str> = anchors
        .iter()
        .map(|a| a.group.split('.').next().unwrap_or(""))
        .collect();
    // group name → (member ids, raw tokens per layer)
    let mut groups: Vec<(String, Vec<String>, Vec<Vec<String>>)> = Vec::new();
    for (id, tokens) in ids {
        let Some(rest) = id.strip_prefix(&prefix) else {
            continue;
        };
        let group = rest.split('.').next().unwrap_or("").to_string();
        if group.is_empty()
            || FIXED_GROUPS.contains(&group.as_str())
            || covered.contains(&group.as_str())
        {
            continue;
        }
        match groups.iter_mut().find(|g| g.0 == group) {
            Some(g) => {
                g.1.push(id);
                g.2.push(tokens);
            }
            None => groups.push((group, vec![id], vec![tokens])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    for (group, members, texts) in groups {
        if on_sphere(scene, &members) {
            continue;
        }
        // The first word of the beat that names this group.
        let mut hit: Option<Hit> = None;
        for text in &texts {
            let raw: Vec<&str> = text.iter().map(String::as_str).collect();
            for span in tokenize(&raw).into_iter().filter(|s| s.tok.content()) {
                if let Some(h) = heard.sequence(std::slice::from_ref(&span.tok)) {
                    hit = earliest(hit, h);
                }
            }
        }
        let Some(hit) = hit else { continue };
        // A name match is a guess: it never reshapes a group, and a group it
        // cannot move simply keeps its planned entrance.
        if let Retimed::Moved(from, to, clamped) = retime(
            scene,
            &members,
            &life,
            hit.at,
            WORD_CUE_LEAD,
            policy,
            Freedom::Rigid,
        ) {
            moves.push(WordCueMove {
                group,
                word: hit.word,
                from,
                to,
                role: None,
                clamped,
            });
        }
    }
    // (0.23 W4) The running total follows the items a cue moved, then every
    // count lands when its number is said.
    follow_items(scene, &moves, &prefix);
    settle_counts(scene, words, anchors);
    WordCues { moves, dropped }
}

// ---------------------------------------------------------------------------
// (0.23 W4) Counts settle when their number is said
// ---------------------------------------------------------------------------

/// A count the fit shortens never runs shorter than this (seconds); a count
/// that was already shorter keeps its length.
pub const COUNT_MIN_S: f64 = 0.4;
/// Seconds a fitted count keeps clear of the landing bound and of the exit.
const COUNT_MARGIN: f64 = 0.05;
/// The frame rate the compiler writes (the QA measures on frames).
const COUNT_FPS: f64 = 30.0;

/// The scene-local time at which the layer first shows the final text of the
/// count `m` (rounded up to the frame the timeline shows it on), computed the
/// way the timeline computes the text: `from + (to - from) * eased progress`.
/// `None` when the curve overshoots (a spring that is not a landing curve) or
/// never reaches the final text.
fn count_shown_at(m: &crate::scene::Motion, scene_start: f64) -> Option<f64> {
    use crate::easing::Easing;
    use crate::scene::MotionOp;
    use crate::timeline::format_count;
    let MotionOp::Count {
        from,
        to,
        decimals,
        grouping,
        prefix,
        suffix,
    } = &m.op
    else {
        return None;
    };
    if m.spring.is_none() && matches!(m.easing, Easing::EditorialSpring | Easing::ImpactSpring) {
        return None;
    }
    let ease = |u: f64| {
        if m.spring.is_some() {
            Easing::OutQuint.apply(u)
        } else {
            m.easing.apply(u)
        }
    };
    let last = format_count(*to, *decimals, *grouping, prefix, suffix);
    let shows =
        |p: f64| format_count(from + (to - from) * p, *decimals, *grouping, prefix, suffix) == last;
    let t = if m.duration <= 0.0 || shows(0.0) {
        m.start
    } else if shows(ease(1.0)) {
        // The text is final from the first progress that rounds to it.
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if shows(ease(mid)) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        m.start + hi * m.duration
    } else {
        return None;
    };
    // The first frame at or after that moment.
    Some(((scene_start + t) * COUNT_FPS - 1e-6).ceil() / COUNT_FPS - scene_start)
}

/// Ids from a top-level layer down to `id` (empty when it is not in the tree).
fn layer_trail<'a>(layers: &'a [crate::scene::Layer], id: &str, out: &mut Vec<&'a str>) -> bool {
    for l in layers {
        out.push(&l.id);
        if l.id == id {
            return true;
        }
        if let crate::scene::LayerKind::Group { children } = &l.kind {
            if layer_trail(children, id, out) {
                return true;
            }
        }
        out.pop();
    }
    false
}

/// (0.23 W4) The running total of an `accumulate` collection steps up as each
/// item lands (`collection::accumulation`: one `Count` step on `total` and one
/// `AccentExpand` step on `bar.fill` per item, the first step together with
/// the fades of `total`, `total.label` and `bar.track`). Those layers are not
/// part of the item group a word cue moves (`items`, or `items.<i>`), so after
/// a cue they kept their planned times and counted up long before or after
/// the items they count (a total still reading "2" a second after the third
/// card landed). Step `i` follows item `i`: by the shift of the group `items`
/// (every item) or `items.<i>`. Durations stay, shortened only so a step ends
/// before the next one starts and before ANTICIPATE. Only a total whose final
/// value shows more than `COUNT_SETTLE_S` after the last card lands, and that
/// following brings back within it, is moved; every other total stays as
/// built.
fn follow_items(scene: &mut crate::scene::Scene, moves: &[WordCueMove], prefix: &str) {
    use crate::scene::MotionOp;
    let Some(life) = scene.lifecycle else {
        return;
    };
    let total = format!("{prefix}total");
    let steps: Vec<usize> = scene
        .motions
        .iter()
        .enumerate()
        .filter(|(_, m)| m.target == total && matches!(m.op, MotionOp::Count { .. }))
        .map(|(i, _)| i)
        .collect();
    if steps.is_empty() {
        return;
    }
    let fill = format!("{prefix}bar.fill");
    let fills: Vec<usize> = scene
        .motions
        .iter()
        .enumerate()
        .filter(|(_, m)| m.target == fill && matches!(m.op, MotionOp::AccentExpand { .. }))
        .map(|(i, _)| i)
        .collect();
    let mut delta = vec![0.0f64; steps.len()];
    for mv in moves {
        let Some(rest) = mv.group.strip_prefix("items") else {
            continue;
        };
        let d = mv.to - mv.from;
        if rest.is_empty() {
            delta.iter_mut().for_each(|x| *x += d);
        } else if let Some(x) = rest
            .strip_prefix('.')
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|i| delta.get_mut(i))
        {
            *x += d;
        }
    }
    if delta.iter().all(|d| d.abs() < 1e-9) {
        return;
    }
    // Only a total that lags the last item it counts (its final value shows
    // more than `COUNT_SETTLE_S` after that card lands) follows, and only when
    // following brings it back within that: a total a little off its items is
    // not a defect, and a planned lag the cue did not cause stays.
    let Some(&last_step) = steps.last() else {
        return;
    };
    let item = format!("{prefix}items.{}", steps.len() - 1);
    let nested = format!("{item}.");
    let arrival = scene
        .motions
        .iter()
        .filter(|m| m.target == item || m.target.starts_with(&nested))
        .map(|m| m.start)
        .reduce(f64::min);
    let (Some(arrival), Some(shown)) = (
        arrival,
        count_shown_at(&scene.motions[last_step], scene.start_seconds),
    ) else {
        return;
    };
    let lag = shown - arrival;
    let shift = delta[steps.len() - 1];
    if lag <= crate::checks::COUNT_SETTLE_S || lag + shift > crate::checks::COUNT_SETTLE_S {
        return;
    }
    // The total, its label and the bar track arrive with the first item.
    let arrivals = [
        total.clone(),
        format!("{prefix}total.label"),
        format!("{prefix}bar.track"),
    ];
    for m in scene.motions.iter_mut() {
        if arrivals.contains(&m.target) && matches!(m.op, MotionOp::Fade { from, to } if to > from)
        {
            m.start = round3((m.start + delta[0]).max(0.0));
        }
    }
    for (k, &i) in steps.iter().enumerate() {
        let m = &mut scene.motions[i];
        m.start = round3((m.start + delta[k]).max(0.0));
    }
    for (k, &i) in fills.iter().enumerate() {
        let m = &mut scene.motions[i];
        m.start = round3((m.start + delta.get(k).copied().unwrap_or(0.0)).max(0.0));
    }
    // Each step ends before the next begins, and before ANTICIPATE.
    for (k, &i) in steps.iter().enumerate() {
        let limit = steps
            .get(k + 1)
            .map_or(life.anticipate, |&next| scene.motions[next].start);
        let m = &mut scene.motions[i];
        if m.start + m.duration > limit {
            m.duration = round3((limit - m.start).max(0.05));
        }
        let (duration, start) = (m.duration, m.start);
        if let Some(&f) = fills.get(k) {
            let fm = &mut scene.motions[f];
            fm.start = start;
            fm.duration = duration;
        }
    }
}

/// (0.23 W4) Make every counting number of a built beat scene land on the
/// word it is said with and hold its final value before the beat moves on
/// (`checks::COUNT_UNSETTLED`).
///
/// A count is judged by the first frame that shows its final text (`ds`):
/// `ds` must be within `COUNT_SETTLE_S` of its anchor and at least
/// `COUNT_HOLD_S` before the layer's exit. The anchor is the start of the word
/// the number is said with (its own value spoken, else the earliest spoken
/// word of the reveal group it belongs to, else READ), and never earlier than
/// the count's own start: a layer that arrives after READ (an EVOLVE-staged
/// number) has until 0.8 s after it arrives. The exit is ANTICIPATE or the
/// layer's (or its parent's) first fade-out, whichever is first.
///
/// Only a defective count is touched, and only its duration (never its start,
/// so every reveal anchor, cue and entrance stays where it was): it is
/// shortened to end within the landing bound and, when that still leaves a
/// count of at least [`COUNT_MIN_S`], before `exit - COUNT_HOLD_S`. When the
/// layer arrives too close to its exit for a real count and the hold (the
/// beat is too short, or the number is named too late in it), landing on the
/// word wins: the count keeps at least [`COUNT_MIN_S`] (it must still read as
/// a count) and the hold is whatever the beat leaves after it. A count that
/// would still be running when the beat anticipates is shortened to end
/// before it (never under that floor). A count that only lacks hold and
/// cannot gain it is left exactly as built.
///
/// `words` are the beat's spoken words (scene-local; empty without a voice-over:
/// the anchor is then READ). Chained counts (a running total) are judged on
/// their last step.
pub(crate) fn settle_counts(
    scene: &mut crate::scene::Scene,
    words: &[(String, f64)],
    anchors: &[RevealAnchor],
) {
    use crate::checks::{COUNT_HOLD_S, COUNT_SETTLE_S};
    use crate::scene::MotionOp;
    let Some(life) = scene.lifecycle else {
        return;
    };
    let prefix = format!(
        "{}.",
        scene
            .layers
            .first()
            .map(|l| l.id.split('.').next().unwrap_or(""))
            .unwrap_or("")
    );
    let heard = Heard::new(words);
    // The last count motion of each counting layer, in scene order.
    let mut last: Vec<(String, usize)> = Vec::new();
    for (i, m) in scene.motions.iter().enumerate() {
        if !matches!(m.op, MotionOp::Count { .. }) {
            continue;
        }
        match last.iter_mut().find(|(t, _)| *t == m.target) {
            Some(slot) => slot.1 = i,
            None => last.push((m.target.clone(), i)),
        }
    }
    for (target, i) in last {
        let m = &scene.motions[i];
        let Some(shown) = count_shown_at(m, scene.start_seconds) else {
            continue;
        };
        let (s, d) = (m.start, m.duration);
        // The word the number is said with.
        let word = match &m.op {
            MotionOp::Count {
                to,
                decimals,
                grouping,
                prefix: pre,
                suffix,
                ..
            } if !words.is_empty() => {
                let text = crate::timeline::format_count(*to, *decimals, *grouping, pre, suffix);
                heard.find(&[text]).map(|h| h.at).or_else(|| {
                    let rest = target.strip_prefix(&prefix)?;
                    anchors
                        .iter()
                        .filter(|a| rest == a.group || rest.starts_with(&format!("{}.", a.group)))
                        .filter_map(|a| heard.find(&a.words))
                        .fold(None, earliest)
                        .map(|h| h.at)
                })
            }
            _ => None,
        };
        let anchor = word.unwrap_or(life.read).max(s);
        // The layer's exit: ANTICIPATE, or its first fade-out.
        let mut trail = Vec::new();
        layer_trail(&scene.layers, &target, &mut trail);
        let exit = scene
            .motions
            .iter()
            .filter(|o| trail.contains(&o.target.as_str()) && o.start >= s)
            .filter_map(|o| match o.op {
                MotionOp::Fade { from, to } if to < from => Some(o.start),
                _ => None,
            })
            .fold(life.anticipate, f64::min);
        let late = shown > anchor + COUNT_SETTLE_S + 1e-9;
        let short = exit - shown < COUNT_HOLD_S - 1e-9;
        if !late && !short {
            continue;
        }
        let floor = d.min(COUNT_MIN_S);
        let mut cap = anchor + COUNT_SETTLE_S - COUNT_MARGIN;
        // The hold: a count that can still end `COUNT_HOLD_S` before the
        // exit as a real count (not shorter than the floor) does; one whose
        // layer arrives too late for that keeps its length (the beat, not
        // the count, limits the hold) unless it lands late.
        let hold_cap = exit - COUNT_HOLD_S;
        if hold_cap - s >= floor {
            cap = cap.min(hold_cap);
        } else if shown > exit {
            // No hold to be had, but the number should not still be counting
            // when the beat moves on (unless that takes it under the floor).
            cap = cap.min(exit - COUNT_MARGIN);
        } else if !late {
            continue;
        }
        let fitted = d.min(cap - s).max(floor);
        if fitted < d - 1e-6 {
            scene.motions[i].duration = round3(fitted);
        }
    }
}

#[cfg(test)]
mod word_cue_tests {
    use super::*;

    fn w(list: &[(&str, f64)]) -> Vec<(String, f64)> {
        list.iter().map(|(a, b)| (norm(a), *b)).collect()
    }

    #[test]
    fn name_time_finds_the_first_spoken_name_and_matches_plurals() {
        let spoken = w(&[
            ("According", 0.3),
            ("to", 0.6),
            ("botanical", 0.9),
            ("bananas", 2.0),
            ("fit", 2.4),
            ("strawberries", 4.0),
            ("don't", 4.5),
        ]);
        assert_eq!(
            name_time(&spoken, &["banana", "fits the definition"]),
            Some(2.0)
        );
        assert_eq!(name_time(&spoken, &["strawberry"]), Some(4.0));
        // "fit" is spoken at 2.4, but "bananas" comes first.
        assert_eq!(name_time(&spoken, &["a true berry"]), None);
        // Stop words and short words never count.
        assert_eq!(name_time(&spoken, &["to the"]), None);
    }

    // --- matching ---------------------------------------------------------

    /// Words of `text` spoken `step` seconds apart from `start`.
    fn said(text: &str, start: f64, step: f64) -> Vec<(String, f64)> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, x)| (x.to_string(), round3(start + i as f64 * step)))
            .collect()
    }

    fn tok(word: &str) -> Tok {
        tokenize(&[word]).remove(0).tok
    }

    /// The single number token `text` reads as.
    fn value(text: &str) -> Option<Num> {
        let raw: Vec<&str> = text.split_whitespace().collect();
        match tokenize(&raw).as_slice() {
            [Span {
                tok: Tok::Num(n), ..
            }] => Some(*n),
            _ => None,
        }
    }

    fn num(v: u128) -> Num {
        Num {
            micro: v * MICRO,
            percent: false,
        }
    }

    #[test]
    fn stemming_table() {
        for (word, expect) in [
            ("berries", "berry"),
            ("berry", "berry"),
            ("markets", "market"),
            ("running", "run"),
            ("sharks", "shark"),
            ("Buffett's", "buffett"),
            ("oldest", "oldest"),
            ("markets'", "market"),
            ("cities", "city"),
            ("glasses", "glass"),
            ("boxes", "box"),
            ("stopped", "stop"),
            ("studied", "study"),
            ("buildings", "build"),
            ("playing", "play"),
            // Not plurals / not inflections / too short.
            ("is", "is"),
            ("this", "this"),
            ("bus", "bus"),
            ("plus", "plus"),
            ("yes", "yes"),
            ("focus", "focus"),
            ("basis", "basis"),
            ("glass", "glass"),
            ("evening", "evening"),
            ("speed", "speed"),
            ("king", "king"),
            ("red", "red"),
        ] {
            assert_eq!(stem(word), expect, "{word}");
        }
        // Pairs that name the same thing (they share a form).
        for (a, b) in [
            ("berries", "berry"),
            ("markets", "market"),
            ("running", "run"),
            ("sharks", "shark"),
            ("Buffett's", "Buffett"),
            ("making", "make"),
            ("changed", "change"),
            ("movies", "movie"),
            ("headaches", "headache"),
            ("houses", "house"),
            ("buses", "bus"),
            ("focuses", "focus"),
            ("shapes", "shape"),
            ("PRICES", "price"),
        ] {
            assert!(tok(a).matches(&tok(b)), "{a} / {b}");
        }
        // Pairs that do not.
        for (a, b) in [
            ("evening", "even"),
            ("planes", "plan"),
            ("oldest", "old"),
            ("strawberries", "berry"),
            ("speed", "sped"),
        ] {
            assert!(!tok(a).matches(&tok(b)), "{a} / {b}");
        }
    }

    #[test]
    fn number_phrases_read_the_same_said_or_written() {
        let pct = |v: u128| Num {
            micro: v * MICRO,
            percent: true,
        };
        let table = [
            (
                "three hundred and eighty one billion",
                "$381 BILLION",
                num(381_000_000_000),
            ),
            (
                "three hundred eighty one billion",
                "381B",
                num(381_000_000_000),
            ),
            ("four point five million", "4.5M", num(4_500_000)),
            ("four point five million", "4.5 million", num(4_500_000)),
            ("twenty three percent", "23%", pct(23)),
            ("twenty-three per cent", "23 %", pct(23)),
            ("eight", "8", num(8)),
            ("nineteen sixty nine", "1969", num(1969)),
            ("twenty twenty four", "2024", num(2024)),
            ("two thousand and twenty four", "2,024", num(2024)),
            ("nineteen hundred", "1900", num(1900)),
            ("one hundred and twelve", "112", num(112)),
            ("fifty thousand", "₹50,000", num(50_000)),
            ("a million", "$1m", num(1_000_000)),
            ("five lakh", "500,000", num(500_000)),
            (
                "one quadrillion",
                "1,000,000,000,000,000",
                num(10u128.pow(15)),
            ),
            (
                "nine hundred and ninety nine trillion",
                "999T",
                num(999_000_000_000_000),
            ),
        ];
        for (spoken, written, expect) in table {
            assert_eq!(value(spoken), Some(expect), "said: {spoken}");
            assert_eq!(value(written), Some(expect), "written: {written}");
        }
        assert_eq!(
            value("two point five"),
            Some(Num {
                micro: 2_500_000,
                percent: false
            })
        );
        assert_eq!(value("−27%"), Some(pct(27)));
        assert_eq!(value("+38%"), Some(pct(38)));
        // Percent flags must agree for a match.
        assert!(!tok("23%").matches(&value("twenty three").map(Tok::Num).unwrap()));
        // Two numbers in a row stay two numbers; "one and a half" is out of
        // scope ("one", then words).
        assert_eq!(tokenize(&["five", "ten"]).len(), 2);
        assert_eq!(
            tokenize(&["one", "and", "a", "half"])[0].tok,
            Tok::Num(num(1))
        );
        // Spoken numbers in a sentence, with the start of their first word.
        let heard = Heard::new(&said(
            "under ten percent and 23% of five thousand two hundred",
            1.0,
            0.2,
        ));
        let nums: Vec<(Num, f64)> = heard
            .toks
            .iter()
            .filter_map(|(t, at, _)| match t {
                Tok::Num(n) => Some((*n, *at)),
                Tok::Word { .. } => None,
            })
            .collect();
        assert_eq!(nums, vec![(pct(10), 1.2), (pct(23), 1.8), (num(5200), 2.2)]);
        assert_eq!(text_number("₹5,200").as_deref(), Some("5200"));
        assert_eq!(text_number("10%").as_deref(), Some("10%"));
        // The lenient reading stays for tokens with stray letters.
        assert_eq!(value("1990s"), Some(num(1990)));
    }

    #[test]
    fn anchor_time_matches_numbers_by_value_both_ways() {
        let spoken = said(
            "by then the debt hit three hundred and eighty one billion dollars",
            0.5,
            0.25,
        );
        // "three" is the 6th word.
        assert_eq!(anchor_time(&spoken, &["$381 billion"]), Some(1.75));
        assert_eq!(anchor_time(&spoken, &["$381 BILLION"]), Some(1.75));
        assert_eq!(anchor_time(&spoken, &["$380 billion"]), None);
        let digits = said("the debt hit $381 billion", 0.0, 0.3);
        assert_eq!(
            anchor_time(&digits, &["three hundred and eighty one billion"]),
            Some(0.9)
        );
        let year = said("it landed in nineteen sixty nine", 0.0, 0.3);
        assert_eq!(anchor_time(&year, &["1969"]), Some(0.9));
        let share = said("about twenty three percent of it", 0.0, 0.3);
        assert_eq!(anchor_time(&share, &["23%"]), Some(0.3));
        assert_eq!(anchor_time(&share, &["23"]), None);
        // A plural keyword finds its singular, an inflection its stem.
        let shark = said("one shark keeps running", 0.0, 0.4);
        assert_eq!(anchor_time(&shark, &["sharks"]), Some(0.4));
        assert_eq!(anchor_time(&shark, &["run"]), Some(1.2));
        // A deliberate short anchor ("ai") matches exactly; function words never do.
        assert_eq!(anchor_time(&said("it is ai", 0.0, 0.3), &["ai"]), Some(0.6));
        assert_eq!(anchor_time(&said("not this", 0.0, 0.3), &["not"]), None);
        assert_eq!(anchor_time(&said("it is ai", 0.0, 0.3), &["is"]), None);
    }

    #[test]
    fn multi_word_entries_match_in_order_within_the_gap_window() {
        let spoken = said("and then wait what happened", 1.0, 0.25);
        assert_eq!(anchor_time(&spoken, &["wait, what"]), Some(1.5));
        // Up to two other words in between...
        let two = said("buying all that power", 0.0, 0.5);
        assert_eq!(anchor_time(&two, &["buying power"]), Some(0.0));
        // ...but not three.
        let three = said("buying all of that power", 0.0, 0.5);
        assert_eq!(anchor_time(&three, &["buying power"]), None);
        // Out of order never matches; each word alone would.
        let swapped = said("power before buying", 0.0, 0.5);
        assert_eq!(anchor_time(&swapped, &["buying power"]), None);
        assert_eq!(anchor_time(&swapped, &["buying", "power"]), Some(0.0));
        // The earliest complete occurrence wins, timed at its first word.
        let again = said("power grid then buying power", 0.0, 0.5);
        assert_eq!(anchor_time(&again, &["buying power"]), Some(1.5));
        // Stop words inside an entry are skipped; a number phrase counts as
        // one word on both sides.
        let debt = said("costs three hundred billion a year", 0.0, 0.2);
        assert_eq!(anchor_time(&debt, &["costs the $300 billion"]), Some(0.0));
        let found = Heard::new(&debt).find(&["costs $300B"]);
        assert_eq!(
            found.map(|h| h.word).as_deref(),
            Some("costs three hundred billion")
        );
    }

    #[test]
    fn a_single_short_anchor_matches_exactly_but_stop_words_never_do() {
        let spoken = w(&[
            ("the", 0.1),
            ("age", 0.3),
            ("of", 0.5),
            ("AI", 0.7),
            ("is", 0.9),
        ]);
        assert_eq!(anchor_time(&spoken, &["AI"]), Some(0.7));
        assert_eq!(anchor_time(&spoken, &["the"]), None);
        assert_eq!(anchor_time(&spoken, &["of"]), None);
        // Inside a longer entry short words still only count in sequence.
        assert_eq!(anchor_time(&spoken, &["age of AI"]), Some(0.3));
    }

    #[test]
    fn abbreviations_match_their_spoken_expansion() {
        let spoken = w(&[
            ("takes", 0.2),
            ("eight", 0.5),
            ("minutes", 0.8),
            ("to", 1.1),
        ]);
        assert_eq!(anchor_time(&spoken, &["8 min"]), Some(0.5));
        assert_eq!(anchor_time(&spoken, &["min"]), Some(0.8));
        let money = w(&[("about", 0.1), ("two", 0.4), ("billion", 0.7)]);
        assert_eq!(anchor_time(&money, &["$2bn"]), Some(0.4));
    }

    /// (0.22) An external consumer reported that "4.1%" could not match
    /// "four point one percent". It always did: numbers match by value,
    /// written or said, in either direction.
    #[test]
    fn decimal_percentages_and_amounts_match_their_spoken_form() {
        let num_pct = |micro: u128| Num {
            micro,
            percent: true,
        };
        for (written, spoken, expect) in [
            ("4.1%", "four point one percent", num_pct(4_100_000)),
            ("4.1 percent", "four point one percent", num_pct(4_100_000)),
            (
                "4.1 per cent",
                "four point one per cent",
                num_pct(4_100_000),
            ),
            ("3.4%", "three point four per cent", num_pct(3_400_000)),
            ("0.5%", "zero point five percent", num_pct(500_000)),
            (
                "12.25%",
                "twelve point two five percent",
                num_pct(12_250_000),
            ),
        ] {
            assert_eq!(value(written), Some(expect), "written: {written}");
            assert_eq!(value(spoken), Some(expect), "said: {spoken}");
            // As anchors, both ways round, timed at the number's first word.
            let line = said(&format!("inflation hit {spoken} last year"), 1.0, 0.25);
            assert_eq!(anchor_time(&line, &[written]), Some(1.5), "{written}");
            let line = said(&format!("inflation hit {written} last year"), 1.0, 0.25);
            assert_eq!(anchor_time(&line, &[spoken]), Some(1.5), "{spoken}");
        }
        // The percent sign must agree: "4.1%" is not a bare 4.1, and not 4.
        let bare = said("it rose four point one points", 0.0, 0.3);
        assert_eq!(anchor_time(&bare, &["4.1%"]), None);
        let four = said("it rose four percent", 0.0, 0.3);
        assert_eq!(anchor_time(&four, &["4.1%"]), None);

        // Amounts: the currency sign and grouping commas are written, the
        // currency name is said after the number.
        let tank = said(
            "a full tank costs seven thousand six hundred dollars",
            0.0,
            0.3,
        );
        assert_eq!(anchor_time(&tank, &["$7,600"]), Some(1.2));
        assert_eq!(anchor_time(&tank, &["$7,600 a tank"]), None);
        assert_eq!(anchor_time(&tank, &["$7,500"]), None);
        let rupees = said("about five thousand two hundred rupees a month", 0.0, 0.3);
        assert_eq!(anchor_time(&rupees, &["₹5,200"]), Some(0.3));
    }

    /// (0.22) Prices with cents are said "one dollar fifteen": the cents
    /// after the currency name belong to the amount.
    #[test]
    fn spoken_cents_belong_to_the_amount() {
        let price = |v: u128| Num {
            micro: v * 10_000,
            percent: false,
        };
        for (spoken, cents) in [
            ("one dollar fifteen", 115),
            ("a dollar fifteen", 115),
            ("one dollar and fifteen cents", 115),
            ("one dollar fifteen cents", 115),
            ("two pounds fifty", 250),
            ("two pounds and fifty pence", 250),
            ("three euros five", 305),
            ("ninety nine rupees ninety nine paise", 9999),
        ] {
            assert_eq!(value(spoken), Some(price(cents)), "said: {spoken}");
        }
        assert_eq!(value("$1.15"), Some(price(115)));
        let line = said("diesel is one dollar fifteen a litre here", 0.5, 0.25);
        assert_eq!(anchor_time(&line, &["$1.15"]), Some(1.0));
        assert_eq!(anchor_time(&line, &["$1"]), None);
        let line = said("diesel costs $1.15 a litre", 0.0, 0.3);
        assert_eq!(anchor_time(&line, &["one dollar fifteen"]), Some(0.6));
        // Not cents: "and" without a minor unit, a big whole amount without
        // one, a clause break after the currency, or more than 99.
        for spoken in [
            "two dollars and fifty people",
            "two thousand dollars twenty years ago",
            "two dollars, fifty people",
            "two dollars one hundred times",
        ] {
            let heard = Heard::new(&said(spoken, 0.0, 0.3));
            assert!(
                matches!(heard.toks.first(), Some((Tok::Num(n), _, _)) if n.micro % MICRO == 0),
                "{spoken}: {:?}",
                heard.toks.first()
            );
            assert!(
                heard
                    .toks
                    .iter()
                    .any(|(t, _, _)| matches!(t, Tok::Word { norm, .. }
                    if norm.starts_with("dollar"))),
                "{spoken}"
            );
        }
        // A bare "a dollar" stays words; "seven thousand six hundred
        // dollars" keeps its currency word for a keyword anchor.
        assert_eq!(tokenize(&["a", "dollar"]).len(), 2);
        let tank = said("seven thousand six hundred dollars", 0.0, 0.3);
        assert_eq!(anchor_time(&tank, &["dollars"]), Some(1.2));
    }

    #[test]
    fn first_term_takes_a_leading_number_phrase_whole() {
        assert_eq!(
            first_term("$381 BILLION owed").as_deref(),
            Some("$381 BILLION")
        );
        assert_eq!(
            first_term("4.5 million tons").as_deref(),
            Some("4.5 million")
        );
        assert_eq!(first_term("flat salary").as_deref(), Some("flat"));
        assert_eq!(first_term("−27%").as_deref(), Some("−27%"));
        assert_eq!(first_term("  ").as_deref(), None);
    }

    // --- cues -------------------------------------------------------------

    use crate::easing::Easing;
    use crate::scene::{
        Color, FontRole, Layer, LayerKind, Lifecycle, Motion, MotionOp, Scene, TextAlign, TextStyle,
    };
    use crate::speech::CUE_MIN_READ;

    const LIFE: Lifecycle = Lifecycle {
        enter: 0.3,
        settle: 1.2,
        read: 1.6,
        evolve: 3.0,
        anticipate: 5.0,
        bridge: 5.5,
    };

    fn policy(title: TitleReveal) -> CuePolicy {
        CuePolicy {
            allow_delay: true,
            min_read: CUE_MIN_READ,
            title,
            focal: None,
        }
    }

    fn layer(id: &str, kind: LayerKind) -> Layer {
        Layer {
            id: id.to_string(),
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            opacity: 1.0,
            z_index: 0,
            visible: true,
            clip: None,
            depth: None,
            z: None,
            tilt: None,
            layout: None,
            kind,
        }
    }

    fn text(id: &str, s: &str) -> Layer {
        layer(
            id,
            LayerKind::Text(TextStyle {
                text: s.to_string(),
                font_role: FontRole::Display,
                font_size: 48.0,
                font_weight: 700,
                italic: false,
                color: Color::rgb(20, 20, 20),
                align: TextAlign::Left,
                line_height: 1.1,
                letter_spacing: 0.0,
                max_width: None,
                uppercase: false,
                ink: None,
            }),
        )
    }

    fn image(id: &str, asset: &str) -> Layer {
        layer(
            id,
            LayerKind::Image {
                asset: asset.to_string(),
                fit: Default::default(),
                treatment: None,
                playback: None,
                insert: None,
            },
        )
    }

    fn group(id: &str, children: Vec<Layer>) -> Layer {
        layer(id, LayerKind::Group { children })
    }

    fn motion(target: &str, start: f64, duration: f64, op: MotionOp) -> Motion {
        Motion {
            id: None,
            target: target.to_string(),
            start,
            duration,
            easing: Easing::default(),
            spring: None,
            op,
        }
    }

    fn fade(target: &str, start: f64, duration: f64) -> Motion {
        motion(
            target,
            start,
            duration,
            MotionOp::Fade { from: 0.0, to: 1.0 },
        )
    }

    /// An entrance (fade + rise) starting at `start` and an exit fade in
    /// ANTICIPATE for `target`.
    fn enters(target: &str, start: f64) -> Vec<Motion> {
        vec![
            fade(target, start, 0.5),
            motion(
                target,
                start + 0.1,
                0.6,
                MotionOp::Move {
                    from: [0.0, 40.0],
                    to: [0.0, 0.0],
                },
            ),
            motion(target, 5.1, 0.3, MotionOp::Fade { from: 1.0, to: 0.0 }),
        ]
    }

    fn scene(children: Vec<Layer>, motions: Vec<Motion>) -> Scene {
        Scene {
            id: "beat_1".to_string(),
            start_seconds: 0.0,
            duration_seconds: 6.0,
            layers: vec![group("b1.stage", children)],
            motions,
            camera: None,
            lifecycle: Some(LIFE),
            post: Vec::new(),
        }
    }

    fn anchor(group: &str, words: &[&str], role: RevealRole) -> RevealAnchor {
        RevealAnchor {
            group: group.to_string(),
            words: words.iter().map(|w| w.to_string()).collect(),
            role,
        }
    }

    /// Starts of the motions on `target`, in scene order.
    fn starts(scene: &Scene, target: &str) -> Vec<f64> {
        scene
            .motions
            .iter()
            .filter(|m| m.target == target)
            .map(|m| m.start)
            .collect()
    }

    fn hero_scene(start: f64) -> Scene {
        scene(
            vec![image("b1.hero", "asset.reef_shark")],
            enters("b1.hero", start),
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// The moves of [`apply_word_cues`] (drops are checked where they matter).
    fn moves_of(
        scene: &mut Scene,
        words: &[(String, f64)],
        anchors: &[RevealAnchor],
        policy: &CuePolicy,
    ) -> Vec<WordCueMove> {
        apply_word_cues(scene, words, anchors, policy).moves
    }

    #[test]
    fn an_anchor_moves_its_group_earlier_down_to_enter() {
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        let hold = policy(TitleReveal::Hold);
        // Named at 1.0: the hero starts 0.12 s before, offsets kept, the exit
        // (in ANTICIPATE) stays.
        let mut s = hero_scene(2.0);
        let moves = moves_of(&mut s, &said("sharks circle", 1.0, 0.3), &anchors, &hold);
        assert_eq!(
            moves,
            vec![WordCueMove {
                group: "hero".into(),
                word: "sharks".into(),
                from: 2.0,
                to: 0.88,
                role: Some(RevealRole::Content),
                clamped: false,
            }]
        );
        assert_eq!(starts(&s, "b1.hero"), vec![0.88, 0.98, 5.1]);
        // Named right after ENTER: the lead is cut at ENTER; on its word, so
        // not clamped.
        let mut s = hero_scene(2.0);
        let moves = moves_of(&mut s, &said("sharks", 0.35, 0.3), &anchors, &hold);
        assert_eq!((moves[0].to, moves[0].clamped), (0.3, false));
        // Named before ENTER (in the previous scene's bridge): held at ENTER,
        // clamped.
        let mut s = hero_scene(2.0);
        let moves = moves_of(&mut s, &said("sharks", 0.1, 0.3), &anchors, &hold);
        assert_eq!((moves[0].to, moves[0].clamped), (0.3, true));
        assert_eq!(starts(&s, "b1.hero"), vec![0.3, 0.4, 5.1]);
        // Already on time (within the minimum shift): nothing moves.
        let mut s = hero_scene(0.95);
        assert!(moves_of(&mut s, &said("sharks", 1.0, 0.3), &anchors, &hold).is_empty());
        assert_eq!(starts(&s, "b1.hero"), vec![0.95, 1.05, 5.1]);
        // Not spoken: the group stays and nothing is recorded.
        let mut s = hero_scene(2.0);
        assert!(moves_of(&mut s, &said("whales", 1.0, 0.3), &anchors, &hold).is_empty());
        assert_eq!(starts(&s, "b1.hero"), vec![2.0, 2.1, 5.1]);
    }

    #[test]
    fn a_cue_never_moves_back_to_back_motions_into_each_other() {
        // (0.23 A4-short) Two moves of one layer that touch: rounding each start
        // to the millisecond on its own used to leave the first running 0.4 ms
        // into the second after a cue, which the validator rejects.
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        let hold = policy(TitleReveal::Hold);
        let mut s = hero_scene(1.0006);
        s.motions = vec![
            motion(
                "b1.hero",
                1.0006,
                0.4994,
                MotionOp::Move {
                    from: [0.0, 40.0],
                    to: [0.0, 0.0],
                },
            ),
            motion(
                "b1.hero",
                1.5,
                0.4,
                MotionOp::Move {
                    from: [0.0, 0.0],
                    to: [0.0, -10.0],
                },
            ),
        ];
        // Named at 1.5, so the hero (planned at 1.0006) is cued 0.2 s later.
        let moves = moves_of(&mut s, &said("sharks", 1.5, 0.3), &anchors, &hold);
        assert_eq!(moves.len(), 1);
        assert!(close(moves[0].to, 1.38), "{:?}", moves[0]);
        let (a, b) = (&s.motions[0], &s.motions[1]);
        assert!(
            a.start + a.duration <= b.start + 1e-9,
            "{} + {} runs into {}",
            a.start,
            a.duration,
            b.start
        );
        // The offset between the two is what it was.
        assert!(close(b.start - a.start, 1.5 - 1.0006));
    }

    #[test]
    fn a_cue_that_rounds_cleanly_still_rounds_to_the_millisecond() {
        // Valid output is unchanged: starts are still whole milliseconds.
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        let hold = policy(TitleReveal::Hold);
        let mut s = hero_scene(2.0004);
        moves_of(&mut s, &said("sharks circle", 1.0, 0.3), &anchors, &hold);
        for m in s.motions.iter().filter(|m| m.target == "b1.hero") {
            assert!(
                close(m.start, round3(m.start)),
                "{} is not on a millisecond",
                m.start
            );
        }
    }

    #[test]
    fn an_anchor_may_delay_its_group_while_it_can_still_be_read() {
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        let hold = policy(TitleReveal::Hold);
        // Planned at 0.5, named at 3.0: enters at 2.88; it lands at 3.58,
        // well before ANTICIPATE - min_read = 4.4.
        let mut s = hero_scene(0.5);
        let moves = moves_of(&mut s, &said("a reef shark", 2.4, 0.3), &anchors, &hold);
        assert_eq!(
            (moves[0].from, moves[0].to, moves[0].clamped),
            (0.5, 2.88, false)
        );
        assert_eq!(starts(&s, "b1.hero"), vec![2.88, 2.98, 5.1]);
        // Without `allow_delay` later words never move a group.
        let mut s = hero_scene(0.5);
        let early_only = CuePolicy {
            allow_delay: false,
            ..hold
        };
        assert!(moves_of(
            &mut s,
            &said("a reef shark", 2.4, 0.3),
            &anchors,
            &early_only
        )
        .is_empty());
        assert_eq!(starts(&s, "b1.hero"), vec![0.5, 0.6, 5.1]);
    }

    /// Durations of the motions on `target`, in scene order.
    fn durations(scene: &Scene, target: &str) -> Vec<f64> {
        scene
            .motions
            .iter()
            .filter(|m| m.target == target)
            .map(|m| m.duration)
            .collect()
    }

    /// When the last motion of `target` that starts before ANTICIPATE ends.
    fn arrival_end(scene: &Scene, target: &str) -> f64 {
        scene
            .motions
            .iter()
            .filter(|m| m.target == target && m.start < LIFE.anticipate)
            .map(|m| m.start + m.duration)
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_late_word_is_clamped_by_the_read_floor() {
        // Named at 4.6, after the read floor (5.0 - 0.6 = 4.4).
        let late = said("shark", 4.6, 0.3);
        let hold = policy(TitleReveal::Hold);
        // A label keeps its entrance whole: it (0.5 .. 1.2) may only shift
        // until it ends on the floor, so it starts at 3.7 and is clamped.
        let label = [anchor("hero", &["shark"], RevealRole::Label)];
        let mut s = hero_scene(0.5);
        let moves = moves_of(&mut s, &late, &label, &hold);
        assert_eq!(
            moves,
            vec![WordCueMove {
                group: "hero".into(),
                word: "shark".into(),
                from: 0.5,
                to: 3.7,
                role: Some(RevealRole::Label),
                clamped: true,
            }]
        );
        assert_eq!(durations(&s, "b1.hero"), vec![0.5, 0.6, 0.3]);
        assert!(close(
            arrival_end(&s, "b1.hero"),
            LIFE.anticipate - CUE_MIN_READ
        ));
        // (0.22) The content it names gets there later with a shorter
        // entrance: offsets kept, each motion cut to end on the floor but
        // never below CUE_MIN_ENTRANCE, so it starts at 4.4 - (0.1 + 0.25)
        // = 4.05. Clamped: the word itself comes too late.
        let content = [anchor("hero", &["shark"], RevealRole::Content)];
        let mut s = hero_scene(0.5);
        let moves = moves_of(&mut s, &late, &content, &hold);
        assert_eq!((moves[0].to, moves[0].clamped), (4.05, true));
        assert_eq!(starts(&s, "b1.hero"), vec![4.05, 4.15, 5.1]);
        assert_eq!(durations(&s, "b1.hero"), vec![0.35, 0.25, 0.3]);
        assert!(close(
            arrival_end(&s, "b1.hero"),
            LIFE.anticipate - CUE_MIN_READ
        ));
        // A name match (no anchor) is a guess and never reshapes a group:
        // the whole entrance shifts, as before 0.22.
        let mut s = scene(
            vec![text("b1.note", "SHARK")],
            vec![fade("b1.note", 0.5, 0.5), fade("b1.note", 0.6, 0.6)],
        );
        let moves = moves_of(&mut s, &late, &[], &hold);
        assert_eq!((moves[0].to, moves[0].role), (3.7, None));
        assert_eq!(durations(&s, "b1.note"), vec![0.5, 0.6]);
    }

    /// The hero's entrance at 0.5 and a slow push from 3.0 until 4.35
    /// (EVOLVE), which leaves the rigid shift no room at all.
    fn pushed_hero_scene() -> Scene {
        let mut motions = enters("b1.hero", 0.5);
        motions.push(motion(
            "b1.hero",
            3.0,
            1.35,
            MotionOp::Scale {
                from: 1.0,
                to: 1.05,
                axis: Default::default(),
            },
        ));
        scene(vec![image("b1.hero", "asset.reef_shark")], motions)
    }

    #[test]
    fn a_later_event_no_longer_blocks_the_arrival() {
        let hold = policy(TitleReveal::Hold);
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        // Named at 2.0: the arrival alone moves (1.88 .. 2.58); the push
        // keeps its time, after the arrival.
        let mut s = pushed_hero_scene();
        let cues = apply_word_cues(&mut s, &said("shark", 2.0, 0.3), &anchors, &hold);
        assert!(cues.dropped.is_empty());
        assert_eq!(
            (cues.moves[0].from, cues.moves[0].to, cues.moves[0].clamped),
            (0.5, 1.88, false)
        );
        assert_eq!(starts(&s, "b1.hero"), vec![1.88, 1.98, 5.1, 3.0]);
        assert_eq!(durations(&s, "b1.hero"), vec![0.5, 0.6, 0.3, 1.35]);
        // Named at 3.2: the arrival (3.08, landing at 3.78) passes the
        // push's start, so the push follows it and is cut to end by
        // ANTICIPATE.
        let mut s = pushed_hero_scene();
        let cues = apply_word_cues(&mut s, &said("shark", 3.2, 0.3), &anchors, &hold);
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (3.08, false));
        assert_eq!(starts(&s, "b1.hero"), vec![3.08, 3.18, 5.1, 3.78]);
        assert_eq!(durations(&s, "b1.hero"), vec![0.5, 0.6, 0.3, 1.22]);
        // Named at 4.6 (past the floor): clamped at the shortest arrival;
        // the push still follows it (4.4 .. 5.0). Before 0.22 this group
        // stayed at 0.5 and nothing was recorded.
        let mut s = pushed_hero_scene();
        let cues = apply_word_cues(&mut s, &said("shark", 4.6, 0.3), &anchors, &hold);
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (4.05, true));
        assert_eq!(starts(&s, "b1.hero"), vec![4.05, 4.15, 5.1, 4.4]);
        assert_eq!(durations(&s, "b1.hero"), vec![0.35, 0.25, 0.3, 0.6]);
        // A title moves its arrival the same way but never shortens it.
        let title = [anchor("hero", &["shark"], RevealRole::Title)];
        let mut s = pushed_hero_scene();
        let cues = apply_word_cues(&mut s, &said("shark", 3.95, 0.3), &title, &hold);
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (3.7, true));
        assert_eq!(durations(&s, "b1.hero")[..2], [0.5, 0.6]);
    }

    /// The Wallet Atlas beat 1 figure card as the documentary look builds it:
    /// the card slides in at 0.687 (fade 0.25, sprung shift and turn 0.9),
    /// its underline draws at EVOLVE + 0.15 and it fades with the depth exit
    /// after ANTICIPATE.
    fn figure_scene() -> Scene {
        let mut s = scene(
            vec![group(
                "b1.figure",
                vec![
                    text("b1.figure.label", "OF WORLD TRADE"),
                    text("b1.figure.value", "80%"),
                    text("b1.figure.underline", "____"),
                ],
            )],
            vec![
                motion(
                    "b1.figure.underline",
                    3.698,
                    0.55,
                    MotionOp::Trim { from: 0.0, to: 1.0 },
                ),
                fade("b1.figure", 0.687, 0.25),
                motion(
                    "b1.figure",
                    0.687,
                    0.9,
                    MotionOp::Move {
                        from: [0.0, 170.0],
                        to: [0.0, 0.0],
                    },
                ),
                motion(
                    "b1.figure",
                    0.687,
                    0.9,
                    MotionOp::Rotate { from: 3.0, to: 0.0 },
                ),
                motion(
                    "b1.figure.depth",
                    5.216,
                    0.22,
                    MotionOp::Fade { from: 1.0, to: 0.0 },
                ),
            ],
        );
        s.lifecycle = Some(Lifecycle {
            enter: 0.507,
            settle: 1.427,
            read: 3.056,
            evolve: 3.548,
            anticipate: 4.286,
            bridge: 4.886,
        });
        s
    }

    /// The beat's words as measured on the Wallet Atlas voice-over.
    fn world_trade_words() -> Vec<(String, f64)> {
        [
            ("Every", 0.3675),
            ("single", 0.6275),
            ("day,", 0.9675),
            ("about", 1.5475),
            ("eighty", 1.8475),
            ("percent", 2.1075),
            ("of", 2.5475),
            ("everything", 2.7475),
            ("the", 3.1075),
            ("world", 3.2275),
            ("trades", 3.5475),
            ("travels", 3.9475),
            ("by", 4.3475),
            ("sea.", 4.5875),
        ]
        .iter()
        .map(|(w, t)| (w.to_string(), *t))
        .collect()
    }

    #[test]
    fn the_figure_card_waits_for_its_number_past_its_evolve_underline() {
        // Before 0.22 the underline (EVOLVE) made the rigid shift overrun the
        // read floor (4.286 - 0.6), so the card stayed at 0.687, readable
        // a second before "eighty", and nothing was recorded.
        let mut s = figure_scene();
        let anchors = [anchor("figure", &["80%"], RevealRole::Value)];
        let cues = apply_word_cues(
            &mut s,
            &world_trade_words(),
            &anchors,
            &policy(TitleReveal::Hold),
        );
        assert!(cues.dropped.is_empty(), "{:?}", cues.dropped);
        assert_eq!(
            cues.moves,
            vec![WordCueMove {
                group: "figure".into(),
                word: "eighty percent".into(),
                from: 0.687,
                to: round3(1.8475 - WORD_CUE_LEAD),
                role: Some(RevealRole::Value),
                clamped: false,
            }]
        );
        // The card enters on its word, whole; the underline stays on EVOLVE
        // and the exit after ANTICIPATE.
        assert_eq!(starts(&s, "b1.figure"), vec![1.728; 3]);
        assert_eq!(durations(&s, "b1.figure"), vec![0.25, 0.9, 0.9]);
        assert_eq!(starts(&s, "b1.figure.underline"), vec![3.698]);
        assert_eq!(starts(&s, "b1.figure.depth"), vec![5.216]);
    }

    #[test]
    fn a_value_too_late_for_its_whole_entrance_lands_on_its_word_shorter() {
        // "eighty" at 3.0: the whole card (0.9 s) would land at 3.78, after
        // the read floor 3.686; shortened it lands right on the floor.
        let mut s = figure_scene();
        let words = said("roughly eighty percent", 2.7, 0.3);
        let anchors = [anchor("figure", &["80%"], RevealRole::Value)];
        let cues = apply_word_cues(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        assert_eq!(
            (cues.moves[0].to, cues.moves[0].clamped),
            (round3(3.0 - WORD_CUE_LEAD), false)
        );
        assert_eq!(starts(&s, "b1.figure"), vec![2.88; 3]);
        assert_eq!(durations(&s, "b1.figure"), vec![0.25, 0.806, 0.806]);
        // The underline drew at 3.698, after the card now lands (3.686):
        // it keeps its time.
        assert_eq!(starts(&s, "b1.figure.underline"), vec![3.698]);
        for m in s.motions.iter().filter(|m| m.start < 4.286) {
            assert!(m.start + m.duration <= 4.286 + 1e-9, "{m:?}");
        }
        // "eighty" at 3.5: the shortest card (0.25 s) still lands by the
        // floor.
        let mut s = figure_scene();
        let words = said("roughly eighty percent", 3.2, 0.3);
        let cues = apply_word_cues(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (3.38, false));
        assert_eq!(durations(&s, "b1.figure"), vec![0.25, 0.306, 0.306]);
        // Never below CUE_MIN_ENTRANCE: "eighty" at 3.6 is too late even
        // for the shortest card, which is clamped to land on the floor.
        let mut s = figure_scene();
        let words = said("roughly eighty percent", 3.3, 0.3);
        let cues = apply_word_cues(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (3.436, true));
        assert_eq!(durations(&s, "b1.figure"), vec![0.25; 3]);
    }

    #[test]
    fn a_group_that_cannot_move_is_recorded_as_dropped() {
        // A stamp planned at 4.2 whose word comes at 4.7 (after the floor):
        // even its shortest landing (4.15) is not later than where it is.
        let make = || {
            scene(
                vec![text("b1.stamp", "SEA")],
                vec![
                    fade("b1.stamp", 4.2, 0.06),
                    motion(
                        "b1.stamp",
                        4.2,
                        0.18,
                        MotionOp::Scale {
                            from: 2.0,
                            to: 1.0,
                            axis: Default::default(),
                        },
                    ),
                ],
            )
        };
        let words = said("by the sea", 4.1, 0.3);
        let hold = policy(TitleReveal::Hold);
        let stamp = [anchor("stamp", &["sea"], RevealRole::Stamp)];
        let mut s = make();
        let before = s.motions.clone();
        let cues = apply_word_cues(&mut s, &words, &stamp, &hold);
        assert!(cues.moves.is_empty());
        assert_eq!(
            cues.dropped,
            vec![WordCueDrop {
                group: "stamp".into(),
                word: "sea".into(),
                at: 4.7,
                from: 4.2,
                role: RevealRole::Stamp,
            }]
        );
        assert!(close(cues.dropped[0].early(), 0.5));
        assert_eq!(s.motions, before);
        // Planned at 4.5 instead, 0.2 s before its word: it cannot wait
        // either, but within REVEAL_LEAD_MAX it still reads on its word, so
        // nothing is recorded.
        let mut s = make();
        for m in &mut s.motions {
            m.start = 4.5;
        }
        let cues = apply_word_cues(&mut s, &words, &stamp, &hold);
        assert_eq!(cues, WordCues::default());
        // The same group found by name matching is a guess: not recorded.
        let mut s = make();
        let cues = apply_word_cues(&mut s, &words, &[], &hold);
        assert!(cues.moves.is_empty() && cues.dropped.is_empty());
        // Unspoken, or delays not allowed: not a drop.
        let mut s = make();
        let cues = apply_word_cues(&mut s, &said("by the shore", 4.1, 0.3), &stamp, &hold);
        assert_eq!(cues, WordCues::default());
        let mut s = hero_scene(0.5);
        let early_only = CuePolicy {
            allow_delay: false,
            ..hold
        };
        let cues = apply_word_cues(
            &mut s,
            &said("a reef shark", 2.4, 0.3),
            &[anchor("hero", &["shark"], RevealRole::Content)],
            &early_only,
        );
        assert_eq!(cues, WordCues::default());
    }

    #[test]
    fn the_focal_group_has_arrived_by_read() {
        // Named at 3.0, after READ (1.6). Any other group waits for it; the
        // beat's focal layer is read from READ on (layout QA judges it
        // there), so its arrival lands by READ instead: shortest entrance
        // (0.1 + 0.25) ending on READ, clamped.
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        let words = said("a reef shark", 2.4, 0.3);
        let mut s = hero_scene(0.5);
        let cues = apply_word_cues(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (2.88, false));
        let focal = CuePolicy {
            focal: Some("b1.hero".into()),
            ..policy(TitleReveal::Hold)
        };
        let mut s = hero_scene(0.5);
        let cues = apply_word_cues(&mut s, &words, &anchors, &focal);
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (1.25, true));
        assert_eq!(starts(&s, "b1.hero"), vec![1.25, 1.35, 5.1]);
        assert!(close(arrival_end(&s, "b1.hero"), LIFE.read));
        // A word before READ is waited for as usual.
        let mut s = hero_scene(0.3);
        let cues = apply_word_cues(&mut s, &said("shark", 0.9, 0.3), &anchors, &focal);
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (0.78, false));
        // Another group of the same beat is not held by the focal floor.
        let mut s = scene(
            vec![
                image("b1.hero", "asset.reef_shark"),
                text("b1.label", "REEF"),
            ],
            [enters("b1.hero", 0.5), enters("b1.label", 0.5)].concat(),
        );
        let label = [anchor("label", &["reef"], RevealRole::Label)];
        let cues = apply_word_cues(&mut s, &words, &label, &focal);
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (2.58, false));
    }

    #[test]
    fn a_held_value_in_the_focal_card_has_arrived_by_read_too() {
        // (0.23 W4) The card is the focal layer and lands at ENTER; its value
        // is a descendant that waits for its word, and meets READ like the card.
        let anchors = [anchor("card.value", &["shark"], RevealRole::Value)];
        let words = said("a reef shark", 2.4, 0.3);
        let layers = || vec![text("b1.card", "CARD"), text("b1.card.value", "60%")];
        let mut s = scene(layers(), enters("b1.card.value", 0.5));
        let cues = apply_word_cues(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (2.88, false));
        let focal = CuePolicy {
            focal: Some("b1.card".into()),
            ..policy(TitleReveal::Hold)
        };
        let mut s = scene(layers(), enters("b1.card.value", 0.5));
        let cues = apply_word_cues(&mut s, &words, &anchors, &focal);
        assert!(cues.moves[0].clamped);
        assert!(close(arrival_end(&s, "b1.card.value"), LIFE.read));
        // A sibling whose id merely starts with the focal id is not inside it.
        let mut s = scene(
            vec![text("b1.cardboard", "BOX"), text("b1.cardboard.value", "1")],
            enters("b1.cardboard.value", 0.5),
        );
        let box_value = [anchor("cardboard.value", &["shark"], RevealRole::Value)];
        let cues = apply_word_cues(&mut s, &words, &box_value, &focal);
        assert_eq!((cues.moves[0].to, cues.moves[0].clamped), (2.88, false));
    }

    #[test]
    fn an_earlier_move_leaves_a_motion_running_past_anticipate_behind() {
        // The hero enters at 1.5 and drifts from 2.8 to 6.0, past
        // ANTICIPATE (5.0) even 0.42 s earlier. Named at 1.2, the arrival
        // comes forward alone (before 0.22 the drift made the move refuse
        // silently).
        let mut motions = enters("b1.hero", 1.5);
        motions.push(motion(
            "b1.hero",
            2.8,
            3.2,
            MotionOp::Scale {
                from: 1.0,
                to: 1.04,
                axis: Default::default(),
            },
        ));
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        let words = said("sharks circle", 1.2, 0.3);
        let hold = policy(TitleReveal::Hold);
        let mut s = scene(vec![image("b1.hero", "asset.reef_shark")], motions.clone());
        let cues = apply_word_cues(&mut s, &words, &anchors, &hold);
        assert_eq!((cues.moves[0].from, cues.moves[0].to), (1.5, 1.08));
        assert_eq!(starts(&s, "b1.hero"), vec![1.08, 1.18, 5.1, 2.8]);
        // The name-matching fallback keeps the old rule: nothing moves.
        let mut s = scene(vec![image("b1.hero", "asset.reef_shark")], motions);
        assert!(moves_of(&mut s, &words, &[], &hold).is_empty());
        assert_eq!(starts(&s, "b1.hero"), vec![1.5, 1.6, 5.1, 2.8]);
    }

    fn title_scene() -> Scene {
        let mut motions = vec![fade("b1.head", 0.3, 0.8)];
        motions.extend(enters("b1.hero", 0.5));
        scene(
            vec![
                text("b1.head", "THE DEBT"),
                image("b1.hero", "asset.debt_pile"),
            ],
            motions,
        )
    }

    #[test]
    fn a_title_waits_for_its_word_only_under_hold() {
        let anchors = [anchor("head", &["debt"], RevealRole::Title)];
        let words = said("then the debt arrived", 1.5, 0.25);
        // Free: the title enters with the beat and no move is recorded.
        let mut s = title_scene();
        let moves = moves_of(&mut s, &words, &anchors, &policy(TitleReveal::Free));
        assert!(moves.iter().all(|m| m.group != "head"), "{moves:?}");
        assert_eq!(starts(&s, "b1.head"), vec![0.3]);
        // Hold: the cascade starts on the word itself (no lead).
        let mut s = title_scene();
        let moves = moves_of(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        let head = moves
            .iter()
            .find(|m| m.group == "head")
            .expect("title cued");
        assert_eq!(
            (head.word.as_str(), head.from, head.to, head.role),
            ("debt", 0.3, 2.0, Some(RevealRole::Title))
        );
        assert_eq!(starts(&s, "b1.head"), vec![2.0]);
        // A title group outside FIXED_GROUPS still stays under Free: the
        // anchor covers it, so the name-matching fallback leaves it alone.
        let mut s = scene(
            vec![text("b1.title", "THE DEBT")],
            vec![fade("b1.title", 0.3, 0.8)],
        );
        let free = policy(TitleReveal::Free);
        let anchors = [anchor("title", &["debt"], RevealRole::Title)];
        assert!(moves_of(&mut s, &words, &anchors, &free).is_empty());
        assert_eq!(starts(&s, "b1.title"), vec![0.3]);
    }

    #[test]
    fn a_stamp_slams_on_its_word() {
        let make = || {
            scene(
                vec![text("b1.stamp", "WRONG")],
                vec![motion(
                    "b1.stamp",
                    1.0,
                    0.3,
                    MotionOp::Scale {
                        from: 2.0,
                        to: 1.0,
                        axis: Default::default(),
                    },
                )],
            )
        };
        let words = said("that guess was wrong", 2.25, 0.25);
        let hold = policy(TitleReveal::Hold);
        let mut s = make();
        let moves = moves_of(
            &mut s,
            &words,
            &[anchor("stamp", &["wrong"], RevealRole::Stamp)],
            &hold,
        );
        assert_eq!((moves[0].to, moves[0].role), (3.0, Some(RevealRole::Stamp)));
        // The same group as content would start WORD_CUE_LEAD earlier.
        let mut s = make();
        let moves = moves_of(
            &mut s,
            &words,
            &[anchor("stamp", &["wrong"], RevealRole::Content)],
            &hold,
        );
        assert_eq!(moves[0].to, round3(3.0 - WORD_CUE_LEAD));
    }

    #[test]
    fn a_label_moves_through_its_anchor_while_the_kicker_stays_fixed() {
        let mut motions = vec![fade("b1.kicker", 0.3, 0.5), fade("b1.data", 0.4, 0.5)];
        motions.extend(enters("b1.hero", 0.5));
        let mut s = scene(
            vec![
                text("b1.kicker", "SHARKS"),
                text("b1.data", "FINS"),
                image("b1.hero", "asset.reef_shark"),
            ],
            motions,
        );
        // "sharks" is spoken late (it would delay a fallback group), "fins"
        // earlier. `data` is a fixed group: only its explicit Label anchor
        // moves it; the kicker has none and never moves.
        let words = said("look at the fins on these sharks", 1.5, 0.3);
        let anchors = [anchor("data", &["fin"], RevealRole::Label)];
        let moves = moves_of(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        let label = moves
            .iter()
            .find(|m| m.group == "data")
            .expect("label cued");
        assert_eq!(
            (label.word.as_str(), label.to, label.role),
            ("fins", round3(2.4 - WORD_CUE_LEAD), Some(RevealRole::Label))
        );
        assert!(moves.iter().all(|m| m.group != "kicker"), "{moves:?}");
        assert_eq!(starts(&s, "b1.kicker"), vec![0.3]);
    }

    #[test]
    fn groups_on_a_revolving_sphere_never_move() {
        let revolve = |target: &str| {
            motion(
                target,
                0.3,
                4.0,
                MotionOp::Revolve {
                    radius: 300.0,
                    at: [0.0, 0.0],
                    from: [0.0, 0.0],
                    to: [180.0, 0.0],
                },
            )
        };
        let mut motions = vec![revolve("b1.item.0"), fade("b1.item.0", 2.0, 0.4)];
        motions.extend([revolve("b1.orb"), fade("b1.orb", 2.0, 0.4)]);
        let mut s = scene(
            vec![
                group("b1.item", vec![image("b1.item.0", "asset.reef_shark")]),
                image("b1.orb", "asset.reef_shark"),
            ],
            motions,
        );
        let before = s.motions.clone();
        let anchors = [anchor("item.0", &["shark"], RevealRole::Content)];
        let words = said("a shark", 0.6, 0.3);
        assert!(moves_of(&mut s, &words, &anchors, &policy(TitleReveal::Hold)).is_empty());
        assert_eq!(s.motions, before);
    }

    #[test]
    fn value_anchor_on_a_written_amount_matches_the_spoken_number() {
        let mut motions = enters("b1.card.value", 0.5);
        motions.push(fade("b1.card.label", 0.5, 0.4));
        let mut s = scene(
            vec![group(
                "b1.card",
                vec![
                    text("b1.card.value", "$381 BILLION"),
                    text("b1.card.label", "owed"),
                ],
            )],
            motions,
        );
        let words = said(
            "the country owed three hundred and eighty one billion dollars",
            0.4,
            0.2,
        );
        let anchors = [anchor("card.value", &["$381 billion"], RevealRole::Value)];
        let moves = moves_of(&mut s, &words, &anchors, &policy(TitleReveal::Free));
        // "three" is the 4th word: 0.4 + 3 * 0.2 = 1.0.
        assert_eq!(
            moves,
            vec![WordCueMove {
                group: "card.value".into(),
                word: "three hundred and eighty one billion".into(),
                from: 0.5,
                to: 0.88,
                role: Some(RevealRole::Value),
                clamped: false,
            }]
        );
        // The label shares the anchored `card` group, so the fallback leaves
        // it where its builder put it although "owed" is spoken.
        assert_eq!(starts(&s, "b1.card.label"), vec![0.5]);
    }

    #[test]
    fn unanchored_groups_fall_back_to_name_matching_and_may_delay() {
        let mut motions = enters("b1.hero", 2.0);
        motions.extend(enters("b1.support", 0.6));
        motions.extend(enters("b1.amount", 2.5));
        let mut s = scene(
            vec![
                image("b1.hero", "asset.reef_shark"),
                text("b1.support", "LEAKS FOUND"),
                text("b1.amount", "₹50,000"),
            ],
            motions,
        );
        // hero is anchored (named at 1.0); support (planned 0.6) is named
        // later at 2.8 by its singular; amount (planned 2.5) is said as words
        // at 1.6.
        let words = said("sharks find fifty thousand in one leak", 1.0, 0.3);
        let anchors = [anchor("hero", &["shark"], RevealRole::Content)];
        let moves = moves_of(&mut s, &words, &anchors, &policy(TitleReveal::Free));
        let summary: Vec<(&str, &str, f64, f64, Option<RevealRole>)> = moves
            .iter()
            .map(|m| (m.group.as_str(), m.word.as_str(), m.from, m.to, m.role))
            .collect();
        // Anchors first, then fallback groups by name.
        assert_eq!(
            summary,
            vec![
                ("hero", "sharks", 2.0, 0.88, Some(RevealRole::Content)),
                ("amount", "fifty thousand", 2.5, 1.48, None),
                ("support", "leak", 0.6, 2.68, None),
            ]
        );
        assert!(moves.iter().all(|m| !m.clamped));
        assert_eq!(starts(&s, "b1.support"), vec![2.68, 2.78, 5.1]);
    }

    // --- (0.23 W4) counts -------------------------------------------------

    /// A `Count` of `to` percent on `target`, out-quint (a landing curve).
    fn count(target: &str, start: f64, duration: f64, from: f64, to: f64) -> Motion {
        let mut m = motion(
            target,
            start,
            duration,
            MotionOp::Count {
                from,
                to,
                decimals: 0,
                grouping: false,
                prefix: String::new(),
                suffix: "%".into(),
            },
        );
        m.easing = Easing::OutQuint;
        m
    }

    fn count_scene(motions: Vec<Motion>) -> Scene {
        scene(vec![text("b1.hero.60", "60%")], motions)
    }

    fn duration_of(scene: &Scene, target: &str) -> f64 {
        scene
            .motions
            .iter()
            .find(|m| m.target == target && matches!(m.op, MotionOp::Count { .. }))
            .map(|m| m.duration)
            .unwrap_or(f64::NAN)
    }

    #[test]
    fn a_count_shows_its_final_text_before_its_motion_ends() {
        let m = count("b1.hero.60", 2.0, 1.6, 0.0, 60.0);
        let at = count_shown_at(&m, 0.0).expect("lands");
        // Out-quint rounds to 60 at about 62% of the motion.
        assert!(at > 2.0 + 0.55 * 1.6 && at < 2.0 + 0.7 * 1.6, "{at}");
        // It is the frame it first shows on, on the project clock.
        let on_clock = (count_shown_at(&m, 0.01).expect("lands") + 0.01) * COUNT_FPS;
        assert!((on_clock - on_clock.round()).abs() < 1e-6, "{on_clock}");
        // A spring that is not a landing curve is not judged.
        let mut spring = m.clone();
        spring.easing = Easing::ImpactSpring;
        assert_eq!(count_shown_at(&spring, 0.0), None);
    }

    #[test]
    fn a_count_that_lands_late_is_shortened_to_end_by_its_word() {
        // "sixty" at 2.3: a 2.4 s count started with the layer at 2.0 shows
        // 60% at about 3.5 s, 1.2 s after the word.
        let words = said("about sixty percent", 1.9, 0.4);
        let mut s = count_scene(vec![count("b1.hero.60", 2.0, 2.4, 0.0, 60.0)]);
        settle_counts(&mut s, &words, &[]);
        // Ends 0.75 s after the word, start untouched.
        assert!(close(duration_of(&s, "b1.hero.60"), 1.05), "{s:?}");
        assert!(close(s.motions[0].start, 2.0));
        let at = count_shown_at(&s.motions[0], 0.0).expect("lands");
        assert!(at <= 2.3 + 0.8, "{at}");
    }

    #[test]
    fn a_count_that_already_lands_on_its_word_is_left_as_built() {
        let words = said("about sixty percent", 1.9, 0.4);
        let mut s = count_scene(vec![count("b1.hero.60", 2.0, 1.0, 0.0, 60.0)]);
        let before = s.clone();
        settle_counts(&mut s, &words, &[]);
        assert_eq!(s, before);
        // Without a voice-over the anchor is READ (1.6): a count that lands
        // within 0.8 s of it and holds before ANTICIPATE stays too.
        let mut quiet = count_scene(vec![count("b1.hero.60", 1.3, 1.0, 0.0, 60.0)]);
        let before = quiet.clone();
        settle_counts(&mut quiet, &[], &[]);
        assert_eq!(quiet, before);
    }

    #[test]
    fn the_anchor_is_the_groups_word_when_the_number_is_not_said() {
        // Nothing says "60%": the group's own word cues it. "reef" at 2.3.
        let words = said("about reef health", 1.9, 0.4);
        let anchors = [anchor("hero", &["reef"], RevealRole::Content)];
        let mut s = count_scene(vec![count("b1.hero.60", 2.0, 2.4, 0.0, 60.0)]);
        settle_counts(&mut s, &words, &anchors);
        assert!(close(duration_of(&s, "b1.hero.60"), 1.05), "{s:?}");
        // No word at all: READ (1.6), never earlier than the count's start.
        let mut s = count_scene(vec![count("b1.hero.60", 2.0, 2.4, 0.0, 60.0)]);
        settle_counts(&mut s, &[], &[]);
        assert!(close(duration_of(&s, "b1.hero.60"), 0.75), "{s:?}");
    }

    #[test]
    fn a_count_is_shortened_to_hold_before_anticipate() {
        // Lands within 0.8 s of its start (3.5), but shows 60% only 0.76 s
        // before ANTICIPATE (5.0): it ends 1 s before it instead.
        let words = said("sixty percent", 3.5, 0.4);
        let mut s = count_scene(vec![count("b1.hero.60", 3.5, 1.2, 0.0, 60.0)]);
        settle_counts(&mut s, &words, &[]);
        assert!(close(duration_of(&s, "b1.hero.60"), 0.5), "{s:?}");
        let at = count_shown_at(&s.motions[0], 0.0).expect("lands");
        assert!(5.0 - at >= 1.0, "{at}");
    }

    #[test]
    fn when_the_beat_cannot_hold_landing_on_the_word_wins() {
        // A layer that arrives 0.6 s before ANTICIPATE has no room for a real
        // count and a 1 s hold: it keeps a count of at least 0.4 s that is
        // done before the exit, and the hold is what the beat leaves.
        let words = said("sixty percent", 4.4, 0.4);
        let mut s = count_scene(vec![count("b1.hero.60", 4.4, 1.2, 0.0, 60.0)]);
        settle_counts(&mut s, &words, &[]);
        let d = duration_of(&s, "b1.hero.60");
        assert!(close(d, 0.55), "{d}");
        assert!(d >= COUNT_MIN_S);
        // Settles before the exit but cannot gain the hold: left as built.
        let mut s = count_scene(vec![count("b1.hero.60", 3.9, 0.8, 0.0, 60.0)]);
        let before = s.clone();
        settle_counts(&mut s, &said("sixty percent", 3.9, 0.4), &[]);
        assert_eq!(s, before);
    }

    #[test]
    fn a_shorter_count_keeps_its_own_length_and_a_fade_out_is_the_exit() {
        // 0.2 s counts (a running total's step) are never lengthened.
        let mut s = count_scene(vec![count("b1.hero.60", 4.5, 0.2, 59.0, 60.0)]);
        let before = s.clone();
        settle_counts(&mut s, &said("sixty", 4.5, 0.4), &[]);
        assert_eq!(s, before);
        // The layer fades out at 3.0, ahead of ANTICIPATE: the hold is
        // measured to it, so a count that settles at 2.4 is too late.
        let mut s = count_scene(vec![
            count("b1.hero.60", 1.5, 1.4, 0.0, 60.0),
            motion(
                "b1.hero.60",
                3.0,
                0.3,
                MotionOp::Fade { from: 1.0, to: 0.0 },
            ),
        ]);
        settle_counts(&mut s, &said("sixty", 1.5, 0.4), &[]);
        let at = count_shown_at(&s.motions[0], 0.0).expect("lands");
        assert!(3.0 - at >= 1.0 - 1e-9, "{at}");
    }

    /// An accumulate beat as the builder lays it out, after a cue moved the
    /// items: two cards, a total that steps once per item, the bar beside it.
    fn accumulate_scene(items: [f64; 2], steps: [f64; 2]) -> Scene {
        let mut motions = vec![
            fade("b1.items.0.card", items[0], 0.5),
            fade("b1.items.1.card", items[1], 0.5),
            fade("b1.bar.track", items[0], 0.4),
            fade("b1.total", items[0] + 0.12, 0.4),
            fade("b1.total.label", items[0] + 0.12, 0.4),
        ];
        for (i, at) in steps.iter().enumerate() {
            motions.push(count("b1.total", *at, 0.42, i as f64, i as f64 + 1.0));
            motions.push(motion(
                "b1.bar.fill",
                *at,
                0.42,
                MotionOp::AccentExpand {
                    to: crate::scene::BoxRect {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0 * (i as f32 + 1.0),
                        height: 10.0,
                    },
                },
            ));
        }
        scene(
            vec![
                text("b1.items.0.card", "a"),
                text("b1.items.1.card", "b"),
                text("b1.total", "0"),
                text("b1.total.label", "items"),
                text("b1.bar.track", ""),
                text("b1.bar.fill", ""),
            ],
            motions,
        )
    }

    fn moved(group: &str, from: f64, to: f64) -> WordCueMove {
        WordCueMove {
            group: group.to_string(),
            word: "x".into(),
            from,
            to,
            role: None,
            clamped: false,
        }
    }

    #[test]
    fn a_total_that_lags_the_items_a_cue_moved_follows_them() {
        // The cue moved both cards 1 s earlier (3.0 -> 2.0, 4.0 -> 3.0); the
        // total's steps (planned 3.12 / 4.18) were left behind: the last one
        // shows more than 1 s after the last card.
        let mut s = accumulate_scene([2.0, 3.0], [3.12, 4.18]);
        follow_items(&mut s, &[moved("items", 3.0, 2.0)], "b1.");
        assert_eq!(starts(&s, "b1.total")[1..], [2.12, 3.18]);
        // The bar fill steps with the total; the first fades with the first item.
        assert_eq!(starts(&s, "b1.bar.fill"), vec![2.12, 3.18]);
        assert!(close(starts(&s, "b1.bar.track")[0], 1.0));
        assert!(close(starts(&s, "b1.total.label")[0], 1.12));
        assert!(close(starts(&s, "b1.total")[0], 1.12));
    }

    #[test]
    fn a_total_a_little_off_its_items_stays_as_built() {
        // Items moved 0.2 s later: the total is 0.2 s early, not a defect.
        let mut s = accumulate_scene([3.2, 4.2], [3.12, 4.18]);
        let before = s.clone();
        follow_items(&mut s, &[moved("items", 3.0, 3.2)], "b1.");
        assert_eq!(s, before);
        // No move, no change.
        follow_items(&mut s, &[], "b1.");
        assert_eq!(s, before);
        // A lag the cue did not cause (the move would not bring it back).
        let mut s = accumulate_scene([2.0, 3.0], [3.12, 4.18]);
        let before = s.clone();
        follow_items(&mut s, &[moved("items", 2.9, 3.0)], "b1.");
        assert_eq!(s, before);
    }

    #[test]
    fn each_step_follows_its_own_item_when_the_cues_are_per_item() {
        // Only the second card moved (4.0 -> 3.0): only the second step goes.
        let mut s = accumulate_scene([2.0, 3.0], [2.12, 4.18]);
        follow_items(&mut s, &[moved("items.1", 4.0, 3.0)], "b1.");
        let steps: Vec<f64> = s
            .motions
            .iter()
            .filter(|m| m.target == "b1.total" && matches!(m.op, MotionOp::Count { .. }))
            .map(|m| m.start)
            .collect();
        assert_eq!(steps, vec![2.12, 3.18]);
    }

    #[test]
    fn a_group_declared_twice_is_cued_by_its_earliest_entry() {
        let mut s = hero_scene(2.0);
        let anchors = [
            anchor("hero", &["fin"], RevealRole::Content),
            anchor("hero", &["shark"], RevealRole::Value),
        ];
        let words = said("a shark fin", 1.0, 0.3);
        let moves = moves_of(&mut s, &words, &anchors, &policy(TitleReveal::Hold));
        assert_eq!(moves.len(), 1);
        assert_eq!(
            (moves[0].word.as_str(), moves[0].role),
            ("shark", Some(RevealRole::Value))
        );
    }
}
