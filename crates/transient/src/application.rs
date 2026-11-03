//! The application object: command-line handling, actions that work app-wide,
//! keyboard shortcuts, the About dialog and opening the main window.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::config;
use crate::launch::LaunchOptions;
use crate::window::Window;

/// Creates the application and runs it until the last window closes.
pub(crate) fn run() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(config::APP_ID)
        .resource_base_path(config::RESOURCE_BASE_PATH)
        // Lets `transient FILE…` (and "Open With" in Files) open files.
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    LaunchOptions::register(&app);
    // Filled in by handle-local-options, read when the window opens.
    let options: Rc<RefCell<LaunchOptions>> = Rc::default();

    let parsed = Rc::clone(&options);
    app.connect_handle_local_options(move |app, dict| {
        let launch = LaunchOptions::from_dict(dict);
        if launch.is_automated() {
            // Normally a second `transient` command hands its files to the running
            // instance. Automated runs need their own process and options.
            app.set_flags(app.flags() | gio::ApplicationFlags::NON_UNIQUE);
        }
        parsed.replace(launch);
        std::ops::ControlFlow::Continue(())
    });

    app.connect_startup(setup_actions);

    let for_activate = Rc::clone(&options);
    app.connect_activate(move |app| {
        present_window(app, &for_activate, Vec::new());
    });
    app.connect_open(move |app, files, _hint| {
        present_window(app, &options, files.to_vec());
    });

    app.run()
}

/// Shows the main window (creating it if needed) and carries out the launch options.
fn present_window(
    app: &adw::Application,
    options: &Rc<RefCell<LaunchOptions>>,
    files: Vec<gio::File>,
) {
    let window = app
        .active_window()
        .and_downcast::<Window>()
        .unwrap_or_else(|| Window::new(app));
    window.present();
    // Options apply once; a second activation (e.g. from the dock) just raises the window.
    let launch = options.take();
    glib::spawn_future_local(launch.run(window, files));
}

/// Registers the `app.*` actions and the keyboard shortcuts.
///
/// Shortcuts follow the GNOME HIG's standard keys where one exists.
/// Keep resources/ui/shortcuts-dialog.blp in sync with this list.
fn setup_actions(app: &adw::Application) {
    // Quit by closing every window, so each can ask about unsaved changes first;
    // the application ends by itself once the last window is gone.
    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &adw::Application, _, _| {
            for window in app.windows() {
                window.close();
            }
        })
        .build();
    let about = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| show_about(app))
        .build();
    app.add_action_entries([quit, about]);

    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
    app.set_accels_for_action("win.import-audio", &["<Control>i"]);
    app.set_accels_for_action("win.new-project", &["<Control>n"]);
    app.set_accels_for_action("win.open-project", &["<Control>o"]);
    app.set_accels_for_action("win.save", &["<Control>s"]);
    app.set_accels_for_action("win.save-as", &["<Shift><Control>s"]);
    app.set_accels_for_action("win.play-pause", &["space"]);
    app.set_accels_for_action("win.go-to-start", &["Home"]);
    app.set_accels_for_action("win.undo", &["<Control>z"]);
    app.set_accels_for_action("win.redo", &["<Shift><Control>z"]);
    app.set_accels_for_action("win.split", &["s"]);
    app.set_accels_for_action("win.delete-clip", &["Delete"]);
    app.set_accels_for_action("win.deselect", &["Escape"]);
    app.set_accels_for_action("win.toggle-mute", &["m"]);
    app.set_accels_for_action("win.toggle-solo", &["<Shift>m"]);
    app.set_accels_for_action("win.toggle-clip-pane", &["F9"]);
    app.set_accels_for_action("win.nudge-left", &["<Control>Left"]);
    app.set_accels_for_action("win.nudge-right", &["<Control>Right"]);
    app.set_accels_for_action(
        "win.zoom-in",
        &["<Control>plus", "<Control>equal", "<Control>KP_Add"],
    );
    app.set_accels_for_action("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]);
    app.set_accels_for_action("win.zoom-fit", &["<Control>0", "<Control>KP_0"]);
}

fn show_about(app: &adw::Application) {
    let debug_info = format!(
        "Transient {}\nProfile: {}\n{}",
        config::VERSION,
        config::PROFILE,
        transient_engine::gstreamer_version(),
    );

    let about = adw::AboutDialog::builder()
        .application_name(gettext("Transient"))
        .application_icon(config::APP_ID)
        .developer_name("Sophie de Klerk")
        .developers(["Sophie de Klerk"])
        .version(config::VERSION)
        .website("https://github.com/sophiedkk/Transient")
        .issue_url("https://github.com/sophiedkk/Transient/issues")
        .license_type(gtk::License::MitX11)
        .copyright("© 2026 Sophie de Klerk")
        // Translators: replace with your name and email, one translator per line.
        .translator_credits(gettext("translator-credits"))
        .debug_info(debug_info)
        .build();

    about.present(app.active_window().as_ref());
}
