//! Recording (`win.record`, R, and the Record button in the bottom bar).
//!
//! Recording starts at the playhead, with the other tracks playing along
//! (use headphones, or the speakers end up in the take). Pressing Record again,
//! or Space, stops it. While recording, the level meter shows the input.
//!
//! The arrow on the Record button chooses the input (`win.record-input`): the
//! system default, or one of the inputs GStreamer sees when the menu opens.
//! The choice is remembered (the `record-input` setting).
//!
//! A take is saved as FLAC into the project's folder for unpacked audio (see
//! `file.rs`), then added as a new track, like an imported file: when the
//! project is saved, the take goes into the project file.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use tracing::{info, warn};
use transient_core::report::Report;
use transient_core::time::Frames;
use transient_core::waveform::Waveform;
use transient_engine::decode::SourceFormat;
use transient_engine::export::ExportError;
use transient_engine::recorder::{Input, Recorder, inputs, save_flac};

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
    /// How many outline values the timeline has been given so far.
    peaks_shown: std::cell::Cell<usize>,
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
        let input = self.chosen_input();
        let recorder = match Recorder::start(&input, rate) {
            Ok(recorder) => recorder,
            Err(error) => {
                warn!("could not start recording: {}", Report(&error));
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
            peaks_shown: std::cell::Cell::new(0),
        }));
        // The take is drawn while it grows; in an empty project, show the
        // timeline (instead of the start page) to see it.
        imp.timeline.start_live_take(start, rate);
        imp.stack.set_visible_child_name("project");
        // The Stop button and the meter are in the bottom bar.
        imp.toolbar_view.set_reveal_bottom_bars(true);
        // The other tracks play along (overdubbing).
        self.set_playing(true);
        self.update_record_button();
    }

    /// Stops recording, saves the take and adds it as a new track. Finishes once
    /// the take is added (`--record-for` waits for that before saving).
    pub(crate) async fn stop_recording(&self) {
        let imp = self.imp();
        let Some(Recording {
            recorder, start, ..
        }) = imp.recording.take()
        else {
            return;
        };
        imp.timeline.end_live_take();
        self.update_record_button();
        self.set_playing(false);

        let Some(folder) = self.audio_folder() else {
            self.show_toast(&gettext("Could not save the recording"));
            self.project_changed();
            return;
        };
        let number = self.next_recording_number();
        let name = gettext("Recording {}").replace("{}", &number.to_string());
        let path = folder.join(format!("{name}.flac"));
        let file = path.clone();
        // Stopping waits for the last audio, and encoding takes a moment.
        let saved = gio::spawn_blocking(move || {
            // `?` turns the recorder's `EngineError` into an `ExportError` (which
            // has a `From` for it), so both steps share one error type.
            let audio = recorder.stop()?;
            save_flac(&audio, &file)?;
            Ok::<_, ExportError>(audio)
        })
        .await;
        let audio = match saved {
            Ok(Ok(audio)) if audio.frames() > 0 => audio,
            Ok(Ok(_)) => {
                self.show_toast(&gettext("Nothing was recorded"));
                self.project_changed();
                return;
            }
            Ok(Err(error)) => {
                warn!("could not save the recording: {}", Report(&error));
                self.show_toast(&gettext("Could not save the recording"));
                self.project_changed();
                return;
            }
            Err(_panic) => {
                warn!("saving the recording stopped unexpectedly");
                self.show_toast(&gettext("Could not save the recording"));
                self.project_changed();
                return;
            }
        };
        info!("recorded {} frames to {}", audio.frames(), path.display());

        let frames = audio.frames();
        let rate = audio.sample_rate();
        let before = imp.project.borrow().clone();
        let track = imp.project.borrow_mut().import(path, frames, name, start);
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
            // The take so far, for the timeline: only the new outline values.
            let new_peaks = recording.recorder.peaks_from(recording.peaks_shown.get());
            recording
                .peaks_shown
                .set(recording.peaks_shown.get() + new_peaks.len());
            self.imp()
                .timeline
                .extend_live_take(&new_peaks, recording.recorder.recorded_frames());
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
                warn!("recording failed: {}", Report(&error));
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

    /// Uses a test tone instead of a microphone (for `--record-for` tests).
    pub(crate) fn use_test_input(&self) {
        self.imp().test_input.set(true);
    }

    /// Sets up the input menu on the Record button. Called once.
    pub(super) fn setup_record_input(&self) {
        let saved = self
            .imp()
            .settings
            .get()
            .map(|settings| settings.string("record-input").to_string())
            .unwrap_or_default();
        // A stateful action: its state is the chosen input's ID ("" = default),
        // and the menu shows it as radio items.
        let action = gio::SimpleAction::new_stateful(
            "record-input",
            Some(glib::VariantTy::STRING),
            &saved.to_variant(),
        );
        let weak = self.downgrade();
        action.connect_activate(move |action, value| {
            let Some(id) = value.and_then(glib::Variant::get::<String>) else {
                return;
            };
            action.set_state(&id.to_variant());
            if let Some(window) = weak.upgrade()
                && let Some(settings) = window.imp().settings.get()
                && let Err(error) = settings.set_string("record-input", &id)
            {
                warn!("could not remember the input: {error}");
            }
        });
        self.add_action(&action);

        // The menu is filled in each time it opens, so a microphone plugged in
        // meanwhile shows up.
        let menu = gio::Menu::new();
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.connect_show(move |_| fill_input_menu(&menu));
        self.imp().record_button.set_popover(Some(&popover));
    }

    /// The input to record from: the test tone in tests, else the chosen input,
    /// else (or if it is gone) the system default.
    fn chosen_input(&self) -> Input {
        if self.imp().test_input.get() {
            return Input::TestTone;
        }
        let chosen = self
            .lookup_action("record-input")
            .and_then(|action| action.state())
            .and_then(|state| state.get::<String>())
            .unwrap_or_default();
        if chosen.is_empty() {
            return Input::Default;
        }
        if let Some(device) = inputs().into_iter().find(|device| device.id() == chosen) {
            return Input::Device(device);
        }
        self.show_toast(&gettext(
            "The chosen input is not connected; recording from the default input",
        ));
        Input::Default
    }
}

/// Lists the inputs in the Record button's menu: the system default first.
fn fill_input_menu(menu: &gio::Menu) {
    menu.remove_all();
    let item = |label: &str, id: &str| {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.record-input"), Some(&id.to_variant()));
        item
    };
    let default = gio::Menu::new();
    default.append_item(&item(&gettext("System Default"), ""));
    menu.append_section(None, &default);
    let devices = gio::Menu::new();
    for device in inputs() {
        devices.append_item(&item(&device.name, &device.id()));
    }
    menu.append_section(None, &devices);
}
