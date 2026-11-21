#!/usr/bin/env bash
# Benchmark: 20 apps built from VAGUE one-line ideas (no UI/control guidance).
# Measures how well Application Drafter turns under-specified requests into working,
# well-designed apps — relying on its own instructions + build harness rather
# than a detailed ticket. Compare pass-rate / UI-variety vs the detailed round2.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

# id-hint | title | vague idea (the ONLY guidance given)
APPS=(
"drug-interaction-explorer|Drug Interaction Explorer|help users check interactions between two or more medications"
"nutrition-analyzer|Nutrition Analyzer|analyze the nutritional profile of foods a user enters"
"symptom-triage|Symptom Triage|help users understand possible causes of their symptoms"
"sleep-coach|Sleep Coach|give personalized advice to improve sleep"
"lab-result-interpreter|Lab Result Interpreter|explain what a user's lab test results mean"
"fitness-planner|Fitness Planner|build a personalized workout plan"
"mental-wellness-companion|Mental Wellness Companion|offer supportive, evidence-based mental-wellness guidance"
"recipe-nutritionist|Recipe Nutritionist|suggest healthy recipes and summarize their nutrition"
"medication-guide|Medication Guide|explain how and when to take a medication"
"allergy-advisor|Allergy Advisor|help users identify and manage allergies"
"vaccine-schedule-helper|Vaccine Schedule Helper|explain recommended vaccination schedules by age"
"first-aid-guide|First Aid Guide|provide first-aid guidance for common situations"
"pregnancy-companion|Pregnancy Companion|give week-by-week pregnancy information"
"chronic-care-coach|Chronic Care Coach|help a user manage a chronic condition day to day"
"mindfulness-guide|Mindfulness Guide|guide breathing and mindfulness exercises"
"hydration-helper|Hydration Helper|advise on daily water intake for a person"
"skin-health-advisor|Skin Health Advisor|give skincare and basic dermatology guidance"
"vision-care-helper|Vision Care Helper|give eye-health advice"
"dental-care-guide|Dental Care Guide|give oral-health guidance"
"travel-health-advisor|Travel Health Advisor|give health advice for a travel destination"
)

ids() { for s in "${APPS[@]}"; do echo "${s%%|*}"; done; }

author_one() {
  local spec="$1"; IFS='|' read -r id title idea <<< "$spec"
  # Deliberately VAGUE: the idea + a usefulness nudge, nothing about controls,
  # the SDK, layout, or the design system. The agent must figure it out.
  local instr="Use the Application Drafter tools to build a Biorouter app with id \"$id\", titled \"$title\", that can $idea. Make it genuinely useful and pleasant to use. Build it and launch it."
  "$HERE/author.sh" "$instr" > "/tmp/author3-$id.log" 2>&1
  echo "[done] $id (exit $?)"
}

case "${1:-}" in
  ids) ids ;;
  ids-batch) n="${2:-1}"; ids | sed -n "$(( (n-1)*10+1 )),$(( n*10 ))p" ;;
  author)
    shift; want=("$@"); n=0
    for s in "${APPS[@]}"; do
      id="${s%%|*}"
      if [ ${#want[@]} -gt 0 ]; then printf '%s\n' "${want[@]}" | grep -qx "$id" || continue; fi
      author_one "$s" &
      n=$((n+1)); [ $((n%2)) -eq 0 ] && wait
    done
    wait; echo "=== vague authoring complete ==="
    ;;
  *) echo "usage: round3.sh author <ids...> | ids | ids-batch <n>" ;;
esac
