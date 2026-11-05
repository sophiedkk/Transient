//! Exporting the mix to an audio file.
//!
//! The pipeline:
//!
//! ```text
//! appsrc → audioconvert → [capsfilter] → encoder → [muxer] → filesink
//! ```
//!
//! The mix is rendered chunk by chunk and pushed into `appsrc`. With
//! `block = true`, pushing waits whenever GStreamer has enough queued, so
//! memory use stays small however long the export is. `audioconvert` turns the
//! float samples into what the encoder wants (16- or 24-bit for WAV and FLAC);
//! lossy formats get a container (Ogg for Opus and Vorbis).
//!
//! This runs synchronously, so the app calls it on a background thread. It
//! reports progress through a callback and stops early when `cancel` is set;
//! an unfinished file is removed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use thiserror::Error;
use transient_core::audio::CHANNELS;
use transient_core::mix::Mix;
use transient_core::time::{Frames, SampleRate, frames_to_f64};

use crate::{EngineError, frames_to_time, make_element};

/// How many frames to render and push at a time.
const CHUNK_FRAMES: usize = 8192;

/// The file format, with its quality setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Uncompressed WAV with this many bits per sample (16 or 24).
    Wav {
        /// Bits per sample.
        bits: u8,
    },
    /// Lossless FLAC with this many bits per sample (16 or 24).
    Flac {
        /// Bits per sample.
        bits: u8,
    },
    /// MP3 at this bitrate (kbit/s, constant).
    Mp3 {
        /// Bitrate in kbit/s.
        kbps: u32,
    },
    /// Opus in an Ogg file at this bitrate (kbit/s).
    Opus {
        /// Bitrate in kbit/s.
        kbps: u32,
    },
    /// Vorbis in an Ogg file at this bitrate (kbit/s).
    Vorbis {
        /// Bitrate in kbit/s.
        kbps: u32,
    },
}

impl ExportFormat {
    /// The usual file extension, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav { .. } => "wav",
            Self::Flac { .. } => "flac",
            Self::Mp3 { .. } => "mp3",
            Self::Opus { .. } => "opus",
            Self::Vorbis { .. } => "ogg",
        }
    }

    /// A sensible default format for a file extension (`"flac"`, `"mp3"`, …).
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "wav" => Some(Self::Wav { bits: 16 }),
            "flac" => Some(Self::Flac { bits: 16 }),
            "mp3" => Some(Self::Mp3 { kbps: 192 }),
            "opus" => Some(Self::Opus { kbps: 128 }),
            "ogg" | "oga" => Some(Self::Vorbis { kbps: 192 }),
            _ => None,
        }
    }

    /// The GStreamer elements this format needs.
    fn element_names(self) -> &'static [&'static str] {
        match self {
            Self::Wav { .. } => &["wavenc"],
            Self::Flac { .. } => &["flacenc"],
            Self::Mp3 { .. } => &["lamemp3enc"],
            Self::Opus { .. } => &["opusenc", "oggmux"],
            Self::Vorbis { .. } => &["vorbisenc", "oggmux"],
        }
    }

    /// Whether the GStreamer plugins for this format are installed.
    #[must_use]
    pub fn is_available(self) -> bool {
        self.element_names()
            .iter()
            .all(|name| gstreamer::ElementFactory::find(name).is_some())
    }

    /// Builds the format's elements, from the capsfilter (if any) to the muxer.
    fn build_elements(self) -> Result<Vec<gstreamer::Element>, EngineError> {
        let integer_caps = |bits: u8| -> Result<gstreamer::Element, EngineError> {
            let format = if bits >= 24 { "S24LE" } else { "S16LE" };
            let caps = gstreamer::Caps::builder("audio/x-raw")
                .field("format", format)
                .build();
            let filter = make_element("capsfilter")?;
            filter.set_property("caps", &caps);
            Ok(filter)
        };
        let bitrate = |kbps: u32| kbps.clamp(8, 512);
        Ok(match self {
            Self::Wav { bits } => vec![integer_caps(bits)?, make_element("wavenc")?],
            Self::Flac { bits } => vec![integer_caps(bits)?, make_element("flacenc")?],
            Self::Mp3 { kbps } => {
                let encoder = make_element("lamemp3enc")?;
                encoder.set_property_from_str("target", "bitrate");
                encoder.set_property("bitrate", i32::try_from(bitrate(kbps)).unwrap_or(192));
                encoder.set_property("cbr", true);
                vec![encoder]
            }
            Self::Opus { kbps } => {
                let encoder = make_element("opusenc")?;
                encoder.set_property(
                    "bitrate",
                    i32::try_from(bitrate(kbps) * 1000).unwrap_or(128_000),
                );
                vec![encoder, make_element("oggmux")?]
            }
            Self::Vorbis { kbps } => {
                let encoder = make_element("vorbisenc")?;
                encoder.set_property(
                    "bitrate",
                    i32::try_from(bitrate(kbps) * 1000).unwrap_or(192_000),
                );
                vec![encoder, make_element("oggmux")?]
            }
        })
    }
}

