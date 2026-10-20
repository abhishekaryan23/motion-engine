//! Spoken forms -> written forms, deterministic and table-based: numbers
//! (English and Hindi/Hinglish), currency, percent, ordinals, dates, clock
//! times. Input is Roman text (run [`super::translit::to_roman`] first); each
//! output word keeps the span of raw words it came from, so timing and the
//! raw transcript stay recoverable. Entity detection runs on this output.

/// What a normalised word stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormKind {
    Kept,
    Number,
    Date,
    Time,
    Ordinal,
}

/// One output word; `from..=to` index the input words it replaces.
#[derive(Debug, Clone, PartialEq)]
pub struct NormWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub from: usize,
    pub to: usize,
    pub kind: NormKind,
}

// ---------------------------------------------------------------- tokens

#[derive(Debug, Clone)]
struct T {
    lead: String,
    /// Lower-case core without edge punctuation.
    core: String,
    trail: String,
    text: String,
    start: f64,
    end: f64,
    src: usize,
}

impl T {
    fn new(text: &str, start: f64, end: f64, src: usize) -> T {
        let text = text.trim();
        let keep = |c: char| c.is_alphanumeric() || "₹$€£%".contains(c);
        let a = text.find(keep).unwrap_or(text.len());
        let b = text
            .rfind(keep)
            .map(|i| i + text[i..].chars().next().map_or(1, char::len_utf8));
        let (lead, core, trail) = match b {
            Some(b) if a < b => (&text[..a], &text[a..b], &text[b..]),
            _ => (text, "", ""),
        };
        T {
            lead: lead.to_string(),
            core: core.to_lowercase(),
            trail: trail.to_string(),
            text: text.to_string(),
            start,
            end,
            src,
        }
    }
    /// A sentence or clause break after this token ends any phrase.
    fn breaks(&self) -> bool {
        self.trail
            .contains(['.', ',', '?', '!', ';', ':', '\u{2014}'])
    }
}

fn tokens(words: &[(String, f64, f64)]) -> Vec<T> {
    let mut out = Vec::new();
    for (i, (w, s, e)) in words.iter().enumerate() {
        let lw = w.trim().to_lowercase();
        // "Rs.500" / "rs500" -> "Rs" "500".
        if let Some(rest) = lw.strip_prefix("rs") {
            let rest = rest.trim_start_matches('.');
            if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                let cut = w.trim().len() - rest.len();
                out.push(T::new("Rs", *s, *s, i));
                out.push(T::new(&w.trim()[cut..], *s, *e, i));
                continue;
            }
        }
        out.push(T::new(w, *s, *e, i));
    }
    out
}

// ------------------------------------------------------------- lexicon

struct Small {
    value: f64,
    /// English tens (twenty..ninety): may take an English ones word.
    eng_tens: bool,
    english: bool,
    /// Also an ordinary word ("do", "ek", "saat", "das", "one"): never a
    /// number on its own.
    ambiguous: bool,
}

