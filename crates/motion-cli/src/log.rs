//! Tiny CLI log level (no dependency). Level 0 = quiet, 1 = default, 2 = verbose.
//! Only progress/status chatter goes through here; data output (`--json`,
//! `inspect`, `critique`, `qa` reports) stays on plain `println!`.

use std::sync::atomic::{AtomicU8, Ordering};

static LEVEL: AtomicU8 = AtomicU8::new(1);

pub fn set_level(quiet: bool, verbose: u8) {
    let level = if quiet { 0 } else { 1 + verbose.min(1) };
    LEVEL.store(level, Ordering::Relaxed);
}

pub fn level() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}

/// Summary line: shown at the default level and above.
macro_rules! info {
    ($($arg:tt)*) => {
        if $crate::log::level() >= 1 {
            println!($($arg)*);
        }
    };
}

/// Per-scene / per-step detail: shown with `-v` only.
macro_rules! detail {
    ($($arg:tt)*) => {
        if $crate::log::level() >= 2 {
            println!($($arg)*);
        }
    };
}

/// Warnings: always shown (stderr).
macro_rules! warn {
    ($($arg:tt)*) => {
        eprintln!($($arg)*)
    };
}
