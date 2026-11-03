//! Transient: a simple multitrack audio editor for GNOME.
//!
//! This file only does start-up work: logging, translations, embedded
//! resources and GStreamer. The app itself lives in `application.rs`.

mod application;
mod config;
mod launch;
mod window;

use gettextrs::{bind_textdomain_codeset, bindtextdomain, textdomain};
use gtk::{gio, glib};
use tracing::{error, warn};
use tracing_subscriber::EnvFilter;

fn main() -> glib::ExitCode {
    // Logging. Choose the level with RUST_LOG, for example `RUST_LOG=transient=debug`.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    setup_translations();

    // The UI files, compiled into the binary by build.rs.
    if let Err(error) = gio::resources_register_include!("transient.gresource") {
        error!("could not load the built-in UI resources: {error}");
        return glib::ExitCode::FAILURE;
    }

    if let Err(error) = transient_engine::init() {
        error!("{error}");
        return glib::ExitCode::FAILURE;
    }

    application::run()
}

/// Tells gettext where Transient's translations are.
/// Failing here is not fatal: the app then runs in English.
///
/// The locale itself (the user's language) is set by GTK when it starts,
/// so there is no `setlocale` call here.
fn setup_translations() {
    if let Err(error) = bindtextdomain(config::GETTEXT_PACKAGE, config::LOCALEDIR) {
        warn!(
            "could not find translations in {}: {error}",
            config::LOCALEDIR
        );
    }
    if let Err(error) = bind_textdomain_codeset(config::GETTEXT_PACKAGE, "UTF-8") {
        warn!("could not set the translation encoding: {error}");
    }
    if let Err(error) = textdomain(config::GETTEXT_PACKAGE) {
        warn!("could not select the translation domain: {error}");
    }
}
