---
name: develop-biorouter-skill
description: "Guide for authoring a Biorouter skill: the SKILL.md format and frontmatter, how Biorouter discovers and loads skills, writing a description that triggers reliably, progressive disclosure with supporting files, composing with subagents/workflows/hooks, evaluating a skill against a baseline with test prompts, graded checks and a review page, optimizing the description with should-trigger and near-miss queries, and packaging a skill as a zip. Load this skill whenever the user wants to create, write, improve, test, evaluate, benchmark or publish a Biorouter (or Claude) skill, turn a workflow into a skill, fix a skill that does not load when it should, or asks for skill-creator, which this skill replaces."
user-invocable: true
---

# Developing a Biorouter Skill

A **skill** is a folder containing a `SKILL.md` file: YAML frontmatter
(`name`, `description`) followed by markdown instructions, plus optional
supporting files (scripts, templates, reference docs) beside it. Skills are how
a user teaches Biorouter *their* way of doing something once, instead of
re-explaining it every session. This guide teaches you to author, evaluate,
tune and package one well.

> Skills encode **method and environment-specific knowledge** ("how *we* do X",
> our endpoints/conventions/quirks), not facts or documents. Those belong in a
> Knowledge base. And not generic knowledge the model already has.

If a skill named `skill-creator` is also installed, it is the earlier
marketplace skill this one replaces. Follow this skill, and tell the person they
can remove that copy from the Skills page or with
`biorouter skill remove skill-creator`.

## Where to start

The whole loop is: capture the intent, draft, run a few realistic prompts with
and without the skill, review the results with the person, improve, repeat, then
tune the description and package. Find out where the person is and join there.
An idea needs the intent captured (below); a draft goes straight to testing. For
"turn this into a skill", the conversation already holds the workflow: read back
the tools used, the steps, the person's corrections and the input and output
formats, then ask only for what is missing. An existing skill that does not load
when it should needs its description fixed; one that loads and works badly needs
its body fixed. Someone who wants a quick skill can skip the formal evaluation:
offer it, do not impose it.

**Capture the intent** with four questions: what the skill should let the agent
do; when it should load (the phrases and situations); what the output looks
like; and whether to set up test cases. Skills with checkable output (file
conversions, data extraction, code generation, fixed procedures) gain from test
cases. Skills whose output is a matter of taste (writing style, visual design)
are better judged by the person reading results. Ask about edge cases, example
files and dependencies before writing test prompts.

Many authors are not programmers: explain "frontmatter", "assertion" or "JSON"
in a few words the first time unless the person clearly knows them.

## How Biorouter loads skills (why structure matters)

Biorouter uses **progressive disclosure**: load the cheap thing always, the
expensive thing on demand:

1. **Discovery (always on, and it is one sentence).** The always-on system
   prompt carries a *count* of the enabled skills and the names of the tools
   that reach them, not the catalog. The agent finds a skill by calling
   **`searchSkills`** (with no query, to page the whole catalog), and what it
   matches against there is the `name` + `description`. So the description
   still does all the triggering work; it simply has to survive a search rather
   than sit permanently in front of the model, which makes concrete trigger
   keywords more important, not less.
2. **Body (loaded on trigger).** Once a description matches the task, the agent
   calls the **`loadSkill`** tool to pull in the full `SKILL.md` body. Keep it
   lean, because it competes with the live conversation for context.
3. **Supporting files (loaded as needed).** Files next to `SKILL.md` are listed
   when the skill loads; the agent reads or executes them only when required, so
   they cost nothing until used.

**Implication:** put the trigger logic in the `description`, the core procedure
in the body, and bulky reference material / large examples / scripts in
supporting files.

## Where skills live

Biorouter auto-discovers skills at session start from several locations
(portable across agent tools that share the convention):

- `~/.config/biorouter/skills/<slug>/`: user-global (the default install dir)
- `~/.claude/skills/<slug>/` and `~/.config/agents/skills/<slug>/`: shared
- Project-local: `.biorouter/skills/`, `.claude/skills/`, `.agents/skills/`
- Bundled inside an installed extension: `extensions/<name>/skills/<slug>/`

