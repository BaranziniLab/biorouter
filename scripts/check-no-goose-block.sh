#!/usr/bin/env bash
# R-31: fail when the tracked tree names the project Biorouter no longer mentions, or
# that project's publisher, anywhere except NOTICE.
#
# The owner ruled (REQUIREMENTS.md, Message 16) that those names are purged from every
# tracked file and every tracked file NAME, with one exception: NOTICE stays verbatim
# because the license text requires it. This guard is the tripwire. It reads the
# TRACKED tree through git, so scratch files and ignored build output never trip it, and
# it runs in `just check-everything` and in scripts/verify-docs.sh.
#
# Allowed, and nothing else:
#   NOTICE                            the license attribution, verbatim
#   scripts/check-no-goose-block.sh   this file, which has to spell the words
#   the token "mongoose"              an unrelated database library, removed before matching
#   the token "check-no-goose-block"  this script's own name, so a recipe that runs it passes
#
# The ordinary word "block" (code block, blocking, a block's height) is never the target
# and is never matched on its own; only the publisher's name, its possessive, its logo
# files and its repository URLs are.
#
# Binary files are skipped (git grep -I). Vendored code under vendor/ is not exempt: if a
# vendored file ever trips this, report it rather than widening the allowlist.
set -euo pipefail
cd "$(dirname "$0")/.."

self='scripts/check-no-goose-block.sh'

# Matched ignoring case. Written without backslashes so awk and grep read it the same way.
ci='goose|geese|gosling|honk|gaggle|block[-_]?lockup|block[.]github[.]io|github[.]com/block([^[:alnum:]_-]|$)|githubusercontent[.]com/block/|repos/block/|://block/'

# Matched exactly as written: "Block" the company, as opposed to the word "block".
cs="Block, Inc|Block Inc([^[:lower:]]|\$)|(^|[^[:alnum:]_])Block's|Square, Inc"

# Reads "path:line:text" on stdin and prints only the lines whose TEXT still matches
# after the allowed tokens are removed. $1 is the regex; $2 is "i" to fold case.
surviving() {
  awk -v re="$1" -v fold="${2:-}" '
    {
      s = $0
      sub(/^[^:]*:[0-9]+:/, "", s)
      if (fold == "i") s = tolower(s)
      gsub(/mongoose/, "", s)
      gsub(/check-no-goose-block/, "", s)
      if (s ~ re) print substr($0, 1, 200)
    }'
}

hits=$(
  {
    git grep -n -I -i -E "$ci" -- . ":(exclude)NOTICE" ":(exclude)$self" 2>/dev/null | surviving "$ci" i || true
    git grep -n -I -E "$cs" -- . ":(exclude)NOTICE" ":(exclude)$self" 2>/dev/null | surviving "$cs" || true
  } | sort -u
)

# File and directory names are part of the tree too.
names=$(
  git ls-files | awk -v re="$ci" -v self="$self" '
    $0 == "NOTICE" || $0 == self { next }
    {
      s = tolower($0)
      gsub(/mongoose/, "", s)
      gsub(/check-no-goose-block/, "", s)
      if (s ~ re) print $0
    }'
)

fail=0
if [ -n "$hits" ]; then
  printf '::error::tracked text names a removed upstream or its publisher (only NOTICE may):\n' >&2
  printf '%s\n' "$hits" >&2
  fail=1
fi
if [ -n "$names" ]; then
  printf '::error::tracked file names carry a removed upstream or its publisher (only NOTICE may):\n' >&2
  printf '%s\n' "$names" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  printf '\nRemove the mention. Do not widen the allowlist in %s.\n' "$self" >&2
  exit 1
fi
echo "check-no-goose-block: the tracked tree is clean (NOTICE excepted)."
