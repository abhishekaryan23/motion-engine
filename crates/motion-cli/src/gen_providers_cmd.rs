//! (P1) Generation providers: `providers connect|tools|quote|generate` and the
//! generation half of `providers show`. Connections live in the user config
//! (`connections.json`, 0600); the policy in `providers.policy.json` beside it.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use motion_providers::policy::{month_of, now_secs, POLICY_FILE, SPEND_LOG_FILE};
use motion_providers::store::{config_dir, mask, CONNECTIONS_FILE};
use motion_providers::{
    generate, generate_image_with_fallback, moss, oauth, Adapter, AssetCache, AssetKind,
    AssetProvider, AssetRequest, Auth, Connection, Connections, ImageSpec, McpProvider,
    MossProvider, Outcome, Policy, PollOpts, Resolved, SpendLog, UreqTransport,
};
use serde_json::Value;

#[derive(Args, Debug)]
pub struct GenArgs {
    /// Connected provider (e.g. `openart`), `moss` (the local MOSS server) or
    /// `auto` (images: the policy's provider_order, MOSS last).
    pub provider: String,
    /// Model id as the platform names it (must be on the policy allow-list);
    /// `moss` and `auto` pick it themselves.
    #[arg(long, default_value = "")]
    pub model: String,
    #[arg(long)]
    pub prompt: String,
    #[arg(long, value_parser = ["image", "video"], default_value = "image")]
    pub kind: String,
    #[arg(long, default_value = "low")]
    pub quality: String,
    /// e.g. 1:1, 9:16.
    #[arg(long)]
    pub aspect: Option<String>,
    /// Extra tool argument `key=value` (value parsed as JSON when it parses).
    #[arg(long = "param")]
    pub params: Vec<String>,
    /// Video the spend is booked against (per-video cap).
    #[arg(long, default_value = "adhoc")]
    pub video: String,
    /// Copy the asset here (default: print the cache path only).
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// (moss/auto) One isolated object keyed to a transparent PNG.
    #[arg(long)]
    pub cutout: bool,
    /// (moss/auto) Explicit seed: the same prompt and seed give the same image.
    #[arg(long, default_value_t = 0)]
    pub seed: u32,
}

fn connections_path() -> PathBuf {
    config_dir().join(CONNECTIONS_FILE)
}

fn request(a: &GenArgs) -> Result<AssetRequest> {
    let mut params = std::collections::BTreeMap::new();
    for p in &a.params {
        let (k, v) = p
            .split_once('=')
            .ok_or_else(|| anyhow!("--param wants key=value: {p}"))?;
        let v = serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string()));
        params.insert(k.to_string(), v);
    }
    Ok(AssetRequest {
        kind: if a.kind == "video" {
            AssetKind::Video
        } else {
            AssetKind::Image
        },
        model: a.model.clone(),
        prompt: a.prompt.clone(),
        quality: (!a.quality.is_empty() && a.quality != "none").then(|| a.quality.clone()),
        aspect: a.aspect.clone(),
        params,
    })
}

fn load_conn(name: &str) -> Result<(Connections, Connection)> {
    let all = Connections::load(&connections_path())?;
    let c = all.providers.get(name).cloned().ok_or_else(|| {
        anyhow!("'{name}' is not connected; run `motion-engine providers connect {name}`")
    })?;
    Ok((all, c))
}

fn adapter_for(name: &str, c: &Connection) -> Adapter {
    Adapter::for_name(&c.adapter).unwrap_or_else(|| Adapter::generic(name, &c.url))
}

/// Run `f` with a provider; save refreshed tokens afterwards.
fn with_provider<T>(name: &str, f: impl FnOnce(&mut McpProvider) -> Result<T>) -> Result<T> {
    let (mut all, conn) = load_conn(name)?;
    let t = UreqTransport::default();
    let mut p = McpProvider::new(adapter_for(name, &conn), conn, &t);
    let r = f(&mut p);
    if p.auth_changed {
        all.providers.insert(name.to_string(), p.conn.clone());
        all.save(&connections_path())?;
    }
    r
}

