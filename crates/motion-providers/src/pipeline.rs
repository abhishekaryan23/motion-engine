//! Policy-gated generation: allow-list → cache → quote → caps → submit → poll → cache → log.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde_json::json;

use crate::cache::AssetCache;
use crate::jsonx::norm;
use crate::policy::{month_of, now_secs, Policy, SpendEntry, SpendLog, SpendTotals};
use crate::provider::{AssetKind, AssetProvider, AssetRequest, JobState, ModelInfo, Quote};

#[derive(Debug, Clone, Copy)]
pub struct PollOpts {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for PollOpts {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(4),
            timeout: Duration::from_secs(900),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    CacheHit(PathBuf),
    Generated { path: PathBuf, quote: Quote },
}

#[allow(clippy::too_many_arguments)]
/// `on_quote` sees the quote and the current totals before any spend.
pub fn generate(
    p: &mut dyn AssetProvider,
    req: &AssetRequest,
    policy: &Policy,
    log: &SpendLog,
    cache: &AssetCache,
    video: &str,
    poll: PollOpts,
    on_quote: &mut dyn FnMut(&Quote, &SpendTotals),
) -> Result<Outcome> {
    let key = AssetCache::key(p.name(), req);
    generate_keyed(
        p, req, &key, policy, log, cache, video, poll, None, on_quote,
    )
}

/// Turns downloaded bytes into the cached asset (e.g. keying a cutout).
pub type PostProcess<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<u8>>;

/// [`generate`] with an explicit cache key and an optional post-process
/// (whose output is stored as PNG).
#[allow(clippy::too_many_arguments)]
pub fn generate_keyed(
    p: &mut dyn AssetProvider,
    req: &AssetRequest,
    key: &str,
    policy: &Policy,
    log: &SpendLog,
    cache: &AssetCache,
    video: &str,
    poll: PollOpts,
    post: Option<PostProcess>,
    on_quote: &mut dyn FnMut(&Quote, &SpendTotals),
) -> Result<Outcome> {
    policy.check_model(req)?;
    let key = key.to_string();
    if let Some(path) = cache.lookup(&key) {
        return Ok(Outcome::CacheHit(path));
    }
    let quote = p.quote(req)?;
    let now = now_secs();
    let month = month_of(now);
    let totals = log.totals(video, &month)?;
    on_quote(&quote, &totals);
    policy.check_spend(quote.credits, &totals)?;

    let job = p.submit(req)?;
    log.append(&SpendEntry {
        ts: now,
        month,
        video: video.to_string(),
        provider: p.name().to_string(),
        model: req.model.clone(),
        credits: quote.credits,
        cache_key: key.clone(),
        job: job.id.clone(),
    })?;
    let start = Instant::now();
    let urls = loop {
        match p.poll(&job)? {
            JobState::Done(urls) => break urls,
            JobState::Failed(why) => bail!("generation failed (job {}): {why}", job.id),
            JobState::Pending => {
                if start.elapsed() > poll.timeout {
                    bail!("job {} still pending after {:?}", job.id, poll.timeout);
                }
                std::thread::sleep(poll.interval);
            }
        }
    };
    let url = &urls[0];
    let mut bytes = p.download(url)?;
    if let Some(post) = post {
        bytes = post(&bytes)?;
    }
    let ext = if post.is_some() {
        "png".to_string()
    } else {
        url.split(['?', '#'])
            .next()
            .and_then(|u| u.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase())
            .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or_else(|| "bin".into())
    };
    let meta = json!({
        "provider": p.name(), "request": req, "credits": quote.credits,
        "job": job.id, "url": url, "ts": now,
    });
    let path = cache.store(&key, &ext, &bytes, &meta)?;
    Ok(Outcome::Generated { path, quote })
}

/// One image to make, provider-neutral (v2.3 fallback).
#[derive(Debug, Clone, PartialEq)]
pub struct ImageSpec {
    pub prompt: String,
    /// One isolated object, keyed to RGBA (magenta ground).
    pub cutout: bool,
    /// `16:9`, `9:16`, `1:1`.
    pub aspect: String,
    /// Always explicit.
    pub seed: u32,
}

/// What the fallback produced, and who made it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub provider: String,
    pub model: String,
    pub path: PathBuf,
    pub credits: f64,
    pub cache_hit: bool,
}

/// The first allowed image model a provider's catalog offers, in policy
/// order: `(catalog model id, required quality)`.
pub fn pick_image_model(
    policy: &Policy,
    catalog: &[ModelInfo],
) -> Option<(String, Option<String>)> {
    policy
        .allow
        .iter()
        .filter(|r| r.kind == AssetKind::Image)
        .find_map(|r| {
            let want = norm(&r.model);
            catalog
                .iter()
                .find(|m| !m.id.is_empty() && norm(&m.id).contains(&want))
                .map(|m| (m.id.clone(), r.quality.clone()))
        })
}

