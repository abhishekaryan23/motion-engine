//! Devanagari -> Roman Hinglish, deterministic and table-based.
//!
//! Casual spelling, not a scholarly scheme: no diacritics, long and short
//! vowels merge ("hai", "kya", "nahi", "bahut"), schwa is deleted at word
//! ends and in the usual medial position ("karna", "samjhna", "namaste").
//! Anything that is not Devanagari (English words, digits, punctuation) passes
//! through unchanged. A word dictionary beats the generic rules for the
//! common words where casual spelling differs from a letter-by-letter one.

/// Common words whose casual Roman spelling the generic rules would miss.
const WORDS: &[(&str, &str)] = &[
    ("है", "hai"),
    ("हैं", "hain"),
    ("हूँ", "hoon"),
    ("हूं", "hoon"),
    ("था", "tha"),
    ("थे", "the"),
    ("थी", "thi"),
    ("बहुत", "bahut"),
    ("क्या", "kya"),
    ("नहीं", "nahi"),
    ("नही", "nahi"),
    ("में", "mein"),
    ("मैं", "main"),
    ("मै", "main"),
    ("मेरा", "mera"),
    ("मेरी", "meri"),
    ("मेरे", "mere"),
    ("और", "aur"),
    ("यह", "yeh"),
    ("ये", "ye"),
    ("वह", "woh"),
    ("वो", "wo"),
    ("को", "ko"),
    ("का", "ka"),
    ("की", "ki"),
    ("के", "ke"),
    ("से", "se"),
    ("तो", "to"),
    ("भी", "bhi"),
    ("कि", "ki"),
    ("जो", "jo"),
    ("हम", "hum"),
    ("आप", "aap"),
    ("तुम", "tum"),
    ("कुछ", "kuch"),
    ("सब", "sab"),
    ("कैसे", "kaise"),
    ("कौन", "kaun"),
    ("कब", "kab"),
    ("कहाँ", "kahan"),
    ("कहां", "kahan"),
    ("क्यों", "kyon"),
    ("क्योंकि", "kyunki"),
    ("लिए", "liye"),
    ("साथ", "saath"),
    ("लोग", "log"),
    ("पैसा", "paisa"),
    ("पैसे", "paise"),
    ("रुपये", "rupaye"),
    ("रुपया", "rupaya"),
    ("लाख", "lakh"),
    ("करोड़", "crore"),
    ("करोड", "crore"),
    ("हज़ार", "hazaar"),
    ("हजार", "hazaar"),
    ("सौ", "sau"),
    ("एक", "ek"),
    ("दो", "do"),
    ("तीन", "teen"),
    ("चार", "chaar"),
    ("पाँच", "paanch"),
    ("पांच", "paanch"),
    ("छह", "chhe"),
    ("छः", "chhe"),
    ("सात", "saat"),
    ("आठ", "aath"),
    ("नौ", "nau"),
    ("दस", "das"),
    ("पचास", "pachaas"),
    ("ढाई", "dhai"),
    ("डेढ़", "dedh"),
    ("डेढ", "dedh"),
    ("साढ़े", "saade"),
    ("साढे", "saade"),
    ("सवा", "sawa"),
    ("पौने", "paune"),
    ("बजे", "baje"),
    ("प्रतिशत", "pratishat"),
    ("अगस्त", "agast"),
    ("आज", "aaj"),
    ("अब", "ab"),
    ("जब", "jab"),
    ("तब", "tab"),
    ("यहाँ", "yahan"),
    ("वहाँ", "wahan"),
    ("अच्छा", "accha"),
    ("अच्छी", "acchi"),
    ("बात", "baat"),
    ("काम", "kaam"),
    ("साल", "saal"),
    ("पर", "par"),
    ("ही", "hi"),
    ("भाई", "bhai"),
    ("दोस्त", "dost"),
    ("वीडियो", "video"),
    ("हाँ", "haan"),
    ("चीज", "cheez"),
    ("चीज़", "cheez"),
    ("बट", "but"),
    ("इंटरनेट", "internet"),
    ("हमें", "humein"),
    ("हमने", "humne"),
    ("वही", "wahi"),
    ("लिया", "liya"),
    ("किया", "kiya"),
    ("जाए", "jaye"),
    ("पाएं", "payen"),
    ("ठीक", "theek"),
    ("समझ", "samajh"),
    ("हां", "haan"),
];

