//! Results: the summary maths, the JSON file and the markdown table.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::runner::BriefResult;

/// Plan section 1, weak profile.
pub const TARGET_SUCCESS: f64 = 0.85;
pub const TARGET_AVG_CALLS: f64 = 1.5;
pub const TARGET_TOOL_DEF_TOKENS: u64 = 1200;
pub const TARGET_REPLY_TOKENS: u64 = 200;

/// Token estimate for text the model reads: four characters a token.
pub fn est_tokens(chars: usize) -> u64 {
    chars.div_ceil(4) as u64
}

/// What the run was.
#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub date: String,
    pub model: String,
    pub endpoint: String,
    pub profile: String,
    pub no_render: bool,
    pub system: String,
    pub temperature: f64,
    pub seed: u64,
    pub max_turns: usize,
    pub max_tokens: u64,
    pub server: String,
}

/// The size of the tool definitions the model was given.
#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub tool_count: usize,
    pub names: Vec<String>,
    /// Characters of the serialized MCP tool list.
    pub chars: usize,
    /// `prompt_tokens` with the tools minus without them, measured on the
    /// model (None when the probe failed).
    pub measured_tokens: Option<u64>,
    /// `prompt_tokens` of the first request of the run.
    pub first_request_prompt_tokens: Option<u64>,
}

impl ToolInfo {
    /// Measured tokens, else characters ÷ 4.
    pub fn tokens(&self) -> u64 {
        self.measured_tokens
            .unwrap_or_else(|| est_tokens(self.chars))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub briefs: usize,
    pub successes: usize,
    pub success_rate: f64,
    pub total_calls: usize,
    /// Model tool calls per brief, over all briefs.
    pub avg_calls: f64,
    /// … over the successful briefs only.
    pub avg_calls_on_success: Option<f64>,
    pub fix_retries: usize,
    pub bad_args: usize,
    pub repeated_calls: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub avg_wall_s: f64,
    /// Tool replies the model read, by estimated tokens (chars ÷ 4).
    pub replies: usize,
    pub reply_tokens_avg: f64,
    pub reply_tokens_max: u64,
    pub tool_def_tokens: u64,
    pub tool_def_chars: usize,
    pub success_ok: bool,
    pub calls_ok: bool,
    pub tool_defs_ok: bool,
    pub replies_ok: bool,
}

pub fn summarize(results: &[BriefResult], tools: &ToolInfo) -> Summary {
    let n = results.len();
    let successes = results.iter().filter(|r| r.success).count();
    let total_calls: usize = results.iter().map(|r| r.tool_calls).sum();
    let success_calls: usize = results
        .iter()
        .filter(|r| r.success)
        .map(|r| r.tool_calls)
        .sum();
    let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    let reply_tokens: Vec<u64> = results
        .iter()
        .flat_map(|r| r.calls.iter())
        .filter(|c| !c.bad_args && !c.refused)
        .map(|c| est_tokens(c.reply_chars))
        .collect();
    let reply_sum: u64 = reply_tokens.iter().sum();
    let reply_tokens_avg = if reply_tokens.is_empty() {
        0.0
    } else {
        reply_sum as f64 / reply_tokens.len() as f64
    };
    let reply_tokens_max = reply_tokens.iter().copied().max().unwrap_or(0);
    let avg_calls = ratio(total_calls, n);
    let success_rate = ratio(successes, n);
    let tool_def_tokens = tools.tokens();
    Summary {
        briefs: n,
        successes,
        success_rate,
        total_calls,
        avg_calls,
        avg_calls_on_success: (successes > 0).then(|| ratio(success_calls, successes)),
        fix_retries: results.iter().map(|r| r.needs_fix).sum(),
        bad_args: results.iter().map(|r| r.bad_args).sum(),
        repeated_calls: results.iter().map(|r| r.repeated_calls).sum(),
        prompt_tokens: results.iter().map(|r| r.prompt_tokens).sum(),
        completion_tokens: results.iter().map(|r| r.completion_tokens).sum(),
        avg_wall_s: if n == 0 {
            0.0
        } else {
            results.iter().map(|r| r.wall_s).sum::<f64>() / n as f64
        },
        replies: reply_tokens.len(),
        reply_tokens_avg,
        reply_tokens_max,
        tool_def_tokens,
        tool_def_chars: tools.chars,
        success_ok: n > 0 && success_rate >= TARGET_SUCCESS,
        calls_ok: n > 0 && avg_calls <= TARGET_AVG_CALLS,
        tool_defs_ok: tool_def_tokens <= TARGET_TOOL_DEF_TOKENS,
        replies_ok: reply_tokens_max <= TARGET_REPLY_TOKENS,
    }
}

/// `<date>-<model>-<profile>[-norender]` (the model id made filename-safe).
pub fn result_stem(date: &str, model: &str, profile: &str, no_render: bool) -> String {
    let safe: String = model
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let suffix = if no_render { "-norender" } else { "" };
    format!("{date}-{safe}-{profile}{suffix}")
}

/// The whole run as JSON.
pub fn to_json(meta: &Meta, tools: &ToolInfo, summary: &Summary, results: &[BriefResult]) -> Value {
    json!({
        "meta": meta,
        "tools": tools,
        "summary": summary,
        "briefs": results,
    })
}

fn pass(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "MISS"
    }
}

