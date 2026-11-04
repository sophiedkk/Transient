//! Project files: new, open, save, save as, unsaved changes and recent projects.
//!
//! A project file (`.transient`) holds the project and a copy of all its audio
//! (see `transient_core::archive`), so it can be moved, backed up or shared as
//! one file, and never loses its audio. Opening unpacks the audio into a
//! temporary folder in the user's cache, which is deleted again when the window
//! closes or opens another project. Saving and opening run on a background
//! thread: copying audio can take a moment.
//!
//! Unsaved changes are found by comparing the project with the one last saved
//! (or opened). Closing or replacing a project with unsaved changes asks first:
//! the one place where the GNOME HIG wants a confirmation, because otherwise
//! work would be lost for good.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use tracing::{info, warn};
use transient_core::archive;
use transient_core::project::Project;
use transient_core::waveform::Waveform;
use transient_engine::decode::decode_file;

use super::Window;
use super::project::{LoadedSource, LoadedSources};

/// The project file extension.
pub(crate) const PROJECT_EXTENSION: &str = "transient";
/// The project file MIME type (see data/io.github.sophiedkk.Transient.mime.xml).
const PROJECT_MIME_TYPE: &str = "application/x-transient-project";

/// Whether `file` is a Transient project (judged by its extension).
pub(crate) fn is_project_file(file: &gio::File) -> bool {
    file.path()
        .and_then(|path| path.extension().map(|ext| ext == PROJECT_EXTENSION))
        .unwrap_or(false)
}

impl Window {
    /// Whether the project has changed since it was last saved or opened.
    pub(super) fn has_unsaved_changes(&self) -> bool {
        let imp = self.imp();
        *imp.project.borrow() != *imp.saved_project.borrow()
    }

    /// The project's display name: its file name, or "Untitled Project".
    pub(super) fn project_title(&self) -> String {
        self.imp()
            .file
            .borrow()
            .as_ref()
            .and_then(gio::File::path)
            .and_then(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| gettext("Untitled Project"))
    }

    /// Saves to the current file, or asks for one (`win.save`, Ctrl+S).
    /// Returns whether the project was saved.
    pub(super) async fn save(&self) -> bool {
        let file = self.imp().file.borrow().clone();
        match file {
            Some(file) => self.write_project(&file).await,
            None => self.save_as().await,
        }
    }

