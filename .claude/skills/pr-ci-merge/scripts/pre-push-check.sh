#!/usr/bin/env bash
# Check the commits a push would add, before pushing them.
#
# usage: pre-push-check.sh [BASE] [--body FILE]
#   BASE       what the branch will be compared against (default: origin/main)
#   --body F   also scan a pull request body you are about to post
#
# Blocks (exit 1) on:
#   - an AI Co-Authored-By trailer (the required no-ai-coauthor check fails on it)
#   - an added or changed credential file (.env, secrets.yaml, live-test.env, keys)
#   - an added line shaped like a credential (cloud keys, tokens, private keys)
#   - a bare @handle in the PR body (GitHub notifies that account on every push)
# Reports, without blocking: merge commits in the range, and commits not yet pushed.
# Never prints a matched credential line, only the file and the pattern name.
# Exit codes: 0 clean, 1 a blocking hit, 2 bad usage.
set -u

base=origin/main
body=""
while [ $# -gt 0 ]; do
  case "$1" in
    --body) body="${2:-}"; [ -n "$body" ] || { echo "--body needs a file" >&2; exit 2; }; shift 2 ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) base="$1"; shift ;;
  esac
done

git rev-parse --verify --quiet "$base^{commit}" >/dev/null || { echo "unknown base: $base" >&2; exit 2; }
range="$base..HEAD"
fail=0
count=$(git rev-list --count "$range")
echo "commits in $range: $count"

# 1. AI co-author trailers: the same pattern .github/workflows/check-commits.yml uses.
trailers=$(git log "$range" --format='%x1e%h %s%n%B' | awk -v RS='\036' '
  NF && tolower($0) ~ /\nco-authored-by:[^\n]*(anthropic|claude|openai|chatgpt|gemini|copilot)/ {
    split($0, line, "\n"); print "BLOCK trailer: " line[1] }')
if [ -n "$trailers" ]; then
  printf '%s\n' "$trailers"
  fail=1
fi

# 2. Credential files. Three dots: only what the branch itself changed.
files=$(git diff --name-only --diff-filter=AM "$base...HEAD")
for f in $files; do
  case "$f" in
    *.example) continue ;;
  esac
  if printf '%s\n' "$f" | grep -qE '(^|/)(live-test\.env|secrets\.yaml|\.env)$|\.pem$|(^|/)id_(rsa|ed25519)$'; then
    echo "BLOCK file: $f"
    fail=1
  fi
done

# 3. Credential-shaped added lines. Printed as file + pattern name only.
added=$(git diff -U0 "$base...HEAD" | awk '
  /^\+\+\+ b\// { f = substr($0, 7); next }
  /^\+/ && !/^\+\+\+/ { print f "\t" substr($0, 2) }')
scan() { # $1 name, $2 extended regex, $3 "i" to ignore case
  local flags=-E
  [ "${3:-}" = i ] && flags=-iE
  printf '%s\n' "$added" | grep $flags -- "$2" | cut -f1 | sort -u | while read -r f; do
    [ -n "$f" ] && echo "BLOCK pattern $1 in $f (read the line; a test fixture may be fine)"
  done
}
hits=$(
  scan aws-key 'A(KIA|SIA)[0-9A-Z]{16}'
  scan github-token 'ghp_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}'
  scan slack-token 'xox[baprs]-[A-Za-z0-9-]{10,}'
  scan google-key 'AIza[0-9A-Za-z_-]{35}'
  scan sk-key '(^|[^A-Za-z0-9_-])sk-[A-Za-z0-9_-]{24,}'
  # A private key marker counts unless it is a one-line test fixture: its END marker on
  # the same line and under 200 characters of base64 between them.
  printf '%s\n' "$added" | awk -F'\t' '
    match($0, /-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----/) {
      rest = substr($0, RSTART + RLENGTH); end = index(rest, "-----END")
      body = (end > 0) ? substr(rest, 1, end - 1) : rest
      gsub(/\\n|[^A-Za-z0-9+\/=]/, "", body)
      if (end == 0 || length(body) >= 200) print $1 }' | sort -u | while read -r f; do
    echo "BLOCK pattern private-key in $f (read the line; a test fixture may be fine)"
  done
  scan env-secret '(VERSA|BASETEN|OPENAI|ANTHROPIC|HF)_[A-Z_]*(KEY|TOKEN|SECRET)[[:space:]]*[=:][[:space:]]*["'\'']?[A-Za-z0-9/+_-]{16,}' i
)
if [ -n "$hits" ]; then
  printf '%s\n' "$hits"
  fail=1
fi

# 4. PR body: bare @handles outside code spans and fences.
if [ -n "$body" ]; then
  [ -r "$body" ] || { echo "cannot read $body" >&2; exit 2; }
  mentions=$(awk '/^[[:space:]]*```/ { fence = !fence; next } !fence' "$body" \
    | sed 's/`[^`]*`//g' \
    | grep -oE '(^|[^A-Za-z0-9._%+/-])@[A-Za-z0-9][A-Za-z0-9-]{0,38}' \
    | sed 's/^[^@]*//' | sort -u)
  if [ -n "$mentions" ]; then
    echo "BLOCK mention in PR body: $(printf '%s ' $mentions)(put code in backticks or drop the @)"
    fail=1
  fi
fi

# 5. Information only.
merges=$(git rev-list --merges --count "$range")
[ "$merges" -gt 0 ] && echo "note: $merges merge commit(s) in the range; merge the PR with --merge, not --rebase"
branch=$(git branch --show-current)
if [ -n "$branch" ] && git rev-parse --verify --quiet "origin/$branch" >/dev/null; then
  echo "note: $(git rev-list --count "origin/$branch..HEAD") commit(s) not yet on origin/$branch"
elif [ -n "$branch" ]; then
  echo "note: origin/$branch does not exist yet; this push creates it"
fi

[ "$fail" -eq 0 ] && echo "OK: nothing blocking"
exit "$fail"
