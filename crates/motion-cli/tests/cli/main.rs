//! The motion-engine binary end to end.
//! One test binary per area (fewer links, fewer first-launch scans); each
//! file is a module. Run one: `cargo test -p motion-cli --test cli <module>::`.

mod asset_cli;
mod candidates_cli;
mod explore_cli;
mod fonts_cli;
mod gen_assets_cli;
mod motion_language;
mod public_interface;
mod qa_intel;
mod reel_music;
mod reference_cli;
mod speech_cli;
mod style_cmds;
mod takes_cli;
mod v0_2;
mod visual_qa_cli;
mod voice_cli;
