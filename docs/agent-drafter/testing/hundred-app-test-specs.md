# Hundred-app test specs for Application Drafter

> **What this is.** A frozen corpus of 100 ambitious app briefs — concept, theme, layout, agent profiles, declared actions, signals, bidirectional loop, platform integration — used as the stress-test workload for Application Drafter and the Apps SDK v2.
> **Status:** Current. Written 2026-07-12 for a specific test campaign, but it is a reusable input rather than a record of work: every brief is expressed in real, still-shipping SDK v2 primitives (`ui_patch`, `app_call`, signals, `br.agent`, `br.kb`, the six theme packs), so it remains valid for future runs. The corpus is **locked** — specs are numbered and cited by number in run results, so amend a brief in place rather than renumbering, inserting or deleting entries.
> **Audience:** agents and maintainers running an Application Drafter test campaign.

Each entry is one project brief to point BioRouter's **Application Drafter** at, one at a time, to build and stress-test the Apps SDK v2. Each brief is deliberately *ambitious* — it is not lowered to what a builder can do today. The value of every app is inseparable from its agentic layer: agents drive a rich screen, inform the user, and the user drives back, in a long interleaved loop.

Specs are identified as `spec-NNN` (spec 1 is `spec-001`) and authored apps are named `spec-NNN-<slug>`. The procedure that consumes this corpus — environment setup, the per-app authoring loop, the functional and aesthetic rubrics, and the findings log — is the [Application Drafter 100-app test-drive runbook](app-test-drive-runbook.md); read it before working through any of these briefs.

> **Note.** Pixel dimensions ("Left 260px rail", "Bottom 64px transport") state *design intent*, not a normative requirement. Judge an application on whether the named region exists in roughly the specified position and proportion, not on exact pixel equality.

## The hard criteria (every one of the 100 obeys these)

1. **Not a chatbot.** The primary surface is a purpose-built interface — a canvas, force-graph, map,
   board, timeline, scene, or multi-panel workbench. A chat/composer may appear only as a *small*
   secondary input, never the main event.
2. **Bidirectional, screen-centric loop.** The **agent drives the screen** (renders/patches UI, moves
   pieces, highlights, narrates its activity via the presence chip), **informs** the user of what it did
   and why, and the **user drives back** through direct manipulation (clicks, drags, lassos, sliders,
   canvas gestures). This produces a rich multi-turn series of interactions. The app is *never* about
   the user configuring or giving feedback on *how* to control the agent — the agent operates the app,
   the user operates the app, and they interleave.
3. **Multi-agent collaboration.** 2–4 named agent profiles with distinct jobs (planner + adversarial
   critic; a pipeline of specialists; a panel of judges), with explicit hand-offs / consults.
4. **Multi-step reasoning.** The interesting requests cascade through several reasoning steps or agent
   hops — never a single answer.
5. **Complex UI/UX.** Real layout with named regions, real interaction states, and specific placement
   of every key control.

## How each spec is structured (the template legend)

Every entry states the same eleven things, in the same order, one per line. Use this as the key when
you land in the middle of the file:

| Label | What it states |
|---|---|
| **Concept** | One sentence, and why it is not a chatbot. |
| **Domain & vibe** | Field + emotional register. |
| **Theme & aesthetic** | The SDK theme pack, typography/density/motion motifs, what it evokes. |
| **Layout** | Every region with position + size intent, and where each key **button** lives. |
| **Agents (multi-agent)** | The 2–4 named profiles, each job, and when they run / hand off / consult. |
| **Agent-driven UI** | Exactly which regions/widgets the agent paints or patches, what it highlights, how the presence chip narrates its steps. |
| **Declared actions** | The typed verbs the agent invokes on the app, with params. |
| **Signals (app→agent)** | The user-interaction events the agent subscribes to. |
| **User interactions** | The direct-manipulation gestures and buttons, and where they are. |
| **The bidirectional loop** | A concrete worked example turn (user → agent A reasons/consults agent B → agent patches screen + narrates → user reacts → agent adjusts). |
| **Platform integration** | Which of KB / model routes / extensions / skills / workflows / scientific figures it leans on. |

## The SDK v2 vocabulary these briefs use

The briefs reference the real Apps SDK v2 primitives so each one is testable. Full signatures are in
the [Apps SDK reference](../../apps-sdk/sdk-reference.md).

| Term | Meaning |
|---|---|
| Catalog widgets | `network` force-graph, `plot`, `table`, `kpi`, `log`, `figure` (embedded Auto-Visualiser figures), `canvas` (author-drawn surface), plus custom author-registered components. |
| `ui_patch` | Incremental agent UI edits into `@region:x` targets or dock panels. |
| `@region:x` | A named render target the app author declares in its layout; the agent patches into it. |
| `app_call` | Invocation of a typed verb the app declared — the **Declared actions** line of each brief. |
| Signals | App→agent events raised by user gestures, which the agent subscribes to. |
| `data-br-bind` | The reactive binding attribute tying DOM nodes to the shared state doc. |
| `br.agent(profile)` / `consult` | Worker agent profiles, and the tool a main agent uses to delegate to one. |
| `br.kb` | Knowledge-base access from inside an app. |
| Model routes | `fast` / `deep` / `local` — which class of model a step runs on. |
| Presence chip | The ambient indicator that narrates what the agent is currently doing. |
| Theme packs | The six packs: `biorouter`, `clinical`, `lab-notebook`, `terminal`, `journal`, `midnight`. |

## Domain index

| # | Domain | Specs | Apps |
|---|--------|-------|------|
| 1 | Biomedical & clinical research consoles | 1–10 | Variant Tribunal; Cohort Funnel Foundry; Pathway Séance; Trial Regia; Omics Loom; Ward Board; Provenance Autopsy; Manhattan Signal Room; Survival Atelier; Diagnosis Odyssey |
| 2 | Scientific simulation & modeling workbenches | 11–20 | Reaction-Diffusion Foundry; Contagion Studio; Orbital Sandbox; Serengeti Engine; FoldScape; AeroCanvas; Automata Loom; SystemDynamics Forge; Circuit Bench; Diffusion Delta |
| 3 | Knowledge cartography & literature synthesis | 21–30 | Radiant; Crossfire; Longitude; Quorum; Lattice; Ledger; Fault Lines; Vantage; Watershed; Keystone |
| 4 | Data investigation & forensic analytics | 31–40 | Anomaly Atlas; Trace Weaver; Causal Court; Drift Sentinel; Ledger Loom; Split Verdict; Chain of Custody; Cohort Contrast; Log Loom; Recon Board |
| 5 | Geospatial & field-ops planners | 41–50 | ContagionScope; ExpeditionForge; RelayOptimizer; IsoReach; StormWatch; BioSentinel; TerraFuture; TrailWeave; VectorFront; GridHarbor |
| 6 | Creative & design studios | 51–60 | Gridwright — Editorial Layout Studio; Palette Séance — Brand Moodboard System; Panelith — Storyboard & Comic Board; Reverb Loom — Soundscape Arranger; Kern & Karma — Typography Lab; Voxel Atelier — 3D Scene Composer; Bindery — Generative Book & Zine Layout; Loomframe — Motion Title Sequencer; Chroma Diplomat — Color System Studio; Set & Setting — Environmental Diorama Composer |
| 7 | Agent-driven worlds, avatars & serious games | 61–70 | Hexfront: The Council of Generals; Biosphere: The Tending Table; The Long Table: Diplomacy Room; Vault-7: The Escape Room Engine; Bastion: Tower-Defense as Roadmap; Gait Studio: The Motion Coaches; Terra Nova: The Colony Council; Pendulum: The Physics Playground Tutors; Reef Wardens: The Living Aquarium; Ironclad: The Heist Table |
| 8 | Operations, monitoring & control rooms | 71–80 | Aurora Grid Balancer; Incident Warroom Timeline; FleetOps Orchestrator; Quant Desk Sentinel; Pipeline Nexus; Brewhouse Console; Sky Sentinel ATC-Ops; Approvals Queue Command; Datacenter Thermal Bridge; Observatory Nightwatch |
| 9 | Education, labs & interactive derivation | 81–90 | Proof Forge; Wet-Bench Virtuoso; Chrono-Reconstruct; Circuit Sandbox; Immersion Atrium; Derivation Loom; Anatomy Atelier; Cipher Workshop; Ecosystem Terrarium; Kernel Foundry |
| 10 | Decision cockpits & mixed-initiative editors | 91–100 | Roadmap Regatta; Tradeoff Radar; Redline Room; Scenario Matrix; Negotiation Table; Budget Loom; Decision Room; Portfolio Kanban Arena; Policy Sandbox; Resume Atelier |

---

## 1. Biomedical & clinical research consoles  ·  #1–10

### 1. Variant Tribunal

**Concept:** A genome-track viewer where competing agents adjudicate the pathogenicity of a single variant across evidence tracks — a courtroom for a VUS, not a chat about it.

**Domain & vibe:** Clinical germline genomics; tense, forensic, high-stakes.

**Theme & aesthetic:** `clinical` pack; monospaced coordinates, dense stacked tracks, near-zero chrome, amber only on contested calls, verdict chips in red/green.

**Layout:** Left 260px rail: variant queue + ACMG criteria checklist (28 tags, lit/unlit). Center: horizontal genome-track viewer (gnomAD frequency, conservation phyloP, splice-AI delta, ClinVar stars, protein-domain lollipop) scrollable/zoomable around the locus. Right 340px inspector: the live "verdict card" with per-criterion votes. Bottom 64px transport: zoom slider, "Re-adjudicate" button, model-route toggle (deep). Floating presence chip top-right narrates which agent is speaking.

**Agents (multi-agent):** *Prosecutor* argues pathogenic, pulling gnomAD rarity + splice/conservation tracks; *Defense* argues benign, citing population frequency + functional nulls; *Chief Justice* weighs both against ACMG rules and writes the classification; *Clerk* pulls citations from br.kb into the record.

**Agent-driven UI:** agents light/unlight ACMG tags in the rail, paint highlight bands onto specific tracks (`annotate_track`), and stream their argument into @region:verdict as struck-through/accepted criteria; presence narrates "Defense refuting PM2…".

**Declared actions:** `focus_locus(chrom,pos)`, `highlight_track(track,span,color)`, `toggle_criterion(tag,state)`, `set_verdict(class,confidence)`, `cite(pmid,span)`, `stage_route(preset)`.

**Signals (app→agent):** `criterion_clicked(tag)`, `track_region_drawn(track,span)`, `variant_selected(id)`, `verdict_challenged(tag)`.

**User interactions:** user drags a span on the conservation track to contest it, clicks an ACMG tag to force re-argument, picks the next variant from the queue, drags the zoom slider.

**The bidirectional loop:** User lassos a splice-region span → Prosecutor consults Defense on that span, invokes highlight_track + toggle_criterion(PS1) → screen lights PS1 red, presence says "Prosecutor invoking PS1 on canonical splice"→ user clicks the lit PS1 to challenge → Chief Justice re-weighs, downgrades to Likely Pathogenic, patches @region:verdict.

**Platform integration:** br.kb (ClinVar/literature), scientific figures (lollipop/track), deep model route for adjudication, genomics skills.

### 2. Cohort Funnel Foundry

**Concept:** A drag-to-carve cohort funnel canvas where agents propose, critique, and refine inclusion/exclusion stages against a live patient count — you sculpt a cohort, you don't describe one.

**Domain & vibe:** Clinical epidemiology / EHR cohort building; deliberate, quantitative, PHI-safe.

**Theme & aesthetic:** `terminal` pack; green-on-black funnel bars, tabular counts, blinking cursor on the active stage, red strike when a stage zeroes out.

**Layout:** Left 280px rail: criterion library (labs, ICD, meds, demographics) as draggable chips. Center: vertical funnel canvas — each stage a bar whose width = surviving N, drop-off ribbon between stages. Right 340px inspector: selected-stage detail (SQL preview, count, drop reasons). Bottom 64px transport: "Materialize cohort" + "Balance arms" buttons. Floating attrition KPI top-right.

**Agents (multi-agent):** *Architect* proposes a stage ordering from the target phenotype; *Auditor* critiques for immortal-time bias, leakage, and tiny cells; *Statistician* checks arm balance and power after each edit; *Scribe* writes a STROBE-style rationale into @region:log.

**Agent-driven UI:** Architect places stage bars and animates count updates; Auditor pins red warning flags on risky stages; Statistician patches a live power gauge; presence narrates "Auditor flags immortal time between stage 2→3".

**Declared actions:** `add_stage(criterion,params)`, `reorder_stage(id,idx)`, `set_count(stage,n)`, `flag_stage(id,reason)`, `place_power_gauge(value)`, `preview_sql(stage)`.

**Signals (app→agent):** `chip_dropped(criterion,idx)`, `stage_reordered(id,idx)`, `stage_selected(id)`, `threshold_edited(stage,value)`.

**User interactions:** user drags a lab chip onto the funnel, reorders bars, edits a numeric threshold inline, clicks a bar to see drop reasons.

**The bidirectional loop:** User drags "eGFR<30" above the diagnosis stage → Architect reorders, invokes set_count → Auditor consults, flags selection bias, invokes flag_stage → presence warns → user drags it back down → Statistician re-runs power, patches gauge green, Scribe updates rationale.

**Platform integration:** institutional model route (PHI-safe SQL), br.kb (phenotype defs), workflows (materialize), scientific figures (attrition funnel).

### 3. Pathway Séance

**Concept:** A force-graph pathway map where agents grow, prune, and stress-test a mechanistic hypothesis network live — the graph is the argument, not a transcript about it.

**Domain & vibe:** Systems biology / mechanism discovery; exploratory, luminous, generative.

**Theme & aesthetic:** `midnight` pack; dark canvas, neon edges by evidence type, pulsing nodes on active reasoning, edges fade by confidence.

**Layout:** Left 240px rail: seed genes/metabolites + evidence-type legend toggles. Center: force-graph pathway map (genes, proteins, metabolites, drugs) with clustering. Right 340px inspector: selected node/edge dossier (papers, direction, confidence). Bottom 64px transport: "Grow", "Prune weak", "Find missing link" buttons + layout preset selector. Floating presence chip.

**Agents (multi-agent):** *Cartographer* expands the graph from seeds via br.kb; *Skeptic* refutes low-evidence edges and dims them; *Bridger* proposes missing intermediary nodes between disconnected clusters; *Narrator* writes the mechanistic story into @region:brief when the graph stabilizes.

**Agent-driven UI:** Cartographer places/links nodes; Skeptic recolors and dims edges; Bridger inserts ghost nodes the user can accept; presence narrates "Bridger proposing IL6→STAT3 to connect clusters".

**Declared actions:** `add_node(id,type)`, `link(a,b,evidence,conf)`, `dim_edge(id)`, `propose_bridge(a,b,intermediary)`, `focus_node(id)`, `stage_layout(preset)`.

**Signals (app→agent):** `node_selected(id)`, `edge_selected(id)`, `lasso(region)`, `bridge_accepted(id)`, `legend_toggled(type)`.

**User interactions:** user lassos a cluster to expand it, clicks an edge to demand evidence, accepts/rejects a ghost bridge node, toggles evidence legends to filter.

**The bidirectional loop:** User lassos two disconnected clusters → Cartographer consults Bridger, invokes propose_bridge → ghost node appears, presence explains → user double-clicks to accept → Skeptic checks evidence, dims one supporting edge → Narrator patches @region:brief with the caveated mechanism.

**Platform integration:** br.kb graph search, scientific figures (network), deep route for bridging inference, pathway-analysis skill.

### 4. Trial Regia

**Concept:** A trial-design studio built on a Gantt/timeline canvas where agents lay out arms, visits, and endpoints while adversarially checking feasibility and power — you compose the protocol on a schedule, not in prose.

**Domain & vibe:** Clinical trial design / biostatistics; meticulous, regulatory, high-consequence.

**Theme & aesthetic:** `journal` pack; serif headers, ruled schedule-of-assessments grid, restrained ink, a single vermilion accent on critical-path bars.

**Layout:** Left 260px rail: arms + endpoint library. Center: Gantt/timeline — rows = arms, columns = study weeks, cells = visits/procedures; a schedule-of-assessments grid docked below. Right 340px inspector: selected-endpoint power + sample-size card. Bottom 64px transport: "Power it", "Check feasibility", "Export protocol" buttons. Floating enrollment-feasibility KPI.

**Agents (multi-agent):** *Designer* lays out arms/visits/endpoints; *Biostatistician* computes power and MDE per endpoint; *Regulatory Critic* checks against ICH-GCP and flags burdensome schedules; *Operationalizer* estimates enrollment feasibility from site data in br.kb.

**Agent-driven UI:** Designer draws visit cells and endpoint bars; Biostatistician patches a live sample-size card + KM projection figure; Regulatory Critic pins compliance flags on cells; presence narrates "Critic flags visit-15 blood draw exceeds volume limit".

**Declared actions:** `place_visit(arm,week,proc)`, `set_endpoint(id,type,effect)`, `compute_power(endpoint)`, `flag_cell(arm,week,reason)`, `place_km_projection(arm)`, `export_protocol()`.

**Signals (app→agent):** `cell_edited(arm,week)`, `endpoint_selected(id)`, `arm_added(name)`, `effect_size_dragged(endpoint,value)`.

**User interactions:** user drags an effect-size slider, adds an arm, drags a visit to a new week, clicks a flagged cell to see the rule.

**The bidirectional loop:** User drags MDE slider down on the primary endpoint → Biostatistician recomputes, invokes compute_power → sample size balloons, KM projection figure repaints → Operationalizer consults site data, warns enrollment infeasible → user shortens follow-up → Regulatory Critic re-checks, clears the flag.

**Platform integration:** institutional model route, br.kb (site/enrollment data), scientific figures (Kaplan-Meier, forest), clinical-biostatistics skill, workflows (protocol export).

### 5. Omics Loom

**Concept:** A multi-omics integrator whose central surface is a linked multi-panel workbench (volcano + heatmap + network) that agents weave together across layers — you interrogate concordance, not ask about it.

**Domain & vibe:** Multi-omics integration; synthetic, weaving, revelatory.

**Theme & aesthetic:** `lab-notebook` pack; graph-paper backdrop, taped-figure cards, handwritten-style annotations, muted washi-tape accents per omics layer.

**Layout:** Left 240px rail: omics-layer selector (transcriptome/proteome/metabolome/methylome) + feature search. Center: three linked panels — volcano (top-left), sample×feature heatmap (top-right), cross-layer network (bottom). Right 340px inspector: selected-feature multi-layer trace. Bottom 64px transport: "Integrate layers", "Find concordant", "Cluster" buttons. Floating concordance KPI.

**Agents (multi-agent):** *Aligner* harmonizes feature IDs across layers; *Correlator* finds concordant/discordant signals and draws cross-layer edges; *Contrarian* surfaces where layers disagree and boxes them on the volcano; *Weaver* writes the integrated interpretation into @region:synthesis.

**Agent-driven UI:** Correlator brushes concordant points across all three panels simultaneously (linked highlight); Contrarian boxes discordant genes on the volcano and pins them; presence narrates "Contrarian: protein up, mRNA down for GENEX".

**Declared actions:** `link_brush(feature_ids)`, `highlight_volcano(gene,box)`, `draw_cross_edge(a,b,layers)`, `cluster_heatmap(method)`, `focus_feature(id)`, `place_kpi(concordance)`.

**Signals (app→agent):** `point_clicked(panel,id)`, `heatmap_cell_selected(row,col)`, `brush(panel,region)`, `layer_toggled(name)`.

**User interactions:** user brushes a volcano region, clicks a heatmap block, toggles an omics layer on/off, selects a network node to trace it everywhere.

**The bidirectional loop:** User brushes upregulated volcano genes → Correlator consults Aligner for cross-layer IDs, invokes link_brush → same genes light in heatmap + network → Contrarian flags one with inverse proteomics, invokes highlight_volcano box → presence explains → user clicks the boxed gene → Weaver patches @region:synthesis with the post-transcriptional hypothesis.

**Platform integration:** br.kb (ID mapping), scientific figures (volcano/heatmap/network), deep route, differential-expression + multi-omics-integration skills.

### 6. Ward Board

**Concept:** A clinical-decision board where a panel of specialist agents debate a live patient across problem-oriented cards while you arbitrate — a tumor-board table, not a Q&A.

**Domain & vibe:** Inpatient clinical decision support; measured, consequential, PHI-safe.

**Theme & aesthetic:** `clinical` pack; card-based problem list, restrained blues, urgency coral on unstable vitals, dense timelines inside cards.

**Layout:** Left 260px rail: problem list + differential ranking. Center: board of problem-oriented cards (each = problem, evidence, plan) arrangeable in a grid; a vitals/labs sparkline strip across the top. Right 340px inspector: selected-problem evidence + orders. Bottom 64px transport: "Round", "Rank differential", "Draft orders" buttons. Floating acuity KPI.

**Agents (multi-agent):** *Hospitalist* frames problems and drafts plans; *Subspecialist* (cardiology/ID, invoked per problem) deep-dives one card; *Devil's Advocate* challenges anchoring and proposes can't-miss diagnoses; *Documentarian* writes the assessment/plan into @region:note.

**Agent-driven UI:** Hospitalist reorders differential cards; Subspecialist expands one card with a trend figure; Devil's Advocate pins a red "consider" card; presence narrates "ID consult: coverage gaps for the eGFR".

**Declared actions:** `add_problem(name)`, `rank_differential(order)`, `expand_card(id,figure)`, `pin_consideration(text)`, `draft_orders(problem,list)`, `place_acuity(value)`.

**Signals (app→agent):** `card_selected(id)`, `card_reordered(id,idx)`, `order_toggled(id)`, `problem_dismissed(id)`.

**User interactions:** user drags problem cards to reprioritize, dismisses a differential, toggles proposed orders, clicks a vitals spark to zoom.

**The bidirectional loop:** User dismisses "sepsis" from the differential → Devil's Advocate consults Subspecialist, invokes pin_consideration("can't-miss: early sepsis") with a lactate trend figure → presence warns → user reinstates it → Hospitalist re-ranks, drafts orders, Documentarian patches @region:note.

**Platform integration:** institutional model route (PHI-safe), br.kb (guidelines), scientific figures (trend/sparkline), clinical-databases skill.

### 7. Provenance Autopsy

**Concept:** A provenance-table forensics console where agents trace a suspect result back through every data transformation, flagging where a bioinformatics pipeline went wrong — a lineage investigation, not a chat log.

**Domain & vibe:** Reproducibility / pipeline forensics; investigative, skeptical, exacting.

**Theme & aesthetic:** `terminal` pack; monospaced provenance table, DAG mini-map, red diff highlights on suspect cells, evidence chain-of-custody strip.

**Layout:** Left 260px rail: artifact list (files, params, versions) + suspicion filter. Center: provenance table (rows = transform steps, cols = inputs/outputs/params/hashes) with a linked upstream DAG mini-map above. Right 340px inspector: selected-step diff + logs. Bottom 64px transport: "Trace back", "Diff runs", "Bisect" buttons. Floating integrity KPI.

**Agents (multi-agent):** *Tracer* walks lineage upstream from the suspect artifact; *Diff Hunter* compares this run against a known-good run cell-by-cell; *Bisector* binary-searches the step where outputs diverged; *Reporter* writes the root-cause narrative into @region:findings.

**Agent-driven UI:** Tracer highlights the active lineage path in the DAG and table; Diff Hunter paints red diff cells; Bisector marks the divergence step with a boundary line; presence narrates "Bisector: divergence isolated to normalization step 7".

**Declared actions:** `trace_upstream(artifact)`, `highlight_path(step_ids)`, `mark_diff(row,col)`, `set_bisect_bound(step,side)`, `focus_step(id)`, `place_integrity(value)`.

**Signals (app→agent):** `row_selected(step)`, `cell_clicked(row,col)`, `run_compared(run_id)`, `suspicion_marked(step)`.

**User interactions:** user clicks a suspect output cell, picks a comparison run, marks a step as trustworthy to narrow the bisect, expands logs.

**The bidirectional loop:** User clicks an anomalous DE count cell → Tracer traces upstream, highlights path → Diff Hunter compares to last-good run, paints red on a params cell → presence flags "seed changed" → user marks step 5 trusted → Bisector narrows to step 7, sets boundary → Reporter patches @region:findings with the root cause.

**Platform integration:** br.kb (pipeline docs), model routes (fast trace, deep root-cause), workflows (re-run step), reproducibility skills.

### 8. Manhattan Signal Room

**Concept:** A GWAS control room built around a live Manhattan/locus-zoom viewer where agents mine peaks, fine-map, and connect hits to biology — you prospect the genome, not query it.

**Domain & vibe:** Statistical genetics / GWAS; prospecting, panoramic, rigorous.

**Theme & aesthetic:** `midnight` pack; wide dark Manhattan plot, coral significance line, teal fine-mapping overlays, monospaced rsIDs.

**Layout:** Left 240px rail: trait/phenotype selector + peak list ranked by p-value. Center: Manhattan plot (top) linked to a locus-zoom panel (bottom) that opens on a selected peak. Right 340px inspector: credible-set + eQTL colocalization card. Bottom 64px transport: "Scan peaks", "Fine-map", "Colocalize" buttons. Floating genomic-inflation KPI.

**Agents (multi-agent):** *Prospector* ranks and annotates genome-wide peaks; *Fine-Mapper* computes credible sets and overlays them on locus-zoom; *Colocalizer* tests eQTL/pQTL overlap to name causal genes; *Interpreter* writes the locus story into @region:locusbrief.

**Agent-driven UI:** Prospector drops labeled pins on peaks; Fine-Mapper shades the credible-set region and dims non-causal SNPs; Colocalizer draws a link from SNP to gene; presence narrates "Colocalizer: rs123 colocalizes with GENEX expression in liver".

**Declared actions:** `pin_peak(chrom,pos,label)`, `open_locuszoom(peak)`, `shade_credible_set(snps)`, `link_to_gene(snp,gene,tissue)`, `focus_snp(rsid)`, `place_inflation(lambda)`.

**Signals (app→agent):** `peak_clicked(chrom,pos)`, `snp_selected(rsid)`, `region_zoomed(span)`, `tissue_toggled(name)`.

**User interactions:** user clicks a peak to open locus-zoom, drags to zoom a region, toggles tissue for colocalization, selects an rsID.

**The bidirectional loop:** User clicks a chr9 peak → Prospector opens locus-zoom → Fine-Mapper computes credible set, shades 5 SNPs → user toggles "liver" tissue → Colocalizer consults br.kb eQTL, links rs123→GENEX, presence narrates → user selects that SNP → Interpreter patches @region:locusbrief with the candidate mechanism.

**Platform integration:** br.kb (eQTL/gene annotation), scientific figures (Manhattan, locus-zoom), deep route (fine-mapping), statistical-genetics skills.

### 9. Survival Atelier

**Concept:** A survival-analysis studio where agents build, stratify, and adversarially validate Kaplan-Meier/Cox models on a plotting canvas as you carve strata — you sculpt survival curves, not request them.

**Domain & vibe:** Clinical outcomes / survival modeling; scrupulous, elegant, cautionary.

**Theme & aesthetic:** `journal` pack; serif captions, a large KM plotting canvas, muted stratum palette, a single crimson for failed proportional-hazards checks.

**Layout:** Left 260px rail: covariate library + stratum builder chips. Center: KM plotting canvas (curves with risk table below); a forest-plot dock on the right edge. Right 340px inspector: selected-stratum hazard ratio + diagnostics. Bottom 64px transport: "Fit Cox", "Check PH", "Adjust confounders" buttons. Floating C-index KPI.

**Agents (multi-agent):** *Modeler* fits KM/Cox and draws curves; *Diagnostician* runs proportional-hazards + residual checks and flags violations; *Confounder Hunter* proposes adjustments from br.kb and re-fits; *Curator* writes the methods/results into @region:methods.

**Agent-driven UI:** Modeler draws stratum curves + risk table; Diagnostician flashes a stratum crimson when PH fails and overlays a Schoenfeld residual mini-figure; Confounder Hunter adds a forest-plot row per adjustment; presence narrates "Diagnostician: PH violated for stage after 24mo".

**Declared actions:** `add_stratum(covariate,cut)`, `fit_cox(covariates)`, `draw_km(strata)`, `flag_ph(stratum,reason)`, `add_forest_row(hr,ci)`, `place_cindex(value)`.

**Signals (app→agent):** `stratum_chip_dropped(covariate,cut)`, `curve_selected(stratum)`, `cutpoint_dragged(covariate,value)`, `forest_row_clicked(id)`.

**User interactions:** user drags a continuous covariate cutpoint, adds a stratum chip, clicks a curve to inspect, drags a confounder into the model.

**The bidirectional loop:** User drags an age cutpoint to 65 → Modeler re-draws two KM curves + risk table → Diagnostician runs PH, flashes the older stratum crimson, overlays Schoenfeld figure → presence warns → user adds "stage" as a confounder → Confounder Hunter re-fits Cox, adds a forest row, C-index KPI updates, Curator patches @region:methods.

**Platform integration:** institutional model route, br.kb (confounder priors), scientific figures (Kaplan-Meier, forest), clinical-biostatistics skill.

### 10. Diagnosis Odyssey

**Concept:** A rare-disease diagnostic-odyssey map where agents traverse a phenotype-to-disease reasoning graph as an explorable journey, backtracking on refutation — you navigate the differential terrain, not interrogate it.

**Domain & vibe:** Rare-disease / undiagnosed-disease diagnostics; patient, exploratory, hopeful-yet-rigorous.

**Theme & aesthetic:** `biorouter` pack; explorable node-map with a traveled-path trail, phenotype constellations, gold on confirmed nodes, faded on refuted branches.

**Layout:** Left 260px rail: HPO phenotype list + evidence strength toggles. Center: reasoning-graph map (phenotypes → syndromes → genes) with an animated traversal trail; a "path taken" breadcrumb strip on top. Right 340px inspector: candidate-disease dossier (matched/missing phenotypes, test to order). Bottom 64px transport: "Advance", "Backtrack", "Order test" buttons. Floating diagnostic-yield KPI.

**Agents (multi-agent):** *Pathfinder* expands the most probable next hop from current phenotypes; *Refuter* tests candidates against missing/excluding phenotypes and fades dead branches; *Test Recommender* proposes the highest-yield next test; *Chronicler* writes the diagnostic journey into @region:odyssey.

**Agent-driven UI:** Pathfinder extends the trail and lights the next node gold; Refuter fades refuted branches and draws an X; Test Recommender pins a test badge on a frontier node; presence narrates "Refuter: absent cardiomyopathy excludes Fabry branch".

**Declared actions:** `advance_to(node)`, `fade_branch(node,reason)`, `light_node(node,state)`, `pin_test(node,test)`, `focus_candidate(id)`, `place_yield(value)`.

**Signals (app→agent):** `node_clicked(id)`, `branch_pruned(node)`, `phenotype_toggled(hpo)`, `test_ordered(id)`.

**User interactions:** user clicks a frontier node to advance, toggles an HPO term on/off, prunes a branch by hand, accepts a recommended test.

