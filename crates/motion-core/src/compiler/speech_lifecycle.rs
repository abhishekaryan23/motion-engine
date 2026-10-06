//! (0.20) Speech-aware lifecycle: with a voice-over, a beat's phases are placed
//! from its own spoken words instead of fixed fractions.
//!
//! - ENTER starts `WORD_CUE_LEAD` before the first content word (never before
//!   the scene's overlap ends; (0.23) on the product path never later than
//!   the dead-air limit allows, see [`open_with_the_sentence`]);
//! - READ starts once the primary is named and its verdict is said (the end of
//!   the first clause after the primary anchor, else its last word + 0.25 s);
//! - EVOLVE starts at the secondary or keyword anchor, else the next clause;
//! - ANTICIPATE starts at the last word's end plus a 0.25–0.6 s tail, before
//!   the bridge.
//!
//! The targets are clamped into the legal ranges derived from the planned
//! lifecycle's own spans (`lifecycle::retarget`), so a short sentence never
//! starves a phase. Each boundary records where it came from (`PhaseSource`).
//! Without speech, or for a beat with no spoken words, nothing changes
//! (golden-safe). See docs/SCENE_LIFECYCLE.md.

use super::speech_plan::{anchor_time, beat_anchors, WORD_CUE_LEAD};
use super::{round3, taste_rules, BeatPlan, Ctx};
use crate::checks::{DEAD_AIR_FIRST_READABLE_S, DEAD_AIR_MAX_HOLD_S};
use crate::intent::CreativeIntent;
use crate::motion::lifecycle::{self, PhaseTargets, TargetsUsed};
use crate::scene::Lifecycle;
use crate::speech::{
    normalize_word, statement_tokens, BeatPhases, PhaseSource, RevealAnchor, SpeechMap,
    EVOLVE_SNAP_WINDOW,
};

/// READ falls back to this long after the primary's last word when no clause
/// ends after it (seconds).
const READ_AFTER_NAME: f64 = 0.25;
/// ANTICIPATE starts this long after the last word at least ...
const TAIL_MIN: f64 = 0.25;
/// ... and at most (seconds).
const TAIL_MAX: f64 = 0.6;
/// How far ahead (in tokens) a spoken word may find its written token when
/// the spoken and written word counts differ.
const ALIGN_LOOKAHEAD: usize = 4;
/// Characters that end a clause when they trail a written token.
const CLAUSE_END: &[char] = &[',', ';', ':', '.', '!', '?', '—', '–', '…'];

/// Function words (and line-opening fillers like "so", "well", "okay") that
/// never start ENTER (normalised: lowercase, no apostrophes). Adverbs such as
/// "only" or "just" count as content. A private list: speech planning keeps
/// its own.
const STOP: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "so", "if", "then", "than", "as", "of", "to", "in", "on",
    "at", "by", "for", "with", "from", "into", "onto", "about", "over", "under", "up", "out",
    "off", "is", "are", "was", "were", "be", "been", "being", "am", "do", "does", "did", "have",
    "has", "had", "will", "would", "can", "could", "shall", "should", "may", "might", "must", "it",
    "its", "this", "that", "these", "those", "there", "here", "i", "me", "my", "we", "us", "our",
    "you", "your", "they", "them", "their", "he", "him", "his", "she", "her", "what", "which",
    "who", "whom", "when", "where", "why", "how", "not", "no", "well", "now", "oh", "okay", "ok",
    "yes", "all", "some", "any", "every", "each", "thats", "theres", "heres", "whats", "lets",
    "im", "ive", "youre", "youve", "theyre", "weve", "isnt", "arent", "wasnt", "dont", "doesnt",
    "didnt", "cant", "wont",
];

/// (0.23 W4) Seconds from ENTER to the first thing a beat shows (its kicker
/// or headline starts to read; 0.05–0.14 s over the looks): the latest ENTER
/// that still keeps the beat inside the dead-air limits is the limit minus
/// this.
const ENTER_TO_CONTENT: f64 = 0.15;

/// One spoken word of the beat, in scene-local seconds.
#[derive(Debug, Clone, PartialEq)]
struct Word {
    text: String,
    start: f64,
    end: f64,
}

