//! Generator-facing prompt specs and the asset cache index (0.5).
//! See docs/GENERATED_ASSET_PROTOCOL.md.
//!
//! `prompt_specs(&AssetPlan)` turns semantic requests into deterministic,
//! vendor-neutral `AssetPromptSpec`s, one per depicted entity (dedup by
//! continuity key + art direction). Weak models never write these.

use crate::assets::{
    AssetCacheIndex, AssetPlan, AssetPromptSet, AssetPromptSpec, AssetRequest, AssetRole,
    AssetSource, Background, CacheConflict, CacheEntry, CacheInsert, Framing, NegativeSpace,
    Presentation, Priority, PromptComposition, PromptOutput, ASSET_PROMPTS_VERSION,
};

/// Derive the generator requests for a plan (rules: docs/GENERATED_ASSET_PROTOCOL.md).
pub fn prompt_specs(plan: &AssetPlan) -> AssetPromptSet {
    // Rules 1-2: generated requests only, grouped by
    // (continuity key, framing, presentation, background), in plan order.
    let mut groups: Vec<Group<'_>> = Vec::new();
    for req in plan
        .requests
        .iter()
        .filter(|r| r.source == AssetSource::GeneratedImage)
    {
        let key = req
            .continuity_key
            .clone()
            .unwrap_or_else(|| continuity_key(&req.subject));
        let framing = framing_for(req.role);
        let found = groups.iter_mut().find(|g| {
            g.key == key
                && g.framing == framing
                && g.first.presentation == req.presentation
                && g.first.background == req.background
        });
        match found {
            Some(g) => g.requests.push(req),
            None => groups.push(Group {
                key,
                framing,
                first: req,
                requests: vec![req],
            }),
        }
    }
    AssetPromptSet {
        version: ASSET_PROMPTS_VERSION.to_string(),
        specs: groups.iter().map(|g| build_spec(plan, g)).collect(),
    }
}

struct Group<'a> {
    key: String,
    framing: Framing,
    first: &'a AssetRequest,
    requests: Vec<&'a AssetRequest>,
}

fn build_spec(plan: &AssetPlan, g: &Group<'_>) -> AssetPromptSpec {
    let first = g.first;
    let framing = g.framing;
    let cutout = first.presentation == Presentation::IsolatedCutout;
    let flat = first.presentation == Presentation::FlatArtifact;
    // Rule 4: negative space only for non-cutouts.
    let negative_space = if cutout || flat {
        NegativeSpace::None
    } else {
        first.negative_space
    };
    let person = matches!(
        framing,
        Framing::FullFigure | Framing::HalfFigure | Framing::HeadAndShoulders
    );
    // Rule 5.
    let composition = PromptComposition {
        framing,
        negative_space,
        subject_whole: (cutout || flat) && framing != Framing::HalfFigure,
        head_inside_frame: person,
    };
    // Rule 7.
    let mut avoid: Vec<String> = [
        "embedded text",
        "letters or numbers",
        "logos",
        "watermark",
        "signature",
        "border or frame",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if cutout {
        avoid.push("background scenery".into());
        avoid.push("cast shadow on the ground".into());
    }
    if composition.head_inside_frame {
        avoid.push("cropped head".into());
    }
    if person {
        avoid.push("extra fingers".into());
        avoid.push("distorted hands".into());
    }
    // Rule 6 (the file stem is filled in once the fingerprint exists).
    let output = PromptOutput {
        file_stem: String::new(),
        alpha_required: first.background == Background::Transparent,
        min_short_side: match first.presentation {
            Presentation::IsolatedCutout => 1024,
            Presentation::FullFrame | Presentation::BackgroundPlate => 1080,
            Presentation::FlatArtifact => 900,
        },
        aspect: aspect_for(framing).to_string(),
    };
    let priority = if g.requests.iter().any(|r| r.priority == Priority::Required) {
        Priority::Required
    } else {
        Priority::Optional
    };
    let mut spec = AssetPromptSpec {
        id: first.id.clone(),
        fingerprint: String::new(),
        continuity_key: g.key.clone(),
        serves: g.requests.iter().map(|r| r.id.clone()).collect(),
        role: first.role,
        priority,
        subject: first.subject.clone(),
        context: first.context.clone().unwrap_or_default(),
        presentation: first.presentation,
        background: first.background,
        style: plan.style.clone(),
        composition,
        avoid,
        output,
        prompt: String::new(),
    };
    // Rule 8 (the prompt is not fingerprinted), then rule 9.
    spec.prompt = build_prompt(&spec);
    spec.fingerprint = spec.compute_fingerprint();
    let hash = spec
        .fingerprint
        .strip_prefix("fp1-")
        .unwrap_or(&spec.fingerprint);
    let short: String = hash.chars().take(10).collect();
    spec.output.file_stem = format!("{}-{short}", spec.continuity_key);
    spec
}

/// Rule 3.
fn framing_for(role: AssetRole) -> Framing {
    match role {
        AssetRole::HeroSubject | AssetRole::ForegroundOccluder => Framing::HalfFigure,
        AssetRole::Portrait => Framing::HeadAndShoulders,
        AssetRole::HeroObject | AssetRole::SupportingObject | AssetRole::TransitionObject => {
            Framing::Object
        }
        AssetRole::EvidenceImage => Framing::Artifact,
        AssetRole::Environment => Framing::Scene,
    }
}

fn aspect_for(framing: Framing) -> &'static str {
    match framing {
        Framing::HalfFigure => "3:4",
        Framing::FullFigure => "2:3",
        Framing::HeadAndShoulders => "4:5",
        Framing::Object => "1:1",
        Framing::Artifact | Framing::Scene => "3:4",
    }
}