/// A table cell: no pipes or newlines.
fn cell(s: &str) -> String {
    s.replace('|', "/").replace('\n', " ")
}

/// The markdown report: a table with one row per brief and a summary
/// against the plan's targets.
pub fn to_markdown(
    meta: &Meta,
    tools: &ToolInfo,
    summary: &Summary,
    results: &[BriefResult],
) -> String {
    let mode = if meta.no_render {
        "check only (--no-render): story quality, no render, no QA"
    } else {
        "rendered"
    };
    let mut md = format!(
        "# motion-mcp eval: {} on the {} profile\n\n\
         - date {}, {mode}\n\
         - endpoint {}, temperature {}, seed {}, system prompt {}, max {} turns\n\
         - tools: {}\n\n",
        meta.model,
        meta.profile,
        meta.date,
        meta.endpoint,
        meta.temperature,
        meta.seed,
        meta.system,
        meta.max_turns,
        tools.names.join(", "),
    );
    md.push_str(
        "| brief | success | calls | fix retries | tokens in/out | wall s | video s | note |\n\
         |---|---|---:|---:|---:|---:|---:|---|\n",
    );
    for r in results {
        let video = r
            .duration_s
            .map(|d| format!("{d:.1}"))
            .unwrap_or_else(|| "-".into());
        md.push_str(&format!(
            "| {} | {} | {} | {} | {}/{} | {:.1} | {} | {} |\n",
            cell(&r.id),
            if r.success { "yes" } else { "no" },
            r.tool_calls,
            r.needs_fix,
            r.prompt_tokens,
            r.completion_tokens,
            r.wall_s,
            video,
            cell(&r.note),
        ));
    }
    let s = summary;
    let on_success = s
        .avg_calls_on_success
        .map(|a| format!(", {a:.2} on the successful ones"))
        .unwrap_or_default();
    let measured = match tools.measured_tokens {
        Some(t) => format!("{t} tokens measured on the model"),
        None => format!(
            "about {} tokens (chars / 4, no probe)",
            est_tokens(tools.chars)
        ),
    };
    let first = tools
        .first_request_prompt_tokens
        .map(|t| format!(", first request {t} prompt tokens"))
        .unwrap_or_default();
    md.push_str(&format!(
        "\n## Summary against the plan targets (section 1, weak profile)\n\n\
         | metric | result | target | |\n|---|---|---|---|\n\
         | success | {}/{} = {:.0} % | >= {:.0} % | {} |\n\
         | tool calls per brief | {:.2} avg{on_success} | <= {} | {} |\n\
         | tool definitions | {} chars, {measured}{first} | <= {} tokens | {} |\n\
         | tool reply size | {:.0} tokens avg, {} max (chars / 4, {} replies) | <= {} tokens | {} |\n\n\
         - fix retries {}, bad arguments {}, repeated identical calls {}, tokens in/out {}/{}, average wall {:.1} s per brief\n",
        s.successes,
        s.briefs,
        s.success_rate * 100.0,
        TARGET_SUCCESS * 100.0,
        pass(s.success_ok),
        s.avg_calls,
        TARGET_AVG_CALLS,
        pass(s.calls_ok),
        s.tool_def_chars,
        TARGET_TOOL_DEF_TOKENS,
        pass(s.tool_defs_ok),
        s.reply_tokens_avg,
        s.reply_tokens_max,
        s.replies,
        TARGET_REPLY_TOKENS,
        pass(s.replies_ok),
        s.fix_retries,
        s.bad_args,
        s.repeated_calls,
        s.prompt_tokens,
        s.completion_tokens,
        s.avg_wall_s,
    ));
    md
}

