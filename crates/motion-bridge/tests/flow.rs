use motion_bridge::flow::{detect, detect_brief, FlowKind};

#[test]
fn payment_has_payer_pov_payee_and_amount() {
    let f = detect("Maya pays Sam $40 for dinner").unwrap();
    assert_eq!(f.flow, FlowKind::Payment);
    assert_eq!(f.actor, "Maya", "the payer owns the device we watch");
    assert_eq!(f.counterparty.as_deref(), Some("Sam"));
    assert_eq!(f.amount.as_deref(), Some("$40"));
    assert_eq!(f.device, "phone");
}

#[test]
fn send_money_without_actor_is_you() {
    let f = detect("send €25 to Priya").unwrap();
    assert_eq!((f.flow, f.actor.as_str()), (FlowKind::Payment, "You"));
    assert_eq!(f.counterparty.as_deref(), Some("Priya"));
}

#[test]
fn send_without_money_is_not_a_payment() {
    let f = detect("Alex shares the deck with Jo").unwrap();
    assert_eq!(f.flow, FlowKind::Share);
    assert_eq!(f.actor, "Alex");
    assert_eq!(f.counterparty.as_deref(), Some("Jo"));
    assert_eq!(f.item.as_deref(), Some("deck"));
}

#[test]
fn toggle_upload_signup() {
    let t = detect("Turn on dark mode").unwrap();
    assert_eq!(
        (t.flow, t.item.as_deref()),
        (FlowKind::ToggleSetting, Some("dark mode"))
    );
    let u = detect("Upload a photo on the web dashboard").unwrap();
    assert_eq!((u.flow, u.device), (FlowKind::Upload, "laptop"));
    let s = detect("Lena signs up for Notion").unwrap();
    assert_eq!(s.flow, FlowKind::Signup);
    assert_eq!(
        (s.actor.as_str(), s.item.as_deref()),
        ("Lena", Some("Notion"))
    );
    assert!(detect("Sign the contract").is_none(), "sign ≠ sign up");
}

#[test]
fn brief_finds_the_flow_and_ignores_quotes() {
    let b = "Make a 15 s promo for our wallet app\n\"Pay anyone in a tap\"\nMaya pays Sam $40 for dinner";
    let f = detect_brief(b).unwrap();
    assert_eq!((f.flow, f.actor.as_str()), (FlowKind::Payment, "Maya"));
    let c = motion_bridge::parse_brief(b);
    assert_eq!(c.flow.as_ref().map(|f| f.flow), Some(FlowKind::Payment));
    // The flow line is fields, not a stat or on-screen words.
    assert_eq!(c.sections.len(), 1);
}

#[test]
fn spec_maps_to_core_flow_and_program() {
    use motion_bridge::flow::ui_flow;
    let f = ui_flow(&detect("Maya pays Sam $40 for dinner").unwrap()).unwrap();
    assert_eq!(f.actor.as_deref(), Some("Maya"));
    assert_eq!(f.counterparty.as_deref(), Some("Sam"));
    assert!(f.check().is_empty());
    // A share needs a counterparty: without one there is no flow.
    assert!(ui_flow(&detect("Alex shares the deck").unwrap()).is_none());
    let brief = "Wallet app, 15 s\n\"Pay anyone in a tap\"\nMaya pays Sam $40 for dinner";
    let (_, plan, p) = motion_bridge::make_from_brief(
        brief,
        None,
        None,
        None,
        &motion_bridge::Clock::steady(126.0, 400),
        0,
    )
    .unwrap();
    assert!(plan
        .bars
        .iter()
        .any(|b| b.technique == motion_bridge::Technique::Flow));
    assert!(p
        .components
        .iter()
        .any(|c| c.body.kind_name() == "device_frame"));
}
