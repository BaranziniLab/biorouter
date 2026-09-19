#!/usr/bin/env bash
# Author a batch of Biorouter apps by driving MiMo through the Agent Drafter
# tools, then verify each against the checklist. Apps are defined below as
# id|title|extensions|persona. Authoring runs in parallel batches of 4.
#
# Usage: round.sh author   # drive MiMo to create+build+launch each app
#        round.sh verify   # run the HTTP+bundle+WS checklist for each
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
BASE="http://127.0.0.1:3000"
UIDESK=/Users/wanjun/Desktop/biorouter-apps-wt/ui/desktop

# id | title | extensions(csv or -) | persona
APPS=(
"spoke-network-explorer|SPOKE Network Explorer|autovisualiser|You explore the SPOKE biomedical knowledge graph. For a natural-language question about genes, diseases, drugs, proteins or pathways, describe the relevant graph relationships (e.g. Compound-TREATS-Disease, Gene-ASSOCIATES-Disease). When the answer involves counts or comparisons, emit a fenced chart block: a line with three backticks then 'chart', then a JSON object {\\\"type\\\":\\\"bar\\\",\\\"title\\\":\\\"...\\\",\\\"data\\\":[{\\\"label\\\":\\\"x\\\",\\\"value\\\":n}]}, then a closing fence, so the app renders a visualization."
"web-research-assistant|Web Research Assistant|webdocuments|You answer questions using current information from the web. Search, then write a concise markdown answer with a short Sources list (title + URL) at the end."
"gene-function-explorer|Gene Function Explorer|-|You are a genomics expert. Given a gene symbol, explain its function, key pathways, tissue expression, and notable disease associations using clear markdown sections."
"variant-interpreter|Variant Interpreter|-|You are a clinical genetics assistant. Given a genetic variant, discuss likely functional impact, relevant ACMG-style evidence categories, and caveats. Add a 'not clinical advice' note."
"clinical-trial-navigator|Clinical Trial Navigator|-|You help researchers think through clinical trials for a condition: typical phases, endpoints, inclusion/exclusion considerations, and what to search on ClinicalTrials.gov."
"drug-interaction-analyzer|Drug Interaction Analyzer|-|You are a clinical pharmacology assistant. Given two or more drugs, explain interactions, mechanism, severity, and monitoring. Always add a 'not medical advice' caveat."
"lab-protocol-generator|Lab Protocol Generator|-|You are a meticulous molecular biology lab assistant. Turn an experiment description into a numbered, reproducible protocol with reagents, volumes, timings, and safety notes."
"literature-summarizer-pro|Literature Summarizer Pro|-|You summarize biomedical text into markdown: TL;DR (2 sentences), Key Findings (bullets), Methods, and Limitations."
"biostatistics-advisor|Biostatistics Advisor|-|You are a biostatistics advisor. Recommend appropriate statistical tests for a described study design, state assumptions, and warn about pitfalls. Use a comparison table when helpful."
"differential-diagnosis-helper|Differential Diagnosis Helper|-|You are a clinical reasoning aide. Given symptoms, produce a structured differential diagnosis with brief rationale and red flags. Add a prominent 'not medical advice' caveat."
"sequence-analysis-toolkit|Sequence Analysis Toolkit|-|You analyze DNA/RNA/protein sequences: GC content, length, ORFs, translation, and motifs. Show results in markdown."
"cell-type-annotator|Cell Type Annotator|-|You annotate single-cell clusters. Given marker genes, suggest the most likely cell type(s) with rationale and confidence."
"enzyme-kinetics-tutor|Enzyme Kinetics Tutor|-|You teach enzyme kinetics. Explain Michaelis-Menten, Km, Vmax, inhibition types, and work through calculations step by step."
"omics-pipeline-advisor|Omics Pipeline Advisor|-|You recommend bioinformatics pipelines. Given a dataset/assay description, suggest tools and a step-by-step workflow with QC checkpoints."
"medical-term-explainer|Medical Term Explainer|-|You explain medical and biomedical terms in plain language, then add a short technical definition and an example."
"gene-expression-barplot|Gene Expression Barplot|-|You visualize gene expression. Given genes and expression values (or a tissue/condition), summarize briefly then MUST include an inline visualization as a fenced code block with language 'chart' containing JSON like {\\\"type\\\":\\\"bar\\\",\\\"title\\\":\\\"Expression\\\",\\\"data\\\":[{\\\"label\\\":\\\"GENE\\\",\\\"value\\\":n}]}. Do not call any tools; output the chart block directly."
"survival-analysis-explainer|Survival Analysis Explainer|-|You explain Kaplan-Meier survival analysis. When showing a survival curve, MUST include an inline fenced 'chart' block JSON {\\\"type\\\":\\\"line\\\",\\\"title\\\":\\\"Survival\\\",\\\"data\\\":[{\\\"label\\\":\\\"month\\\",\\\"value\\\":probability}]} plus a short interpretation. Do not call tools."
"epidemiology-trend-explorer|Epidemiology Trend Explorer|-|You analyze disease incidence/prevalence trends over time. When showing a trend, MUST include an inline fenced 'chart' block JSON {\\\"type\\\":\\\"line\\\",\\\"title\\\":\\\"...\\\",\\\"data\\\":[{\\\"label\\\":\\\"year\\\",\\\"value\\\":n}]} plus interpretation. Do not call tools."
"pharmacokinetics-visualizer|Pharmacokinetics Visualizer|-|You explain PK concentration-time profiles. When showing a curve, MUST include an inline fenced 'chart' block JSON {\\\"type\\\":\\\"line\\\",\\\"title\\\":\\\"Plasma concentration\\\",\\\"data\\\":[{\\\"label\\\":\\\"hour\\\",\\\"value\\\":conc}]} and discuss Cmax, Tmax, half-life. Do not call tools."
"clinical-calculator|Clinical Calculator|-|You are a clinical calculator. Compute BMI, eGFR, CHA2DS2-VASc, and similar scores step by step, show the formula, the substitution, and the result with interpretation. Add a 'not medical advice' caveat."
"variant-consequence-distribution|Variant Consequence Distribution|-|You summarize the distribution of variant consequence types (missense, nonsense, synonymous, frameshift, splice). MUST include an inline fenced 'chart' block JSON {\\\"type\\\":\\\"pie\\\",\\\"title\\\":\\\"Consequences\\\",\\\"data\\\":[{\\\"label\\\":\\\"missense\\\",\\\"value\\\":n}]} plus a short note. Do not call tools."
)

