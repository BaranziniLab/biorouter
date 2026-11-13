# Dictation dictionary

> **What this is.** The design of the personal dictionary that dictation recognizes: its data model and store, how privacy tiers filter it, who may read and change it on each surface, how it steers recognition and correction, how it learns from the person's edits, the `dictionary` capability the agent uses to ingest terms, automatic extraction from chats, the left sidebar **Dictionary** page and the terminal commands.
> **Status:** Current: the record of local dictation as it was built on branch `feat/local-dictation`, PR-01 to PR-33 (PR-07 and PR-32 withdrawn), with every code pull request committed by 2026-11-10 and before the release that carries it. Design rationale and the measurements taken while building it are kept with their dates; "As built" notes say where the code differs from the design.
> **Audience:** developers who maintain the store, the bias set, the capability, the extractor and the Dictionary page (PR-13, PR-14, PR-16, PR-20, PR-22, PR-23, PR-24, PR-26); reviewers of changes to them; the owner, whose decisions O-3, O-17, O-23 and O-31 to O-38 of 2026-11-05, O-43 and O-44 of 2026-11-10, and the plan's defaults O-5 to O-8 and O-21 this document follows.

Each account that runs Biorouter has one dictionary: a file of terms such as UCSF or EPAS1 that dictation should spell the person's way. The daemon, the CLI and the agent's `dictionary` capability read and write that one file under one lock. Dictation feeds the eligible terms to the speech model as a token prompt and a decoder bonus, then corrects the transcript with guarded exact matching. Because Whisper can copy its prompt into the transcript when it hears no speech, a term learned in a private chat must never shape dictation aimed at a public model, so every channel reads one tier filtered set. This document specifies each part to the level an engineer can build from, with the acceptance check for each. The [implementation plan](implementation-plan.md) holds the decision register this document elaborates; where they differ, the plan wins.

## How to read this document

Load bearing claims carry a label:

| Label | Meaning |
|---|---|
| [M] | Measured on 2026-11-04 during this design, on real binaries, browsers or audio. Accuracy numbers here are design measurements on benchmark audio, not claims about the shipped product (DD-95) |
| [C] | Read in Biorouter's code at `main` (`52917f33e`, three commits after the v1.94.1 tag; the workspace version is 1.94.1); a line number is against `main`. Citations added after a pull request landed name the file and symbol on `feat/local-dictation` at that pull request; the symbol is the anchor, not the line |
| [R] | Reported by a third party's source, documentation or issue tracker, not measured here |
| [I] | Inferred; the text names the measurement that would settle it when it matters |

Identifiers:

