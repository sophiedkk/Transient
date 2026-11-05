//! Command-line options, for scripted and automated runs.
//!
//! ```sh
//! transient song.flac                       # open a file
//! transient --play --silent song.flac       # play it without sound output
//! transient --screenshot=shot.png song.flac # save an image of the window, then quit
//! transient --play --quit-after=5 song.flac # quit after five seconds
//! transient --seek=30 --zoom=4 song.flac    # zoom in around 0:30
//! transient --seek=5 --action=split a.flac  # run window actions (repeatable)
//! transient --select=2..4 --action=ripple-delete a.flac  # cut 0:02–0:04, close the gap
//! transient a.flac --save-to=mix.transient  # save the project afterwards
//! transient mix.transient                   # open a project
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

use crate::window::{Window, is_project_file};

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
    /// Record and measure a test tone instead of a microphone.
    pub(crate) test_input: bool,
    /// Save a PNG of the window to this path, then quit.
    pub(crate) screenshot: Option<PathBuf>,
    /// Wait this many seconds before the screenshot (for example until keys
    /// typed with `HEADLESS_KEYS` have arrived).
    pub(crate) screenshot_delay: Option<u32>,
    /// Quit after this many seconds.
    pub(crate) quit_after: Option<u32>,
    /// Move the playhead to this position (in seconds) after opening.
    pub(crate) seek: Option<f64>,
    /// Zoom in this many steps (around the playhead) after opening.
    pub(crate) zoom: Option<u32>,
    /// Select this stretch of time (seconds, on every track) before the actions.
    pub(crate) select: Option<(f64, f64)>,
    /// Add every clip playing at these positions (seconds) to the selection, as
    /// Ctrl+click would, before the actions.
    pub(crate) select_clips_at: Vec<f64>,
    /// Record a test tone for this many seconds after the actions (for tests).
    pub(crate) record_for: Option<u32>,
    /// Use this file for the first missing audio file after opening, as
    /// Locate… in the banner does (before the actions).
    pub(crate) locate: Option<PathBuf>,
    /// Window actions to run after opening, in order (for example "split").
    pub(crate) actions: Vec<String>,
    /// Save the project to this file after the actions.
    pub(crate) save_to: Option<PathBuf>,
    /// Export the mix to this file after the actions (format from the extension).
    pub(crate) export_to: Option<PathBuf>,
}

/// One command-line option, as GApplication registers it (and `--help` lists it).
struct Opt {
    name: &'static str,
    /// A one-letter alias (`-p`), if any.
    short: Option<u8>,
    arg: glib::OptionArg,
    help: &'static str,
    /// What `--help` calls the value (`--seek=SECONDS`).
    arg_name: Option<&'static str>,
}

