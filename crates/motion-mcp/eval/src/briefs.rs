//! The briefs (`briefs.jsonl`) and the "does the story match the brief" check.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Brief {
    pub id: String,
    pub topic: String,
    pub structure: String,
    pub brief: String,
    pub expect: Expect,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Expect {
    /// Inclusive `[min, max]` beat count.
    pub beats: [usize; 2],
    /// Words or numbers that must appear in the story's text.
    #[serde(default)]
    pub key_terms: Vec<String>,
}

/// Read a JSONL file: one brief per line, blank lines skipped.
pub fn load(path: &Path) -> Result<Vec<Brief>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the briefs {}", path.display()))?;
    parse(&text).with_context(|| format!("in {}", path.display()))
}

pub fn parse(text: &str) -> Result<Vec<Brief>> {
    let mut briefs: Vec<Brief> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let brief: Brief =
            serde_json::from_str(line).with_context(|| format!("line {}: not a brief", i + 1))?;
        if briefs.iter().any(|b| b.id == brief.id) {
            bail!("line {}: duplicate id {}", i + 1, brief.id);
        }
        briefs.push(brief);
    }
    Ok(briefs)
}

/// Keep the briefs named in `only` (all when empty); an unknown id is an error.
pub fn select(briefs: Vec<Brief>, only: &[String]) -> Result<Vec<Brief>> {
    if only.is_empty() {
        return Ok(briefs);
    }
    for id in only {
        if !briefs.iter().any(|b| &b.id == id) {
            bail!("no brief with id {id}");
        }
    }
    Ok(briefs
        .into_iter()
        .filter(|b| only.contains(&b.id))
        .collect())
}

/// Lower-case, and drop the commas inside numbers ("10,000" → "10000"), so a
/// term matches however the model formatted the figure. Commas between words
/// stay.
pub fn normalize(s: &str) -> String {
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        let in_number = c == ','
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if !in_number {
            out.push(c);
        }
    }
    out
}

/// Beat fields whose text counts for the brief (lite story and intent beats).
const TEXT_FIELDS: [&str; 11] = [
    "say",
    "show",
    "number",
    "list",
    "compare",
    "change",
    "layers",
    "narration",
    "statement",
    "primary",
    "secondary",
];

/// Every string and number inside `v`, appended with spaces.
fn collect_text(v: &Value, out: &mut String) {
    match v {
        Value::String(s) => {
            out.push_str(s);
            out.push(' ');
        }
        Value::Number(n) => {
            out.push_str(&n.to_string());
            out.push(' ');
        }
        Value::Array(a) => a.iter().for_each(|x| collect_text(x, out)),
        Value::Object(m) => m.values().for_each(|x| collect_text(x, out)),
        _ => {}
    }
}

/// The beats of a story (lite or intent).
pub fn beats_of(story: &Value) -> &[Value] {
    story
        .get("beats")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// The concatenated say/show/number/list/compare/change/layers text of every beat.
pub fn story_text(story: &Value) -> String {
    let mut out = String::new();
    for beat in beats_of(story) {
        for field in TEXT_FIELDS {
            if let Some(v) = beat.get(field) {
                collect_text(v, &mut out);
            }
        }
    }
    out
}

/// The result of matching a story against a brief's expectations.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BriefMatch {
    pub beats: usize,
    pub beats_ok: bool,
    pub expected_beats: [usize; 2],
    /// Key terms not found in the story's text.
    pub missing_terms: Vec<String>,
    pub ok: bool,
}

pub fn match_brief(story: &Value, expect: &Expect) -> BriefMatch {
    let beats = beats_of(story).len();
    let beats_ok = beats >= expect.beats[0] && beats <= expect.beats[1];
    let text = normalize(&story_text(story));
    let missing_terms: Vec<String> = expect
        .key_terms
        .iter()
        .filter(|t| !text.contains(&normalize(t)))
        .cloned()
        .collect();
    BriefMatch {
        beats,
        beats_ok,
        expected_beats: expect.beats,
        ok: beats_ok && missing_terms.is_empty(),
        missing_terms,
    }
}

