#!/bin/sh
# Remember to rerun after changes to the icon
set -e
cd "$(dirname "$0")"
ictool="$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool"
pngs="$(mktemp -d)"
for size in 16 24 32 48 64 256; do
    "$ictool" ../illuminator.icon --export-image --output-file "$pngs/$size.png" --platform macOS --rendition Default --width $size --height $size --scale 1 > /dev/null
done
magick "$pngs/16.png" "$pngs/24.png" "$pngs/32.png" "$pngs/48.png" "$pngs/64.png" "$pngs/256.png" illuminator.ico
rm -r "$pngs"
