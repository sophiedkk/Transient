#!/usr/bin/env bash
# One-time setup on the HOST (not inside the toolbox). Needs no sudo.
#
#   dev/setup.sh            create the transient-dev toolbox
#   dev/setup.sh --rebuild  rebuild the image and recreate the toolbox
#                           (after changing dev/Containerfile or dev/packages.txt)
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
    podman build -t "$image" -f "$repo/dev/Containerfile" "$repo/dev"
    podman rm -f "$container" >/dev/null 2>&1 || true
fi

if ! podman container exists "$container"; then
    echo "==> Creating the $container toolbox"
    toolbox create --image "$image" "$container"
fi

echo "==> Installing the Rust toolchain from rust-toolchain.toml"
(cd "$repo" && rustup toolchain install)

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
echo "RustRover: open the project normally, then Settings → Rust → Toolchain location:"
echo "  $repo/dev/toolbox-bin"
