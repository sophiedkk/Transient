//! Build script: runs before the `transient` crate is compiled.
//!
//! 1. Passes build settings (app ID, profile, locale directory) to the code as
//!    compile-time constants (read in `src/config.rs`). Meson sets them for
//!    installed builds; a plain `cargo run` uses the development defaults below.
//! 2. Compiles the Blueprint UI files to GtkBuilder XML.
//! 3. Bundles the UI files into a GResource, which is embedded in the binary.

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

/// Blueprint files, relative to this crate. Add new `.blp` files here
/// and in `resources/resources.gresource.xml`.
const BLUEPRINTS: [&str; 5] = [
    "resources/ui/window.blp",
    "resources/ui/effect-slider.blp",
    "resources/ui/shortcuts-dialog.blp",
    "resources/ui/track-header.blp",
    "resources/ui/export-dialog.blp",
];

/// Build settings and their defaults for development builds.
const SETTINGS: [(&str, &str); 3] = [
    ("TRANSIENT_APP_ID", "io.github.sophiedkk.Transient.Devel"),
    ("TRANSIENT_PROFILE", "development"),
    ("TRANSIENT_LOCALEDIR", "/usr/share/locale"),
];

fn main() -> Result<(), Box<dyn Error>> {
    for (name, default) in SETTINGS {
        println!("cargo::rerun-if-env-changed={name}");
        let value = env::var(name).unwrap_or_else(|_| default.to_owned());
        println!("cargo::rustc-env={name}={value}");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    compile_blueprints(&out_dir)?;

    glib_build_tools::compile_resources(
        // The app icons are bundled too (see the .gresource.xml), so the start
        // page can show the icon even when Transient is not installed (`just run`).
        &[
            PathBuf::from("resources"),
            out_dir,
            PathBuf::from("../../data/icons/hicolor/scalable/apps"),
        ],
        "resources/resources.gresource.xml",
        "transient.gresource",
    );
    Ok(())
}

/// Turns every `.blp` file into `$OUT_DIR/ui/<name>.ui`.
fn compile_blueprints(out_dir: &std::path::Path) -> Result<(), Box<dyn Error>> {
    for blueprint in BLUEPRINTS {
        println!("cargo::rerun-if-changed={blueprint}");
    }
    let status = Command::new("blueprint-compiler")
        .arg("batch-compile")
        .arg(out_dir.join("ui"))
        .arg("resources/ui")
        .args(BLUEPRINTS)
        .status()
        .map_err(|error| {
            format!("could not run blueprint-compiler ({error}); build inside the toolbox")
        })?;
    if !status.success() {
        return Err("blueprint-compiler reported errors (see above)".into());
    }
    Ok(())
}
