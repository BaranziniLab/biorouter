#!/usr/bin/env bash
# Wait for every check on a pull request's head to finish, then exit with the verdict.
# Run it with the Bash tool's run_in_background (no nohup, no &, no pipe after it):
# the harness tells you when it exits, so you never poll from your own turns.
#
# usage: wait-for-pr-checks.sh PR [HEAD_SHA] [MAX_MINUTES]
#   PR           pull request number
#   HEAD_SHA     the full commit you expect CI to test (default: the PR's head now)
#   MAX_MINUTES  give up after this long (default 120)
# env: REPO (default BaranziniLab/biorouter), INTERVAL seconds between reads (default 120)
#
# Exit codes, each with one summary line on stdout:
#   0 GREEN      every check passed or was skipped, and all five required names passed
#   1 RED        a check failed or was cancelled (cancelled is never green)
#   3 MOVED      the PR's head changed; your SHA is no longer what CI tests
#   8 TIMEOUT    still pending at the deadline
#   2            bad usage, or the PR could not be read
set -u

pr="${1:-}"; want="${2:-}"; max="${3:-120}"
[ -n "$pr" ] || { sed -n '2,17p' "$0"; exit 2; }
repo="${REPO:-BaranziniLab/biorouter}"
interval="${INTERVAL:-120}"
required=("test (ubuntu-latest)" "test (macos-latest)" "test (windows-latest)" "Unit tests (vitest)" "no-ai-coauthor")

head_now() { gh pr view "$pr" --repo "$repo" --json headRefOid --jq .headRefOid 2>/dev/null; }
pin=$(head_now)
[ -n "$pin" ] || { echo "cannot read PR $pr in $repo" >&2; exit 2; }
if [ -n "$want" ] && [ "$want" != "$pin" ]; then
  echo "MOVED PR $pr head is $pin, not $want"; exit 3
fi
echo "watching PR $pr at $pin (deadline ${max} min)"

deadline=$(( $(date +%s) + max * 60 )); missing=""
while [ "$(date +%s)" -lt "$deadline" ]; do
  now=$(head_now)
  if [ -n "$now" ] && [ "$now" != "$pin" ]; then
    echo "MOVED PR $pr head moved from $pin to $now"; exit 3
  fi
  checks=$(gh pr checks "$pr" --repo "$repo" --json name,bucket 2>/dev/null)
  if [ -n "$checks" ]; then
    bad=$(printf '%s' "$checks" | jq -r '.[] | select(.bucket=="fail" or .bucket=="cancel") | "\(.name)=\(.bucket)"')
    if [ -n "$bad" ]; then
      echo "RED PR $pr at $pin: $(printf '%s' "$bad" | paste -sd ',' -)"; exit 1
    fi
    pending=$(printf '%s' "$checks" | jq '[.[] | select(.bucket=="pending")] | length')
    missing=""
    for name in "${required[@]}"; do
      ok=$(printf '%s' "$checks" | jq --arg n "$name" '[.[] | select(.name==$n and .bucket=="pass")] | length')
      [ "$ok" -gt 0 ] || missing="$missing [$name]"
    done
    if [ "$pending" -eq 0 ] && [ -z "$missing" ]; then
      echo "GREEN PR $pr at $pin: $(printf '%s' "$checks" | jq -r 'group_by(.bucket) | map("\(.[0].bucket)=\(length)") | join(" ")')"
      exit 0
    fi
  fi
  sleep "$interval"
done
echo "TIMEOUT PR $pr at $pin: still pending after ${max} min${missing:+; required not passed:$missing}"
exit 8