fn build_prompt(spec: &AssetPromptSpec) -> String {
    let mut parts: Vec<String> = vec![sentence(&spec.subject)];
    if !spec.context.trim().is_empty() {
        parts.push(format!("Context: {}", spec.context.trim()));
    }
    parts.push(
        match spec.presentation {
            Presentation::IsolatedCutout => {
                "Isolated photographic cutout on a fully transparent background."
            }
            Presentation::FullFrame => "Full-frame photograph.",
            Presentation::FlatArtifact => "Flat document-like artifact, photographed straight on.",
            Presentation::BackgroundPlate => "Soft environmental background plate.",
        }
        .to_string(),
    );
    parts.push(
        match spec.composition.framing {
            Framing::FullFigure => {
                "Full figure, head to feet, whole subject inside the frame with a margin."
            }
            Framing::HalfFigure => {
                "Half figure, head to waist, head fully inside the frame with headroom."
            }
            Framing::HeadAndShoulders => {
                "Head and shoulders, head fully inside the frame with headroom."
            }
            Framing::Object => "A single object, shown whole with a margin around it.",
            Framing::Scene => "A wide scene with no single dominant subject.",
            Framing::Artifact => "The whole artifact, flat and straight on, edge to edge.",
        }
        .to_string(),
    );
    let side = match spec.composition.negative_space {
        NegativeSpace::None => None,
        NegativeSpace::Left => Some("left"),
        NegativeSpace::Right => Some("right"),
        NegativeSpace::Top => Some("top"),
        NegativeSpace::Bottom => Some("bottom"),
    };
    if let Some(side) = side {
        parts.push(format!("Leave empty space on the {side}."));
    }
    let st = &spec.style;
    parts.push(format!(
        "Style: {}; {}; {}; {}; {}; {}; {}.",
        st.medium,
        st.realism,
        st.lighting,
        st.contrast,
        st.palette_tendency,
        st.edge_treatment,
        st.camera_feel
    ));
    parts.push(format!("Avoid: {}.", spec.avoid.join(", ")));
    parts.join(" ")
}

/// `"{Subject}."`: first letter capitalized, exactly one terminal stop.
fn sentence(text: &str) -> String {
    let t = text.trim();
    let mut chars = t.chars();
    let cap: String = match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    if cap.ends_with(['.', '!', '?']) {
        cap
    } else {
        format!("{cap}.")
    }
}

/// Same rule as the planner: first four alphanumeric words, lowercased, `_`-joined.
fn continuity_key(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "subject".to_string()
    } else {
        words.join("_")
    }
}

pub(crate) fn cache_lookup<'a>(
    index: &'a AssetCacheIndex,
    fingerprint: &str,
) -> Option<&'a CacheEntry> {
    index.entries.iter().find(|e| e.fingerprint == fingerprint)
}

pub(crate) fn cache_insert(
    index: &mut AssetCacheIndex,
    entry: CacheEntry,
    replace: bool,
) -> Result<CacheInsert, CacheConflict> {
    match index
        .entries
        .iter()
        .position(|e| e.fingerprint == entry.fingerprint)
    {
        Some(i) => {
            let cached = index.entries[i].content_hash.clone();
            if cached == entry.content_hash {
                return Ok(CacheInsert::Unchanged);
            }
            if !replace {
                return Err(CacheConflict::ContentChanged {
                    fingerprint: entry.fingerprint,
                    cached,
                    incoming: entry.content_hash,
                });
            }
            index.entries[i] = entry;
            Ok(CacheInsert::Replaced)
        }
        None => {
            index.entries.push(entry);
            index
                .entries
                .sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));
            Ok(CacheInsert::Added)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuity_key_takes_four_words() {
        assert_eq!(
            continuity_key("Office worker, at a desk late"),
            "office_worker_at_a"
        );
        assert_eq!(continuity_key("!!!"), "subject");
    }

    #[test]
    fn sentence_adds_one_stop() {
        assert_eq!(sentence("  office worker"), "Office worker.");
        assert_eq!(sentence("Done."), "Done.");
    }

    #[test]
    fn framing_and_aspect_by_role() {
        assert_eq!(
            framing_for(AssetRole::ForegroundOccluder),
            Framing::HalfFigure
        );
        assert_eq!(framing_for(AssetRole::Portrait), Framing::HeadAndShoulders);
        assert_eq!(framing_for(AssetRole::TransitionObject), Framing::Object);
        assert_eq!(framing_for(AssetRole::EvidenceImage), Framing::Artifact);
        assert_eq!(framing_for(AssetRole::Environment), Framing::Scene);
        assert_eq!(aspect_for(Framing::HeadAndShoulders), "4:5");
        assert_eq!(aspect_for(Framing::Object), "1:1");
    }
}
