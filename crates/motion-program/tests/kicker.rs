//! `ledger::is_kicker` (the `eyebrow_kicker` detector's text rule).

use motion_program::ledger::is_kicker;

#[test]
fn kickers_are_flagged() {
    for t in [
        "Last year, in numbers",
        "EST. 2019",
        "Since 1998",
        "2019",
        "Q3 highlights",
    ] {
        assert!(is_kicker(t), "{t}");
    }
}

#[test]
fn content_lines_are_not_kickers() {
    for t in [
        "Recapture the market",
        "Year insights",
        "Q3 2024",
        "2024 EDITION",
        "THE CANON OF PROPORTION",
        "SOVEREIGN OPTICAL TELEMETRY",
        "EDITION",
    ] {
        assert!(!is_kicker(t), "{t}");
    }
}
