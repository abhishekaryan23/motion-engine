//! (0.10 Q) Taste rules for shorts — core-owned budgets shared by builders and
//! layout QA. They answer the owner's review of weak-model reels: giant
//! headlines (narration written into `statement`), subjects too small or
//! buried behind type, frames far bigger than their asset, assets that vanish
//! into the ground, kickers that read like captions.
//!
//! FROZEN: constants, `DisplayText`, `display_text`, `kicker_ok`.
//! Builders consume them; layout QA enforces them.

use crate::intent::{Beat, Subject};

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// A statement longer than this is narration, not a title: the screen shows a
/// short title and the full sentence is spoken (captions) or set as body copy.
/// (Fixtures and examples top out at 10 words, so legacy output is unchanged.)
pub const LONG_STATEMENT_WORDS: usize = 12;
/// A derived title never exceeds this many words.
pub const TITLE_MAX_WORDS: usize = 6;
/// Display headline lines: tall canvases / square & wide canvases.
pub const HEADLINE_MAX_LINES_TALL: usize = 4;
pub const HEADLINE_MAX_LINES_WIDE: usize = 3;
/// Display headline block height as a fraction of canvas height, when the beat
/// also shows a subject image / when type is the whole picture.
pub const HEADLINE_MAX_H_WITH_SUBJECT: f32 = 0.30;
pub const HEADLINE_MAX_H_TYPE_ONLY: f32 = 0.46;
/// Body copy (long statements without a voice-over): lines at most.
pub const BODY_MAX_LINES: usize = 4;
/// Kicker / furniture labels: words at most (never an asset description).
pub const KICKER_MAX_WORDS: usize = 3;

// ---------------------------------------------------------------------------
// Subjects (delivered images and library cutouts)
// ---------------------------------------------------------------------------

/// Minimum subject alpha-bounds area as a fraction of the canvas area, by
/// layout class (tall, square, wide), for the beat's hero subject image.
pub const SUBJECT_MIN_AREA: (f32, f32, f32) = (0.16, 0.14, 0.12);
/// Below this alpha-bounds aspect (w/h) a subject is "narrow" (a standing
/// figure): at full height its area shrinks with its aspect, so its minimum
/// scales by `aspect / NARROW_SUBJECT_ASPECT`, never below half the base.
pub const NARROW_SUBJECT_ASPECT: f32 = 0.5;
/// A subject also meets the size rule when its alpha bounds are at least this
/// tall relative to the canvas height: on square and wide canvases a standing
/// figure at half the frame height covers only a few percent of the area.
pub const SUBJECT_MIN_HEIGHT: f32 = 0.42;
/// Text may cover at most this fraction of a subject's alpha bounds (ghost
/// words and background fields exempt); heads never.
pub const TEXT_OVER_SUBJECT_MAX: f32 = 0.04;
/// A card/frame/plate drawn behind a subject may exceed the subject's bounds
/// by at most this fraction of the subject size on each side.
pub const FRAME_PAD_MAX: f32 = 0.12;
/// Luminance contrast (WCAG ratio) between a subject's mean opaque colour and
/// the ground under it below which the subject gets a contrast treatment
/// (sticker outline in paper/ink + soft shadow, or a shrink-wrapped plate).
pub const ASSET_GROUND_MIN_CONTRAST: f32 = 2.0;

// ---------------------------------------------------------------------------
// Display vs spoken text
// ---------------------------------------------------------------------------

