//! Playing audio.
//!
//! The pipeline:
//!
//! ```text
//! appsrc → audioconvert → level → audioresample → autoaudiosink (or fakesink)
//! ```
//!
//! `appsrc` is GStreamer's door for data coming *from* our code. Whenever the
//! pipeline wants more audio it calls `need-data`, and we render the next chunk
//! of the [`Mix`]. When the user seeks, GStreamer calls `seek-data` with the new
//! position and we continue from there. These callbacks run on GStreamer's own
//! threads, so the mix and position are shared through a `Mutex`.
//!
//! After an edit, [`Player::set_mix`] swaps in the new mix; playback continues
//! with it from the next chunk on.
//!
//! The `level` element measures the output 20 times a second and posts the
//! result on the bus; [`Player::poll_events`] picks it up for the level meter.

use std::sync::{Arc, Mutex};

use gstreamer::prelude::*;
use gstreamer_app::{AppSrc, AppSrcCallbacks, AppStreamType};
use transient_core::audio::CHANNELS;
use transient_core::mix::Mix;
use transient_core::time::{Frames, SampleRate};

use crate::{EngineError, make_element};

/// How many frames to hand to GStreamer at a time (about 85 ms at 48 kHz).
const CHUNK_FRAMES: usize = 4096;
/// How often the level meter is updated.
const LEVEL_INTERVAL: gstreamer::ClockTime = gstreamer::ClockTime::from_mseconds(50);

/// Where the sound goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioOutput {
    /// The system's default output (PipeWire), so you hear it.
    #[default]
    Speakers,
    /// Nowhere, but still in real time. For tests and automated runs.
    Silent,
}

/// Something the player reports back to the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    /// Playback reached the end.
    Finished,
    /// Playback failed; the message explains why.
    Error(String),
}

/// What the `need-data` and `seek-data` callbacks share.
#[derive(Debug)]
struct Source {
    mix: Mix,
    /// The next frame to hand to GStreamer.
    next_frame: Frames,
}

/// Plays a [`Mix`].
#[derive(Debug)]
pub struct Player {
    pipeline: gstreamer::Pipeline,
    source: Arc<Mutex<Source>>,
    sample_rate: SampleRate,
    duration: Frames,
    playing: bool,
    /// A seek GStreamer has not finished yet. Seeks complete asynchronously, and
    /// until then the pipeline still reports the old position.
    pending_seek: Option<Frames>,
    /// Where the last seek went, while not playing. GStreamer cannot always
    /// report a position after a seek (not at the very end, for example), so
    /// while stopped the player reports this one. Playing forgets it.
    seeked_to: Option<Frames>,
    /// The latest output level per channel, in dBFS (see [`Player::levels`]).
    levels: Option<[f64; CHANNELS]>,
}

impl Player {
    /// Builds a player for `mix`, paused at the start.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the pipeline cannot be built.
    pub fn new(mix: Mix, output: AudioOutput) -> Result<Self, EngineError> {
        let sample_rate = mix.sample_rate();
        let duration = mix.length();
        let channels = u32::try_from(CHANNELS).unwrap_or(2);
        let caps = gstreamer_audio::AudioInfo::builder(
            gstreamer_audio::AudioFormat::F32le,
            sample_rate.hz(),
            channels,
        )
        .build()?
        .to_caps()?;

        let appsrc = AppSrc::builder()
            .caps(&caps)
            .format(gstreamer::Format::Time)
            .stream_type(AppStreamType::Seekable)
            .build();
        let source = Arc::new(Mutex::new(Source { mix, next_frame: 0 }));
        appsrc.set_callbacks(callbacks(&source, sample_rate));

        let convert = make_element("audioconvert")?;
        let resample = make_element("audioresample")?;
        let level = make_element("level")?;
        level.set_property("interval", LEVEL_INTERVAL.nseconds());
        level.set_property("post-messages", true);
        let sink = match output {
            AudioOutput::Speakers => make_element("autoaudiosink")?,
            AudioOutput::Silent => {
                let sink = make_element("fakesink")?;
                sink.set_property("sync", true);
                sink
            }
        };

        let pipeline = gstreamer::Pipeline::new();
        let elements = [appsrc.upcast_ref(), &convert, &level, &resample, &sink];
        pipeline.add_many(elements)?;
        gstreamer::Element::link_many(elements)?;
        pipeline.set_state(gstreamer::State::Paused)?;

        Ok(Self {
            pipeline,
            source,
            sample_rate,
            duration,
            playing: false,
            pending_seek: None,
            seeked_to: None,
            levels: None,
        })
    }

