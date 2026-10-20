//! (v2.3 P4) A local MOSS-Image-SFT server (stable-diffusion.cpp
//! `sd-server`, see docs/PROVIDERS.md): 0 credits, ~35 s per image.
//!
//! Requests go to the async job endpoint `/sdcpp/v1/img_gen` with the turbo
//! settings (4-step LoRA, euler, `txt_cfg` 1, the LoRA's own sigmas), the
//! same shape as the MOSS-Image reference client. The seed is always explicit. The server is
//! never started from here: when it is not reachable the error is
//! [`Unavailable`], after at most the 3 s connect timeout.
//!
//! Timeouts: a running job may take at most [`JOB_TIMEOUT`] (180 s); the
//! whole wait, including time queued behind other clients' jobs (the server
//! is shared), at most [`QUEUE_TIMEOUT`] (600 s).

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};

use crate::pipeline::PollOpts;
use crate::provider::{AssetProvider, AssetRequest, Job, JobState, ModelInfo, Quote};

pub const NAME: &str = "moss";
pub const MODEL: &str = "moss-image-sft";
pub const DEFAULT_URL: &str = "http://127.0.0.1:7860";
pub const IMG_GEN: &str = "/sdcpp/v1/img_gen";
/// 4-step turbo LoRA and the noise levels it was trained on.
pub const TURBO_LORA: &str = "moss_4step_f16.safetensors";
pub const TURBO_SIGMAS: [f64; 5] = [1.0, 0.91, 0.717, 0.024, 0.0];
pub const START_HINT: &str =
    "start a MOSS-Image server (stable-diffusion.cpp sd-server) or set `[moss] url` in providers.toml; see docs/PROVIDERS.md";

/// The provider cannot be reached (server down, network refused). The
/// fallback skips to the next provider on it.
#[derive(Debug, Clone)]
pub struct Unavailable(pub String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Unavailable {}

/// Plate size for an aspect: multiples of 16, long side ≤ 1024.
pub fn plate_size(aspect: Option<&str>) -> (u32, u32) {
    match aspect.unwrap_or("16:9").trim() {
        "16:9" => (768, 432),
        "9:16" => (432, 768),
        "1:1" => (576, 576),
        other => {
            let ratio = other
                .split_once(':')
                .and_then(|(a, b)| {
                    Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))
                })
                .filter(|(a, b)| *a > 0.0 && *b > 0.0);
            match ratio {
                Some((a, b)) if a >= b => (768, snap16(768.0 * b / a)),
                Some((a, b)) => (snap16(768.0 * a / b), 768),
                None => (768, 432),
            }
        }
    }
}

fn snap16(v: f64) -> u32 {
    ((v / 16.0).round() as u32).max(1) * 16
}

/// MOSS-Image layout prompt (the reference client's `layout_prompt`): boxes in
/// top-left px become the model's centre-origin yxyx coordinates. Formatted
/// like Python's `json.dumps` (", " / ": " separators).
fn layout_prompt(caption: &str, label: &str, item: &str, b: [i64; 4], w: u32, h: u32) -> String {
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let r = |v: f64| v.round() as i64;
    let bbox = [
        r(b[1] as f64 - cy),
        r(b[0] as f64 - cx),
        r(b[3] as f64 - cy),
        r(b[2] as f64 - cx),
    ];
    let s = |t: &str| serde_json::to_string(t).unwrap_or_default();
    format!(
        "{{\"summary\": {{\"caption\": {}}}, \"items\": [{{\"label\": {}, \"caption\": {}, \"bbox\": [{}, {}, {}, {}]}}], \"bbox_coord_space\": \"abs_centralized\", \"bbox_format\": \"yxyx\"}}",
        s(caption),
        s(label),
        s(item),
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[3]
    )
}

/// Single isolated object on flat magenta (#FF00FF) for keying
/// (the reference client's `cutout_prompt`, layout box with a 10 % margin, no shadows).
pub fn cutout_prompt(desc: &str, w: u32, h: u32) -> String {
    let bg = "pure flat solid #FF00FF chroma key magenta background";
    let margin = 0.10;
    let x0 = (f64::from(w) * margin).round().max(16.0) as i64;
    let y0 = (f64::from(h) * margin).round().max(16.0) as i64;
    let x1 = ((f64::from(w) * (1.0 - margin)).round() as i64).min(i64::from(w) - 16);
    let y1 = ((f64::from(h) * (1.0 - margin)).round() as i64).min(i64::from(h) - 16);
    let scene = format!(
        "Studio product isolation shot on a {bg}. Completely uniform flat solid color backdrop filling the entire frame from edge to edge and visible through any open rings or holes, zero cast shadows, no floor, no horizon line, no gradient, no vignette, no text."
    );
    let item = format!(
        "Single complete isolated object centered inside box: {}, floating against a {bg}, zero cast shadow, crisp clean silhouette edges.",
        desc.trim()
    );
    layout_prompt(&scene, "object", &item, [x0, y0, x1, y1], w, h)
}

