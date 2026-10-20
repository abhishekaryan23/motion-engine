//! HTTP seam: the real `ureq` transport and a trait tests replace.

use std::io::Read;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Largest body read (downloads included).
pub const MAX_BODY: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub trait Transport {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<HttpResponse>;
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse>;
}

/// Real transport over `ureq` (rustls). Non-2xx statuses are returned, not errors.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(20))
                .timeout(Duration::from_secs(300))
                .build(),
        }
    }
}

fn finish(r: std::result::Result<ureq::Response, ureq::Error>) -> Result<HttpResponse> {
    let resp = match r {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(t)) => bail!("network error: {t}"),
    };
    let status = resp.status();
    let headers = resp
        .headers_names()
        .into_iter()
        .filter_map(|n| resp.header(&n).map(|v| (n.clone(), v.to_string())))
        .collect();
    let mut body = Vec::new();
    resp.into_reader()
        .take(MAX_BODY)
        .read_to_end(&mut body)
        .context("reading response body")?;
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

impl Transport for UreqTransport {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<HttpResponse> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        finish(req.send_bytes(body))
    }
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        finish(req.call())
    }
}
