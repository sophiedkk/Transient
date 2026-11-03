#!/usr/bin/env bash
# One-time setup on the HOST (not inside the toolbox). Needs no sudo.
#
#   dev/setup.sh            create the transient-dev toolbox and the RustRover launcher
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

echo "==> Enabling the git hooks in .githooks/"
git -C "$repo" config core.hooksPath .githooks

echo "==> Adding the 'RustRover (Transient)' launcher"
launcher="$HOME/.local/share/applications/transient-rustrover.desktop"
mkdir -p "$(dirname "$launcher")"
cat > "$launcher" <<EOF
[Desktop Entry]
Type=Application
Name=RustRover (Transient)
Comment=RustRover running inside the transient-dev toolbox
Exec=toolbox run -c $container $repo/dev/rustrover.sh $repo
Icon=rustrover
Terminal=false
Categories=Development;IDE;
EOF

echo
echo "Done. Next steps:"
echo "  toolbox enter $container"
echo "  cd $repo && just run"