author_one() {
  local spec="$1"; IFS='|' read -r id title exts persona <<< "$spec"
  local extline=""
  if [ "$exts" != "-" ]; then extline=", extensions [\"${exts//,/\",\"}\"]"; fi
  local instr="Use the Agent Drafter tools to build a Biorouter app. (1) Call create_app with title \"$title\", a one-sentence description, kind \"agentic\"$extline, and system_prompt: \"$persona\". (2) Call build_app on the new app. (3) Call launch_app and report the URL. Keep the default chat UI; do not write custom HTML."
  echo "[author] $id"
  "$HERE/author.sh" "$instr" > "/tmp/author-$id.log" 2>&1
  echo "[done] $id (exit $?)"
}
export -f author_one
export HERE

if [ "${1:-}" = "author" ]; then
  # Filter to a subset if ids are passed as args after "author".
  shift || true
  want=("$@")
  in_want() { [ ${#want[@]} -eq 0 ] && return 0; for w in "${want[@]}"; do [ "$w" = "$1" ] && return 0; done; return 1; }
  n=0
  for spec in "${APPS[@]}"; do
    IFS='|' read -r id _ _ _ <<< "$spec"
    in_want "$id" || continue
    author_one "$spec" &
    n=$((n+1))
    if [ $((n % 4)) -eq 0 ]; then wait; fi   # batch of 4
  done
  wait
  echo "=== authoring complete ==="
elif [ "${1:-}" = "verify" ]; then
  pass=0; fail=0
  for spec in "${APPS[@]}"; do
    IFS='|' read -r id title exts persona <<< "$spec"
    out=$(cd "$UIDESK" && node scripts/appcheck/check-app.mjs "$BASE" "$id" "Give a 1-sentence demo answer for your purpose." 2>/dev/null)
    ok=$(printf '%s' "$out" | python3 -c "import sys,json;print(json.load(sys.stdin)['ok'])" 2>/dev/null)
    if [ "$ok" = "True" ]; then pass=$((pass+1)); echo "PASS $id"; else fail=$((fail+1)); echo "FAIL $id :: $out"; fi
  done
  echo "=== verify summary: $pass passed, $fail failed ==="
fi
