//! Decoding audio files into memory.
//!
//! The pipeline:
//!
//! ```text
//! uridecodebin → audioconvert → audioresample → appsink
//!  (any format)   (to f32)       (to 48 kHz)     (hands the samples to Rust)
//! ```
//!
//! `uridecodebin` picks the right decoder itself (MP3, FLAC, Ogg, Opus, WAV, …).
//! The caps on the appsink ask for exactly Transient's layout: 32-bit float,
//! stereo, interleaved, at the project sample rate. `audioconvert` and
//! `audioresample` do whatever conversion is needed to get there.
//!
//! This runs synchronously and can take a while for long files, so the app
//! calls it on a background thread.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use thiserror::Error;
use transient_core::audio::{AudioBuffer, CHANNELS};
use transient_core::time::SampleRate;

use crate::{EngineError, make_element};

/// The format of the file before decoding, shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFormat {
    /// Original sample rate in Hz.
    pub sample_rate: u32,
    /// Original number of channels (1 = mono, 2 = stereo, …).
    pub channels: u32,
}

/// A decoded file.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// The audio, converted to the project sample rate and stereo.
    pub buffer: AudioBuffer,
    /// What the file contained before conversion, if GStreamer reported it.
    pub source_format: Option<SourceFormat>,
}

/// Why a file could not be decoded.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The engine itself failed (a missing plugin, for example).
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// GStreamer could not read the file: it does not exist, is not audio,
    /// or no decoder is installed for its format.
    #[error("could not read the file: {0}")]
    Unreadable(String),
    /// The file was read but contained no audio.
    #[error("the file contains no audio")]
    NoAudio,
}

/// Decodes the audio file at `path` to stereo at `sample_rate`.
///
/// # Errors
///
/// See [`DecodeError`].
pub fn decode_file(path: &Path, sample_rate: SampleRate) -> Result<DecodedAudio, DecodeError> {
    // GStreamer wants a URI (file:///…), and that needs an absolute path.
    let path =
        std::path::absolute(path).map_err(|error| DecodeError::Unreadable(error.to_string()))?;
    let uri = gstreamer::glib::filename_to_uri(&path, None)
        .map_err(|error| DecodeError::Unreadable(error.to_string()))?;

    let pipeline = gstreamer::Pipeline::new();
    let decoder = make_element("uridecodebin")?;
    decoder.set_property("uri", uri.as_str());
    let convert = make_element("audioconvert")?;
    let resample = make_element("audioresample")?;
    let caps = target_caps(sample_rate)?;
    let sink = AppSink::builder().caps(&caps).sync(false).build();

    pipeline
        .add_many([&decoder, &convert, &resample, sink.upcast_ref()])
        .map_err(EngineError::from)?;
    gstreamer::Element::link_many([&convert, &resample, sink.upcast_ref()])
        .map_err(EngineError::from)?;

    // uridecodebin only creates its output pads once it knows what is in the
    // file. Link the first audio pad to audioconvert, and remember its format.
    // Other streams (an album cover in an MP3, for example) go to a fakesink.
    let source_format: Arc<Mutex<Option<SourceFormat>>> = Arc::default();
    let format_slot = Arc::clone(&source_format);
    let convert_weak = convert.downgrade();
    let pipeline_weak = pipeline.downgrade();
    decoder.connect_pad_added(move |_, pad| {
        let (Some(convert), Some(pipeline)) = (convert_weak.upgrade(), pipeline_weak.upgrade())
        else {
            return;
        };
        let audio_info = pad
            .current_caps()
            .and_then(|caps| gstreamer_audio::AudioInfo::from_caps(&caps).ok());
        let sink_pad = convert.static_pad("sink");
        match (audio_info, sink_pad) {
            (Some(info), Some(sink_pad)) if !sink_pad.is_linked() => {
                if let Ok(mut slot) = format_slot.lock() {
                    *slot = Some(SourceFormat {
                        sample_rate: info.rate(),
                        channels: info.channels(),
                    });
                }
                if let Err(error) = pad.link(&sink_pad) {
                    tracing::warn!("could not link the decoded audio: {error:?}");
                }
            }
            _ => discard_pad(&pipeline, pad),
        }
    });

    let samples = pull_all_samples(&pipeline, &sink);
    if let Err(error) = pipeline.set_state(gstreamer::State::Null) {
        tracing::warn!("could not stop the decoder: {error}");
    }
    let samples = samples?;
    if samples.is_empty() {
        return Err(DecodeError::NoAudio);
    }

    let buffer = AudioBuffer::new(sample_rate, samples)
        .map_err(|error| DecodeError::Unreadable(error.to_string()))?;
    let source_format = source_format.lock().ok().and_then(|slot| *slot);
    Ok(DecodedAudio {
        buffer,
        source_format,
    })
}

