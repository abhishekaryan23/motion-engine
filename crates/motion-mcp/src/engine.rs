//! (1b) The engine runner: `motion-engine reel` as a subprocess (Phase 1),
//! stage progress parsed from its `[n/5]` lines, the job's QA via
//! `motion-engine qa --json`, the 720p preview via ffmpeg. Phase 2 replaces
//! the subprocess with the in-process pipeline library.
//!
//! Every subprocess runs with stdin closed and stdout / stderr piped (never
//! inherited: the server's stdout is the MCP channel). The reel runs in its own
//! process group so a cancellation can stop it and everything it started.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use motion_core::audio::MusicWord;
use motion_core::compiler::WARN_MUSIC_FIT;
use serde_json::Value;

use crate::args::ProductionOptions;
use crate::job::{JobId, Stage, StoredRequest, INTENT_JSON, LOG_TXT, MANIFEST_JSON, PREVIEW_MP4};
use crate::job::{REEL_DIR, STYLE_JSON, VIDEO_MP4};
use crate::profile::{Profile, ServerConfig};
use crate::reply::{Qa, MAX_LINES};

/// Largest preview file before it is re-encoded at a higher crf.
pub const PREVIEW_MAX_BYTES: u64 = 30 * 1024 * 1024;
/// Preview crf steps (the first one that fits wins; the last one is kept).
const PREVIEW_CRFS: [u32; 3] = [26, 32, 38];
/// How long a cancelled process group gets between TERM and KILL.
const KILL_GRACE: Duration = Duration::from_secs(5);
/// Lines of engine output kept for the one-line error.
const TAIL_LINES: usize = 60;

/// One queued render, as the engine sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineJob {
    pub id: JobId,
    /// `<jobs>/<id>`: holds `intent.json`, `style.json`, maybe the manifest.
    pub dir: PathBuf,
    /// The normalised request (production options, profile).
    pub request: StoredRequest,
    /// The intent title: `reel` names its outputs after it.
    pub title: String,
    /// The job has `assets.manifest.json` (user images).
    pub has_manifest: bool,
}

/// A finished render.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineOutput {
    /// `<job>/video.mp4`.
    pub video: PathBuf,
    /// `<job>/preview_720p.mp4` (None when ffmpeg failed; see findings).
    pub preview: Option<PathBuf>,
    pub duration_s: Option<f64>,
    /// None when QA could not run (see findings).
    pub qa: Option<Qa>,
    /// QA failures first, then engine notes; one line each, at most 5.
    pub findings: Vec<String>,
}

/// Cancellation of one running job: the reason, and the process group to stop.
#[derive(Debug, Default)]
pub struct Cancel {
    reason: Mutex<Option<String>>,
    pgid: Mutex<Option<u32>>,
}

impl Cancel {
    pub fn new() -> Arc<Cancel> {
        Arc::new(Cancel::default())
    }

    /// Cancel with `reason` (the first reason wins) and stop the registered
    /// process group, if any.
    pub fn cancel(self: &Arc<Self>, reason: &str) {
        {
            let mut r = lock(&self.reason);
            if r.is_none() {
                *r = Some(reason.to_string());
            }
        }
        let pgid = *lock(&self.pgid);
        if let Some(pgid) = pgid {
            self.stop(pgid);
        }
    }

