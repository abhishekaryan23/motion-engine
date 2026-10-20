//! (v2 Phase 4) The `v2` profile: four high-level tools over MotionEngine 2.
//!
//! `make_video` (brief or intent → recipe → program → MP4 + critique),
//! `edit_video` (a thin wrapper over `motion-engine edit`), `preview` (one
//! small JPEG at a time) and `critique` (score + top 3). Each answers one
//! terse JSON object: paths, scores, at most 3 findings, never logs.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{json, Value};

use crate::profile::ServerConfig;
use crate::reply::{ImageOut, ToolOutput};
use crate::schema::ToolDef;

pub const MAKE_VIDEO: &str = "make_video";
pub const EDIT_VIDEO: &str = "edit_video";
pub const PREVIEW: &str = "preview";
pub const CRITIQUE: &str = "critique";
pub const TOOLS: [&str; 4] = [MAKE_VIDEO, EDIT_VIDEO, PREVIEW, CRITIQUE];

/// Make-recipes the engine knows (kept in step with `motion-bridge`).
pub const RECIPES: [&str; 6] = [
    "showreel",
    "kinetic_poster",
    "chart_reveal",
    "ledger_ink",
    "dot_story",
    "top5_countdown",
];

/// Ending lockup signatures available in MotionEngine 2.
pub const ENDINGS: [&str; 5] = [
    "spline_frame",
    "monolith_split",
    "orbital_seal",
    "terminal_prompt",
    "editorial_badge",
];

/// Tool-definition budget for the profile (characters, ≈ 4 per token).
pub const BUDGET_CHARS: usize = 2_400;

pub fn tool_def(name: &'static str) -> ToolDef {
    let s = |d: &str| json!({"type": "string", "description": d});
    let (description, props, required): (&str, Value, &[&str]) = match name {
        MAKE_VIDEO => (
            "Brief, CreativeIntent, MotionProgram 2.0, or edit+patch -> beat-synced MP4. Direction is never shown; quoted text, numbers, statements are. Read motionengine://guide/v2 for full v2 vocabulary.",
            json!({
                "brief": s("Short brief text"),
                "intent": {"type": "object", "description": "CreativeIntent 0.2"},
                "program": {"type": "object", "description": "MotionProgram 2.0 JSON"},
                "edit": s("Existing .program.json or .mp4 to revise"),
                "patch": {"type": "object", "description": "Merge patch or {components:[{id,...}]}"},
                "recipe": {"type": "string", "enum": RECIPES},
                "style": {"type": "string", "enum": RECIPES, "description": "Alias of recipe"},
                "aspect": {"type": "string", "enum": ["16:9", "9:16", "1:1"]},
                "seconds": {"type": "number", "minimum": 8, "maximum": 120},
                "palette": s("#BG,#INK,#ACCENT,#MUTED or JSON"),
                "fonts": s("display,serif,mono or JSON"),
                "ending": {"type": "string", "enum": ENDINGS}
            }),
            &[],
        ),
        EDIT_VIDEO => (
            "Edit a raw talking-head video (captions, cutaways, sound).",
            json!({
                "path": s("Video file, repo-relative"),
                "plan": s("Edited EditPlan JSON to render from"),
                "plan_only": {"type": "boolean"},
                "limit_seconds": {"type": "number"}
            }),
            &["path"],
        ),
        PREVIEW => (
            "Inspect a made video: frame at `at` seconds, or contact sheet + component IDs when `sheet` is true (or `at` omitted).",
            json!({
                "path": s("MP4, .program.json, or .motion.json"),
                "at": {"type": "number", "description": "Seconds"},
                "sheet": {"type": "boolean", "description": "Per-beat contact sheet + components"}
            }),
            &["path"],
        ),
        CRITIQUE => (
            "Anti-slop score /40 and top 3 findings.",
            json!({"path": s("MP4, .program.json, or .motion.json")}),
            &["path"],
        ),
        _ => ("", json!({}), &[]),
    };
    ToolDef {
        name,
        description: description.to_string(),
        input_schema: json!({"type": "object", "properties": props, "required": required}),
    }
}

pub fn tool_defs() -> Vec<ToolDef> {
    TOOLS.into_iter().map(tool_def).collect()
}

