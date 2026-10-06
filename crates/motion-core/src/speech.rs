//! SpeechMap v0.1 (0.10): provider-neutral word timing for a voice-over.
//!
//! A `<name>.speech.json` sits beside the intent as an OPERATOR file (weak
//! models never write it). It is produced outside the deterministic core by
//! `motion-engine voice` (crate `motion-voice`, which owns every network
//! call) and read here as plain data. Everything in this module is pure and
//! deterministic. See docs/VOICE.md.
//!
//! FROZEN: the types, field names and the public function signatures.
//! Bodies are filled by the 0.10 voice tasks.

use serde::{Deserialize, Serialize};

pub const SPEECH_VERSION: &str = "0.1";

/// Minimum word duration after repair (seconds).
pub const MIN_WORD_SECONDS: f64 = 0.060;
/// Gaps between consecutive words inside one sentence are clamped to this
/// (seconds); longer silences are pauses between sentences only.
pub const MAX_INTRA_SENTENCE_GAP: f64 = 0.600;
/// EVOLVE events snap to the nearest word start within this window (seconds).
pub const EVOLVE_SNAP_WINDOW: f64 = 0.120;

/// `<name>.speech.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechMap {
    pub version: String,
    /// The mixed voice-over WAV, relative to the directory holding the speech file.
    pub audio: String,
    pub sample_rate: u32,
    /// Seconds.
    pub duration: f64,
    /// `openrouter`, `macos_say` or `fixture`.
    pub provider: String,
    /// e.g. `deepgram/flux-tts:free`, `fish-audio/s2.1-pro-free:free`, `macos/say`.
    pub model: String,
    pub voice: String,
    pub words: Vec<SpeechWord>,
    /// One per beat, in beat order (per-sentence synthesis makes these exact).
    pub sentences: Vec<SpeechSentence>,
    /// (0.20) What the recogniser heard (`--align local|asr`): the recognised
    /// words with their measured times, kept beside the script words so QA
    /// can report what was actually said (`spoken_mismatch`). Empty with
    /// onset timing or on files written before 0.20.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recognised: Vec<RecognisedWord>,
    /// (0.20) How the word times were measured: `onset`, `asr`, or
    /// `local:<model>:beam<n>`. `None` on files written before 0.20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechWord {
    /// The word as written in the beat statement (punctuation stripped).
    pub text: String,
    /// Seconds on the voice-over timeline.
    pub start: f64,
    pub end: f64,
    /// 1.0 = provider timestamp; < 1.0 = derived (onset-snapped estimate).
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechSentence {
    /// Beat index (0-based) whose statement this sentence speaks.
    pub beat: usize,
    pub start: f64,
    pub end: f64,
}

/// What the repair pass changed or could not match. Never invents words.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RepairReport {
    /// Words whose times were moved to make the map monotonic.
    pub reordered: usize,
    /// Words stretched to [`MIN_WORD_SECONDS`].
    pub stretched: usize,
    /// Intra-sentence gaps clamped to [`MAX_INTRA_SENTENCE_GAP`].
    pub gaps_clamped: usize,
    /// Statement words with no spoken match ("beat N: word").
    pub unmatched_statement_words: Vec<String>,
    /// Spoken words with no statement match ("beat N: word").
    pub unmatched_spoken_words: Vec<String>,
}

/// Speech-led beat timing for the compiler (CompileOptions.speech).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechPadding {
    /// Silence before a beat's sentence starts (seconds).
    pub lead: f64,
    /// Hold after the last sentence ends (seconds).
    pub tail: f64,
}

impl SpeechMap {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Words whose start lies inside `sentence` (in order).
    pub fn words_in(&self, sentence: &SpeechSentence) -> Vec<&SpeechWord> {
        self.words
            .iter()
            .filter(|w| w.start >= sentence.start - 1e-9 && w.start < sentence.end + 1e-9)
            .collect()
    }
}

/// Normalise a word for matching: lowercase, alphanumerics plus `%`, `.`
/// inside numbers, currency symbols kept. Pure.
pub fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|c| {
            c.is_alphanumeric() || matches!(c, '%' | '₹' | '$' | '€' | '£' | '+' | '−' | '-')
        })
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// One EVOLVE event moved onto a word start (absolute project seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolveSnap {
    pub beat: usize,
    pub from: f64,
    pub to: f64,
}