    /// Asks where to save, then saves (`win.save-as`, Shift+Ctrl+S).
    pub(super) async fn save_as(&self) -> bool {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Save Project"))
            .modal(true)
            .initial_name(format!("{}.{PROJECT_EXTENSION}", self.project_title()))
            .filters(&project_filters())
            .build();
        match dialog.save_future(Some(self)).await {
            Ok(file) => self.write_project(&with_project_extension(&file)).await,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    warn!("could not open the save dialog: {error}");
                }
                false
            }
        }
    }

    /// Writes the project, with its audio, to `file` (used by `--save-to` too).
    pub(crate) async fn write_project(&self, file: &gio::File) -> bool {
        let imp = self.imp();
        let Some(path) = file.path() else {
            self.show_toast(&gettext("Projects can only be saved on this computer"));
            return false;
        };
        let project = imp.project.borrow().clone();
        let to_save = project.clone();
        let result = gio::spawn_blocking(move || archive::save(&to_save, &path)).await;
        let not_included = match result {
            Ok(Ok(not_included)) => not_included,
            Ok(Err(error)) => {
                warn!("could not save to {}: {error}", file.uri());
                self.show_toast(&gettext("Could not save the project"));
                return false;
            }
            Err(_) => {
                warn!("saving {} stopped unexpectedly", file.uri());
                self.show_toast(&gettext("Could not save the project"));
                return false;
            }
        };
        if !not_included.is_empty() {
            // Audio that was missing already: the project still points at it.
            let count = u32::try_from(not_included.len()).unwrap_or(u32::MAX);
            self.show_toast(
                &ngettext(
                    "Saved without {} missing audio file",
                    "Saved without {} missing audio files",
                    count,
                )
                .replace("{}", &not_included.len().to_string()),
            );
        }
        info!("saved {}", file.uri());
        imp.saved_project.replace(project);
        imp.file.replace(Some(file.clone()));
        add_to_recent(file);
        self.project_changed();
        true
    }

    /// Asks for a project file and opens it (`win.open-project`, Ctrl+O).
    pub(super) async fn open_project_dialog(&self) {
        if !self.confirm_discard_changes().await {
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open Project"))
            .modal(true)
            .filters(&project_filters())
            .build();
        match dialog.open_future(Some(self)).await {
            Ok(file) => self.open_project(&file).await,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    warn!("could not open the file chooser: {error}");
                }
            }
        }
    }

    /// Opens a project file, replacing the current project. Audio files that
    /// cannot be found are reported in a banner; their clips stay, greyed out.
    pub(crate) async fn open_project(&self, file: &gio::File) {
        let imp = self.imp();
        let Some(path) = file.path() else {
            self.show_toast(&gettext("Only projects on this computer can be opened"));
            return;
        };
        info!("opening project {}", path.display());
        let previous_page = imp.stack.visible_child_name();
        imp.loading_page
            .set_title(&gettext("Opening “{}”").replace("{}", &display_name(&path)));
        imp.stack.set_visible_child_name("loading");

        let opened = match new_project_cache() {
            Ok(cache) => {
                let (file_path, cache_path) = (path.clone(), cache.path().to_owned());
                gio::spawn_blocking(move || archive::open(&file_path, &cache_path))
                    .await
                    // The thread panicked; its payload says nothing useful to show.
                    .map_err(|_panic| "opening stopped unexpectedly".to_owned())
                    .and_then(|result| result.map_err(|error| error.to_string()))
                    .map(|project| (project, cache))
            }
            Err(error) => Err(format!("no folder for its audio: {error}")),
        };
        let (project, cache) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                warn!("could not open {}: {error}", path.display());
                if let Some(page) = previous_page {
                    imp.stack.set_visible_child_name(&page);
                }
                self.show_toast(
                    &gettext("Could not open “{}”").replace("{}", &display_name(&path)),
                );
                return;
            }
        };
        let (sources, missing) = load_sources(&project).await;
        // The old project's unpacked audio is deleted with its folder.
        imp.project_cache.replace(Some(cache));

        imp.project.replace(project.clone());
        imp.saved_project.replace(project);
        imp.sources.replace(sources);
        imp.file.replace(Some(file.clone()));
        imp.history.take();
        self.clear_clipboard();
        self.select(None);
        add_to_recent(file);
        self.show_missing_sources(&missing);
        self.project_changed();
    }

    /// Starts an empty project (`win.new-project`, Ctrl+N).
    pub(super) async fn new_project(&self) {
        if !self.confirm_discard_changes().await {
            return;
        }
        let imp = self.imp();
        imp.project.replace(Project::default());
        imp.saved_project.replace(Project::default());
        imp.sources.replace(LoadedSources::new());
        imp.file.replace(None);
        imp.history.take();
        imp.project_cache.replace(None);
        self.clear_clipboard();
        self.select(None);
        self.show_missing_sources(&[]);
        self.project_changed();
    }

    /// If there are unsaved changes, asks whether to save them. Returns whether
    /// it is fine to go on (saved, discarded, or nothing to save).
    pub(super) async fn confirm_discard_changes(&self) -> bool {
        if !self.has_unsaved_changes() {
            return true;
        }
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Save Changes?"))
            .body(gettext(
                "The project has unsaved changes. Changes which are not saved will be permanently lost.",
            ))
            .close_response("cancel")
            .default_response("save")
            .build();
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("discard", &gettext("_Discard")),
            ("save", &gettext("_Save")),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        match dialog.choose_future(Some(self)).await.as_str() {
            "discard" => true,
            "save" => self.save().await,
            _ => false,
        }
    }

    /// Fills the start screen's "Recent Projects" list.
    pub(super) fn update_recent_projects(&self) {
        let imp = self.imp();
        let list = &imp.recent_list;
        list.remove_all();
        let mut items: Vec<gtk::RecentInfo> = gtk::RecentManager::default()
            .items()
            .into_iter()
            .filter(|item| item.uri().ends_with(&format!(".{PROJECT_EXTENSION}")) && item.exists())
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.modified().to_unix()));
        items.truncate(5);
        imp.recent_group.set_visible(!items.is_empty());
        for item in items {
            let file = gio::File::for_uri(&item.uri());
            let folder = file
                .parent()
                .and_then(|parent| parent.path())
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            let name = file.path().map_or_else(
                || item.display_name().to_string(),
                |path| display_name(&path),
            );
            let row = adw::ActionRow::builder()
                .title(name.trim_end_matches(&format!(".{PROJECT_EXTENSION}")))
                .subtitle(folder)
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let weak = self.downgrade();
            row.connect_activated(move |_| {
                let (Some(window), file) = (weak.upgrade(), file.clone()) else {
                    return;
                };
                glib::spawn_future_local(async move {
                    if window.confirm_discard_changes().await {
                        window.open_project(&file).await;
                    }
                });
            });
            list.append(&row);
        }
    }

    /// Shows or hides the "missing audio" banner.
    fn show_missing_sources(&self, missing: &[PathBuf]) {
        let banner = &self.imp().missing_banner;
        if missing.is_empty() {
            banner.set_revealed(false);
            return;
        }
        let count = u32::try_from(missing.len()).unwrap_or(u32::MAX);
        banner.set_title(
            &ngettext(
                "{} audio file could not be found",
                "{} audio files could not be found",
                count,
            )
            .replace("{}", &missing.len().to_string()),
        );
        banner.set_revealed(true);
        for path in missing {
            warn!("missing audio file: {}", path.display());
        }
    }
}