fn is_dev(c: char) -> bool {
    ('\u{0900}'..='\u{097F}').contains(&c)
}

/// Whether `s` contains any Devanagari.
pub fn has_devanagari(s: &str) -> bool {
    s.chars().any(is_dev)
}

fn consonant(c: char) -> Option<&'static str> {
    Some(match c {
        'क' => "k",
        'ख' => "kh",
        'ग' => "g",
        'घ' => "gh",
        'ङ' => "n",
        'च' => "ch",
        'छ' => "chh",
        'ज' => "j",
        'झ' => "jh",
        'ञ' => "n",
        'ट' => "t",
        'ठ' => "th",
        'ड' => "d",
        'ढ' => "dh",
        'ण' => "n",
        'त' => "t",
        'थ' => "th",
        'द' => "d",
        'ध' => "dh",
        'न' => "n",
        'प' => "p",
        'फ' => "ph",
        'ब' => "b",
        'भ' => "bh",
        'म' => "m",
        'य' => "y",
        'र' => "r",
        'ल' => "l",
        'व' => "v",
        'श' => "sh",
        'ष' => "sh",
        'स' => "s",
        'ह' => "h",
        'ळ' => "l",
        // Precomposed nukta letters (U+0958..U+095E).
        '\u{0958}' => "k",
        '\u{0959}' => "kh",
        '\u{095A}' => "g",
        '\u{095B}' => "z",
        '\u{095C}' => "d",
        '\u{095D}' => "dh",
        '\u{095E}' => "f",
        _ => return None,
    })
}

fn vowel(c: char) -> Option<&'static str> {
    Some(match c {
        'अ' => "a",
        'आ' => "a",
        'इ' => "i",
        'ई' => "i",
        'उ' => "u",
        'ऊ' => "u",
        'ऋ' => "ri",
        'ए' => "e",
        'ऐ' => "ai",
        'ओ' => "o",
        'औ' => "au",
        'ऑ' => "o",
        _ => return None,
    })
}

fn matra(c: char) -> Option<&'static str> {
    Some(match c {
        'ा' => "a",
        'ि' => "i",
        'ी' => "i",
        'ु' => "u",
        'ू' => "u",
        'े' => "e",
        'ै' => "ai",
        'ो' => "o",
        'ौ' => "au",
        'ॉ' => "o",
        'ृ' => "ri",
        'ॅ' => "e",
        _ => return None,
    })
}

const VIRAMA: char = '्';
const NUKTA: char = '़';

/// One written unit: a consonant cluster plus an explicit vowel, or an
/// implicit schwa (`vowel == None`), or a bare vowel (`cons` empty).
struct Unit {
    cons: String,
    vowel: Option<&'static str>,
    /// More than one consonant (a conjunct).
    conjunct: bool,
    /// Trailing nasal ("n"/"m"/"h") written after the vowel.
    tail: String,
}

