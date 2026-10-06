//! (1b) The job store: `output/jobs/<id>/` files ([`crate::job`]), status
//! reads and writes, the one-render-at-a-time queue and retention pruning.
//!
//! * `status.json` is written atomically (tmp + rename) on every change.
//! * One worker thread renders queued jobs in FIFO order; waiters are woken on
//!   every status change (progress) and when a job finishes.
//! * Several servers may share one jobs folder (e.g. a weak and a creator
//!   server registered on the same repository):
//!   - a queued / running job records its server's pid (`owner_pid`);
//!   - a job another live server owns is waited on (by polling its
//!     `status.json`), never re-run here ([`Submitted::Foreign`]);
//!   - renders stay one at a time across servers: the worker holds an
//!     exclusive advisory lock on `<jobs>/.render.lock` while it renders;
//!     queue positions are per server.
//! * On start, jobs left `queued` / `running` by a server that is gone (its
//!   pid is not alive) become `failed` "interrupted (server restarted)";
//!   asking for them again re-runs them (a failed job is re-run when
//!   requested again). Another live server's jobs are left alone.
//! * Retention after each finished job: the newest `retention.max_jobs` job
//!   directories and at most `retention.max_bytes` in total, oldest deleted
//!   first. Only directories directly in the jobs root whose names are job ids
//!   are considered; queued / running jobs and jobs of a live owner are never
//!   touched.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::engine::{Cancel, Engine, EngineJob, EngineOutput};
use crate::job::{JobId, JobState, JobStatus, Stage, PREVIEW_MP4, STATUS_JSON, VIDEO_MP4};
use crate::profile::{Retention, ServerConfig};
use crate::reply::MAX_LINES;

/// Held (advisory, exclusive) by whichever server is rendering.
pub const RENDER_LOCK: &str = ".render.lock";
/// Error of a job left behind by a server that is gone.
pub const INTERRUPTED: &str = "interrupted (server restarted)";
/// Error of a job stopped because the server is stopping.
pub const STOPPED: &str = "interrupted (server stopped)";
/// Error of a job whose caller cancelled the request.
pub const CANCELLED: &str = "cancelled";
/// Longest a waiter sleeps before re-checking its job and its cancellation.
const POLL: Duration = Duration::from_millis(200);
/// How often a waiter on another server's job checks that server is alive.
const OWNER_CHECK: Duration = Duration::from_secs(1);
/// How often the worker retries the render lock.
const RENDER_LOCK_RETRY: Duration = Duration::from_millis(250);
/// Extra time a cancelling waiter gives the render to stop.
const CANCEL_GRACE: Duration = Duration::from_secs(10);

/// What [`Store::submit`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Submitted {
    /// Written and queued here.
    Queued,
    /// The same job is already queued or running here.
    Active,
    /// Another live server has the same job queued or running.
    Foreign,
    /// The same job is already done and its video exists.
    Done(Box<JobStatus>),
}

#[derive(Default)]
struct State {
    queue: VecDeque<EngineJob>,
    /// Statuses of this server's queued and running jobs (mirrors their
    /// `status.json`).
    active: HashMap<JobId, JobStatus>,
    running: Option<(JobId, Arc<Cancel>)>,
    /// Calls currently waiting, per job.
    waiters: HashMap<JobId, usize>,
    shutdown: bool,
}

pub struct Store {
    config: ServerConfig,
    engine: Arc<dyn Engine>,
    state: Mutex<State>,
    changed: Condvar,
}

