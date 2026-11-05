//! The Preferences dialog (`win.preferences`, Ctrl+, or the main menu).
//!
//! It holds the recording offset, to fine-tune where takes land when the delay
//! Transient measures is not the whole story (see `transient_core::take`), and
//! the format the Export dialog starts with. That format is also remembered by
//! the Export dialog itself (the last one exported to), so most people never
//! need to come here for it. Settings take effect straight away; there is no
//! Apply button, as the GNOME HIG asks.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::gio;
use tracing::warn;

use super::Window;
use crate::widgets::export_dialog::{save_format_choice, setup_format_rows};

impl Window {
    /// Shows the Preferences dialog.
    pub(super) fn show_preferences(&self) {
        let Some(settings) = self.imp().settings.get().cloned() else {
            self.show_toast(&gettext("Preferences are not available"));
            return;
        };
        let page = adw::PreferencesPage::new();
        page.add(&recording_group(settings.clone()));
        page.add(&export_group(&settings));
        let dialog = adw::PreferencesDialog::new();
        dialog.add(&page);
        dialog.present(Some(self));
    }
}

/// The "Recording" group: the recording offset.
fn recording_group(settings: gio::Settings) -> adw::PreferencesGroup {
    let offset = adw::SpinRow::with_range(-1000.0, 1000.0, 1.0);
    offset.set_title(&gettext("Recording Offset"));
    offset.set_subtitle(&gettext(
        "In milliseconds; negative moves recordings earlier",
    ));
    offset.set_value(f64::from(settings.int("recording-offset")));
    offset.connect_value_notify(move |row| {
        // The row only allows whole numbers from −1000 to 1000.
        let value = transient_core::convert::i32_from_f64(row.value());
        if let Err(error) = settings.set_int("recording-offset", value) {
            warn!("could not save the recording offset: {error}");
        }
    });

    let group = adw::PreferencesGroup::builder()
        .title(gettext("Recording"))
        .description(gettext(
            "Recordings are lined up with what was playing while they were \
             recorded, using the delay Transient can measure. If they still \
             sound early or late, adjust them here.",
        ))
        .build();
    group.add(&offset);
    group
}

/// The "Export" group: the format and quality the Export dialog starts with.
fn export_group(settings: &gio::Settings) -> adw::PreferencesGroup {
    let format_row = adw::ComboRow::builder().title(gettext("Format")).build();
    let quality_row = adw::ComboRow::builder().title(gettext("Quality")).build();
    setup_format_rows(&format_row, &quality_row, Some(settings));
    // Saved on every change. For a new format this runs after
    // `setup_format_rows` has refilled the qualities (signal handlers run in
    // the order they were connected), so the quality saved is the new one.
    // The handlers hold weak references to the rows: a row whose own handler
    // held it strongly would never be freed.
    for row in [&format_row, &quality_row] {
        let settings = settings.clone();
        let (format_weak, quality_weak) = (format_row.downgrade(), quality_row.downgrade());
        row.connect_selected_notify(move |_| {
            if let (Some(format_row), Some(quality_row)) =
                (format_weak.upgrade(), quality_weak.upgrade())
            {
                save_format_choice(&settings, &format_row, &quality_row);
            }
        });
    }

    let group = adw::PreferencesGroup::builder()
        .title(gettext("Export"))
        .description(gettext(
            "The format the Export dialog starts with. Exporting to another \
             format makes that one the new start.",
        ))
        .build();
    group.add(&format_row);
    group.add(&quality_row);
    group
}
