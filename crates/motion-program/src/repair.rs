//! Auto-repair: program edits that clear ledger gates.
//!
//! The CLI runs QA → repair → QA at most twice ([`MAX_PASSES`]). A repair
//! says which ledger ids it handles and edits the program; anything still
//! failing after the last pass is reported as `fail: [ids]`, never shipped
//! silently.
//!
//! **Plug-in hook:** other sessions add repairs by implementing [`Repair`]
//! and adding one line to [`repairs`] (the design-system session's palette /
//! calm-zone repairs go there).

use crate::ledger::GateFail;
use crate::program::{ComponentBody, MotionProgram};

pub const MAX_PASSES: usize = 2;

pub trait Repair {
    fn name(&self) -> &'static str;
    /// Ledger ids this repair can clear.
    fn handles(&self, id: &str) -> bool;
    /// Edit the program for these failures; true when anything changed.
    fn apply(&self, p: &mut MotionProgram, fails: &[GateFail]) -> bool;
}

/// Every registered repair, applied in order.
pub fn repairs() -> Vec<Box<dyn Repair>> {
    vec![
        Box::new(DropMeta),
        Box::new(DropEcho),
        Box::new(DropEyebrow),
        Box::new(crate::layout::DesignSystem),
    ]
}

/// Applies every repair that handles one of `fails`; true when the program
/// changed.
pub fn repair(p: &mut MotionProgram, fails: &[GateFail]) -> bool {
    let mut changed = false;
    for r in repairs() {
        let mine: Vec<GateFail> = fails.iter().filter(|f| r.handles(&f.id)).cloned().collect();
        if !mine.is_empty() {
            changed |= r.apply(p, &mine);
        }
    }
    changed
}

fn tokens(t: &str) -> Vec<String> {
    t.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Meta text never renders: HUD corners and lines naming fps/bpm, the
/// engine, the track or the program are removed.
pub struct DropMeta;
impl Repair for DropMeta {
    fn name(&self) -> &'static str {
        "drop_meta"
    }
    fn handles(&self, id: &str) -> bool {
        id == "meta_text"
    }
    fn apply(&self, p: &mut MotionProgram, _: &[GateFail]) -> bool {
        let name: Vec<String> = p.meta.name.split('_').map(str::to_lowercase).collect();
        let track: Vec<String> = p
            .music
            .as_ref()
            .map(|m| m.track.split('_').map(str::to_lowercase).collect())
            .unwrap_or_default();
        let meta = |t: &str| {
            let tk = tokens(t);
            tk.iter()
                .any(|w| matches!(w.as_str(), "fps" | "bpm" | "motionengine"))
                || (name.len() >= 2 && name.iter().all(|w| tk.contains(w)))
                || (track.len() >= 2 && track.iter().all(|w| tk.contains(w)))
        };
        let before = p.components.len();
        p.components.retain(|c| match &c.body {
            ComponentBody::Hud { .. } => false,
            ComponentBody::Title { text, .. } => !meta(text),
            _ => true,
        });
        before != p.components.len()
    }
}

/// Captions that echo the brief or on-screen labels are removed.
pub struct DropEcho;
impl Repair for DropEcho {
    fn name(&self) -> &'static str {
        "drop_echo"
    }
    fn handles(&self, id: &str) -> bool {
        id == "echo_brief" || id == "echo_labels"
    }
    fn apply(&self, p: &mut MotionProgram, fails: &[GateFail]) -> bool {
        let ids: Vec<String> = fails
            .iter()
            .filter_map(|f| {
                f.detail
                    .rsplit_once('(')
                    .map(|(_, r)| r.trim_end_matches(')').to_string())
            })
            .collect();
        let before = p.components.len();
        p.components.retain(|c| !ids.contains(&c.id));
        before != p.components.len()
    }
}

/// Kickers above heroes and template clichés are removed.
pub struct DropEyebrow;
impl Repair for DropEyebrow {
    fn name(&self) -> &'static str {
        "drop_eyebrow"
    }
    fn handles(&self, id: &str) -> bool {
        id == "eyebrow" || id == "eyebrow_kicker"
    }
    fn apply(&self, p: &mut MotionProgram, fails: &[GateFail]) -> bool {
        let owner = |f: &GateFail| {
            f.detail
                .rsplit_once('(')
                .map(|(_, r)| r.trim_end_matches(')').to_string())
        };
        let ids: Vec<String> = fails
            .iter()
            .filter(|f| f.id == "eyebrow")
            .filter_map(owner)
            .collect();
        // A kicker inside a bento or lockup: drop the text, keep the content.
        let kicker_ids: Vec<String> = fails
            .iter()
            .filter(|f| f.id == "eyebrow_kicker")
            .filter_map(owner)
            .collect();
        let mut changed = false;
        for c in p
            .components
            .iter_mut()
            .filter(|c| kicker_ids.contains(&c.id))
        {
            match &mut c.body {
                ComponentBody::Bento(b) => {
                    if b.title.as_deref().is_some_and(crate::ledger::is_kicker) {
                        b.title = None;
                        changed = true;
                    }
                }
                ComponentBody::Lockup(l) => {
                    if l.badge.as_deref().is_some_and(crate::ledger::is_kicker) {
                        l.badge = None;
                        changed = true;
                    }
                    let n = l.specs.len();
                    l.specs.retain(|s| !crate::ledger::is_kicker(s));
                    changed |= n != l.specs.len();
                }
                _ => {}
            }
        }
        let before = p.components.len();
        p.components.retain(|c| !ids.contains(&c.id));
        changed || before != p.components.len()
    }
}
