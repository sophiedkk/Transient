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
use gtk::glib;
use transient_core::mix::SILENT_DB;
use transient_core::project::{Track, TrackId};

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

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::subclass::prelude::*;
    use gtk::glib;
    use transient_core::project::TrackId;

    use super::ChangedCallback;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/sophiedkk/Transient/ui/track-header.ui")]
    pub(crate) struct TrackHeader {
        #[template_child]
        pub(super) name_label: TemplateChild<gtk::Label>,
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
        imp.updating.set(false);
        self.update_tooltips();
    }

    /// Calls `callback` with the track and its new settings whenever the user
    /// changes a control.
    pub(crate) fn connect_changed(&self, callback: impl Fn(TrackId, TrackSettings) + 'static) {
        self.imp().on_changed.replace(Some(Box::new(callback)));
    }

    /// The settings as the controls show them now.
    fn settings(&self) -> TrackSettings {
        let imp = self.imp();
        TrackSettings {
            volume_db: to_f32(imp.volume_scale.value()),
            pan: to_f32(imp.pan_scale.value()),
            mute: imp.mute_button.is_active(),
            solo: imp.solo_button.is_active(),
        }
    }

    fn setup_controls(&self) {
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
    fn update_tooltips(&self) {
        let imp = self.imp();
        let settings = self.settings();
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

/// A slider value as `f32` (slider ranges are small).
fn to_f32(value: f64) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "slider values are within -60..12, far inside f32's range"
    )]
    let value = value as f32;
    value
}
