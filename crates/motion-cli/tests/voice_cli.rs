//! (0.10) `providers` and `voice` through the real binary. No network: the
//! config dir is redirected with MOTION_CONFIG_DIR to a temp dir and the key
//! env var is removed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

struct Temp(PathBuf);
impl Temp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("motion-cli-voice-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("tempdir");
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn motion(config_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .env("MOTION_CONFIG_DIR", config_dir)
        .env_remove("OPENROUTER_API_KEY")
        .args(args)
        .output()
        .expect("run motion-engine")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn providers_show_without_key_prints_none() {
    let t = Temp::new("show-none");
    let o = motion(&t.0, &["providers", "show"]);
    assert!(o.status.success());
    let s = out(&o);
    assert!(s.contains("provider: openrouter"), "{s}");
    assert!(s.contains("key: none"), "{s}");
    assert!(s.contains("source: none"), "{s}");
    assert!(s.contains(t.0.to_str().unwrap()), "{s}");
}

#[test]
fn set_key_via_stdin_then_show_masks_it_and_never_prints_it() {
    let t = Temp::new("setkey");
    let key = "sk-or-v1-0123456789abcdefQRST";
    let mut child = Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .env("MOTION_CONFIG_DIR", &t.0)
        .env_remove("OPENROUTER_API_KEY")
        .args(["providers", "set-key", "openrouter"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        writeln!(stdin, "  {key}  ").expect("write key");
    }
    let o = child.wait_with_output().expect("wait");
    assert!(o.status.success());
    let s = out(&o);
    assert!(s.contains("sk-or-…QRST"), "{s}");
    assert!(!s.contains("0123456789"), "{s}");

    let shown = motion(&t.0, &["providers", "show"]);
    let s = out(&shown);
    assert!(s.contains("key: sk-or-…QRST"), "{s}");
    assert!(s.contains("source: config"), "{s}");
    assert!(!s.contains("0123456789"), "{s}");
    assert!(!s.contains("warning"), "{s}");
}

#[test]
#[cfg(unix)]
fn show_warns_about_wide_permissions_without_changing_them() {
    use std::os::unix::fs::PermissionsExt;
    let t = Temp::new("perm");
    let file = t.0.join("providers.toml");
    std::fs::write(
        &file,
        "OPENROUTER_API_KEY = \"sk-or-v1-topleveltoplevel9999\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let o = motion(&t.0, &["providers", "show"]);
    let s = out(&o);
    assert!(s.contains("key: sk-or-…9999"), "{s}");
    assert!(s.contains("warning:") && s.contains("0644"), "{s}");
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644);
}

#[test]
fn voice_offline_with_empty_cache_is_a_clear_error() {
    let t = Temp::new("offline");
    let cache = t.0.join("cache");
    let outdir = t.0.join("out");
    let o = motion(
        &t.0,
        &[
            "voice",
            "examples/editorial_demo.intent.json",
            "--style",
            "examples/editorial_demo.style.json",
            "--offline",
            "--cache",
            cache.to_str().unwrap(),
            "-o",
            outdir.to_str().unwrap(),
        ],
    );
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("offline"), "{err}");
}

#[test]
fn voice_without_key_suggests_say() {
    let t = Temp::new("nokey");
    let cache = t.0.join("cache");
    let outdir = t.0.join("out");
    let o = motion(
        &t.0,
        &[
            "voice",
            "examples/editorial_demo.intent.json",
            // (0.10 Q) `auto` falls back to free models (down to `say`); an
            // explicitly chosen key-needing model reports the fix instead.
            "--tts-model",
            "deepgram",
            "--cache",
            cache.to_str().unwrap(),
            "-o",
            outdir.to_str().unwrap(),
        ],
    );
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("--tts-model say"), "{err}");
    assert!(err.contains("providers set-key"), "{err}");
}

#[test]
#[cfg(target_os = "macos")]
fn voice_with_say_builds_wav_and_speech_map_then_reruns_from_cache() {
    if Command::new("say").arg("-v").arg("?").output().is_err()
        || Command::new("ffmpeg").arg("-version").output().is_err()
    {
        eprintln!("skipping: say/ffmpeg unavailable");
        return;
    }
    let t = Temp::new("say");
    let cache = t.0.join("cache");
    let outdir = t.0.join("out");
    let args = [
        "voice",
        "examples/editorial_demo.intent.json",
        "--style",
        "examples/editorial_demo.style.json",
        "--tts-model",
        "say",
        "--cache",
        cache.to_str().unwrap(),
        "-o",
        outdir.to_str().unwrap(),
    ];
    let first = motion(&t.0, &args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let s1 = out(&first);
    assert!(s1.contains("cache hits: 0"), "{s1}");
    let json_file = std::fs::read_dir(&outdir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".speech.json"))
        .expect("speech.json written");
    let map: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_file).unwrap()).unwrap();
    assert_eq!(map["version"], "0.1");
    assert_eq!(map["provider"], "macos_say");
    assert!(map["sentences"].as_array().unwrap().len() >= 2);
    let wav = outdir.join(map["audio"].as_str().unwrap());
    let before = std::fs::read(&wav).unwrap();

    let second = motion(&t.0, &args);
    assert!(second.status.success());
    let s2 = out(&second);
    assert!(s2.contains("provider calls: 0"), "{s2}");
    assert_eq!(before, std::fs::read(&wav).unwrap());
}

/// (0.20) `--offline` blocks provider calls only: on-device word timing
/// (whisper, and the CTC aligner when built and installed) still runs on an
/// empty recogniser cache. Needs `--features asr-local`, macOS `say` +
/// ffmpeg and whisper-base.en installed; skipped otherwise.
#[test]
#[cfg(all(target_os = "macos", feature = "asr-local"))]
fn voice_offline_runs_local_word_timing_on_an_empty_recogniser_cache() {
    let models = std::env::var_os("MOTION_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".cache/motionengine/models")
        });
    if !models.join("ggml-base.en.bin").is_file()
        || Command::new("say").arg("-v").arg("?").output().is_err()
        || Command::new("ffmpeg").arg("-version").output().is_err()
    {
        eprintln!("skipping: whisper-base.en, say or ffmpeg unavailable");
        return;
    }
    let t = Temp::new("offline-local");
    let cache = t.0.join("cache");
    let voice = |out: &str, extra: &[&str]| {
        let outdir = t.0.join(out);
        let mut args = vec![
            "voice",
            "examples/editorial_demo.intent.json",
            "--style",
            "examples/editorial_demo.style.json",
            "--tts-model",
            "say",
            "--cache",
            cache.to_str().unwrap(),
            "-o",
            outdir.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        motion(&t.0, &args)
    };
    // 1. Fill the TTS cache; onset timing leaves the recogniser cache empty.
    let first = voice("out1", &["--align", "onset"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let asr = cache.join("asr");
    assert!(!asr.exists() || std::fs::read_dir(&asr).unwrap().count() == 0);
    // 2. Offline with local timing: the take from the cache, the recogniser
    //    (and CTC, when available) runs on the miss.
    let second = voice("out2", &["--offline", "--align", "local"]);
    let s = out(&second);
    assert!(
        second.status.success(),
        "{s}\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(s.contains("provider calls: 0"), "{s}");
    assert!(
        s.contains("script words measured (local:whisper-base.en:beam1"),
        "{s}"
    );
    assert!(std::fs::read_dir(&asr).unwrap().count() > 0);
}