pub fn connect(target: &str, token_stdin: bool) -> Result<()> {
    let (name, adapter) = match Adapter::for_name(target) {
        Some(a) => (target.to_string(), a),
        None if target.starts_with("https://") => {
            let host = target
                .trim_start_matches("https://")
                .split(['/', ':'])
                .next()
                .unwrap_or("mcp");
            (host.to_string(), Adapter::generic(host, target))
        }
        None => bail!("unknown provider '{target}' (openart, or an https MCP URL)"),
    };
    let t = UreqTransport::default();
    let auth = if token_stdin {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("reading token from stdin")?;
        let tok = s.trim();
        if tok.is_empty() {
            bail!("no token on stdin");
        }
        Auth {
            access_token: tok.to_string(),
            ..Default::default()
        }
    } else {
        oauth::browser_login(&t, &adapter.url, &|url| {
            eprintln!("Sign in to {name} in the browser (opening it now). If it does not open, visit:\n{url}");
            let _ = std::process::Command::new("open").arg(url).status();
        })?
    };
    let conn = Connection {
        adapter: if Adapter::for_name(&name).is_some() {
            name.clone()
        } else {
            "mcp".into()
        },
        url: adapter.url.clone(),
        auth,
    };
    // Prove the connection before saving it.
    let mut p = McpProvider::new(adapter, conn, &t);
    let n = p.tools()?.len();
    let bal = p.balance().ok().flatten();
    let path = connections_path();
    let mut all = Connections::load(&path)?;
    all.providers.insert(name.clone(), p.conn.clone());
    all.save(&path)?;
    info!(
        "connected {name}: {n} tools, balance {}",
        bal.map(|b| b.to_string()).unwrap_or_else(|| "?".into())
    );
    Ok(())
}

/// Generation half of `providers show`.
pub fn show() -> Result<()> {
    let path = connections_path();
    let all = Connections::load(&path)?;
    let policy = Policy::load_or_init(&config_dir().join(POLICY_FILE))?;
    let log = SpendLog::new(config_dir().join(SPEND_LOG_FILE));
    let month = month_of(now_secs());
    let spent = log.totals("", &month)?.month;
    println!(
        "generation: {} (month {month}: {spent} / {} credits)",
        path.display(),
        policy.monthly_cap
    );
    for (name, c) in &all.providers {
        let bal = with_provider(name, |p| p.balance())
            .map(|b| b.map(|b| b.to_string()).unwrap_or_else(|| "?".into()))
            .unwrap_or_else(|e| format!("error: {e}"));
        println!(
            "  {name}: {} token {} balance {bal}",
            c.url,
            mask(&c.auth.access_token)
        );
    }
    let m = MossProvider::new(&moss::configured_url(&config_dir()));
    println!(
        "  moss: {} ({}, 0 credits)",
        m.base,
        if m.reachable() {
            "reachable"
        } else {
            "not reachable"
        }
    );
    println!("order: {}", policy.provider_order.join(" > "));
    let allow: Vec<String> = policy
        .allow
        .iter()
        .map(|r| {
            format!(
                "{}{}",
                r.model,
                r.quality
                    .as_ref()
                    .map(|q| format!("/{q}"))
                    .unwrap_or_default()
            )
        })
        .collect();
    println!("allowed: {}", allow.join(", "));
    Ok(())
}

pub fn tools(name: &str, json: bool) -> Result<()> {
    with_provider(name, |p| {
        for t in p.tools()? {
            if json {
                println!("{} {}", t.name, t.input_schema);
            } else {
                println!("{}", t.name);
            }
        }
        Ok(())
    })
}

pub fn call(name: &str, tool: &str, args: &str) -> Result<()> {
    let args: Value = serde_json::from_str(args).context("args must be JSON")?;
    let v = with_provider(name, |p| p.call_named(tool, args))?;
    println!("{v}");
    Ok(())
}

