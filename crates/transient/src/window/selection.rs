//! The time selection: a stretch of time over some tracks, dragged in the
//! timeline (or Ctrl+A for everything), and what Delete, Ctrl+Delete and S do
//! with it.
//!
//! A clip selection and a time selection never exist at the same time:
//! selecting one clears the other. The window keeps the selection as track IDs
//! (not lane numbers), so it stays right when tracks are removed or undone; the
//! timeline gets it as lanes to draw.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use transient_core::project::TrackId;
use transient_core::range::TimeRange;
use transient_core::time::format_position;

use super::Window;
use crate::widgets::timeline_view::LaneRange;

/// A stretch of time over some tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimeSelection {
    pub(crate) range: TimeRange,
    pub(crate) tracks: Vec<TrackId>,
}

impl Window {
    /// Listens for time selections dragged in the timeline. Called once.
    pub(super) fn setup_time_selection(&self) {
        let weak = self.downgrade();
        self.imp()
            .timeline
            .connect_range_selected(move |_, selection| {
                if let Some(window) = weak.upgrade() {
                    let tracks: Vec<TrackId> = {
                        let project = window.imp().project.borrow();
                        project
                            .tracks
                            .get(selection.first_lane..=selection.last_lane)
                            .unwrap_or_default()
                            .iter()
                            .map(|track| track.id)
                            .collect()
                    };
                    window.set_time_selection(Some(TimeSelection {
                        range: selection.range,
                        tracks,
                    }));
                    // Playback starts at the selection, as in other audio editors.
                    window.seek_to_frame(selection.range.start);
                }
            });
    }

    /// Selects a stretch of time (or nothing). Clears the clip selection.
    pub(crate) fn set_time_selection(&self, selection: Option<TimeSelection>) {
        let selection = selection.filter(|s| !s.range.is_empty() && !s.tracks.is_empty());
        if selection.is_some() {
            self.select(None);
        }
        self.imp().time_selection.replace(selection);
        self.show_time_selection();
        self.update_edit_actions();
    }

    /// Passes the time selection to the timeline, as lanes. Also called after
    /// every project change: tracks that are gone drop out of the selection.
    pub(super) fn show_time_selection(&self) {
        let imp = self.imp();
        let lanes = {
            let project = imp.project.borrow();
            let mut selection = imp.time_selection.borrow_mut();
            if let Some(current) = selection.as_mut() {
                current
                    .tracks
                    .retain(|id| project.tracks.iter().any(|track| track.id == *id));
            }
            if selection.as_ref().is_some_and(|s| s.tracks.is_empty()) {
                *selection = None;
            }
            selection.as_ref().and_then(|selection| {
                let lanes: Vec<usize> = selection
                    .tracks
                    .iter()
                    .filter_map(|id| project.tracks.iter().position(|track| track.id == *id))
                    .collect();
                Some(LaneRange {
                    range: selection.range,
                    first_lane: *lanes.iter().min()?,
                    last_lane: *lanes.iter().max()?,
                })
            })
        };
        imp.timeline.set_time_selection(lanes);
        self.show_selection_times();
    }

    /// Shows the time selection's start, end and length in the bottom bar (or
    /// hides the label when there is no selection).
    fn show_selection_times(&self) {
        let imp = self.imp();
        let rate = imp.project.borrow().sample_rate;
        let text = imp.time_selection.borrow().as_ref().map(|selection| {
            let range = selection.range;
            gettext("Selected {start}–{end} ({length})")
                .replace("{start}", &format_position(range.start, rate))
                .replace("{end}", &format_position(range.end, rate))
                .replace("{length}", &format_position(range.length(), rate))
        });
        imp.selection_label.set_visible(text.is_some());
        imp.selection_label.set_label(&text.unwrap_or_default());
    }

    /// The time selection, if there is one.
    pub(super) fn time_selection(&self) -> Option<TimeSelection> {
        self.imp().time_selection.borrow().clone()
    }

    /// Selects everything on every track (`win.select-all`, Ctrl+A).
    pub(super) fn select_all(&self) {
        let selection = {
            let project = self.imp().project.borrow();
            TimeSelection {
                range: TimeRange {
                    start: 0,
                    end: project.length(),
                },
                tracks: project.track_ids(),
            }
        };
        self.set_time_selection(Some(selection));
    }

    /// Selects `start..end` seconds on every track (for `--select`).
    pub(crate) fn select_seconds(&self, start: f64, end: f64) {
        let rate = self.imp().project.borrow().sample_rate;
        let frame = |seconds: f64| rate.seconds_to_frames(seconds.max(0.0));
        let (Some(start), Some(end)) = (frame(start), frame(end)) else {
            return;
        };
        let tracks = self.imp().project.borrow().track_ids();
        self.set_time_selection(Some(TimeSelection {
            range: TimeRange::between(start, end),
            tracks,
        }));
    }

    /// Delete with a time selection: cuts it out, leaving a gap. Returns false
    /// when there is no time selection (Delete then deletes the selected clip).
    pub(super) fn delete_time_selection(&self) -> bool {
        let Some(selection) = self.time_selection() else {
            return false;
        };
        self.apply_edit(|project| project.delete_range(&selection.tracks, selection.range));
        self.set_time_selection(None);
        self.show_undo_toast(&gettext("Selection deleted"));
        true
    }

    /// Ripple delete (`win.ripple-delete`, Ctrl+Delete): cuts the time selection
    /// out and closes the gap on every track. With only a clip selected, it cuts
    /// that clip's stretch of time on its track.
    pub(super) fn ripple_delete(&self) {
        let selection = self.time_selection().or_else(|| {
            let id = self.imp().selected.get()?;
            let project = self.imp().project.borrow();
            let (track, clip) = project.clip(id)?;
            Some(TimeSelection {
                range: TimeRange {
                    start: clip.start,
                    end: clip.end(),
                },
                tracks: vec![track],
            })
        });
        let Some(selection) = selection else {
            return;
        };
        if self.apply_edit(|project| project.ripple_delete(&selection.tracks, selection.range)) {
            self.set_time_selection(None);
            self.select(None);
            self.seek_to_frame(selection.range.start);
            self.show_undo_toast(&gettext("Deleted and closed the gap"));
        }
    }

    /// S with a time selection: splits at both of its ends. Returns false when
    /// there is no time selection (S then splits at the playhead).
    pub(super) fn split_time_selection(&self) -> bool {
        let Some(selection) = self.time_selection() else {
            return false;
        };
        self.apply_edit(|project| project.split_range(&selection.tracks, selection.range));
        true
    }

    /// Shows a toast with an Undo button, for a change that removed something.
    pub(super) fn show_undo_toast(&self, title: &str) {
        let toast = adw::Toast::builder()
            .title(title)
            .button_label(gettext("_Undo"))
            .action_name("win.undo")
            .build();
        self.imp().toast_overlay.add_toast(toast.clone());
        self.imp().undo_toast.replace(Some(toast));
    }
}