const ENGLISH: &[(&str, u32)] = &[
    ("zero", 0),
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

/// Hindi/Hinglish 0..99 (common spellings).
const HINDI: &[(&str, u32)] = &[
    ("shunya", 0),
    ("ek", 1),
    ("do", 2),
    ("teen", 3),
    ("chaar", 4),
    ("char", 4),
    ("paanch", 5),
    ("panch", 5),
    ("chhe", 6),
    ("chhah", 6),
    ("chheh", 6),
    ("saat", 7),
    ("aath", 8),
    ("nau", 9),
    ("das", 10),
    ("gyarah", 11),
    ("gyaarah", 11),
    ("barah", 12),
    ("baarah", 12),
    ("terah", 13),
    ("chaudah", 14),
    ("pandrah", 15),
    ("pandra", 15),
    ("solah", 16),
    ("sola", 16),
    ("satrah", 17),
    ("atharah", 18),
    ("athaarah", 18),
    ("unnis", 19),
    ("unnees", 19),
    ("bees", 20),
    ("ikkis", 21),
    ("ikkees", 21),
    ("baais", 22),
    ("bais", 22),
    ("teis", 23),
    ("tevis", 23),
    ("chaubis", 24),
    ("chaubees", 24),
    ("pachchis", 25),
    ("pachis", 25),
    ("pachees", 25),
    ("chhabbis", 26),
    ("chhabis", 26),
    ("sattais", 27),
    ("sattaees", 27),
    ("atthais", 28),
    ("athaees", 28),
    ("untis", 29),
    ("unatis", 29),
    ("tees", 30),
    ("tis", 30),
    ("ikatis", 31),
    ("ikattis", 31),
    ("battis", 32),
    ("tentis", 33),
    ("chauntis", 34),
    ("paintis", 35),
    ("chhattis", 36),
    ("saintis", 37),
    ("adtis", 38),
    ("untalis", 39),
    ("chalis", 40),
    ("chalees", 40),
    ("iktalis", 41),
    ("bayalis", 42),
    ("taitalis", 43),
    ("chauvalis", 44),
    ("paintalis", 45),
    ("chhiyalis", 46),
    ("saintalis", 47),
    ("adtalis", 48),
    ("unchas", 49),
    ("pachaas", 50),
    ("pachas", 50),
    ("ikyavan", 51),
    ("baavan", 52),
    ("tirpan", 53),
    ("chauvan", 54),
    ("pachpan", 55),
    ("chhappan", 56),
    ("sattavan", 57),
    ("atthavan", 58),
    ("unsath", 59),
    ("saath", 60),
    ("sath", 60),
    ("iksath", 61),
    ("baasath", 62),
    ("tirsath", 63),
    ("chausath", 64),
    ("painsath", 65),
    ("chhiyasath", 66),
    ("adsath", 67),
    ("athsath", 68),
    ("unhattar", 69),
    ("sattar", 70),
    ("ikhattar", 71),
    ("bahattar", 72),
    ("tihattar", 73),
    ("chauhattar", 74),
    ("pachhattar", 75),
    ("chhihattar", 76),
    ("satattar", 77),
    ("athattar", 78),
    ("unnasi", 79),
    ("assi", 80),
    ("ikyasi", 81),
    ("bayasi", 82),
    ("tirasi", 83),
    ("chaurasi", 84),
    ("pachasi", 85),
    ("chhiyasi", 86),
    ("sattasi", 87),
    ("atthasi", 88),
    ("nawasi", 89),
    ("nabbe", 90),
    ("ikyanve", 91),
    ("baanve", 92),
    ("tiranve", 93),
    ("chauranve", 94),
    ("pachanve", 95),
    ("chhiyanve", 96),
    ("sattanve", 97),
    ("atthanve", 98),
    ("ninyanve", 99),
];

/// Also ordinary words; they convert only inside a quantity or time.
const AMBIGUOUS: &[&str] = &[
    "one", "ek", "do", "saat", "das", "sath", "saath", "tis", "nau", "tees", "bees",
];

fn small(w: &str) -> Option<Small> {
    if let Some(&(_, v)) = ENGLISH.iter().find(|(n, _)| *n == w) {
        return Some(Small {
            value: f64::from(v),
            eng_tens: v >= 20,
            english: true,
            ambiguous: AMBIGUOUS.contains(&w),
        });
    }
    HINDI.iter().find(|(n, _)| *n == w).map(|&(n, v)| Small {
        value: f64::from(v),
        eng_tens: false,
        english: false,
        ambiguous: AMBIGUOUS.contains(&n),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Sc {
    Thousand,
    Lakh,
    Crore,
    Million,
    Billion,
    LakhCrore,
}

impl Sc {
    fn val(self) -> f64 {
        match self {
            Sc::Thousand => 1e3,
            Sc::Lakh => 1e5,
            Sc::Crore => 1e7,
            Sc::Million => 1e6,
            Sc::Billion => 1e9,
            Sc::LakhCrore => 1e12,
        }
    }
    /// The word kept in the output (thousands become digits).
    fn word(self) -> Option<&'static str> {
        match self {
            Sc::Thousand => None,
            Sc::Lakh => Some("lakh"),
            Sc::Crore => Some("crore"),
            Sc::Million => Some("million"),
            Sc::Billion => Some("billion"),
            Sc::LakhCrore => Some("lakh crore"),
        }
    }
}

fn scale_of(w: &str) -> Option<Sc> {
    Some(match w {
        "thousand" | "hazaar" | "hazar" | "hajar" | "hajaar" => Sc::Thousand,
        "lakh" | "lakhs" | "lac" | "lacs" => Sc::Lakh,
        "crore" | "crores" | "karod" | "karor" | "karodh" => Sc::Crore,
        "million" | "millions" => Sc::Million,
        "billion" | "billions" => Sc::Billion,
        _ => return None,
    })
}

fn is_hundred(w: &str) -> bool {
    matches!(w, "hundred" | "sau" | "so")
}

const RUPEE_AFTER: &[&str] = &["rupaye", "rupaya", "rupees", "rupee", "rs", "inr", "ruppe"];
const RUPEE_BEFORE: &[&str] = &["rs", "inr", "rupees", "rupee", "₹"];
const DOLLAR: &[&str] = &["dollar", "dollars", "usd", "bucks"];
const EURO: &[&str] = &["euro", "euros"];
const PERCENT: &[&str] = &["percent", "pratishat", "pratishath", "taka", "%", "fisadi"];
/// Units after a fractional quantity ("saade teen saal" is 3.5 years).
const QUANTITY_UNITS: &[&str] = &[
    "saal",
    "sal",
    "din",
    "mahine",
    "mahina",
    "ghante",
    "ghanta",
    "minute",
    "minutes",
    "hours",
    "years",
    "days",
    "months",
    "log",
    "baar",
    "times",
    "kilo",
    "km",
    "meter",
    "percent",
    "pratishat",
];

fn quantity_follows(t: &[T], k: usize) -> bool {
    t.get(k).is_some_and(|x| {
        scale_of(&x.core).is_some()
            || is_hundred(&x.core)
            || QUANTITY_UNITS.contains(&x.core.as_str())
            || RUPEE_AFTER.contains(&x.core.as_str())
            || PERCENT.contains(&x.core.as_str())
    })
}

// -------------------------------------------------------- number parse

fn digit_value(w: &str) -> Option<f64> {
    let ok = w.chars().next().is_some_and(|c| c.is_ascii_digit())
        && w.chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
        && w.chars().filter(|&c| c == '.').count() <= 1
        && !w.ends_with(',');
    if !ok {
        return None;
    }
    w.replace(',', "").parse::<f64>().ok()
}

#[derive(Debug, Clone)]
struct Num {
    value: f64,
    used: usize,
    last: Option<Sc>,
    /// Digits `total / last-scale` is a "nice" figure (<= 2 decimals).
    indian: bool,
    /// One numeral token, no other word.
    digit_only: bool,
    /// Every consumed word is ambiguous ("do", "ek").
    all_ambiguous: bool,
    /// Single English word below 10.
    english_small: bool,
    /// Value read from words (at least one number word).
    spelled: bool,
}

/// Decimal digits after "point": single digits (words or numerals) or one
/// numeral string. Returns (fraction, tokens used).
fn decimals(t: &[T], k: usize) -> Option<(f64, usize)> {
    let mut digits = String::new();
    let mut used = 0;
    while let Some(x) = t.get(k + used) {
        if let Some(s) = small(&x.core).filter(|s| s.value < 10.0) {
            digits.push_str(&format!("{}", s.value as u32));
        } else if x.core.chars().all(|c| c.is_ascii_digit()) && !x.core.is_empty() {
            digits.push_str(&x.core);
        } else {
            break;
        }
        used += 1;
        if x.breaks() || digits.len() >= 4 {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    format!("0.{digits}").parse::<f64>().ok().map(|f| (f, used))
}

/// A fractional group word: "dhai" 2.5, "derh" 1.5, "sawa N" N.25,
/// "saade N" N.5, "paune N" N-0.25. Needs a quantity after it.
fn frac_group(t: &[T], k: usize) -> Option<(f64, usize)> {
    let w = t.get(k)?.core.as_str();
    match w {
        "dhai" | "adhai" if quantity_follows(t, k + 1) => Some((2.5, 1)),
        "derh" | "dedh" | "der" if quantity_follows(t, k + 1) => Some((1.5, 1)),
        "sawa" | "saade" | "sade" | "saadhe" | "sadhe" | "paune" | "paun" => {
            let n = t.get(k + 1)?;
            let v = digit_value(&n.core).or_else(|| small(&n.core).map(|s| s.value))?;
            if !quantity_follows(t, k + 2) {
                return None;
            }
            let add = match w {
                "sawa" => v + 0.25,
                "paune" | "paun" => v - 0.25,
                _ => v + 0.5,
            };
            Some((add, 2))
        }
        _ => None,
    }
}

fn parse_num(t: &[T], i: usize) -> Option<Num> {
    let n = t.len();
    let mut k = i;
    let (mut total, mut g) = (0.0f64, 0.0f64);
    let mut g_has = false;
    let mut last: Option<Sc> = None;
    let mut indian = false;
    let mut prev_eng_tens = false;
    let mut after_hundred = false;
    let mut all_amb = true;
    let mut words = 0usize;
    let mut digit_toks = 0usize;
    let mut ended = false;
    let mut eng_small_only = false;
    while k < n && !ended {
        let w = t[k].core.as_str();
        let mut advance = 1;
        if let Some(d) = digit_value(w) {
            if g_has {
                break;
            }
            g = d;
            g_has = true;
            digit_toks += 1;
            all_amb = false;
            prev_eng_tens = false;
            after_hundred = false;
        } else if let Some((v, used)) = frac_group(t, k).filter(|_| !g_has) {
            g = v;
            g_has = true;
            advance = used;
            all_amb = false;
            words += used;
            prev_eng_tens = false;
            after_hundred = false;
        } else if let Some(s) = small(w) {
            if !g_has {
                g = s.value;
                g_has = true;
                eng_small_only = s.english && s.value < 10.0;
            } else if after_hundred && g % 100.0 == 0.0 && s.value < 100.0 {
                g += s.value;
                after_hundred = false;
            } else if prev_eng_tens && s.english && (1.0..10.0).contains(&s.value) {
                g += s.value;
            } else {
                break;
            }
            prev_eng_tens = s.eng_tens && g_has && g % 10.0 == 0.0;
            all_amb &= s.ambiguous;
            words += 1;
            if !(g < 10.0 && eng_small_only) {
                eng_small_only = false;
            }
        } else if is_hundred(w) {
            if g_has && g < 100.0 {
                g *= 100.0;
            } else if !g_has && k == i && w != "so" {
                g = 100.0;
                g_has = true;
            } else {
                break;
            }
            after_hundred = true;
            prev_eng_tens = false;
            all_amb = false;
            words += 1;
            eng_small_only = false;
        } else if let Some(sc) = scale_of(w) {
            if sc == Sc::Crore && last == Some(Sc::Lakh) && !g_has {
                total *= 1e7;
                last = Some(Sc::LakhCrore);
            } else {
                if !g_has {
                    break;
                }
                if last.is_some_and(|l| sc.val() >= l.val()) {
                    break;
                }
                total += g * sc.val();
                g = 0.0;
                g_has = false;
                last = Some(sc);
            }
            if matches!(sc, Sc::Lakh | Sc::Crore) {
                indian = true;
            }
            all_amb = false;
            words += 1;
            prev_eng_tens = false;
            after_hundred = false;
            eng_small_only = false;
        } else if w == "and"
            && g_has
            && g >= 100.0
            && t.get(k + 1).is_some_and(|x| small(&x.core).is_some())
        {
            // "two hundred and fifty"
            k += 1;
            continue;
        } else {
            break;
        }
        k += advance;
        // "one point five"
        if g_has
            && t.get(k)
                .is_some_and(|x| matches!(x.core.as_str(), "point" | "dashmlav"))
            && !t[k - 1].breaks()
        {
            if let Some((f, used)) = decimals(t, k + 1) {
                g += f;
                k += 1 + used;
                eng_small_only = false;
                all_amb = false;
                words += 1;
                ended = t[k - 1].breaks();
                continue;
            }
        }
        ended = t[k - 1].breaks();
    }
    if k == i {
        return None;
    }
    if g_has {
        total += g;
    }
    Some(Num {
        value: total,
        used: k - i,
        last,
        indian,
        digit_only: digit_toks == 1 && words == 0 && k - i == 1,
        all_ambiguous: all_amb && words > 0 && digit_toks == 0,
        english_small: eng_small_only && k - i == 1,
        spelled: words > 0,
    })
}

// ------------------------------------------------------------ rendering

fn plain(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if (r - r.round()).abs() < 1e-9 {
        format!("{}", r.round() as i64)
    } else {
        let mut s = format!("{r:.3}");
        while s.ends_with('0') {
            s.pop();
        }
        s
    }
}

fn group(v: f64, indian: bool) -> String {
    let int = v.trunc() as u64;
    let frac = plain(v - v.trunc());
    let digits = int.to_string();
    let grouped = if digits.len() <= 3 {
        digits
    } else {
        let (head, tail) = digits.split_at(digits.len() - 3);
        let mut parts: Vec<String> = Vec::new();
        let hb = head.as_bytes();
        let mut end = hb.len();
        let step = if indian { 2 } else { 3 };
        while end > 0 {
            let s = end.saturating_sub(step);
            parts.push(head[s..end].to_string());
            end = s;
        }
        parts.reverse();
        format!("{},{}", parts.join(","), tail)
    };
    if frac == "0" {
        grouped
    } else {
        format!("{grouped}{}", &frac[1..])
    }
}

/// `(value text, optional scale word)`.
fn render(num: &Num, currency: Option<char>, percent: bool) -> (String, Option<&'static str>) {
    let indian = num.indian || currency == Some('₹');
    let year_like = currency.is_none()
        && !percent
        && (1900.0..=2100.0).contains(&num.value)
        && num.value.fract() == 0.0
        && num.value % 100.0 != 0.0
        && matches!(num.last, None | Some(Sc::Thousand));
    let mut text;
    let mut word = None;
    match num.last.and_then(|s| s.word().map(|w| (s, w))) {
        Some((sc, w)) => {
            let q = num.value / sc.val();
            let nice = ((q * 100.0).round() / 100.0 - q).abs() < 1e-9;
            if nice {
                text = plain(q);
                word = Some(w);
            } else {
                text = group(num.value, indian);
            }
        }
        None if year_like => text = format!("{}", num.value as i64),
        None => text = group(num.value, indian),
    }
    if let Some(c) = currency {
        text.insert(0, c);
    }
    if percent {
        text.push('%');
    }
    (text, word)
}

// ------------------------------------------------------------ dates etc.

const MONTHS: &[(&[&str], &str, bool)] = &[
    (&["january", "janvari"], "January", false),
    (&["february", "farvari", "pharvari"], "February", false),
    (&["march"], "March", true),
    (&["april", "aprail"], "April", false),
    (&["may"], "May", true),
    (&["june", "joon", "jun"], "June", false),
    (&["july", "julai"], "July", false),
    (&["august", "agast", "agust"], "August", false),
    (&["september", "sitambar"], "September", false),
    (&["october", "aktubar", "october"], "October", false),
    (&["november", "navambar"], "November", false),
    (&["december", "disambar"], "December", false),
];

fn month_at(t: &[T], k: usize) -> Option<(&'static str, bool)> {
    let w = t.get(k)?.core.as_str();
    MONTHS
        .iter()
        .find(|(names, _, _)| names.contains(&w))
        .map(|&(_, canon, ambiguous)| (canon, ambiguous))
}

const ORD_WORDS: &[(&str, u32)] = &[
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("fifth", 5),
    ("sixth", 6),
    ("seventh", 7),
    ("eighth", 8),
    ("ninth", 9),
    ("tenth", 10),
    ("eleventh", 11),
    ("twelfth", 12),
    ("thirteenth", 13),
    ("fourteenth", 14),
    ("fifteenth", 15),
    ("sixteenth", 16),
    ("seventeenth", 17),
    ("eighteenth", 18),
    ("nineteenth", 19),
    ("twentieth", 20),
    ("thirtieth", 30),
];

fn suffix(n: u32) -> &'static str {
    match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    }
}

/// An ordinal at `k`: `(value, tokens used)` ("fourth", "twenty first", "15th").
fn ordinal_at(t: &[T], k: usize) -> Option<(u32, usize)> {
    let w = t.get(k)?.core.as_str();
    if let Some(&(_, v)) = ORD_WORDS.iter().find(|(n, _)| *n == w) {
        return Some((v, 1));
    }
    if let Some(s) = small(w).filter(|s| s.eng_tens && s.value < 40.0) {
        if let Some(&(_, v)) = t
            .get(k + 1)
            .and_then(|x| ORD_WORDS.iter().find(|(n, _)| *n == x.core.as_str()))
            .filter(|(_, v)| *v < 10)
        {
            return Some((s.value as u32 + v, 2));
        }
    }
    for suf in ["st", "nd", "rd", "th"] {
        if let Some(d) = w.strip_suffix(suf) {
            if !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()) {
                return d.parse().ok().map(|v| (v, 1));
            }
        }
    }
    None
}

/// A day number at `k`: `(1..=31, tokens used)` from numerals, number words
/// or ordinals.
fn day_at(t: &[T], k: usize) -> Option<(u32, usize)> {
    if let Some(o) = ordinal_at(t, k) {
        return (1..=31).contains(&o.0).then_some(o);
    }
    let num = parse_num(t, k)?;
    let ok = num.last.is_none()
        && num.value.fract() == 0.0
        && (1.0..=31.0).contains(&num.value)
        && !t[k + num.used - 1].core.contains('.');
    ok.then_some((num.value as u32, num.used))
}

/// A year at `k`; `after_month` also accepts a short Hindi year ("solah").
fn year_at(t: &[T], k: usize, after_month: bool) -> Option<(u32, usize)> {
    let first = t.get(k)?;
    if first.core.len() == 4 && first.core.chars().all(|c| c.is_ascii_digit()) {
        let v: u32 = first.core.parse().ok()?;
        return (1900..=2100).contains(&v).then_some((v, 1));
    }
    if let Some(num) = parse_num(t, k) {
        if num.value.fract() == 0.0
            && (1900.0..=2100.0).contains(&num.value)
            && num.used >= 2
            && matches!(num.last, None | Some(Sc::Thousand))
        {
            return Some((num.value as u32, num.used));
        }
        // "nineteen ninety five", "twenty sixteen".
        if num.used == 1 && matches!(first.core.as_str(), "nineteen" | "twenty") && !first.breaks()
        {
            let a = if first.core == "nineteen" { 1900 } else { 2000 };
            if let Some(b) = parse_num(t, k + 1) {
                if b.last.is_none() && (10.0..100.0).contains(&b.value) && b.value.fract() == 0.0 {
                    return Some((a + b.value as u32, 1 + b.used));
                }
            }
        }
        if after_month
            && num.last.is_none()
            && num.value.fract() == 0.0
            && (10.0..100.0).contains(&num.value)
            && !t[k + num.used - 1].core.contains('.')
        {
            let v = num.value as u32;
            let y = if v <= 30 { 2000 + v } else { 1900 + v };
            return Some((y, num.used));
        }
    }
    None
}

// ---------------------------------------------------------------- main

struct Out<'a> {
    t: &'a [T],
    words: Vec<NormWord>,
}

impl Out<'_> {
    fn push(&mut self, i: usize, last: usize, text: String, kind: NormKind) {
        let (a, z) = (&self.t[i], &self.t[last]);
        let text = format!("{}{}{}", a.lead, text, z.trail);
        self.words.push(NormWord {
            text,
            start: a.start,
            end: z.end,
            from: a.src,
            to: z.src,
            kind,
        });
    }
}

