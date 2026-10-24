//! (v2.3) MOSS provider over a local mock HTTP server, the image fallback
//! with fake providers (no network, no credits), the cache, the lockfile and
//! the cutout keyer.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use base64::Engine as _;
use motion_providers::lockfile::{lock_path, resolve_all, LockAction, Lockfile};
use motion_providers::pipeline::{generate_image_with_fallback, ImageSpec, PollOpts, Resolved};
use motion_providers::policy::{Policy, SpendLog};
use motion_providers::{
    cutout, moss, AssetCache, AssetProvider, AssetRequest, Job, JobState, ModelInfo, MossProvider,
    Quote, Unavailable,
};
use resvg::tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};
use serde_json::{json, Value};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "motion-moss-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A small PNG: `ground` with a teal disc in the middle.
fn png(ground: Color, w: u32, h: u32) -> Vec<u8> {
    let mut pm = Pixmap::new(w, h).unwrap();
    pm.fill(ground);
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(30, 160, 150, 255));
    let r = w.min(h) as f32 * 0.3;
    let path = PathBuilder::from_circle(w as f32 / 2.0, h as f32 / 2.0, r).unwrap();
    pm.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    pm.encode_png().unwrap()
}

/// Mock `sd-server`: records POST bodies; a job is pending once, then done.
fn mock_moss() -> (String, Arc<Mutex<Vec<Value>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = bodies.clone();
    let image = base64::engine::general_purpose::STANDARD.encode(png(Color::WHITE, 32, 18));
    std::thread::spawn(move || {
        let mut polls = 0;
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let reply = if line.starts_with("POST /sdcpp/v1/img_gen") {
                seen.lock()
                    .unwrap()
                    .push(serde_json::from_slice(&body).unwrap());
                json!({"id": "job_1", "poll_url": "/sdcpp/v1/jobs/job_1"})
            } else if line.starts_with("GET /sdcpp/v1/jobs/job_1") {
                polls += 1;
                if polls == 1 {
                    json!({"status": "processing"})
                } else {
                    json!({"status": "completed", "result": {"images": [{"b64_json": image}]}})
                }
            } else {
                json!({"error": "not found"})
            };
            let b = reply.to_string();
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                b.len()
            );
        }
    });
    (base, bodies)
}

fn fast() -> PollOpts {
    PollOpts {
        interval: Duration::from_millis(10),
        timeout: Duration::from_secs(5),
    }
}

fn spec(prompt: &str, seed: u32) -> ImageSpec {
    ImageSpec {
        prompt: prompt.into(),
        cutout: false,
        aspect: "16:9".into(),
        seed,
    }
}

struct Env {
    policy: Policy,
    log: SpendLog,
    cache: AssetCache,
}

fn env(tag: &str) -> Env {
    let d = tmp(tag);
    Env {
        policy: Policy::default(),
        log: SpendLog::new(d.join("spend.jsonl")),
        cache: AssetCache::new(d.join("cache")),
    }
}

fn no_key(_: &[u8]) -> Result<Vec<u8>> {
    Err(anyhow!("keyer must not run for plates"))
}

#[test]
fn moss_request_shape_and_result() {
    let (base, bodies) = mock_moss();
    let e = env("shape");
    let mut events = Vec::new();
    let mut open = |name: &str| -> Result<(Box<dyn AssetProvider>, PollOpts)> {
        assert_eq!(name, "moss");
        Ok((Box::new(MossProvider::new(&base)), fast()))
    };
    let r = generate_image_with_fallback(
        &["moss".to_string()],
        &mut open,
        &spec("a crisp fern macro", 7),
        &e.policy,
        &e.log,
        &e.cache,
        "t",
        &no_key,
        &mut |l| events.push(l.to_string()),
    )
    .unwrap();
    assert_eq!(
        (r.provider.as_str(), r.model.as_str()),
        ("moss", "moss-image-sft")
    );
    assert!(!r.cache_hit && r.credits == 0.0);
    assert!(std::fs::read(&r.path).unwrap().starts_with(b"\x89PNG"));
    let b = bodies.lock().unwrap()[0].clone();
    assert_eq!(b["prompt"], "a crisp fern macro");
    assert_eq!(
        (b["width"].as_u64(), b["height"].as_u64()),
        (Some(768), Some(432))
    );
    assert_eq!(b["seed"], 7);
    assert_eq!(b["sample_params"]["sample_method"], "euler");
    assert_eq!(b["sample_params"]["sample_steps"], 4);
    assert_eq!(
        b["sample_params"]["custom_sigmas"],
        json!([1.0, 0.91, 0.717, 0.024, 0.0])
    );
    assert_eq!(b["sample_params"]["guidance"]["txt_cfg"], 1.0);
    assert_eq!(b["lora"][0]["path"], "moss_4step_f16.safetensors");
    assert_eq!(b["lora"][0]["multiplier"], 1.0);
    // Spend log: provider and model actually used, 0 credits.
    let log = e.log.entries().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(
        (
            log[0].provider.as_str(),
            log[0].model.as_str(),
            log[0].credits
        ),
        ("moss", "moss-image-sft", 0.0)
    );
}