A project-local skill overrides a global one of the same slug. Enable/disable
per skill from the **Skills** page or with `biorouter skill enable|disable
<name>` in the CLI (or in `biorouter configure` → Toggle Extensions for the
`skills` extension as a whole); per-skill disabled state
persists in `~/.config/biorouter/skills-config.json`. Built-in skills that ship
with the app (like `about-biorouter`) can be toggled off but are restored if the
folder is deleted.

## SKILL.md format and standards

```markdown
---
name: figure-style
description: Apply the lab's figure formatting conventions whenever creating or revising plots, charts, or figures.
---

# Lab figure style

- Use colorblind-safe palettes (Okabe-Ito).
- Label axes with units; sentence case; no chart junk.
- Export at 300 dpi, PDF + PNG.
```

**Frontmatter rules** (Biorouter parses `name` + `description`; extra keys such
as `user-invocable: true` are tolerated and ignored by the loader):

| Field | Required | Standard |
|---|---|---|
| `name` | yes | Make it equal the folder slug. Lowercase letters, numbers, hyphens; ≤ 64 chars. Avoid reserved words "anthropic"/"claude". Prefer a gerund or action phrase (`analyzing-spreadsheets`, `figure-style`), never `helper`/`utils`/`tools`. |
| `description` | yes | Non-empty, ≤ ~1024 chars. **Third person.** State both *what it does* and *when to use it*, with concrete trigger keywords. |

⚠ **Those standards are authoring conventions, not validation.** The loader
requires the frontmatter to parse with both `name` and `description` present
(`SkillMetadata` has neither as an `Option`, so serde rejects a missing one),
and its line-based fallback parser additionally requires `name` to be non-empty.
That is all. Nothing checks the length of `name` or `description`, nothing rejects
a reserved word, and nothing enforces that `name` matches the folder: a skill
whose `name` disagrees with its directory installs happily and then answers to a
different label than the one on disk. Hold yourself to the table; the tooling
will not.

**Body standards:**
- Keep the body **under ~500 lines**. Past that, split detail into supporting
  files in the same folder and link to them (one level deep, because the agent may
  only preview deeply nested files and read them incompletely).
