#!/usr/bin/env bash
# Fails if a Rust source file is longer than 800 lines (tests included). A file
# that grows past that holds more than one concern: split it by concern
# (see ROADMAP.md, milestone 14). Run by `just check` and CI.
#
#   dev/check-file-size.sh
set -euo pipefail

limit=800
cd "$(dirname "$0")/.."

too_long=0
while IFS= read -r file; do
    lines=$(wc -l < "$file")
    if [ "$lines" -gt "$limit" ]; then
        echo "error: $file has $lines lines (at most $limit); split it by concern" >&2
        too_long=1
    fi
done < <(git ls-files 'crates/*.rs')

exit "$too_long"