/// Why an export did not finish.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The engine itself failed (a missing plugin, for example).
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// There is nothing to export.
    #[error("the project is empty")]
    Empty,
    /// The export was cancelled; the unfinished file was removed.
    #[error("the export was cancelled")]
    Cancelled,
}

/// Exports `mix` to `path` in `format`. Calls `progress` with the fraction done
/// (0 to 1) after every chunk, and stops early when `cancel` becomes true.
///
/// # Errors
///
/// See [`ExportError`]. On any error the incomplete file is removed.
pub fn export(
    mix: &Mix,
    path: &Path,
    format: ExportFormat,
    mut progress: impl FnMut(f64),
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    if mix.length() == 0 {
        return Err(ExportError::Empty);
    }
    let result = run_pipeline(mix, path, format, &mut progress, cancel);
    if result.is_err() {
        // Best effort: an unfinished file is useless and misleading.
        if let Err(error) = std::fs::remove_file(path) {
            tracing::debug!(
                "could not remove the unfinished {}: {error}",
                path.display()
            );
        }
    }
    result
}

fn run_pipeline(
    mix: &Mix,
    path: &Path,
    format: ExportFormat,
    progress: &mut impl FnMut(f64),
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    let rate = mix.sample_rate();
    let caps = gstreamer_audio::AudioInfo::builder(
        gstreamer_audio::AudioFormat::F32le,
        rate.hz(),
        u32::try_from(CHANNELS).unwrap_or(2),
    )
    .build()
    .map_err(EngineError::from)?
    .to_caps()
    .map_err(EngineError::from)?;
    let appsrc = AppSrc::builder()
        .caps(&caps)
        .format(gstreamer::Format::Time)
        .block(true)
        .max_bytes(4 * 1024 * 1024)
        .build();
    let convert = make_element("audioconvert")?;
    let sink = make_element("filesink")?;
    sink.set_property("location", absolute(path).as_path());

    let mut elements = vec![appsrc.clone().upcast::<gstreamer::Element>(), convert];
    elements.extend(format.build_elements()?);
    elements.push(sink);

    let pipeline = gstreamer::Pipeline::new();
    pipeline.add_many(&elements).map_err(EngineError::from)?;
    gstreamer::Element::link_many(&elements).map_err(EngineError::from)?;
    pipeline
        .set_state(gstreamer::State::Playing)
        .map_err(EngineError::from)?;

    let result = push_mix(mix, &appsrc, rate, progress, cancel).and_then(|()| {
        appsrc.end_of_stream().map_err(EngineError::from)?;
        wait_for_end(&pipeline)
    });
    if let Err(error) = pipeline.set_state(gstreamer::State::Null) {
        tracing::warn!("could not stop the export pipeline: {error}");
    }
    result
}

/// Renders the mix chunk by chunk into `appsrc`.
fn push_mix(
    mix: &Mix,
    appsrc: &AppSrc,
    rate: SampleRate,
    progress: &mut impl FnMut(f64),
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    let length = mix.length();
    let chunk_frames = Frames::try_from(CHUNK_FRAMES).unwrap_or(1);
    let mut samples = vec![0.0_f32; CHUNK_FRAMES * CHANNELS];
    let mut position: Frames = 0;
    while position < length {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let frames = (length - position).min(chunk_frames);
        let count = usize::try_from(frames).unwrap_or(0) * CHANNELS;
        let chunk = samples.get_mut(..count).unwrap_or_default();
        mix.render(position, chunk);
        let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut buffer = gstreamer::Buffer::from_mut_slice(bytes);
        if let Some(buffer) = buffer.get_mut() {
            buffer.set_pts(frames_to_time(position, rate));
            buffer.set_duration(frames_to_time(frames, rate));
        }
        appsrc.push_buffer(buffer).map_err(EngineError::from)?;
        position += frames;
        progress(frames_to_f64(position) / frames_to_f64(length));
    }
    Ok(())
}

