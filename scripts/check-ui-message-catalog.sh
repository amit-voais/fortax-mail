#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
raw_keys=$(mktemp)
source_keys=$(mktemp)
catalog_keys=$(mktemp)
missing_keys=$(mktemp)
trap 'rm -f -- "$raw_keys" "$source_keys" "$catalog_keys" "$missing_keys"' EXIT

find "$repo_root/src" -type f -name '*.rs' -print0 \
  | xargs -0 perl -0777 -ne '
      while (/UiMessage::(?:plain|detail|arguments|three_arguments)\(\s*"((?:\\.|[^"\\])*)"/g) {
          print "$1\n";
      }
    ' >"$raw_keys"

# grep, not rg: ripgrep is not on a stock macOS, and this repo builds there.
constructor_count=$(
  grep -rhoE \
    'UiMessage::(plain|detail|arguments|three_arguments)\(' \
    "$repo_root/src" --include='*.rs' \
    | wc -l
)
literal_count=$(wc -l <"$raw_keys")
if [[ $constructor_count -ne $literal_count ]]; then
  echo "Every UiMessage constructor must receive a string-literal catalog key." >&2
  exit 1
fi

sort -u "$raw_keys" >"$source_keys"
perl -ne '
    while (/if \(key == "((?:\\.|[^"\\])*)"\)/g) {
        print "$1\n";
    }
  ' "$repo_root/ui/app.slint" \
  | sort -u >"$catalog_keys"

comm -23 "$source_keys" "$catalog_keys" >"$missing_keys"
if [[ -s $missing_keys ]]; then
  echo "UiMessage keys missing from the Slint translation dispatcher:" >&2
  sed 's/^/  /' "$missing_keys" >&2
  exit 1
fi
