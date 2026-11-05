//! Recording from a microphone or line input into memory.
//!
//! ```text
//! input → audioconvert → audioresample → capsfilter → level → appsink
//!          (to f32)       (to 48 kHz)   (stereo f32)  (meter) (to Rust)
//! ```
//!
//! The input is the system default (`pulsesrc`, which works through PipeWire,
//! also inside the Flatpak), one of the devices from [`inputs`], or a test
//! tone for automated tests. The samples collect in memory; [`Recorder::stop`]
//! hands them over as an [`AudioBuffer`]. Like the player, the `level`
//! element measures the input for the level meter ([`Recorder::poll_levels`]).

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSinkCallbacks};
use transient_core::audio::{AudioBuffer, CHANNELS};
use transient_core::mix::{AudioPool, Mix};
use transient_core::project::Project;
use transient_core::time::{Frames, SampleRate};

use crate::decode::target_caps;
use crate::export::{ExportError, ExportFormat, export};
use crate::player::read_levels;
use crate::{EngineError, make_element};

/// How many frames each value of [`Recorder::peaks_from`] covers (about 21 ms).
pub const PEAK_BLOCK: usize = 1024;

/// How often the input level is measured.
const LEVEL_INTERVAL: Duration = Duration::from_millis(50);
/// How long [`Recorder::stop`] waits for the last audio to arrive.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Where to record from.
#[derive(Debug, Clone, Default)]
pub enum Input {
    /// The input chosen in the system's sound settings.
    #[default]
    Default,
    /// A specific device (see [`inputs`]).
    Device(InputDevice),
    /// A 440 Hz tone instead of a microphone, for automated tests.
    TestTone,
}

/// A sound input that can be recorded from, such as a microphone.
#[derive(Debug, Clone)]
pub struct InputDevice {
    /// The name to show, for example "Built-in Audio Analog Stereo".
    pub name: String,
    device: gstreamer::Device,
}

impl InputDevice {
    /// An identifier that stays the same between runs (to remember the choice).
    #[must_use]
    pub fn id(&self) -> String {
        // PipeWire and PulseAudio devices carry a stable node or device name.
        self.device
            .properties()
            .and_then(|properties| {
                ["node.name", "device.name", "object.path"]
                    .iter()
                    .find_map(|key| properties.get::<String>(*key).ok())
            })
            .unwrap_or_else(|| self.name.clone())
    }
}

/// The sound inputs (microphones, line inputs) GStreamer can see now.
///
/// "Monitor" inputs, which only capture what is playing, are left out, and each
/// input is listed once (PipeWire and PulseAudio can both report the same one).
#[must_use]
pub fn inputs() -> Vec<InputDevice> {
    let monitor = gstreamer::DeviceMonitor::new();
    let _filter = monitor.add_filter(Some("Audio/Source"), None);
    if monitor.start().is_err() {
        return Vec::new();
    }
    let mut devices: Vec<InputDevice> = Vec::new();
    for device in monitor.devices() {
        let is_monitor = device.properties().is_some_and(|properties| {
            properties.get::<String>("device.class").ok().as_deref() == Some("monitor")
        });
        let name = device.display_name().to_string();
        if !is_monitor && devices.iter().all(|known| known.name != name) {
            devices.push(InputDevice { name, device });
        }
    }
    monitor.stop();
    devices
}

/// A recording in progress.
#[derive(Debug)]
pub struct Recorder {
    pipeline: gstreamer::Pipeline,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: SampleRate,
}

