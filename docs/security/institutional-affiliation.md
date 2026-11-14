# Institutional affiliation and cross-institution warnings

> **What this is.** The user-facing guide to Biorouter's institutional affiliation check: why a
> cross-institution warning appears, what the values mean, what accepting one commits you to, and
> what it deliberately does not decide for you.
> **Status:** Proposed — the affiliation axis is built on the `feat/privacy-tiers` branch
> (issue #56) and is not in a released build yet. Behaviour described here is the branch's.
> **Audience:** researchers who have just met a cross-institution warning, and anyone deciding how
> strictly a lab machine should be configured.

You asked the agent to use a connector, or you switched the chat's model, and Biorouter said
something like this:

```text
Cross-institutional data flow. The extension `ucsfomopagent` holds data belonging to UCSF (ucsf),
but this chat is bound to a model covered by Stanford (stanford)'s agreements. Using it would send
`ucsfomopagent`'s inputs and results across that boundary. Compliance does not transfer between
institutions: a model approved at one has no permission over another's data unless a BAA, DUA or
IRB approval covers this specific flow. Proceed only if you know one does.
```

Nothing was sent. This page explains what Biorouter noticed, what it wants from you, and — the part
that matters most — what your answer does and does not settle.

## Why this warning exists

Biorouter already asks *how sensitive is this?* — that is the public/private tier, and it is what
keeps a private chat off an externally hosted model. Affiliation asks a different question:
**under whose agreements?**

The two do not compose. A HIPAA-compliant model approved at one institution has no blanket
permission over another institution's protected health information. Compliance is established **per
data flow** — by business associate agreements (BAAs), subcontractor chains, data use agreements
(DUAs) and IRB approvals — and it does not transfer merely because both ends of the flow happen to
be labelled "private".

The concrete case: UCSF's Versa model reaching the UCSF OMOP connector is the arrangement everyone
papered. The *same* Versa model reaching another site's connector is a cross-institutional linkage
nobody papered. Both are private-to-private, so both pass every tier check. Affiliation is the axis
that can tell them apart.

## What carries an affiliation

### A model

A model has one of three states:

| Value | Means | Examples |
|---|---|---|
| `Local` | Inference runs on this machine; nothing leaves it | Llama Server, or Ollama on a loopback address |
| An institution | Covered by that institution's agreements | Versa (Azure and Bedrock) resolve to `ucsf` |
| None | A public model — externally hosted, covered by no institutional agreement | Anthropic, OpenAI, Google direct |

Two notes on the edges. A self-hosted server at a **non-loopback** address — `OLLAMA_HOST` or
`LLAMACPP_EXTERNAL_HOST` pointed at a box in another building — is not `Local`: it is someone else's
machine, and it is treated as public rather than inheriting a local model's reach. And a
lead/worker pair discloses the whole transcript to *both* endpoints, so it is covered by both
institutions at once, which narrows rather than widens what it may reach.

### An extension

An extension has one of two states:

| Value | Means |
|---|---|
| Unconstrained | Safe for any private model. The default for a private extension with no institutional constraint |
| A named set | An allowlist: only models covered by those institutions may reach it |

The set comes from the marketplace registry, compiled into the build. Today exactly two extensions
are classified private — `cdwagent` and `ucsfomopagent` — and both name `ucsf`.

### A private dictionary term

Local dictation steers recognition toward the terms in your personal dictionary, and a term can
end up written into the text it produces. So a private term carries the institutions whose data
it may hold, and this axis decides which models it may reach. Design:
[private rows and institutions](../dictation/dictionary.md#private-rows-and-institutions).

A private term's institutions are those of the chat it was written in: the institutions behind
that chat's model, together with every institution whose connector that chat used. A chat on a
local model that read the UCSF OMOP connector therefore writes terms tagged `ucsf`. This holds
whether the agent added the term, a correction taught it, or you added it inside that chat with
**Add to Biorouter dictionary…** or `/dictionary add`.

| What the term records | When | Dictation it may shape |
|---|---|---|
| No institution | You added it with no chat: on the **Dictionary** page, with `biorouter dictionary add`, or by import without one | Dictation into any private chat |
| One institution | Its chat's model and connectors named one | Dictation aimed at a model of that institution alone, or at a local model |
| Two or more | They named several | Dictation aimed at a local model only |
| Unknown | Biorouter could not read its chat's record or model, or the chat named more than eight | Dictation aimed at a local model only |

Each institution is asked the same question this page describes for an extension, one at a time,
so a term tagged `ucsf` never shapes dictation aimed at a Stanford model, even though both are
private. Three things lift the check: privacy tiers switched off, the `open` mixing mode, and you
choosing **Include my private terms** for one recording after the warning, which says that
Biorouter cannot then keep your terms from other institutions' models.

Unlike an extension mismatch, nothing here asks you to approve: dictation makes no extension call
there could be an approval for, so a term the check leaves out does not shape that dictation, and
nothing in the result says it was left out. No control changes a term's institutions. To make a
term reach every private chat, delete it and add it again on the **Dictionary** page, which
records no institution.

Two limits are worth knowing. Content that reached a chat through a knowledge base search or a
recalled chat adds no institution, because only a call to an institution's connector is recorded,
and nothing is recorded while privacy tiers are switched off. And dictation into a chat whose
agent has not loaded yet does not know the chat's model, so it uses only the private terms that
carry no institution; the owner kept this behaviour on 2026-11-10 (O-43 in the
[dictation plan](../history/local-dictation-2026-11/implementation-plan.md#owner-decisions)).

## The rule, and the part everyone gets backwards

Affiliation is asked only after tier has been, and only when both ends are private. The whole rule:

| Your model | The extension | Result |
|---|---|---|
| `Local` | anything | Allowed |
| An institution | unconstrained | Allowed |
| An institution | an allowlist that covers your model's institutions | Allowed |
| An institution | any other allowlist | **Mismatch — you are warned** |

> **Warning.** `Local` is the **most** permissive value on this axis, not the narrowest. Readers
> assume the opposite — that a local model is the most restricted thing in the system — and it is
> exactly backwards.

Say why, because the reason is the rule: a local model **discloses nothing**. No data crosses an
organisational boundary, no third party receives anything, so there is no data flow for an agreement
to govern. A BAA exists to cover a disclosure; where there is no disclosure there is nothing to
cover. A local model therefore reaches everything private, including every institution's connectors,
and it is the one configuration this check never warns about.

The institution rows are a **subset** test, and the direction is load-bearing. A model covered by
`{ucsf, stanford}` reaching a `{ucsf}`-only connector is a mismatch: the Stanford half is in the
pipeline, so UCSF data would reach Stanford. "The allowlist names one of my institutions" is the
intuitive reading and it is the wrong one.

## What a mismatch actually does

It **warns, names both institutions, and proceeds if you accept**. It is not a block, because a
control researchers route around by switching the feature off protects nobody, and genuine
multi-site work under a real DUA exists. It is not silent either, because a flow nobody stated is
not a flow anybody accepted.

You meet it at three places:

- **The agent tries to call the connector.** The call is refused before anything is sent, the agent
  is told to ask you, and an **Approve this flow for `<extension>`** button appears under the
  refusal. Approving records your acceptance; the agent's next attempt goes through.
- **You switch the chat's model.** The switch succeeds and a warning notice names the flows that
  are now mismatched.
- **You attach another institution's connector.** The extension attaches and the same notice
  appears.

> **The agent can ask; it can never answer.** Approving requires proof the request came from the
> keyboard. No tool call, no MCP server, no subagent and no shell command can produce that proof —
> the acceptance has exactly one door and it is the button you press. An agent that hits this
> refusal is instructed to explain what it needs and stop.

Two related refusals **cannot** be cleared by approving: reading another chat's history, and reading
a knowledge base, when that content belongs to institutions your model is not covered by. There is
no button on those. The way forward is to switch this chat to a model covered by the institution in
question.

## What your approval covers

An approval is recorded against a **triple**: this chat, this extension, and this model's
institution. Biorouter states the scope before you press, in these words:

```text
Approving records your acceptance for this chat, this extension and this model's institution only.
It is not remembered for other chats, for other extensions, or if you switch this chat to a model
covered by a different institution's agreements. Each of those is a different data flow and would
be asked again.
```

Read as consequences:

- **Not per turn.** You are not asked again for every call in this chat. A control that fires
  constantly is one people click through without reading.
- **Never machine-wide.** The risk statement was about this conversation's data flow. Another chat
  gets asked again.
- **Per extension, not per category.** Accepting one connector's disclosure accepts that connector.
- **Invalidated by rebinding.** Switch this chat to a model covered by a different institution and
  the approved flow no longer exists — the next call is refused and stated afresh.
- **Inherited by subagents, never exceeded.** A subagent acts inside authority its parent already
  holds; it cannot create one.

One limitation worth knowing: the approval names the extension by its configured **name**. If that
entry is later reconfigured to point at a different server, the stored approval still matches a flow
whose owning institution you were never shown.

## Choosing how strictly this machine mixes institutions

Three modes govern the affiliation axis, and only that axis.

| Mode | A cross-institution mismatch | What it costs you |
|---|---|---|
| `open` | Resolves and is displayed, but never refuses | Nothing to click — and no statement of the flow before it happens |
| `standard` (default) | Refuses; one in-app approval clears it | One click per chat, extension and institution |
| `strict` | Refuses; clearing it also takes your operating system's own authentication | A real system password prompt each time |

> **All three keep the public/private barrier.** This setting decides how institutional mixing is
> policed and changes nothing else: private chats, private models and private extensions stay
> separated from public ones in every mode. `open` is not the master switch.

The setting lives in its own record beside `config.yaml`, not inside it — a value written into
`config.yaml` is ignored — and changing it requires proof the request came from the keyboard.
Tightening is free; **loosening** (for example leaving `strict`) raises a system authentication
prompt, so the careful configuration is never the expensive one to choose.

One honesty note about `open`: it silences the *extension* path. Reading another chat's history or a
knowledge base owned by an institution your model is not covered by still refuses, in every mode.
That is the safe direction — a surprising refusal rather than an unstated disclosure — but it is
narrower than the mode's name suggests.

## What this does not do

> **Warning.** Accepting a cross-institution warning is not authorisation. It is not a compliance
> determination, it is not an approval under any agreement, and Biorouter has no way to know whether
> a BAA, DUA or IRB approval covers your flow. The check surfaces a question; **your compliance
> office answers it.** A user who reads the button as permission is the failure this section exists
> to prevent.

The rest of the honest posture, stated rather than left to be discovered:

- **This is a safety boundary, not a security boundary.** It reliably prevents mistakes by someone
  who is cooperating: a forgotten model binding, an agent reaching for a clinical connector because
  the task mentioned patients, a chat that drifted across a boundary unnoticed. It is not built to
  withstand a determined operator or a prompt-injected agent.
- **The barrier sits above the filesystem, and the filesystem is open.** An agent with shell access
  can read files a private session wrote. Reaching a private connector from a public model takes two
  file edits — renaming the entry in `config.yaml` and deleting its provenance record. No rebuild is
  needed. That is a live query path to clinical data, not merely a leak of what was already written,
  and it is the reason the paragraph above says "safety".
- **The check is exactly as good as the tagging.** An extension the build does not know about is
  treated as public and unconstrained. An institution the registry does not publish a name for still
  counts — it is shown by its raw identifier rather than quietly dropped.
- **A private model that states no institution is treated as covered by none**, so it mismatches
  every institutionally-owned connector rather than being waved through.

If the flow you need is genuinely papered, approve it and record why. If you are not sure, the
answer is not in this application.

## Related documentation

- [Data privacy and patient data](data-privacy-and-phi.md) — which providers are acceptable for PHI in the first place, before affiliation is ever asked
- [Privacy tiers](privacy-tiers.md) — the public/private axis this one is orthogonal to, and the gates that enforce it
- [Permission modes](permission-modes.md) — the autonomy setting, which is **not** a lever on any privacy gate
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the full provider inventory, including which models are local
- [Dictation dictionary](../dictation/dictionary.md#private-rows-and-institutions): how a private dictionary term records its institutions, and the tests that hold the rule (P-16)
- [Security documentation index](README.md) — the other security layers and where each is decided
