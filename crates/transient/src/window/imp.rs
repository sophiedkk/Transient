//! The private side of [`super::Window`]: the GObject subclass that GTK calls into.

use std::cell::{Cell, OnceCell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{CompositeTemplate, gio, glib};
use transient_core::edit::History;
use transient_core::project::{ClipId, Project};
use transient_engine::player::{AudioOutput, Player};

use super::clip_pane::Normalize;
use super::mixer::MixerControl;
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
    #[template_child]
    pub(super) level_meter: TemplateChild<gtk::Box>,
    #[template_child]
    pub(super) left_meter: TemplateChild<gtk::LevelBar>,
    #[template_child]
    pub(super) right_meter: TemplateChild<gtk::LevelBar>,
    #[template_child]
    pub(super) master_scale: TemplateChild<gtk::Scale>,
    #[template_child]
    pub(super) split_view: TemplateChild<adw::OverlaySplitView>,
    #[template_child]
    pub(super) pane_stack: TemplateChild<adw::ViewStack>,
    #[template_child]
    pub(super) pane_clip_group: TemplateChild<adw::PreferencesGroup>,
    #[template_child]
    pub(super) gain_row: TemplateChild<adw::SpinRow>,
    #[template_child]
    pub(super) fade_in_row: TemplateChild<adw::SpinRow>,
    #[template_child]
    pub(super) fade_out_row: TemplateChild<adw::SpinRow>,

    pub(super) settings: OnceCell<gio::Settings>,
    /// The open project (what gets saved).
    pub(super) project: RefCell<Project>,
    /// The decoded audio of the project's sources.
    pub(super) sources: RefCell<LoadedSources>,
    /// The undo/redo history.
    pub(super) history: RefCell<History>,
    /// The selected clip, if any.
    pub(super) selected: Cell<Option<ClipId>>,
    /// The last mixer slider change and when, to coalesce a drag into one undo step.
    pub(super) last_mixer_change: RefCell<Option<(MixerControl, std::time::Instant)>>,
    /// True while the code sets mixer controls, so that is not taken as a change.
    pub(super) updating_controls: Cell<bool>,
    /// The "Clip deleted" toast, closed when the deletion is undone.
    pub(super) undo_toast: RefCell<Option<adw::Toast>>,
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
        klass.install_action("win.undo", None, |window, _, _| window.undo());
        klass.install_action("win.redo", None, |window, _, _| window.redo());
        klass.install_action("win.split", None, |window, _, _| {
            window.split_at_playhead();
        });
        klass.install_action("win.delete-clip", None, |window, _, _| {
            window.delete_selected_clip();
        });
        klass.install_action("win.deselect", None, |window, _, _| window.select(None));
        klass.install_action("win.toggle-clip-pane", None, |window, _, _| {
            window.toggle_clip_pane();
        });
        klass.install_action_async("win.normalize-peak", None, |window, _, _| async move {
            window.normalize_selected_clip(Normalize::Peak).await;
        });
        klass.install_action_async("win.normalize-loudness", None, |window, _, _| async move {
            window.normalize_selected_clip(Normalize::Loudness).await;
        });
        klass.install_action("win.fade-in-to-playhead", None, |window, _, _| {
            window.fade_in_to_playhead();
        });
        klass.install_action("win.fade-out-from-playhead", None, |window, _, _| {
            window.fade_out_from_playhead();
        });
        klass.install_action("win.toggle-mute", None, |window, _, _| window.toggle_mute());
        klass.install_action("win.toggle-solo", None, |window, _, _| window.toggle_solo());
        klass.install_action("win.select-clip-at-playhead", None, |window, _, _| {
            window.select_clip_at_playhead();
        });
        klass.install_action("win.nudge-left", None, |window, _, _| {
            window.nudge_selected_clip(-1);
        });
        klass.install_action("win.nudge-right", None, |window, _, _| {
            window.nudge_selected_clip(1);
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
        window.setup_editing();
        window.setup_mixer();
        window.setup_clip_pane();
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
