//! `plan_assets(job)`: the pictures a job is missing (nouns the library cannot
//! show, so they would appear as text) as image-generator prompts in the
//! video's own style. Nothing is generated here: the model makes the pictures
//! elsewhere (with its own image tools, within its own budget) and passes the
//! files back through `assets: "<folder>"` (plan §5a).
//!
//! The engine does the work: `motion-engine plan-assets <job>/intent.json
//! --style <job>/style.json --assets <assets> --asset-family <the look's
//! families>` and `motion-engine asset-prompts`. Only requests the library
//! cannot serve (`generated_image`) become prompts, and pictures the job
//! already has from the user's images are left out.

use std::path::Path;
use std::time::Duration;

use motion_core::compiler::art_direction::{look_defaults, Look};
use motion_core::{CreativeIntent, StyleProfile};
use serde_json::{json, Value};

use super::explore::auto_look;
use super::{job_dir_of, needs_fix, parse_args, rel, ToolOutput};
use crate::args::PlanAssetsArgs;
use crate::job::{INTENT_JSON, MANIFEST_JSON, REQUEST_JSON, STYLE_JSON};
use crate::profile::ServerConfig;
use crate::reply::{cut, Fix, Reply};

/// Folder of a job's asset plan.
pub const PLAN_DIR: &str = "assets_plan";
/// Prompt lines in the reply text.
pub const MAX_TEXT_LINES: usize = 5;
/// Characters of a prompt in a text line.
pub const PROMPT_CHARS: usize = 80;
pub const NEXT_MAKE: &str = "Make these images elsewhere, put them in assets/inbox/<folder>/ as beatN.png, then call make_video with assets: \"<folder>\".";
pub const NEXT_NONE: &str = "Nothing to make: every picture comes from the library.";

const ENGINE_TIMEOUT: Duration = Duration::from_secs(120);

/// One picture to make.
#[derive(Debug, Clone, PartialEq)]
pub struct Missing {
    /// 1-based beat (from the request id `beat_<n>.<role>`).
    pub beat: Option<usize>,
    pub role: String,
    pub subject: String,
    pub prompt: String,
    /// The first request id the picture serves.
    pub id: String,
    /// Every request id it serves.
    pub serves: Vec<String>,
}

/// The file name the picture should have in the user's folder (§5a).
pub fn file_name(m: &Missing) -> String {
    let kind = match m.role.as_str() {
        "hero_subject" | "portrait" => "person",
        "environment" => "place",
        _ => "object",
    };
    match m.beat {
        Some(b) => format!("beat{b}_{kind}.png"),
        None => format!("{}.png", m.id.replace('.', "_")),
    }
}

/// The families the job's compile enables: the request's own list, else the
/// look's (`options.art`, `auto` = the look `--art auto` picks).
pub fn families_for(request: &Value, intent: &CreativeIntent, style: &StyleProfile) -> Vec<String> {
    let listed: Vec<String> = request["options"]["families"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_str())
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    let art = request["options"]["art"].as_str().map(str::trim);
    let look = match art {
        Some("none") => return Vec::new(),
        Some(name) if !name.is_empty() && name != "auto" => {
            Look::parse(name).unwrap_or_else(|| auto_look(intent, style).0)
        }
        _ => auto_look(intent, style).0,
    };
    look_defaults(look)
        .families
        .iter()
        .map(|f| f.to_string())
        .collect()
}

/// The prompts that would be generated, from an `asset-prompts` file, leaving
/// out pictures whose every request the job's manifest already serves.
pub fn missing_from(prompts: &Value, manifest_ids: &[String]) -> Vec<Missing> {
    prompts["specs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["id"].as_str()?.to_string();
            let serves: Vec<String> = s["serves"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| vec![id.clone()]);
            if serves.iter().all(|r| manifest_ids.contains(r)) {
                return None;
            }
            let beat = id
                .strip_prefix("beat_")
                .and_then(|r| r.split('.').next())
                .and_then(|n| n.parse::<usize>().ok());
            let role = s["role"]
                .as_str()
                .map(str::to_string)
                .or_else(|| id.split('.').nth(1).map(str::to_string))
                .unwrap_or_default();
            Some(Missing {
                beat,
                role,
                subject: s["subject"].as_str().unwrap_or("").to_string(),
                prompt: s["prompt"].as_str().unwrap_or("").to_string(),
                id,
                serves,
            })
        })
        .collect()
}