    /// TERM to the group now, KILL after a grace period if it is still
    /// registered (not yet reaped).
    fn stop(self: &Arc<Self>, pgid: u32) {
        kill_group(pgid, "TERM");
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(KILL_GRACE);
            if *lock(&me.pgid) == Some(pgid) {
                kill_group(pgid, "KILL");
            }
        });
    }

    pub fn reason(&self) -> Option<String> {
        lock(&self.reason).clone()
    }

    pub fn is_cancelled(&self) -> bool {
        lock(&self.reason).is_some()
    }

    /// The engine started a process group; stop it at once if already cancelled.
    pub fn register(self: &Arc<Self>, pgid: u32) {
        *lock(&self.pgid) = Some(pgid);
        if self.is_cancelled() {
            self.stop(pgid);
        }
    }

    /// The process group has exited (and was reaped).
    pub fn unregister(&self) {
        *lock(&self.pgid) = None;
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `kill -<signal> -- -<pgid>` (no new crates; works with BSD and procps kill).
fn kill_group(pgid: u32, signal: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg("--")
        .arg(format!("-{pgid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Runs one job. `stage` is called at every stage boundary; `cancel` is
/// registered with any process group the engine starts.
pub trait Engine: Send + Sync {
    fn run(
        &self,
        config: &ServerConfig,
        job: &EngineJob,
        stage: &dyn Fn(Stage),
        cancel: &Arc<Cancel>,
    ) -> Result<EngineOutput, String>;
}

/// Phase 1: the `motion-engine` binary as a subprocess.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubprocessEngine;

impl Engine for SubprocessEngine {
    fn run(
        &self,
        config: &ServerConfig,
        job: &EngineJob,
        stage: &dyn Fn(Stage),
        cancel: &Arc<Cancel>,
    ) -> Result<EngineOutput, String> {
        let reel_dir = job.dir.join(REEL_DIR);
        std::fs::create_dir_all(&reel_dir)
            .map_err(|e| format!("cannot create {}: {e}", reel_dir.display()))?;
        let log = Log::open(&job.dir.join(LOG_TXT));

        // 1. The reel.
        let args = reel_args(config, job);
        log.line(&format!("$ motion-engine {}", show_args(&args)));
        let mut cmd = config.engine_command();
        cmd.args(&args);
        let run = run_streaming(cmd, &log, cancel, &mut |line| {
            if let Some(s) = stage_of_line(line) {
                stage(s);
            }
        })?;
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        if !run.success {
            log.line(&match run.code {
                Some(c) => format!("(reel exited with code {c})"),
                None => "(reel stopped by a signal)".to_string(),
            });
            let seen = run.stdout.iter().rev().find_map(|l| stage_of_line(l));
            return Err(one_line_error(seen, &run.stderr, &run.stdout, run.code));
        }
        let reel_video = run
            .stdout
            .iter()
            .rev()
            .find_map(|l| video_of_line(l))
            .map(|p| absolute(config, p))
            .unwrap_or_else(|| default_video(&reel_dir, &job.title));
        if !reel_video.is_file() {
            return Err(format!(
                "the reel finished without a video at {}",
                reel_video.display()
            ));
        }

        // 2. QA (the reel already printed it; this reads it as JSON).
        stage(Stage::Qa);
        let mut findings = Vec::new();
        let qa = match run_qa(config, &reel_dir, &job.title, &reel_video, &log) {
            Ok((qa, lines)) => {
                findings.extend(lines);
                Some(qa)
            }
            Err(e) => {
                findings.push(format!("qa did not run: {e}"));
                None
            }
        };
        // (0.20) The compile's story and timing warnings, after QA failures.
        findings.extend(compile_warnings(&run.stdout));
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }

        // 3. The job's video and its preview.
        stage(Stage::Preview);
        let video = job.dir.join(VIDEO_MP4);
        link_or_copy(&reel_video, &video)
            .map_err(|e| format!("cannot place the video in the job: {e}"))?;
        let preview_path = job.dir.join(PREVIEW_MP4);
        let preview = match make_preview(&video, &preview_path, &log) {
            Ok(()) => Some(preview_path),
            Err(e) => {
                findings.push(format!("no preview: {e}"));
                None
            }
        };
        let duration_s = probe_duration(&video);
        if let Some(reason) = cancel.reason() {
            return Err(reason);
        }
        // (0.23) The bed and why, last, so the cap never drops it.
        if let Some(music) = music_finding(&run.stdout) {
            findings.truncate(MAX_LINES - 1);
            findings.push(music);
        }
        findings.truncate(MAX_LINES);
        Ok(EngineOutput {
            video,
            preview,
            duration_s,
            qa,
            findings,
        })
    }
}

/// The `motion-engine` arguments for a job's reel (everything after the binary).
pub fn reel_args(config: &ServerConfig, job: &EngineJob) -> Vec<OsString> {
    let o: &ProductionOptions = &job.request.options;
    let operator = job.request.profile == Profile::Operator;
    let word = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("auto")
            .to_string()
    };
    let tts = match &o.tts_model {
        Some(m) if operator && !m.trim().is_empty() => m.trim().to_string(),
        _ => config.tts_model.clone(),
    };
    let mut a: Vec<OsString> = vec![
        "reel".into(),
        job.dir.join(INTENT_JSON).into(),
        "--style".into(),
        job.dir.join(STYLE_JSON).into(),
        "-o".into(),
        job.dir.join(REEL_DIR).into(),
        "--assets".into(),
        config.assets.clone().into(),
        "--tts-model".into(),
        tts.into(),
        "--music".into(),
        word(&o.music).into(),
        "--art".into(),
        word(&o.art).into(),
    ];
    if o.captions == Some(false) {
        a.push("--no-captions".into());
    }
    if o.variety.as_ref().and_then(Value::as_str).map(str::trim) == Some("off") {
        a.push("--no-variety".into());
    }
    // (0.23) Take 0 is the default and adds nothing to the command line.
    if job.request.take != 0 {
        a.push("--take".into());
        a.push(job.request.take.to_string().into());
    }
    // (0.23) The music word: `auto` (read the mood from the story) adds nothing.
    if job.request.music != MusicWord::Auto {
        a.push("--music-mood".into());
        a.push(job.request.music.as_str().into());
    }
    for f in o
        .families
        .iter()
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
    {
        a.push("--asset-family".into());
        a.push(f.into());
    }
    if let Some(aspect) = o.aspect.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        a.push("--aspect".into());
        a.push(aspect.into());
    }
    if job.has_manifest {
        a.push("--asset-manifest".into());
        a.push(job.dir.join(MANIFEST_JSON).into());
    }
    if o.keep_frames && operator {
        a.push("--keep-frames".into());
    }
    a.extend(config.reel_extra.iter().map(OsString::from));
    a
}

/// The finding for options the Phase 1 reel cannot honour.
pub fn variety_note(options: &ProductionOptions) -> Option<String> {
    options
        .variety
        .as_ref()
        .filter(|v| v.is_number())
        .map(|_| "variety seed applies from Phase 2".to_string())
}

/// The stage a `reel` stdout line starts (`[n/5] …`).
pub fn stage_of_line(line: &str) -> Option<Stage> {
    let rest = line.trim_start().strip_prefix('[')?;
    let (tag, _) = rest.split_once(']')?;
    match tag {
        "1/5" => Some(Stage::Voice),
        "2/5" => Some(Stage::Compile),
        "3/5" => Some(Stage::Music),
        "4/5" => Some(Stage::Render),
        "5/5" => Some(Stage::Qa),
        _ => None,
    }
}

/// The video path of the reel's final `video: <path>` line.
pub fn video_of_line(line: &str) -> Option<PathBuf> {
    let path = line.strip_prefix("video: ")?.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn absolute(config: &ServerConfig, p: PathBuf) -> PathBuf {
    if p.is_absolute() {
        p
    } else {
        config.repo.join(p)
    }
}

/// Where `reel` writes the video: `<out>/<title>/<title>.mp4`.
pub fn default_video(reel_dir: &Path, title: &str) -> PathBuf {
    reel_dir.join(title).join(format!("{title}.mp4"))
}

/// Filename-safe stem, the same rule as `reel` (`stem_of` in reel_cmd.rs).
pub fn stem_of(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "reel".to_string()
    } else {
        s
    }
}

/// Story warnings first: they say what to change in the story.
const STORY_WARNINGS: [&str; 6] = [
    "unnamed_picture",
    "value_dropped",
    "title_states_conclusion",
    "unrelated_beats",
    "carry_ignored",
    // (0.23) The `music` word asks for a mood the story does not read as.
    "music_fit",
];

/// (0.22) The story review behind `mode: check`: compile the checked story
/// (no voice, no render: about a second) with the flags the reel would use,
/// and return the compiler's story warnings as findings (an unnamed picture, a
/// title that spoils its line, beats that do not relate …), so a model can
/// fix the story before paying for a render. Best effort: an engine that
/// cannot compile here gives no review (the render path reports problems).
pub fn story_review(
    config: &ServerConfig,
    id: &JobId,
    request: &StoredRequest,
    intent: &Value,
    style: &Value,
    manifest: Option<&crate::byo::PreparedImages>,
) -> Vec<String> {
    let dir = config.jobs.join("_check").join(id.as_str());
    let run = || -> Option<Vec<String>> {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(dir.join(INTENT_JSON), intent.to_string()).ok()?;
        std::fs::write(dir.join(STYLE_JSON), style.to_string()).ok()?;
        let o = &request.options;
        let word = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("auto")
                .to_string()
        };
        let mut c = config.engine_command();
        c.arg("compile")
            .arg(dir.join(INTENT_JSON))
            .arg("--style")
            .arg(dir.join(STYLE_JSON))
            .arg("--assets")
            .arg(&config.assets)
            .arg("--art")
            .arg(word(&o.art))
            .arg("--output")
            .arg(dir.join("review.motion.json"));
        if o.variety.as_ref().and_then(Value::as_str).map(str::trim) != Some("off") {
            // (0.23 B3) The review compiles the candidate the reel will ship
            // (`reel` defaults to best of 4).
            c.arg("--variety").arg("auto").arg("--candidates").arg("4");
        }
        // (0.23) The review compiles the take the job will render.
        if request.take != 0 {
            c.arg("--take").arg(request.take.to_string());
        }
        for f in o
            .families
            .iter()
            .map(|f| f.trim())
            .filter(|f| !f.is_empty())
        {
            c.arg("--asset-family").arg(f);
        }
        if let Some(aspect) = o.aspect.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            c.arg("--aspect").arg(aspect);
        }
        if let Some(images) = manifest {
            let m = crate::byo::write_manifest(images, &dir).ok()?;
            c.arg("--asset-manifest").arg(m);
        }
        let out = c
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        Some(compile_warnings(&lines))
    };
    let review = run().unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    // Leave no trace: `_check` goes too once no other review is using it.
    if let Some(parent) = dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    review
}