/// What a speech-led compile did, written to `ProjectMeta.speech` (absent
/// without `--speech`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechRecord {
    pub model: String,
    pub voice: String,
    /// Number of words in the (repaired) SpeechMap.
    pub words: usize,
    /// Each beat's EVOLVE event that moved onto a word start.
    pub evolve_snaps: Vec<EvolveSnap>,
    /// Words that did not align: statement words as `beat N: word`, spoken
    /// words as `beat N: ~word` (N is 1-based).
    pub unmatched: Vec<String>,
    /// A MusicPlan was supplied too; its downbeats were NOT used for beat
    /// durations (speech overrides music snapping).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub music_ignored: bool,
    /// (0.10 Q) Content groups that now enter when their word is spoken.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub word_cues: Vec<WordCue>,
    /// (0.20) Where each beat's phase boundaries came from (speech-aware
    /// lifecycle); empty before 0.20 and for beats without narration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<BeatPhases>,
}

/// (0.10 Q) One content group moved to enter on its spoken word (scene time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordCue {
    pub beat: usize,
    pub group: String,
    pub word: String,
    pub from: f64,
    pub to: f64,
    /// (0.20) The role of the explicit anchor that cued the group; `None`
    /// when the move came from the name-matching fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<RevealRole>,
    /// (0.20) The move was limited (a delay that would have left too little
    /// read time, or an earlier move held at ENTER).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clamped: bool,
}

// ---------------------------------------------------------------------------
// (0.20) Word-anchored reveals: contracts shared by builders, the cue
// matcher, the lifecycle planner and speech QA.
// ---------------------------------------------------------------------------

/// (0.20) One word as the recogniser heard it (measured times, seconds on
/// the voice-over timeline). Provider-neutral: whisper or Deepgram.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecognisedWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

/// (0.20) What a reveal anchor stands for. The role decides how a word cue
/// may move the group and which QA rule judges it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealRole {
    /// A picture, card or figure: the thing the words name.
    Content,
    /// The display title. The look's `TitleReveal` policy decides whether it
    /// waits for its key word.
    Title,
    /// A label naming a part (a layer of the column, a relation node).
    Label,
    /// A stamp that slams on its keyword.
    Stamp,
    /// Value text (the amount on a figure card).
    Value,
}

/// (0.20) A builder's declaration that the layers of `group` stand for
/// `words`, so they should become readable when the narrator says them.
///
/// `group` is a layer-id path after the scene prefix (`hero`, `card.0`,
/// `title`): a layer belongs to the anchor when its id minus the prefix
/// equals `group` or starts with `group` + `"."`. `words` are matched as
/// spoken (light stemming, numbers as digits or as said, a multi-word entry
/// as a sequence); the earliest match in the beat wins. Builders tag anchors
/// through `B::reveal`; the compiler records them in `ArtRecord.reveals`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevealAnchor {
    pub group: String,
    pub words: Vec<String>,
    pub role: RevealRole,
}

/// (0.20) Where a planned lifecycle boundary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseSource {
    /// Placed from the beat's own spoken words (speech-aware lifecycle).
    Speech,
    /// The closed-form fraction of the beat (`lifecycle::plan`).
    Fraction,
}

/// (0.20) The source of each phase boundary of one beat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeatPhases {
    pub beat: usize,
    pub enter: PhaseSource,
    pub read: PhaseSource,
    pub evolve: PhaseSource,
    pub anticipate: PhaseSource,
}

/// `speech_rate` FAILs when a sentence's syllables per second fall outside
/// this range (crammed narration or a stalled take).
pub const SPEECH_RATE_FAIL: (f64, f64) = (3.0, 9.0);
/// `speech_rate` WARNs outside this range.
pub const SPEECH_RATE_WARN: (f64, f64) = (3.5, 7.5);
/// `reveal_before_speech`: an anchored layer may become readable at most
/// this long (seconds) before its anchor word starts. The kicker is exempt.
pub const REVEAL_LEAD_MAX: f64 = 0.35;
/// `reveal_late` (WARN): readable more than this long (seconds) after its
/// anchor word ends means the viewer heard it before seeing it.
pub const REVEAL_LATE_MAX: f64 = 0.8;
/// A layer counts as readable from this opacity ...
pub const READABLE_OPACITY: f32 = 0.5;
/// ... with at most this blur (px at a 1080 px short side), on canvas, and
/// with any glyph cascade landed.
pub const READABLE_BLUR_PX: f32 = 2.0;
/// A word cue may delay a group only while its arrival still finishes this
/// long (seconds) before ANTICIPATE, so it can still be read.
pub const CUE_MIN_READ: f64 = 0.6;

