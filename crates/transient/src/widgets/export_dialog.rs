//! `ExportDialog`: choose a format and quality, choose a file, export with progress.
//!
//! The dialog gets a [`Mix`] (a snapshot of the project) when it opens, so edits
//! made while exporting do not affect the file being written. The export itself
//! runs on a background thread; progress comes back through an `async_channel`,
//! the usual way in gtk-rs to send data from another thread to the UI.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use tracing::{info, warn};
use transient_core::mix::Mix;
use transient_engine::export::{ExportError, ExportFormat, export};

/// What happened to an export, reported to the window.
#[derive(Debug)]
pub(crate) enum ExportOutcome {
    /// The file was written.
    Done(gio::File),
    /// The user stopped it.
    Cancelled,
    /// It failed; the message is for the log.
    Failed(String),
}

type FinishedCallback = Box<dyn Fn(ExportOutcome)>;

/// The quality choices for each format: label and value (bits or kbit/s).
fn qualities(format_index: u32) -> &'static [(&'static str, u32)] {
    match format_index {
        0 | 1 => &[("16-bit", 16), ("24-bit", 24)],
        2 => &[
            ("128 kbps", 128),
            ("192 kbps", 192),
            ("256 kbps", 256),
            ("320 kbps", 320),
        ],
        3 => &[
            ("64 kbps", 64),
            ("96 kbps", 96),
            ("128 kbps", 128),
            ("192 kbps", 192),
        ],
        _ => &[("128 kbps", 128), ("192 kbps", 192), ("256 kbps", 256)],
    }
}

/// The preselected quality for each format: CD quality, or a good bitrate.
fn default_quality(format_index: u32) -> u32 {
    match format_index {
        2 | 4 => 1,
        3 => 2,
        _ => 0,
    }
}

fn format_for(format_index: u32, value: u32) -> ExportFormat {
    let bits = u8::try_from(value).unwrap_or(16);
    match format_index {
        0 => ExportFormat::Wav { bits },
        1 => ExportFormat::Flac { bits },
        2 => ExportFormat::Mp3 { kbps: value },
        3 => ExportFormat::Opus { kbps: value },
        _ => ExportFormat::Vorbis { kbps: value },
    }
}

mod imp {
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use adw::subclass::prelude::*;
    use gtk::{CompositeTemplate, glib};
    use transient_core::mix::Mix;

    use super::FinishedCallback;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/sophiedkk/Transient/ui/export-dialog.ui")]
    pub(crate) struct ExportDialog {
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) export_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub(super) format_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub(super) quality_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub(super) range_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub(super) quality_list: TemplateChild<gtk::StringList>,
        #[template_child]
        pub(super) progress_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub(super) progress_bar: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub(super) stop_button: TemplateChild<gtk::Button>,