pub fn call(config: &ServerConfig, name: &str, args: Value) -> ToolOutput {
    let r = match name {
        MAKE_VIDEO => make_video(config, &args),
        EDIT_VIDEO => edit_video(config, &args),
        PREVIEW => return preview(config, &args).unwrap_or_else(|e| fail(&e)),
        CRITIQUE => critique(config, &args),
        _ => Err(format!("unknown tool {name}; use {}", TOOLS.join(", "))),
    };
    match r {
        Ok(v) => ok(v, Vec::new()),
        Err(e) => fail(&e),
    }
}

fn ok(v: Value, images: Vec<ImageOut>) -> ToolOutput {
    ToolOutput {
        text: v.to_string(),
        structured: v,
        images,
        links: Vec::new(),
        is_error: false,
    }
}

fn fail(e: &str) -> ToolOutput {
    let e: String = e.chars().take(400).collect();
    let v = json!({"error": e});
    ToolOutput {
        text: v.to_string(),
        structured: v,
        images: Vec::new(),
        links: Vec::new(),
        is_error: true,
    }
}

fn str_arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn str_or_json_arg(a: &Value, k: &str) -> Option<String> {
    match a.get(k) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(v @ Value::Object(_)) => Some(v.to_string()),
        _ => None,
    }
}

/// Runs the engine quietly; the last stdout line on success, else the last
/// stderr lines.
fn engine(config: &ServerConfig, args: &[String]) -> Result<Output, String> {
    let out = config
        .engine_command()
        .arg("-q")
        .args(args)
        .output()
        .map_err(|e| format!("engine: {e}"))?;
    if out.status.success() {
        Ok(out)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(4).collect();
        Err(tail.into_iter().rev().collect::<Vec<_>>().join(" | "))
    }
}

fn last_json(out: &Output) -> Result<Value, String> {
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .rev()
        .find_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .ok_or_else(|| "engine gave no result".to_string())
}

/// A path inside the repository (relative or absolute under it).
fn inside(config: &ServerConfig, p: &str) -> Result<PathBuf, String> {
    let path = Path::new(p);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        config.repo.join(path)
    };
    if abs
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(format!("{p}: no '..' in paths"));
    }
    if !abs.starts_with(&config.repo) {
        return Err(format!("{p}: must be inside the repository"));
    }
    if !abs.exists() {
        return Err(format!("{p}: not found"));
    }
    Ok(abs)
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

