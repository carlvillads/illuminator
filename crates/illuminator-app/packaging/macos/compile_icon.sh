#!/bin/sh
# Icon composer icon into Assets.car and illuminator.icns
# Remember to rerun after changes to the icon
set -e
cd "$(dirname "$0")"
plist="$(mktemp)"
xcrun actool ../illuminator.icon --compile . --platform macosx --minimum-deployment-target 11.0 --app-icon illuminator --output-partial-info-plist "$plist" > /dev/null
rm "$plist"