fn clock(h: u32, m: u32) -> String {
    format!("{h}:{m:02}")
}

/// An hour 0..=24 at `k`: `(hour, tokens used)`.
fn hour_at(t: &[T], k: usize) -> Option<(u32, usize)> {
    let x = t.get(k)?;
    if let Some(d) = digit_value(&x.core).filter(|d| d.fract() == 0.0 && *d <= 24.0) {
        return Some((d as u32, 1));
    }
    small(&x.core)
        .filter(|s| s.value <= 24.0)
        .map(|s| (s.value as u32, 1))
}

fn try_time(t: &[T], i: usize, out: &mut Out<'_>) -> Option<usize> {
    let w = t[i].core.as_str();
    let baje = |k: usize| {
        t.get(k)
            .is_some_and(|x| matches!(x.core.as_str(), "baje" | "bajey" | "oclock" | "o'clock"))
    };
    match w {
        "quarter" | "half" => {
            let rel = t.get(i + 1)?.core.as_str();
            if !matches!(rel, "past" | "to") {
                return None;
            }
            let (h, u) = hour_at(t, i + 2)?;
            if !(1..=12).contains(&h) {
                return None;
            }
            let text = match (w, rel) {
                ("quarter", "past") => clock(h, 15),
                ("half", "past") => clock(h, 30),
                ("quarter", _) => clock(if h == 1 { 12 } else { h - 1 }, 45),
                _ => clock(if h == 1 { 12 } else { h - 1 }, 30),
            };
            out.push(i, i + 2 + u - 1, text, NormKind::Time);
            Some(i + 2 + u)
        }
        "dhai" | "derh" | "dedh" | "der" if baje(i + 1) => {
            let h = if w == "dhai" { 2 } else { 1 };
            out.push(i, i + 1, clock(h, 30), NormKind::Time);
            Some(i + 2)
        }
        "saade" | "sade" | "saadhe" | "sadhe" | "sawa" | "paune" | "paun" => {
            let (h, u) = hour_at(t, i + 1)?;
            if !(1..=12).contains(&h) || quantity_follows(t, i + 1 + u) {
                return None;
            }
            let text = match w {
                "sawa" => clock(h, 15),
                "paune" | "paun" => clock(if h == 1 { 12 } else { h - 1 }, 45),
                _ => clock(h, 30),
            };
            let last = if baje(i + 1 + u) { i + 1 + u } else { i + u };
            out.push(i, last, text, NormKind::Time);
            Some(last + 1)
        }
        _ => {
            let (h, u) = hour_at(t, i)?;
            if !baje(i + u) || t[i + u - 1].breaks() {
                return None;
            }
            out.push(i, i + u, clock(h, 0), NormKind::Time);
            Some(i + u + 1)
        }
    }
}

