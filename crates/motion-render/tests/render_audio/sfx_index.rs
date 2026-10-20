//! SFX library tooling (SOUND_DESIGN §2): measurement, indexing, loading, zip.
//! Fixtures are synthesised with ffmpeg lavfi into a unique temp dir.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use motion_core::audio::{
    CurationEntry, SfxCuration, SfxFamily, LIBRARY_FILE, SFX_CURATION_VERSION,
};
use motion_render::sfx::{
    build_pack, index_pack, load_library, measure_sound, sha256_file, IndexOptions,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "motion_sfx_index_{}_{}_{}",
            std::process::id(),
            tag,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        Scratch(dir)
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn synth(expr: &str, dur: f64, out: &Path, extra: &[&str]) -> bool {
    let src = format!("aevalsrc='{expr}':s=48000:d={dur}");
    Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", &src])
        .args(extra)
        .arg(out)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const BURST: &str = "if(between(t,0.300,0.400),0.5*sin(2*PI*1000*t),0)";
const HIT: &str = "if(gte(t,1.2),exp(-(t-1.2)*20)*sin(2*PI*200*t),0)";

fn burst_wav(dir: &Scratch, name: &str) -> PathBuf {
    let p = dir.path(name);
    assert!(synth(BURST, 1.0, &p, &[]), "ffmpeg burst");
    p
}

fn hit_wav(dir: &Scratch, name: &str) -> PathBuf {
    let p = dir.path(name);
    assert!(synth(HIT, 2.0, &p, &[]), "ffmpeg hit");
    p
}

fn entry(id: &str, family: SfxFamily, path: &str) -> CurationEntry {
    CurationEntry {
        id: id.to_string(),
        family,
        path: path.to_string(),
        tags: vec![],
    }
}

fn curation(sounds: Vec<CurationEntry>) -> SfxCuration {
    SfxCuration {
        version: SFX_CURATION_VERSION.to_string(),
        sounds,
    }
}

/// A pack dir with two valid wavs.
fn make_pack(s: &Scratch) -> (PathBuf, SfxCuration) {
    let pack = s.path("pack");
    std::fs::create_dir_all(pack.join("sub")).expect("mkdir");
    assert!(synth(BURST, 1.0, &pack.join("sub/Burst.WAV"), &[]));
    assert!(synth(HIT, 2.0, &pack.join("hit.wav"), &[]));
    let c = curation(vec![
        entry("b_hit", SfxFamily::HitSoft, "hit.wav"),
        entry("a_burst", SfxFamily::Click, "sub/Burst.WAV"),
    ]);
    (pack, c)
}

#[test]
fn sine_burst_measurement() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("burst");
    let p = burst_wav(&s, "burst.wav");
    let m = measure_sound(&p).expect("measure");
    eprintln!("burst: {m:?}");
    assert!((m.peak - 0.305).abs() <= 0.010, "peak {}", m.peak);
    assert!((m.onset - 0.300).abs() <= 0.010, "onset {}", m.onset);
    assert!(
        (m.audible_end - 0.40).abs() <= 0.015,
        "end {}",
        m.audible_end
    );
    assert!((m.peak_db + 6.0).abs() <= 0.3, "peak_db {}", m.peak_db);
    assert!((m.duration - 1.0).abs() <= 0.002, "duration {}", m.duration);
    assert!(0.0 <= m.onset && m.onset <= m.peak && m.peak <= m.audible_end);
    assert!(m.audible_end <= m.duration);
}

#[test]
fn decaying_hit_peak() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("hit");
    let p = hit_wav(&s, "hit.wav");
    let m = measure_sound(&p).expect("measure");
    eprintln!("hit: {m:?}");
    assert!((m.peak - 1.2).abs() <= 0.010, "peak {}", m.peak);
    assert!(m.onset <= m.peak && m.peak <= m.audible_end && m.audible_end <= m.duration);
}

