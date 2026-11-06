"""Types keys and clicks into the headless compositor (dev/headless.sh), for
testing keyboard and mouse behaviour that scripted actions cannot reach, such as
typing in a text field or double-clicking a clip.

    python3 -I dev/send-keys.py t e s t Return Control_L+s
    python3 -I dev/send-keys.py move:400,120 double-click
    python3 -I dev/send-keys.py move:60,150 press glide:60,260 release

Each argument is one key press, by its GDK key name (t, space, Return, Escape,
Delete, …); "+" presses keys together (Control_L+s is Ctrl+S). Keys go to
whatever has keyboard focus. Pointer steps:

    move:X,Y      moves the pointer to X,Y on the screen (the headless monitor is
                  1280x800 and the app's window fills it)
    click         clicks the left button
    double-click  clicks it twice, quickly
    press         presses the left button and holds it (for a drag)
    release       lets it go again
    glide:X,Y     moves the pointer to X,Y in small steps from where the last
                  move or glide left it, as a hand would (GTK starts a drag
                  only after the pointer moves a little with the button held)
    scroll:STEPS  turns the mouse wheel STEPS clicks where the pointer is
                  (positive scrolls down, negative up)
    wait:SECONDS  does nothing for a while, e.g. to hold a drag for a screenshot
                  (the session ends after the last step, which lets go of the
                  button)

It uses mutter's RemoteDesktop D-Bus API over one connection, because mutter ends
a session when its client disconnects. The first click of a run can get lost, so
start with a click on empty space.

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
# Pause between steps, so the app handles each one in turn.
PAUSE_SECONDS = 0.3
# The left mouse button, as Linux input event code BTN_LEFT.
BUTTON_LEFT = 0x110
# Far enough to reach the top left corner from anywhere on the screen.
TO_CORNER = -100_000.0
# A glide moves this many pixels per step at most, with a short pause after each.
GLIDE_STEP = 8.0
GLIDE_PAUSE_SECONDS = 0.02


def keysym(name: str) -> int:
    value = Gdk.keyval_from_name(name)
    if value in (0, Gdk.KEY_VoidSymbol):
        sys.exit(f"send-keys: unknown key name {name!r}")
    return value


def parse(arg: str):
    """One step: ("keys", [keysyms]), ("move"/"glide", (x, y)), ("click", count),
    ("button", pressed), ("scroll", steps) or ("wait", seconds)."""
    for kind in ("move", "glide"):
        if arg.startswith(kind + ":"):
            try:
                x, y = (float(part) for part in arg.removeprefix(kind + ":").split(","))
            except ValueError:
                sys.exit(f"send-keys: expected {kind}:X,Y, got {arg!r}")
            return (kind, (x, y))
    if arg in ("click", "double-click"):
        return ("click", 2 if arg == "double-click" else 1)
    if arg in ("press", "release"):
        return ("button", arg == "press")
    if arg.startswith("scroll:"):
        try:
            return ("scroll", int(arg.removeprefix("scroll:")))
        except ValueError:
            sys.exit(f"send-keys: expected scroll:STEPS, got {arg!r}")
    if arg.startswith("wait:"):
        try:
            return ("wait", float(arg.removeprefix("wait:")))
        except ValueError:
            sys.exit(f"send-keys: expected wait:SECONDS, got {arg!r}")
    return ("keys", [keysym(name) for name in arg.split("+")])


def main() -> None:
    steps = [parse(arg) for arg in sys.argv[1:]]
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)

    def call(path: str, interface: str, method: str, args: GLib.Variant | None = None):
        return bus.call_sync(REMOTE_DESKTOP, path, interface, method, args, None, 0, -1, None)

    def key(value: int, pressed: bool) -> None:
        call(session, SESSION, "NotifyKeyboardKeysym", GLib.Variant("(ub)", (value, pressed)))

    def move_by(dx: float, dy: float) -> None:
        call(session, SESSION, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (dx, dy)))

    def button(pressed: bool) -> None:
        call(session, SESSION, "NotifyPointerButton", GLib.Variant("(ib)", (BUTTON_LEFT, pressed)))

    def scroll(steps: int) -> None:
        # Mouse wheel clicks on the vertical axis (0); positive scrolls down.
        call(session, SESSION, "NotifyPointerAxisDiscrete", GLib.Variant("(ui)", (0, steps)))

    session = call("/org/gnome/Mutter/RemoteDesktop", REMOTE_DESKTOP, "CreateSession").unpack()[0]
    call(session, SESSION, "Start")
    # Where the pointer is, as far as this script knows (the corner at first).
    x, y = 0.0, 0.0
    # The first key of a new virtual keyboard gets lost: warm it up with Shift.
    for step in [("keys", [Gdk.KEY_Shift_L]), *steps]:
        kind, value = step
        if kind == "keys":
            for keyval in value:
                key(keyval, True)
            for keyval in reversed(value):
                key(keyval, False)
        elif kind == "move":
            # Relative moves need no screen-cast stream: go to the corner, then on.
            move_by(TO_CORNER, TO_CORNER)
            move_by(*value)
            x, y = value
        elif kind == "glide":
            dx, dy = value[0] - x, value[1] - y
            count = max(1, int(max(abs(dx), abs(dy)) / GLIDE_STEP))
            for _ in range(count):
                move_by(dx / count, dy / count)
                time.sleep(GLIDE_PAUSE_SECONDS)
            x, y = value
        elif kind == "button":
            button(value)
        elif kind == "scroll":
            scroll(value)
        elif kind == "wait":
            time.sleep(value)
        else:
            for _ in range(value):
                button(True)
                button(False)
                time.sleep(0.05)
        time.sleep(PAUSE_SECONDS)
    call(session, SESSION, "Stop")
    print(f"send-keys: sent {len(steps)} steps", file=sys.stderr)


if __name__ == "__main__":
    main()
