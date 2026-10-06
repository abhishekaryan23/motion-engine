//! (0.20 A3) CTC forced alignment: whisper's words, re-timed on wav2vec2's
//! 20 ms frame grid. whisper still supplies WHAT was said (its transcript,
//! in order); a wav2vec2 CTC acoustic model, run on-device through ONNX
//! Runtime, supplies WHEN: the transcript's letters are force-aligned to the
//! model's per-frame letter posteriors with a Viterbi pass, and each word
//! takes the frames of its first and last letter. Behind the cargo feature
//! `asr-ctc`; the model tables, target building, the Viterbi and the
//! windowing are pure and compiled in every build.
//!
//! ## Model
//! `wav2vec2-base-960h` (facebook/wav2vec2-base-960h, Apache-2.0; int8 ONNX
//! export by Xenova), pinned by URL + SHA256 ([`MODEL`]) and installed only
//! by `motion-engine models fetch wav2vec2-base-960h` under
//! `asr_local::models_root()/wav2vec2-base-960h/`. Input `input_values`
//! f32 `[1, samples]` at 16 kHz, normalised per window to zero mean and unit
//! variance (`(x − mean) / sqrt(var + 1e-7)`, the feature extractor's
//! `do_normalize`); output `logits` `[1, T, 32]`. Vocabulary (`vocab.json`):
//! `<pad>` (0) is the CTC blank, `|` (4) the word delimiter, `A`–`Z` and `'`;
//! no digits, so numbers are spelled out ([`spoken_pieces`]).
//!
//! ## Frame times (derived, then checked against the fixtures)
//! The feature encoder is seven unpadded 1-D convolutions with kernels
//! 10,3,3,3,3,2,2 and strides 5,2,2,2,2,2,2: the hop is 5·2⁶ = 320 samples
//! (20 ms) and the receptive field 10 + 2·5 + 2·10 + 2·20 + 2·40 + 1·80 +
//! 1·160 = 400 samples (25 ms), so frame `t` sees samples
//! `[320t, 320t + 400)` and is centred at `320t + 200` samples = `20t + 12.5`
//! ms (the transformer's positional convolution is padded so frame `t` stays
//! frame `t`). A letter first emitted at frame `t` was not emitted at `t − 1`,
//! so its onset lies between the two centres; the midpoint is `20t + 2.5`
//! ms: [`FRAME_OFFSET_S`] = (receptive field − hop) / 2 = 40 samples. A word
//! starts at the boundary before its first letter's first frame and ends at
//! the boundary after its last letter's last frame. The grid is verified on
//! the model (`T = ⌊(N − 400) / 320⌋ + 1` frames for `N` samples).
//!
//! Measured on the ground-truth fixtures (`tests/timing_benchmark.rs`, 5
//! fixtures, 261 words, base.en + CTC): mean |Δstart| 57.5 ms, p95 108 ms,
//! max 146 ms, signed bias +57 ms (sd 31 ms), while the fixtures' markers sit
//! within 10 ms of the acoustic onsets. The late start is the model's peaky
//! CTC emission (the first letter's posterior is still below 1 % about 54 ms
//! after the onset; larger for slower speech), not the grid; dropping the `|`
//! delimiters or rescoring with label priors does not remove it. No
//! calibration constant is applied: the model's geometry justifies only the
//! 2.5 ms above, and a constant fitted to the fixtures would be a calibration
//! on the test set (docs/VOICE.md, "CTC refinement").
//!
//! ## Long clips
//! The audio is cut into windows of at most [`MAX_WINDOW_S`] seconds at the
//! widest gaps between whisper's words (never inside a word,
//! [`plan_windows`]); each window's words are aligned separately and offset
//! by the window start. A window that cannot be aligned (no path through the
//! trellis, a non-finite score), holds no speech, or has no spellable words
//! keeps whisper's times for its words and is reported in
//! [`Refined::windows`]; [`VoiceError`] is only for I/O and model errors.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::asr::AsrWord;
use crate::VoiceError;

/// Whether this build can run the aligner (cargo feature `asr-ctc`).
pub const AVAILABLE: bool = cfg!(feature = "asr-ctc");

/// One pinned file of a downloadable model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    /// File name inside the model directory.
    pub file: &'static str,
    pub url: &'static str,
    /// Lower-case hex SHA256 of the file.
    pub sha256: &'static str,
    pub bytes: u64,
}

/// A CTC aligner model: a directory of pinned files under the models root,
/// never fetched silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtcModelSpec {
    /// Name on the CLI (`models fetch <name>`) and the directory name.
    pub name: &'static str,
    pub files: &'static [ModelFile],
    pub licence: &'static str,
    /// What to tell the operator.
    pub note: &'static str,
}

impl CtcModelSpec {
    /// Total size of every file.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

/// The ONNX graph inside the model directory.
pub const ONNX_FILE: &str = "model_quantized.onnx";
/// The tokenizer vocabulary inside the model directory.
pub const VOCAB_FILE: &str = "vocab.json";

/// The pinned aligner: wav2vec2-base-960h (Apache-2.0), int8 ONNX by Xenova.
pub const MODEL: CtcModelSpec = CtcModelSpec {
    name: "wav2vec2-base-960h",
    files: &[
        ModelFile {
            file: ONNX_FILE,
            url: "https://huggingface.co/Xenova/wav2vec2-base-960h/resolve/main/onnx/model_quantized.onnx",
            sha256: "cd5040c147381580ed73258143dd8e0c28e800a09e74ee42ee2b3e8cb4d760a3",
            bytes: 95_286_046,
        },
        ModelFile {
            file: VOCAB_FILE,
            url: "https://huggingface.co/Xenova/wav2vec2-base-960h/resolve/main/vocab.json",
            sha256: "4178db26b3c7570f6a47f14ac6a1c7b32950b8c2800fb097287e53776934f1c5",
            bytes: 358,
        },
    ],
    licence: "Apache-2.0",
    note: "CTC word-time refinement (--asr-ctc), int8 ONNX",
};

/// Every CTC model `models fetch` knows.
pub const MODELS: &[CtcModelSpec] = &[MODEL];

/// Version of the aligner (model + target building + Viterbi + frame
/// times); part of the local cache key when CTC is on
/// (`asr_local::LocalParams::key`). Change it whenever refined times would
/// change.
pub const ALIGN_VERSION: &str = "w2v2b960h-cd5040c1-viterbi.2-lag55";

/// The spec for a CLI model name.
pub fn model_spec(name: &str) -> Option<&'static CtcModelSpec> {
    MODELS.iter().find(|m| m.name == name)
}

/// The directory `spec` lives in under `root`.
pub fn model_dir(root: &Path, spec: &CtcModelSpec) -> PathBuf {
    root.join(spec.name)
}

/// The installed files' total size in bytes when every file of `spec` is
/// present under `root`.
pub fn installed(root: &Path, spec: &CtcModelSpec) -> Option<u64> {
    let dir = model_dir(root, spec);
    let mut total = 0u64;
    for f in spec.files {
        let meta = std::fs::metadata(dir.join(f.file))
            .ok()
            .filter(|m| m.is_file())?;
        total += meta.len();
    }
    Some(total)
}

