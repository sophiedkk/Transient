#!/usr/bin/env bash
# Builds Transient as an AppImage: one file that runs without installing, on
# recent Linux distributions. Used by the release workflow
# (.github/workflows/release.yml); it runs on Fedora 43, the oldest system with
# GTK 4.20 and Libadwaita 1.8, so the AppImage runs on Fedora 43+, Ubuntu 26.04+,
# Arch and openSUSE Tumbleweed (newer glibc), not on older systems.
#
#   dev/build-appimage.sh 0.1.0      # → _appimage/Transient-0.1.0-x86_64.AppImage
#
# Needs the packages in dev/packages.txt plus patchelf, file and curl.
#
# How it works: Meson installs Transient into an AppDir (a folder laid out like
# /usr), then linuxdeploy copies in every library it needs, with its plugins
# for GTK (settings, pixbuf loaders) and GStreamer (all installed plugins, so
# the AppImage decodes what the system here decodes). The tools are downloaded
# at fixed versions and checked against their SHA-256 sums.
set -euo pipefail

version="${1:?usage: dev/build-appimage.sh VERSION}"
app_id="io.github.sophiedkk.Transient"
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$root/_appimage"
tools="$work/tools"
appdir="$work/AppDir"

# linuxdeploy and its plugins, pinned (a release, or a commit for the scripts).
declare -A urls=(
    [linuxdeploy-x86_64.AppImage]="https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
    [linuxdeploy-plugin-appimage-x86_64.AppImage]="https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/1-alpha-20250213-1/linuxdeploy-plugin-appimage-x86_64.AppImage"
    [linuxdeploy-plugin-gtk.sh]="https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/7a3fbc31a9e5075073ff8790f26effbac5f84453/linuxdeploy-plugin-gtk.sh"
    [linuxdeploy-plugin-gstreamer.sh]="https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gstreamer/432af639725bc8b99df12351e86bb2e8a6b3585f/linuxdeploy-plugin-gstreamer.sh"
)
declare -A sums=(
    [linuxdeploy-x86_64.AppImage]="c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"
    [linuxdeploy-plugin-appimage-x86_64.AppImage]="992d502a248e14ab185448ddf6f6e7d25558cb84d4623c354c3af350c25fccb3"
    [linuxdeploy-plugin-gtk.sh]="b0f4cbc684a0103a9651f0955b635eaea0096b3a66c0f5a2c2aa337960375171"
    [linuxdeploy-plugin-gstreamer.sh]="d1cf90701047aa9583c274bb66dd82db352255374fbfb8f3c4c89a05fe23bce0"
)

mkdir -p "$tools"
for name in "${!urls[@]}"; do
    if [ ! -f "$tools/$name" ]; then
        curl --fail --silent --show-error --location -o "$tools/$name" "${urls[$name]}"
    fi
    echo "${sums[$name]}  $tools/$name" | sha256sum --check --quiet
    chmod +x "$tools/$name"
done
export PATH="$tools:$PATH"
# The tools are AppImages themselves; in a container there is no FUSE to mount them.
export APPIMAGE_EXTRACT_AND_RUN=1

# 1. Build and install Transient into the AppDir.
rm -rf "$work/build" "$appdir"
meson setup "$work/build" "$root" --prefix=/usr --buildtype=release -Dprofile=default
meson compile -C "$work/build"
DESTDIR="$appdir" meson install -C "$work/build"

# Libadwaita apps use the Adwaita icon theme's symbolic icons (play, record, …);
# systems without GNOME may not have them.
mkdir -p "$appdir/usr/share/icons"
cp -r /usr/share/icons/Adwaita "$appdir/usr/share/icons/"

# 2. Copy in the libraries, GTK's and GStreamer's files.
# linuxdeploy's own strip is too old for Fedora's libraries (it fails on their
# .relr.dyn sections); they are stripped already, so skip it.
export NO_STRIP=1
export DEPLOY_GTK_VERSION=4
export GSTREAMER_PLUGINS_DIR=/usr/lib64/gstreamer-1.0
export GSTREAMER_HELPERS_DIR=/usr/libexec/gstreamer-1.0
linuxdeploy-x86_64.AppImage --appdir "$appdir" \
    --desktop-file "$appdir/usr/share/applications/$app_id.desktop" \
    --icon-file "$appdir/usr/share/icons/hicolor/scalable/apps/$app_id.svg" \
    --plugin gtk --plugin gstreamer

# The GTK plugin's start-up hook was written for GTK 3: it forces a theme
# (GTK_THEME), which breaks Libadwaita's own styling (dark mode, accent
# colours), and X11 (GDK_BACKEND), which loses native Wayland. Remove both.
hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
sed -i -e '/^export GTK_THEME=/d' -e '/^export GDK_BACKEND=/d' "$hook"
if grep -qE '^export (GTK_THEME|GDK_BACKEND)=' "$hook"; then
    echo "error: could not remove GTK_THEME/GDK_BACKEND from $hook" >&2
    exit 1
fi

# 3. Pack the AppDir into one file.
export OUTPUT="$work/Transient-$version-x86_64.AppImage"
linuxdeploy-x86_64.AppImage --appdir "$appdir" --output appimage
echo "built $OUTPUT"
