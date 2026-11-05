# Common tasks. Run them inside the toolbox (`toolbox enter transient-dev`).
# `just` on its own lists the recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

manifest := "build-aux/io.github.sophiedkk.Transient.Devel.json"
app_id := "io.github.sophiedkk.Transient.Devel"
# Flatpak runs on the host; from inside the toolbox we reach it with flatpak-spawn.
host := if path_exists("/run/.toolboxenv") == "true" { "flatpak-spawn --host" } else { "" }

# List the available recipes
default:
    @just --list

# Build and run Transient (development build, debug logging)
run *args: schemas
    GSETTINGS_SCHEMA_DIR=target/gschemas RUST_LOG=${RUST_LOG:-transient=debug,info} cargo run -p transient -- {{args}}

# Run the tests
test:
    cargo nextest run --workspace

# Download the test audio (if needed) and decode every file
test-audio:
    dev/fetch-test-audio.sh
    cargo nextest run --workspace --run-ignored ignored-only

# Decode audio files and print JSON reports, e.g. `just probe test-audio/*`
probe +files:
    cargo run -q -p transient-cli -- probe {{files}}

# Run Transient in an invisible compositor (no window on screen), e.g. `just headless --play --silent --quit-after=3 song.flac`
headless *args: schemas
    cargo build -q -p transient
    GSETTINGS_SCHEMA_DIR=target/gschemas dev/headless.sh target/debug/transient {{args}}

# Save a screenshot of the window to target/screenshot.png, e.g. `just screenshot test-audio/piano.flac`
screenshot *args:
    just headless --silent --screenshot=target/screenshot.png {{args}}

# Format the code
fmt:
    cargo fmt --all

# Everything CI checks: formatting, lints, tests, licences, and the app metadata
check: lint test deny meson-test

# Check formatting, run clippy with warnings as errors, and check file sizes
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    dev/check-file-size.sh

# Check dependencies for security advisories and licences
deny:
    cargo deny check

# Validate the desktop file, metainfo and settings schema
meson-test:
    @[ -d _build ] || meson setup _build -Dprofile=development
    meson test -C _build --print-errorlogs

# Update po/transient.pot after changing translatable strings
pot:
    @[ -d _build ] || meson setup _build -Dprofile=development
    meson compile -C _build transient-pot

# Build, install and run the Flatpak (what users will get)
flatpak:
    {{host}} flatpak run org.flatpak.Builder --user --install --force-clean \
        --install-deps-from=flathub --ccache --state-dir=_flatpak/state \
        _flatpak/build {{manifest}}
    {{host}} flatpak run {{app_id}}

# Compile the settings schema so `just run` can remember the window size
[private]
schemas:
    @mkdir -p target/gschemas
    @glib-compile-schemas --strict --targetdir target/gschemas data
