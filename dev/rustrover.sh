#!/usr/bin/env bash
# Starts RustRover inside the toolbox, so it uses the toolbox's GTK, GStreamer and
# build tools. Used by the "RustRover (Transient)" launcher that dev/setup.sh creates.
# Install RustRover with JetBrains Toolbox first.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

for candidate in \
    "$HOME/.local/share/JetBrains/Toolbox/scripts/rustrover" \
    "$HOME/.local/share/JetBrains/Toolbox/apps/rustrover/bin/rustrover" \
    "$HOME/.local/share/JetBrains/Toolbox/apps/rustrover/bin/rustrover.sh"; do
    if [ -x "$candidate" ]; then
        exec "$candidate" "$@"
    fi
done

echo "RustRover not found. Install it with JetBrains Toolbox, then try again." >&2
exit 1
