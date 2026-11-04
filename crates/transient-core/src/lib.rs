//! The core of Transient: the project model and the audio maths.
//!
//! This crate deliberately knows nothing about GTK or GStreamer. Everything in it
//! is plain Rust, which keeps it fast to compile and easy to test.

pub mod audio;
pub mod clipboard;
pub mod edit;
pub mod mix;
pub mod normalize;
pub mod project;
pub mod range;
pub mod ruler;
pub mod time;
pub mod view;
pub mod waveform;