/// Seconds since the epoch.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// True when process `pid` exists (`kill -0`; this process counts).
pub fn pid_alive(pid: u32) -> bool {
    pid == std::process::id()
        || Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// True when a job's owning server is alive.
fn owner_alive(status: &JobStatus) -> bool {
    status.owner_pid.is_some_and(pid_alive)
}

fn in_flight(status: &JobStatus) -> bool {
    matches!(status.state, JobState::Queued | JobState::Running)
}

/// A path as replies show it: repository-relative when it is inside the
/// repository, else absolute.
pub fn shown_path(config: &ServerConfig, path: &Path) -> String {
    match path.strip_prefix(&config.repo) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// The file behind a path from a reply or `status.json`.
pub fn resolve_path(config: &ServerConfig, shown: &str) -> PathBuf {
    let p = Path::new(shown);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        config.repo.join(p)
    }
}

/// Read `<dir>/status.json`.
pub fn read_status(dir: &Path) -> Option<JobStatus> {
    let text = std::fs::read_to_string(dir.join(STATUS_JSON)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write `<dir>/status.json` atomically (tmp + rename; the tmp name is
/// per process, so two servers never share it).
pub fn write_status(dir: &Path, status: &JobStatus) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(status).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{STATUS_JSON}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join(STATUS_JSON))
}

/// Mark an in-flight status failed with `error` (owner cleared).
fn interrupt(mut s: JobStatus, error: &str) -> JobStatus {
    s.state = JobState::Failed;
    s.error = Some(error.to_string());
    s.finished_unix = Some(now_unix());
    s.owner_pid = None;
    s
}

impl Store {
    /// Open the store under `config.jobs`: create it, fail the jobs a server
    /// that is gone left queued or running, and start the render worker.
    pub fn open(config: ServerConfig, engine: Arc<dyn Engine>) -> std::io::Result<Arc<Store>> {
        std::fs::create_dir_all(&config.jobs)?;
        let me = std::process::id();
        for (id, dir) in job_dirs(&config.jobs) {
            let Some(s) = read_status(&dir) else {
                continue;
            };
            // Our own pid on disk can only be a dead server's (pid reuse):
            // this process has not queued anything yet.
            let other_live = s.owner_pid.is_some_and(|p| p != me && pid_alive(p));
            if in_flight(&s) && !other_live {
                if let Err(e) = write_status(&dir, &interrupt(s, INTERRUPTED)) {
                    eprintln!("motion-mcp: cannot update {id}: {e}");
                }
            }
        }
        let store = Arc::new(Store {
            config,
            engine,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let worker = Arc::clone(&store);
        std::thread::Builder::new()
            .name("motion-mcp-render".into())
            .spawn(move || worker.work())?;
        Ok(store)
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn dir(&self, id: &JobId) -> PathBuf {
        id.dir(&self.config.jobs)
    }

    /// The job's status: in memory while queued or running here, else
    /// `status.json`.
    pub fn status(&self, id: &JobId) -> Option<JobStatus> {
        let st = self.lock();
        self.status_locked(&st, id)
    }

    fn status_locked(&self, st: &State, id: &JobId) -> Option<JobStatus> {
        st.active
            .get(id)
            .cloned()
            .or_else(|| read_status(&self.dir(id)))
    }

    /// 1-based place in this server's queue of a queued job.
    pub fn queue_position(&self, id: &JobId) -> Option<usize> {
        position(&self.lock(), id)
    }

    /// Queue `job` unless the same job is already queued or running (here or
    /// in another live server) or done (with its video on disk; `force`
    /// re-renders a done job). `write` puts the job's input files into its
    /// directory first.
    pub fn submit(
        &self,
        job: EngineJob,
        initial: JobStatus,
        force: bool,
        write: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<Submitted, String> {
        let mut st = self.lock();
        if st.shutdown {
            return Err("the server is stopping".to_string());
        }
        if st.active.contains_key(&job.id) {
            return Ok(Submitted::Active);
        }
        let dir = self.dir(&job.id);
        if let Some(s) = read_status(&dir) {
            let video_ok = s
                .video
                .as_deref()
                .is_some_and(|v| resolve_path(&self.config, v).is_file());
            if s.state == JobState::Done && video_ok && !force {
                return Ok(Submitted::Done(Box::new(s)));
            }
            if in_flight(&s) && owner_alive(&s) {
                return Ok(Submitted::Foreign);
            }
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create the job: {e}"))?;
        for stale in [VIDEO_MP4, PREVIEW_MP4] {
            let _ = std::fs::remove_file(dir.join(stale));
        }
        let wrote = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write(&dir)))
            .unwrap_or_else(|_| Err("writing the job's files failed".to_string()));
        wrote?;
        let mut status = initial;
        status.job = job.id.clone();
        status.state = JobState::Queued;
        status.owner_pid = Some(std::process::id());
        write_status(&dir, &status).map_err(|e| format!("cannot write the job status: {e}"))?;
        st.active.insert(job.id.clone(), status);
        st.queue.push_back(job);
        drop(st);
        self.changed.notify_all();
        Ok(Submitted::Queued)
    }

    /// Wait up to `timeout` for job `id` to finish. `on_change` sees the
    /// status (and queue position) at the start and after every change.
    /// Another server's job is followed through its `status.json`; if that
    /// server dies, the job is marked failed.
    ///
    /// When `cancelled()` turns true: if `may_cancel` (this call submitted the
    /// job) and no other call is waiting on it, the job is cancelled and the
    /// wait goes on (a little longer) until it has stopped; otherwise the call
    /// just stops waiting. `None` = unknown job.
    pub fn wait(
        &self,
        id: &JobId,
        timeout: Duration,
        may_cancel: bool,
        cancelled: &dyn Fn() -> bool,
        on_change: &mut dyn FnMut(&JobStatus, Option<usize>),
    ) -> Option<(JobStatus, Option<usize>)> {
        *self.lock().waiters.entry(id.clone()).or_default() += 1;
        let out = self.wait_registered(id, timeout, may_cancel, cancelled, on_change);
        let mut st = self.lock();
        if let Some(n) = st.waiters.get_mut(id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                st.waiters.remove(id);
            }
        }
        out
    }

    /// Calls currently waiting on job `id`.
    pub fn waiting(&self, id: &JobId) -> usize {
        self.lock().waiters.get(id).copied().unwrap_or(0)
    }

    fn wait_registered(
        &self,
        id: &JobId,
        timeout: Duration,
        may_cancel: bool,
        cancelled: &dyn Fn() -> bool,
        on_change: &mut dyn FnMut(&JobStatus, Option<usize>),
    ) -> Option<(JobStatus, Option<usize>)> {
        let mut deadline = Instant::now() + timeout;
        let mut asked_to_cancel = false;
        let mut owner_checked = Instant::now();
        let mut st = self.lock();
        loop {
            let mut status = self.status_locked(&st, id)?;
            let pos = position(&st, id);
            // Another server's job whose server is gone: it will not finish.
            if in_flight(&status)
                && !st.active.contains_key(id)
                && owner_checked.elapsed() >= OWNER_CHECK
            {
                owner_checked = Instant::now();
                if !owner_alive(&status) {
                    status = interrupt(status, STOPPED);
                    self.persist(&status);
                }
            }
            on_change(&status, pos);
            if !in_flight(&status) {
                return Some((status, pos));
            }
            if !asked_to_cancel && cancelled() {
                asked_to_cancel = true;
                let others = st.waiters.get(id).copied().unwrap_or(1).saturating_sub(1);
                if !may_cancel || others > 0 {
                    return Some((status, pos));
                }
                deadline = deadline.max(Instant::now() + CANCEL_GRACE);
                self.cancel_locked(&mut st, id, CANCELLED);
                continue;
            }
            let now = Instant::now();
            if now >= deadline {
                return Some((status, pos));
            }
            let nap = (deadline - now).min(POLL);
            st = self
                .changed
                .wait_timeout(st, nap)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    /// Wake every waiter (e.g. after a caller's cancellation).
    pub fn wake(&self) {
        self.changed.notify_all();
    }

    /// Cancel this server's queued (dropped from the queue) or running
    /// (process group stopped) job; it ends `failed` with `reason`.
    pub fn cancel(&self, id: &JobId, reason: &str) {
        let mut st = self.lock();
        self.cancel_locked(&mut st, id, reason);
    }

    fn cancel_locked(&self, st: &mut State, id: &JobId, reason: &str) {
        if let Some(i) = st.queue.iter().position(|j| &j.id == id) {
            st.queue.remove(i);
            if let Some(s) = st.active.remove(id) {
                self.persist(&interrupt(s, reason));
            }
        } else if let Some((running, cancel)) = &st.running {
            if running == id {
                cancel.cancel(reason);
            }
        }
        self.changed.notify_all();
    }

    /// Stop: fail every queued job, stop the running one (waiting up to a few
    /// seconds for it to wind down) and end the worker.
    pub fn shutdown(&self) {
        let mut st = self.lock();
        st.shutdown = true;
        while let Some(job) = st.queue.pop_front() {
            if let Some(s) = st.active.remove(&job.id) {
                self.persist(&interrupt(s, STOPPED));
            }
        }
        if let Some((_, cancel)) = &st.running {
            cancel.cancel(STOPPED);
        }
        self.changed.notify_all();
        let deadline = Instant::now() + Duration::from_secs(6);
        while st.running.is_some() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            st = self
                .changed
                .wait_timeout(st, (deadline - now).min(POLL))
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    fn persist(&self, status: &JobStatus) {
        if let Err(e) = write_status(&self.dir(&status.job), status) {
            eprintln!("motion-mcp: cannot write status of {}: {e}", status.job);
        }
    }

    /// The render worker: one job at a time, FIFO; one render at a time
    /// across every server on this jobs folder.
    fn work(self: Arc<Self>) {
        loop {
            let (job, cancel) = {
                let mut st = self.lock();
                let job = loop {
                    if st.shutdown {
                        return;
                    }
                    if let Some(job) = st.queue.pop_front() {
                        break job;
                    }
                    st = self.changed.wait(st).unwrap_or_else(|e| e.into_inner());
                };
                let cancel = Cancel::new();
                st.running = Some((job.id.clone(), Arc::clone(&cancel)));
                drop(st);
                self.changed.notify_all();
                (job, cancel)
            };
            // The job stays `queued` while another server renders.
            let render_lock = match self.render_lock(&cancel) {
                Ok(lock) => lock,
                Err(e) => {
                    self.finish(&job, Err(e), &cancel);
                    continue;
                }
            };
            if self.adopt_if_done_elsewhere(&job) {
                continue;
            }
            self.set_running(&job.id);
            let set_stage = |stage: Stage| self.set_stage(&job.id, stage);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.engine.run(&self.config, &job, &set_stage, &cancel)
            }))
            .unwrap_or_else(|_| Err("the engine runner crashed".to_string()));
            drop(render_lock);
            self.finish(&job, result, &cancel);
            let pruned = self.prune();
            if !pruned.is_empty() {
                eprintln!("motion-mcp: retention removed {} job(s)", pruned.len());
            }
        }
    }

    /// Take `<jobs>/.render.lock` (released when the file is dropped),
    /// waiting while another server renders; a cancellation stops the wait.
    fn render_lock(&self, cancel: &Cancel) -> Result<std::fs::File, String> {
        let path = self.config.jobs.join(RENDER_LOCK);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        loop {
            if let Some(reason) = cancel.reason() {
                return Err(reason);
            }
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(format!("cannot lock {}: {e}", path.display()))
                }
            }
            std::thread::sleep(RENDER_LOCK_RETRY);
        }
    }

    /// Two servers that queued the same job at the same moment: the second
    /// finds it done once it gets the render lock and takes that result.
    fn adopt_if_done_elsewhere(&self, job: &EngineJob) -> bool {
        let Some(s) = read_status(&job.dir) else {
            return false;
        };
        let video_ok = s
            .video
            .as_deref()
            .is_some_and(|v| resolve_path(&self.config, v).is_file());
        if s.state != JobState::Done || !video_ok {
            return false;
        }
        let mut st = self.lock();
        st.active.remove(&job.id);
        st.running = None;
        drop(st);
        self.changed.notify_all();
        true
    }

    fn set_running(&self, id: &JobId) {
        let mut st = self.lock();
        if let Some(s) = st.active.get_mut(id) {
            s.state = JobState::Running;
            s.stage = Some(Stage::Check);
            s.percent = Some(Stage::Check.start_percent());
            s.owner_pid = Some(std::process::id());
            let s = s.clone();
            self.persist(&s);
        }
        drop(st);
        self.changed.notify_all();
    }

    fn set_stage(&self, id: &JobId, stage: Stage) {
        let mut st = self.lock();
        let Some(s) = st.active.get_mut(id) else {
            return;
        };
        if s.stage == Some(stage) {
            return;
        }
        s.stage = Some(stage);
        s.percent = Some(stage.start_percent());
        let s = s.clone();
        drop(st);
        self.persist(&s);
        self.changed.notify_all();
    }

    fn finish(&self, job: &EngineJob, result: Result<EngineOutput, String>, cancel: &Cancel) {
        let mut st = self.lock();
        let mut s = st
            .active
            .remove(&job.id)
            .or_else(|| read_status(&job.dir))
            .unwrap_or_else(|| JobStatus {
                job: job.id.clone(),
                state: JobState::Running,
                stage: None,
                percent: None,
                video: None,
                preview: None,
                duration_s: None,
                qa: None,
                changed: Vec::new(),
                findings: Vec::new(),
                error: None,
                parent: None,
                created_unix: now_unix(),
                finished_unix: None,
                owner_pid: None,
            });
        s.finished_unix = Some(now_unix());
        s.owner_pid = None;
        let result = match cancel.reason() {
            Some(reason) => Err(reason),
            None => result,
        };
        match result {
            Ok(out) => {
                s.state = JobState::Done;
                s.stage = None;
                s.percent = Some(100);
                s.video = Some(shown_path(&self.config, &out.video));
                s.preview = out.preview.map(|p| shown_path(&self.config, &p));
                s.duration_s = out.duration_s;
                s.qa = out.qa;
                let mut findings = out.findings;
                findings.append(&mut s.findings);
                findings.truncate(MAX_LINES);
                s.findings = findings;
                s.error = None;
            }
            Err(e) => {
                s.state = JobState::Failed;
                s.error = Some(crate::reply::cut(&e));
            }
        }
        self.persist(&s);
        st.running = None;
        drop(st);
        self.changed.notify_all();
    }

    /// Apply the retention policy; returns the removed jobs.
    pub fn prune(&self) -> Vec<JobId> {
        let st = self.lock();
        let keep_active = |id: &JobId| st.active.contains_key(id);
        prune_jobs(&self.config.jobs, self.config.retention, &keep_active)
    }
}

fn position(st: &State, id: &JobId) -> Option<usize> {
    st.queue.iter().position(|j| &j.id == id).map(|i| i + 1)
}

/// The job directories directly in `root` (names that are job ids; real
/// directories, not symlinks).
fn job_dirs(root: &Path) -> Vec<(JobId, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<(JobId, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let id = JobId::parse(name.to_str()?).ok()?;
            if id.as_str() != name.to_str()? {
                return None;
            }
            let path = e.path();
            path.symlink_metadata()
                .ok()
                .filter(|m| m.is_dir())
                .map(|_| (id, path))
        })
        .collect();
    out.sort();
    out
}

/// Bytes under `path` (symlinks are not followed).
fn dir_bytes(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|rd| rd.flatten().map(|e| dir_bytes(&e.path())).sum())
        .unwrap_or(0)
}

