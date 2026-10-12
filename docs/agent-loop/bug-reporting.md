# The bug reporter

> **What this is.** The design of `platform__report_bug` — the tool that lets the agent gather a session's own evidence, work out what went wrong, and file a Biorouter issue after the user approves the exact text. Covers where it lives, why it is two calls, the `/bug` command that starts it, how it checks the documentation and the source before filing, what a private chat may do with it, where it is not offered, and how it reaches GitHub.
> **Status:** Current.
> **Audience:** contributors

The user-facing guide is [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md).
This page is the reasoning behind it.

## Why a tool at all

Before this, reporting a bug took three clicks to reach a modal whose "File Bug on GitHub"
button opened a template with the system information filled in and **nothing else** — no
description of what went wrong, no failure list, and a diagnostics zip the user had to
generate separately and drag on by hand. The information a maintainer actually needs was
in the session, and the one participant that had read the session was the agent.

So the shape is: the agent reads its own transcript, distils it, and asks the user to
approve a finished report.

## Where it lives, and why it is not an extension

It is the fifth `platform__*` tool — declared in
[`agents/platform_tools.rs`](../../crates/biorouter/src/agents/platform_tools.rs),
dispatched by `Agent::dispatch_tool_call`, implemented in
[`agents/bug_report/`](../../crates/biorouter/src/agents/bug_report/).

A platform tool rather than a `PlatformExtension` because it needs the session row and the
session manager, which `PlatformExtensionContext` does not carry — the same reason
`platform__ingest_source` is dispatched by the agent loop. Widening that context to carry
a provider is a documented security regression, so it is not the answer here either.

Adding a fifth name to `PLATFORM_TOOL_NAMES` is what makes it survive the Code Execution
filter and stay out of the JS module catalogue; that list is the single source those gates
read, and two tests pin the membership and the dispatch branch.

## Two calls, not one

| Call | Reads | Writes | Approval |
|---|---|---|---|
| `action: "analyze"` | The session's failed tool calls, graded | Nothing | None |
| `action: "file"` | The report the model wrote | A GitHub issue, or a prefilled page the user submits | Proof-backed, showing the body |

The action is **inferred toward `analyze`** whenever the model does not say — including for
an unrecognised value, and for a call carrying a title but no description. A model that
omits an enum must not thereby publish something.

## The `/bug` command

`/bug [description]` is the entry point a user types. It is a built-in agent command, a
`CommandDef` in [`agents/execute_commands.rs`](../../crates/biorouter/src/agents/execute_commands.rs),
so every surface that reads `list_commands()` picks it up with no edit of its own: the
desktop `/` menu (as a **BUILT-IN** row), the terminal UI's palette, help and forwarding,
and the reserved workflow names. The classic CLI keeps a hand-written completion list and
help table, and both carry it; a test asserts every `list_commands()` name is in the
completion list.

The command files nothing itself. `handle_bug_command` returns one of two things.

- **An answer for the person, with no model turn**, when the tool could not be used
  anyway. It samples `user_proof_available()` once (false on a `biorouter serve` daemon)
  and asks Crew whether the chat may use `platform__report_bug` (a Crew-scoped chat admits
  only `crew__` and `todo__` tools, and a Crew registry that cannot be read counts as a
  refusal). These are the checks `Agent::dispatch_tool_call` makes first. Sending the
  model a prompt to call a tool it was not given ends in an apology that helps nobody, so
  `bug_command_unavailable` writes the reason for the person and says where to report
  instead.
- **A model-only expansion** otherwise. The agent loop stores the typed `/bug …` as
  user-visible and agent-hidden, and the expansion as user-hidden and agent-visible, then
  runs the turn. The expansion names the tool in both spellings (a coding-agent child sees
  `mcp__biorouter__platform__report_bug`), says to call it with `action: "analyze"` first,
  and wraps the user's words in `<user-bug-description>` as data to pass on unchanged.
  It never starts with `/`. It carries no investigation method and no URLs: the tool
  description and the analyze result own those, so each is written once.

