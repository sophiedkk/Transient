# Changelog

All notable changes to Transient are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
  deselect (Escape).
- Mixer: each track header has mute, solo, volume (−60 to +12 dB) and pan
  sliders; a master volume and a stereo output level meter (red when the mix
  clips) are in the bottom bar. Tracks that are not heard are drawn faded.
  M and Shift+M mute and solo the selected clip's track. A slider drag is one
  undo step.
- Fades and clip gain: drag the round handles on a selected clip, or use the
  clip pane (F9 or the sidebar button) to set gain, fade-in and fade-out, or
  fade in to / out from the playhead. Fades use an equal-power curve, so
  overlapping clips with fades crossfade smoothly. Fades are drawn as shaded
  curves; waveforms show the clip gain.
- Normalize a clip: peak to −1 dBFS, or loudness to −16 LUFS (EBU R128) without
  letting the peak pass −1 dBFS.
- Undo and redo for every change (Ctrl+Z, Shift+Ctrl+Z, and buttons in the
  header bar). Deleting shows a toast with an Undo button instead of asking
  for confirmation.
- Zoom and scroll the timeline: click or drag to move the playhead,
  Shift+scroll or a touchpad to move sideways, Ctrl+scroll,
  Ctrl+plus/minus/0 or the zoom buttons to zoom. The view follows the playhead
  while playing.
- Transport bar: play/pause (Space), go to start (Home) and the current time.
  The file's name, format and length are shown in the header bar.
- Open files from the command line (`transient FILE`) and from "Open With".
- Command-line options for testing: `--play`, `--silent`, `--screenshot`,
  `--quit-after`, `--seek`, `--zoom`, `--action`; `transient-cli probe` decodes files and
  `transient-cli mix` mixes them, both printing JSON.
- Keyboard shortcuts dialog (Ctrl+?) and About dialog.
- The window remembers its size.
