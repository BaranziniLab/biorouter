# Open documentation issues

> **What this is.** The unresolved problems found during the 2026-07-18 documentation
> cleanup — every issue that needs a human to decide which of two conflicting sources is
> right, or to supply content nobody could invent.
> **Status:** Current — a working register, not a historical record. Delete an entry when it is fixed.
> **Audience:** maintainers, and anyone editing a document listed below.

On 2026-07-18 every Markdown file under `docs/` was rewritten to the
[documentation house style](documentation-style.md). Fifty-eight rewrite agents reported
409 free-text concerns. Most were fixed in the pass itself. The entries below were
**deliberately not fixed**: each one is a place where the documentation and the code disagree, or two
documents disagree, or the source material is simply gone. Silently picking a side would
have meant inventing a fact. Every claim marked *verified* was checked against the code in
this worktree; `file:line` references point at real current lines.

> **How to use this file.** Fix an issue, then **delete its subsection**. This register is
> only useful while it is accurate. If you decide an entry is not worth fixing, move it to
> the "Reported but verified as not an issue" section with the reasoning.

## Summary

| Severity | Meaning | Open |
|---|---|---|
| [Correctness risk](#correctness-risk) | Documentation that would lead someone into an unsafe or wrong action | 8 |
| [Doc/code contradiction](#doccode-contradiction) | The docs and the code disagree, or two docs disagree | 15 |
| [Dead references](#dead-references) | Cited documents, branches, artifacts, or paths that do not exist | 7 |
| [Coverage gaps](#coverage-gaps) | Things a reader will look for and not find | 7 |
| [Cosmetic](#cosmetic) | Worth noting, not worth chasing | 8 |
| **Total** | | **45** |

## Correctness risk

### Live UCSF Versa credentials remain in the main checkout and in git history

The institutional-providers plan carried three apparently real credentials in its Playwright
validation steps: a Versa Azure API key (a base64 blob), a Bedrock access key id
`<redacted — see the affected commits>`, and a matching secret access key. **Verified:** they were removed from this
worktree's working tree, but they are still present in the main checkout at
`/Users/wanjun/Desktop/biorouter/docs/superpowers/plans/2026-05-07-institutional-providers.md`
(three matches), and in this repo's history at commits `8c74498a` and `2c7c873f` (removed in
`6787fa76`). Redacting the working copy does not undo either. **Decision needed:** rotate the
Versa Azure key and the Bedrock key pair with UCSF regardless, redact the main checkout, and
decide whether history rewriting is warranted or rotation alone suffices.
Affected: [institutional-providers design](../history/institutional-providers/versa-providers-design.md).

### `BIOROUTER_DISABLE_KEYRING=true` implies the value matters — it does not

[Secret storage](../security/secret-storage.md) documents the variable as
`BIOROUTER_DISABLE_KEYRING=true`, twice, as though `false` would keep the keyring on.
**Verified:** `crates/biorouter/src/config/base.rs:212-219` does
`match env::var("BIOROUTER_DISABLE_KEYRING") { Ok(_) => SecretStorage::File, Err(_) => Keyring }`
— the value is ignored entirely, so `BIOROUTER_DISABLE_KEYRING=false` writes secrets to
plaintext `~/.config/biorouter/secrets.yaml`.
[Environment variables](../configuration/environment-variables.md) (line 176) states this
correctly. **Decision needed:** reword secret-storage.md's table and dev-setup line to match,
or change the code to honour the value.

### The built-in extensions table marks four default-on extensions as "Disabled"

[Extensions and skills guide](../extensions/extensions-and-skills-guide.md) lists Computer
Controller (line 22), Memory (23), Auto Visualiser (25) and Code Execution (46) as
`Disabled`. **Verified:** `ui/desktop/src/components/settings/capabilities/capabilities.ts`
sets `defaultEnabled: true` for eleven capabilities and `false` only for chat recall.
Four pages under `built-in/` also walk readers through "enabling" an extension
that is already on. The cleanup added Note callouts rather than editing table rows.
**Decision needed:** correct the four rows and drop the callouts.

### `biorouter schedule delete` is not a real subcommand

[Scheduled jobs](../workflows/scheduled-jobs.md) (line 89) tells readers to run
`biorouter schedule delete <job-id>`. **Verified:** the subcommand is `Remove`
(`crates/biorouter-cli/src/cli.rs:562-563`, handler `handle_schedule_remove` at
`crates/biorouter-cli/src/commands/schedule.rs:171`), and
[CLI command reference](../cli/command-reference.md) (line 462) documents
`biorouter schedule remove --schedule-id …`. The command as printed fails.
**Decision needed:** fix scheduled-jobs.md.

### The managed-policy guide understates a Windows trust gap the design doc calls load-bearing

[Managed policy](../security/managed-policy.md) frames Windows as "`%ProgramData%` is
admin-writable-only by default, so phase 1 trusts the location; deep ACL owner verification is
a planned follow-up". [Managed policy tier](../agent-loop/designs/managed-policy-tier.md)
states outright that `verify_trusted()` is **a no-op on Windows**, and the cross-platform
parity audit tracks it as GAP-7. An administrator reading only the guide would not learn that
the ownership check the tier's entire trust claim rests on does not run.
**Decision needed:** agree one wording; the guide is the user-facing surface.

### A sample config grants an external agent unattended workspace-write access

The external-subagent example in [Subagents](../agent-loop/subagents.md) ships a
`~/.codex/config.toml` with `approval_policy = "never"` and sandbox mode `workspace-write`.
Copy-pasting it lets an external agent execute and write without ever prompting. The cleanup
left the block byte-identical per the no-edits-to-code-samples rule and did not add a warning,
because authoring a risk assessment is a content decision. **Decision needed:** a
human-written `> **Warning.**` above the block, or a safer default in the sample.

### The PHI page's local-model guidance predates the bundled provider, and has no review date

[Data privacy and PHI](../security/data-privacy-and-phi.md) names only Ollama in every
local-model recommendation. Llama Server (`llamacpp`) is the bundled sidecar and ranks first
among local providers. The cleanup added a drift note but did **not** add `llamacpp` to the
recommended-provider tables — saying a provider is approved for PHI is a compliance claim, not
a formatting fix. The page also carries no "last reviewed" date despite its own premise that
institutional agreements change, and routes governance questions to a named individual's
personal address rather than a role address. **Decision needed:** someone who can speak for
UCSF governance must rule on the tables, stamp a review date, and pick a durable contact.

### UCSF readers are pointed at the commercial providers, not the institutional ones

[Choosing a model provider](../getting-started/choosing-a-model-provider.md) and
[Installation](../getting-started/installation.md) describe generic "Azure OpenAI" and "Amazon
Bedrock" as the UCSF path. In code those are the *commercial* providers (`azure_openai`,
`aws_bedrock`); the institutional ones are separate modules, `versa_azure` and
`versa_bedrock`, in their own "Institutional Models" panel group. Neither has a documentation
page anywhere. UCSF users following either page will configure the wrong provider.
**Decision needed:** author the two `versa_*` sections and correct the conflation.

## Doc/code contradiction

### `export_app` does ship Windows launchers — CLAUDE.md is the stale side

[Agent Drafter](../agent-drafter/apps-platform-design.md) claims exports ship launchers for all
three OSes; the root `CLAUDE.md` describes only `biorouter-launch.sh` + `run.sh`/`run.command`.
**Verified: the doc is right.** `crates/biorouter-mcp/src/agent_drafter/render.rs:953-954`
writes both `run.ps1` (`run_ps1()`) and `run.bat` (`run_bat()`, a thin PowerShell wrapper
defined at line 625), and tests at `render.rs:1428-1450` assert the pair.
**Decision needed:** update `CLAUDE.md`, not the doc.

### `Cargo.toml` claims `just release-binary` uses the `release-dist` profile; nothing does

**Verified:** `Cargo.toml:48` comments that "`just release-binary` uses it" for
`[profile.release-dist]`, but grepping the `Justfile` and all of `scripts/` for `release-dist`
returns no hits — `release-binary` runs `cargo build --release`. This also means the
performance report's follow-up ("wire release-binary to release-dist after a notarized-packaging
smoke test") is still open, contrary to the record.
Affected: [jcode borrows implementation report](../history/performance-2026-06/jcode-borrows-implementation-report.md).
**Decision needed:** wire it or correct the comment.

### The llama-server sidecar defaults in `CLAUDE.md` match neither the code nor the checklist

Three disagreements, all in `crates/biorouter/src/providers/llamacpp_sidecar.rs`.
Context size: `CLAUDE.md` says model-native (`--ctx-size 0`, read back from `/props`);
`default_context_size()` at line 620 returns a **memory-tiered** value (131072 at ≥64 GiB,
65536 at ≥16 GiB, else 32768) and never passes `0`; the
[model catalog QA checklist](../providers/llama-server/model-catalog-qa-checklist.md) says a
flat 32768. Thinking: `CLAUDE.md` says enabled by default; `build_args` at lines 761-772
defaults `LLAMACPP_ENABLE_THINKING` to **false** and emits `--reasoning off`, asserted by unit
tests at lines 1780-1781 and 1831-1851. Catalog: the checklist names Qwen3.5 and Gemma-4 ids
that no longer exist — the shipping `MODEL_CATALOG` is `gemma4`, `gemma4-e2b`, `gemma4-12b`,
`gemma4-26b`, `gemma4-31b`, `qwen3.6`, `qwen3.6-27b`, so the checklist's
`LLAMACPP_SURVEY_MODELS=qwen3.5-0.8b,gemma-4-e2b` selects nothing. Relatedly,
`crates/biorouter/tests/llamacpp_survey.rs:253` hardcodes "32k ctx" into its report header.
**Decision needed:** reconcile all three against the code, in one pass.

### The Versa provider constants in the history record no longer match the code

The institutional-providers documents state deployment `gpt-5.2-2025-12-11` and API version
`2024-10-21`; `crates/biorouter/src/providers/versa_azure.rs:18-20` reads
`gpt-5.5-2026-04-24` and `2025-01-01-preview`. Likewise the plan quotes
`VERSA_BEDROCK_DEFAULT_MODEL = us.anthropic.claude-sonnet-4-6` against a shipping
`us.anthropic.claude-opus-4-6-v1` and a longer model list. These are historical records, so the
old values are arguably correct *there* — but the current values have no user-facing home.
**Decision needed:** publish the live constants in a provider page (see the coverage gap
above), then leave the history alone.

### `CLAUDE.md` says v1.87.2; the repo is at v1.88.3

**Verified:** `Cargo.toml` and `ui/desktop/package.json` both read `1.88.3`, and
`docs/releases/notes/` runs through `v1.88.3.md`. `CLAUDE.md:7` says v1.87.2. Several rewritten
docs declined to stamp a current version because of this.
**Decision needed:** `scripts/release.sh bump` does not touch `CLAUDE.md`; either add it or
stop citing a version there.

### Provider section ordering: two history docs say Institutional first, the code says Local

**Verified:** `ui/desktop/src/components/settings/providers/ProviderGrid.tsx` renders Local
Models (line 208), then Institutional Models (218), then Commercial Models (227) — matching
`CLAUDE.md`. Both institutional-providers documents specify Institutional → Local →
Commercial. The cleanup added superseded notes. **Decision needed:** confirm the shipped order
is intended and close out the design's requirement.

### The Knowledge plans put the module in the wrong crate, including in the architecture diagram

[Plan 1](../history/knowledge-base-buildout/plan-1-storage-git-and-graph.md) places the whole
Knowledge module under `crates/biorouter/src/knowledge/` throughout; the shipped code is in
`crates/biorouter-mcp/src/knowledge/`, with `crates/biorouter/src/knowledge/mod.rs` being just
`pub use biorouter_mcp::knowledge::*;` to break a dependency cycle. The founding design's
architecture diagram draws the layering **inverted**, which is worse than the path drift
because `CLAUDE.md` calls the distinction out as load-bearing.
**Decision needed:** whether a mass path correction is worth it; the diagram at minimum should
be redrawn.

### The apps-SDK v2 design and the shipped SDK reference disagree in six places

All between [v2 design](../apps-sdk/v2-design.md) and [SDK reference](../apps-sdk/sdk-reference.md):
the design's risks table says "seven pillars" while the body defines nine; theme packs differ
(the design lists `glass`, which never shipped, and omits the `biorouter` base pack);
multi-agent turns are described as running in parallel by the design and as serialized by the
reference; per-app skills scoping is "enforced" in the design and advisory in the reference;
export staging of `.brxt` bundles and the first-run credential dialog are promised by the design
and recorded as out of scope by the reference; and a `canvas` widget kind plus a
`ui.allow_components` capability appear only in the design. **Decision needed:** one pass
marking the design's unshipped items as intent, or a roadmap update.

### The branding spec and the UI-overhaul execution status disagree on the shipped mark

[Logo and wordmark spec](../design/branding/logo-and-wordmark-spec.md) presents the typeface as
an open blocker; [execution status](../design/ui-overhaul/execution-status.md) records Inter
chosen, both studios re-tuned, and every asset re-propagated on 2026-07-18. They also disagree
on the coral: the spec is emphatic about coral-700 `#a94f2a`, explicitly rejecting coral-600
`#b85a32`, which is the value execution-status says shipped. And on geometry: weight 600 vs
Bold/700, underline gap 2% vs 0.21em, thickness 10% vs 0.15em, mark 60% vs a ~76% lockup.
**Decision needed:** fold the Inter decision into the spec or demote the spec to a historical
record — and check the built assets to settle the hex.

### The notification spec's central token mechanism never shipped

[Notification redesign spec](../history/notification-redesign/notification-surface-design.md)
specifies `--fill-{status}` tokens; grepping `ui/desktop/src` finds no `--fill-` definitions at
all — `NotificationSurface.tsx` uses a literal `CHIP_TINT` Tailwind map instead, because
Tailwind's JIT only sees literal class strings. The contrast-verification bullet is therefore
unmet, and its path (`scripts/check-contrast.mjs`) is wrong — the file is
`ui/desktop/scripts/check-contrast.mjs`. Geometry also drifted (radius, a 48px close gutter vs
the specified 40px), and `SystemNotificationInline.tsx` never migrated despite being listed
under "Files touched". Separately, the spec says "no 3px left bar" while `design.md` §4.3
apparently still asks for one — the component's header comment resolves this with "Code wins;
the spec yields. Do NOT add the bar." **Decision needed:** update `design.md` §4.3 so the next
reader does not re-add the bar, and reconcile the spec's geometry.

### The agent-loop campaign's outcome report still says nothing is merged to `main`

[Outcome report](../history/agent-loop-campaign/outcome-report.md) line 3 states "**Nothing is
merged to `main`.**" and its closing line recommends opening a PR from `agent-loop-integration`
— a branch that no longer exists. The wave reports' briefs assert the code *is* on main, and
the modules they name are present in the tree. Relatedly,
[mid-flight review index](../history/agent-loop-campaign/mid-flight-review-index.md) line 53
calls BR-54 "designed, not implemented" against a shipped `mcp_pool.rs`.
**Decision needed:** confirm the merge and restamp both files.

### The skills-packaging designs and the shipped code use different names for the same things

[Bundled skills design](../history/skills-packaging/brxt-bundled-skills-design.md) specifies
`skills_preview` (snake_case) where the plan, `ui/desktop/src/main.ts:2849-2860` and preload all
use `skillsPreview`; wires removal in `ExtensionsView.tsx` where the code uses
`settings/extensions/ExtensionsSection.tsx`. [Skill bundles design](../history/skills-packaging/skill-bundles-design.md)'s
Rust `Skill` struct and TypeScript `Skill`/`SkillBundle` interfaces do not match
`skills_extension.rs` or `skillUtils.ts`, and it describes enablement via a `disabled` array in
`skills-config.json` where the shipped UI drives it through the `skillOverrides` store — two
different persistence stories for one feature. **Decision needed:** correct the designs or mark
them as early sketches.

### Configuration precedence: three tiers or four

[Config file reference](../configuration/config-file-reference.md) lists env vars > config file >
defaults. [Managed policy](../security/managed-policy.md) documents four: Default < User <
Project < Managed. It is not determinable from the docs whether the managed tier overrides plain
`config.yaml` settings or only the permissions and hooks surfaces it scopes itself to.
**Decision needed:** read `crates/biorouter/src/config/base.rs` and write one authoritative list.

### The dashboard-mode removal record and its specification disagree on release numbers

[The dashboard-mode index](../history/dashboard-mode/README.md) says dashboard mode shipped in
v1.76.0 with fold mode following in v1.85.3. [The removal
specification](../history/dashboard-mode/dashboard-mode-removal-spec.md) says v1.75.0 and
v1.76.0. The two documents carry the same removal record, so one of them transcribed the
versions wrongly, and there is no release-notes file for either candidate first release —
`docs/releases/notes/` starts at v1.75.2. **Decision needed:** check the `landing/about.html`
changelog, which the removal specification names as the record for the first release, and
correct whichever document is wrong.

### Two divergent snapshots of the Alma Mater light and Roche Limit theme studios existed

Both studio pages were untracked working files when they were first committed, and they were
committed twice from two working trees: the documentation reorganization filed one snapshot of
each under [`design/theming/`](../design/theming/README.md) as
`alma-mater-light-theme-studio.html` and `roche-limit-theme-studio.html` (commit `b2b13ee8`,
13:19), and the theme feature commit `44995208` (18:03 the same day) committed another as
`design/alma-mater-light-redesign.html` and `design/roche-limit-theme.html`. Identical `<title>`
and section set, divergent bodies: the `theming/` Roche page adds a film-grain layer and an
orbital diagram and drops the "Cell · resting versus active" specimen, and the two Alma Mater
pages argue the same conclusions under different headings. The root duplicates were removed on
2026-07-19 to leave one page per studio at the conformant path; the other snapshot is recoverable
from `44995208`. **Decision needed:** someone with the design context should confirm the
surviving pages are the intended ones, and port the dropped specimen if it is still wanted.

## Dead references

### The entire `shots/` screenshot corpus is gone

Every one of the ~40 files under
[agent-drafter test drive 100](../history/agent-drafter-testdrive-100/) references a `shots/`
directory. **Verified:** no such directory exists anywhere under `docs/`. The folder README
lists it as a deliverable, the runbook's result template prescribes it, and the findings
register's repro for the spec-012 theme-corruption defect instructs the reader to "compare
against `shots/spec-012/baseline.png`". The layout probes lose the most — radial-canvas's entire
before/after theme argument rested on three images. The cleanup converted the links to plain
text. **Decision needed:** are these archived anywhere? If not, the repro instructions need
rewording.

### BR-20 and BR-53 have register entries but no design documents

BR-20 (always-on catastrophic denylist) is cited as a live dependency by BR-21, BR-64 and BR-65;
BR-53 (the SSE patch protocol) is cited by BR-45 as what stable message ids unblock.
**Verified:** neither has a design doc under [designs](../agent-loop/designs/), but both **do**
have full entries in the register —
[improvement proposals](../history/agent-loop-review/improvement-proposals.md) lines 292 and 642.
So the references are thin rather than dead. **Decision needed:** either link the register
entries explicitly from the citing designs, or promote both to design docs like their siblings.

### Seven approval artifacts referenced as authoritative do not exist

None of these is in the repository: `logo-specimen.html` (the specimen the branding spec was
approved against — the two studio files beside it are the later Inter-era tools with different
controls, so the 0.7046em cap-height and the 3.34/4.35/5.15:1 contrast figures are
unverifiable); `.superpowers/brainstorm/…/direction-B-full.html` (the notification spec's
authoritative visual reference for all 14 variants — **verified:** no `.superpowers/` directory
exists at all); `/Users/wgu/Downloads/biorouter_knowledge.html` (a personal path anchoring every
"matching the mockup" justification in Knowledge plans 4 and 5); the 2026-07-12 notification
audit catalogue (sole justification for its 18-defect count);
`ui/desktop/tests/fixtures/build-test-brxts.ts`; `components/ui/DocumentTabs.tsx`; and the three
chat-groups candidate drafts (lift-state, minimal-shell, reuse-dashboard) that
[design judgement and plan](../history/chat-groups/design-judgement-and-plan.md) adjudicates and
which survive nowhere else. **Decision needed:** recover what can be recovered, and reword the
verification steps that cannot.

### Two Wave-3 wave reports were never written or were lost

The campaign wave table lists four Wave 3 clusters (verify BR-47-51, perf BR-53-59, cross-platform,
frontend), but only `wave-3-polish.md` and the cross-platform parity verification report exist.
Nothing covers BR-47-51 or BR-53-59, yet
[outcome report](../history/agent-loop-campaign/outcome-report.md) claims Gate 3 at +220 tests
across four clusters. **Decision needed:** confirm whether they were written.

### `full-review-report.md` was deleted and its generator is orphaned

A parallel agent deleted `docs/history/agent-loop-review/full-review-report.md` (863 KB of HTML
in a `.md` file) during the pass. **Verified:** the file is gone, and
`generate_review_html.py` — the script that produced it — survives in the folder as the only
non-Markdown file in `docs/`. **Decision needed:** regenerate, delete the generator, or move it
under `scripts/`.

### The `.mcp.json` `agent-browser` MCP server does not exist

[Agent browser debugging](../desktop-ui/agent-browser-debugging.md) claims a root `.mcp.json`
registers an `agent-browser` MCP server and refers to a "bundled playwright-electron MCP
server". **Verified:** no `.mcp.json` exists in either this worktree or the main checkout.
**Decision needed:** was it removed, or moved into plugin configuration?

### Assorted moved-path pointers

`docs/release-notes/v1.76.0.md`, cited by the
[dashboard mode record](../history/dashboard-mode/README.md) as the provenance for "shipped in
v1.76.0" — the real directory is `docs/releases/notes/`, and no `v1.76.0.md` predecessor exists
below `v1.75.2.md` for the v1.72.1 case flagged in
[desktop UI fixes](../history/desktop-ui-fixes/). `scripts/check-brand-consistency.sh` still
greps three design HTML files at their pre-reorganization paths. The convert plan's
`convert/xlsx.rs` is really `convert/spreadsheet.rs`. **Decision needed:** low-effort sweep.

## Coverage gaps

### Migration damage: code blocks that survived as comments with no commands

The Docusaurus-to-Markdown migration stripped component-rendered content, leaving blocks that
instruct readers to run nothing. At least twelve `**Examples**` blocks in
[environment variables](../configuration/environment-variables.md) are empty or comment-only
(the AWS Bedrock, Databricks and OTLP blocks are entirely empty ```bash fences; lines 131-153
are nine comments with zero commands). In
[workflow storage and discovery](../workflows/workflow-storage-and-discovery.md), the blocks
under "Add custom workflow directories" and "Configure GitHub workflow repository" **both**
contain only `biorouter workflow list`, which does neither. Restoring these needs someone who can
run the commands. **Decision needed:** this is the single biggest remaining quality problem in
the reference pages.

### Six features are referenced everywhere and documented nowhere

No page under `docs/` covers: `.biorouterignore` (load-bearing security guidance — it is the
mechanism the developer page recommends for protecting `.env` files and private keys, and exists
in prose only); granular per-tool permissions (referenced twice by
[permission modes](../security/permission-modes.md) and by the `biorouter configure` TUI's "Tool
Permission" setting); the extension allowlist (`BIOROUTER_ALLOWLIST`); logs; LLM rate-limit handling; and
in-session interrupt / message queueing — which
[common problems](../troubleshooting/common-problems-and-fixes.md) still directs users to. The
in-session content appears to have been lost in the migration rather than moved.
**Decision needed:** write them, starting with `.biorouterignore`.

### The Auto Visualiser page documents 8 of 34 tools

[Auto Visualiser](../extensions/built-in/auto-visualiser.md) covers eight chart families against
34 registered tools. The most conspicuous omission is `render_dashboard`, the composite-report
tool `CLAUDE.md` describes as the thing the model should reach for whenever an answer needs more
than one figure. The cleanup named every missing tool in a Warning and pointed at the source
rather than invent 26 parameter tables. **Decision needed:** this needs a content pass, not a
formatting pass.

### Four providers have modules but no entry in the provider guide

[Choosing a model provider](../getting-started/choosing-a-model-provider.md) omits `llamacpp`
(Llama Server — the bundled sidecar, ranked *first* in the app's grid), `tetrate` (which the
quickstart recommends twice as *the* quickstart path), `versa_azure` and `versa_bedrock`.
Writing these needs real env vars, default models and setup steps.
**Decision needed:** author four sections; see also the UCSF conflation under Correctness risk.

### The environment-variable catalogue is missing four families

[Environment variables](../configuration/environment-variables.md) does not document: the
llama.cpp sidecar variables (`LLAMACPP_CONTEXT_SIZE`, `LLAMACPP_EXTRA_ARGS`,
`LLAMACPP_EXTERNAL_HOST`, `BIOROUTER_LLAMACPP_BIN`, `LLAMACPP_ENABLE_THINKING`) — of which
`LLAMACPP_CONTEXT_SIZE` has no home anywhere under `docs/`; the auto-update variables
(`BIOROUTER_UPDATE_FEED_URL`, `BIOROUTER_UPDATE_AUTO_INSTALL`); the hooks variables
(`BIOROUTER_VERIFY_BUILD`, `BIOROUTER_SKIP_VERIFY_HOOK`, `BIOROUTER_ALLOW_PROJECT_HOOKS`,
documented only inside [hooks](../agent-loop/hooks/)); and roughly twenty `BIOROUTER_*`
loop-safety variables enumerated only inside a historical wave report
([wave 2 loop detection](../history/agent-loop-campaign/wave-reports/wave-2-loop-detection.md)),
none of them described. **Decision needed:** port them across.

### `config-file-reference.md` never mentions `hooks:`

The `hooks:` block lives in `config.yaml`, and
[config file reference](../configuration/config-file-reference.md) is billed as the reference for
that file, yet contains no mention of hooks at all. **Decision needed:** add the section, or
state that hooks are documented separately.

### Seven CLI subcommands lost their guides in the migration

[CLI command reference](../cli/command-reference.md) had dead links, now removed, to guides that
have no replacement anywhere: running tasks (`biorouter run`), benchmarking (`bench`), ACP
clients (`acp`), managing projects (`project`/`projects`), terminal integration, custom slash
commands for workflows, and logs. Terminal integration matters most — the `@biorouter` / `@g`
section now says the aliases "are created when you set up terminal integration" with no
instructions for doing so. **Decision needed:** write at least the terminal-integration setup.

## Cosmetic

### Brand capitalisation is inconsistent across the tree, and in one product string

`Biorouter` and `BioRouter` both appear widely. The rewrite pass standardised
[releases](../releases/) on `BioRouter` while
[installation](../getting-started/installation.md),
[common problems](../troubleshooting/common-problems-and-fixes.md) and the release-notes H1s use
`Biorouter`. `CLAUDE.md` and the landing site use `BioRouter`. There is also a user-facing string
at `crates/biorouter/src/agents/skills_extension.rs:615` reading "Biorouter's". One global
decision, then a sweep.

### Stale model ids in illustrative examples

`gpt-4`, `gpt-4o`, `claude-3.5-sonnet`, `claude-sonnet-4-20250514` and
`claude-3-7-sonnet-latest` appear across the configuration and workflows pages. Two internal
contradictions sit inside
[choosing a model provider](../getting-started/choosing-a-model-provider.md): the Azure default
is `gpt-5.4-2026-03-05` above a list containing only `gpt-4o`/`gpt-4o-mini`/`gpt-4`, and the
Bedrock default is `us.anthropic.claude-sonnet-4-6` above a list naming "Claude Sonnet 4.5".
Substituting current ids would be inventing facts. Also: `zai.rs:24` pins
`ZAI_DEFAULT_MODEL = "glm-4.6"` while `ZAI_KNOWN_MODELS` right below lists `glm-5.2`/`glm-5.1`/
`glm-5`/`glm-5-turbo` — if that pin is deliberate, the reason belongs in a code comment.

### Every `file.rs:line` citation in the history corpus has drifted

The agent-loop review, the campaign wave reports, the subsystem reviews, the apps-SDK RFCs, the
chat-groups spike, the desktop-menu plan and the performance reports collectively carry hundreds
of line-number citations taken before the code moved. Each file now carries a blanket caveat
("paths authoritative, line numbers historical"). No re-anchoring was attempted. The
cross-platform parity audit is the worst case: it records no commit anchor and the branch it
diffed no longer exists.

### Test counts and headline figures disagree between siblings

`cargo test -p biorouter --lib` is recorded as 1,396 and 1,400 in two 2026-07 reviews; desktop
tests as 978, 985 and 988/989; `vitest run` as 1266/1266 in the UI-overhaul gates table and
1290/1290 in a later section of the same file; `routes::apps` as 26 in one place and ~54 in
another; the auto-visualiser corpus disagrees on 31 vs 33 vs 34 tools. The
[GUI QA regression pass](../history/gui-qa-2026-06/) claims "~40 items verified PASS" against a
row-by-row count of roughly 34.

### Identifier schemes collide and nothing enforces them

The three proposal lenses each independently number `P-1…P-N`, resolvable only via the master
list's prefixed form ("performance P-3"); any renumbering silently corrupts that mapping. The
performance corpus uses three incompatible schemes (`A1-L`, `#1-#24`, `FW*`/`SW*`) over
overlapping work. The agent-drafter stress test claims "8 drafter improvements shipped (H1-H8)"
in the same bullet that says H4 was deferred — seven shipped.

### `scripts/verify-docs.sh` is orphaned and would now fail

**Verified:** the script exists but nothing invokes it — not the `Justfile`, not
`just check-everything`, not any workflow. It would also fail against the current tree by design:
its "no `.html` files in `docs/`" check is violated by eleven deliberate design studio pages, its
"no goose/geese references" check by
[the Goose competitive report](../research/coding-agent-landscape/goose.md), and all of its
exclusions are written as `grep -v 'superpowers/'` against a `docs/superpowers/` directory that
no longer exists. Also drifted from the inline copy embedded in the migration plan.

### A malformed command line preserved verbatim

[Skill bundles plan](../history/skills-packaging/skill-bundles-plan.md) Task 5 Step 2 prints
`cargo test -p biorouter --test-thread=1 …`. The flag is `--test-threads` and belongs after a
`--` separator; it will not run as written. Kept under the no-edits-to-commands rule.

### Load-bearing knowledge is duplicated across three to five places

The glibc 2.31 floor rationale, the winpthread link-order fix and `LZMA_API_STATIC=1` are each
documented in `ci-gate.md`, `CLAUDE.md` and `scripts/cross-env.sh`. The POSIX-only-safety-net
finding appears in three cross-platform docs. The workflow retry mechanism is described three
times and structured output twice. Nothing prevents the copies from diverging.

## Reported but verified as not an issue

Eleven reported concerns were checked against the code and found not to be real. They are
recorded here so nobody re-opens them.

- **"The daemon logs its full spawn env, including secrets, to stdout."** Asserted at
  [app test-drive runbook](../agent-drafter/testing/app-test-drive-runbook.md) line 105, where it
  is the stated rationale for every secret-handling warning in the document. **False.** Nothing
  in `crates/biorouter-server/` or `crates/biorouter/` dumps the environment: the only
  `std::env::vars()` call in the workspace is in `security/policy/target.rs:130` (policy
  evaluation, not logging), and `crates/biorouter-server/src/logging.rs:39` sends the console
  layer to **stderr**, not stdout. `ui/desktop/src/biorouterd.ts:126` logs only the binary path,
  port and directory before spawning, and relays whatever the child itself prints. The advice to
  keep daemon logs out of shared channels is still sensible prudence — but the reason given is
  wrong and should be rewritten rather than repeated.
- **Windows launchers missing from `export_app`.** They ship; see the doc/code contradiction
  above. `CLAUDE.md` is the stale side.
- **The `scripts/clippy-lint.sh` jq-parser bug**, independently diagnosed in four wave reports as
  a false-pass risk. **Fixed.** The script is now 40 lines with no `jq` at all — it sources
  `clippy-baseline.sh` and calls `check_all_baseline_rules`.
- **`main.ts` killing the shared backend when any window closes**, reported from the
  auto-visualiser stress test as a reproducible platform bug. **Fixed.** `main.ts:900-913` now
  refcounts via `retainBackend`/`releaseBackend` and kills only when the last dependent window
  closes.
- **`WorkflowsView.tsx:167` still routing to `/dashboard`.** Gone — the dashboard-removal commits
  landed after that was recorded, and grepping both `WorkflowsView.tsx` and `SessionListView.tsx`
  for `navigate('/dashboard')` returns nothing.
- **Absolute `/docs/guides/…` links surviving the migration**, reported by five agents across a
  dozen files. **Zero remain** in the tree.
- **`docs/sessions/README.md` and `docs/security/README.md` as stripped husks** with empty
  `## 📚 Documentation & Guides` sections. Repaired — no file in `docs/` still contains those
  markers.
- **macOS Keychain "single credential" vs Windows "chunked credentials".** Both are correct; it is
  real platform behaviour. `KEYRING_SERVICE = "biorouter"` is one service entry, and a comment at
  `crates/biorouter/src/config/base.rs:60-62` explains that Windows Credential Manager's
  2560-byte cap forces `secrets.1`, `secrets.2`, … continuation entries.
- **The llama-server checklist's "thinking-off" pass criteria being inverted.** It is not — the
  code defaults thinking to off. Only the cited *flag form* is stale (`--chat-template-kwargs`
  vs the current `--reasoning on|off`), and it is `CLAUDE.md` that is wrong here.
- **`--params` vs `--param` in [subworkflows](../workflows/subworkflows.md).** Correct as written:
  the reference documents `--params` on `run` and `-p, --param` on `workflow deeplink`/`open`.
  (The genuine error is in [the workflows README](../workflows/README.md), which uses singular
  `--param` with `run` — listed under Correctness risk adjacent issues.)
- **Dead `PATH_MAP.md` entries**, reported by two agents. `PATH_MAP.md` no longer exists anywhere
  in the tree and nothing references it, so the mapping rows are moot.

## Related documentation

- [Documentation house style](documentation-style.md) — the rules the 2026-07-18 pass applied, and the rules any fix to the issues above must follow
- [Docs migration history](../history/docs-migration/README.md) — the May 2026 Docusaurus-to-Markdown migration that caused the stripped code blocks and lost pages catalogued under Coverage gaps
- [Improvement proposals register](../history/agent-loop-review/improvement-proposals.md) — the authoritative index for every `BR-NN` identifier cited above
- [Agent-loop campaign outcome report](../history/agent-loop-campaign/outcome-report.md) — the merge-status document that several contradictions above turn on
