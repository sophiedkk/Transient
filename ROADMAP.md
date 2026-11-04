# Roadmap

Transient is built milestone by milestone; each one ends with something that
runs. Finished work is described in `CHANGELOG.md`. This file is the source of
truth for what comes next: update it when a milestone is finished or plans
change.

## Done

| # | Milestone | Commit |
|---|---|---|
| 0 | Project setup and an empty Adwaita window | `fda7462` |
| 1 | Import and play one audio file | `fda7462` |
| 2 | Waveform view with zoom, scrolling and seeking | `983b566` |
| 3 | Tracks and timeline, mixing several files | `9055854` |
| 4 | Editing (move, trim, split, delete, snapping) with undo/redo | `6330f9f` |
| 5 | Mixer: track volume, pan, mute, solo, master volume, level meter | `c9fa9ae` |
| 6 | Fades, clip gain, normalize (peak/LUFS), clip pane | `7210ccf` |
| 7 | Save and open projects, recent projects | `f603d76` |
| 8 | Export to WAV, FLAC, MP3, Opus, Ogg Vorbis | `33e85b2` |

Already there from the original milestone 9 list: shortcuts dialog, About
dialog, dark mode (Libadwaita), error toasts.

## 9 · Polish ("usable every day")

- [x] **Touch and touchpad gestures** (requested): pinch to zoom
      (`GtkGestureZoom`, anchored at the point between the fingers), kinetic
      scrolling, dragging on a touchscreen. Two-finger sideways scroll and
      Ctrl+scroll zoom already work.
