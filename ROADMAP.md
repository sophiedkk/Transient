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
- [ ] **Application icon.** Replace the placeholder
      (`data/icons/hicolor/scalable/apps/`) with an icon that follows the
      [GNOME HIG app icon guidelines](https://developer.gnome.org/hig/guidelines/app-icons.html):
      128×128 grid, simple shapes, the GNOME palette, a slight bottom edge for
      depth, readable at 32 px and 16 px. Take inspiration from the trans flag:
      light blue (#5BCEFA), pink (#F5A9B8) and white, matched to the nearest
      GNOME palette colours. Keep the waveform-with-a-transient idea. Update the
      symbolic icon (`hicolor/symbolic/apps/`) to match and check both in App
      Icon Preview.
- [ ] **Preferences dialog** (e.g. default export format).
- [ ] **Export a selection**, not only the whole project (planned for milestone
      8; needs time selection from milestone 10).
- [ ] **"Locate…" for missing audio files** in the missing-files banner.
- [ ] **Keyboard focus and accessibility pass.**
- [ ] **Flathub-ready**: AppStream metainfo with screenshots and releases,
      vendored Cargo sources (flatpak-cargo-generator), reconsider
      `--filesystem=home`.
- [ ] **CI**: check for flaky jobs.

## 10 · Cutting

Inspired by Audacity 4's clip editing, kept GNOME-simple (no tool modes).

- [ ] **Time selection.** Drag over empty lane space or the ruler to select a time
      range, as well as selecting a clip. Delete removes only that range, leaving
      a gap. The maths (two splits, then remove the piece in between) lives in
      `transient-core::edit`, covered by property tests.
- [ ] **Ripple delete** (Ctrl+Delete, "Delete and Close Gap"): remove the
      selection and shift everything after it left on all tracks, so the tracks
      stay in sync.
- [ ] **Split at the mouse**: hold S (or Ctrl) and click inside a clip to split it
      there. S on its own keeps splitting at the playhead.

## 11 · Tracks

Tracks stay (the mixer and ripple delete need them), but they should appear
when needed instead of being managed by hand.

- [x] **New track by dropping** a clip below the last track, or with Move to
      New Track in the clip pane (`Project::move_clip_to_new_track`, unit-tested).
- [ ] **Import onto the selected track** at the playhead, or onto a new track as now.
- [ ] **Rename** (double-click the name) and **reorder** (drag the header, plus
      a keyboard shortcut), in the track menu next to Remove Track.
- [x] **Remove Track** in the track menu (Undo toast, no confirmation). A track
      whose last clip is dragged to another track is removed automatically
      (decided 2026-11-04: tracks come from imported files).
- No mono/stereo track types: everything is stereo, and pan covers left/right.

## 12 · Clipboard and multiple selection

- [ ] **Multiple selection**: Shift+click selects several clips; move, nudge and
      delete act on all of them.
- [ ] **Cut, Copy, Paste** (Ctrl+X/C/V) for clips and time ranges, plus "Insert
      and Keep Tracks in Sync" (Shift+Ctrl+V).

## Later (after the MVP)

- Recording
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