pub fn quote(a: &GenArgs) -> Result<()> {
    let req = request(a)?;
    let policy = Policy::load_or_init(&config_dir().join(POLICY_FILE))?;
    policy.check_model(&req)?;
    let q = with_provider(&a.provider, |p| p.quote(&req))?;
    let cached =
        AssetCache::new(AssetCache::default_dir()).lookup(&AssetCache::key(&a.provider, &req));
    println!(
        "quote: {} {} = {} credits{}",
        q.provider,
        q.model,
        q.credits,
        if cached.is_some() {
            " (cached: 0 to spend)"
        } else {
            ""
        }
    );
    Ok(())
}

/// Image through the fallback (`order`, else the policy's provider_order):
/// skipped providers and the result are logged.
/// An MCP provider that saves refreshed tokens to `path` when dropped (the
/// fallback holds providers as trait objects, so `with_provider` cannot).
struct Persisting<'t> {
    name: String,
    path: PathBuf,
    inner: McpProvider<'t>,
}

impl AssetProvider for Persisting<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn catalog(&mut self) -> Result<Vec<motion_providers::ModelInfo>> {
        self.inner.catalog()
    }
    fn balance(&mut self) -> Result<Option<f64>> {
        self.inner.balance()
    }
    fn quote(&mut self, req: &AssetRequest) -> Result<motion_providers::Quote> {
        self.inner.quote(req)
    }
    fn submit(&mut self, req: &AssetRequest) -> Result<motion_providers::Job> {
        self.inner.submit(req)
    }
    fn poll(&mut self, job: &motion_providers::Job) -> Result<motion_providers::JobState> {
        self.inner.poll(job)
    }
    fn download(&mut self, url: &str) -> Result<Vec<u8>> {
        self.inner.download(url)
    }
}

impl Drop for Persisting<'_> {
    fn drop(&mut self) {
        if !self.inner.auth_changed {
            return;
        }
        let r = Connections::load(&self.path).and_then(|mut all| {
            all.providers
                .insert(self.name.clone(), self.inner.conn.clone());
            all.save(&self.path)
        });
        if let Err(e) = r {
            eprintln!(
                "warning: could not save refreshed {} token to {}: {e:#}",
                self.name,
                self.path.display()
            );
        }
    }
}

pub fn fallback_image(order: Option<&[String]>, spec: &ImageSpec, video: &str) -> Result<Resolved> {
    let policy = Policy::load_or_init(&config_dir().join(POLICY_FILE))?;
    let log = SpendLog::new(config_dir().join(SPEND_LOG_FILE));
    let cache = AssetCache::new(AssetCache::default_dir());
    let order: Vec<String> =
        order.map_or_else(|| policy.provider_order.clone(), <[String]>::to_vec);
    let t = UreqTransport::default();
    let conns = Connections::load(&connections_path())?;
    let moss_url = moss::configured_url(&config_dir());
    let mut open = |name: &str| -> Result<(Box<dyn AssetProvider + '_>, PollOpts)> {
        if name == moss::NAME {
            return Ok((
                Box::new(MossProvider::new(&moss_url)),
                MossProvider::poll_opts(),
            ));
        }
        let c = conns
            .providers
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("not connected"))?;
        Ok((
            Box::new(Persisting {
                name: name.to_string(),
                path: connections_path(),
                inner: McpProvider::new(adapter_for(name, &c), c, &t),
            }),
            PollOpts::default(),
        ))
    };
    let scripts = motion_providers::cutout::scripts_dir();
    let keyer = |b: &[u8]| motion_providers::cutout::key_magenta(b, &scripts);
    generate_image_with_fallback(
        &order,
        &mut open,
        spec,
        &policy,
        &log,
        &cache,
        video,
        &keyer,
        &mut |line| info!("{line}"),
    )
}