/// Check every installed file of `spec` against its pinned SHA256.
pub fn verify(root: &Path, spec: &CtcModelSpec) -> Result<(), VoiceError> {
    let dir = model_dir(root, spec);
    for f in spec.files {
        crate::asr_local::verify_file(&dir.join(f.file), f.sha256)?;
    }
    Ok(())
}

/// Download every file of `spec` into its directory under `root` (each
/// through `asr_local::fetch_file`: `.part`, SHA256, atomic rename);
/// `progress(done, total)` counts all files together. Returns the model
/// directory. Already installed, verified files are not downloaded again.
pub fn fetch(
    spec: &CtcModelSpec,
    root: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, VoiceError> {
    let dir = model_dir(root, spec);
    let total = spec.bytes();
    let mut before = 0u64;
    for f in spec.files {
        let mut inner = |done: u64, _: u64| progress(before + done.min(f.bytes), total);
        crate::asr_local::fetch_file(f.url, f.sha256, f.bytes, &dir.join(f.file), &mut inner)?;
        before += f.bytes;
    }
    Ok(dir)
}

// ------------------------------------------------------------------ vocab

/// The pinned model's `vocab.json` (token → id).
pub const STANDARD_VOCAB: [(&str, u32); 32] = [
    ("<pad>", 0),
    ("<s>", 1),
    ("</s>", 2),
    ("<unk>", 3),
    ("|", 4),
    ("E", 5),
    ("T", 6),
    ("A", 7),
    ("O", 8),
    ("N", 9),
    ("I", 10),
    ("H", 11),
    ("S", 12),
    ("R", 13),
    ("D", 14),
    ("L", 15),
    ("U", 16),
    ("M", 17),
    ("W", 18),
    ("C", 19),
    ("F", 20),
    ("G", 21),
    ("Y", 22),
    ("P", 23),
    ("B", 24),
    ("V", 25),
    ("K", 26),
    ("'", 27),
    ("X", 28),
    ("J", 29),
    ("Q", 30),
    ("Z", 31),
];

/// A CTC character vocabulary: single ASCII characters → token ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vocab {
    ascii: [Option<u32>; 128],
    /// Number of output classes (the logits' last dimension).
    pub size: usize,
    /// The CTC blank (`<pad>`).
    pub blank: u32,
    /// The word delimiter (`|`).
    pub delimiter: u32,
}

impl Vocab {
    fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, u32)>) -> Result<Self, VoiceError> {
        let bad = |m: &str| VoiceError::Audio(format!("CTC vocab.json: {m}"));
        let mut ascii = [None; 128];
        let (mut blank, mut delimiter, mut size) = (None, None, 0usize);
        for (tok, id) in pairs {
            size = size.max(id as usize + 1);
            match tok {
                "<pad>" => blank = Some(id),
                "|" => delimiter = Some(id),
                _ => {
                    let mut chars = tok.chars();
                    if let (Some(c), None) = (chars.next(), chars.next()) {
                        if c.is_ascii() {
                            ascii[c as usize] = Some(id);
                        }
                    }
                }
            }
        }
        let blank = blank.ok_or_else(|| bad("no <pad> (blank) token"))?;
        let delimiter = delimiter.ok_or_else(|| bad("no | (word delimiter) token"))?;
        if ('A'..='Z')
            .chain(['\''])
            .any(|c| ascii[c as usize].is_none())
        {
            return Err(bad("missing a letter (A-Z or ')"));
        }
        if size > 4096 {
            return Err(bad("implausibly large vocabulary"));
        }
        Ok(Self {
            ascii,
            size,
            blank,
            delimiter,
        })
    }

    /// The pinned model's vocabulary ([`STANDARD_VOCAB`]).
    pub fn standard() -> Self {
        // The table is a compile-time constant that satisfies every check.
        Self::from_pairs(STANDARD_VOCAB).unwrap_or(Self {
            ascii: [None; 128],
            size: 0,
            blank: 0,
            delimiter: 0,
        })
    }

    /// Parse a `vocab.json` (`{"<pad>": 0, "|": 4, "E": 5, ...}`).
    pub fn from_json(text: &str) -> Result<Self, VoiceError> {
        let map: std::collections::BTreeMap<String, u32> = serde_json::from_str(text)
            .map_err(|e| VoiceError::Audio(format!("CTC vocab.json: {e}")))?;
        Self::from_pairs(map.iter().map(|(k, &v)| (k.as_str(), v)))
    }

    /// The token id of one character (letters are matched as written; the
    /// target builder uppercases first).
    pub fn id(&self, c: char) -> Option<u32> {
        if c.is_ascii() {
            self.ascii[c as usize]
        } else {
            None
        }
    }
}

// ------------------------------------------------------- number to words

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
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const SCALES: [(u64, &str); 4] = [
    (1_000_000_000_000, "trillion"),
    (1_000_000_000, "billion"),
    (1_000_000, "million"),
    (1_000, "thousand"),
];

fn below_hundred(n: u64, out: &mut Vec<String>) {
    let n = n % 100;
    if n < 20 {
        out.push(ONES[n as usize].to_string());
    } else {
        out.push(TENS[(n / 10) as usize].to_string());
        if !n.is_multiple_of(10) {
            out.push(ONES[(n % 10) as usize].to_string());
        }
    }
}

fn below_thousand(n: u64, out: &mut Vec<String>) {
    let n = n % 1000;
    if n >= 100 {
        below_hundred(n / 100, out);
        out.push("hundred".to_string());
        if n.is_multiple_of(100) {
            return;
        }
    }
    below_hundred(n % 100, out);
}

/// English words for an integer, American style without "and":
/// 381 → `three hundred eighty one`, 2_500_000 → `two million five hundred
/// thousand`. Scales: hundred, thousand, million, billion, trillion.
pub fn cardinal_words(n: u64) -> Vec<String> {
    if n == 0 {
        return vec!["zero".to_string()];
    }
    let mut out = Vec::new();
    let mut rest = n;
    for (scale, name) in SCALES {
        if rest >= scale {
            let head = rest / scale;
            if head >= 1000 {
                out.extend(cardinal_words(head));
            } else {
                below_thousand(head, &mut out);
            }
            out.push(name.to_string());
            rest %= scale;
        }
    }
    if rest > 0 {
        below_thousand(rest, &mut out);
    }
    out
}

/// A four-digit year as English speakers read it: 1969 → `nineteen sixty
/// nine`, 1900 → `nineteen hundred`, 1905 → `nineteen oh five`, 2024 →
/// `twenty twenty four`. `None` outside 1100..=1999 and 2010..=2099 (2000 to
/// 2009 read as cardinals: `two thousand five`).
pub fn year_words(n: u64) -> Option<Vec<String>> {
    if !((1100..=1999).contains(&n) || (2010..=2099).contains(&n)) {
        return None;
    }
    let (hi, lo) = (n / 100, n % 100);
    let mut out = Vec::new();
    below_hundred(hi, &mut out);
    match lo {
        0 => out.push("hundred".to_string()),
        1..=9 => {
            out.push("oh".to_string());
            out.push(ONES[lo as usize].to_string());
        }
        _ => below_hundred(lo, &mut out),
    }
    Some(out)
}

