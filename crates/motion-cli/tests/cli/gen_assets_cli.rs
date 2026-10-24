//! (v2.3) Generated assets in programs through the real binary: `gen:` refs
//! resolve only from the lockfile (compile/validate never call a provider),
//! a missing entry is a validation error, and compile is byte-identical.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use motion_providers::lockfile::{lock_path, prompt_hash, sha256_file, LockEntry, Lockfile};
use motion_providers::ImageSpec;
use motion_render::tiny_skia::{Color, Pixmap};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("gen_assets_cli")
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cfg")).expect("mkdir");
    // MOSS pointed at a closed port: a test can never reach the real server.
    std::fs::write(
        dir.join("cfg/providers.toml"),
        "[moss]\nurl = \"http://127.0.0.1:9\"\n",
    )
    .unwrap();
    dir
}

#[test]
fn invalid_program_generates_nothing() {
    let dir = scratch("invalid");
    let program = demo_with_lock(&dir, false);
    let text = std::fs::read_to_string(&program).unwrap().replace(
        "\"id\": \"spray_word\",",
        "\"id\": \"spray_word\", \"start\": 50.0,",
    );
    std::fs::write(&program, text).unwrap();
    let out = run(&dir, &["program", "assets", s(&program)]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("not a valid program (nothing generated)"),
        "{err}"
    );
    assert!(!err.contains("skipped"), "no provider was tried: {err}");
    assert!(!lock_path(&program).exists(), "no lockfile written");
}

/// The binary with an empty config dir: no connections, so a provider call
/// could never reach OpenArt.
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .env("MOTION_CONFIG_DIR", dir.join("cfg"))
        .env("MOTION_ASSET_CACHE", dir.join("cache"))
        .args(args)
        .output()
        .expect("run motion-engine")
}

fn plate(path: &Path, c: Color) {
    let mut pm = Pixmap::new(96, 54).unwrap();
    pm.fill(c);
    pm.save_png(path).unwrap();
}

/// The demo program copied into `dir`, plus a lockfile for its assets
/// pointing at synthetic plates.
fn demo_with_lock(dir: &Path, lock: bool) -> PathBuf {
    let src = root().join("examples/v2/gen_assets.program.json");
    let program = dir.join("gen_assets.program.json");
    std::fs::copy(&src, &program).unwrap();
    if lock {
        let p: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&program).unwrap()).unwrap();
        let mut l = Lockfile::default();
        for (i, a) in p["assets"].as_array().unwrap().iter().enumerate() {
            let file = dir.join(format!("plate_{i}.png"));
            plate(&file, Color::from_rgba8(40 + 60 * i as u8, 120, 140, 255));
            let file = file.canonicalize().unwrap();
            let spec = ImageSpec {
                prompt: a["prompt"].as_str().unwrap().into(),
                cutout: false,
                aspect: "16:9".into(),
                seed: a["seed"].as_u64().unwrap() as u32,
            };
            l.assets.insert(
                a["id"].as_str().unwrap().into(),
                LockEntry {
                    provider: "moss".into(),
                    model: "moss-image-sft".into(),
                    seed: spec.seed,
                    prompt_hash: prompt_hash(&spec),
                    sha256: sha256_file(&file).unwrap(),
                    path: file.to_string_lossy().into_owned(),
                },
            );
        }
        l.save(&lock_path(&program)).unwrap();
    }
    program
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn missing_lockfile_entry_is_a_validation_error() {
    let dir = scratch("missing");
    let program = demo_with_lock(&dir, false);
    let out = run(&dir, &["program", "validate", s(&program)]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("run `program assets`"), "{err}");
    assert!(err.contains("components[1].style.fill.image"), "{err}");
    let out = run(
        &dir,
        &[
            "program",
            "compile",
            s(&program),
            "-o",
            s(&dir.join("x.motion.json")),
        ],
    );
    assert!(!out.status.success());
    // Offline resolution never generates.
    let out = run(&dir, &["program", "assets", "--offline", s(&program)]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not in the lockfile"));
}

#[test]
fn compile_with_lockfile_is_byte_identical() {
    let dir = scratch("identical");
    let program = demo_with_lock(&dir, true);
    let out = run(&dir, &["program", "validate", s(&program)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run(&dir, &["program", "assets", "--offline", s(&program)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (a, b) = (dir.join("a.motion.json"), dir.join("b.motion.json"));
    for o in [&a, &b] {
        let out = run(&dir, &["program", "compile", s(&program), "-o", s(o)]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let (ta, tb) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    assert_eq!(ta, tb, "same program + lockfile → byte-identical scene");
    let text = String::from_utf8(ta).unwrap();
    let plate0 = dir.join("plate_0.png").canonicalize().unwrap();
    assert!(
        text.contains(s(&plate0)),
        "gen: resolved to the cached file"
    );
    assert!(!text.contains("\"gen:"), "no unresolved gen: path left");
}

#[test]
fn unknown_gen_id_is_rejected() {
    let dir = scratch("unknown");
    let program = demo_with_lock(&dir, true);
    let text = std::fs::read_to_string(&program)
        .unwrap()
        .replace("gen:fern_plate", "gen:no_such_plate");
    std::fs::write(&program, text).unwrap();
    let out = run(&dir, &["program", "validate", s(&program)]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no generated asset 'no_such_plate'"), "{err}");
}
