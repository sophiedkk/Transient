//! The private side of [`super::Window`]: the GObject subclass that GTK calls into.

use std::cell::{Cell, OnceCell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{CompositeTemplate, gio, glib};
use transient_core::history::History;
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
    pub(super) start_page: TemplateChild<adw::StatusPage>,
    #[template_child]
    pub(super) record_button: TemplateChild<adw::SplitButton>,
    #[template_child]
    pub(super) split_button: TemplateChild<gtk::Button>,
    #[template_child]
    pub(super) delete_button: TemplateChild<gtk::Button>,
    #[template_child]
    pub(super) selection_label: TemplateChild<gtk::Label>,
    #[template_child]
    pub(super) timeline_menu: TemplateChild<gio::MenuModel>,
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
    pub(super) recent_group: TemplateChild<gtk::Box>,
    #[template_child]
    pub(super) recent_list: TemplateChild<gtk::ListBox>,
    #[template_child]
    pub(super) primary_menu_button: TemplateChild<gtk::MenuButton>,
    #[template_child]
    pub(super) missing_banner: TemplateChild<adw::Banner>,
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
    /// The project as last saved or opened, to detect unsaved changes.
    pub(super) saved_project: RefCell<Project>,
    /// The project's file, once saved or opened.
    pub(super) file: RefCell<Option<gio::File>>,
    /// Set once the user agreed to close despite unsaved changes.
    pub(super) close_confirmed: Cell<bool>,
    /// The decoded audio of the project's sources.
    pub(super) sources: RefCell<LoadedSources>,
    /// The undo/redo history.
    pub(super) history: RefCell<History>,
    /// The primary selected clip, if any: the last one clicked. The clip pane,
    /// fades and the track commands act on it.
    pub(super) selected: Cell<Option<ClipId>>,
    /// Every selected clip, including the primary one. Moving, nudging, deleting
    /// and splitting act on all of them.
    pub(super) selection: RefCell<Vec<ClipId>>,
    /// The last mixer slider change and when, to coalesce a drag into one undo step.
    pub(super) last_mixer_change: RefCell<Option<(MixerControl, std::time::Instant)>>,
    /// True while the code sets mixer controls, so that is not taken as a change.
    pub(super) updating_controls: Cell<bool>,
    /// The "Clip deleted" toast, closed when the deletion is undone.
    pub(super) undo_toast: RefCell<Option<adw::Toast>>,
    /// The open project's audio, unpacked from its file (see `file.rs`). The
    /// folder is deleted when this is replaced or the window closes.
    pub(super) project_cache: RefCell<Option<super::file::ProjectCache>>,
    /// The recording in progress, if any, and where to record from (see
    /// `recording.rs`).
    pub(super) recording: RefCell<Option<super::recording::Recording>>,
    /// Record a test tone instead of an input (`--record-for`).
    pub(super) test_input: Cell<bool>,
    /// What Cut or Copy took (see `clipboard.rs`).
    pub(super) clipboard: RefCell<Option<transient_core::clipboard::Clipboard>>,
    /// The time selection (see `selection.rs`).
    pub(super) time_selection: RefCell<Option<super::selection::TimeSelection>>,
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

        install_project_actions(klass);
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
        klass.install_action("win.deselect", None, |window, _, _| window.deselect_all());
        klass.install_action("win.select-all", None, |window, _, _| window.select_all());
        klass.install_action("win.record", None, |window, _, _| window.toggle_recording());
        klass.install_action("win.cut", None, |window, _, _| window.cut());
        klass.install_action("win.copy", None, |window, _, _| {
            window.copy();
        });
        klass.install_action("win.paste", None, |window, _, _| window.paste(false));
        klass.install_action("win.paste-insert", None, |window, _, _| window.paste(true));
        klass.install_action("win.context-menu", None, |window, _, _| {
            window.imp().timeline.show_context_menu();
        });
        klass.install_action("win.ripple-delete", None, |window, _, _| {
            window.ripple_delete();
        });
        klass.install_action("win.move-to-new-track", None, |window, _, _| {
            window.move_selected_clip_to_new_track();
        });
        klass.install_action("win.rename-track", None, |window, _, _| {
            window.rename_selected_track();
        });
        klass.install_action("win.move-track-up", None, |window, _, _| {
            window.move_selected_track(true);
        });
        klass.install_action("win.move-track-down", None, |window, _, _| {
            window.move_selected_track(false);
        });
        klass.install_action("win.remove-track", None, |window, _, _| {
            window.remove_selected_track();
        });
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
            window.zoom(1.0);
        });
        klass.install_action("win.zoom-out", None, |window, _, _| {
            window.zoom(-1.0);
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
        // The start page shows the app icon (striped for development builds).
        self.start_page.set_icon_name(Some(config::APP_ID));

        if let Some(settings) = super::Window::open_settings() {
            self.settings.get_or_init(|| settings);
            window.load_window_state();
        }

        window.setup_transport();
        window.setup_editing();
        window.setup_time_selection();
        window.setup_shortcuts();
        window.setup_record_input();
        window.setup_style_menu();
        window.imp().timeline.set_context_menu(&self.timeline_menu);
        window.setup_mixer();
        window.setup_clip_pane();
        window.update_recent_projects();
    }
}

impl WidgetImpl for Window {}

impl WindowImpl for Window {
    /// Runs when the window is about to close. With unsaved changes, it asks
    /// first ("Save Changes?") and only closes once that is settled.
    fn close_request(&self) -> glib::Propagation {
        let window = self.obj();
        if window.has_unsaved_changes() && !self.close_confirmed.get() {
            let window = window.clone();
            glib::spawn_future_local(async move {
                if window.confirm_discard_changes().await {
                    window.imp().close_confirmed.set(true);
                    window.close();
                }
            });
            return glib::Propagation::Stop;
        }
        window.save_window_state();
        // Stop playback right away instead of when the window is finally freed.
        self.player.replace(None);
        self.parent_close_request()
    }
}

impl ApplicationWindowImpl for Window {}
impl AdwApplicationWindowImpl for Window {}

/// The actions for projects and files: import, new, open, save, export and
/// preferences (the main menu's commands). Called once, from `class_init`.
fn install_project_actions(klass: &mut <Window as ObjectSubclass>::Class) {
    // `win.import-audio`: used by the header bar button, the empty-state button
    // and Ctrl+I. An async action can await the file chooser.
    klass.install_action_async("win.import-audio", None, |window, _, _| async move {
        window.choose_and_import().await;
    });
    klass.install_action_async("win.new-project", None, |window, _, _| async move {
        window.new_project().await;
    });
    klass.install_action_async("win.open-project", None, |window, _, _| async move {
        window.open_project_dialog().await;
    });
    klass.install_action_async("win.save", None, |window, _, _| async move {
        window.save().await;
    });
    klass.install_action_async("win.save-as", None, |window, _, _| async move {
        window.save_as().await;
    });
    klass.install_action("win.export", None, |window, _, _| {
        window.show_export_dialog();
    });
    klass.install_action("win.preferences", None, |window, _, _| {
        window.show_preferences();
    });
}
