# CLAUDE.md

Transient is a simple multitrack audio editor for GNOME (Rust, GTK 4, Libadwaita,
GStreamer), meant as a lower-complexity alternative to Tenacity. App ID
`io.github.sophiedkk.Transient` (`.Devel` for development builds). License MIT.

The owner is new to Rust and gtk-rs: explain non-obvious code in comments and in
replies, prefer clear code over clever code, and work in small, runnable steps.

## Architecture

- `crates/transient-core`: project model (sources, tracks, clips with
  start/offset/length; serde JSON), `Mix` (renders any stretch of the timeline),
  waveform peak pyramid, ruler maths, time in frames (`u64`).
  **Never depends on GTK or GStreamer.**
- `crates/transient-engine`: GStreamer only: decode files into in-memory `f32`
  buffers (48 kHz stereo), play the mix through `appsrc`, export through encoders.
  Our own mixer, not GStreamer Editing Services.
- `crates/transient-cli`: `probe FILE…` (decode), `mix FILE…` (one track per
  file, render the mix), `export PROJECT|FILES --output OUT.ext` and `inputs`
  (recording inputs) — the engine without a window, JSON output.
- Recording: `transient-engine::recorder` (input → f32 in memory, level meter,
  `save_flac`); `window/recording.rs` (Record button/R, input menu, takes as new
  tracks).
- Export: `transient-engine::export` (appsrc → encoder → filesink, blocking
  push, progress + cancel, removes unfinished files; round-trip tested for every
  format). `ExportDialog` runs it on a background thread with an async-channel.
- `crates/transient`: the GTK app. Blueprint UI in `resources/ui/`, compiled by
  `build.rs`; one GObject subclass per custom widget (`mod.rs` public, `imp.rs` private).
  The window owns the `Project` plus decoded sources; after every change
  `Window::project_changed()` rebuilds track headers, timeline lanes, the player's
  `Mix` and the title. `TimelineView` (ruler, lanes, clips) and the track headers
  line up via `RULER_HEIGHT`/`LANE_HEIGHT`.
- Dependencies point downward only: `transient`/`transient-cli` → `transient-engine` →
  `transient-core`.
- Undo/redo = snapshots of the whole project (`transient-core::history::History`).
  Every change goes through `Window::apply_edit`, which records the snapshot.
  The timeline only previews drags and emits `clip-changed` on release.
- Clips carry `gain_db`, `fade_in`, `fade_out` (`Clip::with_fades` keeps fades
  inside the clip; trims and splits preserve that). `Mix` applies equal-power
  fades (`fade_shape`). `transient-core::normalize` computes peak/LUFS gains
  (ebur128). The clip pane (`window/clip_pane.rs`, F9) edits the selected clip.
- Project files (`window/file.rs`, `transient-core::archive`): a `.transient` file is
  a ZIP with `project.json` and the audio as is (`audio/<source id>/<name>`).
  Opening unpacks the audio into a locked folder in `~/.cache/transient` (deleted
  with the window's `ProjectCache`; unlocked leftovers are removed at startup).
  Old format-1 files (plain JSON with paths) still open.
  Unsaved changes = `project != saved_project`. Closing, quitting (app.quit closes
  each window) and New/Open ask "Save Changes?" via `confirm_discard_changes`.
  Clips whose audio is missing stay, drawn grey (`LaneClip::waveform` is `None`).
- Mixer settings (track volume/pan/mute/solo, master volume) live on `Track`/
  `Project`; `Mix` applies them (balance pan law). Slider changes go through
  `Window::apply_mixer_change`: coalesced undo steps and `update_mix()` only,
  so headers are never rebuilt mid-drag. `TrackHeader` (Blueprint template) must
  stay exactly `LANE_HEIGHT` tall; the header column is clamped to 200 px with
  `hexpand: false` (slider hexpand would otherwise spread up).

## UI rules (GNOME HIG)

- Destructive actions are undoable with an "Undo" toast, never a confirmation dialog.
- Header bar: few icon buttons with tooltips, primary menu at the end. Transport
  controls go in a bottom bar; clip properties in a right utility pane (F9).
- Standard shortcuts from the HIG (Ctrl+Z, Shift+Ctrl+Z, Ctrl+S, …); everything works
  by keyboard. Shortcuts that could clash with typing (single keys, Ctrl+C/V/Z,
  Delete, …) go in `src/window/shortcuts.rs` (run after the focused widget, so text
  fields get their keys); only clash-free ones are app accels in `src/application.rs`.
  Keep `resources/ui/shortcuts-dialog.blp` and the context menu in sync (tested).
- Header capitalization for buttons and menus, sentence capitalization for labels;
  an ellipsis (…) only when more input follows. All UI text is translatable
  (`_()` in Blueprint, `gettext()` in Rust) and listed in `po/POTFILES.in`.
- Custom drawing uses theme colours (widget colour, accent), never hardcoded ones.

## Code rules

Based on the general-purpose parts of FerroEHR's Rust rules (ROADMAP.md,
milestone 14). Each rule says what checks it; "review" means no tool does.