fn try_date(t: &[T], i: usize, out: &mut Out<'_>) -> Option<usize> {
    // Month first: "August 15", "August 2016", "agast pandrah".
    if let Some((canon, ambiguous)) = month_at(t, i) {
        if ambiguous || t[i].breaks() {
            return None;
        }
        if let Some(y) = t.get(i + 1).and_then(|x| digit_value(&x.core)) {
            if (1900.0..=2100.0).contains(&y) && t[i + 1].core.len() == 4 {
                out.push(i, i + 1, format!("{canon} {}", y as u32), NormKind::Date);
                return Some(i + 2);
            }
        }
        let (d, du) = day_at(t, i + 1)?;
        out.push(i, i + du, format!("{canon} {d}"), NormKind::Date);
        return Some(i + du + 1);
    }
    // Day first: "15 August", "pandrah agast 2016", "15th of August".
    let (d, du) = day_at(t, i)?;
    if t[i + du - 1].breaks() {
        return None;
    }
    let mut k = i + du;
    if t.get(k).is_some_and(|x| x.core == "of") {
        k += 1;
    }
    let (canon, _) = month_at(t, k)?;
    let mut last = k;
    let mut text = format!("{d} {canon}");
    if !t[last].breaks() {
        if let Some((y, yu)) = year_at(t, last + 1, true) {
            text.push_str(&format!(" {y}"));
            last += yu;
        }
    }
    out.push(i, last, text, NormKind::Date);
    Some(last + 1)
}

