//! The core of Transient: the project model and the audio maths.
//!
//! This crate deliberately knows nothing about GTK or GStreamer. Everything in it
//! is plain Rust, which keeps it fast to compile and easy to test.

pub mod archive;
pub mod audio;
pub mod clip;
pub mod clipboard;
pub mod convert;
pub mod edit;
pub mod history;
pub mod mix;
pub mod normalize;
pub mod project;
pub mod range;
pub mod report;
pub mod ruler;
pub mod time;
pub mod view;
pub mod waveform;

#[cfg(test)]
mod test_support;
