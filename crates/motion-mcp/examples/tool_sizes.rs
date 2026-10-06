//! Print each profile's tool definitions and their sizes (budget review).
//! `cargo run -p motion-mcp --example tool_sizes [-- --json weak]`

use motion_mcp::profile::{budget_chars, Profile};
use motion_mcp::schema::{defs_chars, tool_defs};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--json") {
        let p = match args.get(2).map(String::as_str) {
            Some("creator") => Profile::Creator,
            Some("operator") => Profile::Operator,
            _ => Profile::Weak,
        };
        let defs: Vec<_> = tool_defs(p, true).iter().map(|d| d.wire_json()).collect();
        println!("{}", serde_json::to_string_pretty(&defs).unwrap());
        return;
    }
    for p in [Profile::Weak, Profile::Creator, Profile::Operator] {
        let defs = tool_defs(p, true);
        println!(
            "{} {} / {} chars",
            p.name(),
            defs_chars(&defs),
            budget_chars(p)
        );
        for d in &defs {
            println!(
                "  {:<20} {}",
                d.name,
                d.wire_json().to_string().chars().count()
            );
        }
    }
}