#[test]
fn mp3_fixture_measures() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("mp3");
    let p = s.path("burst.mp3");
    if !synth(BURST, 1.0, &p, &["-c:a", "libmp3lame", "-b:a", "192k"]) {
        return;
    }
    let m = measure_sound(&p).expect("measure mp3");
    eprintln!("mp3: {m:?}");
    // The burst is flat, so lossy ripple may move the loudest window anywhere
    // inside it; it must stay within the burst (plus encoder slack).
    assert!((0.29..=0.43).contains(&m.peak), "peak {}", m.peak);
    assert!((m.onset - 0.300).abs() <= 0.030, "onset {}", m.onset);
    assert!(m.onset <= m.peak && m.peak <= m.audible_end && m.audible_end <= m.duration);
}

#[test]
fn corrupt_file_errors_and_is_reported() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("corrupt");
    let (pack, mut c) = make_pack(&s);
    // Deterministic pseudo-random garbage.
    let mut x: u32 = 0x1234_5678;
    let bytes: Vec<u8> = (0..4096)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    std::fs::write(pack.join("x.wav"), &bytes).expect("write");
    assert!(measure_sound(&pack.join("x.wav")).is_err());

    c.sounds.push(entry("junk", SfxFamily::Pop, "x.wav"));
    let out = s.path("lib");
    let r = index_pack(&pack, &c, &out, &IndexOptions::default()).expect("index");
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].0, "junk");
    assert_eq!(r.library.sounds.len(), 2);
    assert!(!out.join("sounds/pop/junk.wav").exists());
    assert!(out.join(LIBRARY_FILE).is_file());
}

#[test]
fn index_is_deterministic_and_cached() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("determinism");
    let (pack, c) = make_pack(&s);
    let out1 = s.path("lib1");
    let out2 = s.path("lib2");
    let r1 = index_pack(&pack, &c, &out1, &IndexOptions::default()).expect("index1");
    assert_eq!((r1.measured, r1.cached, r1.failed.len()), (2, 0, 0));
    let r2 = index_pack(&pack, &c, &out2, &IndexOptions::default()).expect("index2");
    assert_eq!(r2.measured, 2);
    let j1 = std::fs::read(out1.join(LIBRARY_FILE)).expect("read1");
    let j2 = std::fs::read(out2.join(LIBRARY_FILE)).expect("read2");
    assert_eq!(j1, j2);
    assert!(j1.ends_with(b"}\n"));

    // Sorted by id; dest path uses family folder, id and lowercased extension.
    let ids: Vec<&str> = r1.library.sounds.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["a_burst", "b_hit"]);
    assert_eq!(r1.library.sounds[0].path, "sounds/click/a_burst.wav");
    assert_eq!(r1.library.sounds[1].path, "sounds/hit_soft/b_hit.wav");
    for snd in &r1.library.sounds {
        let f = out1.join(&snd.path);
        assert_eq!(sha256_file(&f).expect("sha"), snd.sha256);
        assert_eq!(snd.sha256.len(), 64);
    }

    // Second run into the same dir hits the cache.
    let r3 = index_pack(&pack, &c, &out1, &IndexOptions::default()).expect("index3");
    assert_eq!((r3.measured, r3.cached), (0, 2));
    assert_eq!(std::fs::read(out1.join(LIBRARY_FILE)).expect("read3"), j1);

    // Explicit cache dir is honoured.
    let cache = s.path("shared-cache");
    let opts = IndexOptions {
        cache_dir: Some(cache.clone()),
    };
    let out3 = s.path("lib3");
    let r4 = index_pack(&pack, &c, &out3, &opts).expect("index4");
    assert_eq!(r4.measured, 2);
    assert!(cache.is_dir());
    assert!(!out3.join(".measure-cache").exists());
}

