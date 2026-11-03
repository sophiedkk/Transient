//! The private side of [`super::Window`]: the GObject subclass that GTK calls into.

use std::cell::{Cell, OnceCell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{CompositeTemplate, gio, glib};
use transient_engine::player::{AudioOutput, Player};

use crate::config;

/// The window's state. `#[template_child]` fields are filled in from window.blp
/// by matching the field name to the object id there.
#[derive(Debug, Default, CompositeTemplate)]
#[template(resource = "/io/github/sophiedkk/Transient/ui/window.ui")]
pub(crate) struct Window {
    #[template_child]
    pub(super) toast_overlay: TemplateChild<adw::ToastOverlay>,
    #[template_child]
    pub(super) toolbar_view: TemplateChild<adw::ToolbarView>,
    #[template_child]
    pub(super) stack: TemplateChild<adw::ViewStack>,
    #[template_child]
    pub(super) loading_page: TemplateChild<adw::StatusPage>,
    #[template_child]
    pub(super) clip_group: TemplateChild<adw::PreferencesGroup>,
    #[template_child]
    pub(super) length_row: TemplateChild<adw::ActionRow>,
    #[template_child]
    pub(super) format_row: TemplateChild<adw::ActionRow>,
    #[template_child]
    pub(super) location_row: TemplateChild<adw::ActionRow>,
    #[template_child]
    pub(super) play_button: TemplateChild<gtk::Button>,
    #[template_child]
    pub(super) position_label: TemplateChild<gtk::Label>,
    #[template_child]
    pub(super) position_scale: TemplateChild<gtk::Scale>,
    #[template_child]
    pub(super) duration_label: TemplateChild<gtk::Label>,

    pub(super) settings: OnceCell<gio::Settings>,
    /// The open file's player; `None` until a file is imported.
    pub(super) player: RefCell<Option<Player>>,
    /// Where playback goes (`--silent` sends it nowhere).
    pub(super) audio_output: Cell<AudioOutput>,
}

#[glib::object_subclass]
impl ObjectSubclass for Window {
    /// Must match `template $TransientWindow` in window.blp.
    const NAME: &'static str = "TransientWindow";
    type Type = super::Window;
    type ParentType = adw::ApplicationWindow;

    fn class_init(klass: &mut Self::Class) {
        klass.bind_template();

        // `win.import-audio`: used by the header bar button, the empty-state button
        // and Ctrl+I. An async action can await the file chooser.
        klass.install_action_async("win.import-audio", None, |window, _, _| async move {
            window.choose_and_import().await;
        });
        klass.install_action("win.play-pause", None, |window, _, _| {
            window.toggle_playback();
        });
        klass.install_action("win.go-to-start", None, |window, _, _| {
            window.seek_to_frame(0);
        });
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for Window {
    /// Runs once, right after the window is created.
    fn constructed(&self) {
        self.parent_constructed();
        let window = self.obj();

        // The striped header bar marks development builds (a GNOME convention).
        if config::is_development() {
            window.add_css_class("devel");
        }

        if let Some(settings) = super::Window::open_settings() {
            self.settings.get_or_init(|| settings);
            window.load_window_state();
        }

        window.setup_transport();
    }
}

impl WidgetImpl for Window {}

impl WindowImpl for Window {
    /// Runs when the window is about to close.
    fn close_request(&self) -> glib::Propagation {
        self.obj().save_window_state();
        // Stop playback right away instead of when the window is finally freed.
        self.player.replace(None);
        self.parent_close_request()
    }
}

impl ApplicationWindowImpl for Window {}
impl AdwApplicationWindowImpl for Window {}