/// What a beat shows and says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayText {
    /// The on-screen headline (the statement when it is short).
    pub title: String,
    /// Body copy shown small under the title (long statement, no voice-over).
    pub body: Option<String>,
    /// What the narrator says (and captions show): narration, else statement.
    pub spoken: String,
    /// True when `title` was derived from a long statement.
    pub derived: bool,
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Display/spoken split for a beat. Pure and deterministic.
///
/// * Short statement (≤ [`LONG_STATEMENT_WORDS`]): title = statement.
/// * Long statement: title = the primary phrase when it is a phrase of at most
///   [`TITLE_MAX_WORDS`] words; else the keyword (≤ 3 words); else the first
///   [`TITLE_MAX_WORDS`] words of the statement, cut back to the last clause
///   punctuation inside that window (no ellipsis) or else ending in "…".
///   The full statement becomes `body` without a voice-over (`speech == false`)
///   and is otherwise left to the captions.
/// * `spoken` = `narration` when present, else the full statement.
pub fn display_text(beat: &Beat, speech: bool) -> DisplayText {
    let statement = beat.statement.trim();
    let spoken = beat
        .narration
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(statement)
        .to_string();
    if words(statement) <= LONG_STATEMENT_WORDS {
        return DisplayText {
            title: statement.to_string(),
            body: None,
            spoken,
            derived: false,
        };
    }
    let phrase = match &beat.primary {
        Subject::Phrase(a) => a.value.as_deref().map(str::trim),
        _ => None,
    }
    .filter(|v| !v.is_empty() && words(v) <= TITLE_MAX_WORDS);
    let keyword = beat
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty() && words(k) <= 3);
    let title = match phrase.or(keyword) {
        Some(t) => t.to_string(),
        None => {
            let head: Vec<&str> = statement.split_whitespace().take(TITLE_MAX_WORDS).collect();
            // Cut back to the last clause end inside the window, if any.
            let cut = head
                .iter()
                .rposition(|w| w.ends_with([',', '.', ';', ':', '?', '!']))
                .filter(|&i| i >= 1);
            let kept = match cut {
                Some(i) => head[..=i].join(" "),
                None => head.join(" "),
            };
            let kept = kept.trim_end_matches([',', ';', ':']).to_string();
            // A clause cut reads complete; a cut mid-clause gets an ellipsis.
            if cut.is_some() {
                kept
            } else {
                format!("{kept}…")
            }
        }
    };
    DisplayText {
        title,
        body: (!speech).then(|| statement.to_string()),
        spoken,
        derived: true,
    }
}

/// True when a kicker/furniture label is acceptable on screen.
pub fn kicker_ok(label: &str) -> bool {
    let n = words(label);
    (1..=KICKER_MAX_WORDS).contains(&n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::CreativeIntent;

    fn beat(json: &str) -> Beat {
        let doc =
            format!(r#"{{"version":"0.2","title":"t","format":"vertical","beats":[{json}]}}"#);
        CreativeIntent::from_json(&doc).unwrap().beats.remove(0)
    }

    #[test]
    fn short_statements_are_titles_and_spoken() {
        let b = beat(
            r#"{"purpose":"emphasize","statement":"Prices kept climbing","primary":{"kind":"phrase","value":"prices"}}"#,
        );
        let d = display_text(&b, true);
        assert_eq!(d.title, "Prices kept climbing");
        assert_eq!(d.spoken, "Prices kept climbing");
        assert!(!d.derived && d.body.is_none());
    }

    #[test]
    fn narration_is_spoken_and_title_stays() {
        let b = beat(
            r#"{"purpose":"emphasize","statement":"Saving feels impossible","narration":"Honestly? Saving money feels impossible at first.","primary":{"kind":"phrase","value":"impossible"}}"#,
        );
        let d = display_text(&b, true);
        assert_eq!(d.title, "Saving feels impossible");
        assert_eq!(
            d.spoken,
            "Honestly? Saving money feels impossible at first."
        );
    }

    #[test]
    fn long_statements_get_a_short_title() {
        // The owner's spotlight test: narration written into `statement`.
        let b = beat(
            r#"{"purpose":"reveal","statement":"Nobody has the time to obsess over your minor mistakes so stop overthinking the room moved on hours ago","primary":{"kind":"phrase","value":"MOVE ON"}}"#,
        );
        let d = display_text(&b, true);
        assert_eq!(d.title, "MOVE ON");
        assert!(d.derived && d.body.is_none());
        assert!(d.spoken.starts_with("Nobody has the time"));
        let silent = display_text(&b, false);
        assert!(silent.body.as_deref().unwrap().starts_with("Nobody"));
        // No usable phrase or keyword: first words, cut at punctuation.
        let n = beat(
            r#"{"purpose":"compare","statement":"Cornell sent students into a room, wearing an embarrassing shirt and only 23 percent noticed it","primary":{"kind":"number","value":"50%"}}"#,
        );
        assert_eq!(
            display_text(&n, true).title,
            "Cornell sent students into a room"
        );
        let m = beat(
            r#"{"purpose":"compare","statement":"Kisi ke paas fursat nahi hai room kab ka aage badh chuka hai yaar","primary":{"kind":"number","value":"23%"}}"#,
        );
        assert_eq!(
            display_text(&m, true).title,
            "Kisi ke paas fursat nahi hai…"
        );
    }

    #[test]
    fn kickers_are_short() {
        assert!(kicker_ok("The result"));
        assert!(!kicker_ok("Family walking together with child"));
    }
}