/// The `/sdcpp/v1/img_gen` body (turbo settings, explicit seed).
pub fn request_body(prompt: &str, w: u32, h: u32, seed: u32) -> Value {
    json!({
        "prompt": prompt,
        "negative_prompt": "",
        "width": w,
        "height": h,
        "seed": seed,
        "strength": 0.75,
        "batch_count": 1,
        "sample_params": {
            "sample_method": "euler",
            "sample_steps": TURBO_SIGMAS.len() - 1,
            "custom_sigmas": TURBO_SIGMAS,
            "guidance": {"txt_cfg": 1.0},
        },
        "lora": [{"path": TURBO_LORA, "multiplier": 1.0, "is_high_noise": false}],
        "output_format": "png",
    })
}

/// `[moss] url = "..."` from `providers.toml` (a tiny reader: one key).
pub fn url_from_toml(text: &str) -> Option<String> {
    let mut in_moss = false;
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.starts_with('[') {
            in_moss = l == "[moss]";
            continue;
        }
        if !in_moss {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            if k.trim() == "url" {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
                if !v.is_empty() {
                    return Some(v.trim_end_matches('/').to_string());
                }
            }
        }
    }
    None
}

/// Base URL: `providers.toml` `[moss] url`, else [`DEFAULT_URL`].
pub fn configured_url(config_dir: &Path) -> String {
    std::fs::read_to_string(config_dir.join("providers.toml"))
        .ok()
        .and_then(|t| url_from_toml(&t))
        .unwrap_or_else(|| DEFAULT_URL.to_string())
}

pub struct MossProvider {
    pub base: String,
    agent: ureq::Agent,
    /// Decoded images of finished jobs, served by `download("moss://<job>.png")`.
    results: HashMap<String, Vec<u8>>,
    /// When each job was first seen running (time queued behind other
    /// clients' jobs does not count toward [`JOB_TIMEOUT`]).
    started: HashMap<String, Instant>,
    pub job_timeout: Duration,
}

/// A running job may take at most this long.
pub const JOB_TIMEOUT: Duration = Duration::from_secs(180);
/// Overall bound, queue included (the server is shared): never hang.
pub const QUEUE_TIMEOUT: Duration = Duration::from_secs(600);

impl MossProvider {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(3))
                .timeout(Duration::from_secs(60))
                .build(),
            results: HashMap::new(),
            started: HashMap::new(),
            job_timeout: JOB_TIMEOUT,
        }
    }

    /// Poll every second; a running job fails after [`JOB_TIMEOUT`], the
    /// whole wait (queue included) after [`QUEUE_TIMEOUT`].
    pub fn poll_opts() -> PollOpts {
        PollOpts {
            interval: Duration::from_millis(1000),
            timeout: QUEUE_TIMEOUT,
        }
    }

    fn unavailable(&self, why: impl std::fmt::Display) -> anyhow::Error {
        anyhow!(Unavailable(format!(
            "moss server at {} is not reachable ({why}); {START_HINT}",
            self.base
        )))
    }

    fn finish(&self, r: std::result::Result<ureq::Response, ureq::Error>) -> Result<Value> {
        let resp = match r {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                bail!(
                    "moss: HTTP {code}: {}",
                    body.chars().take(300).collect::<String>()
                )
            }
            Err(ureq::Error::Transport(t)) => return Err(self.unavailable(t)),
        };
        let mut body = Vec::new();
        resp.into_reader()
            .take(crate::transport::MAX_BODY)
            .read_to_end(&mut body)
            .context("moss: reading response")?;
        serde_json::from_slice(&body).context("moss: response is not JSON")
    }

    /// Any HTTP answer within the connect timeout counts as reachable.
    pub fn reachable(&self) -> bool {
        match self.agent.get(&format!("{}/", self.base)).call() {
            Ok(_) | Err(ureq::Error::Status(..)) => true,
            Err(ureq::Error::Transport(_)) => false,
        }
    }

    fn url_for(&self, path: &str) -> Result<String> {
        if path.starts_with('/') {
            Ok(format!("{}{path}", self.base))
        } else if path.starts_with(&self.base) {
            Ok(path.to_string())
        } else {
            bail!("moss: refusing poll url outside the server: {path}")
        }
    }
}

