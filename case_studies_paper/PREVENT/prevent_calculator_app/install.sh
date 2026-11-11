#!/usr/bin/env bash
# Installs the PREVENT UCSF Risk Calculator (a Biorouter app) and the Biorouter skill
# that sends cohorts to it from a chat, then opens the calculator in your browser.
# Run from a terminal:  bash install.sh
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$HERE/app/prevent-ucsf-risk-calculator"
SKILL_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/biorouter/skills/prevent-calculator-handoff"

[ -f "$APP/run.sh" ] || { echo "app/prevent-ucsf-risk-calculator is missing: unzip the whole bundle first." >&2; exit 1; }
case "$APP" in
  *'|'* | *'&'* | *'\'*) echo "Move this folder to a path without |, & or \\ and run install.sh again." >&2; exit 1 ;;
esac

# 1. The skill, pointed at this copy of the calculator (keep the folder where it is)
mkdir -p "$SKILL_DIR"
if [ -f "$SKILL_DIR/SKILL.md" ]; then
  cp "$SKILL_DIR/SKILL.md" "$SKILL_DIR/SKILL.md.bak"
  echo "Your previous copy of the skill is kept as $SKILL_DIR/SKILL.md.bak"
fi
sed "s|{{CALCULATOR_DIR}}|$APP|g" "$HERE/skill/prevent-calculator-handoff/SKILL.md" > "$SKILL_DIR/SKILL.md"
echo "Skill installed: $SKILL_DIR/SKILL.md"

# 2. Sending cohorts from a chat runs a small Node.js script
if ! command -v node >/dev/null 2>&1; then
  echo "Note: install Node.js 18 or later (https://nodejs.org) to send cohorts from a Biorouter chat."
fi

# 3. Install the app into Biorouter and open it (the launcher finds Biorouter's background service)
if [ "${PREVENT_INSTALL_NO_OPEN:-0}" = 1 ]; then
  echo "Skipped opening the calculator (PREVENT_INSTALL_NO_OPEN=1)."
else
  bash "$APP/run.sh"
fi
