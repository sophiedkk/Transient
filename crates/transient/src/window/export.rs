//! Exporting the mix (`win.export`, Ctrl+E): opens the export dialog with a
//! snapshot of the current mix, and reports the result with a toast.

use std::path::Path;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::gio;
use tracing::{info, warn};
use transient_core::mix::{AudioPool, Mix};
use transient_core::report::Report;
use transient_engine::export::ExportFormat;

use super::Window;
use crate::widgets::export_dialog::{ExportDialog, ExportOutcome};

impl Window {
    /// Opens the export dialog.
    pub(super) fn show_export_dialog(&self) {
        self.export_dialog().present(Some(self));
    }

    /// Exports straight to `path` without asking (for `--export-to`). The format
    /// follows the extension.
    pub(crate) async fn export_to(&self, path: &Path) {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        let Some(format) = ExportFormat::from_extension(extension) else {
            warn!("unknown export format for {}", path.display());
            return;
        };
        let dialog = self.export_dialog();
        dialog.present(Some(self));
        dialog.export_to(gio::File::for_path(path), format).await;
    }

    /// A dialog holding a snapshot of the current mix.
    fn export_dialog(&self) -> ExportDialog {
        let imp = self.imp();
        let pool: AudioPool = imp
            .sources
            .borrow()
            .iter()
            .map(|(id, source)| (*id, source.waveform.buffer().clone()))
            .collect();
        let project = imp.project.borrow();
        let mix = Mix::new(&project, &pool);
        // The time selection: its stretch of time, on its tracks only.
        let selection = self.time_selection().map(|selection| {
            let mut selected = project.clone();
            selected
                .tracks
                .retain(|track| selection.tracks.contains(&track.id));
            Mix::new(&selected, &pool).excerpt(selection.range)
        });
        let weak = self.downgrade();
        ExportDialog::new(mix, selection, &self.project_title(), move |outcome| {
            if let Some(window) = weak.upgrade() {
                window.export_finished(outcome);
            }
        })
    }

    fn export_finished(&self, outcome: ExportOutcome) {
        match outcome {
            ExportOutcome::Done(file) => {
                info!("exported {}", file.uri());
                let name = file
                    .basename()
                    .map(|name| name.display().to_string())
                    .unwrap_or_default();
                let toast = adw::Toast::builder()
                    .title(gettext("Exported “{}”").replace("{}", &name))
                    .button_label(gettext("Show in _Folder"))
                    .build();
                let weak = self.downgrade();
                toast.connect_button_clicked(move |_| {
                    let launcher = gtk::FileLauncher::new(Some(&file));
                    let parent = weak.upgrade();
                    launcher.open_containing_folder(
                        parent.as_ref(),
                        None::<&gio::Cancellable>,
                        |result| {
                            if let Err(error) = result {
                                warn!("could not show the folder: {error}");
                            }
                        },
                    );
                });
                self.imp().toast_overlay.add_toast(toast);
            }
            ExportOutcome::Cancelled => self.show_toast(&gettext("Export stopped")),
            ExportOutcome::NotLocal => {
                self.export_failed("the chosen file is not on this computer");
            }
            ExportOutcome::Failed(error) => self.export_failed(Report(&error)),
            ExportOutcome::Crashed => self.export_failed("the export stopped unexpectedly"),
        }
    }

    /// Logs why an export failed and tells the user it did.
    fn export_failed(&self, reason: impl std::fmt::Display) {
        warn!("export failed: {reason}");
        self.show_toast(&gettext("Export failed"));
    }
}