**The bidirectional loop:** User toggles "on" a new HPO seizure term → Pathfinder re-expands, lights two syndrome nodes gold → Refuter consults br.kb, fades one for an excluding feature, draws X → presence explains → user clicks the surviving node → Test Recommender pins a gene-panel badge, yield KPI rises → Chronicler patches @region:odyssey.

**Platform integration:** br.kb (HPO/OMIM/gene-disease graph), scientific figures (network), deep route (differential reasoning), rare-disease + clinical-databases skills.

## 2. Scientific simulation & modeling workbenches  ·  #11–20

### 11. Reaction-Diffusion Foundry

**Concept:** A live GPU Turing-pattern canvas where agents steer feed/kill chemistry toward a target morphology while you paint perturbations by hand — a wet-lab bench, not a chat.

**Domain & vibe:** Morphogenesis / pattern formation; hypnotic, tactile, slightly alien.

**Theme & aesthetic:** `lab-notebook` pack; monospace captions, medium density, ink-on-cream, patterns rendered in a single teal→magenta ramp; motion only in the simulation cell.

**Layout:** Left 240px reagent rail (feed F, kill K, diffusion Du/Dv sliders, brush picker); Center: full-bleed reaction-diffusion `canvas` widget; Right 340px inspector with a live `plot` of the F/K phase diagram (a dot marks current regime) plus a `figure` of the extracted pattern's spectral fingerprint; Bottom 64px transport bar (play/pause, step, speed, seed, "capture frame"). Floating presence chip top-right. "Steer to target" button sits atop the inspector; "Snapshot to timeline" bottom-right.

**Agents (multi-agent):** *Cartographer* maps where in F/K space the current pattern lives and proposes moves; *Perturbationist* runs micro-sweeps, applying transient brush strokes to test stability; *Morphologist* scores each frame against the user's target (spots/stripes/labyrinth) and vetoes drift, handing accepted regimes back to Cartographer.

**Agent-driven UI:** Agents drag the F/K dot across the phase `plot`, paint seed blooms onto the canvas, and patch the spectral `figure`; presence narrates "Perturbationist nudged K +0.004 → stripes stabilizing."

**Declared actions:** `set_param(name,value)`, `paint_seed(x,y,r,species)`, `run_stage(steps)`, `stage_regime(F,K)`, `score_pattern(target)`, `capture_frame(label)`.

**Signals (app→agent):** `brush_stroke(path,species)`, `param_dragged(name,value)`, `region_lassoed(mask)`, `target_chosen(kind)`.

**User interactions:** You drag reagent sliders, paint seeds/erasers directly on the canvas, lasso a region to protect, and hit "Steer to target."

**The bidirectional loop:** You lasso a labyrinth patch and pick "make it spots" → Cartographer locates the regime and proposes ΔF; Perturbationist consults Morphologist on stability, applies a test bloom, narrates the plan → agent slides the F/K dot and paints seeds, captioning each step → you drag F back to disrupt it → Morphologist flags divergence and Cartographer re-plots a corrective path.

**Platform integration:** model routes (fast for per-frame scoring, deep for regime planning), scientific figures for spectral fingerprints, KB of canonical Gray-Scott regimes, workflow to batch-sweep overnight.

### 12. Contagion Studio

**Concept:** A compartmental-epidemic workbench where agents fit and stress-test an SIR/SEIR model against a live epicurve while you drag intervention levers on a timeline — an outbreak control room, not a chatbot.

**Domain & vibe:** Epidemiology / public-health modeling; tense, consequential, briefing-room.

**Theme & aesthetic:** `clinical` pack; crisp sans, high data density, white/steel with a single coral accent reserved for the active outbreak wave; near-zero chrome, KPIs in tabular figures.

**Layout:** Left 260px compartment rail (β, σ, γ, R₀ readout, population sliders, vaccine coverage); Center: stacked `plot` of S/E/I/R curves over a scrubbable time axis with draggable intervention markers; Right 340px inspector with `kpi` tiles (peak load, attSince rate, R_eff) and a `table` of scenario runs; Bottom 64px transport (play, scrub, "add intervention", speed). Floating presence chip. "Fit to data" button top-left of center; "Compare scenarios" opens a right-side grid.

**Agents (multi-agent):** *Fitter* calibrates β/σ/γ to uploaded case data via least-squares sweeps; *Adversary* injects worst-case variants (higher R₀, waning immunity) and refutes over-optimistic fits; *Policy Analyst* proposes NPIs (masking, closures, vaccination timing) and writes accepted plans into the timeline; *Reporter* narrates and writes the brief.

**Agent-driven UI:** Agents drop intervention markers on the timeline, repaint fitted curves, flash coral on threshold breaches, and patch `kpi` tiles; presence narrates "Adversary: with waning immunity, second wave peaks at 2.1× ICU capacity."

**Declared actions:** `fit_model(data)`, `set_rate(name,value)`, `add_intervention(t,type,strength)`, `run_forecast(days)`, `compare_scenarios(ids)`, `highlight_threshold(metric,limit)`.

**Signals (app→agent):** `marker_dragged(t,type)`, `rate_slider(name,value)`, `scenario_selected(id)`, `region_scrubbed(t0,t1)`.

**User interactions:** You scrub the timeline, drag intervention markers, tug compartment sliders, and click "Compare scenarios."

**The bidirectional loop:** You upload case counts and hit "Fit to data" → Fitter sweeps rates, plots the calibrated curve, narrates fit quality → you drag a "school closure" marker to week 6 → Policy Analyst re-forecasts while Adversary stress-tests waning immunity, both patching the curve and KPI tiles → coral flashes on an ICU breach → you slide vaccine coverage up until the breach clears.

**Platform integration:** KB of NPI effect sizes, model routes (fast sweeps / deep policy synthesis), scientific figures for age-stratified panels, workflow to sweep parameter grids, table export.

### 13. Orbital Sandbox

**Concept:** A gravitational N-body playground where agents design and stabilize orbital configurations while you fling bodies and set velocities by dragging vectors — a mission-planning console, not a chat box.

**Domain & vibe:** Celestial mechanics / astrodynamics; awe, precision, a little dangerous.

**Theme & aesthetic:** `midnight` pack; thin luminous strokes, low density, deep-space black with orbit trails in a cool blue-violet ramp, mass labels in condensed mono; smooth inertial motion.

**Layout:** Left 220px body rail (per-body mass, position, velocity vector, add/delete); Center: full-bleed 2D orbital `canvas` with draggable bodies and velocity arrows, trails, barycenter cross; Right 340px inspector with a phase-space `plot` (energy vs. time, angular momentum) and a `table` of orbital elements (a, e, i, period); Bottom 64px transport (play, timescale, step, "reverse time"). Floating presence chip. "Stabilize system" button top-right; "Detect resonance" in the inspector.

**Agents (multi-agent):** *Navigator* proposes velocity/position tweaks to reach a target orbit or Lagrange point; *Chaos Auditor* integrates forward and flags ejections/collisions, refuting fragile configs via Lyapunov estimates; *Ephemeris Scribe* logs stable configurations and writes orbital-element tables.

**Agent-driven UI:** Agents drag velocity arrows, place bodies at computed Lagrange points, draw predicted trajectory ghosts on the canvas, and patch the energy `plot`; presence narrates "Chaos Auditor: this trojan drifts — ejection in ~40 periods."

**Declared actions:** `set_velocity(id,vx,vy)`, `place_body(mass,x,y)`, `run_integration(dt,steps)`, `stabilize(target)`, `detect_resonance()`, `ghost_trajectory(id,horizon)`.

**Signals (app→agent):** `body_dragged(id,x,y)`, `vector_pulled(id,vx,vy)`, `body_added(mass)`, `timescale_changed(rate)`.

**User interactions:** You drag bodies, pull velocity arrows, spin the timescale dial, and hit "Stabilize system."

**The bidirectional loop:** You drop a third body between two stars and hit "Stabilize" → Navigator computes an L4 solution and drags it there with a velocity arrow → Chaos Auditor integrates 200 periods, draws a drifting ghost trail, narrates the instability → you nudge its mass down → Navigator recomputes, Ephemeris Scribe logs the now-stable trojan into the elements table.

**Platform integration:** model routes (fast integration steps / deep resonance analysis), scientific figures for phase portraits, KB of restricted three-body solutions, workflow to batch-sweep initial conditions.

### 14. Serengeti Engine

**Concept:** An agent-based predator-prey ecosystem on a spatial grid where agents tune population dynamics and habitat while you paint terrain and drop herds — a living diorama, not a chat interface.

**Domain & vibe:** Ecology / population dynamics; playful yet precarious, wildlife-documentary gravitas.

**Theme & aesthetic:** `journal` pack; warm serif captions, medium density, parchment ground with grass/water/rock tiles in muted earth tones, agents as tiny colored motes; organic drifting motion.

**Layout:** Left 250px species rail (birth/death rates, speed, vision radius, energy per species; brush: grass/water/rock/spawn); Center: full-bleed grid `canvas` of the ecosystem; Right 340px inspector with a Lotka-Volterra `plot` (predator vs. prey populations over time) plus a phase-space loop `figure` and a species `table`; Bottom 64px transport (play, speed, "seed random", generation counter). Floating presence chip. "Balance ecosystem" button top-right; "Introduce species" bottom-left.

**Agents (multi-agent):** *Ranger* tunes rates toward a stable coexistence and paints habitat corridors; *Invasive Species* adversarially introduces a competitor and hunts for collapse; *Naturalist* watches the phase loop, classifies the regime (stable/oscillating/extinction), and writes field notes.

**Agent-driven UI:** Agents paint water corridors and grass onto the grid, drop herds, animate the L-V `plot` and phase `figure`, and highlight collapsing quadrants; presence narrates "Naturalist: prey crashed in the north — corridor fragmentation."

**Declared actions:** `set_species_rate(species,param,value)`, `paint_terrain(mask,type)`, `spawn_herd(species,x,y,n)`, `run_generations(n)`, `classify_regime()`, `balance()`.

**Signals (app→agent):** `terrain_painted(mask,type)`, `herd_dropped(species,x,y)`, `rate_slider(species,param,value)`, `quadrant_selected(rect)`.

**User interactions:** You paint terrain, scatter herds by clicking, drag rate sliders, and hit "Balance ecosystem."

**The bidirectional loop:** You paint a river bisecting the map → Naturalist flags fragmented prey, narrates the risk → you hit "Balance" → Ranger paints a grass corridor across a ford and lowers predator vision, animating the phase loop toward a closed orbit → Invasive Species drops rabbits to test it and predicts a crash → you cull the rabbits by lassoing a quadrant → Naturalist confirms the orbit re-closes.

**Platform integration:** KB of ecological parameter ranges, model routes (fast stepping / deep regime classification), scientific figures for phase portraits, workflow to sweep initial densities overnight.

### 15. FoldScape

**Concept:** A protein energy-landscape explorer where agents descend folding funnels and propose mutations while you drag the chain and rotate the 3D structure — a structural-biology cockpit, not a chatbot.

**Domain & vibe:** Structural biology / molecular biophysics; meditative, high-stakes, elegant.

**Theme & aesthetic:** `biorouter` pack; clean sans, medium-high density, cool neutral ground, energy surface in a viridis ramp, backbone ribbons in chain-colored strokes; slow orbital rotation of the 3D view.

**Layout:** Left 240px residue rail (sequence strip, per-residue φ/ψ, mutation picker, force-field toggle); Center split: top a 3D structure `canvas` (rotatable ribbon), bottom a 2D energy-landscape `plot` (RMSD vs. energy funnel with a descending marker); Right 340px inspector with a Ramachandran `figure` and a contact-map `figure`; Bottom 64px transport (minimize, step, temperature, "run folding"). Floating presence chip. "Minimize energy" button over the structure; "Mutate & rescore" in the residue rail.

**Agents (multi-agent):** *Folder* runs simulated-annealing descents down the funnel; *Mutagenesis Critic* proposes stabilizing mutations and adversarially predicts destabilizing ones (ΔΔG); *Validator* checks Ramachandran outliers and clashes, vetoing physically implausible states before they're accepted.

**Agent-driven UI:** Agents move the marker down the energy `plot`, rotate and re-ribbon the 3D structure, recolor strained residues coral, and patch the Ramachandran and contact-map `figure`s; presence narrates "Mutagenesis Critic: L34→P kinks the helix, ΔΔG +2.1 kcal/mol — rejected."

**Declared actions:** `set_dihedral(res,phi,psi)`, `mutate(res,aa)`, `run_minimization(steps,temp)`, `score_state()`, `highlight_residues(ids)`, `orient_structure(view)`.

**Signals (app→agent):** `residue_selected(id)`, `dihedral_dragged(res,phi,psi)`, `structure_rotated(view)`, `mutation_chosen(res,aa)`.

**User interactions:** You drag φ/ψ dihedrals, rotate the ribbon, pick mutations from the rail, and hit "Minimize energy."

**The bidirectional loop:** You select a bulging loop and drag its dihedral → Validator flags a steric clash in coral → you ask to fix it → Folder runs a local minimization, walking the marker down the funnel and re-ribboning the loop → Mutagenesis Critic proposes G→A to rigidify, predicts ΔΔG, updates the contact map → you accept, then rotate to inspect the new packing.

**Platform integration:** KB of rotamer libraries and ΔΔG data, model routes (fast scoring / deep mutation reasoning), scientific figures for Ramachandran + contact maps, workflow to batch-mutate a scanning panel.

### 16. AeroCanvas

**Concept:** A 2D computational-fluid wind-tunnel where agents shape an airfoil and steer flow conditions while you sketch obstacles and drag the inlet vector — a wind-tunnel console, not a chat window.

**Domain & vibe:** Fluid dynamics / aerodynamics; sleek, kinetic, engineering-cool.

**Theme & aesthetic:** `terminal` pack; monospace HUD, high density, near-black ground with streamlines in a cyan→amber velocity ramp, vorticity in a diverging map; crisp, immediate motion, coral only on separation/stall warnings.

**Layout:** Left 230px condition rail (inlet velocity, angle of attack, viscosity/Reynolds, obstacle brush); Center: full-bleed fluid `canvas` showing streamlines/vorticity around a draggable airfoil; Right 340px inspector with a live `plot` of lift/drag vs. angle of attack and a pressure-coefficient `figure` around the surface; Bottom 64px transport (play, reset flow, "inject dye", speed). Floating presence chip. "Optimize airfoil" button top-right; "Find stall angle" in the inspector.

**Agents (multi-agent):** *Aerodynamicist* reshapes the airfoil control points to maximize lift/drag; *Turbulence Skeptic* probes for flow separation and vortex shedding, refuting configs that stall early; *Instrumenter* logs Cl/Cd and writes the polar `plot` and Cp `figure`.

**Agent-driven UI:** Agents drag airfoil control points, inject dye streaklines, repaint the streamline field, flash coral on separation bubbles, and patch the lift/drag `plot`; presence narrates "Turbulence Skeptic: separation at 12° — trailing edge sheds vortices."

**Declared actions:** `set_inlet(v,angle)`, `reshape_airfoil(points)`, `set_reynolds(re)`, `run_solve(steps)`, `inject_dye(x,y)`, `sweep_aoa(range)`.

**Signals (app→agent):** `control_point_dragged(id,x,y)`, `inlet_vector_pulled(v,angle)`, `obstacle_drawn(path)`, `aoa_slider(deg)`.

**User interactions:** You drag the airfoil's control points, pull the inlet vector, sketch obstacles, and hit "Optimize airfoil."

**The bidirectional loop:** You pull the angle of attack to 15° → Turbulence Skeptic detects separation, flashes a coral bubble, narrates the stall → you hit "Optimize" → Aerodynamicist thickens the leading edge and drags control points, re-solving the flow → Instrumenter sweeps AoA and redraws the polar, marking the new stall angle → you sketch a slat to test → the Skeptic reruns and confirms delayed separation.

**Platform integration:** model routes (fast per-frame solve / deep shape optimization), scientific figures for Cp distributions and polars, KB of airfoil families (NACA series), workflow to sweep Reynolds numbers.

### 17. Automata Loom

**Concept:** A cellular-automata rule-space explorer where agents hunt for interesting rules (gliders, oscillators, edge-of-chaos) while you draw seed patterns and edit the rule table — a discovery loom, not a chatbot.

**Domain & vibe:** Complexity science / discrete dynamical systems; obsessive, puzzle-like, retro-scientific.

**Theme & aesthetic:** `terminal` pack; blocky mono, maximal density, phosphor-green cells on black with a subtle scanline, rule bits as a toggle strip; stepwise flicker motion, amber highlight on discovered structures.

**Layout:** Left 240px rule rail (birth/survival toggle strip, neighborhood picker, states count, brush palette); Center: full-bleed CA grid `canvas`; Right 340px inspector with a Langton's-λ / entropy `plot` classifying the rule, a `log` of discovered structures, and a `table` of periods/velocities; Bottom 64px transport (play, step, speed, generation counter, "randomize seed"). Floating presence chip. "Hunt for gliders" button top-right; "Classify rule" in the inspector.

**Agents (multi-agent):** *Explorer* mutates the rule table and seeds test patterns, hunting Wolfram class IV behavior; *Taxonomist* detects and classifies emergent structures (still lifes, oscillators, spaceships) with periods/velocities; *Archivist* refutes duplicates against the KB and writes genuinely novel finds into the log.

**Agent-driven UI:** Agents flip rule-table bits, stamp seed patterns onto the grid, box-highlight discovered gliders in amber, animate the λ/entropy `plot`, and append to the `log`; presence narrates "Taxonomist: period-4 spaceship, velocity c/4 orthogonal — new."

**Declared actions:** `set_rule(birth,survival)`, `stamp_pattern(x,y,name)`, `run_generations(n)`, `classify_rule()`, `highlight_structure(rect,label)`, `mutate_rule(temperature)`.

**Signals (app→agent):** `cell_painted(x,y,state)`, `rule_bit_toggled(index)`, `region_boxed(rect)`, `seed_stamped(name)`.

**User interactions:** You paint cells, toggle rule bits, box a moving structure to identify, and hit "Hunt for gliders."

**The bidirectional loop:** You toggle a survival bit and paint a blob → Taxonomist classifies the rule as chaotic on the λ plot → you hit "Hunt" → Explorer mutates the rule toward the class-IV band, stamping test seeds, narrating each trial → a glider emerges; Taxonomist boxes it in amber and logs period/velocity → Archivist checks the KB, flags it novel → you box a second structure to name it yourself.

**Platform integration:** KB of known CA rules and named patterns (Life lexicon), model routes (fast stepping / deep classification), scientific figures for λ-entropy plots, workflow to batch-scan rule space overnight.

### 18. SystemDynamics Forge

**Concept:** A stock-and-flow systems-dynamics modeler where agents build and calibrate feedback-loop models on a canvas while you wire stocks to flows and drag converter values — a policy simulator, not a chat box.

**Domain & vibe:** Systems dynamics / policy modeling; deliberate, cybernetic, boardroom-analytical.

**Theme & aesthetic:** `biorouter` pack; clean sans, medium density, cool paper ground, stocks as rounded rectangles, flows as valved pipes, feedback loops arced in blue (reinforcing) and orange (balancing); smooth flowing-fluid pipe animation.

**Layout:** Left 250px element rail (stock/flow/converter/connector tools, equation editor for the selected node); Center: node-and-pipe `canvas` (the stock-flow diagram) with a docked mini `plot` strip below showing each stock over time; Right 340px inspector with a loop-dominance `figure`, sensitivity `plot`, and a `table` of parameters; Bottom 64px transport (run, speed, time horizon, "reset stocks"). Floating presence chip. "Auto-wire model" button top-right; "Find dominant loop" in the inspector.

**Agents (multi-agent):** *Architect* proposes stocks/flows/loops from a described problem and wires the diagram; *Loop Analyst* identifies reinforcing vs. balancing loops and computes dominance over time; *Calibrator* fits converter constants to reference behavior and adversarially checks for unintended oscillation/overshoot.

**Agent-driven UI:** Agents drop stocks and draw valved flows onto the canvas, arc feedback loops, animate the per-stock `plot` strip, and highlight the currently dominant loop in the `figure`; presence narrates "Loop Analyst: reinforcing loop R2 dominates after t=30 — exponential blowup."

**Declared actions:** `add_node(type,x,y)`, `connect(from,to,kind)`, `set_equation(node,expr)`, `run_simulation(horizon)`, `find_dominant_loop()`, `highlight_loop(id)`.

**Signals (app→agent):** `node_dragged(id,x,y)`, `connector_drawn(from,to)`, `equation_edited(node,expr)`, `node_selected(id)`.

**User interactions:** You drag stocks onto the canvas, draw connectors between them, edit equations, and hit "Auto-wire model."

**The bidirectional loop:** You describe "adoption with word-of-mouth and saturation" and hit "Auto-wire" → Architect drops Adopters/Potential stocks and a WOM flow, arcing R1 and B1 loops → Loop Analyst runs it, animates the S-curve, highlights R1 early then B1 late → you drag the contact-rate converter up → Calibrator warns of overshoot, patches the plot, suggests a lower value → you accept and rerun.

**Platform integration:** KB of canonical system archetypes (limits to growth, etc.), model routes (fast simulation / deep loop analysis), scientific figures for loop-dominance and sensitivity, workflow to sweep policy levers.

### 19. Circuit Bench

**Concept:** An analog-circuit simulation bench where agents design and tune filter/amplifier topologies on a schematic canvas while you drag component values and probe nodes — an oscilloscope-driven bench, not a chatbot.

**Domain & vibe:** Electronics / analog systems engineering; precise, hands-on, hobbyist-meets-lab.

**Theme & aesthetic:** `lab-notebook` pack; mono labels, high density, graph-paper ground, components as clean IEEE symbols, wires in ink-blue, live nodes glowing amber; needle-smooth scope traces.

**Layout:** Left 240px parts rail (R/L/C/op-amp/source palette, value editor for selected part); Center: schematic `canvas` with draggable components and wires; Right 340px inspector: an oscilloscope `plot` (probed node voltages vs. time) atop a Bode `figure` (gain/phase vs. frequency); Bottom 64px transport (run, AC/DC/transient mode, frequency sweep, "auto-probe"). Floating presence chip. "Design filter" button top-right; "Find -3dB point" in the inspector.

**Agents (multi-agent):** *Designer* synthesizes a topology to hit a target spec (cutoff, gain, Q) and places parts; *SPICE Analyst* runs the transient/AC solve and reports poles/zeros; *Tolerance Critic* Monte-Carlos component tolerances and refutes designs that drift out of spec.

**Agent-driven UI:** Agents place components and route wires on the schematic, attach probes, draw scope traces and the Bode plot, mark the -3dB point, and flash coral on saturation/instability; presence narrates "Tolerance Critic: 5% caps push cutoff ±180Hz — 12% out of spec."

**Declared actions:** `place_part(type,x,y)`, `set_value(id,value)`, `wire(a,b)`, `run_analysis(mode,params)`, `probe_node(id)`, `sweep_frequency(range)`.

**Signals (app→agent):** `part_dragged(id,x,y)`, `value_edited(id,value)`, `wire_drawn(a,b)`, `node_probed(id)`.

**User interactions:** You drag components onto the grid, wire nodes, edit values, clip probes to nodes, and hit "Design filter."

**The bidirectional loop:** You ask for a 1kHz low-pass and hit "Design filter" → Designer places an RC/op-amp Sallen-Key, wires it, narrates the topology → SPICE Analyst sweeps frequency, draws the Bode curve, marks -3dB at 1.02kHz → you drag a resistor value up → the scope and Bode redraw live → Tolerance Critic Monte-Carlos tolerances, flashes a coral spec-violation band, suggests tighter caps → you swap them and rerun.

**Platform integration:** model routes (fast solve / deep topology synthesis), scientific figures for Bode/pole-zero plots, KB of filter topologies and op-amp specs, workflow to Monte-Carlo tolerance sweeps.

### 20. Diffusion Delta

**Concept:** A geophysical advection-diffusion transport simulator where agents model a contaminant plume across a terrain grid while you paint sources, sinks, and wind/flow fields — a hazard-response map, not a chat interface.

**Domain & vibe:** Environmental / geophysical modeling; urgent, cartographic, mission-serious.

**Theme & aesthetic:** `midnight` pack; condensed sans, medium-high density, dark basemap with terrain contours, concentration in a plasma ramp, flow field as faint vector arrows; smooth plume advection, coral isopleth on threshold exceedance.

**Layout:** Left 250px field rail (diffusivity, decay rate, wind speed/direction dial, source/sink/barrier brush); Center: full-bleed geospatial `map` canvas with the plume overlay and vector field; Right 340px inspector with a breakthrough-curve `plot` (concentration at monitor points vs. time), a dosage `kpi`, and a `table` of monitor stations; Bottom 64px transport (play, speed, "release plume", time). Floating presence chip. "Forecast plume" button top-right; "Site monitors" in the rail.

**Agents (multi-agent):** *Meteorologist* sets the wind/flow field and advects the plume; *Dispersion Modeler* runs the advection-diffusion solve and forecasts arrival times; *Risk Assessor* adversarially evaluates worst-case wind shifts, marks exceedance isopleths, and writes evacuation-priority notes.

**Agent-driven UI:** Agents paint the wind vector field, release the plume overlay on the map, draw coral exceedance isopleths, drop monitor markers, and patch the breakthrough `plot`; presence narrates "Risk Assessor: a NW wind shift puts the plume over the reservoir in 90 min."

**Declared actions:** `set_field(param,value)`, `paint_source(x,y,rate)`, `set_wind(speed,dir)`, `run_transport(minutes)`, `place_monitor(x,y)`, `mark_exceedance(threshold)`.

**Signals (app→agent):** `source_painted(x,y,rate)`, `wind_dial(speed,dir)`, `barrier_drawn(path)`, `monitor_placed(x,y)`.

**User interactions:** You paint sources/sinks/barriers on the map, spin the wind dial, drop monitor stations, and hit "Forecast plume."

**The bidirectional loop:** You paint a spill source and hit "Forecast" → Meteorologist sets the prevailing wind and advects the plume across the map → Dispersion Modeler forecasts breakthrough at three monitors, drawing their curves → Risk Assessor tests a NW wind shift, draws a coral exceedance isopleth over a reservoir, narrates the risk → you drag a barrier berm to divert flow → the Modeler reruns and the isopleth retreats.

**Platform integration:** KB of dispersion coefficients and terrain data, model routes (fast advection steps / deep worst-case analysis), scientific figures for breakthrough curves, extensions for geospatial basemaps, workflow to sweep wind scenarios.

## 3. Knowledge cartography & literature synthesis  ·  #21–30

### 21. Radiant

**Concept:** A radial living concept-map studio where the agent grows a domain's idea-galaxy outward from a seed term and the user prunes, pins, and re-centers it by dragging — a cartography workbench, not a chat that spits links.

**Domain & vibe:** Interdisciplinary science synthesis; the calm exhilaration of watching a field's structure bloom.

**Theme & aesthetic:** `journal` pack; warm ivory ground, serif node labels, hairline radial spokes, ink-blue accents that saturate only on live/expanding nodes; slow easing on layout tweens, near-zero chrome.

**Layout:** Left 240px seed rail (seed terms, saved galaxies, "Grow depth" stepper); Center: full-bleed radial force-graph (`network`) with a fixed center node and concentric rings; Right 340px inspector showing selected concept's definition, top citations, and neighbor edges; Bottom 56px transport bar with **Grow**, **Prune weak edges**, **Re-center**, **Snapshot** buttons; a floating presence chip top-right.

**Agents (multi-agent):** *Cartographer* queries `br.kb.graph`/`search` to expand a node into child concepts and lays them on rings; *Weeder* scores each new edge's evidence and flags thin ones amber; *Scribe* writes the inspector's concept brief when a node is selected.

**Agent-driven UI:** Cartographer patches `@region:map` with new nodes/edges via app_call; Weeder recolors weak edges and posts a dock list of "edges needing evidence"; presence narrates "Expanding *neuroinflammation* — 7 children, 2 weak."

**Declared actions:** `grow_node(id,depth)`, `place_ring(nodes[],ring)`, `flag_edge(src,dst,reason)`, `recenter(id)`, `write_brief(id,md)`, `prune(ids[])`.

**Signals (app→agent):** `node_selected(id)`, `node_dragged(id,ring)`, `node_pinned(id)`, `prune_requested(ids)`.

**User interactions:** Double-click a node to request growth; drag nodes between rings to reorder importance; pin a node to freeze it; lasso to prune; click **Re-center** to make a node the new sun.

**The bidirectional loop:** User double-clicks *microglia* → Cartographer consults `br.kb.graph`, places 6 children on ring 2, narrates → Weeder amber-flags 2 sparse edges and docks them → user drags *TREM2* inward and pins it → Cartographer re-weights the ring and Scribe rewrites *TREM2*'s brief citing three KB pages.

**Platform integration:** `br.kb` graph/search core; deep model route for briefs, fast for growth; scientific `figure` embeds a mini evidence plot in the inspector.

### 22. Crossfire

**Concept:** An argument-tree debate mapper where two adversarial agents build and attack a claim's pro/con tree on a branching canvas while the user arbitrates nodes — a steelman workbench, not a debate chatbot.

**Domain & vibe:** Contested science/policy (e.g. "statins for primary prevention"); tense, courtroom-adjacent focus.

**Theme & aesthetic:** `terminal` pack; near-black canvas, mono labels, green claim nodes / red rebuttals / amber unresolved; sharp orthogonal connectors, snappy motion, coral only on the node currently under attack.

**Layout:** Left 220px claim stack (root claims, verdict tallies); Center: top-down argument tree (`network`, hierarchical) with expandable pro/con children; Right 360px inspector: selected node's evidence, source quality bar, "steelman this" and "refute this" buttons; Bottom 60px transport: **Advance debate**, **Auto-steelman**, **Score branch**, **Freeze verdict**.

**Agents (multi-agent):** *Advocate* builds the strongest pro sub-tree; *Prosecutor* attaches rebuttals and finds counter-evidence via `br.kb.search`; *Referee* scores each branch's net strength and marks unresolved forks amber; hands off Advocate→Prosecutor→Referee each round.

**Agent-driven UI:** Advocate/Prosecutor patch `@region:tree` with new child nodes and orthogonal edges; Referee writes per-branch scores into node badges and a dock verdict panel; presence narrates "Prosecutor attacking node 3.2 with 2 RCTs."

**Declared actions:** `add_claim(parent,text,stance)`, `attach_evidence(node,cite)`, `score_branch(node,net)`, `mark_unresolved(node)`, `focus_node(id)`, `collapse(node)`.

**Signals (app→agent):** `node_selected(id)`, `stance_flipped(node)`, `verdict_frozen(node)`, `evidence_challenged(cite)`.

**User interactions:** Click a node to inspect; press **Refute this** to sic the Prosecutor; drag to reparent a stray argument; toggle a node's accepted/rejected chip; freeze a verdict to lock a subtree.

**The bidirectional loop:** User clicks root claim, presses **Auto-steelman** → Advocate grows 4 pro nodes citing KB pages, narrates → Prosecutor attacks the weakest with a meta-analysis, recoloring it coral → user challenges the meta-analysis's quality → Referee re-scores the branch amber and docks "unresolved: heterogeneity" → user freezes the two solid children.