/// Write `<stem>.json` and `<stem>.md` into `out`; returns both paths.
pub fn write_results(
    out: &Path,
    stem: &str,
    json: &Value,
    markdown: &str,
) -> std::io::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(out)?;
    let json_path = out.join(format!("{stem}.json"));
    let md_path = out.join(format!("{stem}.md"));
    let mut text = serde_json::to_string_pretty(json).map_err(std::io::Error::other)?;
    text.push('\n');
    std::fs::write(&json_path, text)?;
    std::fs::write(&md_path, markdown)?;
    Ok((json_path, md_path))
}

/// `YYYY-MM-DD` (UTC) of a Unix time in seconds.
pub fn date_of(unix_secs: u64) -> String {
    let (y, m, d) = civil_from_days((unix_secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Today's date (UTC).
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    date_of(secs)
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefs::BriefMatch;
    use crate::runner::CallRecord;

    fn call(reply_chars: usize) -> CallRecord {
        CallRecord {
            turn: 1,
            name: "make_video".into(),
            arguments: "{}".into(),
            forwarded: None,
            bad_args: false,
            refused: false,
            status: Some("done".into()),
            is_error: false,
            reply: String::new(),
            reply_chars,
        }
    }

    fn result(id: &str, success: bool, calls: &[usize], prompt: u64) -> BriefResult {
        BriefResult {
            id: id.into(),
            topic: "t".into(),
            structure: "s".into(),
            success,
            final_status: Some("done".into()),
            qa: Some("pass".into()),
            tool_calls: calls.len(),
            bad_args: 0,
            needs_fix: calls.len().saturating_sub(1),
            repeated_calls: 0,
            tool_errors: 0,
            turns: calls.len(),
            prompt_tokens: prompt,
            completion_tokens: 100,
            first_prompt_tokens: Some(prompt),
            wall_s: 10.0,
            duration_s: success.then_some(20.0),
            job: None,
            beats: 3,
            changed: vec![],
            findings: vec![],
            brief_match: BriefMatch {
                beats: 3,
                beats_ok: true,
                expected_beats: [2, 6],
                missing_terms: vec![],
                ok: true,
            },
            note: if success {
                "".into()
            } else {
                "bad | pipe".into()
            },
            final_text: String::new(),
            finish_reason: None,
            final_raw: None,
            calls: calls.iter().map(|&c| call(c)).collect(),
        }
    }

    fn tools(chars: usize, measured: Option<u64>) -> ToolInfo {
        ToolInfo {
            tool_count: 4,
            names: vec!["make_video".into(), "get_video".into()],
            chars,
            measured_tokens: measured,
            first_request_prompt_tokens: Some(1500),
        }
    }

    fn meta() -> Meta {
        Meta {
            date: "2026-10-03".into(),
            model: "lfm-eval".into(),
            endpoint: "http://localhost:1234/v1".into(),
            profile: "weak".into(),
            no_render: false,
            system: "generic".into(),
            temperature: 0.2,
            seed: 7,
            max_turns: 6,
            max_tokens: 3000,
            server: "target/release/motion-mcp".into(),
        }
    }

    #[test]
    fn summary_maths() {
        // 4 briefs: 3 succeed with 1, 1 and 2 calls; one fails after 3 calls.
        let results = vec![
            result("a", true, &[400], 1000),
            result("b", true, &[600], 1000),
            result("c", true, &[200, 800], 2000),
            result("d", false, &[100, 100, 100], 3000),
        ];
        let s = summarize(&results, &tools(4000, Some(1100)));
        assert_eq!((s.briefs, s.successes), (4, 3));
        assert_eq!(s.success_rate, 0.75);
        assert!(!s.success_ok);
        assert_eq!(s.total_calls, 7);
        assert_eq!(s.avg_calls, 1.75);
        assert!(!s.calls_ok);
        // (1 + 1 + 2) / 3 successful briefs.
        assert!((s.avg_calls_on_success.unwrap() - 4.0 / 3.0).abs() < 1e-9);
        assert_eq!(s.fix_retries, 3);
        assert_eq!(s.prompt_tokens, 7000);
        assert_eq!(s.completion_tokens, 400);
        assert_eq!(s.avg_wall_s, 10.0);
        // Replies: 400, 600, 200, 800, 100 x3 chars -> 100, 150, 50, 200, 25 x3 tokens.
        assert_eq!(s.replies, 7);
        assert_eq!(s.reply_tokens_max, 200);
        assert!(s.replies_ok);
        assert!((s.reply_tokens_avg - 575.0 / 7.0).abs() < 1e-9);
        assert_eq!(s.tool_def_tokens, 1100);
        assert!(s.tool_defs_ok);
    }

    #[test]
    fn targets_are_inclusive() {
        let results: Vec<_> = (0..20)
            .map(|i| result(&format!("b{i}"), i < 17, &[100], 100))
            .collect();
        // 17 of 20 = 85 %, one call each, replies of exactly 200 tokens.
        let results: Vec<_> = results
            .into_iter()
            .map(|mut r| {
                r.calls[0].reply_chars = 800;
                r
            })
            .collect();
        let s = summarize(&results, &tools(4800, None));
        assert_eq!(s.success_rate, 0.85);
        assert!(s.success_ok && s.calls_ok && s.replies_ok);
        // No probe: 4800 chars / 4 = 1200 tokens, exactly the target.
        assert_eq!(s.tool_def_tokens, 1200);
        assert!(s.tool_defs_ok);
        let over = summarize(&results, &tools(4804, None));
        assert!(!over.tool_defs_ok);
    }

    #[test]
    fn an_empty_run_summarizes_to_zeros() {
        let s = summarize(&[], &tools(100, None));
        assert_eq!((s.briefs, s.successes, s.total_calls), (0, 0, 0));
        assert_eq!(s.avg_calls, 0.0);
        assert!(s.avg_calls_on_success.is_none());
        assert!(!s.success_ok && !s.calls_ok);
        assert_eq!(s.reply_tokens_max, 0);
    }

    #[test]
    fn bad_and_refused_calls_do_not_count_as_replies() {
        let mut r = result("a", false, &[400, 400], 100);
        r.calls[0].bad_args = true;
        r.calls[1].refused = true;
        let s = summarize(&[r], &tools(100, None));
        assert_eq!(s.replies, 0);
    }

    #[test]
    fn markdown_has_a_row_per_brief_and_the_targets() {
        let results = vec![
            result("ai_agent_parts", true, &[400], 1500),
            result("sci_atmosphere", false, &[400, 400], 3000),
        ];
        let t = tools(4000, Some(1100));
        let s = summarize(&results, &t);
        let md = to_markdown(&meta(), &t, &s, &results);
        assert!(md.contains(
            "| brief | success | calls | fix retries | tokens in/out | wall s | video s | note |"
        ));
        assert!(
            md.contains("| ai_agent_parts | yes | 1 | 0 | 1500/100 | 10.0 | 20.0 |  |"),
            "{md}"
        );
        // A pipe in a note cannot break the table; no video length shows a dash.
        assert!(
            md.contains("| sci_atmosphere | no | 2 | 1 | 3000/100 | 10.0 | - | bad / pipe |"),
            "{md}"
        );
        assert!(
            md.contains("| success | 1/2 = 50 % | >= 85 % | MISS |"),
            "{md}"
        );
        assert!(
            md.contains(
                "| tool calls per brief | 1.50 avg, 1.00 on the successful ones | <= 1.5 | PASS |"
            ),
            "{md}"
        );
        assert!(md.contains("4000 chars, 1100 tokens measured on the model, first request 1500 prompt tokens | <= 1200 tokens | PASS |"), "{md}");
        assert!(md.contains("<= 200 tokens | PASS |"), "{md}");
        assert!(md.contains("lfm-eval") && md.contains("make_video, get_video"));
    }

    #[test]
    fn json_keeps_the_raw_arguments() {
        let mut r = result("a", true, &[400], 100);
        r.calls[0].arguments = "{\"story\": {}}".into();
        let t = tools(100, None);
        let s = summarize(std::slice::from_ref(&r), &t);
        let v = to_json(&meta(), &t, &s, std::slice::from_ref(&r));
        assert_eq!(v["briefs"][0]["calls"][0]["arguments"], "{\"story\": {}}");
        assert_eq!(v["meta"]["model"], "lfm-eval");
        assert_eq!(v["summary"]["successes"], 1);
        assert_eq!(v["tools"]["chars"], 100);
        assert!(v["briefs"][0]["calls"][0].get("forwarded").is_none());
    }

    #[test]
    fn file_names() {
        assert_eq!(
            result_stem("2026-10-03", "lfm-eval", "weak", true),
            "2026-10-03-lfm-eval-weak-norender"
        );
        assert_eq!(
            result_stem("2026-10-03", "google/gemma 4:e4b", "creator", false),
            "2026-10-03-google-gemma-4-e4b-creator"
        );
    }

    #[test]
    fn dates() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(19_723 * 86_400), "2024-01-01");
        assert_eq!(date_of(19_782 * 86_400 + 86_399), "2024-02-29");
        assert_eq!(date_of(20_729 * 86_400 + 3_600), "2026-10-03");
        assert_eq!(date_of(20_818 * 86_400), "2026-12-31");
        assert_eq!(date_of(20_819 * 86_400), "2027-01-01");
        assert_eq!(today().len(), 10);
    }
}