The desktop appends composer chips to the message as markup, and the expansion treats the
two kinds differently. One function splits them out for both the expansion and the
evidence: `resource_refs::split_composer_text`, which follows the renderer's
`splitComposerText`. It claims only a tag the desktop draws as a chip (a known `type`
naming a non-empty value) and a quotation block it draws as a card (exactly
`<biorouter-quote>`, a valid payload, a non-blank `text`); anything else stays in the
prose as typed. `<biorouter-ref …>` tags are dropped from the description: they
still resolve from the user-visible copy of the message, so quoting them as the user's
words would only add noise. `<biorouter-quote>…</biorouter-quote>` blocks are taken out of
the description too, but carried in a separate paragraph of the expansion, marked as quoted
source data rather than the user's own words. Nothing re-attaches a quotation from the
hidden row, so dropping it would lose it.

The split has two consequences elsewhere.

- **The evidence has to read what the user typed, not the expansion.**
  `evidence::recent_user_text` skips every row whose `metadata.user_visible` is false
  (slash expansions and hook context are both stored that way) and strips the same chip
  tags. Without the filter, the reporter quoted its own instructions back to the user as
  "your last message". `/bug` is also one of `TRIGGER_PHRASES`, so
  `/bug the chart panel is blank for one row` yields "the chart panel is blank for one
  row" as the description.
- **`bug` is now a reserved name.** A workflow shortcut someone had bound to `/bug` stops
  resolving, because reservation follows `list_commands()`.

In the desktop composer a leading `/bug ` becomes a chip, drawn like the reference chips
skills and knowledge bases produce. The wire format does not change: the message still
starts with `/bug <text>`, because `execute_command` reads the command there, and chip
reference tags stay appended at the end. `utils/composerCommand.ts` claims only a
lowercase `/bug` followed by exactly one space at index 0. A bare `/bug` stays text, so
splitting a message and joining it back is lossless and the caret never moves under the
user. Sent, queued and history messages draw the same chip.

⚠ A private chat whose turn Gate B refuses (it found no private model to bind) never
reaches `execute_command`, so `/bug` is refused along with the turn. The Diagnostics
dialog's **File Bug on GitHub** needs no model and is the path there.

## Finding what went wrong

There is no failures table, no error index and no persisted failure record in Biorouter.
`tool_monitor`'s `ToolOutcome` is in-memory and per-turn, so by the time anyone says
"report a bug" it is gone. The durable account of a failed call is the tool response in
`messages.content_json`, and that is what `evidence.rs` reads.

Two things about that data decide the implementation:

- **A failure has two spellings**, and the commoner one does not look like a failure.
  `{"status":"error"}` is a transport-level failure; `{"status":"success","value":{…,"isError":true}}`
  is a tool that ran and reported a domain failure — a build that broke, a query that
  errored. Both are handled by going through `tool_errors::classify`, which already knows
  the difference, rather than by testing `status` directly. A scan for the first alone
  reports a clean session while the user looks at a red error.
- **Some failures are Biorouter refusing on purpose.** The only failed call in the one real
  session read while designing this was privacy Gate C refusing
  `workspace_read_conversation` on a public model — not retryable, `ToolFailure` kind, a
  long error string, indistinguishable from a hard bug on every coarse signal. Filing it
  would file "the privacy boundary worked" as a defect. Such failures are detected from
  constants `privacy::refusal` exports (so a reword there is a compile error, not a silent
  mis-grade), **labelled rather than hidden** — a refusal genuinely can be the wrong thing
  refused — and excluded from the evidence's own "is this conclusive?" test.

## The push-back

`is_conclusive()` is not "a failure exists". One retryable 429 is the ordinary weather of a
long session; bad arguments are the model's own mistake, which it can already see. When
nothing conclusive happened **and** the user has not said what to report, `analyze` returns
an instruction to ask the user a specific question and not to file until they answer.

That is a plain result, not an error. An error invites a retry, and a retry cannot produce
information the model does not have.

The next step it names is another `analyze` call with the user's own words in
`description`, not a `file` call. That holds after asking the user, and also when the
earlier messages already describe the problem. Routing back through analysis is what puts
the investigation guidance below in front of the model before it writes any report.

## Investigating before filing

When `analyze` can tell what went wrong (the conclusive branch, which a description always
reaches), it follows the digest with investigation guidance. One helper builds that text,
so every path that needs it says the same thing. The steps:

1. Compare what the user expected with what happened, from the chat's own record.
2. Check whether the behaviour is documented and intended, at
   <https://biorouter.ucsf.edu/docs>. That is one large page, past the 128 KiB a web fetch
   returns inline, so the guidance also names its sources: `docs/website/pages/<page>.html`
   in the repository, with contributor docs under `docs/`.
