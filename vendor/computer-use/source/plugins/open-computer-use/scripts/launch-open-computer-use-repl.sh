#!/usr/bin/env bash

set -euo pipefail

plugin_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_root="$(cd "${plugin_root}/../.." && pwd)"
adapter_candidates=(
  "${plugin_root}/scripts/open-computer-use-repl.mjs"
  "${repo_root}/scripts/node-repl/open-computer-use-repl.mjs"
)
native_candidates=(
  "${plugin_root}/Open Computer Use.app/Contents/MacOS/OpenComputerUse"
  "${plugin_root}/Open Computer Use (Dev).app/Contents/MacOS/OpenComputerUse"
  "${plugin_root}/OpenComputerUse.app/Contents/MacOS/OpenComputerUse"
  "${plugin_root}/open-computer-use"
  "${plugin_root}/open-computer-use.exe"
  # install-codex-plugin.sh stores the plugin source and runtime payload as
  # siblings beneath the versioned cache directory.
  "${repo_root}/Open Computer Use.app/Contents/MacOS/OpenComputerUse"
  "${repo_root}/Open Computer Use (Dev).app/Contents/MacOS/OpenComputerUse"
  "${repo_root}/OpenComputerUse.app/Contents/MacOS/OpenComputerUse"
  "${repo_root}/open-computer-use"
  "${repo_root}/open-computer-use.exe"
  "${repo_root}/dist/Open Computer Use (Dev).app/Contents/MacOS/OpenComputerUse"
  "${repo_root}/dist/Open Computer Use.app/Contents/MacOS/OpenComputerUse"
  "${repo_root}/dist/OpenComputerUse.app/Contents/MacOS/OpenComputerUse"
  "${repo_root}/dist/linux/arm64/open-computer-use"
  "${repo_root}/dist/linux/amd64/open-computer-use"
  "${repo_root}/dist/windows/arm64/open-computer-use.exe"
  "${repo_root}/dist/windows/amd64/open-computer-use.exe"
)

adapter=""
for candidate in "${adapter_candidates[@]}"; do
  if [[ -f "${candidate}" ]]; then adapter="${candidate}"; break; fi
done

if [[ -z "${adapter}" ]]; then
  echo "open-computer-use could not find its Node REPL adapter." >&2
  exit 1
fi

for native in "${native_candidates[@]}"; do
  if [[ -x "${native}" ]]; then
    exec node "${adapter}" -- "${native}" mcp
  fi
done

if command -v open-computer-use >/dev/null 2>&1; then
  exec node "${adapter}" -- "$(command -v open-computer-use)" mcp
fi

echo "open-computer-use could not find a runnable native runtime for the JS REPL adapter." >&2
exit 1
