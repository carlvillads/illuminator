#!/bin/sh
# Remember to rerun after changes to the icon
set -e
cd "$(dirname "$0")"
ictool="$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool"
mkdir -p icons
for size in 32 48 64 128 256 512; do
    "$ictool" ../illuminator.icon --export-image --output-file "icons/${size}x${size}.png" --platform macOS --rendition Default --width $size --height $size --scale 1 > /dev/null
done