#[test]
fn moss_cutout_wraps_prompt_on_magenta() {
    let (base, bodies) = mock_moss();
    let mut p = MossProvider::new(&base);
    let mut req = AssetRequest {
        kind: motion_providers::AssetKind::Image,
        model: "moss-image-sft".into(),
        prompt: "a brass compass".into(),
        quality: None,
        aspect: Some("1:1".into()),
        params: Default::default(),
    };
    // No seed → refused (the seed is always explicit).
    assert!(p.submit(&req).is_err());
    req.params.insert("seed".into(), json!(3));
    req.params.insert("cutout".into(), json!(true));
    p.submit(&req).unwrap();
    let b = bodies.lock().unwrap()[0].clone();
    assert_eq!(
        (b["width"].as_u64(), b["height"].as_u64()),
        (Some(576), Some(576))
    );
    let prompt = b["prompt"].as_str().unwrap();
    assert!(prompt.contains("#FF00FF") && prompt.contains("a brass compass"));
    assert!(prompt.contains("\"bbox_format\": \"yxyx\""));
}

#[test]
fn unreachable_server_is_unavailable_fast() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut p = MossProvider::new(&format!("http://127.0.0.1:{port}"));
    let mut req = AssetRequest {
        kind: motion_providers::AssetKind::Image,
        model: "moss-image-sft".into(),
        prompt: "x y z".into(),
        quality: None,
        aspect: None,
        params: Default::default(),
    };
    req.params.insert("seed".into(), json!(1));
    let t0 = Instant::now();
    let err = p.submit(&req).unwrap_err();
    assert!(t0.elapsed() < Duration::from_secs(4), "{:?}", t0.elapsed());
    assert!(err.downcast_ref::<Unavailable>().is_some(), "{err:#}");
    assert!(err.to_string().contains("docs/PROVIDERS.md"));
    assert!(!p.reachable());
}

/// An in-memory provider: catalog, fixed quote, instant job.
struct Fake {
    name: String,
    models: Vec<&'static str>,
    credits: f64,
    submits: Arc<Mutex<Vec<AssetRequest>>>,
}

impl AssetProvider for Fake {
    fn name(&self) -> &str {
        &self.name
    }
    fn catalog(&mut self) -> Result<Vec<ModelInfo>> {
        Ok(self
            .models
            .iter()
            .map(|m| ModelInfo {
                id: m.to_string(),
                name: m.to_string(),
                kind: "image".into(),
            })
            .collect())
    }
    fn balance(&mut self) -> Result<Option<f64>> {
        Ok(None)
    }
    fn quote(&mut self, req: &AssetRequest) -> Result<Quote> {
        Ok(Quote {
            provider: self.name.clone(),
            model: req.model.clone(),
            credits: self.credits,
        })
    }
    fn submit(&mut self, req: &AssetRequest) -> Result<Job> {
        self.submits.lock().unwrap().push(req.clone());
        Ok(Job {
            id: "j".into(),
            urls: vec!["https://fake/x.png".into()],
        })
    }
    fn poll(&mut self, job: &Job) -> Result<JobState> {
        Ok(JobState::Done(job.urls.clone()))
    }
    fn download(&mut self, _: &str) -> Result<Vec<u8>> {
        Ok(png(Color::WHITE, 16, 16))
    }
}

type Calls = Arc<Mutex<Vec<AssetRequest>>>;