/// Decodes a project's sources in the background. Returns the decoded ones and
/// The folder holding an open project's unpacked audio. It is deleted when this
/// is dropped (another project opened, the window closed). While it exists, the
/// `in-use` file in it is locked, so another Transient never removes it (see
/// [`remove_unused_project_caches`]).
#[derive(Debug)]
pub(super) struct ProjectCache {
    // Fields drop in order: unlock first, then delete the folder.
    _lock: std::fs::File,
    folder: tempfile::TempDir,
}

impl ProjectCache {
    fn path(&self) -> &Path {
        self.folder.path()
    }
}

/// Where the unpacked audio of open projects goes: `~/.cache/transient`.
fn cache_base() -> PathBuf {
    glib::user_cache_dir().join("transient")
}

/// A new, empty, locked folder for an opened project's audio.
fn new_project_cache() -> std::io::Result<ProjectCache> {
    let base = cache_base();
    std::fs::create_dir_all(&base)?;
    let folder = tempfile::Builder::new()
        .prefix("project-")
        .tempdir_in(base)?;
    let lock = std::fs::File::create(folder.path().join(IN_USE_FILE))?;
    lock.try_lock().map_err(std::io::Error::other)?;
    Ok(ProjectCache {
        _lock: lock,
        folder,
    })
}

/// The file in a project cache folder that its owner keeps locked.
const IN_USE_FILE: &str = "in-use";

/// Removes project cache folders left behind by a Transient that did not get
/// to clean up (it quit abruptly). A folder is only removed when nobody holds
/// the lock on its `in-use` file, so folders of running Transients stay.
pub(crate) fn remove_unused_project_caches() {
    let Ok(entries) = std::fs::read_dir(cache_base()) else {
        return;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        let is_project = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("project-"));
        if !is_project {
            continue;
        }
        let unused = std::fs::File::open(folder.join(IN_USE_FILE))
            .map_or(true, |file| file.try_lock().is_ok());
        if unused {
            match std::fs::remove_dir_all(&folder) {
                Ok(()) => info!("removed unused {}", folder.display()),
                Err(error) => warn!("could not remove {}: {error}", folder.display()),
            }
        }
    }
}

/// the paths that could not be read.
async fn load_sources(project: &Project) -> (LoadedSources, Vec<PathBuf>) {
    let wanted: Vec<_> = project
        .sources
        .iter()
        .map(|source| (source.id, source.path.clone()))
        .collect();
    let rate = project.sample_rate;
    gio::spawn_blocking(move || {
        let mut loaded = LoadedSources::new();
        let mut missing = Vec::new();
        for (id, path) in wanted {
            match decode_file(&path, rate) {
                Ok(decoded) => {
                    let source = LoadedSource {
                        waveform: Waveform::new(decoded.buffer),
                        format: decoded.source_format,
                    };
                    loaded.insert(id, source);
                }
                Err(error) => {
                    warn!("could not load {}: {error}", path.display());
                    missing.push(path);
                }
            }
        }
        (loaded, missing)
    })
    .await
    .unwrap_or_default()
}

/// File chooser filters: Transient projects only.
fn project_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&gettext("Transient Projects")));
    filter.add_mime_type(PROJECT_MIME_TYPE);
    filter.add_suffix(PROJECT_EXTENSION);
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    filters
}

/// Adds `.transient` if the chosen file name has no extension.
fn with_project_extension(file: &gio::File) -> gio::File {
    match file.path() {
        Some(path) if path.extension().is_none() => {
            gio::File::for_path(path.with_extension(PROJECT_EXTENSION))
        }
        _ => file.clone(),
    }
}

/// Remembers a project in GNOME's recent files (also shown on the start screen).
fn add_to_recent(file: &gio::File) {
    gtk::RecentManager::default().add_item(&file.uri());
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
