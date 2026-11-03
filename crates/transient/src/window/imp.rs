//! The private side of [`super::Window`]: the GObject subclass that GTK calls into.

use std::cell::{Cell, OnceCell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{CompositeTemplate, gio, glib};
use transient_core::project::Project;
use transient_engine::player::{AudioOutput, Player};

use super::project::LoadedSources;
use crate::config;
use crate::widgets::timeline_view::TimelineView;

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
    pub(super) header_title: TemplateChild<adw::WindowTitle>,
    #[template_child]
    pub(super) loading_page: TemplateChild<adw::StatusPage>,
    #[template_child]
    pub(super) track_headers: TemplateChild<gtk::Box>,
    #[template_child]
    pub(super) timeline: TemplateChild<TimelineView>,
    #[template_child]
    pub(super) play_button: TemplateChild<gtk::Button>,
    #[template_child]
    pub(super) position_label: TemplateChild<gtk::Label>,
    #[template_child]
    pub(super) duration_label: TemplateChild<gtk::Label>,

    pub(super) settings: OnceCell<gio::Settings>,
    /// The open project (what gets saved).
    pub(super) project: RefCell<Project>,
    /// The decoded audio of the project's sources.
    pub(super) sources: RefCell<LoadedSources>,
    /// Plays the project's mix; `None` until something is imported.
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
        // window.blp uses $TransientTimelineView, so its type must exist first.
        TimelineView::ensure_type();
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
        klass.install_action("win.zoom-in", None, |window, _, _| {
            window.zoom_in();
        });
        klass.install_action("win.zoom-out", None, |window, _, _| {
            window.imp().timeline.zoom_out();
        });
        klass.install_action("win.zoom-fit", None, |window, _, _| {
            window.imp().timeline.zoom_to_fit();
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
