#!/usr/bin/env bash
# Retakes the screenshots in data/screenshots/ (README, MetaInfo, GNOME Software,
# Flathub), each in the light style and as a "-dark" copy:
#
#   dev/screenshots.sh      (inside the toolbox; needs test-audio/)
#
# They show dev/demo-project.transient, a small arrangement of the test audio.
# It is an old format-1 project (plain JSON that points at the audio), so it
# stays readable here; the app saves it as real projects into a made-up home
# folder, which is also what the start screen lists as recent projects ("~/Music").
# Everything runs in the invisible compositor of dev/headless.sh.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"
out="data/screenshots"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
home="$work/home"
mkdir -p "$home/Music" "$work/data"

if [ ! -f test-audio/piano.flac ]; then
    echo "test-audio/ is missing: run dev/fetch-test-audio.sh first." >&2
    exit 1
fi
just schemas
cargo build -q -p transient

# Runs the app headless with the made-up home folder and a data folder that
# keeps its recent projects from one run to the next.
run() {
    GSETTINGS_SCHEMA_DIR=target/gschemas HEADLESS_DATA_HOME="$work/data" \
        dev/headless.sh env HOME="$home" target/debug/transient --silent "$@"
}

# Two recent projects in ~/Music (the start screen at its default size has room
# for two); Podcast Intro, saved last, is the one shown everywhere. Every project
# opened lands in the recent projects, so the demo is opened from a copy there,
# with its audio paths made absolute, rather than from dev/.
rehearsal="$home/Music/Band Rehearsal.transient"
project="$home/Music/Podcast Intro.transient"
sed "s|\"\.\./test-audio/|\"$repo/test-audio/|" dev/demo-project.transient >"$rehearsal"
run --quit-after=1 --save-to="$rehearsal" "$rehearsal"
# The start screen sorts by time in whole seconds.
sleep 1
run --quit-after=1 --save-to="$project" "$rehearsal"

# Takes one screenshot in both styles: NAME, then the app's options.
shoot() {
    local name="$1"
    shift
    run "$@" --screenshot="$out/$name.png"
    ADW_DEBUG_COLOR_SCHEME=prefer-dark run "$@" --screenshot="$out/$name-dark.png"
}

# The main window: the playhead at 0:12 and the voice clip under it selected
# (on the top track; --select-clips-at would also select the piano below).
shoot main-window --seek=12 --action=select-clip-at-playhead --quit-after=3 "$project"
# The clip pane, for the piano (the only clip playing at 0:01).
shoot clip-pane --seek=12 --select-clips-at=1 --action=toggle-clip-pane \
    --quit-after=3 "$project"
# The Track tab, for the voice track (low cut, noise gate and compressor on).
shoot track-effects --seek=12 --action=select-clip-at-playhead --action=track-effects \
    --quit-after=3 "$project"
shoot export --seek=12 --action=export --quit-after=3 "$project"
shoot start-screen --quit-after=3

echo "Screenshots written to $out/"