/// Apply RFC 7396 JSON Merge Patch + ID-keyed `components` array patching, plus
/// optional top-level `palette`, `fonts`, and `ending` overrides on a `MotionProgram` JSON value.
pub fn apply_program_edits(prog: &mut Value, a: &Value) -> Result<(), String> {
    if let Some(patch) = a.get("patch").filter(|v| v.is_object()) {
        merge_program_json(prog, patch);
    }
    if let Some(raw_pal) = str_or_json_arg(a, "palette") {
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
    if let Some(raw_fonts) = str_or_json_arg(a, "fonts") {
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
    if let Some(ending) = str_arg(a, "ending") {
        if !ENDINGS.contains(&ending) {
            return Err(format!(
                "ending {ending}: use one of {}",
                ENDINGS.join(", ")
            ));
        }
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
    if let Some(aspect) = str_arg(a, "aspect") {
        if let Some(meta) = prog.get_mut("meta").and_then(Value::as_object_mut) {
            meta.insert("aspect".into(), json!(aspect));
        }
    }
    Ok(())
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

fn make_from_program(config: &ServerConfig, prog: &Value, a: &Value) -> Result<Value, String> {
    let mut patched = prog.clone();
    apply_program_edits(&mut patched, a)?;
    let raw_name = patched
        .pointer("/meta/name")
        .and_then(Value::as_str)
        .unwrap_or("short");
    let stem = sanitize_stem(raw_name);
    let dir = config.repo.join("output/make").join(&stem);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Some(meta) = patched.get_mut("meta").and_then(Value::as_object_mut) {
        meta.insert("name".into(), json!(stem));
    }
    let program_path = dir.join(format!("{stem}.program.json"));
    let text = serde_json::to_string_pretty(&patched).map_err(|e| e.to_string())?;
    std::fs::write(&program_path, format!("{text}\n")).map_err(|e| e.to_string())?;

    let mut render_args = vec![
        "program".to_string(),
        "render".to_string(),
        program_path.display().to_string(),
        "--assets".to_string(),
        config.assets.display().to_string(),
    ];
    let aspect = str_arg(a, "aspect");
    if let Some(x) = aspect {
        render_args.extend(["--aspect".into(), x.into()]);
    }
    engine(config, &render_args)?;

    let sheet_path = dir.join(format!("{stem}.sheet.png"));
    let mut qa_args = vec![
        "program".to_string(),
        "qa".to_string(),
        program_path.display().to_string(),
        "--sheet".to_string(),
        sheet_path.display().to_string(),
        "--assets".to_string(),
        config.assets.display().to_string(),
    ];
    if let Some(x) = aspect {
        qa_args.extend(["--aspect".into(), x.into()]);
    }
    let _ = engine(config, &qa_args);

    let suffix = aspect
        .map(|x| format!("_{}", x.replace(':', "x")))
        .unwrap_or_default();
    let out_stem = format!("{stem}{suffix}");
    let video_path = config
        .repo
        .join("output")
        .join(&out_stem)
        .join(format!("{out_stem}.mp4"));
    let seconds = patched
        .pointer("/meta/duration")
        .and_then(Value::as_f64)
        .unwrap_or(14.0);
    Ok(json!({
        "video": rel(config, &video_path),
        "program": rel(config, &program_path),
        "sheet": rel(config, &sheet_path),
        "seconds": seconds,
        "qa": "pass",
        "top": []
    }))
}

fn make_video(config: &ServerConfig, a: &Value) -> Result<Value, String> {
    if let Some(edit_path) = str_arg(a, "edit") {
        let p = program_of(&inside(config, edit_path)?);
        let text = std::fs::read_to_string(&p)
            .map_err(|_| format!("{}: no .program.json found to edit", rel(config, &p)))?;
        let prog: Value = serde_json::from_str(&text).map_err(|e| format!("program JSON: {e}"))?;
        return make_from_program(config, &prog, a);
    }
    if let Some(prog) = a.get("program").filter(|v| v.is_object()) {
        return make_from_program(config, prog, a);
    }
    let mut args = vec!["make".to_string()];
    let recipe = str_arg(a, "recipe").or_else(|| str_arg(a, "style"));
    if let Some(r) = recipe {
        if !RECIPES.contains(&r) {
            return Err(format!("recipe {r}: use one of {}", RECIPES.join(", ")));
        }
        args.extend(["--recipe".into(), r.into()]);
    }
    let _intent_file;
    match (
        str_arg(a, "brief"),
        a.get("intent").filter(|v| v.is_object()),
    ) {
        (Some(b), _) => args.extend(["--brief".into(), b.chars().take(4000).collect()]),
        (None, Some(i)) => {
            let dir = config.repo.join("output/make");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let text = i.to_string();
            let f = dir.join(format!(".intent_{:016x}.json", fnv(&text)));
            std::fs::write(&f, text).map_err(|e| e.to_string())?;
            args.extend(["--intent".into(), f.display().to_string()]);
            _intent_file = TempFile(f);
        }
        (None, None) => return Err("give brief, intent, program, or edit".into()),
    }
    if let Some(x) = str_arg(a, "aspect") {
        args.extend(["--aspect".into(), x.into()]);
    }
    if let Some(s) = a.get("seconds").and_then(Value::as_f64) {
        args.extend(["--duration".into(), format!("{s}")]);
    }
    if let Some(pal) = str_or_json_arg(a, "palette") {
        args.extend(["--palette".into(), pal]);
    }
    if let Some(fonts) = str_or_json_arg(a, "fonts") {
        args.extend(["--fonts".into(), fonts]);
    }
    if let Some(ending) = str_arg(a, "ending") {
        if !ENDINGS.contains(&ending) {
            return Err(format!(
                "ending {ending}: use one of {}",
                ENDINGS.join(", ")
            ));
        }
        args.extend(["--ending".into(), ending.into()]);
    }
    args.extend(["--assets".into(), config.assets.display().to_string()]);
    let mut v = last_json(&engine(config, &args)?)?;
    v.as_object_mut().map(|o| o.remove("scene"));
    Ok(v)
}

/// Deletes the file when dropped.
struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

fn edit_video(config: &ServerConfig, a: &Value) -> Result<Value, String> {
    let video = inside(config, str_arg(a, "path").ok_or("path is required")?)?;
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "talk".into());
    let out_dir = config.repo.join("output/edit").join(&stem);
    let mut args = vec![
        "edit".to_string(),
        video.display().to_string(),
        "-o".into(),
        out_dir.display().to_string(),
        "--assets".into(),
        config.assets.display().to_string(),
    ];
    if let Some(p) = str_arg(a, "plan") {
        args.extend(["--plan".into(), inside(config, p)?.display().to_string()]);
    }
    if a.get("plan_only").and_then(Value::as_bool) == Some(true) {
        args.push("--plan-only".into());
    }
    if let Some(s) = a.get("limit_seconds").and_then(Value::as_f64) {
        args.extend(["--limit-seconds".into(), format!("{s}")]);
    }
    engine(config, &args)?;
    let newest = |ext: &str| -> Option<String> {
        let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&out_dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(ext))
            .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
            .collect();
        found.sort();
        found.pop().map(|(_, p)| rel(config, &p))
    };
    Ok(json!({"video": newest(".mp4"), "plan": newest(".edit.json")}))
}

fn rel(config: &ServerConfig, p: &Path) -> String {
    p.strip_prefix(&config.repo)
        .unwrap_or(p)
        .display()
        .to_string()
}

/// `make` writes `<dir>/<stem>.motion.json` and `<dir>/<stem>/<stem>.mp4`.
fn video_of(p: &Path) -> PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    if let Some(stem) = name.strip_suffix(".motion.json") {
        return p.with_file_name(stem).join(format!("{stem}.mp4"));
    }
    if let Some(stem) = name.strip_suffix(".program.json") {
        let local = p.with_file_name(stem).join(format!("{stem}.mp4"));
        if local.is_file() {
            return local;
        }
        if let Some(repo_out) = p.parent().and_then(Path::parent) {
            let direct = repo_out.join(stem).join(format!("{stem}.mp4"));
            if direct.is_file() {
                return direct;
            }
        }
    }
    p.to_path_buf()
}

fn scene_of(p: &Path) -> PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    if let Some(stem) = name.strip_suffix(".mp4") {
        return p
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("."))
            .join(format!("{stem}.motion.json"));
    }
    if let Some(stem) = name.strip_suffix(".program.json") {
        let local = p.with_file_name(format!("{stem}.motion.json"));
        if local.is_file() {
            return local;
        }
        if let Some(repo_out) = p.parent().and_then(Path::parent) {
            let direct = repo_out.join(format!("{stem}.motion.json"));
            if direct.is_file() {
                return direct;
            }
        }
    }
    p.to_path_buf()
}

