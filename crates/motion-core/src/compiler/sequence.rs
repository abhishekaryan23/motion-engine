//! (0.21) Story order: does the story count?
//!
//! Beat numbers ("01 — …" kickers, folios, index numerals) used to appear on
//! every beat of every story. A number on screen says "this is item N", so the
//! engine now shows one only when the story actually counts: a countdown
//! ("Number five … Number four …") or things listed in order ("Step 1 …",
//! "First … Second … Third …"). Everything else carries no numbering.
//!
//! A beat is ranked when its `statement`, else its `narration`, opens with a
//! rank marker: `#5`, `5.`, `5)`, `5:`, `5 —`, a counting word plus a number
//! (`Number five`, `No. 3`, `Step 2`, `Fact #4`, `Reason 1`, `Day 3` …), or an
//! ordinal (`First`, `Second`, `3rd` …); a leading "and", "now", "so" or
//! "then" is skipped. `Finally` / `Lastly` continue a run. The story counts
//! only when at least [`MIN_RUN`] beats are ranked and their ranks run in
//! story order by one: up from 1 (a list in order) or down (a countdown), all
//! within 1..=[`MAX_RANK`]. Any rank out of step means no numbering at all —
//! a wrong number is worse than none. Pure and deterministic.

use crate::intent::{Beat, CreativeIntent};

/// Ranked beats a story needs before it counts.
pub const MIN_RUN: usize = 3;
/// Larger "ranks" are years or amounts, not positions in a list.
pub const MAX_RANK: u32 = 20;

/// How a counting story counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// 1, 2, 3 …: things listed in order.
    Ascending,
    /// … 3, 2, 1: a countdown.
    Countdown,
}

/// The rank each beat shows (`None` = no number) and the story's order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sequence {
    pub order: Option<Order>,
    pub ranks: Vec<Option<u32>>,
}

impl Sequence {
    /// The rank of beat `index` (0-based), if the story counts and the beat is an item.
    pub fn rank(&self, index: usize) -> Option<u32> {
        self.ranks.get(index).copied().flatten()
    }
}

/// A rank as shown on screen: two digits, e.g. `05`.
pub fn rank_text(rank: u32) -> String {
    format!("{rank:02}")
}

/// What a beat's opening words say about its place in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Rank(u32),
    /// "Finally", "Lastly": the next step of the run, whatever its number.
    Last,
}

const COUNTING_WORDS: &[&str] = &[
    "number",
    "no",
    "num",
    "step",
    "tip",
    "reason",
    "fact",
    "rule",
    "way",
    "sign",
    "lesson",
    "habit",
    "mistake",
    "secret",
    "day",
    "part",
    "chapter",
    "level",
    "stage",
    "rank",
    "pick",
    "item",
    "myth",
    "trick",
    "law",
    "principle",
    "question",
    "spot",
    "place",
];

const NUMBER_WORDS: &[&str] = &[
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
    "twenty",
];

const ORDINALS: &[&str] = &[
    "zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
    "tenth",
];

const FILLERS: &[&str] = &["and", "now", "so", "then"];

