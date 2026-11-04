"""Types keys into the headless compositor (dev/headless.sh), for testing keyboard
behaviour that scripted actions cannot reach, such as typing in a text field.

    python3 -I dev/send-keys.py t e s t Return Control_L+s

Each argument is one key press, by its GDK key name (t, space, Return, Escape,
Delete, …); "+" presses keys together (Control_L+s is Ctrl+S). The keys go to
whatever has keyboard focus. It uses mutter's RemoteDesktop D-Bus API over one
connection, because mutter ends a session when its client disconnects.

Normally started by dev/headless.sh through HEADLESS_KEYS; see CLAUDE.md.
"""

import sys
import time

import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gio", "2.0")
from gi.repository import Gdk, Gio, GLib  # noqa: E402

REMOTE_DESKTOP = "org.gnome.Mutter.RemoteDesktop"
SESSION = REMOTE_DESKTOP + ".Session"
# Pause between key presses, so the app handles each one in turn.
PAUSE_SECONDS = 0.3


def keysym(name: str) -> int:
    value = Gdk.keyval_from_name(name)
    if value in (0, Gdk.KEY_VoidSymbol):
        sys.exit(f"send-keys: unknown key name {name!r}")
    return value


def main() -> None:
    chords = [[keysym(name) for name in arg.split("+")] for arg in sys.argv[1:]]
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)

    def call(path: str, interface: str, method: str, args: GLib.Variant | None = None):
        return bus.call_sync(REMOTE_DESKTOP, path, interface, method, args, None, 0, -1, None)

    def key(value: int, pressed: bool) -> None:
        call(session, SESSION, "NotifyKeyboardKeysym", GLib.Variant("(ub)", (value, pressed)))

    session = call("/org/gnome/Mutter/RemoteDesktop", REMOTE_DESKTOP, "CreateSession").unpack()[0]
    call(session, SESSION, "Start")
    # The first key of a new virtual keyboard gets lost: warm it up with Shift.
    for chord in [[Gdk.KEY_Shift_L], *chords]:
        for value in chord:
            key(value, True)
        for value in reversed(chord):
            key(value, False)
        time.sleep(PAUSE_SECONDS)
    call(session, SESSION, "Stop")
    print(f"send-keys: typed {len(chords)} keys", file=sys.stderr)


if __name__ == "__main__":
    main()