/// The compile warnings a reel printed (`warning[code]: beat N: …`, 0.20
/// DECISIONS 121) as findings without the code: story warnings first, then
/// the rest (e.g. `cue_clamped`), each kind in printed order.
pub fn compile_warnings(stdout: &[String]) -> Vec<String> {
    let mut story = Vec::new();
    let mut other = Vec::new();
    for line in stdout {
        let Some(rest) = line.trim().strip_prefix("warning[") else {
            continue;
        };
        let Some((code, message)) = rest.split_once("]:") else {
            continue;
        };
        let mut message = message.trim().to_string();
        if code == WARN_MUSIC_FIT {
            // "…, but this story reads X; using Y music as asked": the reply
            // keeps the part that says what conflicts.
            if let Some((conflict, _)) = message.split_once("; using ") {
                message = conflict.to_string();
            }
        }
        if STORY_WARNINGS.contains(&code) {
            story.push(message);
        } else {
            other.push(message);
        }
    }
    story.extend(other);
    story
}

/// (0.23) The reel's music decision as one finding, from its
/// `[3/5] music: <bed> (<reason>)` line: "music: tech_pulse (neutral
/// explainer, money story)", or "music: none (no bed fits a somber story;
/// silence)". A bare `music: none` (no music asked for, nothing to explain)
/// gives no finding.
pub fn music_finding(stdout: &[String]) -> Option<String> {
    let rest = stdout
        .iter()
        .find_map(|l| l.trim_start().strip_prefix("[3/5] music:"))?
        .trim();
    (!rest.is_empty() && rest != "none").then(|| format!("music: {rest}"))
}

