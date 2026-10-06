//! Reply and fix-it shapes (plan §4.1, §7). Every tool answers with one
//! [`Reply`]: structured content plus the same as one short text line
//! ([`Reply::text`]), within the profile's reply budget.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// At most this many `changed` / `findings` / `fixes` / `plan` lines in a reply.
pub const MAX_LINES: usize = 5;
/// Check-mode plans list every beat (2–12), one short line each.
pub const MAX_PLAN_LINES: usize = 12;
/// Every line is cut to this many characters (ending in "…").
pub const MAX_LINE_CHARS: usize = 110;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The video is rendered and checked.
    Done,
    /// The job is rendering; call `get_video(job, wait_s)`.
    Running,
    /// Waiting for the render before it (one render at a time).
    Queued,
    /// `mode: check` finished: the plan is in `plan`, nothing was rendered.
    Checked,
    /// The story has problems that were not fixed automatically; see `fixes`.
    NeedsFix,
    /// The engine failed; `error` says why in one line.
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Done => "done",
            Status::Running => "running",
            Status::Queued => "queued",
            Status::Checked => "checked",
            Status::NeedsFix => "needs_fix",
            Status::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Qa {
    Pass,
    Fail,
}

/// Where a running job is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// check, voice, compile, music, render, qa, preview
    pub stage: String,
    pub percent: u8,
}

/// A problem the model can fix in one retry: which beat and field, what is
/// wrong, and the values that would work.
///
/// Rendered as one line, e.g.
/// `beat 2 picture: no picture for "berry_bush" — use "strawberry" or "blueberry", or leave it (shown as text)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// 1-based beat; `None` for story-level problems.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<usize>,
    /// The field to change: `say`, `picture`, `beats`, `assets`, `job` …
    pub field: String,
    pub problem: String,
    /// Values that would work, best first (at most 3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// What else the model may do, e.g. `leave it (shown as text)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<String>,
}

impl Fix {
    pub fn new(beat: Option<usize>, field: &str, problem: impl Into<String>) -> Self {
        Fix {
            beat,
            field: field.to_string(),
            problem: problem.into(),
            options: Vec::new(),
            otherwise: None,
        }
    }

    pub fn options<I, S>(mut self, options: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.options = options.into_iter().map(Into::into).take(3).collect();
        self
    }

    pub fn otherwise(mut self, text: impl Into<String>) -> Self {
        self.otherwise = Some(text.into());
        self
    }
}

impl fmt::Display for Fix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.beat {
            Some(n) => write!(f, "beat {n} {}: {}", self.field, self.problem)?,
            None => write!(f, "{}: {}", self.field, self.problem)?,
        }
        let quoted: Vec<String> = self.options.iter().map(|o| format!("\"{o}\"")).collect();
        match (quoted.is_empty(), &self.otherwise) {
            (false, Some(o)) => write!(f, " — use {}, or {o}", quoted.join(" or ")),
            (false, None) => write!(f, " — use {}", quoted.join(" or ")),
            (true, Some(o)) => write!(f, " — {o}"),
            (true, None) => Ok(()),
        }
    }
}

/// The answer of every video tool (`make_video`, `revise_video`, `get_video`).
/// Empty fields are left out of the JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// Repository-relative path of the finished video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
    /// Repository-relative path of the 720p preview (≤ 30 MB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa: Option<Qa>,
    /// What was fixed automatically (≤ 5 lines).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
    /// QA findings and warnings (≤ 5 lines).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// Only with `needs_fix` (≤ 5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixes: Vec<Fix>,
    /// Only with `checked`: one line per beat (structure, pictures found or
    /// shown as text) plus the estimated duration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<usize>,
    /// One-line reason with `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the model should do next, in one sentence.
    pub next: String,
}

impl Reply {
    pub fn new(status: Status, next: impl Into<String>) -> Self {
        Reply {
            status,
            job: None,
            video: None,
            preview: None,
            duration_s: None,
            qa: None,
            changed: Vec::new(),
            findings: Vec::new(),
            fixes: Vec::new(),
            plan: Vec::new(),
            progress: None,
            queue_position: None,
            error: None,
            next: next.into(),
        }
    }

    /// `needs_fix` with the fixes (capped) and the standard next step.
    pub fn needs_fix(fixes: Vec<Fix>) -> Self {
        let mut r = Reply::new(
            Status::NeedsFix,
            "Fix the listed fields and call the same tool again.",
        );
        r.fixes = fixes;
        r.capped()
    }

    /// (0.23) On a finished video (`done`), `next` mentions once how to ask
    /// for another version: `take: N+1 for another version`, where `take` is
    /// the job's take. Nothing is added to any other status, or after the
    /// highest take.
    pub fn with_take_hint(mut self, take: u64) -> Self {
        if self.status == Status::Done && take < crate::policy::MAX_TAKE {
            self.next = format!("{} Use take: {} for another version.", self.next, take + 1);
        }
        self
    }

    /// `failed` with a one-line reason.
    pub fn failed(job: Option<String>, error: impl Into<String>) -> Self {
        let mut r = Reply::new(
            Status::Failed,
            "The engine failed; simplify the story or try again.",
        );
        r.job = job;
        r.error = Some(cut(&error.into()));
        r
    }

