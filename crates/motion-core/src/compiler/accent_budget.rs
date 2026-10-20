//! (0.24 M9) The accent budget: how many loud colour moves a story may spend.
//!
//! * **Full-frame colour fields** (a slam's inverse cut, a panel wipe, an
//!   accent flood): at most one per video, reserved for the payoff, the last
//!   reveal beat or else the last impact beat.
//! * **Accent label blocks** (a band or highlight box behind words): at most
//!   one per three beats.
//! * **Stamps** (a keyword stamped on the frame): at most one per three beats.
//! * **Ghost word** (the giant faint background word): at most one per video,
//!   on the payoff beat only (the last beat when there is none), and never when it repeats text the beat shows
//!   (`ghost_repeats`).
//!
//! Pure and deterministic: the plan is a function of the beats alone. The
//! builders ask `*_ok(beat)`; the `slop_accent_overuse` QA measures the result.

use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::intent::{Beat, Energy, Purpose};

/// Which beats may spend which accent (see the module docs). Blocks and
/// stamps are claimed in build order: the first beat of each window of three
/// that actually draws one keeps it, so a keyword nobody says never wastes
/// the slot.
#[derive(Debug, Clone, Default)]
pub(crate) struct AccentBudget {
    limited: bool,
    field: Option<usize>,
    /// The beat that may set the ghost word: the payoff, else the last beat.
    ghost: Option<usize>,
    beats: usize,
    windows: usize,
    claims: RefCell<Claims>,
}

/// Window -> the beat that spent its slot.
#[derive(Debug, Clone, Default)]
struct Claims {
    blocks: BTreeMap<usize, usize>,
    stamps: BTreeMap<usize, usize>,
}

impl AccentBudget {
    /// No limits (unit tests and previews build contexts without a story).
    pub fn unlimited() -> AccentBudget {
        AccentBudget::default()
    }

    /// The budget of a story.
    pub fn plan(beats: &[Beat]) -> AccentBudget {
        let field = beats
            .iter()
            .rposition(|b| b.purpose == Purpose::Reveal)
            .or_else(|| beats.iter().rposition(|b| b.energy == Energy::Impact));
        AccentBudget {
            limited: true,
            field,
            ghost: field.or(beats.len().checked_sub(1)),
            beats: beats.len(),
            windows: (beats.len() / 3).max(1),
            claims: RefCell::default(),
        }
    }

    fn window(&self, i: usize) -> usize {
        (i * self.windows / self.beats.max(1)).min(self.windows - 1)
    }

    /// Beat `i` is the last of its window.
    fn closes_window(&self, i: usize) -> bool {
        i + 1 >= self.beats || self.window(i + 1) != self.window(i)
    }

    /// Beat `i` may take a full-frame colour field (cut, wipe or flood).
    pub fn field_ok(&self, i: usize) -> bool {
        !self.limited || self.field == Some(i)
    }

    /// Beat `i` may set the story's one ghost word: the payoff beat.
    pub fn ghost_ok(&self, i: usize) -> bool {
        !self.limited || self.ghost == Some(i)
    }

    /// Beat `i` may put one accent block behind words; claims its window's
    /// slot for `i` (asking again for the same beat stays true).
    pub fn block_ok(&self, i: usize) -> bool {
        if !self.limited {
            return true;
        }
        let w = self.window(i);
        *self.claims.borrow_mut().blocks.entry(w).or_insert(i) == i
    }

    /// Beat `i` may stamp a keyword on the frame; claims its window's slot.
    /// A beat that already holds the window's block hands the stamp to a later
    /// beat of the window, unless it is the window's last.
    pub fn stamp_ok(&self, i: usize) -> bool {
        if !self.limited {
            return true;
        }
        let w = self.window(i);
        let mut claims = self.claims.borrow_mut();
        if claims.blocks.get(&w) == Some(&i) && !self.closes_window(i) {
            return claims.stamps.get(&w) == Some(&i);
        }
        *claims.stamps.entry(w).or_insert(i) == i
    }
}