**Platform integration:** `br.kb` search for evidence; deep route for Prosecutor's counter-arguments; `figure` renders a forest-plot of the cited trials in the inspector.

### 23. Longitude

**Concept:** A timeline-of-ideas cartography board where the agent traces how a concept mutated across decades of literature and the user scrubs, branches, and annotates the intellectual lineage — a history-of-thought canvas, not a Q&A box.

**Domain & vibe:** History and philosophy of science; reflective, archival gravity.

**Theme & aesthetic:** `lab-notebook` pack; gridded cream paper, dated margin ticks, sepia node cards, a single teal "you are here" playhead; ruled connectors, gentle parallax on scrub.

**Layout:** Top 72px era ruler (decades, zoomable); Center: horizontal swimlane timeline (`canvas`) with idea-cards on lanes per school/lab; Left 200px lane rail (schools, toggle visibility); Right 340px inspector: selected paper's abstract, who-cited-whom, "trace forward/back"; Bottom 56px transport: **Trace lineage**, **Branch here**, **Collapse era**, **Snapshot**.

**Agents (multi-agent):** *Archivist* pulls seminal papers per era from `br.kb.search`; *Genealogist* draws citation-lineage edges and detects concept splits/merges; *Annotator* writes margin notes explaining each turning point.

**Agent-driven UI:** Archivist places dated cards on lanes in `@region:timeline`; Genealogist patches lineage arcs and highlights the branch under study; Annotator writes margin cards; presence narrates "Tracing *fitness landscape* from Wright 1932 forward — 3 forks."

**Declared actions:** `place_card(paper,year,lane)`, `draw_lineage(from,to,kind)`, `mark_fork(year,concept)`, `scrub_to(year)`, `annotate(card,md)`, `collapse_era(range)`.

**Signals (app→agent):** `playhead_moved(year)`, `card_selected(id)`, `lane_toggled(school)`, `branch_requested(card)`.

**User interactions:** Drag the playhead to scrub eras; click a card to inspect; drag a card to a different lane to re-attribute a school; press **Branch here** to fork an alternate lineage; annotate cards inline.

**The bidirectional loop:** User scrubs to 1975 and clicks a card → Archivist surfaces 5 contemporaries, narrates → Genealogist draws lineage arcs and flags a concept fork → user presses **Branch here** on the fork → Genealogist spawns a parallel lane and Annotator writes "term bifurcates: ecological vs. statistical fitness."

**Platform integration:** `br.kb` search/graph for citations; fast route for card placement, deep for lineage reasoning; `figure` plots a citations-per-year sparkline in the inspector.

### 24. Quorum

**Concept:** A systematic-review board where a pipeline of specialist agents screens, extracts, and grades studies onto a PRISMA-style kanban while the user adjudicates borderline papers by dragging cards — a review cockpit, not a chat assistant.

**Domain & vibe:** Evidence synthesis / meta-science; meticulous, audit-trail seriousness.

**Theme & aesthetic:** `clinical` pack; cool white, dense tabular cards, status-tinted column headers, one coral "conflict" badge; crisp shadows, minimal motion, checklist iconography.

**Layout:** Left 240px protocol rail (inclusion criteria, PICO chips, PRISMA counts); Center: horizontal kanban (`canvas`) — Identified → Screened → Eligible → Included → Excluded columns of study cards; Right 360px inspector: selected study's extracted fields + risk-of-bias grid; Bottom 60px transport: **Screen batch**, **Extract fields**, **Grade bias**, **Export PRISMA**.

**Agents (multi-agent):** *Screener* filters titles/abstracts against criteria via `br.kb.search`; *Extractor* pulls PICO + outcomes into structured fields; *Appraiser* scores risk-of-bias and flags conflicts for human review; handoff Screener→Extractor→Appraiser per card.

**Agent-driven UI:** Screener moves cards across `@region:board` columns with reasons; Extractor fills the inspector's field grid; Appraiser paints a RoB heat-row and docks conflicts; presence narrates "Screened 40 — 12 eligible, 3 need adjudication."

**Declared actions:** `move_card(id,column,reason)`, `extract_fields(id,fields)`, `grade_bias(id,domains)`, `flag_conflict(id)`, `update_prisma(counts)`, `focus_card(id)`.

**Signals (app→agent):** `card_dragged(id,column)`, `criterion_edited(chip)`, `field_overridden(id,key,val)`, `conflict_resolved(id)`.

**User interactions:** Edit PICO chips to re-run screening; drag a borderline card between columns to overrule the Screener; correct an extracted field inline; resolve a conflict badge.

**The bidirectional loop:** User tightens the "RCT only" chip → Screener re-screens, narrates, moves 5 cards to Excluded → user drags one back, disagreeing → Extractor re-pulls its fields → Appraiser grades high RoB and flags a conflict → user resolves it, updating the PRISMA flow counts live.

**Platform integration:** `br.kb` search + `page` for full-text; `systematic-review`/`clinical-biostatistics` skills; deep route for appraisal; `figure` renders the PRISMA flow diagram on export.

### 25. Lattice

**Concept:** A hypothesis-lattice explorer where agents generate a partially-ordered graph of nested hypotheses and rank them by testability while the user promotes, forks, and kills branches — a discovery-planning surface, not a brainstorming chat.

**Domain & vibe:** Experimental biology hypothesis generation; ambitious, generative optimism.

**Theme & aesthetic:** `midnight` pack; deep indigo ground, glowing node halos keyed to support strength, violet-to-cyan gradient edges; springy layout physics, comet-trail motion when a branch is promoted.

**Layout:** Left 220px seed rail (research question, constraints, "generate N"); Center: DAG lattice (`network`, layered by specificity) of hypothesis nodes; Right 340px inspector: selected hypothesis's rationale, required assays, testability score breakdown; Bottom 56px transport: **Generate children**, **Rank testability**, **Prune dead ends**, **Design experiment**.

**Agents (multi-agent):** *Generator* proposes child hypotheses via deep route grounded in `br.kb`; *Falsifier* stress-tests each for testability and prior refutation, dimming weak ones; *Planner* drafts an experiment plan for a promoted node.

**Agent-driven UI:** Generator adds layered nodes to `@region:lattice`; Falsifier adjusts node halo brightness and docks "hard-to-test" warnings; Planner writes an assay checklist into the inspector; presence narrates "Generated 5 sub-hypotheses for *pathway X*; 2 falsifiable."

**Declared actions:** `spawn_children(id,n)`, `score_testability(id,breakdown)`, `dim_node(id,reason)`, `promote(id)`, `draft_experiment(id,plan)`, `link_prior(id,cite)`.

**Signals (app→agent):** `node_selected(id)`, `node_promoted(id)`, `node_killed(id)`, `constraint_edited(chip)`.

**User interactions:** Click **Generate children** on a node; drag to re-order sibling priority; kill a branch with a swipe; promote a node to summon an experiment plan; edit constraints to re-rank.

**The bidirectional loop:** User seeds a question, presses **Generate children** → Generator proposes 5 hypotheses citing KB → Falsifier dims 2 as untestable, narrates why → user kills one, promotes another → Planner drafts a CRISPR-screen protocol into the inspector and links two prior papers as controls.

**Platform integration:** `br.kb` for grounding + prior-art; `single-cell`/`crispr-screens` skills for assay plans; deep route for generation, fast for scoring; `figure` sketches an expected-effect plot per hypothesis.

### 26. Ledger

**Concept:** A claims-vs-evidence matrix where agents populate a grid of claims (rows) against studies (columns) with support/refute/mixed cells while the user audits and re-weights cells — a synthesis spreadsheet driven by reasoning, not a chatbot.

**Domain & vibe:** Contested-literature reconciliation; forensic, ledger-keeping rigor.

**Theme & aesthetic:** `terminal` pack; charcoal grid, mono cell glyphs (+ / − / ~), green/red/amber cell fills, one bright cursor cross-hair; instant cell transitions, coral only on the cell being computed.

**Layout:** Left 260px claims rail (row headers, add-claim, sort by consensus); Top 48px study ribbon (column headers, quality chips); Center: dense scrollable matrix (`table` upgraded to interactive grid) of support cells; Right 340px inspector: selected cell's quote, page, and confidence; Bottom 60px transport: **Fill row**, **Fill column**, **Recompute consensus**, **Export matrix**.

**Agents (multi-agent):** *Miner* reads each study via `br.kb.page` and marks its stance on each claim; *Auditor* verifies quotes and downgrades over-claims; *Synthesizer* writes a per-claim consensus verdict into the row header.

**Agent-driven UI:** Miner fills matrix cells in `@region:grid`; Auditor recolors disputed cells and docks "quote not supported" flags; Synthesizer writes row verdict badges; presence narrates "Filling row *drug reduces mortality* across 8 studies."

**Declared actions:** `fill_cell(row,col,stance,quote)`, `audit_cell(row,col,verdict)`, `set_consensus(row,label)`, `weight_study(col,quality)`, `focus_cell(row,col)`, `export_matrix(fmt)`.

**Signals (app→agent):** `cell_selected(row,col)`, `cell_edited(row,col,stance)`, `study_reweighted(col)`, `claim_added(text)`.

**User interactions:** Click a cell to read its quote; override a stance glyph; drag a study column to re-order by quality; add a claim row to trigger a fill; adjust a study's quality weight to recompute.

**The bidirectional loop:** User adds a claim row → Miner reads 8 studies, fills the row, narrates → Auditor flags one cell's quote as unsupported, recolors it amber → user downgrades that study's quality weight → Synthesizer recomputes and flips the row verdict from "mixed" to "supported," updating the consensus badge.

**Platform integration:** `br.kb` page/search for full-text stance mining; deep route for auditing quotes; `differential-expression`/`clinical-biostatistics` skills where claims are quantitative; `figure` renders a support-tally bar per claim.

### 27. Fault Lines

**Concept:** A contradiction-finder map where agents crawl a literature set to surface pairs of mutually incompatible findings and plot them as tension-edges the user investigates and reconciles — a disagreement radar, not a chat.

**Domain & vibe:** Reproducibility / conflicting-results detective work; suspicious, investigative edge.

**Theme & aesthetic:** `midnight` pack; near-black, findings as pale nodes, contradiction edges as pulsing red seismic lines, reconciled edges fade to slate; jittery pulse motion on active fault lines only.

**Layout:** Left 220px finding rail (findings list, "scan for contradictions", severity filter); Center: force-graph (`network`) where red edges = contradictions, node size = citation weight; Right 360px inspector: the two conflicting statements side-by-side + candidate reconciliations; Bottom 56px transport: **Scan**, **Explain conflict**, **Propose reconciliation**, **Mark resolved**.

**Agents (multi-agent):** *Prospector* extracts atomic findings and pairs contradictory ones via `br.kb.search`; *Adjudicator* explains why they conflict and rates severity; *Mediator* proposes reconciliations (population diffs, methods, dosage) and drafts a resolving note.

**Agent-driven UI:** Prospector draws red tension-edges in `@region:map`; Adjudicator pulses the active fault and docks a severity readout; Mediator writes the reconciliation into the inspector and fades resolved edges; presence narrates "Found 4 conflicts on *biomarker X*; 1 severe."

**Declared actions:** `add_finding(text,cite)`, `link_conflict(a,b,severity)`, `explain_conflict(edge,md)`, `propose_reconcile(edge,options)`, `resolve_edge(edge,note)`, `focus_edge(edge)`.

**Signals (app→agent):** `edge_selected(a,b)`, `finding_selected(id)`, `reconcile_accepted(edge,option)`, `severity_filter_changed(level)`.

**User interactions:** Click a red edge to open the side-by-side; filter by severity; accept or reject a proposed reconciliation; mark an edge resolved (it fades); add a finding manually to test for conflicts.

**The bidirectional loop:** User presses **Scan** → Prospector links 4 contradiction edges, narrates → user clicks the severe one → Adjudicator explains the conflict, pulsing the edge → Mediator proposes "different cell lines" as the resolver → user accepts, edge fades to slate and a resolving note is filed to the inspector.

**Platform integration:** `br.kb` search/page for finding extraction; deep route for adjudication; `scientific-research` skill; `figure` overlays the two conflicting effect sizes on one plot.

### 28. Vantage

**Concept:** A steelman debate-map where a panel of persona-agents each argues a distinct stance on a wheel-of-viewpoints while the user pits them, merges positions, and forces synthesis — a perspectives cockpit, not a single-voice chatbot.

**Domain & vibe:** Ethics/policy of science (e.g. gene-editing governance); deliberative, roundtable poise.

**Theme & aesthetic:** `biorouter` pack; soft neutral ground, each viewpoint a colored sector, position cards with quote pull-outs; smooth radial rotation when the wheel re-centers, accent only on the speaking persona.

**Layout:** Center: radial "wheel of viewpoints" (`canvas`) with 4-6 stance sectors and a central synthesis disc; Left 220px stance rail (persona roster, add/remove viewpoint); Right 340px inspector: selected stance's strongest argument, evidence, blind spots; Bottom 56px transport: **Convene panel**, **Cross-examine**, **Merge stances**, **Synthesize**.

**Agents (multi-agent):** three-plus persona *Advocates* (e.g. Bioethicist, Clinician, Patient-Advocate) each build their sector's case via `br.kb`; a *Moderator* cross-examines and finds tensions between sectors; a *Synthesizer* writes a balanced position into the center disc.

**Agent-driven UI:** each Advocate paints its sector's position cards in `@region:wheel`; Moderator draws tension-lines between sectors and narrates cross-exam; Synthesizer fills the central disc; presence chip names the speaking persona.

**Declared actions:** `build_stance(persona,md)`, `cross_examine(a,b,tension)`, `highlight_blindspot(persona,text)`, `merge_stances(ids)`, `write_synthesis(md)`, `rotate_to(persona)`.

**Signals (app→agent):** `sector_selected(persona)`, `stance_merged(ids)`, `persona_added(name)`, `synthesis_requested()`.

**User interactions:** Click a sector to hear its case; drag two sectors together to force a merge; add a persona to the wheel; press **Cross-examine** to pit two; demand **Synthesize** for the center disc.

**The bidirectional loop:** User convenes the panel → three Advocates fill their sectors citing KB, presence names each → user drags Clinician and Patient-Advocate together → Moderator surfaces their tension over access-vs-safety → Synthesizer writes a merged position into the disc → user rejects it, presses **Cross-examine** on the Bioethicist to sharpen the caveats.

**Platform integration:** `br.kb` per-persona grounding; deep route for synthesis, fast for stance cards; `scientific-research`/`anti-ai-writing` skills for prose quality; `figure` optional stakeholder-impact plot.

### 29. Watershed

**Concept:** A living review-map that continuously ingests a topic's new preprints and re-draws the field's cluster topography, alerting the user to emerging sub-fields via a spatial map they steer — a monitoring cartography surface, not a chat feed.

**Domain & vibe:** Research-front surveillance; the vigilance of a weather station.

**Theme & aesthetic:** `lab-notebook` pack; contour-map aesthetic, papers as dots in topographic clusters, elevation = citation density, new work in fresh green; slow drift animation as clusters shift, ripple on new arrivals.

**Layout:** Center: 2-D embedding map (`canvas`) with contour clusters and labeled basins; Left 220px watch rail (tracked topics, ingest cadence, "scan now"); Right 340px inspector: selected cluster's summary, top papers, growth trend; Bottom 56px transport: **Scan feed**, **Recompute clusters**, **Name basin**, **Alert on growth**; floating alert toast top-right.

**Agents (multi-agent):** *Harvester* pulls new papers via `br.kb.search`/workflow and embeds them; *Cartographer* re-clusters and re-labels basins; *Sentinel* detects fast-growing or newly-emerged clusters and raises alerts with a written brief.

**Agent-driven UI:** Harvester drops new dots onto `@region:terrain`; Cartographer redraws contours and renames basins; Sentinel ripples an emerging cluster and toasts an alert; presence narrates "Ingested 22 preprints; new basin forming near *spatial omics*."

**Declared actions:** `ingest_batch(source)`, `recluster(params)`, `label_basin(cluster,name)`, `raise_alert(cluster,brief)`, `focus_cluster(id)`, `set_cadence(topic,interval)`.

**Signals (app→agent):** `cluster_selected(id)`, `basin_renamed(id,name)`, `alert_dismissed(id)`, `cadence_changed(topic)`.

**User interactions:** Click a basin to read its summary; rename a basin to correct the label; set an ingest cadence; dismiss or pin an alert; drag to pan/zoom the terrain.

**The bidirectional loop:** Scheduled scan fires → Harvester ingests 22 preprints, narrates → Cartographer redraws and a new green basin swells → Sentinel ripples it and toasts "emerging: *spatial multi-omics*" → user clicks it, reads the brief, renames the basin → Cartographer re-labels and pins it to the watch rail for future tracking.

**Platform integration:** `br.kb` search + a scheduled **workflow** for cadence; fast route for embedding, deep for basin briefs; `spatial-transcriptomics`/`scientific-research` skills; `figure` plots each basin's papers-per-week trend.

### 30. Keystone

**Concept:** A citation-graph load-bearing analyzer where agents map a claim's full support scaffold and stress-test which papers are structurally critical, letting the user pull citations to watch the argument collapse or hold — a structural-integrity workbench, not a chatbot.

**Domain & vibe:** Research-integrity / citation forensics; the tension of Jenga at scientific scale.

**Theme & aesthetic:** `clinical` pack; clean white, support graph as an architectural truss, load-bearing nodes glow amber, retracted/weak in hazard-stripe; taut spring physics — pull a keystone and dependents sag before failing.

**Layout:** Center: directed support-graph (`network`, gravity toward a top claim) rendered as a truss; Left 240px claim rail (target claims, "map scaffold", integrity score); Right 360px inspector: selected paper's role, what it supports, retraction/quality status; Bottom 56px transport: **Map scaffold**, **Stress test**, **Pull citation**, **Recompute integrity**.

**Agents (multi-agent):** *Architect* builds the support DAG from claim to primary sources via `br.kb.graph`; *Inspector* grades each node's quality/retraction status and marks load-bearing ones; *Demolition* simulates removing a node and reports downstream collapse.

**Agent-driven UI:** Architect draws the truss into `@region:scaffold`; Inspector glows keystones amber and hazard-stripes weak nodes, docking an integrity readout; Demolition animates a sag/collapse on pull and narrates the fallout; presence narrates "Scaffold: 18 nodes, 3 load-bearing, 1 retracted."

**Declared actions:** `map_scaffold(claim)`, `grade_node(id,status)`, `mark_keystone(id)`, `simulate_pull(id,report)`, `recompute_integrity(score)`, `focus_node(id)`.

**Signals (app→agent):** `node_selected(id)`, `citation_pulled(id)`, `claim_selected(id)`, `stress_test_requested()`.

**User interactions:** Click **Map scaffold** on a claim; click a node to see its role; **Pull citation** to yank a paper and watch dependents sag; hover a keystone to preview blast radius; recompute the integrity score.

**The bidirectional loop:** User maps a claim's scaffold → Architect builds an 18-node truss, narrates → Inspector glows 3 keystones amber and hazard-stripes a retracted paper → user pulls the retracted node → Demolition sags its 4 dependents and reports "integrity 0.71→0.38" → user pulls a keystone next, sees the claim collapse, and files the fragile chain for review.

**Platform integration:** `br.kb` graph/page for citation scaffolding + retraction lookup; deep route for collapse simulation; `scientific-research` skill; `figure` plots the integrity score across pull scenarios.

## 4. Data investigation & forensic analytics  ·  #31–40

### 31. Anomaly Atlas

**Concept:** A pivot-table forensics workbench where the agent hunts outliers across a live pivot grid and paints leads directly onto cells — not a chatbot because the primary surface is an interactive multi-dimensional pivot table with agent-driven cell heat and a lead ledger.

**Domain & vibe:** Financial/operational anomaly hunting; tense, procedural, "someone is cooking the numbers."

**Theme & aesthetic:** `terminal` pack; monospace, dense grid, near-zero chrome, amber only on flagged cells, hairline rules, no rounded corners; motion limited to a 200ms cell-flash when a lead lands.

**Layout:** Left 240px rail: dimension shelf (drag fields to rows/cols/measures) + saved cuts; Center: the pivot grid (scrollable, sticky headers, sparkline column); Right 340px inspector: selected-cell drill (contributing rows, z-score, peer band); Bottom 60px transport: model-route toggle (fast/deep), "Hunt" button, lead counter. Floating top-right: presence chip. Lead ledger docks as a collapsible bottom drawer.

**Agents (multi-agent):** *Hunter* scans every pivot cell for z-score/seasonality breaks and proposes leads; *Verifier* re-pivots the underlying rows to reproduce each flagged cell and kills false positives; *Narrator* writes accepted leads into the case-file page (`@region:casefile`) with cause hypotheses.

**Agent-driven UI:** Hunter patches cell backgrounds (amber intensity ∝ severity) and appends lead rows to the ledger; Verifier crosses out refuted leads with a strikethrough; presence narrates "re-pivoting 4,102 rows for cell [West·Q3·Refunds]…".

**Declared actions:** `set_pivot(rows,cols,measure)`, `flag_cell(coord,severity,note)`, `open_drill(coord)`, `strike_lead(id,reason)`, `write_casefile(md)`, `set_route(fast|deep)`.

**Signals (app→agent):** `cell_selected(coord)`, `pivot_changed(spec)`, `lead_pinned(id)`, `lead_dismissed(id)`.

**User interactions:** Drag dimensions to reshape the pivot, click a hot cell to drill, pin/dismiss ledger leads, double-click a cell to demand re-verification.

**The bidirectional loop:** User drags Region→rows, Month→cols → Hunter scans, flags West·Q3·Refunds amber, narrates → Verifier re-pivots, confirms a 6σ spike → user clicks the cell, drill shows three duplicate refund IDs → user pins the lead → Narrator writes it to the case file with a "duplicate-submission" hypothesis and suggests the next cut.

**Platform integration:** model routes (fast scan / deep reproduce), scientific `figure` for the z-distribution, KB to store the case file, `table` + `plot` widgets.

### 32. Trace Weaver

**Concept:** A distributed-trace incident forensics scope where the agent walks a service dependency graph and reconstructs the failure path as a highlighted spanning tree — not a chatbot because the main event is a live force-graph of services + a flamegraph timeline the agent physically drives.

**Domain & vibe:** SRE / log & incident forensics; 3am war-room adrenaline, cold and exact.

**Theme & aesthetic:** `midnight` pack; deep indigo canvas, neon edge glows, tabular side panels, motion = pulsing edges along the traced path; coral reserved for the culprit node.

**Layout:** Left 220px rail: incident list + time-window scrubber; Center top: service force-graph (nodes=services, edges=calls, size=RPS); Center bottom 40%: flamegraph/latency timeline synced to graph selection; Right 360px inspector: span table + log tail for the selected hop; Bottom 56px transport: "Reconstruct", replay-speed slider, blast-radius toggle.

**Agents (multi-agent):** *Pathfinder* traces the request path from the error signature and proposes the failure chain; *Reproducer* replays sampled traces to confirm the latency inflection point; *Scribe* writes the incident timeline + root-cause summary into `@region:postmortem`.

**Agent-driven UI:** Pathfinder highlights the culprit spanning tree edge-by-edge and pins the offending node coral; Reproducer overlays a red latency band on the flamegraph; presence narrates "isolating the p99 inflection at 02:14:31 in auth-svc…".

**Declared actions:** `focus_node(id)`, `trace_path(from,to)`, `highlight_span(traceId,spanId)`, `set_window(start,end)`, `mark_culprit(id,reason)`, `write_postmortem(md)`.

**Signals (app→agent):** `node_selected(id)`, `span_clicked(traceId,spanId)`, `window_scrubbed(range)`, `culprit_confirmed(id)`.

**User interactions:** Scrub the time window, click a service to load its spans, drag to reroute the graph, confirm/reject the proposed culprit, click a span to jump the flamegraph.

**The bidirectional loop:** User scrubs to the error spike → Pathfinder traces gateway→auth→db and glows the chain → Reproducer replays 40 traces, plants a red band on auth-svc → user clicks the auth span, log tail shows a connection-pool timeout → user confirms culprit → Scribe writes the postmortem and proposes a pool-size remediation.

**Platform integration:** `network` force-graph, `figure` flamegraph, model routes (fast trace / deep replay), extensions for log ingestion, KB postmortem archive.

### 33. Causal Court

**Concept:** A causal-DAG explorer where an adversarial pair of agents build and attack a causal graph explaining an anomaly, and the user is the judge who adjudicates edges — not a chatbot because the surface is an editable DAG canvas with a verdict docket, not a message thread.

**Domain & vibe:** Root-cause / causal fraud analytics; courtroom gravity, deliberate and skeptical.

**Theme & aesthetic:** `journal` pack; warm parchment, serif headers, ink-blue edges, hand-drawn node feel; motion = edges fade in as "testimony", struck edges get a red slash.

**Layout:** Left 260px rail: variable palette + evidence list; Center: causal DAG canvas (draggable nodes, directional edges with confounder badges); Right 340px docket: per-edge argument (Prosecutor claim vs Defender rebuttal, effect size, test used); Bottom 64px transport: "Propose DAG", intervention slider (do-operator), "Render verdict".

**Agents (multi-agent):** *Prosecutor* proposes causal edges with supporting tests (correlation, Granger, backdoor adjustment); *Defender* attacks each with confounders, reversed causation, or collider bias; *Registrar* records the surviving DAG and effect estimates into `@region:ruling`.

**Agent-driven UI:** Prosecutor draws candidate edges; Defender annotates contested edges with a confounder badge and files a rebuttal card in the docket; presence narrates "Defender raises collider bias on Promotion→Chargeback…".

**Declared actions:** `add_edge(from,to,test,effect)`, `contest_edge(id,objection)`, `add_confounder(node,between)`, `run_intervention(node,value)`, `settle_edge(id,verdict)`, `write_ruling(md)`.

**Signals (app→agent):** `edge_clicked(id)`, `node_dragged(id,pos)`, `intervention_set(node,value)`, `edge_verdict(id,keep|drop)`.

**User interactions:** Drag nodes, click an edge to read the argument, slide the do-operator to simulate an intervention, gavel each edge keep/drop, request a new confounder search.

**The bidirectional loop:** User asks why chargebacks spiked → Prosecutor draws Promo→Chargeback with a Granger p-value → Defender flags "Holiday" as a confounder and files a rebuttal → user slides do(Promo=0), watches the effect shrink → user rules the edge "drop" → Registrar rewrites the ruling with the adjusted DAG and the true driver.

**Platform integration:** model routes (deep for causal reasoning), `figure` for effect-size forest plots, KB evidence store, skills for causal-inference methods.

### 34. Drift Sentinel

**Concept:** A sensor-drift detective for a stream of instrument channels, where the agent annotates a stacked time-series wall with drift regimes and change-points the user confirms — not a chatbot because the main surface is a multi-channel time-series annotator, not a conversation.

**Domain & vibe:** IoT / lab-instrument sensor forensics; calm vigilance, "the calibration is slipping."

**Theme & aesthetic:** `lab-notebook` pack; graph-paper grid, muted channel colors, gridline density high; motion = annotation brackets that draw on; teal for confirmed regimes, dashed gray for proposed.

**Layout:** Left 200px rail: channel list + drift-score badges; Center: stacked synchronized time-series lanes (shared x-axis, brushable); Right 320px inspector: change-point stats (CUSUM, slope, reference band) for the selected span; Bottom 60px transport: window scrubber, "Scan drift", sensitivity slider.

**Agents (multi-agent):** *Detector* runs change-point/CUSUM across channels and proposes drift spans; *Corroborator* cross-checks correlated channels and environmental covariates to separate real drift from shared events; *Logger* writes confirmed regimes into `@region:calibration-log`.

**Agent-driven UI:** Detector draws dashed brackets over suspect spans and tags each channel's drift badge; Corroborator recolors spurious spans gray and links co-moving channels with a tie-line; presence narrates "cross-checking temp channel — this looks like an HVAC event, not drift…".

**Declared actions:** `annotate_span(channel,range,label)`, `set_regime(channel,range,confirmed)`, `link_channels(ids,reason)`, `set_sensitivity(k)`, `focus_channel(id)`, `write_callog(md)`.

**Signals (app→agent):** `span_brushed(channel,range)`, `annotation_clicked(id)`, `regime_confirmed(id)`, `channel_toggled(id)`.

**User interactions:** Brush a suspicious span, click a bracket to inspect stats, confirm/reject a regime, drag the sensitivity slider, toggle channels on the wall.

**The bidirectional loop:** User brushes a slow rise on pH-3 → Detector marks a CUSUM change-point, draws a dashed bracket → Corroborator checks temp/flow, ties pH-3 to pump-2, narrates "co-moving with pump vibration" → user rejects it as a real event, re-brushes a later flat drift → Detector confirms a calibration slope → Logger writes the regime with recalibration date.

**Platform integration:** `plot` multi-lane, `figure` for CUSUM charts, model routes (fast scan / deep corroborate), extensions for sensor feeds, KB calibration log.

### 35. Ledger Loom

**Concept:** A money-flow fraud explorer where the agent grows a transaction graph outward from a seed account and knits suspicious rings into highlighted subgraphs — not a chatbot because the primary surface is an expanding entity graph with a ring dossier, not chat.

**Domain & vibe:** AML / financial-crime forensics; quiet menace, "follow the money."

**Theme & aesthetic:** `biorouter` pack; clean slate, restrained accents, tabular dossier; motion = new nodes spring in on expansion, ring edges thicken; crimson only on flagged flows.

**Layout:** Left 240px rail: seed accounts + watchlist + typology filters (structuring, layering, mule); Center: transaction force-graph (nodes=accounts, edges=transfers weighted by amount, temporal layout); Right 360px dossier: ring members table + flow timeline + risk score; Bottom 56px transport: "Expand 1 hop", amount threshold slider, "Freeze ring".

**Agents (multi-agent):** *Tracer* expands the graph along money flows and proposes candidate rings by typology; *Auditor* re-walks each ring to verify round-tripping/structuring and prices the exposure; *Filer* drafts a SAR-style narrative into `@region:dossier`.

**Agent-driven UI:** Tracer springs new hop nodes and lassos a candidate ring; Auditor thickens confirmed laundering edges crimson and fills the dossier table; presence narrates "tracing 3 hops from ACC-8841 — detected a 4-node round-trip…".

**Declared actions:** `expand_node(id,hops)`, `highlight_ring(nodeIds,typology)`, `set_threshold(amount)`, `score_ring(id)`, `pin_entity(id)`, `write_dossier(md)`.

**Signals (app→agent):** `node_selected(id)`, `ring_lassoed(nodeIds)`, `entity_pinned(id)`, `threshold_changed(v)`.

**User interactions:** Click an account to expand, lasso a suspicious cluster, raise the amount threshold to prune noise, pin mules, freeze a ring to send to the dossier.