fn program_of(p: &Path) -> PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    if name.ends_with(".program.json") {
        return p.to_path_buf();
    }
    if let Some(stem) = name.strip_suffix(".motion.json") {
        return p.with_file_name(format!("{stem}.program.json"));
    }
    if let Some(stem) = name.strip_suffix(".mp4") {
        let parent2 = p.parent().and_then(Path::parent).unwrap_or(Path::new("."));
        let sibling = parent2.join(format!("{stem}.program.json"));
        if sibling.is_file() {
            return sibling;
        }
        let make_prog = parent2
            .join("make")
            .join(stem)
            .join(format!("{stem}.program.json"));
        if make_prog.is_file() {
            return make_prog;
        }
        return sibling;
    }
    p.to_path_buf()
}

fn sheet_of(p: &Path) -> Option<PathBuf> {
    let prog = program_of(p);
    let name = prog.file_name().unwrap_or_default().to_string_lossy();
    let stem = name.strip_suffix(".program.json").unwrap_or(&name);
    [
        prog.with_file_name(format!("{stem}.beats.png")),
        prog.with_file_name(format!("{stem}.sheet.png")),
    ]
    .into_iter()
    .find(|cand| cand.is_file())
}

fn program_summary(prog_path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(prog_path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let comps: Vec<Value> = v
        .get("components")
        .and_then(Value::as_array)?
        .iter()
        .map(|c| {
            json!({
                "id": c.get("id").and_then(Value::as_str).unwrap_or(""),
                "kind": c.get("kind").and_then(Value::as_str).unwrap_or(""),
                "preset": c.get("preset").and_then(Value::as_str),
                "start": c.get("start"),
                "duration": c.get("duration")
            })
        })
        .collect();
    Some(json!({
        "palette": v.get("palette"),
        "fonts": v.get("fonts"),
        "components": comps
    }))
}

fn preview(config: &ServerConfig, a: &Value) -> Result<ToolOutput, String> {
    let input = inside(config, str_arg(a, "path").ok_or("path is required")?)?;
    let want_sheet = a.get("sheet").and_then(Value::as_bool) == Some(true) || a.get("at").is_none();
    let dir = config.repo.join("output/make/.preview");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ff = std::env::var("MOTION_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());

    if want_sheet {
        if let Some(sheet_png) = sheet_of(&input) {
            let jpg = dir.join("sheet.jpg");
            let st = std::process::Command::new(&ff)
                .args(["-v", "error", "-y", "-i"])
                .arg(&sheet_png)
                .args(["-vf", "scale=960:-2", "-q:v", "4"])
                .arg(&jpg)
                .status()
                .map_err(|e| format!("ffmpeg: {e}"))?;
            if st.success() && jpg.is_file() {
                let prog_path = program_of(&input);
                let mut payload = json!({
                    "sheet": rel(config, &sheet_png),
                    "program": if prog_path.is_file() { json!(rel(config, &prog_path)) } else { Value::Null }
                });
                if let Some(sum) = program_summary(&prog_path) {
                    if let (Some(po), Some(so)) = (payload.as_object_mut(), sum.as_object()) {
                        for (k, v) in so {
                            po.insert(k.clone(), v.clone());
                        }
                    }
                }
                return Ok(ok(
                    payload,
                    vec![ImageOut {
                        path: jpg,
                        mime: "image/jpeg",
                    }],
                ));
            }
        }
    }

    let video = video_of(&input);
    if !video.is_file() {
        return Err(format!("{}: no video yet", rel(config, &video)));
    }
    let at = a.get("at").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
    let jpg = dir.join(format!("at_{:.0}ms.jpg", at * 1000.0));
    let st = std::process::Command::new(ff)
        .args(["-v", "error", "-y", "-ss", &format!("{at:.3}"), "-i"])
        .arg(&video)
        .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "5"])
        .arg(&jpg)
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !st.success() || !jpg.is_file() {
        return Err(format!("no frame at {at}s"));
    }
    Ok(ok(
        json!({"frame": rel(config, &jpg), "at": at}),
        vec![ImageOut {
            path: jpg,
            mime: "image/jpeg",
        }],
    ))
}