/// Fallback over fakes: `openart` connected or not; `moss` always there.
fn run_fallback(e: &Env, openart: bool, s: &ImageSpec) -> (Resolved, Vec<String>, Calls, Calls) {
    let (oa, ms): (Calls, Calls) = Default::default();
    let (oa2, ms2) = (oa.clone(), ms.clone());
    let mut open = move |name: &str| -> Result<(Box<dyn AssetProvider>, PollOpts)> {
        match name {
            "openart" if openart => Ok((
                Box::new(Fake {
                    name: "openart".into(),
                    models: vec!["openai/gpt-image-2.5-flare", "flux-pro"],
                    credits: 5.0,
                    submits: oa2.clone(),
                }),
                fast(),
            )),
            "moss" => Ok((
                Box::new(Fake {
                    name: "moss".into(),
                    models: vec!["moss-image-sft"],
                    credits: 0.0,
                    submits: ms2.clone(),
                }),
                fast(),
            )),
            _ => Err(anyhow!("not connected")),
        }
    };
    let mut events = Vec::new();
    let r = generate_image_with_fallback(
        &e.policy.provider_order,
        &mut open,
        s,
        &e.policy,
        &e.log,
        &e.cache,
        "v",
        &no_key,
        &mut |l| events.push(l.to_string()),
    )
    .unwrap();
    (r, events, oa, ms)
}

#[test]
fn fallback_skips_unconnected_openart_to_moss() {
    let e = env("fb1");
    assert_eq!(e.policy.provider_order, ["openart", "higgsfield", "moss"]);
    let (r, events, oa, ms) = run_fallback(&e, false, &spec("a fern macro plate", 4));
    assert_eq!(r.provider, "moss");
    assert!(oa.lock().unwrap().is_empty());
    assert_eq!(ms.lock().unwrap().len(), 1);
    assert_eq!(ms.lock().unwrap()[0].params["seed"], 4);
    assert!(
        events[0].starts_with("openart: skipped (not connected"),
        "{events:?}"
    );
    assert!(events[1].starts_with("higgsfield: skipped"), "{events:?}");
    let log = e.log.entries().unwrap();
    assert_eq!(
        (
            log[0].provider.as_str(),
            log[0].model.as_str(),
            log[0].credits
        ),
        ("moss", "moss-image-sft", 0.0)
    );
}

#[test]
fn fallback_prefers_connected_openart_with_gpt_image_rules() {
    let e = env("fb2");
    let (r, events, oa, ms) = run_fallback(&e, true, &spec("a fern macro plate", 4));
    assert_eq!(
        (r.provider.as_str(), r.model.as_str()),
        ("openart", "openai/gpt-image-2.5-flare")
    );
    assert!(
        events[0]
            .starts_with("openart: quote 5 credits (openai/gpt-image-2.5-flare; video v: 0 / 150"),
        "{events:?}"
    );
    assert!(events[1].starts_with("openart: generated"), "{events:?}");
    assert_eq!(r.credits, 5.0);
    let req = oa.lock().unwrap()[0].clone();
    assert_eq!(req.quality.as_deref(), Some("low"));
    assert_eq!(req.aspect.as_deref(), Some("16:9"));
    assert!(req.params.is_empty(), "no MOSS params sent to OpenArt");
    assert!(ms.lock().unwrap().is_empty());
    let log = e.log.entries().unwrap();
    assert_eq!((log[0].provider.as_str(), log[0].credits), ("openart", 5.0));
}

#[test]
fn fallback_skips_when_caps_refuse_the_quote() {
    let mut e = env("fb3");
    e.policy.max_per_request = 1.0; // openart's 5 credits are refused
    let (r, events, oa, _) = run_fallback(&e, true, &spec("a fern macro plate", 4));
    assert_eq!(r.provider, "moss");
    assert!(
        oa.lock().unwrap().is_empty(),
        "nothing submitted to openart"
    );
    // The quote is shown, then the caps skip openart.
    assert!(
        events[0].starts_with("openart: quote 5 credits"),
        "{events:?}"
    );
    assert!(
        events[1].starts_with("openart: skipped") && events[1].contains("max_per_request"),
        "{events:?}"
    );
}