- [x] **Auto-scroll** while dragging a clip or clip edge near the edge of the view.
- [x] **Application icon.** Replace the placeholder
      (`data/icons/hicolor/scalable/apps/`) with an icon that follows the
      [GNOME HIG app icon guidelines](https://developer.gnome.org/hig/guidelines/app-icons.html):
      128×128 grid, simple shapes, the GNOME palette, a slight bottom edge for
      depth, readable at 32 px and 16 px. Take inspiration from the trans flag:
      light blue (#5BCEFA), pink (#F5A9B8) and white, matched to the nearest
      GNOME palette colours. Keep the waveform-with-a-transient idea. Update the
      symbolic icon (`hicolor/symbolic/apps/`) to match and check both in App
      Icon Preview.
- [ ] **Preferences dialog** (e.g. default export format).
- [x] **Export a selection**: the export dialog offers Whole Project or
      Selection (the selected stretch on the selected tracks).
- [ ] **"Locate…" for missing audio files** in the missing-files banner. Only
      projects saved before format 2 (and audio that was missing when saving)
      can still lack audio, so this matters much less now.
- [x] **Keyboard focus and accessibility pass**: the timeline is usable from
      the keyboard (arrows like a text cursor, Up/Down for tracks), every icon
      button has an accessible name, and the timeline describes and announces
      its state. Not yet checked with a real screen reader (Orca).
- [ ] **Release-ready MetaInfo**: screenshots, a releases list and a content
      rating (see "Publishing" below).
- [ ] **CI**: check for flaky jobs.

## 10 · Cutting

Inspired by Audacity 4's clip editing, kept GNOME-simple (no tool modes).

- [x] **Time selection** (decided 2026-11-04, like Audacity 4): drag over a
      clip's waveform or empty lane space to select time over the lanes dragged
      across; a clip's name strip moves it. Ctrl+A selects everything, Escape
      clears. Delete cuts the selection, leaving a gap (`transient-core::range`,
      property-tested).
- [x] **Ripple delete** (Ctrl+Delete, "Delete and Close Gap"): cuts the
      selection and moves everything after it left on all tracks; tracks outside
      the selection are moved, not cut.
- [x] **Split a selection**: S splits at both ends of the time selection (no
      separate split-at-the-mouse gesture: click, then press S).
- [x] **Show the selection's start, end and length** in the bottom bar.

## 11 · Tracks

Tracks stay (the mixer and ripple delete need them), but they should appear
when needed instead of being managed by hand.

- [x] **New track by dropping** a clip below the last track, or with Move to
      New Track in the clip pane (`Project::move_clip_to_new_track`, unit-tested).
- [x] **Import at the playhead** (decided 2026-11-04): every imported file
      still gets a new track, starting at the playhead instead of 0:00.
- [x] **Rename** (double-click the name, Rename… in the track menu, or F2) and
      **Move Up / Move Down** in the track menu (Alt+Up / Alt+Down).
- [ ] **Reorder by dragging** a track header (later; the menu covers it for now).
- [x] **Remove Track** in the track menu (Undo toast, no confirmation). A track
      whose last clip is dragged to another track is removed automatically
      (decided 2026-11-04: tracks come from imported files).
- No mono/stereo track types: everything is stereo, and pan covers left/right.

## 12 · Clipboard and multiple selection

- [x] **Multiple selection**: Ctrl+click or Shift+click selects several clips; move, nudge and
      delete act on all of them.
- [x] **Cut, Copy, Paste** (Ctrl+X/C/V) for clips and time ranges, plus "Paste
      and Make Room" (Shift+Ctrl+V), which keeps tracks in sync.

## 13 · Recording (before 1.0)

Requested 2026-11-04: recording comes before 1.0, not after.

- [x] **Record onto a new track** at the playhead (a Record button in the
      transport bar and R), with the level meter showing the input while
      recording. Stop (or Space) ends the take; it becomes an ordinary clip.
- [x] **Choose the input** (microphone, line in) from the arrow on the Record
      button; remembered in the settings. `transient-cli inputs` lists them.
- [ ] **See the input's level before recording** (to set the microphone's
      volume), for example while the input menu is open.
- [x] **Play the other tracks while recording** (overdubbing), so a voice can be
      recorded over music.
- [ ] **Correct for the delay** between what plays and what is recorded
      (latency), so overdubs line up exactly; measure it with a loopback test.
- [x] **Show the take growing** on the timeline while recording, with its
      outline, below the last track; the start page has a Record Audio button.
- [x] **Where takes are stored**: as FLAC in the project file, like imported
      audio (decided 2026-11-04).
- [ ] **Permissions**: there is no portal for the microphone yet, so the
      Flatpak needs PulseAudio/PipeWire access (`--socket=pulseaudio`), which
      Flathub accepts for audio apps; note it in the submission.

## Publishing (1.0 and Flathub)

Checked against [Flathub's requirements](https://docs.flathub.org/docs/for-app-authors/requirements)
on 2026-11-04; check them again before submitting, the AI rules changed twice
in 2026.

**Flathub's rules on generative AI** (they apply because Transient is largely
written with an AI agent):

- Submitters must disclose AI-generated code, documentation and packaging:
  which parts, and roughly how much. Disclosure does not guarantee acceptance;
  reviewers may reject based on the amount of AI-generated material, code
  quality, or how reviewable and maintainable it is.
- The Flathub manifest must not contain any AI-generated or AI-assisted
  content, even with disclosure. `build-aux/*.Devel.json` was written by the
  agent, so it cannot be submitted.
- No AI or agent may open or automate the submission pull request, write its
  commit messages or description, or write replies to reviewers. AI-agent
  reviews must not be requested.
- Undisclosed or understated AI use can mean rejection, and repeated
  violations a permanent ban. Commit messages describe changes, not the tools
  used to make them, so the extent of AI use is stated in the submission
  itself (see "Draft the AI disclosure" below).

**To do before submitting** (the agent can help with these):

- [ ] **Development history**: Flathub wants sustained commits, tagged
      releases, real-world use and a commitment to maintenance. Tag releases
      regularly (0.1, 0.2, …) and use Transient for real work before 1.0.
- [x] **Project files hold their audio** (decided 2026-11-04, like Audacity's
      `.aup3`): a `.transient` file is a ZIP with the project and its audio as
      is, so the file chooser portal covers opening and saving.
- [ ] **Drop `--filesystem=home`** from the Flatpak. To do (by hand, later):
      remove the line from `build-aux/*.Devel.json`, reinstall the Flatpak
      (`just flatpak`), and check that these work without it:
      1. Open a project (Ctrl+O).
      2. Save, and Save As to a new place.
      3. Double-click a `.transient` file in Files.
      4. Recent projects on the start screen (most likely to fail: the saved
         paths may not be reachable without the permission).
      5. Import audio (Ctrl+I), then save.
      If all work, commit the removal; if not, keep the permission for now and
      look into the failing case.
- [ ] **MetaInfo**: screenshots, a `<releases>` list, a content rating
      (OARS), brand colours; must pass validation (already checked by
      `meson test`).
- [ ] **Translations** only from people: Flathub refuses machine-generated
      translations.
- [ ] **Runtime**: the latest GNOME runtime at the time of submitting.
- [x] **`RELEASING.md`** and a release workflow: a version tag builds a
      Flatpak bundle and an AppImage into a draft GitHub release.
- [x] **First test run of the release workflow**: both files build (run
      41901052553, 2026-11-04; the AppImage is about 120 MB).
- [ ] **Try the AppImage on a desktop** (by hand): icons, dark mode following
      the system, native Wayland, playback, recording, opening a project.
      Download it with `gh run download <run id> -n appimage`.
- [ ] **AppImage translations**: Transient looks for translations in an
      absolute folder (`/usr/share/locale`), which an AppImage cannot provide;
      once there are translations, look them up relative to the program.
- [ ] **Draft the AI disclosure** (a statement about the project; not part of
      the manifest or the pull request text).

**To do by the maintainer, without AI** (Flathub's rules):

- [ ] Write the Flathub manifest `io.github.sophiedkk.Transient.json`, with Rust
      crates as sources (generated by `flatpak-cargo-generator`, a regular
      tool) and no network access during the build.
- [ ] Open the submission pull request on `flathub/flathub` and handle the
      review conversation.

Already fine: the app ID (`io.github.` prefix, verified through the GitHub
account), the MIT licence, the SVG icon and the desktop file.

**If Flathub says no**: publish a `.flatpak` bundle with each GitHub release
(CI already builds one), or host our own Flatpak repository.

## Later (after the MVP)

- Effects (the model leaves room for per-track effects)
- Volume automation
- Level meters per track, in the track headers (the master meter is under the
  master volume)
- Clip names and colours (a fixed set that works in light and dark themes)
- Markers and regions, and exporting one file per region
- GitHub issue-based workflow instead of this file (not yet)

Deliberately left out: changing a clip's speed by stretching it, drawing
individual samples, workspaces, plugin hosting.

## Open questions and known gaps

- Clips on one track stack (decided 2026-11-04, like Ardour): where they
  overlap, only the top one is seen and heard; nothing is cut. Fades on the
  top clip make a crossfade. A clip covered completely is hidden and cannot be
  clicked until the clip on top is moved away; it may need a marker, or a
  "Bring to Front"/"Send to Back" command.
- Mouse interaction (dragging clips, trim and fade handles, header sliders,
  spin rows, file choosers) is not covered by automation, only the maths behind
  it and the scripted actions. Needs testing by hand.
- Waveforms show clip gain but not fades.
