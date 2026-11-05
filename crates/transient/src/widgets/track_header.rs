//! `TrackHeader`: a track's name and mixer controls (mute, solo, volume, pan).
//!
//! The header does not change the project itself. When a control changes, it
//! calls the callback set with [`TrackHeader::connect_changed`] with the new
//! settings; the window applies them (with undo) and updates the mix.
//!
//! Small widgets like this keep `mod.rs` and `imp.rs` in one file: the `imp`
//! module below is the private GObject side, as in the larger widgets.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use transient_core::convert::f32_from_f64;
use transient_core::mix::SILENT_DB;
use transient_core::project::{Track, TrackColor, TrackId};

/// The mixer settings a header shows and edits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TrackSettings {
    /// Volume in dB.
    pub(crate) volume_db: f32,
    /// Balance, −1 (left) to 1 (right).
    pub(crate) pan: f32,
    /// Muted.
    pub(crate) mute: bool,
    /// Soloed.
    pub(crate) solo: bool,
}

/// What a header reports when the user changes a control.
type ChangedCallback = Box<dyn Fn(TrackId, TrackSettings)>;
/// What a header reports when the user chooses Remove Track.
type RemoveCallback = Box<dyn Fn(TrackId)>;
/// What a header reports when the user renames the track.
type RenameCallback = Box<dyn Fn(TrackId, String)>;
/// What a header reports for Move Up (`true`) or Move Down (`false`).
type MoveCallback = Box<dyn Fn(TrackId, bool)>;
/// What a header reports when the user picks a colour (`None`: the default).
type ColorCallback = Box<dyn Fn(TrackId, Option<TrackColor>)>;

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::subclass::prelude::*;
    use gtk::{gio, glib};
    use transient_core::project::TrackId;

    use super::{ChangedCallback, ColorCallback, MoveCallback, RemoveCallback, RenameCallback};

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/sophiedkk/Transient/ui/track-header.ui")]
    pub(crate) struct TrackHeader {
        #[template_child]
        pub(super) name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) name_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub(super) name_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub(super) menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub(super) mute_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub(super) solo_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub(super) volume_scale: TemplateChild<gtk::Scale>,
        #[template_child]
        pub(super) pan_scale: TemplateChild<gtk::Scale>,

        pub(super) track: Cell<TrackId>,
        pub(super) on_changed: RefCell<Option<ChangedCallback>>,
        pub(super) on_remove: RefCell<Option<RemoveCallback>>,
        pub(super) on_rename: RefCell<Option<RenameCallback>>,
        pub(super) on_move: RefCell<Option<MoveCallback>>,
        pub(super) on_color: RefCell<Option<ColorCallback>>,
        /// `trackcolor.set`: its state is the track's colour (see `setup_color_picker`).
        pub(super) color_action: RefCell<Option<gio::SimpleAction>>,
        /// True while the code sets the controls, so that does not count as a change.
        pub(super) updating: Cell<bool>,
    }

    impl std::fmt::Debug for TrackHeader {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TrackHeader")
                .field("track", &self.track.get())
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TrackHeader {
        const NAME: &'static str = "TransientTrackHeader";
        type Type = super::TrackHeader;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            // "track.remove", used by the header's menu: the action lives on the
            // header itself, so it always means this header's track.
            klass.install_action("track.rename", None, |header, _, _| {
                header.start_rename();
            });
            klass.install_action("track.move-up", None, |header, _, _| {
                header.report_move(true);
            });
            klass.install_action("track.move-down", None, |header, _, _| {
                header.report_move(false);
            });
            klass.install_action("track.remove", None, |header, _, _| {
                let imp = header.imp();
                if let Some(callback) = imp.on_remove.borrow().as_ref() {
                    callback(imp.track.get());
                }
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for TrackHeader {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_controls();
        }
    }

    impl WidgetImpl for TrackHeader {}
    impl BoxImpl for TrackHeader {}
}

glib::wrapper! {
    /// A track's name and mixer controls.
    pub(crate) struct TrackHeader(ObjectSubclass<imp::TrackHeader>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl TrackHeader {
    /// An empty header; [`Self::set_track`] fills it in.
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }

    /// Shows `track`'s name and settings, without reporting them as a change.
    /// `details` (for example the original format) appears in the name's tooltip.
    pub(crate) fn set_track(&self, track: &Track, details: &str) {
        let imp = self.imp();
        imp.track.set(track.id);
        imp.updating.set(true);
        imp.name_label.set_label(&track.name);
        imp.name_label
            .set_tooltip_text(Some(&format!("{}\n{details}", track.name)));
        imp.mute_button.set_active(track.mute);
        imp.solo_button.set_active(track.solo);
        imp.volume_scale.set_value(f64::from(track.volume_db));
        imp.pan_scale.set_value(f64::from(track.pan));
        if let Some(action) = imp.color_action.borrow().as_ref() {
            action.set_state(&track.color.map_or("", TrackColor::id).to_variant());
        }
        imp.updating.set(false);
        self.update_tooltips();
    }

    /// Calls `callback` with the track and its new settings whenever the user
    /// changes a control.
    pub(crate) fn connect_changed(&self, callback: impl Fn(TrackId, TrackSettings) + 'static) {
        self.imp().on_changed.replace(Some(Box::new(callback)));
    }

    /// Calls `callback` with the track when the user chooses Remove Track.
    pub(crate) fn connect_remove(&self, callback: impl Fn(TrackId) + 'static) {
        self.imp().on_remove.replace(Some(Box::new(callback)));
    }

    /// Calls `callback` with the track and its new name after the user renamed it.
    pub(crate) fn connect_rename(&self, callback: impl Fn(TrackId, String) + 'static) {
        self.imp().on_rename.replace(Some(Box::new(callback)));
    }

    /// Calls `callback` with the track and the colour the user picked (`None`:
    /// the default accent colour).
    pub(crate) fn connect_color(&self, callback: impl Fn(TrackId, Option<TrackColor>) + 'static) {
        self.imp().on_color.replace(Some(Box::new(callback)));
    }

    /// Calls `callback` with the track and `true` (up) or `false` (down) for
    /// Move Up and Move Down.
    pub(crate) fn connect_move(&self, callback: impl Fn(TrackId, bool) + 'static) {
        self.imp().on_move.replace(Some(Box::new(callback)));
    }

    /// The track this header shows.
    pub(crate) fn track_id(&self) -> TrackId {
        self.imp().track.get()
    }

    /// Greys out Move Up for the first track and Move Down for the last.
    pub(crate) fn set_position(&self, first: bool, last: bool) {
        self.action_set_enabled("track.move-up", !first);
        self.action_set_enabled("track.move-down", !last);
    }

    /// Swaps the name for a text field holding it, all selected, ready to type.
    pub(crate) fn start_rename(&self) {
        let imp = self.imp();
        imp.name_entry.set_text(&imp.name_label.label());
        // The buttons make room, so the whole header width is there for typing.
        self.set_buttons_visible(false);
        imp.name_stack.set_visible_child_name("entry");
        imp.name_entry.grab_focus();
        imp.name_entry.select_region(0, -1);
    }

    /// Ends renaming: shows the name again and, with `save`, reports the new one.
    fn finish_rename(&self, save: bool) {
        let imp = self.imp();
        // Called again when the field loses focus after Enter or Escape: once is enough.
        if imp.name_stack.visible_child_name().as_deref() != Some("entry") {
            return;
        }
        imp.name_stack.set_visible_child_name("label");
        self.set_buttons_visible(true);
        let name = imp.name_entry.text().to_string();
        if save
            && name.trim() != imp.name_label.label().as_str()
            && let Some(callback) = imp.on_rename.borrow().as_ref()
        {
            callback(imp.track.get(), name);
        }
    }

    fn set_buttons_visible(&self, visible: bool) {
        let imp = self.imp();
        imp.mute_button.set_visible(visible);
        imp.solo_button.set_visible(visible);
        imp.menu_button.set_visible(visible);
    }

    fn report_move(&self, up: bool) {
        let imp = self.imp();
        if let Some(callback) = imp.on_move.borrow().as_ref() {
            callback(imp.track.get(), up);
        }
    }

    /// The colour circles in the track menu, like the accent colour picker in
    /// Settings. They share one stateful action, `trackcolor.set`, whose state
    /// is the track's colour id ("" for the default accent colour); each circle
    /// is a toggle button for one value, so GTK shows the current colour as
    /// pressed. Called once, from `setup_controls`.
    fn setup_color_picker(&self) {
        let imp = self.imp();
        let action =
            gio::SimpleAction::new_stateful("set", Some(glib::VariantTy::STRING), &"".to_variant());
        let weak = self.downgrade();
        action.connect_activate(move |action, value| {
            let Some(id) = value.and_then(glib::Variant::get::<String>) else {
                return;
            };
            action.set_state(&id.to_variant());
            if let Some(header) = weak.upgrade() {
                let imp = header.imp();
                if let Some(callback) = imp.on_color.borrow().as_ref() {
                    callback(imp.track.get(), TrackColor::from_id(&id));
                }
            }
        });
        let group = gio::SimpleActionGroup::new();
        group.add_action(&action);
        self.insert_action_group("trackcolor", Some(&group));
        imp.color_action.replace(Some(action));

        // Two rows of five: "Default" first, then the named colours.
        let grid = gtk::Grid::builder()
            .column_spacing(8)
            .row_spacing(8)
            .halign(gtk::Align::Center)
            .css_classes(["track-colors"])
            .build();
        let choices = std::iter::once(("default", "", gettext("Default Color"))).chain(
            TrackColor::ALL
                .into_iter()
                .map(|color| (color.id(), color.id(), color_name(color))),
        );
        for (index, (class, target, name)) in (0..).zip(choices) {
            let button = gtk::ToggleButton::builder()
                .action_name("trackcolor.set")
                .action_target(&target.to_variant())
                .css_classes(["track-color", class])
                .child(&gtk::Image::from_icon_name("object-select-symbolic"))
                .build();
            crate::widgets::set_button_text(&button, &name);
            grid.attach(&button, index % 5, index / 5, 1, 1);
        }
        if let Some(popover) = imp.menu_button.popover().and_downcast::<gtk::PopoverMenu>() {
            popover.add_child(&grid, "colors");
        }
    }

    /// Double-click to rename; in the text field, Enter (or clicking elsewhere)
    /// saves and Escape cancels. Called once, from `setup_controls`.
    fn setup_rename(&self) {
        let imp = self.imp();
        let double_click = gtk::GestureClick::new();
        let weak = self.downgrade();
        double_click.connect_pressed(move |_, presses, _, _| {
            if presses == 2
                && let Some(header) = weak.upgrade()
            {
                header.start_rename();
            }
        });
        imp.name_label.add_controller(double_click);

        let weak = self.downgrade();
        imp.name_entry.connect_activate(move |_| {
            if let Some(header) = weak.upgrade() {
                header.finish_rename(true);
            }
        });
        let escape = gtk::EventControllerKey::new();
        let weak = self.downgrade();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(header) = weak.upgrade() {
                header.finish_rename(false);
            }
            glib::Propagation::Stop
        });
        imp.name_entry.add_controller(escape);
        let focus = gtk::EventControllerFocus::new();
        let weak = self.downgrade();
        focus.connect_leave(move |_| {
            if let Some(header) = weak.upgrade() {
                header.finish_rename(true);
            }
        });
        imp.name_entry.add_controller(focus);
    }

    /// The settings as the controls show them now.
    fn settings(&self) -> TrackSettings {
        let imp = self.imp();
        TrackSettings {
            volume_db: f32_from_f64(imp.volume_scale.value()),
            pan: f32_from_f64(imp.pan_scale.value()),
            mute: imp.mute_button.is_active(),
            solo: imp.solo_button.is_active(),
        }
    }

    fn setup_controls(&self) {
        self.setup_rename();
        self.setup_color_picker();
        let imp = self.imp();
        let weak = self.downgrade();
        let changed = move || {
            if let Some(header) = weak.upgrade() {
                header.update_tooltips();
                header.report_change();
            }
        };
        let on_toggle = changed.clone();
        imp.mute_button.connect_toggled(move |_| on_toggle());
        let on_toggle = changed.clone();
        imp.solo_button.connect_toggled(move |_| on_toggle());
        let on_value = changed.clone();
        imp.volume_scale.connect_value_changed(move |_| on_value());
        imp.pan_scale.connect_value_changed(move |_| changed());

        // Snap to 0 dB and to the center when close: easy to get back to neutral.
        imp.volume_scale.connect_change_value(|scale, _, value| {
            if value.abs() < 0.75 {
                scale.set_value(0.0);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        imp.pan_scale.connect_change_value(|scale, _, value| {
            if value.abs() < 0.06 {
                scale.set_value(0.0);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }

    fn report_change(&self) {
        let imp = self.imp();
        if imp.updating.get() {
            return;
        }
        if let Some(callback) = imp.on_changed.borrow().as_ref() {
            callback(imp.track.get(), self.settings());
        }
    }

    /// Tooltips show the exact values, for example "Volume: −6.0 dB".
    /// Brings the tooltips (and the mute icon) in line with the controls.
    fn update_tooltips(&self) {
        let imp = self.imp();
        let settings = self.settings();
        // The icon says what you hear now; the tooltip says what a click does.
        let (icon, mute_tooltip) = if settings.mute {
            ("audio-volume-muted-symbolic", gettext("Unmute"))
        } else {
            ("audio-volume-high-symbolic", gettext("Mute"))
        };
        imp.mute_button.set_icon_name(icon);
        super::set_button_text(&*imp.mute_button, &mute_tooltip);
        let volume = if settings.volume_db <= SILENT_DB {
            gettext("Volume: silent")
        } else {
            gettext("Volume: {} dB").replace("{}", &format!("{:+.1}", settings.volume_db))
        };
        imp.volume_scale.set_tooltip_text(Some(&volume));
        let percent = (settings.pan.abs() * 100.0).round();
        let pan = if percent < 1.0 {
            gettext("Pan: center")
        } else if settings.pan < 0.0 {
            gettext("Pan: {}% left").replace("{}", &format!("{percent}"))
        } else {
            gettext("Pan: {}% right").replace("{}", &format!("{percent}"))
        };
        imp.pan_scale.set_tooltip_text(Some(&pan));
    }
}

/// A colour's name, for its circle's tooltip and screen readers.
fn color_name(color: TrackColor) -> String {
    match color {
        TrackColor::Blue => gettext("Blue"),
        TrackColor::Teal => gettext("Teal"),
        TrackColor::Green => gettext("Green"),
        TrackColor::Yellow => gettext("Yellow"),
        TrackColor::Orange => gettext("Orange"),
        TrackColor::Red => gettext("Red"),
        TrackColor::Pink => gettext("Pink"),
        TrackColor::Purple => gettext("Purple"),
        TrackColor::Slate => gettext("Slate"),
    }
}