    /// Enforce the caps: ≤ [`MAX_LINES`] per list (≤ [`MAX_PLAN_LINES`] plan
    /// lines), every line ≤ [`MAX_LINE_CHARS`].
    pub fn capped(mut self) -> Self {
        for list in [&mut self.changed, &mut self.findings] {
            list.truncate(MAX_LINES);
            for l in list.iter_mut() {
                *l = cut(l);
            }
        }
        self.plan.truncate(MAX_PLAN_LINES);
        for l in self.plan.iter_mut() {
            *l = cut(l);
        }
        self.fixes.truncate(MAX_LINES);
        if let Some(e) = &self.error {
            self.error = Some(cut(e));
        }
        self.next = cut(&self.next);
        self
    }

    /// The reply as short text (what most clients show the model). Fields in
    /// a fixed order, `·`-separated, lists `;`-joined.
    pub fn text(&self) -> String {
        let mut parts: Vec<String> = vec![self.status.as_str().to_string()];
        if let Some(j) = &self.job {
            parts.push(format!("job {j}"));
        }
        if let Some(p) = &self.progress {
            parts.push(format!("{} {}%", p.stage, p.percent));
        }
        if let Some(q) = self.queue_position {
            parts.push(format!("queue {q}"));
        }
        if let Some(d) = self.duration_s {
            parts.push(format!("{d:.1} s"));
        }
        if let Some(q) = self.qa {
            parts.push(format!(
                "qa {}",
                if q == Qa::Pass { "pass" } else { "fail" }
            ));
        }
        if let Some(v) = &self.video {
            parts.push(format!("video {v}"));
        }
        if let Some(e) = &self.error {
            parts.push(format!("error: {e}"));
        }
        let mut text = parts.join(" · ");
        for (label, lines) in [
            ("plan", &self.plan),
            ("changed", &self.changed),
            ("findings", &self.findings),
        ] {
            if !lines.is_empty() {
                text.push_str(&format!("\n{label}: {}", lines.join("; ")));
            }
        }
        if !self.fixes.is_empty() {
            let fixes: Vec<String> = self.fixes.iter().map(|f| cut(&f.to_string())).collect();
            text.push_str(&format!("\nfix: {}", fixes.join("; ")));
        }
        text.push_str(&format!("\nnext: {}", self.next));
        text
    }
}

/// Cut a line to [`MAX_LINE_CHARS`] characters (on a char boundary, "…").
pub fn cut(s: &str) -> String {
    let s = s.trim().replace('\n', " ");
    if s.chars().count() <= MAX_LINE_CHARS {
        s
    } else {
        let mut out: String = s.chars().take(MAX_LINE_CHARS - 1).collect();
        out.push('…');
        out
    }
}

/// An image a tool returns (creator `view_frames`, `explore_styles`, drafts):
/// sent as MCP image content, ≤ 1024 px on the long side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageOut {
    pub path: PathBuf,
    /// `image/png` or `image/jpeg`.
    pub mime: &'static str,
}

/// What a tool handler returns: the structured reply, its text form and any
/// images. Tools other than the video tools use `structured` freely (e.g.
/// `find_assets` returns `{word: [names]}`), but keep `text` within budget.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub structured: serde_json::Value,
    pub text: String,
    pub images: Vec<ImageOut>,
    /// Repository-relative files to also return as MCP resource links.
    pub links: Vec<String>,
    /// True for `needs_fix` / `failed` / refused calls (MCP `isError`).
    pub is_error: bool,
}

impl From<Reply> for ToolOutput {
    fn from(r: Reply) -> Self {
        let r = r.capped();
        let links = [r.video.clone(), r.preview.clone()]
            .into_iter()
            .flatten()
            .collect();
        ToolOutput {
            text: r.text(),
            is_error: matches!(r.status, Status::NeedsFix | Status::Failed),
            structured: serde_json::to_value(&r).expect("Reply serializes"),
            images: Vec::new(),
            links,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_lines_read_like_the_plan() {
        let f = Fix::new(Some(2), "picture", "no picture for \"berry_bush\"")
            .options(["strawberry", "blueberry"])
            .otherwise("leave it (shown as text)");
        assert_eq!(
            f.to_string(),
            "beat 2 picture: no picture for \"berry_bush\" — use \"strawberry\" or \"blueberry\", or leave it (shown as text)"
        );
        assert_eq!(
            Fix::new(None, "beats", "need 2 to 12 beats (got 1)").to_string(),
            "beats: need 2 to 12 beats (got 1)"
        );
    }

    #[test]
    fn replies_stay_short() {
        let mut r = Reply::new(
            Status::Done,
            "Done. Call revise_video with changes if needed.",
        );
        r.job = Some("j_3f9a1c27b0".into());
        r.video = Some("output/jobs/j_3f9a1c27b0/video.mp4".into());
        r.preview = Some("output/jobs/j_3f9a1c27b0/preview_720p.mp4".into());
        r.duration_s = Some(32.5);
        r.qa = Some(Qa::Pass);
        r.changed = (0..9)
            .map(|i| {
                format!(
                    "beat {i}: picture 'berry_bush' → 'strawberry' {}",
                    "x".repeat(200)
                )
            })
            .collect();
        let out = ToolOutput::from(r);
        assert!(out.text.len() <= 800, "{} chars", out.text.len());
        assert_eq!(out.links.len(), 2);
        assert!(!out.is_error);
        assert_eq!(
            out.structured["changed"].as_array().unwrap().len(),
            MAX_LINES
        );
        assert!(out
            .text
            .starts_with("done · job j_3f9a1c27b0 · 32.5 s · qa pass"));
    }
}
