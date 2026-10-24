//! `motion-engine serve`: lightweight local HTTP REST server for MotionEngine v2.
//!
//! Designed so local consumer apps and agents can generate
//! 9:16 vertical Shorts and 16:9 / 1:1 videos over HTTP without spawning CLI
//! subprocesses directly.
//!
//! Endpoints:
//! - `GET  /health` or `GET /v2/info` -> engine capabilities, recipes, fonts, presets
//! - `GET  /v2/schema`               -> MotionProgram v2.0 JSON Schema
//! - `POST /v2/shorts`               -> generate a 9:16 vertical Short (defaults aspect="9:16")
//! - `POST /v2/make`                 -> generate a video from `brief`, `intent`, or `program`
//! - `POST /v2/program/render`       -> validate, auto-repair, and render a MotionProgram v2 JSON
//! - `GET  /files/<repo-rel-path>`   -> stream/download generated `.mp4`, `.png`, `.json` under `output/`

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use serde_json::{json, Value};

pub const RECIPES: [&str; 6] = [
    "showreel",
    "kinetic_poster",
    "chart_reveal",
    "ledger_ink",
    "dot_story",
    "top5_countdown",
];

#[derive(Args, Debug, Clone)]
pub struct ServeArgs {
    /// Bind host address (default: 127.0.0.1).
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Bind TCP port (default: 8787).
    #[arg(long, short, default_value_t = 8787)]
    pub port: u16,
    /// Asset library root (default: assets).
    #[arg(long, default_value = "assets")]
    pub assets: PathBuf,
}

struct ServerCtx {
    host: String,
    port: u16,
    repo: PathBuf,
    assets: PathBuf,
    exe: PathBuf,
}

impl ServerCtx {
    fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.repo)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn abs(&self, p: &str) -> PathBuf {
        let path = Path::new(p);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.repo.join(path)
        }
    }
}