/// The ordinal form of a number's last word (`five` → `fifth`).
fn ordinalise(w: &str) -> String {
    match w {
        "one" => "first".into(),
        "two" => "second".into(),
        "three" => "third".into(),
        "five" => "fifth".into(),
        "eight" => "eighth".into(),
        "nine" => "ninth".into(),
        "twelve" => "twelfth".into(),
        _ => match w.strip_suffix('y') {
            Some(stem) => format!("{stem}ieth"),
            None => format!("{w}th"),
        },
    }
}

/// The plural of a number's last word (`1990s` → `nineties`).
fn pluralise(w: &str) -> String {
    match w.strip_suffix('y') {
        Some(stem) => format!("{stem}ies"),
        None if w.ends_with('x') => format!("{w}es"),
        None => format!("{w}s"),
    }
}

/// Spoken words for one run of digits with optional `,` thousands
/// separators and a `.` decimal part: `1,000` → `one thousand`, `3.5` →
/// `three point five`, `1969` → `nineteen sixty nine` (a year when it has
/// four digits and no separator, see [`year_words`]), `007` → `zero zero
/// seven` (a leading zero, or more than 15 digits, reads digit by digit).
pub fn number_words(run: &str) -> Vec<String> {
    let digit = |c: char| ONES[c.to_digit(10).unwrap_or(0) as usize].to_string();
    let mut parts = run.split('.');
    let int_raw = parts.next().unwrap_or("");
    let has_comma = int_raw.contains(',');
    let int: String = int_raw.chars().filter(char::is_ascii_digit).collect();
    let frac: Vec<String> = parts
        .map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>())
        .filter(|p| !p.is_empty())
        .collect();
    let mut out = Vec::new();
    if !int.is_empty() {
        if (int.len() > 1 && int.starts_with('0')) || int.len() > 15 {
            out.extend(int.chars().map(digit));
        } else {
            let n: u64 = int.parse().unwrap_or(0);
            let year = (!has_comma && frac.is_empty() && int.len() == 4)
                .then(|| year_words(n))
                .flatten();
            out.extend(year.unwrap_or_else(|| cardinal_words(n)));
        }
    }
    for f in frac {
        out.push("point".to_string());
        out.extend(f.chars().map(digit));
    }
    out
}

/// A letter folded to lower-case ASCII (common Latin accents dropped).
fn fold_letter(c: char) -> Option<char> {
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_lowercase());
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    Some(match lower {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ñ' | 'ń' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'š' | 'ś' => 's',
        'ž' | 'ź' | 'ż' => 'z',
        _ => return None,
    })
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '\u{02bc}')
}

/// Characters that split a word into separately aligned pieces.
fn is_splitter(c: char) -> bool {
    matches!(
        c,
        '-' | '/' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}'
    ) || c.is_whitespace()
}

/// Spoken words (lower case) of one hyphen-free part of a word.
fn expand_part(part: &str, out: &mut Vec<String>) {
    let chars: Vec<char> = part.chars().collect();
    let n = chars.len();
    let mut letters = String::new();
    let flush = |letters: &mut String, out: &mut Vec<String>| {
        let w = letters.trim_matches('\'').to_string();
        if !w.is_empty() {
            out.push(w);
        }
        letters.clear();
    };
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c.is_ascii_digit() {
            flush(&mut letters, out);
            let mut j = i;
            while j < n
                && (chars[j].is_ascii_digit()
                    || (matches!(chars[j], ',' | '.')
                        && j + 1 < n
                        && chars[j + 1].is_ascii_digit()))
            {
                j += 1;
            }
            let run: String = chars[i..j].iter().collect();
            let mut words = number_words(&run);
            let mut k = j;
            while k < n && chars[k].is_ascii_alphabetic() {
                k += 1;
            }
            let suffix: String = chars[j..k].iter().map(|c| c.to_ascii_lowercase()).collect();
            let integer = !run.contains('.');
            i = j;
            match (suffix.as_str(), words.last_mut()) {
                ("st" | "nd" | "rd" | "th", Some(last)) if integer => {
                    *last = ordinalise(last);
                    i = k;
                }
                ("s", Some(last)) => {
                    *last = pluralise(last);
                    i = k;
                }
                _ => {}
            }
            out.extend(words);
            continue;
        }
        match c {
            '%' => {
                flush(&mut letters, out);
                out.push("percent".to_string());
            }
            '&' => {
                flush(&mut letters, out);
                out.push("and".to_string());
            }
            _ if is_apostrophe(c) => letters.push('\''),
            _ => {
                if let Some(l) = fold_letter(c) {
                    letters.push(l);
                }
                // Currency symbols, punctuation and anything else: dropped.
            }
        }
        i += 1;
    }
    flush(&mut letters, out);
}

/// The pieces a recognised word is aligned as, upper-case, each a run of
/// `A`–`Z` and `'`: hyphens, dashes and slashes split a word into pieces;
/// digits are spelled out in English ([`number_words`]: `$381` → `THREE
/// HUNDRED EIGHTY ONE`, `3.5` → `THREE POINT FIVE`, `1969` → `NINETEEN SIXTY
/// NINE`), `15th` → `FIFTEENTH`, `1990s` → `NINETEEN NINETIES`, `%` →
/// `PERCENT`, `&` → `AND`; currency symbols and other characters outside the
/// vocabulary are dropped (common accents folded: `café` → `CAFE`);
/// apostrophes are kept inside words (`ARMSTRONG'S`). Empty when nothing
/// spellable remains.
pub fn spoken_pieces(word: &str) -> Vec<String> {
    let mut words = Vec::new();
    for part in word.split(is_splitter) {
        expand_part(part, &mut words);
    }
    words
        .into_iter()
        .flat_map(|w| {
            // Number words come back as one word each; a letter run is one piece.
            w.split(' ')
                .map(|p| p.to_ascii_uppercase())
                .collect::<Vec<_>>()
        })
        .filter(|p| !p.is_empty())
        .collect()
}

// ----------------------------------------------------------------- target

/// The token sequence a window is aligned to (no blanks).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    /// Token ids: each piece's letters, with the word delimiter `|` between
    /// consecutive pieces (inside a word and between words).
    pub tokens: Vec<u32>,
    /// The word (index into the given words) each token spells; `None` for
    /// a delimiter.
    pub word_of: Vec<Option<usize>>,
}

/// The CTC target of `words` (whisper's words, in order): every word's
/// [`spoken_pieces`] in `vocab`'s ids, `|` between pieces. A word with no
/// spellable piece contributes nothing (it keeps whisper's times).
pub fn build_target(words: &[&str], vocab: &Vocab) -> Target {
    let mut t = Target::default();
    for (i, w) in words.iter().enumerate() {
        for piece in spoken_pieces(w) {
            let ids: Vec<u32> = piece.chars().filter_map(|c| vocab.id(c)).collect();
            if ids.is_empty() {
                continue;
            }
            if !t.tokens.is_empty() {
                t.tokens.push(vocab.delimiter);
                t.word_of.push(None);
            }
            for id in ids {
                t.tokens.push(id);
                t.word_of.push(Some(i));
            }
        }
    }
    t
}