/// Waits until the file is completely written (or GStreamer reports an error).
fn wait_for_end(pipeline: &gstreamer::Pipeline) -> Result<(), ExportError> {
    let bus = pipeline.bus().ok_or(EngineError::NoBus)?;
    let message = bus.timed_pop_filtered(
        gstreamer::ClockTime::NONE,
        &[gstreamer::MessageType::Eos, gstreamer::MessageType::Error],
    );
    let Some(message) = message else {
        return Err(EngineError::Stopped.into());
    };
    match message.view() {
        gstreamer::MessageView::Eos(_) => Ok(()),
        _ => Err(crate::bus_error(&message).into()),
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use transient_core::audio::{AudioBuffer, to_dbfs};
    use transient_core::mix::AudioPool;
    use transient_core::project::Project;

    use super::*;
    use crate::decode::decode_file;

    /// Two seconds of a 440 Hz sine at half scale, as a mix.
    fn tone() -> Mix {
        let frames = 2 * 48_000;
        let samples: Vec<f32> = (0..frames)
            .flat_map(|n: u32| {
                let phase = f64::from(n) * 440.0 * std::f64::consts::TAU / 48_000.0;
                #[expect(
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    reason = "test"
                )]
                let value = 0.5 * phase.sin() as f32;
                [value, value]
            })
            .collect();
        let mut project = Project::default();
        let track = project.import("tone.wav".into(), u64::from(frames), "tone".to_owned(), 0);
        let source = project.track(track).unwrap().clips[0].source;
        let audio = AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap();
        Mix::new(&project, &AudioPool::from([(source, audio)]))
    }

    /// Exports the tone, decodes the file again, and checks length and level.
    fn round_trip(format: ExportFormat) {
        crate::init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("tone.{}", format.extension()));
        let mut last_progress = 0.0;
        export(
            &tone(),
            &path,
            format,
            |p| last_progress = p,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            (last_progress - 1.0).abs() < f64::EPSILON,
            "progress ends at 1"
        );

        let decoded = decode_file(&path, SampleRate::HZ_48000).unwrap();
        let seconds = decoded.buffer.duration_seconds();
        // Lossy codecs pad a little at the start and end.
        assert!((seconds - 2.0).abs() < 0.08, "{format:?}: {seconds} s");
        let peak = to_dbfs(decoded.buffer.peak()).unwrap();
        assert!((peak + 6.02).abs() < 0.5, "{format:?}: peak {peak:.2} dBFS");
    }

    #[test]
    fn wav_round_trip() {
        round_trip(ExportFormat::Wav { bits: 16 });
        round_trip(ExportFormat::Wav { bits: 24 });
    }

    #[test]
    fn flac_round_trip() {
        round_trip(ExportFormat::Flac { bits: 24 });
    }

    #[test]
    fn mp3_round_trip() {
        round_trip(ExportFormat::Mp3 { kbps: 192 });
    }

    #[test]
    fn opus_round_trip() {
        round_trip(ExportFormat::Opus { kbps: 128 });
    }

    #[test]
    fn vorbis_round_trip() {
        round_trip(ExportFormat::Vorbis { kbps: 192 });
    }

    #[test]
    fn a_cancelled_export_leaves_no_file() {
        crate::init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancelled.wav");
        let result = export(
            &tone(),
            &path,
            ExportFormat::Wav { bits: 16 },
            |_| {},
            &AtomicBool::new(true),
        );
        assert!(matches!(result, Err(ExportError::Cancelled)), "{result:?}");
        assert!(!path.exists());
    }

    #[test]
    fn an_empty_project_is_refused() {
        let mix = Mix::silent(SampleRate::HZ_48000);
        let result = export(
            &mix,
            Path::new("/nonexistent/empty.wav"),
            ExportFormat::Wav { bits: 16 },
            |_| {},
            &AtomicBool::new(false),
        );
        assert!(matches!(result, Err(ExportError::Empty)));
    }

    #[test]
    fn extensions_map_to_formats() {
        for format in [
            ExportFormat::Wav { bits: 16 },
            ExportFormat::Flac { bits: 16 },
            ExportFormat::Mp3 { kbps: 192 },
            ExportFormat::Opus { kbps: 128 },
            ExportFormat::Vorbis { kbps: 192 },
        ] {
            assert_eq!(
                ExportFormat::from_extension(format.extension()),
                Some(format)
            );
        }
        assert_eq!(ExportFormat::from_extension("txt"), None);
    }
}
