//! Custom widgets. Each is a GObject subclass, so Blueprint files can use it
//! (as `$TransientTimelineView`, for example) once its type is registered.

pub(crate) mod export_dialog;
pub(crate) mod timeline_view;
pub(crate) mod track_header;

use gtk::prelude::*;

/// Sets an icon button's tooltip and its accessible name (what a screen reader
/// says) to `text`. An icon-only button has no other name: a tooltip alone only
/// becomes its description. Use this whenever such a button's meaning changes.
pub(crate) fn set_button_text(button: &(impl IsA<gtk::Widget> + IsA<gtk::Accessible>), text: &str) {
    button.set_tooltip_text(Some(text));
    button.update_property(&[gtk::accessible::Property::Label(text)]);
}
