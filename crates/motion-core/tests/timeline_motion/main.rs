//! Timeline evaluation and motion operators: easing, springs, glyphs, 3D, cameras, carries.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test timeline_motion <module>::`.

mod camera;
mod cinematic_arrivals;
mod easing;
mod focal_camera;
mod kinetic;
mod motion_ops_2;
mod perspective;
mod picture_carry;
mod revolve_3d;
mod shared_motion;
mod sphere_gallery;
mod spring_physics;
mod sprite_timeline;
mod stagger;
mod tilt_motion;
mod timeline;
mod tint_channel;