/// `beat 2 hero_object "zorb": <prompt cut to 80 chars>`.
pub fn line_of(m: &Missing) -> String {
    let prompt: String = m.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let prompt = if prompt.chars().count() > PROMPT_CHARS {
        let mut p: String = prompt.chars().take(PROMPT_CHARS - 1).collect();
        p.push('…');
        p
    } else {
        prompt
    };
    let who = match m.beat {
        Some(b) => format!("beat {b} {}", m.role),
        None => m.role.clone(),
    };
    format!("{who} \"{}\": {prompt}", m.subject)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The `plan_assets` tool.
pub fn plan_assets(config: &ServerConfig, args: Value) -> ToolOutput {
    let a: PlanAssetsArgs = match parse_args(args) {
        Ok(a) => a,
        Err(fix) => return needs_fix(fix),
    };
    let (id, dir) = match job_dir_of(config, &a.job) {
        Ok(x) => x,
        Err(out) => return out,
    };
    let intent_path = dir.join(INTENT_JSON);
    let style_path = dir.join(STYLE_JSON);
    let intent = std::fs::read_to_string(&intent_path)
        .ok()
        .and_then(|t| CreativeIntent::from_json(&t).ok());
    let Some(intent) = intent else {
        return needs_fix(
            Fix::new(None, "job", format!("job {id} has no readable story"))
                .otherwise("call make_video first"),
        );
    };
    let style = std::fs::read_to_string(&style_path)
        .ok()
        .and_then(|t| StyleProfile::from_json(&t).ok())
        .unwrap_or_default();
    let request = read_json(&dir.join(REQUEST_JSON)).unwrap_or(Value::Null);
    let families = families_for(&request, &intent, &style);

    let work = dir.join(PLAN_DIR);
    if let Err(e) = std::fs::create_dir_all(&work) {
        return ToolOutput::from(Reply::failed(
            Some(id.to_string()),
            format!("cannot write {}: {e}", work.display()),
        ));
    }
    let plan = work.join("plan.json");
    let prompts = work.join("prompts.json");
    let mut cmd = config.engine_command();
    cmd.arg("plan-assets")
        .arg(&intent_path)
        .arg("--style")
        .arg(&style_path)
        .arg("--assets")
        .arg(&config.assets);
    for f in &families {
        cmd.arg("--asset-family").arg(f);
    }
    cmd.arg("-o").arg(&plan);
    let mut cmd2 = config.engine_command();
    cmd2.arg("asset-prompts").arg(&plan).arg("-o").arg(&prompts);
    let ran = super::proc::run("plan-assets", cmd, ENGINE_TIMEOUT)
        .and_then(|_| super::proc::run("asset-prompts", cmd2, ENGINE_TIMEOUT));
    if let Err(e) = ran {
        return ToolOutput::from(Reply::failed(Some(id.to_string()), e));
    }
    let Some(prompt_set) = read_json(&prompts) else {
        return ToolOutput::from(Reply::failed(
            Some(id.to_string()),
            "asset-prompts wrote no readable prompts",
        ));
    };
    let manifest_ids: Vec<String> = read_json(&dir.join(MANIFEST_JSON))
        .and_then(|m| {
            m["assets"].as_array().map(|a| {
                a.iter()
                    .filter_map(|e| e["id"].as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default();
    let missing = missing_from(&prompt_set, &manifest_ids);

    let structured_missing: Vec<Value> = missing
        .iter()
        .map(|m| {
            json!({
                "beat": m.beat, "role": m.role, "subject": m.subject, "prompt": m.prompt,
                "id": m.id, "serves": m.serves, "file": file_name(m),
            })
        })
        .collect();
    let next = if missing.is_empty() {
        NEXT_NONE
    } else {
        NEXT_MAKE
    };
    let mut text = if missing.is_empty() {
        format!("done · job {id} · nothing missing: every picture comes from the library")
    } else if missing.len() > MAX_TEXT_LINES {
        format!(
            "done · job {id} · {} pictures to make (first {MAX_TEXT_LINES} shown)",
            missing.len()
        )
    } else {
        format!("done · job {id} · {} picture(s) to make", missing.len())
    };
    for m in missing.iter().take(MAX_TEXT_LINES) {
        text.push_str(&format!("\n{}", cut(&line_of(m))));
    }
    text.push_str(&format!("\nnext: {next}"));
    ToolOutput {
        structured: json!({
            "status": "done",
            "job": id.as_str(),
            "missing": structured_missing,
            "families": families,
            "prompts": rel(config, &prompts),
            "next": next,
        }),
        text,
        images: Vec::new(),
        links: Vec::new(),
        is_error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, role: &str, subject: &str, prompt: &str) -> Value {
        json!({"id": id, "role": role, "subject": subject, "prompt": prompt, "serves": [id]})
    }

    #[test]
    fn lines_name_beat_role_subject_and_cut_the_prompt() {
        let set = json!({"specs": [spec(
            "beat_2.hero_object", "hero_object", "zorb",
            &format!("A zorb {}", "ball ".repeat(40)),
        )]});
        let m = missing_from(&set, &[]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].beat, Some(2));
        let line = line_of(&m[0]);
        assert!(
            line.starts_with("beat 2 hero_object \"zorb\": A zorb ball"),
            "{line}"
        );
        let prompt = line.split_once(": ").unwrap().1;
        assert_eq!(prompt.chars().count(), PROMPT_CHARS);
        assert!(prompt.ends_with('…'));
        assert_eq!(file_name(&m[0]), "beat2_object.png");
    }

    #[test]
    fn user_images_already_cover_their_requests() {
        let set = json!({"specs": [
            spec("beat_1.hero_subject", "hero_subject", "a nurse", "x"),
            spec("beat_3.hero_object", "hero_object", "zorb", "y"),
        ]});
        let m = missing_from(&set, &["beat_1.hero_subject".to_string()]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].subject, "zorb");
        assert_eq!(file_name(&missing_from(&set, &[])[0]), "beat1_person.png");
    }

    #[test]
    fn families_follow_the_request_then_the_look() {
        let intent =
            CreativeIntent::from_json(include_str!("../../tests/fixtures/creator/intent.json"))
                .unwrap();
        let style = StyleProfile::default();
        let own = json!({"options": {"families": ["woodcut_kitchen", " "]}});
        assert_eq!(families_for(&own, &intent, &style), vec!["woodcut_kitchen"]);
        let forced = json!({"options": {"art": "clay_pop"}});
        let fam = families_for(&forced, &intent, &style);
        assert_eq!(
            fam,
            look_defaults(Look::ClayPop)
                .families
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
        );
        assert!(families_for(&json!({"options": {"art": "none"}}), &intent, &style).is_empty());
        // `auto` and a missing look both resolve to the story's own look.
        assert_eq!(
            families_for(&json!({"options": {"art": "auto"}}), &intent, &style),
            families_for(&Value::Null, &intent, &style)
        );
    }
}
