//! The style choice at the top of the main menu, as in GNOME Text Editor: three
//! circles for following the system's style, light and dark. Each circle is a
//! toggle button for one value of `app.color-scheme` (see `application.rs`),
//! so GTK shows the current choice as pressed.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;

use super::Window;

impl Window {
    /// Puts the style circles into the main menu. Called once, from `constructed`.
    pub(super) fn setup_style_menu(&self) {
        let choices = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .css_classes(["style-choices"])
            .build();
        for (value, name) in [
            ("default", gettext("Follow System Style")),
            ("light", gettext("Light Style")),
            ("dark", gettext("Dark Style")),
        ] {
            // The check mark sits in the bottom right corner as a badge, so it
            // shows on any of the three circles (one is half light, half dark).
            let check = gtk::Image::builder()
                .icon_name("object-select-symbolic")
                .pixel_size(12)
                .halign(gtk::Align::End)
                .valign(gtk::Align::End)
                .build();
            let button = gtk::ToggleButton::builder()
                .action_name("app.color-scheme")
                .action_target(&value.to_variant())
                .css_classes(["style-choice", value])
                .child(&check)
                .build();
            crate::widgets::set_button_text(&button, &name);
            choices.append(&button);
        }
        if let Some(popover) = self
            .imp()
            .primary_menu_button
            .popover()
            .and_downcast::<gtk::PopoverMenu>()
        {
            popover.add_child(&choices, "style");
        }
    }
}