fn try_number(t: &[T], i: usize, out: &mut Out<'_>) -> Option<usize> {
    // Currency before the number ("Rs 500", "rupees fifty").
    let mut start = i;
    let mut currency: Option<char> = None;
    if RUPEE_BEFORE.contains(&t[i].core.as_str()) && !t[i].breaks() && i + 1 < t.len() {
        currency = Some('₹');
        start = i + 1;
    } else if t[i].core.starts_with(['₹', '$', '€', '£']) {
        return None;
    }
    let num = parse_num(t, start)?;
    let mut end = start + num.used; // exclusive
    let mut percent = false;
    let last_tok = &t[end - 1];
    if !last_tok.breaks() {
        if let Some(x) = t.get(end) {
            let c = x.core.as_str();
            if currency.is_none() && RUPEE_AFTER.contains(&c) {
                currency = Some('₹');
                end += 1;
            } else if currency.is_none() && DOLLAR.contains(&c) {
                currency = Some('$');
                end += 1;
            } else if currency.is_none() && EURO.contains(&c) {
                currency = Some('€');
                end += 1;
            } else if PERCENT.contains(&c) {
                percent = true;
                end += 1;
            } else if c == "per" && t.get(end + 1).is_some_and(|y| y.core == "cent") {
                percent = true;
                end += 2;
            }
        }
    }
    let context = currency.is_some() || percent;
    if !context {
        if num.digit_only {
            return None;
        }
        if num.used == 1 && (num.english_small || num.all_ambiguous) {
            return None;
        }
        if num.all_ambiguous {
            return None;
        }
    }
    // A bare English number below ten stays a word even with other text
    // ("five reasons"), already handled; a bare "zero" likewise.
    let (text, scale) = render(&num, currency, percent);
    let last = end - 1;
    match scale {
        Some(sc) => {
            // Keep the spoken scale word as its own token, timed on the
            // word that carried it.
            let sc_idx = (start..start + num.used)
                .rev()
                .find(|&k| scale_of(&t[k].core).is_some() || t[k].core == "crore")
                .unwrap_or(start + num.used - 1);
            let sc_idx = sc_idx.min(last);
            let before = if sc_idx > i { sc_idx - 1 } else { i };
            let a = &t[i];
            let head_end = t[before].end;
            out.words.push(NormWord {
                text: format!("{}{}", a.lead, text),
                start: a.start,
                end: head_end,
                from: a.src,
                to: t[before].src,
                kind: NormKind::Number,
            });
            let z = &t[last];
            let sc_tok = &t[sc_idx];
            // "lakh crore" spans two spoken words; one token here.
            out.words.push(NormWord {
                text: format!("{sc}{}", z.trail),
                start: sc_tok.start,
                end: z.end,
                from: sc_tok.src,
                to: z.src,
                kind: NormKind::Number,
            });
        }
        None => out.push(i, last, text, NormKind::Number),
    }
    Some(end)
}