impl Recorder {
    /// Starts recording from `input`, at `sample_rate`, in stereo.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the input cannot be opened (no microphone,
    /// no permission) or GStreamer is missing an element.
    pub fn start(input: &Input, sample_rate: SampleRate) -> Result<Self, EngineError> {
        let source = match input {
            Input::Default => make_element("pulsesrc")?,
            Input::Device(device) => device.device.create_element(None)?,
            Input::TestTone => {
                let tone = make_element("audiotestsrc")?;
                tone.set_property("is-live", true);
                tone.set_property("freq", 440.0_f64);
                tone.set_property("volume", 0.5_f64);
                tone
            }
        };
        let convert = make_element("audioconvert")?;
        let resample = make_element("audioresample")?;
        let format = make_element("capsfilter")?;
        format.set_property("caps", target_caps(sample_rate)?);
        let level = make_element("level")?;
        level.set_property(
            "interval",
            u64::try_from(LEVEL_INTERVAL.as_nanos()).unwrap_or(u64::MAX),
        );
        level.set_property("post-messages", true);
        let sink = AppSink::builder().sync(false).build();

        let samples = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&samples);
        sink.set_callbacks(
            AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink
                        .pull_sample()
                        .map_err(|_stopped| gstreamer::FlowError::Eos)?;
                    if let Some(buffer) = sample.buffer()
                        && let Ok(map) = buffer.map_readable()
                        && let Ok(mut samples) = collected.lock()
                    {
                        // Little-endian 32-bit floats, 4 bytes each (see the caps).
                        samples.extend(
                            map.chunks_exact(4).map(|bytes| {
                                f32::from_le_bytes(bytes.try_into().unwrap_or_default())
                            }),
                        );
                    }
                    Ok(gstreamer::FlowSuccess::Ok)
                })
                .build(),
        );

        let pipeline = gstreamer::Pipeline::new();
        let elements = [
            &source,
            &convert,
            &resample,
            &format,
            &level,
            sink.upcast_ref(),
        ];
        pipeline.add_many(elements)?;
        gstreamer::Element::link_many(elements)?;
        pipeline.set_state(gstreamer::State::Playing)?;
        Ok(Self {
            pipeline,
            samples,
            sample_rate,
        })
    }

    /// How much has been recorded so far.
    #[must_use]
    pub fn recorded_frames(&self) -> Frames {
        let samples = self.samples.lock().map_or(0, |samples| samples.len());
        Frames::try_from(samples / CHANNELS).unwrap_or(0)
    }

    /// A rough outline of what was recorded, to draw while recording: the
    /// loudest sample (of either channel) in each block of [`PEAK_BLOCK`]
    /// frames, starting at block `first`. Only complete blocks; ask again from
    /// where the last answer ended to get just the new ones.
    #[must_use]
    pub fn peaks_from(&self, first: usize) -> Vec<f32> {
        let Ok(samples) = self.samples.lock() else {
            return Vec::new();
        };
        samples
            .chunks_exact(PEAK_BLOCK * CHANNELS)
            .skip(first)
            .map(|block| {
                block
                    .iter()
                    .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
            })
            .collect()
    }

    /// The input's latest level per channel, in dBFS (for the level meter), or
    /// `None` if nothing new was measured. Call this regularly.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Pipeline`] if the input stopped with an error
    /// (for example, the microphone was unplugged).
    pub fn poll_levels(&self) -> Result<Option<[f64; CHANNELS]>, EngineError> {
        let Some(bus) = self.pipeline.bus() else {
            return Ok(None);
        };
        let mut levels = None;
        while let Some(message) = bus.pop_filtered(&[
            gstreamer::MessageType::Element,
            gstreamer::MessageType::Error,
        ]) {
            match message.view() {
                gstreamer::MessageView::Element(element) => {
                    if let Some(measured) = element.structure().and_then(read_levels) {
                        levels = Some(measured);
                    }
                }
                gstreamer::MessageView::Error(error) => {
                    return Err(EngineError::Stream(error.error()));
                }
                _ => {}
            }
        }
        Ok(levels)
    }

    /// Stops recording and returns what was recorded.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if GStreamer fails to stop.
    pub fn stop(self) -> Result<AudioBuffer, EngineError> {
        // End of stream first, so the audio still on its way arrives too.
        self.pipeline.send_event(gstreamer::event::Eos::new());
        if let Some(bus) = self.pipeline.bus() {
            let timeout = gstreamer::ClockTime::from_nseconds(
                u64::try_from(STOP_TIMEOUT.as_nanos()).unwrap_or(u64::MAX),
            );
            let _last = bus.timed_pop_filtered(
                timeout,
                &[gstreamer::MessageType::Eos, gstreamer::MessageType::Error],
            );
        }
        self.pipeline.set_state(gstreamer::State::Null)?;
        let samples = self
            .samples
            .lock()
            .map(|mut samples| std::mem::take(&mut *samples))
            .unwrap_or_default();
        Ok(AudioBuffer::new(self.sample_rate, samples)?)
    }
}

/// Writes a recording to `path` as a 24-bit FLAC file (lossless, about half the
/// size of a WAV). It goes through the export code, as a mix of one clip.
///
/// # Errors
///
/// Returns [`ExportError`] if the file cannot be written.
pub fn save_flac(audio: &AudioBuffer, path: &Path) -> Result<(), ExportError> {
    let mut project = Project {
        sample_rate: audio.sample_rate(),
        ..Project::default()
    };
    let track = project.import(path.to_owned(), audio.frames(), String::new(), 0);
    let source = project
        .track(track)
        .and_then(|track| track.clips.first())
        .map(|clip| clip.source);
    let pool: AudioPool = source.into_iter().map(|id| (id, audio.clone())).collect();
    export(
        &Mix::new(&project, &pool),
        path,
        ExportFormat::Flac { bits: 24 },
        |_progress| {},
        &AtomicBool::new(false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_a_test_tone() {
        crate::init().unwrap();
        let recorder = Recorder::start(&Input::TestTone, SampleRate::HZ_48000).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let levels = recorder.poll_levels().unwrap();
        // The outline for drawing: one peak per block, about half (the tone's volume).
        let peaks = recorder.peaks_from(0);
        assert!(!peaks.is_empty());
        assert!(
            peaks.iter().all(|peak| (0.4..0.6).contains(peak)),
            "peaks {peaks:?}"
        );
        assert_eq!(recorder.peaks_from(peaks.len() + 1000), Vec::<f32>::new());
        let audio = recorder.stop().unwrap();

        // About half a second (a live source starts with a little delay).
        let seconds = SampleRate::HZ_48000.frames_to_seconds(audio.frames());
        assert!((0.3..0.8).contains(&seconds), "recorded {seconds} s");
        // A tone at half volume peaks at about −6 dBFS.
        let peak = audio
            .samples()
            .iter()
            .fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!((0.4..0.6).contains(&peak), "peak {peak}");
        let levels = levels.expect("the meter measured the input");
        assert!(
            levels.iter().all(|db| (-8.0..-4.0).contains(db)),
            "levels {levels:?}"
        );

        // Saved as FLAC, it decodes to the same length.
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("take.flac");
        save_flac(&audio, &file).unwrap();
        let decoded = crate::decode::decode_file(&file, SampleRate::HZ_48000).unwrap();
        assert_eq!(decoded.buffer.frames(), audio.frames());
    }
}
