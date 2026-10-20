//! Audio plans, music, ducking, speech timing, word cues and captions.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test audio_speech <module>::`.

mod audio_count_vo;
mod audio_cues_taste;
mod audio_plan;
mod bed_alignment;
mod captions;
mod dead_air;
mod duck_envelope;
mod music_mood;
mod speech_lifecycle;
mod speech_timing;
mod word_cues;