    /// Starts or resumes playback. At the end, playback restarts from the beginning.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if GStreamer refuses to start.
    pub fn play(&mut self) -> Result<(), EngineError> {
        if self.position() >= self.duration {
            self.seek(0)?;
        }
        self.pipeline.set_state(gstreamer::State::Playing)?;
        self.playing = true;
        // From now on the pipeline's own position is the right one.
        self.seeked_to = None;
        Ok(())
    }

    /// Pauses playback, keeping the position.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if GStreamer refuses to pause.
    pub fn pause(&mut self) -> Result<(), EngineError> {
        self.pipeline.set_state(gstreamer::State::Paused)?;
        self.playing = false;
        Ok(())
    }

    /// Whether audio is playing right now.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Jumps to `frame` (clamped to the end).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the seek fails.
    pub fn seek(&mut self, frame: Frames) -> Result<(), EngineError> {
        let frame = frame.min(self.duration);
        self.pipeline.seek_simple(
            gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::ACCURATE,
            frames_to_time(frame, self.sample_rate),
        )?;
        self.pending_seek = Some(frame);
        self.seeked_to = Some(frame);
        Ok(())
    }

    /// The current playback position in frames.
    #[must_use]
    pub fn position(&self) -> Frames {
        if let Some(frame) = self.pending_seek {
            return frame;
        }
        if !self.playing
            && let Some(frame) = self.seeked_to
        {
            return frame;
        }
        self.pipeline
            .query_position::<gstreamer::ClockTime>()
            .map_or(0, |time| time_to_frames(time, self.sample_rate))
            .min(self.duration)
    }

    /// Replaces what is played, keeping the position. Used after every edit.
    /// Audio GStreamer has already received (about 0.1 s) still plays as it was.
    pub fn set_mix(&mut self, mix: Mix) {
        self.duration = mix.length();
        if let Ok(mut source) = self.source.lock() {
            source.mix = mix;
        }
    }

    /// The output level per channel (left, right) in dBFS: 0 is full scale,
    /// above 0 clips. `None` until the first measurement, and while not playing.
    #[must_use]
    pub fn levels(&self) -> Option<[f64; CHANNELS]> {
        self.levels.filter(|_| self.playing)
    }

    /// The length of the audio in frames.
    #[must_use]
    pub fn duration(&self) -> Frames {
        self.duration
    }