**The bidirectional loop:** User seeds ACC-8841 → Tracer expands 2 hops, lassos a 4-node cycle → Auditor re-walks, confirms funds return within 48h, prices $2.1M, colors edges crimson → user raises threshold to prune small legs, pins two mule accounts → Filer drafts the SAR narrative naming the typology and the pinned mules.

**Platform integration:** `network` force-graph, `table` dossier, model routes (deep for typology reasoning), KB SAR archive, skills for AML typologies.

### 36. Split Verdict

**Concept:** An A/B-test autopsy workbench where a panel of statistician agents dissect why an experiment "won" or "lost" across segments and paint the culprit segments onto a lift heatmap — not a chatbot because the surface is a segment lift heatmap + funnel, not a message log.

**Domain & vibe:** Growth/experimentation forensics; forensic skepticism, "this result is too good to be true."

**Theme & aesthetic:** `clinical` pack; white, precise, blue/red diverging heat, high whitespace; motion = cells fade to their lift color, Simpson's-reversal cells shimmer once.

**Layout:** Left 240px rail: metric + segment-dimension picker + guardrail list; Center: segment×variant lift heatmap (diverging), with a funnel strip below; Right 340px inspector: per-segment CI, sample ratio, p-value, novelty check; Bottom 60px transport: "Autopsy", multiple-comparison correction toggle, "Trust / Distrust".

**Agents (multi-agent):** *Analyst* computes segment lifts and proposes the winning/losing story; *Skeptic* attacks it with SRM checks, peeking, novelty effects, and Simpson's paradox; *Referee* writes the trustworthy verdict into `@region:readout`.

**Agent-driven UI:** Analyst colors the heatmap and marks the top movers; Skeptic flags cells failing SRM with a hazard hatch and shimmers a Simpson's-reversal pair; presence narrates "aggregate lift +8% but reverses in mobile — checking sample ratio…".

**Declared actions:** `set_segmentation(dim)`, `paint_lift(cellMatrix)`, `flag_cell(coord,issue)`, `run_check(name)`, `set_correction(method)`, `write_readout(md)`.

**Signals (app→agent):** `cell_selected(coord)`, `segment_changed(dim)`, `verdict_set(trust|distrust)`, `guardrail_toggled(id)`.

**User interactions:** Switch the segmenting dimension, click a hot cell for its CI, toggle multiple-comparison correction, mark the result trust/distrust, drill a suspicious funnel step.

**The bidirectional loop:** User runs autopsy on "Checkout v2" → Analyst paints +8% aggregate, greenest on desktop → Skeptic finds a sample-ratio mismatch on mobile, hatches those cells, shimmers a Simpson's reversal → user clicks mobile, sees a bot-traffic skew → user marks "distrust" → Referee writes a readout recommending re-randomization and an SRM gate.

**Platform integration:** `figure` for CI forest + funnel, model routes (deep for stats reasoning), skills for experimentation stats, KB experiment readouts.

### 37. Chain of Custody

**Concept:** A supply-chain trace investigator that reconstructs a contaminated/late shipment's provenance across a map + timeline and pins the break-point node — not a chatbot because the main surfaces are a geo map and a synchronized provenance timeline the agent drives together.

**Domain & vibe:** Supply-chain / provenance forensics; investigative, "where did the chain break?"

**Theme & aesthetic:** `biorouter` pack over a muted map; route lines animate along the path; density medium; orange only on the identified break-point leg.

**Layout:** Left 220px rail: lot/shipment list + hop filter (supplier/carrier/warehouse); Center top: Leaflet map with routed legs + facility markers; Center bottom 38%: horizontal provenance timeline (custody handoffs, dwell times, temperature excursions); Right 340px inspector: selected-leg documents, chain-of-custody records, deviation flags; Bottom 56px transport: "Trace lot", playback scrubber, "Mark break".

**Agents (multi-agent):** *Tracker* reconstructs the custody path from records and places map markers + timeline segments; *Inspector* checks each handoff for dwell/temperature/document gaps and proposes the break-point; *Recorder* writes the provenance report into `@region:trace-report`.

**Agent-driven UI:** Tracker draws routed legs and drops facility markers; Inspector colors the offending leg orange and pins a deviation card on the timeline; presence narrates "temperature excursion at Cold-Hub Memphis, 6h over threshold…".

**Declared actions:** `place_marker(facility,coord)`, `draw_leg(from,to,mode)`, `flag_leg(id,deviation)`, `focus_hop(id)`, `set_playback(t)`, `write_report(md)`.

**Signals (app→agent):** `marker_clicked(id)`, `leg_selected(id)`, `timeline_scrubbed(t)`, `break_marked(id)`.

**User interactions:** Click facilities on the map, scrub the timeline, select a leg to read its documents, mark the true break-point, filter by hop type.

**The bidirectional loop:** User traces lot LOT-77 → Tracker draws six legs on the map + timeline → Inspector flags the Memphis cold-hub leg orange for a 6h temp excursion → user scrubs to that dwell, opens the custody doc, sees a missing reefer log → user marks it the break → Recorder writes the report naming the carrier and the CAPA action.

**Platform integration:** `map` (Leaflet) + `plot` timeline, `figure` for temperature trace, model routes (deep for deviation reasoning), extensions for logistics feeds, KB trace reports.

### 38. Cohort Contrast

**Concept:** A cohort-diff forensics table where the agent surfaces which features most separate a "bad" cohort from a matched control and paints the discriminating rows onto a diff heatmap — not a chatbot because the surface is a cohort-diff heatmap + feature ledger, not chat.

**Domain & vibe:** Clinical/product cohort forensics; diagnostic curiosity, "what makes this group different?"

**Theme & aesthetic:** `clinical` pack; sterile white, diverging feature-effect heat, monospaced numerics; motion = rows sort-animate as effect sizes resolve; magenta for the strongest separators.

**Layout:** Left 260px rail: cohort definition builder (case vs control filters, matching keys); Center: feature×cohort diff heatmap, rows = features sorted by separation; Right 340px inspector: per-feature distribution overlay (case vs control), standardized mean diff, confounding check; Bottom 60px transport: "Contrast", matching-method toggle, "Accept separator".

**Agents (multi-agent):** *Prospector* computes standardized differences per feature and proposes top separators; *Matcher* re-runs on a propensity-matched control to kill confounded features; *Author* writes the accepted differentiators into `@region:phenotype`.

**Agent-driven UI:** Prospector sorts and heats the feature rows; Matcher fades features that vanish after matching and re-ranks survivors magenta; presence narrates "age-adjusting the cohort — 'ER visits' survives, 'insurance type' collapses…".

**Declared actions:** `define_cohort(case,control)`, `rank_features(matrix)`, `set_matching(method)`, `flag_confounded(feature)`, `focus_feature(id)`, `write_phenotype(md)`.

**Signals (app→agent):** `feature_selected(id)`, `cohort_edited(spec)`, `matching_changed(method)`, `separator_accepted(id)`.

**User interactions:** Build case/control filters, click a feature to see its distribution overlay, switch matching methods, accept/reject separators, drill a suspicious feature.

**The bidirectional loop:** User defines readmitted vs not → Prospector heats "ER visits", "age", "insurance" as top separators → Matcher propensity-matches on age/sex, collapses "insurance", keeps "ER visits" magenta → user clicks it, sees a clean bimodal split → user accepts the separator → Author writes the phenotype summary with the matched effect size.

**Platform integration:** `figure` for distribution overlays + SMD forest, model routes (deep for matching reasoning), skills for clinical biostatistics, KB phenotype library.

### 39. Log Loom

**Concept:** A raw-log incident forensics scope where the agent clusters millions of log lines into a template timeline and threads the causal storyline the user pins into a case file — not a chatbot because the main surface is a log-template river + entity timeline, not a message pane.

**Domain & vibe:** Security/ops log forensics; hunter's focus, "the signal is buried in a haystack."

**Theme & aesthetic:** `terminal` pack; green-on-black, monospace, ultra-dense, blinking cursor accents; motion = template lanes stream and spike; red only on the anomalous burst.

**Layout:** Left 220px rail: log-template list (Drain-clustered) with frequency bars; Center top: template river (stacked count-over-time lanes); Center bottom 40%: entity/session timeline of correlated events; Right 340px inspector: raw lines for the selected template + regex-extracted fields; Bottom 56px transport: time-window scrubber, "Cluster & hunt", severity filter.

**Agents (multi-agent):** *Clusterer* templatizes raw logs and surfaces rare/spiking templates; *Correlator* stitches templates across hosts/sessions into a candidate attack/incident storyline; *Narrator* writes the ordered timeline into `@region:case`.

**Agent-driven UI:** Clusterer paints template lanes and reddens anomalous bursts; Correlator draws thread lines linking related templates on the entity timeline; presence narrates "spike in template T-431 'auth failure' correlates with T-88 'privilege escalation' on host db-02…".

**Declared actions:** `cluster_logs(window)`, `highlight_template(id,severity)`, `thread_events(templateIds)`, `set_window(range)`, `extract_fields(templateId,regex)`, `write_case(md)`.

**Signals (app→agent):** `template_selected(id)`, `window_scrubbed(range)`, `event_pinned(id)`, `thread_confirmed(id)`.

**User interactions:** Scrub the window, click a template lane to read raw lines, pin events onto the storyline, confirm/reject a stitched thread, filter by severity.

**The bidirectional loop:** User scrubs to a login-failure spike → Clusterer reddens template T-431 → Correlator threads it to a privilege-escalation template on db-02 and narrates the sequence → user clicks T-431, reads raw lines, pins the escalation event → Correlator extends the thread to a data-exfil template → Narrator writes the ordered incident timeline with hosts and IOCs.

**Platform integration:** `plot` template river, `log` widget, model routes (fast cluster / deep correlate), extensions for log sources, KB incident cases.

### 40. Recon Board

**Concept:** A free-form investigation corkboard where the agent pins evidence cards, strings red-thread connections between them, and grows the theory-of-the-case the user rearranges — not a chatbot because the primary surface is a spatial evidence canvas with author-drawn threads, not a transcript.

**Domain & vibe:** Investigative / open-ended fraud & OSINT forensics; detective obsession, "the wall of clues."

**Theme & aesthetic:** `journal` pack; cork texture, pinned index cards, handwritten labels, red yarn threads; motion = yarn animates taut between pinned cards; a single spotlight on the current lead.

**Layout:** Left 240px rail: evidence inbox (documents, transactions, entities awaiting placement); Center: pannable/zoomable corkboard canvas (draggable cards, drawn threads, cluster frames); Right 340px inspector: selected-card source + provenance + confidence; Bottom 60px transport: "Pursue lead", thread-mode toggle, "Freeze theory".

**Agents (multi-agent):** *Investigator* pulls evidence from sources and pins candidate cards; *Connector* proposes red-thread links (shared entity, timing, money path) between cards; *Skeptic* challenges weak threads and flags coincidences; *Chronicler* writes the theory-of-the-case into `@region:theory`.

**Agent-driven UI:** Investigator pins new cards into cluster frames; Connector draws animated yarn between related cards with a labeled link; Skeptic dims weak threads and tags them "coincidence?"; presence narrates "pinning shell-company filing — same registered agent as card #7, drawing a thread…".

**Declared actions:** `pin_card(evidence,pos)`, `draw_thread(a,b,relation)`, `frame_cluster(cardIds,label)`, `challenge_thread(id,reason)`, `spotlight(cardId)`, `write_theory(md)`.

**Signals (app→agent):** `card_dragged(id,pos)`, `thread_clicked(id)`, `cards_lassoed(ids)`, `card_pinned_by_user(evidence)`.

**User interactions:** Drag cards to arrange, draw your own threads, lasso cards into a cluster, click a thread to read its rationale, freeze the current theory for review.

**The bidirectional loop:** User drops a suspicious invoice into the board → Investigator pins related shell-company cards → Connector strings yarn on a shared registered agent → Skeptic challenges one thread as a common-address coincidence → user drags two cards into a cluster and draws a money-path thread → Chronicler writes the theory naming the shell network and the weak link to verify next.

**Platform integration:** `canvas` author-drawn surface, `network` for entity links, model routes (deep for link reasoning), extensions/skills for OSINT + AML, KB theory-of-the-case archive.

## 5. Geospatial & field-ops planners  ·  #41–50

### 41. ContagionScope

**Concept:** A time-slider choropleth war-room where agents model and narrate an outbreak's spread across counties while you redraw containment zones by hand — not a chatbot, the map is the argument.

**Domain & vibe:** Field epidemiology; tense, clinical urgency with a sense of a ticking clock.

**Theme & aesthetic:** `clinical` pack; IBM Plex Sans, dense data readouts, near-zero chrome, coral reserved for active-transmission counties and the live playhead. Motion: choropleth cross-fades between weeks, markers pulse once when placed.

**Layout:** Left 260px rail: scenario list + intervention deck (cards you drag onto the map). Center: full-bleed Leaflet choropleth of counties. Right 340px inspector: selected-county R₀, case curve `figure`, agent rationale log. Bottom 64px transport bar: week time-slider + play/scrub, "Run Projection" button (kicks the forecast), "Compare Scenarios" toggle. Floating top-right: presence chip. **Reset Zones** button sits top-left of the map.

**Agents (multi-agent):** *Modeler* runs the SEIR projection week-by-week and repaints the choropleth; *Epidemiologist* interprets hotspots and drafts containment recommendations; *Skeptic* stress-tests each recommendation against under-reporting and mobility assumptions, flagging weak ones.

**Agent-driven UI:** Modeler patches county fills each simulated week and drops `place_marker` outbreak seeds; Epidemiologist writes ranked recommendations into `@region:inspector`; presence narrates "Week 6: superspread in Alameda, R₀ 2.3 — Skeptic disputes travel data."

**Declared actions:** `set_week(n)`, `paint_choropleth(metric)`, `place_marker(county,label)`, `run_projection(weeks)`, `highlight_counties(ids)`, `annotate(county,text)`.

**Signals (app→agent):** `zone_drawn(polygon)`, `county_selected(id)`, `intervention_dropped(card,county)`, `week_scrubbed(n)`.

**User interactions:** Lasso a containment cordon; drag a "mask mandate" or "travel ban" card onto counties; scrub weeks; click a county to inspect.

**The bidirectional loop:** You lasso three counties as a cordon → Modeler re-runs SEIR inside/outside, Skeptic consults on leakage → agents repaint spread and narrate "cordon cuts peak 40%, but border porosity underestimated" → you drag a travel-ban card onto the seam → Epidemiologist revises and re-ranks.

**Platform integration:** Auto-Visualiser `figure` for case curves; KB of prior outbreak parameters; deep model route for projections; fast route for narration.

### 42. ExpeditionForge

**Concept:** A layered-overlay expedition planner where agents assemble a field campaign — sites, routes, permits, weather windows — onto a topo map you sculpt with drawn regions and dragged waypoints; the plan is a living map, not a transcript.

**Domain & vibe:** Remote field research / expedition logistics; adventurous, meticulous, a little high-stakes.

**Theme & aesthetic:** `lab-notebook` pack; Söhne + monospace coordinates, gridded paper texture, hand-annotation feel, ink-blue accents with amber for hazard overlays. Motion: overlays slide in like tracing sheets.

**Layout:** Left 280px rail: layer stack (terrain/weather/hazard/permits toggles) + sample-site checklist. Center: Leaflet topo with drawable regions and draggable waypoints. Right 340px inspector: selected-site logistics card (gear, personnel, elevation profile `figure`). Bottom 64px: day-by-day itinerary timeline. Floating top-right: presence chip + **Validate Plan** button (agents audit feasibility).

**Agents (multi-agent):** *Cartographer* places candidate sampling sites and draws access routes; *Logistician* computes gear/porter/fuel loads and daily distances; *Risk Officer* overlays hazards (crevasse, flood, altitude) and vetoes unsafe legs, handing corrections back to Cartographer.

**Agent-driven UI:** Cartographer patches waypoints and route polylines; Logistician fills the itinerary timeline and inspector logistics; Risk Officer paints hazard overlays and highlights vetoed segments red; presence narrates "Rerouting day 4 around glacier melt zone."

**Declared actions:** `place_marker(site,label)`, `draw_route(waypoints)`, `toggle_layer(name)`, `set_itinerary(days)`, `flag_hazard(span,reason)`, `stage_layout(preset)`.

**Signals (app→agent):** `waypoint_dragged(id,latlng)`, `region_drawn(polygon)`, `site_selected(id)`, `layer_toggled(name)`.

**User interactions:** Drag waypoints along a valley; draw a study region; toggle the weather layer; reorder itinerary days.

**The bidirectional loop:** You draw a study region on a ridge → Cartographer proposes four sample sites, Logistician sizes the load → Risk Officer flags an avalanche corridor red and consults Cartographer → agents reroute and narrate the trade → you drag day-3 camp downhill → Logistician recomputes distances and updates the timeline.

**Platform integration:** KB of terrain/permit rules; elevation `figure`; deep route for routing; workflows for permit checklists.

### 43. RelayOptimizer

**Concept:** A route-optimizer canvas where agents design and continuously re-solve a multi-vehicle delivery network while you drag stops, forbid roads, and pin priorities directly on the map — an operations board, not a chat.

**Domain & vibe:** Last-mile logistics / fleet routing; brisk, competitive, dispatch-room energy.

**Theme & aesthetic:** `terminal` pack; JetBrains Mono, dark slate, high-contrast route colors per vehicle, green-on-black KPI ticker. Motion: routes animate as they re-solve; a solve pulse sweeps the map.

**Layout:** Left 240px rail: vehicle roster + constraint chips (time windows, capacity, no-go zones). Center: Leaflet with color-coded routes and draggable stops. Right 320px inspector: per-vehicle manifest, cost `kpi` cluster, utilization `plot`. Bottom 64px transport bar: **Solve**, **Lock Route**, objective toggle (cost/time/emissions). Floating top-right: presence chip.

**Agents (multi-agent):** *Router* solves the VRP and draws routes; *Cost Analyst* scores each solution on the chosen objective and updates KPIs; *Dispatcher* enforces human constraints (locked routes, driver breaks) and adversarially checks the Router's solution for infeasibility.

**Agent-driven UI:** Router patches route polylines and stop assignments; Cost Analyst repaints the KPI cluster and utilization plot; Dispatcher highlights violated constraints amber; presence narrates "Re-solved: 7 stops moved off Truck 3, cost −11%."

**Declared actions:** `solve_routes(objective)`, `assign_stop(stop,vehicle)`, `draw_route(vehicle,path)`, `lock_route(vehicle)`, `forbid_edge(polyline)`, `focus_vehicle(id)`.

**Signals (app→agent):** `stop_dragged(id,latlng)`, `route_locked(vehicle)`, `edge_drawn(polyline)`, `constraint_toggled(chip)`.

**User interactions:** Drag a stop to another cluster; draw a forbidden road segment; lock a driver's route; toggle the objective.

**The bidirectional loop:** You drag a priority stop earlier → Router re-solves, Dispatcher checks time windows → agents redraw two routes and narrate "Truck 2 now breaches window at stop 9" → Cost Analyst consults and proposes a swap → you lock Truck 1 and hit Solve → agents re-optimize the rest around the lock.

**Platform integration:** Fast model route for rapid re-solves; deep route for full network optimization; KPI/plot widgets; skills for VRP heuristics.

### 44. IsoReach

**Concept:** An isochrone-planning workbench where agents compute and compare travel-time reachability polygons to site a new clinic or depot while you drop candidate locations and adjust mode/time budgets — the reachable-area map is the decision surface, not a conversation.

**Domain & vibe:** Urban planning / access equity; deliberate, civic, quietly optimistic.

**Theme & aesthetic:** `journal` pack; Freight Text headings + clean sans body, generous whitespace, soft teal isochrone bands, warm coral for gaps in coverage. Motion: isochrone rings bloom outward on compute.

**Layout:** Left 260px rail: candidate-site list + mode selector (walk/transit/drive) + time-budget sliders (5/10/15 min). Center: Leaflet with translucent isochrone bands and coverage choropleth. Right 340px inspector: population-reached `kpi`, equity breakdown `figure`, gap list. Bottom 64px: scenario comparison strip (A/B/C thumbnails). Floating top-right: presence chip + **Score Coverage** button.

**Agents (multi-agent):** *Reachability Engine* computes isochrones per candidate and paints bands; *Equity Auditor* overlays demographics and quantifies who is left out; *Planner* proposes the best-scoring site set and consults the Auditor to refine against under-served groups.

**Agent-driven UI:** Engine patches isochrone bands and coverage choropleth; Auditor writes the equity breakdown and highlights coverage gaps coral; Planner ranks scenarios in the comparison strip; presence narrates "Site B covers +18k residents but misses the east transit desert."

**Declared actions:** `compute_isochrone(site,mode,minutes)`, `paint_coverage(metric)`, `place_marker(site)`, `highlight_gaps(regions)`, `score_scenario(id)`, `compare_scenarios(ids)`.

**Signals (app→agent):** `site_placed(latlng)`, `budget_changed(minutes)`, `mode_selected(mode)`, `gap_clicked(region)`.

**User interactions:** Click to drop a candidate site; slide the time budget; switch travel mode; click a coverage gap to interrogate it.

**The bidirectional loop:** You drop a candidate clinic → Engine blooms its 15-min transit isochrone, Auditor overlays demographics → agents highlight an eastern gap coral and narrate the shortfall → you slide the budget to 20 min → Engine recomputes, Planner consults Auditor and proposes a second complementary site → you compare A vs A+B in the strip.

**Platform integration:** KB of census/transit data; equity `figure`; deep route for isochrone scoring; workflows for siting reports.

### 45. StormWatch

**Concept:** A disaster-response common operating picture where agents triage incidents, stage resources, and re-task crews across a live hazard map while you draw evacuation zones and assign teams by dragging — an incident-command board, not a chat window.

**Domain & vibe:** Emergency management / disaster response; adrenal, coordinated, high-tempo.

**Theme & aesthetic:** `midnight` pack; Inter tight, dark command-console look, amber/red hazard gradients, cyan for friendly assets. Motion: incident pins drop with a shockwave ping; reassigned crews trail a motion line.

**Layout:** Left 260px rail: incident queue (sortable by severity) + resource roster (crews, shelters, supplies). Center: Leaflet hazard map with drawable evac zones, incident pins, asset icons. Right 340px inspector: selected-incident dossier + crew ETA `kpi` + situation `log`. Bottom 64px transport bar: clock, **Broadcast Order**, **Auto-Stage** button. Floating top-right: presence chip.

**Agents (multi-agent):** *Triage Officer* ranks incoming incidents by severity and life-safety; *Resource Coordinator* stages and re-tasks crews/shelters to incidents; *Safety Marshal* checks routes and evac zones against the hazard footprint and vetoes crew paths through danger.

**Agent-driven UI:** Triage repaints the incident queue and drops pins; Coordinator draws crew-assignment lines and updates ETA KPIs; Safety Marshal shades unsafe corridors red and reroutes; presence narrates "Reassigning Engine 4 — its route crosses the flood crest."

**Declared actions:** `place_marker(incident,severity)`, `assign_crew(crew,incident)`, `draw_evac_zone(polygon)`, `stage_resource(id,location)`, `flag_hazard(span,reason)`, `broadcast(order)`.

**Signals (app→agent):** `evac_zone_drawn(polygon)`, `crew_dragged(crew,incident)`, `incident_selected(id)`, `shelter_placed(latlng)`.

**User interactions:** Draw an evac polygon; drag a crew onto an incident; place a shelter marker; click an incident to open its dossier.

**The bidirectional loop:** You draw an evac zone around a neighborhood → Triage re-ranks trapped-resident incidents, Coordinator stages two crews → Safety Marshal flags one route through the flood crest red and consults Coordinator → agents reroute and narrate → you drag a shelter marker inland → Coordinator recomputes ETAs and updates the queue.

**Platform integration:** KB of resource inventories + hazard feeds; ETA KPIs; fast route for triage; deep route for staging optimization; workflows for order broadcasts.

### 46. BioSentinel

**Concept:** An environmental-monitoring overlay map where agents fuse sensor telemetry, detect anomalies, and forecast plume drift while you draw sampling transects and flag suspect stations — a live monitoring console, not a chatbot.

**Domain & vibe:** Environmental science / pollution surveillance; watchful, investigative, faintly ominous.

**Theme & aesthetic:** `biorouter` pack; Söhne + tabular numerals, cool grey-green base, a spectral heat ramp for concentrations, magenta for anomaly flags. Motion: plume contours advect smoothly across the frame; anomalous sensors flicker.

**Layout:** Left 260px rail: sensor-station list (status dots) + pollutant selector + threshold sliders. Center: Leaflet with interpolated concentration heat layer, station markers, drawable transects. Right 340px inspector: station time-series `figure`, anomaly reasoning, plume forecast `kpi`. Bottom 64px: time-slider over the telemetry window. Floating top-right: presence chip + **Forecast Drift** button.

**Agents (multi-agent):** *Sensor Analyst* ingests telemetry and paints the interpolated concentration field; *Anomaly Hunter* detects out-of-band readings and flags stations; *Dispersion Modeler* forecasts plume drift under wind and hands suspected sources back to the Analyst to confirm.

**Agent-driven UI:** Analyst patches the heat layer and station dots; Anomaly Hunter flags stations magenta and writes findings to the inspector; Dispersion Modeler draws forecast plume contours; presence narrates "PM2.5 spike at Station 12 — modeling upwind source near the rail yard."

**Declared actions:** `paint_heatfield(pollutant)`, `flag_station(id,reason)`, `forecast_plume(hours)`, `draw_contour(polygon)`, `set_window(range)`, `place_marker(source,label)`.

**Signals (app→agent):** `transect_drawn(line)`, `station_selected(id)`, `threshold_changed(value)`, `window_scrubbed(range)`.

**User interactions:** Draw a sampling transect; click a flagged station; drag the threshold slider; scrub the telemetry timeline.

**The bidirectional loop:** You draw a transect through a suspect corridor → Analyst interpolates along it, Anomaly Hunter flags two stations magenta → agents narrate the exceedance → Dispersion Modeler forecasts drift and drops a candidate-source marker, consulting the Analyst → you scrub back six hours → agents re-detect onset and refine the source estimate.

**Platform integration:** KB of sensor metadata + regulatory thresholds; time-series `figure`; deep route for dispersion modeling; scientific figures for exceedance charts.

### 47. TerraFuture

**Concept:** An urban-planning what-if sandbox where agents simulate the knock-on effects of zoning and infrastructure edits across a city while you paint land-use parcels and draw new transit lines — a scenario map, not a chat log.

**Domain & vibe:** Urban design / policy simulation; imaginative, argumentative, civic-tech optimism.

**Theme & aesthetic:** `journal` pack; GT Sectra headings, editorial layout, muted parcel palette by land-use class, coral for stressed infrastructure. Motion: parcels recolor with a soft wipe when re-zoned; ripple effects propagate outward.

**Layout:** Left 280px rail: land-use palette (residential/commercial/park/transit brushes) + policy levers (density, parking min). Center: Leaflet parcel choropleth with painting + line-drawing. Right 340px inspector: impact dashboard (traffic/tax/greenspace `kpi` + jobs `figure`). Bottom 64px: scenario timeline (baseline → year 5 → year 10). Floating top-right: presence chip + **Simulate Impacts** button.

**Agents (multi-agent):** *Zoning Simulator* propagates land-use changes into population/traffic/tax models and repaints impacts; *Transit Planner* proposes and draws transit lines to serve new density; *Community Advocate* critiques displacement and greenspace loss, handing objections back to the Simulator to reconcile.

**Agent-driven UI:** Simulator repaints stressed-infrastructure parcels coral and updates the impact KPIs; Transit Planner draws proposed lines; Advocate annotates displacement risk in the inspector; presence narrates "Upzoning the corridor adds 4k units but overloads the north junction."

**Declared actions:** `rezone_parcels(ids,class)`, `draw_transit(path)`, `simulate_impacts(years)`, `paint_stress(metric)`, `annotate(parcel,text)`, `set_scenario_year(n)`.

**Signals (app→agent):** `parcel_painted(ids,class)`, `line_drawn(path)`, `lever_changed(name,value)`, `parcel_selected(id)`.

**User interactions:** Paint parcels with a land-use brush; draw a new light-rail line; pull the density lever; scrub the scenario year.

**The bidirectional loop:** You paint a corridor commercial → Zoning Simulator projects traffic, flags the north junction coral → Advocate objects on displacement and consults the Simulator → Transit Planner draws a relief bus line and narrates → you scrub to year 10 → agents re-simulate cumulative impacts and update the dashboard.

**Platform integration:** KB of parcel/zoning + demographic data; jobs/traffic `figure`; deep route for impact simulation; workflows for scenario reports.

### 48. TrailWeave

**Concept:** A backcountry route-planning canvas where agents braid together trail segments, water sources, and campsites into a multi-day thru-hike while you drag the line and set daily mileage caps — the route itself is the interface, not a chat thread.

**Domain & vibe:** Outdoor recreation / thru-hike logistics; calm, aspirational, trail-worn.

**Theme & aesthetic:** `lab-notebook` pack; Caslon-ish headings + mono elevations, kraft-paper base, forest-green trail ink, orange for resupply/water-critical flags. Motion: the route draws itself segment by segment; campsite pins settle with a gentle bounce.

**Layout:** Left 260px rail: segment library + daily-mileage cap slider + resupply toggle. Center: Leaflet topo with the draggable route polyline, water/campsite markers. Right 340px inspector: elevation profile `figure`, daily-load `kpi`, water-carry warnings. Bottom 64px: day-strip (D1…Dn with mileage + gain). Floating top-right: presence chip + **Balance Days** button.

**Agents (multi-agent):** *Route Weaver* stitches trail segments into a continuous line honoring mileage caps; *Provisioner* places water sources, campsites, and resupply points; *Conditions Scout* checks seasonal snow, fire closures, and water reliability, vetoing risky segments back to the Weaver.

**Agent-driven UI:** Weaver patches the route polyline and day boundaries; Provisioner drops water/campsite/resupply markers and fills daily-load KPIs; Scout flags closed or dry segments orange; presence narrates "Day 5 is a 22-mile dry stretch — inserting a cache at mile 61."

**Declared actions:** `weave_route(segments)`, `place_marker(type,latlng)`, `balance_days(cap)`, `flag_segment(span,reason)`, `set_resupply(point)`, `focus_day(n)`.

**Signals (app→agent):** `route_dragged(handle,latlng)`, `mileage_cap_changed(miles)`, `marker_moved(id,latlng)`, `day_selected(n)`.

**User interactions:** Drag the route line onto a preferred ridge; set the daily-mileage cap; move a campsite marker; click a day to inspect its profile.

**The bidirectional loop:** You drag the line over a scenic ridge → Weaver re-splits days, Provisioner reseats campsites → Scout flags a snow-covered pass orange and consults the Weaver → agents reroute lower and narrate → you lower the mileage cap to 15 → Route Weaver adds a day and Balance Days redistributes gain.

**Platform integration:** KB of trail/water/closure data; elevation `figure`; deep route for day-balancing; skills for backcountry logistics.

### 49. VectorFront

