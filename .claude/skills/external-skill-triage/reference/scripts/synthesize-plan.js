export const meta = {
  name: 'external-skill-triage-plan',
  description: 'Merge every triage stream into one BAAM implementation plan: optional catalog audit, synthesis, completeness and value/licence critics, final plan',
  whenToUse: 'After every triage stream has its stream report. All paths come in args.',
  phases: [
    { title: 'Catalog audit', detail: 'optional: Biorouter catalog self-triage in slices' },
    { title: 'Plan', detail: 'one plan with one winner per job' },
    { title: 'Critics', detail: 'completeness, value and licence' },
    { title: 'Final plan', detail: 'decide every critic finding, rewrite the plan' },
  ],
}

// args (absolute paths):
// {
//   out: '/abs/biorouter-runs/skill-triage-<date>',          // writes plan/ and catalog/ here
//   skillsRepo: '/abs/clone of biorouter-skills at its remote main (read only)',
//   landing: '/abs/biorouter worktree at origin/main/landing (read only)',
//   skillDir: '/abs/<biorouter worktree>/.claude/skills/external-skill-triage',   // optional
//   goal: 'what the owner asked for',
//   inputs: ['/abs/.../research', '/abs/.../ecosystem/ECOSYSTEM-REPORT.md', ...], // stream folders or report files
//   priorPlan: '/abs/.../plan/triage-plan.v1.json',          // optional, folded in
//   ownerDecisions: ['decided: ...', 'open: ...'],
//   auditSlices: [{ label: 'core 1', filter: 'the first 23 core items in catalog order' }],   // optional
// }

const A = args || {}
for (const k of ['out', 'skillsRepo', 'landing', 'goal', 'inputs']) if (!A[k]) throw new Error(`args.${k} is required`)
const OUT = A.out
const FULL = A.skillDir ? ` The triage rules, licence policy and catalog conventions are in ${A.skillDir}/SKILL.md and ${A.skillDir}/reference/; read them.` : ''
const COMMON = `
Context: Biorouter (UCSF Baranzini Lab) ships BAAM skills from ${A.skillsRepo} (catalog.json; README section "The catalog" explains rows and rules D1 to D6 and G1) and extensions listed in ${A.landing}/catalog/extensions.json, with the job vocabulary, built-in jobs and required jobs in ${A.landing}/catalog/jobs.json. Read freely; modify nothing there.
Owner's goal: ${A.goal} One strong skill per job: enrich the job's owner before adding, add only where no active item owns the job, avoid catalog bloat.
Rules: plain prose with no em dashes; never write the name of the project Biorouter was derived from or its publisher; never print credentials. Adapt with attribution only from permissive sources (MIT, Apache-2.0, BSD, ISC, CC0) whose lineage is also permissive; CC BY 4.0 waits for an owner decision; everything else is ideas only. Every adopted item carries the security conditions its independent review set. Replacing an item is a minor version bump. If the harness refuses a Write of a report or plan file, return the full text in the schema's markdown field. Your final message is data for an orchestrator.${FULL}
Owner decisions so far: ${(A.ownerDecisions || []).join(' | ') || 'none recorded'}`

const FINDINGS = { type: 'object', properties: { report_path: { type: 'string' }, markdown: { type: 'string' }, findings: { type: 'array', items: { type: 'object', properties: {
  id: { type: 'string' }, ids: { type: 'array', items: { type: 'string' } }, issue: { type: 'string' }, detail: { type: 'string' }, evidence: { type: 'string' }, proposal: { type: 'string' },
  value: { type: 'string', enum: ['high', 'medium', 'low'] } }, required: ['id', 'issue', 'detail', 'proposal'] } } }, required: ['findings'] }
const ITEM = { type: 'object', properties: {
  wid: { type: 'string' }, type: { type: 'string', enum: ['new', 'enrich', 'merge', 'retire', 'fix'] }, kind: { type: 'string', enum: ['skill', 'bundle-component', 'extension'] },
  title: { type: 'string' }, target_ids: { type: 'array', items: { type: 'string' } }, new_id: { type: 'string' }, category: { type: 'string' }, job: { type: 'string' },
  folders: { type: 'array', items: { type: 'string' } }, rationale: { type: 'string' }, scope: { type: 'string' },
  sources: { type: 'array', items: { type: 'object', properties: { repo: { type: 'string' }, commit: { type: 'string' }, paths: { type: 'array', items: { type: 'string' } }, licence: { type: 'string' }, reuse: { type: 'string', enum: ['adapt-with-attribution', 'ideas-only'] } }, required: ['repo', 'reuse'] } },
  security_conditions: { type: 'string' }, distinct_from: { type: 'array', items: { type: 'string' } }, depends_on: { type: 'array', items: { type: 'string' } },
  owner_decision: { type: 'string' }, priority: { type: 'string', enum: ['p1', 'p2', 'p3'] } },
  required: ['wid', 'type', 'kind', 'title', 'folders', 'rationale', 'scope', 'priority'] }
const PLAN = { type: 'object', properties: { plan_path: { type: 'string' }, json_path: { type: 'string' }, markdown: { type: 'string' }, summary: { type: 'string' },
  work_items: { type: 'array', items: ITEM }, owner_decisions_needed: { type: 'array', items: { type: 'string' } },
  decisions: { type: 'array', items: { type: 'object', properties: { finding: { type: 'string' }, decision: { type: 'string', enum: ['accept', 'partly', 'reject'] }, reason: { type: 'string' } }, required: ['finding', 'decision', 'reason'] } } },
  required: ['plan_path', 'summary', 'work_items'] }

