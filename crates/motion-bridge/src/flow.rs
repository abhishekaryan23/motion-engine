//! Product / UI flows: a brief that describes one ("Maya pays Sam $40 for
//! dinner", "turn on dark mode") picks a typed flow recipe and fills its
//! structured fields. The bridge never writes UI copy or state order: the
//! `ui_flow` component (v2-ui) derives both from the fields, so POV and
//! cause → effect are correct by construction.

use serde::Serialize;
use serde_json::{json, Value};

/// Flow recipes `ui_flow` knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    Payment,
    Signup,
    Upload,
    Share,
    ToggleSetting,
}

/// The structured fields of one flow. Every name is the brief's own; no copy.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FlowSpec {
    pub flow: FlowKind,
    /// Whose device we watch (the one who acts). Default "You".
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub device: &'static str,
}

const PAY: &[&str] = &[
    "pay",
    "pays",
    "paid",
    "paying",
    "send",
    "sends",
    "sent",
    "transfer",
    "transfers",
    "split",
    "splits",
    "venmo",
    "tip",
    "tips",
];
const SIGNUP: &[&str] = &[
    "signup",
    "sign",
    "signs",
    "register",
    "registers",
    "join",
    "joins",
    "onboard",
    "onboarding",
    "subscribe",
    "subscribes",
];
const UPLOAD: &[&str] = &["upload", "uploads", "uploading", "backup", "backs"];
const SHARE: &[&str] = &["share", "shares", "sharing", "invite", "invites"];
const TOGGLE: &[&str] = &[
    "turn", "turns", "enable", "enables", "disable", "disables", "toggle", "toggles", "switch",
    "switches",
];
const DESKTOP: &[&str] = &["laptop", "desktop", "mac", "web", "browser", "dashboard"];
const NOT_NAMES: &[&str] = &[
    "I",
    "You",
    "We",
    "They",
    "The",
    "A",
    "An",
    "On",
    "Off",
    "In",
    "My",
    "Your",
    "Our",
    "Their",
    "Make",
    "Show",
    "Then",
    "And",
    "With",
    "To",
    "For",
    "From",
    "Dark",
    "Light",
    "Mode",
    "Tap",
    "Pay",
    "Send",
    "Hold",
    "Click",
    "Open",
    "Wallet",
    "Mobile",
    "Phone",
    "App",
    "Confirmed",
    "Done",
    "Step",
];

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || ",;:!?()\"“”".contains(c))
        .map(|w| w.trim_end_matches('.').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn has(words: &[String], set: &[&str]) -> Option<usize> {
    words
        .iter()
        .position(|w| set.contains(&w.to_lowercase().as_str()))
}

/// A capitalised word that is not sentence furniture: a person or app name.
fn is_name(w: &str) -> bool {
    w.chars().next().is_some_and(char::is_uppercase)
        && w.chars()
            .all(|c| c.is_alphanumeric() || c == '\'' || c == '-')
        && !NOT_NAMES.contains(&w)
        && w.chars().any(char::is_lowercase)
}

fn money(w: &str) -> bool {
    let sym = w.starts_with(['$', '€', '£', '¥', '₹']) || w.ends_with(['€']);
    sym && w.chars().any(|c| c.is_ascii_digit())
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect::<String>().trim().to_string()
}

/// Detect a flow in one brief line. Order matters: a payment line can say
/// "send", a share line "send a file"; money decides.
pub fn detect(line: &str) -> Option<FlowSpec> {
    let w = tokens(line);
    let device = if has(&w, DESKTOP).is_some() {
        "laptop"
    } else {
        "phone"
    };
    let amount = w.iter().find(|x| money(x)).map(|a| clip(a, 12));
    let verb = |set: &[&str]| has(&w, set);
    // Names before the verb act; names after it receive.
    let split = |at: usize| -> (Option<String>, Option<String>) {
        let before = w[..at]
            .iter()
            .rev()
            .find(|x| is_name(x))
            .map(|x| clip(x, 18));
        let after = w[at + 1..]
            .iter()
            .find(|x| is_name(x) && Some(x.as_str()) != before.as_deref())
            .map(|x| clip(x, 18));
        (before, after)
    };
    let actor_or_you = |a: Option<String>| a.unwrap_or_else(|| "You".into());
    // What follows the verb, without names, money and particles.
    let rest_after = |at: usize, skip: &[&str]| -> Option<String> {
        let s: Vec<&str> = w[at + 1..]
            .iter()
            .filter(|x| !money(x) && !is_name(x))
            .map(String::as_str)
            .filter(|x| !skip.contains(&x.to_lowercase().as_str()))
            .collect();
        (!s.is_empty()).then(|| clip(&s.join(" "), 24))
    };
    if let (Some(at), Some(_)) = (verb(PAY), &amount) {
        let (a, b) = split(at);
        return Some(FlowSpec {
            flow: FlowKind::Payment,
            actor: actor_or_you(a),
            counterparty: b,
            amount,
            item: None,
            device,
        });
    }
    if let Some(at) = verb(TOGGLE) {
        const P: &[&str] = &[
            "on", "off", "the", "a", "an", "my", "your", "setting", "in", "settings",
        ];
        return Some(FlowSpec {
            flow: FlowKind::ToggleSetting,
            actor: actor_or_you(split(at).0),
            counterparty: None,
            amount: None,
            item: rest_after(at, P),
            device,
        });
    }
    if let Some(at) = verb(UPLOAD) {
        const P: &[&str] = &[
            "a", "an", "the", "my", "your", "to", "up", "their", "his", "her",
        ];
        return Some(FlowSpec {
            flow: FlowKind::Upload,
            actor: actor_or_you(split(at).0),
            counterparty: None,
            amount: None,
            item: rest_after(at, P),
            device,
        });
    }
    if let Some(at) = verb(SHARE) {
        let (a, b) = split(at);
        const P: &[&str] = &[
            "a", "an", "the", "my", "your", "with", "to", "their", "his", "her",
        ];
        return Some(FlowSpec {
            flow: FlowKind::Share,
            actor: actor_or_you(a),
            counterparty: b,
            amount: None,
            item: rest_after(at, P),
            device,
        });
    }
    if let Some(at) = verb(SIGNUP) {
        // "signs up for Acme": the app or plan is the first name after.
        let item = w[at + 1..].iter().find(|x| is_name(x)).map(|x| clip(x, 24));
        let actor = w[..at]
            .iter()
            .rev()
            .find(|x| is_name(x))
            .map(|x| clip(x, 18));
        let up = w.get(at + 1).is_some_and(|x| x.eq_ignore_ascii_case("up"));
        let sign_only = w[at].eq_ignore_ascii_case("sign") || w[at].eq_ignore_ascii_case("signs");
        if !sign_only || up {
            return Some(FlowSpec {
                flow: FlowKind::Signup,
                actor: actor_or_you(actor),
                counterparty: None,
                amount: None,
                item,
                device,
            });
        }
    }
    None
}

/// The first flow the brief describes (quoted text is copy, not a flow).
pub fn detect_brief(brief: &str) -> Option<FlowSpec> {
    brief.lines().find_map(|l| {
        let unquoted = l
            .replace(['“', '”'], "\"")
            .split('"')
            .step_by(2)
            .collect::<Vec<_>>()
            .join(" ");
        detect(&unquoted)
    })
}

/// The core's typed flow (v2-ui `device_frame.flow`) for this spec, or
/// `None` when a required field is missing (the core decides the copy).
pub fn ui_flow(f: &FlowSpec) -> Option<motion_program::flow::UiFlow> {
    use motion_program::flow::{FlowKind as K, UiFlow};
    let name = |s: &Option<String>| s.as_ref().map(|x| clip(x, 18));
    let actor = Some(f.actor.clone())
        .filter(|a| a != "You")
        .map(|a| clip(&a, 12));
    let flow = match f.flow {
        FlowKind::Payment => UiFlow {
            kind: K::Payment,
            app: None,
            actor,
            counterparty: name(&f.counterparty).or_else(|| Some("Alex".into())),
            amount: f.amount.as_ref().map(|a| clip(a, 12)),
            item: None,
            note: None,
            reply: None,
        },
        // Signup's `item` is the app or plan signed up for.
        FlowKind::Signup => UiFlow {
            kind: K::Signup,
            app: None,
            actor,
            counterparty: None,
            amount: None,
            item: f.item.as_ref().map(|a| clip(a, 24)),
            note: None,
            reply: None,
        },
        FlowKind::Upload => UiFlow {
            kind: K::Upload,
            app: None,
            actor,
            counterparty: None,
            amount: None,
            item: f.item.as_ref().map(|a| clip(a, 24)),
            note: None,
            reply: None,
        },
        FlowKind::Share => UiFlow {
            kind: K::Share,
            app: None,
            actor,
            counterparty: name(&f.counterparty),
            amount: None,
            item: f.item.as_ref().map(|a| clip(a, 24)),
            note: None,
            reply: None,
        },
        FlowKind::ToggleSetting => UiFlow {
            kind: K::ToggleSetting,
            app: None,
            actor,
            counterparty: None,
            amount: None,
            item: f.item.as_ref().map(|a| clip(a, 24)),
            note: None,
            reply: None,
        },
    };
    flow.check().is_empty().then_some(flow)
}

/// Seconds the derived flow needs: the end of its last step, the result
/// landing (≈ 1.1 s) and the core's read time for it (1.9 s).
pub fn seconds(flow: &motion_program::flow::UiFlow) -> f64 {
    let (_, steps) = flow.expand();
    steps
        .iter()
        .map(|s| {
            let v = serde_json::to_value(s).unwrap_or_default();
            s.at + v["secs"].as_f64().unwrap_or(0.0)
        })
        .fold(0.0, f64::max)
        + 3.1
}

/// The `device_frame` component that plays `flow` from `start_beat`.
pub fn component(
    id: &str,
    flow: &motion_program::flow::UiFlow,
    device: &str,
    start_beat: usize,
    end: f64,
) -> Value {
    json!({
        "id": id, "kind": "device_frame", "device": device, "slot": "lead",
        "flow": flow, "start_beat": start_beat, "end": end,
    })
}

/// Our narration per flow state (state 0 = the first screen), never UI copy
/// and never the brief's words; `@` stands for the amount.
fn narration(kind: FlowKind, state: usize) -> &'static [&'static str] {
    use FlowKind as K;
    const PAY: [&[&str]; 3] = [
        &["ONE", "TAP"],
        &["@", "HOLD", "TO", "SEND"],
        &["SENT", "THAT'S", "IT"],
    ];
    const UP: [&[&str]; 3] = [
        &["PICK", "A", "FILE"],
        &["HOLD", "IT", "GOES", "UP"],
        &["SAVED", "SAFE"],
    ];
    const SHARE: [&[&str]; 3] = [
        &["SHARE", "IN", "ONE", "TAP"],
        &["PICK", "WHO"],
        &["SENT", "THERE"],
    ];
    const TOGGLE: [&[&str]; 2] = [&["ONE", "SWITCH"], &["FLIP", "IT", "ON", "SET"]];
    const SIGNUP: [&[&str]; 2] = [
        &["NEW", "HERE", "ONE", "FORM"],
        &["YOU'RE", "IN", "WELCOME"],
    ];
    let table: &[&[&str]] = match kind {
        K::Payment => &PAY,
        K::Upload => &UP,
        K::Share => &SHARE,
        K::ToggleSetting => &TOGGLE,
        K::Signup => &SIGNUP,
    };
    table[state.min(table.len() - 1)]
}