**Concept:** A vector-borne-disease spread map where agents couple mosquito habitat, climate, and case reports to project transmission fronts while you deploy interventions and draw surveillance grids — an epidemiological control room, not a chatbot.

**Domain & vibe:** Global health / vector ecology; grave, data-rich, field-driven.

**Theme & aesthetic:** `clinical` pack; Plex Sans + tabular density, muted sand base, a habitat-suitability green-to-red ramp, violet for intervention zones. Motion: transmission fronts advance frame-by-frame with the time-slider; intervention zones fade in.

**Layout:** Left 260px rail: intervention deck (spray, bednets, larvicide) + climate-layer toggles (rainfall, temp). Center: Leaflet suitability choropleth with drawable surveillance grids + case pins. Right 340px inspector: transmission curve `figure`, intervention efficacy `kpi`, agent rationale. Bottom 64px transport bar: month time-slider + **Project Front** button. Floating top-right: presence chip.

**Agents (multi-agent):** *Ecologist* maps habitat suitability from climate layers; *Transmission Modeler* projects case fronts across suitable terrain; *Intervention Strategist* places control measures and consults the Modeler to estimate averted cases, while a *Skeptic* pass challenges surveillance blind spots.

**Agent-driven UI:** Ecologist paints the suitability choropleth; Modeler advances the transmission front and drops case pins; Strategist shades intervention zones violet and fills efficacy KPIs; presence narrates "Post-rains suitability surges in the delta — front reaches the district in ~3 weeks."

**Declared actions:** `paint_suitability(month)`, `project_front(weeks)`, `deploy_intervention(type,zone)`, `place_marker(case,label)`, `draw_grid(polygon)`, `set_month(n)`.

**Signals (app→agent):** `grid_drawn(polygon)`, `intervention_dropped(type,zone)`, `month_scrubbed(n)`, `case_pin_clicked(id)`.

**User interactions:** Draw a surveillance grid; drag a larvicide intervention onto a zone; scrub the month; click a case pin.

**The bidirectional loop:** You draw a surveillance grid over a delta → Ecologist paints rising suitability, Modeler projects the front → agents narrate the ~3-week arrival → you drag bednets + larvicide onto the corridor → Strategist consults the Modeler on averted cases, Skeptic flags an unmonitored upstream village → agents extend the grid and re-project.

**Platform integration:** KB of vector ecology + climate reanalysis; transmission `figure`; deep route for coupled modeling; workflows for intervention costing.

### 50. GridHarbor

**Concept:** A coastal resilience planner where agents model sea-level rise, storm surge, and flood exposure across a harbor city while you draw seawalls, elevate parcels, and reroute critical infrastructure — a resilience map, not a chat.

**Domain & vibe:** Climate adaptation / coastal engineering; sober, long-horizon, resolute.

**Theme & aesthetic:** `midnight` pack; Inter + condensed labels, deep-navy base, a bathymetric blue-to-white inundation ramp, amber for at-risk critical assets. Motion: inundation floods in as you raise the sea-level slider; defended parcels dry with a receding wipe.

**Layout:** Left 260px rail: adaptation toolkit (seawall, levee, elevate, managed-retreat brushes) + asset-criticality filter. Center: Leaflet inundation map with drawable defenses + critical-asset markers. Right 340px inspector: exposure `kpi`, benefit-cost `figure`, agent rationale. Bottom 64px: sea-level-rise slider (0–2m) + surge-event selector. Floating top-right: presence chip + **Assess Resilience** button.

**Agents (multi-agent):** *Flood Modeler* computes inundation for the current SLR + surge and paints exposure; *Infrastructure Analyst* identifies threatened hospitals/substations and scores criticality; *Adaptation Economist* prices defenses and their benefit-cost, adversarially challenged by a *Resilience Critic* on residual and long-tail risk.

**Agent-driven UI:** Flood Modeler paints the inundation layer and shades at-risk parcels amber; Infrastructure Analyst pins threatened assets; Economist fills benefit-cost figures; Critic annotates residual-risk warnings; presence narrates "A 1.5m seawall protects the substation but shifts flooding to the south marsh."

**Declared actions:** `paint_inundation(slr,surge)`, `draw_defense(type,path)`, `elevate_parcels(ids,meters)`, `flag_asset(id,reason)`, `score_benefit_cost(scenario)`, `set_slr(meters)`.

**Signals (app→agent):** `defense_drawn(type,path)`, `parcel_selected(ids)`, `slr_changed(meters)`, `surge_selected(event)`.

**User interactions:** Draw a seawall along the waterfront; brush managed-retreat over a floodplain; raise the SLR slider; select a surge event.

**The bidirectional loop:** You draw a seawall along the harbor → Flood Modeler recomputes inundation, Infrastructure Analyst clears the substation amber flag → Resilience Critic warns the wall displaces flooding south and consults the Modeler → agents repaint the marsh at-risk → you brush managed-retreat there → Economist rescores benefit-cost and updates the figure.

**Platform integration:** KB of DEM/bathymetry + asset registries; benefit-cost `figure`; deep route for flood modeling; workflows for adaptation costing.

## 6. Creative & design studios  ·  #51–60

### 51. Gridwright — Editorial Layout Studio

**Concept:** A generative-layout studio where agents propose and place editorial page compositions on a live baseline grid; you nudge, lock, and redirect frames by hand — it is a page-design workbench, not a chatbot.

**Domain & vibe:** Magazine/editorial print design; disciplined, typographically precise, quietly obsessive.

**Theme & aesthetic:** `journal` theme pack; serif display + mono captions, high density, ink-on-cream, hairline column guides; motion is snap-to-grid with soft settle; color only on selected frames and constraint violations (amber).

**Layout:** Left 240px rail: content inventory (headline, deck, body runs, images) as draggable chips; Center: infinite `canvas` spread showing facing pages with a toggleable 12-col baseline grid; Right 340px inspector: type scale, leading, measure, and a "constraint log" (`log` widget); Bottom 64px transport: page selector + **Generate Spread**, **Lock Frame**, **Rebalance** buttons; floating top-right: presence chip.

**Agents (multi-agent):** *Director* plans the reading hierarchy and grid regions; *Compositor* places each content chip into frames via `ui_patch`; *Proofer* critiques rag, widows/orphans, and contrast, flagging into the constraint log and sending fixes back to Compositor.

**Agent-driven UI:** Agents paint frames onto `@region:spread`, highlight the active column span, redline violations, and patch the inspector's type scale; presence narrates "Compositor placing pull-quote across cols 7-9."

**Declared actions:** `generate_spread(pages)`, `place_frame(chipId,colSpan,row)`, `lock_frame(id)`, `set_type(frameId,{size,leading})`, `rebalance(scope)`, `redline(frameId,note)`.

**Signals (app→agent):** `frame_moved(id,cell)`, `frame_locked(id)`, `chip_dropped(chipId,cell)`, `measure_dragged(cols)`.

**User interactions:** Drag chips from the rail onto columns, drag frame edges across the grid, click Lock to pin, drag the measure handle to reset column width.

**The bidirectional loop:** User drops a hero image spanning cols 1-6 → Director re-plans hierarchy and consults Proofer on contrast → Compositor patches the deck below and narrows body to cols 8-12, narrating each move → Proofer redlines an orphaned line in amber → user drags the frame taller → Compositor reflows and clears the redline.

**Platform integration:** model routes (deep for planning, fast for reflow), scientific `figure` widget for embedded data-graphics frames, KB for a house style guide, skills: `taste-skill`/`frontend-design`.

### 52. Palette Séance — Brand Moodboard System

**Concept:** A moodboard/brand-system studio on an infinite canvas where agents assemble swatches, type specimens, and imagery clusters into a coherent identity; you rearrange and lock clusters to steer the system — a curation surface, not a chat.

**Domain & vibe:** Brand strategy & visual identity; exploratory, seductive, decisive.

**Theme & aesthetic:** `midnight` theme pack; large imagery tiles, thin sans labels, low chrome, generous negative space; motion is drifting parallax as clusters settle; saturated color reserved for the "locked palette" strip.

**Layout:** Left 220px rail: brand brief + attribute sliders (warm↔cool, playful↔austere); Center: infinite `canvas` of moodboard clusters; Right 320px inspector: extracted palette (`kpi` chips of hex + WCAG), type pairings, `table` of asset provenance; Bottom 56px: **Summon Board**, **Extract Palette**, **Lock Cluster**; floating top-right presence chip.

**Agents (multi-agent):** *Curator* gathers image/type/texture candidates into thematic clusters; *Colorist* derives and harmonizes the palette, enforcing contrast; *Skeptic* critiques cliché and coherence, pruning off-brand tiles and consulting Curator for replacements.

**Agent-driven UI:** Agents place clustered tiles on `@region:board`, patch the palette strip in the inspector, glow-outline the cluster under revision, and narrate "Colorist re-keying to a cooler accent for AA contrast."

**Declared actions:** `summon_board(themeSeed)`, `cluster(tileIds,label)`, `extract_palette(scope)`, `harmonize(target)`, `prune_tile(id)`, `lock_cluster(id)`.

**Signals (app→agent):** `tile_moved(id,pos)`, `cluster_locked(id)`, `slider_changed(attr,val)`, `tile_pinned(id)`.

**User interactions:** Drag tiles between clusters, pinch-zoom the canvas, drag attribute sliders, click Lock Cluster to freeze a direction, double-click a swatch to pin it.

**The bidirectional loop:** User drags the warm↔cool slider cool and pins a teal swatch → Curator refetches imagery, Colorist re-derives a palette around teal → Skeptic flags two tiles as stocky cliché and consults Curator for edgier replacements → agents patch the board and palette strip, narrating changes → user locks the resulting cluster → Skeptic freezes it and rebalances neighbors.

**Platform integration:** KB (`br.kb` brand archives + past decks), model routes (deep for critique), scientific `figure` for contrast/harmony charts, skills: `frontend-design`, extensions for image sourcing.

### 53. Panelith — Storyboard & Comic Board

**Concept:** A storyboard/comic-board studio where agents block out panels, camera framing, and beat pacing across a timeline of pages; you re-order panels, resize gutters, and redirect the scene — a directing board, not a chatbot.

**Domain & vibe:** Sequential art / film previz; kinetic, cinematic, collaborative-writers-room energy.

**Theme & aesthetic:** `lab-notebook` theme pack recolored dark; bold panel borders, hand-lettered caption font, medium density; motion is filmstrip scrubbing with panel pop-in; color-coded beat tags (setup/turn/payoff).

**Layout:** Left 240px rail: script beats as reorderable cards; Center: page `canvas` of panels in a resizable gutter grid; Right 340px inspector: shot list `table` (angle, lens, subject) + continuity `log`; Bottom 72px timeline transport: page filmstrip + **Block Scene**, **Split Panel**, **Retime**; floating top-right presence chip.

**Agents (multi-agent):** *Screenwright* segments the script into beats and shots; *Framer* draws panel thumbnails and camera framing via `ui_patch`; *Continuity Critic* checks eyeline, prop, and 180-degree consistency, flagging and handing corrections to Framer.

**Agent-driven UI:** Agents render panel thumbs into `@region:page`, patch the shot-list table, highlight the panel under edit, and drop continuity flags; presence narrates "Framer widening to an establishing shot for beat 3."

**Declared actions:** `block_scene(beatIds)`, `draw_panel(beatId,{angle,lens})`, `split_panel(id)`, `retime(pageId,pacing)`, `flag_continuity(panelId,note)`, `reframe(panelId,shot)`.

**Signals (app→agent):** `panel_reordered(id,idx)`, `gutter_resized(pageId,dims)`, `panel_selected(id)`, `beat_dropped(beatId,slot)`.

**User interactions:** Drag panels to reorder, drag gutter handles to resize, scrub the filmstrip, drag beat cards onto pages, click a flag to accept a continuity fix.

**The bidirectional loop:** User reorders panels 4 and 5 → Screenwright re-derives the beat flow and consults Continuity Critic → Critic warns the reversal breaks eyeline and hands a reframe note → Framer flips the camera and repaints both panels, narrating why → user resizes the gutter to emphasize the payoff → Framer re-crops and Critic clears the flag.

**Platform integration:** model routes (deep for beat logic, fast for reframes), scientific `figure` for pacing/tension curves, skills: `taste-skill`, workflows for export to a shot-list doc.

### 54. Reverb Loom — Soundscape Arranger

**Concept:** A soundscape/music arranger where agents lay stems, textures, and automation lanes onto a timeline; you drag clips, mute lanes, and redirect the mood — a multitrack arranging surface, not a chat window.

**Domain & vibe:** Ambient/score composition; immersive, moody, flow-state.

**Theme & aesthetic:** `midnight` theme pack; waveform-forward, neon lane accents, low chrome, dark canvas; motion is smooth playhead glide and clip-drop ripples; color per stem family (pads/perc/field/melody).

**Layout:** Left 220px rail: sound library (stems/textures) as audition chips; Center: multitrack timeline `canvas` with lanes, clips, and automation curves; Right 320px inspector: mixer `kpi` (LUFS, spectral balance) + arrangement `log`; Bottom 72px transport: play/loop + **Sketch Arrangement**, **Layer Texture**, **Automate**; floating top-right presence chip.

**Agents (multi-agent):** *Arranger* plans song sections and places stems on lanes; *Sound Designer* selects/synthesizes textures and writes automation via `ui_patch`; *Mix Critic* checks loudness, masking, and mono-compat, flagging into the log and consulting Arranger to thin lanes.

**Agent-driven UI:** Agents place clips on `@region:timeline`, draw automation curves, highlight the active section, and patch the mixer KPIs; presence narrates "Sound Designer sidechaining pads under the swell."

**Declared actions:** `sketch_arrangement(sections)`, `place_clip(stemId,lane,bar)`, `draw_automation(lane,param,curve)`, `layer_texture(section,mood)`, `thin_lanes(section)`, `set_loop(barRange)`.

**Signals (app→agent):** `clip_moved(id,bar)`, `lane_muted(laneId)`, `region_looped(barRange)`, `clip_auditioned(stemId)`.

**User interactions:** Drag clips along bars, mute/solo lanes, draw a loop region, drag automation nodes, click an audition chip to preview.

**The bidirectional loop:** User loops bars 17-24 and mutes the perc lane → Arranger re-reads the section as a breakdown and consults Mix Critic → Critic warns the pad now masks the melody → Sound Designer carves an EQ automation and thins one pad layer, narrating the move → agents patch the mixer KPIs green → user drags a field-recording clip in → Arranger reseats it and Critic re-checks LUFS.

**Platform integration:** model routes (deep for arrangement, fast for automation tweaks), scientific `figure` for spectral/LUFS plots, extensions for audio synthesis, skills: `frontend-design`.

### 55. Kern & Karma — Typography Lab

**Concept:** A poster/typography lab on a node-based generator where agents compose type systems, then propagate spacing and hierarchy rules through connected nodes; you rewire nodes and lock specimens to steer output — a parametric type workbench, not a chatbot.

**Domain & vibe:** Type design & poster craft; precise, playful-but-rigorous, craftsman's pride.

**Theme & aesthetic:** `terminal` theme pack; monospace UI, node wires on graph paper, near-zero chrome, coral only on live/selected nodes; motion is wire-pulse dataflow and specimen re-render fade.

**Layout:** Left 200px rail: node palette (font, scale, kerning, layout, constraint nodes); Center: node-graph `canvas` (a `network` graph of generator nodes) with a live poster preview inset; Right 320px inspector: selected node params + metrics `kpi` (contrast, rhythm) + `log`; Bottom 56px: **Grow Graph**, **Propagate**, **Lock Specimen**; floating top-right presence chip.

**Agents (multi-agent):** *Typographer* proposes the node graph and type pairings; *Kerning Critic* auto-spaces and flags rhythm/contrast faults per node; *Systematizer* propagates accepted rules downstream and writes the specimen sheet into `@region:preview`.

**Agent-driven UI:** Agents add/rewire nodes on `@region:graph`, pulse the edge being evaluated, patch the poster preview and node params, and narrate "Kerning Critic tightening display node to a 1.2 rhythm."

**Declared actions:** `grow_graph(seedNodes)`, `add_node(type,params)`, `connect(fromId,toId)`, `propagate(fromNodeId)`, `lock_specimen(nodeId)`, `respace(nodeId)`.

**Signals (app→agent):** `node_selected(id)`, `edge_drawn(from,to)`, `param_edited(nodeId,key,val)`, `specimen_locked(id)`.

**User interactions:** Drag nodes from palette, draw/cut wires, edit node params, click Propagate, lock a specimen to freeze its rules.

**The bidirectional loop:** User rewires the scale node into the headline node → Typographer re-derives hierarchy and consults Kerning Critic → Critic reports the display now clashes rhythmically and proposes tracking → Systematizer propagates the fix downstream, re-renders the poster preview, narrating each hop → user locks the headline specimen → Systematizer freezes it and reflows only unlocked nodes.

**Platform integration:** model routes (deep for graph reasoning, local for fast respacing), scientific `figure` for kerning-rhythm plots, KB for foundry specimen archives, skills: `taste-skill`.

### 56. Voxel Atelier — 3D Scene Composer

**Concept:** A 3D-scene composer where agents block out set geometry, lighting rigs, and camera moves in a viewport; you drag props, re-key lights, and redirect the shot — a scene-blocking workbench, not a chat.

**Domain & vibe:** 3D previz / set design; spatial, atmospheric, exacting.

**Theme & aesthetic:** `midnight` theme pack; dark viewport, volumetric light accents, gizmo-forward, medium chrome; motion is smooth orbit and light-bloom transitions; color-coded rig lights (key/fill/rim).

**Layout:** Left 240px rail: asset library (props, lights, cameras) as draggable thumbs; Center: 3D viewport `canvas` (author-registered component) with transform gizmos; Right 340px inspector: scene outliner `table` + render `kpi` (exposure, contrast ratio) + `log`; Bottom 64px: **Block Set**, **Rig Lights**, **Set Camera**; floating top-right presence chip.

**Agents (multi-agent):** *Set Dresser* places and scales geometry; *Gaffer* rigs and keys lights for mood and readability; *DP Critic* evaluates composition, exposure, and depth cues, flagging and consulting Gaffer/Set Dresser for adjustments.

**Agent-driven UI:** Agents instantiate props and lights in `@region:viewport`, animate the camera, highlight the selected object's gizmo, and patch the outliner + render KPIs; presence narrates "Gaffer raising the rim light to separate the subject from the backdrop."

**Declared actions:** `block_set(layout)`, `place_prop(assetId,transform)`, `rig_light(type,{pos,intensity,temp})`, `set_camera(shot)`, `reframe(cameraId,composition)`, `grade_exposure(target)`.

**Signals (app→agent):** `object_transformed(id,matrix)`, `light_selected(id)`, `camera_orbited(view)`, `prop_dropped(assetId,pos)`.

**User interactions:** Drag props into the viewport, orbit/pan/zoom, drag gizmos to move/rotate/scale, drag light thumbs to place, click Set Camera to lock a shot.

**The bidirectional loop:** User drags the hero prop off-center → Set Dresser re-derives staging and consults DP Critic → Critic warns the composition now violates thirds and the subject is underlit → Gaffer re-keys the fill and DP nudges the camera, narrating each change → agents patch the render KPIs and outliner → user orbits to check silhouette → Critic re-checks depth cues and clears the flag.

**Platform integration:** model routes (deep for composition, fast for transforms), scientific `figure` for exposure histograms, extensions for asset/geometry generation, skills: `frontend-design`.

### 57. Bindery — Generative Book & Zine Layout

**Concept:** A generative-layout studio for multi-signature books and zines where agents flow long-form content across a grid layout system spanning many spreads; you re-order sections, lock spreads, and redirect the flow — a bookbinding workbench, not a chatbot.

**Domain & vibe:** Book/zine design & self-publishing; craft-driven, warm, patient.

**Theme & aesthetic:** `journal` theme pack; paper texture, letterpress-inspired heads, high density, muted duotone; motion is page-turn flip and grid-snap; color only on section tabs and overset-text warnings (rust).

**Layout:** Left 240px rail: section/TOC tree (draggable); Center: grid-layout `canvas` showing a scrollable ribbon of spreads with master-grid overlay; Right 320px inspector: master-page controls + flow `log` (overset, orphans) + page `kpi`; Bottom 64px: **Flow Content**, **Apply Master**, **Lock Spread**; floating top-right presence chip.

**Agents (multi-agent):** *Editor* structures sections and TOC; *Compositor* flows text/images across spreads on the master grid via `ui_patch`; *Press Critic* checks overset, orphans, image DPI, and imposition, flagging into the log and consulting Compositor.

**Agent-driven UI:** Agents flow content into `@region:ribbon`, patch master-grid frames, highlight the spread under revision, and mark overset in rust; presence narrates "Compositor pushing the sidebar to the verso to clear overset."

**Declared actions:** `flow_content(sectionId)`, `apply_master(spreadId,masterId)`, `reflow(fromSpread)`, `lock_spread(id)`, `place_image(assetId,frame)`, `mark_overset(spreadId)`.

**Signals (app→agent):** `section_reordered(id,idx)`, `spread_locked(id)`, `frame_resized(spreadId,frame)`, `image_dropped(assetId,frame)`.

**User interactions:** Drag TOC sections to re-order, scroll the spread ribbon, resize frames, drag images into wells, click Lock Spread to freeze layout.

**The bidirectional loop:** User re-orders a chapter earlier in the TOC → Editor re-derives pagination and consults Press Critic → Critic warns of new overset and a bad imposition break → Compositor reflows downstream spreads and swaps a master, narrating fixes → user locks a hero spread → Compositor reflows around it while Press Critic re-checks DPI and clears the rust warnings.

**Platform integration:** KB (`br.kb` for reference texts/citations), model routes (deep for pagination logic), scientific `figure` for embedded data pages, workflows for export to print-ready PDF, skills: `taste-skill`.

### 58. Loomframe — Motion Title Sequencer

**Concept:** A timeline-arranger studio for animated title/motion sequences where agents key text, shapes, and easing across a temporal track; you drag keyframes, retime beats, and redirect the motion feel — a motion-design workbench, not a chat.

**Domain & vibe:** Motion graphics / title design; snappy, expressive, rhythm-obsessed.

**Theme & aesthetic:** `terminal` theme pack; grid-ruler timeline, mono labels, near-zero chrome, coral on the active keyframe/playhead; motion is eased keyframe interpolation and ghost-frame onion-skin.

**Layout:** Left 220px rail: element library (title, kicker, shape, mask) draggable; Center: split — top preview `canvas`, bottom keyframe timeline with per-element lanes and easing curves; Right 320px inspector: element params + motion `kpi` (velocity, overshoot) + `log`; Bottom 72px transport: play/scrub + **Sequence Titles**, **Ease**, **Retime**; floating top-right presence chip.

**Agents (multi-agent):** *Choreographer* plans the entrance/exit beats and staggers; *Easer* authors interpolation curves and secondary motion via `ui_patch`; *Motion Critic* checks rhythm, readability dwell time, and overshoot, flagging and consulting Choreographer.

**Agent-driven UI:** Agents drop keyframes into `@region:timeline`, draw easing curves, highlight the active element in preview, and patch motion KPIs; presence narrates "Easer adding a 6-frame overshoot to the kicker for snap."

**Declared actions:** `sequence_titles(elements)`, `key(elementId,frame,props)`, `draw_ease(laneId,curve)`, `stagger(group,offset)`, `retime(range,scale)`, `set_dwell(elementId,ms)`.

**Signals (app→agent):** `keyframe_moved(id,frame)`, `curve_edited(laneId,handle)`, `playhead_scrubbed(frame)`, `element_dropped(id,lane)`.

**User interactions:** Drag keyframes along lanes, bend easing handles, scrub the playhead, drag elements onto lanes, click Retime to rescale a range.

**The bidirectional loop:** User drags the title's entrance keyframe later → Choreographer re-derives the stagger and consults Motion Critic → Critic warns readability dwell dropped below threshold → Easer re-eases the exit and extends dwell, narrating the fix → agents patch the velocity KPI and re-render preview → user bends the ease into a harder snap → Motion Critic re-checks overshoot and clears the flag.

**Platform integration:** model routes (deep for choreography, fast for easing tweaks), scientific `figure` for velocity/overshoot plots, skills: `frontend-design`, workflows to export a Lottie/spec sheet.

### 59. Chroma Diplomat — Color System Studio

**Concept:** A color-system studio on a grid layout where agents build and stress-test accessible palettes, tokens, and themes across component swatches; you drag anchors, lock ramps, and redirect the mood — a design-token workbench, not a chatbot.

**Domain & vibe:** Design systems & UI theming; systematic, diplomatic (balancing brand vs. accessibility), satisfying.

**Theme & aesthetic:** `clinical` theme pack; crisp grid of swatch cards, neutral chrome, precise numeric labels; motion is swatch cross-fade and ramp-slide; live state (failing contrast) flagged in a single alert red.

**Layout:** Left 220px rail: brand anchors + intent sliders (vibrancy, temperature); Center: grid `canvas` of ramps and applied-component swatches (buttons, cards, charts); Right 340px inspector: token `table` (name, hex, role) + contrast `kpi` matrix + `log`; Bottom 56px: **Generate Ramps**, **Audit Contrast**, **Lock Ramp**; floating top-right presence chip.

**Agents (multi-agent):** *Systematist* derives ramps and semantic tokens from anchors; *A11y Auditor* runs WCAG/APCA contrast across every pairing and flags failures; *Brand Advocate* pushes for on-brand vibrancy, negotiating with the Auditor and consulting Systematist for compromises.

**Agent-driven UI:** Agents paint ramps and component swatches into `@region:grid`, patch the token table, redden failing cells in the contrast matrix, and narrate "A11y Auditor: primary-on-surface fails AA; Brand Advocate proposes a darker anchor."

**Declared actions:** `generate_ramps(anchors)`, `assign_token(role,hex)`, `audit_contrast(scope)`, `remap(role,hex)`, `lock_ramp(id)`, `simulate(cvdType)`.

**Signals (app→agent):** `anchor_dragged(id,hex)`, `ramp_locked(id)`, `slider_changed(intent,val)`, `cell_selected(pairId)`.

**User interactions:** Drag anchor swatches on the color field, adjust intent sliders, click a failing matrix cell to request a fix, lock a ramp to protect it, toggle color-blindness simulation.

**The bidirectional loop:** User drags the primary anchor toward a brighter hue → Systematist re-derives the ramp and A11y Auditor re-audits → Auditor reddens three failing pairings → Brand Advocate negotiates, proposing a darker step that keeps vibrancy, consulting Systematist → agents remap tokens, patch the matrix green, narrating the trade → user locks the ramp → Auditor re-runs CVD simulation and confirms.

**Platform integration:** model routes (deep for negotiation logic, fast for re-audit), scientific `figure` for the contrast/APCA matrix, KB for brand + a11y guidelines, skills: `frontend-design`.

### 60. Set & Setting — Environmental Diorama Composer

**Concept:** An infinite-canvas diorama composer where agents assemble layered illustrated scenes (backdrops, midground props, foreground actors, weather) into a coherent environment; you drag layers, lock focal elements, and redirect the atmosphere — a scene-curation surface, not a chat.

**Domain & vibe:** Illustration & world-building / concept art; dreamy, atmospheric, cinematic.

**Theme & aesthetic:** `lab-notebook` theme pack tinted for concept art; layered parallax tiles, soft grain, medium chrome; motion is depth-parallax drift and layer-settle; color guided by a mood-key strip (dawn/dusk/storm) that tints the whole canvas.

**Layout:** Left 240px rail: layer stack (backdrop→foreground) + asset shelf; Center: infinite parallax `canvas` of the diorama; Right 320px inspector: layer props (depth, opacity, tint) + composition `kpi` (balance, focal clarity) + `log`; Bottom 64px: **Compose Scene**, **Set Mood Key**, **Lock Focal**; floating top-right presence chip.

**Agents (multi-agent):** *World Builder* assembles the environment and places layers by depth; *Atmospherist* keys lighting, weather, and color grade; *Composition Critic* checks focal hierarchy, balance, and depth staging, flagging and consulting the other two.

**Agent-driven UI:** Agents place layered tiles into `@region:diorama`, patch depth/tint props, highlight the focal element, and re-tint via the mood key; presence narrates "Atmospherist rolling to a storm key and pushing haze between mid and background."

**Declared actions:** `compose_scene(brief)`, `place_layer(assetId,depth)`, `set_mood_key(key)`, `grade_atmosphere(params)`, `reorder_depth(layerId,z)`, `lock_focal(id)`.

**Signals (app→agent):** `layer_moved(id,pos)`, `layer_reordered(id,z)`, `mood_key_set(key)`, `focal_locked(id)`.

**User interactions:** Drag layers on the canvas, reorder the layer stack, drag the mood-key strip, adjust depth/opacity sliders, lock a focal element to protect it.

**The bidirectional loop:** User drags a lone figure into the foreground and locks it → World Builder re-stages depth and consults Composition Critic → Critic warns the busy midground now competes with the focal figure → Atmospherist pushes atmospheric haze and desaturates the midground, narrating the depth cue → agents patch the focal-clarity KPI → user rolls the mood key to dusk → Atmospherist re-grades the whole canvas and Critic confirms balance.

**Platform integration:** model routes (deep for composition, fast for tint/parallax), scientific `figure` for balance/saliency heatmaps, extensions for illustrated-asset generation, skills: `frontend-design`, `taste-skill`.

## 7. Agent-driven worlds, avatars & serious games  ·  #61–70

### 61. Hexfront: The Council of Generals

**Concept:** A hex-based grand-strategy war-room where three rival AI generals physically move armies, forts, and supply lines on a shared campaign map you also command — not a chatbot but a living tabletop you and the AIs push pieces across.

**Domain & vibe:** Military strategy / operational planning; tense, deliberate, cartographic.

**Theme & aesthetic:** `terminal` pack recolored to parchment-on-slate; monospace unit labels, thin hex grid, near-zero chrome, amber pulse only on contested hexes; motion = pieces slide along drawn arcs.

**Layout:** Left 240px order-of-battle roster (unit cards, drag to deploy); Center: full-bleed `network`/`canvas` hex map with elevation shading; Right 340px inspector (selected hex terrain, stacked units, combat odds `plot`); Bottom 64px transport bar with turn counter + "Resolve Turn" button; floating top-right presence chip narrating each general's moves; top-left "Table Talk" mini composer.

**Agents (multi-agent):** Ironside (aggressive attacker) proposes offensives; Fabian (defensive strategist) countersimulates the same terrain for a hold; Cartographer scores both against supply/attrition and writes the verdict into @region:orders. They hand off each turn: proposal → refutation → arbitration.

**Agent-driven UI:** Agents call place_marker to slide pieces along drawn supply arcs, highlight() contested hexes coral, and patch the odds `plot` in the inspector; presence chip says "Ironside probing your left flank via hex G7."

**Declared actions:** `move_unit(id,hex)`, `draw_supply(from,to)`, `highlight_hex(hex,color)`, `stage_offensive(preset)`, `set_odds(hex,pct)`, `narrate(step)`.

