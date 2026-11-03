//! The main window.
//!
//! gtk-rs splits a custom widget into two parts:
//! - this module: the *public* type (`Window`) and its methods;
//! - `imp.rs`: the *private* implementation that GTK calls into
//!   (template children, lifecycle hooks such as "window is closing").
//!
//! The methods are spread over a few files by topic:
//! `import.rs` (opening files) and `transport.rs` (play, pause, seek).
//!
//! See the gtk-rs book, chapter "GObject Subclassing":
//! <https://gtk-rs.org/gtk4-rs/stable/latest/book/g_object_subclassing.html>

mod imp;
mod import;
mod transport;

use std::path::Path;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use tracing::warn;
use transient_engine::player::AudioOutput;

use crate::config;

glib::wrapper! {
    /// The main Transient window.
    pub(crate) struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Window {
    /// Creates a new main window for `app`.
    pub(crate) fn new(app: &adw::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    /// Chooses where playback goes. Applies to files opened after this call.
    pub(crate) fn set_audio_output(&self, output: AudioOutput) {
        self.imp().audio_output.set(output);
    }

    /// Shows a short message at the bottom of the window.
    fn show_toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }

    /// Saves an image of the window as it looks now, as a PNG file.
    ///
    /// # Errors
    ///
    /// Fails if the window has not been drawn yet, or the file cannot be written.
    pub(crate) fn save_screenshot(&self, path: &Path) -> Result<(), glib::BoolError> {
        let paintable = gtk::WidgetPaintable::new(Some(self));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(self.width()), f64::from(self.height()));
        let node = snapshot
            .to_node()
            .ok_or_else(|| glib::bool_error!("the window has nothing to draw yet"))?;
        let renderer = self
            .renderer()
            .ok_or_else(|| glib::bool_error!("the window is not shown yet"))?;
        renderer.render_texture(node, None).save_to_png(path)
    }

    /// Opens the settings, or returns `None` if the schema is not installed.
    ///
    /// With a plain `cargo run` the schema is not installed; `just run` compiles
    /// it and points `GSETTINGS_SCHEMA_DIR` at it. Without it, the window simply
    /// does not remember its size.
    fn open_settings() -> Option<gio::Settings> {
        let schema = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup(config::SETTINGS_SCHEMA_ID, true));
        if schema.is_none() {
            warn!(
                "settings schema {} not found; window size will not be saved",
                config::SETTINGS_SCHEMA_ID
            );
        }
        schema.map(|schema| gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None))
    }

    /// Restores the size from the previous session.
    fn load_window_state(&self) {
        let Some(settings) = self.imp().settings.get() else {
            return;
        };
        self.set_default_size(settings.int("window-width"), settings.int("window-height"));
        if settings.boolean("window-maximized") {
            self.maximize();
        }
    }

    /// Remembers the size for the next session.
    fn save_window_state(&self) {
        let Some(settings) = self.imp().settings.get() else {
            return;
        };
        let (width, height) = self.default_size();
        let saved = settings
            .set_int("window-width", width)
            .and_then(|()| settings.set_int("window-height", height))
            .and_then(|()| settings.set_boolean("window-maximized", self.is_maximized()));
        if let Err(error) = saved {
            warn!("could not save the window size: {error}");
        }
    }
}
