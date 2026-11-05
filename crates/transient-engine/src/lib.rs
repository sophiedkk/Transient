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
use transient_core::time::{Frames, SampleRate};

/// Why the audio engine failed.
///
/// Each variant keeps GStreamer's own error as its source (see
/// [`Report`](transient_core::report::Report) to print the whole chain).
#[derive(Debug, Error)]
pub enum EngineError {
    /// GStreamer could not start, usually because it is not installed correctly.
    #[error("could not start GStreamer")]
    Init(#[source] gstreamer::glib::Error),
    /// A GStreamer plugin that Transient needs is not installed.
    #[error("the GStreamer element “{0}” is not installed")]
    MissingElement(&'static str),
    /// Building or setting up a pipeline failed.
    #[error("could not set up the audio pipeline")]
    Setup(#[from] gstreamer::glib::BoolError),
    /// A pipeline could not start, pause or stop.
    #[error("the audio pipeline could not change state")]
    StateChange(#[from] gstreamer::StateChangeError),
    /// Audio could not be passed into a pipeline.
    #[error("could not pass audio to the pipeline")]
    Flow(#[from] gstreamer::FlowError),
    /// GStreamer reported an error while running (a decoder, an encoder, a
    /// sound device or a file).
    #[error("the audio pipeline failed")]
    Stream(#[source] gstreamer::glib::Error),
    /// A pipeline has no message bus (GStreamer gives every pipeline one, so this
    /// should not happen).
    #[error("the audio pipeline has no message bus")]
    NoBus,
    /// A pipeline stopped without saying why.
    #[error("the audio pipeline stopped unexpectedly")]
    Stopped,
    /// The audio a pipeline produced is not usable.
    #[error("the audio is unusable")]
    Audio(#[from] transient_core::audio::AudioError),
}

/// The error a pipeline reported on its bus: [`EngineError::Stream`] for an
/// error message, [`EngineError::Stopped`] for anything else.
fn bus_error(message: &gstreamer::Message) -> EngineError {
    match message.view() {
        gstreamer::MessageView::Error(error) => EngineError::Stream(error.error()),
        _ => EngineError::Stopped,
    }
}

/// Frames → GStreamer time (nanoseconds), rounding down.
fn frames_to_time(frames: Frames, rate: SampleRate) -> gstreamer::ClockTime {
    let nanos = u128::from(frames) * 1_000_000_000 / u128::from(rate.hz());
    gstreamer::ClockTime::from_nseconds(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// GStreamer time → frames, rounding to the nearest frame.
fn time_to_frames(time: gstreamer::ClockTime, rate: SampleRate) -> Frames {
    let frames =
        (u128::from(time.nseconds()) * u128::from(rate.hz()) + 500_000_000) / 1_000_000_000;
    Frames::try_from(frames).unwrap_or(Frames::MAX)
}

/// Starts GStreamer. Call this once, before using anything else in this crate.
///
/// # Errors
///
/// Returns [`EngineError::Init`] if GStreamer cannot be initialised.
pub fn init() -> Result<(), EngineError> {
    gstreamer::init().map_err(EngineError::Init)
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

    #[test]
    fn time_and_frames_convert_both_ways() {
        let rate = SampleRate::HZ_48000;
        assert_eq!(frames_to_time(48_000, rate), gstreamer::ClockTime::SECOND);
        for frames in [0, 1, 441, 48_000, 123_456_789] {
            assert_eq!(time_to_frames(frames_to_time(frames, rate), rate), frames);
        }
    }
}
