# Data privacy and protected health information

> **What this is.** UCSF-specific guidance on which model providers are acceptable for
> protected health information (PHI), clinical records, and other sensitive research data,
> plus the de-identification and data-minimisation practices expected before any such data
> enters a BioRouter session.
> **Status:** Current. **Last reviewed 2026-08-06 — against the shipping provider inventory
> only.** That review corrected the local-model guidance, which previously named only Ollama and
> predated the bundled Llama Server provider. It did **not** re-confirm the institutional
> guidance on this page — which providers UCSF approves for which data classification — with
> UCSF compliance, and data use agreements change. Treat the provider names as current and the
> approvals as needing confirmation; verify with UCSF compliance before relying on this page.
> **Audience:** end users at UCSF handling patient, clinical, or otherwise regulated data.

BioRouter routes your inputs and conversation context to a large language model (LLM) provider
for processing. The privacy properties of a session therefore depend entirely on **which
provider you selected**, not on BioRouter itself. This page states which provider classes are
acceptable for which data classifications, and what to do before sending data at all.

> **Note — two local providers, one data-handling property.** BioRouter ships two local
> options: **Llama Server** (`llamacpp`, a bundled llama.cpp sidecar, ranked first and the
> first card a new user sees) and **Ollama**. Both run the model on your own device, so
> neither transmits your data to an external service — that property is what the guidance
> below rests on, and it holds for both. Which of them your institution *authorises* for
> regulated data is a separate question this page cannot answer; confirm with UCSF IT. See
> [choosing a model provider](../getting-started/choosing-a-model-provider.md).

## How provider choice determines data handling

Different providers have fundamentally different data handling policies:

- **Commercial cloud APIs** (Anthropic, OpenAI, Google, etc.) — data is processed on the
  provider's cloud infrastructure. Review the provider's privacy policy and data processing
  terms before use.
- **Institution-managed cloud services** (UCSF Azure OpenAI, UCSF Amazon Bedrock) — data is
  processed within infrastructure governed by UCSF's institutional agreements. These may offer
  stronger privacy protections than personal API accounts.
- **Local models** (bundled Llama Server, Ollama) — data is processed entirely on your own
  device. Nothing is transmitted to any external service.
- **Subscription-billed coding agents** (Claude Code, Codex) — a vendor CLI on your own machine
  runs the turn on your own consumer subscription. The subprocess is local but the **inference is
  not**: it happens at Anthropic or OpenAI, on a plan that carries **no BAA and no zero-data-
  retention agreement**. Treat these as commercial cloud APIs with weaker paperwork, never as
  local models — **no PHI**. Both are `Public`, so the bind gate refuses them to a private chat.
  See [coding-agent providers](../providers/coding-agents/compliance.md).

## What a non-private model can reach

Biorouter shows this to you the first time you bind a model that is not private, and keeps it in
front of you afterwards on the model chip, in Settings → Privacy, and above the Commercial section
of the provider grid. **It is shown whether or not privacy tiers are enabled** — turning the
feature off removes the enforcement, not the exposure, so with it off this is larger rather than
smaller.

> **{Provider} is not hosted by your institution.**
>
> It is not HIPAA-compliant, is not hosted on-premise, and does not run on this machine. It can
> read **files on this computer**. Anything a chat on this model can reach, it can send there:
> the contents of your working directory, and whatever a command you approve prints.
>
> Biorouter does stop three things: this model cannot read another chat's transcript, cannot read
> a knowledge base marked private, and cannot use an extension marked private or switch this chat
> to a private model to reach one.
>
> It **does not** stop it reading ordinary files on this computer through the shell, including files
> an earlier private chat wrote outside Biorouter's own storage. If the work involves patient
> data, use a local model or an institutional one.

The one-line form, which appears on the model chip and in `biorouter configure`:

> Not HIPAA-compliant, not on-premise, not local. This model can read files on this computer.
> Biorouter will not hand it another chat's transcript or a knowledge base marked private.

> **Maintenance.** These are the exact words of `COPY_LONG` and `COPY_SHORT` in
> [`crates/biorouter/src/privacy/disclosure.rs`](../../crates/biorouter/src/privacy/disclosure.rs),
> which is the single definition every surface renders. Quote them; do not paraphrase them. The
> app fetches them over `GET /privacy/disclosure` rather than shipping a second copy in the
> renderer, for the same reason: four hand-written copies drift within one release, and the
> drifted one is always the one somebody reads.

