//! Recording (`win.record`, R, and the Record button in the bottom bar).
//!
//! Recording starts at the playhead, with the other tracks playing along
//! (use headphones, or the speakers end up in the take). Pressing Record again,
//! or Space, stops it. While recording, the level meter shows the input.
//!
//! A take is saved as FLAC into the project's folder for unpacked audio (see
//! `file.rs`), then added as a new track, like an imported file: when the
//! project is saved, the take goes into the project file.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use tracing::{info, warn};
use transient_core::time::Frames;
use transient_core::waveform::Waveform;
use transient_engine::decode::SourceFormat;
use transient_engine::recorder::{Input, Recorder, save_flac};

use super::Window;
use super::project::LoadedSource;

/// A recording in progress.
#[derive(Debug)]
pub(super) struct Recording {
    recorder: Recorder,
    /// Where on the timeline the take starts.
    start: Frames,
    /// The input's last measured level, for the level meter.
    levels: std::cell::Cell<Option<[f64; 2]>>,
}

impl Window {
    /// Starts recording, or stops the recording in progress (`win.record`, R).
    pub(crate) fn toggle_recording(&self) {
        if self.is_recording() {
            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = window)]
                self,
                async move { window.stop_recording().await }
            ));
        } else {
            self.start_recording();
        }
    }

    /// Whether a recording is in progress.
    pub(super) fn is_recording(&self) -> bool {
        self.imp().recording.borrow().is_some()
    }

    fn start_recording(&self) {
        let imp = self.imp();
        let rate = imp.project.borrow().sample_rate;
        let input = imp.record_input.borrow().clone();
        let recorder = match Recorder::start(&input, rate) {
            Ok(recorder) => recorder,
            Err(error) => {
                warn!("could not start recording: {error}");
                self.show_toast(&gettext("Could not record from the input"));
                return;
            }
        };
        let start = self.playhead_position().unwrap_or(0);
        info!("recording from {input:?} at frame {start}");
        imp.recording.replace(Some(Recording {
            recorder,
            start,
            levels: std::cell::Cell::new(None),
        }));
        // The other tracks play along (overdubbing).
        self.set_playing(true);
        self.update_record_button();
    }

    /// Stops recording, saves the take and adds it as a new track.
    pub(super) async fn stop_recording(&self) {
        let imp = self.imp();
        let Some(Recording {
            recorder, start, ..
        }) = imp.recording.take()
        else {
            return;
        };
        self.update_record_button();
        self.set_playing(false);

        let Some(folder) = self.audio_folder() else {
            self.show_toast(&gettext("Could not save the recording"));
            return;
        };
        let number = self.next_recording_number();
        let name = gettext("Recording {}").replace("{}", &number.to_string());
        let path = folder.join(format!("{name}.flac"));
        let file = path.clone();
        // Stopping waits for the last audio, and encoding takes a moment.
        let saved = gio::spawn_blocking(move || {
            let audio = recorder.stop().map_err(|error| error.to_string())?;
            save_flac(&audio, &file).map_err(|error| error.to_string())?;
            Ok::<_, String>(audio)
        })
        .await;
        let audio = match saved {
            Ok(Ok(audio)) if audio.frames() > 0 => audio,
            Ok(Ok(_)) => {
                self.show_toast(&gettext("Nothing was recorded"));
                return;
            }
            Ok(Err(error)) => {
                warn!("could not save the recording: {error}");
                self.show_toast(&gettext("Could not save the recording"));
                return;
            }
            Err(_panic) => {
                warn!("saving the recording stopped unexpectedly");
                self.show_toast(&gettext("Could not save the recording"));
                return;
            }
        };
        info!("recorded {} frames to {}", audio.frames(), path.display());

        let frames = audio.frames();
        let rate = audio.sample_rate();
        let before = imp.project.borrow().clone();
        let track = imp
            .project
            .borrow_mut()
            .import_at(path, frames, name, start);
        let source = imp
            .project
            .borrow()
            .track(track)
            .and_then(|track| track.clips.first())
            .map(|clip| clip.source);
        if let Some(source) = source {
            imp.sources.borrow_mut().insert(
                source,
                LoadedSource {
                    waveform: Waveform::new(audio),
                    format: Some(SourceFormat {
                        sample_rate: rate.hz(),
                        channels: 2,
                    }),
                },
            );
        }
        // One undo step, like an import.
        imp.history.borrow_mut().record(before);
        self.project_changed();
    }

    /// The input's latest level while recording (for the level meter), polled
    /// with the transport updates. `None` when not recording or nothing was
    /// measured yet. If the input failed, the recording stops (with a toast).
    pub(super) fn poll_recording(&self) -> Option<[f64; 2]> {
        let polled = {
            let recording = self.imp().recording.borrow();
            let recording = recording.as_ref()?;
            recording.recorder.poll_levels().map(|levels| {
                // Keep the last measurement: the meter updates more often than
                // the input is measured.
                if levels.is_some() {
                    recording.levels.set(levels);
                }
                recording.levels.get()
            })
        };
        match polled {
            Ok(levels) => levels,
            Err(error) => {
                warn!("recording failed: {error}");
                self.show_toast(&gettext("Recording stopped: the input failed"));
                self.toggle_recording();
                None
            }
        }
    }

    /// The Record button: a red record icon, or a red Stop button while recording.
    pub(super) fn update_record_button(&self) {
        let imp = self.imp();
        let button = &*imp.record_button;
        if self.is_recording() {
            button.set_icon_name("media-playback-stop-symbolic");
            button.add_css_class("destructive-action");
            crate::widgets::set_button_text(button, &gettext("Stop Recording"));
        } else {
            button.set_icon_name("media-record-symbolic");
            button.remove_css_class("destructive-action");
            crate::widgets::set_button_text(button, &gettext("Record"));
        }
    }

    /// The number for the next take's name: one more than the highest
    /// "Recording N" track so far.
    fn next_recording_number(&self) -> u32 {
        let prefix = gettext("Recording {}").replace("{}", "");
        self.imp()
            .project
            .borrow()
            .tracks
            .iter()
            .filter_map(|track| track.name.strip_prefix(&prefix)?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Stops the recording in progress and waits until its take is added (for
    /// `--record-for`, which saves the project afterwards).
    pub(crate) async fn stop_recording_now(&self) {
        self.stop_recording().await;
    }

    /// Uses a test tone instead of a microphone (for `--record-for` tests).
    pub(crate) fn use_test_input(&self) {
        self.imp().record_input.replace(Input::TestTone);
    }
}
