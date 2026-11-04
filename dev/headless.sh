#!/usr/bin/env bash
# Runs a command inside an invisible GNOME compositor (mutter --headless), so the app
# can be started, screenshotted and tested without a window appearing on screen.
# It gets its own D-Bus session, so it cannot interfere with your desktop,
# in-memory settings, so it does not change the saved window size, and its own
# data folder, so it does not add to your recent files.
#
#   dev/headless.sh cargo run -p transient -- --screenshot=target/screenshot.png
#
# With HEADLESS_KEYS set, the keys are typed into the app once it has started
# (after HEADLESS_KEYS_DELAY seconds, 4 by default), through dev/send-keys.py:
#
#   HEADLESS_KEYS="t e s t Return" dev/headless.sh target/debug/transient …
#
# Run inside the toolbox. Exits with 0 if the command succeeded, otherwise 1.
set -euo pipefail

if [ $# -eq 0 ]; then
    echo "usage: dev/headless.sh COMMAND [ARGUMENTS…]" >&2
    exit 2
fi

if [ -n "${HEADLESS_KEYS:-}" ]; then
    # Run the command in the background and type the keys into it.
    send_keys="$(dirname "$0")/send-keys.py"
    set -- bash -c 'set -u
        "$@" &
        app=$!
        sleep "${HEADLESS_KEYS_DELAY:-4}"
        # The keys are separate words on purpose.
        # shellcheck disable=SC2086
        python3 -I "$0" $HEADLESS_KEYS
        wait "$app"' "$send_keys" "$@"
fi

display="transient-headless-$$"
# A throwaway data folder, so test runs do not add to your recent files.
data_home="$(mktemp -d)"
trap 'rm -rf "$data_home"' EXIT
dbus-run-session -- \
    mutter --headless --no-x11 --virtual-monitor 1280x800 --wayland-display="$display" \
    -- env GTK_A11Y=none GDK_DEBUG=no-portals ADW_DISABLE_PORTAL=1 GSETTINGS_BACKEND=memory \
    XDG_DATA_HOME="$data_home" "$@" \
    2> >(grep --line-buffered -v -E '^(libmutter-Message|\*\* Message|Gdk-Message: .*Broken pipe)' >&2)
