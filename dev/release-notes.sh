#!/usr/bin/env bash
# Prints the notes for one version from CHANGELOG.md: everything under its
# "## [VERSION]" heading, up to the next "## [" heading. Fails if the version
# has no section, so a release cannot go out without its notes.
#
#   dev/release-notes.sh 0.1.0
set -euo pipefail

version="${1:?usage: dev/release-notes.sh VERSION}"
changelog="$(dirname "$0")/../CHANGELOG.md"

notes="$(awk -v heading="## [$version]" '
    index($0, heading) == 1 { found = 1; next }
    found && /^## \[/ { exit }
    found { print }
' "$changelog")"

if [ -z "${notes//[[:space:]]/}" ]; then
    echo "error: CHANGELOG.md has no notes under \"## [$version]\"" >&2
    exit 1
fi
printf '%s\n' "$notes"