    /// The sample rate of the audio being played.
    #[must_use]
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Collects what happened since the last call. Call this regularly (the app
    /// does so on every display update). After [`PlayerEvent::Finished`] the
    /// player is paused at the end.
    pub fn poll_events(&mut self) -> Vec<PlayerEvent> {
        let Some(bus) = self.pipeline.bus() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        while let Some(message) = bus.pop_filtered(&[
            gstreamer::MessageType::Eos,
            gstreamer::MessageType::Error,
            gstreamer::MessageType::AsyncDone,
            gstreamer::MessageType::Element,
        ]) {
            match message.view() {
                // A seek (or start-up) finished: the pipeline's position is right again.
                gstreamer::MessageView::AsyncDone(_) => self.pending_seek = None,
                gstreamer::MessageView::Element(element) => {
                    if let Some(levels) = element.structure().and_then(read_levels) {
                        self.levels = Some(levels);
                    }
                }
                gstreamer::MessageView::Eos(_) => {
                    if let Err(error) = self.pause() {
                        tracing::warn!("could not pause at the end: {error}");
                    }
                    events.push(PlayerEvent::Finished);
                }
                gstreamer::MessageView::Error(error) => {
                    self.playing = false;
                    events.push(PlayerEvent::Error(error.error().to_string()));
                }
                _ => {}
            }
        }
        events
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Err(error) = self.pipeline.set_state(gstreamer::State::Null) {
            tracing::warn!("could not stop the player: {error}");
        }
    }
}

/// Reads a `level` message: the "decay" field holds the peak level per channel in
/// dBFS, falling back slowly after a peak, which makes a calm meter.
pub(crate) fn read_levels(structure: &gstreamer::StructureRef) -> Option<[f64; CHANNELS]> {
    if structure.name() != "level" {
        return None;
    }
    let decay = structure.get::<gstreamer::glib::ValueArray>("decay").ok()?;
    let mut levels = [f64::NEG_INFINITY; CHANNELS];
    for (level, value) in levels.iter_mut().zip(decay.iter()) {
        *level = value.get::<f64>().ok()?;
    }
    Some(levels)
}

/// The appsrc callbacks: hand out audio, and move the read position on seeks.
fn callbacks(source: &Arc<Mutex<Source>>, sample_rate: SampleRate) -> AppSrcCallbacks {
    let for_data = Arc::clone(source);
    let for_seek = Arc::clone(source);
    AppSrcCallbacks::builder()
        .need_data(move |appsrc, _| {
            let Ok(mut source) = for_data.lock() else {
                return;
            };
            let start = source.next_frame;
            let remaining = source.mix.length().saturating_sub(start);
            if remaining == 0 {
                if let Err(error) = appsrc.end_of_stream() {
                    tracing::debug!("end of stream: {error}");
                }
                return;
            }
            let frames = remaining.min(Frames::try_from(CHUNK_FRAMES).unwrap_or(1));
            let mut samples = vec![0.0; usize::try_from(frames).unwrap_or(0) * CHANNELS];
            source.mix.render(start, &mut samples);
            let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
            let mut buffer = gstreamer::Buffer::from_mut_slice(bytes);
            if let Some(buffer) = buffer.get_mut() {
                buffer.set_pts(frames_to_time(start, sample_rate));
                buffer.set_duration(frames_to_time(frames, sample_rate));
            }
            source.next_frame = start.saturating_add(frames);
            if let Err(error) = appsrc.push_buffer(buffer) {
                tracing::debug!("push_buffer: {error}");
            }
        })
        .seek_data(move |_, offset| {
            // With Format::Time, `offset` is a position in nanoseconds.
            let Ok(mut source) = for_seek.lock() else {
                return false;
            };
            source.next_frame =
                time_to_frames(gstreamer::ClockTime::from_nseconds(offset), sample_rate);
            true
        })
        .build()
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use transient_core::audio::AudioBuffer;
    use transient_core::mix::AudioPool;
    use transient_core::project::Project;

    use super::*;

    /// A mix of one quarter-second clip.
    fn quarter_second() -> Mix {
        let mut project = Project::default();
        let track = project.import("tone.wav".into(), 12_000, "tone".to_owned());
        let source = project.track(track).unwrap().clips[0].source;
        let audio = AudioBuffer::new(SampleRate::HZ_48000, vec![0.1; 2 * 12_000]).unwrap();
        Mix::new(&project, &AudioPool::from([(source, audio)]))
    }

    #[test]
    fn time_and_frames_convert_both_ways() {
        let rate = SampleRate::HZ_48000;
        assert_eq!(frames_to_time(48_000, rate), gstreamer::ClockTime::SECOND);
        for frames in [0, 1, 441, 48_000, 123_456_789] {
            assert_eq!(time_to_frames(frames_to_time(frames, rate), rate), frames);
        }
    }

    #[test]
    fn plays_to_the_end_and_reports_it() {
        crate::init().unwrap();
        let mut player = Player::new(quarter_second(), AudioOutput::Silent).unwrap();
        player.play().unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while events.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            events = player.poll_events();
        }

        assert_eq!(events, vec![PlayerEvent::Finished]);
        // A constant 0.1 is −20 dBFS; the meter must have seen it.
        let levels = player.levels.unwrap();
        assert!(
            levels.iter().all(|db| (db + 20.0).abs() < 0.5),
            "levels {levels:?}"
        );
        assert!(!player.is_playing());
        assert_eq!(player.position(), player.duration());
    }

    #[test]
    fn seeking_moves_the_position() {
        crate::init().unwrap();
        let mut player = Player::new(quarter_second(), AudioOutput::Silent).unwrap();
        player.seek(6_000).unwrap();
        // Right away, before GStreamer has finished the seek…
        assert_eq!(player.position(), 6_000);
        // …and after it has.
        let _ = player.pipeline.state(gstreamer::ClockTime::from_seconds(5));
        player.poll_events();
        assert_eq!(player.position(), 6_000);
    }

    /// Seeking to the very end (after pasting at the end of the timeline, the
    /// playhead goes there) must not show the position as 0:00.
    #[test]
    fn seeking_to_the_end_stays_at_the_end() {
        crate::init().unwrap();
        let mut player = Player::new(quarter_second(), AudioOutput::Silent).unwrap();
        let end = player.duration();
        player.seek(end).unwrap();
        let _ = player.pipeline.state(gstreamer::ClockTime::from_seconds(5));
        std::thread::sleep(Duration::from_millis(200));
        player.poll_events();
        assert_eq!(player.position(), end);
    }
}
