//! Audio mixing, loudness, music and SFX indexes, voice mixes.
//! One test binary per area (fewer links, fewer first-launch scans); each
//! file is a module. Run one: `cargo test -p motion-render --test render_audio <module>::`.

mod audio_mix;
mod cue_room;
mod makeup_gain;
mod mix_qa;
mod music_index;
mod music_mix;
mod sfx_index;
mod voice_mix;
mod voice_relative_mix;
