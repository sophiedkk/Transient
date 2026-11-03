//! Command-line options, for scripted and automated runs.
//!
//! ```sh
//! transient song.flac                       # open a file
//! transient --play --silent song.flac       # play it without sound output
//! transient --screenshot=shot.png song.flac # save an image of the window, then quit
//! transient --play --quit-after=5 song.flac # quit after five seconds
//! ```
//!
//! `transient --help` lists the options. They are meant for testing (by people
//! and by AI tools); see also `dev/headless.sh` to run without a visible window.

use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use tracing::{error, info};
use transient_engine::player::AudioOutput;

use crate::window::Window;

/// How long to let the window settle (layout, first frames) before a screenshot.
const SETTLE_TIME: Duration = Duration::from_millis(800);
/// If the window is not drawn yet, try again this often, this many times (5 s).
const RETRY_INTERVAL: Duration = Duration::from_millis(250);
const RETRIES: u32 = 20;

/// The options given on the command line.
#[derive(Debug, Default, Clone)]
pub(crate) struct LaunchOptions {
    /// Start playing as soon as the file is open.
    pub(crate) play: bool,
    /// Send audio nowhere (still in real time).
    pub(crate) silent: bool,
    /// Save a PNG of the window to this path, then quit.
    pub(crate) screenshot: Option<PathBuf>,
    /// Quit after this many seconds.
    pub(crate) quit_after: Option<u32>,
}

impl LaunchOptions {
    /// Registers the options with GApplication, which then also lists them in `--help`.
    pub(crate) fn register(app: &adw::Application) {
        let none = glib::OptionFlags::NONE;
        app.add_main_option(
            "play",
            glib::Char::from(b'p'),
            none,
            glib::OptionArg::None,
            "Start playing once the file is open",
            None,
        );
        app.add_main_option(
            "silent",
            glib::Char::from(0),
            none,
            glib::OptionArg::None,
            "Play without sound output (for tests)",
            None,
        );
        app.add_main_option(
            "screenshot",
            glib::Char::from(0),
            none,
            glib::OptionArg::Filename,
            "Save a PNG image of the window to FILE, then quit",
            Some("FILE"),
        );
        app.add_main_option(
            "quit-after",
            glib::Char::from(0),
            none,
            glib::OptionArg::Int,
            "Quit after SECONDS",
            Some("SECONDS"),
        );
    }

    /// Reads the options GApplication parsed from the command line.
    pub(crate) fn from_dict(dict: &glib::VariantDict) -> Self {
        Self {
            play: dict.contains("play"),
            silent: dict.contains("silent"),
            screenshot: dict.lookup::<PathBuf>("screenshot").ok().flatten(),
            quit_after: dict
                .lookup::<i32>("quit-after")
                .ok()
                .flatten()
                .and_then(|seconds| u32::try_from(seconds).ok()),
        }
    }

    /// Whether this is an automated run, which should get its own process
    /// instead of handing its files to an already-running Transient.
    pub(crate) fn is_automated(&self) -> bool {
        self.play || self.silent || self.screenshot.is_some() || self.quit_after.is_some()
    }

    /// Carries out the options for a freshly shown window.
    pub(crate) async fn run(self, window: Window, files: Vec<gio::File>) {
        if self.silent {
            window.set_audio_output(AudioOutput::Silent);
        }
        if !files.is_empty() {
            window.import_files(&files).await;
        }
        if self.play {
            window.toggle_playback();
        }
        if let Some(path) = &self.screenshot {
            save_screenshot_when_drawn(&window, path).await;
        }
        if let Some(seconds) = self.quit_after {
            glib::timeout_future_seconds(seconds).await;
        } else if self.screenshot.is_none() {
            return;
        }
        if let Some(app) = window.application() {
            info!("quitting as requested on the command line");
            app.quit();
        }
    }
}

/// Waits for the window to settle, then saves a screenshot. If the window has not
/// been drawn yet (a slow start), it tries again a little later, for up to 5 seconds.
async fn save_screenshot_when_drawn(window: &Window, path: &Path) {
    glib::timeout_future(SETTLE_TIME).await;
    let mut result = window.save_screenshot(path);
    for _ in 0..RETRIES {
        if result.is_ok() {
            break;
        }
        glib::timeout_future(RETRY_INTERVAL).await;
        result = window.save_screenshot(path);
    }
    match result {
        Ok(()) => info!("screenshot saved to {}", path.display()),
        Err(err) => error!("could not save the screenshot: {err}"),
    }
}