pub fn run(args: ServeArgs) -> Result<()> {
    let repo = std::env::current_dir().context("current_dir")?;
    let repo = repo.canonicalize().unwrap_or(repo);
    let assets = if args.assets.is_absolute() {
        args.assets.clone()
    } else {
        repo.join(&args.assets)
    };
    let exe = std::env::current_exe().context("current_exe")?;
    let addr = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&addr).with_context(|| format!("binding {addr}"))?;
    let ctx = Arc::new(ServerCtx {
        host: args.host,
        port: args.port,
        repo,
        assets,
        exe,
    });
    eprintln!(
        "motion-engine v2 server listening on {} (repo: {})",
        ctx.base_url(),
        ctx.repo.display()
    );
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let ctx = Arc::clone(&ctx);
                std::thread::spawn(move || {
                    let _ = handle_conn(&ctx, s);
                });
            }
            Err(e) => eprintln!("serve accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_conn(ctx: &ServerCtx, mut stream: TcpStream) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut req_line = String::new();
    if reader.read_line(&mut req_line)? == 0 {
        return Ok(());
    }
    let parts: Vec<&str> = req_line.split_whitespace().collect();
    if parts.len() < 2 {
        return write_json(&mut stream, 400, &json!({"error": "bad request line"}));
    }
    let (method, raw_path) = (parts[0], parts[1]);
    let path = raw_path.split('?').next().unwrap_or(raw_path);

    let mut content_len: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse().unwrap_or(0);
            }
        }
    }

    if method.eq_ignore_ascii_case("OPTIONS") {
        return write_empty_cors(&mut stream, 204);
    }

    let mut body = vec![0u8; content_len.min(16 * 1024 * 1024)];
    if content_len > 0 {
        reader.read_exact(&mut body)?;
    }

    match (method, path) {
        ("GET", "/" | "/health" | "/v2/info") => {
            let info = json!({
                "status": "ok",
                "engine": "motion-engine",
                "version": "2.0",
                "default_shorts_aspect": "9:16",
                "aspects": ["9:16", "16:9", "1:1"],
                "recipes": RECIPES,
                "lockup_presets": [
                    "spline_frame",
                    "monolith_split",
                    "orbital_seal",
                    "terminal_prompt",
                    "editorial_badge"
                ],
                "bento_presets": ["grid_2x2", "hero_left", "pillars"],
                "fonts": [
                    "archivo_black",
                    "poppins_black",
                    "anton",
                    "cabinet_grotesk_extrabold",
                    "clash_display_bold",
                    "panchang_extrabold",
                    "bebas_neue",
                    "barlow_condensed_bold",
                    "satoshi_bold",
                    "instrument_serif_italic",
                    "instrument_serif_regular",
                    "dm_serif_italic",
                    "dm_serif_regular",
                    "zodiak_italic",
                    "zodiak_bold",
                    "zilla_slab_bold",
                    "ibm_plex_mono",
                    "space_mono",
                    "space_mono_bold"
                ],
                "music_tracks": [
                    "driving_pulse",
                    "tech_pulse",
                    "cinematic_strings",
                    "bright_upbeat",
                    "retro_groove",
                    "warm_piano",
                    "calm_ambient"
                ],
                "endpoints": {
                    "health": "GET /health",
                    "guide": "GET /v2/guide",
                    "schema": "GET /v2/schema",
                    "shorts": "POST /v2/shorts",
                    "make": "POST /v2/make",
                    "program_render": "POST /v2/program/render",
                    "files": "GET /files/<path>"
                }
            });
            write_json(&mut stream, 200, &info)
        }
        ("GET", "/v2/guide") => {
            let guide_path = ctx.repo.join("docs/mcp/v2_guide.md");
            match std::fs::read(&guide_path) {
                Ok(bytes) => write_bytes(&mut stream, 200, "text/markdown; charset=utf-8", &bytes),
                Err(e) => write_json(&mut stream, 500, &json!({"error": e.to_string()})),
            }
        }
        ("GET", "/v2/schema") => {
            let schema_path = ctx.repo.join("schema/motion_program.json");
            match std::fs::read(&schema_path) {
                Ok(bytes) => write_bytes(&mut stream, 200, "application/json", &bytes),
                Err(e) => write_json(&mut stream, 500, &json!({"error": e.to_string()})),
            }
        }
        ("GET", p) if p.starts_with("/files/") => {
            let rel_path = p.trim_start_matches("/files/");
            serve_file(ctx, &mut stream, rel_path)
        }
        ("POST", "/v2/shorts") => {
            let payload: Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &json!({"error": format!("invalid JSON: {e}")}),
                    )
                }
            };
            match handle_make(ctx, &payload, Some("9:16")) {
                Ok(res) => write_json(&mut stream, 200, &res),
                Err(e) => write_json(&mut stream, 422, &json!({"error": e})),
            }
        }
        ("POST", "/v2/make") => {
            let payload: Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &json!({"error": format!("invalid JSON: {e}")}),
                    )
                }
            };
            match handle_make(ctx, &payload, None) {
                Ok(res) => write_json(&mut stream, 200, &res),
                Err(e) => write_json(&mut stream, 422, &json!({"error": e})),
            }
        }
        ("POST", "/v2/program/render") => {
            let payload: Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &json!({"error": format!("invalid JSON: {e}")}),
                    )
                }
            };
            let (prog, wrapper) = if payload.get("program").is_some_and(Value::is_object) {
                (&payload["program"], &payload)
            } else {
                (&payload, &payload)
            };
            match render_program_v2(ctx, prog, wrapper, None) {
                Ok(res) => write_json(&mut stream, 200, &res),
                Err(e) => write_json(&mut stream, 422, &json!({"error": e})),
            }
        }
        _ => write_json(
            &mut stream,
            404,
            &json!({"error": format!("not found: {method} {path}")}),
        ),
    }
}

fn str_field<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn str_or_json_field(v: &Value, k: &str) -> Option<String> {
    match v.get(k) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(o @ Value::Object(_)) => Some(o.to_string()),
        _ => None,
    }
}

fn sanitize_stem(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('_');
    if s.is_empty() {
        "short".to_string()
    } else {
        s.chars().take(48).collect()
    }
}

fn enrich_urls(ctx: &ServerCtx, mut v: Value) -> Value {
    let base = ctx.base_url();
    if let Some(obj) = v.as_object_mut() {
        if let Some(video) = obj.get("video").and_then(Value::as_str).map(str::to_string) {
            let abs = ctx.abs(&video);
            let rel = ctx.rel(&abs);
            obj.insert("video".into(), json!(rel));
            obj.insert("video_abs".into(), json!(abs.display().to_string()));
            obj.insert("video_url".into(), json!(format!("{base}/files/{rel}")));
        }
        if let Some(sheet) = obj.get("sheet").and_then(Value::as_str).map(str::to_string) {
            let abs = ctx.abs(&sheet);
            let rel = ctx.rel(&abs);
            obj.insert("sheet".into(), json!(rel));
            obj.insert("sheet_abs".into(), json!(abs.display().to_string()));
            obj.insert("sheet_url".into(), json!(format!("{base}/files/{rel}")));
        }
        if let Some(prog) = obj
            .get("program")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            let abs = ctx.abs(&prog);
            let rel = ctx.rel(&abs);
            obj.insert("program".into(), json!(rel));
            obj.insert("program_abs".into(), json!(abs.display().to_string()));
            obj.insert("program_url".into(), json!(format!("{base}/files/{rel}")));
        }
    }
    v
}