// Optional: audit Biorouter's own catalog in slices, in parallel (the plan needs all of them).
let audits = []
if ((A.auditSlices || []).length) {
  phase('Catalog audit')
  audits = (await parallel(A.auditSlices.map(s => () => agent(`${COMMON}
Audit a slice of Biorouter's own skill catalog: ${s.filter} of ${A.skillsRepo}/catalog.json. Read every SKILL.md in your slice (for a bundle, every component) and the summary, job and distinct_from of ALL items, so you see overlaps across the whole catalog. Find duplicates or near duplicates, unclear boundaries, thin skills (generic advice, little method), stale content you can confirm by reading, broken structure, and gaps a researcher would expect next to these items. Give each finding an id (${s.label.replace(/\s+/g, '-')}-F1, ...) and a concrete fix: merge X into Y, enrich X with named content, sharpen a boundary, retire, or add a skill. Edit nothing. Write ${OUT}/catalog/${s.label.replace(/\s+/g, '-')}.md. Return the schema.`, { label: `audit ${s.label}`, phase: 'Catalog audit', schema: FINDINGS })))).filter(Boolean)
  log(`catalog audits: ${audits.length}/${A.auditSlices.length}`)
}

phase('Plan')
const draft = await agent(`${COMMON}
Synthesize ONE implementation plan for BAAM from every triage input. Read each in full: ${A.inputs.join(', ')} (for a folder, its *-REPORT.md and every <repo>/REPORT.md).${A.priorPlan ? ` Fold in the earlier plan ${A.priorPlan}, keeping its work item ids where an item survives.` : ''}${audits.length ? ` Catalog audit findings (files under ${OUT}/catalog/): ${JSON.stringify(audits.map(a => a.findings))}` : ''}
Produce work items, each self contained for one implementer: type new (proposed id checked against every id, alias, component name and built-in name; category; a job from jobs.json or a new one you name; distinct_from boundaries), enrich (the Biorouter id and the exact content to add with its sources), merge (which ids fold into which; aliases for top level items only; components keep their rows), retire, or fix. List the folders each item may touch; items that share a folder must say so in depends_on. Give every source its repo, pinned commit, paths, licence and reuse class, and every item the security conditions its independent review set. One winner per job, runners-up folded in; new extensions are their own items. Mark the owner decision an item waits on. Include a coverage map of the research lifecycle before and after the plan, and dependencies between items. Prioritise p1 (clear high value), p2, p3.
Write ${OUT}/plan/triage-plan.md and ${OUT}/plan/triage-plan.json. Return the schema.`, { label: 'synthesize plan', phase: 'Plan', schema: PLAN })
if (!draft) return { stopped: 'the synthesis agent returned nothing' }

phase('Critics')
const critics = await parallel([
  () => agent(`${COMMON}
Completeness critic. Read ${OUT}/plan/triage-plan.md, every input (${A.inputs.join(', ')}) and ${A.skillsRepo}/catalog.json. What is missing? A high value skill or enrichment a report found but the plan dropped; a research stage or required job with no owner; an item misjudged as skip; a source read only partly that the plan relies on; a catalog duplicate left unresolved; an enrichment too vague to implement. Verify each claim against the files. Give findings ids C1, C2, ... with evidence and a concrete proposal. Write ${OUT}/plan/critic-completeness.md. Return the schema.`, { label: 'critic completeness', phase: 'Critics', schema: FINDINGS }),
  () => agent(`${COMMON}
Value and licence critic. Read ${OUT}/plan/triage-plan.md and the sources it cites. Flag: bloat and near duplicates; enrichments that would dilute a focused skill or push it past a sensible size; any adapt item whose source, per item licence or upstream lineage is not permissive (including quotations of named authors and instrument text under NC-ND terms); ideas only items whose scope in practice needs copying; jobs that collide in jobs.json; ids that clash with ids, aliases, component names or built-in names; merges that need aliases or redirect stubs; required jobs whose owner change in jobs.json is missing. Verify each claim against the files. Give findings ids VL1, VL2, ... with evidence and a concrete proposal. Write ${OUT}/plan/critic-value-licence.md. Return the schema.`, { label: 'critic value+licence', phase: 'Critics', schema: FINDINGS }),
])
const found = critics.filter(Boolean)
const findingsText = JSON.stringify(found.map(c => c.findings))
log(`critic findings: ${found.reduce((n, c) => n + (c.findings || []).length, 0)}; passed whole, ${findingsText.length} chars`)

phase('Final plan')
const final = await agent(`${COMMON}
Revise the plan ${OUT}/plan/triage-plan.json in light of EVERY critic finding below (passed whole; the critics' notes are in ${OUT}/plan/critic-*.md). Accept, partly accept or reject each with one line of reasoning, after checking it against the files, and record the table in ${OUT}/plan/critique-decisions.md. Rewrite ${OUT}/plan/triage-plan.md and ${OUT}/plan/triage-plan.json as the FINAL plan (keep the earlier version as triage-plan.v1.*), keep the coverage map current, and list every owner decision still needed.
Critic findings: ${findingsText}
Return the schema with the final work items and the decisions.`, { label: 'final plan', phase: 'Final plan', schema: PLAN })
return { draft: { summary: draft.summary, items: (draft.work_items || []).length }, critics: found.map(c => (c.findings || []).length), final }