/// The QA verdict and ≤ 5 one-line findings (failures first) from
/// `motion-engine qa … --speech … --json` (fields used: `verdict`,
/// `checks[].name`, `checks[].status`, `checks[].detail`).
pub fn parse_qa(json: &str) -> Result<(Qa, Vec<String>), String> {
    let v: Value = serde_json::from_str(json.trim())
        .map_err(|e| format!("qa printed no JSON report ({e})"))?;
    let verdict = v
        .get("verdict")
        .and_then(Value::as_str)
        .ok_or("qa report has no verdict")?;
    let qa = if verdict.eq_ignore_ascii_case("pass") {
        Qa::Pass
    } else {
        Qa::Fail
    };
    let checks = v
        .get("checks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let line = |c: &Value| {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("check");
        let detail = c.get("detail").and_then(Value::as_str).unwrap_or("");
        if detail.is_empty() {
            format!("qa {name} failed")
        } else {
            format!("qa {name}: {}", detail.trim())
        }
    };
    let mut findings: Vec<String> = checks
        .iter()
        .filter(|c| c.get("status").and_then(Value::as_str) == Some("fail"))
        .map(line)
        .collect();
    if qa == Qa::Fail && findings.is_empty() {
        findings.push(format!("qa verdict {verdict}"));
    }
    findings.truncate(MAX_LINES);
    Ok((qa, findings))
}

fn run_qa(
    config: &ServerConfig,
    reel_dir: &Path,
    title: &str,
    video: &Path,
    log: &Log,
) -> Result<(Qa, Vec<String>), String> {
    let stem = stem_of(title);
    let file = |ext: &str| reel_dir.join(format!("{stem}.{ext}.json"));
    let mut cmd = config.engine_command();
    cmd.arg("qa")
        .arg(file("motion"))
        .arg("--speech")
        .arg(file("speech"))
        .arg("--audio-plan")
        .arg(file("audio"))
        .arg("--mixed")
        .arg(video)
        .arg("--json");
    log.line("$ motion-engine qa … --json");
    let out = output(cmd).map_err(|e| format!("cannot start motion-engine: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    log.text(&stdout);
    log.text(&stderr);
    if !out.status.success() {
        let lines = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
        return Err(one_line_error(
            None,
            &lines(&stderr),
            &lines(&stdout),
            out.status.code(),
        ));
    }
    parse_qa(&stdout)
}

/// The preview: short side 720 (never upscaled), H.264 + AAC, faststart,
/// re-encoded at a higher crf while it is over 30 MB.
fn make_preview(video: &Path, out: &Path, log: &Log) -> Result<(), String> {
    let scale = "scale='if(lte(iw,ih),trunc(min(720,iw)/2)*2,-2)':'if(lte(iw,ih),-2,trunc(min(720,ih)/2)*2)'";
    for crf in PREVIEW_CRFS {
        let mut cmd = Command::new(tool("ffmpeg"));
        cmd.args(["-nostdin", "-v", "error", "-y", "-i"])
            .arg(video)
            .args(["-map", "0:v:0", "-map", "0:a?", "-vf", scale])
            .args(["-c:v", "libx264", "-preset", "veryfast", "-crf"])
            .arg(crf.to_string())
            .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "128k"])
            .args(["-movflags", "+faststart"])
            .arg(out);
        log.line(&format!("$ ffmpeg … -crf {crf} {}", out.display()));
        let res = output(cmd).map_err(|e| format!("cannot start ffmpeg: {e}"))?;
        if !res.status.success() {
            let err = String::from_utf8_lossy(&res.stderr).into_owned();
            log.text(&err);
            let lines: Vec<String> = err.lines().map(str::to_string).collect();
            return Err(one_line_error(None, &lines, &[], res.status.code()));
        }
        let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
        if size <= PREVIEW_MAX_BYTES {
            break;
        }
    }
    Ok(())
}