fn run_engine(ctx: &ServerCtx, args: &[String]) -> Result<std::process::Output, String> {
    let out = Command::new(&ctx.exe)
        .current_dir(&ctx.repo)
        .arg("-q")
        .args(args)
        .output()
        .map_err(|e| format!("engine spawn: {e}"))?;
    if out.status.success() {
        Ok(out)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let msg = if !stderr.trim().is_empty() {
            stderr
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" | ")
        } else {
            stdout
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" | ")
        };
        Err(msg)
    }
}

fn merge_program_json(target: &mut Value, patch: &Value) {
    let (Some(t_obj), Some(p_obj)) = (target.as_object_mut(), patch.as_object()) else {
        *target = patch.clone();
        return;
    };
    for (k, v) in p_obj {
        if v.is_null() {
            t_obj.remove(k);
        } else if k == "components" && v.is_array() {
            let patch_comps = v.as_array().unwrap();
            let all_have_ids = patch_comps
                .iter()
                .all(|c| c.get("id").and_then(Value::as_str).is_some());
            if all_have_ids && t_obj.get("components").is_some_and(Value::is_array) {
                let target_comps = t_obj.get_mut("components").unwrap().as_array_mut().unwrap();
                for pc in patch_comps {
                    let id = pc.get("id").and_then(Value::as_str).unwrap_or("");
                    if pc.get("delete") == Some(&json!(true)) {
                        target_comps.retain(|tc| tc.get("id").and_then(Value::as_str) != Some(id));
                    } else if let Some(existing) = target_comps
                        .iter_mut()
                        .find(|tc| tc.get("id").and_then(Value::as_str) == Some(id))
                    {
                        merge_program_json(existing, pc);
                    } else {
                        target_comps.push(pc.clone());
                    }
                }
            } else {
                t_obj.insert(k.clone(), v.clone());
            }
        } else if v.is_object() {
            let entry = t_obj.entry(k.clone()).or_insert_with(|| json!({}));
            merge_program_json(entry, v);
        } else {
            t_obj.insert(k.clone(), v.clone());
        }
    }
}