impl BriefMatch {
    /// What did not match, for the table's note; empty when it matched.
    pub fn problem(&self) -> String {
        let mut parts = Vec::new();
        if !self.beats_ok {
            parts.push(format!(
                "{} beats, wanted {}-{}",
                self.beats, self.expected_beats[0], self.expected_beats[1]
            ));
        }
        if !self.missing_terms.is_empty() {
            parts.push(format!("missing: {}", self.missing_terms.join(", ")));
        }
        parts.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expect(beats: [usize; 2], terms: &[&str]) -> Expect {
        Expect {
            beats,
            key_terms: terms.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn the_shipped_briefs_load() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("briefs.jsonl");
        let briefs = load(&path).unwrap();
        assert_eq!(briefs.len(), 20);
        assert!(briefs
            .iter()
            .all(|b| b.expect.beats[0] <= b.expect.beats[1]));
        let vague = briefs.iter().filter(|b| b.structure == "vague").count();
        assert_eq!(vague, 2);
    }

    #[test]
    fn parse_reports_the_line() {
        let ok = r#"{"id":"a","topic":"t","structure":"s","brief":"b","expect":{"beats":[2,6],"key_terms":[]}}"#;
        assert_eq!(
            parse(&format!("{ok}\n\n{ok}x\n")).unwrap_err().to_string(),
            "line 3: not a brief"
        );
        let dup = parse(&format!("{ok}\n{ok}\n")).unwrap_err().to_string();
        assert_eq!(dup, "line 2: duplicate id a");
        assert_eq!(parse(&format!("{ok}\n")).unwrap().len(), 1);
    }

    #[test]
    fn select_filters_and_rejects_unknown_ids() {
        let b = |id: &str| Brief {
            id: id.into(),
            topic: "t".into(),
            structure: "s".into(),
            brief: "b".into(),
            expect: expect([2, 6], &[]),
        };
        let all = vec![b("a"), b("b"), b("c")];
        assert_eq!(select(all.clone(), &[]).unwrap().len(), 3);
        let some = select(all.clone(), &["c".into(), "a".into()]).unwrap();
        assert_eq!(
            some.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            ["a", "c"]
        );
        assert!(select(all, &["zzz".into()]).is_err());
    }

    #[test]
    fn numbers_match_with_or_without_commas_and_any_case() {
        assert_eq!(normalize("10,000 Homes, ABC"), "10000 homes, abc");
        assert_eq!(normalize("Rock, Paper"), "rock, paper");
        let story = json!({"beats": [
            {"say": "Training one model can use as much ELECTRICITY as 10000 homes use in a year."},
            {"say": "That is a lot.", "number": "10,000 homes"}
        ]});
        // The term has a comma, the story does not (and the other way round).
        let m = match_brief(&story, &expect([2, 6], &["electricity", "10,000"]));
        assert!(m.ok, "{m:?}");
        let story2 = json!({"beats": [
            {"say": "It costs 10,000 dollars."}, {"say": "And Electricity too."}]});
        assert!(match_brief(&story2, &expect([2, 6], &["10000", "ELECTRICITY"])).ok);
    }

    #[test]
    fn missing_terms_and_beat_range_are_reported() {
        let story = json!({"beats": [{"say": "Memory is one part."}]});
        let m = match_brief(&story, &expect([3, 8], &["memory", "tools", "planner"]));
        assert_eq!(m.beats, 1);
        assert!(!m.beats_ok && !m.ok);
        assert_eq!(m.missing_terms, ["tools", "planner"]);
        assert_eq!(m.problem(), "1 beats, wanted 3-8; missing: tools, planner");
        // No expectations beyond the range: any text matches.
        let two = json!({"beats": [{"say": "a"}, {"say": "b"}]});
        let m = match_brief(&two, &expect([2, 12], &[]));
        assert!(m.ok);
        assert_eq!(m.problem(), "");
        // No story at all.
        assert_eq!(match_brief(&Value::Null, &expect([2, 12], &[])).beats, 0);
    }

    #[test]
    fn text_comes_from_every_structure() {
        let story = json!({"beats": [
            {"say": "S1", "show": "Show1", "number": "$381B", "meaning": "ignored meaning"},
            {"say": "S2", "list": ["robot", "microchip"]},
            {"say": "S3", "compare": {"a": "cost", "b": "output", "how": "separate"}},
            {"say": "S4", "change": {"what": "water", "from": "ice", "to": "steam"}},
            {"say": "S5", "layers": {"names": ["sunlit", "twilight"], "focus": "twilight"},
             "picture": "ignored picture"}
        ]});
        let t = normalize(&story_text(&story));
        for want in [
            "s1",
            "show1",
            "$381b",
            "robot",
            "microchip",
            "cost",
            "output",
            "separate",
            "water",
            "ice",
            "steam",
            "sunlit",
            "twilight",
        ] {
            assert!(t.contains(want), "{want} not in {t}");
        }
        assert!(!t.contains("ignored"), "{t}");
    }

    #[test]
    fn intent_beats_count_too() {
        let story = json!({"version": "0.2", "beats": [
            {"narration": "The Moon landing was in 1969.", "statement": "Apollo 11",
             "primary": {"kind": "number", "value": "1969"}},
            {"narration": "Two people walked on the Moon."}
        ]});
        assert!(match_brief(&story, &expect([2, 6], &["1969", "moon"])).ok);
    }
}