impl AssetProvider for MossProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn catalog(&mut self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: MODEL.into(),
            name: "MOSS-Image-SFT (local, 4-step turbo)".into(),
            kind: "image".into(),
        }])
    }

    fn balance(&mut self) -> Result<Option<f64>> {
        Ok(None)
    }

    fn quote(&mut self, req: &AssetRequest) -> Result<Quote> {
        Ok(Quote {
            provider: NAME.into(),
            model: req.model.clone(),
            credits: 0.0,
        })
    }

    /// `params.seed` (required), `params.cutout` (magenta-ground wrapper).
    fn submit(&mut self, req: &AssetRequest) -> Result<Job> {
        if crate::jsonx::norm(&req.model) != crate::jsonx::norm(MODEL) {
            bail!("moss serves only {MODEL}, not {}", req.model);
        }
        let seed = req
            .params
            .get("seed")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("moss: an explicit seed is required (params.seed)"))?;
        let seed = u32::try_from(seed).context("moss: seed must fit u32")?;
        let cutout = req
            .params
            .get("cutout")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (w, h) = plate_size(req.aspect.as_deref());
        let prompt = if cutout {
            cutout_prompt(&req.prompt, w, h)
        } else {
            req.prompt.trim().to_string()
        };
        let body = request_body(&prompt, w, h, seed);
        let v = self.finish(
            self.agent
                .post(&format!("{}{IMG_GEN}", self.base))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string()),
        )?;
        let poll = v
            .get("poll_url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                v.get("id")
                    .or_else(|| v.get("job_id"))
                    .and_then(Value::as_str)
                    .map(|id| format!("{IMG_GEN}/{id}"))
            })
            .ok_or_else(|| anyhow!("moss: submit reply has no poll_url: {v}"))?;
        Ok(Job {
            id: poll,
            urls: Vec::new(),
        })
    }

    fn poll(&mut self, job: &Job) -> Result<JobState> {
        let url = self.url_for(&job.id)?;
        let st = self.finish(self.agent.get(&url).call())?;
        let status = st
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(status.as_str(), "failed" | "cancelled" | "canceled") {
            return Ok(JobState::Failed(format!(
                "{status}: {}",
                st.get("error").map(|e| e.to_string()).unwrap_or_default()
            )));
        }
        if !matches!(status.as_str(), "completed" | "succeeded" | "done") {
            if status != "queued" && status != "pending" {
                let t0 = *self
                    .started
                    .entry(job.id.clone())
                    .or_insert_with(Instant::now);
                if t0.elapsed() > self.job_timeout {
                    return Ok(JobState::Failed(format!(
                        "running longer than {:?}",
                        self.job_timeout
                    )));
                }
            }
            return Ok(JobState::Pending);
        }
        let imgs = st
            .pointer("/result/images")
            .or_else(|| st.get("images"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let first = imgs
            .first()
            .ok_or_else(|| anyhow!("moss: finished job has no image"))?;
        let data = match first {
            Value::String(s) => s.as_str(),
            o => o
                .get("b64_json")
                .or_else(|| o.get("data"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("moss: image has no data"))?,
        };
        let data = match data.strip_prefix("data:") {
            Some(rest) => rest.split_once(',').map(|(_, d)| d).unwrap_or(rest),
            None => data,
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .context("moss: image is not base64")?;
        if !bytes.starts_with(b"\x89PNG") {
            bail!("moss: image is not a PNG");
        }
        let key = format!("moss://{}.png", self.results.len());
        self.results.insert(key.clone(), bytes);
        Ok(JobState::Done(vec![key]))
    }

    fn download(&mut self, url: &str) -> Result<Vec<u8>> {
        self.results
            .remove(url)
            .ok_or_else(|| anyhow!("moss: no result {url}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(plate_size(Some("16:9")), (768, 432));
        assert_eq!(plate_size(Some("9:16")), (432, 768));
        assert_eq!(plate_size(Some("1:1")), (576, 576));
        assert_eq!(plate_size(None), (768, 432));
        let (w, h) = plate_size(Some("4:5"));
        assert!(w % 16 == 0 && h % 16 == 0 && h == 768 && w < h);
    }

    #[test]
    fn toml_url() {
        let t = "[openrouter]\nurl = \"x\"\n\n[moss]\nurl = \"http://10.0.0.2:7860/\" # lan\n";
        assert_eq!(url_from_toml(t).as_deref(), Some("http://10.0.0.2:7860"));
        assert_eq!(url_from_toml("[openrouter]\nurl = \"x\"\n"), None);
    }

    #[test]
    fn cutout_prompt_matches_client() {
        let p = cutout_prompt("a brass compass", 576, 576);
        let v: Value = serde_json::from_str(&p).unwrap();
        assert_eq!(v["bbox_format"], "yxyx");
        // 10 % margin: x0 = y0 = 58, x1 = y1 = 518 around centre 288.
        assert_eq!(v["items"][0]["bbox"], json!([-230, -230, 230, 230]));
        assert!(p.contains("#FF00FF") && p.contains("zero cast shadow"));
        assert!(p.starts_with("{\"summary\": {\"caption\": "));
    }
}
