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
- Waveform view: the whole file at a glance, drawn in the accent colour from a
  precomputed peak overview. Click or drag to move the playhead; scroll to move
  through the file; zoom with Ctrl+scroll, Ctrl+plus/minus/0 or the zoom
  buttons. The view follows the playhead while playing.
- Transport bar: play/pause (Space), go to start (Home) and the current time.
  The file's name, format and length are shown in the header bar.
- Open files from the command line (`transient FILE`) and from "Open With".
- Command-line options for testing: `--play`, `--silent`, `--screenshot`,
  `--quit-after`, `--seek`, `--zoom`; `transient-cli probe` decodes files and prints JSON.
- Keyboard shortcuts dialog (Ctrl+?) and About dialog.
- The window remembers its size.