/// Keep the newest `retention.max_jobs` job directories and at most
/// `retention.max_bytes` in total (the newest job is always kept); delete the
/// rest, oldest first. Queued / running jobs (`is_active`, or by their
/// `status.json`) and jobs whose owning server is alive are never deleted.
pub fn prune_jobs(
    root: &Path,
    retention: Retention,
    is_active: &dyn Fn(&JobId) -> bool,
) -> Vec<JobId> {
    struct Entry {
        id: JobId,
        dir: PathBuf,
        when: u64,
        bytes: u64,
    }
    let mut entries: Vec<Entry> = job_dirs(root)
        .into_iter()
        .filter_map(|(id, dir)| {
            let status = read_status(&dir);
            let busy = is_active(&id)
                || status
                    .as_ref()
                    .is_some_and(|s| in_flight(s) || owner_alive(s));
            if busy {
                return None;
            }
            let when = status
                .map(|s| s.finished_unix.unwrap_or(s.created_unix))
                .or_else(|| {
                    dir.symlink_metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                })
                .unwrap_or(0);
            let bytes = dir_bytes(&dir);
            Some(Entry {
                id,
                dir,
                when,
                bytes,
            })
        })
        .collect();
    // Newest first; ties by id (deterministic).
    entries.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| b.id.cmp(&a.id)));
    let mut kept = 0usize;
    let mut kept_bytes = 0u64;
    let mut doomed = Vec::new();
    for e in entries {
        let fits = kept_bytes.saturating_add(e.bytes) <= retention.max_bytes;
        if kept < retention.max_jobs && (fits || kept == 0) {
            kept += 1;
            kept_bytes = kept_bytes.saturating_add(e.bytes);
        } else {
            doomed.push(e);
        }
    }
    // Oldest first.
    doomed.reverse();
    let mut removed = Vec::new();
    for e in doomed {
        // `e.dir` is `<root>/<job id>`, a real directory (checked above).
        if e.dir.parent() == Some(root) && std::fs::remove_dir_all(&e.dir).is_ok() {
            removed.push(e.id);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::ProductionOptions;
    use crate::job::{StoredRequest, StoryKind};
    use crate::profile::Profile;
    use crate::reply::Qa;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("motion-mcp-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn status(id: &JobId, state: JobState, created: u64) -> JobStatus {
        JobStatus {
            job: id.clone(),
            state,
            stage: None,
            percent: None,
            video: None,
            preview: None,
            duration_s: None,
            qa: None,
            changed: Vec::new(),
            findings: Vec::new(),
            error: None,
            parent: None,
            created_unix: created,
            finished_unix: None,
            owner_pid: None,
        }
    }

    fn id(n: u32) -> JobId {
        JobId::parse(&format!("j_{n:010x}")).unwrap()
    }

    fn make_job(root: &Path, n: u32, state: JobState, created: u64, bytes: usize) -> PathBuf {
        let dir = id(n).dir(root);
        std::fs::create_dir_all(&dir).unwrap();
        write_status(&dir, &status(&id(n), state, created)).unwrap();
        std::fs::write(dir.join("video.mp4"), vec![0u8; bytes]).unwrap();
        dir
    }

    #[test]
    fn retention_keeps_the_newest_and_never_touches_anything_else() {
        let root = temp("retention");
        for n in 1..=5 {
            make_job(&root, n, JobState::Done, 100 + n as u64, 1000);
        }
        make_job(&root, 6, JobState::Running, 50, 1000);
        make_job(&root, 7, JobState::Failed, 10, 1000);
        // Not jobs: never touched.
        std::fs::create_dir_all(root.join("_cache/images")).unwrap();
        std::fs::create_dir_all(root.join("j_NOTANID00")).unwrap();
        std::fs::write(root.join("notes.txt"), "keep").unwrap();
        let outside = temp("retention-outside");
        std::fs::write(outside.join("precious.txt"), "keep").unwrap();
        let link = root.join("j_00000000ff");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let removed = prune_jobs(
            &root,
            Retention {
                max_jobs: 3,
                max_bytes: u64::MAX,
            },
            &|_| false,
        );
        // Oldest first: 7 (t=10), then 1, 2.
        assert_eq!(removed, [id(7), id(1), id(2)]);
        for n in [3, 4, 5, 6] {
            assert!(id(n).dir(&root).is_dir(), "job {n} kept");
        }
        assert!(root.join("_cache/images").is_dir());
        assert!(root.join("j_NOTANID00").is_dir());
        assert!(root.join("notes.txt").is_file());
        assert!(outside.join("precious.txt").is_file());
        assert!(link.symlink_metadata().is_ok());

        // Byte cap: 2.5 jobs' worth keeps 2; an active job is never removed.
        let removed = prune_jobs(
            &root,
            Retention {
                max_jobs: 50,
                max_bytes: 2500,
            },
            &|j| j == &id(3),
        );
        assert_eq!(removed, Vec::<JobId>::new());
        let removed = prune_jobs(
            &root,
            Retention {
                max_jobs: 50,
                max_bytes: 2500,
            },
            &|_| false,
        );
        assert_eq!(removed, [id(3)]);
        // The newest job survives even when it alone is over the cap.
        let removed = prune_jobs(
            &root,
            Retention {
                max_jobs: 50,
                max_bytes: 10,
            },
            &|_| false,
        );
        assert_eq!(removed, [id(4)]);
        assert!(id(5).dir(&root).is_dir() && id(6).dir(&root).is_dir());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// An engine that "renders" by writing a file, counting runs, and can be
    /// held at the render stage until released or cancelled.
    struct FakeEngine {
        runs: AtomicUsize,
        hold: Mutex<bool>,
        released: Condvar,
    }

    impl Engine for FakeEngine {
        fn run(
            &self,
            _config: &ServerConfig,
            job: &EngineJob,
            stage: &dyn Fn(Stage),
            cancel: &Arc<Cancel>,
        ) -> Result<EngineOutput, String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            stage(Stage::Voice);
            stage(Stage::Render);
            let mut hold = self.hold.lock().unwrap();
            while *hold && !cancel.is_cancelled() {
                hold = self
                    .released
                    .wait_timeout(hold, Duration::from_millis(20))
                    .unwrap()
                    .0;
            }
            if let Some(r) = cancel.reason() {
                return Err(r);
            }
            let video = job.dir.join(VIDEO_MP4);
            std::fs::write(&video, b"mp4").map_err(|e| e.to_string())?;
            Ok(EngineOutput {
                video,
                preview: None,
                duration_s: Some(1.0),
                qa: Some(Qa::Pass),
                findings: vec!["qa note".into()],
            })
        }
    }

    fn engine_job(config: &ServerConfig, n: u32) -> EngineJob {
        EngineJob {
            id: id(n),
            dir: id(n).dir(&config.jobs),
            request: StoredRequest {
                profile: Profile::Weak,
                kind: StoryKind::Lite,
                story: json!({}),
                style: json!({}),
                format: "vertical".into(),
                assets: None,
                options: ProductionOptions::default(),
                take: 0,
                music: Default::default(),
            },
            title: "t".into(),
            has_manifest: false,
        }
    }

    #[test]
    fn queue_runs_one_at_a_time_and_cancels() {
        let repo = temp("queue");
        let config = ServerConfig::new(Profile::Weak, &repo);
        // A job a previous server left running is failed on open.
        let left = id(9).dir(&config.jobs);
        std::fs::create_dir_all(&left).unwrap();
        write_status(&left, &status(&id(9), JobState::Running, 1)).unwrap();
        let engine = Arc::new(FakeEngine {
            runs: AtomicUsize::new(0),
            hold: Mutex::new(true),
            released: Condvar::new(),
        });
        let store = Store::open(config.clone(), engine.clone()).unwrap();
        let s9 = store.status(&id(9)).unwrap();
        assert_eq!(
            (s9.state, s9.error.as_deref()),
            (JobState::Failed, Some(INTERRUPTED))
        );

        let mut initial = status(&id(1), JobState::Queued, 1);
        initial.findings = vec!["prepared note".into()];
        let write =
            |dir: &Path| std::fs::write(dir.join("intent.json"), "{}").map_err(|e| e.to_string());
        for n in 1..=3 {
            let got = store
                .submit(engine_job(&config, n), initial.clone(), false, &write)
                .unwrap();
            assert_eq!(got, Submitted::Queued);
        }
        // Same job again while active: not queued twice.
        assert_eq!(
            store
                .submit(engine_job(&config, 2), initial.clone(), false, &write)
                .unwrap(),
            Submitted::Active
        );
        // Job 1 reaches the render stage; 2 and 3 wait in order.
        let mut seen = Vec::new();
        let (s, _) = store
            .wait(
                &id(1),
                Duration::from_millis(500),
                false,
                &|| false,
                &mut |s, _| seen.push((s.state, s.stage)),
            )
            .unwrap();
        assert_eq!((s.state, s.stage), (JobState::Running, Some(Stage::Render)));
        assert!(seen.contains(&(JobState::Running, Some(Stage::Render))));
        assert_eq!(store.queue_position(&id(2)), Some(1));
        assert_eq!(store.queue_position(&id(3)), Some(2));

        // A cancelled call that did not submit the job (get_video) only stops
        // waiting.
        let (s3, _) = store
            .wait(
                &id(3),
                Duration::from_secs(2),
                false,
                &|| true,
                &mut |_, _| {},
            )
            .unwrap();
        assert_eq!(s3.state, JobState::Queued);
        // The submitter's cancel leaves a job another call is waiting on.
        std::thread::scope(|scope| {
            let other = scope.spawn(|| {
                store.wait(
                    &id(3),
                    Duration::from_millis(600),
                    false,
                    &|| false,
                    &mut |_, _| {},
                )
            });
            while store.waiting(&id(3)) == 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
            let (s, _) = store
                .wait(
                    &id(3),
                    Duration::from_secs(2),
                    true,
                    &|| true,
                    &mut |_, _| {},
                )
                .unwrap();
            assert_eq!(s.state, JobState::Queued);
            assert!(other.join().unwrap().is_some());
        });
        assert_eq!(store.queue_position(&id(3)), Some(2));
        assert_eq!(store.waiting(&id(3)), 0);

        // The submitter alone: its cancel cancels the queued job.
        let (s3, _) = store
            .wait(
                &id(3),
                Duration::from_secs(2),
                true,
                &|| true,
                &mut |_, _| {},
            )
            .unwrap();
        assert_eq!(
            (s3.state, s3.error.as_deref()),
            (JobState::Failed, Some(CANCELLED))
        );
        assert_eq!(store.queue_position(&id(3)), None);

        // Cancel the running job: it fails; job 2 starts.
        store.cancel(&id(1), CANCELLED);
        let (s1, _) = store
            .wait(
                &id(1),
                Duration::from_secs(5),
                false,
                &|| false,
                &mut |_, _| {},
            )
            .unwrap();
        assert_eq!(s1.error.as_deref(), Some(CANCELLED));
        *engine.hold.lock().unwrap() = false;
        let (s2, _) = store
            .wait(
                &id(2),
                Duration::from_secs(5),
                false,
                &|| false,
                &mut |_, _| {},
            )
            .unwrap();
        assert_eq!(s2.state, JobState::Done);
        assert_eq!(s2.qa, Some(Qa::Pass));
        assert_eq!(s2.findings, ["qa note", "prepared note"]);
        assert_eq!(
            s2.video.as_deref(),
            Some("output/jobs/j_0000000002/video.mp4")
        );
        assert_eq!(read_status(&id(2).dir(&config.jobs)).unwrap(), s2);
        // Done + video on disk: served, not re-run.
        assert!(matches!(
            store
                .submit(engine_job(&config, 2), initial.clone(), false, &write)
                .unwrap(),
            Submitted::Done(_)
        ));
        assert_eq!(engine.runs.load(Ordering::SeqCst), 2);
        // A failed job is re-run when asked again.
        assert_eq!(
            store
                .submit(engine_job(&config, 1), initial, false, &write)
                .unwrap(),
            Submitted::Queued
        );
        let (s1, _) = store
            .wait(
                &id(1),
                Duration::from_secs(5),
                false,
                &|| false,
                &mut |_, _| {},
            )
            .unwrap();
        assert_eq!(s1.state, JobState::Done);
        assert_eq!(engine.runs.load(Ordering::SeqCst), 3);
        assert!(store
            .wait(&id(77), Duration::ZERO, false, &|| false, &mut |_, _| {})
            .is_none());
        store.shutdown();
        let _ = std::fs::remove_dir_all(&repo);
    }

    fn fake(hold: bool) -> Arc<FakeEngine> {
        Arc::new(FakeEngine {
            runs: AtomicUsize::new(0),
            hold: Mutex::new(hold),
            released: Condvar::new(),
        })
    }

    #[test]
    fn recovery_spares_jobs_of_live_servers() {
        let repo = temp("recovery");
        let config = ServerConfig::new(Profile::Weak, &repo);
        let mut live = Command::new("sleep").arg("30").spawn().unwrap();
        let mut gone = Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let job = |n: u32, state: JobState, owner: Option<u32>| {
            let dir = id(n).dir(&config.jobs);
            std::fs::create_dir_all(&dir).unwrap();
            let mut s = status(&id(n), state, n as u64);
            s.owner_pid = owner;
            write_status(&dir, &s).unwrap();
            dir
        };
        let other = job(1, JobState::Running, Some(live.id()));
        let dead = job(2, JobState::Running, Some(gone.id()));
        let legacy = job(3, JobState::Queued, None);
        let store = Store::open(config.clone(), fake(false)).unwrap();
        assert_eq!(read_status(&other).unwrap().state, JobState::Running);
        for dir in [&dead, &legacy] {
            let s = read_status(dir).unwrap();
            assert_eq!(
                (s.state, s.error.as_deref(), s.owner_pid),
                (JobState::Failed, Some(INTERRUPTED), None)
            );
        }
        // Another live server's job: waited on, not re-run, never pruned.
        let write = |_: &Path| Ok(());
        let initial = status(&id(1), JobState::Queued, 1);
        assert_eq!(
            store
                .submit(engine_job(&config, 1), initial.clone(), false, &write)
                .unwrap(),
            Submitted::Foreign
        );
        let removed = prune_jobs(
            &config.jobs,
            Retention {
                max_jobs: 0,
                max_bytes: 0,
            },
            &|_| false,
        );
        // Oldest first (job 2 was failed first); the live server's job stays.
        assert_eq!(removed, [id(2), id(3)]);
        assert!(other.is_dir());
        // When that server dies, a waiter marks its job failed.
        live.kill().unwrap();
        live.wait().unwrap();
        let (s, _) = store
            .wait(
                &id(1),
                Duration::from_secs(5),
                false,
                &|| false,
                &mut |_, _| {},
            )
            .unwrap();
        assert_eq!(
            (s.state, s.error.as_deref()),
            (JobState::Failed, Some(STOPPED))
        );
        // ... and asking again runs it here.
        assert_eq!(
            store
                .submit(engine_job(&config, 1), initial, false, &write)
                .unwrap(),
            Submitted::Queued
        );
        store.shutdown();
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn two_stores_render_one_at_a_time() {
        let repo = temp("two-stores");
        let config = ServerConfig::new(Profile::Weak, &repo);
        let (engine_a, engine_b) = (fake(true), fake(false));
        let a = Store::open(config.clone(), engine_a.clone()).unwrap();
        let b = Store::open(config.clone(), engine_b.clone()).unwrap();
        let write = |_: &Path| Ok(());
        let initial = status(&id(1), JobState::Queued, 1);
        let none = &mut |_: &JobStatus, _: Option<usize>| {};
        a.submit(engine_job(&config, 1), initial.clone(), false, &write)
            .unwrap();
        let (s, _) = a
            .wait(&id(1), Duration::from_millis(300), false, &|| false, none)
            .unwrap();
        assert_eq!(s.state, JobState::Running);
        assert_eq!(s.owner_pid, Some(std::process::id()));
        // b: the same job belongs to a live server; another job waits for
        // the render lock (still queued, not rendered).
        assert_eq!(
            b.submit(engine_job(&config, 1), initial.clone(), false, &write)
                .unwrap(),
            Submitted::Foreign
        );
        b.submit(engine_job(&config, 2), initial, false, &write)
            .unwrap();
        let (s2, _) = b
            .wait(&id(2), Duration::from_millis(600), false, &|| false, none)
            .unwrap();
        assert_eq!(s2.state, JobState::Queued);
        assert_eq!(engine_b.runs.load(Ordering::SeqCst), 0);
        // a finishes; b's waiter on job 1 sees it done, then b renders job 2.
        *engine_a.hold.lock().unwrap() = false;
        let (s1, _) = b
            .wait(&id(1), Duration::from_secs(5), false, &|| false, none)
            .unwrap();
        assert_eq!((s1.state, s1.owner_pid), (JobState::Done, None));
        let (s2, _) = b
            .wait(&id(2), Duration::from_secs(5), false, &|| false, none)
            .unwrap();
        assert_eq!(s2.state, JobState::Done);
        assert_eq!(engine_a.runs.load(Ordering::SeqCst), 1);
        assert_eq!(engine_b.runs.load(Ordering::SeqCst), 1);
        a.shutdown();
        b.shutdown();
        let _ = std::fs::remove_dir_all(&repo);
    }
}