- Write **imperative operating instructions for the agent**, not human-facing
  prose explaining concepts the model already knows. ("Run the linter, then
  fix any errors", not "Linters are tools that check code style.")
- **Explain why** a step matters instead of stacking capitalized MUST and NEVER.
  A reason generalizes to cases the rule did not foresee; if you find yourself
  writing one in capitals, reframe it as the reason.
- Use **consistent terminology** (pick one word per concept and stick to it).
- Avoid **time-sensitive phrasing** ("before August 2025…") and hardcoded
  values that will drift; they rot the skill. Verify against the live codebase
  instead of pinning specifics.
- When the skill produces structured output, give an **exact template**, and
  anchor behavior with a short **input and output example**.
- A reference file over ~100 lines should start with a short table of contents
  so a partial read still reveals its scope.
- Nothing in a skill should surprise the person given its stated purpose: no
  hidden network calls, data collection or exploit code.

## Writing a description that triggers reliably

The `description` is the single matching surface across potentially many
installed skills, and it is the most important thing you write.

- **Third person, what + when.** "Generates commit messages by analyzing git
  diffs. Use when the user asks for help writing a commit message or reviewing
  staged changes." Not "I can help you write commit messages."
- **Include the words the user will actually say**: file types, formats,
  domain terms, task verbs.
- **Counter under-triggering.** Models tend to under-load skills. Name the
  contexts explicitly ("…even when the user doesn't say 'dashboard' but asks to
  display metrics"). For a skill that loses to a tempting inline default, an
  imperative description ("Always load this skill before X; do not do X
  directly") triggers far more reliably, but reserve forceful language for
  skills that need it, or overlapping "always" directives dilute each other.
- Keep all the "when" guidance **in the description**, not scattered in the body.
- To tune it with measurements instead of judgment, see
  [Optimizing the description](#optimizing-the-description).

## Set the right degree of freedom

Match how prescriptive the instructions are to how fragile the task is:

- **High freedom (prose):** multiple valid approaches, judgment-driven (e.g. a
  code-review checklist). Describe goals and principles.
- **Medium freedom (parameterized steps / pseudocode):** a preferred pattern
  with acceptable variation.
- **Low freedom (exact commands, "run this verbatim"):** fragile or
  consistency-critical operations (migrations, releases). Say "do not modify the
  command or add flags."

## Composing with Biorouter's other primitives

A skill is *guidance loaded inline*. Have it delegate when another primitive
fits better, and know when **not** to:

- **Subagents.** When a step is high-volume, parallelizable, or produces
  throwaway intermediate output (broad search, log triage, multi-file audit),
  instruct the agent to spawn a subagent so that work stays out of the main
  context. Keep the orchestration/decision logic in the skill.
- **Workflows.** For deterministic multi-step automation (loops, fan-out,
  verify stages), point the user at a workflow. A workflow can **pin skills**
  via its `skills:` field so the automation always runs with the right methods
  loaded. Use a skill for the *method*; a workflow for the *orchestration*.
- **Hooks (harness).** Instructions are followed *most* of the time but can be
  reasoned around under pressure. For a **non-negotiable guarantee** ("always
  run the formatter after editing", "block this dangerous command"), a hook is
  deterministic where skill prose is probabilistic. If a skill rule keeps being
  skipped, that rule probably belongs in a `PostToolUse`/`PreToolUse` hook, not
  the body. Rule of thumb: **skills guide judgment; hooks enforce guarantees.**
- **Extensions.** A skill that needs real tools rather than instructions belongs
  in an extension; see the `develop-biorouter-extension` skill.

## Supporting files

Put bulky or optional material beside `SKILL.md`, keep it one level deep, and
reference it from the body:

```
my-skill/
  SKILL.md
  reference.md          ← detailed schemas / API docs (read on demand)
  scripts/validate.py   ← run for deterministic, repeatable steps
  templates/report.md   ← output template
```

Prefer a pre-written script over asking the agent to regenerate the same code:
more reliable, consistent, and the script's *source* never enters context,
only its output. Make execution intent explicit ("**Run** `validate.py`" vs
"**See** `validate.py` for the algorithm"), have scripts handle their own
errors, and justify any magic numbers.

## Testing a skill (robustness & consistency)

There are **two separate failure modes**, and you test each:

**1. Does it trigger?** In a *fresh* session, give a task that *should* load
the skill **without naming it** (e.g. "plot this distribution" for a
figure-style skill), and one that should not. Confirm the agent calls
`loadSkill` (directly, or inside an `execute_code` script when Code Execution
is on) and follows the instructions. If it under-triggers, sharpen the
`description`, which is the match surface, not the body. (The user can always
force it: "use the figure-style skill".)

**2. Does it execute every step?** A loaded skill can still silently skip
internal steps, especially invisible verification. Make critical steps produce
**visible output** ("before delivering, output a checklist of each rule and
whether you applied it") so you and the agent can confirm compliance.

Skills are probabilistic, so judge a *rate* over several runs, not one pass.
Test with your usual hooks and project instructions present (unrelated context
can suppress a weak description), sanity-check on a smaller model tier if you
deploy across tiers, and keep each skill **small and single-purpose**: overlapping
skills dilute each other, so split a sprawling one into a bundle. For a skill
that matters, measure both failure modes with the next two sections.

## Evaluating a skill against a baseline

This measures whether the skill makes the work better than the same model
without it (or than its previous version), with a few realistic prompts, graded
checks and a page the person reads. **Keep the workspace outside every skills
folder**, for example `<skill-name>-workspace/` in the chat's working folder:
discovery scans folders such as `~/.config/biorouter/skills/`, so a snapshot
`SKILL.md` kept there becomes a second skill with the same name. Keep the draft
under test outside them too until it is ready to install. Organize results by
round (`iteration-1/`, `iteration-2/`), one folder per test case in each.

**1. Write 2 or 3 test prompts** of the kind a real person would type, show them
to the person, and save them to `<workspace>/evals.json`:

```json
{"skill_name": "figure-style",
 "evals": [{"id": 1, "name": "volcano-plot-from-csv", "prompt": "…",
            "expected_output": "…", "files": [], "assertions": []}]}
```

**2. Start every run of the round together**: for each prompt, one run with the
skill and one baseline, so they finish at about the same time. The baseline is no
skill at all for a new skill, and the previous version for an existing one (copy
the skill folder to `<workspace>/skill-snapshot/` before you edit it). Each run
saves its files under `<workspace>/iteration-N/<eval-name>/with_skill/outputs/`
(or `baseline/outputs/`). Two ways to run them:

- **Subagents**, when this chat offers the `subagent` tool (it needs the
  Completely Autonomous mode and at least one extension switched on). Give each
  child the prompt, the input files and where to save its outputs; tell the run
  with the skill to read `<path>/SKILL.md` and follow it, and an improving
  round's baseline to read the snapshot's. When a copy of the skill is
  installed, pass `remove_skills: ["<skill-name>"]` to every child so the
  installed copy cannot stand in for the version under test (a name that is not
  installed refuses the spawn, so leave it out then). Each child's time and
  tokens are on its card and in `biorouter session subagents <chat id> --format
  json` (`elapsedSeconds`, `totalTokens`).
- **`biorouter run`** in the shell, one process per run, started from that run's
  folder so its files land there:

  ```bash
  biorouter run --no-session --output-format json \
    --provider <this chat's provider> --model <this chat's model> \
    --system "Before you start, read /abs/path/SKILL.md and follow it. Do not load an installed skill named <skill-name>." \
    -t "<the prompt>" > run.json
  ```

  Point `--system` at the snapshot for an improving round's baseline, and leave
  it out for a no-skill one. In `run.json`, `messages` is the transcript and
  `metadata.total_tokens`, `metadata.wall_s` and `metadata.tool_calls` are the
  cost and time. A run without `-s` cannot ask for approval, so a call that needs
  one is skipped: choose prompts that do not depend on one. Use this chat's own
  provider and model, never a more public one: the prompts are this chat's data.

**3. Draft assertions while the runs work.** Good assertions are objective and
named so a glance says what they check ("exports one PDF and one PNG at 300
dpi"). Check with a script whatever a script can check, and reuse it in later
rounds. Do not force assertions onto taste; leave those cases to the person.

**4. Grade each run** into `grading.json` beside its outputs, as
`{"expectations": [{"text": "…", "passed": true, "evidence": "…"}]}` (a grader
subagent can do this from the assertions, outputs and transcript).

**5. Aggregate** into `<workspace>/iteration-N/benchmark.json`: for each
configuration, the pass rate, seconds and tokens as mean and standard deviation
over the test cases, and the difference between the configurations. Then read it
as an analyst: an assertion that passes in both configurations tells you
nothing; a test case whose result swings between repeats is probably flaky; a
skill that doubles the tokens for a small gain is a trade the person should see.

**6. Build the review page, and show it before you judge the outputs yourself.**
Write one self-contained `<workspace>/iteration-N/review.html`. Writing the file
opens it in the side panel, and the person can also open it in a browser. The
panel runs inline styles and scripts but allows no network, so embed everything:
no remote scripts, fonts or stylesheets, and images as `data:` URIs. Give it two
views:

- **Outputs**, one test case at a time: the prompt, the output with the skill
  and the baseline output (text inline, images embedded, other files listed by
  path), the grades folded away, and from round 2 the previous round's output
  and the person's earlier comment, also folded away.
- **Benchmark**: the numbers from step 5, one row per test case, and your
  analyst notes.

Ask the person to reply in the chat with a comment per test case by name (none
means it was fine), and save the replies to `iteration-N/feedback.json` as
`{"reviews": [{"eval": "<name>", "feedback": "…"}]}` for the next round's page.

**7. Improve the skill and run the next round** into `iteration-<N+1>/`, with
fresh baselines, until the person is satisfied, the comments come back empty, or
rounds stop helping. While improving:

- **Generalize from the comments.** The skill will meet far more prompts than
  these few, so a fix that only fits them is overfitting. Try a different
  framing or pattern before adding another rule.
- **Keep it lean.** Read the transcripts, not only the outputs, and cut whatever
  makes the agent spend time on unproductive steps.
- **Bundle repeated work.** If every run wrote a similar helper script, write it
  once in `scripts/` and have the skill run it.

To answer "is the new version really better?", run a **blind comparison**: give
a fresh subagent the prompt and the two outputs labelled A and B in random order,
ask which better serves the prompt and why, and only then say which was which.

## Optimizing the description

Do this once the body works; a description tuned for a body that later changes
has to be tuned again. It measures triggering directly: how often a fresh agent,
given a realistic request and the person's installed skills, loads this skill.
Say what it costs first: 20 queries, 3 runs each and up to 6 descriptions is up
to 360 short runs, each carrying the full system prompt, so offer fewer repeats
or rounds. Run on this chat's own provider and model, so the result matches what
the person will see and no query reaches a more public model.

**1. Write 20 trigger queries** and save them to
`<workspace>/trigger-evals.json` as `[{"query": "…", "should_trigger": true}]`:

- 8 to 10 that should load the skill: the same need phrased formally and
  casually, requests that never name the skill or the file type, uncommon uses,
  and cases where another skill competes and this one should win.
- 8 to 10 that should not: near misses that share words or concepts with the
  skill but need something else. An obviously unrelated request tests nothing.
- Make each one concrete and substantive, as a real person types: a file path,
  column names, a little backstory, the odd typo or abbreviation, mixed lengths.
  A one-step request ("read this PDF") is answered directly and loads no skill
  whatever the description says, so it measures nothing. Weak: `"Format this
  data"`. Strong: `"my PI sent qc_metrics_march.xlsx and wants a column with the
  percent of reads passing filter, total is in C and passing in D i think"`.

**2. Have the person review the set** (a numbered list in the chat is enough)
before anything runs: bad queries produce a bad description. **3. Split it
once**, 60 percent for tuning and 40 percent held out, each with both kinds of
query, and keep that split for every round.

**4. Score a description.** Install the skill where discovery finds it, with
that description in its frontmatter, then run each query 3 times, each in a new
process:

```bash
biorouter run --no-session --output-format json --max-turns 5 \
  --provider <this chat's provider> --model <this chat's model> \
  -t "<query>" > trial.json
jq -e --arg s "<skill-name>" 'any(.messages[].content[]?
  | select(.type == "toolRequest") | .toolCall.value? // empty;
  ((.name | endswith("loadSkill")) and .arguments.name? == $s)
  or ((.name | endswith("execute_code"))
      and ((.arguments.code? // "") | contains("loadSkill") and contains($s))))' \
  trial.json   # exit status 0: the skill was loaded
```

The second test is there because Code Execution, on by default, can make the
`loadSkill` call inside an `execute_code` script. A new process per trial keeps
the score about the current text: the skill catalog notices a skill folder being
added or removed, but a description edited in place changes no folder it
watches, so a running app or chat can keep matching the old wording. Exit status
3 (the action cap) is normal; a skill loads in the first few actions or not at
all.

A query's trigger rate is the share of its 3 runs that loaded the skill. A
should-load query passes at 0.5 or more, a should-not query below 0.5. Score the
tuning and held-out sets separately, as passed queries over queries.

**5. Improve for up to 5 rounds.** Write a better description from the tuning
set's failures and the descriptions already tried: third person, what and when,
roughly 100 to 200 words and well under 1024 characters. Never look at held-out
results while writing it; that is what keeps the final score honest. Score each
new description on both sets, and stop early when the tuning set passes in full.

**6. Keep the round with the best held-out score** (on a tie, the shorter
description), put it in the skill's frontmatter, and show the person the old and
new descriptions with both scores.

## Bundles (grouping related skills)

A **bundle** is a parent folder containing multiple skill folders; Biorouter
discovers each and toggles them as a unit:

```
my-bundle/
  skill-one/
    SKILL.md
  skill-two/
    SKILL.md
    helper.md
```

## Packaging & installing (zip export, the Biorouter way)

Share a skill by zipping its folder. Accepted layouts:

- **Single skill:** `SKILL.md` at the zip root, **or** one folder deep
  `<slug>/SKILL.md`.
- **Bundle:** `<bundle>/<slug>/SKILL.md` (two levels deep).
- **Declared by a manifest:** a `biorouter-package.json`,
  `.codex-plugin/plugin.json`, `.claude-plugin/plugin.json` or
  `skills-manifest.json` names the components explicitly, and that beats the
  shape on disk.
- **Ambiguous, refused until you answer:** two or more skill folders sitting
  *loose at the archive root* with **no** manifest. That is equally the shape of
  one package and of a folder somebody happened to zip, so Biorouter asks
  instead of guessing. Resolve it with `--as bundle` or `--as individual` (over
  HTTP the question comes back as a 200 with `needsChoice`, not an error). A
  named parent directory is **not** ambiguous: putting the skills inside
  `my-bundle/` has already said they belong together.

```bash
# Single skill
cd my-skill && zip -r ../my-skill.zip SKILL.md reference.md scripts/

# Or zip the folder itself
zip -r my-skill.zip my-skill/
```

Install it with the CLI or the GUI:

```bash
biorouter skill install my-skill.zip                    # a local .zip
biorouter skill install https://github.com/you/skills   # or a repository URL
biorouter skill install my-skills.zip --as bundle       # answer the ambiguity above
biorouter skill list                                    # verify it registered
```

`install` takes one `source` argument (a path to a `.zip` **or** a repository
URL) plus `--force` to replace an already-installed slug and
`--as bundle|individual` for a source that could be either. When you update an
installed skill, keep its `name` and folder slug unchanged, or it installs as a
second skill instead of replacing the first.

In the desktop app, the **Skills** page → **Add Skill** (or drag the zip in)
does the same, unpacking into `~/.config/biorouter/skills/<slug>/`. You can also
copy a skill's `SKILL.md` to the clipboard from the Skills page to share it
quickly.

## Authoring checklist

```
[ ] Folder slug == frontmatter name (lowercase, hyphens, ≤64 chars)
[ ] description is third person, says what AND when, with trigger keywords
[ ] Body is imperative agent instructions, < ~500 lines, no generic filler,
    reasons instead of capitalized MUST/NEVER
[ ] Bulky/optional material moved to supporting files, one level deep
[ ] Degree of freedom matches task fragility (prose vs exact commands)
[ ] Delegates to subagents/workflows where appropriate; guarantees → hooks
[ ] No time-sensitive phrasing or hardcoded values that will drift
[ ] Trigger-tested in a fresh session without naming the skill (and a
    negative case), repeated for consistency
[ ] Critical steps emit visible output so execution can be verified
[ ] For a skill that matters: evaluated against a baseline, reviewed by the
    person on the review page, description tuned on held-out queries
[ ] Evaluation workspace kept outside every skills folder and out of the zip
[ ] Packs as SKILL.md at root, <slug>/SKILL.md, or a named parent for a
    bundle, never several skills loose at the zip root with no manifest,
    which install refuses as ambiguous
[ ] Installs cleanly via `biorouter skill install <zip-or-repo-url>`
```

<!-- The evaluation loop and description optimization above are adapted from the skill-creator skill in anthropics/skills (Apache License 2.0), rewritten for Biorouter's runs, subagents, side panel and skill catalog. -->
