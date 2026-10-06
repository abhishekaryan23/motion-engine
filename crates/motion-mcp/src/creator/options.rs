//! `list_options` and [`options_json`]: the values `make_video` options accept
//! (looks, asset families with their medium, music beds, tones), each with a
//! one-line best use. Static data, so the same JSON is also the
//! `motionengine://options` resource.

use std::path::Path;

use motion_core::compiler::art_direction::{look_defaults, Look};
use serde_json::{json, Value};

use super::{needs_fix, parse_args, ToolOutput};
use crate::args::ListOptionsArgs;
use crate::profile::ServerConfig;
use crate::reply::Fix;

/// The text of a `list_options` reply stays within this many characters.
pub const TEXT_BUDGET: usize = 1_000;

/// Best use of a look, at most 15 words (from the look's doc comment).
pub fn look_best_use(look: Look) -> &'static str {
    match look {
        Look::ClassicalNeon => "Dark marble statues in greyscale with one neon accent. Drama, luxury, serious ideas.",
        Look::HalftoneCutout => "Black-and-white halftone photo cutouts on bold print grounds. Energy, urgency, retro fun.",
        Look::ClayPop => "Soft clay props and icons that pop in with a spring. Joyful, friendly topics.",
        Look::OrnamentEditorial => "Editorial page with ornament corners and dividers on paper. Trust, warmth, handmade stories.",
        Look::Journey => "A ribbon path draws through the beats. Calm, precise, step-by-step explainers.",
        Look::StreetCollage => "Big cutouts, stencil words, tape, hard cuts and shake. Music, sports, street culture.",
        Look::Dossier => "Evidence documents slide in, get stamped and highlighted. Facts, money, history, investigations.",
        Look::HypeSlam => "Words and pictures slam in on fast cuts with shake and glitch. Short punchy scripts.",
        Look::StudioPop => "One big cutout over a colour disc, spoken words behind it. Creator and brand stories.",
        Look::Cinematic3d => "Beats staged in depth with a flying camera and rack focus. Technology, science, the future.",
    }
}

/// Best use of a tone word, at most 15 words (`None` for a tone this table
/// does not know yet).
pub fn tone_best_use(tone: &str) -> Option<&'static str> {
    Some(match tone {
        "auto" => "The engine picks from the story: the classic warm editorial collage.",
        "editorial" => "Magazine design: serif and sans contrast, generous space, measured pacing.",
        "technical" => "Data and tech topics: structured grid, condensed or mono type, precise motion.",
        "playful" => "Bold type and big colour fields with energetic motion. Light, fun topics.",
        "street" => "Music, sports and street culture: big cutouts, tape, stencil words, hard cuts.",
        "documentary" => "Investigative explainer: stamped evidence documents, film grain. Facts, money, history.",
        "hype" => "Fast slams timed to the voice. Short, punchy scripts and openers.",
        "studio" => "Creator or brand story: one big cutout over a colour disc.",
        "cinematic" => "Beats staged in depth with a flying camera. Big ideas: technology, space, the future.",
        _ => return None,
    })
}

/// The medium of an asset family, from its name (`mixed` when unknown).
pub fn medium_of(family: &str) -> &'static str {
    match family {
        "halftone_retro_objects" => "halftone print",
        "sketch_icons" => "line sketch",
        "woodcut_kitchen" => "woodcut",
        "aikakirja_ornaments" => "ornaments",
        "grounds" => "background plates",
        f if f.starts_with("clay_") => "clay 3D",
        f if f.starts_with("people_") => "photo cutout people",
        f if f.starts_with("classical_") => "classical engraving",
        f if f.starts_with("editorial_") => "editorial photo cutout",
        f if f.starts_with("retro_cars_") => "retro illustration",
        _ => "mixed",
    }
}

/// `<assets>/library/<family>/catalog.json` for every family, by name:
/// `{name, assets, medium}`.
fn families(assets: &Path) -> Vec<Value> {
    let Ok(rd) = std::fs::read_dir(assets.join("library")) else {
        return Vec::new();
    };
    let mut out: Vec<(String, usize)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let text = std::fs::read_to_string(e.path().join("catalog.json")).ok()?;
            let cat: Value = serde_json::from_str(&text).ok()?;
            let n = cat
                .get("assets")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Some((name, n))
        })
        .collect();
    out.sort();
    out.into_iter()
        .map(|(name, n)| json!({ "name": name, "assets": n, "medium": medium_of(&name) }))
        .collect()
}

