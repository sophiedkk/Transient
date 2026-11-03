#!/usr/bin/env bash
# Downloads the test audio listed in dev/test-audio.tsv into test-audio/ and
# verifies every file against its SHA-256 checksum. Files that are already present
# and correct are skipped. Also writes test-audio/CREDITS.md (required by the
# CC BY / CC BY-SA licences when the files are shared).
#
# The audio itself is not committed to git; only this script and the manifest are.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$repo/dev/test-audio.tsv"
dest="$repo/test-audio"
user_agent="TransientTestAudio/0.1 (https://github.com/sophiedkk/Transient)"

mkdir -p "$dest"
credits="$dest/CREDITS.md"
printf '# Test audio credits\n\nAll files come from Wikimedia Commons.\n\n' > "$credits"
printf '| File | Licence | Author | Source |\n|---|---|---|---|\n' >> "$credits"

failed=0
while IFS=$'\t' read -r name sha256 licence author page url; do
    case "$name" in '#'* | '') continue ;; esac
    file="$dest/$name"

    if [ -f "$file" ] && echo "$sha256  $file" | sha256sum --check --quiet --status; then
        echo "ok        $name"
    else
        echo "download  $name"
        curl --silent --show-error --fail --location --user-agent "$user_agent" \
            --output "$file.part" "$url"
        if echo "$sha256  $file.part" | sha256sum --check --quiet --status; then
            mv "$file.part" "$file"
        else
            echo "CHECKSUM MISMATCH for $name; the file was not kept" >&2
            rm -f "$file.part"
            failed=1
        fi
    fi
    printf '| %s | %s | %s | %s |\n' "$name" "$licence" "$author" "$page" >> "$credits"
done < "$manifest"

exit "$failed"