/// Every command-line option, in the order `--help` lists them.
const OPTIONS: &[Opt] = &[
    Opt {
        name: "play",
        short: Some(b'p'),
        arg: glib::OptionArg::None,
        help: "Start playing once the file is open",
        arg_name: None,
    },
    Opt {
        name: "silent",
        short: None,
        arg: glib::OptionArg::None,
        help: "Play without sound output (for tests)",
        arg_name: None,
    },
    Opt {
        name: "test-input",
        short: None,
        arg: glib::OptionArg::None,
        help: "Use a test tone instead of a microphone (for recording and the input level)",
        arg_name: None,
    },
    Opt {
        name: "screenshot",
        short: None,
        arg: glib::OptionArg::Filename,
        help: "Save a PNG image of the window to FILE, then quit",
        arg_name: Some("FILE"),
    },
    Opt {
        name: "seek",
        short: None,
        arg: glib::OptionArg::Double,
        help: "Move the playhead to SECONDS after opening",
        arg_name: Some("SECONDS"),
    },
    Opt {
        name: "zoom",
        short: None,
        arg: glib::OptionArg::Int,
        help: "Zoom in STEPS times after opening",
        arg_name: Some("STEPS"),
    },
    Opt {
        name: "select",
        short: None,
        arg: glib::OptionArg::String,
        help: "Select START..END seconds on every track before the actions",
        arg_name: Some("START..END"),
    },
    Opt {
        name: "select-clips-at",
        short: None,
        arg: glib::OptionArg::StringArray,
        help: "Add every clip playing at SECONDS to the selection (repeatable)",
        arg_name: Some("SECONDS"),
    },
    Opt {
        name: "locate",
        short: None,
        arg: glib::OptionArg::Filename,
        help: "Use FILE for the first missing audio file, as Locate… does (before the actions)",
        arg_name: Some("FILE"),
    },
    Opt {
        name: "record-for",
        short: None,
        arg: glib::OptionArg::Int,
        help: "Record a test tone for SECONDS (instead of a microphone) after the actions",
        arg_name: Some("SECONDS"),
    },
    Opt {
        name: "action",
        short: None,
        arg: glib::OptionArg::StringArray,
        help: "Run the window action NAME after opening (repeatable), for example split or undo",
        arg_name: Some("NAME"),
    },
    Opt {
        name: "save-to",
        short: None,
        arg: glib::OptionArg::Filename,
        help: "Save the project to FILE after the actions",
        arg_name: Some("FILE"),
    },
    Opt {
        name: "export-to",
        short: None,
        arg: glib::OptionArg::Filename,
        help: "Export the mix to FILE (wav, flac, mp3, opus or ogg) after the actions",
        arg_name: Some("FILE"),
    },
    Opt {
        name: "screenshot-delay",
        short: None,
        arg: glib::OptionArg::Int,
        help: "Wait SECONDS before taking the screenshot",
        arg_name: Some("SECONDS"),
    },
    Opt {
        name: "quit-after",
        short: None,
        arg: glib::OptionArg::Int,
        help: "Quit after SECONDS",
        arg_name: Some("SECONDS"),
    },
];

impl LaunchOptions {
    /// Registers the options with GApplication, which then also lists them in `--help`.
    pub(crate) fn register(app: &adw::Application) {
        for option in OPTIONS {
            app.add_main_option(
                option.name,
                glib::Char::from(option.short.unwrap_or(0)),
                glib::OptionFlags::NONE,
                option.arg,
                option.help,
                option.arg_name,
            );
        }
    }

    /// Reads the options GApplication parsed from the command line.
    pub(crate) fn from_dict(dict: &glib::VariantDict) -> Self {
        Self {
            play: dict.contains("play"),
            silent: dict.contains("silent"),
            test_input: dict.contains("test-input"),
            screenshot: dict.lookup::<PathBuf>("screenshot").ok().flatten(),
            screenshot_delay: dict
                .lookup::<i32>("screenshot-delay")
                .ok()
                .flatten()
                .and_then(|seconds| u32::try_from(seconds).ok()),
            quit_after: dict
                .lookup::<i32>("quit-after")
                .ok()
                .flatten()
                .and_then(|seconds| u32::try_from(seconds).ok()),
            seek: dict.lookup::<f64>("seek").ok().flatten(),
            zoom: dict
                .lookup::<i32>("zoom")
                .ok()
                .flatten()
                .and_then(|steps| u32::try_from(steps).ok()),
            select: dict
                .lookup::<String>("select")
                .ok()
                .flatten()
                .and_then(|text| parse_range(&text)),
            record_for: dict
                .lookup::<i32>("record-for")
                .ok()
                .flatten()
                .and_then(|seconds| u32::try_from(seconds).ok()),
            select_clips_at: dict
                .lookup::<Vec<String>>("select-clips-at")
                .ok()
                .flatten()
                .unwrap_or_default()
                .iter()
                .filter_map(|text| text.trim().parse().ok())
                .collect(),
            locate: dict.lookup::<PathBuf>("locate").ok().flatten(),
            actions: dict
                .lookup::<Vec<String>>("action")
                .ok()
                .flatten()
                .unwrap_or_default(),
            save_to: dict.lookup::<PathBuf>("save-to").ok().flatten(),
            export_to: dict.lookup::<PathBuf>("export-to").ok().flatten(),
        }
    }

