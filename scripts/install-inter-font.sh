#!/usr/bin/env bash
set -euo pipefail

font_dir="\${XDG_DATA_HOME:-$HOME/.local/share}/fonts"
archive="$(mktemp)"
extract_dir="$(mktemp -d)"
trap 'rm -f "$archive"; rm -rf "$extract_dir"' EXIT

mkdir -p "$font_dir"
echo "Installing Inter 4.1 for Archia Browser..."
curl --fail --location --silent --show-error \
    "https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip" \
    --output "$archive"

font_entry="$(unzip -Z1 "$archive" | grep -E '(^|/)Inter\.ttc$' | head -n 1)"
if [[ -z "$font_entry" ]]; then
    echo "Inter.ttc was not found in the official archive." >&2
    exit 1
fi

unzip -p "$archive" "$font_entry" > "$font_dir/Inter.ttc"
license_entry="$(unzip -Z1 "$archive" | grep -E '(^|/)LICENSE\.txt$' | head -n 1 || true)"
if [[ -n "$license_entry" ]]; then
    unzip -p "$archive" "$license_entry" > "$font_dir/Inter-OFL-LICENSE.txt"
fi

if command -v fc-cache >/dev/null 2>&1; then
    fc-cache -f "$font_dir"
fi

echo "Inter installed in $font_dir"
