//! Typed UI flows: a closed vocabulary of product flows (`payment`,
//! `signup`, `upload`, `share`, `toggle_setting`) with structured fields.
//! States, their order and every system string are derived from the fields,
//! so the point of view (the device belongs to `actor`), cause → effect and
//! the single success affordance are right by construction:
//!
//! - success resolves in place (button → tick → an in-app receipt); the app
//!   context (title, counterparty, amount) persists through it;
//! - a notification, if any, comes later and carries new information from
//!   the counterparty (`reply`), never a repeat of the result.
//!
//! Free-text states stay allowed; [`copy_problems`] validates them with the
//! same rules (the ledger's `pov_mismatch`, `duplicate_confirmation`,
//! `empty_app_state`, `orphan_notification`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::device::{Device, UiIcon, UiOp, UiShape, UiState, UiStep};
use crate::program::Swatch;

/// A product flow on one actor's device; states and copy are derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiFlow {
    pub kind: FlowKind,
    /// The app's name (its icon opens the flow), at most 12 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Whose device this is (a first name, or "You" = unnamed; signup
    /// greets a name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// The other person (payment, share): a first name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterparty: Option<String>,
    /// Payment: the amount with its currency, e.g. "€24.00".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    /// The thing acted on: payment's source account, signup's app or plan,
    /// upload's and share's file, toggle's setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// A short note (payment's memo, upload's destination).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Later news from the counterparty as their verb phrase, e.g. "liked
    /// your payment"; shown as a notification after the result. Never a
    /// repeat of what the screen shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    /// Send `amount` to `counterparty`: app icon → payment sheet → hold to
    /// send → tick → receipt (same sheet).
    Payment,
    /// Create an account (`item` = the app or plan): form → welcome.
    Signup,
    /// Upload `item`: card → hold (progress) → tick → uploaded card.
    Upload,
    /// Share `item` with `counterparty`: share sheet → tap them → shared.
    Share,
    /// Turn on the `item` setting: a settings card whose switch flips.
    ToggleSetting,
}

impl UiFlow {
    /// Field problems as `(path suffix, message)`.
    pub fn check(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let need =
            |out: &mut Vec<(String, String)>, v: &Option<String>, k: &str, max: usize| match v {
                None => out.push((format!(".flow.{k}"), format!("{:?} needs {k}", self.kind))),
                Some(s) if s.trim().is_empty() || s.chars().count() > max => {
                    out.push((format!(".flow.{k}"), format!("1..={max} characters")))
                }
                _ => {}
            };
        match self.kind {
            FlowKind::Payment => {
                need(&mut out, &self.counterparty, "counterparty", 18);
                need(&mut out, &self.amount, "amount", 12);
            }
            FlowKind::Signup => {}
            FlowKind::Upload => need(&mut out, &self.item, "item", 24),
            FlowKind::Share => {
                need(&mut out, &self.item, "item", 24);
                need(&mut out, &self.counterparty, "counterparty", 18);
            }
            FlowKind::ToggleSetting => need(&mut out, &self.item, "item", 24),
        }
        for (k, v, max) in [
            ("app", &self.app, 12),
            ("actor", &self.actor, 18),
            ("counterparty", &self.counterparty, 18),
            ("amount", &self.amount, 12),
            ("item", &self.item, 24),
            ("note", &self.note, 20),
            ("reply", &self.reply, 24),
        ] {
            if let Some(s) = v {
                if s.chars().count() > max {
                    out.push((format!(".flow.{k}"), format!("at most {max} characters")));
                }
                if let Some(c) = crate::device::banned_glyph(s) {
                    out.push((format!(".flow.{k}"), format!("'{c}' is not UI type")));
                }
            }
        }
        if self.reply.is_some() && self.counterparty.is_none() {
            out.push((".flow.reply".into(), "a reply needs a counterparty".into()));
        }
        out
    }