/// `<assets>/music/catalog.json` beds: `{id, emotions, bpm}` plus, from a v0.2
/// catalog, the mood families the bed fits (`moods`: what the story's mood and
/// the `music` word pick by) and its `energy`, 1 (still) to 5 (driving).
fn music(assets: &Path) -> Vec<Value> {
    let beds = std::fs::read_to_string(assets.join("music/catalog.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|c| c.get("beds").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    beds.iter()
        .filter_map(|b| {
            let mut bed = json!({
                "id": b.get("id")?.as_str()?,
                "emotions": b.get("emotions").cloned().unwrap_or_else(|| json!([])),
            });
            if let Some(bpm) = b.get("bpm") {
                bed["bpm"] = bpm.clone();
            }
            if let Some(moods) = b.get("moods").filter(|m| m.is_array()) {
                bed["moods"] = moods.clone();
            }
            if let Some(energy) = b.get("energy").and_then(Value::as_u64).filter(|e| *e > 0) {
                bed["energy"] = json!(energy);
            }
            Some(bed)
        })
        .collect()
}

/// What a bed is good for, as words: its mood families (v0.2 catalog), else
/// its typography emotions (v0.1).
fn bed_labels(bed: &Value) -> Vec<&str> {
    let words = |key: &str| -> Vec<&str> {
        bed[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    };
    let moods = words("moods");
    if moods.is_empty() {
        words("emotions")
    } else {
        moods
    }
}

/// The `list_options` data: looks (with the families each draws from), asset
/// families (name, picture count, medium), music beds (id, moods, emotions) and
/// tones, each with its best use. Shared by the tool and the
/// `motionengine://options` resource.
pub fn options_json(config: &ServerConfig) -> Value {
    json!({
        "looks": Look::ALL.iter().map(|l| json!({
            "name": l.name(),
            "best_use": look_best_use(*l),
            "families": look_defaults(*l).families,
        })).collect::<Vec<_>>(),
        "families": families(&config.assets),
        "music": music(&config.assets),
        "tones": crate::schema::tones().iter().map(|t| json!({
            "name": t,
            "best_use": tone_best_use(t).unwrap_or("See the tone's description in the style schema."),
        })).collect::<Vec<_>>(),
        "defaults": {"art": "auto", "music": "auto", "captions": true, "variety": "auto"},
        "use": "Pass a look as options.art, families as options.families, a bed as options.music, a tone as style. A mood word as music (auto, none, calm, upbeat, serious, dramatic, playful, neutral) picks the bed by mood.",
    })
}

/// Cut `s` to `max` characters (ending in "…").
fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn names(items: &Value) -> Vec<String> {
    items
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| i.get("name").or_else(|| i.get("id")))
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `name: best use` lines for one topic, the best uses cut so the whole text
/// fits the budget.
fn detail_lines(title: &str, items: &Value, best_use: impl Fn(&Value) -> String) -> String {
    let rows: Vec<(String, String)> = items
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| {
                    let name = i
                        .get("name")
                        .or_else(|| i.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string();
                    (name, best_use(i))
                })
                .collect()
        })
        .unwrap_or_default();
    // "title\nname: use\nname: use…": what is not best use is fixed.
    let fixed: usize = title.chars().count()
        + 1
        + rows
            .iter()
            .map(|(n, _)| n.chars().count() + 2)
            .sum::<usize>()
        + rows.len().saturating_sub(1);
    let room = TEXT_BUDGET.saturating_sub(fixed + 8);
    let each = (room / rows.len().max(1)).max(20);
    let lines: Vec<String> = rows
        .iter()
        .map(|(n, u)| format!("{n}: {}", cap_words(u, each)))
        .collect();
    format!("{title}\n{}", lines.join("\n"))
}

/// Cut `s` to at most `max` characters at a word boundary (ending in "…").
fn cap_words(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    let cut = match head.rfind(' ') {
        Some(i) if i > max / 2 => &head[..i],
        _ => head.as_str(),
    };
    format!("{}…", cut.trim_end_matches([',', '.', ':', ';', ' ']))
}

/// The `list_options` tool.
pub fn list_options(config: &ServerConfig, args: Value) -> ToolOutput {
    let a: ListOptionsArgs = match parse_args(args) {
        Ok(a) => a,
        Err(fix) => return needs_fix(fix),
    };
    let topic = a.topic.as_deref().map(str::trim).unwrap_or("all");
    let data = options_json(config);
    let (structured, text) = match topic {
        "" | "all" => {
            let by_medium = {
                let mut groups: Vec<(String, Vec<String>)> = Vec::new();
                for f in data["families"].as_array().into_iter().flatten() {
                    let (Some(name), Some(medium)) = (f["name"].as_str(), f["medium"].as_str())
                    else {
                        continue;
                    };
                    match groups.iter_mut().find(|(m, _)| m == medium) {
                        Some((_, v)) => v.push(name.to_string()),
                        None => groups.push((medium.to_string(), vec![name.to_string()])),
                    }
                }
                groups
                    .iter()
                    .map(|(m, v)| format!("{m} ({})", v.join(", ")))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            let beds: Vec<String> = data["music"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| {
                    let id = b["id"].as_str()?;
                    Some(format!("{id} ({})", bed_labels(b).join("/")))
                })
                .collect();
            let mut text = format!(
                "looks: {}\nfamilies: {by_medium}\nmusic: {}\ntones: {}\nTopics looks, families, music or tones give each best use.",
                names(&data["looks"]).join(", "),
                beds.join(", "),
                names(&data["tones"]).join(", "),
            );
            if text.chars().count() > TEXT_BUDGET {
                // Names only for the families, as a last resort.
                text = format!(
                    "looks: {}\nfamilies: {}\nmusic: {}\ntones: {}\nTopics looks, families, music or tones give each best use.",
                    names(&data["looks"]).join(", "),
                    names(&data["families"]).join(", "),
                    names(&data["music"]).join(", "),
                    names(&data["tones"]).join(", "),
                );
            }
            (data.clone(), text)
        }
        "looks" => (
            json!({ "looks": data["looks"] }),
            detail_lines("looks (options.art):", &data["looks"], |i| {
                i["best_use"].as_str().unwrap_or("").to_string()
            }),
        ),
        "tones" => (
            json!({ "tones": data["tones"] }),
            detail_lines("tones (style):", &data["tones"], |i| {
                i["best_use"].as_str().unwrap_or("").to_string()
            }),
        ),
        "families" => (
            json!({ "families": data["families"] }),
            detail_lines("families (options.families):", &data["families"], |i| {
                format!(
                    "{}, {} pictures",
                    i["medium"].as_str().unwrap_or("mixed"),
                    i["assets"]
                )
            }),
        ),
        "music" => (
            json!({ "music": data["music"] }),
            detail_lines("music beds (options.music):", &data["music"], |i| {
                format!(
                    "{}{}{}",
                    bed_labels(i).join(", "),
                    match i["energy"].as_u64() {
                        Some(e) => format!(" · energy {e}"),
                        None => String::new(),
                    },
                    match i["bpm"].as_f64() {
                        Some(b) => format!(" · {b:.0} bpm"),
                        None => String::new(),
                    }
                )
            }),
        ),
        other => {
            return needs_fix(
                Fix::new(None, "topic", format!("unknown topic '{other}'"))
                    .options(["looks", "families", "music"])
                    .otherwise("tones, or all"),
            )
        }
    };
    let mut structured = structured;
    structured["status"] = json!("done");
    ToolOutput {
        structured,
        text: cap(&text, TEXT_BUDGET),
        images: Vec::new(),
        links: Vec::new(),
        is_error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> usize {
        s.split_whitespace().count()
    }

    #[test]
    fn every_look_and_tone_has_a_short_best_use() {
        for l in Look::ALL {
            let u = look_best_use(l);
            assert!(!u.is_empty() && words(u) <= 15, "{}: {u}", l.name());
        }
        for t in crate::schema::tones() {
            let u = tone_best_use(&t).unwrap_or_else(|| panic!("no best use for tone {t}"));
            assert!(words(u) <= 15, "{t}: {u}");
        }
    }

    #[test]
    fn mediums_come_from_the_family_name() {
        for (family, medium) in [
            ("clay_props_3d", "clay 3D"),
            ("clay_concepts_3d", "clay 3D"),
            ("halftone_retro_objects", "halftone print"),
            ("people_everyday", "photo cutout people"),
            ("people_work", "photo cutout people"),
            ("sketch_icons", "line sketch"),
            ("classical_greyscale", "classical engraving"),
            ("editorial_cutout", "editorial photo cutout"),
            ("woodcut_kitchen", "woodcut"),
            ("retro_cars_a", "retro illustration"),
            ("aikakirja_ornaments", "ornaments"),
            ("something_new", "mixed"),
        ] {
            assert_eq!(medium_of(family), medium, "{family}");
        }
    }
}