#[test]
fn second_request_is_a_cache_hit() {
    let e = env("cache");
    let s = spec("a fern macro plate", 9);
    let (a, _, _, ms) = run_fallback(&e, false, &s);
    let (b, events, _, ms2) = run_fallback(&e, false, &s);
    assert!(!a.cache_hit && b.cache_hit);
    assert_eq!(a.path, b.path);
    assert_eq!(ms.lock().unwrap().len(), 1);
    assert!(ms2.lock().unwrap().is_empty(), "no model call on a hit");
    assert!(events.iter().any(|l| l.contains("cache hit")));
    // A different seed is a different asset.
    let (c, _, _, _) = run_fallback(&e, false, &spec("a fern macro plate", 10));
    assert!(!c.cache_hit && c.path != a.path);
}

#[test]
fn lockfile_written_and_reused() {
    let e = env("lock");
    let dir = tmp("lockdir");
    let program = dir.join("demo.program.json");
    assert_eq!(lock_path(&program), dir.join("demo.assets.lock.json"));
    let specs = vec![
        ("sea".to_string(), spec("ocean spray macro plate", 7)),
        ("fern".to_string(), spec("a fern macro plate", 11)),
    ];
    let mut calls = 0;
    let mut make = |_: &str, s: &ImageSpec| {
        calls += 1;
        Ok(run_fallback(&e, false, s).0)
    };
    let mut lock = Lockfile::default();
    let done = resolve_all(&mut lock, &specs, false, &mut make).unwrap();
    assert!(done.iter().all(|(_, a)| matches!(a, LockAction::Made(_))));
    lock.save(&lock_path(&program)).unwrap();
    let mut lock2 = Lockfile::load(&lock_path(&program)).unwrap();
    assert_eq!(lock2, lock);
    let entry = &lock2.assets["sea"];
    assert_eq!((entry.provider.as_str(), entry.seed), ("moss", 7));
    assert!(PathBuf::from(&entry.path).is_absolute());
    let done = resolve_all(&mut lock2, &specs, true, &mut make).unwrap();
    assert!(done.iter().all(|(_, a)| *a == LockAction::Reused));
    assert_eq!(lock2.paths().len(), 2);
    assert_eq!(calls, 2, "reuse makes no model call");

    // A changed prompt is not reused; offline then refuses.
    let changed = vec![("sea".to_string(), spec("ocean spray, wider", 7))];
    let mut never = |_: &str, _: &ImageSpec| -> Result<Resolved> { Err(anyhow!("no call")) };
    let err = resolve_all(&mut lock2.clone(), &changed, true, &mut never).unwrap_err();
    assert!(err.to_string().contains("run `program assets`"), "{err}");
    // A file that no longer matches its sha256 is not resolved.
    std::fs::write(&lock2.assets["fern"].path, b"tampered").unwrap();
    assert!(!lock2.paths().contains_key("fern"));
}

/// The keying scripts need python3 with numpy, scipy and Pillow.
fn keying_tools_available() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import numpy, scipy, PIL"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn keyed_cutout_is_magenta_free() {
    if !keying_tools_available() {
        eprintln!("skipping: python3 with numpy, scipy and Pillow is not available");
        return;
    }
    let raw = png(Color::from_rgba8(255, 0, 255, 255), 256, 256);
    // The raw generation is all magenta ground: the check refuses it.
    assert!(cutout::check_magenta_free(&raw).is_err());
    let keyed = cutout::key_magenta(&raw, &cutout::scripts_dir()).unwrap();
    cutout::check_magenta_free(&keyed).unwrap();
    let (residue, cast, clear) = cutout::magenta_stats(&keyed).unwrap();
    assert_eq!(residue, 0);
    assert!(cast <= cutout::CAST_TOLERANCE && clear > 0);
    // The teal disc survives, opaque.
    let pm = Pixmap::decode_png(&keyed).unwrap();
    let c = pm
        .pixel(pm.width() / 2, pm.height() / 2)
        .unwrap()
        .demultiply();
    assert_eq!(c.alpha(), 255);
    assert!(c.green() > 120 && c.red() < 80, "{c:?}");
}

#[test]
fn moss_name_and_catalog() {
    let mut p = MossProvider::new(moss::DEFAULT_URL);
    assert_eq!(p.name(), "moss");
    assert_eq!(p.catalog().unwrap()[0].id, "moss-image-sft");
    assert_eq!(p.balance().unwrap(), None);
}
