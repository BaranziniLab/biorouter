#!/usr/bin/env bash
# Verification script for docs consolidation.
# All checks must pass (exit 0) when migration is complete.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
DOCS="$REPO/docs"
FAIL=0

check() {
  local desc="$1"; local cmd="$2"; local expect_empty="${3:-true}"
  echo -n "  CHECK: $desc ... "
  local result
  result=$(eval "$cmd" 2>/dev/null || true)
  if [ "$expect_empty" = "true" ] && [ -z "$result" ]; then
    echo "PASS"
  elif [ "$expect_empty" = "false" ] && [ -n "$result" ]; then
    echo "PASS"
  else
    echo "FAIL"
    [ -n "$result" ] && echo "    -> $result" | head -5
    FAIL=1
  fi
}

echo "=== Biorouter Docs Verification ==="

check "no .html files in docs/" \
  "find '$DOCS' -name '*.html' 2>/dev/null"

check "no .mp4/.mp3 files in docs/" \
  "find '$DOCS' \( -name '*.mp4' -o -name '*.mp3' \) 2>/dev/null"

check "no recipe/recipes references in markdown (outside superpowers/)" \
  "grep -ril '\brecipe\b\|\brecipes\b' '$DOCS' --include='*.md' 2>/dev/null \
   | grep -v 'superpowers/' || true"

check "docs/docs/ directory does not exist" \
  "[ -d '$DOCS/docs' ] && echo 'exists' || echo ''" \
  "true"

check "documentation/ directory does not exist" \
  "[ -d '$REPO/documentation' ] && echo 'exists' || echo ''" \
  "true"

check "all files in docs/ (outside superpowers/) are .md" \
  "find '$DOCS' -not -path '*/superpowers/*' -type f ! -name '*.md' 2>/dev/null \
   | grep -v '/\.' || true"

echo ""
if [ "$FAIL" -eq 0 ]; then
  echo "ALL CHECKS PASSED"
  exit 0
else
  echo "SOME CHECKS FAILED"
  exit 1
fi
