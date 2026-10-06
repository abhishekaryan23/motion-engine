//! (0.20) `models list|fetch|path`: the local word-timing models under
//! `$MOTION_MODELS_DIR` or `~/.cache/motionengine/models`: whisper.cpp ggml
//! weights (kind `whisper`, one file each) and (0.20 A3) the CTC aligner
//! `wav2vec2-base-960h` (kind `ctc`, a directory holding the int8 ONNX graph
//! and its vocabulary). Nothing downloads unless the operator runs `fetch`;
//! every download is a pinned URL checked against a pinned SHA256 and written
//! atomically.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Subcommand;
use motion_voice::asr_ctc::{self, CtcModelSpec};
use motion_voice::asr_local::{self, ModelSpec};

#[derive(Subcommand)]
pub enum ModelsCmd {
    /// Show every known model, its kind, size and whether it is installed.
    List,
    /// Download a model (whisper-tiny.en | whisper-base.en | whisper-small.en |
    /// wav2vec2-base-960h) and verify its SHA256.
    Fetch {
        /// Model name (see `models list`).
        name: String,
    },
    /// Print the installed model's path (a directory for a ctc model; exit 1
    /// when it is missing).
    Path {
        /// Model name (see `models list`).
        name: String,
    },
}

/// A known model of either kind.
enum Known {
    Whisper(&'static ModelSpec),
    Ctc(&'static CtcModelSpec),
}

impl Known {
    fn find(name: &str) -> Option<Self> {
        asr_local::model_spec(name)
            .map(Known::Whisper)
            .or_else(|| asr_ctc::model_spec(name).map(Known::Ctc))
    }

    fn all() -> Vec<Self> {
        asr_local::MODELS
            .iter()
            .map(Known::Whisper)
            .chain(asr_ctc::MODELS.iter().map(Known::Ctc))
            .collect()
    }

    fn name(&self) -> &'static str {
        match self {
            Known::Whisper(m) => m.name,
            Known::Ctc(m) => m.name,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Known::Whisper(_) => "whisper",
            Known::Ctc(_) => "ctc",
        }
    }

    fn bytes(&self) -> u64 {
        match self {
            Known::Whisper(m) => m.bytes,
            Known::Ctc(m) => m.bytes(),
        }
    }

    fn note(&self) -> &'static str {
        match self {
            Known::Whisper(m) => m.note,
            Known::Ctc(m) => m.note,
        }
    }

    /// Installed size in bytes (every file of a ctc model present).
    fn installed(&self, root: &Path) -> Option<u64> {
        match self {
            Known::Whisper(m) => asr_local::installed(root, m),
            Known::Ctc(m) => asr_ctc::installed(root, m),
        }
    }

    /// The model file (whisper) or directory (ctc).
    fn path(&self, root: &Path) -> PathBuf {
        match self {
            Known::Whisper(m) => asr_local::model_path(root, m),
            Known::Ctc(m) => asr_ctc::model_dir(root, m),
        }
    }

    fn sources(&self) -> Vec<&'static str> {
        match self {
            Known::Whisper(m) => vec![m.url],
            Known::Ctc(m) => m.files.iter().map(|f| f.url).collect(),
        }
    }
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn names() -> String {
    Known::all()
        .iter()
        .map(Known::name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn run(cmd: ModelsCmd) -> Result<()> {
    let root = asr_local::models_root();
    match cmd {
        ModelsCmd::List => {
            println!("models root: {}", root.display());
            println!(
                "{:<19} {:<8} {:>9}  {:<10} note",
                "name", "kind", "size", "installed"
            );
            for m in Known::all() {
                let state = match m.installed(&root) {
                    Some(b) if b == m.bytes() => "yes".to_string(),
                    Some(b) => format!("partial ({:.0} MiB)", mib(b)),
                    None => "no".to_string(),
                };
                println!(
                    "{:<19} {:<8} {:>6.0} MiB  {:<10} {}",
                    m.name(),
                    m.kind(),
                    mib(m.bytes()),
                    state,
                    m.note()
                );
            }
            println!(
                "fetch one with `motion-engine models fetch <name>`; `voice --align local` uses {} \
                 and `--asr-ctc` uses {}",
                asr_local::DEFAULT_MODEL,
                asr_ctc::MODEL.name
            );
            Ok(())
        }
        ModelsCmd::Fetch { name } => {
            let Some(model) = Known::find(&name) else {
                bail!("unknown model '{name}' (one of: {})", names());
            };
            println!(
                "fetching {} ({:.0} MiB) from {}",
                model.name(),
                mib(model.bytes()),
                model.sources().join(", ")
            );
            let mut last = 0u64;
            let mut progress = |done: u64, total: u64| {
                // One line per 8 MiB so logs stay short.
                if done.saturating_sub(last) >= 8 << 20 || done == total {
                    last = done;
                    eprint!("\r  {:>5.0} / {:.0} MiB", mib(done), mib(total));
                    let _ = std::io::stderr().flush();
                }
            };
            let (path, sha) = match model {
                Known::Whisper(spec) => (
                    asr_local::fetch(spec, &root, &mut progress)?,
                    spec.sha256.to_string(),
                ),
                Known::Ctc(spec) => (
                    asr_ctc::fetch(spec, &root, &mut progress)?,
                    spec.files
                        .iter()
                        .map(|f| format!("{} {}", f.file, f.sha256))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            };
            eprintln!();
            println!("installed {} (sha256 {sha})", path.display());
            Ok(())
        }
        ModelsCmd::Path { name } => {
            let Some(model) = Known::find(&name) else {
                bail!("unknown model '{name}' (one of: {})", names());
            };
            let path = model.path(&root);
            if model.installed(&root).is_none() {
                bail!(
                    "{} is not installed: run `motion-engine models fetch {}`",
                    path.display(),
                    model.name()
                );
            }
            println!("{}", path.display());
            Ok(())
        }
    }
}
