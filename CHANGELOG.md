# Changelog

All notable changes to Transient are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-04

The first release.

### Added

- Project setup: Cargo workspace (`transient-core`, `transient-engine`, `transient`),
  Meson build, Flatpak manifest, Toolbx development environment and CI.
- Main window with an empty state, a header bar and the primary menu.
- Import and play an audio file (MP3, FLAC, Ogg Vorbis, Opus, WAV and anything
  else GStreamer can decode), converted to 48 kHz stereo in memory.
- Multitrack timeline: importing files puts each on its own track. A time ruler,
  one lane per track with its clips and waveforms, track headers with the name
  and original format, and a playhead across all tracks.
- Mixing: all tracks play together; edits reach the player without stopping it.
- Editing: click a clip to select it, drag it to move it (also to another track),
  drag its edges to trim it; edges snap to other clips, the playhead and the
  start. Split at the playhead (S), delete (Delete), nudge (Ctrl+Left/Right),
  deselect (Escape). Clips on one track stack: a moved clip goes on top, and
  where clips overlap you see and hear only the top one. Nothing is cut, so
  moving the top clip away brings the clip underneath back whole. Where the top
  clip fades in or out, the clip underneath fades the other way, so they
  crossfade. Clips are labelled with the file they play. A track whose last
  clip is dragged to another track is removed; Remove Track in a track's menu
  removes one yourself (with an Undo toast). Dragging a clip below the last
  track, or Move to New Track in the clip pane, gives it a track of its own.
- Time selection: drag over a clip's waveform (or empty space) to select a
  stretch of time on the tracks you drag across; drag a clip by its name strip
  to move it. Delete cuts the selection, Ctrl+Delete ("Delete and Close Gap")
  cuts it and moves everything after it to the left on all tracks, S splits at
  both of its ends, Ctrl+A selects everything and Escape clears it.
- Without shortcuts: a context menu on clips and selections (right-click, long
  press, or the Menu key / Shift+F10) with Split, Delete, Delete and Close Gap,
  Move to New Track and Select All, showing their shortcuts; Split and Delete
  buttons in the bottom bar, whose tooltips explain how to select.
- Tracks: rename one by double-clicking its name, with Rename… in its menu or
  F2; move it with Move Up / Move Down in its menu or Alt+Up / Alt+Down.
  Imported files start at the playhead.
- Several clips at once: Ctrl+click or Shift+click adds a clip to the
  selection (or removes it). Dragging one of them by its name strip moves them
  all, also across tracks; Ctrl+Left/Right, Delete and S act on all of them.
- Cut, Copy and Paste (Ctrl+X/C/V, also in the context menu) for clips and time
  selections: Paste puts the copy at the playhead, starting on the selected
  track and keeping its arrangement, or right after the audio there, so it
  never covers anything; the playhead then moves to its end, so pasting again
  lines the copies up. "Paste and Make Room" (Shift+Ctrl+V) first moves
  everything after the playhead to the right on all tracks.
- The bottom bar shows a time selection's start, end and length, and Export
  can export just the selection (its stretch of time on its tracks).
- Mixer: each track header has mute, solo, volume (−60 to +12 dB) and pan
  sliders; the mute button shows whether the track is heard. The bottom bar
  has a master volume and a slim stereo output level meter (red when the mix
  clips, dimmed while nothing plays). Tracks that are not heard are drawn faded.
  M and Shift+M mute and solo the selected clip's track. A slider drag is one
  undo step.
- Fades and clip gain: drag the round handles on a selected clip, or use the
  clip pane (F9 or the sidebar button) to set gain, fade-in and fade-out, or
  fade in to / out from the playhead. Fades use an equal-power curve, so
  overlapping clips with fades crossfade smoothly. Fades are drawn as shaded
  curves; waveforms show the clip gain.
- Normalize a clip: peak to −1 dBFS, or loudness to −16 LUFS (EBU R128) without
  letting the peak pass −1 dBFS.
- Projects: New (Ctrl+N), Open (Ctrl+O), Save (Ctrl+S) and Save As
  (Shift+Ctrl+S) for `.transient` project files, also opened from the command
  line, from Files and from the recent projects on the start screen. A project
  file holds a copy of all its audio (as imported: an MP3 stays an MP3), so it
  can be moved, backed up or shared as one file and never loses its audio.
  A "•" in the title marks unsaved changes; closing, quitting or starting a new
  project asks to save them. Projects from before this change still open (and
  take in their audio when saved); audio missing there is reported in a
  banner and its clips stay, greyed out.
- Export (Ctrl+E, header bar button or menu): the whole mix to WAV or FLAC
  (16/24-bit), MP3, Opus or Ogg Vorbis, with a progress bar and Stop; a toast
  offers "Show in Folder". `transient-cli export` exports projects or files
  without a window.
- Undo and redo for every change (Ctrl+Z, Shift+Ctrl+Z, and buttons in the
  header bar). Deleting shows a toast with an Undo button instead of asking
  for confirmation.
- Zoom and scroll the timeline (with a little space before 0:00, so the
  playhead and the first clip's edge are clear of the track headers): click or
  drag to move the playhead,
  Shift+scroll or a touchpad to move sideways, Ctrl+scroll,
  Ctrl+plus/minus/0 or the zoom buttons to zoom. The view follows the playhead
  while playing.
- Touch and touchpad: pinch to zoom around your fingers (moving both fingers
  also scrolls), the timeline glides on after a quick sideways swipe, and
  touchpad scrolling follows the fingers exactly. On a touchscreen, clip edges
  and fade handles are easier to grab.
- Dragging a clip, a clip edge, a fade handle or the playhead near the left or
  right edge of the timeline scrolls it, faster the closer you get.
- Transport bar: play/pause (Space), go to start (Home) and the current time.
  The file's name, format and length are shown in the header bar.
- Open files from the command line (`transient FILE`) and from "Open With".
- Command-line options for testing: `--play`, `--silent`, `--screenshot`,
  `--quit-after`, `--seek`, `--zoom`, `--action`, `--save-to`, `--export-to`; `transient-cli probe` decodes files and
  `transient-cli mix` mixes them, both printing JSON.
- Keyboard shortcuts dialog (Ctrl+?) and About dialog.
- Recording: the Record button (or R) records onto a new track at the
  playhead, while the other tracks play along; the level meter shows the
  input. Stop (or Space) ends the take, which is saved as FLAC inside the
  project; while recording, the take grows on the timeline. The start page has
  a Record Audio button. The arrow next to the Record button chooses the input (the system
  default or a specific microphone); the choice is remembered.
  `transient-cli inputs` lists the inputs.
- The timeline works from the keyboard: it gets focus when a project opens (or
  with Tab or a click); Left/Right move the playhead, Shift+Left/Right select
  time, Up/Down go to the track above or below and select the clip there, End
  goes to the end. A band shows the current track. Screen readers get names
  for every icon button and a description of the timeline (track, playhead,
  selection), and hear track and selection changes.
- App icon: five waveform bars in the colours of the trans flag, the tall white
  one in the middle being the "transient"; a matching symbolic icon, and a
  striped variant for development builds. The start screen shows the app icon.
- The window remembers its size.
