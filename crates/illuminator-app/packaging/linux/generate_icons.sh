#!/usr/bin/env bash
# Remember to rerun after changes to the icon
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ICON_DIR="${SCRIPT_DIR}/icons"
MACOS_ICNS="${SCRIPT_DIR}/../macos/illuminator.icns"

mkdir -p "${ICON_DIR}"

SIZES=(32 48 64 128 256 512)
SRGB_PROFILE="/System/Library/ColorSync/Profiles/sRGB Profile.icc"
for size in "${SIZES[@]}"; do
    out="${ICON_DIR}/${size}x${size}.png"
    sips -s format png --matchTo "${SRGB_PROFILE}" -z "${size}" "${size}" "${MACOS_ICNS}" --out "${out}" >/dev/null
done

echo "Done."