fn try_year(t: &[T], i: usize, out: &mut Out<'_>) -> Option<usize> {
    if !matches!(t[i].core.as_str(), "nineteen" | "twenty") {
        return None;
    }
    let (y, u) = year_at(t, i, false).filter(|(_, u)| *u >= 2)?;
    out.push(i, i + u - 1, y.to_string(), NormKind::Number);
    Some(i + u)
}

fn try_ordinal(t: &[T], i: usize, out: &mut Out<'_>) -> Option<usize> {
    let (v, u) = ordinal_at(t, i)?;
    // "first/second/third" are ordinary words outside a date; digits+suffix
    // ("15th") are already written.
    if v < 4 && u == 1 || t[i].core.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    out.push(i, i + u - 1, format!("{v}{}", suffix(v)), NormKind::Ordinal);
    Some(i + u)
}

fn try_short_year(t: &[T], i: usize, out: &mut Out<'_>) -> Option<usize> {
    if !matches!(t[i].core.as_str(), "saal" | "sal" | "year") || t[i].breaks() {
        return None;
    }
    let num = parse_num(t, i + 1)?;
    let after = t.get(i + 1 + num.used)?;
    let ok = num.spelled
        && num.last.is_none()
        && (10.0..100.0).contains(&num.value)
        && num.value.fract() == 0.0
        && matches!(
            after.core.as_str(),
            "mein" | "me" | "se" | "tak" | "ke" | "ki"
        );
    if !ok {
        return None;
    }
    let v = num.value as u32;
    let y = if v <= 30 { 2000 + v } else { 1900 + v };
    let word = t[i].text.clone();
    let last = i + num.used;
    out.push(i, last, format!("{word} {y}"), NormKind::Date);
    Some(last + 1)
}