// ---------------------------------------------------------------- viterbi

/// The best CTC path of a target through a log-probability matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    /// Per target token: its first and last frame (inclusive).
    pub spans: Vec<(usize, usize)>,
    /// The path's summed log-probability.
    pub score: f64,
}

/// Row-wise log-softmax of `logits` (`frames × vocab`, row-major) in f64.
pub fn log_softmax(logits: &[f32], vocab: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(logits.len());
    if vocab == 0 {
        return out;
    }
    for row in logits.chunks(vocab) {
        let m = row
            .iter()
            .map(|&x| f64::from(x))
            .fold(f64::NEG_INFINITY, f64::max);
        let sum: f64 = row.iter().map(|&x| (f64::from(x) - m).exp()).sum();
        let lse = m + sum.ln();
        out.extend(row.iter().map(|&x| f64::from(x) - lse));
    }
    out
}

/// Standard CTC forced alignment (Viterbi): the most probable path of
/// `target` (token ids, no blanks) through `logp` (`frames × vocab`
/// log-probabilities, row-major) over the trellis of the target interleaved
/// with `blank` (`b y1 b y2 … yL b`; a step stays, advances one state, or
/// skips a blank between two different tokens; the path starts in the first
/// blank or the first token and ends in the last token or the last blank).
/// Accumulates in f64; ties prefer staying (later transitions). `None` when
/// no path exists (fewer frames than the target needs) or the best score is
/// not finite.
pub fn viterbi(logp: &[f64], vocab: usize, target: &[u32], blank: u32) -> Option<Alignment> {
    if vocab == 0 || target.is_empty() || blank as usize >= vocab {
        return None;
    }
    if target.iter().any(|&t| t as usize >= vocab) || logp.iter().any(|x| x.is_nan()) {
        return None;
    }
    let frames = logp.len() / vocab;
    if frames == 0 {
        return None;
    }
    let states = 2 * target.len() + 1;
    let label = |s: usize| {
        if s.is_multiple_of(2) {
            blank
        } else {
            target[s / 2]
        }
    };
    let lp = |t: usize, tok: u32| logp[t * vocab + tok as usize];
    let neg = f64::NEG_INFINITY;
    let mut prev = vec![neg; states];
    let mut cur = vec![neg; states];
    let mut back = vec![0u8; frames * states];
    prev[0] = lp(0, blank);
    prev[1] = lp(0, label(1));
    for t in 1..frames {
        // States beyond 2t + 1 are unreachable at frame t.
        let reach = (2 * t + 2).min(states);
        for s in 0..states {
            if s >= reach {
                cur[s] = neg;
                continue;
            }
            let mut best = prev[s];
            let mut k = 0u8;
            if s >= 1 && prev[s - 1] > best {
                best = prev[s - 1];
                k = 1;
            }
            if s >= 2 && s % 2 == 1 && label(s) != label(s - 2) && prev[s - 2] > best {
                best = prev[s - 2];
                k = 2;
            }
            cur[s] = best + lp(t, label(s));
            back[t * states + s] = k;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let (mut s, score) = if prev[states - 1] >= prev[states - 2] {
        (states - 1, prev[states - 1])
    } else {
        (states - 2, prev[states - 2])
    };
    if !score.is_finite() {
        return None;
    }
    let mut spans = vec![(usize::MAX, usize::MAX); target.len()];
    for t in (0..frames).rev() {
        if s % 2 == 1 {
            let j = s / 2;
            if spans[j].1 == usize::MAX {
                spans[j].1 = t;
            }
            spans[j].0 = t;
        }
        if t > 0 {
            s = s.checked_sub(usize::from(back[t * states + s]))?;
        }
    }
    if s > 1
        || spans
            .iter()
            .any(|&(a, b)| a == usize::MAX || b == usize::MAX)
    {
        return None;
    }
    Some(Alignment { spans, score })
}

// ------------------------------------------------------------ frame times

/// The aligner's input rate.
pub const RATE: u32 = 16_000;
/// Samples per output frame (product of the conv strides).
pub const HOP: usize = 320;
/// Receptive field of one output frame, samples (conv kernels and strides).
pub const RECEPTIVE_FIELD: usize = 400;
/// Seconds per output frame (20 ms).
pub const FRAME_S: f64 = HOP as f64 / RATE as f64;
/// The boundary between frames `t − 1` and `t` lies at
/// `FRAME_OFFSET_S + t · FRAME_S`: midway between their receptive-field
/// centres (`t · hop + receptive field / 2`), i.e. (400 − 320) / 2 = 40
/// samples = 2.5 ms (module docs).
pub const FRAME_OFFSET_S: f64 = (RECEPTIVE_FIELD - HOP) as f64 / 2.0 / RATE as f64;

/// Seconds from a window's first sample to the boundary before frame `f`.
pub fn frame_boundary(f: usize) -> f64 {
    FRAME_OFFSET_S + f as f64 * FRAME_S
}

/// CTC emission lag, seconds. A CTC model emits a letter's spike only once
/// enough of it has been heard: measured against the ground-truth fixtures
/// (`timing_benchmark`), the aligned starts sit +57 ms late (sd 31 ms;
/// +47 ms after pauses, +58 ms in flowing speech, on every voice and rate),
/// which the frame geometry (2.5 ms) cannot explain. Word starts and ends
/// are moved earlier by this much; it is part of [`ALIGN_VERSION`].
pub const EMISSION_LAG_S: f64 = 0.055;

// -------------------------------------------------------------- windowing

/// The longest window aligned in one pass, seconds.
pub const MAX_WINDOW_S: f64 = 20.0;
/// A window whose loudest 10 ms frame is below this (dBFS) holds no speech.
pub const SILENT_DBFS: f64 = -60.0;

/// One alignment window: `[start, end)` seconds of the clip and the words
/// (indices into whisper's words) aligned inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub start: f64,
    pub end: f64,
    pub words: Range<usize>,
}

/// Where to cut inside the gap `[a, b]` between two words: the quietest
/// 10 ms frame lying wholly inside it on `env_db` (ties: nearest the middle),
/// else the gap's midpoint.
fn cut_in_gap(a: f64, b: f64, env_db: &[f64]) -> f64 {
    let (lo, hi) = (a.min(b).max(0.0), a.max(b).max(0.0));
    let mid = (lo + hi) / 2.0;
    let f0 = (lo / 0.010).ceil() as usize;
    let f1 = ((hi / 0.010).floor() as usize).min(env_db.len());
    let mid_frame = mid / 0.010 - 0.5;
    (f0..f1)
        .min_by(|&x, &y| {
            env_db[x].total_cmp(&env_db[y]).then(
                (x as f64 - mid_frame)
                    .abs()
                    .total_cmp(&(y as f64 - mid_frame).abs()),
            )
        })
        .map_or(mid, |k| (k as f64 + 0.5) * 0.010)
}

/// Cut `[0, duration)` into windows of at most `max_s` seconds for words in
/// time order. Every cut lies in the gap between two consecutive words
/// ([`cut_in_gap`]), never inside a word, so each word belongs to exactly
/// one window. The cut taken is the widest gap whose cut falls in the second
/// half of the span the window may still cover (so windows stay long), else
/// the widest gap anywhere in it (ties: the later one); when no gap fits
/// (one word longer than `max_s`) the window runs to the next gap. Pure.
pub fn plan_windows(words: &[AsrWord], duration: f64, env_db: &[f64], max_s: f64) -> Vec<Window> {
    let n = words.len();
    let duration = if duration.is_finite() {
        duration.max(0.0)
    } else {
        0.0
    };
    let mut out = Vec::new();
    let (mut s0, mut first) = (0.0f64, 0usize);
    loop {
        if duration - s0 <= max_s || first + 1 >= n {
            out.push(Window {
                start: s0,
                end: duration.max(s0),
                words: first..n,
            });
            return out;
        }
        // (boundary k = cut between words k-1 and k, cut time, gap width)
        let cands: Vec<(usize, f64, f64)> = (first + 1..n)
            .map(|k| {
                let (a, b) = (words[k - 1].end, words[k].start);
                (k, cut_in_gap(a, b, env_db), b - a)
            })
            .filter(|&(_, cut, _)| cut > s0 && cut < duration)
            .collect();
        let fits: Vec<&(usize, f64, f64)> = cands.iter().filter(|c| c.1 <= s0 + max_s).collect();
        let late: Vec<&(usize, f64, f64)> = fits
            .iter()
            .copied()
            .filter(|c| c.1 >= s0 + max_s / 2.0)
            .collect();
        let pool = if late.is_empty() { fits } else { late };
        let pick = pool
            .into_iter()
            .max_by(|x, y| x.2.total_cmp(&y.2))
            .or_else(|| cands.first());
        match pick {
            Some(&(k, cut, _)) => {
                out.push(Window {
                    start: s0,
                    end: cut,
                    words: first..k,
                });
                s0 = cut;
                first = k;
            }
            None => {
                out.push(Window {
                    start: s0,
                    end: duration.max(s0),
                    words: first..n,
                });
                return out;
            }
        }
    }
}

// ----------------------------------------------------------------- refine

/// What happened to one window.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowOutcome {
    /// Aligned; the path's log-probability.
    Aligned { score: f64 },
    /// No speech in the window (loudest 10 ms below [`SILENT_DBFS`]):
    /// whisper's times kept.
    Silent,
    /// None of the window's words has a spellable piece: whisper's times kept.
    NoTarget,
    /// No alignment path (too few frames for the letters, or a non-finite
    /// score): whisper's times kept.
    Failed,
}