/// Words of the opening, lowercased, with quotes stripped.
fn opening(text: &str) -> Vec<String> {
    text.split_whitespace()
        .take(5)
        .map(|w| {
            w.trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’'))
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn trim_punct(w: &str) -> &str {
    w.trim_end_matches([',', ':', '.', ';', '!', ')', '—', '–'])
}

/// A number written as digits (optionally after `#`) or as a word.
fn number(w: &str) -> Option<u32> {
    let w = trim_punct(w).trim_start_matches('#');
    if !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()) {
        return w.parse().ok();
    }
    NUMBER_WORDS.iter().position(|n| *n == w).map(|i| i as u32)
}

/// `1st`, `2nd`, `first`, `secondly` …
fn ordinal(w: &str) -> Option<u32> {
    let w = trim_punct(w);
    let w = w
        .strip_suffix("ly")
        .filter(|s| ORDINALS.contains(s))
        .unwrap_or(w);
    if let Some(i) = ORDINALS.iter().position(|o| *o == w) {
        return Some(i as u32);
    }
    let digits: String = w.chars().take_while(char::is_ascii_digit).collect();
    let suffix = &w[digits.len()..];
    (!digits.is_empty() && matches!(suffix, "st" | "nd" | "rd" | "th"))
        .then(|| digits.parse().ok())
        .flatten()
}

/// The rank marker a text opens with. A "rank" outside 1..=[`MAX_RANK`] is a
/// year or an amount ("2024: …"), not a marker: the beat simply has none.
fn marker(text: &str) -> Option<Marker> {
    opening_marker(text).filter(|m| match m {
        Marker::Rank(r) => (1..=MAX_RANK).contains(r),
        Marker::Last => true,
    })
}

fn opening_marker(text: &str) -> Option<Marker> {
    let mut words = opening(text);
    if words.len() > 1 && FILLERS.contains(&trim_punct(&words[0])) {
        words.remove(0);
    }
    let first = words.first()?;
    let next = words.get(1).map(String::as_str);
    // "#5", "#5:"
    if first.starts_with('#') {
        return number(first).map(Marker::Rank);
    }
    // "5." "5)" "5:" — but not "5.5" or "5%"; a bare "5" only before a dash.
    let digits: String = first.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        let rest = &first[digits.len()..];
        let dashed = rest.is_empty() && matches!(next, Some("—" | "–" | "-" | "|"));
        if matches!(rest, "." | ")" | ":" | "—" | "–") || dashed {
            return digits.parse().ok().map(Marker::Rank);
        }
        return ordinal(first).map(Marker::Rank);
    }
    // "Number five", "No. 3", "Step 2:", "Fact #4"
    if COUNTING_WORDS.contains(&trim_punct(first)) {
        return next.and_then(number).map(Marker::Rank);
    }
    if matches!(trim_punct(first), "finally" | "lastly") {
        return Some(Marker::Last);
    }
    ordinal(first).map(Marker::Rank)
}

fn beat_marker(beat: &Beat) -> Option<Marker> {
    marker(&beat.statement).or_else(|| beat.narration.as_deref().and_then(marker))
}

/// The story's order and each beat's rank.
pub fn detect(intent: &CreativeIntent) -> Sequence {
    let none = Sequence {
        order: None,
        ranks: vec![None; intent.beats.len()],
    };
    let marked: Vec<(usize, Marker)> = intent
        .beats
        .iter()
        .enumerate()
        .filter_map(|(i, b)| beat_marker(b).map(|m| (i, m)))
        .collect();
    if marked.len() < MIN_RUN {
        return none;
    }
    let mut ranks = vec![None; intent.beats.len()];
    let mut order: Option<Order> = None;
    let mut prev: Option<u32> = None;
    for (i, m) in marked {
        let rank = match (m, prev, order) {
            (Marker::Rank(r), None, _) => r,
            (Marker::Rank(r), Some(p), None) if r == p + 1 => {
                order = Some(Order::Ascending);
                r
            }
            (Marker::Rank(r), Some(p), None) if r + 1 == p => {
                order = Some(Order::Countdown);
                r
            }
            (Marker::Rank(r), Some(p), Some(Order::Ascending)) if r == p + 1 => r,
            (Marker::Rank(r), Some(p), Some(Order::Countdown)) if r + 1 == p => r,
            (Marker::Last, Some(p), Some(Order::Ascending)) => p + 1,
            (Marker::Last, Some(p), Some(Order::Countdown)) if p > 1 => p - 1,
            _ => return none,
        };
        if !(1..=MAX_RANK).contains(&rank) {
            return none;
        }
        ranks[i] = Some(rank);
        prev = Some(rank);
    }
    // A list in order starts at one.
    let first = ranks.iter().flatten().next().copied();
    if order == Some(Order::Ascending) && first != Some(1) {
        return none;
    }
    Sequence { order, ranks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{Atom, Continuity, Energy, Format, Purpose, Subject, INTENT_VERSION};

    fn story(lines: &[(&str, Option<&str>)]) -> CreativeIntent {
        CreativeIntent {
            version: INTENT_VERSION.into(),
            title: "t".into(),
            format: Format::Vertical,
            beats: lines
                .iter()
                .map(|(statement, narration)| Beat {
                    purpose: Purpose::Emphasize,
                    statement: statement.to_string(),
                    primary: Subject::Phrase(Atom::default()),
                    secondary: None,
                    relationship: None,
                    energy: Energy::Building,
                    continuity: Continuity::None,
                    keyword: None,
                    narration: narration.map(str::to_string),
                })
                .collect(),
        }
    }

    fn ranks(lines: &[(&str, Option<&str>)]) -> Vec<Option<u32>> {
        detect(&story(lines)).ranks
    }

    #[test]
    fn plain_stories_have_no_numbers() {
        let s = story(&[
            (
                "Leave it alone",
                Some("Put one thousand dollars away today."),
            ),
            (
                "Interest on interest",
                Some("At seven percent a year, it grows."),
            ),
            (
                "Patience pays",
                Some("So that 1,000 dollars becomes 7,600."),
            ),
        ]);
        assert_eq!(
            detect(&s),
            Sequence {
                order: None,
                ranks: vec![None; 3]
            }
        );
    }

    #[test]
    fn a_countdown_counts_down() {
        let s = story(&[
            (
                "Five animals older than trees",
                Some("Here are five animals older than trees."),
            ),
            (
                "Sharks",
                Some("Number five: sharks, swimming for 450 million years."),
            ),
            ("Horseshoe crabs", Some("Number four, the horseshoe crab.")),
            ("Jellyfish", Some("Number three is the jellyfish.")),
            ("Sponges", Some("And number two: sponges.")),
            ("#1 Comb jellies", None),
            (
                "That's older than trees",
                Some("All of them were here first."),
            ),
        ]);
        let seq = detect(&s);
        assert_eq!(seq.order, Some(Order::Countdown));
        assert_eq!(
            seq.ranks,
            vec![None, Some(5), Some(4), Some(3), Some(2), Some(1), None]
        );
        assert_eq!(rank_text(5), "05");
    }

    #[test]
    fn steps_and_ordinals_list_in_order() {
        assert_eq!(
            ranks(&[
                ("Step 1: gather", None),
                ("Step 2: mix", None),
                ("Step 3: bake", None),
            ]),
            vec![Some(1), Some(2), Some(3)]
        );
        assert_eq!(
            ranks(&[
                (
                    "The habit loop",
                    Some("First, the cue tells your brain to act.")
                ),
                ("Routine", Some("Second, the routine you repeat.")),
                ("Reward", Some("Third, the reward.")),
                ("The loop", Some("Finally, it all repeats.")),
            ]),
            vec![Some(1), Some(2), Some(3), Some(4)]
        );
        assert_eq!(
            ranks(&[("1. Start", None), ("2) Drive", None), ("3: Finish", None)]),
            vec![Some(1), Some(2), Some(3)]
        );
    }

    #[test]
    fn numbers_that_are_not_ranks_never_count() {
        // Amounts, percents, years and two-item runs.
        assert_eq!(
            ranks(&[
                ("381 billion dollars in cash", None),
                ("70% of it in bonds", None),
                ("5 things to know", None),
            ]),
            vec![None; 3]
        );
        assert_eq!(
            ranks(&[
                ("2023: launch", None),
                ("2024: growth", None),
                ("2025: profit", None)
            ]),
            vec![None; 3]
        );
        assert_eq!(
            ranks(&[("Step 1", None), ("Step 2", None), ("Done", None)]),
            vec![None; 3]
        );
        // Out of step: no numbers at all.
        assert_eq!(
            ranks(&[("Step 1", None), ("Step 3", None), ("Step 4", None)]),
            vec![None; 3]
        );
        // A list in order must start at one.
        assert_eq!(
            ranks(&[("Step 2", None), ("Step 3", None), ("Step 4", None)]),
            vec![None; 3]
        );
        // "5.5" and "5%" are values, not markers.
        assert_eq!(marker("5.5 metres deep"), None);
        assert_eq!(marker("5% of it"), None);
        // A year-led beat inside a countdown is just an unranked beat.
        assert_eq!(
            ranks(&[
                ("Number three: sharks", None),
                ("1969: the year it was found", None),
                ("Number two: jellyfish", None),
                ("Number one: sponges", None),
            ]),
            vec![Some(3), None, Some(2), Some(1)]
        );
    }
}
