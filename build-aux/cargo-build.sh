#!/bin/sh
# Called by meson.build: builds the `transient` binary with Cargo and copies it
# to where Meson expects it.
# Usage: cargo-build.sh <cargo> <source-root> <debug|release> <output-file>
set -eu

cargo=$1
source_root=$2
profile=$3
output=$4

if [ "$profile" = release ]; then
    "$cargo" build --release --manifest-path "$source_root/Cargo.toml" -p transient
else
    "$cargo" build --manifest-path "$source_root/Cargo.toml" -p transient
fi

cp "$source_root/target/$profile/transient" "$output"