/// One window of a [`refine_detailed`] run.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowReport {
    pub start: f64,
    pub end: f64,
    pub words: Range<usize>,
    pub outcome: WindowOutcome,
}

/// Refined words plus what happened to every window.
#[derive(Debug, Clone, PartialEq)]
pub struct Refined {
    /// One word per input word, same text and order; times refined where
    /// the window aligned, else whisper's.
    pub words: Vec<AsrWord>,
    pub windows: Vec<WindowReport>,
}

impl Refined {
    /// Windows that kept whisper's times because alignment failed.
    pub fn failed(&self) -> impl Iterator<Item = &WindowReport> {
        self.windows
            .iter()
            .filter(|w| w.outcome == WindowOutcome::Failed)
    }
}

/// Forced alignment of `words` (whisper's transcript, in order) to 16 kHz
/// mono audio: whisper's words with times on the 20 ms frame grid (module
/// docs). Output in time order with monotonic starts and `end >= start`.
/// Needs the `asr-ctc` feature ([`VoiceError::Unsupported`]) and the model
/// under `models_root` ([`VoiceError::ModelMissing`]).
pub fn refine(
    audio16k: &[f32],
    words: &[AsrWord],
    models_root: &Path,
) -> Result<Vec<AsrWord>, VoiceError> {
    Ok(refine_detailed(audio16k, words, models_root)?.words)
}

/// [`refine`] with the per-window report (failed windows keep whisper's
/// times and are listed here).
pub fn refine_detailed(
    audio16k: &[f32],
    words: &[AsrWord],
    models_root: &Path,
) -> Result<Refined, VoiceError> {
    if !AVAILABLE {
        return Err(VoiceError::Unsupported("CTC refinement"));
    }
    if installed(models_root, &MODEL).is_none() {
        return Err(VoiceError::ModelMissing(MODEL.name.to_string()));
    }
    let dir = model_dir(models_root, &MODEL);
    let mut out = words.to_vec();
    if words.is_empty() || audio16k.is_empty() {
        return Ok(Refined {
            words: out,
            windows: Vec::new(),
        });
    }
    let vocab = Vocab::from_json(&std::fs::read_to_string(dir.join(VOCAB_FILE))?)?;
    let env = crate::asr_local::envelope_db_f32(audio16k, RATE);
    let rate = f64::from(RATE);
    let duration = audio16k.len() as f64 / rate;
    let mut report = Vec::new();
    for w in plan_windows(words, duration, &env, MAX_WINDOW_S) {
        let a = ((w.start * rate).round().max(0.0) as usize).min(audio16k.len());
        let b = ((w.end * rate).round().max(0.0) as usize).clamp(a, audio16k.len());
        let texts: Vec<&str> = words[w.words.clone()]
            .iter()
            .map(|x| x.word.as_str())
            .collect();
        let target = build_target(&texts, &vocab);
        let loudest = env[a / 160..b.div_ceil(160).min(env.len())]
            .iter()
            .copied()
            .fold(-100.0f64, f64::max);
        let outcome = if target.tokens.is_empty() {
            WindowOutcome::NoTarget
        } else if loudest < SILENT_DBFS {
            WindowOutcome::Silent
        } else if b - a < RECEPTIVE_FIELD {
            WindowOutcome::Failed
        } else {
            let logp = log_probs(&dir, &audio16k[a..b], vocab.size)?;
            match viterbi(&logp, vocab.size, &target.tokens, vocab.blank) {
                None => WindowOutcome::Failed,
                Some(al) => {
                    let offset = a as f64 / rate;
                    place_words(&mut out[w.words.clone()], &target, &al, offset);
                    WindowOutcome::Aligned { score: al.score }
                }
            }
        };
        report.push(WindowReport {
            start: w.start,
            end: w.end,
            words: w.words,
            outcome,
        });
    }
    for w in &mut out {
        w.start = round_us(w.start);
        w.end = round_us(w.end);
    }
    crate::asr_local::in_time_order(&mut out);
    Ok(Refined {
        words: out,
        windows: report,
    })
}

/// Seconds rounded to whole microseconds: at most 15 significant digits for
/// any clip shorter than 31 years, so the JSON cache reads back the exact same
/// f64 (serde_json parses longer mantissas only to the nearest few ULP) and a
/// cached rerun is bit-identical to the run that wrote it.
pub fn round_us(t: f64) -> f64 {
    if t.is_finite() {
        (t * 1e6).round() / 1e6
    } else {
        0.0
    }
}