**Signals (app→agent):** `hex_selected(hex)`, `unit_dragged(id,hex)`, `turn_resolved()`, `arc_drawn(from,to)`.

**User interactions:** Drag units from the roster onto hexes, draw supply arcs with a click-drag, select a hex to see odds, press "Resolve Turn" to trigger the generals' turn.

**The bidirectional loop:** You drag a corps to hex G7 → Ironside consults Cartographer on attrition, plans an encirclement → agent slides two enemy stacks and paints G7 amber, narrating the pincer → you draw a supply arc to reinforce → Fabian re-runs odds, patches the inspector to 61% hold, warns of overextension.

**Platform integration:** model routes (deep for arbitration, fast for piece moves); scientific `figure` for attrition curves; KB of doctrine; consult tool between generals.

### 62. Biosphere: The Tending Table

**Concept:** A top-down living-ecosystem sandbox where AI naturalists introduce species, tune climate, and cull invaders on a terrain you also sculpt — a self-running world, never a chat window.

**Domain & vibe:** Ecology / systems biology; contemplative, alive, faintly anxious about tipping points.

**Theme & aesthetic:** `lab-notebook` pack in mossy greens; hand-drawn species glyphs, dotted population contours, soft organic motion of drifting herds; coral only on collapsing populations.

**Layout:** Left 260px species palette (drag organisms onto the world); Center: top-down `canvas` biome map with animated herds/flora; Right 340px inspector with food-web `network` + population `plot`; Bottom 64px climate transport (temp/rainfall sliders + "Advance Season"); floating top-right presence chip; a food-web mini-map lower-left.

**Agents (multi-agent):** Populator seeds candidate species matched to biome; Predator-Modeler simulates trophic cascades and flags collapses; Steward proposes interventions and writes them into @region:field-log; a Skeptic stress-tests the Steward's fix against drought.

**Agent-driven UI:** Agents place herds via spawn_species, redraw food-web edges with ui_patch, pulse crashing nodes coral, and animate migrations; presence narrates "Wolves reintroduced — deer browse pressure dropping in 3 seasons."

**Declared actions:** `spawn_species(id,coords)`, `set_climate(temp,rain)`, `cull(species,pct)`, `draw_food_edge(a,b)`, `advance_season(n)`, `annotate(region,text)`.

**Signals (app→agent):** `species_dropped(id,coords)`, `region_lassoed(poly)`, `season_advanced()`, `climate_slider(param,val)`.

**User interactions:** Drag species onto terrain, lasso a region to protect it, drag climate sliders, press "Advance Season" to run the sim forward.

**The bidirectional loop:** You drag rabbits into a meadow → Populator matches predators, Predator-Modeler forecasts a hare boom-bust → agent animates overgrazing and pulses the grass node coral, narrating the crash → you lasso the meadow and raise rainfall → Skeptic re-runs, Steward patches the field-log: "stabilizes, but foxes now limiting."

**Platform integration:** KB of trophic ecology; scientific `figure` for population dynamics; model routes (fast for spawns, deep for cascade sims); consult between Steward and Skeptic.

### 63. The Long Table: Diplomacy Room

**Concept:** A top-down negotiation table where AI envoys for rival factions table proposals, redline clauses, and shift territory tokens on a shared treaty map you also edit — a diplomatic scene, not a chatbot.

**Domain & vibe:** Diplomacy / conflict resolution; charged, formal, high-stakes calm.

**Theme & aesthetic:** `journal` pack, ivory paper + wax-seal accents; serif clause type, dense two-column treaty text, deliberate fade-in motion; coral only on rejected clauses.

**Layout:** Left 280px faction seats (avatars + demands meters); Center: the treaty `canvas` — a scrollable clause ledger with territory `figure` map inset; Right 340px inspector (selected clause history, concession `kpi`s); Bottom 64px transport ("Table Motion", "Call Vote"); floating presence chip; top-right "Aside" whisper composer.

**Agents (multi-agent):** Envoy-A and Envoy-B each defend a faction's red lines and counter-draft clauses; Mediator finds Pareto trades and patches compromise text into @region:draft; Historian consults br.kb for precedent and annotates risky clauses.

**Agent-driven UI:** Envoys strike/insert clause spans via ui_patch, drag territory tokens on the map figure, flash rejected clauses coral, and update demand meters; presence narrates "Envoy-B concedes the delta in exchange for tariff relief."

**Declared actions:** `draft_clause(section,text)`, `redline(span)`, `move_territory(token,region)`, `set_demand(faction,pct)`, `call_vote(clause)`, `annotate(span,note)`.

**Signals (app→agent):** `clause_selected(id)`, `token_dragged(token,region)`, `vote_called(clause)`, `whisper(faction,text)`.

**User interactions:** Edit clause text directly, drag territory tokens, whisper an aside to one envoy, press "Call Vote."

**The bidirectional loop:** You whisper Envoy-A to soften on fishing rights → Envoy-A drafts a swap, Mediator checks it against Envoy-B's red lines → agent inserts the clause and shifts a coastal token, narrating the trade → you redline the tariff span → Historian flags a 1997 precedent, Envoy-B counters, demand meters shift.

**Platform integration:** br.kb (treaty precedent) heavily; deep model route for trade discovery; scientific `figure` territory map; consult between Envoys and Mediator.

### 64. Vault-7: The Escape Room Engine

**Concept:** A side-view point-and-click escape-room world where AI characters inhabit rooms, manipulate props, and drop clues on a scene you also click through — an interactive world, not a chat transcript.

**Domain & vibe:** Puzzle / narrative adventure; eerie, clever, mounting-pressure.

**Theme & aesthetic:** `midnight` pack, noir teal-black; pixel-lit props, chunky inventory chrome, flickering CRT motion; coral only on active hotspots and the countdown.

**Layout:** Left 220px inventory grid (drag items to combine/use); Center: side-view `canvas` room scene with clickable hotspots and NPC sprites; Right 320px inspector (examined-item detail, clue log); Bottom 64px transport (room-switch tabs + countdown timer); floating presence chip; top-right hint composer.

**Agents (multi-agent):** Gamemaster stages rooms and gates progress; Cipher, an in-world NPC, speaks riddles and reacts to inventory; Trickster plants red-herring clues that Warden (fair-play critic) audits so the room stays solvable.

**Agent-driven UI:** Agents animate NPC sprites, reveal/hide hotspots via toggle_hotspot, patch the clue log, and pulse solvable interactions coral; presence narrates "Cipher slides the third dial — you hear a latch upstairs."

**Declared actions:** `set_scene(room)`, `toggle_hotspot(id,state)`, `give_item(id)`, `speak(npc,line)`, `plant_clue(hotspot,text)`, `advance_timer(delta)`.

**Signals (app→agent):** `hotspot_clicked(id)`, `item_combined(a,b)`, `item_used(id,hotspot)`, `room_changed(room)`.

**User interactions:** Click hotspots, drag inventory items to combine or use on props, switch rooms, ask for a hint.

**The bidirectional loop:** You combine key+wax to make a mold → Gamemaster validates, Cipher reacts in-world → agent animates the safe unlocking and reveals a new hotspot, narrating it → you click the fresh hotspot → Trickster plants a decoy map, Warden audits it as fair and patches the clue log with a hedged hint.

**Platform integration:** model routes (fast for NPC lines, deep for puzzle-graph consistency); KB of puzzle mechanics; consult between Trickster and Warden; scheduled countdown via signals.

### 65. Bastion: Tower-Defense as Roadmap

**Concept:** A top-down tower-defense board that is secretly a project-risk planner — AI engineers place defenses against waves of failure modes on a lane map you also fortify, not a chat assistant.

**Domain & vibe:** Project/risk planning disguised as a game; strategic, playful-serious, momentum-driven.

**Theme & aesthetic:** `biorouter` pack, cobalt + circuit motifs; crisp iso tiles, tower tooltips, wave-spawn pulses; coral only on breached lanes and overdue risks.

**Layout:** Left 240px tower shop (mitigations as buildable towers); Center: top-down `canvas` lane map, enemies = risks flowing toward a "ship date" core; Right 340px inspector (wave forecast `plot`, tower stats); Bottom 64px transport ("Start Wave", speed toggle); floating presence chip; top-right risk-inbox composer.

**Agents (multi-agent):** Threat-Caster generates the next wave of risks from the plan; Architect proposes tower placements to counter them; Red-Team probes gaps and sends a breach to prove weakness; Chronicler writes surviving mitigations into @region:playbook.

**Agent-driven UI:** Agents place/upgrade towers via build_tower, spawn labeled risk-enemies, draw predicted breach paths, pulse threatened lanes coral, and patch the forecast plot; presence narrates "Architect walls the data-migration lane; Red-Team routes around via dependency creep."

**Declared actions:** `spawn_wave(risks[])`, `build_tower(type,tile)`, `upgrade_tower(id)`, `draw_path(lane)`, `start_wave()`, `annotate(lane,note)`.

**Signals (app→agent):** `tower_placed(type,tile)`, `lane_selected(lane)`, `wave_started()`, `risk_added(text)`.

**User interactions:** Drag towers onto lanes, upgrade by clicking, add a risk to the inbox, press "Start Wave" to simulate.

**The bidirectional loop:** You add "vendor slips 2 weeks" to the inbox → Threat-Caster spawns it as a fast enemy, Architect proposes a buffer tower → agent builds it and draws the breach path, narrating the counter → you drag a second tower to the flank → Red-Team routes around it, Chronicler patches the playbook with the surviving mitigation and residual risk.

**Platform integration:** workflows (import a real plan); model routes (deep for wave generation); scientific `figure` for burndown; consult between Architect and Red-Team.

### 66. Gait Studio: The Motion Coaches

**Concept:** A side-view physical-therapy motion-coaching scene where AI clinicians animate a skeleton avatar through corrective exercises, mark joint angles, and adapt reps to your live input — a coaching world, not a chatbot.

**Domain & vibe:** Rehabilitation / biomechanics; encouraging, precise, clinical-warm.

**Theme & aesthetic:** `clinical` pack, calm whites + teal; rounded panels, generous spacing, smooth eased skeleton motion; coral only on out-of-range joints and pain flags.

**Layout:** Left 240px exercise library (drag routines to the timeline); Center: side-view `canvas` with an animated skeleton avatar + goniometer overlays; Right 340px inspector (per-joint ROM `plot`, rep counter `kpi`); Bottom 64px transport (play/pause, tempo, rep count); floating presence chip; top-right symptom composer.

**Agents (multi-agent):** Coach demonstrates the target motion on the avatar; Biomechanist measures joint angles and flags compensations; Adaptor rescales difficulty from your reported effort/pain; Safety-Officer vetoes any progression that risks the flagged joint.

**Agent-driven UI:** Agents drive the avatar via play_motion, draw goniometer arcs, pulse out-of-range joints coral, and patch the ROM plot + rep counter; presence narrates "Coach dropped to 60° flexion; Biomechanist sees hip hike — correcting."

**Declared actions:** `play_motion(exercise,tempo)`, `set_angle_target(joint,deg)`, `mark_joint(joint,state)`, `set_reps(n)`, `progress_difficulty(level)`, `annotate(joint,note)`.

**Signals (app→agent):** `effort_reported(0-10)`, `pain_flag(joint)`, `rep_completed()`, `exercise_dragged(id)`.

**User interactions:** Drag exercises to the timeline, tap a pain flag on a joint, report effort on a slider, adjust tempo, play/pause.

**The bidirectional loop:** You flag knee pain at rep 4 → Biomechanist re-measures, sees valgus collapse → Adaptor proposes a regression, Safety-Officer approves → agent re-animates a shallower squat, paints the knee coral then green, narrating the fix → you report effort 3 → Coach adds two reps, patches the counter and ROM trend.

**Platform integration:** KB of PT protocols; scientific `figure` for ROM-over-session; model routes (fast for adaptation, deep for compensation analysis); consult between Adaptor and Safety-Officer.

### 67. Terra Nova: The Colony Council

**Concept:** A top-down settlement/colony sim where AI advisors zone districts, route logistics, and dispatch colonists on a growing map you also plan — a living colony, never a chat box.

**Domain & vibe:** City/colony management; hopeful, resourceful, quietly precarious.

**Theme & aesthetic:** `lab-notebook` pack, warm ochre blueprint lines; grid-snapped buildings, isometric hint, colonists as moving dots; coral only on shortages and unrest.

**Layout:** Left 260px build palette (zones/buildings to drag); Center: top-down `canvas` colony map with animated colonists + resource flows; Right 340px inspector (resource `kpi` bar, supply `network`); Bottom 64px transport (day counter, speed, "Next Day"); floating presence chip; lower-left minimap.

**Agents (multi-agent):** Planner zones districts for growth; Logistician routes resources and staffs buildings; Sociologist watches morale/unrest and petitions for amenities; Auditor (critic) stress-tests the plan against a supply shock and writes findings into @region:council-notes.

**Agent-driven UI:** Agents place buildings via zone_district, animate colonist assignment, draw resource-flow edges, pulse shortages coral, and patch the resource KPIs; presence narrates "Logistician rerouted grain; Sociologist warns the east ward lacks water — unrest rising."

**Declared actions:** `zone_district(type,tiles)`, `assign_colonists(building,n)`, `route_resource(a,b)`, `trigger_event(shock)`, `advance_day(n)`, `annotate(district,note)`.

**Signals (app→agent):** `tile_painted(type,tiles)`, `building_selected(id)`, `day_advanced()`, `petition_ack(id)`.

**User interactions:** Paint zones onto tiles, drag to route roads/supply, click a building to reassign colonists, press "Next Day."

**The bidirectional loop:** You paint a housing block on the ridge → Planner zones it, Logistician staffs it and routes water → agent animates colonists moving in and draws a water line, narrating the flow → you paint a second block → Sociologist flags the well overloaded, pulses it coral; Auditor runs a drought shock and patches council-notes with a cistern recommendation.

**Platform integration:** model routes (deep for Auditor shocks, fast for zoning); scientific `figure` for resource trends; workflows to seed a scenario; consult between Planner and Auditor.

### 68. Pendulum: The Physics Playground Tutors

**Concept:** A side-view 2D physics sandbox where AI tutors build machines, launch projectiles, and annotate forces on a playground you also drag pieces into — a manipulable world, not a chat.

**Domain & vibe:** Physics education / mechanical intuition; curious, exploratory, aha-driven.

**Theme & aesthetic:** `terminal` pack on graph-paper cyan; thin vector arrows, blueprint parts, springy real-time motion; coral only on force overloads and failed constraints.

**Layout:** Left 220px parts bin (ramps, springs, gears, weights to drag); Center: side-view physics `canvas` with live gravity + force-vector overlays; Right 320px inspector (energy `plot`, part properties); Bottom 64px transport (play/reset, gravity slider, slow-mo); floating presence chip; top-right challenge composer.

**Agents (multi-agent):** Builder assembles a candidate contraption to meet a goal; Physicist annotates forces/energy and predicts the outcome; Breaker adversarially finds the failure (tips, snaps, overshoots); Explainer writes the corrected principle into @region:lesson.

**Agent-driven UI:** Agents place parts via place_part, draw force vectors with draw_vector, pulse overloaded joints coral, run the sim, and patch the energy plot; presence narrates "Builder adds a counterweight; Breaker predicts a tip at 40°."

**Declared actions:** `place_part(type,x,y)`, `connect(a,b)`, `draw_vector(point,force)`, `set_gravity(g)`, `run_sim()`, `annotate(part,note)`.

**Signals (app→agent):** `part_dropped(type,x,y)`, `part_dragged(id,x,y)`, `sim_run()`, `challenge_set(text)`.

**User interactions:** Drag parts onto the canvas, connect with click-drag joints, tweak gravity, press play, set a challenge ("get the ball into the cup").

**The bidirectional loop:** You set "launch ball into the cup" → Builder assembles a ramp+spring, Physicist predicts the arc and draws it → agent runs the sim, ball overshoots, Breaker pulses the spring coral, narrating the excess energy → you drag the spring weaker → Physicist re-annotates, Explainer patches the lesson with the energy trade-off.

**Platform integration:** scientific `figure` for energy/trajectory; model routes (deep for failure prediction); KB of mechanics principles; consult between Physicist and Breaker.

### 69. Reef Wardens: The Living Aquarium

**Concept:** A side-view aquarium/reef sandbox where AI marine biologists stock species, tune water chemistry, and treat disease in a tank you also arrange — a living scene, not a chat interface.

**Domain & vibe:** Aquatic biology / husbandry; serene, absorbing, subtly perilous.

**Theme & aesthetic:** `midnight` pack in deep-sea blues with bioluminescent accents; soft caustic light, drifting fish sprites, gentle sway motion; coral only on chemistry alarms and sick fish.

**Layout:** Left 240px stock palette (fish/coral/plants to drag in); Center: side-view aquarium `canvas` with swimming sprites + water-column gradient; Right 340px inspector (water-chem `kpi`s, compatibility `network`); Bottom 64px transport (feed, water-change, "Advance Week"); floating presence chip; top-right observation composer.

**Agents (multi-agent):** Stocker suggests compatible species for the tank; Chemist monitors pH/nitrate/salinity and predicts crashes; Vet diagnoses disease from sprite behavior and prescribes treatment; Ethicist-critic vetoes overcrowding and writes stocking limits into @region:logbook.

**Agent-driven UI:** Agents add sprites via stock_species, redraw the compatibility network, pulse chemistry alarms and sick fish coral, animate treatment, and patch the chem KPIs; presence narrates "Chemist sees nitrate spiking; Vet flags ich on the clownfish — dosing."

**Declared actions:** `stock_species(id,n)`, `set_chem(param,val)`, `treat(species,med)`, `feed(amount)`, `advance_week(n)`, `annotate(species,note)`.

**Signals (app→agent):** `species_dropped(id)`, `fish_selected(id)`, `week_advanced()`, `observation_logged(text)`.

**User interactions:** Drag species into the tank, click a fish to inspect, adjust chemistry sliders, feed, press "Advance Week."

**The bidirectional loop:** You drag in six tangs → Stocker checks compatibility, Ethicist-critic warns of bioload → agent redraws the network with a red edge, narrating the crowding risk → you add a bigger tank tile → Chemist forecasts nitrate stabilizing, Vet spots stress-ich on one fish, pulses it coral and patches the logbook with a quarantine step.

**Platform integration:** KB of aquarium husbandry; scientific `figure` for chemistry trends; model routes (fast for stocking, deep for disease diagnosis); consult between Vet and Ethicist-critic.

### 70. Ironclad: The Heist Table

**Concept:** A top-down heist-planning board where AI specialists trace guard patrols, place your crew, and rehearse the run on a floorplan you also mark up — a planning table, not a chat window.

**Domain & vibe:** Caper / operations planning; cool, precise, adrenaline-under-control.

**Theme & aesthetic:** `terminal` pack, blueprint cyan on black; dashed patrol paths, sharp iconography, clipped tactical motion; coral only on detection risk and blown timing.

**Layout:** Left 240px crew roster (specialists + gadgets to place); Center: top-down `canvas` floorplan with guard patrol paths + sightline cones; Right 340px inspector (timeline `plot` of the run, alarm `kpi`); Bottom 64px transport ("Rehearse Run", timeline scrubber); floating presence chip; top-right intel composer.

**Agents (multi-agent):** Caser maps guard routes and cameras from intel; Planner sequences the crew's movements; Guard-Sim (adversary) actively hunts the plan and triggers alarms; Fixer patches contingencies into @region:playbook when the run breaks.

**Agent-driven UI:** Agents draw patrol paths via draw_patrol, place crew markers, animate the rehearsal along a timeline, pulse detection hotspots coral, and patch the run timeline plot; presence narrates "Guard-Sim spots your hacker in the east hall at 0:47 — alarm."

**Declared actions:** `draw_patrol(guard,path)`, `place_crew(role,pos)`, `rehearse_run(speed)`, `set_sightline(cam,cone)`, `trigger_alarm(pos)`, `annotate(zone,note)`.

**Signals (app→agent):** `crew_placed(role,pos)`, `zone_lassoed(poly)`, `run_rehearsed()`, `intel_added(text)`.

**User interactions:** Drag crew and gadgets onto the floorplan, redraw a guard path, scrub the rehearsal timeline, lasso a room to mark it, add intel.

**The bidirectional loop:** You place the hacker by the east server room → Caser overlays the nearest patrol, Guard-Sim rehearses and catches them at 0:47 → agent pulses the hall coral and marks the alarm on the timeline, narrating the bust → you drag a lookout to stall the guard → Planner re-sequences, Fixer patches the playbook with a smoke-bomb contingency and a new safe window.

**Platform integration:** model routes (deep for Guard-Sim adversarial rehearsal); scientific `figure` for the run timeline; KB of security patterns; consult between Planner and Guard-Sim.

## 8. Operations, monitoring & control rooms  ·  #71–80

### 71. Aurora Grid Balancer

**Concept:** A live energy-grid control room where the primary surface is a geographic topology map of substations, feeders, and generation assets that agents rebalance in real time — direct dispatch, not chat.

**Domain & vibe:** Utility grid operations; taut, high-stakes, aviation-cockpit calm.

**Theme & aesthetic:** `midnight` theme; monospaced telemetry, ultra-dense, near-zero chrome, amber only on constraint violations, teal on healthy flow, motion limited to flowing power-line dashes and pulsing alerts.

**Layout:** Left 240px rail: asset tree (regions → substations → feeders) with load bars; Center: full-bleed force-directed grid topology map (nodes = buses, edges = lines, thickness = MW flow); Right 360px inspector: selected-asset telemetry (voltage, frequency, thermal margin) + a `plot` of the last 60 min; Bottom 72px transport bar: system frequency gauge, reserve margin KPI, and an **[Approve Dispatch]** button; a floating alert stack top-right. **[Simulate]** button in the inspector runs a what-if; **[Shed Load]** sits red-guarded bottom-right.

**Agents (multi-agent):** *Sentinel* watches SCADA streams and raises constraint/thermal alerts; *Forecaster* projects 30-min load from weather + demand curves (deep route); *Dispatcher* proposes redispatch/switching plans; *Auditor* checks each plan against N-1 contingency rules and refuses unsafe ones before they reach approval.

**Agent-driven UI:** Sentinel paints alert chips and reddens overloaded edges on the map; Dispatcher highlights the proposed switching path in amber and patches the inspector's plan panel; presence chip narrates "Forecaster: peak in 22 min on Feeder 7 → rerouting via Bus 14."

**Declared actions:** `raise_alert(asset,severity)`, `highlight_path(edge_ids)`, `propose_dispatch(plan)`, `stage_contingency(preset)`, `focus_asset(id)`, `annotate(bus,text)`, `commit_dispatch(plan_id)`.

**Signals (app→agent):** `asset_selected(id)`, `edge_hover(id)`, `simulate_requested(scenario)`, `dispatch_approved(plan_id)`, `threshold_dragged(asset,value)`.

**User interactions:** Click a bus to inspect, drag thermal thresholds on the plot, lasso a region to constrain optimization scope, press **[Approve Dispatch]** to commit.

**The bidirectional loop:** Sentinel flags Feeder 7 overheating → Dispatcher consults Forecaster, drafts a reroute, Auditor validates N-1 → Dispatcher highlights the amber path and patches the plan panel, presence narrates the tradeoff → user drags the thermal limit tighter → Dispatcher re-optimizes and re-submits for approval.

**Platform integration:** deep/fast model routes, scientific `figure` load plots, KB of switching-order runbooks, N-1 contingency skill.

### 72. Incident Warroom Timeline

**Concept:** An SRE incident-response war room whose main surface is a synchronized multi-lane alert-and-action timeline, where agents triage a live outage and write the postmortem as it unfolds.

**Domain & vibe:** Site reliability / on-call; urgent, focused, adrenaline-under-discipline.

**Theme & aesthetic:** `terminal` theme; green-on-black monospace, blinking cursor accents, dense log density, red only on firing alerts, hairline lane dividers.

**Layout:** Left 280px rail: affected-services tree with health dots; Center: horizontal multi-lane timeline (lanes = alerts, deploys, agent actions, human notes) scrubbable with a playhead; Right 340px inspector: selected-event detail + linked runbook step; Bottom 60px transport bar: MTTR clock, severity chip, **[Declare Resolved]**; floating **[Page Human]** top-right. A **[Run Runbook]** button in the inspector executes a staged remediation.

**Agents (multi-agent):** *Watcher* ingests metrics/log streams and drops alert events onto the timeline; *Correlator* clusters alerts into a probable root-cause hypothesis (deep route, consults KB of past incidents); *Remediator* proposes runbook steps and stages rollbacks; *Scribe* writes the running postmortem into @region:postmortem.

**Agent-driven UI:** Watcher pushes alert cards onto lanes; Correlator draws a causal link overlay between correlated events and highlights the suspect deploy; Scribe live-patches the postmortem doc; presence narrates "Correlator: 3 alerts trace to deploy #4821 at 14:02 — 87% match to INC-2231."

**Declared actions:** `drop_event(lane,event)`, `link_cause(a,b)`, `highlight_event(id)`, `stage_runbook(step_id)`, `patch_postmortem(section,text)`, `set_severity(level)`, `focus_lane(name)`.

**Signals (app→agent):** `event_selected(id)`, `playhead_moved(t)`, `lane_filtered(name)`, `runbook_approved(step_id)`, `note_added(text)`.

**User interactions:** Scrub the playhead to replay the incident, click events to inspect, filter lanes, approve staged runbook steps, drag severity up.

**The bidirectional loop:** Watcher drops a latency-spike alert → Correlator consults incident KB, links it to deploy #4821 and narrates the match → user clicks the deploy event, hits **[Run Runbook]** → Remediator stages a rollback, Scribe records the action and timestamp → rollback clears alerts → user drags severity down and Scribe closes the postmortem section.

**Platform integration:** KB of historical incidents, deep model route for correlation, runbook skill, log/table catalog widgets.

### 73. FleetOps Orchestrator

**Concept:** A delivery-fleet control room whose primary surface is a live map of vehicles, depots, and routes that agents re-optimize on the fly as traffic and orders shift — a dispatch board, not a chat.

**Domain & vibe:** Logistics / last-mile ops; brisk, operational, mission-control energy.

**Theme & aesthetic:** `biorouter` theme; clean sans, medium density, animated route polylines, coral only on SLA-breach risk, soft motion on vehicle glyphs gliding.

**Layout:** Left 260px rail: driver/vehicle roster with status chips; Center: full-bleed map (vehicle markers, depot pins, route polylines, geofences); Right 340px inspector: selected-route ETA breakdown + `kpi` tiles (on-time %, idle, cost/mile); Bottom 68px transport bar: fleet-wide SLA gauge, **[Commit Reroute]**; floating exception queue top-right. **[Rebalance]** button in the rail triggers a full re-optimization.

**Agents (multi-agent):** *Tracker* watches GPS/traffic streams and flags ETA slippage; *Optimizer* recomputes route assignments (deep route) under time-window and capacity constraints; *Negotiator* checks proposed swaps against driver hours/labor rules; *Dispatcher-Scribe* logs every reroute and messages drivers.

**Agent-driven UI:** Tracker drops SLA-risk markers and reddens at-risk polylines; Optimizer redraws proposed routes as ghost lines and patches the inspector's ETA table; presence narrates "Optimizer: moving 3 stops from Van 12 to Van 7 saves 18 min, no HOS breach."

**Declared actions:** `place_marker(loc,kind)`, `redraw_route(vehicle,path)`, `flag_sla(stop,risk)`, `propose_swap(from,to,stops)`, `focus_vehicle(id)`, `commit_plan(id)`, `message_driver(id,text)`.

**Signals (app→agent):** `vehicle_selected(id)`, `stop_dragged(stop,vehicle)`, `geofence_drawn(poly)`, `rebalance_requested()`, `plan_approved(id)`.

**User interactions:** Drag a stop from one vehicle to another on the map, draw a geofence to exclude a zone, click a route to inspect ETAs, press **[Commit Reroute]**.

**The bidirectional loop:** Tracker flags Van 12 slipping SLA → Optimizer drafts a 3-stop swap, Negotiator confirms no hours-of-service breach → Optimizer draws ghost routes and patches ETA tiles, presence explains the savings → user drags one extra stop onto Van 7 manually → Optimizer re-solves, Scribe messages both drivers on commit.

**Platform integration:** fast route for tracking, deep route for VRP optimization, map widget, labor-rules skill, KPI tiles.

### 74. Quant Desk Sentinel

**Concept:** A trading/portfolio control desk whose main surface is a tiled risk-and-position dashboard where agents monitor exposure, flag anomalies, and propose hedges the trader approves — an ops desk, not a chatbot.

**Domain & vibe:** Quantitative trading / risk management; sharp, cold, split-second precision.

**Theme & aesthetic:** `midnight` theme; tabular monospace, extreme density, green/red P&L flashes, gold only on breach-of-limit, sparkline motion everywhere.

**Layout:** Left 220px rail: portfolio/book tree with live P&L; Center: tiled grid — positions `table`, exposure heatmap, factor-attribution `plot`, VaR `kpi`; Right 340px inspector: selected-position greeks + proposed-hedge ticket; Bottom 64px transport bar: net exposure gauge, drawdown KPI, **[Execute Hedge]** (guarded); floating limit-breach stack top-right. **[Stress Test]** button above the heatmap.

**Agents (multi-agent):** *Monitor* streams marks and greeks, raises limit-breach and drift alerts; *Analyst* attributes moves to factors and runs stress scenarios (deep route); *Strategist* proposes hedge tickets; *Compliance* validates each ticket against mandate/limit rules and blocks violations before approval.

**Agent-driven UI:** Monitor flashes breached cells gold and pushes alert chips; Strategist patches the hedge ticket in the inspector and highlights the offending exposure tile; presence narrates "Analyst: 60% of today's drawdown is rate-factor — hedging with 200 lots 2Y."

**Declared actions:** `flag_breach(book,limit)`, `highlight_tile(id)`, `propose_hedge(ticket)`, `run_stress(scenario)`, `focus_position(id)`, `attribute_move(factor)`, `stage_ticket(id)`.

**Signals (app→agent):** `position_selected(id)`, `cell_edited(book,limit)`, `stress_requested(scenario)`, `hedge_approved(ticket_id)`, `tile_zoomed(id)`.

**User interactions:** Click a heatmap cell to drill in, edit a risk limit inline, run a stress scenario, adjust hedge size in the ticket, press **[Execute Hedge]**.

**The bidirectional loop:** Monitor flashes a VaR breach gold → Analyst attributes it to rate factor and narrates → Strategist drafts a 2Y hedge ticket, Compliance clears it against mandate → inspector shows the ticket and highlights the exposure tile → trader trims size and edits the limit → Strategist re-prices, Compliance re-checks, ready to execute.

**Platform integration:** fast route for marks, deep route for attribution/stress, table+heatmap+kpi widgets, mandate-rules skill, factor-model KB.

### 75. Pipeline Nexus

**Concept:** A CI/CD and data-pipeline orchestration room whose primary surface is a live DAG topology graph of build/deploy/ETL stages that agents watch, retry, and reroute — a control graph, not a chat window.

**Domain & vibe:** DevOps / data engineering; methodical, systems-thinking, quietly relentless.