/// Tokens of a beat statement as written (punctuation trimmed from both ends,
/// no empties): the words a voice-over speaks and captions show.
pub fn statement_tokens(statement: &str) -> Vec<String> {
    statement
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| {
                !(c.is_alphanumeric() || matches!(c, '%' | '₹' | '$' | '€' | '£' | '+'))
            })
        })
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Minimum Levenshtein ratio for a spoken word to match a statement word.
pub const MATCH_RATIO: f64 = 0.6;

/// `1 - distance / max(len)` over the characters of two normalised words.
fn levenshtein_ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / longest as f64
}

/// Words a spoken number or amount is made of ("five thousand two hundred
/// rupees", "forty percent").
fn is_number_word(w: &str) -> bool {
    const WORDS: [&str; 48] = [
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
        "thirty",
        "forty",
        "fifty",
        "sixty",
        "seventy",
        "eighty",
        "ninety",
        "hundred",
        "thousand",
        "lakh",
        "lakhs",
        "crore",
        "crores",
        "million",
        "billion",
        "trillion",
        "point",
        "and",
        "percent",
        "rupees",
        "rupee",
        "dollars",
        "dollar",
        "euros",
        "euro",
        "pounds",
        "cents",
    ];
    WORDS.contains(&w)
}

fn is_numeric_token(t: &str) -> bool {
    t.chars().any(|c| c.is_ascii_digit())
}

const MAX_NUMBER_RUN: usize = 12;

/// One aligned unit: spoken words `spoken[from..to]` match statement word `stmt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Link {
    spoken_from: usize,
    spoken_to: usize,
    stmt: usize,
}

#[derive(Clone, Copy)]
enum Step {
    None,
    /// A run of this many spoken number words ↔ one numeric statement word.
    Group(usize),
    Match,
    SkipSpoken,
    SkipStatement,
}

/// Monotone alignment of spoken words to statement words by DP: one spoken
/// word to one statement word when their Levenshtein ratio is at least
/// [`MATCH_RATIO`], or a run of spoken number words to one numeric statement
/// word ("five thousand two hundred" ↔ "₹5,200"). Either side may be skipped
/// (reported, never invented). Maximises the summed match score; ties prefer
/// number groups, then 1:1 matches, then skipping spoken, then statement words.
fn align(spoken: &[String], statement: &[String]) -> Vec<Link> {
    let sp: Vec<String> = spoken.iter().map(|w| normalize_word(w)).collect();
    let st: Vec<String> = statement.iter().map(|w| normalize_word(w)).collect();
    let (m, k) = (sp.len(), st.len());
    // best[i][j]: score aligning spoken[..i] with statement[..j].
    let mut best = vec![vec![0.0f64; k + 1]; m + 1];
    let mut step = vec![vec![Step::None; k + 1]; m + 1];
    for i in 0..=m {
        for j in 0..=k {
            if i == 0 && j == 0 {
                continue;
            }
            let mut cur = f64::NEG_INFINITY;
            let mut choice = Step::None;
            if i > 0 && j > 0 && is_numeric_token(&statement[j - 1]) {
                let mut run = 0;
                while run < MAX_NUMBER_RUN && run < i && is_number_word(&sp[i - 1 - run]) {
                    run += 1;
                    let score = best[i - run][j - 1] + 1.0 + 0.01 * run as f64;
                    if score > cur + 1e-12 {
                        cur = score;
                        choice = Step::Group(run);
                    }
                }
            }
            if i > 0 && j > 0 {
                let ratio = levenshtein_ratio(&sp[i - 1], &st[j - 1]);
                if ratio >= MATCH_RATIO {
                    let score = best[i - 1][j - 1] + ratio;
                    if score > cur + 1e-12 {
                        cur = score;
                        choice = Step::Match;
                    }
                }
            }
            if i > 0 && best[i - 1][j] > cur + 1e-12 {
                cur = best[i - 1][j];
                choice = Step::SkipSpoken;
            }
            if j > 0 && best[i][j - 1] > cur + 1e-12 {
                cur = best[i][j - 1];
                choice = Step::SkipStatement;
            }
            best[i][j] = cur;
            step[i][j] = choice;
        }
    }
    let (mut i, mut j) = (m, k);
    let mut links = Vec::new();
    while i > 0 || j > 0 {
        match step[i][j] {
            Step::Group(len) => {
                links.push(Link {
                    spoken_from: i - len,
                    spoken_to: i,
                    stmt: j - 1,
                });
                i -= len;
                j -= 1;
            }
            Step::Match => {
                links.push(Link {
                    spoken_from: i - 1,
                    spoken_to: i,
                    stmt: j - 1,
                });
                i -= 1;
                j -= 1;
            }
            Step::SkipSpoken => i -= 1,
            Step::SkipStatement => j -= 1,
            // Only reachable when one side is empty.
            Step::None => {
                if i > 0 {
                    i -= 1;
                } else {
                    j -= 1;
                }
            }
        }
    }
    links.reverse();
    links
}