/// Duration in seconds (0.1 s) via ffprobe.
pub fn probe_duration(video: &Path) -> Option<f64> {
    let mut cmd = Command::new(tool("ffprobe"));
    cmd.args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(video);
    let out = output(cmd).ok()?;
    let d: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    (d.is_finite() && d > 0.0).then(|| (d * 10.0).round() / 10.0)
}

/// Hard link `src` at `dst` (replacing it), else copy.
pub fn link_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.symlink_metadata().is_ok() {
        std::fs::remove_file(dst)?;
    }
    match std::fs::hard_link(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::copy(src, dst).map(|_| ()),
    }
}

/// A command's output with stdin closed (stdout / stderr captured).
fn output(mut cmd: Command) -> std::io::Result<std::process::Output> {
    with_tool_path(&mut cmd);
    cmd.stdin(Stdio::null()).output()
}

/// MCP clients often start servers with a minimal PATH; the engine and ffmpeg
/// live in the usual Homebrew / local prefixes.
const EXTRA_PATH: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

fn with_tool_path(cmd: &mut Command) {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&current).collect();
    let mut changed = false;
    for extra in EXTRA_PATH {
        if !dirs.iter().any(|d| d == Path::new(extra)) && Path::new(extra).is_dir() {
            dirs.push(PathBuf::from(extra));
            changed = true;
        }
    }
    if changed {
        if let Ok(path) = std::env::join_paths(dirs) {
            cmd.env("PATH", path);
        }
    }
}

/// `name` from PATH, else from the usual prefixes, else `name` itself.
fn tool(name: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(EXTRA_PATH.iter().map(PathBuf::from))
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// The engine log (`log.txt`): every line scrubbed of anything key-like.
struct Log {
    file: Mutex<Option<std::fs::File>>,
}

impl Log {
    fn open(path: &Path) -> Log {
        let file = OpenOptions::new().create(true).append(true).open(path).ok();
        Log {
            file: Mutex::new(file),
        }
    }

    fn line(&self, line: &str) {
        if let Some(f) = lock(&self.file).as_mut() {
            let _ = writeln!(f, "{}", scrub(line));
        }
    }

    fn text(&self, text: &str) {
        for l in text.lines() {
            self.line(l);
        }
    }
}

struct Streamed {
    success: bool,
    code: Option<i32>,
    /// The last [`TAIL_LINES`] lines of each stream (all `[n/5]` / `video:`
    /// lines of stdout are kept).
    stdout: Vec<String>,
    stderr: Vec<String>,
}

/// Run `cmd` in its own process group with stdin closed and both output
/// streams piped into the log; `on_line` sees every stdout line as it comes.
fn run_streaming(
    mut cmd: Command,
    log: &Log,
    cancel: &Arc<Cancel>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Streamed, String> {
    with_tool_path(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start motion-engine: {e}"))?;
    cancel.register(child.id());
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();
    let (stdout_tail, stderr_tail) = std::thread::scope(|s| {
        let err_thread = s.spawn(move || {
            let mut tail = Vec::new();
            if let Some(err) = stderr {
                for_each_line(err, &mut |l| {
                    log.line(l);
                    push_tail(&mut tail, l);
                });
            }
            tail
        });
        let mut tail = Vec::new();
        if let Some(out) = stdout {
            for_each_line(out, &mut |l| {
                log.line(l);
                on_line(l);
                push_tail(&mut tail, l);
            });
        }
        (tail, err_thread.join().unwrap_or_default())
    });
    let status = child.wait();
    cancel.unregister();
    let status = status.map_err(|e| format!("motion-engine did not finish: {e}"))?;
    Ok(Streamed {
        success: status.success(),
        code: status.code(),
        stdout: stdout_tail,
        stderr: stderr_tail,
    })
}

fn for_each_line(stream: impl Read, f: &mut dyn FnMut(&str)) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                f(line.trim_end_matches(['\n', '\r']));
            }
        }
    }
}