- Lints are set in the root `Cargo.toml` (`[workspace.lints]`, `pedantic` at deny),
  clippy settings in `clippy.toml`. CI runs clippy with `-D warnings`: fix
  warnings, don't silence them.
- **No suppressions in production code.** Rewrite the code instead. Tests may use
  `#[expect(lint, reason = "…")]`; never `#[allow]` (clippy:
  `allow_attributes`, `allow_attributes_without_reason`). The only exceptions are a
  lint firing inside a third-party macro (glib's `Properties` derive) and the CLI
  writing errors to stderr; their reason says so. (review)
- No panics: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`, no `v[i]`
  indexing or `&s[a..b]` slicing outside tests: a panic in a GTK callback aborts
  the app. Return `Result`; show the user a toast; log with `tracing` (clippy).
- No `as` between number types (clippy `as_conversions`): `From`/`TryFrom`, frame
  conversions in `transient_core::time`, everything else through
  `transient_core::convert` (checked with `num-traits`, tested).
- Errors are types that keep their cause: `thiserror` enums with `#[source]`/
  `#[from]`, never `.to_string()` into a `String` variant, and a variant's message
  does not repeat its source. Log with `Report(&error)` (`transient_core::report`),
  which prints the whole chain. (review; `map_err_ignore` catches dropped errors)
- `unsafe_code` is forbidden in the whole workspace.
- At most 800 lines per `.rs` file, tests included; split by concern
  (`dev/check-file-size.sh`, in `just check` and CI).
- No function that only forwards to another (a body that is one call with the
  same or fixed arguments): call the target. Trait methods are exempt; reaching
  into a field (`self.imp().timeline.zoom(…)`) is fine. (review)
- Code is written once: logic with one reason to change has one owner, never a
  `util`/`helpers` module ("one copy is fine, two are looked at, three are
  extracted"). Shared test helpers live in `transient-core`'s `test_support`.
  Measure with `npx jscpd@4.0.5 --format rust crates` when in doubt. (review)
- Doc comments start with one short summary sentence, then a blank `///` line
  (clippy `too_long_first_doc_paragraph`); rustdoc lints are deny.
- No re-exports (`pub use`, clippy `pub_use`): import from where a thing is defined.
- One `impl Default` per struct, and container-level `#[serde(default)]` on project
  file structs so older files keep opening (never `#[serde(default = "path")]`).
  Changing the file format means raising `project::FORMAT_VERSION` and updating
  the snapshot test.
- New dependencies go in `[workspace.dependencies]` with a version; crates use
  `name.workspace = true`.
- Tests: `cargo nextest`; `insta` snapshots for file formats; `proptest` for maths
  invariants. Never weaken or delete a test to make it pass.

## Build and test

Everything runs inside the `transient-dev` toolbox (see README.md):

```sh
just run      # cargo run with the compiled settings schema
just check    # fmt --check, clippy -D warnings, nextest, cargo deny, meson test
just flatpak  # full Flatpak build (GNOME 51 runtime) and run
```

One `./target` for everything: Meson's Cargo build uses it too. Never set
`CARGO_TARGET_DIR`.

## Verifying changes yourself (no window on the user's screen)

Always check UI and audio changes with these before reporting them done:

```sh
just probe test-audio/*                         # decode files → one JSON line each
just screenshot test-audio/piano.flac           # → target/screenshot.png; then Read the PNG
just headless --silent --play --quit-after=3 test-audio/drum-loop.wav   # run, read the log
just test-audio                                 # decode the whole test collection
```

- `just headless` / `just screenshot` run the app in an invisible mutter compositor
  (`dev/headless.sh`) with its own D-Bus session, in-memory settings and a
  throwaway XDG data folder (so tests never touch the user's recent files).
  Dark mode: prefix with `ADW_DEBUG_COLOR_SCHEME=prefer-dark`.
- App options (`transient --help`): `FILE…` opens files, `--play`, `--silent`
  (no sound output, still real time), `--screenshot=FILE`, `--quit-after=SECONDS`,
  `--seek=SECONDS`, `--zoom=STEPS` (zoom in around the playhead),
  `--select=START..END` (seconds, every track; runs before the actions),
  `--select-clips-at=SECONDS` (repeatable; adds every clip playing there to the
  selection, like Ctrl+click), and
  `--action=NAME` (repeatable; runs `win.NAME`, e.g. `split`, `undo`,
  `select-clip-at-playhead`, `select-all`, `context-menu` (opens it; popovers are
  not in screenshots, check the debug log), `nudge-right`, `delete-clip`
  (deletes the time selection if there is one), `ripple-delete`, `toggle-mute`,
  `toggle-solo`, `fade-in-to-playhead`, `fade-out-from-playhead`,
  `normalize-peak`, `normalize-loudness`, `move-to-new-track`, `remove-track`,
  `rename-track` (opens the name field), `move-track-up`, `move-track-down`,
  `cut`, `copy`, `paste`, `paste-insert` (paste at the playhead),
  `toggle-clip-pane`, `new-project`, `app.quit`) to script edits. `--save-to=FILE` saves afterwards; passing a
  `.transient` file opens it — together they test a full save/open round trip.
  `--export-to=FILE.ext` exports through the app's export path.
  `--record-for=SECONDS` records a test tone (no microphone needed) onto a new
  track at the playhead, then continues with `--save-to` etc.
  `--screenshot-delay=SECONDS` waits before the screenshot (e.g. for keys typed
  with `HEADLESS_KEYS`; the headless window is never "active", so GTK's focus
  outline does not show there). With `--play`, the level meter (red = clipping)
  shows whether the mix is as loud as expected.
- Typing and real key presses: `HEADLESS_KEYS="t e s t Return Control_L+s"
  just headless …` types GDK key names into the app ~4 s after it starts
  (`dev/send-keys.py`, mutter's RemoteDesktop API; "+" = keys together). Use it
  for anything keyboard-related (shortcuts, text fields), then check the saved
  project or log; e.g. `--action=rename-track` first, then type a name.
  Mouse drags cannot be scripted; their maths lives in `transient-core::clip`, `::edit`
  and `transient-core::range` (unit- and property-tested).
  Add a new option in `src/launch.rs` when a feature needs scripted testing.
- Never play audio through the speakers in automated runs: always pass `--silent`.
- `test-audio/` is downloaded by `dev/fetch-test-audio.sh` from `dev/test-audio.tsv`
  (Wikimedia Commons, checksummed, licences in `test-audio/CREDITS.md`). It is not
  committed. Add files by adding a manifest row with its SHA-256.

## Releases

`RELEASING.md` is the checklist. A `v*` tag runs `.github/workflows/release.yml`:
Flatpak bundle (`build-aux/io.github.sophiedkk.Transient.json`, not for Flathub)
and AppImage (`dev/build-appimage.sh`, Fedora 43, pinned linuxdeploy) into a draft
release with notes from `dev/release-notes.sh`.

## Roadmap and way of working

- `ROADMAP.md` holds the milestones, open tasks and known gaps; it is the source of
  truth for what comes next. Update it when a milestone is finished or plans change.
- Plan together first: ask questions (with a recommended option) before big decisions.
- Work milestone by milestone. After each one: `just check`, verify with screenshots
  and headless runs, update the installed Flatpak (the flatpak-builder command from
  `just flatpak`, without running the app), then commit and push to `main` when the
  owner asks. Report what was and was not verified.
- The owner tests by hand in the installed Flatpak and reports UX problems: fix those first.
- No GitHub issues yet; keep plans in `ROADMAP.md`.

## Git

- Branches: `feat/…`, `fix/…`, `chore/…`, `docs/…`; Conventional Commit messages.
- No AI attribution lines (`Co-Authored-By: Claude`, "Generated with Claude
  Code", 🤖) in commits, PRs or comments: messages describe the change, and the
  README's AI disclosure section is where the use of AI is recorded.
- User-visible changes get an entry under `[Unreleased]` in `CHANGELOG.md`.
- Commit or push only when asked.
