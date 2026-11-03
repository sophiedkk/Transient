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
- Transport bar: play/pause (Space), go to start (Home), a position slider and
  the current time.
- Open files from the command line (`transient FILE`) and from "Open With".
- Command-line options for testing: `--play`, `--silent`, `--screenshot`,
  `--quit-after`; `transient-cli probe` decodes files and prints JSON.
- Keyboard shortcuts dialog (Ctrl+?) and About dialog.
- The window remembers its size.