#[test]
fn pack_is_deterministic_and_listed() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("zip");
    let (pack, c) = make_pack(&s);
    let lib = s.path("lib");
    index_pack(&pack, &c, &lib, &IndexOptions::default()).expect("index");
    std::fs::write(lib.join("LICENSE.md"), "license\n").expect("write");

    let z1 = s.path("out/a.zip");
    let z2 = s.path("out/b.zip");
    let n1 = build_pack(&lib, &z1).expect("pack1");
    let n2 = build_pack(&lib, &z2).expect("pack2");
    // manifest + 2 sounds + LICENSE.md (no README.md)
    assert_eq!((n1, n2), (4, 4));
    let b1 = std::fs::read(&z1).expect("read");
    assert_eq!(b1, std::fs::read(&z2).expect("read"));

    // Also loadable from the manifest file path.
    let n3 = build_pack(&lib.join(LIBRARY_FILE), &s.path("c.zip")).expect("pack3");
    assert_eq!(n3, 4);

    if let Ok(o) = Command::new("unzip").arg("-l").arg(&z1).output() {
        if o.status.success() {
            let t = String::from_utf8_lossy(&o.stdout);
            for name in [
                "sfx-library.json",
                "sounds/click/a_burst.wav",
                "sounds/hit_soft/b_hit.wav",
                "LICENSE.md",
            ] {
                assert!(t.contains(name), "unzip -l missing {name}:\n{t}");
            }
            assert!(!t.contains("README.md"));
        }
    }
    if let Ok(o) = Command::new("unzip").arg("-tq").arg(&z1).output() {
        // `unzip -t` verifies crc32 of every entry.
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    }
}

#[test]
fn load_library_variants() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("load");
    let (pack, c) = make_pack(&s);
    let lib = s.path("lib");
    index_pack(&pack, &c, &lib, &IndexOptions::default()).expect("index");

    let (root_a, la) = load_library(&lib).expect("dir");
    let (root_b, lb) = load_library(&lib.join(LIBRARY_FILE)).expect("file");
    assert_eq!(root_a, lib);
    assert_eq!(root_b, lib);
    assert_eq!(la, lb);
    assert_eq!(la.sounds.len(), 2);

    std::fs::remove_file(lib.join("sounds/hit_soft/b_hit.wav")).expect("rm");
    let err = load_library(&lib).expect_err("missing sound");
    assert!(err.to_string().contains("b_hit"), "{err}");
    assert!(build_pack(&lib, &s.path("z.zip")).is_err());

    assert!(load_library(&s.path("nonexistent")).is_err());
}

#[test]
fn bad_curation_entries_fail_individually() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("badcur");
    let (pack, mut c) = make_pack(&s);
    let abs = pack.join("hit.wav").to_string_lossy().to_string();
    c.sounds.push(entry("abs", SfxFamily::Pop, &abs));
    c.sounds
        .push(entry("dots", SfxFamily::Pop, "../pack/hit.wav"));
    c.sounds.push(entry("missing", SfxFamily::Pop, "nope.wav"));
    c.sounds.push(entry("a_burst", SfxFamily::Pop, "hit.wav"));
    let out = s.path("lib");
    let r = index_pack(&pack, &c, &out, &IndexOptions::default()).expect("index");
    let failed: Vec<&str> = r.failed.iter().map(|(i, _)| i.as_str()).collect();
    assert_eq!(failed, ["abs", "dots", "missing", "a_burst"]);
    assert_eq!(r.library.sounds.len(), 2);
    // The duplicate did not clobber the original's file or manifest entry.
    assert_eq!(
        r.library.get("a_burst").map(|s| s.family),
        Some(SfxFamily::Click)
    );
    assert!(out.join("sounds/click/a_burst.wav").is_file());
    assert!(
        !out.join("sounds/pop").exists()
            || std::fs::read_dir(out.join("sounds/pop"))
                .map(|d| d.count() == 0)
                .unwrap_or(true)
    );

    let mut bad_version = c.clone();
    bad_version.version = "9.9".to_string();
    assert!(index_pack(&pack, &bad_version, &out, &IndexOptions::default()).is_err());
}
