//! Bring-your-own-images sandbox (plan §5a, §7): references stay inside the
//! asset roots, only real images of a sane size pass, and a good folder
//! resolves with the right roles and reaches the manifest.

mod common;

use std::path::{Path, PathBuf};

use motion_mcp::args::MakeVideoArgs;
use motion_mcp::byo::{self, Sandbox};
use motion_mcp::lite::ImageRole;
use motion_mcp::policy::{self, Prepared};
use motion_mcp::profile::{Profile, ServerConfig};
use motion_mcp::reply::Fix;
use serde_json::{json, Value};

/// A scratch asset root with a few hostile files (made at test time: no
/// symlinks or huge files are committed).
fn scratch_root(name: &str) -> (PathBuf, PathBuf) {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("motion-mcp-sandbox")
        .join(name);
    let root = base.join("inbox");
    let outside = base.join("outside");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(root.join("trip")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let good = common::fixtures().join("inbox/demo/beat1.jpg");
    std::fs::copy(&good, outside.join("secret.jpg")).unwrap();
    std::fs::copy(&good, root.join("trip/ok.jpg")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.join("secret.jpg"), root.join("trip/link.jpg")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    }
    // Text with an image extension.
    std::fs::write(root.join("trip/fake.jpg"), "this is not an image at all").unwrap();
    std::fs::write(root.join("trip/notes.txt"), "hello").unwrap();
    // Sparse 41 MB file with a PNG signature.
    let big = std::fs::File::create(root.join("trip/huge.png")).unwrap();
    big.set_len(41 * 1024 * 1024).unwrap();
    // A PNG header claiming 9000 × 100 px.
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend([0, 0, 0, 13]);
    png.extend(b"IHDR");
    png.extend(9000u32.to_be_bytes());
    png.extend(100u32.to_be_bytes());
    png.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    std::fs::write(root.join("trip/wide.png"), png).unwrap();
    (root, outside)
}

fn config_with_root(root: &Path) -> ServerConfig {
    let mut c = common::config(Profile::Weak, "sandbox");
    c.asset_roots = vec![root.to_path_buf()];
    c
}

fn story_with(picture: Value) -> Value {
    json!({"title": "sandbox", "beats": [
        {"say": "Here is the picture the user wanted to show everyone.", "picture": picture},
        {"say": "And that is all there is to say about it, really."}
    ]})
}

fn prepare(
    config: &ServerConfig,
    story: Value,
    assets: Option<&str>,
) -> Result<Prepared, Vec<Fix>> {
    let a = MakeVideoArgs {
        story,
        assets: assets.map(str::to_string),
        ..Default::default()
    };
    policy::prepare(&a, &common::ctx(config))
}

fn picture_fix(config: &ServerConfig, picture: Value) -> String {
    match prepare(config, story_with(picture.clone()), None) {
        Ok(p) => panic!("{picture}: expected a fix, got {:?}", p.plan),
        Err(f) => {
            assert_eq!(f[0].beat, Some(1), "{f:?}");
            assert_eq!(f[0].field, "picture");
            f[0].to_string()
        }
    }
}

#[test]
fn hostile_references_are_fixes() {
    let (root, outside) = scratch_root("refs");
    let config = config_with_root(&root);
    let abs = format!("file:{}", outside.join("secret.jpg").display());
    let f = picture_fix(&config, json!(abs));
    assert!(f.contains("is an absolute path"), "{f}");
    let f = picture_fix(&config, json!("file:../outside/secret.jpg"));
    assert!(
        f.contains("is outside the asset folder — use a path inside"),
        "{f}"
    );
    let f = picture_fix(&config, json!({"file": "trip/../../outside/secret.jpg"}));
    assert!(f.contains("is outside the asset folder"), "{f}");
    let f = picture_fix(&config, json!("file:trip\\ok.jpg"));
    assert!(f.contains("backslash"), "{f}");
    #[cfg(unix)]
    {
        let f = picture_fix(&config, json!("file:trip/link.jpg"));
        assert!(f.contains("a link leads out"), "{f}");
        let f = picture_fix(&config, json!("file:escape/secret.jpg"));
        assert!(f.contains("a link leads out"), "{f}");
    }
    let f = picture_fix(&config, json!("file:trip/fake.jpg"));
    assert!(
        f.contains("is not a readable jpg, png or webp image"),
        "{f}"
    );
    let f = picture_fix(&config, json!("file:trip/notes.txt"));
    assert!(f.contains("is not a jpg, png or webp image"), "{f}");
    let f = picture_fix(&config, json!("file:trip/huge.png"));
    assert!(f.contains("is 41 MB, over the 40 MB limit"), "{f}");
    let f = picture_fix(&config, json!("file:trip/wide.png"));
    assert!(f.contains("9000×100 px, over the 8000 px limit"), "{f}");
    let f = picture_fix(&config, json!("file:trip/missing.jpg"));
    assert!(f.contains("'trip/missing.jpg' not found in"), "{f}");
    let f = picture_fix(&config, json!("file:trip/ok.jpeg"));
    assert!(
        f.contains("— use \"trip/ok.jpg\""),
        "near names are offered: {f}"
    );
    let f = picture_fix(&config, json!("file:trip"));
    assert!(f.contains("is a folder, not an image"), "{f}");
}

#[test]
fn hostile_folders_are_fixes_and_missing_ones_notes() {
    let (root, _) = scratch_root("folders");
    let config = config_with_root(&root);
    let story = story_with(json!("rocket"));
    for bad in ["../outside", "/tmp", "escape"] {
        let f = prepare(&config, story.clone(), Some(bad)).unwrap_err();
        assert_eq!(f[0].field, "assets", "{bad}");
        assert!(
            f[0].problem.contains("outside") || f[0].problem.contains("absolute"),
            "{bad}: {f:?}"
        );
    }
    let p = prepare(&config, story.clone(), Some("nope")).unwrap();
    assert!(p
        .changed
        .contains(&"assets 'nope' not found; using library pictures".to_string()));
    // The root's own path written out is accepted.
    let sb = Sandbox {
        roots: &config.asset_roots,
        repo: &config.repo,
    };
    assert!(sb.resolve("./trip/ok.jpg").is_ok());
}

#[test]
fn a_good_folder_resolves_with_roles() {
    let config = common::config(Profile::Creator, "sandbox_demo");
    let story = json!({"title": "garage", "beats": [
        {"say": "Meet Sam, who started the whole thing in a tiny garage."},
        {"say": "This tiny rocket was the very first thing we ever built.", "picture": "rocket"},
        {"say": "We launched it on a windy morning down at the beach."},
        {"say": "Three hundred people came to watch and nobody left early.", "list": ["friends", "family", "strangers"]}
    ]});
    let p = prepare(&config, story, Some("demo")).unwrap_or_else(|f| panic!("{f:#?}"));
    let images = p.images.clone().expect("images");
    let by_id = |id: &str| images.entries.iter().find(|e| e.id == id).cloned();
    // beat2_object.png: role from the name; replaces the library noun.
    let obj = by_id("beat_2.hero_object").expect("object image");
    assert_eq!(obj.role, ImageRole::Object);
    assert!(!obj.role_guessed);
    assert!(obj.serves.contains(&"beat_2.supporting_object".to_string()));
    assert!(
        p.changed
            .contains(&"beat 2: picture 'rocket' → your image beat2_object.png".to_string()),
        "{:?}",
        p.changed
    );
    assert!(p.findings.iter().any(|f| f == "beat 2: photo is 300 px tall, needs ≥ 720 — use a larger file or a library picture"), "{:?}", p.findings);
    // background.jpg: the environment of beats 3 and 4 only.
    for n in [3, 4] {
        let e = by_id(&format!("beat_{n}.environment")).expect("background");
        assert_eq!(e.role, ImageRole::Place);
        assert_eq!(e.file_name, "background.jpg");
    }
    assert!(by_id("beat_1.environment").is_none());
    // beat1.jpg: no role in the name; Apple Vision finds the person.
    if common::has_vision() {
        let person = by_id("beat_1.hero_subject").expect("person image");
        assert_eq!(person.role, ImageRole::Person);
        assert!(person.role_guessed);
        assert_eq!(p.intent.beats[0].primary.kind_name(), "phrase");
        if common::engine().is_file() {
            assert!(person.cutout, "an opaque person photo is cut out");
            assert!(person.alpha && person.file_name.ends_with(".png"));
        } else {
            eprintln!(
                "skip cutout check: no engine at {}",
                common::engine().display()
            );
        }
    } else {
        eprintln!("skip role guess: Apple Vision is not available here");
        assert!(by_id("beat_1.hero_object").is_some());
    }
    assert_eq!(p.request.assets.as_deref(), Some("demo"));
    assert!(
        p.plan
            .iter()
            .any(|l| l.starts_with("background: photo background.jpg (place")),
        "{:?}",
        p.plan
    );

    // The job manifest: files copied, a valid engine manifest.
    let job = Path::new(env!("CARGO_TARGET_TMPDIR")).join("motion-mcp-sandbox-job");
    let _ = std::fs::remove_dir_all(&job);
    let path = byo::write_manifest(&images, &job).unwrap();
    assert_eq!(path, job.join("assets.manifest.json"));
    let m: motion_core::assets::AssetManifest =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    m.validate().unwrap();
    for e in &m.assets {
        assert!(job.join(&e.path).is_file(), "{}", e.path);
        assert!(e.path.starts_with("images/"));
    }
    // Same folder, same images → same job id; the fingerprint has no paths.
    let p2 = prepare(
        &config,
        serde_json::from_value(p.request.story.clone()).unwrap(),
        Some("demo"),
    )
    .unwrap();
    assert_eq!(
        motion_mcp::job::job_id(&p.key),
        motion_mcp::job::job_id(&p2.key)
    );
    assert!(!images.fingerprint().to_string().contains('/'));
}

#[test]
fn a_ready_manifest_passes_through_after_the_sandbox() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("motion-mcp-manifest");
    let root = base.join("inbox");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(root.join("kit")).unwrap();
    std::fs::copy(
        common::fixtures().join("inbox/demo/beat2_object.png"),
        root.join("kit/thing.png"),
    )
    .unwrap();
    let manifest = json!({"version": "0.2", "assets": [
        {"id": "beat_1.hero_object", "path": "thing.png", "width": 1, "height": 1, "alpha": true,
         "subject_anchor": {"x": 0.5, "y": 0.6}}
    ]});
    std::fs::write(root.join("kit/manifest.json"), manifest.to_string()).unwrap();
    let mut config = common::config(Profile::Operator, "sandbox_manifest");
    config.asset_roots = vec![root.clone()];
    let p = prepare(&config, story_with(json!("rocket")), Some("kit")).unwrap();
    let e = &p.images.as_ref().unwrap().entries[0];
    assert_eq!(e.id, "beat_1.hero_object");
    assert_eq!((e.width, e.height), (211, 300), "measured, not trusted");
    let json = byo::manifest_json(p.images.as_ref().unwrap()).unwrap();
    assert_eq!(json["assets"][0]["subject_anchor"]["y"], 0.6);
    // A manifest path leading out of the folder is refused.
    let bad = json!({"version": "0.2", "assets": [
        {"id": "beat_1.hero_object", "path": "../../outside.png", "width": 1, "height": 1, "alpha": true}
    ]});
    std::fs::write(root.join("kit/manifest.json"), bad.to_string()).unwrap();
    let f = prepare(&config, story_with(json!("rocket")), Some("kit")).unwrap_err();
    assert_eq!(f[0].field, "assets");
    assert!(f[0].problem.contains("outside the asset folder"), "{f:?}");
}

