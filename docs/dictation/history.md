# Dictation history

> **What this is.** The design of the dictation history as it is built: what is recorded and when, the tier and institutions each entry is stamped with, the store and its file format, who may list and change it on each daemon, the managed policy key, the terminal commands, the agent's read only search, and the tests that hold each rule.
> **Status:** Current: built on branch `feat/dictation-hold-insert-history` (2026-11-26) under DD-96 and DD-100, which reverse the earlier rule that a transcript lives only in its composer (P-2) and await the owner's ratification, with SD-20 for `biorouter serve`. On 2026-11-27 the owner confirmed that dictations outside any chat are public under **Save new dictations as public** (O-48), and DD-102 keeps an open History tab current in every window.
> **Audience:** developers working on dictation, the dictation routes, the dictionary capability or the privacy gates; reviewers checking a change against P-2, P-17 and P-18.

Until this change a transcript lived only in the composer it went into, so text that was dictated and then lost (a closed editor, a refused paste, a crash) could not be found again. The dictation history keeps the text of every dictation a person started, on the host that transcribed it, private unless the person chooses otherwise. The audio is still never saved (P-1). How the history looks on each surface, the History tab and every message a person sees are in [surfaces](surfaces.md#dictation-history); this page is the part underneath.

Identifiers: DD numbers are the [decision register](../history/local-dictation-2026-11/implementation-plan.md#decision-register) of the dictation plan; P numbers are the [privacy invariants](dictionary.md#privacy-invariants); SD numbers are the [serve decisions](../deployment/serve-decisions.md); MSG ids are rows of the [message catalog](surfaces.md#message-catalog). Gate D and DR-33 are the [privacy tiers](../security/privacy-tiers.md) rules for searching chats and for ratcheting a reader.

## The three rules

| Rule | What it says | Where it is held |
|---|---|---|
| P-2, as amended | A transcript is kept in two places only: the composer it went into and, while **Dictation history** is on, the history of the host that transcribed it. Never in notifications, logs, `messages.metadata_json`, telemetry or the worker's stderr | `a_stream_leaves_nothing_on_disk_or_in_a_log_line` in `crates/biorouter-server/tests/dictation_routes.rs` (history off or no proof changes nothing on disk; with the proof and history on only `history.jsonl` changes; no log line holds the text) |
| P-17 | The history is the person's. The person lists, changes and deletes it; an agent only reads it, through one tool, under the tier rules. Private entries are listed over HTTP to the person alone | the route binaries below; `crates/biorouter/tests/dictation_history_readers.rs` |
| P-18 | The history never reaches recognition. The bias set, the chat extractor, usage counts and observations never read it; the path inspector refuses an agent's file tools that name it; diagnostics bundles leave it out | `recognition_never_names_the_dictation_history` in `dictation_history_readers.rs`; the inspector tests in `crates/biorouter/src/security/dictionary_store.rs`; `a_bundle_never_holds_the_dictation_history` in `crates/biorouter/src/session/diagnostics.rs` |

P-18 matters because Whisper copies its prompt into the transcript on silence ([dictionary](dictionary.md#the-bias-set)), so a prompt built from earlier dictations would put them into new ones.

## What is recorded

One entry per finished stream that a person started, written once, after the transcript is out:

- **On the daemon**, in `decode()` of `crates/biorouter-server/src/routes/dictation.rs`, once per stream, off the request's task, beside the usage record. A stream counts as the person's when `HttpCaller::records_dictation_history` admits it: the user action proof on a daemon that holds a key, and the person in the served page on one that holds none (SD-20). A caller holding only the daemon secret gets its transcript and leaves no entry.
- **In the terminal**, in `crates/biorouter-cli/src/dictation/history_record.rs`, called by `biorouter dictate` and by the terminal front ends (the TUI, the classic REPL, a shared chat and `session attach`) once their text is out. It takes the `CaptureConsent` that opened the microphone, which only a person at an interactive terminal with `BIOROUTER_TERMINAL` unset can hold, so an agent's shell never records.

Nothing is recorded for a stream that was cancelled, crossed by its `DELETE` or failed (a **Retry** is a new stream), that heard no speech, came back empty or came back made only of dictionary words, while **Dictation history** is off, or under managed `history: false`. `biorouter dictate --no-history` records nothing, and `biorouter transcribe` never records. A history that cannot be written never fails the dictation: the transcript is delivered and the entry is dropped, with one fixed log line, naming the stream's log id and never the text, unless the file was unreadable. The switches and the managed key are read when the entry would be written, so turning the history off while the person speaks is honoured.

**An open History tab follows every dictation** (DD-102). The window that dictated reads its tab again half a second after its own transcript. Every other Biorouter window learns of it from main: after the hidden capture page's finished transcript (the tray, the global shortcut and fn in another app) and after any window's `dictation:transcribed`, main sends `dictation:history:changed` to every window in its window list, with no payload and at most every 250 ms, and each History tab reads again half a second later, as it would for its own. The message carries nothing, so no text, id or tier travels between windows; each tab reads the listing itself, with its own proof. A dictation in the terminal reaches an open tab when its window gains focus or becomes visible, or at the next 60 s poll.

An entry (`HistoryEntry` in `crates/biorouter-dictation/src/history/schema.rs`, `deny_unknown_fields`):

| Field | Meaning |
|---|---|
| `id` | `dch_` and a random UUID v4, never derived from the text |
| `created_at` | when the transcript was ready, in whole seconds |
| `text` | the corrected text, at most 32 KiB, cut on a character boundary |
| `truncated` | set when the text was cut |
| `tier` | `public` or `private`, the dictionary's own tier type |
| `institutions` | private entries only, in the dictionary row's meaning (P-16): absent or `null` for none, `[]` for a model on this computer only, ids otherwise |
| `surface` | where the text went: `composer`, `crew`, `clipboard`, `other_app`, `app_preview` or `terminal` |
| `duration_ms`, `model` | the recording's length and the speech model's catalog id |
| `private_terms` | whether the person's private dictionary terms were eligible to shape the recording (its bias scope was private) |
| `session_id`, `session_incarnation` | the chat the text went into, when the caller could reach it, and the incarnation that chat had then |

The surface word comes from the stream request's optional `surface` field; a missing or unknown word is derived from `target_kind` (`chat`, `new_chat` and `editor` give `composer`, `crew` gives `crew`, `external` gives `clipboard`). An older daemon ignores the field. The same word is what the History tab shows and what the agent's search returns as `where`.

## The tier stamp

`history_stamp` in `crates/biorouter/src/dictation/history.rs` decides an entry's tier and institutions, and is called from exactly two files: the daemon's stream route and the CLI's recorder.

| `public_by_default` | Private terms eligible | Target | Tier |
|---|---|---|---|
| off | either | any | private |
| on | yes | any | private |
| on | no | a public chat the caller reached | public |
| on | no | a private chat, or a chat the caller could not reach or read | private |
| on | no | a new chat whose bind would be private | private |
| on | no | a new chat whose bind would be public | public |
| on | no | outside any chat (Crew, the clipboard, another app, the app preview, the terminal) | public |

Anything unknown fails toward private. The setting changes new entries only. With the setting on, dictations outside any chat are public: the owner confirmed it on 2026-11-27 (O-48).

A private entry's institutions are the union of the target chat's (as P-16 computes them for a dictionary row) and those of every private dictionary row the stream's bias set used. Any input that could not be read, a chat that could not be reached, or a union of more than eight ids gives `[]`, which only a local model clears; an empty union gives `null`. A dictation outside any chat, or into a new chat, adds no institution of its own, so it carries only its rows' institutions (whether those entries should instead be `[]` is another open owner question).

## The store

The leaf crate's `crates/biorouter-dictation/src/history/` decides nothing about privacy: a search takes the caller's visibility as a closure and applies it before the limit, and every write that names entries takes a closure that says which ones the caller may touch.

- **Where.** `<local data>/dictation/history/` (`history_dir()`, `Paths::in_local_data_dir`): `~/.local/share/biorouter/dictation/history` on macOS and Linux, `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\history` on Windows, where it never roams, and `<root>/data/dictation/history` under `BIOROUTER_PATH_ROOT`. A desktop app and a terminal on one account share it; each process holds one shared handle (`open_history()`).
- **Files.** `history.jsonl` (a header line `{"format":"biorouter.dictation.history","version":1}`, then one entry per line, oldest first), `settings.json` (`keep_history`, on by default, and `public_by_default`, off by default) and `.lock`. The folder is created 0700 and its files 0600 on Unix.
- **Writes.** An append takes the lock within 2 s or drops its entry, cuts a torn last line back to the last newline, writes one line and flushes it. Every other write (delete, clear, a tier change, a settings change) takes the lock, reads fresh and replaces the file atomically; a write that changes nothing writes nothing.
- **Caps.** At most 5,000 entries and 8 MiB. Passing either drops the oldest entries in the same write until both are at 90 percent. Pruning is silent, which the History tab's InfoTip says.
- **Failing closed.** A file with an unknown format, version or key, a middle line that does not parse, or two entries with one id is unreadable: reads answer MSG-HISTORY-FILE, every write is refused, and the file is never rewritten from a failed parse. An unreadable `settings.json` reads as nothing recorded and nothing public.
- **Reads.** Readers take no lock and cache the parse by the file's modification time, length, identity and, on Unix, change time. A search walks newest first, applies the caller's visibility, the time window and the query (the dictionary's fold key, as a substring) before an entry counts toward the limit, so private entries never use up a public caller's page; its total counts visible matches only.

## Who may read and change it

### Over HTTP

`crates/biorouter-server/src/routes/dictation_history.rs`, behind `check_token`:

| Route | Answer |
|---|---|
| `GET /dictation/history?q=&cursor=&limit=` | `{entries, next_cursor, total}`, newest first, `limit` at most 200 (100 by default), `q` at most 200 characters; an `ETag` over this caller's own filtered page, and 304 on `If-None-Match` |
| `DELETE /dictation/history/{id}` | 204 |
| `POST /dictation/history/delete` `{ids}` | `{deleted}` |
| `POST /dictation/history/clear` `{through}` | `{deleted}`, for the entries the caller can list up to that time |
| `POST /dictation/history/{id}/tier` `{tier}` | `{entry}` |
| `GET` and `POST /dictation/history/settings` | `{keep_history, public_by_default, authority}`, where `authority` holds `list_private`, `write`, `tier_change` and `raise_settings`, each `{allowed, reason}`, so a control that would be refused is disabled before the click |

An entry is shown without its incarnation and institutions, with the model's display name; `private_terms` only to a caller that may list private entries; and `session_id` only while the chat holding that id still has the recorded incarnation and the caller may list it, so an entry made public from a private chat never hands that chat's id to a public caller. Every `{id}` route sits behind `session_reach::gate_dictation_history_entry`, which answers 404 with MSG-HISTORY-NOT-FOUND for a missing id and an unlistable one alike, before the id is read. No answer counts withheld entries, and a refusal names no text. There is no route that adds an entry or edits its text: the stream route is the only writer.

The decisions are three `HttpCaller` methods in `crates/biorouter-server/src/routes/session_reach.rs`:

| Caller | Lists private entries | Writes |
|---|---|---|
| A request with the user action proof (the desktop) | yes | every write |
| The person in a `biorouter serve` page (SD-20) | yes | every write |
| A holder of the secret on a keyed daemon, without the proof | no | refused with MSG-HISTORY-AGENT |
| A holder of the secret on a keyless daemon who is not the served person | no | delete and clear of entries it can list, and lowering a setting; a tier change refused with MSG-HISTORY-HOST-TIER, a raise with MSG-HISTORY-HOST-SETTINGS |
| A program that states a private `X-Caller-Provider` | no | as its daemon's row above |
| Any caller, with the privacy master switch off (DR-15) | yes | as its daemon's row above |

The stated capability row is stricter than the dictionary on purpose: such a program is a model, and the history is a record of the person's speech from every surface, not one chat's data, so no model lists it over HTTP. A model reads the history only through the agent's search, below. The served person is a field of `HttpCaller` that only these three methods read; `session_reach` never reads it (`the_transcript_gate_does_not_read_the_served_person`).

### In the terminal

`biorouter dictation history` (`crates/biorouter-cli/src/commands/dictation_history.rs`) works offline on the store under its lock, so no daemon is needed:

| Situation | List and show | Delete and clear | Make public | Make private | Raise a setting | Lower a setting |
|---|---|---|---|---|---|---|
| An interactive terminal, `BIOROUTER_TERMINAL` unset | every entry | yes | after the typed word `public` | yes | after a question | yes |
| No terminal (a pipe, cron, `ssh host cmd`) | public entries, with no count of the rest | refused, exit 2 | refused | refused | refused | yes |
| An agent's shell (`BIOROUTER_TERMINAL=1`) | public entries, with no count of the rest | refused, exit 2 | refused | refused | refused | yes |
| Managed `history: false` | as above | yes | refused, exit 2 | refused | refused | yes |

An id the caller may not list answers exactly as a missing one (MSG-HISTORY-NOT-FOUND, exit 1). The commands are in the [command reference](../cli/command-reference.md#dictation-history-command-options).

### The agent's search

`dictionary__search_dictations` is the fourth tool of the `dictionary` capability (DD-100, P-7), in `crates/biorouter/src/agents/dictionary_extension.rs`. It is read only, and the capability's instructions tell the model to use it only when the person asks about something they dictated.

- **Input** `{query?, after?, before?, limit?, cursor?}`, with `limit` 1 to 50 (10 by default), `query` 1 to 200 characters and the times RFC 3339. Any other argument, `tier` and `include_private` among them, is refused before the store is read.
- **Output** `{"dictations":[{"id","at","where","tier","text"}],"next_cursor"}`, each text clipped to 1,200 characters around the first match. No count, no institutions, no chat id, so an empty answer is the same whether nothing matched or every match was withheld; a cursor that names a withheld entry reads as one that names none.
- **Where it answers.** Only in the person's own chat: a User chat that is not Crew scoped, outside any ACP process, and not when the newest user message was put there by an agent or a spawn.
- **What one call may see** is decided once, by `agent_reads_dictation` on the call's admitted `CallCapability`: Gate D's rule (hidden when the sample enforced the tiers and its tier may not see the entry's), then each institution the entry stores asked through the capability's own DR-26 check, exactly as a private dictionary row is asked (P-16). It reads only the capability it is handed, never the master switch or the provider slot, so a call admitted Public stays Public whatever happens to the chat while it runs. A public chat, the coding-agent bridge and `POST /agent/call_tool` (which dispatches as `public_enforced()`) see public entries only.
- **The reader ratchet.** Before a private entry reaches an enforced private capability whose chat row is still Public, the chat is raised through `raise_reader_privacy` with the reason `dictation:<id>` (DR-33). If that fails, the call returns nothing.
- **Refusals.** Managed `history: false` denies the tool, as does every existing denial of the capability (`enabled: false`, `dictionary: false`, `dictionary_agent_writes: false`), in the loop and at `POST /agent/call_tool`. The path inspector refuses `cat` and `text_editor view` of the history's files in both tiers, in the loop, at `POST /agent/call_tool` and in an `execute_code` script, with a refusal that names the dictation history.

## Managed policy

`history` is the ninth key of the managed `dictation:` block (`DICTATION_POLICY_KEYS` in `crates/biorouter/src/managed/dictation.rs`). `history: false` stops recording, denies the agent's search, and refuses tier changes and raised settings with the policy's sentence; listing, delete and clear stay open, so a person can still remove what was kept. It narrows nothing else: the dictionary, its other tools, capture and installs stay as they were.

⚠ The desktop app reads the laptop's own policy with a second copy of the key list (`LAPTOP_DICTATION_POLICY_KEYS` in `ui/desktop/src/utils/managedDictationPolicy.ts`). A key missing there reads as unknown and turns dictation off on the laptop for every policy that names it, so the two lists change together; `managedDictationPolicy.test.ts` reads the Rust list and fails when they differ. A binary from before this change also reads `history` as unknown, so a policy file that sets it turns dictation off on such a binary (the fail closed rule of O-27); release notes must say so. The block is documented for administrators in [managed policy](../security/managed-policy.md#the-dictation-block).

## What is and is not claimed

All of it is accident prevention for cooperative actors, as everywhere in Biorouter's privacy tiers (DR-17), not a boundary:

- A shell can read `history.jsonl` directly. The inspector stops a cooperative agent's file tools and named paths, not a command that assembles the path.
- On a `biorouter serve` daemon, a model with a shell that reads the browser token and the secret from the daemon's environment and forges the served page's headers with `curl` is treated as the person (SD-20, the residual SD-18 accepted).
- A `--no-token` `serve` daemon cannot tell its page from any other local caller, so nobody there is the served person: nothing is recorded and private entries are listed to nobody.
- Pruning at the caps is silent, and an entry is kept until the person deletes it or the caps drop it.

## Tests

| Test | Asserts |
|---|---|
| `cargo test -p biorouter-dictation --lib history` | round trip; an unknown key, format or version fails closed with the file byte identical; a torn last line ignored, then repaired; a bad middle line fails closed; two processes append under the lock; the caps prune to 90 percent in one write; the 32 KiB cut on a character boundary; 0700 and 0600; the cache reparses on an identity change; a search fills a public caller's page when private entries are newer |
| `cargo test -p biorouter --lib -- dictation::history` | the `history_stamp` table over the setting, the scope, every target and the institution union; the `agent_reads_dictation` truth table over caller tier, enforcement, affiliation and entry institutions, matching P-16; a source guard that `history.rs` spells neither `privacy_tiers_enabled` nor `CallCapability::sample` |
| `cargo test -p biorouter --lib -- security::dictionary_store managed::dictation session::diagnostics` | inspector refusals for the history in both tiers; `history_false_denies_the_search_and_nothing_else`; `a_bundle_never_holds_the_dictation_history` |
| `cargo test -p biorouter --lib -- dictionary_extension` | the `dictation_search` module: a public chat's full quota with 60 newer private entries, the empty answer, both tiers for a private chat, the ratchet and its failure, the closed schema, the session refusals, ACP, an agent's prompt, managed denial, a shell read refused, and `a_swap_after_admission_does_not_change_what_this_call_may_search` |
| `crates/biorouter-server/tests/dictation_history_routes.rs` | a proven stream records one entry with its facts; an unproven, cancelled, silent, echo, empty or crossed stream records nothing; history off and managed `history: false` record nothing; private entries listed to the person alone; a chat id shown only for a listable chat that still holds its incarnation; every write needs the proof on a keyed daemon; an entry that cannot be written still delivers the dictation, leaves the history as it was and logs one line that names the stream's log id and never the text (`an_unwritable_history_still_delivers_the_dictation_and_logs_no_text`, DD-102) |
| `crates/biorouter-server/tests/dictation_history_no_user_key.rs` | SD-20: the served person lists and manages private history; any other caller gets public entries and the host sentences, decided before the id is read; only the served person's streams are kept. Its own binary, because the user action digest is a process global `OnceLock` |
| `crates/biorouter-server/tests/dictation_history_oracles.rs` | an unlistable id answers byte for byte as a missing one; a private append changes neither the `ETag` nor the `total` a public caller polls; refusals name no text and no id |
| `crates/biorouter/tests/dictation_history_readers.rs` | only the listed files open the history, and recognition never names it (P-18) |
| `cargo test -p biorouter-server --lib -- routes::agent` | the search reaches public entries only at `POST /agent/call_tool`, also from a script, and never ratchets the chat it names; managed `history: false` refuses it there; a shell read of the history file is refused there |
| `cargo test -p biorouter-cli --lib` | `dictate` records `terminal` and `clipboard`; `--no-history` and `transcribe` record nothing; the terminal rules table above |
| `managedDictationPolicy.test.ts` | the laptop's key list equals `DICTATION_POLICY_KEYS` |
| `useDictationHistory.test.ts`, `dictationSession.test.ts`, `dictationAnywhere.test.ts`, `dictationAnywhereMain.test.ts` (DD-102) | an open tab reads again half a second after main says the history changed elsewhere, and after its own transcript, once each; a window tells main with the word alone; main's broadcast carries no payload, reaches its own windows only, at most every 250 ms, after the capture page's transcript and any window's |

The History tab, the Settings group and their tests are in [surfaces](surfaces.md#dictation-history) and [testing and release](testing-and-release.md#sidebar-and-settings).

## Related documentation

- [Surfaces](surfaces.md#dictation-history): the History tab, the Settings group, every message and the surface words.
- [Dictionary](dictionary.md#privacy-invariants): P-2 as amended, P-17 and P-18 beside the other invariants, and the capability's tools.
- [Serve decisions](../deployment/serve-decisions.md): SD-20, the served person and the history.
- [Managed policy](../security/managed-policy.md#the-dictation-block): the `history` key for administrators.
- [Privacy tiers](../security/privacy-tiers.md): Gate D, DR-17, DR-26 and DR-33, which the search and the stamp follow.
- [Command reference](../cli/command-reference.md): `biorouter dictation history` and `dictate --no-history`.
- [Implementation plan](../history/local-dictation-2026-11/implementation-plan.md#decision-register): DD-96, DD-100 and DD-102, the owner's ruling O-48, and the open owner questions.
- [Testing and release](testing-and-release.md): where these tests run.
