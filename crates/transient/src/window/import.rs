//! Importing audio: the file chooser, decoding in the background, and showing
//! the result.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::gio;
use tracing::{info, warn};
use transient_core::waveform::Waveform;
use transient_engine::decode::{DecodedAudio, SourceFormat, decode_file};

use super::Window;
use super::project::LoadedSource;

impl Window {
    /// Asks the user for audio files (the `win.import-audio` action), then imports them.
    pub(super) async fn choose_and_import(&self) {
        let audio = gtk::FileFilter::new();
        audio.set_name(Some(&gettext("Audio Files")));
        audio.add_mime_type("audio/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&audio);

        let dialog = gtk::FileDialog::builder()
            .title(gettext("Import Audio"))
            .modal(true)
            .filters(&filters)
            .default_filter(&audio)
            .build();

        match dialog.open_multiple_future(Some(self)).await {
            Ok(files) => {
                let files: Vec<gio::File> = files.iter::<gio::File>().flatten().collect();
                self.import_files(&files).await;
            }
            // Pressing Cancel is not an error.
            Err(error) if error.matches(gtk::DialogError::Dismissed) => {}
            Err(error) => {
                warn!("could not open the file chooser: {error}");
                self.show_toast(&gettext("Could not open the file chooser"));
            }
        }
    }

    /// Imports audio files, each onto a new track: from the file chooser, or from
    /// the command line. Files that cannot be read are reported and skipped.
    pub(crate) async fn import_files(&self, files: &[gio::File]) {
        let paths: Vec<PathBuf> = files.iter().filter_map(gio::File::path).collect();
        if paths.len() < files.len() {
            self.show_toast(&gettext("Only files on this computer can be opened"));
        }
        let [first, ..] = paths.as_slice() else {
            return;
        };

        let imp = self.imp();
        let previous_page = imp.stack.visible_child_name();
        let title = if paths.len() == 1 {
            gettext("Opening “{}”").replace("{}", &display_name(first))
        } else {
            let count = u32::try_from(paths.len()).unwrap_or(u32::MAX);
            ngettext("Opening {} File", "Opening {} Files", count)
                .replace("{}", &paths.len().to_string())
        };
        imp.loading_page.set_title(&title);
        imp.stack.set_visible_child_name("loading");

        // Decoding (and computing the waveform overviews) takes a while for long
        // files, so it runs on a background thread. `.await` keeps the window
        // responsive in the meantime.
        let sample_rate = imp.project.borrow().sample_rate;
        let to_decode = paths.clone();
        let results = gio::spawn_blocking(move || {
            to_decode
                .into_iter()
                .map(|path| {
                    info!("importing {}", path.display());
                    let decoded = decode_file(&path, sample_rate).map(|decoded| {
                        let waveform = Waveform::new(decoded.buffer.clone());
                        (decoded, waveform)
                    });
                    (path, decoded)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();

        let mut failed = Vec::new();
        for (path, decoded) in results {
            match decoded {
                Ok((decoded, waveform)) => self.add_to_project(path, &decoded, waveform),
                Err(error) => {
                    warn!("could not open {}: {error}", path.display());
                    failed.push(display_name(&path));
                }
            }
        }
        match failed.as_slice() {
            [] => {}
            [name] => self.show_toast(&gettext("Could not open “{}”").replace("{}", name)),
            names => {
                let count = u32::try_from(names.len()).unwrap_or(u32::MAX);
                self.show_toast(
                    &ngettext("Could not open {} file", "Could not open {} files", count)
                        .replace("{}", &names.len().to_string()),
                );
            }
        }

        if imp.project.borrow().tracks.is_empty() {
            if let Some(page) = previous_page {
                imp.stack.set_visible_child_name(&page);
            }
        } else {
            self.project_changed();
        }
    }

    /// Adds a decoded file to the project as a new track.
    fn add_to_project(&self, path: PathBuf, decoded: &DecodedAudio, waveform: Waveform) {
        let imp = self.imp();
        let name = path.file_stem().map_or_else(
            || display_name(&path),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let frames = decoded.buffer.frames();
        let track = imp.project.borrow_mut().import(path, frames, name);
        let Some(source) = imp
            .project
            .borrow()
            .track(track)
            .and_then(|track| track.clips.first())
            .map(|clip| clip.source)
        else {
            return;
        };
        imp.sources.borrow_mut().insert(
            source,
            LoadedSource {
                waveform,
                format: decoded.source_format,
            },
        );
    }
}

/// The file name to show, for example "drum-loop.wav".
fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A short description such as "44.1 kHz · Stereo".
pub(super) fn describe_format(format: Option<SourceFormat>) -> String {
    let Some(format) = format else {
        return gettext("Unknown");
    };
    // Whole kHz, then the remaining Hz without trailing zeros: 48, 44.1, 11.025.
    let khz = format.sample_rate / 1000;
    let hz = format.sample_rate % 1000;
    let rate = if hz == 0 {
        format!("{khz} kHz")
    } else {
        let decimals = format!("{hz:03}");
        format!("{khz}.{} kHz", decimals.trim_end_matches('0'))
    };
    let channels = match format.channels {
        1 => gettext("Mono"),
        2 => gettext("Stereo"),
        n => ngettext("{} channel", "{} channels", n).replace("{}", &n.to_string()),
    };
    format!("{rate} · {channels}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_described_briefly() {
        let format = |sample_rate, channels| {
            describe_format(Some(SourceFormat {
                sample_rate,
                channels,
            }))
        };
        assert_eq!(format(44_100, 2), "44.1 kHz · Stereo");
        assert_eq!(format(48_000, 1), "48 kHz · Mono");
        assert_eq!(format(22_050, 2), "22.05 kHz · Stereo");
        assert_eq!(format(11_025, 6), "11.025 kHz · 6 channels");
        assert_eq!(describe_format(None), "Unknown");
    }

    #[test]
    fn display_name_is_the_file_name() {
        assert_eq!(display_name(Path::new("/music/song.flac")), "song.flac");
    }
}