pub fn run_generate(a: &GenArgs) -> Result<()> {
    if a.provider == moss::NAME || a.provider == "auto" {
        if a.kind != "image" {
            bail!("{}: images only", a.provider);
        }
        let spec = ImageSpec {
            prompt: a.prompt.clone(),
            cutout: a.cutout,
            aspect: a
                .aspect
                .clone()
                .unwrap_or_else(|| if a.cutout { "1:1" } else { "16:9" }.to_string()),
            seed: a.seed,
        };
        let only = [moss::NAME.to_string()];
        let order = (a.provider == moss::NAME).then_some(&only[..]);
        let t0 = std::time::Instant::now();
        let r = fallback_image(order, &spec, &a.video)?;
        info!(
            "{} {}: {} in {:.1}s",
            r.provider,
            r.model,
            if r.cache_hit {
                "cache hit"
            } else {
                "generated"
            },
            t0.elapsed().as_secs_f64()
        );
        return emit(&r.path, a.out.as_deref());
    }
    if a.model.is_empty() {
        bail!("--model is required for {}", a.provider);
    }
    let req = request(a)?;
    let policy = Policy::load_or_init(&config_dir().join(POLICY_FILE))?;
    let log = SpendLog::new(config_dir().join(SPEND_LOG_FILE));
    let cache = AssetCache::new(AssetCache::default_dir());
    let out = with_provider(&a.provider, |p| {
        generate(
            p,
            &req,
            &policy,
            &log,
            &cache,
            &a.video,
            PollOpts::default(),
            &mut |q, t| {
                info!(
                    "quote: {} {} = {} credits (video {}: {} / {}, month {} / {})",
                    q.provider,
                    q.model,
                    q.credits,
                    a.video,
                    t.video,
                    policy.per_video_cap,
                    t.month,
                    policy.monthly_cap
                );
            },
        )
    })?;
    let path = match out {
        Outcome::CacheHit(p) => {
            info!("cache hit: 0 credits");
            p
        }
        Outcome::Generated { path, quote } => {
            info!("generated: {} credits spent", quote.credits);
            path
        }
    };
    emit(&path, a.out.as_deref())
}

fn emit(path: &std::path::Path, out: Option<&std::path::Path>) -> Result<()> {
    if let Some(dst) = out {
        if let Some(d) = dst.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(path, dst)?;
        println!("{}", dst.display());
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_providers::HttpResponse;

    struct NoNet;
    impl motion_providers::Transport for NoNet {
        fn post(&self, _: &str, _: &[(&str, &str)], _: &[u8]) -> Result<HttpResponse> {
            bail!("no network in tests")
        }
        fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse> {
            bail!("no network in tests")
        }
    }

    #[test]
    fn refreshed_token_is_saved_on_drop() {
        let dir = std::env::temp_dir().join(format!("motion-persist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONNECTIONS_FILE);
        let conn = |tok: &str| Connection {
            adapter: "openart".into(),
            url: "https://example.invalid/mcp".into(),
            auth: Auth {
                access_token: tok.into(),
                ..Default::default()
            },
        };
        let mut all = Connections::default();
        all.providers.insert("openart".into(), conn("old"));
        all.save(&path).unwrap();
        let read = |p: &PathBuf| {
            Connections::load(p).unwrap().providers["openart"]
                .auth
                .access_token
                .clone()
        };
        let t = NoNet;
        // Unchanged auth: nothing is written.
        drop(Persisting {
            name: "openart".into(),
            path: path.clone(),
            inner: McpProvider::new(Adapter::openart(), conn("unsaved"), &t),
        });
        assert_eq!(read(&path), "old");
        // Refreshed auth: the new token reaches connections.json.
        let mut p = Persisting {
            name: "openart".into(),
            path: path.clone(),
            inner: McpProvider::new(Adapter::openart(), conn("old"), &t),
        };
        p.inner.conn.auth.access_token = "fresh".into();
        p.inner.auth_changed = true;
        drop(p);
        assert_eq!(read(&path), "fresh");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