    /// Whether this is an automated run, which should get its own process
    /// instead of handing its files to an already-running Transient.
    pub(crate) fn is_automated(&self) -> bool {
        self.play
            || self.silent
            || self.test_input
            || self.screenshot.is_some()
            || self.quit_after.is_some()
            || self.seek.is_some()
            || self.zoom.is_some()
            || self.select.is_some()
            || !self.select_clips_at.is_empty()
            || self.record_for.is_some()
            || self.locate.is_some()
            || !self.actions.is_empty()
            || self.save_to.is_some()
            || self.export_to.is_some()
    }

    /// Carries out the options for a freshly shown window.
    pub(crate) async fn run(self, window: Window, files: Vec<gio::File>) {
        if self.silent {
            window.set_audio_output(AudioOutput::Silent);
        }
        if self.test_input {
            window.use_test_input();
        }
        // A project file is opened; anything else is imported as audio.
        if let Some(project) = files.iter().find(|file| is_project_file(file)) {
            window.open_project(project).await;
        } else if !files.is_empty() {
            window.import_files(&files).await;
        }
        if self.seek.is_some() || self.zoom.is_some() {
            wait_for_layout(&window).await;
        }
        if let Some(seconds) = self.seek {
            window.seek_to_seconds(seconds);
        }
        if let Some(steps) = self.zoom {
            window.zoom(f64::from(steps));
        }
        if let Some((start, end)) = self.select {
            window.select_seconds(start, end);
        }
        for seconds in &self.select_clips_at {
            window.add_clips_at_seconds(*seconds);
        }
        if let Some(path) = &self.locate {
            window
                .locate(&std::path::absolute(path).unwrap_or(path.clone()))
                .await;
        }
        for action in &self.actions {
            let name = if action.contains('.') {
                action.clone()
            } else {
                format!("win.{action}")
            };
            match WidgetExt::activate_action(&window, &name, None) {
                Ok(()) => info!("ran action {name}"),
                Err(err) => error!("could not run action {name}: {err}"),
            }
        }
        // With --screenshot, the screenshot shows the recording in progress
        // (taken just before it stops) instead of the result.
        let mut screenshot_taken = false;
        if let Some(seconds) = self.record_for {
            window.use_test_input();
            window.toggle_recording();
            glib::timeout_future_seconds(seconds).await;
            if let Some(path) = &self.screenshot {
                save_screenshot_when_drawn(&window, path).await;
                screenshot_taken = true;
            }
            window.stop_recording().await;
        }
        if let Some(path) = &self.save_to {
            let file = gio::File::for_path(std::path::absolute(path).unwrap_or(path.clone()));
            window.write_project(&file).await;
        }
        if let Some(path) = &self.export_to {
            let path = std::path::absolute(path).unwrap_or(path.clone());
            window.export_to(&path).await;
        }
        if self.play {
            window.toggle_playback();
        }
        if let Some(path) = self.screenshot.as_ref().filter(|_| !screenshot_taken) {
            if let Some(seconds) = self.screenshot_delay {
                glib::timeout_future_seconds(seconds).await;
            }
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

/// Waits (up to 2 seconds) until the waveform has a size, so zooming can work.
async fn wait_for_layout(window: &Window) {
    for _ in 0..40 {
        if window.is_laid_out() {
            return;
        }
        glib::timeout_future(Duration::from_millis(50)).await;
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

/// Reads "START..END" (seconds, for example "1.5..3"), as `--select` takes it.
fn parse_range(text: &str) -> Option<(f64, f64)> {
    let (start, end) = text.split_once("..")?;
    Some((start.trim().parse().ok()?, end.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn a_range_is_two_numbers_with_dots_between() {
        assert_eq!(parse_range("1.5..3"), Some((1.5, 3.0)));
        assert_eq!(parse_range(" 2 .. 4 "), Some((2.0, 4.0)));
        assert_eq!(parse_range("2-4"), None);
        assert_eq!(parse_range("a..4"), None);
    }
}
