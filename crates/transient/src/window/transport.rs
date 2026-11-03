//! The transport bar: play/pause, go to start, the time display and zoom buttons,
//! plus seeking by clicking on the waveform.
//!
//! The player runs on GStreamer's threads; the window asks it for the position
//! a few times per second (`update_transport`) and updates the controls.

use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::glib;
use tracing::warn;
use transient_core::time::{Frames, SampleRate, format_position};
use transient_engine::player::{Player, PlayerEvent};

use super::Window;

/// How often the time display is updated while the window is open.
const UPDATE_INTERVAL: Duration = Duration::from_millis(40);

impl Window {
    /// Connects the slider and starts the display updates. Called once, from `constructed`.
    pub(super) fn setup_transport(&self) {
        self.set_playback_enabled(false);

        // Clicking or dragging on the waveform seeks.
        let weak = self.downgrade();
        self.imp().waveform.connect_seek(move |_, frame| {
            if let Some(window) = weak.upgrade() {
                window.seek_to_frame(frame);
            }
        });

        // A weak reference, so this timer does not keep a closed window alive.
        let weak = self.downgrade();
        glib::timeout_add_local(UPDATE_INTERVAL, move || {
            let Some(window) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            window.update_transport();
            glib::ControlFlow::Continue
        });
    }

    /// Prepares the controls for a newly opened file of `frames` length.
    pub(super) fn reset_transport(&self, frames: Frames, rate: SampleRate) {
        let imp = self.imp();
        imp.duration_label
            .set_label(&format!("/ {}", format_position(frames, rate)));
        imp.toolbar_view.set_reveal_bottom_bars(true);
        self.set_playback_enabled(true);
        self.update_transport();
    }

    /// Plays if paused, pauses if playing (the `win.play-pause` action, Space).
    pub(crate) fn toggle_playback(&self) {
        let result = {
            let mut player = self.imp().player.borrow_mut();
            let Some(player) = player.as_mut() else {
                return;
            };
            if player.is_playing() {
                player.pause()
            } else {
                player.play()
            }
        };
        if let Err(error) = result {
            warn!("playback failed: {error}");
            self.show_toast(&gettext("Playback failed"));
        }
        self.update_transport();
    }

    /// Jumps to a position (the `win.go-to-start` action uses frame 0).
    pub(crate) fn seek_to_frame(&self, frame: Frames) {
        let result = self
            .imp()
            .player
            .borrow_mut()
            .as_mut()
            .map(|player| player.seek(frame));
        if let Some(Err(error)) = result {
            warn!("seeking failed: {error}");
        }
        // After a deliberate jump, make sure the playhead is in view.
        self.imp().waveform.set_playhead(frame, true);
        self.update_transport();
    }

    /// Jumps to a position in seconds (used by `--seek`).
    pub(crate) fn seek_to_seconds(&self, seconds: f64) {
        let rate = self.imp().player.borrow().as_ref().map(Player::sample_rate);
        if let Some(frame) = rate.and_then(|rate| rate.seconds_to_frames(seconds.max(0.0))) {
            self.seek_to_frame(frame);
        }
    }

    /// Brings the controls up to date with the player, and handles its events.
    fn update_transport(&self) {
        let imp = self.imp();
        let (state, events) = {
            let mut player = imp.player.borrow_mut();
            let Some(player) = player.as_mut() else {
                return;
            };
            let events = player.poll_events();
            let state = (player.position(), player.sample_rate(), player.is_playing());
            (state, events)
        };
        let (position, rate, playing) = state;

        for event in events {
            if let PlayerEvent::Error(message) = event {
                warn!("playback error: {message}");
                self.show_toast(&gettext("Playback failed"));
            }
        }

        imp.position_label
            .set_label(&format_position(position, rate));
        // While playing, the view scrolls along with the playhead.
        imp.waveform.set_playhead(position, playing);
        let (icon, tooltip) = if playing {
            ("media-playback-pause-symbolic", gettext("Pause"))
        } else {
            ("media-playback-start-symbolic", gettext("Play"))
        };
        imp.play_button.set_icon_name(icon);
        imp.play_button.set_tooltip_text(Some(&tooltip));
    }

    fn set_playback_enabled(&self, enabled: bool) {
        self.action_set_enabled("win.play-pause", enabled);
        self.action_set_enabled("win.go-to-start", enabled);
        self.action_set_enabled("win.zoom-in", enabled);
        self.action_set_enabled("win.zoom-out", enabled);
        self.action_set_enabled("win.zoom-fit", enabled);
    }
}
