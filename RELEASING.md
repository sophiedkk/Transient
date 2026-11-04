# Releasing Transient

Releases are made by pushing a version tag. The release workflow
(`.github/workflows/release.yml`) then builds a Flatpak bundle and an AppImage
and creates a **draft** release on GitHub; you check it and press Publish.

Versions follow [Semantic Versioning](https://semver.org/): before 1.0, a new
feature raises the middle number (0.1.0 → 0.2.0), a fix-only release the last
(0.2.0 → 0.2.1).

## Checklist

1. **Test by hand** in the installed Flatpak (`just flatpak`): the things the
   automated runs cannot do (dragging, recording with a real microphone,
   opening from Files), plus whatever changed in this release.
2. **Version number** in `Cargo.toml` (`[workspace.package] version`) and
   `meson.build` (`project(… version: …)`); both must match.
3. **`CHANGELOG.md`**: rename `## [Unreleased]` to `## [0.2.0] - YYYY-MM-DD` and
   add a fresh, empty `## [Unreleased]` above it. The release notes are taken
   from this section (`dev/release-notes.sh 0.2.0` shows them).
4. **MetaInfo** (`data/io.github.sophiedkk.Transient.metainfo.xml.in.in`): add a
   `<release version="0.2.0" date="YYYY-MM-DD">` with a short description.
   Software and Flathub show these.
5. **`just check`** passes, and **commit**: `chore: release 0.2.0`.
6. **Tag and push**: `git tag -a v0.2.0 -m "Transient 0.2.0"` then
   `git push origin main v0.2.0`.
7. **Wait for the workflow** (`gh run watch`), then open the draft on GitHub's
   Releases page: check the notes and that both files are attached, then
   **Publish**.

## What the release contains

- `Transient-X.Y.Z.flatpak`: installs with `flatpak install --user
  Transient-X.Y.Z.flatpak`; it fetches the GNOME runtime from Flathub by itself.
  Built from `build-aux/io.github.sophiedkk.Transient.json`, which is for these
  bundles only: Flathub needs its own, hand-written manifest (see `ROADMAP.md`,
  "Publishing").
- `Transient-X.Y.Z-x86_64.AppImage`: runs without installing, on Fedora 43+,
  Ubuntu 26.04+, Arch and openSUSE Tumbleweed (built on Fedora 43 by
  `dev/build-appimage.sh`; older systems have a too-old glibc and GTK).

## Trying the builds without releasing

`gh workflow run release.yml` builds both files without making a release; they
appear as artifacts on the workflow run's page.
