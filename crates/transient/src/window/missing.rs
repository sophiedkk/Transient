//! Audio files a project cannot find: the "missing audio" banner and its
//! Locate… button (`win.locate-missing`).
//!
//! Only projects saved before format 2 (which pointed at audio files instead of
//! holding them) and audio that was already missing when a project was saved
//! can lack audio. A source is missing when clips play it but its audio could
//! not be decoded; those clips are drawn grey.
//!
//! Locating a file gives its clips a new source with the new path (see
//! `Project::replace_source`), as one undo step. The decoded audio is kept
//! under the new source's ID, so undoing makes the file missing again and
//! redoing finds it once more, without decoding it again.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::gio;
use tracing::{info, warn};
use transient_core::project::{Source, SourceId};
use transient_core::report::Report;
use transient_core::waveform::Waveform;
use transient_engine::decode::{DecodeError, SourceFormat, decode_file};

use super::Window;
use super::import::{audio_filters, display_name};
use super::project::LoadedSource;

/// A file decoded for a missing source: the source, the file, and its audio.
type Found = (
    SourceId,
    PathBuf,
    Result<(Waveform, Option<SourceFormat>), DecodeError>,
);

impl Window {
    /// The sources that clips play but whose audio is not loaded, in the order
    /// they were added to the project.
    fn missing_sources(&self) -> Vec<Source> {
        let imp = self.imp();
        let project = imp.project.borrow();
        let loaded = imp.sources.borrow();
        // A source no clip plays any more is not worth a warning.
        let played: HashSet<SourceId> = project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.source)
            .collect();
        project
            .sources
            .iter()
            .filter(|source| played.contains(&source.id) && !loaded.contains_key(&source.id))
            .cloned()
            .collect()
    }

    /// Shows the banner while audio is missing and hides it otherwise. Called
    /// from `project_changed`, so it follows undo and redo too.
    pub(super) fn update_missing_banner(&self) {
        let missing = self.missing_sources();
        let banner = &self.imp().missing_banner;
        let title = match missing.as_slice() {
            [] => {
                banner.set_revealed(false);
                return;
            }
            [source] => {
                gettext("“{}” could not be found").replace("{}", &display_name(&source.path))
            }
            sources => {
                let count = u32::try_from(sources.len()).unwrap_or(u32::MAX);
                ngettext(
                    "{} audio file could not be found",
                    "{} audio files could not be found",
                    count,
                )
                .replace("{}", &sources.len().to_string())
            }
        };
        banner.set_title(&title);
        banner.set_revealed(true);
    }

    /// Asks where the first missing file is now, then uses it
    /// (`win.locate-missing`, the banner's Locate… button).
    pub(super) async fn locate_missing(&self) {
        let Some(first) = self.missing_sources().into_iter().next() else {
            return;
        };
        let mut builder = gtk::FileDialog::builder()
            .title(gettext("Locate “{}”").replace("{}", &display_name(&first.path)))
            .modal(true)
            .filters(&audio_filters());
        // Start in the folder the file used to be in, if that is still there.
        if let Some(folder) = first.path.parent().filter(|folder| folder.is_dir()) {
            builder = builder.initial_folder(&gio::File::for_path(folder));
        }
        match builder.build().open_future(Some(self)).await {
            Ok(file) => match file.path() {
                Some(path) => self.locate(&path).await,
                None => self.show_toast(&gettext("Only files on this computer can be opened")),
            },
            // Pressing Cancel is not an error.
            Err(error) if error.matches(gtk::DialogError::Dismissed) => {}
            Err(error) => {
                warn!("could not open the file chooser: {error}");
                self.show_toast(&gettext("Could not open the file chooser"));
            }
        }
    }

    /// Uses `path` for the first missing file (also `--locate`). Files that went
    /// missing together usually moved together, so the other missing files are
    /// looked for in the same folder, by name, and used too if they are there.
    pub(crate) async fn locate(&self, path: &Path) {
        let missing = self.missing_sources();
        let Some((first, others)) = missing.split_first() else {
            return;
        };
        let mut wanted = vec![(first.id, path.to_owned())];
        if let Some(folder) = path.parent() {
            for source in others {
                let candidate = source.path.file_name().map(|name| folder.join(name));
                if let Some(candidate) = candidate.filter(|candidate| candidate.is_file()) {
                    wanted.push((source.id, candidate));
                }
            }
        }

        // Decoding takes a while for long files, so it runs in the background.
        let rate = self.imp().project.borrow().sample_rate;
        let decoded: Vec<Found> = gio::spawn_blocking(move || {
            wanted
                .into_iter()
                .map(|(id, path)| {
                    let audio = decode_file(&path, rate)
                        .map(|decoded| (Waveform::new(decoded.buffer), decoded.source_format));
                    (id, path, audio)
                })
                .collect()
        })
        .await
        .unwrap_or_default();

        let mut found = Vec::new();
        for (id, path, audio) in decoded {
            match audio {
                Ok(audio) => found.push((id, path, audio)),
                Err(error) => {
                    warn!("could not open {}: {}", path.display(), Report(&error));
                    // Only the file the user chose is worth a message.
                    if id == first.id {
                        self.show_toast(
                            &gettext("Could not open “{}”").replace("{}", &display_name(&path)),
                        );
                    }
                }
            }
        }
        if found.is_empty() {
            return;
        }
        // The files found besides the one chosen.
        let more = found.iter().filter(|(id, ..)| *id != first.id).count();
        // One undo step for all files found. The decoded audio goes in while
        // the project changes, so `project_changed` (run by `apply_edit`
        // afterwards) draws the waveforms straight away.
        let sources = &self.imp().sources;
        self.apply_edit(|project| {
            let mut changed = false;
            for (old, path, (waveform, format)) in found {
                info!("located missing audio at {}", path.display());
                let frames = waveform.buffer().frames();
                if let Some(new) = project.replace_source(old, path, frames) {
                    sources
                        .borrow_mut()
                        .insert(new, LoadedSource { waveform, format });
                    changed = true;
                }
            }
            changed
        });
        if more > 0 {
            self.show_toast(
                &ngettext(
                    "Also found {} more file in the same folder",
                    "Also found {} more files in the same folder",
                    u32::try_from(more).unwrap_or(u32::MAX),
                )
                .replace("{}", &more.to_string()),
            );
        }
    }
}