/// Words that do not align between one sentence's spoken words and its
/// statement: `(unmatched statement words, unmatched spoken words)`.
fn unmatched_in(spoken: &[String], statement: &[String]) -> (Vec<String>, Vec<String>) {
    let links = align(spoken, statement);
    let mut stmt_used = vec![false; statement.len()];
    let mut spoken_used = vec![false; spoken.len()];
    for l in &links {
        stmt_used[l.stmt] = true;
        spoken_used[l.spoken_from..l.spoken_to]
            .iter_mut()
            .for_each(|u| *u = true);
    }
    let pick = |words: &[String], used: &[bool]| -> Vec<String> {
        words
            .iter()
            .zip(used)
            .filter(|(_, u)| !**u)
            .map(|(w, _)| w.clone())
            .collect()
    };
    (pick(statement, &stmt_used), pick(spoken, &spoken_used))
}

/// Words of `map` that do not align with `statements`, formatted like
/// [`SpeechRecord::unmatched`] (`beat N: word` / `beat N: ~word`, N 1-based).
/// Pure; used by the compiler to record what repair could not match.
pub fn unmatched_words(map: &SpeechMap, statements: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for s in &map.sentences {
        let Some(statement) = statements.get(s.beat) else {
            continue;
        };
        let spoken: Vec<String> = map.words_in(s).iter().map(|w| w.text.clone()).collect();
        let (stmt, spk) = unmatched_in(&spoken, &statement_tokens(statement));
        out.extend(
            stmt.into_iter()
                .map(|w| format!("beat {}: {w}", s.beat + 1)),
        );
        out.extend(
            spk.into_iter()
                .map(|w| format!("beat {}: ~{w}", s.beat + 1)),
        );
    }
    out
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// Timing repair of one sentence's words (already sorted by start and assigned
/// to the window `[lo, hi]`).
fn repair_timing(words: &mut [SpeechWord], lo: f64, hi: f64, report: &mut RepairReport) {
    let n = words.len();
    // (a) Clamp into the sentence; ends never precede starts and never run past
    // the next word's start.
    let originals: Vec<(f64, f64)> = words.iter().map(|w| (w.start, w.end)).collect();
    for w in words.iter_mut() {
        w.start = w.start.clamp(lo, hi);
        w.end = w.end.clamp(w.start, hi);
    }
    for i in 0..n.saturating_sub(1) {
        let next = words[i + 1].start;
        if words[i].end > next {
            words[i].end = next.max(words[i].start);
        }
    }
    for (w, (s, e)) in words.iter().zip(&originals) {
        if !same(w.start, *s) || !same(w.end, *e) {
            report.reordered += 1;
        }
    }
    // (b) Minimum duration: borrow from the following gap; when the next word
    // sits right behind, it is pushed later (never overlapped).
    let mut prev_end = lo;
    for w in words.iter_mut() {
        if w.start < prev_end {
            w.start = prev_end;
            w.end = w.end.max(w.start);
            report.reordered += 1;
        }
        if w.end - w.start < MIN_WORD_SECONDS - 1e-9 {
            w.end = w.start + MIN_WORD_SECONDS;
            report.stretched += 1;
        }
        prev_end = w.end;
    }
    // A cramped sentence: pull the tail back inside `hi`, keeping every word
    // at least MIN_WORD_SECONDS when the sentence is long enough.
    if prev_end > hi + 1e-9 {
        let mut limit = hi;
        for w in words.iter_mut().rev() {
            w.end = w.end.min(limit);
            w.start = w.start.min((w.end - MIN_WORD_SECONDS).max(lo));
            limit = w.start;
        }
    }
    // (c) Gaps longer than MAX_INTRA_SENTENCE_GAP: word starts are the acoustic
    // truth (onset-snapped or provider-given) and captions are timed from them,
    // so NOTHING is shifted; each long gap is only counted in `gaps_clamped`.
    for pair in words.windows(2) {
        if pair[1].start - pair[0].end > MAX_INTRA_SENTENCE_GAP + 1e-9 {
            report.gaps_clamped += 1;
        }
    }
}

/// Align one sentence's words to its statement and rewrite matched text.
fn align_sentence(
    words: Vec<SpeechWord>,
    statement: &[String],
    beat: usize,
    report: &mut RepairReport,
) -> Vec<SpeechWord> {
    let spoken: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
    let links = align(&spoken, statement);
    let mut stmt_used = vec![false; statement.len()];
    let mut spoken_link: Vec<Option<usize>> = vec![None; words.len()];
    for (li, l) in links.iter().enumerate() {
        stmt_used[l.stmt] = true;
        for slot in &mut spoken_link[l.spoken_from..l.spoken_to] {
            *slot = Some(li);
        }
    }
    let mut out = Vec::with_capacity(words.len());
    let mut emitted = vec![false; links.len()];
    for (i, w) in words.iter().enumerate() {
        match spoken_link[i] {
            None => {
                report
                    .unmatched_spoken_words
                    .push(format!("beat {}: {}", beat + 1, w.text));
                out.push(w.clone());
            }
            Some(li) if !emitted[li] => {
                emitted[li] = true;
                let l = links[li];
                let group = &words[l.spoken_from..l.spoken_to];
                out.push(SpeechWord {
                    text: statement[l.stmt].clone(),
                    start: group[0].start,
                    end: group[group.len() - 1].end,
                    confidence: group
                        .iter()
                        .map(|g| g.confidence)
                        .fold(f32::INFINITY, f32::min),
                });
            }
            Some(_) => {}
        }
    }
    for (w, used) in statement.iter().zip(&stmt_used) {
        if !used {
            report
                .unmatched_statement_words
                .push(format!("beat {}: {w}", beat + 1));
        }
    }
    out
}

/// Deterministic repair of a [`SpeechMap`] against the beat statements.
///
/// 1. Sentences are sorted by start, clamped to `[0, duration]`, ends ≥ starts.
/// 2. Each word joins the sentence nearest to it (ties: the earlier) and is
///    clamped inside it; per sentence starts are made monotonic, ends ≥
///    starts and never past the next word, words shorter than
///    [`MIN_WORD_SECONDS`] borrow from the following gap (or push the next
///    word later), and intra-sentence gaps above [`MAX_INTRA_SENTENCE_GAP`]
///    are recorded (`gaps_clamped`) but not moved: word starts are the
///    acoustic truth and captions are timed from them.
/// 3. Spoken words are aligned to `statements[beat]` (Levenshtein ratio ≥
///    [`MATCH_RATIO`], monotone DP; a run of spoken number words matches one
///    numeric statement word). A matched word takes the statement's written
///    form ("₹5,200"). Unmatched words on either side are reported, never
///    invented; unmatched spoken words are kept as spoken.
///
/// Pure. FROZEN signature.
pub fn repair(map: &SpeechMap, statements: &[String]) -> (SpeechMap, RepairReport) {
    let mut report = RepairReport::default();
    let duration = map.duration.max(0.0);
    let clamp_t = |t: f64| {
        if t.is_finite() {
            t.clamp(0.0, duration)
        } else {
            0.0
        }
    };

    let mut sentences: Vec<SpeechSentence> = map
        .sentences
        .iter()
        .map(|s| {
            let start = clamp_t(s.start);
            SpeechSentence {
                beat: s.beat,
                start,
                end: clamp_t(s.end).max(start),
            }
        })
        .collect();
    sentences.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.beat.cmp(&b.beat)));

    // Words sorted by start (stable), then dealt to their nearest sentence.
    let mut sorted: Vec<SpeechWord> = map.words.clone();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    report.reordered += sorted
        .iter()
        .zip(&map.words)
        .filter(|(a, b)| a.start != b.start || a.text != b.text)
        .count();
    let mut groups: Vec<Vec<SpeechWord>> = vec![Vec::new(); sentences.len().max(1)];
    for mut w in sorted {
        w.start = clamp_t(w.start);
        w.end = clamp_t(w.end);
        let idx = sentences
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let d = if w.start < s.start {
                    s.start - w.start
                } else if w.start > s.end {
                    w.start - s.end
                } else {
                    0.0
                };
                (i, d)
            })
            .fold(None::<(usize, f64)>, |best, cand| match best {
                Some(b) if b.1 <= cand.1 => Some(b),
                _ => Some(cand),
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        groups[idx].push(w);
    }

    let mut words = Vec::with_capacity(map.words.len());
    for (idx, mut group) in groups.into_iter().enumerate() {
        let (lo, hi, beat) = match sentences.get(idx) {
            Some(s) => (s.start, s.end, Some(s.beat)),
            None => (0.0, duration, None),
        };
        repair_timing(&mut group, lo, hi, &mut report);
        match beat.and_then(|b| statements.get(b).map(|s| (b, s))) {
            Some((b, statement)) => {
                words.extend(align_sentence(
                    group,
                    &statement_tokens(statement),
                    b,
                    &mut report,
                ));
            }
            None => words.extend(group),
        }
    }

    let repaired = SpeechMap {
        recognised: map.recognised.clone(),
        alignment: map.alignment.clone(),
        sentences,
        words,
        duration,
        ..map.clone()
    };
    (repaired, report)
}

