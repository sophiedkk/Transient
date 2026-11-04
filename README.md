<div align="center">

<img src="data/icons/hicolor/scalable/apps/io.github.sophiedkk.Transient.svg" width="128" height="128" alt="Transient's icon: five waveform bars in light blue, pink and white">

# Transient

A simple multitrack audio editor for GNOME

[![CI](https://github.com/sophiedkk/Transient/actions/workflows/ci.yml/badge.svg)](https://github.com/sophiedkk/Transient/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/sophiedkk/Transient)](https://github.com/sophiedkk/Transient/releases/latest)
[![Licence: MIT](https://img.shields.io/github/license/sophiedkk/Transient)](LICENSE)
[![Made for GNOME](https://img.shields.io/badge/made_for-GNOME-4a86cf?logo=gnome&logoColor=white)](https://apps.gnome.org/)

</div>

Put audio files on tracks, trim and arrange them, record over them, balance
their volume, and export the mix. Built with Rust, GTK 4, Libadwaita and
GStreamer.

![Transient with four tracks: a voice clip selected, a piano bed and drums with
fades, and applause at the end](data/screenshots/main-window.png)

> **Status:** early development (0.1). You can import or record audio,
> arrange, trim and cut it on tracks, add fades, mix the tracks and export the
> result; see [`ROADMAP.md`](ROADMAP.md) for what comes next.

## Install

Download a file from the [latest release](https://github.com/sophiedkk/Transient/releases/latest):

- **Flatpak** (any distribution with Flatpak): `flatpak install --user
  Transient-0.1.0.flatpak`. It fetches the GNOME runtime from Flathub by itself.
- **AppImage** (Fedora 43+, Ubuntu 26.04+, Arch, openSUSE Tumbleweed): make it
  executable (`chmod +x Transient-0.1.0-x86_64.AppImage`) and start it.

## The name

In audio, a *transient* is the short, sharp burst at the start of a sound: the
crack of a snare drum, the "t" in "test", the moment a string is plucked. They
are what you look for when you cut audio, and the tall white bar in the middle
of the icon is one. The icon's light blue, pink and white come from the
transgender pride flag, which the name nods to as well.

## Development

Development happens inside a [Toolbx](https://containertoolbx.org/) container, so
nothing has to be installed on your system except Podman, Toolbx and Flatpak
(all included in Fedora Workstation) and [rustup](https://rustup.rs/).

### First-time setup (on the host)

```sh
dev/setup.sh
```

This builds the `transient-dev` image from [`dev/Containerfile`](dev/Containerfile),
creates the toolbox, installs the Rust version from `rust-toolchain.toml`, enables the
git hooks and adds a **RustRover (Transient)** launcher that runs RustRover inside the
toolbox. After changing `dev/Containerfile` or `dev/packages.txt`, run
`dev/setup.sh --rebuild`.

### Everyday work (inside the toolbox)

```sh
toolbox enter transient-dev
just run        # build and start Transient
just test       # run the tests
just check      # everything CI checks: format, clippy, tests, cargo-deny, metadata
just flatpak    # build, install and run the Flatpak
```

Test audio (free licences, from Wikimedia Commons) is downloaded with
`dev/fetch-test-audio.sh` into `test-audio/`; `just test-audio` decodes all of it.

### Without a window

```sh
just run test-audio/piano.flac                  # open a file directly
just probe test-audio/*                         # decode files, print JSON
just screenshot test-audio/piano.flac           # save target/screenshot.png
just headless --silent --play --quit-after=3 test-audio/drum-loop.wav
```

`just headless` and `just screenshot` run Transient in an invisible GNOME compositor,
which is handy for testing and for AI-assisted development. `transient --help` lists
the command-line options.

`just` without arguments lists all recipes.

### Project layout

| Path | What it is |
|---|---|
| `crates/transient-core` | Project model and audio maths. No GTK or GStreamer, so everything is unit-tested. |
| `crates/transient-engine` | Audio input and output with GStreamer: decoding, playback, export. |
| `crates/transient-cli` | `transient-cli`: the engine on the command line (JSON output). |
| `crates/transient` | The GTK app. UI layouts are Blueprint files in `resources/ui/`. |
| `data/` | Desktop file, AppStream metainfo, icons and the GSettings schema. |
| `po/` | Translations. |
| `build-aux/` | Flatpak manifest and Meson helper script. |
| `dev/` | Toolbx image, setup script and RustRover launcher. |

Dependencies point one way: `transient` and `transient-cli` → `transient-engine` →
`transient-core`.

## License

[MIT](LICENSE)