**Theme & aesthetic:** `lab-notebook` theme; humanist mono, medium density, flowing edge particles on running stages, rust-orange only on failed nodes, graph pans smoothly.

**Layout:** Left 240px rail: pipeline list with run-status dots; Center: full-canvas DAG (`network` force-graph — nodes = stages, edges = data/build deps, badge = duration); Right 360px inspector: selected-stage logs (`log` widget) + artifact list + retry controls; Bottom 66px transport bar: pipeline health KPI, queue depth, **[Approve Rollout]**; floating failure queue top-right. **[Retry Failed]** and **[Skip Stage]** buttons in the inspector.

**Agents (multi-agent):** *Sentry* watches stage exit-codes/logs and marks failures; *Diagnostician* reads logs to classify failure cause (flaky vs real, deep route, consults KB of known errors); *Operator* proposes retry/skip/reroute plans; *Chronicler* logs decisions and updates the run history.

**Agent-driven UI:** Sentry reddens failed nodes and pulses their edges; Diagnostician annotates the node with a root-cause chip and patches the log inspector to the relevant line; presence narrates "Diagnostician: OOM in `transform-3`, not flaky — suggest bumping memory, not retry."

**Declared actions:** `mark_node(id,status)`, `annotate_node(id,cause)`, `highlight_edge(ids)`, `propose_action(stage,verb)`, `focus_stage(id)`, `patch_logs(range)`, `stage_rollout(plan)`.

**Signals (app→agent):** `node_selected(id)`, `edge_selected(id)`, `retry_clicked(stage)`, `stage_skipped(id)`, `rollout_approved(plan)`.

**User interactions:** Click a DAG node to read logs, drag to pan the graph, press **[Retry Failed]** or **[Skip Stage]**, approve the rollout on the transport bar.

**The bidirectional loop:** Sentry reddens `transform-3` → Diagnostician reads the log, classifies it OOM (not flaky), consults the error KB → node gets a root-cause chip, inspector jumps to the OOM line, presence narrates the recommendation → user rejects blind retry, bumps memory and clicks retry → Operator reruns downstream, Chronicler records the fix in run history.

**Platform integration:** network graph widget, log widget, deep route for diagnosis, known-errors KB, retry/rollout runbook skill.

### 76. Brewhouse Console

**Concept:** A brewery/bioreactor process-control console whose main surface is a live P&ID schematic with vessel gauges that agents monitor and adjust setpoints on across a fermentation batch — an instrument panel, not a chatbot.

**Domain & vibe:** Bioprocess / craft-brew instrument control; warm, tactile, patient-artisan-meets-precision.

**Theme & aesthetic:** `lab-notebook` theme; slab-serif labels, medium-high density, gentle needle sweeps on gauges, copper accents, amber only on out-of-band process variables.

**Layout:** Left 240px rail: batch/vessel list with phase chips (mash→boil→ferment→condition); Center: interactive P&ID schematic (tanks, valves, pumps, sensors) with inline gauges; Right 340px inspector: selected-vessel trend `plot` (temp, gravity, pH, DO) + setpoint sliders; Bottom 68px transport bar: batch-phase progress, alarm count, **[Apply Setpoints]**; floating alarm banner top-right. **[Advance Phase]** button in the rail.

**Agents (multi-agent):** *Monitor* streams sensor telemetry and raises out-of-band alarms; *Fermentation-Model* predicts gravity/attenuation trajectory (deep route); *Process-Engineer* proposes setpoint/valve changes to hit the target curve; *Batch-Scribe* writes the batch log and flags deviations for QA.

**Agent-driven UI:** Monitor flags off-spec sensors amber on the schematic; Process-Engineer highlights the valve/heater to adjust and patches the setpoint sliders with proposed values; presence narrates "Model: attenuation stalling at 1.020 — raise ferment temp 1.5°C to finish by day 9."

**Declared actions:** `raise_alarm(sensor,band)`, `highlight_component(id)`, `propose_setpoint(vessel,var,value)`, `predict_curve(vessel)`, `focus_vessel(id)`, `annotate_batch(text)`, `advance_phase(vessel)`.

**Signals (app→agent):** `vessel_selected(id)`, `setpoint_dragged(var,value)`, `valve_toggled(id)`, `phase_advanced(vessel)`, `alarm_ack(id)`.

**User interactions:** Click a tank to inspect, drag setpoint sliders, toggle a valve on the schematic, acknowledge alarms, press **[Apply Setpoints]**.

**The bidirectional loop:** Monitor flags fermentation temp drifting → Fermentation-Model predicts a stalled gravity curve, narrates the risk → Process-Engineer highlights the glycol valve and proposes +1.5°C on the sliders → user nudges the slider a bit lower and toggles the valve manually → Model re-predicts the finish date, Scribe logs the deviation and the operator override.

**Platform integration:** deep route for kinetic modeling, scientific `figure` trend plots, batch-record KB, QA-deviation skill, fast route for telemetry.

### 77. Sky Sentinel ATC-Ops

**Concept:** An airspace-operations monitoring room whose primary surface is a live radar-scope map of flights, sectors, and weather cells where agents flag conflicts and propose reroutes controllers approve — a scope, not a chat box.

**Domain & vibe:** Air-traffic flow management; vigilant, procedural, quiet-tension.

**Theme & aesthetic:** `terminal` theme; phosphor-green vector strokes, sweeping radar motion, very low chrome, red only on conflict pairs, blips leave fading trails.

**Layout:** Left 260px rail: sector/flight strip board (departure order, altitude, status); Center: full radar scope (flight blips with vectors, sector polygons, weather overlays); Right 340px inspector: selected-flight strip detail + conflict-resolution options; Bottom 64px transport bar: sector load gauge, conflict count KPI, **[Clear Reroute]**; floating conflict alert stack top-right. **[Weather Overlay]** and **[Sector Handoff]** buttons in the rail.

**Agents (multi-agent):** *Radar-Watch* streams tracks and predicts loss-of-separation conflicts; *Flow-Planner* projects sector congestion and proposes metering (deep route); *Router* drafts conflict-free reroutes/altitude changes; *Log-Keeper* records every clearance and handoff for audit.

**Agent-driven UI:** Radar-Watch draws red conflict-pair lines and pulses the two blips; Router paints a proposed dashed reroute vector and patches the strip inspector with the amended clearance; presence narrates "Radar-Watch: UAL221 and DAL88 converge in 4 min at FL340 — Router: descend DAL88 to FL320."

**Declared actions:** `flag_conflict(a,b)`, `draw_vector(flight,path)`, `highlight_sector(id)`, `propose_clearance(flight,change)`, `focus_flight(id)`, `overlay_weather(cell)`, `log_clearance(id)`.

**Signals (app→agent):** `flight_selected(id)`, `strip_reordered(seq)`, `sector_clicked(id)`, `clearance_approved(id)`, `weather_toggled(cell)`.

**User interactions:** Click a blip to select, reorder departure strips, click a sector to load-check, toggle weather cells, press **[Clear Reroute]**.

**The bidirectional loop:** Radar-Watch predicts a loss-of-separation in 4 min → Flow-Planner checks sector load, Router drafts a descent for DAL88 → scope draws the dashed reroute, inspector shows the amended clearance, presence narrates the geometry → controller drags the vector slightly north to dodge a weather cell → Router revalidates separation, Log-Keeper records the issued clearance.

**Platform integration:** fast route for track updates, deep route for flow prediction, map/scope widget, separation-rules skill, procedures KB.

### 78. Approvals Queue Command

**Concept:** A financial-operations approvals war room whose main surface is a prioritized approvals queue plus an evidence workbench, where agents triage payment/access requests, assemble justification, and route to the right approver — a queue desk, not a chat.

**Domain & vibe:** FinOps / access governance; measured, audit-minded, trust-but-verify.

**Theme & aesthetic:** `clinical` theme; crisp sans, high density, restrained motion (rows slide in), blue-grey palette, red only on policy violations, subtle SLA countdown ticks.

**Layout:** Left 300px rail: prioritized queue (`table` — requester, amount, risk score, SLA timer); Center: evidence workbench for the selected request (policy checks, prior approvals, linked docs); Right 340px inspector: recommended decision + routing chain + approver picker; Bottom 60px transport bar: queue depth, breach-risk KPI, **[Approve]** / **[Reject]** (guarded); floating escalations stack top-right. **[Auto-Triage]** button atop the queue.

**Agents (multi-agent):** *Intake* scores and prioritizes incoming requests by risk/SLA; *Investigator* gathers evidence — policy matches, duplicate/fraud signals, prior decisions (deep route, KB); *Advisor* recommends approve/reject/escalate with rationale; *Recorder* writes the audit trail and notifies routed approvers.

**Agent-driven UI:** Intake reorders the queue and badges risk; Investigator patches the evidence workbench with policy-check cards and duplicate flags; Advisor fills the recommendation panel and highlights the routing chain; presence narrates "Investigator: matches a duplicate vendor invoice from March — flagging, recommend hold."

**Declared actions:** `prioritize_queue(order)`, `populate_evidence(request,cards)`, `recommend(request,decision)`, `route_to(approver)`, `flag_duplicate(id)`, `focus_request(id)`, `record_decision(id)`.

**Signals (app→agent):** `request_selected(id)`, `evidence_expanded(card)`, `decision_overridden(id,choice)`, `approver_reassigned(id)`, `queue_filtered(criteria)`.