/// The ghost `word` says what the beat already shows: every word of it is a
/// word of the title or of a subject's phrase / figure.
pub(crate) fn ghost_repeats(beat: &Beat, word: &str) -> bool {
    fn tokens(s: &str) -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_lowercase)
            .collect()
    }
    let wanted = tokens(word);
    if wanted.is_empty() {
        return true;
    }
    let mut shown = tokens(&beat.statement);
    for s in std::iter::once(&beat.primary).chain(beat.secondary.as_ref()) {
        // A picture shows no text of its own: only its figure.
        if s.kind() != crate::intent::SubjectKind::Object {
            shown.extend(s.display_text().map(tokens).unwrap_or_default());
        }
        shown.extend(s.value().map(tokens).unwrap_or_default());
    }
    wanted.iter().all(|w| shown.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::CreativeIntent;

    fn beats(spec: &[(&str, &str)]) -> Vec<Beat> {
        let rows: Vec<String> = spec
            .iter()
            .map(|(p, e)| {
                format!(
                    r#"{{"purpose":"{p}","statement":"S","primary":{{"kind":"phrase","value":"v"}},"energy":"{e}"}}"#
                )
            })
            .collect();
        let json = format!(
            r#"{{"version":"0.2","title":"t","format":"vertical","beats":[{}]}}"#,
            rows.join(",")
        );
        serde_json::from_str::<CreativeIntent>(&json)
            .expect("intent")
            .beats
    }

    #[test]
    fn one_field_for_the_payoff_and_one_slot_per_three_beats() {
        let b = beats(&[
            ("emphasize", "impact"),
            ("compare", "building"),
            ("explain", "calm"),
            ("emphasize", "impact"),
            ("emphasize", "building"),
            ("reveal", "impact"),
            ("emphasize", "calm"),
            ("emphasize", "calm"),
            ("emphasize", "impact"),
        ]);
        let plan = AccentBudget::plan(&b);
        assert_eq!((0..9).filter(|&i| plan.field_ok(i)).count(), 1);
        assert!(plan.field_ok(5), "the last reveal is the payoff");
        // Asked in build order, every beat wants a block and a stamp.
        let blocks: Vec<usize> = (0..9).filter(|&i| plan.block_ok(i)).collect();
        let stamps: Vec<usize> = (0..9).filter(|&i| plan.stamp_ok(i)).collect();
        assert_eq!(blocks, [0, 3, 6]);
        assert_eq!(
            stamps,
            [1, 4, 7],
            "a stamp steps aside for its beat's block"
        );
    }

    #[test]
    fn a_beat_that_never_asks_leaves_the_slot_to_the_next() {
        let b = beats(&[("emphasize", "calm"); 3]);
        let plan = AccentBudget::plan(&b);
        // Beat 0 never asks; beat 1 spends the window's stamp, beat 2 cannot.
        assert!(plan.stamp_ok(1));
        assert!(!plan.stamp_ok(2));
        assert!(plan.stamp_ok(1), "asking again for the same beat holds");
    }

    #[test]
    fn without_a_reveal_the_last_impact_is_the_payoff() {
        let b = beats(&[
            ("emphasize", "impact"),
            ("emphasize", "calm"),
            ("emphasize", "impact"),
            ("emphasize", "calm"),
        ]);
        let plan = AccentBudget::plan(&b);
        assert!(plan.field_ok(2));
        assert!(!plan.field_ok(0) && !plan.field_ok(3));
    }

    #[test]
    fn no_payoff_means_no_field_and_short_stories_keep_one_slot() {
        let b = beats(&[("emphasize", "calm"), ("emphasize", "calm")]);
        let plan = AccentBudget::plan(&b);
        assert!((0..2).all(|i| !plan.field_ok(i)));
        assert_eq!((0..2).filter(|&i| plan.block_ok(i)).count(), 1);
    }

    #[test]
    fn one_ghost_word_on_the_payoff_beat() {
        let b = beats(&[
            ("emphasize", "impact"),
            ("reveal", "impact"),
            ("emphasize", "calm"),
        ]);
        let plan = AccentBudget::plan(&b);
        assert_eq!((0..3).filter(|&i| plan.ghost_ok(i)).count(), 1);
        assert!(plan.ghost_ok(1));
    }

    #[test]
    fn a_ghost_that_repeats_shown_text_is_dropped() {
        let b = beats(&[("emphasize", "impact")]);
        // The statement is "S", the primary phrase "v".
        assert!(ghost_repeats(&b[0], "S"));
        assert!(ghost_repeats(&b[0], "v"));
        assert!(!ghost_repeats(&b[0], "light"));
    }

    #[test]
    fn unlimited_allows_everything() {
        let plan = AccentBudget::unlimited();
        assert!(plan.field_ok(3) && plan.block_ok(3) && plan.stamp_ok(3) && plan.ghost_ok(3));
    }
}