fn units_of(word: &str) -> Vec<Unit> {
    let chars: Vec<char> = word.chars().collect();
    let mut out: Vec<Unit> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(v) = vowel(c) {
            out.push(Unit {
                cons: String::new(),
                vowel: Some(v),
                conjunct: false,
                tail: String::new(),
            });
            i += 1;
            continue;
        }
        if consonant(c).is_some() {
            let mut cons = String::new();
            let mut n = 0;
            let mut explicit: Option<&'static str> = None;
            // Gather consonant + virama + consonant ... chains.
            loop {
                let c = chars[i];
                let mut piece = consonant(c).unwrap_or("").to_string();
                i += 1;
                if chars.get(i) == Some(&NUKTA) {
                    piece = match c {
                        'क' => "k",
                        'ख' => "kh",
                        'ग' => "g",
                        'ज' => "z",
                        'ड' => "d",
                        'ढ' => "dh",
                        'फ' => "f",
                        _ => piece.as_str(),
                    }
                    .to_string();
                    i += 1;
                }
                // ज्ञ is "gy", क्ष is "ksh" (the latter falls out of the tables).
                if c == 'ज' && chars.get(i) == Some(&VIRAMA) && chars.get(i + 1) == Some(&'ञ') {
                    cons.push_str("gy");
                    i += 2;
                    n += 1;
                } else {
                    cons.push_str(&piece);
                    n += 1;
                }
                if chars.get(i) == Some(&VIRAMA) {
                    i += 1;
                    if chars.get(i).is_some_and(|&n| consonant(n).is_some()) {
                        continue;
                    }
                    // Trailing virama: the cluster has no vowel at all.
                    explicit = Some("");
                    break;
                }
                break;
            }
            if explicit.is_none() {
                if let Some(m) = chars.get(i).and_then(|&c| matra(c)) {
                    explicit = Some(m);
                    i += 1;
                }
            }
            out.push(Unit {
                cons,
                vowel: explicit,
                conjunct: n > 1,
                tail: String::new(),
            });
            continue;
        }
        match c {
            'ं' | 'ँ' => {
                if let Some(u) = out.last_mut() {
                    u.tail.push('n');
                }
            }
            'ः' => {
                if let Some(u) = out.last_mut() {
                    u.tail.push('h');
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Whether unit `u` ends up with a pronounced vowel.
fn voiced(u: &Unit, deleted: bool) -> bool {
    match u.vowel {
        Some(v) => !v.is_empty(),
        None => !deleted,
    }
}

fn devanagari_word(word: &str) -> String {
    if let Some((_, r)) = WORDS.iter().find(|(d, _)| *d == word) {
        return (*r).to_string();
    }
    let units = units_of(word);
    let n = units.len();
    // Right to left: delete implicit schwas at the end and in V C_ C V position.
    let mut deleted = vec![false; n];
    for i in (0..n).rev() {
        let u = &units[i];
        if u.vowel.is_some() || u.cons.is_empty() {
            continue;
        }
        if i == n - 1 && n > 1 {
            deleted[i] = u.tail.is_empty();
        } else if i > 0 && i + 1 < n {
            let prev = voiced(&units[i - 1], deleted[i - 1]);
            let next = voiced(&units[i + 1], deleted[i + 1]);
            // Keep it before a conjunct ("namaste") and when it carries a nasal.
            deleted[i] = prev && next && !units[i + 1].conjunct && u.tail.is_empty() && !u.conjunct;
        }
    }
    let mut out = String::new();
    for (i, u) in units.iter().enumerate() {
        out.push_str(&u.cons);
        match u.vowel {
            Some(v) => out.push_str(v),
            None if u.cons.is_empty() => {}
            None if !deleted[i] => out.push('a'),
            None => {}
        }
        // "ai"/"au" before a nasal stay; the nasal letter follows.
        let _ = i;
        out.push_str(&u.tail);
    }
    // Danda and digits are handled by the caller; collapse a doubled "aa".
    out
}

/// Transliterate every Devanagari run in `text`; other characters pass
/// through unchanged.
pub fn to_roman(text: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if !run.is_empty() {
            out.push_str(&devanagari_word(run));
            run.clear();
        }
    };
    for c in text.chars() {
        if is_dev(c) && !matches!(c, '।' | '॥') && !('०'..='९').contains(&c) {
            run.push(c);
            continue;
        }
        flush(&mut run, &mut out);
        match c {
            '।' => out.push('.'),
            '॥' => out.push('.'),
            '०'..='९' => out.push(char::from(b'0' + (c as u32 - '०' as u32) as u8)),
            _ => out.push(c),
        }
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> String {
        to_roman(s)
    }

    #[test]
    fn common_words() {
        assert_eq!(t("है"), "hai");
        assert_eq!(t("बहुत"), "bahut");
        assert_eq!(t("क्या"), "kya");
        assert_eq!(t("नहीं"), "nahi");
        assert_eq!(t("में"), "mein");
        assert_eq!(t("और"), "aur");
        assert_eq!(t("पचास हज़ार"), "pachaas hazaar");
        assert_eq!(t("करोड़"), "crore");
    }

    #[test]
    fn schwa_deletion_generic_rules() {
        assert_eq!(t("कमल"), "kamal");
        assert_eq!(t("करना"), "karna");
        assert_eq!(t("समझ"), "samajh");
        assert_eq!(t("समझना"), "samajhna");
        assert_eq!(t("नमस्ते"), "namaste");
        assert_eq!(t("अब"), "ab");
        assert_eq!(t("सवाल"), "saval");
        assert_eq!(t("कहना"), "kahna");
    }

    #[test]
    fn english_and_digits_pass_through_and_mix() {
        assert_eq!(t("hello world 2016"), "hello world 2016");
        assert_eq!(t("यह video बहुत अच्छा है"), "yeh video bahut accha hai");
        assert_eq!(t("१५ अगस्त"), "15 agast");
        assert_eq!(t("ठीक है।"), "theek hai.");
    }
}