/// When each derived state starts (seconds from the component start):
/// a click shows its next state at once, a hold after its run and the
/// ≈ 1.1 s tick.
pub fn state_times(flow: &motion_program::flow::UiFlow) -> Vec<(f64, usize)> {
    let (_, steps) = flow.expand();
    let mut out = vec![(0.0, 0)];
    for s in &steps {
        let v = serde_json::to_value(s).unwrap_or_default();
        let Some(next) = v["then"].as_u64() else {
            continue;
        };
        let t = match v["do"].as_str() {
            Some("hold") => s.at + v["secs"].as_f64().unwrap_or(0.0) + 1.1,
            _ => s.at,
        };
        out.push((t, next as usize));
    }
    out
}

/// The beat-cut type column beside the device: one word a beat, narrating
/// the state on screen at that beat (`beat_times` from the bar start).
pub fn column(
    kind: FlowKind,
    flow: &motion_program::flow::UiFlow,
    amount: Option<&str>,
    banned: &[String],
    beat_times: &[f64],
) -> Vec<String> {
    let times = state_times(flow);
    let mut seen = [0usize; 8];
    beat_times
        .iter()
        .map(|t| {
            let state = times
                .iter()
                .filter(|(at, _)| *at <= *t + 1e-6)
                .map(|(_, k)| *k)
                .next_back()
                .unwrap_or(0);
            let words: Vec<String> = narration(kind, state)
                .iter()
                .filter_map(|w| match *w {
                    "@" => amount.map(|a| clip(a, 12)),
                    w => Some(w.to_string()),
                })
                .filter(|w| w.chars().count() < 4 || !banned.contains(&w.to_lowercase()))
                .collect();
            let i = seen[state.min(7)];
            seen[state.min(7)] += 1;
            // Past the phase's words, the last one holds the beat (re-cut).
            words
                .get(i)
                .or(words.last())
                .cloned()
                .unwrap_or_else(|| "·".into())
        })
        .collect()
}
