//! Provider flow against an in-process mock MCP server (OpenArt-shaped tools).

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use motion_providers::policy::SpendTotals;
use motion_providers::{
    generate, Adapter, AssetCache, AssetKind, AssetProvider, AssetRequest, Auth, Connection,
    HttpResponse, McpProvider, Outcome, Policy, PollOpts, SpendLog, Transport,
};
use serde_json::{json, Value};

#[derive(Default)]
struct Mock {
    calls: RefCell<Vec<String>>,
    polls_until_done: RefCell<u32>,
    sse: bool,
    quote: f64,
    /// Reject this token with 401.
    stale_token: Option<String>,
}

fn reply(id: &Value, result: Value, sse: bool) -> HttpResponse {
    let msg = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
    let (ct, body) = if sse {
        (
            "text/event-stream",
            format!("event: message\ndata: {msg}\n\n"),
        )
    } else {
        ("application/json", msg)
    };
    HttpResponse {
        status: 200,
        headers: vec![
            ("Content-Type".into(), ct.into()),
            ("Mcp-Session-Id".into(), "s1".into()),
        ],
        body: body.into_bytes(),
    }
}

fn text(v: Value) -> Value {
    json!({"content": [{"type": "text", "text": v.to_string()}]})
}

fn schema(props: &[&str]) -> Value {
    let p: serde_json::Map<String, Value> = props
        .iter()
        .map(|k| (k.to_string(), json!({"type": "string"})))
        .collect();
    json!({"type": "object", "properties": p})
}

impl Transport for Mock {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<HttpResponse> {
        if url.ends_with("/token") {
            self.calls.borrow_mut().push("refresh".into());
            let b = String::from_utf8_lossy(body);
            assert!(b.contains("grant_type=refresh_token"));
            return Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: br#"{"access_token":"fresh","expires_in":3600}"#.to_vec(),
            });
        }
        let auth = headers
            .iter()
            .find(|(k, _)| *k == "Authorization")
            .map(|x| x.1);
        if let (Some(stale), Some(a)) = (&self.stale_token, auth) {
            if a == format!("Bearer {stale}") {
                return Ok(HttpResponse {
                    status: 401,
                    ..Default::default()
                });
            }
        }
        let m: Value = serde_json::from_slice(body)?;
        let method = m["method"].as_str().unwrap_or("");
        if m.get("id").is_none() {
            return Ok(HttpResponse {
                status: 202,
                ..Default::default()
            });
        }
        let id = &m["id"];
        let res = match method {
            "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}}),
            "tools/list" => json!({"tools": [
                {"name": "openart_account_get", "inputSchema": schema(&[])},
                {"name": "openart_model_list", "inputSchema": schema(&["type"])},
                {"name": "openart_model_cost", "inputSchema": schema(&["mode", "model", "params"])},
                {"name": "openart_generate_image", "inputSchema": schema(&["mode", "model", "params", "projectId"])},
                {"name": "openart_creation_get", "inputSchema": schema(&["historyId"])},
            ]}),
            "tools/call" => {
                let name = m["params"]["name"].as_str().unwrap_or("").to_string();
                let args = &m["params"]["arguments"];
                self.calls.borrow_mut().push(name.clone());
                match name.as_str() {
                    "openart_account_get" => text(json!({"user": {"credit_balance": 812}})),
                    "openart_model_list" => text(json!({"models": [
                        {"id": "gpt-image-2.5-flare", "name": "GPT Image 2.5 Flare", "type": "image"}]})),
                    "openart_model_cost" => {
                        assert_eq!(args["model"], "gpt-image-2.5-flare");
                        assert_eq!(args["mode"], "text2image");
                        assert_eq!(args["params"]["quality"], "low");
                        text(json!({"estimate": {"credits": self.quote}}))
                    }
                    "openart_generate_image" => {
                        assert_eq!(args["params"]["prompt"], "a paper boat on ink");
                        assert_eq!(args["params"]["aspectRatio"], "1:1");
                        text(json!({"historyId": "c-42", "status": "queued"}))
                    }
                    "openart_creation_get" => {
                        assert_eq!(args["historyId"], "c-42");
                        let mut left = self.polls_until_done.borrow_mut();
                        if *left > 0 {
                            *left -= 1;
                            text(json!({"status": "processing"}))
                        } else {
                            text(json!({"status": "completed",
                                "images": [{"url": "https://cdn.example/x/flare_1.png"}]}))
                        }
                    }
                    _ => json!({"isError": true, "content": [{"type": "text", "text": "no"}]}),
                }
            }
            _ => {
                return Ok(HttpResponse {
                    status: 404,
                    ..Default::default()
                })
            }
        };
        Ok(reply(id, res, self.sse))
    }

    fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResponse> {
        self.calls.borrow_mut().push(format!("GET {url}"));
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: b"\x89PNG fake".to_vec(),
        })
    }
}

fn conn(token: &str) -> Connection {
    Connection {
        adapter: "openart".into(),
        url: "https://mcp.example/mcp".into(),
        auth: Auth {
            access_token: token.into(),
            refresh_token: Some("r".into()),
            client_id: Some("cid".into()),
            token_endpoint: Some("https://auth.example/token".into()),
            ..Default::default()
        },
    }
}