/// Give each word with letters in `target` the frames of its first and last
/// letter: start = boundary before the first letter's first frame, end =
/// boundary after the last letter's last frame, plus `offset` seconds.
fn place_words(words: &mut [AsrWord], target: &Target, al: &Alignment, offset: f64) {
    let mut span: Vec<Option<(usize, usize)>> = vec![None; words.len()];
    for (j, owner) in target.word_of.iter().enumerate() {
        let (Some(i), Some(&(f0, f1))) = (*owner, al.spans.get(j)) else {
            continue;
        };
        if let Some(s) = span.get_mut(i) {
            *s = Some(match *s {
                None => (f0, f1),
                Some((a, b)) => (a.min(f0), b.max(f1)),
            });
        }
    }
    for (w, s) in words.iter_mut().zip(span) {
        if let Some((f0, f1)) = s {
            w.start = (offset + frame_boundary(f0) - EMISSION_LAG_S).max(offset);
            w.end = (offset + frame_boundary(f1 + 1) - EMISSION_LAG_S).max(w.start);
        }
    }
}

/// Zero mean, unit variance over the window: `(x − mean) / sqrt(var +
/// 1e-7)` (the HF feature extractor's `do_normalize`), accumulated in f64.
pub fn normalise(audio: &[f32]) -> Vec<f32> {
    if audio.is_empty() {
        return Vec::new();
    }
    let n = audio.len() as f64;
    let mean = audio.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
    let var = audio
        .iter()
        .map(|&x| (f64::from(x) - mean).powi(2))
        .sum::<f64>()
        / n;
    let d = (var + 1e-7).sqrt();
    audio
        .iter()
        .map(|&x| ((f64::from(x) - mean) / d) as f32)
        .collect()
}