    /// The flow's states and steps (seconds from the component start).
    pub fn expand(&self) -> (Vec<UiState>, Vec<UiStep>) {
        let cp = self.counterparty.clone().unwrap_or_default();
        let item = self.item.clone().unwrap_or_default();
        let st = |shape: UiShape, title: String| UiState {
            shape,
            title,
            detail: None,
            items: Vec::new(),
            amount: None,
            cta: None,
            icon: None,
            fill: None,
            switches: Vec::new(),
        };
        let step = |at: f64, op: UiOp| UiStep {
            at,
            beat: None,
            bar: None,
            op,
        };
        let just_now = |note: &Option<String>| match note {
            Some(n) => format!("{n} · Just now"),
            None => "Just now".to_string(),
        };
        let mut states = Vec::new();
        let mut steps = Vec::new();
        // The result state and when it lands (for a later reply).
        let result_at: f64;
        let result: usize;
        match self.kind {
            FlowKind::Payment => {
                let app = self.app.clone().unwrap_or_else(|| "Pay".into());
                let source = format!(
                    "From {}",
                    self.item.clone().unwrap_or("Main account".into())
                );
                states.push(UiState {
                    fill: Some(Swatch::Accent),
                    ..st(UiShape::Icon, app)
                });
                states.push(UiState {
                    detail: Some(self.note.clone().unwrap_or_else(|| "Instant".into())),
                    amount: self.amount.clone(),
                    items: vec![source.clone()],
                    cta: Some("Hold to send".into()),
                    ..st(UiShape::Sheet, format!("Send to {cp}"))
                });
                // The receipt: the same sheet, transformed.
                states.push(UiState {
                    detail: Some(just_now(&self.note)),
                    amount: self.amount.clone(),
                    items: vec![source],
                    cta: Some("Done".into()),
                    ..st(UiShape::Sheet, format!("Sent to {cp}"))
                });
                steps.push(step(
                    1.9,
                    UiOp::Click {
                        then: Some(1),
                        row: None,
                    },
                ));
                steps.push(step(
                    3.4,
                    UiOp::Hold {
                        secs: 0.9,
                        then: Some(2),
                    },
                ));
                result = 2;
                result_at = 3.4 + 0.9 + 1.1;
            }
            FlowKind::Signup => {
                let what = self
                    .item
                    .clone()
                    .or(self.app.clone())
                    .unwrap_or_else(|| "the app".into());
                states.push(UiState {
                    detail: Some(format!("Join {what}")),
                    items: vec!["Email".into()],
                    cta: Some("Continue".into()),
                    ..st(UiShape::Card, "Create account".into())
                });
                states.push(UiState {
                    detail: Some("Your account is ready".into()),
                    cta: Some("Get started".into()),
                    ..st(
                        UiShape::Card,
                        match self.actor.as_deref() {
                            Some(a) if !a.eq_ignore_ascii_case("you") => format!("Welcome, {a}"),
                            _ => "Welcome".into(),
                        },
                    )
                });
                steps.push(step(
                    2.2,
                    UiOp::Click {
                        then: Some(1),
                        row: None,
                    },
                ));
                result = 1;
                result_at = 2.2;
            }
            FlowKind::Upload => {
                states.push(UiState {
                    detail: Some(item.clone()),
                    items: self.note.iter().map(|n| format!("To {n}")).collect(),
                    cta: Some("Hold to upload".into()),
                    ..st(UiShape::Card, "Upload".into())
                });
                states.push(UiState {
                    detail: Some(just_now(&None)),
                    items: self.note.iter().map(|n| format!("In {n}")).collect(),
                    cta: Some("Share".into()),
                    ..st(UiShape::Card, format!("Uploaded {item}"))
                });
                steps.push(step(
                    2.0,
                    UiOp::Hold {
                        secs: 1.2,
                        then: Some(1),
                    },
                ));
                result = 1;
                result_at = 2.0 + 1.2 + 1.1;
            }
            FlowKind::Share => {
                states.push(UiState {
                    detail: Some(item.clone()),
                    items: vec![cp.clone(), "Copy link".into(), "Mail".into()],
                    ..st(UiShape::Sheet, "Share".into())
                });
                states.push(UiState {
                    detail: Some(item.clone()),
                    cta: Some("Done".into()),
                    ..st(UiShape::Sheet, format!("Shared with {cp}"))
                });
                steps.push(step(
                    2.4,
                    UiOp::Click {
                        then: Some(1),
                        row: Some(0),
                    },
                ));
                result = 1;
                result_at = 2.4;
            }
            FlowKind::ToggleSetting => {
                let app = self.app.clone().unwrap_or_else(|| "Settings".into());
                states.push(UiState {
                    items: vec![item.clone()],
                    switches: vec![false],
                    ..st(UiShape::Card, app)
                });
                steps.push(step(
                    1.8,
                    UiOp::Click {
                        then: None,
                        row: Some(0),
                    },
                ));
                result = 0;
                result_at = 1.8;
            }
        }
        if let Some(r) = &self.reply {
            // News from them, after the result has been read.
            let k = states.len();
            states.push(UiState {
                detail: Some("Just now".into()),
                ..st(UiShape::Toast, format!("{cp} {r}"))
            });
            let at = result_at + crate::device::read_time(&states[result]) + 0.2;
            steps.push(step(
                at,
                UiOp::Toast {
                    state: k,
                    hold: None,
                },
            ));
        }
        (states, steps)
    }
}

