//! OAuth 2.1 for hosted MCP servers: protected-resource discovery, dynamic
//! client registration, PKCE (S256) with a loopback redirect, refresh.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::mcp::snippet;
use crate::store::Auth;
use crate::transport::Transport;

#[derive(Debug, Clone)]
pub struct ServerMeta {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub scopes: Vec<String>,
}

fn origin(url: &str) -> Result<(String, String)> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| anyhow!("only https MCP URLs are supported: {url}"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    Ok((
        format!("https://{host}"),
        format!("/{path}").trim_end_matches('/').to_string(),
    ))
}

fn get_json(t: &dyn Transport, url: &str) -> Result<Option<Value>> {
    let r = t.get(url, &[("Accept", "application/json")])?;
    if r.status != 200 {
        return Ok(None);
    }
    Ok(serde_json::from_slice(&r.body).ok())
}

/// Find the authorization server for an MCP resource URL (RFC 9728 → RFC 8414).
pub fn discover(t: &dyn Transport, resource: &str) -> Result<ServerMeta> {
    let (o, path) = origin(resource)?;
    let prm = [
        format!("{o}/.well-known/oauth-protected-resource{path}"),
        format!("{o}/.well-known/oauth-protected-resource"),
    ]
    .iter()
    .find_map(|u| get_json(t, u).ok().flatten());
    let issuer = prm
        .as_ref()
        .and_then(|p| {
            p.get("authorization_servers")?
                .as_array()?
                .first()?
                .as_str()
        })
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or(o.clone());
    let scopes: Vec<String> = prm
        .as_ref()
        .and_then(|p| p.get("scopes_supported")?.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect();
    let (io, ipath) = origin(&issuer)?;
    let meta = [
        format!("{io}/.well-known/oauth-authorization-server{ipath}"),
        format!("{io}/.well-known/openid-configuration{ipath}"),
    ]
    .iter()
    .find_map(|u| get_json(t, u).ok().flatten())
    .ok_or_else(|| anyhow!("no OAuth metadata at {issuer}"))?;
    let s = |k: &str| meta.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(ServerMeta {
        authorization_endpoint: s("authorization_endpoint").context("no authorization_endpoint")?,
        token_endpoint: s("token_endpoint").context("no token_endpoint")?,
        registration_endpoint: s("registration_endpoint"),
        scopes,
    })
}

pub fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn unpct(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", pct(k), pct(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn random_b64(n: usize) -> Result<String> {
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .context("reading /dev/urandom")?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn register(t: &dyn Transport, endpoint: &str, redirect: &str) -> Result<String> {
    let body = json!({
        "client_name": "MotionEngine",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    });
    let r = t.post(
        endpoint,
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ],
        &serde_json::to_vec(&body)?,
    )?;
    if !(200..300).contains(&r.status) {
        bail!(
            "client registration failed: HTTP {} {}",
            r.status,
            snippet(&r.text())
        );
    }
    let v: Value = serde_json::from_slice(&r.body)?;
    v.get("client_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("registration reply without client_id"))
}

fn token_request(t: &dyn Transport, endpoint: &str, pairs: &[(&str, &str)]) -> Result<Value> {
    let r = t.post(
        endpoint,
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Accept", "application/json"),
        ],
        form(pairs).as_bytes(),
    )?;
    if !(200..300).contains(&r.status) {
        bail!("token endpoint: HTTP {} {}", r.status, snippet(&r.text()));
    }
    Ok(serde_json::from_slice(&r.body)?)
}

fn auth_from(
    v: &Value,
    client_id: &str,
    token_endpoint: &str,
    old_refresh: Option<String>,
) -> Result<Auth> {
    let access = v
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("token reply without access_token"))?;
    Ok(Auth {
        access_token: access.to_string(),
        refresh_token: v
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(old_refresh),
        expires_at: v
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|s| now() + s),
        client_id: Some(client_id.to_string()),
        token_endpoint: Some(token_endpoint.to_string()),
    })
}

/// Refresh an expiring token. `None` when there is nothing to refresh with.
pub fn refresh(t: &dyn Transport, auth: &Auth, resource: &str) -> Result<Option<Auth>> {
    let (Some(rt), Some(cid), Some(te)) =
        (&auth.refresh_token, &auth.client_id, &auth.token_endpoint)
    else {
        return Ok(None);
    };
    let v = token_request(
        t,
        te,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", rt),
            ("client_id", cid),
            ("resource", resource),
        ],
    )?;
    auth_from(&v, cid, te, Some(rt.clone())).map(Some)
}

/// True when the token expires within a minute.
pub fn expiring(auth: &Auth) -> bool {
    auth.expires_at.is_some_and(|e| e <= now() + 60)
}

/// Browser sign-in: register a client, open the authorize URL, catch the code
/// on a loopback port, exchange it. `open` gets the URL (prints / opens a browser).
pub fn browser_login(t: &dyn Transport, resource: &str, open: &dyn Fn(&str)) -> Result<Auth> {
    let meta = discover(t, resource)?;
    let listener = TcpListener::bind("127.0.0.1:0").context("binding loopback port")?;
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr()?.port()
    );
    let client_id = register(
        t,
        meta.registration_endpoint
            .as_deref()
            .ok_or_else(|| anyhow!("server has no dynamic registration; use --token-stdin"))?,
        &redirect,
    )?;
    let verifier = random_b64(48)?;
    let state = random_b64(16)?;
    let scope = meta.scopes.join(" ");
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("state", state.as_str()),
        ("resource", resource),
        ("code_challenge_method", "S256"),
    ];
    let challenge = pkce_challenge(&verifier);
    params.push(("code_challenge", &challenge));
    if !scope.is_empty() {
        params.push(("scope", &scope));
    }
    let sep = if meta.authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    open(&format!(
        "{}{sep}{}",
        meta.authorization_endpoint,
        form(&params)
    ));

    listener.set_nonblocking(false)?;
    let deadline = SystemTime::now() + Duration::from_secs(600);
    let code = loop {
        if SystemTime::now() > deadline {
            bail!("timed out waiting for the browser sign-in");
        }
        let (mut stream, _) = listener.accept()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        let target = line.split_whitespace().nth(1).unwrap_or("");
        let query = target.split_once('?').map(|x| x.1).unwrap_or("");
        let q: Vec<(String, String)> = query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (unpct(k), unpct(v)))
            .collect();
        let get = |k: &str| q.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        if !target.starts_with("/callback") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let ok = get("state").as_deref() == Some(state.as_str()) && get("code").is_some();
        let msg = if ok {
            "MotionEngine is connected. You can close this tab."
        } else {
            "Sign-in failed. Return to the terminal."
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{msg}",
            msg.len()
        );
        if let Some(err) = get("error") {
            bail!("authorization denied: {err}");
        }
        if !ok {
            bail!("callback state mismatch");
        }
        break get("code").unwrap_or_default();
    };
    let v = token_request(
        t,
        &meta.token_endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", &client_id),
            ("code_verifier", &verifier),
            ("resource", resource),
        ],
    )?;
    auth_from(&v, &client_id, &meta.token_endpoint, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn percent_round_trip() {
        let s = "a b/c?d=é&";
        assert_eq!(unpct(&pct(s)), s);
    }
}