3. Read the code involved at the version the user runs:
   `https://github.com/BaranziniLab/biorouter/tree/v{ver}`, and raw files at
   `https://raw.githubusercontent.com/BaranziniLab/biorouter/v{ver}/<path>`, where
   `{ver}` is the evidence's `app_version`. If that tag does not exist, use `main`. If
   `main` differs, the bug may already be fixed, and the report says so. A working
   directory that is a Biorouter checkout is read in place. The `develop-biorouter` skill
   maps where code lives.
4. Use whatever web fetch or shell tool the chat has. With none, the report says the
   source and the docs were not checked.
5. If the docs and the code show Biorouter behaved as designed, explain the intended use
   to the user instead of filing, and file only if they still want to. A misleading doc or
   error message is itself worth reporting.

The source link is pinned to the tag because the website deploys from `main` and can
describe a newer version than the user runs. The docs are a claim to check against the
code, not the answer. The links name `issue::DEFAULT_REPO`, not `issue::repo()`: the
second is where a report is sent, which `BIOROUTER_BUG_REPORT_REPO` can point at a fork or
a scratch repository, while the code the user runs is still Biorouter's.

**Where the URLs live.** The system prompt (`prompts/system.md`) names the docs site and
the repository once, so the model knows both exist on every turn. The version-pinned links
appear only in the analyze result, which is read only while reporting. The tool
description carries no URLs.