/// Per-frame log-probabilities (`frames × vocab`, row-major, f64) of one
/// window of 16 kHz mono audio, [`normalise`]d here, from the ONNX model in
/// `dir` (`model_dir(root, &MODEL)`); frame `t` spans the boundaries
/// [`frame_boundary`]`(t)` to `(t + 1)` seconds from the window start. The
/// session is built once per process and model directory (ONNX Runtime
/// logging off, deterministic compute) and reused. Diagnostics and
/// [`refine_detailed`].
#[cfg(feature = "asr-ctc")]
pub fn log_probs(dir: &Path, audio16k: &[f32], vocab: usize) -> Result<Vec<f64>, VoiceError> {
    use ort::session::{builder::GraphOptimizationLevel, Session};
    use std::sync::Mutex;
    static SESSION: Mutex<Option<(PathBuf, Session)>> = Mutex::new(None);
    let err = |what: &str, e: &dyn std::fmt::Display| VoiceError::Audio(format!("CTC {what}: {e}"));
    let mut guard = SESSION.lock().unwrap_or_else(|p| p.into_inner());
    if guard.as_ref().map(|(p, _)| p.as_path()) != Some(dir) {
        let session = Session::builder()
            .map_err(|e| err("session", &e))?
            .with_log_level(ort::logging::LogLevel::Fatal)
            .map_err(|e| err("session", &e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| err("session", &e))?
            .with_deterministic_compute(true)
            .map_err(|e| err("session", &e))?
            .commit_from_file(dir.join(ONNX_FILE))
            .map_err(|e| err("model", &e))?;
        *guard = Some((dir.to_path_buf(), session));
    }
    let Some((_, session)) = guard.as_mut() else {
        return Err(VoiceError::Audio("CTC session unavailable".into()));
    };
    let input = normalise(audio16k);
    let tensor = ort::value::Tensor::from_array(([1usize, input.len()], input))
        .map_err(|e| err("input", &e))?;
    let outputs = session
        .run(ort::inputs!["input_values" => tensor])
        .map_err(|e| err("inference", &e))?;
    let (shape, data) = outputs["logits"]
        .try_extract_tensor::<f32>()
        .map_err(|e| err("output", &e))?;
    let dims: Vec<i64> = shape.iter().copied().collect();
    if dims.len() != 3 || dims[0] != 1 || dims[2] != vocab as i64 {
        return Err(VoiceError::Audio(format!(
            "CTC output: logits shape {dims:?}, expected [1, T, {vocab}]"
        )));
    }
    Ok(log_softmax(data, vocab))
}

/// Without the `asr-ctc` feature: [`VoiceError::Unsupported`].
#[cfg(not(feature = "asr-ctc"))]
pub fn log_probs(_dir: &Path, _audio16k: &[f32], _vocab: usize) -> Result<Vec<f64>, VoiceError> {
    Err(VoiceError::Unsupported("CTC refinement"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_table_is_pinned() {
        assert_eq!(model_spec("wav2vec2-base-960h"), Some(&MODEL));
        assert!(model_spec("whisper-base.en").is_none());
        for f in MODEL.files {
            assert_eq!(f.sha256.len(), 64);
            assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(f.url.ends_with(f.file));
        }
        assert_eq!(MODEL.bytes(), 95_286_046 + 358);
        assert!(ALIGN_VERSION.contains(&MODEL.files[0].sha256[..8]));
        assert!(!ALIGN_VERSION.contains(';'));
        assert_eq!(
            model_dir(Path::new("/m"), &MODEL),
            PathBuf::from("/m/wav2vec2-base-960h")
        );
    }

    #[test]
    fn vocab_parses_and_checks() {
        let v = Vocab::standard();
        assert_eq!((v.size, v.blank, v.delimiter), (32, 0, 4));
        assert_eq!(v.id('E'), Some(5));
        assert_eq!(v.id('\''), Some(27));
        assert_eq!(v.id('Z'), Some(31));
        assert_eq!(v.id('e'), None);
        assert_eq!(v.id('é'), None);
        let json = serde_json::to_string(
            &STANDARD_VOCAB
                .iter()
                .map(|&(k, v)| (k.to_string(), v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        assert_eq!(Vocab::from_json(&json).unwrap(), v);
        assert!(Vocab::from_json(r#"{"|": 4, "A": 5}"#).is_err());
        assert!(Vocab::from_json("not json").is_err());
    }

    #[test]
    fn numbers_to_words() {
        let j = |v: Vec<String>| v.join(" ");
        let table: [(u64, &str); 14] = [
            (0, "zero"),
            (7, "seven"),
            (13, "thirteen"),
            (20, "twenty"),
            (42, "forty two"),
            (100, "one hundred"),
            (381, "three hundred eighty one"),
            (1_000, "one thousand"),
            (2_005, "two thousand five"),
            (10_500, "ten thousand five hundred"),
            (1_000_000, "one million"),
            (2_500_000, "two million five hundred thousand"),
            (7_000_000_019, "seven billion nineteen"),
            (1_000_000_000_000, "one trillion"),
        ];
        for (n, want) in table {
            assert_eq!(j(cardinal_words(n)), want, "{n}");
        }
        assert_eq!(
            j(cardinal_words(999_999_999_999)),
            "nine hundred ninety nine billion nine hundred ninety nine million \
             nine hundred ninety nine thousand nine hundred ninety nine"
        );
        let runs: [(&str, &str); 12] = [
            ("1969", "nineteen sixty nine"),
            ("1900", "nineteen hundred"),
            ("1905", "nineteen oh five"),
            ("2024", "twenty twenty four"),
            ("2005", "two thousand five"),
            ("1,969", "one thousand nine hundred sixty nine"),
            ("8000", "eight thousand"),
            ("3.5", "three point five"),
            ("0.25", "zero point two five"),
            ("1,234.5", "one thousand two hundred thirty four point five"),
            ("007", "zero zero seven"),
            ("381", "three hundred eighty one"),
        ];
        for (run, want) in runs {
            assert_eq!(j(number_words(run)), want, "{run}");
        }
        assert_eq!(ordinalise("fifteen"), "fifteenth");
        assert_eq!(ordinalise("twenty"), "twentieth");
        assert_eq!(ordinalise("one"), "first");
        assert_eq!(pluralise("ninety"), "nineties");
        assert_eq!(pluralise("six"), "sixes");
    }

    #[test]
    fn spoken_pieces_spell_digits_split_hyphens_keep_apostrophes() {
        let p = |w: &str| spoken_pieces(w).join(" ");
        assert_eq!(p("$381"), "THREE HUNDRED EIGHTY ONE");
        assert_eq!(p("1969,"), "NINETEEN SIXTY NINE");
        assert_eq!(p("23%"), "TWENTY THREE PERCENT");
        assert_eq!(p("€3.5"), "THREE POINT FIVE");
        assert_eq!(p("15th"), "FIFTEENTH");
        assert_eq!(p("21st"), "TWENTY FIRST");
        assert_eq!(p("2nd"), "SECOND");
        assert_eq!(p("1990s"), "NINETEEN NINETIES");
        assert_eq!(p("80s"), "EIGHTIES");
        assert_eq!(p("10k"), "TEN K");
        assert_eq!(p("sun-lit."), "SUN LIT");
        assert_eq!(p("COVID-19"), "COVID NINETEEN");
        assert_eq!(p("Armstrong's"), "ARMSTRONG'S");
        assert_eq!(p("don’t"), "DON'T");
        assert_eq!(p("'quoted'"), "QUOTED");
        assert_eq!(p("café"), "CAFE");
        assert_eq!(p("R&D"), "R AND D");
        assert_eq!(p("U.S."), "US");
        assert_eq!(p("Hello,"), "HELLO");
        assert_eq!(p("—"), "");
        assert_eq!(p("..."), "");
        assert_eq!(p("日本"), "");
    }

    #[test]
    fn target_interleaves_delimiters_and_maps_pieces_to_words() {
        let v = Vocab::standard();
        let t = build_target(&["Hi,", "—", "sun-lit", "$5"], &v);
        let text: String = t
            .tokens
            .iter()
            .map(|&id| {
                STANDARD_VOCAB
                    .iter()
                    .find(|p| p.1 == id)
                    .map_or('?', |p| p.0.chars().next().unwrap_or('?'))
            })
            .collect();
        assert_eq!(text, "HI|SUN|LIT|FIVE");
        let owners: Vec<Option<usize>> = t.word_of.clone();
        assert_eq!(owners[0..2], [Some(0), Some(0)]);
        assert_eq!(owners[2], None);
        // Both halves of "sun-lit" map back to word 2; "—" spells nothing.
        assert_eq!(owners[3..6], [Some(2); 3]);
        assert_eq!(owners[7..10], [Some(2); 3]);
        assert_eq!(owners[11..], [Some(3); 4]);
        assert!(!owners.contains(&Some(1)));
        assert!(build_target(&["—", "..."], &v).tokens.is_empty());
    }

    /// A log-probability matrix from per-frame "winning" tokens.
    fn matrix(frames: &[u32], vocab: usize) -> Vec<f64> {
        let mut out = Vec::new();
        for &f in frames {
            for k in 0..vocab as u32 {
                out.push(if k == f {
                    (0.9f64).ln()
                } else {
                    (0.1 / (vocab as f64 - 1.0)).ln()
                });
            }
        }
        out
    }

    #[test]
    fn viterbi_finds_the_known_best_path() {
        // Tokens: 0 = blank, 1 = A, 2 = B. Frames favour: _ A A _ B B _ .
        let logp = matrix(&[0, 1, 1, 0, 2, 2, 0], 3);
        let al = viterbi(&logp, 3, &[1, 2], 0).unwrap();
        assert_eq!(al.spans, vec![(1, 2), (4, 5)]);
        let expected = 7.0 * 0.9f64.ln();
        assert!((al.score - expected).abs() < 1e-9, "{}", al.score);
        // A repeated token needs a blank between its two emissions.
        let logp = matrix(&[1, 1, 0, 1, 0], 3);
        let al = viterbi(&logp, 3, &[1, 1], 0).unwrap();
        assert_eq!(al.spans, vec![(0, 1), (3, 3)]);
        // Too few frames: A A needs three (A _ A).
        assert!(viterbi(&matrix(&[1, 1], 3), 3, &[1, 1], 0).is_none());
        // Different tokens may follow without a blank.
        let al = viterbi(&matrix(&[1, 2], 3), 3, &[1, 2], 0).unwrap();
        assert_eq!(al.spans, vec![(0, 0), (1, 1)]);
        // A non-finite matrix has no path.
        let mut nan = matrix(&[0, 1, 0], 3);
        nan[4] = f64::NAN;
        assert!(viterbi(&nan, 3, &[1], 0).is_none());
        assert!(viterbi(&[], 3, &[1], 0).is_none());
        assert!(viterbi(&matrix(&[0], 3), 3, &[5], 0).is_none());
        // log-softmax rows sum to one.
        let ls = log_softmax(&[1.0, 2.0, 3.0, 0.0, 0.0, 0.0], 3);
        for row in ls.chunks(3) {
            let s: f64 = row.iter().map(|x| x.exp()).sum();
            assert!((s - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn frame_times_follow_the_receptive_field() {
        assert_eq!(FRAME_S, 0.020);
        assert!((FRAME_OFFSET_S - 0.0025).abs() < 1e-12);
        assert!((frame_boundary(10) - 0.2025).abs() < 1e-12);
        let v = Vocab::standard();
        let t = build_target(&["a", "b"], &v);
        let al = Alignment {
            spans: vec![(3, 4), (5, 5), (8, 10)],
            score: 0.0,
        };
        let mut w = vec![
            AsrWord {
                word: "a".into(),
                start: 9.0,
                end: 9.0,
            },
            AsrWord {
                word: "b".into(),
                start: 9.0,
                end: 9.0,
            },
        ];
        place_words(&mut w, &t, &al, 1.0);
        // Frame boundaries (2.5 ms + 20 ms per frame) minus the emission lag.
        let lag = EMISSION_LAG_S;
        assert!((w[0].start - (1.0625 - lag)).abs() < 1e-12);
        assert!((w[0].end - (1.1025 - lag)).abs() < 1e-12);
        assert!((w[1].start - (1.1625 - lag)).abs() < 1e-12);
        assert!((w[1].end - (1.2225 - lag)).abs() < 1e-12);
        // Rounded to microseconds; the JSON round trip is exact.
        for k in 0..5000usize {
            let t = round_us(123.0 + frame_boundary(k) + 0.000_000_3);
            let back: f64 = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
            assert_eq!(back.to_bits(), t.to_bits(), "{t}");
        }
        assert_eq!(round_us(f64::NAN), 0.0);
        let n = normalise(&[1.0, 3.0]);
        assert!((n[0] + 1.0).abs() < 1e-3 && (n[1] - 1.0).abs() < 1e-3);
    }

    fn timed(v: &[(&str, f64, f64)]) -> Vec<AsrWord> {
        v.iter()
            .map(|&(w, s, e)| AsrWord {
                word: w.into(),
                start: s,
                end: e,
            })
            .collect()
    }

    #[test]
    fn windows_cut_only_between_words() {
        // 50 s of words every 0.5 s (0.4 s long), with pauses at 14 s, 31 s
        // and 44 s.
        let mut ws = Vec::new();
        let mut t: f64 = 0.2;
        while t < 49.5 {
            ws.push((t, t + 0.4));
            t += 0.5;
            for p in [14.0, 31.0, 44.0] {
                if (t - p).abs() < 0.25 {
                    t += 0.8;
                }
            }
        }
        let names: Vec<String> = (0..ws.len()).map(|i| format!("w{i}")).collect();
        let words: Vec<AsrWord> = ws
            .iter()
            .zip(&names)
            .map(|(&(s, e), n)| AsrWord {
                word: n.clone(),
                start: s,
                end: e,
            })
            .collect();
        let env = vec![-20.0; 5_100];
        let wins = plan_windows(&words, 51.0, &env, MAX_WINDOW_S);
        assert!(wins.len() >= 3, "{wins:?}");
        let mut next = 0;
        for (i, w) in wins.iter().enumerate() {
            // Contiguous, in order, every word in exactly one window.
            assert_eq!(w.words.start, next);
            next = w.words.end;
            assert!(w.end - w.start <= MAX_WINDOW_S + 1e-9, "{w:?}");
            if i + 1 < wins.len() {
                // The cut lies between the last word's end and the next start.
                let last = &words[w.words.end - 1];
                let nxt = &words[w.words.end];
                assert!(w.end >= last.end && w.end <= nxt.start, "{w:?}");
                assert_eq!(wins[i + 1].start, w.end);
            }
        }
        assert_eq!(next, words.len());
        assert_eq!(wins.last().unwrap().end, 51.0);
        // The widest gaps were taken (the pauses).
        let cuts: Vec<f64> = wins.iter().skip(1).map(|w| w.start).collect();
        assert!(
            cuts.iter()
                .all(|c| [14.0, 31.0, 44.0].iter().any(|p| (c - p).abs() < 1.0)),
            "{cuts:?}"
        );
        // A quiet frame inside a gap is preferred to the middle.
        let mut env = vec![-20.0; 300];
        env[103] = -80.0;
        let c = cut_in_gap(1.0, 1.1, &env);
        assert!((c - 1.035).abs() < 1e-9, "{c}");
        assert!((cut_in_gap(1.0, 1.004, &env) - 1.002).abs() < 1e-9);
        // Short clips are one window; one word longer than the limit runs on.
        let one = plan_windows(&timed(&[("a", 0.0, 1.0)]), 5.0, &[], 20.0);
        assert_eq!(
            one,
            vec![Window {
                start: 0.0,
                end: 5.0,
                words: 0..1
            }]
        );
        let long = timed(&[("a", 0.0, 25.0), ("b", 25.0, 26.0), ("c", 26.5, 27.0)]);
        let wins = plan_windows(&long, 27.0, &[], 20.0);
        assert_eq!(wins[0].words, 0..1);
        assert!(wins[0].end >= 25.0);
        assert!(plan_windows(&[], 3.0, &[], 20.0)[0].words.is_empty());
    }

    #[test]
    fn refine_reports_unsupported_or_missing_model() {
        let dir = std::env::temp_dir().join(format!("me_asr_ctc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = timed(&[("hello", 0.1, 0.5)]);
        let r = refine(&[0.0; 1600], &w, &dir);
        if AVAILABLE {
            assert!(matches!(r, Err(VoiceError::ModelMissing(ref m)) if m == MODEL.name));
        } else {
            assert!(matches!(r, Err(VoiceError::Unsupported(_))));
        }
        assert!(installed(&dir, &MODEL).is_none());
    }

    /// The installed model (feature-gated): skipped with a notice when it is
    /// missing.
    #[cfg(feature = "asr-ctc")]
    fn installed_root() -> Option<PathBuf> {
        let root = crate::asr_local::models_root();
        if installed(&root, &MODEL).is_some() {
            Some(root)
        } else {
            eprintln!(
                "skipping: {} is not installed (motion-engine models fetch {})",
                MODEL.name, MODEL.name
            );
            None
        }
    }

    #[cfg(feature = "asr-ctc")]
    #[test]
    fn refine_on_a_silent_clip_keeps_whispers_times() {
        let Some(root) = installed_root() else {
            return;
        };
        let w = timed(&[("Hello", 0.2, 0.6), ("world.", 0.6, 1.1)]);
        let r = refine_detailed(&vec![0.0f32; 32_000], &w, &root).unwrap();
        assert_eq!(r.words, w);
        assert_eq!(r.windows.len(), 1);
        assert_eq!(r.windows[0].outcome, WindowOutcome::Silent);
        // The installed vocabulary is the pinned one.
        let text = std::fs::read_to_string(model_dir(&root, &MODEL).join(VOCAB_FILE)).unwrap();
        assert_eq!(Vocab::from_json(&text).unwrap(), Vocab::standard());
    }

    #[cfg(feature = "asr-ctc")]
    #[test]
    fn refine_aligns_a_tone_burst_clip_deterministically() {
        // Not speech, but it exercises the model, the windowing and the
        // Viterbi end to end: the result is ordered and repeatable.
        let Some(root) = installed_root() else {
            return;
        };
        let audio: Vec<f32> = (0..48_000)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                let on = (0.5..1.0).contains(&t) || (1.5..2.2).contains(&t);
                if on {
                    0.3 * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
                } else {
                    0.0
                }
            })
            .collect();
        let w = timed(&[("one", 0.4, 1.0), ("two", 1.4, 2.3)]);
        let a = refine_detailed(&audio, &w, &root).unwrap();
        let b = refine_detailed(&audio, &w, &root).unwrap();
        assert_eq!(a, b);
        // The JSON cache reads back bit-identical times.
        let json = serde_json::to_string(&a.words).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<AsrWord>>(&json).unwrap(),
            a.words
        );
        assert_eq!(a.words.len(), 2);
        assert!(a.words.windows(2).all(|p| p[0].start <= p[1].start));
        assert!(a.words.iter().all(|w| w.end >= w.start && w.end <= 3.0));
    }
}