#[test]
fn the_vision_probe_guesses_roles() {
    if !common::has_vision() {
        eprintln!("skip: Apple Vision is not available here");
        return;
    }
    let config = common::config(Profile::Weak, "sandbox_probe");
    let sb = Sandbox {
        roots: &config.asset_roots,
        repo: &config.repo,
    };
    let role = |r: &str| {
        let f = sb.resolve(r).unwrap();
        let p = byo::probe(&f, &config).unwrap();
        // Cached by hash.
        assert!(byo::cache_dir(&config, &f.sha256)
            .join("probe.json")
            .is_file());
        (byo::guess_role(&p), p)
    };
    let (r, p) = role("demo/beat1.jpg");
    assert_eq!(r, ImageRole::Person, "{p:?}");
    assert!(p.opaque && p.humans >= 1);
    let (r, p) = role("demo/background.jpg");
    assert_eq!(r, ImageRole::Place, "{p:?}");
    let (r, p) = role("demo/beat2_object.png");
    assert_eq!(r, ImageRole::Object, "{p:?}");
    assert!(!p.opaque);
}

#[test]
fn small_photos_get_size_feedback() {
    let config = common::config(Profile::Weak, "sandbox_small");
    let p = prepare(
        &config,
        story_with(json!({"file": "loose/small.jpg", "role": "place"})),
        None,
    )
    .unwrap();
    assert!(
        p.findings.contains(
            &"beat 1: photo is 320 px tall, needs ≥ 720 — use a larger file or a library picture"
                .to_string()
        ),
        "{:?}",
        p.findings
    );
    assert!(
        p.plan[0].contains("photo small.jpg (place, 320 px tall)"),
        "{:?}",
        p.plan
    );
    let e = &p.images.unwrap().entries[0];
    assert_eq!(e.id, "beat_1.environment");
    assert!(e.serves.contains(&"beat_1.hero_object".to_string()));
}