/// Normalise spoken forms in `words` (Roman text, with times).
pub fn normalize(words: &[(String, f64, f64)]) -> Vec<NormWord> {
    let t = tokens(words);
    let mut out = Out {
        t: &t,
        words: Vec::new(),
    };
    let mut i = 0;
    while i < t.len() {
        let next = try_time(&t, i, &mut out)
            .or_else(|| try_date(&t, i, &mut out))
            .or_else(|| try_short_year(&t, i, &mut out))
            .or_else(|| try_year(&t, i, &mut out))
            .or_else(|| try_number(&t, i, &mut out))
            .or_else(|| try_ordinal(&t, i, &mut out));
        match next {
            Some(n) if n > i => i = n,
            _ => {
                out.words.push(NormWord {
                    text: t[i].text.clone(),
                    start: t[i].start,
                    end: t[i].end,
                    from: t[i].src,
                    to: t[i].src,
                    kind: NormKind::Kept,
                });
                i += 1;
            }
        }
    }
    out.words
}

/// Smallest spacing between word starts and smallest word length (seconds):
/// recognisers without forced alignment emit zero-length and stacked words.
pub const MIN_WORD: f64 = 0.08;

/// Make word times usable: starts never go backwards and are at least
/// [`MIN_WORD`] apart, every word lasts at least [`MIN_WORD`].
pub fn regularize_times(words: &mut [NormWord]) {
    let mut prev: Option<f64> = None;
    for w in words.iter_mut() {
        if let Some(p) = prev {
            w.start = w.start.max(p + MIN_WORD);
        }
        w.end = w.end.max(w.start + MIN_WORD);
        prev = Some(w.start);
    }
}