// ---------------------------------------------------------------------------
// Syllables (0.10 in motion-voice; shared here since 0.20 so speech QA can
// measure the speech rate without depending on the voice crate).
// ---------------------------------------------------------------------------

fn is_vowel(c: char) -> bool {
    matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

/// Vowel groups of a letters-only run, minus a silent final `e`; minimum 1.
/// An all-capitals vowel-less run ("TV", "UK") is spelled out: one per letter.
fn letter_syllables(run: &str) -> usize {
    let mut groups = 0;
    let mut prev = false;
    for c in run.chars() {
        let v = is_vowel(c);
        if v && !prev {
            groups += 1;
        }
        prev = v;
    }
    if groups == 0 {
        let n = run.chars().count();
        return if n >= 2 && run.chars().all(|c| c.is_uppercase()) {
            n
        } else {
            1
        };
    }
    let lower = run.to_ascii_lowercase();
    if groups > 1 && lower.ends_with('e') && !lower.ends_with("le") {
        groups -= 1;
    }
    groups.max(1)
}

fn below_twenty(n: u64) -> usize {
    match n {
        0 | 7 | 13..=16 | 18 | 19 => 2,
        11 | 17 => 3,
        _ => 1, // 1..=6, 8, 9, 10, 12
    }
}

fn tens_syllables(tens: u64) -> usize {
    if tens == 7 {
        3
    } else {
        2
    }
}

/// Syllables of an integer read aloud ("five thousand two hundred" = 6).
pub fn number_syllables(n: u64) -> usize {
    if n == 0 {
        return 2;
    }
    const SCALES: [(u64, usize); 3] = [(1_000_000_000, 2), (1_000_000, 2), (1_000, 2)];
    let mut rest = n;
    let mut total = 0;
    for (scale, syl) in SCALES {
        if rest >= scale {
            total += under_thousand(rest / scale) + syl;
            rest %= scale;
        }
    }
    if rest > 0 {
        total += under_thousand(rest);
    }
    total
}

fn under_thousand(n: u64) -> usize {
    let mut total = 0;
    let mut rest = n;
    if rest >= 100 {
        total += below_twenty(rest / 100) + 2; // "hundred"
        rest %= 100;
    }
    if rest >= 20 {
        total += tens_syllables(rest / 10);
        rest %= 10;
        if rest > 0 {
            total += below_twenty(rest);
        }
    } else if rest > 0 {
        total += below_twenty(rest);
    }
    total
}

/// Estimated spoken syllables of one word, at least 1. Numbers are expanded:
/// the integer part as read aloud ("₹5,200" = 6; a currency symbol adds
/// nothing), a decimal part as "point" plus one syllable per digit, `%` adds
/// two ("percent"). Mixed tokens ("3D") sum their digit and letter runs.
pub fn estimate_syllables(word: &str) -> usize {
    let mut total = 0;
    let mut letters = String::new();
    let mut digits = String::new();
    let mut frac = false;
    let mut frac_digits = 0usize;
    let flush_letters = |letters: &mut String, total: &mut usize| {
        if !letters.is_empty() {
            *total += letter_syllables(letters);
            letters.clear();
        }
    };
    let flush_digits = |digits: &mut String, total: &mut usize| {
        if !digits.is_empty() {
            *total += match digits.parse::<u64>() {
                Ok(n) => number_syllables(n),
                Err(_) => digits.len(),
            };
            digits.clear();
        }
    };
    for c in word.chars() {
        if c.is_ascii_digit() {
            flush_letters(&mut letters, &mut total);
            if frac {
                frac_digits += 1;
            } else {
                digits.push(c);
            }
        } else if c == ',' && !digits.is_empty() && !frac {
            // thousands grouping inside one number
        } else if c == '.' && !digits.is_empty() && !frac {
            frac = true;
            flush_digits(&mut digits, &mut total);
        } else if c.is_alphabetic() {
            if frac {
                total += 1 + frac_digits;
                frac = false;
                frac_digits = 0;
            }
            flush_digits(&mut digits, &mut total);
            letters.push(c);
        } else if c == '%' {
            flush_digits(&mut digits, &mut total);
            flush_letters(&mut letters, &mut total);
            total += 2;
        } else {
            flush_digits(&mut digits, &mut total);
            flush_letters(&mut letters, &mut total);
        }
    }
    flush_digits(&mut digits, &mut total);
    flush_letters(&mut letters, &mut total);
    if frac && frac_digits > 0 {
        total += 1 + frac_digits;
    }
    total.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start: f64, end: f64) -> SpeechWord {
        SpeechWord {
            text: text.to_string(),
            start,
            end,
            confidence: 0.7,
        }
    }

    fn map(words: Vec<SpeechWord>, sentences: Vec<(f64, f64)>) -> SpeechMap {
        SpeechMap {
            recognised: Vec::new(),
            alignment: None,
            version: SPEECH_VERSION.to_string(),
            audio: "a.wav".into(),
            sample_rate: 48_000,
            duration: 20.0,
            provider: "fixture".into(),
            model: "m".into(),
            voice: "v".into(),
            words,
            sentences: sentences
                .into_iter()
                .enumerate()
                .map(|(beat, (start, end))| SpeechSentence { beat, start, end })
                .collect(),
        }
    }

    fn st(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn makes_times_monotonic_and_clamped() {
        let m = map(
            vec![
                w("beta", 2.0, 1.5),
                w("alpha", 1.0, 1.6),
                w("gamma", 2.4, 9.0),
            ],
            vec![(1.0, 3.0)],
        );
        let (r, rep) = repair(&m, &st(&["alpha beta gamma"]));
        let t: Vec<&str> = r.words.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(t, ["alpha", "beta", "gamma"]);
        for p in r.words.windows(2) {
            assert!(p[1].start >= p[0].end - 1e-9, "{p:?}");
        }
        for x in &r.words {
            assert!(x.end >= x.start && x.start >= 1.0 && x.end <= 3.0 + 1e-9);
        }
        assert!(rep.reordered > 0);
    }

    #[test]
    fn enforces_min_word_without_overlap() {
        let m = map(
            vec![
                w("one", 1.0, 1.01),
                w("two", 1.5, 1.52),
                w("tri", 1.53, 1.9),
            ],
            vec![(1.0, 2.0)],
        );
        let (r, rep) = repair(&m, &st(&["one two tri"]));
        for x in &r.words {
            assert!(x.end - x.start >= MIN_WORD_SECONDS - 1e-9, "{x:?}");
        }
        for p in r.words.windows(2) {
            assert!(p[1].start >= p[0].end - 1e-9);
        }
        assert!(rep.stretched >= 2);
        // "one" borrowed from the gap before "two": its start is untouched.
        assert_eq!(r.words[0].start, 1.0);
        assert!(r.words[0].end <= 1.5 + 1e-9);
    }

    #[test]
    fn long_gaps_are_recorded_not_moved() {
        let m = map(
            vec![w("so", 1.0, 1.2), w("slow", 2.5, 2.9)],
            vec![(1.0, 3.0)],
        );
        let (r, rep) = repair(&m, &st(&["so slow"]));
        assert_eq!(rep.gaps_clamped, 1);
        assert_eq!(r.words[1].start, 2.5);
        assert_eq!(r.words[0].end, 1.2);
    }

    #[test]
    fn written_form_replaces_spoken_numbers() {
        let m = map(
            vec![
                w("save", 1.0, 1.3),
                w("five", 1.4, 1.6),
                w("thousand", 1.6, 1.9),
                w("two", 1.9, 2.0),
                w("hundred", 2.0, 2.3),
                w("rupees", 2.3, 2.7),
                w("today", 2.8, 3.2),
            ],
            vec![(1.0, 3.5)],
        );
        let (r, rep) = repair(&m, &st(&["Save ₹5,200 today!"]));
        let t: Vec<&str> = r.words.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(t, ["Save", "₹5,200", "today"]);
        let n = &r.words[1];
        assert_eq!((n.start, n.end), (1.4, 2.7));
        assert!(rep.unmatched_spoken_words.is_empty(), "{rep:?}");
        assert!(rep.unmatched_statement_words.is_empty(), "{rep:?}");
        // Percentages too.
        let m2 = map(
            vec![w("forty", 1.0, 1.3), w("percent", 1.3, 1.7)],
            vec![(1.0, 2.0)],
        );
        let (r2, _) = repair(&m2, &st(&["40%"]));
        assert_eq!(r2.words.len(), 1);
        assert_eq!(r2.words[0].text, "40%");
    }

    #[test]
    fn fuzzy_match_takes_the_written_form_and_unmatched_are_reported() {
        let m = map(
            vec![
                w("colour", 1.0, 1.3),
                w("um", 1.4, 1.6),
                w("bold", 1.7, 2.0),
            ],
            vec![(1.0, 2.1), (3.0, 4.0)],
        );
        let statements = st(&["Color bold now", "Second line"]);
        let (r, rep) = repair(&m, &statements);
        let t: Vec<&str> = r.words.iter().map(|x| x.text.as_str()).collect();
        // "colour" ~ "Color" (ratio 0.83); "um" kept as spoken; nothing invented.
        assert_eq!(t, ["Color", "um", "bold"]);
        assert_eq!(rep.unmatched_spoken_words, ["beat 1: um"]);
        assert_eq!(
            rep.unmatched_statement_words,
            ["beat 1: now", "beat 2: Second", "beat 2: line"]
        );
        assert_eq!(unmatched_words(&r, &statements).len(), 4);
    }

    #[test]
    fn sentences_are_sorted_and_clamped_and_repair_is_pure() {
        let mut m = map(vec![], vec![(5.0, 6.0), (1.0, 2.0)]);
        m.sentences[0].end = 99.0;
        let (r, _) = repair(&m, &[]);
        assert_eq!(r.sentences[0].start, 1.0);
        assert_eq!(r.sentences[1].end, 20.0);
        assert_eq!(repair(&m, &[]).0, r);
    }

    #[test]
    fn clean_map_is_unchanged() {
        let m = map(
            vec![w("Hello", 1.0, 1.4), w("world", 1.5, 2.0)],
            vec![(1.0, 2.0)],
        );
        let (r, rep) = repair(&m, &st(&["Hello world."]));
        assert_eq!(r, m);
        assert_eq!(rep, RepairReport::default());
    }
}
