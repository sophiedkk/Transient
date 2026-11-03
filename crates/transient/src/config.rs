//! Build-time settings. The `TRANSIENT_*` values are set by build.rs
//! (from Meson for installed builds, or development defaults for `cargo run`).

/// The application ID; ends in `.Devel` for development builds.
pub(crate) const APP_ID: &str = env!("TRANSIENT_APP_ID");

/// `"development"` or `"default"`. Development builds get a striped header bar.
pub(crate) const PROFILE: &str = env!("TRANSIENT_PROFILE");

/// Where the compiled translations (`.mo` files) are installed.
pub(crate) const LOCALEDIR: &str = env!("TRANSIENT_LOCALEDIR");

/// The gettext domain; matches `translation-domain` in the Blueprint files.
pub(crate) const GETTEXT_PACKAGE: &str = "transient";

/// The app version, taken from Cargo.toml.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the bundled resources live. Set explicitly because the `.Devel` app ID
/// would otherwise make Libadwaita look in a different place.
pub(crate) const RESOURCE_BASE_PATH: &str = "/io/github/sophiedkk/Transient/";

/// The GSettings schema (data/io.github.sophiedkk.Transient.gschema.xml).
/// Development and stable builds share it.
pub(crate) const SETTINGS_SCHEMA_ID: &str = "io.github.sophiedkk.Transient";

/// Returns whether this is a development build.
pub(crate) fn is_development() -> bool {
    PROFILE == "development"
}