fn req() -> AssetRequest {
    AssetRequest {
        kind: AssetKind::Image,
        model: "gpt-image-2.5-flare".into(),
        prompt: "a paper boat on ink".into(),
        quality: Some("low".into()),
        aspect: Some("1:1".into()),
        params: Default::default(),
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("motion-providers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fast() -> PollOpts {
    PollOpts {
        interval: Duration::ZERO,
        timeout: Duration::from_secs(5),
    }
}

#[test]
fn discovers_tools_catalog_and_balance_over_sse() {
    let mock = Mock {
        sse: true,
        ..Default::default()
    };
    let mut p = McpProvider::new(Adapter::openart(), conn("t"), &mock);
    assert_eq!(p.tools().unwrap().len(), 5);
    assert_eq!(p.balance().unwrap(), Some(812.0));
    let cat = p.catalog().unwrap();
    assert_eq!(cat[0].id, "gpt-image-2.5-flare");
    assert_eq!(cat[0].kind, "image");
}

#[test]
fn generate_then_cache_hit() {
    let d = tmp("gen");
    let mock = Mock {
        quote: 2.0,
        ..Default::default()
    };
    *mock.polls_until_done.borrow_mut() = 2;
    let policy = Policy::default();
    let log = SpendLog::new(d.join("spend.jsonl"));
    let cache = AssetCache::new(d.join("cache"));
    let mut seen = None;
    let mut p = McpProvider::new(Adapter::openart(), conn("t"), &mock);
    let out = generate(
        &mut p,
        &req(),
        &policy,
        &log,
        &cache,
        "demo",
        fast(),
        &mut |q, t| seen = Some((q.credits, *t)),
    )
    .unwrap();
    assert_eq!(seen, Some((2.0, SpendTotals::default())));
    let Outcome::Generated { path, quote } = out else {
        panic!("expected generation")
    };
    assert_eq!(quote.credits, 2.0);
    assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG fake");
    assert!(path.to_string_lossy().ends_with("asset.png"));
    let log_entries = log.entries().unwrap();
    assert_eq!(log_entries.len(), 1);
    assert_eq!(log_entries[0].job, "c-42");

    let n = mock.calls.borrow().len();
    let again = generate(
        &mut p,
        &req(),
        &policy,
        &log,
        &cache,
        "demo",
        fast(),
        &mut |_, _| panic!("no quote on a cache hit"),
    )
    .unwrap();
    assert_eq!(again, Outcome::CacheHit(path));
    assert_eq!(
        mock.calls.borrow().len(),
        n,
        "cache hit must not call the server"
    );
    assert_eq!(log.entries().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn quote_above_cap_stops_before_submit() {
    let d = tmp("cap");
    let mock = Mock {
        quote: 41.0,
        ..Default::default()
    };
    let mut p = McpProvider::new(Adapter::openart(), conn("t"), &mock);
    let log = SpendLog::new(d.join("spend.jsonl"));
    let err = generate(
        &mut p,
        &req(),
        &Policy::default(),
        &log,
        &AssetCache::new(d.join("c")),
        "v",
        fast(),
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("max_per_request"), "{err}");
    assert!(!mock.calls.borrow().iter().any(|c| c.contains("generate")));
    assert!(log.entries().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn disallowed_model_or_quality_never_reaches_the_server() {
    let d = tmp("deny");
    let mock = Mock {
        quote: 1.0,
        ..Default::default()
    };
    let mut p = McpProvider::new(Adapter::openart(), conn("t"), &mock);
    let log = SpendLog::new(d.join("s"));
    let cache = AssetCache::new(d.join("c"));
    for r in [
        AssetRequest {
            model: "flux-pro".into(),
            ..req()
        },
        AssetRequest {
            quality: Some("high".into()),
            ..req()
        },
    ] {
        assert!(generate(
            &mut p,
            &r,
            &Policy::default(),
            &log,
            &cache,
            "v",
            fast(),
            &mut |_, _| {}
        )
        .is_err());
    }
    assert!(mock.calls.borrow().is_empty());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn stale_token_is_refreshed_once() {
    let mock = Mock {
        stale_token: Some("old".into()),
        ..Default::default()
    };
    let mut p = McpProvider::new(Adapter::openart(), conn("old"), &mock);
    assert_eq!(p.balance().unwrap(), Some(812.0));
    assert!(p.auth_changed);
    assert_eq!(p.conn.auth.access_token, "fresh");
    assert_eq!(p.conn.auth.refresh_token.as_deref(), Some("r"));
}

#[test]
fn monthly_cap_counts_logged_spend() {
    let d = tmp("month");
    let log = SpendLog::new(d.join("s"));
    let month = motion_providers::policy::month_of(motion_providers::policy::now_secs());
    log.append(&motion_providers::SpendEntry {
        ts: 0,
        month: month.clone(),
        video: "other".into(),
        provider: "openart".into(),
        model: "m".into(),
        credits: 1499.0,
        cache_key: "k".into(),
        job: String::new(),
    })
    .unwrap();
    let t = log.totals("v", &month).unwrap();
    assert_eq!(t.video, 0.0);
    assert!(Policy::default()
        .check_spend(2.0, &t)
        .unwrap_err()
        .to_string()
        .contains("monthly_cap"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn cache_key_is_stable_and_model_normalized() {
    let a = AssetCache::key("openart", &req());
    let b = AssetCache::key(
        "openart",
        &AssetRequest {
            model: "GPT Image 2.5 Flare".into(),
            ..req()
        },
    );
    assert_eq!(a, b);
    assert_ne!(
        a,
        AssetCache::key(
            "openart",
            &AssetRequest {
                prompt: "other".into(),
                ..req()
            }
        )
    );
}