/// The caps (GStreamer's word for "format description") Transient wants.
fn target_caps(sample_rate: SampleRate) -> Result<gstreamer::Caps, EngineError> {
    let channels = u32::try_from(CHANNELS).unwrap_or(2);
    let info = gstreamer_audio::AudioInfo::builder(
        gstreamer_audio::AudioFormat::F32le,
        sample_rate.hz(),
        channels,
    )
    .build()?;
    Ok(info.to_caps()?)
}

/// Sends a stream we do not want into a fakesink, so it does not stall the pipeline.
fn discard_pad(pipeline: &gstreamer::Pipeline, pad: &gstreamer::Pad) {
    let Ok(fakesink) = make_element("fakesink") else {
        return;
    };
    let linked = pipeline
        .add(&fakesink)
        .ok()
        .and_then(|()| fakesink.static_pad("sink"))
        .is_some_and(|sink_pad| pad.link(&sink_pad).is_ok());
    if linked && fakesink.sync_state_with_parent().is_err() {
        tracing::warn!("could not start the fakesink for an unused stream");
    }
}

/// Runs the pipeline and collects every decoded sample until the end of the file.
fn pull_all_samples(
    pipeline: &gstreamer::Pipeline,
    sink: &AppSink,
) -> Result<Vec<f32>, DecodeError> {
    pipeline
        .set_state(gstreamer::State::Playing)
        .map_err(|error| {
            tracing::debug!("could not start decoding: {error}");
            read_error(pipeline)
        })?;

    let bus = pipeline
        .bus()
        .ok_or(EngineError::Pipeline("no bus".into()))?;
    let poll_interval = gstreamer::ClockTime::from_mseconds(100);
    let mut samples = Vec::new();

    loop {
        if let Some(sample) = sink.try_pull_sample(poll_interval) {
            if let Some(buffer) = sample.buffer()
                && let Ok(map) = buffer.map_readable()
            {
                samples.extend(
                    map.as_slice()
                        .chunks_exact(4)
                        .filter_map(|bytes| bytes.first_chunk::<4>())
                        .map(|bytes| f32::from_le_bytes(*bytes)),
                );
            }
            continue;
        }
        if sink.is_eos() {
            return Ok(samples);
        }
        // No sample and no end of file yet: check whether GStreamer reported an error,
        // otherwise keep waiting (decoding is still starting up).
        if let Some(message) = bus.pop_filtered(&[gstreamer::MessageType::Error]) {
            return Err(error_from_message(&message));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Turns the first error on the bus into a [`DecodeError`].
fn read_error(pipeline: &gstreamer::Pipeline) -> DecodeError {
    pipeline
        .bus()
        .and_then(|bus| bus.pop_filtered(&[gstreamer::MessageType::Error]))
        .map_or_else(
            || DecodeError::Unreadable("the decoder could not start".into()),
            |message| error_from_message(&message),
        )
}

fn error_from_message(message: &gstreamer::Message) -> DecodeError {
    match message.view() {
        gstreamer::MessageView::Error(error) => DecodeError::Unreadable(error.error().to_string()),
        _ => DecodeError::Unreadable("unknown error".into()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Writes a minimal 16-bit PCM WAV file (written by hand, so the test
    /// does not depend on the downloaded test audio).
    fn write_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[i16]) {
        let data_len = u32::try_from(samples.len() * 2).unwrap();
        let block_align = channels * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::File::create(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }

    #[test]
    fn a_mono_44_khz_file_becomes_stereo_at_48_khz() {
        crate::init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("half-scale.wav");
        // One second of a constant half-scale signal, mono, 44.1 kHz.
        write_wav(&path, 44_100, 1, &vec![i16::MAX / 2; 44_100]);

        let decoded = decode_file(&path, SampleRate::HZ_48000).unwrap();

        assert_eq!(
            decoded.source_format,
            Some(SourceFormat {
                sample_rate: 44_100,
                channels: 1
            })
        );
        let frames = decoded.buffer.frames();
        assert!(
            frames.abs_diff(48_000) <= 64,
            "expected about 1 s at 48 kHz, got {frames}"
        );
        // The middle of the file (away from resampler edges) keeps its level in both channels.
        let middle = decoded.buffer.frames_from(24_000, 1);
        assert!(
            middle.iter().all(|s| (s - 0.5).abs() < 0.01),
            "got {middle:?}"
        );
    }

    #[test]
    fn a_missing_file_is_an_error() {
        crate::init().unwrap();
        let error = decode_file(Path::new("/nonexistent/file.mp3"), SampleRate::HZ_48000);
        assert!(
            matches!(error, Err(DecodeError::Unreadable(_))),
            "got {error:?}"
        );
    }

    #[test]
    fn a_text_file_is_an_error() {
        crate::init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.mp3");
        std::fs::write(&path, "this is not audio").unwrap();
        let error = decode_file(&path, SampleRate::HZ_48000);
        assert!(error.is_err(), "got {error:?}");
    }
}