**Biorouter Copilot widens this past the working directory.** The disclosure above bounds the
exposure at files a chat can reach. With Biorouter Copilot approved for a request, the model also
receives screenshots of the display and the text of other running applications, including
anything on screen that has nothing to do with the task. A model that is not private is allowed
this, with an extra sentence in the approval saying so; the approval itself covers one user
request and ends when the reply finishes or you stop it. See
[permission modes](permission-modes.md#what-still-asks-whatever-your-mode).

## The provider guidance on this page is now enforced

Everything below used to be advice you could follow or ignore. Since **privacy tiers** shipped, the
central rule — *a conversation that has touched a private model or a private data source never
reaches a model hosted outside your institution* — is enforced by the app rather than left to you.

What that means in practice:

- **A chat remembers where it has been.** Run one turn on a local model or on UCSF Versa, or call a
  private data extension (UCSF OMOP, CDW), and the chat is marked **private**, permanently. It is a
  ratchet: it only ever goes up on its own.
- **A private chat cannot be switched to a commercial model.** The model picker, the CLI, the HTTP
  API and another chat steering this one all reach the same check and are refused the same way: the
  chat is left unchanged, the refusal names the **provider** it declined, and retrying does not
  help. Starting a *new* chat on the commercial model is always available and is the intended way
  through — the boundary is the transcript, not the model.
- **A public chat cannot reach private material through the tools it has by default.** It cannot
  call the private data extensions, read a private chat's transcript through chat recall, read a
  knowledge base marked private, or spawn a subagent on a private model to fetch any of it on its
  behalf. ⚠ **Two paths are exceptions and are listed under *What it does not do* below** — read
  them before relying on this bullet.
- **Knowledge bases carry the same mark.** A base takes the tier of the most sensitive chat that has
  written into it, and a public chat may not read a private base. You can publicize or privatize a
  base deliberately; a publicize tells you how many pages and sources it is about to release.
- **Institution matters, not just sensitivity.** A model your institution hosts has no blanket
  permission over *another* institution's regulated data — HIPAA compliance is established per data
  flow and does not transfer. A flow that crosses institutions is warned about, and accepting it is
  a deliberate act that is recorded.

**Undoing it is a deliberate, recorded act, and how deliberate depends on why the chat was marked.**
The single-confirmation path is narrow: it applies only to a chat Biorouter watched *run a turn* on
a private model. Everything else gets the stronger control — a chat that reached a private **data
source** (a private extension, a private knowledge base, or a private parent it was spawned from),
a chat that was imported, and, importantly, **every chat the one-time migration marked private**.
Those carry a "we inferred this from the model you last used" provenance rather than an observed
turn, so they take the strong path even though all they ever did was run turns; on day one that is
likely to be most of your private chats.
[What happens to your existing chats](privacy-tiers-migration.md#fixing-a-chat-the-migration-got-wrong)
explains why.

The stronger control asks for two proofs, because they answer different questions: you type the last
six characters of the session id (*which* chat), and you clear your operating-system password prompt
(*who* you are) — the same prompt macOS, Windows or Linux raises for any privileged action. Either
way the change is written to a ledger. From the terminal:
`biorouter session declassify <session-id>`.

**Turning it off is one switch, and it is not subtle.** Settings → Privacy disables every guardrail
on this machine, for every chat, behind a typed confirmation. Nothing is classified while it is off,
and re-enabling it does not go back and classify the gap. The disclosure above is shown *whether or
not* the feature is enabled, because turning it off removes the enforcement and not the exposure.

⚠ **What it does not do.** Privacy tiers govern what the *agent* can reach through Biorouter's own
storage and tools, and they do not encrypt anything at rest. Three gaps are known and deliberate
enough to name, because each one is a way a commercial model still reaches material you would call
private:

- **The shell reads ordinary files.** Nothing stops a public chat from reading files on this
  computer through the shell — including a file an earlier private chat wrote somewhere on disk.
  This is the one the guardrails were explicitly not extended to cover, and it is why the
  [what a non-private model can reach](#what-a-non-private-model-can-reach) disclosure is shown
  whether or not privacy tiers are enabled.
- **Workspace Control reads private transcripts.** If you have turned on the **Workspace Control**
  extension (it is off by default, and you get the whole tool set when you enable it), a public chat
  can read *any* other chat's transcript with `workspace_read_conversation` — including a private
  one. That tool checks only whether a chat is hidden, not its privacy tier, so the door chat recall
  closes is left open beside it. **"Chat recall refused it" does not mean the content is
  unreachable.** If you use Workspace Control and you keep PHI in chats, treat every chat on the
  machine as reachable by whichever model is driving.
- **The chat list is not filtered.** Anything that can talk to the local Biorouter daemon can list
  every chat on the machine — name, working directory and privacy mark, though not the contents.

That is why the guidance below still matters: **de-identify first, and pick the right provider
first.** The enforcement is a backstop for a chat that drifts into sensitive territory, not a licence
to start one there.

For the mechanics — what the migration did to chats you already have, and how each guardrail works —
see [what happens to your existing chats](privacy-tiers-migration.md) and
[privacy tiers](privacy-tiers.md).

## Patient data and sensitive research data

> **Warning.** If you need to work with patient data, PHI, clinical records, genomic data
> linked to individuals, or any data subject to the Health Insurance Portability and
> Accountability Act (HIPAA), institutional data governance policies,
> or other regulatory requirements:
>
> - **Use only institution-managed services or fully local models.**
> - Do NOT use personal commercial API accounts (for example, your personal Anthropic API key
>   or personal OpenAI account) with patient or sensitive data.
> - The safest option for data that must remain completely private is a **local model —
>   bundled Llama Server or Ollama** — because the data never leaves your device.

### Providers recommended for sensitive data

| Provider | Data stays within | Recommended for |
|---|---|---|
| **Llama Server or Ollama (local)** | Your device only — no external transmission | Highest sensitivity data, air-gapped requirements |
| **UCSF Azure OpenAI** | UCSF's institutional Azure tenant | Institution-approved use cases — verify with your institution |
| **UCSF Amazon Bedrock** | UCSF's institutional AWS environment | Institution-approved use cases — verify with your institution |

### Providers not recommended for patient data

These providers use personal or commercial API accounts and are generally **not appropriate**
for patient data without explicit institutional authorization:

- Anthropic (direct API)
- OpenAI (direct API)
- Google Gemini (direct API)
- OpenRouter
- Venice AI
- X.AI (Grok)
- Any other third-party commercial API

## Verifying before you begin

**Always verify with your institution before working with sensitive data.**

Even institution-managed services (UCSF Azure OpenAI, UCSF Amazon Bedrock) may have specific
terms of use, approved use cases, and restrictions that change over time. Before using
BioRouter with any sensitive data:

1. Confirm that your intended use case is covered by the institutional data use agreement for
   that provider.
2. Check with UCSF IT or your Institutional Review Board (IRB) or compliance office if you are
   unsure.
3. Ensure that the data classification level of your data is compatible with the service tier
   you are using.

UCSF policies around data handling, HIPAA compliance, and acceptable use of cloud services
evolve. The BioRouter development team cannot advise on the current status of institutional
agreements. Always check directly with UCSF compliance and IT.

## Handling data inside a session

**De-identify before using BioRouter.** Remove names, dates of birth, medical record numbers,
addresses, and other direct identifiers before inputting clinical data into any BioRouter
session, unless you have explicit authorization and a compliant data pathway to do so with
identifiers present.

**Minimize data exposure.** Provide only the data necessary for the task. Avoid pasting entire
datasets into the chat when a representative sample or summary would suffice.

**Use local models when possible.** For exploratory work, algorithm development, or testing
with real data, a capable local model — through the bundled Llama Server or Ollama — is the
safest option; see the provider table above.

**Review session logs.** BioRouter logs sessions locally. Session history stored in
`~/.config/biorouter/` on your device may contain data you entered. Protect access to your
device accordingly.

**Do not share sessions containing sensitive data.** BioRouter supports sharing sessions and
workflows. Do not share sessions that contain patient data or other sensitive information.

**Close clinical windows before approving Biorouter Copilot.** An approval lets the model capture a
whole display, not only the window you had in mind, so close or hide an EHR, imaging viewer or
spreadsheet before you grant it.

## Summary by data type

| Data type | Recommended approach |
|---|---|
| De-identified research data | Institution-managed providers, or a local model |
| Patient data / PHI | A local model only, or institution-managed with explicit compliance approval |
| Public / non-sensitive data | Any provider |
| Proprietary unpublished research data | A local model or institution-managed — verify confidentiality requirements |
| Anything visible on screen, once Biorouter Copilot is approved | Close or hide clinical and other sensitive windows first; the screenshot goes to whichever model the chat is bound to |

**When in doubt: use a local model (Llama Server or Ollama) or check with your institution.**

## Who to contact

For questions about data governance, HIPAA compliance, and approved data use pathways, contact
**UCSF IT Security or your departmental compliance officer** — not the BioRouter team, which
cannot speak to the status of institutional agreements.

UCSF BioRouter is developed by Wanjun Gu (`wanjun.gu@ucsf.edu`) at the
[Baranzini Lab](https://baranzinilab.ucsf.edu/) at UCSF, with support from UCSF IT and
Information Commons.

## Related documentation

- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the full
  provider inventory, including the institutional `versa_azure` / `versa_bedrock` providers and
  the bundled Llama Server.
- [Coding-agent providers — compliance](../providers/coding-agents/compliance.md) — why a
  subscription-billed Claude Code or Codex turn is not covered by any BAA, and the bind gate that
  keeps it away from a private chat.
- [Privacy tiers](privacy-tiers.md) — the design behind the enforcement described above: how models,
  chats, extensions and knowledge bases acquire a tier, and where each guardrail sits.
- [Privacy tiers — what happens to your existing chats](privacy-tiers-migration.md) — which of the
  chats you already have were marked private, and how to fix one the guess got wrong.
- [Permission modes](permission-modes.md) — limiting what the agent may do with the data once
  it is in a session.
- [Managed enterprise policy](managed-policy.md) — how an administrator enforces tool
  restrictions that a user cannot disable.
- [Sessions](../getting-started/managing-sessions.md) — what session history retains on disk, which matters if a
  session ever contained regulated data.
- [Secret storage](secret-storage.md) — where the provider API keys behind these choices are
  held.