| Scheme | Meaning | Index |
|---|---|---|
| R1 to R8 | The owner's requirements | [Implementation plan](implementation-plan.md), "The owner's requirements" |
| S1 to S20 | The canonical surfaces | [Surfaces](surfaces.md) |
| DD-1 onward | Design decisions; this document elaborates DD-33 to DD-59, DD-67, DD-68, DD-71, DD-75 and DD-77 | [Implementation plan](implementation-plan.md), "Decision register" |
| PR-01 to PR-33 | Pull requests in delivery order (PR-07 and PR-32 withdrawn) | [Implementation plan](implementation-plan.md), "Delivery plan" |
| O-1 onward | The owner's decisions of 2026-11-05 and the plan's defaults; the text follows each | [Implementation plan](implementation-plan.md), "Owner decisions" |
| M-1 onward | Measurements, defined in testing and release and dated in the checklist's log | [Testing and release](testing-and-release.md#measurements) |
| MSG-… | Exact text a person sees | [Surfaces](surfaces.md), "Message catalog" |
| P-1 to P-16 | Privacy invariants, each held by a named test | [Privacy invariants](#privacy-invariants) |

Code that exists at `main` is cited as a file plus a symbol. Files this plan creates are written as code spans, never as links. Rust and TypeScript blocks are sketches of the shape to build, not finished code.

## What the dictionary is for

| Req | The owner's words, condensed | How this document meets it |
|---|---|---|
| R3 | A personal dictionary of words (UCSF, EPAS1) that dictation recognizes. It grows with the person's workflow, and they can add to it as they go. | The store ([The data model](#the-data-model), [The store](#the-store)); recognition with the dictionary ([How dictation uses the dictionary](#how-dictation-uses-the-dictionary), [Correction after decoding](#correction-after-decoding)); nine ways to add: the sidebar page, **Add to Biorouter dictionary…** in the context menu, **Add to dictionary** on a tool row, `/dictionary add`, `biorouter dictionary add`, import, learning from the person's corrections, the agent's capability and the chat extractor |
| R4 | A component in the left sidebar where people add, maintain and delete entries. | A **Dictionary** row in the sidebar's Components group at `/dictionary`, the same page on the desktop and in every `serve` browser ([The left sidebar Dictionary page](#the-left-sidebar-dictionary-page)) |
| R5 | A plugin or MCP extension that ingests keywords from conversations into the dictionary, "kind of like the same idea as putting things into memory". | The `dictionary` capability with three tools, plus the chat extractor ([The `dictionary` capability](#the-dictionary-capability), [Automatic extraction from chats](#automatic-extraction-from-chats)) |

The default reading of R5:

1. **Plugin or MCP extension.** The capability is a platform extension in the core crate. The agent reaches it through the same tool dispatch as any MCP extension, as `dictionary__add_terms`, `dictionary__list_terms` and `dictionary__remove_added_terms`. It is a platform extension rather than a bundled MCP server because it must read the caller's capability from the dispatch and the calling session's messages, and because app manifests already refuse a platform name they do not list (DD-51).
2. **From conversations.** The agent may take terms from anywhere in the conversation it sees, not only from text the person typed (O-31, DD-55; decided by the owner on 2026-11-05).
3. **Like memory.** Memory has two stores. Its project store is written at once, but its instructions tell the model to act when the person asks and otherwise "offer to remember it and wait for agreement before writing" [C `crates/biorouter-mcp/src/memory/mod.rs` `base_instructions`]. Its machine wide store needs the person's approval for every read, write and delete [C `crates/biorouter/src/security/global_memory.rs` `GlobalMemoryInspector`]. The owner decided on 2026-11-05 that the dictionary follows memory's own protocol by default (`on_request`, O-3): a term the person asked the agent to add, or named in a reply to the agent’s offer, can land at once when the agent marks it requested; the store does not check whether the reply agreed, including “no, thanks” (O-34). Every other agent term lands as a suggestion the person accepts, because the dictionary is machine wide. Waiting for review always (`suggest`) and immediate writes with **Keep** and **Undo** (`add`) are the person's settings on the Dictionary page and in `biorouter dictionary settings --agent-writes`, not new code (DD-53). The default departs from memory once: the capability is off until the person ticks **Suggest dictionary words from my chats** at the first model install, while memory is on in every chat by default [C `crates/biorouter/src/config/extensions.rs` `inject_builtin_extensions`, `builtin_entry` with `enabled: true`].
4. **Ingestion that needs no agent turn.** The chat extractor finds term shaped tokens in the person's own messages and proposes them as suggestions. It shipped switched off: M-21, its precision gate (DD-57), was not run, because the precision run waits for the owner's own labels (O-44).

## The data model

The schema is v1, snake_case on disk and in the HTTP API (DD-34). It lives in `crates/biorouter-dictation/src/store/schema.rs` (PR-13).

### Entry fields

| Field | Type and limits | Set by | Meaning |
|---|---|---|---|
| `id` | `dct_` plus a random UUID v4 | the store | Opaque; never derived from the term |
| `rev` | integer, starts at 1 | the store | Per entry revision; the `If-Match` token of an edit. Usage never changes it |
| `written` | NFC, trimmed, 1 to 64 characters, at most 6 words | person, agent, extractor, learner, import | The spelling dictation writes |
| `tier` | `public` or `private` | the server's stamp, never a client | Which dictations may use the row ([Who stamps the tier](#who-stamps-the-tier)) |
| `status` | `active` or `suggested` | store rules | Only `active` rows reach recognition |
| `source` | `user`, `import`, `agent`, `auto`, `correction` | the writer | Where the row came from; picks the page group |
| `reviewed` | bool | the store | `false` on an `agent` row the person has not kept; `true` everywhere else |
| `spoken_forms` | at most 8 strings, each validated like `written` | the person, import | How the person says it (`epas one`); correction sources only, never prompt or bonus content |
| `misrecognitions` | at most 16 `{text, count, last_seen}` | the learner, import | What the model wrote instead; used by correction once `count` reaches 2 |
| `kind` | `gene_symbol`, `acronym`, `name`, `term` or null | the writer | Sets the default `match` and case handling |
| `match` | `fuzzy`, `exact` or `prompt_only` | defaults by kind; the person | How far correction may go ([Defaults by kind](#defaults-by-kind)) |
| `bias` | `auto`, `always` or `never` | default `auto`; the person | `always` is "pinned"; `never` keeps the row out of the prompt and the bonus |
| `context_cues` | at most 8 strings, each at most 32 characters | the person | Words that license a wordy alias or a fuzzy edit when they occur in the utterance |
| `note` | null or one line, at most 280 characters | the person | Free text shown on the page |
| `reason` | null or one line, at most 140 characters | the agent | Why the agent proposed the term |
| `suggested_in` | session id or null | extractor, agent, learner | The chat it came from; shown only to callers with private standing |
| `seen` | integer | the store | Distinct sightings (sessions for the extractor and the agent, record ids for the learner) |
| `created_at`, `updated_at` | RFC 3339 | the store | |
| `provenance` | null or `{reason: "publicized_by_user" \| "privatized_by_user", at}` | tier changes | Records the person's tier change |
| `affiliation` | private rows only: `null`; `{"institutions": [id, …]}` with 1 to 8 normalized institution ids; or `{"institutions": []}` when unknown (a chat record, a chat model or an imported value that could not be read) | the server's stamp, never a client | The institutions whose data the row may hold: those behind the model of the chat it was written in and those whose connectors that chat had touched; `bias_set` keeps the row out of dictation aimed at a model any of them does not admit ([Private rows and institutions](#private-rows-and-institutions), O-32, O-35, O-36) |

The store file:

```json
{
  "format": "biorouter.dictionary.store",
  "version": 1,
  "settings": { "learn_from_chats": false, "agent_writes": "on_request" },
  "entries": [
    {
      "id": "dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60",
      "rev": 3,
      "written": "EPAS1",
      "tier": "public",
      "status": "active",
      "source": "user",
      "reviewed": true,
      "spoken_forms": ["epas one"],
      "misrecognitions": [{ "text": "a pass one", "count": 2, "last_seen": "2026-11-04T17:02:11Z" }],
      "kind": "gene_symbol",
      "match": "exact",
      "bias": "auto",
      "context_cues": [],
      "note": null,
      "reason": null,
      "suggested_in": null,
      "seen": 1,
      "created_at": "2026-11-01T09:00:00Z",
      "updated_at": "2026-11-04T17:02:11Z",
      "provenance": null,
      "affiliation": null
    }
  ],
  "tombstones": [{ "fold_key": "phd2", "tier": "private", "at": "2026-11-03T12:00:00Z" }]
}
```

`learn_from_chats` in a new store takes the compiled default `LEARN_FROM_CHATS_DEFAULT`, which PR-24 sets from the M-21 result ([The M-21 gate](#the-m-21-gate)); it ships `false`, because M-21 has not run (owner decision, 2026-11-10). `agent_writes` starts at `on_request` (O-3, decided by the owner on 2026-11-05).

PR-16 adds one optional key, `sightings`: the observations the store has applied ([The observations route](#the-observations-route)), so a resend counts once. Each holds digests only, never the correction's words: `key`, the first 32 hexadecimal digits of the SHA-256 of (`record_id`, `from`, `to`), the stamp `tier`, `at`, and for a rejection `rejected`, the digest of (entry id, `from`) whose second distinct sighting changes the entry. At most 1,000 per tier are kept, the oldest dropped first (`MAX_SIGHTINGS_PER_TIER`). A store that has had no observation writes no `sightings` key, so its bytes are what PR-13 wrote. At the cap the key adds 208,021 B to a 7,800 row file of 4,064,540 B, and the parse a dictation's `view()` pays once after each change takes 5 % longer (136 ms against 130 ms in a debug build, `store::observe::tests::measure_the_sightings_cost`) [M, 2026-11-06]; at the release build's 8.7 ms reparse that is about 0.4 ms, paid once per change to the store and never between a stop and the text.

The Rust shape:

```rust
// crates/biorouter-dictation/src/store/schema.rs
pub const STORE_FORMAT: &str = "biorouter.dictionary.store";
pub const STORE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct StoreFile {
    pub format: String,
    pub version: u32,
    pub settings: StoreSettings,
    pub entries: Vec<Entry>,
    pub tombstones: Vec<Tombstone>,
}

#[derive(Serialize, Deserialize)]
pub struct StoreSettings { pub learn_from_chats: bool, pub agent_writes: AgentWrites }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentWrites { Suggest, OnRequest, Add } // ordered: raising means a later variant; a new store starts at OnRequest

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EntryTier { Public, Private } // Private > Public, so a stamp is a max

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: EntryId,
    pub rev: u64,
    pub written: String,
    pub tier: EntryTier,
    pub status: EntryStatus,          // Active | Suggested
    pub source: EntrySource,          // User | Import | Agent | Auto | Correction
    pub reviewed: bool,
    pub spoken_forms: Vec<String>,
    pub misrecognitions: Vec<Misrecognition>,
    pub kind: Option<TermKind>,       // GeneSymbol | Acronym | Name | Term
    #[serde(rename = "match")]
    pub match_policy: MatchPolicy,    // Fuzzy | Exact | PromptOnly
    pub bias: BiasPolicy,             // Auto | Always | Never
    pub context_cues: Vec<String>,
    pub note: Option<String>,
    pub reason: Option<String>,
    pub suggested_in: Option<String>,
    pub seen: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provenance: Option<Provenance>,
    pub affiliation: Option<RowAffiliation>, // private rows only (O-32)
}

pub struct Misrecognition { pub text: String, pub count: u32, pub last_seen: DateTime<Utc> }
pub struct Tombstone { pub fold_key: String, pub tier: EntryTier, pub at: DateTime<Utc> }
pub struct RowAffiliation { pub institutions: Vec<String> } // normalized ids, empty when unknown (O-35); the core maps them (P-16)
```

### Validation

`crates/biorouter-dictation/src/store/validate.rs` is the one validator. The routes, the capability, the extractor, the learner, the CLI and import all call it. A term (and each spoken form, misrecognition and cue) is refused when:

| Code | Rule |
|---|---|
| `empty` or `too_long` | fewer than 1 or more than 64 characters after NFC and trim (cues: more than 32) |
| `too_many_words` | more than 6 whitespace separated words |
| `line_break` | contains CR, LF, NEL (U+0085), or the Unicode line or paragraph separator (U+2028, U+2029), which are not control characters |
| `control_character` | contains any Unicode control character, or a formatting character that hides or reorders text (zero width characters, the direction overrides and isolates, the word joiners, the byte order mark, the soft hyphen) |
| `angle_bracket` | contains `<` or `>` |
| `url` | contains `://` or starts with `www.` |
| `email` | has the shape `local@domain.tld` |
| `path` | starts with `/`, `~`, `./`, `../` or a drive letter, or contains two or more `/` or `\` separators |
| `digits_only` | only digits and separators |
| `long_hex` | a run of 16 or more hexadecimal digits |
| `secret_shaped` | a word of 12 or more characters that starts with a known key prefix (`sk-`, `AKIA`, `ghp_`, `gho_`, `ghu_`, `ghs_`, `github_pat_`, `xox`), or a base64 shaped run of 32 or more characters that holds a digit, a capital and a small letter. A long hyphenated phrase passes: one with no digit never mixes all three, and a run whose hyphen separated pieces are each a word of at most 16 letters (`Factor`, `alpha`, `RNA`), a number of at most 6 digits or a symbol of at most 8 capitals and digits (`HIF1A`) is a phrase, so `Hypoxia-Inducible-Factor-1-alpha` and `Interleukin-1-Receptor-Antagonist` pass. A random key's pieces mix cases and digits, and an `_`, `+`, `/` or `=` makes a piece no word |
| `too_many_forms` | more than 8 spoken forms, 16 misrecognitions or 8 cues on one row |
| `common_word_misrecognition` | a misrecognition that is a single common English word, on an `agent`, `auto` or `correction` row. The learner applies the same rule to every row it changes ([Effects on the dictionary](#effects-on-the-dictionary)). It needs the English word list, so it arrives with PR-14; PR-13 has no writer that stores such a row |

The leaf crate cannot call the core's secret scanner, so every caller that can (the routes, the capability, the extractor and the CLI) also refuses a term for which `biorouter::guardrails::secret_output::redact_text` returns `Some` [C `crates/biorouter/src/guardrails/secret_output.rs` `redact_text`]. `StoreFile::import` takes that scan as an argument, so an import refuses the same terms as an add.

### Uniqueness, fold keys and twins

- Rows are unique on (`written` exact after NFC, `tier`) (DD-34). The same `written` may exist once as public and once as private (DD-35); this document calls such a pair twins.
- `fold_key` is derived on load and never stored: case fold, strip spaces, hyphens and dots, Greek letter names to letters (`HIF-2alpha` and `HIF2α` both fold to `hif2α`). It drives tombstones and the "same letters" warning.
- An add whose `fold_key` matches a row the caller can list is allowed and answered with MSG-SAME-LETTERS: "A term with the same letters exists: `<written>`". Gene symbols are case significant (human EPAS1, mouse Epas1), so this is a warning, never a merge.
- Two active rows with the same `fold_key` and a different `written` disable that key in the exact key matcher, so correction never rewrites one into the other.
- Twins are merged for recognition inside `EligibleEntries` when both are eligible: the union of spoken forms, active misrecognitions and cues; the larger usage figures; pinned if either is pinned; the stricter `match` (`prompt_only` over `exact` over `fuzzy`); `bias = never` if either says so. Twins never disable each other's key.

### Derived flags

Computed per row on load from the embedded word list ([The English word list](#the-english-word-list)), never stored:

| Flag | True when |
|---|---|
| `collides` | the written form, or its letter run before a letter and digit boundary, case folds to a word or listed abbreviation (SPOKE, PHD2, CRISPRi, UCSF through `uc`) |
| `digits_literal` | `collides` and the written form contains a digit (PHD2) |
| `letter_digit` | the written form matches `^[A-Za-z]+[0-9]` or holds both a letter run and a digit run |

### Usage

`usage.json` sits beside the store with two maps of the same shape: `{"all": {"<id>": {"uses": 14, "last_used_at": "2026-11-04T17:02:11Z", "corrected_to": 2}}, "public": {"<id>": {…}}}`. It is a separate file so a usage bump never changes an entry's `rev` (a spurious `412` on a person's open edit) and never rewrites the entries file per utterance (DD-33). `uses` and `last_used_at` move when a dictation's final text contains the row's written form as a whole word (exactly as written, or, for a form written with a small first letter such as `metformin`, capitalized as at the start of a sentence; no letter or digit may touch it, so `EPAS1` is not in `EPAS10`), once per dictation, for the rows of that dictation's own bias set (`DictionaryRequest::usage_bumps` in `bias.rs`, `RecognitionEntry::occurs_in`), and only after the text is out: the write is never on the path from stop to text. `biorouter transcribe` flushes its one dictation's batch after it has written the transcript and, on Unix, pointed its standard output at `/dev/null`, so a reader of a pipe has the whole text before the write starts; it tries the store's lock once rather than waiting, so the exit never waits for another writer. It raises nothing with `--no-dictionary`, under the managed `dictionary: false`, or in an agent's shell (`BIOROUTER_TERMINAL` set), which writes nothing else in the dictionary either (PR-14b; before it, `uses` stayed 0 after 168 transcriptions [M]). The file is replaced by a rename with no flush to disk, because its counters are loss tolerant and each flush (an `F_FULLFSYNC` on macOS) sat before the one shot command's exit; a power cut can lose the last batch or leave the file empty, and an empty file starts over, while one this version cannot parse is still left as it is. `corrected_to` moves when the person corrects dictated text to the row ([Effects on the dictionary](#effects-on-the-dictionary)). Every bump goes to `all`. A dictation decoded with the public rows table, and a correction stamped public, also bump `public`. Ranking for a public scope request reads only `public`, so private chats' dictation never changes which public terms a public prompt carries (oracle rule 11). The usage fields a caller with private standing sees come from `all`. The daemon's service (PR-15) batches bumps in memory and flushes at most every 30 s and at the worker's idle unload; every such flush waits at most the extractor's lock budget, 50 ms (`USAGE_FLUSH_BUDGET`), and a lost bump is acceptable.

### Caps per tier

Caps count only the caller tier's rows, so a public chat at a cap learns nothing about private rows (DD-40, DD-41):

| Cap | Value | Beyond the cap |
|---|---|---|
| Active rows | 5,000 per tier | An agent or extractor term lands `suggested`; a person's add is refused with the cap text below |
| Pending suggestions | 200 per tier | The oldest `auto` row with `seen = 1` is evicted first; if none, the term is not recorded and the tool's count says how many were |
| Unreviewed agent rows | 200 per tier | Further agent terms land `suggested` |
| Tombstones | 5,000 per tier | The oldest tombstone is dropped |
| Agent terms per session | 30 | The 31st and later land `suggested` in every mode |
| Terms per tool call | 10 | A call with 11 or more is refused by the tool's schema |

A row with one spoken form, one misrecognition and a short note takes 538 B in the file, so a store near both active caps (7,800 such rows) is 4.2 MB [M, 2026-11-06].

### Validation and cap texts

These sentences are in the message catalog in [Surfaces](surfaces.md) under the ids below. Every Rust caller reads them from `ValidationCode::message` and the store's constants in `crates/biorouter-dictation/src/store/` (PR-13), and `components/dictionary/copy.ts` reads them from the catalog:

| Code | Id | Text |
|---|---|---|
| `too_long` | MSG-TERM-TOO-LONG | Use a term of at most 64 characters. |
| `too_many_words` | MSG-TERM-WORDS | Use a term of at most six words. |
| `empty`, `line_break`, `control_character`, `angle_bracket` | MSG-TERM-PLAIN | Use one line of plain text. |
| `too_many_forms` | MSG-TERM-FORMS | Use at most eight spoken forms and sixteen misrecognitions for a term. |
| `url`, `email`, `path` | MSG-TERM-ADDRESS | A dictionary term cannot be a web address, an email address or a file path. |
| `digits_only`, `long_hex`, `secret_shaped` | MSG-TERM-IDENTIFIER | This looks like an identifier or a key, so it was not added. |
| `cap` | MSG-DICTIONARY-CAP | Your dictionary holds 5,000 active terms here, the most dictation can use. Delete terms you no longer dictate, then add this one. |
| `stale` | MSG-DICTIONARY-STALE | This term changed since you opened it. Reload it and try again. |
| `duplicate` (an edit to a spelling another row of its tier has) | MSG-DICTIONARY-DUPLICATE | That spelling is already a term here, so the edit was not made. |
| `unreadable` (an import row) | MSG-IMPORT-ROW-UNREADABLE | This row could not be read. |

**Acceptance.** PR-13: the validation table above (control characters, newline, angle brackets, 65 characters, seven words, NFC, URL, email, path, digits only, long hex); rows per tier (a private add of an existing public term leaves the public row byte identical); the "same letters" warning; tombstones per tier; caps per tier; a usage bump leaves `rev` unchanged. PR-14: twins merge in `EligibleEntries` and never disable each other's key.

## The store

### Files and location

| OS or deployment | Folder |
|---|---|
| macOS and Linux | `~/.config/biorouter/dictionary/` |
| Windows | `%APPDATA%\UCSF\biorouter\config\dictionary\` |
| `BIOROUTER_PATH_ROOT` set | `$BIOROUTER_PATH_ROOT/config/dictionary/` |
| systemd service account (S8) | the service account's config folder, written with `sudo -u biorouter -H biorouter dictionary …` |
| Docker (S16) | the config volume at `/home/biorouter/.config/biorouter` |

The core resolves the folder with `Paths::in_config_dir("dictionary")` [C `crates/biorouter/src/config/paths.rs`] and passes it to the leaf crate, which takes every path as an argument. Since PR-13 that is `biorouter::security::dictionary_store::dictionary_dir()`, and `open_store()` there opens the store; the same module re-exports the leaf crate's `store`, so the CLI and the routes reach the store and its guard through one module. The folder holds `dictionary.json`, `usage.json` and `.lock`. `crates/biorouter/tests/path_resolver_agreement.rs` has a row for it (`dictionary_store_honours_the_sandbox_root`). The store holds no absolute paths, so the Windows legacy folder rewrite needs no row. Two machines, and a Windows desktop and its WSL account, have two dictionaries; export and import move terms between them ([Syncing two hosts](#syncing-two-hosts)).

### Writing and locking

Memory's pattern [C `crates/biorouter-mcp/src/memory/mod.rs` `StoreLock`, `lock_store`] (DD-33):

1. Take an exclusive `fs2` lock on `.lock` (`fs2` 0.4.3 is in `Cargo.lock` [C]).
2. Read and parse both files fresh under the lock.
3. Apply the change; bump `rev` and `updated_at` on every changed row.
4. Write `dictionary.json.tmp`, `fsync` it, rename it over `dictionary.json`, `fsync` the folder on Unix. On Windows a rename refused by an open handle is retried up to 10 times over 200 ms [I; size the retry against antivirus scanning once on a scanned profile].
5. Release the lock.

The folder is created 0700 and the files 0600 on Unix, because private rows may sit there.

A write that leaves the file as it was (an add of a term already there, an edit that changes nothing) writes nothing, so the file stays byte identical. The file holds one row per line, so a person can read it and repair it by hand.

Two budgets: the daemon's and the CLI's ordinary writes wait for the lock. The extractor and usage flushes use `try_write_within(50 ms)`, which retries `try_lock_exclusive` every 5 ms and drops the write when the budget ends.

Measured at 7,800 rows (4.2 MB) on this Mac's SSD in a release build [M, 2026-11-06]: an add takes 40 ms, most of it the two `fsync` calls, which on macOS flush the disk's cache; a flush of 30 usage bumps takes 19 ms; an import of 1,000 new rows takes 57 ms in one write. An import does not compute the "same letters" warning, which it never reports, so it stays linear in the store's size. None of these sits between a stop and the text: writes are the person's or the agent's acts, and usage flushes run after the text is out.

### Reading and caching

Readers parse without the lock. The daemon and an interactive CLI cache the parsed view keyed by the modification time, length and identity of both files and reparse on any change, so a term added in one process is listed by another at once. Identity is in the key because every write renames a new file into place: two writes within one clock tick that leave the length alone still reparse. It is the inode on Unix and the NTFS file index on Windows (`GetFileInformationByHandle` on the open file), never the creation time: NTFS tunnelling gives a file created or renamed onto a name removed within the last 15 s that file's creation time, so each write's `dictionary.json.tmp` would carry the previous one's. Each reparse increments an in process generation number. The generation is never exposed: no route reports a store wide revision (DD-40).

The view is what dictation reads when a recording starts: a cached view costs two `stat` calls, 2.1 µs, and a reparse at 7,800 rows takes 8.7 ms [M, 2026-11-06, release build], paid once after each change to the store, not per dictation. That holds for the dictionary routes, which share one cached handle (`routes::dictionary::shared_store`, the gate of PR-16 included); a `transcribe` is one process and parses once. As built at PR-16 it does not hold for the daemon's streams: `stream_dictionary` in `routes/dictation.rs` opens a fresh store for every stream, so every dictation parses `dictionary.json` again, and every view's new generation gives the bonus table a new revision. PR-16's live check measured 112 to 130 ms of `dictionary_ms` per stream at 4,993 rows with the store unchanged (debug build), against 0 ms at 3 rows, and stop to text on `small.en` rose from a median of about 120 ms to 181 to 198 ms at that size; how much of the rise is correction over 5,000 rows was not separated [M, 2026-11-06]. The dictionary is prepared beside the model load, so a small store pays nothing at the stop. Reading the shared handle there is a one line change that PR-16 left for later, and after it the writes that do not read the view back (delete, bulk delete, keep, accept, dismiss, import, observations) would leave the next stream one reparse at its creation.

### When the file cannot be read

A parse failure, an unknown `format`, a `version` other than 1, a key this version does not know, or two rows with one id fails closed (an unknown key fails rather than being dropped, because a CLI on the PATH often lags the app, and an older writer would otherwise erase a newer field on its next write):

- every dictation runs with an empty bias set; every listing, write and observation is refused with MSG-DICTIONARY-FILE: "The dictionary file at `<path>` could not be read, so nothing was changed. Fix or remove it, then try again." (the routes answer 503 `unreadable`, and the CLI, `list` included, exits 1);
- the file is never rewritten from a failed parse, so the person can repair it by hand;
- `biorouter doctor` and `biorouter dictation status --verbose` are to name the file; as of PR-16 neither does, and no pull request's file list includes it yet.

### Store API

PR-13 builds it in `crates/biorouter-dictation/src/store/`: `mod.rs` (the store, its cache and errors), `schema.rs`, `validate.rs`, `fold.rs`, `lock.rs`, `ops.rs` (what each write does to the file: adds with the tier, tombstone and cap rules, edits, deletes, accepts, and `remove_confirmed`, which deletes only the rows a person confirmed that still match), `tier_user.rs`, `transfer.rs` (import and export) and `usage.rs`. PR-16 adds `observe.rs` (`StoreFile::observe`, what one observation does, [Effects on the dictionary](#effects-on-the-dictionary)), `Entry::came_from_a_chat` in `schema.rs` (moved from the CLI so the routes and the Dictionary page share it), and `StoreView::detached`, a view of a few rows that no store read, which the observations route hands to `bias_set` to ask whether one private row's institutions admit a chat's model, so that rule keeps one spelling.

```rust
// crates/biorouter-dictation/src/store/mod.rs
pub struct DictionaryStore { dir: PathBuf, cache: Mutex<Option<(CacheKey, Arc<StoreView>)>> }

impl DictionaryStore {
    pub fn open(dir: impl Into<PathBuf>) -> Self;
    /// Cached by the modification time, length and identity of dictionary.json and usage.json.
    pub fn view(&self) -> Result<Arc<StoreView>, StoreError>;
    /// Exclusive lock, fresh parse, change, temp file, fsync, rename.
    pub fn write<T>(&self, f: impl FnOnce(&mut StoreFile) -> Result<T, StoreError>) -> Result<T, StoreError>;
    /// The extractor's and usage flushes' form: None when the budget ran out.
    pub fn try_write_within<T>(&self, budget: Duration, f: impl FnOnce(&mut StoreFile) -> Result<T, StoreError>)
        -> Result<Option<T>, StoreError>;
    /// Each delta names its scope: `All` bumps the `all` map, `Public` bumps both maps.
    pub fn flush_usage(&self, deltas: &[(EntryId, UsageScope, UsageDelta)], budget: Duration) -> Result<bool, StoreError>;
}

pub enum StoreError {
    Unreadable { path: PathBuf },        // MSG-DICTIONARY-FILE
    Invalid(ValidationCode),
    Stale { current_rev: u64 },          // If-Match mismatch
    NotFound,                             // also for a row the caller may not list
    Cap(CapKind),
    Duplicate,                            // an edit that would give two rows of one tier one written form
    Io(std::io::Error),
}
```

A usage file that cannot be read counts as empty for `view` (a lost usage figure costs ranking, never a term), and `flush_usage` refuses to write over it; an empty one, which only a power cut after an unflushed replace leaves, starts over. `flush_usage` replaces the file by a rename with no flush to disk (PR-14b's review), unlike every write of `dictionary.json`.

**Acceptance.** PR-13: a suggestion racing a delete loses nothing (two processes); a corrupt file refuses writes and stays byte identical; a term added in one terminal is listed by a second process at once.

## Privacy tiers and the bias set

### Rows per tier

A term seen in a private chat never changes a public row. The private occurrence lands in a private row, and the public row stays as it was (DD-35). A single row that ratcheted to private would vanish from public dictation when a private chat used the word, which is a silent loss for the person and an existence oracle for a public observer [I]. Publicize merges a private row into the public twin (union of spoken forms and misrecognitions, the public note kept, the private row removed) or moves it when no public twin exists. Privatize does the reverse. Both are the person's acts only ([Tier changes](#tier-changes)).

### Who stamps the tier

No caller sends a tier that the server trusts. Each writer's stamp:

| Writer | Stamp |
|---|---|
| `dictionary__add_terms` | The call's admitted capability, `McpMeta.capability` [C `crates/biorouter/src/agents/mcp_client.rs` `McpMeta`]. The tool has no `tier` argument; an unknown `tier` field is rejected by the schema |
| The chat extractor | `chat_reach` over the session's cached classification and its bound provider's tier; Private unless it answers Public |
| Observations (`POST /dictionary/observations`) | `chat_reach` over the named chat's stored classification (read after `session_reach`) and the live agent's bound provider; Private unless it answers Public, and Private when the request names no chat. An observation whose stamp is a tier the caller cannot list is a silent no op |
| A person on a keyed daemon (the desktop) | The person's choice; the default follows O-6 (public, private when the add starts from a private chat) |
| Any caller on a keyless daemon (`serve`) | The caller's standing, for an add and an edit alike. An edit reaches only a row the caller can list, so `max(entry.tier, standing)` is the standing. A `private: true` add or import from a caller without private standing is refused with MSG-DICTIONARY-HOST-PRIVATE, decided from the request before any row is read |
| A person at an interactive terminal | The person's choice (`--private`); in a `/dictionary add` inside a session, the session's tier by default |
| Import | Rows that carry `tier: "private"` import private; the rest take the importer's stamp above. On a keyless daemon such rows need private standing, as above |

`max` orders Private above Public. An edit whose stamp rises above the row's tier is written into the private twin (created from the public row if absent), and the public row stays unchanged; an edit never moves a row between tiers, because that would be a privatize, which a keyless daemon refuses (DD-38) and which would make the public row vanish (DD-35). On a keyless daemon no stamp is ever above the caller's standing, so a caller without private standing never creates a private row or moves a public one into the private tier and never learns what it holds (oracle rule 1, P-14). The one such caller that lists private rows, a served page on a host whose operator's provider is private (SD-10), may change those rows; P-7 records that residual.

Private rows also carry an institution stamp, from the same read united with the chat's record of the connectors it touched (O-35); [Private rows and institutions](#private-rows-and-institutions) gives it per writer.

One function answers "is this chat private?" for every dictation door, so the stream target, the observation stamp and the extractor stamp cannot disagree:

```rust
// crates/biorouter/src/dictation/target.rs
pub enum ChatReach { Private, Public, Unknown }

/// `stored` is None when the chat's row could not be read. `live_provider` is the tier of the
/// provider a live agent has bound, which can be Private before Gate B ratchets the row.
pub fn chat_reach(stored: Option<SessionClassification>, live_provider: Option<ProviderTier>) -> ChatReach {
    if stored == Some(SessionClassification::Private) || live_provider.is_some_and(|t| t.is_private()) {
        ChatReach::Private
    } else if stored.is_none() {
        ChatReach::Unknown
    } else {
        ChatReach::Public
    }
}
```

The two uses fail in opposite directions, each toward the safe side. A stamp treats `Unknown` as Private, because a stamp that fails toward private costs the person a publicize click. A stream target treats `Unknown` as public, because a target that failed toward private would let private terms shape text bound for a chat whose tier nobody knows. Without the live provider arm, a correction made in a chat bound to a private model before its first turn ratcheted the row would be stamped public, and the term would then bias public dictation [I].

Stamps follow the real tier of the model and the chat even when the privacy master switch is off; only the filters go inert then (DD-36).

A consequence on `serve`: on a host whose operator's provider is private, the served page states that provider in `X-Caller-Provider` with every call, so every browser caller has private standing and every term added from that browser is private. Public terms on such a host are added at the host's terminal with `biorouter dictionary add`. On a host whose operator's provider is public, browser callers have no private standing, so **Only in private chats** is disabled before the click with MSG-DICTIONARY-HOST-PRIVATE (`authority.add_private`), and private terms are added at the host's terminal with `biorouter dictionary add --private`.

### The bias set

`bias_set` in `crates/biorouter/src/dictation/bias.rs` (PR-14) is the only producer of the entries recognition may use. One value feeds the token prompt, the decoder bonus, the alias automaton, the exact key matcher and the fuzzy matcher (DD-36). The leaf crate defines the newtype; a repository walk test, `eligible_entries_are_built_only_by_bias_set`, pins every call of its constructor to `bias.rs`.

```rust
// crates/biorouter-dictation/src/recognize/mod.rs
pub struct EligibleEntries {
    entries: Arc<[RecognitionEntry]>, // twins merged, active misrecognitions only, derived flags set
    table: BonusTable,                // PublicRows | AllRows | PrivateRows(affiliation key): which decoder bonus table to use
}
impl EligibleEntries {
    /// Called only from crates/biorouter/src/dictation/bias.rs.
    #[doc(hidden)]
    pub fn new_filtered(entries: Vec<RecognitionEntry>, table: BonusTable) -> Self;
    pub fn empty() -> Self;
}
```

```rust
// crates/biorouter/src/dictation/bias.rs
/// Whether private entries may shape this caller's text: the user action proof, a private
/// capability stated in X-Caller-Provider, or a person at an interactive terminal. The field is private, so no
/// crate can write `Standing { private: true }`; `private_standing_is_built_in_two_files` pins
/// every call of `from_checked` to `HttpCaller::dictation_standing` in
/// crates/biorouter-server/src/routes/session_reach.rs and to the terminal checks in
/// `target::cli_bias`, crates/biorouter/src/dictation/target.rs (interactive terminal,
/// BIOROUTER_TERMINAL unset), where PR-25 left them: its crates/biorouter-cli/src/dictation/consent.rs
/// mints `CaptureConsent` and asks `cli_bias`, and builds no `Standing` itself.
pub struct Standing { private: bool }
impl Standing {
    pub const fn public() -> Self { Self { private: false } }
    #[doc(hidden)]
    pub const fn from_checked(private: bool) -> Self { Self { private } }
    pub const fn is_private(&self) -> bool { self.private }
}

/// The model a dictation is aimed at. `private` comes from chat_reach, new_chat_target_tier or the
/// CLI rule of DD-77; `affiliation` from the same provider read (None: unstated or unknown);
/// `person_included` is include_private_terms or --private-terms, admitted under O-8; it covers institution tagged rows too (O-37).
pub struct BiasTarget { pub private: bool, pub affiliation: Option<ModelAffiliation>, pub person_included: bool }

pub fn bias_set(view: &StoreView, target: &BiasTarget, standing: Standing, enforced: bool) -> EligibleEntries {
    // Private reach only for a private capable target AND a caller with private standing.
    let reach = if !enforced || (target.private && standing.is_private()) {
        ProviderTier::Private
    } else {
        ProviderTier::Public
    };
    let rows: Vec<RecognitionEntry> = view
        .entries()
        .filter(|e| e.status == EntryStatus::Active) // bias = never stays, for correction
        .filter(|e| privacy::visible_to(reach, classification_of(e.tier)))
        .filter(|e| target.person_included || admits(e, target, reach, enforced))
        .map(RecognitionEntry::from)
        .collect();
    let table = BonusTable::for_target(reach, target); // PublicRows, AllRows, or PrivateRows(affiliation key)
    EligibleEntries::new_filtered(merge_twins(rows), table)
}

/// DR-26 for one row (O-32, O-35, P-16): every institution the row names must admit the target's
/// model, each asked as a private extension of the row's tier whose allowlist is that one
/// institution. `null` asks nothing; `[]` asks once with an empty allowlist, which only a local
/// model clears. A public row never mismatches; the gate reads the mixing policy.
fn admits(e: &Entry, target: &BiasTarget, reach: ProviderTier, enforced: bool) -> bool {
    affiliation::dictionary_row_allowlists(e.institutions()).into_iter().all(|allowlist| {
        let class = ExtensionClassification { tier: provider_tier_of(e.tier), affiliation: allowlist };
        affiliation::gate_cross_affiliation_warning(enforced, reach, target.affiliation, "dictionary", &class)
            .is_none()
    })
}
```

The predicate is `status = active ∧ (¬enforced ∨ tier = public ∨ (target_private ∧ standing.private))`, compared through `privacy::visible_to` [C `crates/biorouter/src/privacy/mod.rs:417`]. A private row then also needs every institution it names to admit the target's model, each asked through `affiliation::gate_cross_affiliation_warning` [C `crates/biorouter/src/privacy/affiliation.rs`] ([Private rows and institutions](#private-rows-and-institutions)). The census in `crates/biorouter/tests/privacy_guard_wiring.rs` gains `bias.rs` as a `visible_to` site and a row for `bias_set` itself ([`HttpCaller` methods and the census](#httpcaller-methods-and-the-census)). It also gains a `Site` row in `crates/biorouter/tests/privacy_capability.rs` for the one `affiliation::gate_cross_affiliation` call in `bias.rs`, which sits inside the loop over a row's institutions, so the census, which counts occurrences in the source text, counts one [C census].

The Tier 1 accuracy test builds its dictionary through `bias_set` like every other caller, so it lives in the core crate as `crates/biorouter/tests/dictation_accuracy.rs`. A test in the leaf crate's own `tests/` folder could reach `EligibleEntries` only through `new_filtered`, which the repository walk test forbids.

As built in PR-14, four details differ from the sketches above, each the smallest change the code needed:

- `new_filtered(entries, table, scope, generation)` also takes the `bias_scope` the set reports and the store's in process generation, because `AllRows` alone cannot say whether the switch is off (scope public) or the person included private terms (scope private). The bonus table's `revision` is a hash of the generation, the table and each row's written form and bonus policy, so every generation rebuilds every table (oracle rule 9) and a mixing policy that changed while the store did not still gets a fresh one.
- Rows with `bias = never` stay in the set, and the sketch and the predicate above now say so. The field table under [Field effects per channel](#field-effects-per-channel) keeps them out of the prompt and the bonus and lets correction use them, which is the more specific statement; DD-36's predicate in the [plan](implementation-plan.md), which also names `bias ≠ never`, would have made `never` mean "never correct to it" too.
- Every consumer of the set takes the `EligibleEntries` value itself, never a slice of rows: `select::plan`, `Corrector::new` and `EligibleEntries::bias_table`. The slice forms are private to the leaf crate, and `RecognitionEntry::from_entry`, which turns a raw store row into an entry, is pinned to `bias.rs` by the same repository walk as `new_filtered`.
- A target's reach is private for `person_included` as well as for a private target, each only with private standing, so a Crew or external stream with an admitted `include_private_terms` gets the private rows the bias table test expects.
- The terminal half of `Standing::from_checked` is `target::cli_bias`, which holds DD-77's rule in the core crate. The design first placed it in `crates/biorouter-cli/src/dictation/consent.rs`, but that file, which PR-25 added, mints `CaptureConsent` and asks `cli_bias` rather than building a `Standing` itself; `private_standing_is_built_in_two_files` (PR-16) names `target.rs`. PR-25, which added the terminal's consent prompt, left both: on 2026-11-10 the production calls are still `HttpCaller::dictation_standing` in `routes/session_reach.rs` and `target::cli_bias` in `target.rs`, the two the test names.

The transcript's `bias_scope` is `private` only when the switch is on, the target is private capable and the caller has private standing; it is `public` for every other caller, including any caller without private standing (DD-29). With the switch off the set holds every row and `bias_scope` reports `public`, so the private draft flag stays silent.

Who decides `target_private` (DD-30, never the client):

| Request | `target_private` |
|---|---|
| `POST /dictation/streams` with `session_id` | `session_reach` first; then true only when `chat_reach` answers Private over `target_tier` [C `crates/biorouter-server/src/routes/session_reach.rs` `target_tier`] and the live agent's bound provider (`AppState::peek_agent`). `Unknown` counts as false here. A refusal silently degrades to false with a byte identical response |
| `target_kind: "new_chat"` (Home, a new tab) | the tier the new chat will start at (`new_chat_target_tier`): the tier its bind would produce (`new_chat_bind_tier`: the provider built from the capability configuration keys, its instance `tier()`, a lead and worker pair reduced with `ProviderTier::least`, Public on any failure), never a declared tier, with the decision models its draft names folded in (`with_draft_decisions`, DR-33). The per tab pick (`new_chat_provider`) and the draft's models (`new_chat_decision_models`) count fully only from a proven caller. Without the proof a pick can only lower the target (the less private of the configured default and the pick counts, because `/agent/start` binds a public pick without the proof), and a draft makes it public |
| `target_kind: "crew"` or `"external"` | false, unless the body carries `include_private_terms: true` from a caller with the proof and private standing, which the renderer sends only after the person confirms MSG-PRIVATE-TERMS for that recording (O-8) |
| TUI and REPL | the session's tier, in process |
| Shared daemon prompt, `session attach` | the chat's stored tier, read offline, and only at an interactive terminal |
| `transcribe` or `dictate` with `--session <id>` | DD-77: the chat is private AND `prompt_can_run()` AND `caller_capability()` is Private AND `BIOROUTER_TERMINAL` is unset |
| `dictate` to stdout or `dictate --copy`, no `--session` | DD-77: false unless the person passes `--private-terms`, admitted only when `prompt_can_run()` is true, `BIOROUTER_TERMINAL` is unset and `caller_capability()` is Private, after MSG-PRIVATE-TERMS and a `y` (O-8) |

**A new chat's decision models (DR-33).** A chat is as private as its least private model, and its decision models count: Versa with a monitor on `joey-1`, which runs on Baseten, is a public chat. A chat that does not exist yet has no slot to read, so the composer sends every decision model its draft names, the draft `/agent/start` will carry as `decision_settings`: the draft's default and each monitor's model, a monitor that is off included. The daemon asks of each one the check the start runs on those settings (`decision::save::tier_change`), on an empty Public chat bound to the model the bind would produce, as `GET /decisions/models` predicts a draft's effect. When any one would make the chat public, the target is public and no private term shapes the dictation. Doubt answers public here, because a target that failed toward private would let private terms shape text bound for a chat whose tier nobody knows: a draft from a caller without the proof, one that does not parse, one that names more models than a draft can hold, a model whose provider declares nothing, and a refused check each make the target public. The field can make the target more public and never more private. Once the chat exists, its stream reads the live slot, the decision-bound provider whose tier already folds the same names, so the chat stays public after its first message and at every reasoning effort, since an effort changes the provider one turn's requests are built on and never the slot.

The target's model affiliation comes from the same read as `target_private`: the live agent's bound provider for `session_id` (unstated when no agent is live), the built instance for `new_chat`, the session's provider in process for the TUI and REPL, and unstated for the shared daemon prompt, `session attach` and `--session`. `crew`, `external` and an admitted `--private-terms` take every private row, institution tagged rows included (O-8, O-37).

Public by default, with private terms only for one recording the person confirms after MSG-PRIVATE-TERMS, and never for an agent (O-8): Crew's composers (O-21), **Dictate to clipboard** and app preview inserts (`external`), and `dictate` to stdout or `--copy`. Public only, whatever the caller: `transcribe` without an admitted `--session`, and ACP, because no proof of a person reaches their transcription.

Why one filtered set and not "every term, because audio stays on the machine": the text lands in a composer whose content goes to that chat's model. Without voice detection, 4 of the 64 nonspeech decodes emitted dictionary terms outright (`PHD2, PHD2, PHD2`) [M]. With a text prompt and the decoder bonus, time reversed speech produced real symbols (`PHD2A, BRCA1`, voice detection off), and with the detector on the reversed speech still passed it and one dictionary term came through [M]. M-22 then measured the token prompt with the bonus, with the detector on: no arm wrote text on the nonspeech clips beyond turbo's one `Mm-hmm.` on a PriMock57 gap, which the no dictionary arm writes too, but both channels together put dictionary terms into probe and reversed clips (turbo 5, `UCSF` for UCSD three times among them; `small.en` 1), where the token prompt alone put 0 and 1 [M, 2026-11-06]. A correction writes a term as surely as an echo does [I].

### Private rows and institutions

Private rows carry an institution affiliation, decided by the owner on 2026-11-05 (O-32, DD-35, DD-36). In the owner's follow up answers of the same day, a row's institutions are those of the chat it was written in: the institutions behind the chat's model and those whose connectors the chat touched (O-35), for a term the person adds inside a private chat too (O-36). Tier asks how sensitive a term is; affiliation asks whose data it may hold. HIPAA compliance does not transfer between institutions, so a term from a chat that held one institution's data must not bias dictation aimed at another institution's model, even when both models are private (DR-26, [institutional affiliation](../security/institutional-affiliation.md)). The design reuses the comparison Biorouter already applies to extensions, asked once per institution, rather than adding a second one.

**What a row records.** A private row carries `affiliation`, one of three values. Public rows never carry one.

| Value | Meaning | Targets it may shape |
|---|---|---|
| `null` | no institution claims the row | every private target |
| `{"institutions": [id, …]}`, 1 to 8 normalized ids (`ucsf`) | the institutions whose data the row may hold | one id: models of that institution alone, and local models; two or more: local models only |
| `{"institutions": []}` | unknown: the writer could not read its chat's record or model, an imported value could not be read, or the union named more than 8 ids | local models only |

The ids are the union of two sets, read at the write:

1. **The chat's model.** The writer's `ModelAffiliation` [C `crates/biorouter/src/privacy/affiliation.rs` `ModelAffiliation`]: `Institutions(S)` contributes the ids of `S`, and a lead and worker pair already folds both halves [C `crates/biorouter/src/providers/lead_worker.rs` `affiliation`, `providers::composite_affiliation`]. `Local` contributes none, because the term never left the machine, and a private model that states no affiliation contributes none, the rule a knowledge base follows for the same two callers [C `crates/biorouter-mcp/src/knowledge/affiliation.rs` `contributed_owners`]. A model covered by two institutions contributes both: a knowledge base records neither only because its wire value names one institution, and the code says the precise answer would be to add both [C `crates/biorouter/src/privacy/affiliation.rs` `caller_affiliation`]; the dictionary lives in the core crate and has no such wire.
2. **The chat's connectors.** `SessionManager::session_affiliations(session_id)`, the `sessions.session_affiliations` column: the institutions whose extensions this chat has reached [C `crates/biorouter/src/session/session_manager.rs` `session_affiliations`]. Dispatch records every institution of a private extension's `Institutions` allowlist when it admits a call, beside the tier ratchet and only while the privacy master switch is on [C `crates/biorouter/src/agents/extension_manager.rs` `dispatch_tool_call`, `ratchet_for_private_extension`; `session_manager.rs` `record_session_affiliation`]. Crew admission records its institutions too, and a copied or diverged chat inherits the source chat's [C `crates/biorouter-server/src/routes/crew.rs` `record_admission_affiliation`; `session_manager.rs` `create_derived_session`].

A new helper in `crates/biorouter/src/privacy/affiliation.rs` (PR-13) forms the union and applies the rules for unknown values, because that file and its `biorouter-mcp` mirror are the only places allowed to hold affiliation logic [C `crates/biorouter/tests/privacy_capability.rs` `compatible_is_the_only_function_that_compares_two_affiliations`] [I: new]:

```rust
// crates/biorouter/src/privacy/affiliation.rs (PR-13, O-35)
pub enum WriterModel { Known(Option<ModelAffiliation>), Unknown } // Unknown: no live agent, no provider in process
pub enum RowInstitutions { Unclaimed, Unknown, Owners(BTreeSet<InstitutionId>) } // null, [], [ids]

pub fn dictionary_row_institutions(
    model: WriterModel,
    touched: anyhow::Result<BTreeSet<InstitutionId>>, // SessionManager::session_affiliations
) -> RowInstitutions {
    let (WriterModel::Known(model), Ok(mut owners)) = (model, touched) else {
        return RowInstitutions::Unknown; // unknown fails toward the stricter value
    };
    if let Some(ModelAffiliation::Institutions(set)) = model {
        owners.extend(set.iter());
    }
    match owners.len() {
        0 => RowInstitutions::Unclaimed,
        1..=8 => RowInstitutions::Owners(owners),
        _ => RowInstitutions::Unknown, // reaches the same targets as any set of two or more
    }
}
```

**Unknown fails toward the stricter value.** `session_affiliations` answers `Err` when the column cannot be read or parsed, and its documentation says that is not "no institutions": chat recall refuses the recall and conversation ingest refuses that chat on `Err`, the direction `KbAffiliation::Unknown` takes for a knowledge base [C `session_manager.rs` `session_affiliations`; `crates/biorouter/src/agents/chatrecall_extension.rs`; `crates/biorouter/src/knowledge/conversation_ingest.rs`; `crates/biorouter-mcp/src/knowledge/affiliation.rs` `KbAffiliation::Unknown`]. So a write whose record answers `Err`, or whose chat's model cannot be read because no agent is live, stores `{"institutions": []}`: the value an unreadable import already stores, which only local models clear, never `null`, which would reach every private target. A chat id with no row reads as an empty set, not an error [C `session_manager.rs` `SessionStorage::session_affiliations`].

**Who stamps it.** The server, or the CLI in process at a terminal, in the same read that stamps the tier, never a client:

| Writer | Affiliation of a private row |
|---|---|
| `dictionary__add_terms` | `McpMeta.capability.affiliation()`, the call's admitted `CallCapability` sampled once with its tier [C `crates/biorouter/src/agents/mcp_client.rs` `McpMeta`, `crates/biorouter/src/privacy/capability.rs` `CallCapability::sample`], united with `session_affiliations(McpMeta.session_id)`, read through `PlatformExtensionContext.session_manager` [C `crates/biorouter/src/agents/extension.rs` `PlatformExtensionContext`]; `[]` when that read answers `Err` |
| The chat extractor | the bound provider's `affiliation()`, read with its tier, united with the session's `session_affiliations`; `[]` on `Err` |
| Observations | the live agent's bound provider's `affiliation()` (`AppState::peek_agent`), read with its tier, united with the named chat's `session_affiliations`, read after `session_reach`; `[]` when no agent is live, the read answers `Err`, or the request names no chat (whose tier stamp is already Private). An observation with no chat has no record or model to read, so it takes the unknown value, unlike a person's add with no chat, which is the person's own act (O-36). The TUI and the classic REPL stamp their own corrections the same way, from the session in process |
| A term the person adds inside a private chat (O-36): **Add to Biorouter dictionary…** from the context menu, whose `POST /dictionary/entries` names the chat the renderer resolved (`origin_session_id`), and `/dictionary add` in the TUI and the classic REPL | that chat's institutions by the same rule: on the daemon the live agent's bound provider and the chat's `session_affiliations`, read after `session_reach`; in the TUI and REPL the session's provider and `session_affiliations`, in process. `[]` when no agent is live, the read answers `Err`, `session_reach` refuses, or the renderer could not resolve the chat (a shared transcript, sent as `origin_unresolved: true`) |
| The tool row's **Add to dictionary** and **Keep** | unchanged: they accept or keep a row that `dictionary__add_terms` already stamped from that chat |
| A person's add with no chat: the Dictionary page, `biorouter dictionary add` at a terminal | `null` (O-36) |
| `privatize`, or an edit on the page that creates a private twin | `null`, the person's act with no chat; merging into an existing private twin keeps the twin's value |
| `publicize` | removed, because public rows carry none |
| Import | the file's `affiliation` on a row marked `"tier": "private"` when it is `null` or `{"institutions": [...]}` with 1 to 8 ids of 1 to 64 characters and no control character; `null` for a row that carries none (a text or CSV line, a JSON row without the key); `{"institutions": []}` for any other value, because a value that is present but cannot be read is unknown, and `null` would reach every private target |

The desktop composer has no `/dictionary add` in this plan; a chat's door there is the context menu. A later pull request that adds one sends `origin_session_id` the same way. The extractor's worker reads the chat's record once per message, before it looks for a candidate: the session row, for the chat's kind and stored tier, and `session_affiliations` when the stamp is private. Both reads come before the store is read or locked, so neither counts against the 50 ms lock budget, and a message with no candidate then returns without reading the store [C `crates/biorouter/src/agents/agent.rs` `queue_dictionary_observation`; `crates/biorouter/src/dictation/extractor.rs` `observe`].

The value is set when the row is created. A later write that finds the row (a second sighting, an agent's add of the same term, a correction) leaves it as it is, so a row's institutions change only when the person publicizes it. Stamps follow the real model and the chat's real record even with the privacy master switch off or the mixing policy `open`, so turning either back restores the filter with correct rows; dispatch adds nothing to the record while the switch is off [C `session_manager.rs` `record_session_affiliation`].

**How the bias set uses it.** `bias_set` asks one question per institution a private row names, through `affiliation::gate_cross_affiliation_warning` [C `crates/biorouter/src/privacy/affiliation.rs`], the function `POST /agent/add_extension` and the subagent spawn already use to decide whether a model may reach a private extension. For each institution the row stands in for a private extension whose allowlist is that one institution, and the row is eligible only when every answer is `None`; `null` asks nothing, and `[]` asks once with an empty allowlist, which only a local model clears. That is the union of owners rule Biorouter applies wherever a chat's touched institutions leave the chat (chat recall's load and search, conversation ingest, Crew admission: a chat that reached two institutions' connectors is reachable from neither institution's model) [C `affiliation::owners_compatible`; `crates/biorouter/src/session/chat_history_search.rs` `affiliation_clause`; `crates/biorouter/src/crew/institution.rs` `check_provider`], asked one owner at a time through the extension gate so the gate's own lifts apply. For a row that names one institution it is exactly the extension rule O-32 chose. Reading the union as one allowlist would widen a row each time its chat touched another institution; `affiliation_clause` rejects that reading for the same column ("`EXISTS (… = ?)` would be the allowlist rule and would return a chat that also reached somebody else") [C]. A second helper in `privacy/affiliation.rs`, `dictionary_row_allowlists` (PR-14), turns stored ids into the allowlists to ask [I: new]. The answers follow DR-26's table [C `affiliation::compatible`, `gate_cross_affiliation`, `unstated_model`]:

| Row's institutions | Target `Local` | Target `{ucsf}` | Target `{stanford}` | Target `{stanford, ucsf}` | Target unstated |
|---|---|---|---|---|---|
| `null` | eligible | eligible | eligible | eligible | eligible |
| `{ucsf}` | eligible | eligible | left out | left out | left out |
| `{stanford, ucsf}` | eligible | left out | left out | left out | left out |
| `[]` (unknown) | eligible | left out | left out | left out | left out |

A row that names two institutions is eligible only for a local model, because no institutional model is admitted by each of two one institution allowlists. A model covered by two institutions reaches only rows with no institution, because the other half of the pair is in its pipeline [C `affiliation::compatible`].

The conditions that lift the extension check lift this one: the privacy master switch off (`enforced` false); the mixing policy `open`, under which institution mixing is allowed silently and `refusing_mismatch` answers `None` [C `crates/biorouter/src/privacy/mixing.rs` `mismatch_refuses`]; and a public row, which the gate never refuses. One condition is the person's: when the person includes private terms for one recording after MSG-PRIVATE-TERMS (O-8), every private row is eligible, institution tagged rows included, decided by the owner on 2026-11-05 (O-37), because the destination is the person's choice and MSG-PRIVATE-TERMS says Biorouter cannot keep the terms from other institutions' models. No grant is asked for: a cross affiliation grant covers a chat, an extension and a model [C `crates/biorouter/src/privacy/grant.rs`], and dictation makes no extension call to grant, so a row the gate leaves out stays out of that dictation.

**The target's model.** `target.rs` reads the target's affiliation from the same provider read that gives `chat_reach` its live tier: the live agent's bound provider for `session_id`, unstated when no agent is live; the instance `new_chat_bind_tier` builds for `new_chat`, whose `affiliation()` it returns beside the tier; the session's provider in process for the TUI and REPL; and unstated for the shared daemon prompt, `session attach` and `--session`, which read a chat's stored tier offline. An unstated target gets the private rows with no institution and none of the others. The owner kept this on 2026-11-10 (O-43): a dictation into a chat whose agent is not loaded yet leaves out private rows that name an institution. The target is its model alone, as for extensions. A chat's touched institutions describe what that chat holds, and every reader of `session_affiliations` in the code treats them as the owners of content leaving the chat; none adds them to the model side of a check, and Gate C's dispatch denial asks only the admitted `CallCapability` [C `crates/biorouter/src/privacy/capability.rs` `CallCapability::cross_affiliation_warning`; `chat_history_search.rs` `affiliation_clause`; `crew/institution.rs` `check_provider`]. So the target chat's own connectors do not narrow what dictation into it may use: the dictated text goes to the target's model, and that model is what a row's institutions must admit.

**What the agent sees.** `dictionary__list_terms` leaves out a private row unless every institution it names admits the caller's capability, each asked through `McpMeta.capability.cross_affiliation_warning("dictionary", &class)` [C `CallCapability::cross_affiliation_warning`], and reports no count, so an agent on one institution's model never reads a term that holds another institution's data. An observation from a chat whose model a private row's institutions do not all admit leaves that row unchanged, silently.

**No oracle.** A row left out changes no field of a dictation response: `bias_scope` is `private` whenever private standing reaches a private target, and no response reports a count (DD-40). Affiliation never changes what a public caller sees, because a public caller sees no private row. A stamp reads a chat's record only after `session_reach` admits the caller to that chat, and `EntryView.affiliation` reaches only callers with private standing. Oracle rule 9 holds per scope: the public table, the table of every row and one private table per target affiliation in use are rebuilt at every dictionary revision, with the filtered set for each.

**On the page.** Line two of a private row starts with where the row is used: MSG-ROW-PRIVATE for `null`; MSG-ROW-PRIVATE-INSTITUTION for a row that names institutions, in its one institution form with the institution's published name (`institution_display_name`, else the id [C `crates/biorouter/src/privacy/affiliation.rs`]) or in its two or more form, which names them joined with "and" and says local models only; and MSG-ROW-PRIVATE-LOCAL for `[]`. `EntryView` carries `affiliation` on private rows: `null`, `{"institutions": []}`, or `{"institutions": [{"id": "ucsf", "name": "UCSF"}]}`, with `name` null when the registry publishes none. Only a caller with private standing sees a private row at all.

**Changing it.** No control edits a row's affiliation, and the owner kept it so (O-36). `publicize` removes it after MSG-PUBLICIZE-WARN, because a public term may shape any chat. A person who wants a term in every private chat deletes the row and adds the term again on the Dictionary page or with `biorouter dictionary add`, which carry no institution; adding it again inside a private chat gives it that chat's institutions.

**Export and import.** Export writes `affiliation` on each private row it carries (private rows only with private standing and MSG-EXPORT-PRIVATE, DD-43). Import keeps a well formed value on a row marked private, stores `null` for a row that carries none, and stores `{"institutions": []}` for any other value, the stricter result for a value that is present but cannot be read, which only local models clear until the person deletes the row and adds the term again.

**What it does not cover.** Private rows carry the union of the institutions behind the chat's model and those whose connectors the chat touched (O-35), which closes the earlier gap where a chat on a local model that read one institution's connector wrote rows with no institution. What remains follows from how Biorouter records a chat's connectors [C `crates/biorouter/src/session/session_manager.rs` `record_session_affiliation`]: only a private extension whose classification names institutions is recorded, and only when dispatch admits a call to it, so content that entered a chat through `kb_search` on an institution's knowledge base or a `chatrecall` load of another institution's chat adds no institution; dispatch records nothing while the privacy master switch is off; and a row keeps the institutions it was created with, so a later sighting, or a correction from another chat whose model the row admits, changes the row and never its institutions. A term the person adds with no chat carries no institution, by the owner's decision (O-36).

**Acceptance.** PR-13: the schema field, import and export (a row without an affiliation imports as `null`, a malformed one as `[]`), `publicize` and `privatize` as above, and the stamp table of `dictionary_row_institutions` (P-16). PR-14: the table above in `bias.rs`, with the master switch, the three mixing policies and `include_private_terms`; `dictionary_row_allowlists`; the `Site` row in `crates/biorouter/tests/privacy_capability.rs`; `compatible_is_the_only_function_that_compares_two_affiliations` green. PR-15: a stream into a chat whose live model is covered by another institution leaves the row out with the same response fields. PR-16: observations and adds with `origin_session_id` stamp the union; a record holding `not-json` (written the way `malformed_source_affiliation_fails_before_copying_a_session` writes one [C `crates/biorouter/src/session/session_manager_required_affiliation_tests.rs`]) or a chat with no live agent gives `[]`; `origin_unresolved: true` gives `[]`; an add with no origin gives `null`; an observation from a chat whose model the row does not admit leaves it unchanged. PR-20: the context menu add sends the origin. PR-23, PR-24: each stamp unites the model with the session's record, so a chat on a local model whose record holds `ucsf` writes `{ucsf}`; `list_terms` from a `{stanford}` chat omits a `{ucsf}` row. PR-26: a private `/dictionary add` in a session takes the session's institutions (P-16).

### The master switch

With privacy tiers switched off (DR-15), `enforced` is false: every active row is eligible, every caller lists every row, and the draft flag stays silent. Stamps still follow the real tier, so turning the switch back on restores the filters with correct rows.

### Existence oracle rules

Each rule has a byte identical test in `crates/biorouter-server/tests/dictionary_oracles.rs` (PR-16) (DD-40, P-6):

1. A public caller's add consults public rows and public tombstones only. A `private: true` add or import from a caller without private standing is refused with one constant sentence before any row is read, so the answer is byte identical whether the term exists as private or not.
2. Ids are random, and naming an id the caller may not list answers exactly as a missing one: 404 with MSG-NOT-FOUND, "No dictionary entry with that id.".
3. Import dry runs and the "same letters" warning count rows the caller can list, and nothing else.
4. No store wide revision is exposed. Edits use the per entry `rev`. The listing's `ETag` hashes the caller's filtered view, so a private write never changes what a public caller polls.
5. `uses`, `last_used_at`, `corrected_to` and `suggested_in` are returned only to a caller with private standing, because usage on a public row moves when private chats dictate.
6. Dictation uses private terms only with standing and a private target, and the response shape is identical either way.
7. Refusals name no term.
8. Caps count only the caller's tier.
9. Decoder bonus tables for every scope in use (public rows, all rows, and one private table per target affiliation in use, [Private rows and institutions](#private-rows-and-institutions)) are rebuilt at every dictionary revision, before the next request, so a request's latency never depends on which tier changed.
10. No response reports a count of eligible or withheld entries.
11. Ranking for a public scope request reads only the `public` usage map ([Usage](#usage)), so its prompt and its fuzzy set are identical whatever private scope dictations and private corrections have happened.

### The private draft flag

Every transcript reports `bias_scope`. The renderer stores it on the dictated span (`DictatedSpan.biasScope`, [Desktop and `serve` ledger](#desktop-and-serve-ledger)). When a draft holds text dictated with `private` scope and the person sends it to a chat bound to a public model, the composer asks MSG-PRIVATE-DRAFT: "This message was dictated with your private dictionary terms turned on, so it may contain them, and `<model>` is a public model. Send it anyway?" with **Send anyway** and **Cancel**. "A public model" is judged by the daemon's instance tier for that chat, never by a declared tier. Switching that chat to a public model asks MSG-PRIVATE-DRAFT-SWITCH first. Either proceeds on the person's say so (DD-71). No agent path ever sees or answers the flag. The composer behaviour is specified in [Surfaces](surfaces.md).

### What is and is not claimed

The tier rules stop a cooperative agent or a person's slip from putting private terms into a public chat's dictation. They do not stop a determined actor with a shell, who can read `dictionary.json` directly (DR-17 left the filesystem open), unset `BIOROUTER_TERMINAL`, or relocate the managed policy file with `BIOROUTER_PATH_ROOT`. On a keyless daemon, any holder of the daemon secret, including an agent shell that recovers it, can install a pinned model and make the dictionary writes a `serve` browser may make (add, accept, dismiss, edit, delete and import on rows it can list; on a host whose operator's provider is private, that includes private rows). An agent holding a Biorouter Copilot grant on Biorouter's own window, or on a browser that shows a `serve` tab, can start the microphone by clicking it or pressing the dictation chord, because Copilot's per task grant is the only gate there (DD-88, the owner's decision O-15 (2026-11-05), SD-17). Every control in this document is accident prevention for cooperative actors. Privacy tiers here, as everywhere in Biorouter, are a safety boundary against mistakes by cooperative actors, not a security boundary against a determined adversary.

Private rows carry an institution affiliation, decided by the owner on 2026-11-05 (O-32, O-35, O-36, [Private rows and institutions](#private-rows-and-institutions)). Private rows carry the union of the institutions behind the chat's model and those whose connectors the chat touched (O-35), which closes the earlier gap where a chat on a local model that read one institution's connector wrote rows with no institution. What remains follows from how Biorouter records a chat's connectors [C `crates/biorouter/src/session/session_manager.rs` `record_session_affiliation`]: only a private extension whose classification names institutions is recorded, and only when dispatch admits a call to it, so content that entered a chat through `kb_search` on an institution's knowledge base or a `chatrecall` load of another institution's chat adds no institution; dispatch records nothing while the privacy master switch is off; and a row keeps the institutions it was created with, so a later sighting, or a correction from another chat whose model the row admits, changes the row and never its institutions. A term the person adds with no chat carries no institution, by the owner's decision (O-36). The mixing policy `open` turns the filter off, as it turns off the extension check.

## Authority on each surface

### Who may do what

Private standing means the user action proof, a private capability the request states in `X-Caller-Provider` (which a `serve` page on a private host sends with every call, SD-12), or a person at an interactive terminal. A `serve` daemon's operator tier (`served_operator`), which SD-10 lets listings read, lists private rows to the served page but is never standing by itself: it never biases dictation or stamps a write (DD-30).

| Surface or caller | Read public rows | Read private rows | Write | Tier change | Private bias |
|---|---|---|---|---|---|
| Desktop app on its own keyed daemon (S1 to S4) | yes | yes; every call sends `userActionHeaders()` | with the proof | with the proof, after MSG-PUBLICIZE-WARN before publicize | private target and the proof |
| `serve` browser, keyless daemon (S5, S6, S8, S16) | yes | with private standing only | admitted for rows the caller can list (O-4); stamped with the caller's standing; a private add without private standing refused with MSG-DICTIONARY-HOST-PRIVATE | refused; disabled before the click with MSG-DICTIONARY-HOST-TIER | private target and private standing |
| External backend, with that server's User Action Key (S17) | yes | yes | as the desktop | as the desktop | as the desktop |
| External backend, keyed server, no key (S17, O-17) | yes | no | refused; disabled before the click with MSG-REMOTE-NO-KEY | refused | never |
| External backend, keyless server (S17) | as `serve` | as `serve` | as `serve` | as `serve` | as `serve` |
| TUI and classic REPL (S9, S19) | yes | yes | yes, in process (`/dictionary add`, `/dictionary review`) | not from the session; `biorouter dictionary publicize` at a terminal | when the session is private capable |
| `biorouter dictionary` at an interactive terminal | yes | yes (DD-37) | yes | after a typed confirmation (O-5) | n/a |
| Noninteractive CLI (pipe, cron, `run`, `session send`) | yes | no | refused (exit 2), except `import` ([Terminal only rules](#terminal-only-rules)) | refused | public only |
| An agent's shell (`BIOROUTER_TERMINAL=1`) | yes | no | refused with MSG-DICTIONARY-AGENT (exit 2) | refused | public only, whatever the flags (DD-77) |
| The agent's tools (`dictionary__*`) | its tier scope | only for a private capability caller | `add_terms` per `agent_writes`; `remove_added_terms` on its own unreviewed rows | never | n/a: an agent never transcribes |
| ACP stdio (S12) | through the tools | as the tools | suggestions only, in every mode | never | public only |
| ACP `--ws` | no capability | no | no | no | public only |
| Crew (S14) | the member's own sidebar | the member's own sidebar | the member's own sidebar; Crew scoped chats cannot call `dictionary__*` | the member's own sidebar | public by default; private terms for one recording after MSG-PRIVATE-TERMS (O-8, O-21) |
| Agent Drafter apps (S15) | no | no | no; manifests refuse the capability | no | public by default; the desktop preview's `external` target takes private terms for one recording after MSG-PRIVATE-TERMS (O-8) |
| **Dictate to clipboard** (S18) | n/a | n/a | n/a | n/a | public by default; private terms for one recording after MSG-PRIVATE-TERMS, never for an agent (O-8) |

### Routes

`crates/biorouter-server/src/routes/dictionary.rs` (PR-16), merged behind `check_token`, utoipa annotated, then `just generate-openapi && cd ui/desktop && npm run generate-api`. Every `{id}` route sits behind `gate_dictionary_entry`, a `route_layer`.

| Route | Request | Answer | Keyed daemon | Keyless daemon (`serve`) |
|---|---|---|---|---|
| `GET /dictionary/entries?status=&q=&cursor=` | `status` `active`, `suggested` or `all` (default); `q` a case folded substring of written, spoken forms and note; `cursor` opaque | 200 `{entries: [EntryView], next_cursor}`, at most 500 rows a page, sorted by `updated_at` descending then `id`; `ETag`; 304 on `If-None-Match` | listing by standing | listing by standing |
| `POST /dictionary/entries` | `{written, spoken_forms?, kind?, note?, private?, match?, bias?, context_cues?, origin_session_id?, origin_unresolved?}`; the two origin fields name the chat the add started in, are used only to stamp a private row's institutions, and a named chat passes `session_reach` first (O-36) | 201 `{entry, warning?}`; 200 `{entry, created: false}` when the row already exists in the stamp tier | proof | admitted; stamped with the caller's standing; `private: true` without private standing refused with MSG-DICTIONARY-HOST-PRIVATE before any row is read |
| `PATCH /dictionary/entries/{id}` with `If-Match: <rev>` | any of `{written, spoken_forms, kind, note, match, bias, context_cues}` | 200 `{entry}`; 412 `{code: "stale", rev}` | proof | admitted if listable; stamped with the caller's standing |
| `DELETE /dictionary/entries/{id}` | none | 204; a tombstone per (`fold_key`, `tier`) | proof | admitted if listable |
| `POST /dictionary/entries/{id}/undo-agent-add` | `{expected_rev}` | 204 only while the listed row is still an active, unreviewed agent row at that revision; otherwise 412 `stale` or 404 for a missing or hidden row | proof | the tool row’s Undo; checked inside the store write lock, never a fallback to generic DELETE |
| `POST /dictionary/entries/delete` | `{ids}` | 200 `{deleted}` over listed ids; other ids are skipped as missing | proof | admitted if listable |
| `POST /dictionary/entries/keep` | `{ids}` | 200 `{kept}`; sets `reviewed: true` on agent rows | proof | admitted if listable |
| `POST /dictionary/entries/{id}/tier` | `{tier}` | 200 `{entry}` (the merged or moved row) | proof, checked before the id is read | refused with MSG-DICTIONARY-HOST-TIER |
| `POST /dictionary/suggestions/accept` | `{ids}` | 200 `{accepted}`; `status: active`, `reviewed: true` | proof | admitted if listable |
| `POST /dictionary/suggestions/dismiss` | `{ids}` | 200 `{dismissed}`; rows deleted, tombstones written | proof | admitted if listable |
| `POST /dictionary/import` | `{format: "lines" \| "csv" \| "tsv" \| "json", content, dry_run, private?}`, at most 1 MiB and 5,000 rows | 200 `{new, merged, duplicate, invalid: [{line, code}]}` | proof | admitted; counts over listable rows; `private: true`, or a row marked `"tier": "private"`, without private standing refused with MSG-DICTIONARY-HOST-PRIVATE before any row is read |
| `GET /dictionary/export?include_private=` | none | 200 export JSON, `Content-Disposition: attachment; filename="biorouter-dictionary.json"` | proof; private rows with standing and the confirmation | listable rows; private only with standing and the confirmation |
| `POST /dictionary/observations` | `{session_id \| null, observations: [{record_id, kind: "correction" \| "rejected", from, to, entry_id?}]}`, at most 16 | 204 always, including a silent no op | proof | admitted; the named chat through `session_reach` (silent no op on refusal); an observation whose stamp the caller cannot list, including one that names no chat from a caller without private standing, is a silent no op |
| `GET /dictionary/settings` | none | 200 `{suggest_from_chats, learn_from_chats, agent_writes, authority}`; `suggest_from_chats` reports whether `dictionary` is enabled for new chats | secret | secret |
| `POST /dictionary/settings` | `{learn_from_chats?, agent_writes?}` | 200 as GET | proof | lowering `agent_writes` and turning `learn_from_chats` off admitted; raising `agent_writes` and turning `learn_from_chats` on refused with MSG-DICTIONARY-HOST-SETTINGS |
| `POST /dictionary/activation` | `{suggest_from_chats, first_install?}` | 200 as `GET /dictionary/settings`; a first-install request is a no-op once activation version 1 exists, and a settings request may change the switch later; both values and the marker persist in one guarded config write | proof | `false` admitted; `true` refused with MSG-DICTIONARY-HOST-SETTINGS |

There is no `POST /dictionary/clear`. A full wipe is `biorouter dictionary clear` at a terminal with a typed phrase, and the page's **Delete all terms…** deletes the listed rows through `POST /dictionary/entries/delete` (DD-38). Managed `dictionary: false` refuses writes and observations with MSG-POLICY; export and delete stay allowed.

As built in PR-16 (`crates/biorouter-server/src/routes/dictionary.rs`), these details are fixed where the table leaves them open, each the smallest choice the routes needed:

- `POST /dictionary/activation` and the `suggest_from_chats` field arrive with the `dictionary` capability (PR-23's files list them), so PR-16's settings answer is `{learn_from_chats, agent_writes, authority}` and its settings route raises only `agent_writes` and `learn_from_chats`.
- A refusal's body is `{code, message}`, with `rev` beside them for `stale`. The codes are `agent`, `host_tier`, `host_settings`, `host_private`, `not_found`, `policy`, `unreadable`, `stale`, `cap`, `duplicate` (409, an edit to a spelling its tier already has), `too_large` (413), `bad_request` (422, a body, query string or cursor this daemon cannot read), `write_failed` and the validation codes. `not_an_export` and `unparsable` (422) are an import whose file cannot be read as a whole.
- The usage figures and `suggested_in` are absent, not `null`, for a caller without private standing. A private row also carries `came_from_a_chat` for a caller with private standing, the store's answer to which publicize warning the page shows, so the page never spells that rule again.
- The bulk routes answer counts (`{deleted: n}`), never ids. Accept is all or nothing: a suggestion that would pass the active cap fails the whole request with 409 and writes nothing. Dismiss and the bulk delete are deletes, so managed `dictionary: false` leaves them allowed, as it leaves `DELETE` allowed.
- On a daemon that holds a key, the export and observations need the proof like every write, and a caller without it gets MSG-DICTIONARY-AGENT; on one that holds none, both are admitted.
- The listing's `ETag` is the quoted first 32 hexadecimal digits of the SHA-256 of the page's bytes; its cursor is the last row's `updated_at` second and id, which the caller already holds.
- On a daemon that holds none, the tier route's MSG-DICTIONARY-HOST-TIER fills `<verb>` from the request's `tier` (`publicize` or `privatize`) and leaves `<term>` as written; `authority.tier_change.reason` is the publicize form. A body with no readable `tier` keeps `<verb>` too.
- An add's `origin_session_id` that `session_reach` refuses stamps the unknown `[]`, as an unresolved origin does, and both origin fields are read only when the stamp is private.
- The terminal can move a row to the other tier, keeping its id, between a request's first read and its write. So every write that names a row (an edit, a delete, the bulk routes) asks again, of the copy it holds under the store's lock, whether the caller may list that row: one it may not list since answers MSG-NOT-FOUND, or is skipped as missing in a bulk count, and an edit of a row that moved to another tier the caller may list answers `stale`. An add's and an edit's answer is the row as that write left it, read under the same lock. The settings route decides whether a request raises a setting again under the lock, from the settings as stored then, so a value the person lowered at the terminal in between is never written back by a request that named the old value (P-7).
- Measured over loopback HTTP on a debug build with 5,000 public rows (a 2.1 MB store) [M, 2026-11-06]: the first listing page of 500 rows takes 11.6 ms, a `304` 11.3 ms, every page 130 ms, a search 2.5 ms, an add 179 ms (p95 184 ms), an edit 153 ms, an add refused at the cap 54 ms, an import of 4,980 rows 441 ms, the export 53 ms and one bulk delete of all 5,000 rows 689 ms. A debug build parses JSON several times slower than a release build, so these are upper bounds; none of them sits between a stop and the text.

`EntryView` is the stored entry minus `seen`, plus the usage fields only for a caller with private standing; `affiliation` appears on private rows only, with each institution's published name ([Private rows and institutions](#private-rows-and-institutions)):

```json
{
  "id": "dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60", "rev": 3, "written": "EPAS1", "tier": "public",
  "status": "active", "source": "user", "reviewed": true, "spoken_forms": ["epas one"],
  "misrecognitions": [{ "text": "a pass one", "count": 2 }], "kind": "gene_symbol", "match": "exact",
  "bias": "auto", "context_cues": [], "note": null, "reason": null, "provenance": null, "affiliation": null,
  "created_at": "2026-11-01T09:00:00Z", "updated_at": "2026-11-04T17:02:11Z",
  "uses": 14, "last_used_at": "2026-11-04T17:02:11Z", "corrected_to": 2, "suggested_in": "20261104_12"
}
```

`authority` in the settings answer tells the page, before any click, what this caller may do, so no surface offers a doomed control. The answer below is a `serve` browser on a host whose operator's provider is public:

```json
{
  "suggest_from_chats": true,
  "learn_from_chats": true,
  "agent_writes": "on_request",
  "authority": {
    "write": { "allowed": true, "reason": null },
    "add_private": { "allowed": false, "reason": "Private terms are added on the server: `biorouter dictionary add --private <term>` or `biorouter dictionary import <file>` (as the service user)." },
    "tier_change": { "allowed": false, "reason": "Do this on the server: `biorouter dictionary publicize <term>` (as the service user)." },
    "raise_settings": { "allowed": false, "reason": "Do this on the server: `biorouter dictionary settings <option>` (as the service user)." }
  }
}
```

`raise_settings` covers raising `agent_writes` and turning `learn_from_chats` or `suggest_from_chats` on. The page fills `<term>` from the row it shows (`publicize` or `privatize`, MSG-DICTIONARY-HOST-TIER) and `<option>` from the control (`--agent-writes add`, `--learn-from-chats on`, `--suggest-from-chats on`, MSG-DICTIONARY-HOST-SETTINGS).

Auto rows with `seen` below 2 are never listed by any route or tool.

### Refusals

| Situation | Status | Body |
|---|---|---|
| Keyed daemon, write without the proof | 403 | MSG-DICTIONARY-AGENT: "Changing the dictionary needs the person. The agent's terms go through dictionary__add_terms." |
| Keyless daemon, tier change | 403 | MSG-DICTIONARY-HOST-TIER with `<term>` left as written, so the refusal names no term; the page fills the term into its disabled tooltip from the row it shows |
| Keyless daemon, raising `agent_writes`, or turning `learn_from_chats` or `suggest_from_chats` on | 403 | MSG-DICTIONARY-HOST-SETTINGS with the requested option |
| Keyless daemon, a `private: true` add or import (or an imported row marked private) from a caller without private standing | 403 | MSG-DICTIONARY-HOST-PRIVATE, one constant sentence decided from the request before any row is read |
| An id that is missing or not listable | 404 | MSG-NOT-FOUND |
| Managed `dictionary: false` on a write or observation | 403 | MSG-POLICY |
| Store cannot be read | 503 | MSG-DICTIONARY-FILE |
| Validation | 422 | `{code}`, one of the codes in [Validation](#validation); the page shows its id from [Validation and cap texts](#validation-and-cap-texts) |
| Stale `If-Match` | 412 | `{code: "stale", rev}` (MSG-DICTIONARY-STALE) |
| A person's add beyond 5,000 active rows in the stamp tier | 409 | `{code: "cap"}` (MSG-DICTIONARY-CAP) |

### `HttpCaller` methods and the census

`HttpCaller::capability` and `HttpCaller::admits` are private to `crates/biorouter-server/src/routes/session_reach.rs` [C `:966`, `:1051`], so the five new functions go inside that file. Listing copies `reach_knowledge_base`, not `routes/memory.rs`, whose `GET /memory/inventory` lists both memory stores to any secret holder [C] (DD-37).

```rust
// crates/biorouter-server/src/routes/session_reach.rs
impl HttpCaller {
    /// Omission, not redaction, and never a count.
    pub fn lists_dictionary_entry(&self, tier: EntryTier) -> bool {
        self.admits(TargetTier::from(tier))
    }
    /// The only HTTP door that builds a private `Standing` (`Standing::from_checked`). The proof
    /// or a stated private capability; never `served_operator`, which SD-10 gives listings only.
    pub fn dictation_standing(&self) -> Standing {
        Standing::from_checked(
            refuse_unless_reachable(self.enforced, TargetTier::Private, self.stated, self.proof).is_ok(),
        )
    }
    /// Keyed: the proof for every write. Keyless: a row the caller can list; tier changes,
    /// raising agent_writes and turning learn_from_chats or suggest_from_chats on refused with
    /// the host sentences; a requested private tier above the caller's standing refused with
    /// MSG-DICTIONARY-HOST-PRIVATE, judged from the request alone before any row is read.
    pub fn authorizes_dictionary_write(&self, write: DictionaryWrite, row: Option<EntryTier>)
        -> Result<(), DictionaryRefusal>;
    /// Keyed and proven: the person's choice (edits keep the row's tier).
    /// Keyless: the caller's standing. A chosen tier above it never reaches here, because
    /// `authorizes_dictionary_write` refused it.
    pub fn dictionary_write_stamp(&self, chosen: EntryTier, existing: Option<EntryTier>) -> EntryTier;
}

/// route_layer for every {id} route: parse the id, read the view, answer 404 with
/// MSG-NOT-FOUND for a missing or unlistable row before the handler runs.
pub async fn gate_dictionary_entry(headers: HeaderMap, Path(id): Path<String>, req: Request, next: Next) -> Response;
```

The tier route checks the proof (keyed) or refuses (keyless) before `gate_dictionary_entry` reads the id, so its answer cannot tell which ids exist.

As built in PR-16: `authorizes_dictionary_write(&DictionaryWrite, Option<EntryTier>)` takes the write as a `DictionaryWrite` (`Entries { private }`, `TierChange { to }`, `Settings { raising }`, `Export`) and answers a `DictionaryRefusal` (`status`, `code`, `message`); `gate_dictionary_entry` takes no state, reads the store through the routes' one cached handle, also answers 404 for an `auto` row seen once, and decides the tier route by reading its body's `tier` before the id. `dictation_standing` was already there from PR-15, so the census gained four `Guard` rows, not five, and new sites for `dictation_standing`, `session_reach`, `http_caller`, `target_tier`, `bias_set` and `chat_reach` in `routes/dictionary.rs`. The ordering table gained three rows for `read_chat`, the one function that reads a chat for a stamp, an add's origin and an observation's chat alike: the gate before `target_tier`, before the live model and before `session_affiliations`. `private_standing_is_built_in_two_files` is in `crates/biorouter/tests/privacy_guard_wiring.rs`, where it reuses the census's production scan (comments, strings and `#[cfg(test)]` items removed): every production call of `Standing::from_checked` must sit in `HttpCaller::dictation_standing` in `routes/session_reach.rs` or in `target::cli_bias` in `crates/biorouter/src/dictation/target.rs`, one each. The dictionary routes add no such call.

Census changes in `crates/biorouter/tests/privacy_guard_wiring.rs` (`routes/session_reach.rs` is one of its three `COMPLETE_MODULES`, so every new `pub fn` there needs a row):

- five new `Guard` rows, `lists_dictionary_entry`, `dictation_standing`, `authorizes_dictionary_write`, `dictionary_write_stamp` and `gate_dictionary_entry`, each `defined_in: SESSION_REACH` and `Status::Wired`;
- two rows written by hand, because the census discovers new predicates only in its `COMPLETE_MODULES` and says to add a row by hand for one outside them [C module doc]: `bias_set` (`defined_in: crates/biorouter/src/dictation/bias.rs`, `Status::Wired`, with its call sites in `routes/dictation.rs` and the CLI's dictation module) and `chat_reach` (`defined_in: crates/biorouter/src/dictation/target.rs`, `Status::Wired`, with its sites in `routes/dictation.rs`, `routes/dictionary.rs` and `extractor.rs`);
- new sites for `session_reach` and `http_caller` in `routes/dictionary.rs` and `routes/dictation.rs`, and for `visible_to` in `bias.rs`;
- a new site for `target_tier` in `routes/dictation.rs`, which moves its row from `Status::WiredThrough("session_reach")` to `Status::Wired`, because at `main` it has no caller outside `session_reach.rs` [C].

The table of `every_gated_route_resolves_the_tier_before_it_touches_the_session`, a test in `crates/biorouter-server/src/routes/session_reach.rs` [C `:1782`] rather than in the census file, gains a row for the observations handler. A repository walk test, `private_standing_is_built_in_two_files`, pins `Standing::from_checked` to `HttpCaller::dictation_standing` and the terminal rule, `target::cli_bias` in `crates/biorouter/src/dictation/target.rs`, where PR-25 left it; `crates/biorouter-cli/src/dictation/consent.rs`, which PR-25 added, mints `CaptureConsent` and asks `cli_bias`. No new `floor`, `raise_privacy`, `.call_tool(` or `CallCapability::sample(` site.

### Tier changes

A tier change is the person's act only (DD-39, P-13). A zero sized proof type, `UserDictionaryTierChange` in `crates/biorouter-dictation/src/store/tier_user.rs`, with a private field and `pub fn from_user_action()`, is the only way to call `StoreFile::set_tier`. It is constructed in exactly two files, the pattern of `UserKbTierChange` [C `crates/biorouter-mcp/src/knowledge/tier_user.rs`]:

- `crates/biorouter-server/src/routes/dictionary.rs`, after the proof and before the id is read;
- `crates/biorouter-cli/src/commands/dictionary.rs`, after the typed confirmation at an interactive terminal with `BIOROUTER_TERMINAL` unset (O-5, the `session declassify` precedent [C `crates/biorouter-cli/src/commands/session.rs` `declassify_command`]).

`the_tier_proof_is_constructed_in_exactly_two_files` pins both. There is no tool for it, ever.

**Acceptance.** PR-16: `tests/dictionary_routes.rs` (including a correction for a chat bound to a private provider whose stored classification is still public, which lands in a private row); `tests/dictionary_routes_auth.rs` (the secret on every route); the keyless binary `tests/dictionary_no_user_key.rs` (add, accept and edit admitted with standing; tier changes, raising `agent_writes` and turning `learn_from_chats` or `suggest_from_chats` on refused with the host sentences, turning them off admitted; a private host's `X-Caller-Provider` lists private rows), in its own binary because the user action digest is a process global `OnceLock`; `tests/dictionary_oracles.rs` (every rule above, byte identical, including: a `private: true` add from a caller without private standing gives the same 403 for an existing private term and for an absent one; such a caller's import never touches private rows; such a caller's observation that names no chat leaves the store byte identical); observations idempotent on `(record_id, from, to)`, with PII and secret pairs dropped. The census is green, and `private_standing_is_built_in_two_files` passes.

## How dictation uses the dictionary

### The pipeline

One implementation for every surface, in the leaf crate's `recognize/` and `engine/` modules, driven by the core service (PR-14). The worker frames (`set_bias`, `bias_ready`, `transcribe`, `result`) are specified in [Engine and models](engine-and-models.md).

```text
audio
 -> voice detection split (segments of 25 s or less)
 -> bias_set (EligibleEntries, tier filtered)        crates/biorouter/src/dictation/bias.rs
 -> rank against the request's context              recognize/select.rs
 -> token prompt: top 30 written forms, <= 150 BPE tokens   recognize/prompt.rs, engine/bpe.rs
    decoder bonus: whole bias set, built but off     engine/booster.rs
 -> greedy decode, timestamps on                     engine/mod.rs
 -> echo, recital and collapse checks, at most one retry   recognize/echo.rs
 -> whole output filter                              recognize/text.rs
 -> correction: aliases, exact keys, guarded fuzzy  recognize/correct.rs
 -> {text, raw_text, changes[], bias_scope, ...}
```

With the bonus on, which no shipped dictation does since PR-14b (only `BiasChannels::BOTH` in tests and `BIOROUTER_DICTATION_MEASURE_BONUS=1` in a debug build turn it on), the service sends `set_bias` for the bonus table a dictation uses (public rows, all rows, or one private table per target affiliation) when the dictation starts, unless the worker already holds that revision, and the worker decodes the English word list only when such a frame arrives (PR-15), so a shipped dictation neither builds a table nor decodes the list; a table's revision follows the store generation, so a changed store gives a new table. After each model load the supervisor sends every installed table again, before any decode. It never waits for `bias_ready`: the worker handles frames in order, so a `set_bias` written before a `transcribe` is built before that decode starts, and no decode waits on the answer. A `transcribe` frame carries the ranked written forms for the prompt and names the table; the worker encodes the prompt, decodes and returns the segments and the exact prompt string it sent, which the service keeps in memory for the echo check and never logs.

### Ranking

`recognize/select.rs` scores each eligible entry per request (DD-45). `in_context` is 1 when a written, spoken or active misrecognition form occurs as a whole word in the request's `context` (case folded). `context` is at most 2 KiB (desktop and `serve`), the text before the cursor up to 1,000 characters (TUI and REPL), or `--context-file` up to 4 KB (`transcribe`); it ranks only, never enters the prompt and is never logged.

```text
score = 5.0 * in_context
      + 3.0 * pinned                                   bias == always
      + 2.0 * min(1, corrected_to / 3)
      + 1.5 * exp(-ln2 * days_since_last_used / 30)    0 when never used
      + 1.0 * min(1, ln(1 + uses) / 3)
      + 1.0 * in_active_kb
      - 4.0 * collides * (1 - in_context)
ties: updated_at descending, then the fold key, then the written form, then id
```

Terms added together share a score and an `updated_at` second, so the order among them comes from the terms themselves: the fold key ([Uniqueness, fold keys and twins](#uniqueness-fold-keys-and-twins)), then the written form, which separates two spellings that fold alike (`EPAS1` before `Epas1`). Until PR-14b ties fell to the random ids, so two dictionaries holding the same terms sent their prompts in different orders, and in PR-14's live check one transcribed 3 of 8 clips differently from the other [M]. The id now decides only between twins, which the bias set has already merged. Equal terms give equal prompts only at equal usage: since `uses` and `last_used_at` move, a term's place in the prompt, and so the transcript of the same clip, can change from one dictation to the next. In PR-14b's live check 3 of 42 `small.en` clips and 1 of 42 turbo clips changed between two repetitions with the dictionary, and none without it; two dictionaries with the same terms under different ids still matched on all 176 paired runs, because their usage moved in the same order [M, 2026-11-06].

There is no penalty for a term used often and never corrected. The first ranking had one (`- 2.0 * saturated` for 5 or more uses with no correction), meant to give prompt room to terms the model gets wrong. Once `uses` moved (PR-14b) it fired on terms the model writes right only because the prompt carries them: in PR-14b's live check EPAS1 went from 0 of 6 on turbo without the dictionary to 6 of 6 with it, then after five dictations ranked below every term used fewer than five times [M], so in a dictionary of more than 30 terms it would leave the prompt and be misheard again. It was removed on 2026-11-06. A use cannot yet tell a prompted dictation from an unprompted one; a penalty can return once it can.

`uses`, `days_since_last_used` and `corrected_to` read the `public` usage map when the request uses the public rows table, and the `all` map otherwise (oracle rule 11). The weights are a starting point [I] and stay as written: the lab recordings that would have tuned them were withdrawn by the owner's decision O-23 (2026-11-05), and the public recordings of M-22 hold too few terms per utterance to tune a ranking. `in_active_kb` is 0 for every entry in v1, because no field yet records which knowledge base a term came from. Two consumers take a slice of the ranking: the prompt (top 30) and the fuzzy set (the `match = fuzzy` entries among the top 30). The decoder bonus, the alias automaton and the exact keys take the whole bias set unranked.

### Token prompt

`recognize/prompt.rs` picks; `engine/bpe.rs` encodes (DD-45):

1. Eligible: in the bias set, and not `collides` unless the entry is in `context` or pinned.
2. Take the top 30 by score. When fewer than 30 entries match the context, the remaining places go to the next entries by score, where pinned and recent ones rank; the prompt is never only the terms predicted for this utterance, because an oracle narrow prompt cost 18 points on real speech by deleting the start of the utterance [M].
3. Written forms only, joined with `", "`, no lead in, no leading space, ascending score so the best entry is last, then a period after the best entry (the tail, `prompt::with_tail`, PR-14b), which `DictionaryRequest::prompt_terms` adds when that entry ends in a letter or a digit, so the period is its own piece and its id is the period token; an entry ending in other punctuation (`C++`) is sent without it. Spoken forms and misrecognitions are never prompt content. Without the tail the list read as an unfinished sentence and Whisper dropped the final punctuation of what it wrote: turbo on 26 of the 42 synthetic dictation clips, 33 of the 75 probe clips and 29 of the real clips that ended a sentence with no dictionary, small.en on 3, 4 and 8. With the tail, measured with the shipped mechanism under a rule fixed before the numbers, it came back on 83 of turbo's 88 and all 15 of small.en's, with real recall 166 against 165 (turbo) and 151 against 151 (small.en) of 189, synthetic recall 70 against 71 and 74 against 73 of 81, and the same insertions [M, 2026-11-06] ([speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md#addendum-2026-11-06-the-prompt-tail-and-the-shipped-defaults-tier-1-floors)).
4. Encode with canonical byte level BPE whose ranks are the loaded model's own token ids, and pass the ids as `prompt_tokens` with `initial_prompt` unset. Cut entries from the head until the prompt is at most 150 tokens. Whisper's window is 223 tokens and drops its head silently [R].
5. No draft text in the prompt in v1.

The encoder: ranks equal the model's token ids (turbo's vocabulary differs from `multilingual.tiktoken` only at id 188; `.en` models use the GPT-2 vocabulary) [M]; the GPT-2 pre split regex through `fancy-regex` (in `Cargo.lock` [C]); `tiktoken-rs` 0.6.0 (already a core dependency [C]) if its `CoreBPE` takes an arbitrary rank map, otherwise a byte level encoder of about 100 lines. No tokenizer text file ships. Golden splits: ` EPAS1` to ` EP|AS|1` and ` Biorouter` to ` B|ior|outer` [M].

Why ids and not text: whisper.cpp tokenizes `initial_prompt` text by greedy longest match, so the model copies a segmentation it was never trained on (` EPA|S|1`). On synthetic speech, the same list as canonical ids raised turbo from 80 to 93 % pooled strict recall and small.en from 85 to 90 % [M]. On real speech a text prompt was the largest single gain measured, +24 to +33 points on gene symbols [M]; ids on real speech are part of M-22.

Budget: 30 terms cost 115 to 118 tokens [M], so the 150 token cap rarely binds. A 54 term prompt (209 to 217 tokens) cost 2 to 10 points against 14 terms on synthetic speech [M].

### Decoder bonus

`engine/booster.rs`, a `logits_filter_callback` over the whole bias set, greedy decoding only (DD-46). It is built and tested but off by default: M-22 found that with the prompt it raised insertions on the probe clips and reversed speech ([The M-22 gate and the declared fallback](#the-m-22-gate-and-the-declared-fallback)), so `BiasChannels::DEFAULT` is the prompt alone. Measurement turns it on with `BiasChannels::BOTH` in a test, or with `BIOROUTER_DICTATION_MEASURE_BONUS=1` for a debug build of `biorouter transcribe`; a release build ignores that variable, as it ignores `BIOROUTER_DICTATION_TEST_MODEL`, so what ships always decodes with the default. It is a measurement switch, not a setting, and no interface, configuration key or route sets it. With the bonus off no table is built and no `set_bias` is sent:

- A trie over the bytes of `" " + written` and `written` for each entry with `bias` `auto` or `always` and not `collides` (unless pinned). Aliases are never boosted.
- Per trie node, the vocabulary ids below end of text whose bytes continue some form from that node, precomputed from a bytes to id map of the model's vocabulary.
- Per callback, stateless: rebuild the byte tail from the decoded tokens back to the last id at or above end of text; find the nodes reached by a suffix that starts at a word boundary (a space, the window start, or after `( [ / " '`); add the gated bonuses.

| Parameter | Value | Meaning |
|---|---|---|
| `b_start`, `gate_start` | 2.0, 2.0 | bonus for a token that starts a form, given only when its logit is within the gate of the row maximum |
| `b_cont`, `gate_cont` | 5.0, 5.0 | bonus and gate for a token that continues a form |
| `ramp` | 2 | continuation bonus and gate scale by `min(1, matched core bytes / 2)` |
| `first` | 0 | no start bonus at step 0 of a window |
| `alias_factor` | 0 | aliases never boosted |
| `min_start` | 2 | a start piece covers at least 2 characters |
| `cap` | 5.0 | the bonus per token is the maximum over sources, clamped here |
| English start pieces | skipped | a start piece that case folds to an English word is skipped unless it is the whole term |
| Prefix steal guard | on | never boost a candidate whose bytes are a strict prefix of the model's current top token when that token is not a candidate |
| End of text | untouched | never an id at or above `whisper_token_eot` |

The callback is reentrant and runs inside `std::panic::catch_unwind`. Tables are built per (model hash, store generation, table) with an LRU of 4, and every table in use is rebuilt at each generation (oracle rule 9). Measured on synthetic speech: 33 to 52 ms to build, 0.57 ms per utterance; 72 % strict recall at 1,000 terms against 49 to 52 % for the same terms as a prompt; 0 dictionary insertions on 75 probe clips [M].

As built in PR-14 (`engine/booster.rs`), measured on an M4 Max with small.en (f16) on Metal, a release build, over the synthetic set's 36 term clips [M, 2026-11-06]: the table for 1,000 terms built in 5.6 ms and the shipped bias set's 864 (the 136 that collide left out) in 1.5 to 1.8 ms, in the worker, when the dictation starts; a callback took 54 to 60 µs, almost all of it reading the row's 51,866 logits for its maximum, about 1.1 ms per utterance, and the decode with the bonus took 106 to 111 ms per clip against 105 to 106 ms without. Reading the row in eight lanes the compiler vectorizes (`best_of`, the same answer as one scan) then cut a call to 10.7 µs against 56 µs for one plain pass over the row, in `reentrant_and_fast` with a 1,000 term table [M, 2026-11-06]; an unoptimized debug build takes about 235 µs a call. That test now holds a call to three plain passes over the row plus 50 µs in every profile, so a callback that reads the row several times fails in CI's debug build too, and to the plan's 100 µs in a release build. Strict recall of the 14 targets: 38 % with no dictionary; with the spike's decode (timestamps off, no detector) and all 1,000 terms 70 % (57 of 81, the spike's 72 % was 58), which `bonus_at_1000_terms_on_the_benchmark_audio` holds as its floor; with the shipped decode 69 % for all 1,000 terms and 68 % for the shipped bias set, because PHD2, SPOKE and CRISPRi `collide` and are boosted only when pinned; the 14 terms as the token prompt alone 91 % in 113 ms, and the prompt with the shipped bonus 99 % (80 of 81) in 116 ms. The start piece filter reads only the words the list holds in lower case or in capitals, the reading `collides` takes: with every listed word it skipped ` PH` (from `pH`) and PHD2 could not start, which cost the spike's set 6 occurrences.

### Decoding parameters

Every dictation request (DD-47); the full table with reasons is in [Engine and models](engine-and-models.md):

| Parameter | Value |
|---|---|
| strategy | `WHISPER_SAMPLING_GREEDY`, `best_of = 5` |
| `language` | `en` for `.en` models; the configured language for turbo, never `auto` unless set |
| `translate`, `no_context` | false, true |
| `no_timestamps` | false; timestamps stripped from text. Timestamps off collapsed small.en on 12 of 49 real utterances, 2 of 49 with them on [M] |
| `single_segment` | false |
| `temperature`, `temperature_inc` | 0, 0.2 (kept by M-22, 2026-11-06) |
| `entropy_thold`, `logprob_thold`, `no_speech_thold` | 2.4, -1.0, 0.6 |
| `suppress_blank`, `suppress_nst` | true, false (`suppress_nst` would block `HLA-B*57:01` and `C++`) |
| `prompt_tokens`, `initial_prompt` | set, unset |
| `flash_attn` | on |
| audio per `whisper_full` call | at most 25 s, asserted on the parameter builder |

### Voice detection and nonspeech filters

Silero always on, on every surface (DD-48): threshold 0.50, minimum speech 250 ms, minimum silence 100 ms, `speech_pad_ms = 30`, and 0.3 s kept before the first and after the last segment. On 16 real gaps (10 of room tone and 6 where only the other party bleeds in, bracketed tags not counted), turbo's output fell from 15 or 16 of 16 to 1 of 16 with the detector, and small.en's from 2 to 8 of 16 to 0 of 16 [M]. No segments or an all zero buffer: nothing is inserted, and the transcript reports `no_speech`.

A whole output filter in `recognize/text.rs` runs after trimming punctuation and removes the output only when all of it is one of: a run of bracketed or parenthesized tags, `you`, `Thank you.`, `Thank you for watching!`, `Okay.`, `so`, `and`, `.`, `Grazie`, `Grazie a tutti`, `Sous-titrage Société Radio-Canada`, `Gracias por ver el video`, `Altyazı M.K.`. Parentheses inside real speech are never stripped.

### Echo, collapse and one retry

Checked against the exact prompt string sent, not against the dictionary:

| Check | Rule |
|---|---|
| Whole echo | 90 % or more of the output's unique words are prompt words, and it continues the prompt's shape: a word repeated 3 or more times, a short fragment (at most 30 characters) ending on the separator, or 3 or more consecutive entries in prompt order [R: OpenWhispr's heuristic, `analyzeDictionaryPromptFragment` in `src/utils/dictionaryEchoFilter.js`; whole prompt echo and term looping on Whisper are reported in OpenWhispr issue #1454 and in that file's comments] |
| Trailing recital | a sentence copying 2 or more consecutive entries from the prompt head, or 5 or more ending at its tail [R: OpenWhispr's `stripTrailingDictionaryEcho` in the same file; the recital was reported on Qwen3-ASR-1.7B, a prompt conditioned model, in OpenWhispr issue #2581, with related reports in #1547 and #1889] |
| Collapse or deleted start | fewer than 1.0 output words per second over more than 3 s of detected speech [I] |

The 90 %, 3, 2 and 5 thresholds are OpenWhispr's design constants, not measurements of Whisper, and the trailing recital is not reported on Whisper at all. Biorouter's own evidence for echo is the nonspeech decodes in [The bias set](#the-bias-set); how often these checks fire wrongly on real speech is unmeasured.

On an echo, a collapse or a deleted start: one retry with no prompt, the voice detector and the bonus still on. The retry is adopted only if it is not itself prompt like; otherwise the transcript is `no_speech`. A trailing recital is stripped, not retried. The transcript reports `retried: {reason}` with no text. A retry without the voice detector is never made, because it reopens hallucination [M].

### The M-22 gate and the declared fallback

The token prompt and the decoder bonus together ship as the default only if M-22 shows the combination, on public real speech (the PriMock57 subset and a subset of the Fareez et al. 2022 interviews) and on synthetic speech, at least as good as each alone, with no rise in nonspeech output on the PriMock57 gaps and the synthetic nonspeech clips, in insertions on the synthetic probe clips, or in collapses (DD-44). Otherwise the declared fallback is the token prompt only, with the bonus off, because the prompt is the only channel with real speech evidence. The choice is a compiled default (`BiasChannels::DEFAULT`) changed only by a release, which no setting and no variable a release build reads can change; correction always runs. **Outcome (2026-11-06).** M-22 chose the declared fallback: both channels together had the best recall but raised insertions on both models (turbo wrote `UCSF` for the probe's UCSD three times, and `EGLN1` and `salbutamol` in reversed speech) [M] ([speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md#the-m-22-channel-decision-2026-11-06)). PR-14b made it the compiled default, `BiasChannels::DEFAULT = { prompt: true, bonus: false }`, kept the bonus and its tests, and set the Tier 1 floors again for it. `BiasChannels::BOTH` (in tests) and `BIOROUTER_DICTATION_MEASURE_BONUS=1` (for a debug build of `biorouter transcribe`; a release build ignores it) turn the bonus on for measurement only; neither is a setting. M-22 also decided `temperature_inc`: it stays 0.2. M-22 replaces M-9, which needed the lab recording set the owner withdrew on 2026-11-05 (O-23). MultiMed is not used for M-22 or for any other test or measurement, decided by the owner on 2026-11-05 (O-38), so how the dictionary shapes gene symbols on real speech stays unsettled: no licensed public recording of real speech with gene symbols exists.

### Long dictation

During a stream, once 8 s or more are buffered the daemon asks the worker for voice segments and queues each closed segment (at most 25 s, split at the longest pause), decoded with `no_context = true` and the same prompt and table; the last segment is decoded on stop (DD-50). Each segment is corrected for the `partial` preview line. At finish, correction runs once more over the joined raw text, so `changes` offsets refer to the final text. M-19 measures boundary word loss on real continuous speech and blocks announcing v1.

**Acceptance.** PR-14: BPE parity with golden ids committed as numbers per vocabulary (only 270 ids hold the same bytes in the GPT-2 and multilingual vocabularies [M], so no id list is shared between them); a 600 token dictionary trims to at most 150 tokens with the top entry last; the bias table test (target tier, entry tier, standing, switch; clipboard, Crew, app and unknown CLI targets public unless `include_private_terms` or `--private-terms` is admitted under O-8); `eligible_entries_are_built_only_by_bias_set`; bonus cases on synthetic logit rows (never at or above end of text, identical from several threads, under 100 µs a call); an agent shell `transcribe --session <private id>` on reversed speech, on a machine configured for a private provider, returns no private term; the `#[ignore]` bonus test reproduces 70 % strict or more at 1,000 terms with small.en; one dispatched Tier 1 run records the dictionary arm. Token count goldens: the 14 term list costs 62 tokens (small.en) and 63 (turbo) under both `whisper_tokenize` and canonical BPE [M]; the 26, 37 and 14 term lists cost 111, 149 and 62 tokens on the `base.en` vocabulary (as small.en) and 119, 160 and 63 on the `tiny` vocabulary (as turbo) under `whisper_tokenize` [M]; PR-14 commits the canonical counts for the two longer lists from its reference encoder, which can be up to 2 % higher [M]. Measured by PR-14's encoder, which gives the research's reference encoder's counts exactly [M, 2026-11-06]: the 26 drug terms cost 118 tokens on `base.en` (6.3 % above `whisper_tokenize`'s 111) and 123 on `tiny` (3.4 % above 119), and the 37 gene terms 152 (2.0 % above 149) and 160 (equal); the 14 terms 62 and 63. So "up to 2 %" held for the gene list and not for the drug list, whose INN suffixes split differently; both stay well inside the 150 token budget. `bpe_parity_on_the_fixture_vocabularies` holds all twelve counts and the golden splits ` EP|AS|1` (`base.en` ids 14724, 1921, 16; `tiny` 25330, 3160, 16) and ` B|ior|outer` (347, 1504, 39605; 363, 1973, 23985).

## Correction after decoding

One pass in `crates/biorouter-dictation/src/recognize/correct.rs`, every change recorded as `{start, end, from, to, stage, entry_id}` and listed in the transcript's `changes` (DD-49). `stage` is `alias`, `exact` or `fuzzy`; `start` and `end` count characters (Unicode scalar values) of the final text and bound `to`. The automaton, the key map and the fuzzy set are built when the dictation starts (`Corrector::new`), so correcting at stop is one pass over the text: 31 to 34 µs for a 5 s utterance with the 14 term dictionary in a debug build, and at most 2.1 ms in the CLI's debug runs [M, 2026-11-06]. `transcribe --format json` schema 1 uses the same names as the HTTP `Transcript`: `changes` with `stage`, `audio_s` and `bias_scope`, so `transcribe`'s callers and the renderer read one shape.

### Field effects per channel

| Field | Token prompt | Decoder bonus | Aliases (step 2) | Exact keys (step 3) | Fuzzy (step 4) |
|---|---|---|---|---|---|
| `written` | sent | startable form | target | key | target |
| `spoken_forms`, active `misrecognitions` | never | never | sources (wordy ones under the context rule) | extra keys | no |
| `bias = never` | excluded | excluded | yes | yes | yes |
| `bias = auto` | by score; excluded if `collides` and not in context | yes unless `collides` | yes | yes | if `match = fuzzy` |
| `bias = always` (pinned) | always ranked | yes, even if `collides` | yes | yes | if `match = fuzzy` |
| `match = prompt_only` | yes | yes | no | no | no |
| `match = exact` | yes | yes | yes | yes | no |
| `match = fuzzy` | yes | yes | yes | yes | yes, within the fuzzy set |

### Step 1: tokenize and protect

Tokenize on Unicode word boundaries. Protect, and never change, spans inside backticks, URLs, emails, file paths, and any token that contains `@ / \ _ = : + # < > * % | ^ ~ &` or an inner `.`.

### Step 2: aliases and misrecognitions

An automaton from `aho-corasick` 1.1.4 (in `Cargo.lock` [C]) over NFC, case folded text, `MatchKind::LeftmostLongest`, whole words only (no letter, mark, digit or `_` adjacent), never across `, . ; : ! ?`, single pass. The output is the entry's `written` form verbatim. Only misrecognitions with `count` 2 or more take part. A wordy form (every token is an English word: `oh mop`, `bio router`) applies only when the entry is in `context`, a `context_cue` occurs in the utterance, or it is a misrecognition the person's corrections made 2 or more times. Measured on trap sentences, unguarded aliases rewrote `Oh mop it up` to OMOP and `The bio router in the closet` to Biorouter [M].

### Step 3: normalized exact keys

Windows of 1 to `min(8, letters(key) + 1)` tokens that do not cross punctuation. The key: NFKC, lowercase, Greek letters to names, number words to digits (`zero` to `nineteen`, tens, `fifty three` and `five three` to `53`), then drop spaces, hyphens, periods and apostrophes. A window is rewritten only when its key equals an entry key (the written form or any spoken form or active misrecognition) exactly. So `E-PAS-1`, `EPAS 1` and `E P A S one` all give `epas1`, and `HIF two alpha` and `HIF2-alpha` give `hif2alpha`. Guards:

- a `digits_literal` entry never matches digits that came from number words (`PhD two` stays);
- a single lowercase English token is not rewritten unless capitalized mid sentence or carrying inner capitals (`spoke` stays; mid sentence `Spoke` becomes SPOKE);
- a window equal to another entry's written form is never rewritten;
- a key shared by two rows with different written forms is skipped ([Uniqueness, fold keys and twins](#uniqueness-fold-keys-and-twins)).

PR-14 adds one guard, the wordy alias rule applied to keys: a window of two or more tokens whose every token is an English word (a small single letter such as `i` or `a` counts as a word; a capital single letter is a spelled letter, and a token with a digit is not a word) is rewritten only when its entry is in `context` or one of its cues occurs in the utterance. Without it the key of `Biorouter` rewrote `The bio router in the closet` and the key of `CRISPRi` rewrote `CRISPR i think`, both negatives of the acceptance below, while `E P A S one` and `pH D2` still match.

The inserted text is the entry's written form in its own case; `gene_symbol` and `acronym` kinds never take the spoken token's case. On every real speech run the exact key stage made no false edit; on synthetic speech it turned `PhD two` into PHD2, which the `digits_literal` guard stops, and `spoke` into SPOKE with the single token guard off [M].

### Step 4: guarded fuzzy matching

Only for entries in the fuzzy set (the `match = fuzzy` entries among the top 30), and only when every guard holds:

| Guard | Rule |
|---|---|
| Span | 1 to 3 tokens, no punctuation inside, key length 4 or more |
| Shape | length ratio between 0.67 and 1.5; equal digit strings |
| Similarity | normalized Levenshtein similarity of keys 0.80 or more, or 0.60 or more with Double Metaphone similarity 0.75 or more (`strsim` 0.11.1 is in `Cargo.lock` [C]; Double Metaphone is implemented locally, not added as a dependency: PR-14's `double_metaphone` is a compact port that covers the consonant classes English spelling confuses most, codes of at most 6 characters, the similarity being the best over the primary and alternate codes) |
| Abstain | the best and the runner up are within 0.04 |
| English | no token of the span is in the word list (with inflections and contractions) unless the entry is in `context` or a `context_cue` occurs |
| Collisions | the span is not a correctly spelled other entry, and not a stop list near collision: UCSD and UCSF, NTRK and NRAS, exon and EGLN1, CRISPR and CRISPRi, clinic and Clenil, continue and codeine, smoke and SPOKE, coding and codeine |
| Prompt pieces | the span is not a truncated piece (a prefix of 4 or more characters) of another entry that was in the prompt sent. The guard reads the prompt's written forms without the tail (`DictionaryRequest::correction_prompt`), because it exempts the entry the span would be corrected to by its written form: read with the period, the best entry (`Modulite.`) stopped exempting itself and a truncated hearing of it (`Modulit`) stayed uncorrected, until the review of PR-14b (`the_tail_does_not_reach_correction`). Only the period the tail added is removed, so a best entry that ends in its own punctuation (`Ph.D.`) is read as it is |
| Stop words | a span of several tokens never starts or ends with a stop word |

No LLM corrector in v1.

### Defaults by kind

| Kind | Default `match` |
|---|---|
| `gene_symbol`, `acronym` | `exact` |
| `name`, `term` of 6 or more letters, no digit, not `collides` | `fuzzy` |
| anything of 5 characters or fewer, any `letter_digit` shape, any `collides` entry | `exact` (never fuzzy) |
| `kind` null | as `term` |
| an agent row | `exact`, always (DD-54) |

The store writes the default when a row is created (`MatchPolicy::default_for`, PR-13). An edit that changes the spelling or the kind and names no `match` of its own moves a stored value equal to the old form's default to the new form's default, turns `fuzzy` into `exact` when the new form's default is `exact`, and keeps anything else (`prompt_only`, an `exact` chosen for a long name) (`match_after_edit` in `ops.rs`): `Ventolinn` edited to `HRAS` stops being fuzzy. An agent row stays `exact`. The `collides` arm needs the English word list, which arrives with PR-14, so PR-14's matcher never applies fuzzy matching to a `collides` entry whatever its stored `match`.

### What was measured

Fuzzy matching is what makes coined names work (`ventilin`, `klenl`, `FLET3`) [M], and over a 26 term drug list the guarded corrector made 0 genuine false edits across 7,108 real consultation utterances [M]. It is unsafe for short symbols: over a 37 term gene list, the guarded pass made 45 false edits across 4,651 real sentences (`here's` to HRAS 21 times, `aren't` and `ain't` to ARNT 12 times, `ras` to HRAS, `norms` to NRAS) [M]. Over the same sentences the 14 term list of the synthetic benchmark made 13 false edits (`UCSD` to UCSF 4 times, `exon 1` to EGLN1 3 times, `spikes` to SPOKE, `CRISPR` to CRISPRi) [M]. On real audio, the corrector turned NTRK, heard as `N-TRAC`, into NRAS [M]. The defaults above and the stop list would have blocked every one of those measured false edits [I]; M-22 and the Tier 1 gate check that on public real corpora.

### The English word list

`crates/biorouter-dictation/assets/english-words.zst`: a SCOWL derived list (size 60, US and GB spellings, with inflections and contractions), lowercase, zstd compressed, embedded with `include_bytes!`, generated by a script under `scripts/dictation/` from a pinned source with every word the retired name guard (R-31) matches removed, plus the hand list `phd uc ucsd crispr rna dna pdf`. It drives `collides`, the bonus's start piece filter, the single token exact guard, the fuzzy English guard, the wordy alias rule and the learner's filters. A system word list is never used: it lacks `coding`, `here's`, `aren't`, `encodes` and `cells`, and is absent on Windows [M].

As built in PR-14 [M, 2026-11-06]: `scripts/dictation/make-english-words.py` reads one export of the English Speller Database (ESDB, the successor of SCOWL) from `app.aspell.net/create` at size 60 with the US, GB, CA and AU spellings, variant level 6 and the hacker and roman numeral lists, pinned by its SHA-256 (the export's header records ESDB's git revision), and stops on any other file. The list holds 113,440 words (44 removed by the guard, whose pattern the script reads from the guard itself so it never spells it), 1,097,006 B as text and 263,573 B compressed; the source's copyright and permission notices travel inside it as `#` lines, because its license asks for them in every copy. A word the export lists only capitalized or in mixed case (`Boston`, `Ventolin`, `pH`; 18,716 of them) carries a `+` mark: it counts as English for the guards that keep correction off ordinary speech (the single token guard, the fuzzy English guard, the wordy rules), and does not make a term `collide` or a start piece English, because nobody says `Ventolin` meaning anything else. Without the mark, `Ventolin` and `Clenil`'s fuzzy positives could never be corrected, since a `collides` entry is never fuzzy matched, and the start piece ` PH` of PHD2 (from `pH`) could never start a term. The list decodes once per process into one buffer and a sorted index, in a few milliseconds, when the first bias set is built, so a lookup is a binary search with no allocation and nothing on the path from stop to text pays for it. The worker decodes its own copy, which its first bonus table reads, on a thread of its own while its model loads: decoded inside that first `set_bias`, which the supervisor sends right after `loaded`, it held a one shot `transcribe`'s first detector request back by 19 to 20 ms in a debug build (the median over 84 paired runs per model), and decoded during the load it holds nothing back (0 ms), which cut the dictionary's whole cost to such a run from 33 to 16 ms in that build [M, 2026-11-06]. Since the bonus is off by default (PR-14b), that decode during every load was spent on a list nothing read, about 20 ms of a debug build's CPU beside the load, `--no-dictionary` included; since PR-15 the worker decodes it only for a bonus table: the reader thread starts the decode on a thread of its own when a `set_bias` frame arrives (`warm_word_list` in `worker/serve.rs`), so no frame waits for it, and a shipped dictation never decodes it there. The parent still decodes its own copy for correction. `the_word_list_passes_the_retired_name_guard` checks every word and every notice line against the guard's own pattern.

**Acceptance.** PR-14: the key table (`E-PAS-1`, `EPAS 1`, `E P A S one` all to `epas1`); positives (`ventilin`, `klenl`, `FLET3`, `ALAS5AP`, `P-R-O-K-R2`, `IGF-1R`, `E-PAS-1`, `pH D2`) and negatives that must stay unchanged (`here's`, `aren't`, `UCSD`, `exon 1`, `N-TRAC`, `clinic`, `continue`, `coding`, `smoke`, `PhD two`, `crisper drawer`, `Oh mop it up`, `The bio router in the closet`, `CRISPR i think`, `encodes`, `cells`, `Boston`); the word list passes the retired name guard.

## Learning from the person's edits

When the person corrects a dictated word before sending, the dictionary learns from it (DD-58, PR-22 on the desktop and `serve`, PR-26 in the terminal). Only short correction pairs ever leave the composer; the raw transcript never does, and nothing is stored on the outgoing message.

### Desktop and `serve` ledger

```ts
// ui/desktop/src/utils/dictationLedger.ts
export type DictatedSpan = {
  recordId: string;            // the stream id of the dictation
  start: number;               // current position in the prose
  end: number;
  current: string;             // the text now in [start, end)
  raw: string;                 // the transcript's raw_text
  inserted: string;            // the text inserted
  changes: TranscriptChange[]; // the transcript's changes
  state: 'pristine' | 'edited' | 'tainted' | 'undone';
  model: string;
  dictionaryRevision: string;  // a listing ETag when known, otherwise ''; never a store wide counter
  biasScope: 'public' | 'private'; // for the private draft flag (DD-71)
};

export type ComposerDraft = { text: string; images: ...; files: ...; dictation?: DictatedSpan[] }; // at most 16 spans
```

- The ledger lives in renderer memory only, on the existing `ComposerDraft` [C `ui/desktop/src/utils/composerDrafts.ts` `ComposerDraft`], so it survives remounts. `isEmptyComposerDraft` ignores it; it is never persisted, logged or attached to message metadata.
- One `useLayoutEffect` maps every span through prose changes with `locateEdit` (a single region prefix and suffix diff, with the post edit caret as the hint). A programmatic replacement that touches a span marks it `tainted`, even when a diff could place that replacement; an undo back to the pre insert text marks it `undone`.
- As built in PR-22, the three exits that hand the composer's text away, `performSubmit`, `handleInterruptionAndQueue` and `handleSteerFromComposer` [C `ui/desktop/src/components/ChatInput.tsx`], are factored into `releaseComposerText(reason)`. `/crew` and `/diverge` drop the ledger. The message editor does the same at `handleSave` in `UserMessage.tsx` for an accepted save in the current chat. **Save as new chat** drops its ledger in v1 because the editor callback does not expose the new destination. A failed or cancelled editor save teaches nothing.
- Pairs are posted only after the send is accepted, with `userActionHeaders()`. A cancelled send posts nothing. The request names the chat the send went to, including the chat a Home send just created; `null` only when no chat id is known. The new chat and Home handlers report that id through a renderer callback before navigation. The callback carries no transcript or ledger. Queued spans follow their queue objects in a memory only side map and return with a refused message. A recording that finishes after its composer unmounts writes its spans into that composer's draft. No dictionary read is added to the insertion or send path just to fill `dictionaryRevision`; it is empty when no listing ETag is known.

### Extraction filters

Two stages. `utils/dictationLedger.ts` holds the span text, so it computes the word level LCS between `inserted` and `current` for each `edited` span and drops a span whose edit is a rewrite (more than half the words changed). `crates/biorouter-dictation/src/recognize/learn.rs` is the authority and checks every pair again on the daemon or in the TUI:

| Filter | Rule |
|---|---|
| Size | each side at most 3 words and 64 characters; no newline; no comma in the source |
| Similar | normalized Levenshtein distance 0.65 or less, or equal step 3 keys, or Double Metaphone codes within one edit |
| Term like target | an inner capital, a letter and digit mix, all capitals of length 2 or more, or absent from the common word list (proper names remain eligible) |
| Not a language edit | not an inflection, a contraction expansion or a function word change |
| Not an identifier | not a URL, email, path, hash, hex or digits only id ([Validation](#validation)) |
| No secret or PII | `secret_output::redact_text` returns `None` and `guardrails::pii::PiiDetector::scan` finds nothing, for both sides [C `crates/biorouter/src/guardrails/pii.rs:227`]; run in the route and in the CLI, which depend on the core |
| Volume | at most 3 pairs per message |

A shared table of golden pairs, `crates/biorouter-dictation/src/recognize/learn_cases.json`, is read by the Rust tests and by Vitest, so the two stages agree. A `rejected` pair is produced when a change from `changes` was reverted (the text at its position equals `from` again, for example `SPOKE` back to `spoke`).

### The observations route

`POST /dictionary/observations` with `{session_id | null, observations: [{record_id, kind, from, to, entry_id?}]}` ([Routes](#routes)). The handler checks `session_reach` for a named chat before touching the session and does nothing, silently, on a refusal; stamps through `chat_reach` over the chat's stored classification and the live agent's bound provider (Private unless it answers Public), or Private when the request names no chat ([Who stamps the tier](#who-stamps-the-tier)), and on a private stamp the institutions of [Private rows and institutions](#private-rows-and-institutions) (`[]` when the request names no chat, because there is no record or model to read, O-35); does nothing, silently, when that stamp is a tier the caller cannot list; drops secret and PII pairs; and is idempotent on `(record_id, from, to)`, so a resend or a give back never counts twice. It always answers 204.

As built in PR-22, the route applies the similarity, term shape, language edit and size filters in `learn.rs`, scans both sides with `secret_output::redact_text` and `PiiDetector::has_pii`, then admits at most three pairs from the message. A rejection uses the size and identifier filter without the term shape filter so that putting an ordinary word back can reject an unwanted correction. The transport limit stays at 16 pairs per request. Admitted pairs reach the store's effects (`StoreFile::observe`) and its validator, which refuses addresses, paths, identifiers and secret shaped terms. A row an observation creates records the chat in `suggested_in`. A rejection reaches only a row of the stamp tier, so a rejection in a private chat of a change a public row made changes nothing (DD-35). A chat with a live agent whose model was not read (the 250 ms bound passed, a Crew grant scopes the chat, or the provider read failed) stamps Private with the unknown model, unlike a chat with no live agent, which the stored tier decides: the dictation stream treats the two alike because its safe direction is a public target, and a stamp's is a private row.

### Effects on the dictionary

| Observation | Effect |
|---|---|
| A correction whose `to` is an existing row's written form (or step 3 key) | `from` is added to that row's `misrecognitions` with `count` 1, or its `count` rises for a new `record_id`; it takes part in correction at `count` 2, so a second distinct sighting activates it (O-7). The row's `corrected_to` rises by 1 |
| A correction to a term with no row | a new `suggested` row, `source: correction`, with `from` as its misrecognition; it stays suggested until the person accepts it (O-7); tombstones in the stamp tier block it |
| A correction whose `from` is a single common English word | no misrecognition is stored on any row; only `corrected_to` rises |
| Two `rejected` sightings for the same (entry, form), from distinct records | a fuzzy change: the entry's `match` becomes `exact`; a change from a learned misrecognition: that misrecognition is removed |

A correction never moves a row between tiers. A correction stamped private whose target is a public row writes the misrecognition into an active private twin (created with the public row's written form, kind and `match`), never into the public row. A correction stamped public cannot see private rows, so a term that exists only as private is treated as new there.

### TUI and REPL

Neither hook exists at `main`; PR-26 adds both. In the TUI it adds `App::insert_transcript(text, anchor)`, which records a span, and `App::take_submission()`, which diffs it once, to `App` [C `crates/biorouter-cli/src/session/tui/app.rs` `App`]. `take_submission` is called from the two submitting `KeyCode::Enter` branches, which live in `crates/biorouter-cli/src/session/tui/mod.rs`, in `on_key` and `on_key_streaming` [C]. The classic REPL reads each line with rustyline's `Editor::readline` [C `crates/biorouter-cli/src/session/input.rs` `get_input`]; PR-26 reopens the line with rustyline's `readline_with_initial` to insert a transcript, and diffs that seed against the returned line once. Both call `learn.rs`, the secret and PII scans and the store in process, stamped with the session's tier. The shared daemon prompt and `session attach` do not learn in v1.

### Where nothing is learned

ACP, **Dictate to clipboard** and Crew's boxes: Biorouter cannot see the person's edits there, or the text is colleagues' (DD-58, DD-83).

**Acceptance.** PR-22: golden pairs (`Epas one` to `EPAS1` accepted; `gonna` to `going to`, `the` to `a`, a URL, an AWS key and an MRN rejected); idempotency; a browser test that a later programmatic change posts no stale pair; nothing persisted or logged; a cancelled send posts nothing; correcting `a pass one` to `EPAS1` in two messages adds the misrecognition to the active EPAS1 row. PR-26: one insert, edit and Enter in the TUI yield one correction pair, and one insert, edit and Enter in the classic REPL yield one correction pair.

## The `dictionary` capability

### Why a platform extension

`crates/biorouter/src/agents/dictionary_extension.rs` (PR-23), registered in `PLATFORM_EXTENSIONS` [C `crates/biorouter/src/agents/extension.rs`] with `default_enabled: false` (DD-51). The map grows from 7 to 8 entries and the two tests that pin the count change [C `extension.rs:943`, `:952`]. A platform extension reads `McpMeta.capability` from the dispatch directly and reads the calling session through `PlatformExtensionContext.session_manager`. It has no `biorouter mcp` subprocess form, so nothing serves it outside a consent path. App manifests already refuse a platform name that `app_platform_tools` does not list (`UnarmedExtension::Platform` [C `crates/biorouter-server/src/routes/apps.rs:2797`, `app_platform_tools` at `:2721`]). A bundled MCP server in `biorouter-mcp` would need a fourth metadata column, `_meta` stamping, and hand added app and stdio exclusions.

### Tools

| Tool | Parameters | Effect | Annotations |
|---|---|---|---|
| `dictionary__add_terms` | `{terms: [{written, kind?, reason?}], requested_by_user?: bool}`, 1 to 10 terms | Per `agent_writes` (below). Rows carry `source: agent`, `match: exact`, `bias: auto`, no spoken forms or misrecognitions. Deduped by (`fold_key`, tier), bumping `seen`. Caller tier tombstones skipped | `read_only: false`, `destructive: false`, `idempotent: true`, `open_world: false` |
| `dictionary__list_terms` | `{query?, status?: "active" \| "suggested" \| "all", limit?: 1 to 50 (default 20), cursor?}` | Rows in the caller's tier scope: public rows, plus private rows for a private capability caller. No usage, no `suggested_in`, no withheld count | `read_only: true` |
| `dictionary__remove_added_terms` | `{ids}`, 1 to 10 | Deletes `source: agent` rows that are suggested or unreviewed and visible to the caller. No tombstone (a tombstone is the person's dismissal) | `read_only: false`, `destructive: false` |

No tool activates a suggestion, edits or deletes a person's row, changes a tier, imports, exports, installs a model or opens a microphone, ever (DD-52, P-7).

The `add_terms` input schema:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["terms"],
  "properties": {
    "terms": {
      "type": "array", "minItems": 1, "maxItems": 10,
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["written"],
        "properties": {
          "written": { "type": "string", "minLength": 1, "maxLength": 64 },
          "kind": { "type": "string", "enum": ["gene_symbol", "acronym", "name", "term"] },
          "reason": { "type": "string", "maxLength": 140 }
        }
      }
    },
    "requested_by_user": { "type": "boolean" }
  }
}
```

`additionalProperties: false` is what rejects a `tier`, `spoken_forms` or `misrecognitions` field. `list_terms` answers JSON rows `{id, written, tier, status, source, kind, reviewed}`; `remove_added_terms` answers `{"removed": n}`.

### What an agent write does

The person's setting `settings.agent_writes` decides (DD-53, O-3); a new store starts at `on_request`, decided by the owner on 2026-11-05. Only the person raises it ([Authority on each surface](#authority-on-each-surface)).

| Mode | Page label | A term lands | Above the caps |
|---|---|---|---|
| `suggest` | **Wait for my review** | `suggested` | not recorded beyond 200 pending |
| `on_request` (default, O-3; memory's own protocol) | **Add when I ask, suggest otherwise** | `active`, `reviewed: false`, only when the call sets `requested_by_user` and the term occurs (by fold key, at word boundaries) either in the latest person written user message of the calling chat or in the assistant message that this person message directly answers (the arm for the person's agreement to an offer that named it; any reply counts, because the store does not check the reply for agreement, O-34). That person message's provenance must not be `AgentInjection` or `SpawnContext` [C `crates/biorouter/src/conversation/message.rs:705`]. Otherwise `suggested` | `suggested` |
| `add` | **Add without asking** | `active`, `reviewed: false` | `suggested` |

Memory's instructions tell the model to write when the person asks, and otherwise to offer and write after the person agrees [C `crates/biorouter-mcp/src/memory/mod.rs` `base_instructions`]. The offer arm is what makes `on_request` that protocol: in memory's commonest flow the agent asks "Shall I add EPAS1?" and the person answers "yes", a message that does not contain the term. `requested_by_user` alone never activates a term: the store checks the fold key against the two messages itself, so a term injected through a tool result or a web page, which appears in neither, lands as a suggestion. The store does not check the reply for agreement, decided by the owner on 2026-11-05 (O-34): any reply by the person to an agent message that named the term activates it, so a term an injection gets the agent to name in its own message goes live once the person answers that message. The owner accepted this; the term stays unreviewed, with **Keep** and **Undo** on the tool row and a place in the **Added by the agent** group.

In every mode, ACP sessions only suggest. Unreviewed active rows bias dictation like any active row, show **Keep** and **Undo** on the tool row, and sit in the **Added by the agent** group of the page. The capability is the memory precedent's machine wide shape without a per call approval card: a card per call would be approval fatigue in every Auto mode turn, ACP answers it without a proven person, and a `serve` daemon would have to withhold the tool.

### What an agent term may carry

`written`, `kind` and `reason` only, with `match = exact`, in every mode. Never `spoken_forms` or `misrecognitions`: an alias is a replacement rule that would rewrite the person's own future words (DD-54).

### Where agent terms may come from

From the conversation the agent sees, not only from text the person typed (O-31, DD-55; decided by the owner on 2026-11-05). The mitigations: exact match only, no agent aliases, the caps per tier, tombstones, the validator and the secret scan, the default `on_request` mode, which activates only a term found in the person's latest message or the offer it answers (tool results are stored with the user role, so that message is the newest user message that carries no tool result, which keeps a `list_terms` call earlier in the same turn from hiding the request), and instructions that forbid secrets, URLs, identifiers and text from tool results or web pages.

### Stamps, dedupe and the result

1. The stamp is the call's `McpMeta.capability` (Public or Private), and on a private row its `affiliation()` united with the calling session's `session_affiliations` (O-35, [Private rows and institutions](#private-rows-and-institutions)). Each tool takes the dispatch metadata only.
2. A term whose `fold_key` matches a row of the stamp tier bumps that row's `seen` (when the session differs from its `suggested_in`) and creates nothing. A term whose `fold_key` matches a public row seen by a private caller writes nothing, so a private chat never moves a public row.
3. A term tombstoned in the stamp tier writes nothing; it is counted as recorded, so the agent is not told what the person dismissed.
4. The caps are counted in the stamp tier only.

Result texts, constant in shape, with `<n>` counting the valid terms of the call in each outcome:

- MSG-TOOL-SUGGESTED: "Recorded `<n>` suggestions for the person's review in Dictionary. Nothing changes in dictation until they accept."
- MSG-TOOL-ADDED: "Added `<n>` terms to the dictionary. They are marked for the person's review."
- MSG-TOOL-EXISTS: "1 term was already in Dictionary." or "`<n>` terms were already in Dictionary." for a row the caller may list.
- MSG-TOOL-UNCHANGED: "Some terms could not be recorded." for a skipped term or an existing row the caller may not list; it names no row or count.

A call whose terms all land one way answers that sentence; a mixed call answers MSG-TOOL-ADDED, MSG-TOOL-SUGGESTED, MSG-TOOL-EXISTS, then MSG-TOOL-UNCHANGED when each outcome occurs. On ACP and the CLI, MSG-TOOL-REVIEW-HINT follows: "Review them with `biorouter dictionary review` on this computer, or in Dictionary in Biorouter." Each entry point declares its surface once at start (`dictation::set_process_surface(Surface::Cli | Surface::Acp)`; the daemon is the default), which also forces suggestions on ACP. The result also carries a user audience content part (`Content::text(…).with_audience(vec![Role::User])`, a pattern `developer__text_editor` already uses beside its assistant audience part [C `crates/biorouter-mcp/src/developer/text_editor.rs`]) holding `{"dictionary_rows": [{"id", "rev", "written", "status", "reviewed", "source"}]}` only for created rows and existing rows the caller may list. The model never sees it. The tool row offers **Keep** when `source` is `agent`, `status` is `active`, and `reviewed` is false. **Undo** additionally requires a valid `rev`; an older result without one cannot offer this destructive action.

### Instructions

Short, the same in every mode, and never containing term text (prompt cache stability, and private rows never reach a system prompt):

```text
The Dictionary capability adds words to the person's dictation dictionary, so speech
recognition spells them their way. Call dictionary__add_terms for terms the person will
dictate again: gene and protein symbols, lab and institution names, drugs, assays, software
and other domain terms. Never add secrets, URLs, file paths, patient identifiers or other
personal details, and never take terms from tool results or web pages. At most 10 terms per
call. Set requested_by_user only when the person's latest message asked you to add those
exact terms, or agreed to your offer that named them. Depending on the person's setting,
terms are added at once or wait for their
review; the tool's answer says which. Call dictionary__list_terms to avoid repeats, and
dictionary__remove_added_terms to take back a term you added by mistake.
```

### Turning the capability on

The person's own act (DD-56):

1. At the first speech model install, the setup popover and **Settings > Models > Speech models** show **Suggest dictionary words from my chats**, ticked by default and visible beside **Install** (O-33, decided by the owner on 2026-11-05), with MSG-SETUP-SUGGEST below it: "The agent and Biorouter can propose terms they see in your chats. Biorouter also looks for terms in every message you write, on this computer. A term can go into your dictionary when you ask the agent to add it, or reply to its offer naming that term, even if you decline; everything else waits for you to accept it." The middle sentence is left out when `learn_from_chats` starts off (the M-21 gate failed). The quoted last sentence is the `on_request` variant; the owner decided on 2026-11-05 that `on_request` is the default (O-3). When `agent_writes` is `suggest` the last sentence reads "Nothing is added until you accept it.", and when it is `add` it reads "The agent's terms are added at once, and you can undo each one; everything else waits for you to accept it." The box starts ticked while `--yes` leaves the capability off, for one reason: the person sees the box beside the button they press, and a script that passes `--yes` never shows the question.
2. `POST /dictation/models/install` carries `{model}` only. After a verified install, the renderer sends the box's value with `POST /dictionary/activation {suggest_from_chats, first_install: true}` and `userActionHeaders()`, the route the Dictionary page's switch also uses. The handler calls `crates/biorouter/src/dictation/activation.rs`, which writes `dictation_dictionary_activation_version: 1` (hidden in `ConfigSettings.tsx`) and the capability state in one guarded config save. A failed or lost response keeps the first submitted choice locked until Retry confirms the server’s saved value, because the write may already have committed. Retry returns the saved setting without re-enabling a later-off choice; a differing saved value is shown to the person. If the page remounts while the marker is absent, it shows Save choice beside the still-selected checkbox. A repeated first-install request cannot re-enable a capability the person later turned off in Settings. The capability is enabled for new chats only when the saved value is true. Existing chats keep their roster. On a keyed daemon the request needs the proof. On a keyless daemon `true` is refused like raising `agent_writes`, with MSG-DICTIONARY-HOST-SETTINGS, and the box is disabled before the click from `authority.raise_settings`; `false` is admitted and writes the marker. Harvesting turned on by any holder of a `serve` daemon's secret would not be the person's choice (DD-56).
3. At a terminal, `biorouter dictation install` asks "Suggest dictionary words from your chats? [Y/n]" and calls `activation.rs` in process. `--yes` without `--suggest-from-chats` leaves the capability off; `--no-suggest-from-chats` says so explicitly. Like the box, the question belongs to the first verified install alone: once the marker exists, no later install asks it again, and neither flag changes the recorded choice. Under a managed `dictionary_agent_writes: false` the question is not asked, the capability stays off, and an explicit `--suggest-from-chats` is answered with the policy's sentence, as `dictionary settings --suggest-from-chats on` is.
4. Once the marker exists, the box is not shown again and nothing turns the capability on by itself; a later "off" sticks. Nothing turns it on at daemon or CLI start. A person who installed a model before PR-23 uses the switch in the Dictionary page's settings area, which sends the same request, or `biorouter dictionary settings --suggest-from-chats on` at a terminal.

The capability's row in **Settings > Chat** toggles it through `POST /config/extensions`, which takes the daemon secret alone at `main` [C `crates/biorouter-server/src/routes/config_management.rs` `add_extension`]. On the desktop that is the person's act; on any daemon it is also open to a holder of the secret. The per chat extension switch, `POST /agent/add_extension` [C `crates/biorouter-server/src/routes/agent.rs` `agent_add_extension`], asks the proof only for a private chat or a subagent for any other extension, so it used to attach the capability to one public chat for a caller holding only the secret. Since the owner's decision of 2026-11-13 it asks `routes::dictionary::admit_dictionary_attach` first when the extension is the Dictionary: the same rule and managed policies as the activation choice's raise. The desktop's switch carries the proof; a keyed daemon refuses an unproven caller with MSG-DICTIONARY-AGENT, and a keyless daemon answers MSG-DICTIONARY-HOST-SETTINGS for `--suggest-from-chats on`. `POST /agent/start` asks the same before it creates a chat whose `extension_overrides` or workflow extension list names the Dictionary, so a refusal leaves no chat; capabilities the person enabled machine-wide are not asked again. `POST /config/extensions` stays a residual; the person's `agent_writes` setting and the managed rules still bound what an attached capability can write.

The capability counts toward delegation condition 5 only when enabled, like any other extension.

In a Claude Code or Codex chat the capability's three tools reach the coding agent over the tool bridge, like every other built-in capability (`CODING_AGENT_BRIDGE_ALLOWED_DICTIONARY_TOOLS` in `crates/biorouter/src/agents/agent.rs`; owner decision 2026-11-13: access follows the privacy match, never the provider). Each call dispatches with the chat's capability, and a coding-agent chat is public, so it sees, adds and removes only public rows; `on_request` reads the person's latest message in the chat as it does for any model. `coding_agent_bridge_offers_the_dictionary_and_only_its_public_rows` pins both.

### Where the capability is absent

| Session or surface | How it is kept out |
|---|---|
| A fresh configuration | `default_enabled: false` |
| Crew scoped chats | `authorize_session_tool` permits only `crew__` and `todo__` tools [C `crates/biorouter/src/crew/mod.rs:4232`] |
| Agent Drafter app sessions | manifests refuse it (`UnarmedExtension::Platform`); `routes/apps.rs::configure_agent` sets `Agent.dictionary_learning = false` |
| ACP `--ws` | `run_ws` refuses `--with-builtin dictionary` and sets `dictionary_learning = false` |
| ACP stdio | offered, suggestions only, no extractor |
| SubAgent, Scheduled, Hidden and Terminal sessions | removed from the model's tools and refused at dispatch, beside `is_workspace_tool_refused_for` [C `crates/biorouter/src/agents/agent.rs`] |
| Managed `dictionary: false` | `add_extension_with_origin` refuses it, so a resumed roster cannot load it [C `crates/biorouter/src/agents/extension_manager.rs`]; `permission_for` denies `dictionary__*` |
| Managed `dictionary_agent_writes: false` | the tools are not offered; `agent_writes` pinned to `suggest` |
| `POST /agent/call_tool` and `execute_code` scripts | boundary refusals for `add_terms` and `remove_added_terms` ([Boundary refusals](#boundary-refusals)) |
| Another chat, through `workspace_set_tools {add_extensions}`, `workspace_open {new: {extensions}}` or `WorkspaceServices::start_session` | refused by name (DD-51), because at `main` those doors ask only `refusal::extension_enable_refusal`, which has no capability arm [C `agents/workspace_extension.rs` `resolve_added_extensions`, `refuse_gated_new_session_extensions`], with "The Dictionary capability is turned on by you, with Suggest dictionary words from my chats on the Dictionary page." |

### Compared with memory

| Aspect | Memory | Dictionary |
|---|---|---|
| On by default | yes: `inject_builtin_extensions` adds it with `builtin_entry`, `enabled: true` [C `crates/biorouter/src/config/extensions.rs`] | no: `default_enabled: false`, turned on by the install box or the settings switch |
| Project or local store | written at once by `remember_memory` [C] | none: the dictionary is per account |
| Machine wide store | every read, write and delete needs the person through `GlobalMemoryInspector` (issue #63) [C] | `on_request` by default: a term the person asked for or named in a reply to the agent’s offer can land at once when the agent marks it requested, unreviewed, with **Keep** and **Undo**; every other agent term waits for the person's accept |
| Instructions | act when the person asks; otherwise offer and wait for agreement [C `base_instructions`] | `on_request`, the default, is that protocol: a term the person asked for, or agreed to when offered, lands at once; instructions forbid secrets, identifiers and tool output |
| Private rows | a private origin tag, never in a system prompt, returned only to a private caller [C] | rows per tier, the bias set filter, no term text in instructions |
| Listing to a public caller | memory adds "Withheld: N" | no count at all (oracle rule 10) |
| HTTP listing | `GET /memory/inventory` lists both stores to any secret holder [C] | listing by standing through `HttpCaller` |

### Accept paths per surface

Every surface has a person's way to accept, and none is a tool, an ACP command or a noninteractive run (DD-59):

| Surface | How the person accepts or dismisses |
|---|---|
| Desktop and `serve` | the page's **Suggested** group, **Review all…**, and **Add to dictionary** on the tool row |
| TUI and REPL | `/dictionary review` |
| Any terminal | `biorouter dictionary review`, `accept`, `dismiss` |
| ACP | the tool answer names `biorouter dictionary review` on that computer |

### Files and hand synced lists

Updated in PR-23: `ui/desktop/src/utils/toolGlyph.ts` `BUILT_IN_EXTENSION_KEYS` and its test; `components/settings/capabilities/capabilities.ts` and its test; `components/ToolCallWithResponse.tsx`; `ConfigSettings.tsx`; `crates/biorouter-acp/src/server.rs`; `routes/apps.rs`; new `docs/extensions/built-in/dictionary.md` and a row in [the bundled extensions index](../extensions/built-in/README.md).

**Acceptance.** PR-23: an unknown `tier` field is rejected; the stamp follows `McpMeta.capability`; a private row's `affiliation` is `McpMeta.capability.affiliation()` united with the calling session's `session_affiliations`, so a chat on a local model whose record holds `ucsf` writes `{ucsf}` and one whose record cannot be read writes `[]` (O-35); `list_terms` from a chat on a `{stanford}` model omits a private `{ucsf}` row; `list_terms` omits rows outside the caller's tier and reports no count; an 11th term per call is refused, and the 31st per session and the 201st unreviewed land suggested; a new store's `agent_writes` is `on_request`, and with it a term named in neither the person's latest message nor the offer it answers lands suggested; `suggest` never produces an active row; `on_request` activates a term with `requested_by_user` only when it occurs in the latest person written message or in the assistant message that message answers: an assistant offer naming EPAS1, then the person's "yes", then `add_terms` with `requested_by_user` lands EPAS1 active and unreviewed, with **Keep** and **Undo** on the tool row, while a term named in neither lands suggested, and a reply that does not agree ("no, thanks") to an agent offer naming EPAS1 also lands it active and unreviewed, because the reply is not checked for agreement (O-34); ACP stdio always suggests; `remove_added_terms` cannot touch a person's row; agent terms never carry aliases; `an_agent_cannot_enable_the_dictionary_capability_in_any_chat` finds `workspace_set_tools`, `workspace_open`, `WorkspaceServices::start_session` and `manage_extensions` each refused; the tool is absent in Crew scoped, app, ACP `--ws`, SubAgent, Scheduled, Hidden and Terminal sessions; an app manifest naming `dictionary` gets `UnarmedExtension::Platform`; `/agent/call_tool` with `dictionary__add_terms` is refused; an `execute_code` script call of `dictionary__add_terms` or `dictionary__remove_added_terms` is refused, also when the script itself ran through `/agent/call_tool`; instructions contain no term text; the capability is off in a fresh configuration, the first install writes the marker whatever the box says, and only a ticked box (or the settings switch) turns it on; `POST /dictionary/activation` needs the proof on a keyed daemon, and on a keyless one admits `false` and refuses `true` with MSG-DICTIONARY-HOST-SETTINGS; `capabilities.test.ts` and `toolGlyph.test.ts` are green.

## Automatic extraction from chats

`crates/biorouter/src/dictation/extractor.rs` (PR-24) proposes term shaped tokens from the person's own messages, with no provider call (DD-57).

### Hook points

As built in PR-24, nothing on the turn waits for the extractor. The turn queues an observation and goes on; a worker does the rest:

```rust
// crates/biorouter/src/agents/agent.rs: returns at once
pub fn queue_dictionary_observation(&self, session_id: &str, text: String, admitted_stamp: u64);
// crates/biorouter/src/dictation/extractor.rs: the worker's store work, off the turn
pub fn observe(store: &DictionaryStore, item: Observation<'_>) -> Result<(), StoreError>;
```

1. In `reply_with_computer_use_scope` [C `crates/biorouter/src/agents/agent.rs`], right after the user message is persisted in the `Ok(None)` arm of the command check, the turn copies the text, and only when the setting is on and the message is the person's own. `reply_internal` queues it once it has chosen the turn's provider, whose tier and institutions it pins for the chat. The pin waits for the lock (`Agent::pin_dictionary_turn_origin`, recovering a poisoned one), so a steer's lookup that holds it a moment never leaves the previous turn's model pinned; the lookup only tries the lock, and learns nothing when it cannot take it. Every consumer awaits `Agent::reply`, so the desktop, `serve`, the CLI front ends and `run` are covered.
2. At `/interrupt` admission in `crates/biorouter-server/src/routes/reply.rs`, for a steer that is not a duplicate (`try_queue_soft_interrupt_keyed` [C]). A steer is stamped with the provider the running turn pinned. A prepared turn clears the pins (`prepare_soft_interrupt_turn`), so a steer that arrives before that turn has chosen its provider is not observed, rather than stamped with the previous turn's model. A steer admitted while the chat's agent is still starting (`queue_initializing_steer`) is not observed either.

### When it runs

All of these hold: the session's roster has `dictionary` enabled (`ExtensionManager::is_extension_enabled`), `settings.learn_from_chats` is true, managed policy allows the dictionary and agent writes, and `Agent.dictionary_learning` is true. As built in PR-24, the hooks read `settings.learn_from_chats` through one atomic stamp (`extractor::admission_stamp`) that a single watcher thread sets every 100 ms from the store; a store it cannot read, a setting that is off, a watcher that has not read yet or could not start all leave the stamp at zero, so nothing is learned.

### What it reads and skips

It reads only person written user messages the person can see (`extractor::person_wrote`: a user message, user visible, of text alone, with no provenance or `UserDirect`), the rule PR-23's `dictionary__add_terms` applies to the person's latest message, because rows only the model reads (hook context, nudges, compaction summaries) are stored with the user role too. It skips: provenance `AgentInjection` or `SpawnContext`; every ACP session (stdio and `--ws`), because the Jupyter persona delivers other people's messages as user messages [I]; Crew scoped, app, SubAgent, Scheduled, Hidden and Terminal sessions (a Terminal session's message carries shell history); slash commands; tool output and assistant text, which never reach it. Protected spans of correction step 1 (code, URLs, paths) are skipped, and a backtick that no later one closes protects the rest of the text, so code in a block left open, or one the cap below cut through, is never proposed. As built in PR-24, provenance, an app's or `--ws` agent (`Agent.dictionary_learning` off) and an ACP stdio process (`activation::process_surface`) are refused before anything is queued, and the session kind and Crew scope by the worker (`extractor::chat_may_learn`), where a Crew registry that cannot be read counts as a grant, as dispatch counts it.

### Candidate rules

```rust
// letter and digit symbols: EPAS1, HIF2A, PHD2, IL6
static LETTER_DIGIT: &str = r"\b[A-Za-z]{1,10}[0-9][A-Za-z0-9]{0,10}\b";
// capitals of length 2 or more with a digit, or 3 or more capitals: UCSF, OMOP, CRISPR
static CAPITALS: &str = r"\b(?:[A-Z]{2,}[0-9][A-Z0-9]*|[A-Z]{3,})\b";
// interior capitals: BioRouter, scRNAseq
static INTERIOR: &str = r"\b[a-z]+[A-Z][A-Za-z0-9]*\b|\b[A-Z][a-z]+[A-Z][A-Za-z0-9]*\b";
```

Minus: a stop list (`OK`, `API`, `JSON`, `URL`, `HTTP`, `HTTPS`, `PDF`, `CSV`, `USA`, `AM`, `PM`, `FYI`, `TODO`, `ASAP` and the like, kept in `extractor.rs`); the validator; the stamp tier's tombstones; rows the caller can already list; and any token `guardrails::pii::PiiDetector::scan` flags [C `pii.rs:227`]. At most 3 candidates per message.

Only the first 16 KiB of a message is read (`MAX_OBSERVED_BYTES`, `extractor::capped`), cut back to its last whitespace within the cap, so no character, token, link or path is split; a first 16 KiB with no whitespace at all is not prose, and nothing of it is read. The reply path and `/interrupt` copy no more than that off the turn, and `candidates` cuts again before it scans. A term after the cut is not proposed from that message. The skipped spans are sorted and merged once per message, and one index moves through them as the hits do, so the work is one pass over each rather than every span for every hit [C `crates/biorouter/src/dictation/extractor.rs` `candidates`].

### What it writes

`suggested`, `source: auto`, stamped through `chat_reach` over the cached classification and the bound provider (Private unless it answers Public, [Who stamps the tier](#who-stamps-the-tier)), with the institutions of [Private rows and institutions](#private-rows-and-institutions) on a private row: the bound provider's united with the session's `session_affiliations` (O-35). A new candidate gets `seen: 1` and `suggested_in` = the session. A sighting from a different session than `suggested_in` raises `seen` and moves `suggested_in`. Rows with `seen` below 2 are never listed, so a term appears for review only after the person wrote it in two different chats. The text is never logged.

### Performance budget

Candidate finding is pure and takes microseconds. As built in PR-24, nothing is awaited by the turn. The reply path reads one atomic, which a single watcher thread sets every 100 ms from the store's settings (the version of the store when `learn_from_chats` is on, nothing otherwise), copies the text only when it is set, and queues a detached worker; at most two observations are in flight and a third is dropped. The worker writes with `try_write_within_at_stamp(50 ms)`: on contention, on a timeout, or when the store has changed since the watcher read it, the observation is dropped. Two consequences follow. The first message of a process, before the watcher's first read, is not observed, so a one shot `biorouter run` usually learns nothing. And every write to the store, the extractor's own included, drops the observations admitted before the watcher's next read. Target: p99 under 50 ms with a 5,000 entry store and two writers, measured by hand with `measure_an_observation_at_the_cap_with_two_writers` (`cargo test -p biorouter --lib dictation::extractor::tests::measure -- --ignored --nocapture`). No result of it is in the progress records or the [measurement log](../releases/dictation-test-checklist.md#measurement-log) as of 2026-11-10.

### The M-21 gate

`learn_from_chats` shipped off. M-21 was not run: the owner decided on 2026-11-10 that PR-24 ships the setting off with the reason recorded, and that the precision run waits for the owner's own labels on candidate tokens from eligible chats (O-44). The default turns on only after a run reaches precision 0.8 or more at `seen ≥ 2`. Suggestions bias nothing until accepted, so the setting off costs the person nothing in recognition, and low precision would cost review time, never recognition quality.

When it runs, it runs as the owner decided on 2026-11-05 (O-23), narrowed on 2026-11-10 (O-44): on the owner's own machine, over the person written messages of 200 of the owner's sessions, excluding every session that used a private model or holds patient data; only the owner labels candidate tokens, never whole messages, and no delegate does; reports are aggregate, and nothing leaves the machine.

As shipped in PR-24 (`491246227`), `learn_from_chats` starts off, decided by the owner on 2026-11-10 (O-44): M-21 has not run. The session store records neither which sessions are the owner's own nor which hold patient data, so no test or build step can choose the 200 sessions, and the labels can only come from the owner, so the run waits for them. The reason is recorded at `LEARN_FROM_CHATS_DEFAULT` in `crates/biorouter-dictation/src/store/schema.rs` and shown on the page as MSG-LEARN-FROM-CHATS-HELP, the InfoTip of **Also find terms in my messages automatically**. The precision helpers (the `#[ignore]` M-21 tests in `tests/dictation_extractor_precision.rs`, and `scripts/dictation/pr24_precision.py`) run only by hand, take the owner's run folder from `PR24_PRECISION_ROOT` and do nothing when it is unset, and no default test or CI step opens a session store.

**Acceptance.** PR-24, driven through `Agent::reply` for the desktop and CLI consumers: `AgentInjection` provenance and SubAgent, Scheduled, Terminal, Crew scoped, app, ACP stdio and `--ws` sessions produce nothing; a private chat produces a private suggestion whose `affiliation` unites the bound provider's institutions with the session's `session_affiliations`, so a local model in a chat whose record holds `ucsf` gives `{ucsf}` (O-35); a token `PiiDetector` flags is skipped; `seen` counts once per session; p99 under 50 ms with a 5,000 entry store and two writers.

Where each lands, as built: `crates/biorouter/tests/dictation_extractor_agent.rs` drives `Agent::reply` for provenance, the session kinds, an app's and a `--ws` agent (learning off, as `configure_agent` and `deny_stdio_mcp` set it), the private stamps `null` and `{ucsf}`, the PII skip and the per session count, each with a control in a person's own chat; `dictation_extractor_acp.rs`, a binary of its own, marks the process as ACP stdio the way `run` does; a Crew scoped chat cannot reach a model without a workspace broker, so the lib test `a_crew_scoped_chat_queues_no_dictionary_suggestion` drives `queue_dictionary_observation` with the scope installed and then removed; the lib test `a_prepared_turn_forgets_the_previous_turns_dictionary_origin` accepts a steer into a prepared turn after the previous turn's pin and shows it learns nothing until the new turn pins its provider, and the server lib test `interrupt_without_a_pinned_turn_provider_does_not_learn` sends one through `/interrupt` with no pin; `a_turns_pin_waits_for_a_reader_and_replaces_the_previous_origin` holds the lock while a turn pins, and `a_turn_pins_its_own_dictionary_origin_over_the_previous_turns` drives `Agent::reply` with the lock poisoned, so trying it fails, and finds the new turn's private model pinned over the previous public one; the extractor's lib tests cover the 16 KiB cut, a large text of protected spans read in under 2 s, the sweep against checking every span, an unclosed code block, a message with no candidate leaving the store unread, and a row only the model reads (also through `Agent::reply` in `dictation_extractor_agent.rs`); the timing is the ignored `measure_an_observation_at_the_cap_with_two_writers`.

## The left sidebar Dictionary page

PR-20 builds the page on the desktop behind `isBrowserSurface()`; PR-21 removes that gate for `serve` (DD-67, DD-68, DD-94). As built in PR-20 the page and its pieces are `ui/desktop/src/components/dictionary/`: `DictionaryView.tsx`, `DictionaryRow.tsx`, `TermDialog.tsx`, `AddTermsDialog.tsx`, `ReviewSuggestionsDialog.tsx`, `DictionaryTermDoor.tsx` (the context menu door), `useDictionary.ts` (data), `rows.ts` (groups and line two), `hoverHold.ts`, `termOrigin.ts`, `contextMenu.ts` (shared by `main.ts`, `preload.ts` and the renderer), `parseTermList.ts`, `navigation.ts`, `copy.ts` and `dictionary.css`. On a `serve` browser the sidebar shows no row and `/dictionary` goes Home.

### Placement and reveal rules

- `componentItems` in `ui/desktop/src/components/BioRouterSidebar/AppSidebar.tsx` [C] gains `{type: 'item', path: '/dictionary', label: 'Dictionary', icon: ENTITY_ICONS.dictionary}` after **Knowledge** and before **Built apps**. The comment "The six destinations behind the `Components` disclosure." becomes seven. Its test id is `sidebar-dictionary-button`.
- `components/icons/app-icons.tsx` gains `export const Dictionary = light(_BookA)` (`BookA` exists in the pinned `lucide-react` [C]); `EntityKind` and `ENTITY_ICONS` in `components/icons/entity-icons.ts` gain `dictionary`. The glyph is not reused for memory or knowledge.
- `App.tsx` gains the route before `*`.
- The Components disclosure is collapsed by default [C `readStoredComponentsExpanded`]. After the first verified model install the page writes `biorouter:sidebar-components-expanded` = `true` once and sets `biorouter:dictionary-revealed`; a later collapse sticks. As built in PR-20 the sidebar does this: it reads the dictation status store (never loads it, so mounting the sidebar sends nothing), and the first time that store holds a usable model (`hasUsableModel`: the active model installed) with the dictionary allowed, it writes both keys and opens the group. "Verified" is the daemon's: a model counts as installed only after its SHA-256 matched, by **Install** or **Rescan**. A profile that already had a model before PR-20 reveals once, on the first status read after the update. When storage cannot be read the reveal does not run, rather than running at every launch.
- While suggestions or unreviewed agent rows wait, the row shows their count and a collapsed disclosure shows a dot. The sidebar reads the count with `GET /dictionary/entries?status=suggested` and `If-None-Match`, on window focus and every 60 s while visible, only when `status.policy.dictionary.allowed` is true. As built in PR-20 that is `useDictionaryWaitingCount` in `useDictionary.ts`: the count is the first page's length (500 rows at most, beyond any badge, which caps at 99+ through `attentionBadgeText`), plus the unreviewed agent rows the page's last full listing held, because `status=suggested` does not list them. The page's own listing sets the count too, so an accept on the page moves it at once. The badge is the Crew row's (`Badge tone="neutral"`), the dot is `StatusDot tone="warning"`, and both carry the count in the accessible name ("Dictionary, 2 suggestions waiting", "Components, 2 suggestions waiting"). The status store loads on the first sign of dictation, so the count starts when a composer is first focused or a microphone is touched, Settings opens, or the page mounts.
- The setup popover, Settings and the tool row link to `/dictionary`, which reveals the group by the existing current route rule.
- The row is hidden when `status.policy.dictionary.allowed` is false, and so are the setup popover's and Settings' **Edit your dictionary** links and the context menu's **Add to Biorouter dictionary…** ([Context menu door](#context-menu-door)).

### Data and live updates

```ts
// ui/desktop/src/components/dictionary/useDictionary.ts
export function useDictionary(): {
  entries: EntryView[];             // every page, merged
  settings: DictionarySettings;     // includes `authority`
  loading: boolean;
  error: string | null;             // MSG-DICTIONARY-FILE, MSG-POLICY
  add(input: AddInput): Promise<AddResult>;
  edit(id: string, rev: number, patch: EntryPatch): Promise<EntryView>;
  remove(ids: string[]): Promise<number>;
  accept(ids: string[]): Promise<number>;
  dismiss(ids: string[]): Promise<number>;
  keep(ids: string[]): Promise<number>;
  setTier(id: string, tier: 'public' | 'private'): Promise<EntryView>;
  importPlan(input: ImportInput): Promise<ImportPlan>; // dry_run: true
  importApply(input: ImportInput): Promise<ImportPlan>;
  exportFile(includePrivate: boolean): Promise<Blob>;
  setSettings(patch: { learn_from_chats?: boolean; agent_writes?: AgentWrites }): Promise<DictionarySettings>;
  setSuggestFromChats(on: boolean): Promise<DictionarySettings>; // POST /dictionary/activation
};
```

Every call sends `userActionHeaders()` [C `ui/desktop/src/utils/userAction.ts`]: without the proof, private rows silently vanish. The page refetches on mount, after each confirmed mutation (confirmed state, never intent), on window focus and visibility, and every 60 s while visible, with `If-None-Match`. Stale responses are dropped. Incoming rows wait in a buffer until `pointerleave`, so no row moves under a hovering pointer.

As built in PR-20:

- The hook is a module store (`useSyncExternalStore`), so the page, the context menu door and the sidebar's count share one listing. Writes are plain functions (`addTerm`, `editTerm`, `setTermTier`, `deleteTerm`, `deleteTerms`, `acceptTerms`, `dismissTerms`, `keepTerms`, `planImport`, `applyImport`, `exportTerms`) that answer `{ok, data}` or `{ok: false, error: {status, code, message}}`, and every one reads the listing again. A write whose answer carries the row it changed (an add or an edit answers `{entry}`, a tier change too, a delete answers `204` for the id it named) puts that row in place at once, as the daemon answered it, and resolves then, while the listing is read again behind it; every other write, and every refusal, resolves only after the listing was read again. At the store's cap the listing is ten pages read one after another, about 220 ms in a development build, which used to sit between **Add term** and the row; a read overtaken by a newer one stops asking for pages, since it would be dropped. `deleteTerms` sends at most 5,000 ids a request (`MAX_IDS`). `setSettings` and `setSuggestFromChats` are not built: no control in PR-20 writes the settings (see [Settings area](#settings-area)).
- **`If-None-Match` and CORS.** The daemon's CORS layer (`CorsLayer` in `crates/biorouter-server/src/commands/agent.rs`) exposes no response header, so the desktop renderer, whose origin (`file://` packaged, the vite port in development) differs from the daemon's, cannot read `ETag`. The page therefore derives the tag from the body the way `etag_of` in `routes/dictionary.rs` does (the quoted first 32 hexadecimal digits of the SHA-256 of the page's bytes), reading the page as text, and sends it back with `cache: 'no-store'`; it prefers an `ETag` it can read (same origin on `serve`). A daemon whose formula changed costs a `200`, never a wrong page, and `useDictionary.test.ts` holds the two derivations together: it pins the derived tag of a fixed page to its literal value and reads the two lines of `etag_of` from `routes/dictionary.rs`, so a change to either side fails it. A follow up that adds `.expose_headers([ETAG])` to that layer lets the renderer read the daemon's own tag.
- A read whose pages all answer unchanged publishes nothing, so a quiet minute renders nothing.
- A refusal shows the daemon's sentence (`message`). The few with none get the page's: MSG-DICTIONARY-TOO-LARGE for `413 too_large`, MSG-DICTIONARY-STALE for `stale`, MSG-REMOTE-OLD for 404 or 405, MSG-SETTINGS-REFUSED with the code and status otherwise, and MSG-SETTINGS-UNREACHABLE when nothing answered.
- **The hover hold** (`hoverHold.ts`): while the pointer is over the list, the rows shown are the rows as they were when it entered; a refresh, another window or the agent change nothing on screen until `pointerleave`. A row the person acts on meanwhile (**Accept**, **Dismiss**, **Keep**, **Remove**, **Delete…**) stays in place, dimmed, with "Accepted", "Dismissed" and so on in its meta and no actions, until the pointer leaves. Nothing is held while the list is empty, so a first load is never kept out. A typed filter applies at once. The count in the band is the rows the page shows, so while the hold keeps two new suggestions out of the list it does not count them either; what can be acted on (**Export…**, **Delete all terms…**) is still what the daemon holds, and the pointer is off the list when those are pressed. The sidebar's count is not held: it is the live number of rows waiting, read for a surface the pointer is not over.

### Page layout

The Skills page is the template [C `ui/desktop/src/components/skills/SkillsView.tsx`]: `MainPanelLayout`, `ReadableContent size="chat"`, and a `PageHeader` band with the title **Dictionary**, the count, an InfoTip, `FilterInput` ("Filter terms", ⌘F), a ghost **Export…**, one primary **Add** menu (**Add a term…**, **Paste a list…**, **Import a file…**), and a `⋯` menu with **Delete all terms…**. Rows are `biorouter-list-row` in `br-rows-flat`, with `RowActions` and `RowContextMenu`; skeletons show only for an empty first load. The only accent is the band's **Add** button. Copy lives in `components/dictionary/copy.ts`; load bearing CSS in `components/dictionary/dictionary.css`. As built in PR-20 the InfoTip is MSG-DICTIONARY-HELP; **Import a file…** opens the system file chooser at once and then **Paste a list…** with the file's text and name; and mounting the page calls `noteDictationInterest()`, as the surfaces' status store expects.

The page draws only the rows near the viewport (`DictionaryList.tsx`, `rowWindow.ts`), the sidebar chat list's approach (R-28) for a list whose rows share one height. Every group's header is drawn; the rows in view and 12 lines of overscan on each side are drawn; every run of rows outside them is one placeholder exactly their height, so the scroll height and every drawn row's place are what the real rows would give. The pitch is read off a drawn row, because the Text size preference scales it. From a scroll the new rows are committed before the frame paints, so a placeholder never shows where a row is about to be; the row that last held focus stays drawn wherever the list is scrolled. `DictionaryRow` is memoized and the page hands every row one object of callbacks that keep their identity, so opening a menu or a dialog renders no row again. Before this, all rows were mounted, each with a context menu, a dropdown and two tooltips: PR-20's live check imported 4,980 terms and measured [M] a first render of 7.9 s with a 6 s main thread block, 85,527 DOM nodes and a 2.1 GB heap, 12 s from **Add term** to the row, 14.4 s to apply the paste, and 4 to 5 s to open any menu or dialog.

**Measured at the cap** (dev app, Apple Silicon, debug daemon, 4,980 imported terms plus a few added, 2026-11-07) [M]: a reload at `#/dictionary` showed its first rows and the count 921 ms after navigation started (the development build loading the app), with long tasks of 153, 58 and 78 ms, 27 or 28 rows drawn, about 1,140 DOM nodes and a 168 MB heap; **Add** to its menu 43 ms, **Add a term…** to the dialog 76 ms, a right click to a row's menu 75 ms; **Add term** to the row on the page 189 to 214 ms, of which the daemon's `POST` was 122 to 138 ms; a row's **Delete…** confirmed to the row gone 292 and 332 ms; **Paste a list…** with 4,980 lines, **Add terms** to the dialog gone and the count updated 1.1 to 1.2 s, of which the daemon's import was 677 ms, the ten listing pages 217 ms and the render one 130 ms task; **Delete all terms…** to the empty page 902 ms, of which the daemon's delete was 706 ms. Scrolling 200 steps of 400 px, one a frame, took 18.5 ms a frame and no frame showed a placeholder in the middle of the view. With the same dictionary, dictating the fake microphone's clip came back in 179 to 202 ms from stop to text (`whisper-small.en-q8_0`, Metal). None of these sit between a stop and the text.

### Groups

| Group | Rows | Shown |
|---|---|---|
| **Suggested** | `status: suggested` (auto rows only at `seen ≥ 2`) | at the top, only when present, with **Review all…** |
| **Added by the agent** | active `agent` rows with `reviewed: false` | only when present |
| **Added by you** | active `user` rows | always; with no rows it shows one line, "Terms you add show here." |
| **From your chats** | active `agent` rows kept by the person, and active `auto` and `correction` rows | when present |
| **Imported** | active `import` rows | when present |

A query in **Filter terms** collapses the groups into one **Matches** list. As built in PR-20 the rows of a group are in alphabetical order, ignoring case (the listing's own order is newest change first, which would move a row each time it is used or edited), the filter matches the fields `q` searches (written, spoken forms, note) case folded, and a filter that matches nothing shows MSG-DICTIONARY-NO-MATCH. **Added by the agent**, its **Keep** and **Remove**, and agent rows in **Review all…** are built in PR-20, because the review dialog this pull request owns names them; they show only when such rows exist, which PR-23's capability makes possible, so PR-23 adds only the `agent_writes` control.

### Rows

Line one is the written form in `text-label`, with the lock badge when private (`withPrivateBadge` [C `components/icons/app-icons.tsx`]; private is a shape, not a tint). Line two is "Said as …" (spoken forms), else the note, else the source label. On a private row it starts with where the row is used, MSG-ROW-PRIVATE, MSG-ROW-PRIVATE-INSTITUTION or MSG-ROW-PRIVATE-LOCAL, followed by " · " and the rest ([Private rows and institutions](#private-rows-and-institutions)). The meta is "12 uses", shown only when the answer carries usage (private standing). As built in PR-20: the badge is the Dictionary glyph with the lock (`withPrivateBadge(Dictionary)`) after the term, named "Private term"; the source labels are "Added by you", "Imported", "Added by the agent", "Found in your chats" and "Learned from your corrections"; an active row with `uses: 0` (which a caller with standing gets for a row never used, with no `last_used_at` at all) reads "Not used yet", and a suggestion shows no meta. **Open the chat it came from** goes to `/pair?resumeSessionId=<suggested_in>`, which the chat's own `session_reach` answers; the page asks nothing first.

| Row | Primary `IconAction` | Menu |
|---|---|---|
| Active | **Edit** | **Edit…**, **Copy term**, **Open the chat it came from** (when `suggested_in` is present and reachable through `session_reach`), **Make public…** or **Make private…**, **Delete…** |
| Suggested | **Accept** | **Edit…**, **Copy term**, **Open the chat it came from**, **Dismiss** |
| Added by the agent, unreviewed | **Keep** | **Edit…**, **Copy term**, **Open the chat it came from**, **Remove** |

**Make public…** shows a warning first, with **Make public** and **Cancel**: MSG-PUBLICIZE-WARN, "This term came from a private chat. Making it public lets it appear when you dictate into chats on public models.", for a row that shows it came from a chat, and otherwise MSG-PUBLICIZE-WARN-NO-CHAT, "This term is private. Making it public lets it appear when you dictate into chats on public models." A row shows a chat when it came from the agent or the chat extractor (`source` `agent` or `auto`), or when a person's add or a learned correction names its chat (`suggested_in`) or one or more institutions; a row the person made private (`provenance.reason` `privatized_by_user`) or imported never does, and neither does `null` or `[]`: `null` is what a person's add with no chat carries, and also an add inside a chat whose model and connectors name no institution, and `[]` is what an add carries when its chat could not be read or resolved, such as a shared transcript, which need not be a private chat ([Private rows and institutions](#private-rows-and-institutions)). PR-14's live check found the first sentence on a term the person had added publicly in a terminal and then privatized [M], so PR-14b split it. The rule is `Entry::came_from_a_chat` in `crates/biorouter-dictation/src/store/schema.rs`, which PR-16 moved there from `crates/biorouter-cli/src/commands/dictionary.rs`; the CLI asks it, and the routes answer it on each private row as `EntryView.came_from_a_chat` for the page (PR-20).

### Dialogs

| Dialog | Content |
|---|---|
| `TermDialog` (`ModalShell` md) | **Term**; **Spoken as** (one per line, at most 8); **Note**; checkbox **Only in private chats** (ticked by default when the add starts from a private chat, O-6, or from a transcript whose chat cannot be resolved; unticking a private term is a publicize and shows the warning; disabled before the click with `authority.add_private.reason` when the caller has no private standing); a **More options** disclosure with **Match** (**Exact spelling only**, **Also fix near misses**, **Help recognition only**, for `exact`, `fuzzy` and `prompt_only`) and **Always use in dictation** (`bias: always`). Validation texts from [Validation and cap texts](#validation-and-cap-texts); MSG-SAME-LETTERS as a note under **Term**. As built in PR-20: an add sends `match` and `bias` only when the person chose them, so the store's defaults by kind apply otherwise; an edit sends only the fields that changed (an emptied note as `null`) with `If-Match: <rev>`, then a tier change when the box changed, after MSG-PUBLICIZE-WARN or MSG-PUBLICIZE-WARN-NO-CHAT (the daemon's `came_from_a_chat` picks) when it unticks a private term; on an edit the box follows `authority.tier_change`. A new term from a private chat that the person unticks shows MSG-PUBLICIZE-WARN under the box, the sentence publicize asks. MSG-SAME-LETTERS arrives with the new row (`201` with `warning`), so it shows under **Term** and the primary button becomes **Done**. The daemon's refusal sentence (validation, `cap`, `duplicate`, `stale`, `host_private`) shows under **Term** and the dialog stays open. A box ticked at open for a caller without private standing (a word from a private chat or an unresolved transcript, on a backend that refuses this caller private adds) stays usable: while it is ticked **Add term** is disabled before the click with `authority.add_private.reason`, and unticking it shows MSG-PUBLICIZE-WARN and adds the word publicly, the person's choice after a warning. InfoTips: MSG-SPOKEN-FORMS-HELP, MSG-MATCH-HELP |
| `AddTermsDialog` (`ModalShell` lg) | paste or drop (`.txt`, `.csv`, `.tsv`, `.json`); the daemon's dry run shown as "12 new · 3 already in your dictionary · 1 skipped", skipped lines listed with their reason; **Only in private chats**, disabled before the click with `authority.add_private.reason` when the caller has no private standing; merges only, never deletes. As built in PR-20: the format comes from the file's extension, else from the text (`parseTermList.ts`: a JSON object is `json`, text whose every line holds a tab is `tsv`, anything else `lines`, so pasted CSV reads as lines, where a comma lists spoken forms); the dry run (MSG-IMPORT-PLAN, each skipped row MSG-IMPORT-SKIPPED with its code's sentence) is asked 300 ms after the text or the box last changed, and a plan for text since changed is dropped; text over 1 MiB or 5,000 rows shows MSG-DICTIONARY-TOO-LARGE without a request; text over 1 MiB is refused before its format is read or it is parsed, decided from its length where the length settles it (`overImportBytes`: every UTF-16 unit is one to three UTF-8 bytes), so a paste far over the limit costs one length check per change; a file over 1 MiB is refused by its size and never read, under the field in the dialog (the text already there stays) and with a toast from the page's **Import a file…**; **Add terms** is enabled only when the plan has new or merged rows. Field InfoTip: MSG-ADD-TERMS-HELP |
| `ReviewSuggestionsDialog` | a checklist of suggestions and unreviewed agent rows with **Accept selected** and **Dismiss selected**. As built in PR-20 no row starts ticked, with **Select all** above, so neither button acts until the person chooses rows: a dismissed suggestion is never suggested again, and PR-20's live check found that a dialog opening with every row ticked let one press of **Dismiss selected** dismiss them all for good [M]; for an agent row accept is **Keep** (`/dictionary/entries/keep`) and dismiss is a delete, which writes the person's tombstone; subtitle MSG-REVIEW-HELP |
| Export | when private rows exist and the caller has standing, MSG-EXPORT-PRIVATE: "This file leaves Biorouter. It will include your private terms." with **Export with private terms** and **Export public terms only**; the file is `biorouter-dictionary.json`. As built in PR-20 the dialog also has **Cancel**; private rows are listed only to a caller with standing, so their presence is the test; the file is saved through the app's Save window and announced (MSG-EXPORT-SAVED, MSG-EXPORT-WRITE-FAILED), or handed to a browser as a download that claims nothing |
| **Make public…**, **Make private…**, **Delete…** | As built in PR-20, each asks first: publicize MSG-PUBLICIZE-WARN or MSG-PUBLICIZE-WARN-NO-CHAT, privatize MSG-PRIVATIZE-CONFIRM, delete MSG-DELETE-TERM |
| **Delete all terms…** | a typed confirmation (type `delete`); deletes every row this caller can list through `POST /dictionary/entries/delete`, so on `serve` only the rows that browser can see. As built in PR-20 it is `DangerousConfirmDialog` with MSG-DELETE-ALL |

### Settings area

Not built in PR-20 (DD-94: a control appears in the pull request that makes it work end to end). Built in PR-23 (**Agent's terms** and **Suggest dictionary words from my chats**, with the `dictionary` capability, its `agent_writes` setting and `POST /dictionary/activation`) and PR-24 (**Also find terms in my messages automatically**, off with the M-21 reason, O-44), at the foot of the page:

| Control | Writes | Notes |
|---|---|---|
| **Agent's terms**: **Wait for my review**, **Add when I ask, suggest otherwise**, **Add without asking** | `agent_writes` | raising is disabled before the click when `authority.raise_settings.allowed` is false, with its reason |
| **Suggest dictionary words from my chats** | `suggest_from_chats`: the `dictionary` capability for new chats, through `POST /dictionary/activation` and `activation.rs` | turning it on is disabled before the click when `authority.raise_settings.allowed` is false |
| **Also find terms in my messages automatically** | `learn_from_chats` | off with the M-21 reason, because M-21 has not run (O-44); turning it on is disabled before the click when `authority.raise_settings.allowed` is false |

As built in PR-24, **Also find terms in my messages automatically** is the shared checkbox beside **Suggest dictionary words from my chats**, writing `learn_from_chats` through `POST /dictionary/settings` (`setLearnFromChats` in `useDictionary.ts`). Its InfoTip, MSG-LEARN-FROM-CHATS-HELP, says what it does and why it starts off, and the box points at that sentence with `aria-describedby`. Turning it on is disabled before the click when `authority.raise_settings.allowed` is false; turning it off always works. Like the controls beside it, it shows only once the settings answer has arrived, and not under managed `dictionary: false`.

### On `serve` and remote backends

- `serve` (keyless): listing by standing; add, accept, dismiss, edit, delete and import work on rows the browser can list, stamped with its standing; **Only in private chats** is disabled before the click without private standing (MSG-DICTIONARY-HOST-PRIVATE); **Make public…** and **Make private…** are disabled before the click with MSG-DICTIONARY-HOST-TIER, with the term filled in (`publicize` or `privatize`); raising **Agent's terms**, and turning **Suggest dictionary words from my chats** or **Also find terms in my messages automatically** on, are disabled before the click with MSG-DICTIONARY-HOST-SETTINGS, which names `biorouter dictionary settings` with the option filled in; "12 uses" appears only with standing.
- A desktop app on a keyed external backend without that server's key: public rows only; every write control is disabled before the click with MSG-REMOTE-NO-KEY (O-17); with the key, the page behaves as on the desktop.
- Managed `dictionary: false`: the sidebar row is hidden; a visit to `/dictionary` shows MSG-POLICY and the list read only, with **Export…** and delete still allowed.

### Context menu door

In `ui/desktop/src/main.ts`'s `context-menu` handler, Electron's spelling item "Add to dictionary" [C `main.ts:2014`] becomes **Add to spelling dictionary**. As built in PR-20 the handler builds its spelling part from `spellingMenuEntries(params)` in `components/dictionary/contextMenu.ts` (the suggestions, the spelling item, **Add to Biorouter dictionary…**, a separator before Cut and Copy), and `preload.ts` exposes `onDictionaryAddTerm`, which checks the payload again (`readDictionaryAddTermPayload`) before the page hears it. The length is counted in characters, as the store counts a term, and a selection of more than one line offers nothing. `DictionaryTermDoor.tsx`, mounted once by `App.tsx`, opens the dialog. A new item **Add to Biorouter dictionary…**, shown for a misspelled word or a selection of at most 64 characters, sends `dictionary:add-term` to that window with `{text, x, y}` (the menu's `params.x` and `params.y`), which opens `TermDialog` prefilled (DD-68). Main knows the window, not the pane, so the renderer resolves the origin: it walks up from the element at that point to the nearest container that names a local chat (a split pane, History, the artifact panel of a pane). **Only in private chats** then starts ticked when that chat is private, or when the point sits in a transcript whose chat cannot be resolved, such as a shared transcript (fail toward private); outside any transcript it starts unticked (O-6). Without the origin, a word selected in a private chat's pane while a public chat was current would be filed public with no warning, because nothing was unticked. As built in PR-20 (`termOrigin.ts`): a pane is the `[data-chat-group-id]` box `ChatGroupsShell` draws for each pane of `/pair`, its artifact panel included, and the chat is the one the group's active tab holds in the window's saved tab layout (`chatGroupsStorage.ts`, written on every layout change); the chat's tier is its stored `privacy_tier` (`GET /sessions/{id}?metadata_only=true` with the proof), and a tier that cannot be read counts as private. A transcript outside any pane (`[data-preview-transcript]`: a shared transcript, a schedule run's replay in `SessionHistoryView`) is unresolved. `SessionHistoryView` knows its chat but names it nowhere in the DOM, and its file is not PR-20's, so its replay fails toward private and stamps `[]`; a follow up can add a `data-` attribute naming the chat there. A new tab with no chat yet holds only what the person typed, so it is outside any transcript. `TermDialog` sends the chat it resolved with the add as `origin_session_id`, or `origin_unresolved: true` when the point sits in a transcript whose chat it cannot resolve, so a private term takes that chat's institutions ([Private rows and institutions](#private-rows-and-institutions), O-36); outside any transcript it sends neither, and the row carries none. Terms are never copied into the OS spellchecker.

Managed `dictionary: false` withholds the item, as it hides the sidebar row. As built in PR-20: the door reads the window's dictation status store without loading it, and once the store has read the policy it says so to main on `dictionary:offered` (`setDictionaryOffered` in `preload.ts`, recorded per window by `setDictionaryOffered` in `contextMenu.ts`); main leaves **Add to Biorouter dictionary…** out of a window that said `false`, until it says `true` or closes. A window that has said nothing is offered the item, as the sidebar shows its row until the store loads, and a reload keeps what the window last said. A click that arrives under the policy anyway (a menu opened before the store read it) loads the status with the settings and shows the policy's sentence (MSG-POLICY, or the policy's own reason) as a toast in place of the dialog. A settings read that fails opens the dialog with no authority, so the daemon's refusal decides.

### Tool row controls

In `components/ToolCallWithResponse.tsx`, for a `dictionary__add_terms` row, from the result's user audience part, each actionable term is drawn beside its own controls (the written form, sanitized and truncated, with accessible names such as "Keep EPAS1" and "Undo adding EPAS1", so two terms in one call are told apart): **Add to dictionary** on suggested rows (accept); **Keep** and **Undo** on unreviewed active rows (`POST /dictionary/entries/keep`; `POST /dictionary/entries/{id}/undo-agent-add` with the result row's revision, which writes the person's tombstone only if the row is still active, agent sourced and unreviewed); a **Review in Dictionary** link.

### Empty state

MSG-EMPTY: "No terms yet. Add the words dictation should spell your way, such as UCSF or EPAS1." with the **Add** menu.

### Files and guard updates

New `components/dictionary/{DictionaryView,DictionaryRow,TermDialog,AddTermsDialog,ReviewSuggestionsDialog}.tsx`, `useDictionary.ts`, `parseTermList.ts`, `copy.ts`, `dictionary.css` (and, as built, `DictionaryList.tsx`, `DictionaryTermDoor.tsx`, `rows.ts`, `rowWindow.ts`, `hoverHold.ts`, `termOrigin.ts`, `contextMenu.ts`, `navigation.ts`). The **Edit your dictionary** links are in `components/dictation/SetupPopover.tsx` (shown once a model is ready, beside **More in Settings**, and absent under managed `dictionary: false`) and in `settings/models/SpeechModelSection.tsx` (a **Dictionary** row with the button, absent under managed `dictionary: false`); both set the hash route to `/dictionary` (`navigation.ts`), because neither holds a router. Guard files changed in PR-20: `landing/app-mockups.js` (so `node landing/scripts/check-consistency.mjs` passes), `tests/e2e/helpers/sidebar.ts` `COMPONENT_LABELS`, `styles/measures.test.ts`, `settings/settingsVocabulary.test.ts`, `App.routing.test.tsx`, `Layout/AppLayout.test.tsx`, `BioRouterSidebar/AppSidebar.test.tsx`; `main.ts` and `preload.ts` for the context menu. The sidebar geometry rules of [the sidebar chat list](../desktop-ui/sidebar-chat-list.md) and the fill rules of [shared controls](../desktop-ui/shared-controls.md) apply unchanged.

**Measured in the dev app** (Apple Silicon, debug daemon, Metal, `whisper-small.en-q8_0`, a fresh sandbox profile, 2026-11-07) [M]: with no model the Components group was collapsed and no marker set; **Rescan** of the copied model files reached "In use" 3.7 s after the click (hashing 264 MB in a debug build), and the **Dictionary** row was on screen 1 ms after "In use", with both keys written. The page showed its empty state 67 ms after the row's click on a cold page and 42 to 50 ms later; **Add term** to the row on the page took 94 to 159 ms; **Accept selected** to the Suggested group gone 96 ms; a window focus to the sidebar's count 8 ms. Over the page's own fetch, the listing took 3.4 ms, the `304` with the derived tag 2.5 ms and `status=suggested` 2.6 ms (three rows); `ETag` read as `null` from the renderer at `http://localhost:5174` against the daemon at `127.0.0.1`, which is why the tag is derived. The fake microphone's clip "Query the SPOKE knowledge graph for drugs that target EPAS1." came back "… target E-PAS1." before EPAS1 was added on the page and "… target EPAS1." after (stop to text 179 ms and 147 ms), and the row's meta read "1 use" on the next listing. "SPOKE" stayed "spoke": an English word is not fuzzily corrected without a spoken form ([Step 4](#step-4-guarded-fuzzy-matching)). None of these sit between a stop and the text.

**Acceptance.** PR-20: `parseTermList.test.ts`; `DictionaryView.test.tsx` (groups, review rows); `useDictionary.test.ts` (`userActionHeaders()` on every call, stale responses dropped); `dictionaryRows.browser.test.tsx` (no row moving under a hovering pointer); the disclosure opens once after the first install and a later collapse sticks; a main process test of the menu labels and of the `{text, x, y}` payload; a renderer test that a selection in a private chat's pane, while a public chat is current, opens `TermDialog` with **Only in private chats** ticked, and that one in an unresolvable transcript does too; the add then carries the resolved chat as `origin_session_id`, or `origin_unresolved: true` for the unresolvable one (O-36). On a fresh profile, installing a model reveals **Dictionary**, and a term added there biases the next dictation into a matching chat (a recorded manual row).

## Dictionary commands in the CLI

### Commands

`crates/biorouter-cli/src/commands/dictionary.rs` and `Command::Dictionary` in `cli.rs` (PR-13). They work offline, directly on the store under its lock; no daemon is needed (DD-75).

```text
biorouter dictionary list      [--format text|json|csv] [--status active|suggested|all]
biorouter dictionary add       TERM… [--spoken "…"] [--private]
biorouter dictionary edit      TERM | --id ID  [--term NEW] [--spoken …]
biorouter dictionary remove    TERM… | --id ID…
biorouter dictionary review
biorouter dictionary accept    ID…
biorouter dictionary dismiss   ID…
biorouter dictionary import    FILE|- [--format txt|csv|tsv|json] [--dry-run]
biorouter dictionary export    [FILE|-] [--include-private]
biorouter dictionary publicize TERM | --id ID
biorouter dictionary privatize TERM | --id ID
biorouter dictionary settings  [--agent-writes suggest|on_request|add] [--learn-from-chats on|off]
                               [--suggest-from-chats on|off]   (PR-23)
biorouter dictionary clear     [--suggested-only]
```

- Every command prints the account and the store folder it acts on to stderr (`Dictionary: <folder> (account <user>)`), so stdout stays clean for `list` and `export`. A refusal prints its sentence alone.
- `--spoken` takes a comma separated list and may repeat; with `add` it goes with one term at a time. `add` stamps public unless `--private` (O-6: a terminal add starts from no chat), and a private terminal add carries no institution (O-36).
- A new store starts with `agent_writes` set to `on_request` (O-3); `settings --agent-writes suggest` lowers it and asks nothing, `--agent-writes add` raises it and asks first.
- A `TERM` names every row with that exact written form that the caller can list. When it names more than one (twins), the command lists them with ids and asks which, or takes `--id`.
- `accept ID…` activates the named rows; `dismiss ID…` acts only on rows still `suggested`, as `POST /dictionary/suggestions/dismiss` (PR-16) does: it deletes each such row with its tombstone and answers any other id as MSG-NOT-FOUND does. That is the stricter of the two rules the code holds, taken as the plan's direction. **Deviation, as built in PR-13:** the CLI's `dismiss` deletes the row each id names with its tombstone whatever its status, an active term the person added included [C `crates/biorouter-cli/src/commands/dictionary.rs` `accept_or_dismiss`]; PR-23, the next pull request that changes that file, narrows it to this rule.
- `review` shows suggestions and unreviewed agent rows in a multiselect (written form, source, "private" for a private row, reason); the selected rows are accepted or kept, then it asks "Dismiss the rest? [y/N]". A row another surface accepted or kept while the person answered is not dismissed.
- `publicize` and `privatize` show the publicize warning (publicize: MSG-PUBLICIZE-WARN or MSG-PUBLICIZE-WARN-NO-CHAT, by the rule under **Make public…** above) and then ask the person to type the term exactly (O-5). Only then is `UserDictionaryTierChange` constructed. A blank answer cancels; one that does not match exits 1 and changes nothing. `publicize TERM` names the private row, `privatize TERM` the public one.
- Escape at any question cancels the command as a blank answer or **Cancel** does: every question comes before the command's one write, so it prints "Nothing was changed." (`export`: "Nothing was exported.") and exits 0; until the live check of 2026-11-06 it printed cliclack's "operation interrupted" and exited 1 [M]. Ctrl-C ends the process with SIGINT (`console` raises it), also before any write; no question holds the store's lock, so another terminal's add lands while one waits [M].
- `clear` counts the rows `list` shows, prints "This deletes `<n>` terms from `<folder>`." (MSG-CLEAR) and asks the person to type `delete <n>`. `--suggested-only` counts suggestions only. It then deletes those rows and no other, as the page's **Delete all terms…** deletes the listed rows: a row another process added while the person typed (an agent's term, a suggestion) stays, `--suggested-only` keeps a suggestion accepted meanwhile, and the report says how many were deleted. An `auto` row seen once, which no listing shows, is neither counted nor deleted. Each deleted row leaves its tombstone, as a delete on the Dictionary page does.
- `import` takes the format from the file's extension (`.txt`, `.csv`, `.tsv`, `.json`); `--format` overrides it, and standard input (`-`) needs it unless the content is the export JSON. It reads at most 16 MB. It prints `Imported: <n> new, <n> merged, <n> duplicate, <n> invalid.` (`Dry run:` with `--dry-run`) and one `line <n>: <code>: <sentence>` per refused row. Without a terminal the rows a file marks private are reported as one count, `Private terms received: <n>.`, never as new or already present, so the plan counts only rows the caller can list (oracle rule 3).
- `settings` with no flag prints the stored settings, which work without a terminal. A flag that raises `agent_writes`, or turns `learn_from_chats` or `suggest_from_chats` on, asks "Change this setting? [y/N]" first; lowering or turning off asks nothing. Under managed `dictionary_agent_writes: false`, raising either is refused with the policy's sentence. `--suggest-from-chats` goes through `activation.rs`, as the install box does, and arrives with it in PR-23; until then `settings` prints two settings. This is the host step that MSG-DICTIONARY-HOST-SETTINGS names on `serve`.
- `list --format text` columns: written, status, tier, source, uses, id; `csv` the same; `json` an array of `EntryView`. Without a terminal `uses` reads `-` and `EntryView` carries no usage fields or `suggested_in`, as a route gives a caller without private standing (oracle rule 5). Like every route, `list` and `review` never show an `auto` row seen fewer than two times.
- Export carries active rows only: a suggestion still waits for the person, and an import elsewhere would make it active. A file written with private terms is 0600 on Unix, also one that was already there (an earlier public export at 0644): it is narrowed while still empty, before a private term is written.

### Terminal only rules

| Situation | Reads | Writes | Tier change | Private rows |
|---|---|---|---|---|
| Interactive terminal (`prompt_can_run()` [C `crates/biorouter-cli/src/commands/needs_terminal.rs:49`]), `BIOROUTER_TERMINAL` unset | yes | yes | after the typed confirmation | listed and reviewed; exported only with `--include-private`, `caller_capability()` Private [C `crates/biorouter-cli/src/session/privacy.rs:65`] and MSG-EXPORT-PRIVATE |
| No terminal (pipe, cron, `ssh host cmd`), `BIOROUTER_TERMINAL` unset | public rows | refused (exit 2), except `import` | refused | withheld, with no count |
| An agent's shell (`BIOROUTER_TERMINAL=1`) | public rows | refused with MSG-DICTIONARY-AGENT (exit 2) | refused | withheld, with no count |
| Managed `dictionary: false` | yes | refused with MSG-POLICY (exit 2); deleting (`remove`, `dismiss`, `clear`) and `export` allowed | refused | as above |

The refusals are checked before the store is touched, in this order: an agent's shell, then the policy, then the terminal. A write other than `import` without a terminal answers "Changing the dictionary needs you at an interactive terminal. To add terms from a file or a pipe, use `biorouter dictionary import`.", and `export --include-private` where DD-77 does not admit it answers "Exporting private terms needs you at your own terminal and a private model configured on this computer. Run the command without --include-private to export public terms."; both exit 2, as MSG-DICTIONARY-TTY and MSG-EXPORT-PRIVATE-REFUSED in the message catalog, where every other sentence these commands print has its id too (MSG-SPOKEN-ONE-TERM to MSG-DICTIONARY-FOLDER), pinned by `the_dictionary_sentences_are_the_catalogs` in `commands/dictionary.rs`. MSG-DICTIONARY-AGENT names `dictionary__add_terms`, which PR-23 adds. A store that cannot be read answers MSG-DICTIONARY-FILE with exit 1.

`import` is admitted without a terminal (still refused under `BIOROUTER_TERMINAL=1` and by policy), because the documented sync pipes into it over SSH; it only merges and never deletes. Rows marked `tier: "private"` import private, and the rest import public. Every other write, `settings` included, needs a terminal, the rule DD-76 applies to live capture, as accident prevention and not a boundary. Private rows follow DD-37 at an interactive terminal with `BIOROUTER_TERMINAL` unset: `list` and `review` show every row. Only `export --include-private` also needs `caller_capability()` Private, the part of DD-77 that applies here.

### In a session

In the TUI and the classic REPL, `/dictionary add <term>` and `/dictionary review` are intercepted in `session/input.rs` beside `/folder-access` [C `crates/biorouter-cli/src/session/input.rs`] and handled in `session/mod.rs` and `session/tui/mod.rs` `handle_slash`; they never reach the model (PR-26). `/dictionary` alone prints the two forms and the counts. `add` defaults to the session's tier, and a private `add` carries the session's institutions: its provider's `affiliation()` united with the session's `session_affiliations`, read in process ([Private rows and institutions](#private-rows-and-institutions), O-36). `review` is the TUI modal or a `cliclack` multiselect, with local human authority (`DecisionAuthority::from_local_human_surface()` [C]). When suggestions wait, the session starts with one quiet line: "3 dictionary suggestions; /dictionary review".

### Import and export formats

Export (DD-43):

```json
{
  "format": "biorouter.dictionary",
  "version": 1,
  "exported_at": "2026-11-04T18:00:00Z",
  "entries": [
    { "written": "EPAS1", "spoken_forms": ["epas one"], "misrecognitions": ["a pass one"],
      "kind": "gene_symbol", "match": "exact", "note": null },
    { "written": "PHD2", "spoken_forms": [], "misrecognitions": [], "kind": "gene_symbol",
      "match": "exact", "note": null, "tier": "private", "affiliation": null }
  ]
}
```

No ids, session ids or usage. Private rows only after MSG-EXPORT-PRIVATE, each carrying `"tier": "private"` so an import elsewhere restamps it private, and its `affiliation` ([Private rows and institutions](#private-rows-and-institutions)).

Import accepts:

| Format | Shape |
|---|---|
| `lines` (`--format txt`) | one term per line; `EPAS1 = epas one, e pass one` adds spoken forms; a `#` at the start of a line or after a space or tab starts a comment, so `C#` and `F#` import as written |
| `csv`, `tsv` | header `written,spoken_forms,note`; spoken forms separated by `;` inside the field |
| `json` | the export shape above |

The plan for each row is `new`, `merged` (union of spoken forms and misrecognitions into the existing row), `duplicate` or `invalid` with its code. Import never deletes. Imported misrecognitions start at `count` 2, since the person supplied them. `--dry-run` prints the plan.

### Syncing two hosts

Two hosts have two dictionaries (a laptop and an SSH server, or a Windows desktop and its WSL account). The only sync is a pipe the person runs, the one MSG-SYNC-HINT shows ("To use your laptop's dictionary terms here, run on your laptop:" followed by the command):

```bash
biorouter dictionary export - | ssh <host> biorouter dictionary import -
```

On a systemd host, the receiving side runs as the service account (`sudo -u biorouter -H biorouter dictionary import -`). Adding `--include-private` on the exporting side carries private terms after the confirmation; they import private.

**Acceptance.** PR-13: every write except `import` is refused without a terminal (exit 2); every write, `import` included, is refused under `BIOROUTER_TERMINAL=1`; a piped `import -` merges; `settings` asks before raising and not before lowering; a piped `list` withholds private rows with no count, and `list` at an interactive terminal shows them on a machine configured for a public provider; `publicize` needs the typed phrase; `UserDictionaryTierChange` built in exactly two files; a term added in one terminal is listed by a second process at once. PR-26: `/dictionary add` and `/dictionary review` never reach the model; a private `/dictionary add` in a session on a local model whose `session_affiliations` holds `ucsf` writes `{ucsf}` (O-36).

## Store protection and boundaries

### The path inspector

New `crates/biorouter/src/security/dictionary_store.rs` (PR-13), modelled on `security/session_store.rs` [C]: `is_in_dictionary_store(path)`, a `DictionaryStoreInspector` registered beside `SessionStoreInspector` in `Agent::create_tool_inspection_manager` [C `agent.rs:6264`], and an `uninspected_boundary_refusal(tool_name, arguments, boundary)`. It refuses tool reads and writes of `<config>/dictionary/` for callers of both tiers, using the same path resolution as the session store inspector (DD-42). Its refusal reaches the model in its own words: `tool_execution::denied_response_text` passes the reasons of both store inspectors through. Before PR-13 the session store's reason was replaced with "The user has declined to run this tool", and the dictionary's was too until a live run showed it: GPT-5.5 on `versa_azure`, asked to `cat` the store and view it with the text editor, batched both into one `execute_code` call, which the inspector refused, and reported that the user had declined [M, 2026-11-06]. After the fix the same run received the refusal, tried no other way to the file, and the private term in the store never appeared in its output [M].

### Boundary refusals

`POST /agent/call_tool` dispatches with `CallCapability::public_enforced()` on the secret alone [C `crates/biorouter-server/src/routes/agent.rs:3398`], so a private chat's model holding the secret could otherwise land public stamped rows. `dictionary_store::uninspected_boundary_refusal` refuses `dictionary__add_terms` and `dictionary__remove_added_terms` at both uninspected doors:

- `call_tool_boundary_refusal` [C `routes/agent.rs:3320`], chained after the global memory, session store and knowledge delete refusals;
- the `execute_code` chain's `uninspected_boundary_refusal` [C `crates/biorouter/src/agents/code_execution_extension.rs:3569`].

Both doors also refuse a call whose already evaluated arguments name the store's folder, as the session store's boundary refusal does for its database, so a path a script assembles at run time is refused there although the inspector cannot see it. PR-13 lands both refusals: `call_tool_refuses_the_dictionary_writing_tools` in `routes/agent.rs` asks the route's chain, and `every_door_asks_this_module` in `security/dictionary_store.rs` reads the inspector registry and both chains. The script door refuses both tools even from a private chat, as DD-42 says. A script's inner calls inherit the capability its `execute_code` call was admitted with [C `code_execution_extension.rs` `get_tool_infos`], and the inspector chain does not reach a script that a person ran through `POST /agent/call_tool` [C `uninspected_boundary_refusal` doc comment]. Such a script runs as `public_enforced()`, so a private chat's term added from it would land public. The agent calls `dictionary__add_terms` directly instead; `dictionary__list_terms` stays callable from scripts.

### Managed policy keys

The managed `dictation:` block ([Engine and models](engine-and-models.md) and [managed policy](../security/managed-policy.md)) has two dictionary keys (DD-90). Each only narrows; an unknown key, a wrong type, an untrusted file or a failed load turns dictation off.

| Key | Effect | Choke points |
|---|---|---|
| `dictionary: false` | no dictionary edits, sidebar row, capability, extractor or bias | every `/dictionary/*` write and observation (MSG-POLICY); `add_extension_with_origin` (a resumed roster cannot load the capability); `permission_for` denies `dictionary__*` [C `crates/biorouter/src/managed/mod.rs`]; the managed deny arm in `call_tool_boundary_refusal`; the extractor; every CLI write (exit 2); `bias_set` returns `EligibleEntries::empty()`, reading the policy at every call, so a dictation under the key sends no prompt, corrects nothing and records no usage (PR-14b, `a_forbidden_dictionary_gives_the_empty_set`); `biorouter transcribe` reads the key once and then prepares no dictionary, as with `--no-dictionary` (`a_forbidden_dictionary_prepares_nothing_and_records_nothing`). Export and delete stay allowed |
| `dictionary_agent_writes: false` | the agent's tools are not offered; the extractor is off; `agent_writes` pinned to `suggest` | `permission_for`; the roster; `POST /dictionary/settings` refuses raising; the person's manual edits and the correction learner still work |

### What a shell can still do

A shell can read `dictionary.json` directly, because DR-17 left the filesystem open. The inspector stops a cooperative agent's file tools and named paths, not a determined actor. It compares the path as written, as the session store's inspector does: through `POST /agent/call_tool` in a live session on `versa_azure`, `cat` and `grep` of the store, `text_editor` view and write, and an `execute_code` script naming it were all refused, while `cd <config> && cat dictionary/dictionary.json`, a glob (`<config>/dict*/`), a doubled slash, a `/./` segment and `Dictionary` on the case insensitive macOS disk each returned the private term [M, 2026-11-06]. On a keyless daemon a shell that recovers the daemon secret (`ps eww`) can also make every dictionary write a `serve` browser may make and install a pinned model ([What is and is not claimed](#what-is-and-is-not-claimed)). These are recorded as residuals, as accident prevention.

**Acceptance.** PR-13: `cat` and `text_editor view` of the store refused for both tiers. PR-23: `/agent/call_tool` with `dictionary__add_terms` refused. PR-23, with the policy tests of PR-05: a resumed roster does not load `dictionary` under policy.

## Privacy invariants

| Id | Invariant | Named tests |
|---|---|---|
| P-1 | Audio stays in memory in Biorouter's processes on the transcribing host: no temp file, no log line, no diagnostics entry, no session row; the stream buffer is zeroed at finish, cancel or the 60 s expiry; no off machine engine in v1. A reverse proxy in front of `serve` is the operator's: `browser-access.md` names the setting that keeps request bodies in memory (`client_body_buffer_size 512k` or `proxy_request_buffering off` on nginx, whose stock configuration wrote every chunk above 16 KiB to disk [M]) | `dictation_routes.rs` (no new file in the sandbox root; tracing capture has no audio derived content); buffer zeroed on drop (unit); `no_cloud_audio_routes.rs`; the `serve` job's nginx log has no "buffered to a temporary file" line |
| P-2 | Transcripts live only in the composer: never in notifications (counts only), logs, `messages.metadata_json`, telemetry or the worker's stderr | tracing capture in `dictation_routes.rs`; the `UserMessage.tsx` grep test; notification tests in `dictationAnywhere.test.ts`; the OpenAPI diff shows no dictation field on `Message`; the log callback test |
| P-3 | One bias set feeds every channel | the `bias.rs` table test; `eligible_entries_are_built_only_by_bias_set` |
| P-4 | The daemon decides the target | `dictation_routes.rs`: a refused `session_id` gives public scope and an identical response; an unproven caller's `new_chat_provider` can only lower the target (a private default with a public pick is public); a `new_chat` stream takes the tier the bind would produce, so an `ollama` cloud tag, a remote `OLLAMA_HOST` and a private worker under a public lead give public scope, and a draft decision model that would make the chat public gives public scope (DR-33) |
| P-5 | Public by default: Crew, clipboard, app preview and `dictate` without an admitted `--session` take private terms only for one recording a person confirms after MSG-PRIVATE-TERMS, never for an agent. Public only: ACP and `transcribe` without an admitted `--session` | `bias.rs` rows for `crew` and `external` with and without `include_private_terms`; `include_private_terms` ignored without the proof; CLI tests (`--private-terms` refused under `BIOROUTER_TERMINAL=1` and without a terminal) |
| P-6 | No existence oracle | `dictionary_oracles.rs` (including the keyless `private: true` add from a caller without private standing, byte identical for an existing and an absent private term); the echo oracle test in `dictation_routes.rs`; caps per tier (unit); `transcribe --format json` has `bias_scope` and no count |
| P-7 | A cooperative agent, and any caller on a keyed daemon without the proof, never opens a microphone through a route, a tool or a capture path, transcribes for itself through a capture path, installs, removes or selects a model, accepts or dismisses suggestions, edits or deletes a person's row, changes a tier, raises `agent_writes` or turns learning on. Residuals, recorded in SD-17 and [What is and is not claimed](#what-is-and-is-not-claimed): on a keyless daemon any holder of the daemon secret, including an agent shell that recovers it, can install a pinned model and make the dictionary writes a `serve` browser may make on rows it can list (private ones on a host whose operator's provider is private); and an agent holding a Biorouter Copilot grant on Biorouter's own window, or on a browser that shows a `serve` tab, can start the microphone by clicking it or pressing the dictation chord, because Copilot's per task grant is the only gate there (DD-88, the owner's decision O-15 (2026-11-05)) | `dictation_keyed.rs`; `capture_consent_is_minted_in_one_file`; CLI refusals when `BIOROUTER_TERMINAL=1`; the capability's tool list is exactly the three tools; boundary refusals at `call_tool` and in scripts; no daemon route and no tool starts capture (`dictationAnywhere.test.ts`); the keyless refusals in `dictionary_no_user_key.rs` |
| P-8 | The person is warned, then free: publicize, export private terms, include private terms in one clipboard, stdout, Crew or app preview recording, send a privately biased draft to a public model or switch to one, dictate to a plain `http:` remote, install turbo on a CPU host | renderer tests for MSG-PUBLICIZE-WARN, MSG-EXPORT-PRIVATE, MSG-PRIVATE-TERMS, MSG-PRIVATE-DRAFT, MSG-PRIVATE-DRAFT-SWITCH, MSG-REMOTE-HTTP, MSG-TURBO-CPU; the `--private-terms` CLI test |
| P-9 | Agent terms carry no aliases; by default (`on_request`, O-3) an agent term is active only when it occurs in the person's latest message or in the agent message that this person message answers (checked mechanically; the reply is not checked for agreement, a residual recorded in the risks), and is a suggestion otherwise; the capability is absent from Crew scoped, app, `--ws`, SubAgent, Scheduled, Hidden and Terminal sessions; ACP only suggests; the extractor never runs in ACP | the capability's tests in PR-23 (a new store at `on_request`, and the mode tests); the extractor tests in PR-24 |
| P-10 | The network facing daemon cannot record: `biorouterd` links no capture code; capture starts only behind `CaptureConsent` or a person's gesture in a page | the daemon binary contract in `dictation_routes_auth.rs` (`biorouterd __br-dictation-worker capture` exits 2; the bytes of the unstripped `biorouterd` contain neither `ma_device_init` nor `libpulse.so.0`); `cargo tree -p biorouter-server -e normal,build` holds no `biorouter-dictation-capture`; on the binaries `cross-build-nightly` builds with the release command itself (`cargo build --release --bin biorouterd --bin biorouter`), the exit check and `strings biorouterd \| grep -c libpulse` is 0; the same `strings` check in every package smoke. The release build is checked as well because one Cargo invocation unifies the features of a crate both binaries use, so a test that builds only `biorouter-server` cannot see what the release build links in [I] |
| P-11 | Only pinned bytes are parsed | installer tests (mismatch deletes, `.partial` never loaded, ids only); the self test refuses a non pinned model; a fresh hash after an mtime change |
| P-12 | The worker is a closed box: frames on stdin and stdout only, no socket, daemon secrets stripped, no configuration or log files, exits on end of file, library logs filtered | `dictation_worker.rs` environment scrub; the 300 byte log line test |
| P-13 | Tier changes are the person's | `the_tier_proof_is_constructed_in_exactly_two_files`; the keyless tier refusal |
| P-14 | No private entry text leaves the store except to a caller allowed to list it | instructions contain no term text; a piped `list` withholds private rows; refusal snapshots name no term; a keyless add from a caller without private standing never answers with a private row |
| P-15 | Policy only narrows | the parse isolation test; `dictation_policy_routes.rs`; a resumed roster does not load `dictionary` under policy |
| P-16 | A private dictionary row carries the institutions whose data it may hold: the union of the institutions behind the model of the chat it was written in (lead and worker folded) and the institutions whose connectors that chat had touched (`sessions.session_affiliations`), for a term the person adds inside a private chat too (O-35, O-36); none for a row with no chat (the Dictionary page, `biorouter dictionary add`, an imported row without an affiliation, `privatize`) or from a chat whose model and connectors name no institution; and an empty list, which only local models clear, when the chat's record or its model cannot be read. It shapes dictation only into a target whose model every one of those institutions admits, each asked through the comparison DR-26 applies to extensions (`affiliation::gate_cross_affiliation_warning`), never by a hand comparison, so a row that names two or more institutions shapes only dictation aimed at a local model. The target is its model alone, and a target whose model is unstated takes only rows with no institution (O-43). Only the privacy master switch off, the mixing policy `open` (DR-27) and the person including private terms for one recording after MSG-PRIVATE-TERMS (O-37) lift it. The agent's `list_terms` applies the same rule. A row left out changes no field of a response and is never counted | the affiliation table in `bias.rs` (rows `null`, `{ucsf}`, `{stanford, ucsf}` and `[]` against targets `Local`, `{ucsf}`, `{stanford}`, `{stanford, ucsf}` and unstated, with the master switch on and off, the mixing policies `standard`, `strict` and `open`, and `include_private_terms` admitted or not; `{stanford, ucsf}` reaches only `Local`); the stamp table of `affiliation::dictionary_row_institutions` (a model `{ucsf}`, `Local`, unstated or `{stanford, ucsf}`, each with a chat that touched nothing, `{ucsf}` or `{stanford}`; an unknown model or a record read that answers `Err` gives `[]`; a union of 9 ids gives `[]`); the stamp tests for `add_terms`, the extractor and observations (a chat on a local model whose `session_affiliations` holds `ucsf` writes `{ucsf}`; a record holding `not-json`, written the way `malformed_source_affiliation_fails_before_copying_a_session` writes one, writes `[]`); a context menu add from such a chat (`POST /dictionary/entries` with `origin_session_id`) writes `{ucsf}`, one with `origin_unresolved: true` writes `[]`, and a private `/dictionary add` in the TUI and REPL follows its session the same way; an add from the Dictionary page and `biorouter dictionary add --private` write `null`; `publicize` removes the value, and an import stores `null` for a row without an affiliation and `[]` for a malformed one; `list_terms` from a chat on a `{stanford}` model omits a `{ucsf}` row; `dictation_routes.rs`: a stream into a chat whose live model is covered by another institution answers the same fields as one where the row is eligible; the `privacy_capability.rs` census row for `affiliation::gate_cross_affiliation` in `bias.rs` (one occurrence, inside the loop over a row's institutions); `compatible_is_the_only_function_that_compares_two_affiliations` stays green |

## Related documentation

- [Implementation plan](implementation-plan.md): the requirements, the decision register this document elaborates, and the order of the pull requests.
- [Voice dictation](README.md): the folder index and what each dictation document covers.
- [Engine and models](engine-and-models.md): the worker protocol that carries the bias set, the decoding parameters, the `/dictation/*` routes and the managed `dictation:` block.
- [Surfaces](surfaces.md): the composer, the private draft flag, the setup popover and the full message catalog quoted here by id.
- [Testing and release](testing-and-release.md): where the dictionary's tests run, the accuracy gates and the measurements M-19, M-21 and M-22.
- [Speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md): the measurements behind the token prompt, the decoder bonus and the correction guards.
- [Dictation tools survey](../research/voice-dictation/dictation-tools-survey.md): how other dictation tools handle vocabularies, aliases and learning.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules, DR-15 and DR-17, that the bias set and the stamps follow.
- [Institutional affiliation](../security/institutional-affiliation.md): the affiliation axis (DR-26) that private dictionary rows carry (O-32, O-35, P-16).
- [Managed policy](../security/managed-policy.md): the policy file that the `dictionary` and `dictionary_agent_writes` keys extend.
- [Memory extension](../extensions/built-in/memory.md): the memory behaviour that R5 compares the `dictionary` capability with.
- [Serve decisions](../deployment/serve-decisions.md): SD-8 and SD-11, the keyless shapes the `serve` write rules follow.
- [Sidebar chat list](../desktop-ui/sidebar-chat-list.md): the sidebar the **Dictionary** row joins.
- [Shared controls](../desktop-ui/shared-controls.md): the fill and control rules the Dictionary page follows.
- [Command reference](../cli/command-reference.md): where the `biorouter dictionary` commands join the CLI reference when they ship.
