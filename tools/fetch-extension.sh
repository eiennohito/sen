#!/bin/sh
# Download the newest LINE extension offered by Chrome's official update service.
# Only extracts WASM + provenance; it never reads a browser profile or logs in.
set -eu
cd "$(dirname "$0")/.."
archive=$(mktemp /tmp/line-extension.XXXXXX.crx)
trap 'rm -f "$archive"' EXIT HUP INT TERM
curl --fail --silent --show-error --location \
  'https://clients2.google.com/service/update2/crx?response=redirect&prodversion=150.0.0.0&acceptformat=crx3&x=id%3Dophjlpahpchlmihnnnihgmmeilfjmjjc%26uc' \
  --output "$archive"
python3 tools/extract_extension.py "$archive"
