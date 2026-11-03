//! Importing audio: the file chooser, decoding in the background, and showing
//! the result.

use std::path::Path;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::gio;
use tracing::{info, warn};
use transient_core::time::{SampleRate, format_position};
use transient_core::waveform::Waveform;
use transient_engine::decode::{DecodedAudio, SourceFormat, decode_file};
use transient_engine::player::Player;

use super::Window;

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

    /// Imports audio files: from the file chooser, or from the command line.
    ///
    /// For now only the first file is opened; multiple tracks arrive in milestone 3.
    pub(crate) async fn import_files(&self, files: &[gio::File]) {
        let Some(file) = files.first() else {
            return;
        };
        if files.len() > 1 {
            self.show_toast(&gettext("Only one file can be opened at a time for now"));
        }
        let Some(path) = file.path() else {
            self.show_toast(&gettext("Only files on this computer can be opened"));
            return;
        };
        let name = display_name(&path);
        info!("importing {}", path.display());

        let imp = self.imp();
        let previous_page = imp.stack.visible_child_name();
        imp.loading_page
            .set_title(&gettext("Opening “{}”").replace("{}", &name));
        imp.stack.set_visible_child_name("loading");

        // Decoding (and computing the waveform overview) takes a while for long files,
        // so it runs on a background thread. `.await` keeps the window responsive.
        let decode_path = path.clone();
        let result = gio::spawn_blocking(move || {
            decode_file(&decode_path, SampleRate::default()).map(|decoded| {
                let waveform = Waveform::new(decoded.buffer.clone());
                (decoded, waveform)
            })
        })
        .await;

        let error = match result {
            Ok(Ok((decoded, waveform))) => match self.show_audio(&name, &path, decoded, waveform) {
                Ok(()) => return,
                Err(error) => error.to_string(),
            },
            Ok(Err(error)) => error.to_string(),
            Err(_panic) => "the decoder crashed".to_owned(),
        };
        warn!("could not open {}: {error}", path.display());
        self.show_toast(&gettext("Could not open “{}”").replace("{}", &name));
        if let Some(page) = previous_page {
            imp.stack.set_visible_child_name(&page);
        }
    }

    /// Replaces whatever was open with the decoded file.
    fn show_audio(
        &self,
        name: &str,
        path: &Path,
        decoded: DecodedAudio,
        waveform: Waveform,
    ) -> Result<(), transient_engine::EngineError> {
        let imp = self.imp();
        let frames = decoded.buffer.frames();
        let rate = decoded.buffer.sample_rate();
        let player = Player::new(decoded.buffer, imp.audio_output.get())?;
        // Replacing the old player drops it, which stops its playback.
        imp.player.replace(Some(player));

        self.set_title(Some(name));
        imp.header_title.set_title(name);
        imp.header_title.set_subtitle(&format!(
            "{} · {}",
            describe_format(decoded.source_format),
            format_position(frames, rate)
        ));
        imp.header_title
            .set_tooltip_text(Some(&path.display().to_string()));
        imp.waveform.set_waveform(Some(waveform));

        self.reset_transport(frames, rate);
        imp.stack.set_visible_child_name("clip");
        info!(
            "opened {name}: {} ({frames} frames)",
            format_position(frames, rate)
        );
        Ok(())
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
fn describe_format(format: Option<SourceFormat>) -> String {
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
