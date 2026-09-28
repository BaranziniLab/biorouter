#!/usr/bin/env bash

set -euo pipefail
cd "$(dirname "$0")/.."

canonical_icon="ui/desktop/src/images/icon.svg"
canonical_glyph="ui/desktop/src/images/glyph.svg"

# The shipped app-icon SVG must be byte-identical everywhere it is copied
# (prepare.sh fans it out to the light variant and the landing surfaces).
for copy in \
  ui/desktop/src/images/icon-light.svg \
  landing/icon.svg \
  landing/video/icon.svg \
  landing/video/assets/icon.svg \
  landing/video/reel/icon.svg; do
  cmp -s "$canonical_icon" "$copy" || {
    echo "Brand asset drift: $copy differs from $canonical_icon" >&2
    exit 1
  }
done

# The CLI logos are byte-for-byte copies of the landing raster.
for copy in \
  crates/biorouter-cli/static/img/logo_dark.png \
  crates/biorouter-cli/static/img/logo_light.png; do
  cmp -s landing/icon.png "$copy" || {
    echo "Brand asset drift: $copy differs from landing/icon.png" >&2
    exit 1
  }
done

# The canonical mark is the "BR" monogram (D-38), set in Inter (D-41): a navy B,
# a coral R, and a split underline. This superseded the abstract circle glyph
# (D-40), so the old 'M 125 220' path assertion is intentionally gone — the mark
# is now live text, not a hand-drawn path.
for svg in "$canonical_glyph" "$canonical_icon"; do
  { grep -q '>B</text>' "$svg" && grep -q '>R</text>' "$svg"; } || {
    echo "Brand asset drift: $svg is not the BR mark" >&2
    exit 1
  }
done
grep -q "font-family=\"Inter" "$canonical_icon" || {
  echo "Brand asset drift: $canonical_icon is not set in Inter (D-41)" >&2
  exit 1
}

grep -q '"productName": "Biorouter"' ui/desktop/package.json
grep -q "glyph.svg" ui/desktop/src/components/icons/BioRouter.tsx

# The desktop's main process writes the native dialogs, and they spell the brand "Biorouter"
# (DOCS-6): an approval-secret prompt titled "BioRouter" reads like another app asking for a
# secret, and the manual quotes these dialogs word for word. A comment or an identifier may keep
# the old spelling; a quoted string may not.
native_strings=(ui/desktop/src/main.ts ui/desktop/src/nativeSecretPrompt.ts)
misspelled="$(grep -nE "['\"\`][^'\"\`]*BioRouter" "${native_strings[@]}" |
  grep -vE '^[^:]+:[0-9]+:[[:space:]]*(//|\*|/\*)' || true)"
if [ -n "$misspelled" ]; then
  echo "Brand drift: a native dialog string spells \"BioRouter\"; write \"Biorouter\":" >&2
  echo "$misspelled" >&2
  exit 1
fi

echo "Biorouter name and canonical logo assets are consistent"
