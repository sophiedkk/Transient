//! The Preferences dialog (`win.preferences`, Ctrl+, or the main menu).
//!
//! For now it holds one setting: the recording offset, to fine-tune where takes
//! land when the delay Transient measures is not the whole story (see
//! `transient_core::take`). Settings take effect straight away; there is no
//! Apply button, as the GNOME HIG asks.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use tracing::warn;

use super::Window;

impl Window {
    /// Shows the Preferences dialog.
    pub(super) fn show_preferences(&self) {
        let Some(settings) = self.imp().settings.get().cloned() else {
            self.show_toast(&gettext("Preferences are not available"));
            return;
        };
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

        let recording = adw::PreferencesGroup::builder()
            .title(gettext("Recording"))
            .description(gettext(
                "Recordings are lined up with what was playing while they were \
                 recorded, using the delay Transient can measure. If they still \
                 sound early or late, adjust them here.",
            ))
            .build();
        recording.add(&offset);
        let page = adw::PreferencesPage::new();
        page.add(&recording);
        let dialog = adw::PreferencesDialog::new();
        dialog.add(&page);
        dialog.present(Some(self));
    }
}
