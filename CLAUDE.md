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
  file, render the mix) and `export PROJECT|FILES --output OUT.ext` — the engine
  without a window, JSON output.
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
- Undo/redo = snapshots of the whole project (`transient-core::edit::History`).
  Every change goes through `Window::apply_edit`, which records the snapshot.
  The timeline only previews drags and emits `clip-changed` on release.
- Clips carry `gain_db`, `fade_in`, `fade_out` (`Clip::with_fades` keeps fades
  inside the clip; trims and splits preserve that). `Mix` applies equal-power
  fades (`fade_shape`). `transient-core::normalize` computes peak/LUFS gains
  (ebur128). The clip pane (`window/clip_pane.rs`, F9) edits the selected clip.
- Project files (`window/file.rs`): `.transient` JSON; `Project::with_paths_relative_to`
  / `with_paths_resolved_from` make sources inside the project folder relative.
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
  by keyboard. Keep `resources/ui/shortcuts-dialog.blp` in sync with
  `src/application.rs`.
- Header capitalization for buttons and menus, sentence capitalization for labels;
  an ellipsis (…) only when more input follows. All UI text is translatable
  (`_()` in Blueprint, `gettext()` in Rust) and listed in `po/POTFILES.in`.
- Custom drawing uses theme colours (widget colour, accent), never hardcoded ones.

## Code rules

- Lints are set in the root `Cargo.toml` (`[workspace.lints]`), clippy settings in
  `clippy.toml`. CI runs clippy with `-D warnings`: fix warnings, don't silence them.
  An exception is a narrow `#[expect(lint, reason = "...")]` at the site.
- No `unwrap`/`expect`/`panic!` outside tests: a panic in a GTK callback aborts the
  app. Return `Result`; show the user a toast; log with `tracing`.
- `unsafe_code` is forbidden in the whole workspace.
- `thiserror` for errors in library crates; the app logs and reports them.
- One `impl Default` per struct, and container-level `#[serde(default)]` on project
  file structs so older files keep opening. Changing the file format means raising
  `project::FORMAT_VERSION` and updating the snapshot test.
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
  `--select=START..END` (seconds, every track; runs before the actions), and
  `--action=NAME` (repeatable; runs `win.NAME`, e.g. `split`, `undo`,
  `select-clip-at-playhead`, `select-all`, `context-menu` (opens it; popovers are
  not in screenshots, check the debug log), `nudge-right`, `delete-clip`
  (deletes the time selection if there is one), `ripple-delete`, `toggle-mute`,
  `toggle-solo`, `fade-in-to-playhead`, `fade-out-from-playhead`,
  `normalize-peak`, `normalize-loudness`, `move-to-new-track`, `remove-track`,
  `toggle-clip-pane`, `new-project`, `app.quit`) to script edits. `--save-to=FILE` saves afterwards; passing a
  `.transient` file opens it — together they test a full save/open round trip.
  `--export-to=FILE.ext` exports through the app's export path. With `--play`, the level meter (red = clipping)
  shows whether the mix is as loud as expected.
  Mouse drags cannot be scripted; their maths lives in `transient-core::edit`
  and `transient-core::range` (unit- and property-tested).
  Add a new option in `src/launch.rs` when a feature needs scripted testing.
- Never play audio through the speakers in automated runs: always pass `--silent`.
- `test-audio/` is downloaded by `dev/fetch-test-audio.sh` from `dev/test-audio.tsv`
  (Wikimedia Commons, checksummed, licences in `test-audio/CREDITS.md`). It is not
  committed. Add files by adding a manifest row with its SHA-256.

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
- **Never add AI attribution** (`Co-Authored-By: Claude`, "Generated with Claude
  Code", 🤖) to commits, PRs or comments. `.githooks/commit-msg` strips it as a backstop.
- User-visible changes get an entry under `[Unreleased]` in `CHANGELOG.md`.
- Commit or push only when asked.
