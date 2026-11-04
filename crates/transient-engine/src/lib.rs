//! Transient's audio engine: everything that talks to GStreamer.
//!
//! Decoding files, playing the mix and exporting it all live here, behind a small
//! API, so the GTK code never handles GStreamer pipelines directly.
//!
//! - [`decode::decode_file`] reads any audio file into an [`AudioBuffer`].
//! - [`player::Player`] plays a mix.
//! - [`export::export`] writes a mix to a WAV, FLAC, MP3, Opus or Ogg Vorbis file.
//! - [`recorder::Recorder`] records from a microphone or other input.
//!
//! [`AudioBuffer`]: transient_core::audio::AudioBuffer

pub mod decode;
pub mod export;
pub mod player;
pub mod recorder;

use thiserror::Error;

/// Why the audio engine failed.
#[derive(Debug, Error)]
pub enum EngineError {
    /// GStreamer could not start, usually because it is not installed correctly.
    #[error("could not start GStreamer: {0}")]
    Init(#[from] gstreamer::glib::Error),
    /// A GStreamer plugin that Transient needs is not installed.
    #[error("the GStreamer element “{0}” is not installed")]
    MissingElement(&'static str),
    /// Building or controlling a pipeline failed.
    #[error("audio pipeline error: {0}")]
    Pipeline(String),
}

impl From<gstreamer::glib::BoolError> for EngineError {
    fn from(error: gstreamer::glib::BoolError) -> Self {
        Self::Pipeline(error.to_string())
    }
}

impl From<gstreamer::StateChangeError> for EngineError {
    fn from(error: gstreamer::StateChangeError) -> Self {
        Self::Pipeline(error.to_string())
    }
}

/// Starts GStreamer. Call this once, before using anything else in this crate.
///
/// # Errors
///
/// Returns [`EngineError::Init`] if GStreamer cannot be initialised.
pub fn init() -> Result<(), EngineError> {
    gstreamer::init()?;
    Ok(())
}

/// The GStreamer version in use, for example "GStreamer 1.28.8".
/// Shown in the About dialog's troubleshooting information.
#[must_use]
pub fn gstreamer_version() -> String {
    gstreamer::version_string().to_string()
}

/// Creates a GStreamer element, with a clear error if its plugin is missing.
fn make_element(factory: &'static str) -> Result<gstreamer::Element, EngineError> {
    gstreamer::ElementFactory::make(factory)
        .build()
        .map_err(|error| {
            tracing::debug!("{factory}: {error}");
            EngineError::MissingElement(factory)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gstreamer_starts() {
        init().unwrap();
        assert!(gstreamer_version().starts_with("GStreamer"));
    }
}
