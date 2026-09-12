# Security

> **What this is.** The index for BioRouter's security documentation: how much autonomy the
> agent has, how an administrator constrains that autonomy, where credentials live, and which
> model providers are acceptable for patient and other sensitive data.
> **Status:** Current. This index was rewritten on 2026-07-18; the previous version was an
> empty husk left by the 2026-05-07 Docusaurus-to-plain-markdown migration, which stripped the
> JSX card components and linked none of the documents below.
> **Audience:** end users deciding how to run BioRouter safely, and administrators deploying it
> to a lab or institutional fleet.

Security in BioRouter is layered. You choose a **permission mode** that sets how freely the
agent may act; an administrator may impose a **managed policy** that overrides your choice on
specific tools; the OS credential store holds your **secrets**; and separately from all of
that, your choice of **model provider** decides where your data physically goes — and **privacy
tiers** now enforce that last choice rather than merely advising it, by refusing to let a
conversation that has touched a private model or a private data source reach an externally hosted
one. The documents below cover those layers in that order.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Permission modes](permission-modes.md) | The four modes — Completely Autonomous, Manual Approval, Smart Approval, Chat Only — and how to switch between them in the desktop app and the CLI. |
| [Managed enterprise policy](managed-policy.md) | The admin-owned policy tier that overrides user and project config for permissions and hooks: file locations, ownership verification, and the YAML schema. |
| [Secret storage](secret-storage.md) | How API keys are held in the macOS Keychain, Windows Credential Manager, or Linux Secret Service, why macOS prompts for a password, and the plaintext escape hatch. |
| [Secret guard](secret-guard.md) | The always-on floor that keeps credential files (`~/.aws/credentials`, SSH private keys, `secrets.yaml`, `.env`) away from the model in every chat and mode: how a tool call's arguments are resolved the way the shell will read them and refused on a match whether or not the file exists, the redaction of credential material in tool output, and the gaps a text scan cannot close. Rebuilt 2026-09-11 for QA-C H1. |
| [Data privacy and patient data](data-privacy-and-phi.md) | UCSF guidance on which providers are acceptable for PHI, clinical records, and other sensitive research data, plus de-identification practice. |
| [Institutional affiliation](institutional-affiliation.md) | **Proposed.** The user-facing guide to cross-institution warnings: why compliance does not transfer between institutions, the three values a model can carry and the two an extension can, why a **local** model is the *most* permissive rather than the narrowest, what accepting a warning covers (this chat, this extension, this model's institution — never machine-wide, never per turn), the `open`/`standard`/`strict` mixing modes, and the explicit statement that accepting is not authorisation. |
| [Privacy tiers](privacy-tiers.md) | **Implemented for v1.** The two-lattice capability system that stops a conversation touched by a private model or a private data source from ever reaching an externally hosted model: how models, sessions, extensions and knowledge bases acquire a tier, the eight enforcement gates, and the cost. **Narrowed by operator ruling on 2026-07-30 (DR-17):** the general filesystem read-deny of §9.5 is out of scope for v1, so a public model with a shell can still read files on the machine — what the gates stop is the agent-mediated path, the transcript path and tier escalation, and the residual is disclosed to the user rather than mechanised. **Read its "What shipped, and what did not" section before treating any other section as a description of the running system** — §7's cross-session matrix, in particular, is written and not wired. |
| [Privacy tiers — implementation brief](privacy-tiers-implementation-brief.md) | **Executed.** The entry point for implementing the design above: where to work, which operator rulings are settled, which enforcement approaches are already dead and why (four review rounds, four defeated designs), how to stage the work in four independently reviewable branches, and what verification does and does not prove. **Stage 3, the filesystem barrier, is descoped** (DR-17). Read this before the design or the plan. |
| [Privacy tiers — implementation plan](privacy-tiers-execution-plan.md) | **Executed** — and it is the record of *how*, including the review provenance this branch actually has (19% independent, 81% self-review) and the open questions implementation left behind. The execution plan for the design above, in seven phases — **six of its task units (14A–14F) are `DEFERRED` by DR-17 and two (29A, 30A) were added by DR-17/DR-18**: the tier model, ten enforcement gates (including the knowledge-base barrier the operator ruled on), the barrier surfaces, the badges and declassification flow, the marketplace, and the migration. Carries the non-negotiable orderings, the decisions of record (including DR-17's scope narrowing and DR-18's user-controlled knowledge-base tier), the costs the operator knowingly accepted — with the three the narrowing retired marked in place — and the anchor-drift table the design's own line numbers need. |
| [Privacy tiers — what happens to your existing chats](privacy-tiers-migration.md) | What the one-time privacy-tiers migration does to the chats you already have: which are marked private (the model each was **last** using, read from one field — never from the transcript), why the unknown cases are deliberately left public, what the day-one notice counts, and how to fix a chat the guess got wrong in either direction. |
| [Where the privacy campaign stands](privacy-tiers-campaign-state.md) | The continuity record: what actually shipped, what the barrier does and does not guarantee, what is left, and the constraints that cost time to learn. Read before the plan — the plan describes intent, this describes reality. |

## Where to start

- **Running BioRouter on your own machine for the first time?** Read
  [permission modes](permission-modes.md) first — it is the control that decides whether the
  agent can modify or delete files without asking.
- **Working with patient or clinical data?** Read
  [data privacy and patient data](data-privacy-and-phi.md) *before* your first session. The
  provider you pick determines where the data goes, and that choice is not reversible after
  the fact.
- **Deploying BioRouter to a lab or a managed fleet?** Read
  [managed enterprise policy](managed-policy.md), which is the only tier a user cannot turn
  off.
- **Being asked for your macOS password repeatedly?** See
  [secret storage](secret-storage.md).
- **A chat refusing to switch to the model you want, or a knowledge base you cannot read?** That is
  privacy tiers. Start with
  [what a non-private model can reach](data-privacy-and-phi.md#what-a-non-private-model-can-reach),
  then the two privacy-tier rows in the table above — the migration page explains why a chat you
  already had is marked private, and the design page explains the rest.

## Boundary with neighbouring folders

This folder holds the security **decisions** — what autonomy to grant, what an administrator
may enforce, where credentials are held, and which providers may see regulated data. The
mechanics of the files and variables those decisions are written into live elsewhere:

- **`configuration/`** holds the reference for `config.yaml`, `permission.yaml`, and
  `secrets.yaml` — their per-platform locations and every environment variable. Read this
  folder to decide what to set; read `configuration/` to find out where to set it.
- **`getting-started/`** holds the full provider inventory. The data-privacy page classifies
  providers by data handling; it does not enumerate them.
- **`agent-loop/`** holds the hooks schema and the managed policy tier's implementation design.
  Hooks are documented there as an agent-loop mechanism; this folder covers only the managed
  tier's authority over them.

## Related documentation

- [Hooks reference](../agent-loop/hooks/hooks-reference.md) — lifecycle hooks are the other
  governance surface the managed policy tier overrides.
- [Configuration file reference](../configuration/config-file-reference.md) — where
  `config.yaml`, `permission.yaml`, and `secrets.yaml` live on each platform.
- [Environment variables](../configuration/environment-variables.md) — the security-relevant
  variables, including `BIOROUTER_DISABLE_KEYRING`.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the full
  provider inventory behind the data-privacy recommendations.
- [Sessions](../getting-started/managing-sessions.md) — what session history retains on disk after a
  conversation ends.