/// The provider request for `spec`, and the request its cache key is taken
/// from (which always carries the seed and the cutout flag).
pub fn image_request(
    provider: &str,
    model: &str,
    quality: Option<String>,
    spec: &ImageSpec,
) -> (AssetRequest, AssetRequest) {
    let moss = provider == crate::moss::NAME;
    let prompt = if spec.cutout && !moss {
        crate::cutout::magenta_ground_prompt(&spec.prompt)
    } else {
        spec.prompt.trim().to_string()
    };
    let mut req = AssetRequest {
        kind: AssetKind::Image,
        model: model.to_string(),
        prompt,
        quality,
        aspect: Some(spec.aspect.clone()),
        params: BTreeMap::new(),
    };
    let mut keyed = req.clone();
    keyed.params.insert("seed".into(), json!(spec.seed));
    keyed.params.insert("cutout".into(), json!(spec.cutout));
    if moss {
        req.params = keyed.params.clone();
    }
    (req, keyed)
}

/// Opens a provider by name with its poll timing; an error (not connected,
/// no adapter) skips it.
pub type Opener<'a, 'p> =
    &'a mut dyn FnMut(&str) -> Result<(Box<dyn AssetProvider + 'p>, PollOpts)>;

/// Try `order` (normally `policy.provider_order`) until one provider makes
/// the image. A provider is skipped, with the reason passed to `on_event`,
/// when it cannot be opened (not connected), its catalog fails (auth,
/// network), it offers no allowed image model, the caps refuse its quote,
/// or its job fails. Cutouts are keyed with `keyer` before caching.
#[allow(clippy::too_many_arguments)]
pub fn generate_image_with_fallback(
    order: &[String],
    open: Opener,
    spec: &ImageSpec,
    policy: &Policy,
    log: &SpendLog,
    cache: &AssetCache,
    video: &str,
    keyer: PostProcess,
    on_event: &mut dyn FnMut(&str),
) -> Result<Resolved> {
    let mut skipped: Vec<String> = Vec::new();
    for name in order {
        let mut skip = |why: String, on_event: &mut dyn FnMut(&str)| {
            let line = format!("{name}: skipped ({why})");
            on_event(&line);
            skipped.push(line);
        };
        let (mut p, poll) = match open(name) {
            Ok(x) => x,
            Err(e) => {
                skip(format!("{e:#}"), on_event);
                continue;
            }
        };
        let catalog = match p.catalog() {
            Ok(c) => c,
            Err(e) => {
                skip(format!("catalog: {e:#}"), on_event);
                continue;
            }
        };
        let Some((model, quality)) = pick_image_model(policy, &catalog) else {
            skip("no allowed image model in its catalog".into(), on_event);
            continue;
        };
        let (req, keyed) = image_request(name, &model, quality, spec);
        let key = AssetCache::key(name, &keyed);
        let post = spec.cutout.then_some(keyer);
        let mut quoted = false;
        match generate_keyed(
            p.as_mut(),
            &req,
            &key,
            policy,
            log,
            cache,
            video,
            poll,
            post,
            // The quote is shown before the caps decide (and before any spend).
            &mut |q, t| {
                quoted = true;
                on_event(&format!(
                    "{name}: quote {} credits ({}; video {video}: {} / {}, month {} / {})",
                    q.credits, q.model, t.video, policy.per_video_cap, t.month, policy.monthly_cap
                ));
            },
        ) {
            Ok(Outcome::CacheHit(path)) => {
                on_event(&format!("{name}: cache hit ({model})"));
                return Ok(Resolved {
                    provider: name.clone(),
                    model,
                    path,
                    credits: 0.0,
                    cache_hit: true,
                });
            }
            Ok(Outcome::Generated { path, quote }) => {
                on_event(&format!(
                    "{name}: generated ({model}, {} credits)",
                    quote.credits
                ));
                return Ok(Resolved {
                    provider: name.clone(),
                    model,
                    path,
                    credits: quote.credits,
                    cache_hit: false,
                });
            }
            Err(e) => {
                let stage = if quoted { "after quote" } else { "quote" };
                skip(format!("{stage}: {e:#}"), on_event);
            }
        }
    }
    bail!(
        "no provider could make the image:\n  {}",
        if skipped.is_empty() {
            "provider_order is empty".to_string()
        } else {
            skipped.join("\n  ")
        }
    )
}