        pub(super) mix: RefCell<Option<Mix>>,
        /// The time selection's mix (its stretch, its tracks), if there is one.
        pub(super) selection_mix: RefCell<Option<Mix>>,
        pub(super) suggested_name: RefCell<String>,
        pub(super) cancel: Arc<AtomicBool>,
        pub(super) on_finished: RefCell<Option<FinishedCallback>>,
        pub(super) exporting: Cell<bool>,
    }

    impl std::fmt::Debug for ExportDialog {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ExportDialog")
                .field("exporting", &self.exporting.get())
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ExportDialog {
        const NAME: &'static str = "TransientExportDialog";
        type Type = super::ExportDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ExportDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for ExportDialog {}
    impl AdwDialogImpl for ExportDialog {}
}

glib::wrapper! {
    /// The export dialog.
    pub(crate) struct ExportDialog(ObjectSubclass<imp::ExportDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ExportDialog {
    /// A dialog that exports `mix` (the whole project), or `selection` (the time
    /// selection's mix) if there is one and it is chosen, which it is at first.
    /// It suggests `name` (without extension) as the file name; `on_finished`
    /// hears how it ended.
    pub(crate) fn new(
        mix: Mix,
        selection: Option<Mix>,
        name: &str,
        on_finished: impl Fn(ExportOutcome) + 'static,
    ) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        imp.mix.replace(Some(mix));
        if selection.is_some() {
            imp.range_row.set_visible(true);
            imp.range_row.set_selected(1);
        }
        imp.selection_mix.replace(selection);
        imp.suggested_name.replace(name.to_owned());
        imp.on_finished.replace(Some(Box::new(on_finished)));
        dialog
    }

    fn setup(&self) {
        let imp = self.imp();
        let weak = self.downgrade();
        imp.format_row.connect_selected_notify(move |row| {
            if let Some(dialog) = weak.upgrade() {
                dialog.fill_qualities(row.selected());
            }
        });
        self.fill_qualities(imp.format_row.selected());

        let weak = self.downgrade();
        imp.cancel_button.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
        });
        let weak = self.downgrade();
        imp.export_button.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                glib::spawn_future_local(async move { dialog.choose_file_and_export().await });
            }
        });
        let cancel = Arc::clone(&imp.cancel);
        imp.stop_button
            .connect_clicked(move |_| cancel.store(true, Ordering::Relaxed));
    }

    fn fill_qualities(&self, format_index: u32) {
        let imp = self.imp();
        let labels: Vec<&str> = qualities(format_index)
            .iter()
            .map(|(label, _)| *label)
            .collect();
        imp.quality_list
            .splice(0, imp.quality_list.n_items(), &labels);
        imp.quality_row.set_selected(default_quality(format_index));
    }

    /// The format and quality as chosen.
    pub(crate) fn chosen_format(&self) -> ExportFormat {
        let imp = self.imp();
        let format_index = imp.format_row.selected();
        let value = qualities(format_index)
            .get(usize::try_from(imp.quality_row.selected()).unwrap_or(0))
            .map_or(16, |(_, value)| *value);
        format_for(format_index, value)
    }

    async fn choose_file_and_export(&self) {
        let format = self.chosen_format();
        let name = format!(
            "{}.{}",
            self.imp().suggested_name.borrow(),
            format.extension()
        );
        let file_dialog = gtk::FileDialog::builder()
            .title(gettext("Export"))
            .modal(true)
            .initial_name(name)
            .build();
        let parent = self.root().and_downcast::<gtk::Window>();
        match file_dialog.save_future(parent.as_ref()).await {
            Ok(file) => self.export_to(file, format).await,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    warn!("could not open the save dialog: {error}");
                }
            }
        }
    }

    /// Exports to `file` (also used by `--export-to`, which skips the file chooser).
    pub(crate) async fn export_to(&self, file: gio::File, format: ExportFormat) {
        let imp = self.imp();
        // The selection when it is chosen (the second entry), else the whole project.
        let mix = if imp.range_row.is_visible() && imp.range_row.selected() == 1 {
            imp.selection_mix.borrow().clone()
        } else {
            imp.mix.borrow().clone()
        };
        let (Some(path), Some(mix)) = (file.path(), mix) else {
            self.finish(ExportOutcome::Failed("not a local file".to_owned()));
            return;
        };
        imp.exporting.set(true);
        self.set_can_close(false);
        imp.header_bar_hidden(true);
        imp.progress_page
            .set_description(Some(&display_name(&path)));
        imp.progress_bar.set_fraction(0.0);
        imp.stack.set_visible_child_name("progress");
        info!("exporting to {} as {format:?}", path.display());

        // Progress flows from the export thread to the progress bar.
        let (sender, receiver) = async_channel::bounded::<f64>(16);
        let bar = imp.progress_bar.clone();
        glib::spawn_future_local(async move {
            while let Ok(fraction) = receiver.recv().await {
                bar.set_fraction(fraction);
                bar.set_text(Some(&format!("{:.0} %", fraction * 100.0)));
            }
        });
        let cancel = Arc::clone(&imp.cancel);
        let result = gio::spawn_blocking(move || {
            let mut last_sent = 0.0;
            export(
                &mix,
                &path,
                format,
                |fraction| {
                    // About a hundred updates is plenty for a progress bar.
                    if fraction - last_sent >= 0.01 || fraction >= 1.0 {
                        last_sent = fraction;
                        sender.send_blocking(fraction).unwrap_or_default();
                    }
                },
                &cancel,
            )
        })
        .await;

        let outcome = match result {
            Ok(Ok(())) => ExportOutcome::Done(file),
            Ok(Err(ExportError::Cancelled)) => ExportOutcome::Cancelled,
            Ok(Err(error)) => ExportOutcome::Failed(error.to_string()),
            Err(_panic) => ExportOutcome::Failed("the export crashed".to_owned()),
        };
        self.finish(outcome);
    }

    fn finish(&self, outcome: ExportOutcome) {
        let imp = self.imp();
        imp.exporting.set(false);
        self.set_can_close(true);
        self.close();
        if let Some(callback) = imp.on_finished.borrow().as_ref() {
            callback(outcome);
        }
    }
}

impl imp::ExportDialog {
    /// Hides the Cancel/Export buttons while exporting (Stop is on the page).
    fn header_bar_hidden(&self, hidden: bool) {
        self.cancel_button.set_visible(!hidden);
        self.export_button.set_visible(!hidden);
    }
}

fn display_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