/// Keep the last [`TAIL_LINES`] lines (stage and `video:` lines are never dropped).
fn push_tail(tail: &mut Vec<String>, line: &str) {
    tail.push(line.to_string());
    if tail.len() > TAIL_LINES * 2 {
        let cut = tail.len() - TAIL_LINES;
        let (old, recent) = tail.split_at(cut);
        let mut kept: Vec<String> = old
            .iter()
            .filter(|l| stage_of_line(l).is_some() || video_of_line(l).is_some())
            .cloned()
            .collect();
        kept.extend_from_slice(recent);
        *tail = kept;
    }
}

fn show_args(args: &[OsString]) -> String {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One line saying why the engine failed: the last meaningful error line of
/// stderr (else stdout), prefixed with the stage, scrubbed of keys.
pub fn one_line_error(
    stage: Option<Stage>,
    stderr: &[String],
    stdout: &[String],
    code: Option<i32>,
) -> String {
    const ERRORISH: [&str; 14] = [
        "error",
        "failed",
        "cannot",
        "can't",
        "not found",
        "missing",
        "invalid",
        "denied",
        "unauthorized",
        "panicked",
        "refused",
        "timed out",
        "no such",
        "not set",
    ];
    let meaningful = |l: &&String| {
        let t = l.trim();
        !t.is_empty()
            && !t.starts_with("Stack backtrace")
            && !t.starts_with("note: run with")
            && !t.starts_with("at ")
            && t != "Caused by:"
            && !t.chars().next().is_some_and(|c| c.is_ascii_digit())
            && stage_of_line(t).is_none()
    };
    let errorish = |l: &&String| {
        let low = l.to_lowercase();
        ERRORISH.iter().any(|w| low.contains(w))
    };
    let pick = |lines: &[String]| -> Option<String> {
        let m: Vec<&String> = lines.iter().filter(meaningful).collect();
        let specific = m.iter().rev().find(|l| {
            let t = l.trim().trim_end_matches(':');
            // "Error: voice failed:" only names the step; prefer what follows.
            errorish(l) && !(t.ends_with("failed") && t.split_whitespace().count() <= 3)
        });
        specific
            .or_else(|| m.iter().rev().find(|l| errorish(l)))
            .or_else(|| m.last())
            .map(|l| l.trim().to_string())
    };
    let line = pick(stderr)
        .or_else(|| pick(stdout))
        .unwrap_or_else(|| match code {
            Some(c) => format!("exit code {c}"),
            None => "stopped by a signal".to_string(),
        });
    let mut line = line
        .trim_start_matches("Error:")
        .trim_start_matches("error:")
        .trim()
        .trim_end_matches(':')
        .to_string();
    if let Some(s) = stage {
        if !line.to_lowercase().contains(s.name()) {
            line = format!("{}: {line}", s.name());
        }
    }
    scrub(&line.replace(['\n', '\r'], " "))
}

/// Replace anything that looks like a secret (`sk-…` keys, `Bearer …`,
/// `…KEY=…`) with `[redacted]`.
pub fn scrub(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut redact_next = false;
    for tok in s.split(' ') {
        if redact_next && !tok.is_empty() {
            out.push("[redacted]".to_string());
            redact_next = false;
            continue;
        }
        if tok.eq_ignore_ascii_case("bearer") || tok.eq_ignore_ascii_case("bearer:") {
            redact_next = true;
            out.push(tok.to_string());
            continue;
        }
        let mut t = redact_sk(tok);
        if let Some(i) = t.find("KEY=").or_else(|| t.find("key=")) {
            if t.len() > i + 4 {
                t = format!("{}[redacted]", &t[..i + 4]);
            }
        }
        out.push(t);
    }
    out.join(" ")
}

/// `sk-…` runs (OpenRouter `sk-or-…` and similar) inside one token.
fn redact_sk(tok: &str) -> String {
    let mut out = String::new();
    let mut rest = tok;
    while let Some(i) = rest.find("sk-") {
        let before_ok = rest[..i]
            .chars()
            .last()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        let run_len = rest[i..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len() - i);
        if before_ok && run_len >= 10 {
            out.push_str(&rest[..i]);
            out.push_str("[redacted]");
        } else {
            out.push_str(&rest[..i + run_len.max(3)]);
        }
        rest = &rest[i + run_len.max(3)..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::StoryKind;
    use serde_json::json;

    /// The real shape of `motion-engine qa … --speech … --json` (0.20).
    const REAL_QA: &str = r#"{
      "verdict": "FAIL",
      "checks": [
        {"name": "caption_timing", "status": "pass", "detail": "250 word(s), max delta 0.5 ms (limit 33.3 ms), 0 finding(s)"},
        {"name": "caption_safe_area", "status": "pass", "detail": "0 finding(s)"},
        {"name": "layout", "status": "pass", "detail": "0 finding(s)"},
        {"name": "sfx_vs_words", "status": "pass", "detail": "23 cue(s), 0 within 80 ms of a word onset"},
        {"name": "loudness", "status": "pass", "detail": "-16.1 LUFS (target -16 +-1)"},
        {"name": "peak", "status": "fail", "detail": "true -0.7 dBTP, sample -0.7 dBFS (limit -1.0, codec margin 0.3)"},
        {"name": "bed_duck", "status": "skip", "detail": "no music"}
      ],
      "max_caption_delta_ms": 0.49854651162206665,
      "caption_limit_ms": 33.333333333333336,
      "words": 250,
      "sfx_conflicts": [],
      "integrated_lufs": -16.1,
      "true_peak_db": -0.7,
      "sample_peak_db": -0.6776980807919154,
      "bed_duck_db": null
    }"#;

    #[test]
    fn stages_come_from_the_reel_lines() {
        let log = "[1/5] voice (auto · emotion energy)
  voice: model fish-audio/s2.1-pro-free:free voice '536d' (emotion energy, single take, offline=true)
[3/5] music: bright_upbeat (emotion energy)
[2/5] compile (art auto, variety auto)
  compiled 13 beat(s) -> /x/five.motion.json (15 scenes, 0 shared, 102.41s)
[4/5] sound design + render
[5/5] speech QA
  FAIL peak: true -0.7 dBTP
video: /x/five_wait_what/five_wait_what.mp4";
        let stages: Vec<Stage> = log.lines().filter_map(stage_of_line).collect();
        assert_eq!(
            stages,
            [
                Stage::Voice,
                Stage::Music,
                Stage::Compile,
                Stage::Render,
                Stage::Qa
            ]
        );
        let video: Vec<PathBuf> = log.lines().filter_map(video_of_line).collect();
        assert_eq!(
            video,
            [PathBuf::from("/x/five_wait_what/five_wait_what.mp4")]
        );
        assert_eq!(stage_of_line("  voice: model x"), None);
        assert_eq!(stage_of_line("[9/5] nope"), None);
    }

    #[test]
    fn qa_json_gives_verdict_and_failures_first() {
        let (qa, findings) = parse_qa(REAL_QA).unwrap();
        assert_eq!(qa, Qa::Fail);
        assert_eq!(
            findings,
            ["qa peak: true -0.7 dBTP, sample -0.7 dBFS (limit -1.0, codec margin 0.3)"]
        );
        let pass = json!({"verdict": "PASS", "checks": [{"name": "layout", "status": "pass", "detail": "ok"}]});
        assert_eq!(parse_qa(&pass.to_string()).unwrap(), (Qa::Pass, vec![]));
        assert!(parse_qa("speech qa: PASS").is_err());
        let bare = json!({"verdict": "FAIL", "checks": []});
        assert_eq!(parse_qa(&bare.to_string()).unwrap().1, ["qa verdict FAIL"]);
    }

    #[test]
    fn errors_are_one_scrubbed_line() {
        let stderr: Vec<String> = [
            "Error: voice failed:",
            "  [stdout of voice]",
            "Error: provider returned 401 Unauthorized for key sk-or-v1-0123456789abcdef",
            "",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let line = one_line_error(Some(Stage::Voice), &stderr, &[], Some(1));
        assert_eq!(
            line,
            "voice: provider returned 401 Unauthorized for key [redacted]"
        );
        assert!(!line.contains('\n'));
        assert_eq!(
            one_line_error(Some(Stage::Render), &[], &[], Some(3)),
            "render: exit code 3"
        );
        assert_eq!(
            scrub("Authorization: Bearer abc.def OPENROUTER_API_KEY=xyz task-runner sk-short"),
            "Authorization: Bearer [redacted] OPENROUTER_API_KEY=[redacted] task-runner sk-short"
        );
        assert_eq!(scrub("(sk-or-v1-aaaaaaaaaaaa)"), "([redacted])");
    }

    #[test]
    fn reel_flags_follow_the_options() {
        let mut config = ServerConfig::new(Profile::Creator, "/repo");
        config.reel_extra = vec!["--offline".into()];
        let mut request = StoredRequest {
            profile: Profile::Creator,
            kind: StoryKind::Lite,
            story: json!({}),
            style: json!({}),
            format: "vertical".into(),
            assets: None,
            options: ProductionOptions::default(),
            take: 0,
            music: MusicWord::Auto,
        };
        let job = |request: &StoredRequest, manifest: bool| EngineJob {
            id: JobId::parse("j_0123456789").unwrap(),
            dir: PathBuf::from("/repo/output/jobs/j_0123456789"),
            request: request.clone(),
            title: "t".into(),
            has_manifest: manifest,
        };
        let show = |a: Vec<OsString>| show_args(&a);
        assert_eq!(
            show(reel_args(&config, &job(&request, false))),
            "reel /repo/output/jobs/j_0123456789/intent.json --style /repo/output/jobs/j_0123456789/style.json \
             -o /repo/output/jobs/j_0123456789/reel --assets /repo/assets --tts-model auto --music auto --art auto --offline"
        );
        request.options = ProductionOptions {
            art: Some("dossier".into()),
            families: vec!["clay".into(), "paper".into()],
            music: Some("none".into()),
            captions: Some(false),
            aspect: Some("4:5".into()),
            variety: Some(json!("off")),
            tts_model: Some("gemini".into()),
            keep_frames: true,
        };
        let creator = show(reel_args(&config, &job(&request, true)));
        assert!(creator.contains("--music none --art dossier --no-captions --no-variety"));
        assert!(creator.contains("--asset-family clay --asset-family paper --aspect 4:5"));
        assert!(creator
            .contains("--asset-manifest /repo/output/jobs/j_0123456789/assets.manifest.json"));
        // Paid voice and kept frames are operator-only.
        assert!(creator.contains("--tts-model auto") && !creator.contains("--keep-frames"));
        request.profile = Profile::Operator;
        let operator = show(reel_args(&config, &job(&request, false)));
        assert!(operator.contains("--tts-model gemini") && operator.contains("--keep-frames"));
        request.options.variety = Some(json!(42));
        assert!(!show(reel_args(&config, &job(&request, false))).contains("variety"));
        assert_eq!(
            variety_note(&request.options).as_deref(),
            Some("variety seed applies from Phase 2")
        );
        assert_eq!(stem_of("city water!"), "city_water_");
    }

    #[test]
    fn reel_gets_the_take_only_when_it_is_not_zero() {
        let mut config = ServerConfig::new(Profile::Weak, "/repo");
        config.reel_extra = vec!["--offline".into()];
        let mut request = StoredRequest {
            profile: Profile::Weak,
            kind: StoryKind::Lite,
            story: json!({}),
            style: json!({}),
            format: "vertical".into(),
            assets: None,
            options: ProductionOptions::default(),
            take: 0,
            music: MusicWord::Auto,
        };
        let job = |request: &StoredRequest| EngineJob {
            id: JobId::parse("j_0123456789").unwrap(),
            dir: PathBuf::from("/repo/output/jobs/j_0123456789"),
            request: request.clone(),
            title: "t".into(),
            has_manifest: false,
        };
        let line = |request: &StoredRequest| show_args(&reel_args(&config, &job(request)));
        // Take 0 is today's command line.
        assert_eq!(
            line(&request),
            "reel /repo/output/jobs/j_0123456789/intent.json --style /repo/output/jobs/j_0123456789/style.json \
             -o /repo/output/jobs/j_0123456789/reel --assets /repo/assets --tts-model auto --music auto --art auto --offline"
        );
        assert!(!line(&request).contains("--take"));
        request.take = 2;
        let with = line(&request);
        assert!(with.contains(" --art auto --take 2 --offline"), "{with}");
        assert_eq!(with.matches("--take").count(), 1);
        request.take = 17;
        assert!(line(&request).contains("--take 17"));
    }

    #[test]
    fn compile_warnings_story_first() {
        let out: Vec<String> = [
            "[2/5] compile (art auto, variety auto)",
            "  warning[cue_clamped]: beat 3: \"evidence\" could not move fully onto \"years\"",
            "  warning[title_states_conclusion]: beat 3: title \"first 20 years\" says \"20\" 4.0 s before the narrator does",
            "  compiled 4 beat(s)",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            compile_warnings(&out),
            vec![
                "beat 3: title \"first 20 years\" says \"20\" 4.0 s before the narrator does"
                    .to_string(),
                "beat 3: \"evidence\" could not move fully onto \"years\"".to_string(),
            ]
        );
    }
}
