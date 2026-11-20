#!/usr/bin/env bash
# Build 50 Biorouter applications with DIVERSE custom UIs (sliders, dropdowns, buttons,
# toggles, chips, tabs, drag-drop, region maps) wired to the agent via br.run().
# Authors via MiMo through the Application Drafter tools, in batches.
#
# Usage: round2.sh author <id ...>     # drive MiMo to build the given app ids
#        round2.sh ids                 # print all ids
#        round2.sh ids-batch <n>       # print ids for batch n (1..5, 10 each)
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

# id | title | extensions(- for none) | ui-controls | persona
APPS=(
"dosage-calculator|Dosage Calculator|-|a weight slider, an age slider, and a drug <select>|You compute weight-based drug dosing and explain the calculation. Add a not-medical-advice caveat."
"pcr-designer|PCR Protocol Designer|-|sliders for cycles, denat/anneal/extend temps and times, plus a polymerase <select>|You design PCR thermocycling protocols from the chosen parameters."
"sample-size-calculator|Sample Size Calculator|-|sliders for effect size, power, and alpha|You compute and explain required sample size for a study."
"serial-dilution-planner|Serial Dilution Planner|-|sliders for stock conc, target conc, and number of steps|You plan serial dilutions with volumes per step."
"population-genetics-sim|Population Genetics Simulator|-|sliders for initial allele frequency, selection coefficient, and generations; render a line chart|You simulate allele-frequency change under selection and emit a chart block."
"enzyme-kinetics-plotter|Enzyme Kinetics Plotter|-|sliders for Km and Vmax; render a line chart of velocity vs substrate|You plot Michaelis-Menten kinetics and emit a chart block."
"buffer-ph-calculator|Buffer pH Calculator|-|sliders for pKa and acid/base ratio|You compute buffer pH via Henderson-Hasselbalch and explain it."
"growth-curve-modeler|Growth Curve Modeler|-|sliders for growth rate and carrying capacity; render a line chart|You model logistic population growth and emit a chart block."
"isotope-decay-calculator|Isotope Decay Calculator|-|a half-life <select> and a time slider; render a line chart|You compute radioactive decay relevant to nuclear imaging and emit a chart block."
"anesthesia-dosing-aide|Anesthesia Dosing Aide|-|weight and age sliders plus an agent <select>|You suggest anesthetic dosing ranges. Add a strong not-medical-advice caveat."
"organism-genome-explorer|Organism Genome Explorer|-|an organism <select>|You report genome size, gene count, and chromosomes for the chosen organism."
"assay-protocol-picker|Assay Protocol Picker|-|an assay-type <select>|You output a concise protocol outline for the selected assay."
"tissue-expression-lookup|Tissue Expression Lookup|-|a gene text input and a tissue <select>|You describe expression of the gene in the selected tissue."
"cancer-biomarker-guide|Cancer Biomarker Guide|-|a cancer-type <select>|You list key biomarkers and the tests used for the selected cancer."
"codon-usage-optimizer|Codon Usage Optimizer|-|an organism <select> and a sequence textarea|You advise on codon optimization for the chosen organism."
"model-organism-comparator|Model Organism Comparator|-|two organism <select> dropdowns; output a comparison table|You compare two model organisms in a markdown table."
"histology-stain-selector|Histology Stain Selector|-|a tissue <select> and a target <select>|You recommend histology stains for the selection."
"vaccine-platform-explorer|Vaccine Platform Explorer|-|a platform <select> (mRNA, viral vector, subunit, inactivated)|You explain pros/cons of the chosen vaccine platform."
"amino-acid-explorer|Amino Acid Explorer|-|a grid of 20 amino-acid buttons (br-grid + br-btn)|You give the properties of the clicked amino acid."
"interactive-codon-table|Interactive Codon Table|-|a clickable grid of codons|You report the amino acid and notes for the clicked codon."
"bio-element-reference|Bio-Element Reference|-|a button grid of biologically important elements|You explain the biological role of the clicked element."
"gene-panel-quickpick|Gene Panel Quick-Pick|-|buttons for common gene panels|You list the genes and clinical indication for the chosen panel."
"neurotransmitter-explorer|Neurotransmitter Explorer|-|a button grid of neurotransmitters|You describe the function and receptors of the clicked neurotransmitter."
"lab-unit-converter|Lab Unit Converter|-|a value input, from/to unit <select>s, and a convert button|You convert clinical lab units and show the work."
"imaging-modality-guide|Imaging Modality Guide|-|buttons for CT, MRI, PET, Ultrasound, X-ray|You explain when to use the clicked imaging modality."
"deg-filter-panel|DEG Filter Panel|-|a fold-change slider, a p-value slider, and toggles for up/down/regulated|You advise on differential-expression filtering for the chosen thresholds."
"sequencing-qc-checklist|Sequencing QC Checklist|-|checkboxes for QC flags (low Q30, duplication, adapter, GC bias)|You interpret the checked sequencing QC issues."
"pathway-db-selector|Pathway DB Selector|-|multi-select chips for KEGG/Reactome/GO/WikiPathways and a gene-list textarea|You advise on pathway enrichment using the selected databases."
"multiomics-layer-planner|Multi-Omics Layer Planner|-|toggles for genomics/transcriptomics/proteomics/metabolomics|You propose an integration strategy for the enabled layers."
"symptom-selector|Symptom Selector|-|multi-select symptom chips|You produce a differential diagnosis from the selected symptoms. Strong not-medical-advice caveat."
"nutrient-intake-advisor|Nutrient Intake Advisor|-|sliders/toggles for key nutrients|You give nutrition guidance from the inputs."
"crispr-guide-filter|CRISPR Guide Filter|-|sliders for GC% and off-target score plus toggles|You advise on guide-RNA selection for the thresholds."
"trial-eligibility-builder|Trial Eligibility Builder|-|checkboxes for inclusion/exclusion criteria|You summarize the resulting eligibility profile."
"disease-prevalence-map|Disease Prevalence by Region|-|a clickable region grid (br-mapgrid/br-region) of continents|You report disease prevalence and drivers for the clicked region."
"outbreak-explorer|Outbreak Explorer|-|a region grid and a pathogen <select>|You summarize outbreak risk for the region+pathogen."
"biobank-locator|Biobank Locator|-|a clickable region grid|You describe major biobanks in the clicked region."
"vector-borne-risk|Vector-Borne Disease Risk|-|a region grid and a disease <select>|You assess vector-borne disease risk for the selection."
"clinical-site-selector|Clinical Trial Site Selector|-|a clickable region grid|You discuss trial-site considerations for the clicked region."
"workflow-builder|Analysis Workflow Builder|-|a drag-to-reorder list (br-draglist/br-dragitem) of pipeline steps and a validate button|You validate the ordered analysis pipeline and flag issues."
"geneset-prioritizer|Gene Set Prioritizer|-|a drag-to-reorder list of genes and a rank button|You explain the rationale for the user's gene ranking."
"abstract-summarizer|Abstract Summarizer|-|a drop zone (br-dropzone) to paste/drop abstract text|You summarize the dropped text into TL;DR/findings/limitations."
"csv-quick-stats|CSV Quick Stats|-|a drop zone for CSV/text and a column textarea|You describe an appropriate statistical analysis for the data."
"protocol-step-sorter|Protocol Step Sorter|-|a drag-to-reorder list of protocol steps and a check button|You verify the protocol step order is correct."
"omics-dashboard|Omics Dashboard|-|tabs (br-tabs) for genomics/transcriptomics/proteomics, each with a query input|You answer per-tab omics questions."
"patient-summary-console|Patient Summary Console|-|tabs for history/labs/imaging, each with a notes textarea|You synthesize the active tab's clinical notes."
"gene-card|Gene Card|-|a gene input and tabs for function/disease/drugs|You fill the active tab for the entered gene."
"compound-explorer|Compound Explorer|-|a compound input and tabs for pharmacology/toxicity/interactions|You fill the active tab for the entered compound."
"study-design-wizard|Study Design Wizard|-|a form with question, population, and outcome fields|You recommend a study design from the form."
"grant-aims-drafter|Grant Aims Drafter|-|a topic input and a number-of-aims slider|You draft specific aims for the topic."
"cohort-builder|Cohort Builder|-|criteria inputs plus condition chips|You define a cohort from the criteria."
)