// ---------------------------------------------------------------- copy rules

/// Verbs that confirm a finished action.
const SUCCESS: [&str; 12] = [
    "sent",
    "paid",
    "received",
    "uploaded",
    "shared",
    "saved",
    "done",
    "complete",
    "completed",
    "confirmed",
    "success",
    "ready",
];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '€' && c != '$' && c != '£' && c != '.')
        .map(|w| w.trim_matches('.').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// A state's shown text, CTA labels excluded (buttons are actions, not
/// statements).
pub fn state_text(s: &UiState) -> Vec<String> {
    let mut v = vec![s.title.clone()];
    v.extend(s.detail.clone());
    v.extend(s.amount.clone());
    v.extend(s.items.iter().cloned());
    v
}

/// Whether a state states success (a success verb, or a tick icon).
pub fn is_success(s: &UiState) -> bool {
    s.icon == Some(UiIcon::Check)
        || [&s.title]
            .into_iter()
            .chain(s.detail.iter())
            .any(|t| words(t).iter().any(|w| SUCCESS.contains(&w.as_str())))
}

/// The people the actor acts on: names after "to" / "with" / "pay" in
/// titles.
pub fn counterparties(states: &[UiState]) -> Vec<String> {
    let mut out = Vec::new();
    for s in states {
        let w: Vec<&str> = s.title.split_whitespace().collect();
        for i in 0..w.len().saturating_sub(1) {
            let lead = w[i].to_lowercase();
            if ["to", "with", "pay", "paid"].contains(&lead.as_str())
                && w[i + 1].chars().next().is_some_and(char::is_uppercase)
            {
                let name = w[i + 1].trim_matches(|c: char| !c.is_alphanumeric());
                if !out.iter().any(|n: &String| n == name) {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

/// Significant tokens: amounts, names, words of 4+ letters.
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in s.split_whitespace() {
        let t =
            raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '€' && c != '$' && c != '£');
        if t.chars().any(|c| c.is_ascii_digit()) || t.chars().count() >= 4 {
            out.push(t.to_lowercase());
        }
    }
    out
}

/// Copy problems of a device's states and steps, as `(ledger id, message)`.
pub fn copy_problems(
    device: Device,
    states: &[UiState],
    steps: &[UiStep],
    actor: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let names = counterparties(states);
    // POV: the device belongs to the actor.
    for (k, s) in states.iter().enumerate() {
        for t in state_text(s) {
            let w: Vec<&str> = t.split_whitespace().collect();
            for i in 0..w.len().saturating_sub(1) {
                let who = w[i].trim_matches(|c: char| !c.is_alphanumeric());
                let verb = w[i + 1].to_lowercase();
                if names.iter().any(|n| n == who)
                    && ["received", "got", "receives", "gets"].contains(&verb.as_str())
                {
                    out.push((
                        "pov_mismatch",
                        format!(
                            "state {k}: '{who} {verb}' is the other person's side; on the sender's device say what was done ('sent to {who}')"
                        ),
                    ));
                }
                if let Some(a) = actor {
                    let greeting = i > 0
                        && ["welcome,", "hi", "hi,", "hello,"]
                            .contains(&w[i - 1].to_lowercase().as_str());
                    if who == a && !greeting {
                        out.push((
                            "pov_mismatch",
                            format!("state {k}: names the device's owner '{a}' as a third party"),
                        ));
                    }
                }
            }
        }
    }
    // Timeline: what is on screen when.
    let mut shown = 0usize;
    let mut changes: Vec<(f64, usize)> = vec![(0.0, 0)];
    for st in steps {
        match st.op {
            UiOp::ToState { state } => changes.push((st.at, state)),
            UiOp::Click { then: Some(k), .. } => changes.push((st.at, k)),
            UiOp::Hold {
                secs,
                then: Some(k),
            } => changes.push((st.at + secs + 1.1, k)),
            _ => {}
        }
    }
    changes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let state_at = |t: f64| {
        changes
            .iter()
            .rev()
            .find(|c| c.0 <= t + 1e-9)
            .map_or(0, |c| c.1)
    };
    for st in steps {
        if let UiOp::Toast { state, .. } = st.op {
            let Some(toast) = states.get(state) else {
                continue;
            };
            let under = &states[state_at(st.at)];
            // One success affordance at a time.
            if is_success(toast) && is_success(under) {
                out.push((
                    "duplicate_confirmation",
                    format!(
                        "toast '{}' confirms again what '{}' already shows",
                        toast.title, under.title
                    ),
                ));
            }
            // A notification adds news.
            let seen: Vec<String> = state_text(under).iter().flat_map(|t| tokens(t)).collect();
            let rep: Vec<String> = state_text(toast)
                .iter()
                .flat_map(|t| tokens(t))
                .filter(|t| seen.contains(t) && !names.contains(&capital(t)))
                .collect();
            let amount = rep.iter().any(|t| t.chars().any(|c| c.is_ascii_digit()));
            if amount || rep.len() >= 2 {
                out.push((
                    "orphan_notification",
                    format!(
                        "toast '{}' repeats {:?} already on screen",
                        toast.title, rep
                    ),
                ));
            }
        }
    }
    // A screen keeps its app: after the first state, phones and laptops show
    // a card or a sheet, not one floating element.
    if matches!(device, Device::Phone | Device::Laptop) {
        for &(t, k) in changes.iter().skip(1) {
            if k != shown && !matches!(states[k].shape, UiShape::Card | UiShape::Sheet) {
                out.push((
                    "empty_app_state",
                    format!(
                        "state {k} ('{}') at {t:.2}s leaves the screen with one floating element; keep the app (a card or sheet that changes)",
                        states[k].title
                    ),
                ));
            }
            shown = k;
        }
    }
    out
}

fn capital(t: &str) -> String {
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(kind: FlowKind) -> UiFlow {
        UiFlow {
            kind,
            app: None,
            actor: Some("Sam".into()),
            counterparty: Some("Mira".into()),
            amount: Some("€24.00".into()),
            item: Some("Q3 deck.pdf".into()),
            note: Some("Dinner, Friday".into()),
            reply: Some("liked your payment".into()),
        }
    }

    #[test]
    fn derived_flows_are_clean() {
        for k in [
            FlowKind::Payment,
            FlowKind::Signup,
            FlowKind::Upload,
            FlowKind::Share,
            FlowKind::ToggleSetting,
        ] {
            let f = flow(k);
            let (states, steps) = f.expand();
            let p = copy_problems(Device::Phone, &states, &steps, f.actor.as_deref());
            assert!(p.is_empty(), "{k:?}: {p:?}");
        }
    }

    #[test]
    fn catches_the_old_ending() {
        let st = |shape, title: &str, detail: Option<&str>, icon| UiState {
            shape,
            title: title.into(),
            detail: detail.map(Into::into),
            items: vec![],
            amount: None,
            cta: None,
            icon,
            fill: None,
            switches: vec![],
        };
        let states = vec![
            st(UiShape::Sheet, "Send to Mira", None, None),
            st(UiShape::Pill, "Sent", None, Some(UiIcon::Check)),
            st(
                UiShape::Toast,
                "Mira received €24.00",
                Some("Just now"),
                Some(UiIcon::Check),
            ),
        ];
        let steps = vec![
            UiStep {
                at: 2.0,
                beat: None,
                bar: None,
                op: UiOp::ToState { state: 1 },
            },
            UiStep {
                at: 4.0,
                beat: None,
                bar: None,
                op: UiOp::Toast {
                    state: 2,
                    hold: None,
                },
            },
        ];
        let ids: Vec<&str> = copy_problems(Device::Phone, &states, &steps, None)
            .iter()
            .map(|p| p.0)
            .collect();
        for id in ["pov_mismatch", "duplicate_confirmation", "empty_app_state"] {
            assert!(ids.contains(&id), "{id} in {ids:?}");
        }
    }
}