/// Move each plan's phase boundaries onto the beat's spoken words and return,
/// per beat, where each boundary came from. Runs after the durations are
/// planned and before `speech_plan::snap_evolve`.
pub(crate) fn apply(
    plans: &mut [BeatPlan],
    intent: &CreativeIntent,
    speech: &SpeechMap,
) -> Vec<BeatPhases> {
    let mut out = Vec::with_capacity(plans.len());
    for plan in plans.iter_mut() {
        let words: Vec<Word> = speech
            .sentences
            .iter()
            .find(|s| s.beat == plan.index)
            .map(|sentence| {
                speech
                    .words_in(sentence)
                    .iter()
                    .map(|w| Word {
                        text: w.text.clone(),
                        start: w.start - plan.start,
                        end: w.end - plan.start,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let beat = intent.beats.get(plan.index);
        let used = match beat {
            Some(beat) if !words.is_empty() => {
                let line = taste_rules::display_text(beat, true).spoken;
                let targets = phase_targets(&plan.life, &beat_anchors(beat), &words, &line);
                let (mut life, used) = lifecycle::retarget(&plan.life, &targets);
                if used.evolve {
                    life.evolve = evolve_on_a_word(&life, &plan.life, &words);
                }
                plan.life = life;
                used
            }
            _ => TargetsUsed::default(),
        };
        out.push(phases(plan.index, used));
    }
    out
}

/// (0.23 W4) Whether the dead-air rules apply to this compile: the product
/// path (`--variety`; the reel and MCP always pass it). `compile` without
/// `--variety` stays byte-identical (plan §0, fix policy F2), so its late
/// ENTERs, 0.75 s fly-in and long wipes and blooms are what they were. Every
/// rule of this task (the ENTER of [`open_with_the_sentence`], the fly-in and
/// bloom of `fx`, the wipe of `transition`, the placeholder kicker of
/// `recipes`) asks here; widening the fix to default compiles is this one
/// line.
pub(crate) fn dead_air_rules(ctx: &Ctx) -> bool {
    ctx.direction_seed.is_some()
}

/// (0.23 W4) How long a hero's entrance takes to read (seconds): a hero that
/// starts to enter later than the dead-air limit minus this has nothing
/// readable on screen within the limit.
const HERO_ENTRANCE_S: f64 = 0.25;

/// (0.23 W4) Whether a hero (or card, or title) named by `words` waits for its
/// word so long that its entrance would not read within [`DEAD_AIR_MAX_HOLD_S`]
/// of the beat's start, in a look that holds titles (`TitleReveal::Hold`:
/// cinematic, documentary, studio) under a voice-over, on the product path.
/// Such a beat shows its kicker and nothing else until the narrator gets
/// there, so its builder leaves a placeholder at the hero's own scale: the
/// card (paper, label) or the label line, never the value.
pub(crate) fn hero_waits(ctx: &Ctx, words: &[String]) -> bool {
    use super::art_direction::TitleReveal;
    if !dead_air_rules(ctx) || ctx.spoken.is_empty() || words.is_empty() {
        return false;
    }
    let holds = ctx
        .art
        .as_ref()
        .is_some_and(|(a, _)| a.fx.title_reveal == TitleReveal::Hold);
    let named: Vec<&str> = words.iter().map(String::as_str).collect();
    holds
        && anchor_time(&ctx.spoken, &named)
            .is_some_and(|at| at - WORD_CUE_LEAD > DEAD_AIR_MAX_HOLD_S - HERO_ENTRANCE_S)
}

/// (0.23 W4) Dead air at a beat's start. A sentence that opens on function
/// words ("Every night, ...", "You do not need ...") has its ENTER placed at
/// the first content word, a second or more into the beat: only the previous
/// beat's handoff or the bare backdrop is on screen until then. When the
/// ENTER the lifecycle settled on would leave the beat's first content past
/// [`DEAD_AIR_FIRST_READABLE_S`] into the video (first beat) or
/// [`DEAD_AIR_MAX_HOLD_S`] after the beat starts, the beat enters with its
/// sentence instead: [`WORD_CUE_LEAD`] before its first spoken word, never
/// before the planned ENTER and never after the latest ENTER that meets the
/// limit. SETTLE moves with ENTER (the arrival keeps its planned length);
/// READ and later are untouched, and word cues still hold every anchored
/// group until its own word.
///
/// `spoken` is the beat's sentence as (word, scene-local start). Returns the
/// plan to build the beat with, or `None` when its ENTER is fine, was not
/// placed from speech, or nothing is spoken. Applied by `recipes::build_beat`
/// on the product path only (`--variety`; `compile` without it stays
/// byte-identical).
pub(crate) fn open_with_the_sentence(
    plan: &BeatPlan,
    spoken: &[(String, f64)],
) -> Option<BeatPlan> {
    plan.enter_override?;
    let first = spoken.first()?.1;
    let limit = if plan.index == 0 {
        DEAD_AIR_FIRST_READABLE_S
    } else {
        DEAD_AIR_MAX_HOLD_S
    };
    let latest = limit - ENTER_TO_CONTENT;
    let enter = plan.life.enter;
    if enter <= latest + 1e-9 {
        return None;
    }
    // The ENTER the overlap alone would give (the lifecycle never goes
    // earlier than its planned ENTER).
    let planned = BeatPlan {
        enter_override: None,
        ..plan.clone()
    }
    .enter_at();
    let target = round3((first - WORD_CUE_LEAD).max(planned).min(latest));
    if target >= enter - 5e-4 {
        return None;
    }
    let mut opened = plan.clone();
    opened.life.enter = target;
    opened.life.settle = round3(opened.life.settle - (enter - target)).max(target);
    opened.enter_override = Some(target);
    Some(opened)
}

fn phases(beat: usize, used: TargetsUsed) -> BeatPhases {
    let src = |s: bool| {
        if s {
            PhaseSource::Speech
        } else {
            PhaseSource::Fraction
        }
    };
    BeatPhases {
        beat,
        enter: src(used.enter),
        read: src(used.read),
        evolve: src(used.evolve),
        anticipate: src(used.anticipate),
    }
}

/// Where the beat's words want its phase boundaries (scene-local seconds,
/// before clamping). `life` is the planned lifecycle, `anchors` the beat's
/// intent-level anchors (`primary`, `secondary`, `keyword`), `words` the
/// sentence's words in time order and `line` the spoken line as written (its
/// punctuation marks the clause ends).
fn phase_targets(
    life: &Lifecycle,
    anchors: &[RevealAnchor],
    words: &[Word],
    line: &str,
) -> PhaseTargets {
    let mut t = PhaseTargets::default();
    if words.is_empty() {
        return t;
    }
    // The same (text, local start) pairs the word cues match against.
    let spoken: Vec<(String, f64)> = words
        .iter()
        .map(|w| (w.text.clone(), round3(w.start)))
        .collect();
    let group = |name: &str| -> Vec<&str> {
        anchors
            .iter()
            .find(|a| a.group == name)
            .map(|a| a.words.iter().map(String::as_str).collect())
            .unwrap_or_default()
    };
    let clause_end = clause_flags(words, &clause_tokens(line));

    // ENTER: just before the first content word.
    t.enter = words
        .iter()
        .find(|w| is_content(&w.text))
        .map(|w| w.start - WORD_CUE_LEAD);

    // READ: the primary is named and its clause is said.
    let primary = group("primary");
    let mut read_clause = None;
    if let Some(pi) = first_hit(&spoken, 0, &primary) {
        match (pi..words.len()).find(|&i| clause_end[i]) {
            Some(ci) => {
                read_clause = Some(ci);
                t.read = Some(words[ci].end);
            }
            None => {
                // The name may run over several words ("earth globe").
                let mut last = pi;
                while last + 1 < words.len() && hits(&spoken[last + 1], &primary) {
                    last += 1;
                }
                t.read = Some(words[last].end + READ_AFTER_NAME);
            }
        }
    }

    // EVOLVE: B is named after READ (secondary, else keyword), else the next
    // clause starts.
    let read_ref = t.read.unwrap_or(life.read);
    let after = words
        .iter()
        .position(|w| round3(w.start) >= round3(read_ref))
        .unwrap_or(words.len());
    let evolve_at = first_hit(&spoken, after, &group("secondary"))
        .or_else(|| first_hit(&spoken, after, &group("keyword")))
        .or_else(|| {
            let from = read_clause.map_or(after, |ci| (ci + 1).max(after));
            (from.max(1)..words.len()).find(|&i| clause_end[i - 1])
        });
    t.evolve = evolve_at.map(|i| round3(words[i].start));

    // ANTICIPATE: the last word plus a tail that grows with the time left.
    let last_end = words.iter().map(|w| w.end).fold(f64::MIN, f64::max);
    let live = life.bridge - life.enter;
    let left = if live > 0.0 {
        ((life.bridge - last_end) / live).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let tail = (TAIL_MIN + (TAIL_MAX - TAIL_MIN) * left).clamp(TAIL_MIN, TAIL_MAX);
    t.anticipate = Some(last_end + tail);
    t
}

/// A speech-placed EVOLVE that the clamp moved off its word goes onto the
/// nearest word start (within [`EVOLVE_SNAP_WINDOW`]) that keeps READ and
/// EVOLVE at their minimum spans, so `speech_plan::snap_evolve` finds it
/// already on a word and leaves it there. Without such a word it stays put.
fn evolve_on_a_word(life: &Lifecycle, planned: &Lifecycle, words: &[Word]) -> f64 {
    let (read_min, evolve_min) = lifecycle::target_minimums(planned);
    let lo = life.read + read_min - 1e-9;
    let hi = life.anticipate - evolve_min + 1e-9;
    let mut best: Option<(f64, f64)> = None; // (distance, local start)
    for w in words {
        let s = round3(w.start);
        let d = (s - life.evolve).abs();
        if d > EVOLVE_SNAP_WINDOW + 1e-9 || s < lo || s > hi || s <= life.read {
            continue;
        }
        // Strict `<` keeps the earlier word on ties (words are in time order).
        if best.is_none_or(|(bd, _)| d < bd - 1e-12) {
            best = Some((d, s));
        }
    }
    best.map_or(life.evolve, |(_, s)| s)
}

/// A word that carries meaning (not a function word).
fn is_content(text: &str) -> bool {
    let n = normalize_word(text);
    !n.is_empty() && !STOP.contains(&n.as_str())
}

/// Whether the spoken word names any of `words` (the shared anchor matcher).
/// Whether one spoken word is one of the words of the anchor entries (an
/// entry may be several words: each counts on its own here, so a name's run
/// can be extended word by word).
fn hits(word: &(String, f64), words: &[&str]) -> bool {
    let singles: Vec<&str> = words.iter().flat_map(|e| e.split_whitespace()).collect();
    !singles.is_empty() && anchor_time(std::slice::from_ref(word), &singles).is_some()
}

/// Index of the first spoken word at or after `from` where any of `words`
/// starts (a multi-word entry matches as a sequence, numbers by value).
fn first_hit(spoken: &[(String, f64)], from: usize, words: &[&str]) -> Option<usize> {
    if words.is_empty() || from >= spoken.len() {
        return None;
    }
    let at = anchor_time(&spoken[from..], words)?;
    (from..spoken.len()).find(|&i| (spoken[i].1 - at).abs() < 1e-9)
}

/// The spoken line's tokens as written (`statement_tokens`), each with
/// whether it ends a clause (trailing `,` `;` `:` `.` `!` `?` or a dash). A
/// free-standing dash ends the clause of the token before it.
fn clause_tokens(line: &str) -> Vec<(String, bool)> {
    let keep = |c: char| c.is_alphanumeric() || matches!(c, '%' | '₹' | '$' | '€' | '£' | '+');
    let mut out: Vec<(String, bool)> = Vec::new();
    for raw in line.split_whitespace() {
        let core = raw.trim_end_matches(|c: char| !keep(c)).len();
        let ends = raw[core..].contains(CLAUSE_END);
        match statement_tokens(raw).into_iter().next() {
            Some(token) => out.push((token, ends)),
            None => {
                if let Some(prev) = out.last_mut() {
                    prev.1 |= ends;
                }
            }
        }
    }
    out
}

/// Per spoken word: whether it ends a clause. Index-by-index when the spoken
/// and written word counts agree; otherwise each word looks for its written
/// token a few tokens ahead (unmatched words end no clause).
fn clause_flags(words: &[Word], tokens: &[(String, bool)]) -> Vec<bool> {
    if words.len() == tokens.len() {
        return tokens.iter().map(|t| t.1).collect();
    }
    let mut flags = vec![false; words.len()];
    let mut next = 0;
    for (i, w) in words.iter().enumerate() {
        let key = normalize_word(&w.text);
        let window = next..tokens.len().min(next + ALIGN_LOOKAHEAD);
        if let Some(k) = window
            .into_iter()
            .find(|&k| normalize_word(&tokens[k].0) == key)
        {
            flags[i] = tokens[k].1;
            next = k + 1;
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech::RevealRole;

    fn words(list: &[(&str, f64, f64)]) -> Vec<Word> {
        list.iter()
            .map(|(t, s, e)| Word {
                text: t.to_string(),
                start: *s,
                end: *e,
            })
            .collect()
    }

    fn anchor(group: &str, words: &[&str]) -> RevealAnchor {
        RevealAnchor {
            group: group.into(),
            words: words.iter().map(|w| w.to_string()).collect(),
            role: RevealRole::Content,
        }
    }

    fn life() -> Lifecycle {
        Lifecycle {
            enter: 0.25,
            settle: 1.15,
            read: 1.45,
            evolve: 3.45,
            anticipate: 5.1,
            bridge: 5.7,
        }
    }

    #[test]
    fn clause_tokens_follow_statement_tokens_and_mark_clause_ends() {
        let line = "Tea is calm, coffee is loud — and \"that's\" it… ₹5,200!";
        let tokens = clause_tokens(line);
        let texts: Vec<String> = tokens.iter().map(|t| t.0.clone()).collect();
        assert_eq!(texts, statement_tokens(line));
        let ends: Vec<bool> = tokens.iter().map(|t| t.1).collect();
        assert_eq!(
            ends,
            vec![false, false, true, false, false, true, false, false, true, true]
        );
    }

    #[test]
    fn clause_flags_align_by_text_when_counts_differ() {
        let tokens = clause_tokens("Tea is calm, coffee is loud.");
        // A spoken filler ("um").
        let w = words(&[
            ("Tea", 0.0, 0.1),
            ("um", 0.1, 0.2),
            ("is", 0.2, 0.3),
            ("calm", 0.3, 0.4),
            ("coffee", 0.4, 0.5),
            ("is", 0.5, 0.6),
            ("loud", 0.6, 0.7),
        ]);
        assert_eq!(
            clause_flags(&w, &tokens),
            vec![false, false, false, true, false, false, true]
        );
        // A dropped word ("is").
        let w = words(&[
            ("Tea", 0.0, 0.1),
            ("calm", 0.2, 0.3),
            ("coffee", 0.4, 0.5),
            ("is", 0.5, 0.6),
            ("loud", 0.6, 0.7),
        ]);
        assert_eq!(
            clause_flags(&w, &tokens),
            vec![false, true, false, false, true]
        );
    }

    #[test]
    fn a_then_b_sentence_targets_every_phase() {
        let w = words(&[
            ("Tea", 0.6, 1.0),
            ("is", 1.1, 1.25),
            ("calm", 1.4, 2.2),
            ("coffee", 3.4, 3.8),
            ("is", 3.9, 4.05),
            ("loud", 4.15, 4.6),
        ]);
        let anchors = [
            anchor("primary", &["Tea"]),
            anchor("secondary", &["Coffee"]),
        ];
        let t = phase_targets(&life(), &anchors, &w, "Tea is calm, coffee is loud.");
        assert!((t.enter.unwrap() - (0.6 - WORD_CUE_LEAD)).abs() < 1e-9);
        assert_eq!(t.read, Some(2.2));
        assert_eq!(t.evolve, Some(3.4));
        let tail = 0.25 + 0.35 * (5.7 - 4.6) / (5.7 - 0.25);
        assert!((t.anticipate.unwrap() - (4.6 + tail)).abs() < 1e-9);
    }

    #[test]
    fn without_a_clause_read_follows_the_name() {
        let w = words(&[
            ("Your", 0.4, 0.6),
            ("earth", 0.7, 1.0),
            ("globe", 1.0, 1.3),
            ("spins", 1.4, 1.8),
        ]);
        let anchors = [anchor("primary", &["earth globe"])];
        let t = phase_targets(&life(), &anchors, &w, "Your earth globe spins");
        assert!((t.enter.unwrap() - (0.7 - WORD_CUE_LEAD)).abs() < 1e-9);
        assert!((t.read.unwrap() - 1.55).abs() < 1e-9);
        assert_eq!(t.evolve, None);
    }

    #[test]
    fn evolve_falls_back_to_the_next_clause_and_ignores_earlier_names() {
        // The keyword is said before READ, so the next clause starts EVOLVE.
        let w = words(&[
            ("Sunlight", 0.4, 0.8),
            ("takes", 0.8, 1.0),
            ("eight", 1.0, 1.2),
            ("minutes", 1.2, 1.6),
            ("to", 1.6, 1.7),
            ("reach", 1.7, 1.9),
            ("us", 1.9, 2.1),
            ("so", 2.4, 2.5),
            ("you", 2.5, 2.6),
            ("see", 2.6, 2.9),
            ("the", 2.9, 3.0),
            ("past", 3.0, 3.4),
        ]);
        let anchors = [
            anchor("primary", &["8 min", "sunlight"]),
            anchor("keyword", &["minutes"]),
        ];
        let line = "Sunlight takes eight minutes to reach us, so you see the past.";
        let t = phase_targets(&life(), &anchors, &w, line);
        assert_eq!(t.read, Some(2.1));
        assert_eq!(t.evolve, Some(2.4));
    }

    #[test]
    fn a_sentence_that_names_nothing_targets_only_enter_and_anticipate() {
        let w = words(&[
            ("It", 0.4, 0.5),
            ("happens", 0.5, 0.9),
            ("every", 0.9, 1.1),
            ("day", 1.1, 1.4),
        ]);
        let anchors = [anchor("primary", &["Rain"]), anchor("keyword", &["storm"])];
        let t = phase_targets(&life(), &anchors, &w, "It happens every day.");
        assert!((t.enter.unwrap() - (0.5 - WORD_CUE_LEAD)).abs() < 1e-9);
        assert_eq!(t.read, None);
        assert_eq!(t.evolve, None);
        assert!(t.anticipate.is_some());
    }

    #[test]
    fn the_tail_is_clamped() {
        let anchors = [anchor("primary", &["x"])];
        // Last word at the very start: the longest tail.
        let early = phase_targets(&life(), &anchors, &words(&[("Go", 0.2, 0.25)]), "Go");
        assert!((early.anticipate.unwrap() - (0.25 + 0.6)).abs() < 1e-9);
        // Last word past the bridge: the shortest tail.
        let late = phase_targets(&life(), &anchors, &words(&[("Go", 5.5, 6.0)]), "Go");
        assert!((late.anticipate.unwrap() - 6.25).abs() < 1e-9);
    }

    #[test]
    fn a_clamped_evolve_moves_onto_a_legal_word_start() {
        let planned = life();
        // read_min = max(0.5, 0.4 * 2.0) = 0.8; evolve_min = max(0.4, 0.4 *
        // 1.65) = 0.66. EVOLVE was clamped up to READ + read_min.
        let placed = Lifecycle {
            read: 2.2,
            evolve: 3.0,
            ..planned
        };
        let w = words(&[
            ("before", 2.97, 3.0), // nearer, but inside the READ minimum
            ("after", 3.05, 3.4),
        ]);
        assert_eq!(evolve_on_a_word(&placed, &planned, &w), 3.05);
        // No legal word within the window: it stays.
        let far = words(&[("before", 2.97, 3.0), ("later", 3.3, 3.5)]);
        assert_eq!(evolve_on_a_word(&placed, &planned, &far), 3.0);
        // Never past ANTICIPATE minus the EVOLVE minimum (5.1 - 0.66).
        let late = Lifecycle {
            evolve: 4.44,
            ..planned
        };
        let w = words(&[("tail", 4.5, 4.9)]);
        assert_eq!(evolve_on_a_word(&late, &planned, &w), 4.44);
    }

    #[test]
    fn stop_words_are_not_content() {
        assert!(!is_content("The"));
        assert!(!is_content("that's"));
        assert!(!is_content("And"));
        assert!(!is_content("Okay"));
        assert!(is_content("only"));
        assert!(is_content("Saturn"));
        assert!(is_content("8"));
        assert!(is_content("₹5,200"));
    }

    // (0.23 W4) Dead air: the ENTER of a sentence that opens on function words.

    const THREE_BEATS: &str = r#"{"version":"0.2","title":"t","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"One","primary":{"kind":"phrase","value":"one"},"energy":"calm"},
        {"purpose":"emphasize","statement":"Two","primary":{"kind":"phrase","value":"two"},"energy":"building"},
        {"purpose":"emphasize","statement":"Three","primary":{"kind":"phrase","value":"three"},"energy":"calm"}]}"#;

    /// Beat `index` of a three-beat plan whose ENTER speech placed at `enter`.
    fn plan_entering_at(index: usize, enter: f64) -> BeatPlan {
        let style: crate::style::StyleProfile =
            serde_json::from_str(r#"{"seed":1}"#).expect("style");
        let intent = CreativeIntent::from_json(THREE_BEATS).expect("intent");
        let mut plan = crate::compiler::plan_timing(&intent, &style)[index].clone();
        let span = plan.life.settle - plan.life.enter;
        plan.life.enter = enter;
        plan.life.settle = enter + span;
        plan.enter_override = Some(enter);
        plan
    }

    fn spoken(list: &[(&str, f64)]) -> Vec<(String, f64)> {
        list.iter().map(|(t, s)| (t.to_string(), *s)).collect()
    }

    #[test]
    fn a_sentence_opening_on_function_words_enters_with_its_sentence() {
        // "Every night, ...": the first content word is a second in.
        let plan = plan_entering_at(0, 0.92);
        let span = plan.life.settle - plan.life.enter;
        let open = open_with_the_sentence(&plan, &spoken(&[("Every", 0.35), ("night", 1.04)]))
            .expect("a late ENTER moves");
        // WORD_CUE_LEAD before "Every" is 0.23: the planned ENTER (0.25) is the floor.
        assert_eq!(open.life.enter, 0.25);
        assert_eq!(open.enter_override, Some(0.25));
        // The arrival keeps its length; READ and later stay.
        assert!((open.life.settle - open.life.enter - span).abs() < 2e-3);
        assert_eq!(open.life.read, plan.life.read);
        assert_eq!(open.life.anticipate, plan.life.anticipate);
        assert!(open.life.enter <= open.life.settle && open.life.settle <= open.life.read);
    }

    #[test]
    fn an_enter_that_meets_the_limit_is_left_alone() {
        // First beat: the latest ENTER is 0.5 - 0.15; later beats: 1.2 - 0.15.
        let words = spoken(&[("Every", 0.35), ("night", 0.5)]);
        assert!(open_with_the_sentence(&plan_entering_at(0, 0.35), &words).is_none());
        assert!(open_with_the_sentence(&plan_entering_at(1, 1.05), &words).is_none());
        assert!(open_with_the_sentence(&plan_entering_at(1, 1.06), &words).is_some());
        // ENTER not placed from speech, or nothing spoken: nothing to do.
        let mut unplaced = plan_entering_at(1, 1.5);
        unplaced.enter_override = None;
        assert!(open_with_the_sentence(&unplaced, &words).is_none());
        assert!(open_with_the_sentence(&plan_entering_at(1, 1.5), &[]).is_none());
    }

    #[test]
    fn a_late_voice_never_pushes_the_opening_past_the_limit() {
        // The narrator starts a second in: the first beat still enters at the
        // latest ENTER that meets the limit, not at the voice.
        let words = spoken(&[("Every", 1.0), ("night", 1.2)]);
        let open = open_with_the_sentence(&plan_entering_at(0, 1.1), &words).expect("moves");
        assert!((open.life.enter - 0.35).abs() < 1e-9, "{:?}", open.life);
        // Later beats may wait for their sentence, up to the limit.
        let words = spoken(&[("You", 1.4), ("need", 1.6)]);
        let open = open_with_the_sentence(&plan_entering_at(1, 1.5), &words).expect("moves");
        assert!((open.life.enter - 1.05).abs() < 1e-9, "{:?}", open.life);
    }
}
