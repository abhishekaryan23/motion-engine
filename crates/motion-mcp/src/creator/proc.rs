//! Subprocess helpers for the creator tools: a command with a deadline whose
//! output is drained (so a chatty child never blocks), and a small scoped
//! thread pool for independent single-frame renders and compiles.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::engine::one_line_error;

/// What a finished command printed.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub stdout: String,
    pub stderr: String,
}

/// Run `cmd` to completion within `timeout`. A failure (cannot start, non-zero
/// exit, timeout) is one line saying why, prefixed with `step`.
pub fn run(step: &str, mut cmd: Command, timeout: Duration) -> Result<Ran, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "{step}: cannot start {}: {e}",
            cmd.get_program().to_string_lossy()
        )
    })?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut p) = pipe {
                let mut bytes = Vec::new();
                let _ = p.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // The drain threads end when the pipes close; a grandchild may
                // still hold them, so never join here.
                return Err(format!("{step}: timed out after {} s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(format!("{step}: cannot wait for the engine: {e}")),
        }
    };
    let ran = Ran {
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    };
    if status.success() {
        return Ok(ran);
    }
    let lines = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
    let why = one_line_error(
        None,
        &lines(&ran.stderr),
        &lines(&ran.stdout),
        status.code(),
    );
    Err(if why.to_lowercase().contains(&step.to_lowercase()) {
        why
    } else {
        format!("{step}: {why}")
    })
}

/// Apply `f` to every item on up to `max_threads` threads; results keep the
/// order of `items`.
pub fn par_map<T, R, F>(items: &[T], max_threads: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let threads = max_threads.max(1).min(items.len().max(1));
    let next = AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::<(usize, R)>::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(item) = items.get(i) else { break };
                let r = f(item);
                results
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((i, r));
            });
        }
    });
    let mut out = results.into_inner().unwrap_or_else(|e| e.into_inner());
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

/// Threads for single-frame renders and compiles: the machine's cores, at
/// most 4 (the CPU is shared with the render queue).
pub fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn par_map_keeps_order() {
        let items: Vec<u32> = (0..20).collect();
        let out = par_map(&items, 4, |i| i * 2);
        assert_eq!(out, (0..20).map(|i| i * 2).collect::<Vec<_>>());
        assert!(par_map(&Vec::<u32>::new(), 4, |i| *i).is_empty());
    }

    #[test]
    fn run_reports_failures_in_one_line() {
        let mut ok = Command::new("sh");
        ok.args(["-c", "echo fine"]);
        assert_eq!(
            run("echo", ok, Duration::from_secs(5))
                .unwrap()
                .stdout
                .trim(),
            "fine"
        );
        let mut bad = Command::new("sh");
        bad.args(["-c", "echo 'Error: no such frame' >&2; exit 3"]);
        let e = run("render", bad, Duration::from_secs(5)).unwrap_err();
        assert!(
            e.starts_with("render: ") && e.contains("no such frame"),
            "{e}"
        );
        let mut slow = Command::new("sh");
        slow.args(["-c", "exec sleep 5"]);
        let e = run("slow", slow, Duration::from_millis(100)).unwrap_err();
        assert!(e.contains("timed out"), "{e}");
    }
}