**In a private chat** the guidance adds a fetch rule: fetch only fixed public pages on
github.com, raw.githubusercontent.com and biorouter.ucsf.edu; never put text from the
conversation into a URL, query string, search or command that reaches the network; write
about Biorouter's behaviour, not the user's data. ⚠ That sentence is the only control
here. No privacy gate restricts outbound fetches of public URLs (`webdocuments` and
`developer` both classify Public), so a GitHub code search built from the conversation
would send that text to github.com before any card was shown. The
[user guide](../troubleshooting/diagnostics-and-bug-reports.md#reporting-from-a-private-chat)
states this as an instruction the agent is given, not a guarantee, and should stay that way
until something enforces it.

Fetched pages are tool output, which the system prompt already frames as data rather than
instructions, so nothing extra guards against a page that tries to steer the model.

## The suspected cause

The report keeps observation and diagnosis apart. `description` is what was observed,
facts only. `suspected_cause` is the diagnosis for the maintainer or debugging agent who
takes the report over: likely files and functions, linked at the version tag; the
hypothesis; the evidence (what the transcript shows, what the code does, what the docs
say, and where the docs and the code disagree); and a confidence of low, medium or high.
When nothing grounds a diagnosis, the model says so in the field instead of guessing: what
it checked, what it could not check (no web-fetch or shell tool, for example), confidence
low. A chat with no such tool records "not checked" here too. The tool used to forbid any
guess about the cause. The split keeps that intent for `description` and gives whoever
picks the issue up somewhere to start.

`render_body` always emits a **Suspected cause** section, between **Expected behavior** and
**Please provide the following information**. When the field is present, the section opens
with a line saying it comes from the reporting agent's analysis of the session, the
documentation and the source at `v{ver}`, and is not yet confirmed by a maintainer. When it
is absent or blank, the section holds the fixed line `issue::NO_SUSPECTED_CAUSE`, so a
debugging agent can tell "not diagnosed" from a section that went missing. `prepare_report`
scrubs the field like `additional`. A cause over 4,000 characters is a validation violation the model fixes
by shortening, which keeps the body well inside the compose URL budget below.

It is not in `REQUIRED_SECTIONS`. The repository's template carries it as an optional
heading, so a report written by hand from the template still validates without it.

## The redaction harness

Two shapes, deliberately different:

- **`scrub`** rewrites what it recognises — home paths (to `~`), other accounts' usernames,
  vendor tokens, JWTs, bearer tokens, credential assignments (keeping the *key*, dropping
  the value), URL passwords, e-mail addresses.
- **`validate_issue`** re-runs the scrub and **refuses** anything still recognisable, plus
  a missing template section, an unusable title, or an over-long body.

One pass that both rewrote and approved would report success for every pattern it forgot.
Nothing here is a guarantee — a secret that looks like prose survives any pattern set — and
the design assumes the person reading the approval card is the last check.

The section list `validate_issue` requires is asserted against the repository's own
`.github/ISSUE_TEMPLATE/bug_report.md`, so a template rename fails a test rather than
silently producing reports that no longer match it.

Two defects surfaced once a suspected cause made quoted code likely:

- **The scrub re-matched its own output.** A credential assignment becomes
  `KEY=[redacted]`, and on the second pass the value class matched `[redacted` again, so
  `validate_issue` refused every report that had contained one. When the match sat in the
  automatic failure list, the model could not edit it and the report could never be filed.
  The replacement now leaves a value that already starts with the placeholder alone and
  does not count it as a finding, so `scrub` is idempotent.
- **Type annotations read as secrets.** `api_key: String` or `token: Option<String>` was
  rewritten, which garbled quoted code. A value that is exactly a common type expression
  (`String`, `Option<…>`, `Vec<…>`, `SecretString` and a short list more) is now exempt.
  Nothing broader is: a capitalised identifier can be a real secret. That holds inside the
  angle brackets too. A generic's arguments must be a known primitive or a capitalised name
  with no digits, behind optional lowercase `seg::` segments, and nothing may follow the
  closing `>`. The first version accepted any alphanumeric argument and a single `:`, so
  `password=Vec<hunter2xyz>` and `auth:Vec<u8>:secret:S3cr3t…` were published whole.
- **Rust paths read as assignments.** The rule took the first `:` of `::` as its
  separator, so `crate::oauth::oauth_flow` became `crate::oauth=[redacted]` with the code
  span left open, and the receipt counted a credential. A bare `:` separator may no longer
  be followed by a second `:` (`=` and `: ` still may, so `password=:hunter2` stays
  redacted). A source location such as `token_counter.rs:100-200` is exempt too, when the
  key ends in a source file extension and the value is only a line, a range or a column.

## What the user approves

The card carries the rendered body **verbatim**, names the destination repository (loudly
if it has been redirected away from the project's own), says whether pressing the button
publishes immediately or opens a page the user still has to submit, and sets
`requires_user_proof`. Those facts are the consent: a card reading "file a bug report?"
would be asking about a category, not about the paragraph that is going to be
world-readable.

Ordering follows `install_extension` — preflight, await approval, re-check that nothing
changed between the card and the click, then act.

⚠ The terminal used to approve the report unseen. The classic CLI's `find_tool_confirmation`
hands its prompt only the card's id and `prompt`, the TUI's permission modal receives only
the prompt, and the tool-request line above either shows the model's raw arguments rather
than the scrubbed body. `confirmation_artefact` (`biorouter-cli/src/session/mod.rs`) now reads
`repository`, `title` and `body` from a `platform__report_bug` card. The classic prompt
prints them between the card's prompt and the question; the TUI modal opens on the prompt
and carries the report below it, read with PageUp and PageDown. Other tools are left alone,
because their arguments are what the tool-request line already shows.

## Private chats: the card warns, the user decides

A chat classified `Private` can report a bug. The reporter used to refuse there, on the
grounds that nothing could certify the model's distillation carried none of the private
material. That made being unable to report a bug the price of a chat being private, which
the Diagnostics dialog had already rejected: it warns about a private chat's bundle and
generates it anyway. The rule now is that the user decides, and the agent never publishes
from a private chat by itself.

- **No filer that publishes on approval.** `choose_filer` never returns `GhCli` for a
  private chat, whatever `gh` can do, and `gh` is not even probed (the probe is a network
  round trip before the card). The report goes out as a prefilled compose page, or is
  handed back to paste when it is too large for a URL. The disclosure is then the user's
  own **Submit** on GitHub, with the text in front of them.
- **A loud card.** The approval prompt opens with a sentence saying the chat is private
  and why, that the report becomes public if submitted, what to look for in it (patient or
  participant data, credentials, unpublished results, institutional information), that the
  failure list quotes raw tool output, and that nothing is posted automatically. The reason
  clause (`private_reason_clause`) reads "ran on a private model" for a `turn:*`
  `privacy_reason`, and takes `privacy::declassify::strong_confirmation_reason`'s wording
  for the other reasons that vocabulary names. An absent or unrecognised reason gets the
  plain "This chat is classified PRIVATE.", because that function's catch-all is a
  sentence about the record, written for the declassify control. The desktop
  draws any `prompt` as a warning banner and hides Always Allow beside it, so this needed
  no frontend change. `requires_user_proof`, `risk: High` and the verbatim body are
  unchanged.
- **The stored tier wins over the turn's snapshot.** The classification is read from the
  session row at filing time, together with `privacy_reason`, and combined with the
  snapshot by `max`, so a chat that turned private mid-turn files as private. A failed
  read counts as private too, and the card says Biorouter could not confirm the chat's
  classification. That case used to be a tool error.
- **The DR-15 master switch** still applies. With tiers off, a `Private` marking was
  written by machinery the user disabled, so the chat files like any other. The card's
  `chatPrivacyTier` (`private`, `public` or `unknown`) names the stored classification
  either way.

The decision is `treat_as_private(tier, tiers_enabled)`, a pure function with the switch
passed in. An earlier predicate read the global switch itself, and flipping it in one test
broke unrelated tests.

⚠ The compose URL carries the body in its query string. Opening the page sends the text to
github.com and leaves it in browser history before anyone presses Submit. "Nothing is
posted until you press Submit" is true of posting, not of sending, so the card's private
sentence says the page address carries the text to github.com, and asks the user to read
the report before approving rather than before submitting.

## Where it is not offered

- **Where no person can approve.** A `biorouter serve` daemon holds no proof-of-user key,
  so the approval refuses forever there. Both halves are withheld, including the read-only
  analysis: a tool that lets the model write a whole report and *then* discover it has
  nowhere to put it is worse than one that is absent. `/bug` answers the person directly
  and points at the issue tracker.
- **In a Crew-scoped chat.** Crew admits only `crew__` and `todo__` tools there, so the
  roster hides the reporter and dispatch refuses it. `/bug` says to report from a regular
  chat.

## Coding-agent children

It **is** available to a coding-agent child (Claude Code, Codex). That took a dispatch arm
of its own: `ChatBridgeDispatch` routes tools by name and hands everything it does not
recognise to the extension manager, which has never heard of a `platform__*` tool, so a
bridged platform tool without an arm answers `Tool not found` — after the child has already
written a whole report. Three things make it work, and each is a place the next platform
tool will need too:

- `dispatch_report_bug`, beside its two ingest siblings in `ChatBridgeDispatch`.
- A `bug_report` flag on `CodingAgentBridgePlan`, **not** a capability target. Every other
  bridged tool is reached through a bundled capability or an installed extension, and
  `enforce_tool_access` re-checks that grant at dispatch — but filing a bug belongs to no
  extension and is not something the user switches off in Settings. Its one gate is
  `user_proof_available()`, the same as in the main roster. Riding on the Knowledge target,
  which is how the two ingest tools travel, would mean a chat without a knowledge base
  cannot report a bug.
- An explicit arm in `enforce_tool_access` before the grant lookup, because a tool with no
  grant would otherwise fall through to "not in this turn's coding-agent bridge".

⚠ Adding a name to the bridge's roster is a security-surface change: that allowlist is
described in the code as "the reviewed builtin router rosters" and deserves a human look
rather than a silent edit. Two things bound the risk here. The child reaches the tool over
the same relay every other bridged tool uses, so it still runs behind Biorouter's
inspectors, permission mode and privacy gates; and the approval still has to reach a
person — on a coding-agent turn the child is blocked on `POST /tool_bridge/{nonce}`, which
is parked on the card, so `next_provider_wake` is what surfaces it (the #107 mechanism).
Both coding-agent providers are also `ProviderTier::Public`, so Gate A has already refused
to bind one to a private chat, and a bridged report never comes from one.

## Every model, one contract

The tool is reached from every provider Biorouter supports — Anthropic, OpenAI,
Versa, Bedrock, Ollama, llama.cpp, and a Claude Code or Codex child over the
bridge — and the tool schema is the only contract between them. Two divergences
are measured behaviours in this tree rather than hypotheses, and
`normalize_arguments` undoes both:

- **An envelope around the arguments.** `autovisualiser::normalize_dashboard_args`
  exists because GPT-5.5 wraps a whole argument object in a `data` envelope and
  retries identically after a rejection. `arguments`, `report`, `issue`, `bug`,
  `data` and `params` are unwrapped — but only when the outer object names none
  of the tool's own fields, so a real call carrying its own `data` keeps it.
- **Stringified structure.** `de_flexible` / `de_stringified` exist for the same
  reason. A wholly stringified argument object, a stringified envelope and a
  stringified `steps` array are all parsed.

`action` matching is case- and whitespace-insensitive and accepts the British
spelling. That is not politeness: exact matching sent `"File"` to the analyze
half, which answers *"now call me with `action: file`"* — so a model that
capitalises loops forever, being told to do the thing it just tried. The safety
direction is unchanged, because it is about the UNRECOGNISED case: absent,
misspelled or nonsense still lands on the half that cannot publish, and
`normalisation_never_turns_an_unrecognised_call_into_a_publish` pins that
against every shape above.

⚠ A tool that only works for the house style of whichever model it was written
against is one that silently stops working when the user switches models — which
they do, from the composer, mid-chat.

## The card has to reach the user, and once did not

⚠ Worth reading before adding any tool the agent loop dispatches itself.

`platform__report_bug` parked its approval correctly, the card was published to
`ActionRequiredManager`, and **no `actionRequired` frame ever reached the reply
stream**. The turn stopped with no dialog and no explanation, and the parked call
would have sat out its full 15-minute time-to-live unanswerable.

The cause is structural, not a race. `handle_approved_and_denied_tools` awaits
`dispatch_tool_call` in a sequential loop, and for an **extension** tool that is
harmless: `ExtensionManager::dispatch_tool_call` returns a `ToolCallResult` whose
`result` is a *deferred* future, so the tool's body runs later, inside the batch,
where `next_batch_wake` already races the card drain. The `platform__*` tools are
dispatched by the agent loop itself, and those branches `.await` their handler
and wrap the finished value (`ToolCallResult::from` is `future::ready`). Their
whole body therefore runs during gating — before `combined` exists and before
`next_batch_wake` is ever entered, so nothing is draining cards.

The fix is `next_gate_wake`, the third wake site of a shape the loop already had
twice: `next_provider_wake` races the provider call, `next_batch_wake` races the
batch, and gating was the one long await that could park and was not raced. It
covers every agent-loop-dispatched tool, present and future.

⚠ The first guess was that `install_extension` had the same problem and
marketplace installs were unapprovable. It does not and they are not — it is an
extension tool, so its body runs in the batch. The rule is about **who dispatches
the tool**, not about which tool it is.

`the_card_reaches_the_stream_while_the_tool_is_still_parked` is the regression
test, and it is deliberately not "was the card yielded". An earlier test asked
only that and passed throughout, because its denier polls the pending-action
registry directly and answers the card whether or not it reached the stream; the
tool then returns and the queued message is drained afterwards — late, but
present. The regression test closes the loop through the stream itself: the
reader signals when it *yields* a card, and only then is the card answered. On
the broken code that signal never comes and the test times out, which is exactly
what the user experiences.

## Reaching GitHub

Nothing in this tree had ever authenticated to the GitHub API. Every existing call is
read-only and unauthenticated; the single `gh` shell-out lives in a CLI workflow whose auth
helper launches an **interactive** `gh auth login`, unusable from a tool call. So there are
two filers:

1. **The user's own `gh`**, when `gh auth status` succeeds *non-interactively* — stdin is
   `null` and prompting is disabled, because the failure mode being guarded against is not
   "gh is missing" but "gh would open a login and hang the turn". This genuinely creates the
   issue, under the user's own account, with no credential passing through Biorouter. It is
   never used from a private chat, because it publishes the moment the card is approved.
2. **A prefilled compose URL**, opened in the browser; the user's click is the submit.

Not a third option: a token Biorouter stores. It would need the credential store, a scope
the user has to reason about and a revocation story, to replace a `gh` most of this
project's users already have.

Which filer a report gets is decided before the card, so the card can say which:

| Chat | `gh` signed in | Body fits a URL | Filer |
|---|---|---|---|
| Not private | Yes | Either | `GhCli`; a `gh` failure falls back to the compose URL, or to pasting |
| Not private | No | Yes | `ComposeUrl` |
| Not private | No | No | `Manual`: the text is handed back to paste |
| Private, or classification unknown | Not probed | Yes | `ComposeUrl` |
| Private, or classification unknown | Not probed | No | `Manual` |

A `Manual` hand-back names `gh` as a reason only when `gh` was probed and found
unavailable (`Filer::Manual { gh_unavailable }`, set by `choose_filer`). From a private
chat it was never probed and may well be signed in, so the card and the receipt say only
that the report is too large for a prefilled link and is not filed automatically. Both
take that clause from `issue::manual_reason`.

**The compose URL opens itself.** After approval, `post_report` opens a `ComposeUrl`, and
the compose URL a failed `gh` filing falls back to, with `issue::open_in_browser`. That
calls `webbrowser::open` on the blocking pool, from the process running the agent (the
desktop app's daemon, or the CLI), the same crate the daemon uses to open an OAuth
sign-in. Until then the card promised that the page would open while the code only
returned the link for the model to relay, and models garble long URLs. The result now says
whether the page opened, gives the link either way as a fallback, and tells the model to
pass it on exactly as written. Under test, `open_in_browser` returns false without opening
anything.

⚠ There is a **size cliff** between the two. GitHub answers 414 on a compose URL well before
any browser's own limit, and the body is percent-encoded on the way in — markdown roughly
triples. So the same report can be fileable through `gh` and far too large for a URL, and
`compose_url` returns `None` rather than producing a link that 414s. The cap is applied to
the *encoded* URL, not the raw body.

`gh_ready` and `file_with_gh` refuse outright in any test binary. `running_under_test` is
`cfg!(test)` **or** an executable under cargo's `deps/` directory, because `cfg!(test)` is
false inside an integration test such as `bug_report_agent_loop`. A test that approved the
card would otherwise create a real, public, permanent issue from `cargo test`, on whatever
machine happened to have `gh` signed in.

## The bundle is not attached

The tool posts a short distilled report; it never uploads a diagnostics zip. The zip
contains `session.json`, which is the whole transcript and is **not** redacted — see the
warning in the [user guide](../troubleshooting/diagnostics-and-bug-reports.md). Attaching it
stays a deliberate act by the user, and the receipt says how. From a private chat it is
the other way round. `render_body` leaves the footer's invitation to attach the bundle out
of the body, because that body is the page the user reads on github.com just before
pressing Submit, and it does not say why (that would put a fact about the chat on a public
page). Every private receipt, the compose page's and the hand-back's alike, tells the
model to say now, not "if they ask", that the bundle holds the private conversation
unredacted and must not be attached to a public issue.

## Tests

```sh
cargo test -p biorouter --lib -- bug_report platform_tool       # the reporter, the roster and dispatch invariants
cargo test -p biorouter --lib -- execute_commands slash_command  # /bug, its expansion and the reserved name
cargo test -p biorouter --lib -- resource_refs                   # the shared chip and quotation split
cargo test -p biorouter --test bug_report_evidence               # against a real exported session
cargo test -p biorouter --test bug_report_agent_loop             # a model's call through a real Agent::reply
cargo test -p biorouter-cli --lib                                # /bug in the classic CLI's completion and help
cd ui/desktop && npx vitest run src/utils/composerCommand.test.ts  # the composer chip's lossless split
```

The `bug_report_evidence` binary runs the extractor against an actual `session.json` produced by
`generate_diagnostics` on a running desktop app — 30 messages, four extensions, a bridged
coding-agent provider — with the user's own content replaced and both failure spellings
injected in the exact form `tool_result_serde` writes. It is there because the unit tests
build their conversations in Rust and can therefore only exercise shapes the author already
believed in; the real export is what turned up the third failure nobody had planned for.

The private-chat rules are pinned as pure functions, `treat_as_private` and
`choose_filer`, as well as through the card. `gh_ready()` is always false under test, so no
card test could ever observe a `GhCli` choice, and a test that only went through the card
would pass whether or not a private chat was kept away from `gh`.

## Related documentation

- [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md) — the user-facing guide to this tool, the diagnostics bundle, and filing an issue by hand.
- [CLI command reference](../cli/command-reference.md#slash-commands) — `/bug` among the slash commands the desktop, the terminal UI and the classic CLI share.
- [Tool routing](tool-routing.md) — how a tool call reaches its handler, and where the platform tools sit in that path.
- [Privacy tiers](../security/privacy-tiers.md) — the classification lattice the private-chat warning reads, and the master switch it honours.
- [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md) — the symptom-by-symptom reference to check before filing anything.