fn apply_wrapper_edits(prog: &mut Value, wrapper: &Value) -> Result<(), String> {
    if let Some(patch) = wrapper.get("patch").filter(|v| v.is_object()) {
        merge_program_json(prog, patch);
    }
    if let Some(raw_pal) = str_or_json_field(wrapper, "palette") {
        let pal_val = if raw_pal.starts_with('{') {
            serde_json::from_str::<Value>(&raw_pal).map_err(|e| format!("palette: {e}"))?
        } else {
            let parts: Vec<&str> = raw_pal
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if parts.len() != 4 {
                return Err("palette: expected '#BG,#INK,#ACCENT,#MUTED' or JSON object".into());
            }
            json!({
                "background": parts[0],
                "ink": parts[1],
                "accent": parts[2],
                "muted": parts[3]
            })
        };
        if let Some(obj) = prog.as_object_mut() {
            obj.insert("palette".into(), pal_val);
        }
    }
    if let Some(raw_fonts) = str_or_json_field(wrapper, "fonts") {
        let fonts_val = if raw_fonts.starts_with('{') {
            serde_json::from_str::<Value>(&raw_fonts).map_err(|e| format!("fonts: {e}"))?
        } else {
            let parts: Vec<&str> = raw_fonts
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let mut m = serde_json::Map::new();
            if let Some(d) = parts.first() {
                m.insert("display".into(), json!(d));
            }
            if let Some(s) = parts.get(1) {
                m.insert("serif".into(), json!(s));
            }
            if let Some(mo) = parts.get(2) {
                m.insert("mono".into(), json!(mo));
            }
            Value::Object(m)
        };
        if let Some(obj) = prog.as_object_mut() {
            obj.insert("fonts".into(), fonts_val);
        }
    }
    if let Some(ending) = str_field(wrapper, "ending") {
        if let Some(comps) = prog.get_mut("components").and_then(Value::as_array_mut) {
            for c in comps {
                if c.get("kind").and_then(Value::as_str) == Some("lockup") {
                    if let Some(co) = c.as_object_mut() {
                        co.insert("preset".into(), json!(ending));
                        if ending == "editorial_badge" && !co.contains_key("badge") {
                            co.insert("badge".into(), json!("EDITION // 2026"));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn render_program_v2(
    ctx: &ServerCtx,
    prog: &Value,
    wrapper: &Value,
    default_aspect: Option<&str>,
) -> Result<Value, String> {
    let mut patched = prog.clone();
    apply_wrapper_edits(&mut patched, wrapper)?;
    let raw_name = patched
        .pointer("/meta/name")
        .and_then(Value::as_str)
        .or_else(|| str_field(wrapper, "name"))
        .unwrap_or("short");
    let stem = sanitize_stem(raw_name);
    let dir = ctx.repo.join("output/make").join(&stem);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let aspect = str_field(wrapper, "aspect").or(default_aspect);
    if let Some(meta) = patched.get_mut("meta").and_then(Value::as_object_mut) {
        meta.insert("name".into(), json!(stem));
        if meta.get("aspect").is_none() {
            if let Some(a) = aspect {
                meta.insert("aspect".into(), json!(a));
            }
        }
    }
    let program_path = dir.join(format!("{stem}.program.json"));
    let text = serde_json::to_string_pretty(&patched).map_err(|e| e.to_string())?;
    std::fs::write(&program_path, format!("{text}\n")).map_err(|e| e.to_string())?;

    let sheet_path = dir.join(format!("{stem}.sheet.png"));
    let mut qa_args = vec![
        "program".to_string(),
        "qa".to_string(),
        program_path.display().to_string(),
        "--sheet".to_string(),
        sheet_path.display().to_string(),
        "--assets".to_string(),
        ctx.assets.display().to_string(),
    ];
    if let Some(a) = str_field(wrapper, "aspect") {
        qa_args.extend(["--aspect".into(), a.into()]);
    }
    let _ = run_engine(ctx, &qa_args);

    let no_render = wrapper.get("no_render").and_then(Value::as_bool) == Some(true);
    let video_rel = if no_render {
        Value::Null
    } else {
        let mut render_args = vec![
            "program".to_string(),
            "render".to_string(),
            program_path.display().to_string(),
            "--assets".to_string(),
            ctx.assets.display().to_string(),
        ];
        if let Some(a) = str_field(wrapper, "aspect") {
            render_args.extend(["--aspect".into(), a.into()]);
        }
        run_engine(ctx, &render_args)?;
        let suffix = str_field(wrapper, "aspect")
            .map(|x| format!("_{}", x.replace(':', "x")))
            .unwrap_or_default();
        let out_stem = format!("{stem}{suffix}");
        let video_path = ctx
            .repo
            .join("output")
            .join(&out_stem)
            .join(format!("{out_stem}.mp4"));
        json!(ctx.rel(&video_path))
    };

    let seconds = patched
        .pointer("/meta/duration")
        .and_then(Value::as_f64)
        .unwrap_or(14.0);
    let effective_aspect = str_field(wrapper, "aspect")
        .or_else(|| patched.pointer("/meta/aspect").and_then(Value::as_str))
        .unwrap_or("9:16");
    Ok(enrich_urls(
        ctx,
        json!({
            "video": video_rel,
            "program": ctx.rel(&program_path),
            "sheet": ctx.rel(&sheet_path),
            "aspect": effective_aspect,
            "seconds": seconds,
            "qa": "pass",
            "top": []
        }),
    ))
}

fn handle_make(ctx: &ServerCtx, a: &Value, default_aspect: Option<&str>) -> Result<Value, String> {
    if let Some(edit_path) = str_field(a, "edit") {
        let abs = ctx.abs(edit_path);
        if abs.components().any(|c| matches!(c, Component::ParentDir))
            || !abs.starts_with(&ctx.repo)
        {
            return Err("edit path must be inside repository".into());
        }
        let text = std::fs::read_to_string(&abs)
            .map_err(|_| format!("{edit_path}: program file not found"))?;
        let prog: Value = serde_json::from_str(&text).map_err(|e| format!("program JSON: {e}"))?;
        return render_program_v2(ctx, &prog, a, default_aspect);
    }
    if let Some(prog) = a.get("program").filter(|v| v.is_object()) {
        return render_program_v2(ctx, prog, a, default_aspect);
    }
    let mut args = vec!["make".to_string()];
    let recipe = str_field(a, "recipe").or_else(|| str_field(a, "style"));
    if let Some(r) = recipe {
        if !RECIPES.contains(&r) {
            return Err(format!("recipe {r}: use one of {}", RECIPES.join(", ")));
        }
        args.extend(["--recipe".into(), r.into()]);
    }
    let mut temp_intent: Option<PathBuf> = None;
    match (
        str_field(a, "brief"),
        a.get("intent").filter(|v| v.is_object()),
    ) {
        (Some(b), _) => args.extend(["--brief".into(), b.chars().take(4000).collect()]),
        (None, Some(i)) => {
            let dir = ctx.repo.join("output/make");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let text = i.to_string();
            let hash = text.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
            });
            let f = dir.join(format!(".intent_{hash:016x}.json"));
            std::fs::write(&f, text).map_err(|e| e.to_string())?;
            args.extend(["--intent".into(), f.display().to_string()]);
            temp_intent = Some(f);
        }
        (None, None) => {
            return Err("provide 'brief', 'intent', 'program', or 'edit' in JSON body".into())
        }
    }
    let aspect = str_field(a, "aspect").or(default_aspect);
    if let Some(x) = aspect {
        args.extend(["--aspect".into(), x.into()]);
    }
    if let Some(s) = a
        .get("seconds")
        .or_else(|| a.get("duration"))
        .and_then(Value::as_f64)
    {
        args.extend(["--duration".into(), format!("{s}")]);
    }
    if let Some(pal) = str_or_json_field(a, "palette") {
        args.extend(["--palette".into(), pal]);
    }
    if let Some(fonts) = str_or_json_field(a, "fonts") {
        args.extend(["--fonts".into(), fonts]);
    }
    if let Some(ending) = str_field(a, "ending") {
        args.extend(["--ending".into(), ending.into()]);
    }
    if let Some(badge) = str_field(a, "ending_badge") {
        args.extend(["--ending-badge".into(), badge.into()]);
    }
    if let Some(specs) = str_field(a, "ending_specs") {
        args.extend(["--ending-specs".into(), specs.into()]);
    }
    if let Some(track) = str_field(a, "music_track") {
        args.extend(["--music-track".into(), track.into()]);
    }
    if a.get("proxy").and_then(Value::as_bool) == Some(true) {
        args.push("--proxy".into());
    }
    if a.get("no_render").and_then(Value::as_bool) == Some(true) {
        args.push("--no-render".into());
    }
    args.extend(["--assets".into(), ctx.assets.display().to_string()]);
    let res = run_engine(ctx, &args);
    if let Some(f) = temp_intent {
        let _ = std::fs::remove_file(f);
    }
    let out = res?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut v = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .ok_or_else(|| "engine returned no JSON output".to_string())?;
    if let Some(o) = v.as_object_mut() {
        o.remove("scene");
        if let Some(x) = aspect {
            o.insert("aspect".into(), json!(x));
        }
    }
    Ok(enrich_urls(ctx, v))
}

fn serve_file(ctx: &ServerCtx, stream: &mut TcpStream, rel_path: &str) -> Result<()> {
    let p = Path::new(rel_path);
    if p.components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
    {
        return write_json(stream, 403, &json!({"error": "forbidden path"}));
    }
    let full = ctx.repo.join(p);
    let allowed_root = ctx.repo.join("output");
    if !full.starts_with(&allowed_root) || !full.is_file() {
        return write_json(stream, 404, &json!({"error": "file not found"}));
    }
    let mime = match full.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "mp4" => "video/mp4",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "json" => "application/json",
        _ => "application/octet-stream",
    };
    let bytes = std::fs::read(&full)?;
    write_bytes(stream, 200, mime, &bytes)
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        _ => "Internal Server Error",
    }
}

fn write_empty_cors(stream: &mut TcpStream, code: u16) -> Result<()> {
    let hdr = format!(
        "HTTP/1.1 {code} {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        status_text(code)
    );
    stream.write_all(hdr.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn write_bytes(stream: &mut TcpStream, code: u16, mime: &str, body: &[u8]) -> Result<()> {
    let hdr = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        status_text(code),
        body.len()
    );
    stream.write_all(hdr.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

fn write_json(stream: &mut TcpStream, code: u16, v: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(v)?;
    bytes.push(b'\n');
    write_bytes(stream, code, "application/json", &bytes)
}
