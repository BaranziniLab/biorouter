{% raw -%}
Turn this conversation into a reusable **workflow**: a saved setup someone can start a fresh chat from and get the same kind of help again, without re-explaining anything.

Write the workflow for the NEXT run, not this one. Generalise away this conversation's specific values — the particular file, the particular date, the particular patient cohort — and turn the ones that will change into parameters.

Reply with _VALID_ JSON and nothing else. Keys:

- `title` — 5-10 words naming the task, not this chat.
- `description` — 1-2 sentences on what the workflow helps with.
- `instructions` — 1-2 paragraphs of standing instruction to the assistant. Say how to do the work, what output format is expected, and any non-standard tool the task depends on. Do not narrate what happened here.
- `activities` — 3-5 very short example prompts (a few words each) a user could click to start.
- `prompt` — OPTIONAL, and the difference between a workflow that can run unattended and one that cannot. The opening message that starts the work. Include it when the workflow has a clear job to do on each run; omit it when the user should say what they want first. Use `{{ parameter_key }}` to refer to a parameter.
- `parameters` — OPTIONAL. The few values a person would change on the next run (the input file, the subject, a date): usually one to three, never more than five. A setting that stays the same from run to run is written into the text, not made a parameter. Reference them from `prompt` and `instructions` as `{{ key }}`. Each is an object with:
  - `key` — snake_case identifier.
  - `input_type` — one of `string`, `number`, `boolean`, `date`, `file`, `select`.
  - `requirement` — `required`, `optional`, or `user_prompt` (ask the user each run).
  - `description` — what to put here, written for whoever runs it.
  - `default` — only for `optional`. Never for `file`.
  - `options` — the allowed values; REQUIRED when `input_type` is `select`, and a `select` parameter's `default` must be one of them.
- `skills` — OPTIONAL. Exact names of skills this conversation clearly used or needed. Use `[]` when none are required; do not guess.

Leave a key out rather than inventing a value for it. An empty `parameters` list is better than parameters nobody will fill in.

If this conversation declared a phase plan, phase gates or decision monitors, they are saved with the workflow as they are, and the next run starts with them in place. Do not restate them in `instructions` or `prompt`, and make no parameter for their phases, gate questions, thresholds or models: say only what the work is.

Example — a conversation that ended up pulling a gene's disease associations out of SPOKE and writing them up:

{
  "title": "Gene-disease association summary from SPOKE",
  "description": "Looks up a gene in the SPOKE knowledge graph and writes a short, sourced summary of the diseases it is associated with.",
  "instructions": "Query SPOKE for the named gene and collect its disease associations. Report the strongest associations first, and give the supporting evidence type for each. Write for a biomedical researcher who does not know the graph schema: name the diseases in plain language, and note when an association rests on a single source. Finish with a short 'what this does not show' paragraph. If the gene is not in the graph, say so plainly rather than substituting a similar one.",
  "activities": [
    "Summarise APOE",
    "Compare two genes",
    "Show the evidence types",
    "Export as markdown"
  ],
  "prompt": "Summarise the disease associations for {{ gene_symbol }} at a {{ detail_level }} level of detail.",
  "parameters": [
    {
      "key": "gene_symbol",
      "input_type": "string",
      "requirement": "user_prompt",
      "description": "HGNC gene symbol, for example APOE or TP53"
    },
    {
      "key": "detail_level",
      "input_type": "select",
      "requirement": "optional",
      "description": "How much detail the summary should go into",
      "default": "brief",
      "options": ["brief", "standard", "exhaustive"]
    }
  ],
  "skills": ["spoke-knowledge-graph"]
}
{%- endraw %}