**User interactions:** Click a queue row to open the workbench, expand evidence cards, reassign the approver, override the recommendation, press **[Approve]**/**[Reject]**.

**The bidirectional loop:** Intake floats a high-risk payment to the top → Investigator gathers evidence, finds a March duplicate, narrates the flag → workbench shows duplicate + policy cards, Advisor recommends hold and highlights the escalation route → user expands the duplicate card, overrides to reject → Recorder writes the audit entry and notifies the requester.

**Platform integration:** deep route for investigation, policy/precedent KB, table widget, fraud-signal skill, audit-trail workflow.

### 79. Datacenter Thermal Bridge

**Concept:** A datacenter facilities control room whose primary surface is a rack-and-airflow floor-plan heatmap where agents watch thermal/power telemetry and propose cooling/workload moves operators approve — a facilities console, not a chatbot.

**Domain & vibe:** Datacenter infrastructure management; steady, efficiency-obsessed, hum-of-machines calm.

**Theme & aesthetic:** `midnight` theme; condensed mono, high density, smooth heat-gradient shading, cyan on healthy PUE, magenta only on hotspots, slow airflow-arrow motion.

**Layout:** Left 240px rail: hall/row/rack tree with temp + power bars; Center: floor-plan heatmap (racks as tiles, CRAC units, airflow arrows) with hotspot glow; Right 360px inspector: selected-rack sensors + power draw `plot` + cooling-setpoint sliders; Bottom 66px transport bar: facility PUE gauge, hotspot count KPI, **[Apply Cooling Plan]**; floating thermal-alert stack top-right. **[Balance Load]** and **[Boost CRAC]** buttons in the inspector.

**Agents (multi-agent):** *Thermal-Watch* streams inlet temps/power and flags hotspots/hot-aisle recirculation; *Efficiency-Modeler* computes PUE impact of cooling/workload options (deep route); *Facilities-Planner* proposes CRAC setpoint or VM-migration plans; *Ops-Scribe* logs actions and change tickets.

**Agent-driven UI:** Thermal-Watch glows hotspot tiles magenta and animates recirculation arrows; Facilities-Planner highlights the target CRAC and racks to migrate, patches setpoint sliders; presence narrates "Modeler: raising CRAC-3 airflow 12% clears the Row-C hotspot, PUE +0.02 — cheaper than migrating 4 VMs."

**Declared actions:** `flag_hotspot(rack,temp)`, `highlight_component(id)`, `propose_cooling(crac,setpoint)`, `propose_migration(vms,target)`, `model_pue(scenario)`, `focus_rack(id)`, `log_change(ticket)`.

**Signals (app→agent):** `rack_selected(id)`, `setpoint_dragged(crac,value)`, `migration_approved(plan)`, `crac_boosted(id)`, `zone_lassoed(region)`.

**User interactions:** Click a rack tile to inspect, drag cooling setpoints, lasso a zone to scope balancing, press **[Boost CRAC]** or **[Apply Cooling Plan]**.

**The bidirectional loop:** Thermal-Watch glows a Row-C hotspot magenta → Efficiency-Modeler compares CRAC-boost vs VM-migration for PUE, narrates the cheaper option → Planner highlights CRAC-3 and patches its airflow slider → operator lassoes Row-C to confirm scope, nudges the slider higher → Modeler recomputes PUE, Scribe logs the change ticket on apply.

**Platform integration:** fast route for telemetry, deep route for PUE modeling, scientific `figure` power plots, floor-plan canvas widget, capacity-planning KB.

### 80. Observatory Nightwatch

**Concept:** A robotic-telescope observatory operations room whose main surface is an all-sky dome chart plus a target-queue schedule where agents watch conditions, retriage the observing plan, and slew instruments the astronomer approves — a mission console, not a chat.

**Domain & vibe:** Astronomy / observatory automation; nocturnal, wondrous-yet-rigorous, quiet-hum-under-stars.

**Theme & aesthetic:** `journal` theme; elegant serif captions over dark sky, low chrome, star-field motion, violet only on weather/fault holds, gentle fades between targets.

**Layout:** Left 280px rail: observing queue (`table` — target, priority, airmass, exposure, window); Center: all-sky dome chart (horizon, target markers, current pointing, cloud overlay) + telescope-status ring; Right 340px inspector: selected-target ephemeris + instrument config + a live `figure` of the last frame; Bottom 64px transport bar: dome/weather status, queue-time-left KPI, **[Approve Slew]**; floating fault/weather-hold stack top-right. **[Re-plan Night]** button atop the queue.

**Agents (multi-agent):** *Sky-Watch* streams seeing/cloud/wind and raises weather holds; *Scheduler* reoptimizes the observing queue by airmass/priority/window (deep route); *Instrument-Op* proposes slew + exposure/filter configs and checks limits; *Night-Log* writes the observing log and flags data-quality issues.

**Agent-driven UI:** Sky-Watch overlays cloud/wind and dims blocked targets; Scheduler reorders the queue and draws the proposed slew arc on the dome; Instrument-Op patches the exposure config; presence narrates "Sky-Watch: cirrus over the east — Scheduler: swapping to the zenith standard, saving 20 min before it clears."

**Declared actions:** `raise_hold(reason)`, `reorder_queue(order)`, `draw_slew(target,arc)`, `propose_config(target,exposure,filter)`, `overlay_conditions(layer)`, `focus_target(id)`, `log_frame(id)`.

**Signals (app→agent):** `target_selected(id)`, `queue_reordered(seq)`, `slew_approved(target)`, `config_edited(field,value)`, `sky_region_lassoed(area)`.

**User interactions:** Click a dome marker or queue row to select, drag to reorder the queue, lasso a sky region to prioritize, edit exposure/filter, press **[Approve Slew]**.

**The bidirectional loop:** Sky-Watch detects cirrus east and raises a hold → Scheduler reoptimizes, swaps in a zenith standard, narrates the time saved → dome draws the new slew arc, inspector patches the exposure config → astronomer lassoes a western region to keep a priority target queued → Scheduler re-solves around the constraint, Night-Log records the plan change and the held target.

**Platform integration:** deep route for schedule optimization, scientific `figure` frame previews, ephemeris/target KB, weather-hold skill, fast route for conditions telemetry.

## 9. Education, labs & interactive derivation  ·  #81–90

### 81. Proof Forge

**Concept:** A visual natural-deduction proof builder where a formal-logic derivation graph — not a chat — is the primary surface; the agent stages inference steps as nodes and the user wires premises to conclusions by dragging.

**Domain & vibe:** Formal logic / discrete math; the satisfying tension of a locked-in QED.

**Theme & aesthetic:** `terminal` theme; monospaced, high-density, near-zero chrome, phosphor-green edges that pulse amber only on an unsound link; sequent lines in crisp serif.

**Layout:** Left 260px rail: goal sequent + rule palette (∧I, →E, RAA…) as draggable chips; Center: the network force-graph derivation DAG (premises pinned top, goal pinned bottom); Right 340px inspector: selected-node justification, rule signature, discharged assumptions; Bottom 64px transport bar: `Check`, `Auto-step`, `Undo`, `Export LaTeX` buttons; floating soundness meter top-right.

**Agents (multi-agent):** *Coach* proposes the next legal inference and highlights candidate nodes; *Examiner* runs after every user wire, validates the step against the rule's typing, and marks unsound edges red; *Hint-smith* consults only when the user presses `Stuck`, offering a graded ladder of hints without revealing the line.

**Agent-driven UI:** Coach paints ghost nodes into @region:graph via ui_patch and app_call `place_step`; Examiner recolors edges and posts a red annotation; presence chip narrates "Examiner: →E needs its antecedent — line 3 is unbound."

**Declared actions:** `place_step(rule,inputs)`, `focus_node(id)`, `mark_unsound(edge,reason)`, `annotate(node,text)`, `discharge(assumption)`, `export_latex()`.

**Signals (app→agent):** `edge_drawn(from,to)`, `node_selected(id)`, `rule_dropped(rule,pos)`, `stuck_pressed()`.

**User interactions:** User drags rule chips onto the canvas, wires node ports to form inferences, double-clicks to discharge an assumption, clicks `Check`.

**The bidirectional loop:** User wires line 3→5 as →E → Examiner detects a free variable, reddens the edge, narrates the fault → Coach consults Hint-smith, ghosts a ∀E node upstream → user drags the ghost solid and rewires → Examiner turns the path green, soundness meter ticks to 100%.

**Platform integration:** deep model route for proof search; scientific figures for the exported proof tree; a `formal-logic` skill for rule semantics.

### 82. Wet-Bench Virtuoso

**Concept:** A virtual molecular-biology bench where instruments and reagents are draggable objects on a canvas, and agents co-pilot the protocol by moving pipettes, flagging contamination, and staging the next step — never a chat transcript.

**Domain & vibe:** Wet-lab molecular biology; the focused calm of a clean bench under fume-hood light.

**Theme & aesthetic:** `lab-notebook` theme; graph-paper canvas, hand-annotation callouts, muted teal glassware, motion limited to liquid-fill animations; ruled margins.

**Layout:** Left 260px rail: reagent/instrument shelf (pipettes, tubes, gels, thermocycler) as draggable icons; Center: the bench canvas with labeled stations; Right 340px inspector: protocol timeline + current-step parameters (volume, temp, cycles); Bottom 64px transport bar: `Run Step`, `Pause`, `Log to Notebook`, `Reset`; floating hazard chip top-right.

**Agents (multi-agent):** *Protocol Planner* lays out the ordered station sequence for the stated experiment (e.g. PCR); *Safety Examiner* watches every user action and blocks unsafe combos (ethidium bromide without gloves); *Bench Coach* demonstrates the next pipetting move by animating a ghost instrument.

**Agent-driven UI:** Planner patches the protocol timeline in @region:steps; Coach drives ghost pipette motion via app_call `demonstrate`; Safety Examiner throws a red hazard overlay; presence narrates "Coach: aspirate 20µL from the master mix."

**Declared actions:** `stage_step(name,params)`, `demonstrate(instrument,path)`, `flag_hazard(station,reason)`, `fill_vessel(id,volume)`, `set_param(step,key,val)`, `log_result(fig)`.

**Signals (app→agent):** `instrument_dropped(id,station)`, `vessel_clicked(id)`, `param_edited(step,key,val)`, `run_pressed()`.

**User interactions:** User drags a pipette to a tube, sets volume via slider, drops a plate into the thermocycler, presses `Run Step`.

**The bidirectional loop:** User drags master mix into 8 wells → Safety Examiner notes missing template control, flags well H12 → Planner consults, inserts a "no-template control" step into the timeline → user drops water into H12 → Coach animates the load, run proceeds, results logged as a figure.

**Platform integration:** scientific figures for gel/qPCR output; `single-cell`/`variant-calling` skills for downstream steps; KB for protocol references.

### 83. Chrono-Reconstruct

**Concept:** A historical-event reconstruction timeline where the agent assembles a contested event as draggable evidence cards on a branching timeline, and the user reorders, corroborates, or disputes them — a workbench, not a Q&A bot.

**Domain & vibe:** History / source criticism; the sober rigor of a courtroom timeline.

**Theme & aesthetic:** `journal` theme; sepia parchment, letterpress serif, ink-stamp corroboration seals, slow fade-in of new cards; ledger rules.

**Layout:** Left 260px rail: source drawer (primary docs, chronicles, artifacts) as draggable cards; Center: horizontal branching timeline with event lanes; Right 340px inspector: selected-card provenance, credibility score, conflicting accounts; Bottom 64px transport bar: `Corroborate`, `Split Branch`, `Fork Counterfactual`, `Publish Narrative`; floating confidence gauge top-right.

**Agents (multi-agent):** *Archivist* pulls candidate sources from the KB and drops them as cards; *Skeptic* cross-examines each card, surfacing contradictions and dating conflicts; *Narrator* writes the accepted, ordered sequence into @region:brief once corroborated.

**Agent-driven UI:** Archivist patches source cards onto lanes; Skeptic draws red conflict connectors and annotates dating gaps; Narrator streams prose into the brief; presence narrates "Skeptic: this dispatch postdates the battle by six weeks."

**Declared actions:** `place_card(source,lane,date)`, `link_conflict(a,b,reason)`, `corroborate(card,evidence)`, `fork_branch(point,label)`, `annotate(card,text)`, `publish_narrative()`.

**Signals (app→agent):** `card_dragged(id,lane,date)`, `cards_linked(a,b)`, `branch_forked(point)`, `publish_pressed()`.

**User interactions:** User drags source cards onto lanes, snaps them to dates, links two as corroborating, forks a counterfactual branch.

**The bidirectional loop:** User drops a memoir onto 1815 → Archivist retrieves a contradicting field report from KB → Skeptic links them red, narrates the date conflict → user reassigns the memoir to a later lane → Narrator marks the report corroborated and writes the settled paragraph into the brief.

**Platform integration:** br.kb search/page for primary sources; deep model route for source cross-examination; `scientific-research` skill for citation credibility.

### 84. Circuit Sandbox

**Concept:** A live breadboard where components are dragged into holes and wires snapped between rails, with agents diagnosing shorts, staging the next build step, and probing nodes — an instrument workbench, not a chat window.

**Domain & vibe:** Electronics / EE education; the hands-on delight of a blinking first LED.

**Theme & aesthetic:** `midnight` theme; dark PCB green-black, neon trace glow, oscilloscope grid, motion only in current-flow particle animation; solder-bead accents.

**Layout:** Left 260px rail: component bin (resistors, LEDs, ICs, caps) as draggable parts with value chips; Center: the breadboard canvas with power rails; Right 340px inspector: selected-node voltage/current readout + a live plot widget of the probed signal; Bottom 64px transport bar: `Power On`, `Probe`, `Auto-route`, `Save Netlist`; floating multimeter top-right.

**Agents (multi-agent):** *Circuit Coach* stages the next component placement for the target circuit (e.g. astable 555); *Fault Examiner* runs SPICE-style checks on power-on and localizes shorts/floating pins; *Probe Guide* narrates expected vs. measured waveforms when the user probes a node.

**Agent-driven UI:** Coach ghosts the next part into @region:board via place_component; Fault Examiner reddens shorted rails and annotates; Probe Guide streams the expected trace into the plot widget; presence narrates "Fault Examiner: pin 4 floating — tie to VCC."

**Declared actions:** `place_component(part,hole)`, `route_wire(a,b)`, `power(state)`, `probe_node(id)`, `flag_fault(node,reason)`, `plot_signal(node,fig)`.

**Signals (app→agent):** `part_dropped(part,hole)`, `wire_snapped(a,b)`, `node_probed(id)`, `power_toggled(state)`.

**User interactions:** User drags a resistor into holes, snaps a wire across the rail, clicks a node to probe, presses `Power On`.

**The bidirectional loop:** User wires an LED without a resistor → Fault Examiner flags overcurrent on power-on, reddens the branch → Coach consults, ghosts a 330Ω resistor in series → user drops it in → Probe Guide plots the corrected current, LED animates lit, netlist saved.

**Platform integration:** local model route for fast SPICE checks; scientific figures for waveform plots; a `circuits` skill for component models.

### 85. Immersion Atrium

**Concept:** A language-immersion scene where a rendered environment (market, café, station) is the stage and agent-actors move, gesture, and speak in-scene while the learner clicks objects and drags phrase-tiles to respond — a role-play world, not a chatbot.

**Domain & vibe:** Language acquisition; the exhilarating vertigo of thinking in a new tongue.

**Theme & aesthetic:** `biorouter` theme; warm illustrative scene canvas, comic speech bubbles, gentle parallax on interaction, object-highlight halos; rounded UI.

**Layout:** Left 260px rail: phrase-tile tray + vocabulary bank as draggable tiles; Center: the scene canvas with clickable characters/objects; Right 340px inspector: literal gloss, grammar note, register meter; Bottom 64px transport bar: `Say It`, `Slower`, `Hint`, `New Scene`; floating fluency chip top-right.

**Agents (multi-agent):** *Scene Director* stages the setting and blocks NPC movement; *Interlocutor* voices an in-scene character that reacts to the learner's constructed sentence; *Correction Coach* silently scores the utterance and, on error, glows the faulty tile and offers a gentler rephrase.

**Agent-driven UI:** Director patches the scene and moves characters via app_call `move_actor`; Interlocutor renders speech bubbles into @region:scene; Correction Coach highlights bad tiles; presence narrates "Director: the vendor turns to you, holding oranges."

**Declared actions:** `move_actor(id,path)`, `speak(actor,text,bubble)`, `highlight_object(id)`, `glow_tile(id,reason)`, `set_register(level)`, `advance_scene(name)`.

**Signals (app→agent):** `object_clicked(id)`, `tiles_assembled(sentence)`, `say_pressed()`, `hint_pressed()`.

**User interactions:** User clicks a market stall, drags phrase-tiles into a sentence slot, presses `Say It`, taps a character to prompt reply.

**The bidirectional loop:** User assembles an over-formal request → Interlocutor answers in-scene but stiffly → Correction Coach glows the formal tile, narrates the register mismatch → user swaps in a casual tile → Interlocutor warms up and hands over the oranges, fluency chip rises.

**Platform integration:** fast model route for low-latency in-scene reply; deep route for grammar critique; a `language-immersion` skill; scientific figures for progress plots.

### 86. Derivation Loom

**Concept:** A physics/math derivation builder where each algebraic transformation is a node on a step-graph and the user drags operators onto equations to transform them, while agents verify each move and expose the assumptions — a manipulable derivation, not a chat.

**Domain & vibe:** Theoretical physics / applied math; the clean thrill of watching an equation collapse to a known law.

**Theme & aesthetic:** `journal` theme; cream paper, LaTeX-rendered equations, chalk-line connectors, subtle ink-bleed on new steps; wide margins for assumptions.

**Layout:** Left 260px rail: operator palette (integrate, substitute, expand, take-limit, non-dimensionalize) as draggable tiles + a symbol bank; Center: vertical derivation graph of equation nodes; Right 340px inspector: selected-step assumptions, validity domain, dimensional check; Bottom 64px transport bar: `Verify Step`, `Simplify`, `Branch`, `Export`; floating dimension-check chip top-right.

**Agents (multi-agent):** *Deriver* suggests the next transformation and ghosts the resulting equation; *Rigor Examiner* re-derives symbolically to confirm the user's step is algebraically valid and dimensionally consistent; *Assumption Keeper* logs every hidden assumption (small-angle, incompressible) into the inspector and warns when a later step violates it.

**Agent-driven UI:** Deriver patches ghost equation nodes into @region:graph; Rigor Examiner recolors invalid steps and annotates the algebra error; Assumption Keeper streams the running assumption ledger; presence narrates "Rigor Examiner: your substitution dropped a factor of 2."

**Declared actions:** `apply_op(op,node,target)`, `ghost_result(node,latex)`, `flag_invalid(node,reason)`, `log_assumption(node,text)`, `dim_check(node)`, `export_derivation()`.

**Signals (app→agent):** `op_dropped(op,node)`, `node_selected(id)`, `symbol_edited(node,expr)`, `verify_pressed()`.

**User interactions:** User drags `take-limit` onto an equation node, edits a symbol inline, presses `Verify Step`, branches an alternate route.

**The bidirectional loop:** User drops `small-angle` onto a pendulum equation → Rigor Examiner confirms validity but Assumption Keeper logs "θ≪1" → three steps later user re-uses a large-angle term → Assumption Keeper reddens the conflict, narrates it → user branches to keep the exact form → Deriver ghosts the elliptic-integral path.

**Platform integration:** deep model route for symbolic verification; scientific figures for plotting the derived law; a `physics-derivations` skill; KB for standard-form references.

### 87. Anatomy Atelier

**Concept:** A manipulable 3D-layered anatomy diagram where the agent peels systems, pins structures, and quizzes by highlighting, while the user rotates, dissects layers, and drags labels onto structures — an interactive dissection, not a chat.

**Domain & vibe:** Medical/anatomy education; the reverent precision of a cadaver lab.

**Theme & aesthetic:** `clinical` theme; sterile white, cool-grey structures, single crimson accent on the active structure, smooth layer-fade motion; label leader-lines.

**Layout:** Left 260px rail: system toggles (skeletal, muscular, vascular, neural) + label bank as draggable tags; Center: the rotatable layered anatomy canvas; Right 340px inspector: selected-structure name, function, clinical notes, related pathologies; Bottom 64px transport bar: `Peel Layer`, `Quiz Me`, `Cross-Section`, `Reset View`; floating mastery chip top-right.

**Agents (multi-agent):** *Dissection Guide* stages the reveal, peeling to the region under study; *Examiner* runs label-drop quizzes, checking each placement against ground truth; *Clinical Consult* is invoked when the user clicks a structure, surfacing relevant pathology and imaging from KB.

**Agent-driven UI:** Guide drives layer peels via app_call `peel_layer` and pins structures; Examiner highlights an unlabeled structure and grades drops; Clinical Consult patches pathology notes into the inspector; presence narrates "Guide: peeling the deltoid to expose the axillary nerve."

**Declared actions:** `peel_layer(system)`, `pin_structure(id)`, `highlight_structure(id)`, `grade_label(id,tag)`, `cross_section(plane)`, `load_clinical(id)`.

**Signals (app→agent):** `structure_clicked(id)`, `label_dropped(id,tag)`, `view_rotated(angles)`, `quiz_pressed()`.

**User interactions:** User rotates the model, toggles off skeletal layer, drags "axillary nerve" label onto a structure, clicks a vessel for clinical notes.

**The bidirectional loop:** User presses `Quiz Me` → Examiner highlights a nerve, asks for its label → user drags the wrong tag → Examiner reddens it, narrates the miss → Guide peels an adjacent muscle to disambiguate → user re-drags correctly → Clinical Consult surfaces a nerve-injury case, mastery chip rises.

**Platform integration:** br.kb for pathology/imaging pages; scientific figures for cross-section plots; deep route for clinical reasoning; an `anatomy` skill.

### 88. Cipher Workshop

**Concept:** A cryptography breaking-bench where a ciphertext, frequency charts, and manipulable substitution grids are the surface, and agents suggest cribs, test keys, and flag contradictions while the user swaps letters and locks in mappings — a codebreaker's desk, not a chatbot.

**Domain & vibe:** Classical cryptography / puzzle-solving; the addictive click of a cracked column.

**Theme & aesthetic:** `terminal` theme; amber-on-black, teletype font, animated frequency bars, glitch flicker on a rejected key; grid-heavy.

**Layout:** Left 260px rail: cipher tools (frequency, bigram, Kasiski, crib list) as draggable analyzers; Center: the substitution grid + live-decrypting ciphertext pane; Right 340px inspector: current key map, confidence per letter, candidate cribs; Bottom 64px transport bar: `Test Key`, `Lock Mapping`, `Auto-solve`, `Reveal Hint`; floating crack-meter top-right.

**Agents (multi-agent):** *Cryptanalyst* proposes probable plaintext cribs and candidate letter mappings; *Skeptic* stress-tests each mapping against the full ciphertext and flags contradictions; *Hint-smith* offers a graded nudge only when the user requests it, never dumping the key.

**Agent-driven UI:** Cryptanalyst fills tentative cells in @region:grid via app_call `propose_mapping`; Skeptic reddens contradictory positions in the ciphertext pane; presence narrates "Cryptanalyst: 'THE' fits columns 4-6; try E→X."

**Declared actions:** `propose_mapping(cipher,plain)`, `lock_cell(cipher,plain)`, `flag_contradiction(pos,reason)`, `run_analyzer(kind)`, `highlight_crib(span)`, `reveal_hint(level)`.

**Signals (app→agent):** `cell_edited(cipher,plain)`, `analyzer_dropped(kind)`, `mapping_locked(cipher)`, `hint_pressed()`.

**User interactions:** User drags the frequency analyzer onto the text, types a letter into a grid cell, locks a confident mapping, presses `Test Key`.

**The bidirectional loop:** User maps X→E from frequency → Skeptic finds the digraph "EE" appearing where impossible, reddens it → Cryptanalyst consults, proposes X→T instead with a crib → user locks it → the ciphertext pane re-decrypts, three words resolve, crack-meter jumps.

**Platform integration:** local model route for fast frequency reasoning; deep route for crib inference; scientific figures for frequency plots; a `cryptography` skill.

### 89. Ecosystem Terrarium

**Concept:** A living food-web simulator where the agent tunes population dynamics on a network canvas and the user drags species, sets rates, and culls links while agents predict cascades and flag collapses — a manipulable simulation, not a chat.

**Domain & vibe:** Ecology / systems biology; the anxious wonder of watching a web wobble toward collapse.

**Theme & aesthetic:** `lab-notebook` theme; graph-paper base, organic node illustrations, animated population pulses along edges, red bloom on an extinction; field-guide callouts.

**Layout:** Left 260px rail: species drawer + parameter sliders (birth, predation, carrying capacity) as draggable items; Center: the force-graph food web with animated edge flows; Right 340px inspector: selected-species population curve (plot widget) + trophic role; Bottom 64px transport bar: `Run Sim`, `Step`, `Perturb`, `Snapshot`; floating stability index top-right.

**Agents (multi-agent):** *Modeler* wires the Lotka-Volterra couplings and stages initial parameters; *Forecaster* runs the ODE forward and predicts which species crash under the current graph; *Intervention Coach* proposes minimal edits (add a predator, cap a rate) to stabilize a collapsing web.

**Agent-driven UI:** Modeler patches nodes/edges into @region:web; Forecaster animates population pulses and reddens crashing nodes; Intervention Coach ghosts a suggested new link; presence narrates "Forecaster: remove the apex and herbivores boom, then starve by t=40."

**Declared actions:** `add_species(name,role)`, `couple(a,b,rate)`, `set_param(id,key,val)`, `run_sim(horizon)`, `flag_collapse(id,time)`, `suggest_intervention(edit)`.

**Signals (app→agent):** `species_dropped(name)`, `edge_drawn(a,b)`, `slider_changed(id,key,val)`, `perturb_pressed(node)`.

**User interactions:** User drags a wolf onto the web, wires a predation edge, drags a birth-rate slider, presses `Run Sim`, culls a link to test resilience.

**The bidirectional loop:** User deletes the apex predator → Forecaster runs forward, reddens the herbivore then the plant node as it overgrazes and starves → Intervention Coach ghosts a mid-tier predator link → user drags it solid, sets its rate → Forecaster re-runs, stability index recovers to green.

**Platform integration:** local route for fast ODE steps; deep route for intervention search; scientific figures for population curves; an `ecology`/`systems-biology` skill.

### 90. Kernel Foundry

**Concept:** A hands-on OS/algorithm lab where memory, scheduler queues, and page tables are manipulable board widgets, and agents step the machine, inject faults, and check the learner's traces while the user drags processes and evicts pages — a machine-state board, not a chatbot.

**Domain & vibe:** Computer systems / OS internals; the crisp gratification of a correctly traced context-switch.

**Theme & aesthetic:** `midnight` theme; dark slate, hex-address monospace, register cells that flash cyan on write, tick-driven motion; near-zero chrome, coral only on a fault.

**Layout:** Left 260px rail: process/instruction bank + policy palette (FIFO, LRU, round-robin) as draggable chips; Center: the machine board — ready/wait queues, RAM frame grid, page table; Right 340px inspector: selected-frame contents, TLB state, cycle counter; Bottom 64px transport bar: `Step Cycle`, `Run`, `Inject Fault`, `Grade Trace`; floating fault-count chip top-right.

**Agents (multi-agent):** *Scheduler Coach* stages the next scheduling decision under the chosen policy and ghosts the move; *Fault Examiner* injects page faults / race conditions and checks the user's eviction/scheduling response against the policy's ground truth; *Trace Grader* consults on `Grade Trace`, diffing the user's cycle-by-cycle log against the correct execution.

**Agent-driven UI:** Coach ghosts the next queue move via app_call `stage_move`; Fault Examiner flashes a coral fault on a page-table row and narrates; Trace Grader annotates each mistraced cycle; presence narrates "Fault Examiner: page 0x2F faults — pick a victim under LRU."

**Declared actions:** `stage_move(process,queue)`, `step_cycle()`, `inject_fault(kind,addr)`, `evict_page(frame)`, `grade_trace(log)`, `highlight_frame(id,reason)`.

**Signals (app→agent):** `process_dragged(id,queue)`, `frame_clicked(id)`, `policy_dropped(kind)`, `step_pressed()`.

**User interactions:** User drags a process into the ready queue, drops the LRU policy chip, evicts a frame by clicking it, presses `Step Cycle`.

**The bidirectional loop:** User sets round-robin and steps → Fault Examiner injects a page fault mid-quantum, flashes the frame coral → user evicts the wrong page → Fault Examiner reddens it, narrates the LRU violation → Coach ghosts the correct victim → user re-evicts, `Grade Trace` shows the cycle log turning green.

**Platform integration:** local route for fast machine-stepping; deep route for trace grading; scientific figures for queue-occupancy plots; an `os-internals` skill.

## 10. Decision cockpits & mixed-initiative editors  ·  #91–100

### 91. Roadmap Regatta

**Concept:** A quarters-as-lanes portfolio studio where the agent proposes epic placements onto a swimlane board and runs capacity what-ifs; the board — not a chat — is the product.

**Domain & vibe:** Product/portfolio planning; calm-but-high-stakes, "steering a fleet."

**Theme & aesthetic:** `journal` pack; serif headers, generous whitespace, hand-inked lane dividers, ink-blue accents that turn amber only on over-capacity lanes; subtle 120ms card-glide motion.

**Layout:** Left 260px rail: initiative backlog (draggable epic cards, filter chips). Center: 5-lane quarter board (Now→Q4), each lane a capacity meter header. Right 340px inspector: selected-epic dependencies, effort sliders, risk notes. Bottom 64px transport bar: **Run What-If**, **Rebalance**, **Lock Lane** buttons. Floating top-right: presence chip narrating agent moves.

**Agents (multi-agent):** *Cartographer* proposes epic-to-quarter placements from backlog + dependencies; *Loadmaster* checks each lane's capacity and flags overflow; *Redliner* adversarially stress-tests the plan against a chosen risk scenario and hands accepted changes back to Cartographer.

**Agent-driven UI:** Agent patches epic cards into lanes via ui_patch into `@region:board`, recolors over-capacity lane headers amber, draws dependency arcs, and writes a rationale into the inspector; presence narrates "Loadmaster moved 2 epics out of Q2."

**Declared actions:** `place_epic(id,lane)`, `set_capacity(lane,points)`, `draw_dependency(from,to)`, `flag_overflow(lane)`, `stage_layout(preset)`, `annotate(epicId,text)`, `run_whatif(scenarioId)`.

**Signals (app→agent):** `epic_dragged(id,lane)`, `slider_changed(epicId,effort)`, `lane_locked(lane)`, `scenario_selected(id)`.

**User interactions:** Drag epics between lanes, resize effort sliders, lock a lane to pin it, click **Run What-If** to launch a scenario, click arcs to inspect dependencies.

**The bidirectional loop:** User drags "Billing v2" into Q2 → Cartographer reasons dependencies, consults Loadmaster who finds Q2 at 130% → agent recolors the lane amber, bumps a dependent epic to Q3, narrates the swap → user locks Q2 and hits **Run What-If: hiring freeze** → Redliner re-runs, patches two cards red and appends a risk note to the inspector.

**Platform integration:** model routes (fast for placement, deep for red-team), br.kb for prior roadmap retros, workflows to export the locked plan, kpi widget for lane load.

### 92. Tradeoff Radar

**Concept:** A vendor/architecture decision cockpit built around a live radar chart and constraint sliders that the agent and user jointly manipulate — a decision instrument, not a Q&A box.

**Domain & vibe:** Technical/procurement decisions; analytical, cool-headed, "mission-console."

**Theme & aesthetic:** `terminal` pack; monospaced, dense grid, near-zero chrome, phosphor-green baseline with coral only on breached constraints; crisp 80ms tick motion.

**Layout:** Left 240px rail: candidate options list with select toggles. Center: radar chart (figure widget) overlaying selected options across 6 criteria axes. Right 320px inspector: per-criterion weight sliders + hard-constraint thresholds. Bottom 64px transport bar: **Score**, **Add Constraint**, **Nominate Winner**. Floating top-right: presence chip.

**Agents (multi-agent):** *Sourcer* pulls candidate options and fills criteria scores from KB; *Weigher* proposes criterion weights and recomputes the radar; *Contrarian* argues the strongest case for the current runner-up and surfaces overlooked risks.

**Agent-driven UI:** Agent repaints the radar figure via app_call, drags weight sliders visibly, marks breached thresholds coral in `@region:inspector`, and writes a nomination rationale card; presence narrates "Weigher raised 'latency' to 0.3, reranking B above A."

**Declared actions:** `load_candidates(query)`, `set_weight(criterion,val)`, `set_threshold(criterion,op,val)`, `recompute_radar()`, `highlight_breach(criterion)`, `nominate(optionId,rationale)`, `run_sensitivity(criterion)`.

**Signals (app→agent):** `option_toggled(id)`, `weight_dragged(criterion,val)`, `threshold_edited(criterion,val)`, `nominate_clicked(id)`.

**User interactions:** Toggle candidates onto the radar, drag weight sliders, type a hard threshold, click **Run Sensitivity**, accept/override a nomination.

**The bidirectional loop:** User toggles three DB vendors → Sourcer fills scores from KB, Weigher renders the radar → user drags "cost" weight up → radar redraws, one option breaches a latency threshold coral → Contrarian consults deep route, argues the breached option still wins on durability, patches a counter-card → user pins its threshold and clicks **Nominate Winner**.

**Platform integration:** figure (radar), br.kb vendor briefs, model routes (deep for Contrarian), skills for scoring rubric, table export of the decision matrix.

### 93. Redline Room

**Concept:** A screen-first document co-editor where margin agents propose edits as tracked suggestions on a live document canvas; the user accepts/rejects by direct manipulation — never by chatting.

**Domain & vibe:** Contract/grant/policy drafting; focused, exacting, "editorial war room."

**Theme & aesthetic:** `lab-notebook` pack; ruled paper, tight leading, quiet grays with a single red for redlines and green for accepted; margin notes slide in at 100ms.

**Layout:** Center: A4 document canvas with inline tracked changes. Left 220px rail: section outline + clause jump-list. Right 360px margin-agent column: stacked suggestion cards (accept/reject/rewrite). Bottom 64px transport bar: **Run Pass**, **Compare Versions**, **Freeze Section**. Floating top-right: presence chip.

**Agents (multi-agent):** *Drafter* proposes clause rewrites for clarity/coverage; *Compliance* checks each clause against a policy KB and flags gaps; *Adversary* reads clauses as an opposing counterparty and inserts exploit-the-loophole objections that route back to Drafter.

**Agent-driven UI:** Agent injects tracked-change spans into `@region:document` via ui_patch, stacks suggestion cards in the margin, highlights the active clause, and links each card to its source policy page; presence narrates "Compliance flagged §4.2 as under-specified."

**Declared actions:** `propose_edit(span,newText,rationale)`, `flag_clause(span,severity)`, `accept_edit(id)`, `reject_edit(id)`, `jump_to(clauseId)`, `diff_versions(a,b)`, `cite_policy(clauseId,pageId)`.

**Signals (app→agent):** `edit_accepted(id)`, `edit_rejected(id)`, `span_selected(range)`, `section_frozen(id)`.

**User interactions:** Select text to request a rewrite, click accept/reject on margin cards, drag clauses to reorder, freeze a finalized section, click **Compare Versions**.

**The bidirectional loop:** User selects §4.2 → Drafter proposes a tighter clause, Compliance consults the policy KB and flags a missing indemnity → margin shows two stacked cards, the clause highlights amber → user accepts the rewrite but rejects the indemnity add → Adversary re-reads, finds a residual loophole, patches a new red objection card → user freezes the section once resolved.

**Platform integration:** br.kb policy corpus, model routes (deep for Adversary), skills (anti-ai-writing for tone), workflows to export the finalized doc, table for a clause-coverage matrix.

### 94. Scenario Matrix

**Concept:** A strategy cockpit where futures are cells in a driver×outcome grid; the agent fills, cross-links, and stress-tests cells while the user pins and reweights — a matrix instrument, not a chat transcript.

**Domain & vibe:** Corporate/geopolitical scenario planning; contemplative, weighty, "situation-room."

**Theme & aesthetic:** `midnight` pack; deep navy, luminous cell borders, low-glare; probability shown as cell fill; teal accent, magenta only on wildcard cells; slow 150ms cell-bloom motion.

**Layout:** Center: N×M scenario matrix (custom grid component), rows=key drivers, cols=time horizons. Left 260px rail: driver library + assumptions. Right 340px inspector: selected-cell narrative, probability slider, signposts. Bottom 64px transport bar: **Populate**, **Stress-Test**, **Collapse to Bets**. Floating top-right: presence chip.

**Agents (multi-agent):** *Futurist* populates cells with scenario narratives from drivers; *Statistician* assigns/updates cell probabilities and normalizes rows; *Wildcarder* injects low-probability high-impact shocks and re-scores affected cells.

**Agent-driven UI:** Agent writes narratives into `@region:matrix` cells via ui_patch, sets fill opacity to probability, links causally-related cells with edges, magenta-flags wildcards, and writes signposts to the inspector; presence narrates "Wildcarder inserted a supply-shock into H2."

**Declared actions:** `populate_cell(row,col,text)`, `set_probability(cell,p)`, `link_cells(a,b)`, `inject_wildcard(cell)`, `collapse_to_bets(threshold)`, `annotate(cell,signpost)`, `stress_test(driverId)`.

**Signals (app→agent):** `cell_selected(r,c)`, `probability_dragged(cell,p)`, `cell_pinned(cell)`, `driver_added(id)`.

**User interactions:** Click cells to read/edit narratives, drag probability sliders, pin cells to protect them, add a driver, click **Collapse to Bets**.

**The bidirectional loop:** User adds driver "AI regulation" → Futurist populates a new row of cells, Statistician assigns probabilities → user drags H3's probability up → row renormalizes, a downstream cell dims → Wildcarder consults deep route, injects a magenta shock and re-links two cells, narrating the cascade → user pins the shock and clicks **Collapse to Bets** to distill three actionable wagers.

**Platform integration:** br.kb signal library, model routes (deep for stress-tests), figure for a probability heatmap, workflows to export bets, kpi for portfolio-of-bets confidence.

### 95. Negotiation Table

**Concept:** A live deal-simulation cockpit where offers are cards on a shared table between two party columns; agents role-play counterparties and a mediator while the user moves cards and sets walkaways — a table you operate, not a chat.

**Domain & vibe:** Deal-making / labor / M&A negotiation; tense, strategic, "war-game."

**Theme & aesthetic:** `clinical` pack; clean white, sharp dividers, blue/red party tint, amber on impasse; ZOPA band rendered as a live bar; 90ms card-slide.

**Layout:** Center: negotiation table split into **Us | Table | Them** columns with offer/counter cards. Left 240px rail: issues list with priorities + BATNA. Right 320px inspector: selected-issue tradeoff sliders, walkaway thresholds. Bottom 64px transport bar: **Send Offer**, **Simulate Reply**, **Find ZOPA**. Floating top-right: presence chip.

**Agents (multi-agent):** *Counterpart* plays "Them," generating realistic counters from their inferred interests; *Mediator* proposes package deals bridging both sides; *Coach* privately red-teams the user's move and warns of concessions given away.

**Agent-driven UI:** Agent deals counter cards into the **Them** column via ui_patch, draws the ZOPA band, highlights the contested issue amber, and writes Coach warnings to `@region:inspector`; presence narrates "Counterpart rejected on term length, held on price."

**Declared actions:** `send_offer(package)`, `generate_counter(package)`, `propose_package(issues)`, `compute_zopa()`, `highlight_issue(id)`, `set_walkaway(issue,val)`, `simulate_round(rounds)`.

**Signals (app→agent):** `card_moved(id,column)`, `slider_changed(issue,val)`, `offer_sent(package)`, `walkaway_set(issue,val)`.

**User interactions:** Drag offer cards between columns, tune tradeoff sliders, set walkaways, click **Simulate Reply**, accept a Mediator package.

**The bidirectional loop:** User drags a price+term package to the table and clicks **Send Offer** → Counterpart infers interests and deals a counter into **Them**, Mediator computes ZOPA showing overlap on price only → contested "term length" glows amber → Coach warns the user overpaid on a low-priority issue → user retightens a slider and clicks **Simulate Reply**; Mediator patches a bridging package both accept.

**Platform integration:** model routes (deep for Counterpart realism), br.kb comparable-deals, figure for ZOPA/utility frontier, skills for negotiation tactics, workflows to export the term sheet.

### 96. Budget Loom

**Concept:** A budget/tradeoff explorer where money flows are a Sankey the agent reshapes as you drag envelope sliders; a fiscal instrument the user weaves, not a chatbot that recites numbers.

**Domain & vibe:** Budget & resource allocation; deliberate, accountable, "treasury desk."

**Theme & aesthetic:** `biorouter` pack; airy, rounded, teal→violet flow gradient; over-budget flows pulse coral; 110ms flow-retween.

**Layout:** Center: Sankey flow diagram (figure) sources→programs→outcomes. Left 260px rail: envelope list with allocation sliders + locks. Right 340px inspector: selected-flow assumptions, ROI estimate, cut/boost impact. Bottom 64px transport bar: **Balance**, **Optimize for Outcome**, **Sensitivity**. Floating top-right: presence chip.

**Agents (multi-agent):** *Allocator* proposes envelope splits toward a stated goal; *Auditor* checks totals against the cap and flags waste/duplication; *Optimizer* runs constrained what-ifs to maximize a chosen outcome and hands the frontier back.

**Agent-driven UI:** Agent reflows the Sankey via app_call, drags allocation sliders visibly, pulses over-budget flows coral, and writes ROI cards into `@region:inspector`; presence narrates "Optimizer shifted $2M from Ops to Prevention for +8% coverage."

**Declared actions:** `set_allocation(envelope,amount)`, `reflow_sankey()`, `flag_overrun(envelope)`, `optimize_for(outcome,cap)`, `lock_envelope(id)`, `annotate(flowId,text)`, `run_sensitivity(envelope)`.

**Signals (app→agent):** `slider_dragged(envelope,amount)`, `envelope_locked(id)`, `flow_selected(id)`, `outcome_target_set(id)`.

**User interactions:** Drag envelope sliders, lock protected programs, click flows to inspect ROI, set an outcome target, click **Optimize for Outcome**.

**The bidirectional loop:** User drags "Prevention" up → Sankey reflows, total exceeds cap, an Ops flow pulses coral → Auditor flags the overrun and a duplicate line item → user locks Prevention and clicks **Optimize for Outcome: coverage** → Optimizer runs a constrained what-if, redistributes from the duplicate, narrates the frontier → user accepts and clicks **Sensitivity** to see fragility.

**Platform integration:** figure (Sankey + tornado sensitivity), model routes (deep for optimization), br.kb prior-year budgets, workflows to export the allocation, kpi for outcome coverage.

### 97. Decision Room

**Concept:** A meeting-decision cockpit that turns a live discussion into a decision board of options, evidence, and votes the agents curate in real time — a shared decision surface, not a meeting chatbot.

**Domain & vibe:** Team/committee decision-making; brisk, facilitated, "control-room for consensus."

**Theme & aesthetic:** `biorouter` pack, light; card-dense, quiet dividers, green on quorum, amber on unresolved objection; 90ms card-settle.

**Layout:** Center: decision board — columns **Options | Evidence | Objections | Decision**. Left 240px rail: agenda items + timer. Right 340px inspector: selected-option pros/cons, vote tally, owner. Bottom 64px transport bar: **Cluster**, **Call Vote**, **Record Decision**. Floating top-right: presence chip.

**Agents (multi-agent):** *Scribe* extracts options and evidence into columns; *Devil* generates the strongest objection to each leading option; *Facilitator* detects stalls, proposes a vote, and writes the recorded decision with owner + due date.

**Agent-driven UI:** Agent populates board columns via ui_patch, links evidence to options, amber-flags unresolved objections, tallies votes live, and writes the ratified decision card to `@region:decision`; presence narrates "Devil raised a cost objection to Option B; unresolved."

**Declared actions:** `add_option(text)`, `attach_evidence(optionId,pageId)`, `raise_objection(optionId,text)`, `cluster_options()`, `call_vote(optionId)`, `record_decision(optionId,owner,due)`, `highlight_stall()`.

**Signals (app→agent):** `option_voted(id,user)`, `objection_resolved(id)`, `card_moved(id,column)`, `agenda_advanced(id)`.

**User interactions:** Drag cards between columns, vote on options, mark objections resolved, click **Cluster**, click **Record Decision**.

**The bidirectional loop:** User adds three options → Scribe attaches evidence pages, Devil raises an objection on each → user drags Option B's objection to resolved after debate → Facilitator detects consensus forming, proposes **Call Vote** → users vote, quorum turns green → Facilitator writes the decision card with owner + due date and narrates the handoff to a follow-up workflow.

**Platform integration:** br.kb evidence retrieval, model routes (fast for Scribe, deep for Devil), workflows to file the decision + action items, table for the vote tally, log widget for the meeting trail.

### 98. Portfolio Kanban Arena

**Concept:** A cross-team kanban cockpit where the agent balances WIP, sequences by value, and flags flow blockers across many swimlanes while the user drags cards — a flow-management board, not a chat helper.

**Domain & vibe:** Delivery/ops portfolio management; kinetic, operational, "ops bridge."

**Theme & aesthetic:** `terminal` pack, dark; monospaced tickets, WIP counters per column, coral on WIP breach, cyan on aging cards; 70ms snap motion.

**Layout:** Center: multi-lane kanban (Backlog→Discovery→Build→Review→Done), WIP-limit headers. Left 240px rail: teams + value filters. Right 320px inspector: selected-card cycle-time, blockers, dependencies. Bottom 64px transport bar: **Sequence by Value**, **Detect Blockers**, **Simulate Throughput**. Floating top-right: presence chip.

**Agents (multi-agent):** *Sequencer* orders cards by value/effort and proposes pulls; *Bottleneck* detects WIP breaches and aging cards, flags blockers; *Forecaster* runs a Monte-Carlo throughput simulation and reports a delivery-date distribution.

**Agent-driven UI:** Agent reorders cards and moves pulls via ui_patch, recolors breached columns coral, tags aging cards cyan, draws dependency links, and writes forecasts to `@region:inspector`; presence narrates "Bottleneck: Review at WIP 6/4, two cards aging."

**Declared actions:** `move_card(id,column)`, `reorder_lane(lane,order)`, `set_wip_limit(column,n)`, `flag_blocker(cardId,reason)`, `sequence_by_value()`, `simulate_throughput(runs)`, `highlight_aging(threshold)`.

**Signals (app→agent):** `card_dragged(id,column)`, `wip_edited(column,n)`, `card_selected(id)`, `value_filter_changed(id)`.

**User interactions:** Drag cards across columns, set WIP limits, filter by team/value, click **Sequence by Value**, click **Simulate Throughput**.

**The bidirectional loop:** User drags a card into Review → Bottleneck detects Review now 6/4, recolors it coral and flags two aging cards → Sequencer proposes pulling a high-value card forward and reorders the lane → user accepts the reorder but raises the Review WIP limit instead → Forecaster reruns Monte-Carlo, patches an updated delivery-date distribution and narrates the shift.

**Platform integration:** model routes (fast for sequencing, deep for forecasting), figure (throughput distribution), br.kb delivery history, workflows to sync the board, kpi for cycle-time.

### 99. Policy Sandbox

**Concept:** A policy-simulation dashboard where the user turns policy levers and the agent recomputes population outcomes on a live map + curve panel, red-teaming for unintended effects — a simulation console, not a chatbot.

**Domain & vibe:** Public-health / economic policy; sober, evidence-forward, "operations center."

**Theme & aesthetic:** `clinical` pack; restrained, high-contrast, blue baseline, coral on harm indicators; choropleth + curves; 100ms curve-tween.

**Layout:** Center split: top choropleth map (geo widget), bottom outcome curves (figure). Left 260px rail: policy levers (sliders/toggles) + population segments. Right 340px inspector: selected-region outcomes, equity breakdown, assumptions. Bottom 64px transport bar: **Run Model**, **Red-Team Effects**, **Compare Policies**. Floating top-right: presence chip.

**Agents (multi-agent):** *Modeler* maps levers to outcomes and recomputes the map + curves; *Equity Auditor* checks distributional impact across segments and flags regressive effects; *Skeptic* red-teams for unintended consequences and second-order effects.

**Agent-driven UI:** Agent recolors the choropleth and retweens curves via app_call, coral-flags harmed regions/segments, writes an equity breakdown into `@region:inspector`, and pins assumption cards; presence narrates "Equity Auditor: benefit concentrated in top quintile."

**Declared actions:** `set_lever(id,val)`, `run_model()`, `recolor_map(metric)`, `update_curves(scenario)`, `flag_inequity(segment)`, `red_team()`, `compare_policies(a,b)`, `annotate(regionId,text)`.

**Signals (app→agent):** `lever_changed(id,val)`, `region_clicked(id)`, `segment_selected(id)`, `compare_clicked(a,b)`.

**User interactions:** Drag policy-lever sliders, click map regions to drill in, select population segments, click **Run Model**, click **Compare Policies**.

**The bidirectional loop:** User raises a subsidy lever → Modeler recomputes, choropleth greens in cities, curves lift → Equity Auditor finds rural regions worse off, coral-flags them and writes an equity card → user clicks a coral region to drill in → Skeptic consults deep route, surfaces a second-order labor effect, pins an assumption card and narrates the caveat → user tweaks a second lever and clicks **Compare Policies**.

**Platform integration:** geo + figure widgets, br.kb evidence + parameter priors, model routes (deep for red-team), skills (clinical-biostatistics), workflows to export a policy brief.

### 100. Resume Atelier

**Concept:** A screen-first resume/portfolio co-editor where a panel of hiring-lens agents rewrites, reorders, and scores sections on a live document canvas against a target role — you sculpt the page, you don't chat about it.

**Domain & vibe:** Career/document crafting; encouraging yet exacting, "design studio."

**Theme & aesthetic:** `journal` pack; elegant serif/sans mix, ample margins, one warm accent, green on strengthened bullets, gray on weak ones; 100ms section-lift.

**Layout:** Center: live resume canvas (author-drawn, section blocks). Left 240px rail: target-role picker + section outline. Right 360px critique column: per-section score cards + suggested rewrites. Bottom 64px transport bar: **Tailor to Role**, **Strengthen Bullets**, **ATS Check**. Floating top-right: presence chip.

**Agents (multi-agent):** *Recruiter* scores each section against the target role's rubric; *Editor* rewrites weak bullets into quantified impact statements; *ATS-Bot* checks keyword coverage and formatting parseability, flagging gaps back to Editor.

**Agent-driven UI:** Agent patches rewritten bullets into `@region:resume` as tracked suggestions, scores section headers (green/gray), highlights missing keywords, and stacks critique cards; presence narrates "Recruiter: Experience 6/10 — impact under-quantified."

**Declared actions:** `score_section(id,role)`, `rewrite_bullet(span,newText)`, `reorder_sections(order)`, `check_ats(role)`, `highlight_keyword_gap(term)`, `tailor_to_role(roleId)`, `annotate(span,tip)`.

**Signals (app→agent):** `bullet_accepted(id)`, `section_reordered(order)`, `role_selected(id)`, `span_selected(range)`.

**User interactions:** Pick a target role, drag sections to reorder, accept/reject bullet rewrites, click a weak bullet to request a stronger one, click **ATS Check**.

**The bidirectional loop:** User picks "Staff ML Engineer" → Recruiter scores each section, marks Experience gray → Editor rewrites two bullets into quantified impact and patches them as suggestions → user accepts one, rejects one and clicks it for another pass → ATS-Bot finds three missing keywords, highlights the gaps and narrates → user drags Skills above Experience and clicks **Tailor to Role** for a final sweep.

**Platform integration:** br.kb role rubrics + exemplar resumes, model routes (deep for rewrites), skills (anti-ai-writing, frontend-design for layout), workflows to export PDF, table for the ATS keyword matrix.

## Related documentation

- [Application Drafter 100-app test-drive runbook](app-test-drive-runbook.md) — the procedure that consumes this corpus: environment setup, authoring loop, rubrics and findings log.
- [100-app test-drive archive](../../history/agent-drafter-testdrive-100/README.md) — per-app results, screenshots and blockers from the campaign run against these specs.
- [Apps SDK reference](../../apps-sdk/sdk-reference.md) — the signatures behind every `br.*`, `ui_*` and `app_call` term used in these briefs.
- [Apps SDK v2 design](../../apps-sdk/v2-design.md) — why the SDK exposes the surface these briefs assume.
- [Applications platform design](../apps-platform-design.md) — how an app, its manifest and its per-app agent fit together.
