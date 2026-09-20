#!/usr/bin/env bash

set -euo pipefail

if [[ $# -lt 2 ]]; then
  cat <<'EOF' >&2
Usage:
  ./scripts/run-isolated-codex-exec.sh <computer-use|open-computer-use|all> [codex exec args...]

Examples:
  ./scripts/run-isolated-codex-exec.sh computer-use --skip-git-repo-check -C /tmp '使用computer-use列出正在运行的前三个应用'
  ./scripts/run-isolated-codex-exec.sh open-computer-use --skip-git-repo-check -C /tmp --json '使用open-computer-use列出正在运行的前三个应用'
  ./scripts/run-isolated-codex-exec.sh all --skip-git-repo-check -C /tmp 'reply with one word: ok'
EOF
  exit 1
fi

mode="$1"
shift

declare -a overrides=()

case "${mode}" in
  computer-use)
    overrides=(-c 'plugins."open-computer-use@open-computer-use-local".enabled=false')
    ;;
  open-computer-use)
    overrides=(-c 'plugins."computer-use@openai-bundled".enabled=false')
    ;;
  all)
    ;;
  *)
    echo "Unsupported mode: ${mode}" >&2
    echo "Expected one of: computer-use, open-computer-use, all" >&2
    exit 1
    ;;
esac

exec codex exec "${overrides[@]}" "$@"