/// Remove decode loops: an n-gram (1..=6 words) repeated more than 3 times
/// in a row keeps its first two occurrences. Returns the words and how many
/// were removed.
pub fn collapse_loops(words: &[(String, f64, f64)]) -> (Vec<(String, f64, f64)>, usize) {
    let key = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let keys: Vec<String> = words.iter().map(|w| key(&w.0)).collect();
    let mut drop = vec![false; words.len()];
    let mut i = 0;
    while i < words.len() {
        let mut hit = false;
        for n in 1..=6usize {
            if i + n > words.len() {
                break;
            }
            let mut reps = 1;
            while i + (reps + 1) * n <= words.len()
                && keys[i..i + n] == keys[i + reps * n..i + (reps + 1) * n]
            {
                reps += 1;
            }
            if reps > 3 {
                for d in drop.iter_mut().take(i + reps * n).skip(i + 2 * n) {
                    *d = true;
                }
                i += reps * n;
                hit = true;
                break;
            }
        }
        if !hit {
            i += 1;
        }
    }
    let removed = drop.iter().filter(|d| **d).count();
    let kept = words
        .iter()
        .zip(&drop)
        .filter(|(_, d)| !**d)
        .map(|(w, _)| w.clone())
        .collect();
    (kept, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> String {
        let words: Vec<(String, f64, f64)> = s
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| (w.to_string(), i as f64, i as f64 + 0.5))
            .collect();
        normalize(&words)
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn numbers() {
        assert_eq!(run("pachaas hazaar"), "50,000");
        assert_eq!(run("dhai lakh"), "2.5 lakh");
        assert_eq!(run("teen sau"), "300");
        assert_eq!(run("das karod"), "10 crore");
        assert_eq!(run("one point five"), "1.5");
        assert_eq!(run("two hundred and fifty"), "250");
        assert_eq!(run("twenty five thousand"), "25,000");
        assert_eq!(run("chalis lakh crore"), "40 lakh crore");
        assert_eq!(run("do lakh pachaas hazaar"), "2,50,000");
        assert_eq!(run("2 lakh"), "2 lakh");
        assert_eq!(run("50 hazaar"), "50,000");
    }

    #[test]
    fn currency_and_percent() {
        assert_eq!(run("pachaas hazaar rupaye"), "₹50,000");
        assert_eq!(run("do lakh pachaas hazaar rupaye"), "₹2,50,000");
        assert_eq!(run("dhai lakh rupaye"), "₹2.5 lakh");
        assert_eq!(run("Rs 500"), "₹500");
        assert_eq!(run("Rs.500"), "₹500");
        assert_eq!(run("fifty thousand dollars"), "$50,000");
        assert_eq!(run("paanch pratishat"), "5%");
        assert_eq!(run("twelve percent"), "12%");
        assert_eq!(run("das taka"), "10%");
    }

    #[test]
    fn dates_years_and_times() {
        assert_eq!(run("15 August"), "15 August");
        assert_eq!(run("pandrah agast"), "15 August");
        assert_eq!(run("pandrah agast solah"), "15 August 2016");
        assert_eq!(run("do hazaar solah"), "2016");
        assert_eq!(run("nineteen ninety five"), "1995");
        assert_eq!(run("twenty sixteen"), "2016");
        assert_eq!(run("2 baje"), "2:00");
        assert_eq!(run("dhai baje"), "2:30");
        assert_eq!(run("saade teen"), "3:30");
        assert_eq!(run("sawa chaar baje"), "4:15");
        assert_eq!(run("quarter past five"), "5:15");
        assert_eq!(run("saade teen lakh"), "3.5 lakh");
    }

    #[test]
    fn ordinals_and_plain_words_stay() {
        assert_eq!(run("the fourth reason"), "the 4th reason");
        assert_eq!(run("first of all"), "first of all");
        assert_eq!(run("the 15th of August"), "the 15 August");
        assert_eq!(run("ek baar do log"), "ek baar do log");
        assert_eq!(
            run("hum sab ke saath bade hue"),
            "hum sab ke saath bade hue"
        );
        assert_eq!(run("I have two apples"), "I have two apples");
        assert_eq!(run("twenty people"), "20 people");
    }

    #[test]
    fn spans_and_punctuation_survive() {
        let w: Vec<(String, f64, f64)> = ["Mere", "pachaas", "hazaar", "rupaye,", "gaye."]
            .iter()
            .enumerate()
            .map(|(i, s)| (s.to_string(), i as f64, i as f64 + 0.5))
            .collect();
        let n = normalize(&w);
        assert_eq!(n[1].text, "₹50,000,");
        assert_eq!((n[1].from, n[1].to), (1, 3));
        assert_eq!((n[1].start, n[1].end), (1.0, 3.5));
    }

    #[test]
    fn regularize_spreads_stacked_words() {
        let mk = |s: f64, e: f64| NormWord {
            text: "x".into(),
            start: s,
            end: e,
            from: 0,
            to: 0,
            kind: NormKind::Kept,
        };
        let mut w = vec![mk(1.0, 1.0), mk(1.0, 1.0), mk(0.5, 0.6), mk(3.0, 3.2)];
        regularize_times(&mut w);
        let starts: Vec<f64> = w.iter().map(|x| x.start).collect();
        for (a, b) in starts.iter().zip([1.0, 1.08, 1.16, 3.0]) {
            assert!((a - b).abs() < 1e-9, "{starts:?}");
        }
        assert!(w.iter().all(|x| x.end - x.start >= MIN_WORD - 1e-12));
    }

    #[test]
    fn loops_collapse() {
        let w: Vec<(String, f64, f64)> = (0..10)
            .map(|i| ("haan".to_string(), i as f64, i as f64 + 0.5))
            .collect();
        let (k, removed) = collapse_loops(&w);
        assert_eq!((k.len(), removed), (2, 8));
    }
}