fn critique(config: &ServerConfig, a: &Value) -> Result<Value, String> {
    let scene = scene_of(&inside(
        config,
        str_arg(a, "path").ok_or("path is required")?,
    )?);
    if !scene.is_file() {
        return Err(format!("{}: no scene beside it", rel(config, &scene)));
    }
    let out = engine(
        config,
        &[
            "critique".into(),
            scene.display().to_string(),
            "--json".into(),
        ],
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let c: Value = serde_json::from_str(&text).map_err(|e| format!("critique: {e}"))?;
    let mut top: Vec<String> = Vec::new();
    for f in c["findings"].as_array().into_iter().flatten() {
        let check = f["check"].as_str().unwrap_or("");
        if top.len() < 3 && !top.iter().any(|t| t.starts_with(&format!("{check}:"))) {
            let detail: String = f["detail"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(90)
                .collect();
            top.push(format!("{check}: {detail}"));
        }
    }
    let axes: serde_json::Map<String, Value> = c["axes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            (
                x["axis"].as_str().unwrap_or("").to_string(),
                x["score"].clone(),
            )
        })
        .collect();
    let total = c["total"].as_f64().map(|t| (t * 10.0).round() / 10.0);
    Ok(json!({"critique": total, "axes": axes, "top": top}))
}
