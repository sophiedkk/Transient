#!/usr/bin/env bash
# One-time setup on the HOST (not inside the toolbox). Needs no sudo.
#
#   dev/setup.sh            create the transient-dev toolbox
#   dev/setup.sh --rebuild  rebuild the image and recreate the toolbox (after
#                           changing dev/Containerfile, dev/packages.txt or
#                           rust-toolchain.toml)
#
# Everything the build needs, Rust included, is in the image (see
# dev/Containerfile); nothing is installed in your home folder.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
image="localhost/transient-dev:latest"
container="transient-dev"

if [ -f /run/.toolboxenv ]; then
    echo "Run this on the host, not inside a toolbox." >&2
    exit 1
fi

if [ "${1:-}" = "--rebuild" ] || ! podman image exists "$image"; then
    echo "==> Building the development image"
    # The build context is only dev/ (so the large target/ folder is not sent
    # along); the extra "repo" context gives the image rust-toolchain.toml.
    podman build -t "$image" -f "$repo/dev/Containerfile" \
        --build-context repo="$repo" "$repo/dev"
    podman rm -f "$container" >/dev/null 2>&1 || true
fi

if ! podman container exists "$container"; then
    echo "==> Creating the $container toolbox"
    toolbox create --image "$image" "$container"
fi

echo "==> Checking the Rust version in the toolbox"
wanted="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$repo/rust-toolchain.toml")"
installed="$(toolbox run --container "$container" rustc --version | cut -d' ' -f2)"
if [ "$installed" != "$wanted" ]; then
    echo "The toolbox has Rust $installed; rust-toolchain.toml asks for $wanted." >&2
    echo "Run dev/setup.sh --rebuild." >&2
    exit 1
fi
echo "    Rust $installed"

# RustRover runs on the desktop and cannot see inside the container, so it gets
# a copy of the standard library's source (for completion and go-to-definition
# in std) in a folder both sides share. Named after the Rust version, so a new
# version gets a fresh copy.
std_src="$HOME/.cache/transient-dev/rust-src-$wanted"
if [ ! -d "$std_src" ]; then
    echo "==> Copying the standard library's source for RustRover"
    mkdir -p "$(dirname "$std_src")"
    toolbox run --container "$container" sh -c \
        'cp -r "$(rustc --print sysroot)/lib/rustlib/src/rust/library" "$1"' sh "$std_src"
fi

# Earlier versions enabled a commit-msg hook from .githooks/ (now removed).
if [ "$(git -C "$repo" config core.hooksPath || true)" = ".githooks" ]; then
    git -C "$repo" config --unset core.hooksPath
fi

# Earlier versions added a launcher that ran RustRover inside the toolbox; RustRover
# now runs on the desktop with dev/toolbox-bin as its toolchain (see README.md).
rm -f "$HOME/.local/share/applications/transient-rustrover.desktop"

echo
echo "Done. Next steps:"
echo "  toolbox enter $container"
echo "  cd $repo && just run"
echo
echo "RustRover: open the project normally, then in Settings → Rust:"
echo "  Toolchain location: $repo/dev/toolbox-bin"
echo "  Standard library:   $std_src"