ids() { for s in "${APPS[@]}"; do echo "${s%%|*}"; done; }

author_one() {
  local spec="$1"; IFS='|' read -r id title exts ui persona <<< "$spec"
  local extline=""; [ "$exts" != "-" ] && extline=", extensions [\"${exts//,/\",\"}\"]"
  local instr="Use the Application Drafter tools to build a Biorouter app with a CUSTOM interactive UI (do NOT use the default chat panel). \
(1) create_app with id \"$id\", title \"$title\", a one-sentence description, kind \"agentic\"$extline, and system_prompt: \"$persona\". \
(2) update_app the index.html: build a UI featuring $ui, composed ONLY with Biorouter design-system classes (br-container, br-card, br-btn, br-select, br-slider, br-switch, br-check, br-chips/br-chip, br-tabs/br-tab, br-grid, br-row, br-dropzone, br-draglist/br-dragitem, br-mapgrid/br-region, br-badge). Include a results element <div class=\"br-output\" id=\"out\" data-placeholder=\"Results appear here\"></div>. Do NOT include any external <script>, <link>, CDN, or your own <style> colors. \
(3) update_app src/main.ts: import { createApp } from \"./sdk\"; const br = createApp({ autoChat: false }); read the control values, compose a natural-language prompt from them, and call br.run(prompt, \"#out\") on the relevant change/click/drop events. \
(4) build_app (fix any esbuild errors it reports). (5) launch_app. Keep it clean and minimal in the Biorouter style."
  "$HERE/author.sh" "$instr" > "/tmp/author2-$id.log" 2>&1
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
    wait; echo "=== authoring complete ==="
    ;;
  *) echo "usage: round2.sh author <ids...> | ids | ids-batch <n>" ;;
esac
